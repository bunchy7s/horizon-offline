//! `OminousItemSpawner`: the item an ominous trial spawner drops on its
//! fight. It floats, growing and turning (`OminousItemSpawnerRenderer`), for
//! 60 to 120 ticks, then falls as an item or is fired straight down as a
//! projectile (`spawnItem`). Saved with its chunk's entities (persist.rs).
use super::Game;
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    item_render,
    items::ItemStack,
    mesher::Vertex,
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `TICKS_BEFORE_ABOUT_TO_SPAWN_SOUND`.
const ABOUT_TO_SPAWN: i64 = 36;
/// `OminousItemSpawnerRenderer`: grows over its first 50 ticks, turning 40
/// degrees a tick.
const TICKS_SCALING: f32 = 50.0;
const ROTATION_SPEED: f32 = 40.0;

pub(super) struct OminousItemSpawner {
    /// Its entity id: from the entity counter on the server, the server's on
    /// a client.
    pub(super) id: u32,
    /// `DATA_ITEM`; `minecraft:air` stands for `ItemStack.EMPTY`.
    pub(super) item: ItemStack,
    pub(super) position: [f64; 3],
    pub(super) spawn_item_after: i64,
    tick_count: i64,
    /// Its UUID, and the compound it was loaded from (fields Horizon doesn't
    /// model survive a save).
    pub(super) uuid: [i32; 4],
    pub(super) raw: Option<Box<horizon_world::nbt::Tag>>,
}

/// `ProjectileItem.DispenseConfig` of the items fired down: power,
/// uncertainty and `overrideDispenseEvent`; None for items that just fall.
fn projectile(item: &str) -> Option<(f32, f32, Option<i32>)> {
    Some(match item {
        "minecraft:fire_charge" => (1.0, 6.666_666_5, Some(1018)),
        "minecraft:wind_charge" => (1.0, 6.666_666_5, Some(1051)),
        "minecraft:firework_rocket" => (0.5, 1.0, Some(1004)),
        "minecraft:experience_bottle"
        | "minecraft:splash_potion"
        | "minecraft:lingering_potion" => (1.375, 3.0, None),
        "minecraft:arrow"
        | "minecraft:spectral_arrow"
        | "minecraft:tipped_arrow"
        | "minecraft:snowball"
        | "minecraft:egg"
        | "minecraft:blue_egg"
        | "minecraft:brown_egg" => (1.1, 6.0, None),
        _ => return None,
    })
}

fn block(p: [f64; 3]) -> [i32; 3] {
    p.map(|v| v.floor() as i32)
}

impl Game<'_> {
    /// `OminousItemSpawner.create` added at `position`.
    pub(super) fn add_ominous_item_spawner(
        &mut self,
        item: ItemStack,
        position: [f64; 3],
        delay: i32,
    ) {
        self.next_entity += 1;
        self.ominous_spawners.push(OminousItemSpawner {
            id: self.next_entity,
            item,
            position,
            spawn_item_after: i64::from(delay),
            tick_count: 0,
            uuid: horizon_client::save::random_uuid(),
            raw: None,
        });
    }
    /// A saved spawner, as `readAdditionalSaveData` leaves it: its tick
    /// count starts again from 0.
    pub(super) fn load_ominous_item_spawner(&mut self, tag: horizon_world::nbt::Tag) {
        let Some((item, at, spawn_item_after)) =
            horizon_client::entity_nbt::ominous_item_spawner_from_nbt(&tag)
        else {
            return;
        };
        self.next_entity += 1;
        self.ominous_spawners.push(OminousItemSpawner {
            id: self.next_entity,
            item,
            position: [at.x, at.y, at.z],
            spawn_item_after,
            tick_count: 0,
            uuid: horizon_client::entity_nbt::uuid(&tag)
                .unwrap_or_else(horizon_client::save::random_uuid),
            raw: Some(Box::new(tag)),
        });
    }
    /// Every item spawner's tick: `commonTick`'s count, `tickServer` (the
    /// warning sound, then the item) and `tickClient` (particles every 5
    /// game ticks).
    pub(super) fn tick_ominous_item_spawners(&mut self) {
        let spawners = std::mem::take(&mut self.ominous_spawners);
        let mut kept = Vec::with_capacity(spawners.len());
        for mut s in spawners {
            s.tick_count += 1;
            let at = block(s.position);
            let center = at.map(|v| f64::from(v) + 0.5);
            if s.tick_count == s.spawn_item_after - ABOUT_TO_SPAWN {
                self.audio.play(
                    "minecraft:block.trial_spawner.about_to_spawn_item",
                    Some(center),
                    1.0,
                    1.0,
                );
            }
            if s.tick_count >= s.spawn_item_after {
                self.spawn_ominous_item(&s);
                continue;
            }
            if self.world.game_time % 5 == 0 {
                self.ominous_spawner_particles(s.position);
            }
            kept.push(s);
        }
        kept.append(&mut self.ominous_spawners);
        self.ominous_spawners = kept;
    }
    /// `handleAddEntity` for a spawner a server sent: its item comes with the
    /// entity data (`ItemStack.EMPTY` until then, which draws nothing).
    pub(super) fn add_remote_ominous_item_spawner(&mut self, id: u32, position: [f64; 3]) {
        self.ominous_spawners.push(OminousItemSpawner {
            id,
            item: ItemStack::new("minecraft:air", 1),
            position,
            spawn_item_after: 0,
            tick_count: 0,
            uuid: [0; 4],
            raw: None,
        });
    }
    /// `OminousItemSpawner.tick` on a client: `tickCount` and `tickClient`,
    /// the particles every 5 game ticks; only the server spawns the item.
    pub(super) fn tick_remote_ominous_item_spawners(&mut self) {
        let at: Vec<[f64; 3]> = self.ominous_spawners.iter().map(|s| s.position).collect();
        for s in &mut self.ominous_spawners {
            s.tick_count += 1;
        }
        if self.world.game_time % 5 == 0 {
            for position in at {
                self.ominous_spawner_particles(position);
            }
        }
    }
    /// `spawnItem`: a projectile shot down, or an item entity; level event
    /// 3021 and the entity_place game event.
    fn spawn_ominous_item(&mut self, s: &OminousItemSpawner) {
        // An empty item spawns nothing, and no event marks it.
        if &*s.item.item == "minecraft:air" {
            return;
        }
        let at = block(s.position);
        let p = s.position;
        match projectile(&s.item.item) {
            Some((power, uncertainty, event)) => {
                if let Some(event) = event {
                    self.level_event_at(event, at, 0);
                }
                let from = Vec3::new(p[0], p[1], p[2]);
                self.shoot_projectile_item(from, [0.0, -1.0, 0.0], &s.item, power, uncertainty);
            }
            None => self.spawn_item(s.item.clone(), p, super::dispenser::new_item_motion(), 0),
        }
        self.level_event_at(3021, at, 1);
        self.post_block_game_event("minecraft:entity_place", at, None, None);
    }
    /// `OminousItemSpawner.addParticles`: one to three ominous spawning
    /// particles from the spawner out to points around it.
    fn ominous_spawner_particles(&mut self, at: [f64; 3]) {
        let options = ParticleOptions::simple(ParticleType::OminousSpawning);
        let r = self.particles.random();
        let count = r.next_int_between_inclusive(1, 3).unwrap_or(1);
        let mut flights = Vec::new();
        for _ in 0..count {
            let mut spread = || 0.4 * (r.next_gaussian() - r.next_gaussian());
            let from = [at[0] + spread(), at[1] + spread(), at[2] + spread()];
            flights.push([from[0] - at[0], from[1] - at[1], from[2] - at[2]]);
        }
        for v in flights {
            self.particles.add(&options, at, v);
        }
    }
    /// `OminousItemSpawnerRenderer`: the item, full bright, growing over its
    /// first 50 ticks and turning.
    pub(super) fn ominous_item_spawner_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        for s in &self.ominous_spawners {
            let at = glam::DVec3::from_array(s.position);
            if at.distance_squared(camera) >= 64.0 * 64.0 {
                continue;
            }
            let state = self.resolve_item(&s.item, DisplayContext::Ground, None, 0);
            if state.is_empty() {
                continue;
            }
            let age = s.tick_count as f32 + alpha as f32;
            let scale = if age <= TICKS_SCALING {
                age.min(TICKS_SCALING) / TICKS_SCALING
            } else {
                1.0
            };
            let spin = (age * ROTATION_SPEED).rem_euclid(360.0);
            let pose = glam::Mat4::from_translation((at - camera).as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::splat(scale))
                * glam::Mat4::from_rotation_y(spin.to_radians());
            let seed =
                i64::from(self.pack.item_renderer.item_id(&s.item.item)) + i64::from(s.item.damage);
            for copy in item_render::cluster_poses(
                &state,
                pose,
                item_render::rendered_amount(s.item.count),
                seed,
            ) {
                self.submit_item_world(&state, copy, vertex_light(0xFF), out);
            }
        }
    }
}
