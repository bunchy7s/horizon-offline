//! The F3 overlay's text entries (26.3 `DebugEntry*`): what each enabled
//! entry of `debug-profile.json` reports to the `DebugScreenDisplayer`, in
//! `DebugScreenEntryList.rebuildCurrentList`'s order (by identifier). The
//! entries that read the server's level ask the integrated server for their
//! lines (`ServerDebug`); on another server they show what vanilla shows
//! there, nothing or `??`.
use super::Game;
use horizon_client::{debug_entries::ENTRIES, debug_overlay as overlay, debug_overlay::Displayer};
use horizon_world::{
    heightmap::{HeightmapState, HeightmapType},
    worldgen::terrain::TerrainBlockStates,
};
use std::collections::{HashMap, HashSet};

/// `DebugEntryLookingAt.RANGE`.
const LOOKING_AT_RANGE: f64 = 20.0;

/// The entries whose lines come from the server thread.
const SERVER_ENTRIES: [&str; 4] = [
    "chunk_source_stats",
    "entity_spawn_counts",
    "heightmap",
    "local_difficulty",
];

/// What the server thread reports for the window's player
/// (`DebugScreenOverlay.getServerChunk` and its `ServerLevel`).
#[derive(Clone, Debug, Default)]
pub(super) struct ServerDebug {
    /// The server chunk's heightmaps at the feet (`keepAfterWorldgen`).
    pub heights: Option<[i32; 4]>,
    pub difficulty: Option<String>,
    pub spawn_counts: Option<String>,
    pub chunks: Option<String>,
}

impl Game<'_> {
    /// Every enabled entry's lines, by identifier.
    pub(super) fn debug_entry_lines(&self, d: &mut Displayer, framebuffer: [i32; 2]) {
        let reduced = self.reduced_debug_info();
        let mut paths: Vec<&str> = ENTRIES
            .iter()
            .map(|e| e.path)
            .filter(|path| self.debug_entries.enabled(path, self.debug, reduced))
            .collect();
        paths.sort_unstable();
        let server = self.server_debug(paths.iter().any(|p| SERVER_ENTRIES.contains(p)));
        for path in paths {
            match path {
                "biome" => self.debug_biome_line(d),
                "chunk_render_stats" => self.debug_chunk_render_line(d),
                "chunk_source_stats" => self.debug_chunk_source_lines(d, server.as_ref()),
                "day_count" => {
                    if self.player.is_some() {
                        d.line(format!("Day #{}", self.world.day_time.div_euclid(24000)));
                    }
                }
                "entity_render_stats" => self.debug_entity_render_line(d),
                "entity_spawn_counts" => {
                    if let Some(line) = server.as_ref().and_then(|s| s.spawn_counts.clone()) {
                        d.line(line);
                    }
                }
                "fps" => self.debug_fps_line(d),
                "game_version" => d.priority(format!(
                    "Minecraft {v} ({v}/horizon)",
                    v = horizon_client::save::VERSION_NAME
                )),
                "gpu_utilization" => d.line(overlay::gpu_line(self.gpu_utilization)),
                "heightmap" => self.debug_heightmap_lines(d, server.as_ref()),
                "light_levels" => {
                    if let Some(feet) = self.debug_feet() {
                        let light = self.world.light(feet[0], feet[1], feet[2]);
                        d.group("light", [overlay::light_line(light >> 4, light & 15)]);
                    }
                }
                "local_difficulty" => {
                    if let Some(line) = server.as_ref().and_then(|s| s.difficulty.clone()) {
                        d.line(line);
                    }
                }
                "looking_at_block_state" | "looking_at_block_tags" => {
                    let lines = self.debug_looking_at_block(path == "looking_at_block_state");
                    d.group("looking_at_block", lines);
                }
                "looking_at_entity" | "looking_at_entity_tags" => {
                    let lines = self.crosshair_entity_type().map_or_else(Vec::new, |kind| {
                        if path == "looking_at_entity" {
                            vec![format!("{}Targeted Entity", overlay::UNDERLINE), kind]
                        } else {
                            overlay::tag_lines(&self.res.mob_data().entity_tags, &kind)
                        }
                    });
                    d.group("looking_at_entity", lines);
                }
                "looking_at_fluid_state" | "looking_at_fluid_tags" => {
                    let lines = self.debug_looking_at_fluid(path == "looking_at_fluid_state");
                    d.group("looking_at_fluid", lines);
                }
                "memory" => {
                    if let Some(lines) = self.memory_meter.borrow_mut().lines() {
                        d.group("memory", lines);
                    }
                }
                "particle_render_stats" => d.line(format!("P: {}", self.particles.len())),
                "player_position" | "player_section_position" => {
                    if let Some(p) = self.player.as_ref() {
                        let mut lines = overlay::position(
                            [p.position.x, p.position.y, p.position.z],
                            self.yaw,
                            self.pitch,
                            self.dim.id(),
                            self.streaming.forced.len(),
                        );
                        let section = lines.pop().unwrap_or_default();
                        if path == "player_position" {
                            d.group("position", lines);
                        } else {
                            d.group("position", [section]);
                        }
                    }
                }
                "player_speed" => {
                    if self.player.is_some() {
                        let speed = self.known_player_movement();
                        d.group(
                            "position",
                            [format!("Speed: {:.3} blocks/tick", speed.length())],
                        );
                    }
                }
                "post_effects" => {
                    let effects = self.requested_post_effects();
                    if !effects.is_empty() {
                        d.line(format!("Post: {}", effects.join(", ")));
                    }
                }
                "simple_performance_impactors" => self.debug_performance_lines(d),
                "sound_cache" => {
                    let (buffers, bytes) = self.audio.cache_stats();
                    d.line(overlay::sound_cache_line(buffers, bytes));
                }
                "sound_mood" => {
                    if self.player.is_some() {
                        d.line(overlay::sound_mood_line(
                            self.audio.engine.used_channels(),
                            [
                                horizon_client::sound_engine::STATIC_CHANNELS,
                                horizon_client::sound_engine::STREAMING_CHANNELS,
                            ],
                            self.ambience.moodiness,
                        ));
                    }
                }
                "system_specs" => {
                    self.debug_system_lines(d, framebuffer);
                    // Horizon's own: the shader pack's GPU time per pass (an
                    // empty group would move others).
                    let pass_lines = self.renderer.as_ref().map_or_else(Vec::new, |r| {
                        overlay::shader_pass_lines(&r.shader_pass_times())
                    });
                    if self.debug && !pass_lines.is_empty() {
                        d.group("shader pack", pass_lines);
                    }
                }
                "tps" => self.debug_tps_line(d),
                // DebugEntryChunkGeneration, where the world is generated
                // here: the window's server thread publishes its generators;
                // the deprecated in-frame path is not extended.
                "chunk_generation_stats" => {
                    if let Some(p) = self.player.as_ref() {
                        let terrain = self
                            .integrated
                            .as_ref()
                            .and_then(|server| server.terrain(self.dim.id()));
                        // `DebugEntryChunkGeneration.display`: the feet block
                        // decides whether the line is sampled again; otherwise
                        // the last result is drawn.
                        let feet = [
                            p.position.x.floor() as i32,
                            p.position.y.floor() as i32,
                            p.position.z.floor() as i32,
                        ];
                        let dim = self.dim.id();
                        let mut cache = self.debug_density.borrow_mut();
                        let line = cache.line(
                            dim,
                            feet,
                            terrain.map(|t| move || t.density_debug_line(feet)),
                        );
                        if let Some(line) = line {
                            d.group("chunk_generation", [line.to_owned()]);
                        }
                    }
                }
                // detailed_memory: the JVM's memory pools, which Horizon
                // has none of. The rest are renderers (debug_renderers.rs).
                _ => {}
            }
        }
    }

    /// The camera's feet (`Entity.blockPosition`).
    fn debug_feet(&self) -> Option<[i32; 3]> {
        let p = self.player.as_ref()?;
        Some([p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32))
    }

    /// The integrated server's lines when an entry wants them; the request
    /// (the dimension and the feet) goes with it, for the next server tick.
    fn server_debug(&self, wanted: bool) -> Option<ServerDebug> {
        let server = self.integrated.as_ref()?;
        let at = self
            .debug_feet()
            .filter(|_| wanted)
            .map(|feet| (self.dim, feet));
        server.request_debug(at);
        at.map(|_| server.debug())
    }

    /// `DebugEntryBiome`: the client level's biome at the feet, inside the
    /// build height.
    fn debug_biome_line(&self, d: &mut Displayer) {
        let Some([x, y, z]) = self.debug_feet() else {
            return;
        };
        if y < self.world.bottom_y() || y >= self.world.top_y() {
            return;
        }
        if let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4))
            && let Some(biome) = self.res.biomes().list.get(chunk.biome(x, y, z) as usize)
        {
            d.line(format!("Biome: {}", biome.name));
        }
    }

    /// `LevelExtractor.sectionStatistics`: the sections drawn of the view
    /// area's (smart culling is always on), the render distance, and
    /// `SectionRenderDispatcher.getStats` (the meshes queued; Horizon has
    /// no buffer pool, so no buffer is ever free).
    fn debug_chunk_render_line(&self, d: &mut Displayer) {
        let Some(r) = self.renderer.as_ref() else {
            return;
        };
        let distance = self.settings.render_distance;
        let side = (distance * 2 + 1) as usize;
        let total = side * side * (self.world.layout().height() / 16) as usize;
        d.line(format!(
            "C: {}/{total} (s) D: {distance}, pC: {:03}, aB: {:02}",
            r.drawn_sections,
            self.streaming.pending(),
            0
        ));
    }

    /// `LevelExtractor.entityStatistics`: the mobs drawn last frame of the
    /// level's entities, and the server's simulation distance.
    fn debug_entity_render_line(&self, d: &mut Displayer) {
        if self.player.is_none() {
            return;
        }
        let simulation = self
            .remote
            .as_ref()
            .map_or(self.settings.simulation_distance, |r| r.simulation_distance);
        d.line(format!(
            "E: {}/{}, SD: {simulation}",
            self.rendered_mobs.get(),
            self.debug_entity_count()
        ));
    }

    /// `Level.getEntityCount` over the entities Horizon keeps.
    fn debug_entity_count(&self) -> usize {
        self.mobs.iter().filter(|m| !m.removed).count()
            + self.dropped.len()
            + self.orbs.len()
            + self.paintings.len()
            + self.item_frames.len()
            + self.armor_stands.len()
            + self.boats.len()
            + self.carts.len()
            + self.crystals.len()
            + self.creatures.projectiles.len()
    }

    /// The sections holding a mob, an item or an orb
    /// (`EntitySectionStorage.count`).
    fn debug_entity_sections(&self) -> usize {
        let sections: HashSet<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| !m.removed)
            .map(|m| [m.position.x, m.position.y, m.position.z])
            .chain(
                self.dropped
                    .iter()
                    .map(|i| [i.position.x, i.position.y, i.position.z]),
            )
            .chain(
                self.orbs
                    .iter()
                    .map(|o| [o.position.x, o.position.y, o.position.z]),
            )
            .map(|p| p.map(|v| (v.floor() as i32) >> 4))
            .collect();
        sections.len()
    }

    /// `DebugEntryChunkSourceStats`: `ClientLevel.gatherChunkSourceStats`
    /// (the chunk storage's size and its loaded chunks, then the entities,
    /// the sections holding them and the chunks that tick them), and the
    /// server level's when this game runs it.
    fn debug_chunk_source_lines(&self, d: &mut Displayer, server: Option<&ServerDebug>) {
        if self.player.is_none() {
            return;
        }
        let range = self
            .remote
            .as_ref()
            .map_or(self.settings.render_distance, |r| r.view_distance);
        let side = (2 * (range.max(2) + 3) + 1) as usize;
        let loaded = self.world.chunks.len();
        d.line(format!(
            "Chunks[C] W: {}, {loaded} E: {},{},{loaded}",
            side * side,
            self.debug_entity_count(),
            self.debug_entity_sections()
        ));
        if let Some(line) = server.and_then(|s| s.chunks.clone()) {
            d.line(line);
        }
    }

    /// `DebugEntryHeightmap`: the client chunk's heightmaps at the feet,
    /// and the server chunk's.
    fn debug_heightmap_lines(&self, d: &mut Displayer, server: Option<&ServerDebug>) {
        let Some([x, _, z]) = self.debug_feet() else {
            return;
        };
        if !self.world.loaded(x, z) {
            return;
        }
        let heights = server.and_then(|s| s.heights);
        let server_height = |kind: HeightmapType| {
            let i = [
                HeightmapType::WorldSurface,
                HeightmapType::OceanFloor,
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
            ]
            .iter()
            .position(|k| *k == kind)
            .unwrap_or(0);
            heights.map_or(0, |h| h[i])
        };
        let lines = overlay::heightmap_lines(
            |kind| self.column_height(kind, x, z),
            heights
                .is_some()
                .then_some(&server_height as &dyn Fn(_) -> i32),
        );
        d.group("heightmaps", lines);
    }

    /// `Heightmap.getFirstAvailable`: one above the highest block the
    /// heightmap counts. The chunk keeps WORLD_SURFACE; every other type is
    /// at or under it, scanned down from there.
    pub(super) fn column_height(&self, kind: HeightmapType, x: i32, z: i32) -> i32 {
        let properties = &self.pack.assets.properties;
        let surface = self
            .world
            .world_surface(x, z, |s| properties.heightmap_state(s).is_air());
        if kind == HeightmapType::WorldSurface {
            return surface;
        }
        (self.world.bottom_y()..surface)
            .rev()
            .find(|&y| kind.is_opaque(properties.heightmap_state(self.world.block(x, y, z))))
            .map_or(self.world.bottom_y(), |y| y + 1)
    }

    /// `Entity.pick(20, 0, false)` from the camera: the block outline hit.
    fn debug_block_hit(&self) -> Option<[i32; 3]> {
        let (origin, dir) = self.aim()?;
        Some(self.clip_ray(origin, dir, LOOKING_AT_RANGE)?.pos)
    }

    /// `DebugEntryLookingAt.BlockStateInfo` or `BlockTagInfo`.
    fn debug_looking_at_block(&self, state: bool) -> Vec<String> {
        let Some(pos) = self.debug_block_hit() else {
            return Vec::new();
        };
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let Some(s) = self.pack.assets.registry.state(id) else {
            return Vec::new();
        };
        if state {
            overlay::state_lines(
                "Targeted Block",
                pos,
                &s.name,
                s.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )
        } else {
            overlay::tag_lines(&self.items().block_tags, &s.name)
        }
    }

    /// `Entity.pick(20, 0, true)` (`ClipContext.Fluid.ANY`): the first fluid
    /// along the view, under its surface, before the block outline hit.
    fn debug_fluid_hit(&self) -> Option<[i32; 3]> {
        let (origin, dir) = self.aim()?;
        let block = self.clip_ray(origin, dir, LOOKING_AT_RANGE);
        let reach = block.map_or(LOOKING_AT_RANGE, |h| {
            (0..3)
                .map(|i| (h.point[i] - origin[i]).powi(2))
                .sum::<f64>()
                .sqrt()
        });
        let mut t = 0.0;
        while t <= reach {
            let p: [f64; 3] = std::array::from_fn(|i| origin[i] + dir[i] * t);
            let b = p.map(|v| v.floor() as i32);
            if let Some((height, _)) = self.fluid_height_of(b[0], b[1], b[2], None)
                && p[1] - f64::from(b[1]) <= f64::from(height)
            {
                return Some(b);
            }
            t += 0.05;
        }
        block.map(|h| h.pos)
    }

    /// `FluidState`'s registered name and properties at `pos`: a source is
    /// `water`/`lava` with `falling`, a flowing one `flowing_*` with `level`.
    fn fluid_state(&self, pos: [i32; 3]) -> (String, Vec<(&'static str, String)>) {
        use horizon_client::blocks::Fluid;
        let f = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]));
        let kind = match f.fluid {
            Fluid::None => return ("minecraft:empty".into(), Vec::new()),
            Fluid::Water => "water",
            Fluid::Lava => "lava",
        };
        let falling = ("falling", f.fluid_falling.to_string());
        if f.fluid_level == 8 && !f.fluid_falling {
            (format!("minecraft:{kind}"), vec![falling])
        } else {
            (
                format!("minecraft:flowing_{kind}"),
                vec![falling, ("level", f.fluid_level.to_string())],
            )
        }
    }

    /// `DebugEntryLookingAt.FluidStateInfo` or `FluidTagInfo`.
    fn debug_looking_at_fluid(&self, state: bool) -> Vec<String> {
        let Some(pos) = self.debug_fluid_hit() else {
            return Vec::new();
        };
        let (name, properties) = self.fluid_state(pos);
        if state {
            overlay::state_lines(
                "Targeted Fluid",
                pos,
                &name,
                properties.iter().map(|(k, v)| (*k, v.as_str())),
            )
        } else {
            overlay::tag_lines(fluid_tags(&self.options.resources), &name)
        }
    }

    /// `Minecraft.crosshairPickEntity`'s type: the nearest entity under the
    /// crosshair before any block.
    fn crosshair_entity_type(&self) -> Option<String> {
        let mut best: Option<(f64, String)> = None;
        let mut offer = |hit: Option<(f64, String)>| {
            if let Some((distance, kind)) = hit
                && self.before_block(distance)
                && best.as_ref().is_none_or(|b| distance < b.0)
            {
                best = Some((distance, kind));
            }
        };
        offer(
            self.mob_hit()
                .map(|(i, d)| (d, self.mobs[i].kind.id.to_string())),
        );
        offer(
            self.boat_hit()
                .map(|(i, d)| (d, self.boats[i].entity_type())),
        );
        offer(
            self.cart_hit()
                .map(|(i, d)| (d, self.carts[i].kind.id().to_owned())),
        );
        offer(
            self.armor_stand_hit()
                .map(|(_, d, _)| (d, "minecraft:armor_stand".to_owned())),
        );
        offer(
            self.frame_hit()
                .map(|(i, d)| (d, self.item_frames[i].kind().to_owned())),
        );
        offer(
            self.painting_hit()
                .map(|(_, d)| (d, "minecraft:painting".to_owned())),
        );
        offer(
            self.dragon_hit()
                .map(|(_, d)| (d, "minecraft:ender_dragon".to_owned())),
        );
        offer(
            self.guest_hit()
                .map(|(_, d)| (d, "minecraft:player".to_owned())),
        );
        best.map(|b| b.1)
    }

    /// The server thread's half of the debug entries, in the level of
    /// `dim` while it is in place: the server chunk's heightmaps, the local
    /// difficulty, the last spawn state and the chunk source's counts.
    pub(super) fn server_debug_lines(&self, feet: [i32; 3]) -> ServerDebug {
        let [x, y, z] = feet;
        let key = (x >> 4, z >> 4);
        let loaded = self.world.chunks.contains_key(&key);
        let inside = y >= self.world.bottom_y() && y < self.world.top_y();
        let heights = loaded.then(|| {
            [
                HeightmapType::WorldSurface,
                HeightmapType::OceanFloor,
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
            ]
            .map(|kind| self.column_height(kind, x, z))
        });
        let difficulty = (loaded && inside).then(|| {
            let at = [f64::from(x), f64::from(y), f64::from(z)];
            overlay::local_difficulty_line(self.local_difficulty(at).effective)
        });
        let spawn_counts = self
            .spawn_counts
            .get(&self.dim)
            .map(|(chunks, counts)| overlay::spawn_counts_line(*chunks, counts));
        // ServerLevel.gatherChunkSourceStats: the chunk map's size, then
        // PersistentEntitySectionManager's known and visible entities, its
        // sections, the chunks it loaded and sees, and its two queues
        // (Horizon loads a chunk's entities with it: both stay empty).
        let (entities, loaded) = (self.debug_entity_count(), self.world.chunks.len());
        let chunks = Some(format!(
            "Chunks[S] W: {loaded} E: {entities},{entities},{},{loaded},{loaded},0,0",
            self.debug_entity_sections()
        ));
        ServerDebug {
            heights,
            difficulty,
            spawn_counts,
            chunks,
        }
    }
}

/// The fluid tags (`data/*/tags/fluid`), loaded once.
fn fluid_tags(root: &std::path::Path) -> &'static HashMap<String, HashSet<String>> {
    static TAGS: std::sync::OnceLock<HashMap<String, HashSet<String>>> = std::sync::OnceLock::new();
    TAGS.get_or_init(|| {
        horizon_client::items::load_tags(root, "fluid", &horizon_world::loot::PackOverlay::NONE)
            .unwrap_or_default()
    })
}
