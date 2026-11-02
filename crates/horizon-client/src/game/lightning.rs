//! Lightning (`ServerLevel.tickThunder`, `LightningBolt`,
//! `LightningBoltRenderer`): strikes on the highest motion-blocking block or
//! a living entity under open sky nearby, skeleton trap horses, fire,
//! `thunderHit` (damage and burning, charged creepers, zombified piglins,
//! witches, mooshroom colours, turtles, burnt items), lightning rods, the sky
//! flash, the sounds and the branching additive bolt.
use super::{Game, commands::Target};
use horizon_client::advancements::EntityView;
use horizon_client::mobs::{Damage, DamageKind, MobEvent, SpawnSetup, Species};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};

pub(super) struct Bolt {
    /// The entity id a server's players know it by (0 until it has one).
    pub id: u32,
    pub at: [f64; 3],
    life: i32,
    flashes: i32,
    seed: i64,
    visual_only: bool,
    /// Mooshrooms and copper golems change once per bolt.
    converted: Vec<u32>,
    /// The mobs it has struck (`hitEntities`).
    hit: Vec<u32>,
    /// The dropped items in the strike box (`hitEntities` holds every entity).
    hit_items: Vec<u32>,
    /// Items a cushion dropped under it, spared for 20 ticks
    /// (`Cushion.dropItem`: `setInvulnerableTime(20)`).
    spared: Vec<u32>,
    /// Blocks it set on fire (`blocksSetOnFire`).
    fires: i32,
    /// The player who channelled it (`LightningBolt.cause`): `Some(None)` for
    /// the host, `Some(Some(conn))` for a guest.
    pub cause: Option<Option<u32>>,
}
/// The box `LightningBolt.tick` collects the `LightningStrikeTrigger` bystanders from: 15 blocks
/// around the bolt, 15 down and `6 + 15` up.
fn in_viewer_box(bolt: [f64; 3], p: [f64; 3]) -> bool {
    (p[0] - bolt[0]).abs() <= 15.0
        && (p[2] - bolt[2]).abs() <= 15.0
        && p[1] >= bolt[1] - 15.0
        && p[1] <= bolt[1] + 21.0
}
impl Game<'_> {
    /// `ServerChunkCache.tickSpawningChunk`: one `tickThunder` roll per
    /// spawning chunk (within 128 blocks of a player) that is in entity
    /// ticking range.
    pub(super) fn tick_thunder(&mut self) {
        // isRaining (rain level > 0.2) and isThundering (> 0.9, sky light, no ceiling).
        let (rain, thunder) = self.world.weather.levels(1.0);
        if rain <= 0.2 || thunder <= 0.9 || self.dim != super::Dim::Overworld || self.world_debug {
            return;
        }
        let mut scratch = std::mem::take(&mut self.spawn_scratch);
        self.spawning_chunks(&self.level_players(), &mut scratch);
        let levels = self.tickets.levels();
        scratch
            .candidates
            .retain(|&(x, z)| levels.random_ticking(x, z));
        for (cx, cz) in scratch.candidates.drain(..) {
            if self.world.random.next_int_bound(100_000).unwrap_or(1) != 0 {
                continue;
            }
            // getBlockRandomPos(minX, 0, minZ, 15)
            let r = self.world.random.next_int();
            let (x, z) = (cx * 16 + (r & 15), cz * 16 + (r >> 16 & 15));
            let Some(pos) = self.lightning_target(x, z) else {
                continue;
            };
            let p = BlockPos::new(pos[0], pos[1], pos[2]);
            if !self.with_level(|b, level, _| b.is_raining_at(level, p)) {
                continue;
            }
            // Skeleton trap: the local effective difficulty / 100
            // (`getCurrentDifficultyAt(pos)`).
            let difficulty = f64::from(
                self.local_difficulty([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])])
                    .effective,
            );
            let below = &self
                .res
                .facts()
                .get(self.world.block(pos[0], pos[1] - 1, pos[2]))
                .name;
            let trap = self.ui.rules.bool("spawn_mobs")
                && self.world.random.next_double() < difficulty * 0.01
                && !below.ends_with("lightning_rod");
            if trap {
                let at = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
                // ServerLevel.tickChunk: a trap horse waits for a player.
                if let Some(id) = self.spawn_mob("minecraft:skeleton_horse", at, false)
                    && let Some(h) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    h.extra.trap = true;
                    h.extra.age = 0;
                }
            }
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            ];
            self.spawn_bolt(at, trap);
        }
        self.spawn_scratch = scratch;
    }
    /// `ServerLevel.findLightningRod`: `PoiManager.findClosest` over the
    /// lightning rod points of interest within 128 blocks of `center` whose
    /// block is the top (`WORLD_SURFACE`) of its column; the strike lands one
    /// above it. Horizon keeps no point-of-interest index, so this scans the
    /// loaded chunks the way `/locate poi` does.
    fn find_lightning_rod(&self, center: [i32; 3]) -> Option<[i32; 3]> {
        const RANGE: i32 = 128;
        let facts = self.res.facts();
        let mut is_rod: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut best: Option<(i64, [i32; 3])> = None;
        let (cx0, cz0) = ((center[0] - RANGE) >> 4, (center[2] - RANGE) >> 4);
        let (cx1, cz1) = ((center[0] + RANGE) >> 4, (center[2] + RANGE) >> 4);
        for cx in cx0..=cx1 {
            for cz in cz0..=cz1 {
                let Some(chunk) = self.world.chunks.get(&(cx, cz)) else {
                    continue;
                };
                for (i, section) in chunk.sections.iter().enumerate() {
                    let mut rod = |state: u32| {
                        *is_rod.entry(state).or_insert_with(|| {
                            facts
                                .get(horizon_world::registry::BlockStateId(state))
                                .name
                                .ends_with("lightning_rod")
                        })
                    };
                    if !section.blocks.maybe_has(&mut rod) {
                        continue;
                    }
                    let base_y = self.world.bottom_y() + 16 * i as i32;
                    for j in 0..4096 {
                        if !rod(section.get(j)) {
                            continue;
                        }
                        let p = [
                            cx * 16 + (j & 15) as i32,
                            base_y + (j >> 8) as i32,
                            cz * 16 + ((j >> 4) & 15) as i32,
                        ];
                        let d = [p[0] - center[0], p[1] - center[1], p[2] - center[2]]
                            .map(|d| i64::from(d) * i64::from(d))
                            .iter()
                            .sum::<i64>();
                        if d > i64::from(RANGE * RANGE) || best.is_some_and(|(b, _)| d >= b) {
                            continue;
                        }
                        let surface = self.world.world_surface(p[0], p[2], |s| facts.is_air(s));
                        if p[1] == surface - 1 {
                            best = Some((d, p));
                        }
                    }
                }
            }
        }
        best.map(|(_, p)| [p[0], p[1] + 1, p[2]])
    }
    /// `findLightningTargetAround`: the closest lightning rod, else a living
    /// entity under open sky in the column (3 blocks around), else the top of
    /// the column.
    fn lightning_target(&mut self, x: i32, z: i32) -> Option<[i32; 3]> {
        if !self.world.loaded(x, z) {
            return None;
        }
        let top =
            self.with_level(|b, level, _| b.motion_blocking_top(level, BlockPos::new(x, 0, z)));
        if let Some(rod) = self.find_lightning_rod([x, top.y, z]) {
            return Some(rod);
        }
        let sees_sky = |g: &Self, p: [i32; 3]| g.world.light(p[0], p[1], p[2]) >> 4 == 15;
        let mut candidates: Vec<[i32; 3]> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| {
                [
                    m.position.x.floor() as i32,
                    m.position.y.floor() as i32,
                    m.position.z.floor() as i32,
                ]
            })
            .filter(|p| (p[0] - x).abs() <= 3 && (p[2] - z).abs() <= 3 && p[1] >= top.y - 3)
            .filter(|p| sees_sky(self, *p))
            .collect();
        if let Some(p) = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying())
            .map(|p| {
                [
                    p.position.x.floor() as i32,
                    p.position.y.floor() as i32,
                    p.position.z.floor() as i32,
                ]
            })
            && (p[0] - x).abs() <= 3
            && (p[2] - z).abs() <= 3
            && p[1] >= top.y - 3
            && sees_sky(self, p)
        {
            candidates.push(p);
        }
        if !candidates.is_empty() {
            let i = self
                .world
                .random
                .next_int_bound(candidates.len() as i32)
                .unwrap_or(0) as usize;
            return Some(candidates[i]);
        }
        Some([x, top.y, z])
    }
    pub(super) fn spawn_bolt(&mut self, at: [f64; 3], visual_only: bool) {
        if !visual_only {
            let ctx = horizon_client::sculk::Context::default();
            self.post_game_event("minecraft:lightning_strike", at, ctx);
        }
        let r = &mut self.world.random;
        let seed = r.next_long();
        let flashes = r.next_int_bound(3).unwrap_or(0) + 1;
        self.lightning_seen[0] += 1;
        let id = self.take_entity_id();
        self.bolts.push(Bolt {
            id,
            at,
            life: 2,
            flashes,
            seed,
            visual_only,
            converted: Vec::new(),
            hit: Vec::new(),
            hit_items: Vec::new(),
            spared: Vec::new(),
            fires: 0,
            cause: None,
        });
    }
    /// `ClientLevel.addEntity` for a server's lightning bolt: on a client it
    /// only flashes, sounds and draws (`LightningBolt.tick` off the server).
    pub(super) fn spawn_remote_bolt(&mut self, id: u32, at: [f64; 3]) {
        self.bolts.retain(|b| b.id != id);
        let r = &mut self.world.random;
        let seed = r.next_long();
        let flashes = r.next_int_bound(3).unwrap_or(0) + 1;
        self.lightning_seen[0] += 1;
        self.bolts.push(Bolt {
            id,
            at,
            life: 2,
            flashes,
            seed,
            visual_only: true,
            converted: Vec::new(),
            hit: Vec::new(),
            hit_items: Vec::new(),
            spared: Vec::new(),
            fires: 0,
            cause: None,
        });
    }
    /// `spawnFire`: fire at the strike point and `extra` random neighbours,
    /// wherever `BaseFireBlock.getState` fire can survive. Returns how many
    /// blocks it set on fire.
    fn bolt_fire(&mut self, at: [f64; 3], extra: i32) -> i32 {
        let mut set = 0;
        let base = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        let mut spots = vec![base];
        for _ in 0..extra {
            let r = &mut self.world.random;
            let mut d = || r.next_int_bound(3).unwrap_or(1) - 1;
            spots.push([base[0] + d(), base[1] + d(), base[2] + d()]);
        }
        for p in spots {
            let state = self.world.block(p[0], p[1], p[2]);
            if !self.res.facts().is_air(state) {
                continue;
            }
            // BaseFireBlock.getState and FireBlock/SoulFireBlock.canSurvive.
            let at = BlockPos::new(p[0], p[1], p[2]);
            let fire = self.with_level(|b, level, ticks| {
                let fire = b.fire_state_for(level, ticks, at);
                b.can_survive(level, ticks, at, fire).then_some(fire)
            });
            if let Some(fire) = fire {
                let _ = self.level_set(p, fire);
                set += 1;
            }
        }
        set
    }
    /// `LightningBolt.tick` for every bolt.
    pub(super) fn tick_bolts(&mut self) {
        let mut bolts = std::mem::take(&mut self.bolts);
        bolts.retain_mut(|b| {
            if b.life == 2 {
                let pitch = 0.8 + self.world.random.next_float() * 0.2;
                self.audio.play(
                    "minecraft:entity.lightning_bolt.thunder",
                    Some(b.at),
                    10000.0,
                    pitch,
                );
                let pitch = 0.5 + self.world.random.next_float() * 0.2;
                self.audio.play(
                    "minecraft:entity.lightning_bolt.impact",
                    Some(b.at),
                    2.0,
                    pitch,
                );
                if !b.visual_only {
                    // Only Normal and Hard start fires with the strike itself.
                    if self.ui.difficulty >= 2 {
                        b.fires += self.bolt_fire(b.at, 4);
                    }
                    self.strike_rod(b.at);
                }
                // LightningBolt.tick runs clearCopperOnLightningStrike for
                // every server bolt, a skeleton trap's visual one included;
                // a client's copy of a server bolt (spawn_remote_bolt) only
                // flashes.
                if self.remote.is_none() {
                    self.bolt_clear_copper(b.at);
                }
            }
            b.life -= 1;
            if b.life < 0 {
                if b.flashes == 0 {
                    self.lightning_strike_triggers(b);
                    return false;
                }
                if b.life < -self.world.random.next_int_bound(10).unwrap_or(0) {
                    b.flashes -= 1;
                    b.life = 1;
                    b.seed = self.world.random.next_long();
                    if !b.visual_only {
                        b.fires += self.bolt_fire(b.at, 0);
                    }
                }
            }
            if b.life >= 0 {
                if self.flash == 0 {
                    self.lightning_seen[1] += 1;
                }
                self.flash = 2;
                if !b.visual_only {
                    let struck = self.thunder_hit(b);
                    if let Some(cause) = b.cause {
                        let who = cause.map_or(Target::Player, Target::Guest);
                        self.for_player(who, |g| g.trigger_channeled_lightning(&struck));
                    }
                }
            }
            true
        });
        bolts.append(&mut self.bolts);
        self.bolts = bolts;
    }
    /// `LightningRodBlock.onLightningStrike`: powered for 8 ticks.
    fn strike_rod(&mut self, at: [f64; 3]) {
        let p = [
            at[0].floor() as i32,
            (at[1] - 1.0e-6).floor() as i32,
            at[2].floor() as i32,
        ];
        let state = self.world.block(p[0], p[1], p[2]);
        if !self.res.facts().get(state).name.ends_with("lightning_rod") {
            return;
        }
        let registry = self.pack.assets.registry;
        if let Some(mut s) = registry.state(state).cloned() {
            s.properties.insert("powered".into(), "true".into());
            if let Some(powered) = registry.id(&s) {
                // The rod's onPlace schedules the 8-tick unpowering.
                let _ = self.level_set(p, powered);
            }
            // Level event 3002 with the rod's axis: sparks along it.
            let axis = match s.properties.get("facing").map(String::as_str) {
                Some("east" | "west") => 0,
                Some("north" | "south") => 2,
                _ => 1,
            };
            self.level_event_at(3002, p, axis);
        }
    }
    /// `LightningBolt.clearCopperOnLightningStrike` at the block the bolt struck.
    fn bolt_clear_copper(&mut self, at: [f64; 3]) {
        let p = BlockPos::new(
            at[0].floor() as i32,
            (at[1] - 1.0e-6).floor() as i32,
            at[2].floor() as i32,
        );
        self.with_level(|b, level, ticks| b.lightning_clear_copper(level, ticks, p));
        let _ = self.process_level_events();
    }
    /// `LightningStrikeTrigger` at the bolt's discard: every player within 256
    /// blocks, with the mobs in its 30-block box that it did not hit.
    fn lightning_strike_triggers(&mut self, b: &Bolt) {
        let at = b.at;
        let bolt = EntityView {
            kind: "minecraft:lightning_bolt".into(),
            position: at,
            blocks_set_on_fire: b.fires,
            ..Default::default()
        };
        let viewers: Vec<EntityView> = self
            .mobs
            .iter()
            .filter(|m| m.alive() && !b.hit.contains(&m.id))
            .filter(|m| in_viewer_box(at, [m.position.x, m.position.y, m.position.z]))
            .map(|m| self.mob_entity(m))
            .collect();
        // The item entities in the box are viewers too (`getEntities` is every entity).
        let mut viewers = viewers;
        viewers.extend(
            self.dropped
                .iter()
                .filter(|e| !b.hit_items.contains(&e.id))
                .filter(|e| in_viewer_box(at, [e.position.x, e.position.y, e.position.z]))
                .map(|e| self.item_entity_view(e)),
        );
        // The players are entities in that box too, unless the bolt struck
        // them (`thunderHit` reaches 3 blocks, 9 up: `hitEntities`).
        let struck = |p: [f64; 3]| {
            (p[0] - at[0]).abs() <= 3.0
                && (p[2] - at[2]).abs() <= 3.0
                && p[1] >= at[1] - 3.0
                && p[1] <= at[1] + 9.0
        };
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().filter(|p| !p.health.dead_or_dying()) else {
                return;
            };
            let pos = [p.position.x, p.position.y, p.position.z];
            if in_viewer_box(at, pos) && !struck(pos) {
                viewers.push(g.player_entity());
            }
        });
        self.for_each_player(|g| {
            let Some(p) = g.player.as_ref().map(|p| p.position) else {
                return;
            };
            let d2 = (p.x - at[0]).powi(2) + (p.y - at[1]).powi(2) + (p.z - at[2]).powi(2);
            if d2 < 256.0 * 256.0 {
                g.trigger_lightning_strike(&bolt, &viewers);
            }
        });
    }
    /// `thunderHit` for entities whose box meets the strike box, 3 blocks
    /// around and up to 9 above (`getEntities(this, AABB)` tests boxes, not
    /// positions). Returns the mobs in that box before the hit, as
    /// `CHANNELED_LIGHTNING` counts them. Boats, minecarts and experience
    /// orbs are hit too (`VehicleEntity.hurtServer` adds 5 x 10 > 40 and
    /// destroys them; an orb has 5 health).
    fn thunder_hit(&mut self, b: &mut Bolt) -> Vec<EntityView> {
        let at = b.at;
        let strike = Aabb::new(
            Vec3::new(at[0] - 3.0, at[1] - 3.0, at[2] - 3.0),
            Vec3::new(at[0] + 3.0, at[1] + 9.0, at[2] + 3.0),
        );
        // Whether the box of an entity standing at `p` (half width `w`,
        // height `h`) meets the strike box.
        let meets = move |p: [f64; 3], w: f64, h: f64| {
            strike.intersects(Aabb::new(
                Vec3::new(p[0] - w, p[1], p[2] - w),
                Vec3::new(p[0] + w, p[1] + h, p[2] + w),
            ))
        };
        let struck: Vec<(u32, EntityView)> = self
            .mobs
            .iter()
            .filter(|m| {
                m.alive()
                    && meets(
                        [m.position.x, m.position.y, m.position.z],
                        m.kind.width / 2.0,
                        m.kind.height,
                    )
            })
            .map(|m| (m.id, self.mob_entity(m)))
            .collect();
        b.hit.extend(struck.iter().map(|(id, _)| *id));
        let damage = Damage {
            kind: DamageKind::Generic,
            attacker: None,
            from: Some(b.at),
        };
        let mut events = Vec::new();
        let peaceful = self.ui.difficulty == 0;
        for m in self.mobs.iter_mut().filter(|m| m.alive()) {
            if !meets(
                [m.position.x, m.position.y, m.position.z],
                m.kind.width / 2.0,
                m.kind.height,
            ) {
                continue;
            }
            let setup = |m: &horizon_client::mobs::Mob| SpawnSetup {
                yaw: m.yaw,
                persistent: true,
                ..Default::default()
            };
            let at = [m.position.x, m.position.y, m.position.z];
            match m.species() {
                Species::Pig if !peaceful => {
                    m.removed = true;
                    events.push(MobEvent::SpawnKind {
                        kind: "minecraft:zombified_piglin".into(),
                        at,
                        setup: setup(m),
                    });
                    continue;
                }
                Species::Villager if !peaceful => {
                    m.removed = true;
                    events.push(MobEvent::SpawnKind {
                        kind: "minecraft:witch".into(),
                        at,
                        setup: setup(m),
                    });
                    continue;
                }
                Species::Mooshroom => {
                    if !b.converted.contains(&m.id) {
                        b.converted.push(m.id);
                        m.extra.variant = 1 - m.extra.variant.min(1);
                        self.audio
                            .play("minecraft:entity.mooshroom.convert", Some(at), 2.0, 1.0);
                    }
                    continue;
                }
                Species::Turtle => {
                    m.damage(f32::MAX, damage);
                    continue;
                }
                Species::Creeper => m.extra.charged = true,
                // CopperGolem.thunderHit (after Entity.thunderHit's fire and
                // damage): once per bolt, an oxidized golem goes back one step
                // and its weathering timer (or wax, -2) is reset to -1.
                Species::CopperGolem if !b.converted.contains(&m.id) => {
                    b.converted.push(m.id);
                    if m.extra.variant > 0 {
                        m.extra.vector[0] = -1.0;
                        m.extra.variant =
                            horizon_client::mobs::copper_golem::weather_previous(m.extra.variant);
                    }
                }
                _ => {}
            }
            // Entity.thunderHit: burning (8 seconds) and 5 lightning damage.
            m.fire_ticks += 1;
            if m.fire_ticks == 0 {
                m.ignite_for_ticks(160);
            }

            m.damage(5.0, damage);
        }
        if !events.is_empty() {
            let _ = self.handle_mob_events(events);
        }
        // Cushion.thunderHit: a cushion whose box meets the strike box breaks and drops its item
        // (the bolt is no player, so no creative spare). Last first, since
        // break_cushion removes the entry.
        for i in (0..self.cushions.len()).rev() {
            let (lo, hi) = self.cushions[i].bounds();
            if strike.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )) {
                let before = self.dropped.len();
                self.break_cushion(i, false);
                b.spared.extend(self.dropped[before..].iter().map(|e| e.id));
            }
        }
        // Boats, then minecarts: `VehicleEntity.hurtServer` for 5 damage
        // (x 10 over the 40 a vehicle takes) destroys them. Lightning is no
        // fire source, so a TNT cart does not prime.
        let mut i = self.boats.len();
        while i > 0 {
            i -= 1;
            let bb = self.boats[i].bounds();
            if strike.intersects(bb) {
                self.hurt_boat(i, 5.0, false);
            }
        }
        let mut i = self.carts.len();
        while i > 0 {
            i -= 1;
            let bb = self.carts[i].bounds();
            if strike.intersects(bb) {
                self.hurt_cart(i, 5.0, false, false);
            }
        }
        // `ExperienceOrb.hurtServer`: 5 damage off 5 health.
        let orbs_before = self.orb_ids();
        self.orbs
            .retain_mut(|o| !(strike.intersects(o.bounds()) && o.hurt(5.0)));
        self.native_orbs_left_since(&orbs_before);
        // Every player in the box, the server's players too (0.6 x 1.8).
        let strike_players = move |g: &mut Self| {
            if let Some(p) = g.player.as_ref().map(|p| p.position)
                && meets([p.x, p.y, p.z], 0.3, 1.8)
            {
                if let Some(p) = g.player.as_mut() {
                    p.fire_ticks += 1;
                    if p.fire_ticks == 0
                        && let Err(error) = p.ignite_for_ticks(160)
                    {
                        g.error = Some(error);
                    }
                }
                g.hurt_player_by("minecraft:lightning_bolt", 5.0);
            }
        };
        strike_players(self);
        self.each_guest_hit(strike_players);
        b.hit_items.extend(
            self.dropped
                .iter()
                .filter(|e| meets([e.position.x, e.position.y, e.position.z], 0.125, 0.25))
                .map(|e| e.id),
        );
        // Dropped items burn up (ItemEntity health 5), save fire-resistant ones.
        let before = self.dropped_ids();
        self.dropped.retain(|e| {
            !meets([e.position.x, e.position.y, e.position.z], 0.125, 0.25)
                || e.stack.item.contains("netherite")
                || (e.age < 20 && b.spared.contains(&e.id))
        });
        self.native_items_left_since(&before);
        struck.into_iter().map(|(_, view)| view).collect()
    }
    /// `LightningBolt.shouldRenderAtSqrDistance`: a bolt is drawn within 64
    /// blocks times the view scale of the camera, and past that only its sky
    /// flash and thunder remain. Simulation-order render inputs; seeded
    /// geometry lives in the renderer.
    pub(super) fn bolt_instances(
        &self,
        camera: glam::DVec3,
    ) -> Vec<horizon_client::gfx::lightning::Bolt> {
        let range = 64.0 * self.view_scale();
        self.bolts
            .iter()
            .filter(|b| {
                let at = glam::DVec3::new(b.at[0], b.at[1], b.at[2]);
                at.distance_squared(camera) < range * range
            })
            .map(|b| horizon_client::gfx::lightning::Bolt {
                at: b.at,
                seed: b.seed,
            })
            .collect()
    }
}

#[cfg(test)]
mod viewer_tests {
    use super::in_viewer_box;

    #[test]
    fn viewer_box_reaches_15_around_and_down_and_21_up() {
        let bolt = [0.0, 64.0, 0.0];
        assert!(in_viewer_box(bolt, [15.0, 49.0, -15.0]));
        assert!(in_viewer_box(bolt, [0.0, 85.0, 0.0]));
        assert!(!in_viewer_box(bolt, [0.0, 85.5, 0.0]));
        assert!(!in_viewer_box(bolt, [0.0, 48.5, 0.0]));
        assert!(!in_viewer_box(bolt, [15.5, 64.0, 0.0]));
    }
}
