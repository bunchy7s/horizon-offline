//! Shader packs: the one picked on Options > Horizon Settings > Shader Packs, or
//! `HORIZON_SHADER_PACK=<name>` (development; it wins over the pick), runs `shaderpacks/<name>`
//! (a folder or a zip beside `options.txt`) over the world; Horizon's own packs go to
//! `game/native_shaders.rs`. The programs of the current dimension's folder are loaded when the
//! dimension changes (spec 1.2).

use horizon_client::camera::CameraFluid;
use horizon_client::gfx::pack_uniforms::{PackInputs, PlayerStatus};
use horizon_client::shader_options::ShaderOptions;
use horizon_client::text::Text;
use horizon_shaders::{BlockIds, Macros, Pack, Properties, dimension_folder, load_post};

use super::{Dim, Game};
use horizon_client::gfx::pack_draw::EntitySpan;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// The values the player chose for pack `name`'s options (`shaderpacks/<name>.txt`).
fn chosen_shader_options(name: &str) -> BTreeMap<String, String> {
    let file = Game::shader_packs_dir().join(horizon_shaders::chosen_file(name));
    std::fs::read_to_string(file)
        .map(|text| horizon_shaders::parse_chosen(&text))
        .unwrap_or_default()
}

/// Pack `name`'s options, screens and names in `language`, with the values the player chose.
pub(super) fn read_shader_options(
    name: &str,
    language: &str,
    macros: &Macros,
) -> Option<ShaderOptions> {
    let pack = match Pack::open(&Game::shader_packs_dir().join(name)) {
        Ok(pack) => pack,
        Err(e) => {
            log::warn!("shader pack {name}: {e}");
            return None;
        }
    };
    let options = horizon_shaders::options(&pack);
    let lang = horizon_shaders::Lang::load(&pack, language);
    let chosen = chosen_shader_options(name);
    let pack = pack.with_chosen(chosen.clone());
    let text = pack.read_text("shaders.properties").unwrap_or_default();
    let properties = Properties::parse(&text, &horizon_shaders::with_options(&pack, macros));
    let screens = horizon_shaders::Screens::parse(&properties, &options);
    Some(ShaderOptions::new(
        name.to_owned(),
        options,
        screens,
        lang,
        chosen,
    ))
}

/// Saves the values chosen on the Shader Options page; none chosen removes the file.
pub(super) fn write_shader_options(options: &ShaderOptions) {
    let file = Game::shader_packs_dir().join(horizon_shaders::chosen_file(&options.pack));
    let saved = if options.chosen.is_empty() {
        std::fs::remove_file(&file).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(&file, horizon_shaders::chosen_text(&options.chosen))
    };
    if let Err(e) = saved {
        log::warn!("shader pack {}: {}: {e}", options.pack, file.display());
    }
}

/// The pack that runs over the world ("": none): `HORIZON_SHADER_PACK`, else the pick
/// (`active_pack`).
fn picked_pack(settings: &horizon_client::settings::Settings) -> &str {
    static DEV_PACK: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    let dev = DEV_PACK.get_or_init(|| std::env::var("HORIZON_SHADER_PACK").ok());
    active_pack(
        dev.as_deref().unwrap_or(&settings.horizon.shader_pack),
        &settings.mods,
    )
}

/// Whether the pack picked (`picked_pack`) draws flat where the device can (`pack_draws_flat`):
/// a renderer made now lays the sections out for it before the first world frame loads it
/// (`Renderer::new`).
pub(super) fn picked_pack_draws_flat(settings: &horizon_client::settings::Settings) -> bool {
    let picked = picked_pack(settings);
    !picked.is_empty() && pack_draws_flat(&Game::shader_packs_dir().join(picked))
}

/// Whether the pack at `path` has passes that draw the terrain flat where the device can,
/// reading translucent quads beside their section's solid ones: a `shaders/` pack, taken to
/// draw terrain, or one of Horizon's own with a `[shadow]` pass.
fn pack_draws_flat(path: &std::path::Path) -> bool {
    !horizon_shader_api::package::is_pack(path)
        || horizon_shader_api::Pack::read(path).is_ok_and(|p| p.manifest.shadow.is_some())
}

// A native mod can bundle a pack as `native-<mod-id>`; its Mods toggle controls it too.
fn active_pack<'a>(picked: &'a str, mods: &horizon_client::mod_list::ModList) -> &'a str {
    if picked.strip_prefix("native-").is_some_and(|id| {
        mods.installed
            .iter()
            .any(|m| m.id == id && m.native.is_some() && mods.disabled.contains(id))
    }) {
        ""
    } else {
        picked
    }
}

/// Whether the pack `settings` pick (or `HORIZON_SHADER_PACK`) is one of Horizon's own: only
/// then do resource loads read the resource packs' LabPBR maps (`sprites::set_labpbr_maps`).
pub(super) fn picks_native_pack(settings: &horizon_client::settings::Settings) -> bool {
    let dev = std::env::var("HORIZON_SHADER_PACK").ok();
    let picked = active_pack(
        dev.as_deref().unwrap_or(&settings.horizon.shader_pack),
        &settings.mods,
    );
    !picked.is_empty()
        && horizon_shader_api::package::is_pack(&Game::shader_packs_dir().join(picked))
}

/// The ids a pack gives entities and block entities (4.1, 5.3), for the spans of the entity
/// geometry.
pub(super) struct PackIds {
    entities: HashMap<String, i32>,
    blocks: BlockIds,
    /// `blockEntityId` by block state, filled as states are drawn.
    block_states: RefCell<HashMap<u32, i32>>,
    /// The id of a name the file lacks: 65535, or 0 without the file.
    missing_entity: i32,
    missing_block: i32,
    /// `item.properties` (4.1) and the id of a held item the file lacks, or of an empty hand:
    /// −1, or 0 without the file (5.3).
    items: HashMap<String, i32>,
    missing_item: i32,
    /// `oldHandLight` (3.2): the first held item and light are those of the brighter hand.
    old_hand_light: bool,
    /// `vignette`: `false` hides the game's vignette over the pack's image.
    vignette: bool,
    /// `oldLighting`: `true` keeps the game's face shading in the block colours.
    old_lighting: bool,
}

impl Game<'_> {
    /// Loads the pack for the current dimension when it changed, and returns this frame's
    /// inputs to the pack's uniforms.
    pub(super) fn shader_pack_inputs(
        &mut self,
        camera: glam::DVec3,
        partial: f32,
        sky: &horizon_client::environment::Sky,
    ) -> PackInputs {
        // A `shaders/` pack lights the faces itself unless `oldLighting=true`: sections mesh
        // without the game's face shading (the pack loaded or dropped below shows next frame).
        let pack_lighting = self.pack_ids.as_ref().is_some_and(|ids| !ids.old_lighting);
        if self.streaming.shader_pack_lighting != pack_lighting {
            self.streaming.shader_pack_lighting = pack_lighting;
            self.all_sections_changed();
        }
        let picked = picked_pack(&self.settings).to_owned();
        if self.native_shaders.picked != picked {
            // Another pack, or none: drop what ran.
            self.native_shaders = super::native_shaders::NativeShaders::picking(&picked);
            self.set_translucent_layers(Vec::new());
            self.pack_ids = None;
            if let Some(renderer) = &mut self.renderer {
                renderer.drop_shader_packs();
            }
        }
        if picked.is_empty() {
            return PackInputs::default();
        }
        let name = picked;
        let dimension = match self.dim {
            Dim::Overworld => 0,
            Dim::Nether => -1,
            Dim::End => 1,
        };
        let path = Self::shader_packs_dir().join(&name);
        if horizon_shader_api::package::is_pack(&path) {
            // One of Horizon's own packs (game/native_shaders.rs).
            self.tick_native_shader_pack(&path);
        } else if let Some(renderer) = &mut self.renderer
            && renderer.shader_pack_dimension() != Some(dimension)
        {
            let name = name.as_str();
            let started = std::time::Instant::now();
            // A pack is not the game's own code: one that makes the shader compiler (or our
            // translation of it) panic is turned off instead of crashing every world load (#3525).
            let loaded = horizon_client::crash::contain(|| {
                match Pack::open(&path).map(|p| p.with_chosen(chosen_shader_options(name))) {
                    Ok(pack) => {
                        let mut macros = Macros::standard();
                        renderer.define_pack_macros(&mut macros);
                        let text = pack.read_text("shaders.properties").unwrap_or_default();
                        // Its conditionals test the pack's options too.
                        let properties = Properties::parse(
                            &text,
                            &horizon_shaders::with_options(&pack, &macros),
                        );
                        for note in macros.define_features(&properties) {
                            log::warn!("shader pack {name}: {note}");
                        }
                        for note in &properties.notes {
                            log::warn!(
                                "shader pack {name}: shaders.properties:{}: {}",
                                note.line,
                                note.message
                            );
                        }
                        let folder = dimension_folder(&pack, dimension);
                        let inputs = renderer.pack_inputs();
                        let mut post = load_post(&pack, &folder, &properties, &macros, &inputs);
                        let biomes = world_biomes!(self);
                        let names: Vec<&str> =
                            biomes.list.iter().map(|b| b.name.as_str()).collect();
                        post.custom.set_biomes(&names);
                        let text = pack.read_text("block.properties").unwrap_or_default();
                        let ids = BlockIds::parse(&Properties::parse(&text, &macros));
                        for note in &ids.notes {
                            log::warn!("shader pack {name}: block.properties: {note}");
                        }
                        let text = pack.read_text("entity.properties");
                        let entities = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "entity",
                        );
                        let text = pack.read_text("item.properties");
                        let items = BlockIds::parse_kind(
                            &Properties::parse(text.as_deref().unwrap_or_default(), &macros),
                            "item",
                        );
                        self.pack_ids = Some(PackIds {
                            items: items.plain(),
                            missing_item: if text.is_some() { -1 } else { 0 },
                            // OQ-21: on unless the pack turns it off.
                            old_hand_light: properties
                                .get("oldHandLight")
                                .is_none_or(|v| v.trim() != "false"),
                            // On unless the pack turns it off (`vignette=false`).
                            vignette: properties
                                .get("vignette")
                                .is_none_or(|v| v.trim() != "false"),
                            // Off unless the pack turns it on (Iris shades no faces by default).
                            old_lighting: properties
                                .get("oldLighting")
                                .is_some_and(|v| v.trim() == "true"),
                            entities: entities.plain(),
                            missing_entity: if text.is_some() { 65535 } else { 0 },
                            missing_block: if ids.is_empty() { 0 } else { 65535 },
                            blocks: ids.clone(),
                            block_states: RefCell::default(),
                        });
                        let started_ids = std::time::Instant::now();
                        let table = block_id_table(&self.pack, &ids);
                        log::info!(
                            "shader pack {name}: mc_Entity for {} of {} block sprites in {} ms",
                            table.chunks(2).filter(|p| p[0] != -1).count(),
                            table.len() / 2,
                            started_ids.elapsed().as_millis()
                        );
                        if let Some((programs, notes)) =
                            renderer.set_shader_pack(dimension, Some(post), &table)
                        {
                            for note in notes {
                                log::warn!("shader pack {name}: {note}");
                            }
                            log::info!(
                                "shader pack {name}: folder `{folder}`, {} programs ({}) in {} ms",
                                programs.len(),
                                programs.join(" "),
                                started.elapsed().as_millis()
                            );
                        }
                    }
                    Err(e) => {
                        self.pack_ids = None;
                        log::warn!("shader pack {name}: {e}");
                        renderer.set_shader_pack(dimension, None, &[]);
                    }
                }
            });
            if let Err(panic) = loaded {
                self.shader_pack_failed(name, dimension, &panic);
            }
        }
        let lerp = |(old, new): (f32, f32)| old + (new - old) * partial;
        let at = camera.floor().as_ivec3();
        let light = self.world.light(at.x, at.y, at.z);
        let biome = self
            .world
            .chunks
            .get(&(at.x >> 4, at.z >> 4))
            .map(|c| c.biome(at.x, at.y, at.z));
        let (held_items, held_light) = self.held_for_pack();
        let status = self.pack_player_status();
        let (end_flash, end_flash_direction) = self.end_flash_for_pack(partial);
        let info = biome.map(|b| world_biomes!(self).get(b));
        PackInputs {
            day_time: self.world.day_time,
            rain: lerp(self.world.weather.rain_level),
            thunder: lerp(self.world.weather.thunder_level),
            eye_in: match self.camera_fluid(camera.to_array()) {
                CameraFluid::None => 0,
                CameraFluid::Water => 1,
                CameraFluid::Lava => 2,
                CameraFluid::PowderSnow => 3,
            },
            blindness: f32::from(u8::from(self.potions.effects.get("blindness").is_some())),
            eye_light: [i32::from(light & 15), i32::from(light >> 4)],
            hide_gui: self.ui.hide_gui,
            brightness: self.settings.gamma,
            biome: biome.map_or(-1, i32::from),
            biome_known: biome.is_some(),
            biome_category: info.map_or(0, |b| horizon_shaders::biome_category(&b.name)),
            biome_precipitation: info.map_or(0, |b| {
                horizon_shaders::biome_precipitation(b.precipitation, b.temperature)
            }),
            temperature: info.map_or(0.0, |b| b.temperature),
            rainfall: info.map_or(0.0, |b| b.downfall),
            held_items,
            held_light,
            right_handed: !self.settings.main_hand_left,
            on_ground: self.player.as_ref().is_some_and(|p| p.ground.on_ground),
            min_y: self.world.shape.min_y,
            height: self.world.shape.height,
            logical_height: self.world.shape.logical_height,
            has_skylight: self.world.shape.sky_light,
            has_ceiling: self.dim == Dim::Nether,
            elytra_flying: self.player.as_ref().is_some_and(|p| p.fall_flying),
            cloud_height: horizon_client::gfx::pack_uniforms::cloud_height(
                sky.cloud_color[3],
                sky.cloud_height,
            ),
            mod_params: self.native_shader_params_now(partial),
            status,
            end_flash,
            end_flash_direction,
        }
    }

    /// The local player's state for Iris's status uniforms, read as the HUD reads it.
    fn pack_player_status(&mut self) -> PlayerStatus {
        let spectator = self.spectator();
        let survival = !self.creative && !spectator;
        let invisible = self.own_invisible();
        let riding = self.riding.is_some();
        let Some(p) = &mut self.player else {
            return PlayerStatus::default();
        };
        PlayerStatus {
            survival,
            health: p.health.health(),
            max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
            food: p.health.food.food as f32,
            air: p.air as f32,
            armor: p
                .attribute("minecraft:armor")
                .or_else(|_| p.attribute("armor"))
                .unwrap_or(0.0) as f32,
            spectator,
            riding,
            feet_in_water: p.fluids.was_touching_water,
            swimming: p.pose == horizon_client::pose::Pose::Swimming,
            burning: p.fire_ticks > 0 && !spectator,
            hurt: p.health.hurt_animation_time > 0,
            invisible,
            sneaking: p.pose == horizon_client::pose::Pose::Crouching,
            sprinting: p.sprinting,
        }
    }

    /// The Shader Options page opened: the picked pack's options, screens and names.
    pub(super) fn open_shader_options(&mut self) {
        let mut macros = Macros::standard();
        if let Some(renderer) = &self.renderer {
            renderer.define_pack_macros(&mut macros);
        }
        let name = &self.settings.horizon.shader_pack;
        self.settings.shader_options = read_shader_options(name, &self.settings.language, &macros);
    }

    /// The Shader Options page closed with changes: they are saved, and the pack loads again.
    pub(super) fn save_shader_options(&mut self) {
        let Some(options) = self.settings.shader_options.take() else {
            return;
        };
        write_shader_options(&options);
        self.pack_ids = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.drop_shader_packs();
        }
    }

    /// Turns off the pack `name` after loading it panicked: nothing runs in its place, it is no
    /// longer the picked pack (so the next start does not load it again), and the chat says so.
    fn shader_pack_failed(&mut self, name: &str, dimension: i32, error: &str) {
        log::error!("shader pack {name}: turned off: {error}");
        self.pack_ids = None;
        if let Some(renderer) = &mut self.renderer {
            // Marks this dimension as loaded, without a pack, so it is not tried again.
            renderer.set_shader_pack(dimension, None, &[]);
        }
        if self.settings.horizon.shader_pack == name {
            self.settings.horizon.shader_pack.clear();
            self.settings.save(&Self::options_path());
        }
        self.system_message(Text::colored(
            format!("Shader pack {name} could not be loaded and was turned off"),
            0xFF5555,
        ));
    }

    /// Whether the game's vignette draws: the option, unless the loaded pack turns it off.
    pub(super) fn vignette_shown(&self) -> bool {
        self.settings.vignette && self.pack_ids.as_ref().is_none_or(|ids| ids.vignette)
    }

    /// `heldItemId(2)` and `heldBlockLightValue(2)`: main hand, then off hand (5.2, 5.3).
    fn held_for_pack(&mut self) -> ([i32; 2], [i32; 2]) {
        if self.pack_ids.is_none() {
            return ([0; 2], [0; 2]);
        }
        let lum = self.luminance();
        let Some(ids) = &self.pack_ids else {
            return ([0; 2], [0; 2]);
        };
        let hands = [self.selected, super::shield::OFFHAND_SLOT].map(|slot| {
            let item = self.stack_in(slot).map(|s| &*s.item);
            let id = item.and_then(|i| ids.items.get(i).copied());
            (
                id.unwrap_or(ids.missing_item),
                item.map_or(0, |i| i32::from(lum.of(i))),
            )
        });
        let first = if ids.old_hand_light && hands[1].1 > hands[0].1 {
            hands[1]
        } else {
            hands[0]
        };
        ([first.0, hands[1].0], [first.1, hands[1].1])
    }
}

impl Game<'_> {
    /// Starts a span of the entity geometry at vertex `start` for entity type `name`
    /// (`minecraft:zombie`; "" for one the pack cannot name). Nothing without a pack.
    pub(super) fn mark_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, false);
    }

    /// Starts a span for entity geometry that blends (name tags, shadows).
    pub(super) fn mark_translucent_entity(&self, start: usize, name: &str) {
        self.mark_entity_part(start, name, true);
    }

    fn mark_entity_part(&self, start: usize, name: &str, translucent: bool) {
        let id = match &self.pack_ids {
            Some(ids) => ids
                .entities
                .get(name)
                .copied()
                .unwrap_or(ids.missing_entity),
            None if native_spans() => name_id(name),
            None => return,
        };
        self.push_span(start, "ENTITIES", id);
        if let Some(last) = self.entity_spans.borrow_mut().last_mut() {
            last.translucent = translucent;
        }
    }

    /// Starts a span for the block entity of block state `state`.
    pub(super) fn mark_block_entity(&self, start: usize, state: u32) {
        let Some(ids) = &self.pack_ids else {
            if native_spans() {
                self.push_span(start, "BLOCK_ENTITIES", state as i32);
            }
            return;
        };
        let id = *ids
            .block_states
            .borrow_mut()
            .entry(state)
            .or_insert_with(|| {
                let registry = self.pack.assets.registry;
                registry
                    .state(horizon_world::registry::BlockStateId(state))
                    .and_then(|s| {
                        ids.blocks
                            .id(&s.name, |k| s.properties.get(k).map(String::as_str))
                    })
                    .unwrap_or(ids.missing_block)
            });
        self.push_span(start, "BLOCK_ENTITIES", id);
        let emission = self
            .res
            .facts()
            .get(horizon_world::registry::BlockStateId(state))
            .emission;
        if let Some(last) = self.entity_spans.borrow_mut().last_mut() {
            last.emissive = emission > 0;
        }
    }

    /// Starts the span of the local player's model; `hidden` in first person, where only a
    /// pack's shadow pass draws it.
    pub(super) fn mark_player(&self, start: usize, hidden: bool) {
        self.mark_entity(start, "minecraft:player");
        if (self.pack_ids.is_some() || native_spans())
            && let Some(last) = self.entity_spans.borrow_mut().last_mut()
        {
            last.player = true;
            last.hidden = hidden;
        }
    }

    /// Marks vertices `start..end` (one entity) as hidden from the camera, for an entity
    /// Horizon's culling would drop but a pack's shadow pass still draws (`EntitySpan::hide`).
    pub(super) fn hide_spans_from(&self, start: usize, end: usize) -> bool {
        EntitySpan::hide(&mut self.entity_spans.borrow_mut(), start, end)
    }

    /// Starts a span of geometry that is not an entity (`PARTICLES`, `DESTROY`...).
    pub(super) fn mark_stage(&self, start: usize, stage: &str) {
        if self.pack_ids.is_some() || native_spans() {
            self.push_span(start, stage, 0);
        }
    }

    fn push_span(&self, start: usize, stage: &str, id: i32) {
        let mut spans = self.entity_spans.borrow_mut();
        let span = EntitySpan {
            start: start as u32,
            stage: horizon_shaders::render_stage(stage),
            id,
            ..Default::default()
        };
        // A span that drew nothing gives way to the next.
        match spans.last_mut() {
            Some(last) if last.start == span.start => *last = span,
            _ => spans.push(span),
        }
    }
}

/// Spans for a native mod (`native_render::RenderNeeds::entity_spans`), without a pack.
fn native_spans() -> bool {
    horizon_client::gfx::native_render::render_needs().entity_spans
}

/// A span's id for entity type `name` without a pack: stable, so a mod can match runs from one
/// frame to the next.
fn name_id(name: &str) -> i32 {
    // FNV-1a.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in name.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    hash as i32
}

/// `mc_Entity` (x, y) of each block sprite layer, as (id, fluid) pairs: terrain vertices carry
/// their sprite layer, not their block. A layer takes the id of the block named like its sprite
/// (`block/oak_leaves` for `oak_leaves`), else the id most blocks drawing it share (ties: the
/// lowest); -1 when no block drawing it has one. y is 1 for the fluid sprites, -1 otherwise.
pub(super) fn block_id_table(pack: &super::PackResources, ids: &BlockIds) -> Vec<i32> {
    use std::collections::{BTreeMap, BTreeSet};
    let layers = pack
        .render
        .sprite_layers
        .values()
        .max()
        .map_or(0, |l| l + 1) as usize;
    let mut table = vec![-1; layers * 2];
    if ids.is_empty() {
        return table;
    }
    let registry = pack.assets.registry;
    let catalog = &pack.render.model_catalog;
    let sprite = |name: &str| pack.render.sprite_layers.get(name).copied();
    // (layer, id) -> (a block named like the sprite, the blocks).
    let mut votes: BTreeMap<(u32, i32), (bool, BTreeSet<&str>)> = BTreeMap::new();
    for i in 0..registry.len() {
        let state_id = horizon_world::registry::BlockStateId(i as u32);
        let Some(state) = registry.state(state_id) else {
            continue;
        };
        let Some(id) = ids.id(&state.name, |k| state.properties.get(k).map(String::as_str)) else {
            continue;
        };
        let block = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);
        let mut vote = |layer: u32| {
            let named = pack
                .layer_names
                .get(&layer)
                .and_then(|n| n.strip_prefix("block/"))
                == Some(block);
            let v = votes.entry((layer, id)).or_default();
            v.0 |= named;
            v.1.insert(&state.name);
        };
        // Fluids draw from their own sprites.
        let fluid = match block {
            "water" => ["water_still", "water_flow"],
            "lava" => ["lava_still", "lava_flow"],
            _ => ["", ""],
        };
        for f in fluid.iter().filter(|f| !f.is_empty()) {
            if let Some(l) = sprite(&format!("minecraft:block/{f}")) {
                vote(l);
            }
        }
        let origin = horizon_core::position::BlockPos::new(0, 0, 0);
        for (_, model) in catalog.collect(state_id, origin).unwrap_or_default() {
            for face in &model.faces {
                vote(face.layer);
            }
        }
    }
    let mut best: BTreeMap<u32, (bool, usize, i32)> = BTreeMap::new();
    for ((layer, id), (named, blocks)) in votes {
        let candidate = (named, blocks.len(), -id);
        let b = best.entry(layer).or_insert(candidate);
        if candidate > *b {
            *b = candidate;
        }
    }
    for (layer, (_, _, id)) in best {
        if let Some(slot) = table.get_mut(layer as usize * 2) {
            *slot = -id;
        }
    }
    for f in ["water_still", "water_flow", "lava_still", "lava_flow"] {
        if let Some(l) = sprite(&format!("minecraft:block/{f}"))
            && let Some(slot) = table.get_mut(l as usize * 2 + 1)
        {
            *slot = 1;
        }
    }
    table
}

#[cfg(test)]
mod native_pack_tests {
    use super::{active_pack, pack_draws_flat};
    use horizon_client::mod_list::{Installed, ModList, NativePage};

    #[test]
    fn shaders_packs_and_packs_with_a_shadow_pass_draw_flat() {
        let root = std::env::temp_dir().join(format!("horizon-flat-packs-{}", std::process::id()));
        // A folder without a manifest is a `shaders/` pack; with one, a pack of Horizon's own.
        let pack = |name: &str, manifest: Option<&str>| {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(extra) = manifest {
                std::fs::write(dir.join("material.wgsl"), "").unwrap();
                let text = format!(
                    "schema = 1\nid = \"{name}\"\nname = \"{name}\"\nversion = \"1\"\napi = \"{}\"\n\
                     [material]\nfile = \"material.wgsl\"\n{extra}",
                    horizon_shader_api::API
                );
                std::fs::write(dir.join(horizon_shader_api::MANIFEST), text).unwrap();
            }
            dir
        };
        assert!(pack_draws_flat(&pack("shaders_pack", None)));
        assert!(!pack_draws_flat(&pack("unshadowed", Some(""))));
        assert!(pack_draws_flat(&pack("shadowed", Some("[shadow]\n"))));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn native_shader_follows_its_mod_without_affecting_other_packs() {
        let mut mods = ModList::default();
        mods.installed.push(Installed {
            id: "horizon_better_visuals".into(),
            title: "Better Visuals".into(),
            description: String::new(),
            problem: None,
            dev: false,
            native: Some(NativePage::default()),
        });
        let owned = "native-horizon_better_visuals";
        assert_eq!(active_pack(owned, &mods), owned);
        mods.disabled.insert("horizon_better_visuals".into());
        assert_eq!(active_pack(owned, &mods), "");
        assert_eq!(active_pack("golden_hour", &mods), "golden_hour");
        mods.installed[0].native = None;
        assert_eq!(active_pack(owned, &mods), owned);
    }
}
