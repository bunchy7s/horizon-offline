//! Structure blocks on the server: `StructureBlockEntity`'s save, load and
//! scan, the `StructureTemplateManager` they go through (templates kept in
//! memory, written to `<world>/generated/<namespace>/structures/<path>.nbt`),
//! `StructureBlock.neighborChanged` and `handleSetStructureBlock`.
use super::{Game, lan::LanHost};
use horizon_client::{
    containers::BlockEntity,
    gizmos::{Gizmos, InvisibleBlock},
    save,
    structure_block::{
        self, RenderMode, SCAN_CORNER_BLOCKS_RANGE, StructureBlock, StructureMode, UpdateType,
    },
    text::Text,
};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::play;
use horizon_world::{
    compression::Compression,
    level::{TemplateSettings, flags},
    nbt::{self, Compound, Limits, NamedTag, Tag},
    worldgen::structure::{BlockInfo, EntityInfo, StructureTemplate, save_nbt},
};
use std::collections::HashSet;

type Pos = (i32, i32, i32);

/// A template the manager holds: its file's tree and the author `setAuthor`
/// gave it (the file does not keep one, so a loaded template's is `?`).
#[derive(Clone)]
pub(super) struct TemplateEntry {
    pub(super) tag: Tag,
    pub(super) author: String,
}

/// `StructureTemplate.size` of a template's tree.
pub(super) fn template_size(tag: &Tag) -> [i32; 3] {
    let mut size = [0; 3];
    if let Some(Tag::List(values)) = tag.get("size") {
        for (slot, value) in size.iter_mut().zip(values) {
            if let Tag::Int(v) = value {
                *slot = *v;
            }
        }
    }
    size
}

/// `FileUtil.isPathPortable`-like check of the path under `structures/`: no
/// empty, `.` or `..` segments.
fn portable_path(path: &str) -> bool {
    path.split('/')
        .all(|s| !s.is_empty() && s != "." && s != "..")
}

impl Game<'_> {
    /// `BlockEntityWithBoundingBoxRenderer`: for a player who may use game
    /// master blocks (or a spectator), each structure block's box and, with
    /// Show Invisible Blocks on, a marker for every barrier, light, structure
    /// void and air block in it. Blocks beyond `getViewDistance` (96) are
    /// not drawn.
    pub(super) fn bounding_box_gizmos(&self, camera: [f64; 3], g: &mut Gizmos) {
        let game_master = self.can_use_game_master_blocks() || self.spectator();
        let facts = self.res.facts();
        for (pos, entity) in self.world.block_entities.boxes() {
            let centre = [pos.0, pos.1, pos.2].map(|v| f64::from(v) + 0.5);
            let dist: f64 = (0..3).map(|i| (centre[i] - camera[i]).powi(2)).sum();
            let block = [pos.0, pos.1, pos.2];
            let (mode, area) = match entity {
                BlockEntity::StructureBlock(b) if game_master && dist < 96.0 * 96.0 => {
                    (b.render_mode(), b.renderable_box())
                }
                BlockEntity::TestInstanceBlock(b) => {
                    if !game_master || dist >= 96.0 * 96.0 {
                        continue;
                    }
                    // TestInstanceBlockEntity.renderMode: BOX.
                    let padding = self.test_holder_padding(*pos);
                    let (offset, size) = b.renderable_box(padding, self.test_rotation(*pos));
                    (
                        RenderMode::Box,
                        horizon_client::structure_block::RenderableBox { offset, size },
                    )
                }
                _ => continue,
            };
            if mode == RenderMode::None {
                continue;
            }
            let invisible = (mode == RenderMode::BoxAndInvisibleBlocks).then(|| {
                let [sx, sy, sz] = area.size;
                let mut cells = vec![None; (sx.max(0) * sy.max(0) * sz.max(0)) as usize];
                for x in 0..sx {
                    for y in 0..sy {
                        for z in 0..sz {
                            let at = [0, 1, 2].map(|i| block[i] + area.offset[i] + [x, y, z][i]);
                            let state = self.world.block(at[0], at[1], at[2]);
                            let kind = if facts.is_air(state) {
                                Some(InvisibleBlock::Air)
                            } else {
                                match &*facts.get(state).name {
                                    "minecraft:structure_void" => {
                                        Some(InvisibleBlock::StructureVoid)
                                    }
                                    "minecraft:barrier" => Some(InvisibleBlock::Barrier),
                                    "minecraft:light" => Some(InvisibleBlock::Light),
                                    _ => None,
                                }
                            };
                            cells[(z * sx * sy + y * sx + x) as usize] = kind;
                        }
                    }
                }
                cells
            });
            g.bounding_box(block, area.offset, area.size, invisible.as_deref());
        }
    }

    fn structure_block_at(&self, pos: Pos) -> Option<&StructureBlock> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::StructureBlock(b) => Some(b),
            _ => None,
        }
    }
    fn structure_block_mut(&mut self, pos: Pos) -> Option<&mut StructureBlock> {
        match self.world.block_entities.get_mut(&pos)? {
            BlockEntity::StructureBlock(b) => Some(b),
            _ => None,
        }
    }

    /// `StructureBlock.setPlacedBy`: the placing player is the author.
    pub(super) fn structure_block_placed(&mut self, pos: Pos) {
        let author = self
            .lan_acting_name
            .clone()
            .unwrap_or_else(|| self.options.player_name());
        if let Some(b) = self.structure_block_mut(pos) {
            b.author = author;
        }
    }

    /// `StructureTemplateManager.get`: the cache (misses included), else the
    /// world's saved structures, else the game's own.
    pub(super) fn template_entry(&mut self, name: &str) -> Option<TemplateEntry> {
        if let Some(cached) = self.structure_templates.get(name) {
            return cached.clone();
        }
        let loaded = self.read_template_tag(name).map(|tag| TemplateEntry {
            tag,
            author: "?".to_owned(),
        });
        self.structure_templates
            .insert(name.to_owned(), loaded.clone());
        loaded
    }

    /// `StructureTemplateManager.save`: the file under the world's
    /// `generated` folder.
    fn write_template(&self, name: &str, tag: &Tag) -> bool {
        let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
        if !portable_path(path) || !portable_path(ns) {
            return false;
        }
        let file = self
            .world_root
            .join("generated")
            .join(ns)
            .join("structures")
            .join(format!("{path}.nbt"));
        let Some(parent) = file.parent() else {
            return false;
        };
        let named = NamedTag {
            name: "".into(),
            tag: tag.clone(),
        };
        let Ok(raw) = nbt::write_named(&named, Limits::templates()) else {
            return false;
        };
        let Ok(bytes) = Compression::Gzip.encode(&raw) else {
            return false;
        };
        std::fs::create_dir_all(parent).is_ok() && std::fs::write(file, bytes).is_ok()
    }

    /// `StructureTemplate.fillFromWorld` for a box at `origin`: every block
    /// but structure voids with its block entity's data, and the entities
    /// (never players) in it.
    fn template_from_world(
        &mut self,
        origin: [i32; 3],
        size: [i32; 3],
        include_entities: bool,
        ignore_air: bool,
    ) -> Tag {
        let registry = self.pack.assets.registry;
        let void = registry.default_state("minecraft:structure_void");
        let air = ignore_air
            .then(|| registry.default_state("minecraft:air"))
            .flatten();
        let mut blocks = Vec::new();
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let at = (origin[0] + x, origin[1] + y, origin[2] + z);
                    let state = self.world.block(at.0, at.1, at.2);
                    if Some(state) == void || Some(state) == air {
                        continue;
                    }
                    let nbt = self.world.block_entities.get(&at).map(|entity| {
                        let name = &*self.res.facts().get(state).name;
                        let Tag::Compound(mut c) = save::encode_block_entity(at, entity, name)
                        else {
                            return Compound::new();
                        };
                        // `saveWithId`: the id, not the position.
                        for key in ["x", "y", "z", "keepPacked"] {
                            c.remove(&key.into());
                        }
                        c
                    });
                    blocks.push(BlockInfo {
                        pos: [x, y, z],
                        state,
                        nbt,
                    });
                }
            }
        }
        let entities = if include_entities {
            self.template_entities(origin, size)
        } else {
            Vec::new()
        };
        let transforms = self.state_transforms().expect("structure block table");
        save_nbt(
            registry,
            transforms,
            size,
            blocks,
            &entities,
            horizon_core::version::DATA_VERSION,
        )
    }

    /// `StructureTemplate.fillEntityList`: the entities inside the box, at
    /// positions relative to its minimum corner.
    fn template_entities(&mut self, origin: [i32; 3], size: [i32; 3]) -> Vec<EntityInfo> {
        let min = origin.map(f64::from);
        let max = [0, 1, 2].map(|i| min[i] + f64::from(size[i]));
        let chunks: HashSet<(i32, i32)> = (origin[0] >> 4..=(origin[0] + size[0] - 1) >> 4)
            .flat_map(|cx| {
                (origin[2] >> 4..=(origin[2] + size[2] - 1) >> 4).map(move |cz| (cx, cz))
            })
            .collect();
        let mut out = Vec::new();
        for (_, tags) in self.entity_chunks(&chunks, false) {
            for tag in tags {
                let Some(Tag::List(pos)) = tag.get("Pos") else {
                    continue;
                };
                let coords: Vec<f64> = pos
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Double(bits) => Some(f64::from_bits(*bits)),
                        _ => None,
                    })
                    .collect();
                if coords.len() != 3 || (0..3).any(|i| coords[i] < min[i] || coords[i] >= max[i]) {
                    continue;
                }
                let Tag::Compound(mut nbt) = tag else {
                    continue;
                };
                let relative = [0, 1, 2].map(|i| coords[i] - min[i]);
                let block_pos = relative.map(|v| v.floor() as i32);
                nbt.remove(&"UUID".into());
                out.push(EntityInfo {
                    pos: relative,
                    block_pos,
                    nbt,
                });
            }
        }
        out
    }

    /// `StructureBlockEntity.saveStructure(boolean)`: the template under the
    /// block's name takes the area, and is written when `to_disk`.
    fn save_structure(&mut self, pos: Pos, to_disk: bool) -> bool {
        let Some(b) = self.structure_block_at(pos).cloned() else {
            return false;
        };
        let Some(name) = b.name.clone() else {
            return false;
        };
        let origin = [pos.0 + b.pos[0], pos.1 + b.pos[1], pos.2 + b.pos[2]];
        self.save_structure_area(
            &name,
            origin,
            b.size,
            b.ignore_entities,
            &b.author,
            to_disk,
            false,
        )
    }

    /// The static `StructureBlockEntity.saveStructure`: `getOrCreate`, then
    /// `fillFromWorld`, which leaves a template alone when a side of the area
    /// is empty; `ignore_air` is `ignoreBlocks = [AIR]`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn save_structure_area(
        &mut self,
        name: &str,
        origin: [i32; 3],
        size: [i32; 3],
        ignore_entities: bool,
        author: &str,
        to_disk: bool,
        ignore_air: bool,
    ) -> bool {
        let existing = self.template_entry(name);
        let tag = if size.iter().all(|s| *s >= 1) {
            self.template_from_world(origin, size, !ignore_entities, ignore_air)
        } else {
            existing.map_or_else(
                || {
                    let transforms = self.state_transforms().expect("structure block table");
                    save_nbt(
                        self.pack.assets.registry,
                        transforms,
                        [0; 3],
                        Vec::new(),
                        &[],
                        horizon_core::version::DATA_VERSION,
                    )
                },
                |e| e.tag,
            )
        };
        let saved = !to_disk || self.write_template(name, &tag);
        self.structure_templates.insert(
            name.to_owned(),
            Some(TemplateEntry {
                tag,
                author: author.to_owned(),
            }),
        );
        saved
    }

    /// `StructureBlockEntity.loadStructureInfo(template)`.
    fn load_structure_info(&mut self, pos: Pos, entry: &TemplateEntry) {
        let size = template_size(&entry.tag);
        if let Some(b) = self.structure_block_mut(pos) {
            b.author = entry.author.clone();
            b.size = size;
        }
    }

    /// `StructureBlockEntity.placeStructure(level, template)`.
    fn place_structure(&mut self, pos: Pos, entry: &TemplateEntry) {
        self.load_structure_info(pos, entry);
        let Some(b) = self.structure_block_at(pos).cloned() else {
            return;
        };
        let at = BlockPos::new(pos.0 + b.pos[0], pos.1 + b.pos[1], pos.2 + b.pos[2]);
        let mut settings = load_settings(b.mirror, b.rotation, b.integrity, b.seed, b.strict);
        settings.ignore_entities = b.ignore_entities;
        self.place_template_entry(entry, at, settings, b.mirror, b.rotation);
    }

    /// `StructureTemplate.placeInWorld` of a template at `at`: the entities
    /// it brings are spawned and the block entities it loaded sent to the
    /// clients. Returns whether it placed.
    pub(super) fn place_template_entry(
        &mut self,
        entry: &TemplateEntry,
        at: BlockPos,
        settings: TemplateSettings,
        mirror: horizon_world::worldgen::structure::Mirror,
        rotation: horizon_world::worldgen::structure::Rotation,
    ) -> bool {
        let (Some(transforms), registry) = (self.state_transforms(), self.pack.assets.registry)
        else {
            return false;
        };
        let template = StructureTemplate::from_nbt(&entry.tag, registry, transforms);
        let placed = self.with_level(|behaviours, level, ticks| {
            behaviours.place_template(level, ticks, &template, at, settings)
        });
        let _ = self.process_level_events();
        let Some((placed, entities)) = placed else {
            return false;
        };
        for entity in entities {
            self.spawn_entity_from_nbt(Tag::Compound(entity));
        }
        // The block entities the template loaded reach the clients.
        let area = template.bounding_box([at.x, at.y, at.z], rotation, [0; 3], mirror);
        let keys: Vec<Pos> = self
            .world
            .block_entities
            .keys()
            .copied()
            .filter(|k| area.is_inside([k.0, k.1, k.2]))
            .collect();
        for key in keys {
            self.mark_block_entity_updated(key);
        }
        placed
    }

    /// `StructureBlockEntity.placeStructureIfSameSize`.
    fn place_structure_if_same_size(&mut self, pos: Pos) -> bool {
        let Some(b) = self.structure_block_at(pos) else {
            return false;
        };
        let (mode, name, size) = (b.mode, b.name.clone(), b.size);
        let Some(name) = name.filter(|_| mode == StructureMode::Load) else {
            return false;
        };
        let Some(entry) = self.template_entry(&name) else {
            return false;
        };
        if template_size(&entry.tag) == size {
            self.place_structure(pos, &entry);
            true
        } else {
            self.load_structure_info(pos, &entry);
            false
        }
    }

    /// `StructureBlockEntity.isStructureLoadable`.
    fn structure_loadable(&mut self, pos: Pos) -> bool {
        let Some(b) = self.structure_block_at(pos) else {
            return false;
        };
        let (mode, name) = (b.mode, b.name.clone());
        mode == StructureMode::Load && name.is_some_and(|name| self.template_entry(&name).is_some())
    }

    /// `StructureBlockEntity.detectSize`: the box the corner blocks of this
    /// structure's name enclose.
    fn detect_structure_size(&mut self, pos: Pos) -> bool {
        let Some(b) = self.structure_block_at(pos) else {
            return false;
        };
        if b.mode != StructureMode::Save {
            return false;
        }
        let name = b.name.clone();
        let range = SCAN_CORNER_BLOCKS_RANGE;
        let corners: Vec<[i32; 3]> = (&self.world.block_entities)
            .into_iter()
            .filter(|((x, _, z), _)| (x - pos.0).abs() <= range && (z - pos.2).abs() <= range)
            .filter_map(|(k, e)| match e {
                BlockEntity::StructureBlock(c)
                    if c.mode == StructureMode::Corner && c.name == name =>
                {
                    Some([k.0, k.1, k.2])
                }
                _ => None,
            })
            .collect();
        let at = [pos.0, pos.1, pos.2];
        let Some((min, max)) = StructureBlock::enclosing_box(&corners, at) else {
            return false;
        };
        let Some((offset, size)) = StructureBlock::detected(min, max, at) else {
            return false;
        };
        if let Some(b) = self.structure_block_mut(pos) {
            b.pos = offset;
            b.size = size;
        }
        true
    }

    /// `StructureBlockEntity.setMode`: the entity's and the block state's.
    fn set_structure_mode(&mut self, pos: Pos, mode: StructureMode) {
        if let Some(b) = self.structure_block_mut(pos) {
            b.mode = mode;
        }
        self.set_mode_property(pos, "minecraft:structure_block", mode.serialized_name());
    }

    /// The `mode` of the block state at `pos` (if it is `block`), set with
    /// `UPDATE_CLIENTS` and the block entity kept.
    pub(super) fn set_mode_property(&mut self, pos: Pos, block: &str, mode: &str) {
        let registry = self.pack.assets.registry;
        let current = registry.state(self.world.block(pos.0, pos.1, pos.2));
        let Some(current) = current.filter(|s| s.name == block) else {
            return;
        };
        let mut wanted = current.clone();
        wanted.properties.insert("mode".into(), mode.to_owned());
        if wanted == *current {
            return;
        }
        if let Ok(id) = registry.resolve(&wanted) {
            // The block entity stays through the state change.
            let entity = self.world.block_entities.take(&pos);
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, at, id, flags::UPDATE_CLIENTS)
            });
            if let Some(entity) = entity {
                self.world.block_entities.put(pos, entity);
            }
            let _ = self.process_level_events();
        }
    }

    /// `StructureBlock.trigger`.
    fn trigger_structure_block(&mut self, pos: Pos) {
        let Some(mode) = self.structure_block_at(pos).map(|b| b.mode) else {
            return;
        };
        match mode {
            StructureMode::Save => {
                self.save_structure(pos, false);
            }
            StructureMode::Load => {
                if let Some(name) = self.structure_block_at(pos).and_then(|b| b.name.clone())
                    && let Some(entry) = self.template_entry(&name)
                {
                    self.place_structure(pos, &entry);
                }
            }
            // `unloadStructure`: the manager forgets it, the next use reads the file.
            StructureMode::Corner => {
                if let Some(name) = self.structure_block_at(pos).and_then(|b| b.name.clone()) {
                    self.structure_templates.remove(&name);
                }
            }
            StructureMode::Data => {}
        }
    }

    /// `StructureBlock.neighborChanged` (the level's neighbour hook): a rising
    /// signal triggers the block once.
    pub(super) fn structure_block_neighbor_changed(&mut self, pos: Pos) {
        if self.structure_block_at(pos).is_none() {
            return;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        let signal = self.with_level(|b, level, ticks| b.has_neighbor_signal(level, ticks, at));
        let powered = self.structure_block_at(pos).is_some_and(|b| b.powered);
        if signal && !powered {
            if let Some(b) = self.structure_block_mut(pos) {
                b.powered = true;
            }
            self.trigger_structure_block(pos);
        } else if !signal
            && powered
            && let Some(b) = self.structure_block_mut(pos)
        {
            b.powered = false;
        }
    }

    /// `ServerGamePacketListenerImpl.handleSetStructureBlock` for a guest:
    /// only a creative player with game master commands edits the block.
    pub(super) fn lan_set_structure_block(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetStructureBlock,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest_messaged(host, conn, |game| {
            if allowed {
                game.apply_set_structure_block(&p);
            }
        });
    }

    /// What `handleSetStructureBlock` does once the player may edit blocks.
    fn apply_set_structure_block(&mut self, p: &play::SetStructureBlock) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        if self.structure_block_at(pos).is_none() {
            return;
        }
        let (Some(mode), Some(mirror), Some(rotation), Some(update)) = (
            StructureMode::from_ordinal(p.mode),
            structure_block::mirror_from_ordinal(p.mirror),
            structure_block::rotation_from_ordinal(p.rotation),
            UpdateType::from_ordinal(p.update_type),
        ) else {
            return;
        };
        self.set_structure_mode(pos, mode);
        if let Some(b) = self.structure_block_mut(pos) {
            b.set_structure_name(&p.name);
            b.pos = p.offset;
            b.size = p.size;
            b.mirror = mirror;
            b.rotation = rotation;
            b.metadata = p.data.clone();
            b.ignore_entities = p.ignore_entities;
            b.strict = p.strict;
            b.show_air = p.show_air;
            b.show_bounding_box = p.show_bounding_box;
            b.integrity = p.integrity;
            b.seed = p.seed;
        }
        let message = |key: &str, name: &str| Text::translate(key, &[Text::literal(name)]);
        match self.structure_block_at(pos).and_then(|b| b.name.clone()) {
            Some(name) => match update {
                UpdateType::SaveArea => {
                    let saved = self
                        .structure_block_at(pos)
                        .is_some_and(|b| b.mode == StructureMode::Save)
                        && self.save_structure(pos, true);
                    let key = if saved {
                        "structure_block.save_success"
                    } else {
                        "structure_block.save_failure"
                    };
                    self.system_message(message(key, &name));
                }
                UpdateType::LoadArea => {
                    let key = if !self.structure_loadable(pos) {
                        "structure_block.load_not_found"
                    } else if self.place_structure_if_same_size(pos) {
                        "structure_block.load_success"
                    } else {
                        "structure_block.load_prepare"
                    };
                    self.system_message(message(key, &name));
                }
                UpdateType::ScanArea => {
                    if self.detect_structure_size(pos) {
                        self.system_message(message("structure_block.size_success", &name));
                    } else {
                        self.system_message(Text::translate("structure_block.size_failure", &[]));
                    }
                }
                UpdateType::UpdateData => {}
            },
            None => {
                self.system_message(message("structure_block.invalid_structure_name", &p.name));
            }
        }
        // `setChanged` and `sendBlockUpdated`: the clients get the new data.
        self.mark_block_entity_updated(pos);
    }
}

/// The `StructurePlaceSettings` that `StructureBlockEntity.placeStructure`
/// and `PlaceCommand.placeTemplate` both build: mirror and rotation, the
/// known shape when strict, the integrity and seed that `place_template`
/// turns into `BlockRotProcessor` and `createRandom(seed)`, and the
/// `setBlock` flags `2 | (strict ? 816 : 0)`.
pub(super) fn load_settings(
    mirror: horizon_world::worldgen::structure::Mirror,
    rotation: horizon_world::worldgen::structure::Rotation,
    integrity: f32,
    seed: i64,
    strict: bool,
) -> TemplateSettings {
    TemplateSettings {
        mirror,
        rotation,
        ignore_entities: false,
        known_shape: strict,
        integrity,
        seed: create_seed(seed),
        flags: flags::UPDATE_CLIENTS | if strict { 816 } else { 0 },
    }
}

/// `StructureBlockEntity.createRandom`'s seed: 0 is the clock.
pub(super) fn create_seed(seed: i64) -> i64 {
    if seed == 0 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as i64)
    } else {
        seed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_stay_under_structures() {
        assert!(portable_path("house/small"));
        assert!(!portable_path("../x"));
        assert!(!portable_path("a//b"));
        assert!(!portable_path("a/./b"));
        assert!(!portable_path(""));
    }

    #[test]
    fn template_size_reads_the_size_list() {
        let tag = Tag::compound([(
            "size",
            Tag::List(vec![Tag::Int(3), Tag::Int(4), Tag::Int(5)]),
        )]);
        assert_eq!(template_size(&tag), [3, 4, 5]);
        assert_eq!(
            template_size(&Tag::compound(Vec::<(&str, Tag)>::new())),
            [0; 3]
        );
    }

    #[test]
    fn a_zero_seed_is_the_clock() {
        assert_eq!(create_seed(42), 42);
        assert_ne!(create_seed(0), 0);
    }

    #[test]
    fn template_settings_are_flags_2_and_unknown_shape_unless_strict() {
        use horizon_world::worldgen::structure::{Mirror, Rotation};
        let loose = load_settings(Mirror::LeftRight, Rotation::Clockwise90, 0.5, 7, false);
        assert_eq!(loose.flags, 2);
        assert!(!loose.known_shape);
        assert!(!loose.ignore_entities);
        assert_eq!((loose.integrity, loose.seed), (0.5, 7));
        assert_eq!(
            (loose.mirror, loose.rotation),
            (Mirror::LeftRight, Rotation::Clockwise90)
        );
        let strict = load_settings(Mirror::None, Rotation::None, 1.0, 7, true);
        assert_eq!(strict.flags, 818);
        assert!(strict.known_shape);
    }
}
