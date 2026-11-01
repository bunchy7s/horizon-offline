//! Level events whose client handler (`LevelEventHandler`) spawns particles
//! on a block: bone meal's growth (1505), dragon egg teleportation (2015), a
//! consumable's teleport (2017), an enderman's (2018), electric sparks (3002),
//! waxing (3003), wax off (3004) and scraping (3005), plus the smoke, drips,
//! egg and dragon fireball particles of 1501, 1502, 1504, 2006, 2009, 2012
//! and 3009.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::particles::{self, ParticleOptions, ParticleType};
use horizon_core::{geometry::trig, position::BlockPos, random::RandomSource};
use horizon_world::level::LevelEvent;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    pub(super) fn dragon_egg_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles.block_teleportation(pos, data, [16, 8, 16]);
    }
    /// Event 2016: `Shulker.teleportSomewhere`'s particles between the
    /// shell's old and new cells.
    pub(super) fn shulker_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles.block_teleportation(pos, data, [8, 8, 8]);
    }
    /// Event 2017: `TeleportRandomlyConsumeEffect`'s directional particles.
    pub(super) fn consume_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles
            .block_teleportation(pos, data, [127, 127, 127]);
    }
    /// Event 2018: an enderman's portal trail.
    pub(super) fn enderman_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles
            .enderman_teleportation(pos, data, self.settings.particles);
    }
    /// `LevelEventHandler.levelEvent` 3002 to 3005, drawing from the client
    /// level's random.
    pub(super) fn block_particle_event(&mut self, id: i32, pos: BlockPos, data: i32) {
        let ty = match id {
            3002 => ParticleType::ElectricSpark,
            3003 => ParticleType::WaxOn,
            3004 => ParticleType::WaxOff,
            3005 => ParticleType::Scrape,
            _ => return,
        };
        let p = [pos.x, pos.y, pos.z];
        let mut spawned = Vec::new();
        let random = self.particles.level_random();
        // 3002 with an axis: a lightning rod; otherwise copper cleaned by lightning.
        match usize::try_from(data) {
            Ok(axis) if id == 3002 && axis < 3 => {
                particles::along_axis(random, axis, p, (10, 19), |p, v| spawned.push((p, v)));
            }
            _ => particles::on_block_faces(random, p, |p, v| spawned.push((p, v))),
        }
        let options = ParticleOptions::simple(ty);
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `BoneMealItem.addGrowthParticles` (event 1505, `count` the data):
    /// happy villager sparks over a spreading block's neighbourhood (grass,
    /// nylium, netherrack, moss) or water, or inside a growing block.
    pub(super) fn growth_particles(&mut self, pos: BlockPos, count: i32) {
        let p = [pos.x, pos.y, pos.z];
        let state = self.world.block(p[0], p[1], p[2]);
        let facts = self.res.facts();
        // ParticleUtils.spawnParticles(level, pos, count, xzSpread, ySpread,
        // allowFloatingParticles, particle).
        let (at, count, xz, height, floating) = match self.res.behaviours().bone_meal_spreads(state)
        {
            Some(spreads) => {
                // BonemealableBlock.getParticlePos: below for rooted dirt
                // (its hanging roots) and mangrove leaves (their propagule).
                let at = match &*facts.get(state).name {
                    "minecraft:rooted_dirt" | "minecraft:mangrove_leaves" => [p[0], p[1] - 1, p[2]],
                    _ => p,
                };
                if spreads {
                    (at, count * 3, 3.0, 1.0, false)
                } else {
                    // ParticleUtils.spawnParticleInBlock: up to the
                    // block's shape (a full block when air).
                    (at, count, 0.5, self.block_shape_top(at), true)
                }
            }
            None if &*facts.get(state).name == "minecraft:water" => (p, count * 3, 3.0, 1.0, false),
            // A mod block the level answered bone meal for: the port knows no
            // `BonemealableBlock.Type` for it, and vanilla's `GROWER` is the
            // one a block growing itself has, so the sparks stay inside it
            // rather than spreading over its neighbours.
            None if !facts.get(state).name.starts_with("minecraft:") => {
                (p, count, 0.5, self.block_shape_top(p), true)
            }
            None => return,
        };
        self.spawn_particles(at, count, xz, height, floating, ParticleType::HappyVillager);
    }
    /// `BlockState.getShape(..).max(Y)` of the block at `at`, 1.0 for air.
    fn block_shape_top(&self, at: [i32; 3]) -> f64 {
        let facts = self.res.facts();
        let inside = self.world.block(at[0], at[1], at[2]);
        if facts.is_air(inside) {
            1.0
        } else {
            facts
                .get(inside)
                .outline
                .iter()
                .map(|b| b[4])
                .fold(0.0, f64::max)
        }
    }
    /// `ParticleUtils.spawnParticles`: `count` slow particles spread over
    /// `xz` blocks around `at` and `height` up, only above solid ground
    /// unless floating ones are allowed.
    fn spawn_particles(
        &mut self,
        at: [i32; 3],
        count: i32,
        xz: f64,
        height: f64,
        floating: bool,
        ty: ParticleType,
    ) {
        let options = ParticleOptions::simple(ty);
        for _ in 0..count.max(0) {
            let r = self.particles.level_random();
            let v = [0.0; 3].map(|_: f64| r.next_gaussian() * 0.02);
            let start = 0.5 - xz;
            let x = f64::from(at[0]) + start + r.next_double() * xz * 2.0;
            let y = f64::from(at[1]) + r.next_double() * height;
            let z = f64::from(at[2]) + start + r.next_double() * xz * 2.0;
            let below = [x.floor() as i32, y.floor() as i32 - 1, z.floor() as i32];
            if floating
                || !self
                    .res
                    .facts()
                    .is_air(self.world.block(below[0], below[1], below[2]))
            {
                self.particles.add(&options, [x, y, z], v);
            }
        }
    }
    /// `LevelEventHandler` 2012 (a turtle egg laid on sand):
    /// `ParticleUtils.spawnParticleInBlock` with happy villager sparks.
    pub(super) fn turtle_egg_particles(&mut self, pos: BlockPos, count: i32) {
        let at = [pos.x, pos.y, pos.z];
        let height = self.block_shape_top(at);
        self.spawn_particles(at, count, 0.5, height, true, ParticleType::HappyVillager);
    }
    /// `LevelEventHandler` 3009 (a sniffer egg cracking on moss):
    /// `spawnParticlesOnBlockFaces(.., EGG_CRACK, UniformInt.of(3, 6))`.
    pub(super) fn sniffer_egg_crack_particles(&mut self, pos: BlockPos) {
        let mut spawned = Vec::new();
        particles::on_block_faces_between(
            self.particles.level_random(),
            [pos.x, pos.y, pos.z],
            3,
            6,
            |p, v| spawned.push((p, v)),
        );
        let options = ParticleOptions::simple(ParticleType::EggCrack);
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// `LevelEventHandler` 1501 (lava put out) and 2009 (a wet sponge dried):
    /// eight puffs just above the block, drawn after the sound.
    pub(super) fn puffs_above_block(&mut self, pos: BlockPos, ty: ParticleType) {
        let options = ParticleOptions::simple(ty);
        for _ in 0..8 {
            let r = self.particles.level_random();
            let x = f64::from(pos.x) + r.next_double();
            let z = f64::from(pos.z) + r.next_double();
            self.particles
                .add(&options, [x, f64::from(pos.y) + 1.2, z], [0.0; 3]);
        }
    }
    /// `LevelEventHandler` 1502 (a redstone torch burning out): five smoke
    /// puffs inside the block, drawn after the sound.
    pub(super) fn torch_burnout_smoke(&mut self, pos: BlockPos) {
        let options = ParticleOptions::simple(ParticleType::Smoke);
        for _ in 0..5 {
            let r = self.particles.level_random();
            let x = f64::from(pos.x) + r.next_double() * 0.6 + 0.2;
            let y = f64::from(pos.y) + r.next_double() * 0.6 + 0.2;
            let z = f64::from(pos.z) + r.next_double() * 0.6 + 0.2;
            self.particles.add(&options, [x, y, z], [0.0; 3]);
        }
    }
    /// `LevelEventHandler` 1503: smoke on the inserted eye.
    pub(super) fn end_portal_frame_smoke(&mut self, pos: BlockPos) {
        let options = ParticleOptions::simple(ParticleType::Smoke);
        for _ in 0..16 {
            let r = self.particles.level_random();
            let x = f64::from(pos.x) + (5.0 + r.next_double() * 6.0) / 16.0;
            let z = f64::from(pos.z) + (5.0 + r.next_double() * 6.0) / 16.0;
            self.particles
                .add(&options, [x, f64::from(pos.y) + 0.8125, z], [0.0; 3]);
        }
    }
    /// `LevelEventHandler` 2006 (`DragonFireball.onHit`): 200 dragon breath
    /// particles thrown outwards, then the explosion sound when `data` is 1.
    pub(super) fn dragon_fireball_burst(&mut self, pos: BlockPos, data: i32) {
        for _ in 0..200 {
            let r = self.particles.level_random();
            let dist = r.next_float() * 4.0;
            let angle = r.next_float() * std::f32::consts::TAU;
            let vx = f64::from(trig::cos(f64::from(angle)) * dist);
            let vy = 0.01 + r.next_double() * 0.5;
            let vz = f64::from(trig::sin(f64::from(angle)) * dist);
            let options = ParticleOptions {
                scale: dist,
                ..ParticleOptions::simple(ParticleType::DragonBreath)
            };
            self.particles.add(
                &options,
                [
                    f64::from(pos.x) + vx * 0.1,
                    f64::from(pos.y) + 0.3,
                    f64::from(pos.z) + vz * 0.1,
                ],
                [vx, vy, vz],
            );
        }
        if data == 1 {
            let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
            let pitch = self.world.random.next_float() * 0.1 + 0.9;
            self.audio.play(
                "minecraft:entity.dragon_fireball.explode",
                Some(at),
                1.0,
                pitch,
            );
        }
    }
    /// `SpeleothemBlock.isStalactite`: a speleothem whose tip points down.
    fn is_stalactite(&self, state: BlockStateId) -> bool {
        self.res.facts().get(state).has_class("SpeleothemBlock")
            && self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("vertical_direction"))
                .is_some_and(|d| d == "down")
    }
    /// `LevelEventHandler` 1504: `PointedDripstoneBlock.spawnDripParticle` at
    /// the tip `pos`. The fluid above the stalactite's root picks the drip;
    /// with none, the dimension's default (lava in the Nether).
    pub(super) fn dripstone_drip_particle(&mut self, pos: BlockPos) {
        let tip_state = self.world.block(pos.x, pos.y, pos.z);
        if !self.is_stalactite(tip_state) {
            return;
        }
        // findRootBlock: up through the same stalactite, at most 11 steps.
        let tip_name = self.res.facts().get(tip_state).name.clone();
        let mut root = None;
        for step in 1..11 {
            let y = pos.y + step;
            if !(self.world.bottom_y()..self.world.top_y()).contains(&y) {
                break;
            }
            let state = self.world.block(pos.x, y, pos.z);
            if self.res.facts().get(state).name != tip_name {
                root = Some(y);
                break;
            }
            if !self.is_stalactite(state) {
                break;
            }
        }
        let Some(root_y) = root else {
            return;
        };
        let above = self.world.block(pos.x, root_y + 1, pos.z);
        let above_facts = self.res.facts().get(above);
        let nether = self.dim == super::Dim::Nether;
        let fluid = if &*above_facts.name == "minecraft:mud" && !nether {
            horizon_client::blocks::Fluid::Water
        } else {
            above_facts.fluid
        };
        let ty = match fluid {
            horizon_client::blocks::Fluid::Water => ParticleType::DrippingDripstoneWater,
            horizon_client::blocks::Fluid::Lava => ParticleType::DrippingDripstoneLava,
            horizon_client::blocks::Fluid::None if nether => ParticleType::DrippingDripstoneLava,
            horizon_client::blocks::Fluid::None => ParticleType::DrippingDripstoneWater,
        };
        let offset = self
            .pack
            .context
            .tables
            .offset(tip_state, pos.x, pos.z)
            .unwrap_or([0.0; 3]);
        // The tip shape starts 5/16 up; the particle sits a pixel below that.
        self.particles.add(
            &ParticleOptions::simple(ty),
            [
                f64::from(pos.x) + 0.5 + offset[0],
                f64::from(pos.y) + 5.0 / 16.0 - 0.0625,
                f64::from(pos.z) + 0.5 + offset[2],
            ],
            [0.0; 3],
        );
    }
    /// `ServerLevel.globalLevelEvent`: heard by every player on the server
    /// (an end portal opening, the dragon's death roar): here, and sent to
    /// every guest whatever their dimension.
    pub(super) fn global_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) {
        // ServerLevel.globalLevelEvent: without global_sound_events it is a
        // plain level event, which the client has no sound for (1023, 1028
        // and 1038 are only in `LevelEventHandler.globalLevelEvent`).
        if !self.ui.rules.bool("global_sound_events") {
            return;
        }
        self.global_level_event_sound(id, pos);
        if self.audio.capture.is_some() {
            self.lan_global_events.push((id, pos, data));
        }
    }
    /// `LevelEventHandler.globalLevelEvent`: the sound two blocks from the
    /// camera towards the event, so it is heard from anywhere.
    pub(super) fn global_level_event_sound(&mut self, id: i32, pos: [i32; 3]) {
        let (sound, volume) = match id {
            1023 => ("minecraft:entity.wither.spawn", 1.0),
            1028 => ("minecraft:entity.ender_dragon.death", 5.0),
            1038 => ("minecraft:block.end_portal.spawn", 1.0),
            _ => return,
        };
        let at = self.eye_position().map(|eye| {
            let d = [
                f64::from(pos[0]) + 0.5 - eye[0],
                f64::from(pos[1]) + 0.5 - eye[1],
                f64::from(pos[2]) + 0.5 - eye[2],
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length < 1.0e-4 {
                return eye;
            }
            [0, 1, 2].map(|k| eye[k] + d[k] / length * 2.0)
        });
        // Each guest plays it from the global packet.
        self.audio.play_unshared(sound, at, volume, 1.0);
    }
    /// `ClientLevel.addBreakingBlockEffects`: the block's hit sound (only for
    /// `play_sound`) and one crumb on the face being mined; nothing for air.
    pub(super) fn breaking_block_effects(
        &mut self,
        pos: [i32; 3],
        face: [i32; 3],
        play_sound: bool,
    ) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self.res.facts().is_air(state) {
            return;
        }
        if play_sound {
            let sounds = self.res.facts().get(state).sounds.clone();
            // Each guest plays it from the level event itself.
            self.audio.play_unshared(
                &sounds.hit,
                Some(pos.map(|v| f64::from(v) + 0.5)),
                (sounds.volume + 1.0) / 8.0,
                sounds.pitch * 0.5,
            );
        }
        self.crack_particle(pos, face, state);
    }
    /// `LevelEventHandler.potionSplashParticles` (2002 with `Effect`, 2007
    /// with `InstantEffect`): eight splash potion crumbs, and a hundred
    /// spell particles in the potion's colour `data`, flung out in a disc.
    pub(super) fn potion_splash_particles(&mut self, pos: BlockPos, data: i32, instant: bool) {
        let ty = if instant {
            ParticleType::InstantEffect
        } else {
            ParticleType::Effect
        };
        // Vec3.atBottomCenterOf.
        let at = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        ];
        let crumb = self.item_particle(&ItemStack::new("minecraft:splash_potion", 1));
        let r = self.particles.level_random();
        let crumbs: Vec<[f64; 3]> = (0..8)
            .map(|_| {
                [
                    r.next_gaussian() * 0.15,
                    r.next_double() * 0.2,
                    r.next_gaussian() * 0.15,
                ]
            })
            .collect();
        for v in crumbs {
            self.particles.add(&crumb, at, v);
        }
        let channel = |shift: i32| ((data >> shift) & 0xFF) as f32 / 255.0;
        let color = [channel(16), channel(8), channel(0)];
        for _ in 0..100 {
            let r = self.particles.level_random();
            let dist = r.next_double() * 4.0;
            let angle = r.next_double() * std::f64::consts::PI * 2.0;
            let velocity = [
                angle.cos() * dist,
                0.01 + r.next_double() * 0.5,
                angle.sin() * dist,
            ];
            let brightness = 0.75 + r.next_float() * 0.25;
            let options = ParticleOptions {
                color: [
                    color[0] * brightness,
                    color[1] * brightness,
                    color[2] * brightness,
                    1.0,
                ],
                scale: dist as f32,
                ..ParticleOptions::simple(ty)
            };
            self.particles.add(
                &options,
                [
                    at[0] + velocity[0] * 0.1,
                    at[1] + 0.3,
                    at[2] + velocity[2] * 0.1,
                ],
                velocity,
            );
        }
    }
    /// Queue and play level event `id` at `pos` (`Level.levelEvent`).
    pub(super) fn level_event_at(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.level_events.push(LevelEvent::LevelEvent {
            id,
            pos: BlockPos::new(pos[0], pos[1], pos[2]),
            data,
        });
        let _ = self.process_level_events();
    }
}
