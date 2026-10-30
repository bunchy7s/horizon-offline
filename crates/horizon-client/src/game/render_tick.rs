//! Per-tick client visuals: the particle engine, `ClientLevel.animateTick`,
//! campfire smoke and rain splashes (`ClientLevel.tickWeatherEffects`).
use super::{Dim, Game};
use horizon_client::assets::Properties;
use horizon_client::fog::{FogData, FogInputs, FogType, fog_type, update_rain_fog};
use horizon_client::{
    animate_tick::{AnimateContext, animate_tick, campfire_particles, marker_particle_target},
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    containers::BlockEntity,
    particles::{ParticleLevel, ParticleOptions, ParticleType},
    world::World,
};
use horizon_client::{
    hud_overlays,
    screen_effects::{ScreenInputs, ScreenQuad, lightmap_brightness},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates};

/// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)` from the chunk's
/// heightmap: one above the highest block in `blocks_motion_in_heightmap` or
/// holding a fluid, however far above the camera it is.
pub(super) fn motion_blocking_height(
    world: &World,
    properties: &Properties,
    x: i32,
    z: i32,
) -> i32 {
    world.motion_blocking(x, z, |s| {
        HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
    })
}
/// `ClientLevel.tickWeatherEffects`: the MOTION_BLOCKING height of the column
/// `x`, `z` when a rain splash may land on it for a camera in block row `y`,
/// that is above the world's bottom and within 10 blocks of the camera.
fn splash_height(world: &World, properties: &Properties, x: i32, y: i32, z: i32) -> Option<i32> {
    let top = motion_blocking_height(world, properties, x, z);
    (top > MIN_Y && (y - 10..=y + 10).contains(&top)).then_some(top)
}
/// The first block at or below `from` that blocks motion (leaves only when
/// `leaves`, as MOTION_BLOCKING_NO_LEAVES skips them) or holds a fluid, plus one.
fn motion_blocking_leaves(
    world: &World,
    facts: &BlockFacts,
    x: i32,
    z: i32,
    from: i32,
    leaves: bool,
) -> i32 {
    let mut y = from.min(MIN_Y + HEIGHT - 1);
    while y > MIN_Y {
        let f = facts.get(world.block(x, y, z));
        // #minecraft:leaves block motion, but MOTION_BLOCKING_NO_LEAVES skips them
        // (their water still counts).
        let blocks = if f.name.ends_with("_leaves") {
            leaves
        } else {
            f.full_collision
        };
        if blocks || f.fluid != Fluid::None {
            return y + 1;
        }
        y -= 1;
    }
    MIN_Y
}
/// `Biome.getPrecipitationAt` is rain (not snow) at this position.
fn rains_at(world: &World, biomes: &Biomes, x: i32, y: i32, z: i32) -> bool {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return false;
    };
    biomes.get(chunk.biome(x, y, z)).snows_at(x, y, z) == Some(false)
}

impl Game<'_> {
    pub(super) fn forward(&self) -> [f64; 3] {
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
        [
            -yaw.sin() * pitch.cos(),
            -pitch.sin(),
            yaw.cos() * pitch.cos(),
        ]
    }
    /// `FogRenderer.setupFog` for the frame's camera, and Horizon's Fog option.
    pub(super) fn frame_fog(
        &self,
        camera: glam::DVec3,
        sky: &horizon_client::environment::Sky,
        biome: u16,
        water_fog_color: [f32; 3],
    ) -> FogData {
        let forward = self.forward();
        let at = [camera.x, camera.y, camera.z];
        let fog_type = fog_type(&self.world, self.res.facts(), at, forward);
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(self.clock.fraction() as f32)
        } else {
            (0.0, 0.0)
        };
        let title_fog = horizon_client::fog::FogTables::default();
        let tables = if self.title.is_some() {
            &title_fog
        } else {
            self.res.fog()
        };
        let input = FogInputs {
            fog_type,
            sky,
            water_fog_color,
            biome,
            render_distance_chunks: self.terrain_distance(),
            distant_terrain: self.lod.is_some(),
            cloud_range_chunks: self.settings.cloud_range,
            camera_y: camera.y,
            min_y: self.level_min_y(),
            flat: self.level_flat(),
            forward: forward.map(|c| c as f32),
            rain,
            thunder,
            skybox: self.dim == Dim::Overworld,
            blindness: self
                .potions
                .effects
                .get("blindness")
                .map(|effect| (effect.duration, effect.duration == -1)),
            darkness: self.darkness_fog(),
            darken_world: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            boss_world_fog: self.boss_overlay.should_create_world_fog(),
            spectator: self.spectator(),
            fire_resistance: self.potions.effects.get("fire_resistance").is_some(),
            night_vision: self.fog_night_vision(),
        };
        let mut fog = horizon_client::fog::setup_fog(tables, &self.fog, &input);
        if !self.settings.horizon.fog {
            horizon_client::fog::clear_distance_fog(&mut fog, &input);
        }
        fog
    }
    /// `FogRenderer.computeFogColor`'s night vision brightenFactor:
    /// `GameRenderer.nightVisionScale`, but not while Darkness is on.
    fn fog_night_vision(&self) -> f32 {
        let partial = self.clock.fraction() as f32;
        match self.potions.effects.get("night_vision") {
            Some(e) if self.potions.effects.get("darkness").is_none() => {
                // MobEffectInstance.endsWithin(200); -1 is infinite.
                if e.duration < 0 || e.duration > 200 {
                    1.0
                } else {
                    0.7 + ((e.duration as f32 - partial) * std::f32::consts::PI * 0.2).sin() * 0.3
                }
            }
            _ => 0.0,
        }
    }
    /// `ClientLevelData.isFlat`: on the server thread, the world-wide flag.
    fn level_flat(&self) -> bool {
        match self.remote.as_ref() {
            Some(r) => r.flat,
            None => self.world_flat,
        }
    }
    /// `Level.getMinY`: the current dimension type's minimum y.
    fn level_min_y(&self) -> i32 {
        match self.remote.as_ref() {
            Some(r) => r.dimension.min_y,
            None => self.dim.min_y(),
        }
    }
    /// `SkyRenderer.shouldRenderDarkDisc`: eye below the horizon and not under
    /// water. `ClientLevelData.getHorizonHeight`: the bottom of a flat world,
    /// otherwise y 63.
    pub(super) fn dark_disc(&self, camera: glam::DVec3) -> bool {
        let horizon = if self.level_flat() {
            f64::from(self.level_min_y())
        } else {
            63.0
        };
        self.dim == Dim::Overworld
            && camera.y < horizon
            && fog_type(
                &self.world,
                self.res.facts(),
                [camera.x, camera.y, camera.z],
                self.forward(),
            ) != FogType::Water
    }
    /// `ScreenEffectRenderer` + `Hud` camera overlays for this frame.
    pub(super) fn screen_quads(&self, partial: f32, size: [u32; 2]) -> Vec<ScreenQuad> {
        let Some(p) = self.player.as_ref() else {
            return Vec::new();
        };
        let eye = [
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ];
        // LevelExtractor.getViewBlockingState: 8 points around the eye.
        // ScreenEffectRenderer: none of it for a spectator (noPhysics).
        let spectator = self.spectator();
        let mut block_overlay = None;
        let w = f64::from(p.dimensions.width) * 0.8;
        for i in (0..8).filter(|_| !spectator) {
            let t = [
                p.position.x + (f64::from(i & 1) - 0.5) * w,
                eye[1] + (f64::from((i >> 1) & 1) - 0.5) * 0.1,
                p.position.z + (f64::from((i >> 2) & 1) - 0.5) * w,
            ];
            let state = self.world.block(
                t[0].floor() as i32,
                t[1].floor() as i32,
                t[2].floor() as i32,
            );
            let f = self.res.facts().get(state);
            if f.render_shape != horizon_client::blocks::RenderShape::Invisible
                && f.solid_render
                && f.full_collision
            {
                // ScreenEffectRenderer: the block's particle material.
                block_overlay = Self::block_particle_sprite(&self.pack, state);
                break;
            }
        }
        let in_water =
            fog_type(&self.world, self.res.facts(), eye, self.forward()) == FogType::Water;
        let water =
            (in_water && !spectator).then(|| (self.eye_brightness(eye), self.yaw, self.pitch));
        // Hud.extractCameraOverlays; the spyglass scope (and its easing) is
        // drawn on the HUD by draw_spyglass.
        let fx = &self.screen_fx;
        let overlays = hud_overlays::CameraOverlays {
            vignette: self.settings.vignette.then_some((
                fx.vignette_brightness,
                self.border.warning_strength(p.position.x, p.position.z),
            )),
            first_person: self.camera_type.first_person(),
            scoping: self.scoping(),
            game_time_delta: 0.0,
            equipment: match &self.armor[0] {
                Some(stack) if &*stack.item == "minecraft:carved_pumpkin" => {
                    vec!["misc/pumpkinblur".to_owned()]
                }
                _ => Vec::new(),
            },
            ticks_frozen: p.ticks_frozen,
            percent_frozen: p.percent_frozen(),
            partial,
            portal: [fx.o_portal, fx.portal],
            nausea: self.nausea_blend(partial),
            screen_effect_scale: self.screen_effect_scale(),
        };
        let mut scope = self.scope_scale;
        let gui_overlays =
            hud_overlays::camera_overlays(size[0] as i32, size[1] as i32, &overlays, &mut scope);
        self.screen_fx.build(&ScreenInputs {
            sprites: &self.gui.atlas.sprites,
            flags: &self.gui.atlas.flags,
            aspect: size[0] as f32 / size[1].max(1) as f32,
            partial,
            block_overlay,
            water,
            on_fire: self.camera_type.first_person()
                && !self.spectator()
                && self.player.as_ref().is_some_and(|p| p.fire_ticks > 0),
            gui_overlays: &gui_overlays,
        })
    }
    /// `Lightmap.getBrightness(getMaxLocalRawBrightness(eye))`.
    fn eye_brightness(&self, eye: [f64; 3]) -> f32 {
        let b = [
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        ];
        let packed = self.world.light(b[0], b[1], b[2]);
        let sky = i32::from(packed >> 4) - self.sky_darken();
        let raw = sky.max(i32::from(packed & 15)).clamp(0, 15) as u8;
        lightmap_brightness(if self.dim == Dim::Nether { 0.1 } else { 0.0 }, raw)
    }
    pub(super) fn tick_particles(&mut self) {
        self.fog.tick_flicker();
        let eye = self.eye_position();
        if let Some(e) = eye {
            let brightness = self.eye_brightness(e);
            let inside_portal = self.client_portal_effect();
            self.screen_fx.tick(brightness, inside_portal);
            // LocalPlayer.aiStep: processPortalCooldown after the transition
            // effect (a local world's player counts down in handlePortal).
            if self.remote.is_some() && self.portals.player.cooldown > 0 {
                self.portals.player.cooldown -= 1;
            }
            let nausea = self
                .potions
                .effects
                .get("nausea")
                .map(|e| !e.ends_within(60));
            self.screen_fx.tick_spinning(nausea);
            // LocalPlayer water vision and AtmosphericFogEnvironment rain fog.
            let in_water =
                fog_type(&self.world, self.res.facts(), e, self.forward()) == FogType::Water;
            self.fog.tick(in_water);
            let b = [
                e[0].floor() as i32,
                e[1].floor() as i32,
                e[2].floor() as i32,
            ];
            let rain = if self.dim == Dim::Overworld {
                self.world.weather.levels(1.0).0
            } else {
                0.0
            };
            let biome = self
                .world
                .chunks
                .get(&(b[0] >> 4, b[2] >> 4))
                .map_or(0, |c| c.biome(b[0], b[1], b[2]));
            let rains = self.res.biomes().get(biome).precipitation;
            update_rain_fog(
                &mut self.fog,
                rain,
                self.world.light(b[0], b[1], b[2]) >> 4,
                rains,
                1.0,
            );
        }
        self.particles.particle_level = self.settings.particles;
        if let Some(e) = eye {
            self.particles.camera = e;
        }
        let (rain, _) = if self.dim == Dim::Overworld {
            self.world.weather.levels(1.0)
        } else {
            (0.0, 0.0)
        };
        let marker = marker_particle_target(self.mode, self.stack_in(self.selected));
        let marker_options = |state| {
            // BlockMarker: the particle sprite, untinted.
            let sprite = Self::block_particle_sprite(&self.pack, state)?;
            Some(ParticleOptions::block(
                ParticleType::BlockMarker,
                sprite,
                0xFFFFFF,
            ))
        };
        let world = &self.world;
        let facts = self.res.facts();
        let biomes = self.res.biomes();
        let properties = &self.pack.assets.properties;
        let raining = rain > 0.2;
        let raining_at = |p: [i32; 3]| {
            raining
                && motion_blocking_height(world, properties, p[0], p[2]) <= p[1]
                && rains_at(world, biomes, p[0], p[1], p[2])
        };
        let height_no_leaves =
            |p: [i32; 3]| motion_blocking_leaves(world, facts, p[0], p[2], i32::MAX, false);
        // The day timeline: firefly bush sounds from 12600 to 23400.
        let firefly_bush_sounds = self.dim == Dim::Overworld
            && (12600..23401).contains(&self.world.day_time.rem_euclid(24000));
        let tables = &self.pack.context.tables;
        let model_offset = |state, x, z| tables.offset(state, x, z);
        let gateway_particles = |state, p| {
            self.res
                .block_entity_render()
                .gateway_faces(world, state, p)
                .len()
        };
        // Level.isThundering: a thunder level above 0.9 where the sky is open.
        let thundering = self.dim == Dim::Overworld && world.weather.levels(1.0).1 > 0.9;
        let ctx = AnimateContext {
            world,
            facts,
            collision: self.collision,
            biomes,
            registry: self.pack.assets.registry,
            raining_at: &raining_at,
            height_no_leaves: &height_no_leaves,
            firefly_bush_sounds,
            sky_darken: self.sky_darken(),
            marker: marker.map(|name| (name, &marker_options as &dyn Fn(_) -> _)),
            model_offset: &model_offset,
            gateway_particles: &gateway_particles,
            thundering,
            ultrawarm: self.dim == Dim::Nether,
        };
        if let Some(e) = eye {
            let block = [
                e[0].floor() as i32,
                e[1].floor() as i32,
                e[2].floor() as i32,
            ];
            animate_tick(self.res.animate(), &mut self.particles, &ctx, block);
            // The Horizon Weather option off means no rain visuals or sounds at all.
            if rain > 0.0 && self.settings.horizon.weather {
                // ClientLevel.tickWeatherEffects: rain splashes within the weather
                // radius, and the rain sound.
                let mut random = LegacyRandom::new(self.world.game_time.wrapping_mul(312_987_231));
                let radius = self.settings.weather_radius;
                let diameter = 2 * radius + 1;
                let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32
                    / if self.settings.particles == 1 { 2 } else { 1 };
                let mut rain_position = None;
                for _ in 0..count {
                    let x = block[0] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let z = block[2] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let Some(top) = splash_height(world, properties, x, block[1], z) else {
                        continue;
                    };
                    if !rains_at(world, biomes, x, top, z) {
                        continue;
                    }
                    let below = top - 1;
                    rain_position = Some([x, below, z]);
                    // The Minimal option stops at the first spot that rains (the
                    // rain sound still plays there).
                    if self.settings.particles >= 2 {
                        break;
                    }
                    let (bx, bz) = (random.next_double(), random.next_double());
                    let state = world.block(x, below, z);
                    let f = facts.get(state);
                    let block_top = match self.collision.cached_shapes().cached_shape(state) {
                        Ok(Some(shape)) if !shape.is_empty() => {
                            shape.max_at(horizon_core::geometry::Axis::Y, bx, bz)
                        }
                        _ => 0.0,
                    };
                    let fluid_top = if f.fluid == Fluid::None {
                        0.0
                    } else {
                        f64::from(f.fluid_level) / 9.0
                    };
                    let smoke = f.fluid == Fluid::Lava
                        || &*f.name == "minecraft:magma_block"
                        || (f.name.ends_with("campfire")
                            && self.res.animate().is_lit_campfire(state.0));
                    let ty = if smoke {
                        ParticleType::Smoke
                    } else {
                        ParticleType::Rain
                    };
                    let p = [
                        f64::from(x) + bx,
                        f64::from(below) + block_top.max(fluid_top),
                        f64::from(z) + bz,
                    ];
                    self.particles
                        .add(&ParticleOptions::simple(ty), p, [0.0; 3]);
                }
                // The last rain column of the tick makes the sound, more often the
                // longer it has been quiet; indoors it is the quieter one.
                if let Some(at) = rain_position {
                    let waited = self.rain_sound_time;
                    self.rain_sound_time += 1;
                    if random.next_int_bound(3).unwrap_or(0) < waited {
                        self.rain_sound_time = 0;
                        let roof = motion_blocking_height(world, properties, block[0], block[2]);
                        let (name, volume, pitch) = if at[1] > block[1] + 1 && roof > block[1] {
                            ("minecraft:weather.rain.above", 0.1, 0.5)
                        } else {
                            ("minecraft:weather.rain", 0.2, 1.0)
                        };
                        let p = at.map(|c| f64::from(c) + 0.5);
                        // playLocalSound: guests make their own rain, so a host
                        // must not capture it for them.
                        self.audio.play_unshared(name, Some(p), volume, pitch);
                    }
                }
            }
        }
        if let Some(r) = &self.renderer {
            campfire_particles(
                self.res.animate(),
                &mut self.particles,
                &ctx,
                r.block_entities(),
                &|pos| match self.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
                    Some(BlockEntity::Campfire(c)) => c.items.each_ref().map(Option::is_some),
                    _ => [false; 4],
                },
            );
        }
        let level = ParticleLevel {
            world,
            facts,
            collision: self.collision,
            player: self
                .player
                .as_ref()
                .map(|p| ([p.position.x, p.position.y, p.position.z], p.velocity.y)),
        };
        self.particles.tick(&level);
        self.tick_pickups();
        for (name, at, volume, pitch) in std::mem::take(&mut self.particles.sounds) {
            self.audio.play(name, Some(at), volume, pitch);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{Properties, World, splash_height};
    use horizon_client::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    /// #3810: under a stone roof more than 64 blocks above the camera, as in a
    /// deep excavation, MOTION_BLOCKING is the roof, so no rain splash lands on
    /// the floor; with the roof gone, it does.
    #[test]
    fn no_rain_splash_under_a_high_roof() {
        // State 0 is air, 1 is stone.
        let properties =
            Properties::for_tests(&[(true, false, false, false), (false, false, true, true)]);
        let stone = BlockStateId(1);
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for x in 0..16 {
            for z in 0..16 {
                world.set(x, 0, z, stone).unwrap();
            }
        }
        // The camera stands on the floor, its block at y 1.
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        for roof in [70, 80, 200] {
            world.set(3, roof, 3, stone).unwrap();
            assert_eq!(
                splash_height(&world, &properties, 3, 1, 3),
                None,
                "roof at {roof}"
            );
            world.set(3, roof, 3, BlockStateId(0)).unwrap();
        }
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(1));
        // A roof within 10 blocks is where the splash lands.
        world.set(3, 8, 3, stone).unwrap();
        assert_eq!(splash_height(&world, &properties, 3, 1, 3), Some(9));
    }
}
