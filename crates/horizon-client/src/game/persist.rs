//! Saving and loading the game state that lives outside chunk blocks:
//! entities (`entities/*.mca`, per chunk like vanilla's `EntityStorage`) and
//! scheduled block/fluid ticks (chunk `block_ticks`/`fluid_ticks`).
use super::{FallingBlock, Game};
use horizon_client::{
    Result,
    chunk::Chunk,
    entity_nbt,
    mobs::Who,
    save::{ChunkExtras, random_uuid},
};
use horizon_core::{geometry::Vec3, position::ChunkPos, random::RandomSource, ticks::ChunkTicks};
use horizon_world::{chunk::BlockState, nbt::Tag, registry::BlockStateId};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// What Horizon remembers about loaded entities to save them faithfully.
#[derive(Default)]
pub struct EntityMemory {
    /// UUIDs of mobs and item entities by Horizon entity ID.
    pub(super) uuids: HashMap<u32, [i32; 4]>,
    /// The compound each mob or item entity was loaded with.
    pub(super) raw: HashMap<u32, Tag>,
    /// Entities of types Horizon does not simulate, kept per chunk and
    /// written back unchanged.
    unknown: HashMap<(i32, i32), Vec<Tag>>,
    /// Entities without a Horizon entity ID (orbs, arrows, falling blocks, crystals,
    /// the dragon): the UUID and compound they were loaded with, found again by
    /// type and position (an entity that has moved since loading is saved afresh).
    anon: HashMap<(String, [u64; 3]), (Vec3, Option<[i32; 4]>, Tag)>,
}

fn anon_key(kind: &str, p: Vec3) -> (String, [u64; 3]) {
    (
        kind.to_owned(),
        [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()],
    )
}
/// The loaded compound underneath the fields Horizon models: what the entity was
/// loaded with, with every key of `modelled` replacing its loaded value.
fn with_raw(modelled: Tag, raw: Option<&Tag>) -> Tag {
    let (Some(Tag::Compound(raw)), Tag::Compound(modelled)) = (raw, &modelled) else {
        return modelled;
    };
    let mut out = raw.clone();
    for (k, v) in modelled {
        // Defaults Horizon invents for fields it does not simulate give way to the save.
        let invented = matches!(
            k.to_string_lossy().as_str(),
            "Fire" | "Air" | "Invulnerable" | "PortalCooldown" | "HasBeenShot"
        );
        if !(invented && out.contains_key(k)) {
            out.insert(k.clone(), v.clone());
        }
    }
    Tag::Compound(out)
}

fn painting_center(p: &horizon_client::entities::Painting) -> Vec3 {
    let (lo, hi) = p.bounds();
    Vec3::new(
        (lo[0] + hi[0]) / 2.0,
        (lo[1] + hi[1]) / 2.0,
        (lo[2] + hi[2]) / 2.0,
    )
}
fn chunk_of(p: Vec3) -> (i32, i32) {
    ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4)
}

impl Game<'_> {
    /// Saved ticks and the save time for a chunk about to be written.
    pub(super) fn attach_save_data(&self, chunk: &mut Chunk) {
        let pos = ChunkPos::new(chunk.x, chunk.z);
        let time = self.world.game_time;
        let mut extras: ChunkExtras = chunk.saved.as_deref().cloned().unwrap_or_default();
        // A chunk the scheduler hasn't registered yet (one loaded and
        // unloaded again before the next tick) still holds its saved ticks:
        // `register_chunk_ticks` empties that list once it takes them.
        if let Some(t) = self.block_ticks.blocks.chunk(pos) {
            extras.block_ticks = t.pack(time);
        }
        if let Some(t) = self.block_ticks.fluids.chunk(pos) {
            extras.fluid_ticks = t.pack(time);
        }
        extras.last_update = time;
        // TheEndGatewayBlockEntity saves its Age (the server copy's).
        for tag in &mut extras.block_entities {
            let pos = [tag.get("x"), tag.get("y"), tag.get("z")]
                .map(|v| horizon_client::save::number(v).unwrap_or(0) as i32);
            if let Some(g) = self.be_anims.gateway_servers.get(&pos)
                && let Some(age) = tag.get_mut("Age")
            {
                *age = horizon_world::nbt::Tag::Long(g.age);
            }
        }
        if let Some(t) = self.world.inhabited.get(&(chunk.x, chunk.z)) {
            extras
                .root
                .insert("InhabitedTime".into(), horizon_world::nbt::Tag::Long(*t));
        }
        chunk.saved = Some(Arc::new(extras));
    }
    /// Register a newly resident chunk with the tick scheduler, with the
    /// block and fluid ticks it was saved with (`LevelChunk.registerTickContainerInLevel`
    /// at FULL). They are unpacked when the chunk starts ticking
    /// (`start_ticking_chunks`).
    pub(super) fn register_chunk_ticks(&mut self, key: (i32, i32)) {
        let pos = ChunkPos::new(key.0, key.1);
        let saved = self.world.chunks.get(&key).and_then(|c| c.saved.clone());
        // ChunkAccess.inhabitedTime as saved.
        if let Some(Tag::Long(t)) = saved
            .as_deref()
            .and_then(|s| s.root.get(&"InhabitedTime".into()))
        {
            self.world.inhabited.insert(key, *t);
        }
        let (blocks, fluids) = saved.as_deref().map_or((Vec::new(), Vec::new()), |s| {
            (s.block_ticks.clone(), s.fluid_ticks.clone())
        });
        let loaded = !blocks.is_empty() || !fluids.is_empty();
        if self
            .block_ticks
            .add_chunk(
                pos,
                ChunkTicks::from_saved(blocks),
                ChunkTicks::from_saved(fluids),
            )
            .is_ok()
            && loaded
        {
            self.tick_chunks.wait(key);
        }
        // The saved list is consumed: later saves take the scheduler's ticks.
        if loaded && let Some(c) = self.world.chunks.get_mut(&key) {
            let c = Arc::make_mut(c);
            if let Some(s) = c.saved.as_mut() {
                let s = Arc::make_mut(s);
                s.block_ticks.clear();
                s.fluid_ticks.clear();
            }
        }
    }
    /// Spawn the entities of chunks that finished loading.
    pub(super) fn spawn_loaded_entities(&mut self) {
        let loaded = std::mem::take(&mut self.world.loaded_entities);
        let player_uuid = self.level.player_uuid;
        for (key, tags) in loaded {
            for tag in tags {
                let kind = entity_nbt::kind(&tag).unwrap_or_default();
                let uuid = entity_nbt::uuid(&tag);
                if matches!(
                    kind.as_str(),
                    "minecraft:experience_orb"
                        | "minecraft:arrow"
                        | "minecraft:fireball"
                        | "minecraft:small_fireball"
                        | "minecraft:falling_block"
                        | "minecraft:end_crystal"
                        | "minecraft:ender_dragon"
                ) && let Some(at) = entity_nbt::position(&tag)
                {
                    self.persist
                        .anon
                        .insert(anon_key(&kind, at), (at, uuid, tag.clone()));
                }
                match kind.as_str() {
                    "minecraft:item" => {
                        self.next_entity += 1;
                        let bob = self.world.random.next_float() * std::f32::consts::PI * 2.0;
                        if let Some(e) =
                            entity_nbt::item_entity_from_nbt(&tag, self.next_entity, bob)
                        {
                            self.remember(e.id, uuid, tag);
                            self.dropped.push(e);
                        }
                    }
                    "minecraft:experience_orb" => {
                        for mut orb in entity_nbt::orbs_from_nbt(&tag) {
                            self.next_entity += 1;
                            orb.id = self.next_entity;
                            self.orbs.push(orb);
                        }
                    }
                    "minecraft:arrow" => {
                        if let Some((mut p, in_block)) = entity_nbt::arrow_from_nbt(
                            &tag,
                            player_uuid,
                            super::dispenser::tipped_effect,
                        ) {
                            if let Some(a) = p.arrow.as_mut() {
                                a.last_state = in_block.and_then(|t| self.state_from_nbt(&t));
                            }
                            self.add_projectile(p);
                        }
                    }
                    "minecraft:fireball" | "minecraft:small_fireball" => {
                        if let Some(p) = entity_nbt::fireball_from_nbt(&tag, player_uuid) {
                            self.add_projectile(p);
                        }
                    }
                    "minecraft:falling_block" => {
                        if let Some((state, position, velocity, age, fall_distance)) =
                            entity_nbt::falling_block_from_nbt(&tag)
                            && let Some(state) = self.state_from_nbt(&state)
                        {
                            let (no_gravity, drop_item) = entity_nbt::falling_block_flags(&tag);
                            self.falling.push(FallingBlock {
                                id: 0,
                                state,
                                // DATA_START_POS is not persisted by vanilla.
                                start_pos: horizon_core::position::BlockPos::default(),
                                position,
                                previous: position,
                                velocity,
                                age,
                                no_gravity,
                                drop_item,
                                fall_distance,
                                stuck: Vec3::ZERO,
                                damage: horizon_client::falling_block::Damage::from_nbt(
                                    &tag,
                                    self.res.facts().get(state).name.ends_with("anvil"),
                                ),
                                saved: uuid.map(|u| Box::new((u, tag.clone()))),
                            });
                        }
                    }
                    "minecraft:firework_rocket" => {
                        if let Some(r) = entity_nbt::firework_rocket_from_nbt(&tag, player_uuid) {
                            self.rockets.push(super::fireworks::Rocket::from_nbt(r));
                        }
                    }
                    // PrimedTnt.readAdditionalSaveData: its fuse (80 when absent) and motion.
                    "minecraft:tnt" => {
                        if let Some(position) = entity_nbt::position(&tag) {
                            let fuse = match tag.get("fuse") {
                                Some(Tag::Short(f)) => i32::from(*f),
                                _ => 80,
                            };
                            self.tnts.push(super::tnt::PrimedTnt {
                                id: 0,
                                position,
                                previous: position,
                                velocity: entity_nbt::motion(&tag),
                                fuse,
                                on_ground: matches!(tag.get("OnGround"), Some(Tag::Byte(1))),
                            });
                        }
                    }
                    k if super::boats::any_boat_wood(k)
                        .is_some_and(|(wood, chest)| self.boat_models.has(wood, chest)) =>
                    {
                        if let Some((position, velocity, yaw)) = entity_nbt::boat_from_nbt(&tag) {
                            let mut tag = tag;
                            let riders = self.take_mob_passengers(&mut tag);
                            self.next_entity += 1;
                            let id = self.next_entity;
                            let (wood, chest) =
                                super::boats::any_boat_wood(k).unwrap_or(("oak", false));
                            let mut b = super::boats::Boat::new(
                                id,
                                std::sync::Arc::from(wood),
                                position,
                                yaw,
                            );
                            if chest {
                                b = b.with_chest();
                                b.items = horizon_client::save::decode_items(
                                    tag.get("Items"),
                                    super::boats::CHEST_SLOTS,
                                );
                            }
                            b.velocity = velocity;
                            self.remember(id, uuid, tag);
                            self.boats.push(b);
                            for rider in riders {
                                self.load_mob(rider, Some(id));
                            }
                        }
                    }
                    k if super::minecarts::CartKind::from_id(k).is_some() => {
                        let kind = super::minecarts::CartKind::from_id(k).unwrap();
                        let id = self.next_entity + 1;
                        if let Some(mut c) = super::minecarts::Minecart::from_nbt(&tag, id, kind) {
                            self.roll_cart_loot(&mut c, &tag);
                            self.next_entity = id;
                            self.remember(id, uuid, tag);
                            self.carts.push(c);
                        }
                    }
                    "minecraft:ominous_item_spawner" => self.load_ominous_item_spawner(tag),
                    "minecraft:ender_dragon" => {
                        let seed = self.world.random.next_long();
                        if self.dragon.is_none()
                            && let Some(mut d) = super::dragon::Dragon::from_nbt(&tag, seed)
                        {
                            // EnderDragon's constructor takes its id and its
                            // parts' from the entity counter.
                            d.id = self.next_entity + 1;
                            self.next_entity += 9;
                            self.dragon = Some(d);
                        }
                    }
                    "minecraft:end_crystal" => {
                        let time = self.world.random.next_int_bound(100_000).unwrap_or(0);
                        if let Some(mut c) = super::dragon::Crystal::from_nbt(&tag, time) {
                            // EndCrystal's constructor takes an id from the
                            // entity counter.
                            self.next_entity += 1;
                            c.id = self.next_entity;
                            self.crystals.push(c);
                        }
                    }
                    "minecraft:armor_stand" => {
                        self.next_entity += 1;
                        if let Some(a) = entity_nbt::armor_stand_from_nbt(&tag, self.next_entity) {
                            self.remember(a.id, uuid, tag);
                            self.armor_stands.push(a);
                        }
                    }
                    "minecraft:item_frame" | "minecraft:glow_item_frame" => {
                        self.next_entity += 1;
                        if let Some(f) = entity_nbt::item_frame_from_nbt(&tag, self.next_entity) {
                            self.remember(f.id, uuid, tag);
                            self.item_frames.push(f);
                        }
                    }
                    "minecraft:cushion" => {
                        self.next_entity += 1;
                        if let Some(c) = entity_nbt::cushion_from_nbt(&tag, self.next_entity) {
                            self.remember(c.id, uuid, tag);
                            self.cushions.push(c);
                        }
                    }
                    "minecraft:painting" => {
                        if let Some((pos, facing, variant)) = entity_nbt::painting_from_nbt(&tag)
                            && let Some(v) = self.painting_variants.get(&variant).cloned()
                        {
                            self.next_entity += 1;
                            let id = self.next_entity;
                            self.remember(id, uuid, tag);
                            self.paintings.push(horizon_client::entities::Painting {
                                id,
                                pos,
                                facing,
                                variant: v.id,
                                width: v.width,
                                height: v.height,
                                check_interval: 0,
                            });
                        } else {
                            self.persist.unknown.entry(key).or_default().push(tag);
                        }
                    }
                    _ if self.res.mob_types().contains_key(&kind) => {
                        self.load_mob(tag, None);
                    }
                    _ if !kind.is_empty() && entity_nbt::position(&tag).is_some() => {
                        self.load_other(tag, &kind);
                    }
                    _ => self.persist.unknown.entry(key).or_default().push(tag),
                }
            }
        }
    }
    /// A saved mob and, under `Passengers`, the mobs riding it.
    pub(super) fn load_mob(&mut self, tag: Tag, mount: Option<u32>) -> Option<u32> {
        self.next_entity += 1;
        let uuid = entity_nbt::uuid(&tag);
        // Structure mobs that generation left to finish: saves from before
        // they were finalized at placement, or a placement that could not.
        let mut tag = tag;
        let mut finalize = false;
        if let Tag::Compound(c) = &mut tag {
            finalize = c.remove(&"horizon:finalize".into()).is_some();
        }
        let riders = self.take_mob_passengers(&mut tag);
        let mut m = entity_nbt::mob_from_nbt(&tag, self.res.mob_types(), self.next_entity)?;
        // `NeutralMob.readPersistentAngerSaveData`: the anger runs until its
        // end time, and the mob is angry at the target again.
        m.extra.anger = horizon_client::mobs::anger_left(m.extra.anger_end, self.world.game_time);
        if m.extra.anger > 0 && m.extra.angry_at.is_some() {
            m.ai.target = Some(horizon_client::mobs::Who::Player);
        }
        m.extra.mount = mount;
        m.extra.passenger = mount.is_some();
        if finalize {
            let facts = self.biome_facts(
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let at = [m.position.x, m.position.y, m.position.z];
            let difficulty = self.local_difficulty(at);
            horizon_client::mobs::spawning::finalize(&mut m, &facts, &difficulty);
            // Cat.finalizeSpawn: black in #cats_spawn_as_black.
            if m.species() == horizon_client::mobs::Species::Cat {
                let p = [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ];
                let black = self.structure_tag("minecraft:cats_spawn_as_black");
                if self.structures_at(p).iter().any(|s| black.contains(s)) {
                    m.extra.variant = 10;
                    m.persistent = true;
                }
            }
        }
        let id = m.id;
        self.remember(id, uuid, tag);
        self.mobs.push(m);
        if finalize {
            self.enchant_spawn_equipment(id);
        }
        for rider in riders {
            if let Some(r) = self.load_mob(rider, Some(id))
                && let Some(v) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                v.extra.rider = Some(r);
            }
        }
        Some(id)
    }
    /// The mobs under an entity's `Passengers`, taken out of it; the
    /// passengers Horizon doesn't simulate stay.
    fn take_mob_passengers(&self, tag: &mut Tag) -> Vec<Tag> {
        let mut riders = Vec::new();
        if let Tag::Compound(c) = tag
            && let Some(Tag::List(list)) = c.remove(&"Passengers".into())
        {
            let mut unknown = Vec::new();
            for p in list {
                let known =
                    entity_nbt::kind(&p).is_some_and(|k| self.res.mob_types().contains_key(&k));
                if known {
                    riders.push(p);
                } else {
                    unknown.push(p);
                }
            }
            if !unknown.is_empty() {
                c.insert("Passengers".into(), Tag::List(unknown));
            }
        }
        riders
    }
    /// The mobs riding entity `id` under `tag`'s `Passengers`, after the
    /// passengers it kept from loading.
    fn put_mob_passengers(&mut self, tag: &mut Tag, id: u32) {
        let riders: Vec<usize> = (0..self.mobs.len())
            .filter(|&j| self.mobs[j].extra.mount == Some(id) && self.mobs[j].alive())
            .collect();
        let riders: Vec<Tag> = riders.into_iter().map(|j| self.mob_tree_nbt(j)).collect();
        if let Tag::Compound(c) = tag {
            let mut list = match c.get(&"Passengers".into()) {
                Some(Tag::List(l)) => l.clone(),
                _ => Vec::new(),
            };
            list.extend(riders);
            if list.is_empty() {
                c.remove(&"Passengers".into());
            } else {
                c.insert("Passengers".into(), Tag::List(list));
            }
        }
    }
    /// An entity of a type Horizon does not simulate, kept as its compound.
    fn load_other(&mut self, mut tag: Tag, kind: &str) -> Option<u32> {
        strip_finalize(&mut tag);
        let position = entity_nbt::position(&tag)?;
        let rotation = match tag.get("Rotation") {
            Some(Tag::List(l)) => {
                let f = |i: usize| match l.get(i) {
                    Some(Tag::Float(v)) => f32::from_bits(*v),
                    _ => 0.0,
                };
                [f(0), f(1)]
            }
            _ => [0.0, 0.0],
        };
        self.next_entity += 1;
        let id = self.next_entity;
        let uuid = entity_nbt::uuid(&tag);
        self.remember(id, uuid, tag);
        self.others.push(super::commands::OtherEntity {
            id,
            kind: kind.to_owned(),
            position,
            yaw: rotation[0],
            pitch: rotation[1],
        });
        Some(id)
    }
    /// The compound of an entity Horizon does not simulate, with its current
    /// position, rotation and UUID.
    pub(super) fn other_to_nbt(&self, o: &super::commands::OtherEntity, uuid: [i32; 4]) -> Tag {
        let mut tag = self
            .persist
            .raw
            .get(&o.id)
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Default::default()));
        if let Tag::Compound(c) = &mut tag {
            c.insert("id".into(), Tag::String(o.kind.as_str().into()));
            c.insert(
                "Pos".into(),
                Tag::List(
                    [o.position.x, o.position.y, o.position.z]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
            c.insert(
                "Rotation".into(),
                Tag::List(vec![
                    Tag::Float(o.yaw.to_bits()),
                    Tag::Float(o.pitch.to_bits()),
                ]),
            );
            c.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        }
        tag
    }
    /// The root of a stack of riders (`Entity.getRootVehicle`).
    fn mob_root(&self, mut i: usize) -> usize {
        for _ in 0..8 {
            let Some(v) = self.mobs[i].extra.mount else {
                break;
            };
            let Some(j) = self.mobs.iter().position(|m| m.id == v) else {
                break;
            };
            i = j;
        }
        i
    }
    /// A mob's entity tag with the mobs riding it under `Passengers`.
    pub(super) fn mob_tree_nbt(&mut self, i: usize) -> Tag {
        let id = self.mobs[i].id;
        let uuid = self.uuid_for(id);
        // `NeutralMob.addPersistentAngerSaveData`: the anger ends at an
        // absolute game time, so it goes on while the chunk is unloaded, and
        // the target is the entity the mob is angry at.
        let anger = self.mobs[i].extra.anger;
        self.mobs[i].extra.anger_end = if anger > 0 {
            self.world.game_time + i64::from(anger)
        } else {
            -1
        };
        self.mobs[i].extra.angry_at = if anger > 0 {
            // A wolf keeps its anger's target itself, also once the target
            // is gone (`tame::update_persistent_anger`).
            if self.mobs[i].species() == horizon_client::mobs::Species::Wolf {
                self.mobs[i].extra.angry_at
            } else {
                self.mobs[i].ai.target
            }
        } else {
            None
        };
        let mut tag = entity_nbt::mob_to_nbt(&self.mobs[i], uuid, self.persist.raw.get(&id));
        self.write_leash(&mut tag, i);
        self.put_mob_passengers(&mut tag, id);
        tag
    }
    /// Forget an entity that left the world (its UUID and loaded compound).
    pub(super) fn forget_entity(&mut self, id: u32) {
        self.persist.uuids.remove(&id);
        self.persist.raw.remove(&id);
    }
    /// The compound an entity without a Horizon entity ID was loaded with
    /// (the crystals and the dragon), found again by type and position.
    pub(super) fn persist_anon(&self, kind: &str, p: Vec3) -> (Option<[i32; 4]>, Option<&Tag>) {
        self.persist
            .anon
            .get(&anon_key(kind, p))
            .map_or((None, None), |(_, uuid, raw)| (*uuid, Some(raw)))
    }
    /// The compound an entity was loaded with (and commands have edited).
    pub(super) fn persist_raw(&self, id: u32) -> Option<&Tag> {
        self.persist.raw.get(&id)
    }
    /// A loaded chest minecart with a loot table and no items: the table
    /// rolled now, as generated containers are (Horizon does not unpack on
    /// first open), and kept to save it again while unchanged.
    fn roll_cart_loot(&self, cart: &mut super::minecarts::Minecart, tag: &Tag) {
        let Some(table) = horizon_client::save::text(tag.get("LootTable")) else {
            return;
        };
        if cart.items.is_empty() || cart.items.iter().any(Option::is_some) {
            return;
        }
        let loot = horizon_client::structure_blocks::loot_data(&self.options.resources);
        let p = cart.position;
        let pos = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let biome = self.biome_name_at(p).unwrap_or_default();
        let seed = horizon_client::structure_blocks::loot_seed(tag);
        cart.loot = Some(horizon_client::structure_blocks::roll_container(
            &mut cart.items,
            loot.as_deref(),
            table,
            seed,
            pos,
            &biome,
        ));
    }
    fn remember(&mut self, id: u32, uuid: Option<[i32; 4]>, tag: Tag) {
        self.restore_portal_cooldown(id, &tag);
        if let Some(u) = uuid {
            self.persist.uuids.insert(id, u);
        }
        self.persist.raw.insert(id, tag);
    }
    /// The mob that was loaded or saved with a UUID.
    pub(super) fn mob_by_uuid(&self, uuid: [i32; 4]) -> Option<u32> {
        self.persist
            .uuids
            .iter()
            .find(|(id, u)| **u == uuid && self.mobs.iter().any(|m| m.id == **id))
            .map(|(id, _)| *id)
    }
    /// A mob's UUID, made up for new ones.
    pub(super) fn mob_uuid(&mut self, id: u32) -> [i32; 4] {
        self.uuid_for(id)
    }
    pub(super) fn uuid_for(&mut self, id: u32) -> [i32; 4] {
        *self.persist.uuids.entry(id).or_insert_with(random_uuid)
    }
    /// `BlockState.CODEC`: a block name or `{id, properties}` (or legacy `{Name, Properties}`).
    fn state_from_nbt(&self, tag: &Tag) -> Option<BlockStateId> {
        let registry = self.pack.assets.registry;
        let state = match tag.get("Name") {
            Some(Tag::String(name)) => BlockState {
                name: name.to_string_lossy(),
                properties: match tag.get("Properties") {
                    Some(Tag::Compound(p)) => p
                        .iter()
                        .filter_map(|(k, v)| match v {
                            Tag::String(v) => Some((k.to_string_lossy(), v.to_string_lossy())),
                            _ => None,
                        })
                        .collect(),
                    _ => Default::default(),
                },
            },
            _ => BlockState::from_nbt(tag).ok()?,
        };
        registry
            .id(&state)
            .or_else(|| registry.default_state(&state.name))
    }
    pub(in crate::game) fn state_to_nbt(&self, state: BlockStateId) -> Tag {
        let registry = self.pack.assets.registry;
        match registry.state(state) {
            Some(s) if registry.default_state(&s.name) == Some(state) => {
                Tag::String(s.name.as_str().into())
            }
            Some(s) => s.to_nbt(),
            None => Tag::String("minecraft:air".into()),
        }
    }
    /// The entity NBT of every simulated entity in `keys`, plus the unknown
    /// entities kept for them. With `remove`, the entities leave the world
    /// (their chunks unload).
    pub(super) fn entity_chunks(
        &mut self,
        keys: &HashSet<(i32, i32)>,
        remove: bool,
    ) -> Vec<((i32, i32), Vec<Tag>)> {
        let mut out: HashMap<(i32, i32), Vec<Tag>> =
            keys.iter().map(|k| (*k, Vec::new())).collect();
        let player_uuid = self.level.player_uuid;
        // A rider is saved inside its root vehicle, in the root's chunk; a
        // boat's passengers inside the boat.
        let mobs: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| {
                self.mob_root(i) == i
                    && self.mobs[i]
                        .extra
                        .mount
                        .is_none_or(|v| self.boats.iter().all(|b| b.id != v))
                    && keys.contains(&chunk_of(self.mobs[i].position))
                    && self.mobs[i].alive()
            })
            .collect();
        for i in mobs {
            let tag = self.mob_tree_nbt(i);
            out.entry(chunk_of(self.mobs[i].position))
                .or_default()
                .push(tag);
        }
        let items: Vec<usize> = (0..self.dropped.len())
            .filter(|&i| keys.contains(&chunk_of(self.dropped[i].position)))
            .collect();
        for i in items {
            let id = self.dropped[i].id;
            let uuid = self.uuid_for(id);
            let tag =
                entity_nbt::item_entity_to_nbt(&self.dropped[i], uuid, self.persist.raw.get(&id));
            out.entry(chunk_of(self.dropped[i].position))
                .or_default()
                .push(tag);
        }
        let anon = |persist: &EntityMemory, kind: &str, at: Vec3| {
            persist
                .anon
                .get(&anon_key(kind, at))
                .map(|(_, uuid, raw)| (uuid.unwrap_or_else(random_uuid), Some(raw.clone())))
                .unwrap_or_else(|| (random_uuid(), None))
        };
        for o in &self.orbs {
            if keys.contains(&chunk_of(o.position)) {
                let (uuid, raw) = anon(&self.persist, "minecraft:experience_orb", o.position);
                out.entry(chunk_of(o.position))
                    .or_default()
                    .push(with_raw(entity_nbt::orb_to_nbt(o, uuid), raw.as_ref()));
            }
        }
        for p in &self.creatures.projectiles {
            if !keys.contains(&chunk_of(p.position)) {
                continue;
            }
            let owner = if p.owner == Some(Who::Player) {
                player_uuid
            } else {
                None
            };
            let (uuid, raw) = anon(&self.persist, p.kind.id(), p.position);
            let in_block = p
                .arrow
                .as_ref()
                .and_then(|a| a.last_state)
                .map(|s| self.state_to_nbt(s));
            let tag = entity_nbt::arrow_to_nbt(p, uuid, owner, in_block, raw.as_ref())
                .or_else(|| entity_nbt::fireball_to_nbt(p, uuid, owner, raw.as_ref()));
            if let Some(tag) = tag {
                out.entry(chunk_of(p.position)).or_default().push(tag);
            }
        }
        for f in &self.falling {
            if keys.contains(&chunk_of(f.position)) {
                let (uuid, raw) = match &f.saved {
                    Some(saved) => (saved.0, Some(saved.1.clone())),
                    None => anon(&self.persist, "minecraft:falling_block", f.position),
                };
                let tag = entity_nbt::falling_block_to_nbt(
                    self.state_to_nbt(f.state),
                    f.position,
                    f.velocity,
                    f.age,
                    f.drop_item,
                    f.damage,
                    f.fall_distance,
                    uuid,
                );
                out.entry(chunk_of(f.position))
                    .or_default()
                    .push(with_raw(tag, raw.as_ref()));
            }
        }
        let boats: Vec<usize> = (0..self.boats.len())
            .filter(|&i| keys.contains(&chunk_of(self.boats[i].position)))
            .collect();
        for i in boats {
            let b = &self.boats[i];
            let (id, at) = (b.id, b.position);
            let uuid = self
                .persist
                .uuids
                .get(&id)
                .copied()
                .unwrap_or_else(random_uuid);
            let mut tag = entity_nbt::boat_to_nbt(
                &b.entity_type(),
                b.position,
                b.velocity,
                b.yaw,
                uuid,
                self.persist.raw.get(&id),
                b.chest.then_some(&b.items[..]),
            );
            self.put_mob_passengers(&mut tag, id);
            out.entry(chunk_of(at)).or_default().push(tag);
        }
        if let Some(d) = &self.dragon
            && keys.contains(&chunk_of(d.position))
        {
            let (uuid, raw) = anon(&self.persist, "minecraft:ender_dragon", d.position);
            out.entry(chunk_of(d.position))
                .or_default()
                .push(d.to_nbt(uuid, raw.as_ref()));
        }
        for c in &self.crystals {
            if keys.contains(&chunk_of(c.position)) {
                let (uuid, raw) = anon(&self.persist, "minecraft:end_crystal", c.position);
                out.entry(chunk_of(c.position))
                    .or_default()
                    .push(c.to_nbt(uuid, raw.as_ref()));
            }
        }
        for s in &self.ominous_spawners {
            let at = Vec3::new(s.position[0], s.position[1], s.position[2]);
            if keys.contains(&chunk_of(at)) {
                let tag = entity_nbt::ominous_item_spawner_to_nbt(
                    &s.item,
                    at,
                    s.spawn_item_after,
                    s.uuid,
                    s.raw.as_deref(),
                );
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for c in &self.carts {
            if keys.contains(&chunk_of(c.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&c.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = c.to_nbt(uuid, self.persist.raw.get(&c.id));
                out.entry(chunk_of(c.position)).or_default().push(tag);
            }
        }
        for a in &self.armor_stands {
            if keys.contains(&chunk_of(a.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&a.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::armor_stand_to_nbt(a, uuid, self.persist.raw.get(&a.id));
                out.entry(chunk_of(a.position)).or_default().push(tag);
            }
        }
        for f in &self.item_frames {
            let (lo, hi) = f.bounds();
            let at = Vec3::new(
                (lo[0] + hi[0]) / 2.0,
                (lo[1] + hi[1]) / 2.0,
                (lo[2] + hi[2]) / 2.0,
            );
            if keys.contains(&chunk_of(at)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&f.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::item_frame_to_nbt(f, uuid, self.persist.raw.get(&f.id));
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for c in &self.cushions {
            if keys.contains(&chunk_of(c.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&c.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::cushion_to_nbt(c, uuid, self.persist.raw.get(&c.id));
                out.entry(chunk_of(c.position)).or_default().push(tag);
            }
        }
        for p in &self.paintings {
            let at = painting_center(p);
            if keys.contains(&chunk_of(at)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&p.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = entity_nbt::painting_to_nbt(p, uuid, self.persist.raw.get(&p.id));
                out.entry(chunk_of(at)).or_default().push(tag);
            }
        }
        for o in &self.others {
            if keys.contains(&chunk_of(o.position)) {
                let uuid = self
                    .persist
                    .uuids
                    .get(&o.id)
                    .copied()
                    .unwrap_or_else(random_uuid);
                let tag = self.other_to_nbt(o, uuid);
                out.entry(chunk_of(o.position)).or_default().push(tag);
            }
        }
        for key in keys {
            let unknown = if remove {
                self.persist.unknown.remove(key)
            } else {
                self.persist.unknown.get(key).cloned()
            };
            if let Some(list) = unknown {
                out.entry(*key).or_default().extend(list);
            }
        }
        if remove {
            let gone = |p: Vec3| keys.contains(&chunk_of(p));
            let mut removed: Vec<u32> = Vec::new();
            let riding = self.riding;
            self.boats.retain(|b| {
                let keep = !gone(b.position) || Some(b.id) == riding;
                if !keep {
                    removed.push(b.id);
                }
                keep
            });
            self.crystals.retain(|c| !gone(c.position));
            self.ominous_spawners
                .retain(|s| !gone(Vec3::new(s.position[0], s.position[1], s.position[2])));
            if self.dragon.as_ref().is_some_and(|d| gone(d.position)) {
                self.dragon = None;
            }
            self.carts.retain(|c| {
                let keep = !gone(c.position) || Some(c.id) == riding;
                if !keep {
                    removed.push(c.id);
                }
                keep
            });
            self.others.retain(|o| {
                let keep = !gone(o.position);
                if !keep {
                    removed.push(o.id);
                }
                keep
            });
            self.armor_stands.retain(|a| {
                let keep = !gone(a.position);
                if !keep {
                    removed.push(a.id);
                }
                keep
            });
            self.item_frames.retain(|f| {
                let (lo, hi) = f.bounds();
                let keep = !gone(Vec3::new(
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ));
                if !keep {
                    removed.push(f.id);
                }
                keep
            });
            self.paintings.retain(|p| {
                let keep = !gone(painting_center(p));
                if !keep {
                    removed.push(p.id);
                }
                keep
            });
            self.cushions.retain(|c| {
                let keep = !gone(c.position);
                if !keep {
                    removed.push(c.id);
                }
                keep
            });
            // Riders leave with their root vehicle, a boat's with the boat.
            let leaving: HashSet<u32> = (0..self.mobs.len())
                .filter(|&i| {
                    let r = self.mob_root(i);
                    match self.mobs[r].extra.mount {
                        Some(v) if removed.contains(&v) => true,
                        Some(v) if self.boats.iter().any(|b| b.id == v) => false,
                        _ => gone(self.mobs[r].position),
                    }
                })
                .map(|i| self.mobs[i].id)
                .collect();
            self.mobs.retain(|m| {
                let keep = !leaving.contains(&m.id);
                if !keep {
                    removed.push(m.id);
                }
                keep
            });
            self.dropped.retain(|e| {
                let keep = !gone(e.position);
                if !keep {
                    removed.push(e.id);
                }
                keep
            });
            self.orbs.retain(|o| !gone(o.position));
            self.creatures
                .projectiles
                .retain(|p| p.arrow.is_none() || !gone(p.position));
            self.falling.retain(|f| !gone(f.position));
            self.persist
                .anon
                .retain(|_, (at, _, _)| !keys.contains(&chunk_of(*at)));
            for id in removed {
                self.persist.uuids.remove(&id);
                self.persist.raw.remove(&id);
            }
        }
        out.into_iter().collect()
    }
    /// Write the entities of every resident chunk (an autosave or quit).
    /// Without `wait` (the autosave) they are written on the region writer
    /// meanwhile: `PersistentEntitySectionManager.autoSave` stores them
    /// without `saveAll`'s flush.
    pub(super) fn save_entities(&mut self, wait: bool) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, false);
        self.prune_entity_memory();
        if wait {
            self.save.save_entities(chunks)
        } else {
            self.save.save_entities_later(chunks)
        }
    }
    pub(super) fn prune_entity_memory(&mut self) {
        // Forget entities that no longer exist.
        let alive: HashSet<u32> = self
            .mobs
            .iter()
            .map(|m| m.id)
            .chain(self.dropped.iter().map(|e| e.id))
            .chain(self.boats.iter().map(|e| e.id))
            .chain(self.carts.iter().map(|e| e.id))
            .chain(self.armor_stands.iter().map(|e| e.id))
            .chain(self.cushions.iter().map(|e| e.id))
            .chain(self.item_frames.iter().map(|e| e.id))
            .chain(self.paintings.iter().map(|e| e.id))
            .chain(self.others.iter().map(|e| e.id))
            .collect();
        self.persist.uuids.retain(|id, _| alive.contains(id));
        self.persist.raw.retain(|id, _| alive.contains(id));
    }
    /// Save and remove the entities of chunks that unloaded.
    pub(super) fn unload_entities(&mut self, unloaded: &[Arc<Chunk>]) {
        if unloaded.is_empty() {
            return;
        }
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = unloaded.iter().map(|c| (c.x, c.z)).collect();
        let chunks = self.entity_chunks(&keys, true);
        if let Err(e) = self.save.save_entities_later(chunks) {
            log::warn!("Horizon: failed to save entities: {e}");
        }
    }
    /// Leaving a dimension: its entities are saved with it.
    pub(super) fn save_dimension_entities(&mut self) -> Result<()> {
        self.spawn_loaded_entities();
        let keys: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        let chunks = self.entity_chunks(&keys, true);
        self.persist = EntityMemory::default();
        self.save.save_entities(chunks)
    }
}
/// Only a mob is finalized; another entity drops the request.
fn strip_finalize(tag: &mut Tag) {
    if let Tag::Compound(c) = tag {
        c.remove(&"horizon:finalize".into());
    }
}
