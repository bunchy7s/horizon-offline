//! Mobs in the game loop: ticking with entity snapshots, natural and
//! chunk-generation spawning (`NaturalSpawner`), mob events (attacks,
//! breeding, conversions, drops), projectiles, player interactions
//! (feeding, shearing, milking, spawn eggs, throwing) and rendering.
use super::{
    Dim, Game, InteractTarget, dispenser, native_mob_draw::NativePlayerRender,
    native_projectiles::Impact, view_vector,
};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    Result,
    arrows::EntityHit,
    blocks::Fluid,
    collisions::SceneCollisions,
    health::{HurtSource, Hurting, Knock, knockback_canceled},
    items::ItemStack,
    mesher::Vertex,
    mobs::{
        Damage, DamageKind, Mob, MobEvent, MobWorld, PlayerView, Seen, SpawnSetup, Species, Who,
        ai::{eating_sound, feed_animal, takes_food},
        farm::Climate,
        spawning::{self, BiomeFacts},
    },
    mod_api::NativeLootDrops,
    particles::{ParticleOptions, ParticleType},
    projectiles::{Hit, Projectile, ProjectileKind, ProjectileWorld},
    text::Text,
};
use horizon_core::{
    geometry::{Aabb, Direction, Vec3, traverse_blocks, trig::Mth},
    position::BlockPos,
    random::RandomSource,
};
use std::sync::Arc;

/// The world impact sound after successful arrow damage, including self-shots.
/// `AbstractArrow.onHitEntity`'s damage: the crit bonus comes from the
/// arrow's own random.
fn arrow_damage(arrow: &mut Projectile, fallback: &mut impl RandomSource) -> f32 {
    let mut state = arrow.arrow.take();
    let damage = match state.as_mut() {
        Some(a) => arrow.arrow_damage(&mut a.random),
        None => arrow.arrow_damage(fallback),
    };
    arrow.arrow = state;
    damage
}

/// A sound a native mod queued, sent when the host is free.
pub(super) struct NativeSound {
    pub dimension: &'static str,
    pub sound: String,
    pub at: [f64; 3],
    pub source: i32,
    pub volume: f32,
    pub pitch: f32,
}

/// Mob-system state owned by the game.
#[derive(Default)]
pub(super) struct Creatures {
    pub projectiles: Vec<Projectile>,
    /// The entities a family tells the mods ticked, kept between ticks
    /// (game/native_ticks.rs).
    pub tick_buffer: Vec<(u32, super::native_ticks::TickKind)>,
    /// Wind bursts from a guest's turn, waiting for it to end (game/wind.rs).
    pub wind_bursts: Vec<super::wind::Burst>,
    /// The players a fishing hook can catch, taken as the hooks tick, and
    /// the pulls on them waiting for the hub (game/fishing.rs).
    pub hook_targets: Vec<super::fishing::HookTarget>,
    pub hook_pulls: Vec<(u32, horizon_core::geometry::Vec3)>,
    /// Mace smash knockback on the other players from a guest's turn, waiting
    /// for it to end (game/mace.rs).
    pub smash_pushes: Vec<super::mace::SmashPush>,
    /// Sweep attacks from a guest's turn, waiting for it to end
    /// (game/lan_pvp.rs).
    pub sweeps: Vec<super::lan_pvp::Sweep>,
    /// Explosion packets from a guest's turn, waiting for it to end
    /// (centre, small, radius, blocks hit, the players it pushed;
    /// game/lan_particles.rs).
    pub lan_explosions: Vec<([f64; 3], bool, f32, usize, Vec<(u32, [f64; 3])>)>,
    /// Ticks left of the elder guardian's apparition (MobAppearanceParticle).
    pub elder_apparition: i32,
    /// The wandering trader spawner's clock.
    pub trader: super::wandering_trader::Spawner,
    /// When each player last put a parrot on a shoulder (`timeEntitySatOnShoulder`).
    pub shoulder_time: std::collections::HashMap<[i32; 4], i64>,
    /// The cat spawner's countdown (`CatSpawner.nextTick`).
    pub cat_tick: i32,
    /// Native sounds waiting for the server host to be available.
    pub lan_sounds: Vec<NativeSound>,
    /// Particles from a guest's turn, waiting for it to end.
    pub lan_particles: Vec<super::lan_particles::QueuedParticle>,
}
pub(super) struct ProjWorld<'a, 'w> {
    pub w: &'a MobWorld<'w>,
    pub targets: Vec<(Who, Aabb)>,
}
impl ProjectileWorld for ProjWorld<'_, '_> {
    fn block_contacts(&self, entity: i32, kind: &str, from: Vec3, to: Vec3, bounds: Aabb) {
        horizon_client::block_contact::swept_inside(entity, kind, from, to, bounds, |p| {
            let state = self.w.world.block(p.x, p.y, p.z);
            (state, &self.w.facts.get(state).name)
        });
    }
    fn clip(&self, from: Vec3, to: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
        let mut scene = SceneCollisions {
            world: self.w.world,
            binding: self.w.collision,
            subject: None,
        };
        let r = traverse_blocks(from, to, 256, |p: BlockPos| {
            let state = self.w.world.block(p.x, p.y, p.z);
            if self.w.facts.is_air(state) {
                return None;
            }
            let shape = self
                .w
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p)
        });
        let hit = r.ok().flatten()?;
        Some((
            hit.location,
            [hit.block.x, hit.block.y, hit.block.z],
            hit.direction.normal(),
        ))
    }
    /// Entity.updateFluidInteraction over the projectile's box, not the
    /// block at its feet (#3072).
    fn in_water(&self, bounds: Aabb) -> bool {
        self.w.fluids(bounds, bounds.max_position().y).in_water
    }
    fn targets(&self) -> &[(Who, Aabb)] {
        &self.targets
    }
    fn empty(&self, p: [i32; 3]) -> bool {
        self.w.facts.is_air(self.w.world.block(p[0], p[1], p[2]))
    }
    fn solid(&self, p: [i32; 3]) -> bool {
        self.w
            .facts
            .get(self.w.world.block(p[0], p[1], p[2]))
            .full_collision
    }
}

/// `AbstractWindCharge` is pickable, so a thrown pearl's
/// `ProjectileUtil.getEntityHitResult` can strike it (`ThrownEnderpearl.onHit`).
/// `WIND_CHARGE_IDS + j` is `projectiles[j]` during `tick_projectiles`, below
/// the dragon's reserved ids.
const WIND_CHARGE_IDS: u32 = super::dragon::SPECIAL - 0x10000;

/// The index of the wind charge a `wind_charge_hitboxes` id stands for.
pub(super) fn wind_charge_hit(id: u32) -> Option<usize> {
    (WIND_CHARGE_IDS..super::dragon::SPECIAL)
        .contains(&id)
        .then(|| (id - WIND_CHARGE_IDS) as usize)
}

fn wind_charge_hitboxes(
    projectiles: &[Projectile],
    alive: &[bool],
    skip: usize,
) -> Vec<(Who, Aabb)> {
    projectiles
        .iter()
        .enumerate()
        .filter(|(j, q)| {
            *j != skip
                && alive[*j]
                && !q.stuck
                && matches!(
                    q.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                )
        })
        .map(|(j, q)| (Who::Mob(WIND_CHARGE_IDS + j as u32), q.aabb()))
        .collect()
}

/// `FlameFeatureRenderer`: fire quads stacked up a burning entity's box,
/// turned to the camera around Y, at full block light. `at` is the entity's
/// feet relative to the camera.
fn flame_geometry(
    atlas: &horizon_client::hud::HudAtlas,
    at: glam::Vec3,
    width: f32,
    height: f32,
    camera_yaw: f32,
    light: u8,
    out: &mut Vec<Vertex>,
) {
    let (Some(fire0), Some(fire1)) = (
        atlas.sprites.get("block/fire_0").copied(),
        atlas.sprites.get("block/fire_1").copied(),
    ) else {
        return;
    };
    let s = width * 1.4;
    let mut h = height / s;
    // Mth.rotationAroundAxis(Y, camera.orientation): the camera's turn about Y.
    let turn = glam::Mat3::from_rotation_y(std::f32::consts::PI - camera_yaw.to_radians());
    let lift = 0.3 - (h as i32) as f32 * 0.02;
    let packed = (u32::from(light >> 4) * 16) | ((15 * 16) << 8);
    let (mut r, mut yo, mut zo, mut ss) = (0.5f32, 0.0f32, 0.0f32, 0);
    while h > 0.0 {
        let tex = if ss % 2 == 0 { fire0 } else { fire1 };
        let (mut u0, v0, mut u1, v1) = (tex.u0, tex.v0, tex.u1, tex.v1);
        if ss / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        for (x, y, uv) in [
            (-r, -yo, [u1, v1]),
            (r, -yo, [u0, v1]),
            (r, 1.4 - yo, [u0, v0]),
            (-r, 1.4 - yo, [u1, v0]),
        ] {
            let p = at + turn * glam::Vec3::new(x, y, zo + lift) * s;
            out.push(Vertex {
                normal: [0; 4],
                pos: p.to_array(),
                uv,
                // HDR output: the flames shine like fire (entity.wgsl fs_hdr_mark).
                layer: u32::from(horizon_client::mesher::INCANDESCENT) << 16,
                color: 0xFFFF_FFFF,
                light: packed,

                glint_uv: [0.0, 0.0],
            });
        }
        h -= 0.45;
        yo -= 0.45;
        r *= 0.9;
        zo -= 0.03;
        ss += 1;
    }
}
pub(super) fn seen(m: &Mob, player: Option<[i32; 4]>) -> Seen {
    m.seen(player)
}
/// Whether a mob a packet named may take the click: `mob_hit` skips a mob in
/// its death animation, the one the player rides (it shares the player's root
/// vehicle) and the camera entity the pick starts from, so a packet that names
/// one of those does nothing rather than reaching the feed, shear, leash or
/// name paths the pick path refuses.
pub(super) fn named_mob_pickable(
    alive: bool,
    id: u32,
    riding: Option<u32>,
    camera: Option<u32>,
) -> bool {
    alive && Some(id) != riding && Some(id) != camera
}

impl Game<'_> {
    pub(super) fn player_view(&self) -> Option<PlayerView> {
        let p = self.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR.
            creative: self.creative || self.spectator(),
            sneaking: self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft),
            swimming: p.swimming,
            look: view_vector(self.yaw, self.pitch),
            main_hand: self.held().map(|s| s.item.clone()),
            off_hand: None,
            head: self.armor[0].as_ref().map(|s| s.item.clone()),
            armor_cover: self.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            invisible: self.potions.effects.get("invisibility").is_some(),
            spectator: self.spectator(),
            gold_armor: self.wearing_gold(),
            hurt_by: self.player_hurt_by(),
            last_hurt: self.combat.last_hurt_mob.map(|(id, at)| (Who::Mob(id), at)),
            velocity: p.velocity,
            yaw: self.yaw,
            sprinting: p.sprinting,
            riding: self.riding.is_some(),
            hero: self.potions.effects.get("hero_of_the_village").is_some(),
            effects: self.potions.effects.list.iter().map(|e| e.id).collect(),
            uuid: self.level.player_uuid.unwrap_or_default(),
            allies: std::sync::Arc::new(self.player_allies()),
            remote: false,
            sleeping: self.sleeping.is_some(),
            sleep_timer: self.sleep_timer,
            passenger: self.riding.is_some(),
            vehicle: self.riding.and_then(|id| {
                if let Some(b) = self.boats.iter().find(|b| b.id == id) {
                    // A boat's box: 1.375 wide and 0.5625 high around its position.
                    let w = 0.6875;
                    return Some(horizon_client::mobs::Vehicle {
                        nautilus: false,
                        bounds: Aabb::new(
                            Vec3::new(b.position.x - w, b.position.y, b.position.z - w),
                            Vec3::new(b.position.x + w, b.position.y + 0.5625, b.position.z + w),
                        ),
                        velocity: b.velocity,
                    });
                }
                let m = self.mobs.iter().find(|m| m.id == id)?;
                matches!(m.species(), Species::Nautilus | Species::ZombieNautilus).then(|| {
                    horizon_client::mobs::Vehicle {
                        nautilus: true,
                        bounds: m.bounds(),
                        velocity: m.velocity,
                    }
                })
            }),
        })
    }
    /// Biome facts at a position for finalizeSpawn and variant rules.
    pub(super) fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4)) else {
            return BiomeFacts::default();
        };
        let b = world_biomes!(self).get(chunk.biome(x, y, z));
        let tags = self.res.mob_data();
        let has = |t: &str| tags.biome_has(t, &b.name);
        let climate = if has("minecraft:spawns_warm_variant_farm_animals") {
            Climate::Warm
        } else if has("minecraft:spawns_cold_variant_farm_animals") {
            Climate::Cold
        } else {
            Climate::Temperate
        };
        BiomeFacts {
            coral_nautilus: has("minecraft:spawns_coral_variant_zombie_nautilus"),
            climate,
            temperature: b.temperature,
            snowy: has("minecraft:spawns_snow_foxes")
                || b.name.contains("snow")
                || b.name.contains("frozen"),
            snow_fox: has("minecraft:spawns_snow_foxes"),
            swamp: b.name.contains("swamp"),
            ocean: has("minecraft:is_ocean"),
            mushroom: b.name == "minecraft:mushroom_fields",
            desert: b.name == "minecraft:desert",
            jungle: has("minecraft:is_jungle"),
            wolf: horizon_client::mobs::tame::wolf_variant_for(&b.name, &has),
            full_moon: (self.world.day_time / 24000).rem_euclid(8) == 0,
            frog: if has("minecraft:spawns_warm_variant_frogs") {
                1
            } else if has("minecraft:spawns_cold_variant_frogs") {
                2
            } else {
                0
            },
        }
    }
    /// Create a mob of `kind` at a position, finalized like a natural spawn.
    pub(super) fn spawn_mob(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32> {
        self.spawn_mob_for(kind, at, persistent, false)
    }
    /// `SummonCommand` and `SpawnEggItem`: vanilla spawns the mob, but it
    /// never calls `Mob.setPersistenceRequired` for it, so the mob is not
    /// persistent: the despawn rule holds it like any other mob, and only
    /// its NBT can make it persistent.
    pub(super) fn spawn_mob_spawned(&mut self, kind: &str, at: [f64; 3]) -> Option<u32> {
        // The spawn is not a natural one (no jockeys, the patrol leader roll
        // stays), but the mob is not persistent.
        let id = self.spawn_mob_for(kind, at, true, false)?;
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.persistent = false;
        }
        Some(id)
    }
    /// `spawn_mob` for a patrol or raid (`EntitySpawnReason.PATROL` / `EVENT`),
    /// which never rolls a patrol leader.
    pub(super) fn spawn_mob_for(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        scripted: bool,
    ) -> Option<u32> {
        let kind = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.res.mob_types().get(&kind).cloned()?;
        self.next_entity += 1;
        let yaw = self.world.random.next_float() * 360.0;
        let mut mob = Mob::new(self.next_entity, t, Vec3::new(at[0], at[1], at[2]), yaw);
        mob.persistent = persistent;
        let facts = self.biome_facts(
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        );
        let difficulty = self.local_difficulty(at);
        spawning::finalize(&mut mob, &facts, &difficulty);
        if !scripted {
            let reason = if persistent {
                horizon_client::mobs::raider::Reason::Other
            } else {
                horizon_client::mobs::raider::Reason::Natural
            };
            horizon_client::mobs::raider::finalize(&mut mob, reason);
        }
        let id = mob.id;
        self.mobs.push(mob);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        self.enchant_spawn_equipment(id);
        self.halloween_spawn_head(id);
        if !persistent && !scripted {
            self.spawn_jockeys(id, true);
        }
        Some(id)
    }
    /// `Mob.burnUndead` for a mob in daylight with an item on its sun
    /// protection slot (the head, or the body for a zombie horse or
    /// nautilus): `hurtAndBreak(random.nextInt(2))` on a damageable item, and
    /// when it breaks the slot empties and `onEquippedItemBroken` broadcasts
    /// the break event.
    fn wear_sun_protection(&mut self, id: u32) {
        use horizon_world::player_inventory::EquipmentSlot;
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let body = horizon_client::mobs::drops::head_is_body(m.species());
        let slot = if body {
            EquipmentSlot::Body
        } else {
            EquipmentSlot::Head
        };
        let Some(mut stack) = m.item_by_slot(slot) else {
            return;
        };
        if !horizon_client::stations::is_damageable(self.res.items(), &stack) {
            return;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let amount = m.random.next_int_bound(2).unwrap_or(0) as u32;
        if amount == 0 {
            return;
        }
        let loss = self.durability_loss(&stack, amount);
        if loss == 0 {
            return;
        }
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(0, |i| i.max_damage);
        stack.damage += loss;
        let broke = stack.damage >= max;
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        match (body, broke) {
            (true, false) => m.extra.body_damage = stack.damage as i32,
            (true, true) => {
                m.extra.head = None;
                m.extra.body_damage = 0;
            }
            (false, false) => m.extra.armor[0] = Some(stack),
            (false, true) => {
                m.extra.head = None;
                m.extra.armor[0] = None;
            }
        }
        if broke {
            // LivingEntity.entityEventForEquipmentBreak: head 49, body 65.
            self.lan_entity_event(id as i32, if body { 65 } else { 49 }, at);
        }
    }
    /// `LivingEntity.hurtArmor` -> `LivingEntity.doHurtEquipment`, from the hit
    /// `MobEvent::ArmorHurt` reported: every worn piece that wears when its
    /// wearer is hurt loses `max(1, damage / 4)` durability, Unbreaking
    /// applied, and at its maximum the slot empties and the equipment-break
    /// event goes out (`LivingEntity.entityEventForEquipmentBreak`: 49 to 52
    /// for the armour, 65 for the body). A fire hit leaves a fire-resistant
    /// piece alone (`ItemStack.canBeHurtBy`).
    fn hurt_mob_armor(&mut self, id: u32, damage: f32, kind: horizon_client::mobs::DamageKind) {
        use horizon_client::mobs::drops;
        use horizon_world::player_inventory::EquipmentSlot;
        if damage <= 0.0 {
            return;
        }
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let body_armour = drops::head_is_body(m.species());
        let amount = ((damage / 4.0) as u32).max(1);
        let name = damage_type(kind, false);
        let fire = self
            .motion
            .health
            .damage
            .id(name)
            .is_some_and(|d| self.motion.health.damage.in_tag(d, "minecraft:is_fire"));
        // A piece's maximum, or none when it may not lose durability at all.
        fn wearable(me: &Game<'_>, stack: &ItemStack, fire: bool, name: &str) -> Option<u32> {
            let info = me.res.items().get(&stack.item)?;
            if !info.equip_damage_on_hurt
                || info.max_damage == 0
                || !horizon_client::stations::is_damageable(me.res.items(), stack)
                || (fire && me.res.items().resists_fire(stack, name))
            {
                return None;
            }
            Some(info.max_damage)
        }
        let mut broke: Vec<i8> = Vec::new();
        // The four armour slots, then the body slot (`extra.head`).
        let event_of = horizon_client::mobs::item_break::equipment_break_event;
        for (index, slot) in [
            (0usize, EquipmentSlot::Head),
            (1, EquipmentSlot::Chest),
            (2, EquipmentSlot::Legs),
            (3, EquipmentSlot::Feet),
        ] {
            let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
                return;
            };
            let Some(stack) = m.extra.armor[index].clone() else {
                continue;
            };
            let Some(max) = wearable(self, &stack, fire, name) else {
                continue;
            };
            let loss = self.durability_loss(&stack, amount);
            if loss == 0 {
                continue;
            }
            let lost = stack.damage + loss;
            let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                return;
            };
            if lost >= max {
                m.extra.armor[index] = None;
                broke.push(event_of(slot));
            } else if let Some(worn) = m.extra.armor[index].as_mut() {
                worn.damage = lost;
            }
        }
        if body_armour {
            let body = self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.item_by_slot(EquipmentSlot::Body));
            if let Some(body) = body
                && let Some(max) = wearable(self, &body, fire, name)
            {
                let loss = self.durability_loss(&body, amount);
                if loss > 0 {
                    let lost = body.damage + loss;
                    let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                        return;
                    };
                    if lost >= max {
                        m.extra.head = None;
                        m.extra.body_damage = 0;
                        broke.push(event_of(EquipmentSlot::Body));
                    } else {
                        m.extra.body_damage = lost as i32;
                    }
                }
            }
        }
        for event in broke {
            // LivingEntity.onEquippedItemBroken through the level's broadcast.
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// What a converted frog gets once its tadpole has grown up: the spawn
    /// finalization of a conversion, then persistence.
    fn finalize_converted(&mut self, id: u32) {
        let Some(at) = self.mobs.iter().find(|m| m.id == id).map(|m| m.position) else {
            return;
        };
        let facts = self.biome_facts(
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let difficulty = self.local_difficulty([at.x, at.y, at.z]);
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            spawning::finalize(m, &facts, &difficulty);
            m.persistent = true;
        }
    }
    /// `Mob.convertTo(type, ConversionParams.single(...))` for a mob already
    /// removed: a new mob of `kind`, made without spawn finalization, takes
    /// its place with what `conversion::single` carries over, and the old
    /// mob's riders move onto it. A zombie that comes out of it rolls
    /// `Zombie.handleAttributes(special, CONVERSION)` (the
    /// `postConversionStep` of its `ConversionTracker`).
    pub(super) fn convert_to(
        &mut self,
        from: &Mob,
        kind: &str,
        params: horizon_client::mobs::conversion::Params,
    ) -> Option<u32> {
        let t = self.res.mob_types().get(kind).cloned()?;
        self.next_entity += 1;
        let mut mob = Mob::new(self.next_entity, t, from.position, from.yaw);
        horizon_client::mobs::conversion::single(from, &mut mob, params);
        if matches!(from.species(), Species::Zombie | Species::Husk) {
            let at = [from.position.x, from.position.y, from.position.z];
            let special = self.local_difficulty(at).special();
            horizon_client::mobs::finalize::zombie_attributes(
                &mut mob,
                special,
                horizon_client::mobs::finalize::Reason::Conversion,
            );
        }
        let id = mob.id;
        // ConversionType.SINGLE: the passengers get off and board the new mob.
        for rider in self.mobs.iter_mut() {
            if rider.extra.mount == Some(from.id) {
                rider.extra.mount = Some(id);
            }
        }
        self.mobs.push(mob);
        self.native_mobs_joined(self.mobs.len() - 1, false);
        Some(id)
    }
    /// The `summon` command: spawn a mob in front of the player or at a position.
    pub(super) fn summon(&mut self, kind: &str, at: Option<[f64; 3]>, baby: bool) -> Result<u32> {
        let at = match at {
            Some(a) => a,
            None => {
                let p = self.player.as_ref().ok_or("no player yet")?;
                let d = view_vector(self.yaw, 0.0);
                [
                    p.position.x + d[0] * 3.0,
                    p.position.y,
                    p.position.z + d[2] * 3.0,
                ]
            }
        };
        let id = self
            .spawn_mob_spawned(kind, at)
            .ok_or_else(|| format!("unknown or unsupported mob {kind}"))?;
        if baby && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.age = if m.species() == Species::Sniffer {
                horizon_client::mobs::sniffer::BABY_START_AGE
            } else {
                -24000
            };
        }
        self.spawn_jockeys(id, false);
        Ok(id)
    }
    /// The `mobs` command: nearby mobs with position, health and running goals.
    pub(super) fn control_mobs(&self, radius: f64) -> serde_json::Value {
        let center = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        self.control_mobs_at(radius, center)
    }
    pub(super) fn control_mobs_at(&self, radius: f64, center: Vec3) -> serde_json::Value {
        let list: Vec<serde_json::Value> = self
            .mobs
            .iter()
            .filter(|m| m.distance_sqr(center) <= radius * radius)
            .map(|m| {
                let mut state = serde_json::json!({
                    "id": m.id,
                    "kind": m.kind.id,
                    "pos": [m.position.x, m.position.y, m.position.z],
                    "health": m.health,
                    "baby": m.is_baby(),
                    "target": m.ai.target.map(|t| format!("{t:?}")),
                    "goals": m.ai.goals.running_names(),
                    "held": m.extra.main_hand.as_deref(),
                    "mount": m.extra.mount,
                    "rider": m.extra.rider,
                    // A player's input this tick, and whether it steers.
                    "ridden": m.rider.map(|r| r.steer),
                    "controlled": horizon_client::mobs::equine::controlled(m),
                    "saddled": m.extra.saddled,
                    "enchants": horizon_client::mobs::gear::gear(m).map(|g| g.slots.iter().map(|s| s.iter().map(|(e, l)| format!("{e}:{l}")).collect::<Vec<_>>()).collect::<Vec<_>>()),
                    "ints": m.extra.ints,
                    "pois": [m.extra.home, m.extra.bed, m.extra.meeting],
                    "profession": m.extra.variant2,
                    "targets": m.ai.targets.running_names(),
                    "path": m.ai.nav.path.as_ref().map(|p| p.nodes.len() - p.next.min(p.nodes.len())),
                    "on_ground": m.on_ground,
                    "variant": m.extra.variant,
                    "color": m.extra.color,
                    "sheared": m.extra.sheared,
                    "burning": m.fire_ticks > 0,
                    "size": m.extra.size,
                    "leash": m.extra.leash.map(|h| format!("{h:?}")),
                    "tame": m.extra.tame,
                    "sitting": m.extra.sitting,
                    "temper": m.extra.variant3,
                    "chested": m.extra.chested,
                    "head": m.extra.head.as_deref(),
                    "warden": horizon_client::mobs::warden::describe(m),
                    "villager": horizon_client::mobs::villager::ext(m).map(|e| serde_json::json!({
                        "food_level": e.food_level,
                        "inventory": e.inventory.iter().map(|s| format!("{}x{}", s.count, s.item)).collect::<Vec<_>>(),
                        "gossips": e.gossips.len(),
                        "last_slept": e.last_slept,
                        "raid": e.raid,
                    })),
                    "raider": horizon_client::mobs::raider::state(m).map(|s| serde_json::json!({
                        "raid": s.raid_id,
                        "wave": s.wave,
                        "leader": s.patrol_leader,
                        "patrolling": s.patrolling,
                        "patrol_target": s.patrol_target,
                        "vehicle": m.extra.mount,
                    })),
                });
                // Drawn seated (Entity.isPassenger); set apart because the
                // json! above is at the macro recursion limit.
                state["passenger"] = m.extra.passenger.into();
                state["guardian_beam"] = serde_json::json!(horizon_client::mobs::guardian::beam(m, 0.0));
                state
            })
            .collect();
        serde_json::json!({ "count": self.mobs.len(), "near": list, "projectiles": self.creatures.projectiles.len() })
    }
    /// Highest motion-blocking (no leaves) block + 1 (Heightmap MOTION_BLOCKING_NO_LEAVES).
    pub(super) fn motion_top(&self, x: i32, z: i32) -> i32 {
        let top = if self.dim == Dim::Nether { 127 } else { 319 };
        for y in (self.world.bottom_y()..=top).rev() {
            let f = self.res.facts().get(self.world.block(x, y, z));
            if (f.full_collision || f.fluid != Fluid::None) && !f.name.ends_with("leaves") {
                return y + 1;
            }
        }
        self.world.bottom_y()
    }
    pub(super) fn mob_world_base<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Level.canHaveWeather: only the overworld's rain and thunder levels
        // ever rise, so nothing rains or thunders in the Nether or the End.
        let weather = self.dim == Dim::Overworld;
        let player = self.player_view();
        MobWorld {
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            fluid_tables: &self.motion.fluids,
            movement: &self.motion.blocks,
            collision: self.collision,
            data: self.res.mob_data(),
            nearest_player: player.as_ref().filter(|p| !p.spectator).map(|p| p.position),
            player,
            others,
            leash_boats: &self.leash_boats,
            others_grid: None,
            others_index: None,
            daylight: self.dim != Dim::Nether && self.sky_darken() < 4,
            // day.json: false from tick 12542, true from 23460 (a step track).
            monsters_burn: self.dim == Dim::Overworld
                && !(12542..23460).contains(&self.world.day_time.rem_euclid(24000)),
            game_time: self.world.game_time,
            day_time: self.world.day_time,
            raining: weather && rain > 0.2,
            thundering: weather && thunder > 0.9,
            nether: self.dim == Dim::Nether,
            block_tags: &self.items().block_tags,
            landing: &self.motion.landing,
            biomes: world_biomes!(self),
            mob_griefing: self.ui.rules.bool("mob_griefing"),
            universal_anger: self.ui.rules.bool("universal_anger"),
            mob_drops: self.ui.rules.bool("mob_drops"),
            max_entity_cramming: self.ui.rules.int("max_entity_cramming"),
            end: self.dim == Dim::End,
            world_seed: self.level.seed,
            sea_level: if self.dim == Dim::Nether { 32 } else { 63 },
            teams: std::sync::Arc::new(self.mob_teams()),
            items: Some(self.items()),
            lid_openers: &self.lid_openers,
            mod_effects: if self.remote.is_none() {
                Some(self)
            } else {
                None
            },
            // The first player in the level: a server has no player of its own.
            difficulty: self.local_difficulty(
                self.level_players()
                    .first()
                    .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z]),
            ),
        }
    }
    /// What the generator finalizes this level's structure mobs with.
    pub(super) fn share_spawn_conditions(&self) {
        if let Some(terrain) = self.end.locator() {
            terrain.set_spawn_conditions(self.difficulty_id() as u8, self.world.day_time);
        }
    }
    /// `ServerLevel.getCurrentDifficultyAt`.
    pub(super) fn local_difficulty(&self, at: [f64; 3]) -> spawning::LocalDifficulty {
        use horizon_world::food::Difficulty;
        let base = match self.world.health_rules.difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        };
        let key = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        let (inhabited, moon) = if self.world.chunks.contains_key(&key) {
            let phase = self.world.day_time.div_euclid(24000).rem_euclid(8) as usize;
            (
                self.world.inhabited.get(&key).copied().unwrap_or(0),
                spawning::MOON_BRIGHTNESS[phase],
            )
        } else {
            (0, 0.0)
        };
        spawning::LocalDifficulty::new(base, self.world.day_time, inhabited, moon)
    }
    /// Generation's animals for the chunks generated since, once lit: chunk
    /// generation places them whether the level ticks or not.
    pub(super) fn spawn_fresh_chunks(&mut self) {
        if !self.world.chunks.has_fresh() {
            return;
        }
        let fresh: Vec<(i32, i32)> = self
            .world
            .chunks
            .fresh_keys_ordered()
            .into_iter()
            .filter(|k| self.world.chunks.get(k).is_some_and(|c| c.fresh && c.lit))
            .collect();
        for key in fresh {
            if let Some(c) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.world.chunks.remove_fresh(&key);
            self.spawn_generation_creatures(key.0, key.1);
        }
    }
    /// Advance every mob one tick, then apply their events.
    pub(super) fn tick_creatures(&mut self) -> Result<()> {
        self.attack_ticks = self.attack_ticks.saturating_add(1);
        self.tick_leads();
        horizon_client::spans::time("spawn_natural", || self.spawn_natural());
        // ServerChunkCache.tickChunks: the custom spawners tick only with spawn_mobs.
        if self.ui.rules.bool("spawn_mobs") {
            self.tick_wandering_trader();
            self.tick_patrol_spawner();
            self.tick_cat_spawner();
            self.tick_village_siege();
        }
        self.tick_raids();
        let ticked =
            horizon_client::spans::time("tick_creature_entities", || self.tick_creature_entities());
        self.dismount_underwater_mob_riders();
        ticked
    }
    /// `DistanceManager.inEntityTickingRange` in an entity-ticking chunk
    /// (block coordinates), from every player's tickets (`update_chunk_tickets`).
    pub(super) fn entity_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.tickets.levels();
        move |x, z| levels.entity_ticking_at(x, z)
    }
    /// The entity phase of `tick_creatures`: every mob's tick, deaths, and the
    /// projectiles and effects that follow from them.
    pub(super) fn tick_creature_entities(&mut self) -> Result<()> {
        // LivingEntity.die runs when the killing blow lands: mobs killed
        // since the last tick (by the player, explosions...) die first.
        let killed: Vec<Mob> = self
            .mobs
            .iter()
            .filter(|m| !m.alive() && !m.removed && !self.died.contains(&m.id))
            .cloned()
            .collect();
        for m in killed {
            self.died.insert(m.id);
            self.mob_died(&m);
        }
        // Their victims have been looted and mourned: no charged creeper is
        // waiting any more.
        self.combat.charged_exploders.clear();
        self.combat.exploded.clear();
        // EntityTickEvent.Pre for the mobs about to tick, before the loop holds them.
        let mut round = self.native_tick_round();
        if round.listening() {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(m) = self.mobs.get(i) {
                let (x, z) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                if self.world.loaded(x, z) && ticking(x, z) {
                    let (id, kind) = (m.id, super::native_ticks::TickKind::Mob(m.kind.clone()));
                    self.native_tick_pre_batch(&mut round, id, kind);
                }
                i += 1;
            }
        }
        let uuid = self.level.player_uuid;
        let mut snapshot: Vec<Seen> = self.mobs.iter().map(|m| seen(m, uuid)).collect();
        let mut events = Vec::new();
        let mut mobs = std::mem::take(&mut self.mobs);
        let mut dead = Vec::new();
        let mut corpses = Vec::new();
        let mut died = std::mem::take(&mut self.died);
        let ticking = self.entity_ticking_area();
        // With guests on, each mob sees its nearest player (game/lan_mobs.rs).
        let guests = self.lan_guest_views();
        let players = self.level_players();
        let mut bindings = match self.lan.as_mut() {
            Some(host) if !guests.is_empty() => std::mem::take(&mut host.mob_players),
            _ => Default::default(),
        };
        // The guests' entity ids by connection, for the shulker bullets fired
        // at the guest a mob sees.
        let guest_ids: Vec<(u32, u32)> = match self.lan.as_ref() {
            Some(host) if !guests.is_empty() => host
                .hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect(),
            _ => Vec::new(),
        };
        let mut guest_events = Vec::new();
        // The block writes since the last tick: mobs re-path around them.
        let path_writes = self
            .world
            .path_writes
            .get_or_insert_with(Vec::new)
            .split_off(0);
        // The mobs whose bodies go this tick: (id, feet, width, height).
        let mut gone = Vec::new();
        let level_random;
        {
            // The level random, lent to the mobs (brain behaviours draw from
            // it) and taken back after their ticks.
            let mut scratch = self.world.random.clone();
            let mut pushes = horizon_client::mobs::PendingPushes::default();
            let mut keep = Vec::with_capacity(mobs.len());
            let mut grid =
                horizon_client::mobs::seen_grid::SeenGrid::new(snapshot.iter().map(|s| s.bounds));
            // All but the snapshot is the same for every mob of the tick, so
            // build it once: the player view and the teams scan every UUID.
            self.leash_boats = self
                .boats
                .iter()
                .map(super::boat_leads::boat_state)
                .collect();
            let base = self.mob_world_base(&[]);
            let path_changes = horizon_client::mobs::collision_changes(&base, &path_writes);
            // Each mob keeps its snapshot slot all tick; the first slot of an
            // id answers, as a scan would.
            let mut index = horizon_client::fast_hash::FxHashMap::<u32, u32>::default();
            for (i, s) in snapshot.iter().enumerate() {
                index.entry(s.id).or_insert(i as u32);
            }
            for i in 0..mobs.len() {
                let mut w = MobWorld {
                    others: &snapshot,
                    others_grid: Some(&grid),
                    others_index: Some(&index),
                    ..base.clone()
                };
                let m = &mut mobs[i];
                // getCurrentDifficultyAt(blockPosition()): the mob's own chunk,
                // not the local player's (a server thread has no player).
                w.difficulty = self.local_difficulty([m.position.x, m.position.y, m.position.z]);
                w.nearest_player = super::natural_spawning::nearest_player(
                    &players,
                    m.position.x,
                    m.position.y,
                    m.position.z,
                )
                .map(|p| p.position);
                let seen_guest = if guests.is_empty() {
                    None
                } else {
                    let bound = bindings.get(&m.id).copied();
                    let conn = super::lan_mobs::player_for(m, w.player.as_ref(), &guests, bound);
                    match conn {
                        Some(conn) => {
                            bindings.insert(m.id, conn);
                            w.player = guests.iter().find(|g| g.0 == conn).map(|g| g.1.clone());
                        }
                        None => {
                            bindings.remove(&m.id);
                        }
                    }
                    conn
                };
                if !w
                    .world
                    .loaded(m.position.x.floor() as i32, m.position.z.floor() as i32)
                {
                    keep.push(!m.hostile() || m.persistent);
                    continue;
                }
                // ServerLevel.tick: every loaded entity checks despawning, but
                // only those in entity-ticking chunks (within the simulation
                // distance) tick.
                if !ticking(m.position.x.floor() as i32, m.position.z.floor() as i32) {
                    let kept = horizon_client::mobs::ai::despawn_check(m, &w);
                    if !kept {
                        died.remove(&m.id);
                        snapshot[i].alive = false;
                    }
                    keep.push(kept);
                    continue;
                }
                // A mod canceled its tick: it stays as it is.
                if round.canceled(m.id) {
                    keep.push(true);
                    continue;
                }
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
                horizon_client::mobs::blocks_changed(m, &path_changes);
                let from = events.len();
                let kept = m.tick(&w, &mut scratch, &mut events);
                // A killed mob's body is removed: its effects that act on
                // death run (`LivingEntity.remove` with KILLED).
                if !kept && !m.alive() && !m.removed && !m.extra.effects.is_empty() {
                    corpses.push(m.clone());
                }
                horizon_client::mobs::take_mob_pushes(&mut events, from, &mut pushes);
                // LivingEntity.tickDeath: after twenty ticks the body is removed.
                if !kept && !m.removed && !m.alive() && m.death_time >= 20 {
                    let (width, height) = (f64::from(m.width()), f64::from(m.height()));
                    gone.push((m.id, m.position, width, height));
                }
                if let Some(conn) = seen_guest {
                    // ShulkerBullet.finalTarget is the guest's entity: the
                    // bullet homes on it while the projectiles tick, where
                    // `Who::Player` is the host's own player.
                    if let Some(&(_, id)) = guest_ids.iter().find(|g| g.0 == conn) {
                        for e in &mut events[from..] {
                            if let MobEvent::Projectile(p) = e
                                && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
                                && *target == Who::Player
                            {
                                *target = Who::Mob(id);
                            }
                        }
                    }
                    let mut k = from;
                    while k < events.len() {
                        if super::lan_mobs::acts_on_player(&events[k]) {
                            guest_events.push((conn, events.remove(k)));
                        } else {
                            k += 1;
                        }
                    }
                }
                // Killed during its own tick (drowning, burning, falling...).
                if !m.alive() && !m.removed && died.insert(m.id) {
                    dead.push(m.clone());
                }
                if !kept {
                    died.remove(&m.id);
                }
                keep.push(kept);
                // ServerLevel ticks entities one after another: the ones
                // after this mob see it as it is now.
                snapshot[i] = seen(m, uuid);
                snapshot[i].alive &= kept;
                grid.update(i, snapshot[i].bounds);
                horizon_client::mobs::apply_brain_events(&mut mobs, &mut events, from);
            }
            let mut flags = keep.into_iter();
            mobs.retain(|_| flags.next().unwrap_or(true));
            for m in &mut mobs {
                horizon_client::mobs::apply_mob_pushes(m, &mut pushes);
            }
            level_random = scratch;
        }
        self.world.random = level_random;
        if let Some(host) = self.lan.as_mut()
            && !guests.is_empty()
        {
            // A set: a scan of every mob for each binding is quadratic, and a
            // server's loaded mobs run into the thousands.
            let live: horizon_client::fast_hash::FxHashSet<u32> =
                mobs.iter().map(|m| m.id).collect();
            bindings.retain(|id, _| live.contains(id));
            host.mob_players = bindings;
        }
        // The mobs added during the tick (into the emptied `self.mobs`) come
        // first, as `append` put them; with none, the ticked list goes back
        // as it is instead of being copied into a new one, every mob a tick.
        if self.mobs.is_empty() {
            self.mobs = mobs;
        } else {
            self.mobs.append(&mut mobs);
        }
        self.died = died;
        self.native_tick_post_batch(round);
        // ... and entity event 60 goes out as it is: the poof.
        for (id, at, width, height) in gone {
            let at = [at.x, at.y, at.z];
            self.lan_entity_event(id as i32, 60, at);
            self.death_poof(at, width, height);
        }
        self.sync_riders();
        self.tick_skeleton_traps();
        self.tick_apparition();
        self.guardian_bubbles();
        self.villager_reputation_tick();
        self.merchant_tick();
        self.villager_item_sense();
        self.villager_pickups();
        for m in dead {
            self.mob_died(&m);
        }
        for mut m in corpses {
            let names: Vec<&'static str> = m
                .extra
                .effects
                .iter()
                .map(|e| e.0.trim_start_matches("minecraft:"))
                .collect();
            let bounds = m.bounds();
            self.effects_on_death(&names, bounds, false, Some(m.id), Some(&mut m.random));
        }
        self.handle_mob_events(events)?;
        self.lan_guest_mob_events(guest_events);
        self.tick_projectiles()?;
        self.tick_fangs();
        self.tick_allays();
        self.tick_pandas();
        self.tick_foxes();
        self.tick_dolphins();
        self.tick_mob_looting();
        self.tick_breeze_effects();
        self.tick_sniffer_effects();
        self.piglin_pickups();
        self.sulfur_cube_pickups();
        self.sulfur_cube_power_priming();
        // Hurt and death sounds of damage dealt outside the mob ticks.
        let mut queued = Vec::new();
        for m in &mut self.mobs {
            queued.append(&mut m.outbox);
        }
        self.handle_mob_events(queued)?;
        Ok(())
    }
    /// `LivingEntity.makePoofParticles` (entity event 60) about a mob's box.
    pub(super) fn death_poof(&mut self, at: [f64; 3], width: f64, height: f64) {
        let options = horizon_client::particles::ParticleOptions::simple(
            horizon_client::particles::ParticleType::Poof,
        );
        for _ in 0..20 {
            let r = self.particles.level_random();
            let xa = r.next_gaussian() * 0.02;
            let ya = r.next_gaussian() * 0.02;
            let za = r.next_gaussian() * 0.02;
            let x = at[0] + width * (2.0 * r.next_double() - 1.0) - xa * 10.0;
            let y = at[1] + height * r.next_double() - ya * 10.0;
            let z = at[2] + width * (2.0 * r.next_double() - 1.0) - za * 10.0;
            self.particles.add(&options, [x, y, z], [xa, ya, za]);
        }
    }
    /// LivingEntity.die / dropAllDeathLoot.
    fn mob_died(&mut self, m: &Mob) {
        // LivingEntity.die: a sleeper gets up, and its bed is free again.
        if let Some(bed) = horizon_client::mobs::villager::sleeping_pos(m) {
            self.wake_villager(m.id);
            // The body may be gone already.
            self.set_bed_occupied(bed, false);
        }
        let killer = self.last_hurt_by_player.remove(&m.id);
        // TamableAnimal.die, before the rest of LivingEntity.die.
        self.tell_owner_pet_died(m, killer);
        self.love_cause.remove(&m.id);
        let by_player = killer.is_some() || m.ai.last_hurt_by_player > 0;
        let mut dying = m.clone();
        // LivingEntity.die: the death is heard, and a sculk catalyst nearby
        // takes the experience (before the loot is dropped). It takes what
        // would drop, whoever killed the mob.
        self.sculk_mob_died(m);
        let mut catalyst_xp = 0;
        let consumed =
            self.catalyst_takes_experience([m.position.x, m.position.y, m.position.z], |_| {
                catalyst_xp = if horizon_client::mobs::drops::drops_experience(&dying) {
                    horizon_client::mobs::drops::base_experience(&mut dying).max(0) as u32
                } else {
                    0
                };
                catalyst_xp
            });
        // CatalystListener: with experience to take, the player who last hurt
        // the mob gets KILL_MOB_NEAR_SCULK_CATALYST.
        if consumed && catalyst_xp > 0 {
            let who = match killer {
                Some(Some(conn)) => Some(super::commands::Target::Guest(conn)),
                Some(None) if self.lan_acting.is_none() => Some(super::commands::Target::Player),
                _ => None,
            };
            if let Some(who) = who {
                self.for_player(who, |g| {
                    // CatalystListener: trigger(player, killedEntity, damageSource).
                    let view = g.mob_entity(m);
                    let blow = m.last_damage.map(|(d, _)| g.damage_view(d));
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        blow.as_ref(),
                    );
                });
            }
        }
        if by_player {
            // A guest's kill is theirs: their statistics, scores and
            // advancements (LivingEntity.die credits lastHurtByPlayer).
            let guest = killer.flatten().filter(|_| self.lan_acting.is_none());
            let credited = guest.is_some_and(|conn| {
                let Some(mut host) = self.lan.take() else {
                    return false;
                };
                let done = self
                    .as_guest(&mut host, conn, |g| g.credit_kill(m))
                    .is_some();
                self.lan = Some(host);
                done
            });
            if !credited {
                self.credit_kill(m);
            }
        }
        self.score_entity_removed(m.id);
        // Raider.die's hero and Villager.tellWitnessesThatIWasMurdered go to
        // the causing entity, not the credited player: a tame wolf's kill is
        // neither its owner's heroism nor a murder by them.
        let slain_by_player = by_player && !killed_by_mob(m);
        self.raider_died(m, slain_by_player);
        self.villager_murdered(m, slain_by_player, killer.flatten());
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        let killer_entity = self.killer_loot_entity(killer.flatten());
        // LAST_DAMAGE_PLAYER only while the player-hurt memory lasts
        // (LivingEntity.dropFromLootTable's playerKilled): a mob that dies to
        // the environment long after a player's hit names no player.
        let player = killer
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|conn| self.killer_uuid(conn));
        if let Some(player) = player.filter(|_| m.extra.leash.is_some()) {
            self.lead_killers.insert(m.id, (player, self.dim));
        }
        self.drop_death_loot(&mut dying, player, by_player, killer_entity, consumed, pos);
        self.create_wither_rose(m);
        if m.species() == Species::CopperGolem {
            // CopperGolem.dropEquipment: the preserved equipment too.
            self.golem_drop_equipment(&mut dying);
        }
        if m.species() == Species::SulfurCube {
            self.split_sulfur_cube(m);
        }
        if m.species() == Species::Slime || m.species() == Species::MagmaCube {
            // Slime.remove: split into 2-4 smaller slimes.
            let size = m.extra.size;
            if size > 1 {
                let n = 2 + self.world.random.next_int_bound(3).unwrap_or(0);
                for k in 0..n {
                    let off = |kk: i32| (f64::from(kk % 2) - 0.5) * f64::from(size) / 4.0;
                    let at = [
                        m.position.x + off(k),
                        m.position.y + 0.5,
                        m.position.z + off(k / 2),
                    ];
                    if let Some(id) = self.spawn_mob(&m.kind.id, at, m.persistent)
                        && let Some(c) = self.mobs.iter_mut().find(|c| c.id == id)
                    {
                        horizon_client::mobs::cube::set_size(c, size / 2);
                    }
                }
            }
        }
    }
    /// The killer's side of a mob's death: `ServerPlayer.awardKillScore`'s
    /// statistics and scores, and the `player_killed_entity` trigger.
    fn credit_kill(&mut self, m: &Mob) {
        // Player.killedEntity: the killed statistic goes to the causing
        // entity, so a kill by a mob (a tame wolf credited to its owner)
        // leaves it out; ServerPlayer.awardKillScore counts the rest.
        if !killed_by_mob(m) {
            self.stats.add("killed", &m.kind.id, 1);
        }
        self.stats.custom("mob_kills", 1);
        self.score_player_killed(super::commands::Target::Mob(m.id));
        let view = self.mob_entity(m);
        let blow = m.last_damage.map(|(d, _)| self.damage_view(d));
        self.trigger_killed(&view, blow.as_ref());
    }
    /// A mob's damage source as advancement predicates see it: its type, the
    /// causing entity, and the projectile when one dealt it.
    pub(super) fn damage_view(&self, d: Damage) -> horizon_client::advancements::DamageView {
        let source = match d.attacker {
            Some(Who::Player) => Some(self.player_entity()),
            Some(Who::Mob(id)) => self
                .mobs
                .iter()
                .find(|o| o.id == id)
                .map(|o| self.mob_entity(o)),
            None => None,
        };
        let projectile = match d.kind {
            DamageKind::Arrow => Some("minecraft:arrow"),
            DamageKind::Fireball => Some("minecraft:fireball"),
            DamageKind::WitherSkull => Some("minecraft:wither_skull"),
            DamageKind::Fireworks => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let direct = match projectile {
            Some(kind) => Some(horizon_client::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            }),
            None => source.clone(),
        };
        horizon_client::advancements::DamageView {
            kind: source_damage_type(d).to_owned(),
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct,
            source,
            ..Default::default()
        }
    }
    /// `dropAllDeathLoot` through `mobs::drops`: the entity loot table from the
    /// world's random sequence, equipment by drop chance, species extras and
    /// the experience orbs, spawned in order.
    /// The killing player as a loot entity: their worn and held items, read
    /// from the guest who struck the last blow when a server's player did
    /// (on the server thread that is every player, the window's too), so
    /// Looting on their weapon counts.
    fn killer_loot_entity(&mut self, guest: Option<u32>) -> horizon_world::loot::LootEntity {
        let own = |g: &Self| {
            let mut e = horizon_world::loot::LootEntity::new("minecraft:player");
            let slots = [
                ("mainhand", g.stack_in(g.selected)),
                ("offhand", g.stack_in(super::shield::OFFHAND_SLOT)),
                ("head", g.armor[0].as_ref()),
                ("chest", g.armor[1].as_ref()),
                ("legs", g.armor[2].as_ref()),
                ("feet", g.armor[3].as_ref()),
            ];
            for (slot, stack) in slots {
                if let Some(stack) = stack {
                    e.equipment.push((slot.into(), to_loot(stack)));
                }
            }
            e
        };
        if let Some(conn) = guest.filter(|_| self.lan_acting.is_none())
            && let Some(mut host) = self.lan.take()
        {
            let entity = self.as_guest(&mut host, conn, |g| own(g));
            self.lan = Some(host);
            if let Some(entity) = entity {
                return entity;
            }
        }
        own(self)
    }
    /// The UUID of the player `last_hurt_by_player` keeps: the guest on
    /// connection `conn` (None once they left), else the player in place
    /// (None on a server, which has none of its own).
    fn killer_uuid(&mut self, conn: Option<u32>) -> Option<u128> {
        match conn {
            // During that guest's own turn, the hub is out of `lan`.
            Some(conn) if self.lan_acting != Some(conn) => {
                Some(self.lan.as_ref()?.hub.guest(conn)?.uuid)
            }
            Some(_) => Some(self.native_acting_id()),
            None if self.lan_acting.is_none() && self.player.is_some() => {
                Some(self.native_acting_id())
            }
            None => None,
        }
    }
    fn drop_death_loot(
        &mut self,
        m: &mut Mob,
        player_uuid: Option<u128>,
        by_player: bool,
        killer: horizon_world::loot::LootEntity,
        consumed: bool,
        pos: [f64; 3],
    ) {
        use horizon_client::mobs::{DamageKind, Who, drops};
        use horizon_world::loot::{LootDamage, LootEntity};
        let Some(loot) = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        ) else {
            return;
        };
        let player = || killer.clone();
        let damage = m.last_damage.map(|(d, _)| {
            let source = match d.attacker {
                Some(Who::Player) => Some(player()),
                Some(Who::Mob(id)) => self
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .map(|o| drops::loot_entity(o, None))
                    // A charged creeper's blast kills after the creeper left the list.
                    .or_else(|| {
                        self.combat
                            .charged_exploders
                            .contains(&id)
                            .then(|| LootEntity {
                                powered: true,
                                ..LootEntity::new("minecraft:creeper")
                            })
                    }),
                None => None,
            };
            let direct = match d.kind {
                DamageKind::Arrow => Some(LootEntity {
                    kind: "minecraft:arrow".into(),
                    ..LootEntity::default()
                }),
                DamageKind::Fireball => Some(LootEntity {
                    kind: "minecraft:fireball".into(),
                    ..LootEntity::default()
                }),
                DamageKind::WitherSkull => Some(LootEntity {
                    kind: "minecraft:wither_skull".into(),
                    ..LootEntity::default()
                }),
                DamageKind::Fireworks => Some(LootEntity {
                    kind: "minecraft:firework_rocket".into(),
                    ..LootEntity::default()
                }),
                _ => source.clone(),
            };
            LootDamage {
                kind: source_damage_type(d).into(),
                source,
                direct,
            }
        });
        let vehicle = m
            .extra
            .mount
            .and_then(|id| self.mobs.iter().find(|o| o.id == id))
            .map(|o| LootEntity::new(&o.kind.id));
        let death = drops::Death {
            loot: &loot,
            damage,
            player: by_player.then(player),
            luck: 0.0,
            vehicle,
            mob_drops: self.ui.rules.bool("mob_drops"),
            world_seed: self.level.seed,
        };
        // LootContextParams.ORIGIN of a death: the mob's position.
        let place = self.loot_place_at([m.position.x, m.position.y, m.position.z], None);
        let (items, skulls) = horizon_world::loot::with_native_loot_place(place, || {
            drops::die(m, &death, &mut self.rand_sequences)
        });
        if skulls && let Some(Who::Mob(id)) = m.last_damage.and_then(|(d, _)| d.attacker) {
            // Creeper.droppedSkulls: one head per charged explosion.
            self.combat.charged_exploders.retain(|c| *c != id);
            if let Some(creeper) = self.mobs.iter_mut().find(|o| o.id == id) {
                creeper.extra.charged = false;
            }
        }
        let raider = horizon_client::mobs::raider::is_raider(m.species());
        // The item drops come before the carried block and the orbs, so the
        // mods get them all at once and they still spawn in that order.
        let mut stacks = Vec::new();
        let mut rest = Vec::new();
        for d in items {
            match d {
                drops::Drop::Item(stack) => stacks.push(
                    if raider && stack.item == horizon_client::mobs::raider::BANNER {
                        horizon_client::mobs::raider::ominous_banner()
                    } else {
                        from_loot(&stack)
                    },
                ),
                drops::Drop::Stack(stack) => stacks.push(stack),
                other => rest.push(other),
            }
        }
        let kind = super::native_hooks::mob_kind(m);
        let table = drops::death_loot_table(m, death.mob_drops).filter(|t| loot.has_table(t));
        let context = NativeLootDrops {
            table: table.as_deref().unwrap_or_default(),
            dimension: self.dim.id(),
            origin: [m.position.x, m.position.y, m.position.z],
            block: None,
            entity: Some((m.id, &kind)),
            player: player_uuid,
            tool: None,
            explosion: None,
            used: false,
        };
        self.native_loot_drops(&context, &mut stacks);
        stacks.retain(|stack| stack.count > 0 && &*stack.item != "minecraft:air");
        for item in stacks {
            // Every death drop is its own ItemEntity, and the drop entity's
            // own random (not the level's) gives it the constructor's
            // horizontal push (its motion otherwise stays [0, 0.2, 0] and
            // the drops land stacked at the mob's centre, without disturbing
            // the level random that the orbs' merge ids and block ticks read
            // next).
            self.spawn_item(item, pos, dispenser::new_item_motion(), 10);
        }
        for d in rest {
            match d {
                drops::Drop::CarriedBlock(state) => {
                    // Enderman.dropCustomDeathLoot: spawnAtLocation for each
                    // of the carried block's silk-touch drops.
                    let stacks = horizon_client::mobs::enderman::carried_block_drops(
                        world_items!(self),
                        self.pack.assets.registry,
                        state,
                        &mut self.world.random,
                        Some((&mut self.rand_sequences, self.level.seed)),
                    );
                    for item in stacks {
                        self.spawn_item(item, pos, dispenser::new_item_motion(), 10);
                    }
                }
                drops::Drop::Orb(value) if !consumed => self.spawn_orbs(pos, value as u32),
                drops::Drop::Item(_) | drops::Drop::Stack(_) | drops::Drop::Orb(_) => {}
            }
        }
    }
    /// Melee or ranged damage to another mob; whether the hit landed.
    fn mob_hits_mob(&mut self, target: u32, amount: f32, source: Damage) -> bool {
        let mut killed = false;
        let mut hurt = false;
        let mut infested = false;
        let aim = self.attacker_aim(source.attacker);
        if let Some(t) = self.mobs.iter_mut().find(|m| m.id == target) {
            let hit = t.damage_aimed(amount, source, aim);
            hurt = hit;
            if hit {
                killed = !t.alive();
                // The hurt or death sound the mob raised.
                let center = t.center();
                for event in std::mem::take(&mut t.outbox) {
                    match event {
                        MobEvent::Sound(s, _, volume, pitch) => {
                            self.audio
                                .play(&format!("minecraft:{s}"), Some(center), volume, pitch);
                        }
                        MobEvent::Infested { .. } => infested = true,
                        other => {
                            // Left for the end of the tick.
                            t.outbox.push(other);
                        }
                    }
                }
            }
        }
        if hurt {
            self.resolve_player_responsible(target, source);
        }
        if infested {
            self.mob_infested(target);
        }
        if hurt
            && let Some(Who::Mob(attacker)) = source.attacker
            && let Some(m) = self
                .mobs
                .iter()
                .find(|m| m.id == attacker && m.species() == Species::Axolotl)
        {
            self.audio.play(
                "minecraft:entity.axolotl.attack",
                Some([m.position.x, m.position.y, m.position.z]),
                1.0,
                1.0,
            );
        }
        if killed && let Some(Who::Mob(killer)) = source.attacker {
            self.zombie_killed(killer, target);
        }
        hurt
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` after `source` hurt a
    /// mob: a tame wolf's hit is its owner's (`setLastHurtByPlayer(uuid, 100)`),
    /// so the kill drops experience and player-only loot and the owner, when
    /// in the world, gets the credit (`getKillCredit`); a tame wolf without an
    /// owner clears it. A player's own hits are recorded where they land.
    pub(super) fn resolve_player_responsible(&mut self, victim: u32, source: Damage) {
        let Some(Who::Mob(attacker)) = source.attacker else {
            return;
        };
        let Some(owner) = self
            .mobs
            .iter()
            .find(|m| m.id == attacker && m.species() == Species::Wolf && m.extra.tame)
            .map(|m| m.extra.owner)
        else {
            return;
        };
        let player = owner.and_then(|uuid| self.player_with_uuid(uuid));
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == victim) else {
            return;
        };
        m.ai.last_hurt_by_player = if owner.is_some() { 100 } else { 0 };
        match player {
            Some(who) => self.last_hurt_by_player.insert(victim, who),
            None => self.last_hurt_by_player.remove(&victim),
        };
    }
    /// The player in the world with `uuid` (`EntityReference.getPlayer`), as
    /// `last_hurt_by_player` keeps them: None for the player in place, else the
    /// guest's connection.
    fn player_with_uuid(&self, uuid: [i32; 4]) -> Option<Option<u32>> {
        if self.player.is_some()
            && horizon_client::mobs::tame::owned_by(Some(uuid), self.acting_uuid())
        {
            return Some(self.lan_acting);
        }
        let uuid = horizon_client::lan::playerdata::ints_uuid(uuid);
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .find(|g| g.placed && g.uuid == uuid)
            .map(|g| Some(g.conn))
    }
    /// `InfestedMobEffect.onMobHurt` for a mob: silverfish from half its height up.
    pub(super) fn mob_infested(&mut self, mob: u32) {
        if let Some(m) = self.mobs.iter().find(|m| m.id == mob) {
            let at = [
                m.position.x,
                m.position.y + f64::from(m.height()) / 2.0,
                m.position.z,
            ];
            let look = m.view_vector();
            self.infested_hurt(at, look, Some(mob));
        }
    }
    /// `LivingEntity.dropFromGiftLootTable`: the gift table from its random
    /// sequence with the mob as `this` (a chicken's egg depends on its
    /// variant); the loot data of the game's resources decides, and without it
    /// the items' table engine (no entity) rolls it.
    fn drop_gift_loot(&mut self, mob: u32, table: &'static str, at: [f64; 3]) {
        use horizon_client::mobs::drops;
        use horizon_world::loot::EntityLootParams;
        let loot = horizon_client::structure_blocks::loot_data(
            &self.options.resources,
            &self.pack_files(),
        );
        let this = self
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| drops::loot_entity(m, None));
        let stacks = match (loot, this) {
            (Some(loot), Some(this)) if loot.has_table(table) => {
                let params = EntityLootParams {
                    this,
                    ..EntityLootParams::default()
                };
                let seed = self.level.seed;
                match loot.random_sequence(table) {
                    Some(id) => {
                        let random = self.rand_sequences.get(id, seed);
                        loot.roll_entity(table, random, &params)
                            .iter()
                            .filter(|s| !s.is_empty())
                            .map(from_loot)
                            .collect()
                    }
                    None => Vec::new(),
                }
            }
            _ => self
                .res
                .items()
                .loot_table(table, false, None, &mut self.world.random),
        };
        for d in stacks {
            // The ItemEntity constructor's motion, from the entity's own
            // random (not the level's).
            self.spawn_item(d, at, dispenser::new_item_motion(), 10);
        }
    }
    /// Adds a projectile, giving it an id from the entity counter
    /// (`Entity.id`, the same counter as the rest) when it has none.
    pub(super) fn add_projectile(&mut self, mut p: horizon_client::projectiles::Projectile) {
        if p.id == 0 {
            self.next_entity += 1;
            p.id = self.next_entity;
        }
        // Projectile.setOwner: the acting player's or the mob's UUID, which
        // its save keeps even once the owner is gone.
        if p.owner_uuid.is_none() {
            p.owner_uuid = match p.owner {
                Some(Who::Player) => Some(self.acting_player_uuid()),
                Some(Who::Mob(id)) => Some(self.uuid_for(id)),
                None => None,
            };
        }
        self.creatures.projectiles.push(p);
    }
    /// `Entity.sendBubbleColumnParticles` at the top of the column block `pos`: two
    /// splashes and two bubbles over it, which the level random places, shown here
    /// and to the players within range.
    pub(super) fn bubble_column_particles(&mut self, pos: BlockPos) {
        for _ in 0..2 {
            let r = &mut self.world.random;
            let splash = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let bubble = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let splash_options = ParticleOptions::simple(ParticleType::Splash);
            self.particles
                .send_particles(&splash_options, splash, [0.0; 3], 1.0, 1);
            self.lan_level_particles(&splash_options, splash, [0.0; 3], 1.0, 1, None);
            let bubble_options = ParticleOptions::simple(ParticleType::Bubble);
            self.particles
                .send_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1);
            self.lan_level_particles(&bubble_options, bubble, [0.0, 0.01, 0.0], 0.2, 1, None);
        }
    }
    pub(super) fn handle_mob_events(&mut self, events: Vec<MobEvent>) -> Result<()> {
        for e in events {
            match e {
                MobEvent::BubbleColumnParticles { pos } => {
                    self.bubble_column_particles(BlockPos::new(pos[0], pos[1], pos[2]))
                }
                MobEvent::SquidCycle { mob, at } => self.lan_entity_event(mob as i32, 19, at),
                MobEvent::RabbitHop { mob, at } => self.lan_entity_event(mob as i32, 1, at),
                MobEvent::Drown { mob, at } => {
                    self.lan_entity_event(mob as i32, super::combat::DROWN_EVENT, at);
                    let v = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map_or([0.0; 3], |m| [m.velocity.x, m.velocity.y, m.velocity.z]);
                    self.drown_particles(at, v);
                }
                MobEvent::SunProtectionWear { mob } => self.wear_sun_protection(mob),
                MobEvent::ArmorHurt { mob, amount, kind } => self.hurt_mob_armor(mob, amount, kind),
                MobEvent::EntityEvent { mob, event, at } => {
                    self.lan_entity_event(mob as i32, event, at)
                }
                MobEvent::LevelParticles {
                    kind,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        self.particles
                            .send_particles(&options, at, spread, speed, count);
                        self.lan_level_particles(&options, at, spread, speed, count, None);
                    }
                }
                MobEvent::Sound(name, at, volume, pitch)
                | MobEvent::LocalSound(name, at, volume, pitch) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(at), volume, pitch)
                }
                MobEvent::LevelEventSound {
                    sound,
                    at,
                    volume,
                    spread,
                } => {
                    // WitherBoss.customServerAiStep: the spawn roar is global
                    // level event 1023 at the block position.
                    if sound == "entity.wither.spawn" {
                        let pos = at.map(|v| v.floor() as i32);
                        self.global_level_event(1023, pos, 0);
                    } else {
                        // The client's own level random: nothing drawn from the server's.
                        let mut r = horizon_core::random::LegacyRandom::new(
                            self.world.game_time.wrapping_mul(341_873_128_712),
                        );
                        let pitch = (r.next_float() - r.next_float()) * spread + 1.0;
                        self.audio
                            .play(&format!("minecraft:{sound}"), Some(at), volume, pitch)
                    }
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.audio.play(&name, Some(at), volume, pitch)
                }
                // The sculk layer tracks the game's steps itself
                // (`entity_motion_events`).
                MobEvent::GameEvent { .. } => {}
                MobEvent::AttackPlayer(amount, from) => {
                    self.mob_hits_player(amount, from, "minecraft:mob_attack");
                }
                // doHurtTarget: the effect only when hurtServer took damage.
                // Bee.doHurtTarget: a sting that hurt leaves the stinger in the
                // body, then the poison.
                MobEvent::StingPlayer {
                    amount,
                    from,
                    poison_ticks,
                } => {
                    if self.mob_hits_player(amount, from, "minecraft:mob_attack") {
                        self.combat.stuck.add_stinger();
                        if poison_ticks > 0 {
                            self.give_player_effect("minecraft:poison", poison_ticks, 0);
                        }
                    }
                }
                MobEvent::AttackPlayerWithEffect {
                    amount,
                    from,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    if self.mob_hits_player(amount, from, "minecraft:mob_attack") {
                        self.give_player_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::DamagePlayer { amount, from, kind } => {
                    self.mob_hits_player(amount, from, kind);
                }
                MobEvent::ElderCurse { at } => self.elder_curse(at),
                MobEvent::SetVelocity { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = v;
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = v;
                            }
                        }
                    }
                }
                MobEvent::RemoveKilled { target } => {
                    // The bite that killed it ran `LivingEntity.die` (loot, such
                    // as a magma cube's froglight, and experience) before
                    // `eatEntity` removes it: a removed mob never reaches the
                    // next tick's deaths, so it dies here first.
                    if let Some(m) = self
                        .mobs
                        .iter()
                        .find(|m| m.id == target && !m.alive() && !m.removed)
                        .cloned()
                    {
                        if self.died.insert(target) {
                            self.mob_died(&m);
                        }
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == target) {
                            m.removed = true;
                        }
                    }
                }
                MobEvent::Push { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.velocity = p.velocity.add_vector(v);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = m.velocity.add_vector(v);
                            }
                        }
                    }
                }
                MobEvent::Knockback {
                    target,
                    power,
                    x,
                    z,
                    source,
                } => match target {
                    // A player's source names no melee attacker, as its damage's doesn't.
                    Who::Player => {
                        let source = HurtSource {
                            damage_type: source_damage_type(source),
                            causing: 0,
                            direct: 0,
                        };
                        self.knock_back_player(power, x, z, source);
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.knockback(power, x, z, &source);
                        }
                    }
                },
                MobEvent::Shift { target, delta } => {
                    let d = Vec3::new(delta[0], delta[1], delta[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.player.as_mut() {
                                p.position = p.position.add_vector(d);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                                m.position = m.position.add_vector(d);
                            }
                        }
                    }
                }
                MobEvent::Wololo { mob } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        // DyeColor.RED.
                        m.extra.color = 14;
                    }
                }
                MobEvent::Fangs {
                    at,
                    yaw,
                    delay,
                    owner,
                } => self.add_fangs(at, yaw, delay, owner),
                MobEvent::Shoot(..) => {}
                MobEvent::ExplosionSource {
                    mob,
                    charged,
                    name,
                    in_water,
                } => {
                    self.combat.explosion_source = Some(mob);
                    self.combat.explosion_in_water = in_water;
                    // The creeper causes the blast's damage, but it is out of
                    // the mob list already: its name comes with the event.
                    self.combat.exploded.push((mob, name));
                    if charged {
                        self.combat.charged_exploders.push(mob);
                    }
                }
                MobEvent::LingeringCloud {
                    at,
                    effects,
                    hidden,
                    ambient,
                    color,
                } => {
                    self.spawn_creeper_cloud(at, effects, hidden, ambient, color);
                }
                MobEvent::Explode(at, radius, species) => {
                    // The exploding mob causes the damage (Creeper.explodeCreeper).
                    let noted = self.combat.explosion_source.is_some();
                    let source_mob = self.combat.explosion_source.or_else(|| {
                        self.mobs
                            .iter()
                            .filter(|m| {
                                (m.position.x - at[0]).powi(2) + (m.position.z - at[2]).powi(2)
                                    < 1.0
                            })
                            .map(|m| m.id)
                            .next()
                    });
                    self.combat.explosion_source = source_mob;
                    self.combat.explosion_source_named = noted;
                    self.combat.explosion_cause = match species {
                        Species::Creeper => Some("minecraft:creeper"),
                        Species::Wither => Some("minecraft:wither"),
                        Species::SulfurCube => Some("minecraft:sulfur_cube"),
                        _ => None,
                    }
                    .map(|kind| horizon_client::advancements::EntityView {
                        kind: kind.into(),
                        position: at,
                        ..Default::default()
                    });
                    // Only a creeper or a wither empties the hives it blows up.
                    // The creeper is already gone from the list, so the event
                    // carries its kind.
                    let result = if matches!(species, Species::Creeper | Species::Wither) {
                        self.with_hive_emptying_blast(|g| g.explode(at, radius))
                    } else {
                        self.explode(at, radius)
                    };
                    self.combat.explosion_source = None;
                    self.combat.explosion_source_named = false;
                    self.combat.explosion_in_water = false;
                    self.combat.explosion_cause = None;
                    result?
                }
                MobEvent::AttackMob {
                    target,
                    amount,
                    source,
                } => {
                    self.mob_hits_mob(target, amount, source);
                }
                MobEvent::CubeHurtPlayer { cube, amount, from } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.hurt_player_from(
                        amount,
                        Some(from),
                        "minecraft:mob_attack",
                        Some(Some(cube)),
                    ) && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                    }
                }
                MobEvent::CubeHurtMob {
                    cube,
                    target,
                    amount,
                    source,
                } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.mob_hits_mob(target, amount, source)
                        && let Some(m) = self.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                    }
                }
                MobEvent::SonicBoom {
                    target,
                    attacker,
                    from,
                    direction,
                } => {
                    // SonicBoom: the push only follows a hit that landed, scaled
                    // by the target's KNOCKBACK_RESISTANCE.
                    let (hurt, resistance) = match target {
                        Who::Player => {
                            let hurt = self.mob_hits_player(10.0, from, "minecraft:sonic_boom");
                            let resistance = self.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            });
                            (hurt, resistance)
                        }
                        Who::Mob(id) => {
                            let source = Damage {
                                kind: DamageKind::SonicBoom,
                                attacker: Some(Who::Mob(attacker)),
                                from: Some(from),
                            };
                            let hurt = self.mob_hits_mob(id, 10.0, source);
                            let resistance = self
                                .mobs
                                .iter()
                                .find(|m| m.id == id)
                                .map_or(0.0, |m| self.mob_knockback_resistance(m));
                            (hurt, resistance)
                        }
                    };
                    if hurt {
                        let horizontal = 2.5 * (1.0 - resistance);
                        let vertical = 0.5 * (1.0 - resistance);
                        let velocity = [
                            direction[0] * horizontal,
                            direction[1] * vertical,
                            direction[2] * horizontal,
                        ];
                        self.handle_mob_events(vec![MobEvent::Push { target, velocity }])?;
                    }
                }
                MobEvent::FinishAdmiring { mob } => self.finish_admiring(mob),
                MobEvent::Hiss { mob } => {
                    if let Some(m) = self.mobs.iter().find(|m| m.id == mob) {
                        let at = m.center();
                        let sound = horizon_client::mobs::finalize::variant_sound(m, "hiss")
                            .unwrap_or("entity.cat.hiss");
                        self.audio
                            .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
                    }
                }
                MobEvent::Tamed { .. } | MobEvent::Buck { .. } | MobEvent::StabDismount { .. } => {
                    self.mount_event(&e)
                }
                MobEvent::Launch { target, up } => match target {
                    Who::Player => {
                        if let Some(p) = self.player.as_mut() {
                            p.velocity.y += up;
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.velocity.y += up;
                        }
                    }
                },
                MobEvent::Bonemeal { pos } => {
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    if self.with_level(|b, level, ticks| b.grow_crop(level, ticks, at)) {
                        self.level_events
                            .push(horizon_world::level::LevelEvent::LevelEvent {
                                id: 1505,
                                pos: at,
                                data: 15,
                            });
                        let _ = self.process_level_events();
                    }
                }
                MobEvent::RingBell { bell } => {
                    let pos = BlockPos::new(bell[0], bell[1], bell[2]);
                    let rang = self
                        .with_level(|b, level, ticks| b.bell_attempt_to_ring(level, ticks, pos));
                    if rang {
                        let _ = self.process_level_events();
                    }
                }
                MobEvent::ChestOpen { chest, open } => self.golem_chest_open(chest, open),
                MobEvent::ChestSlot { chest, slot, stack } => {
                    self.golem_chest_slot(chest, slot, stack)
                }
                MobEvent::BecomeStatue { mob } => self.golem_become_statue(mob),
                MobEvent::EnterHive { mob, hive } => self.bee_enter_hive(mob, hive),
                MobEvent::ParrotToShoulder { mob } => self.parrot_to_shoulder(mob),
                MobEvent::Infested { mob } => self.mob_infested(mob),
                MobEvent::Gift { target, item } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == target)
                        && m.saddle_item().is_none()
                    {
                        m.set_saddle(Some(ItemStack::new(item, 1)));
                    }
                }
                MobEvent::HurtSelf { mob, kind, amount } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.damage(amount, horizon_client::mobs::Damage::of(kind));
                    }
                }
                MobEvent::SetTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.target = Some(target);
                    }
                }
                MobEvent::HuntHoglin { mob, target, ticks } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.ints[0] = ticks;
                        if m.ai.target.is_none() {
                            m.ai.target = Some(Who::Mob(target));
                        }
                    }
                }
                MobEvent::Projectile(mut p) => {
                    self.next_entity += 1;
                    p.id = self.next_entity;
                    let (owner, at) = (p.owner, [p.position.x, p.position.y, p.position.z]);
                    // LlamaSpit.recreateFromPacket: the spit's burst on the client.
                    if p.kind == ProjectileKind::LlamaSpit {
                        let v = p.velocity;
                        self.particles.llama_spit_burst(at, [v.x, v.y, v.z]);
                    }
                    self.add_projectile(p);
                    self.projectile_event("minecraft:projectile_shoot", owner, at);
                }
                MobEvent::Spawn(mut m) => {
                    self.next_entity += 1;
                    m.id = self.next_entity;
                    self.mobs.push(*m);
                    self.native_mobs_joined(self.mobs.len() - 1, false);
                }
                MobEvent::SpawnKind { kind, at, setup } => {
                    if let Some(id) = self.spawn_mob(&kind, at, setup.persistent) {
                        self.apply_setup(id, &setup);
                    }
                }
                MobEvent::Convert { from, kind, params } => {
                    let id = self.convert_to(&from, kind, params);
                    // `Piglin.finishConversion` and `Hoglin.finishConversion`:
                    // the converted mob is nauseous for 10 seconds.
                    if matches!(kind, "minecraft:zombified_piglin" | "minecraft:zoglin")
                        && let Some(m) = self.mobs.iter_mut().find(|m| Some(m.id) == id)
                    {
                        m.add_effect("minecraft:nausea", 200, 0);
                    }
                    // `Tadpole.ageUp`: the frog is finalized for the
                    // conversion and made persistent.
                    if kind == "minecraft:frog"
                        && let Some(id) = id
                    {
                        self.finalize_converted(id);
                    }
                }
                MobEvent::ThrowItem {
                    mob,
                    stack,
                    at,
                    motion,
                    pickup_delay,
                } => {
                    // `thrownItem.setThrower(the mob)`.
                    self.spawn_item(stack, at, motion, pickup_delay);
                    let thrower = self.mob_uuid(mob);
                    if let Some(thrown) = self.just_spawned_item() {
                        thrown.thrower = Some(thrower);
                    }
                }
                MobEvent::DropItem { item, count, at } => self.spawn_item(
                    ItemStack {
                        item,
                        count,
                        damage: 0,
                        extra: None,
                    },
                    at,
                    // The ItemEntity constructor's motion, from the entity's
                    // own random (not the level's), so spawnAtLocation drops
                    // (a fox's berries, a panda's held food, a goat's horn)
                    // land apart.
                    dispenser::new_item_motion(),
                    10,
                ),
                MobEvent::DropGoatHorn {
                    mob,
                    screaming,
                    at,
                    motion,
                } => {
                    let instrument =
                        horizon_client::mobs::goat::create_horn(self.mob_uuid(mob), screaming);
                    let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                    horn.data_mut().set_other(
                        "minecraft:instrument",
                        Some(horizon_world::nbt::Tag::String(instrument.into())),
                    );
                    // Goat.dropHorn's ItemEntity constructor leaves pickupDelay at
                    // 0 (no setDefaultPickUpDelay).
                    self.spawn_item(horn, at, motion, 0);
                }
                MobEvent::DropGift { table, at } => {
                    let stacks =
                        self.res
                            .items()
                            .loot_table(table, false, None, &mut self.world.random);
                    for stack in stacks {
                        // The ItemEntity constructor's motion, from the
                        // entity's own random (not the level's).
                        self.spawn_item(stack, at, dispenser::new_item_motion(), 10);
                    }
                }
                // `Level.setBlockAndUpdate`: neighbour and shape updates, so a crop
                // above trampled farmland breaks and drops.
                MobEvent::SetBlock { pos, state } => self.level_set(pos, state)?,
                MobEvent::CropRaided {
                    pos,
                    state,
                    old,
                    mob,
                } => {
                    self.level_set_with_flags(
                        pos,
                        state,
                        horizon_world::level::flags::UPDATE_CLIENTS,
                    )?;
                    if let Some(old) = old {
                        self.post_block_game_event(
                            "minecraft:block_change",
                            pos,
                            Some(Who::Mob(mob)),
                            None,
                        );
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                }
                MobEvent::CrackTurtleEgg { pos } => {
                    let state = self.world.block(pos[0], pos[1], pos[2]);
                    let eggs = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .filter(|s| s.name == "minecraft:turtle_egg")
                        .and_then(|s| s.properties.get("eggs"))
                        .and_then(|e| e.parse::<i32>().ok());
                    if let Some(eggs) = eggs {
                        let at = BlockPos::new(pos[0], pos[1], pos[2]);
                        // decreaseEggs: the break sound and the remaining
                        // eggs (a stack of one is destroyed after it).
                        self.with_level(|b, level, ticks| {
                            b.turtle_egg_destroyed_by_player(level, ticks, at, state);
                            if eggs <= 1 {
                                b.destroy_block(level, ticks, at, false);
                            }
                        });
                        self.process_level_events()?;
                    }
                }
                MobEvent::BedOccupied { pos, occupied } => self.set_bed_occupied(pos, occupied),
                MobEvent::SetDoorOpen { pos, open } => {
                    let pos = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_door_open(level, ticks, pos, open));
                    self.process_level_events()?;
                }
                MobEvent::BreakDoor { pos } => {
                    // BreakDoorGoal.tick: removeBlock, then level events 1021
                    // and 2001 with the state that is there now.
                    let air = self.world.air;
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_block(level, ticks, at, air, 3));
                    self.process_level_events()?;
                    self.level_event_at(1021, pos, 0);
                    let left = self.world.block(pos[0], pos[1], pos[2]);
                    self.level_event_at(2001, pos, left.0 as i32);
                }
                MobEvent::DestroyProgress { mob, pos, progress } => {
                    self.destroy_block_progress(mob as i32, pos, progress);
                    // ServerLevel.destroyBlockProgress: every player nearby sees it.
                    if let Some(host) = self.lan.as_ref() {
                        host.hub.broadcast_near(
                            [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                            64.0,
                            &horizon_protocol::packets::play::Clientbound::BlockDestruction(
                                horizon_protocol::packets::play::BlockDestruction {
                                    id: mob as i32,
                                    pos,
                                    progress: if (0..10).contains(&progress) {
                                        progress as u8
                                    } else {
                                        255
                                    },
                                },
                            ),
                            None,
                        );
                    }
                }
                MobEvent::LevelEvent { id, pos, data } => self.level_event_at(id, pos, data),
                MobEvent::SetBlockName {
                    pos,
                    name,
                    effect,
                    flags,
                } => {
                    if effect {
                        // EatBlockGoal passes the block's default state constant
                        // (Blocks.GRASS_BLOCK.defaultBlockState()), not the live state
                        // (which could be snowy).
                        let old = self.world.block(pos[0], pos[1], pos[2]);
                        let state = self
                            .pack
                            .assets
                            .registry
                            .default_state(&self.res.facts().get(old).name)
                            .unwrap_or(old);
                        self.level_event_at(2001, pos, state.0 as i32);
                    }
                    if let Some(state) = self.pack.assets.registry.default_state(name) {
                        self.level_set_with_flags(pos, state, flags)?;
                    }
                }
                MobEvent::Infest {
                    pos,
                    infested,
                    poof,
                } => {
                    let registry = &self.pack.assets.registry;
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    if let Some(mut state) = registry.state(old).cloned() {
                        let (from, to) = if infested {
                            ("minecraft:", "minecraft:infested_")
                        } else {
                            ("minecraft:infested_", "minecraft:")
                        };
                        state.name = state.name.replacen(from, to, 1);
                        if let Ok(id) = registry.resolve(&state) {
                            self.level_set(pos, id)?;
                        }
                    }
                    if let Some(mob) = poof {
                        self.mob_poof(mob);
                    }
                }
                MobEvent::BreakBlock { pos, drop } => {
                    let old = self.world.block(pos[0], pos[1], pos[2]);
                    // InfestedBlock.spawnAfterBreak: its silverfish comes out
                    // when blocks drop.
                    if drop
                        && self
                            .res
                            .facts()
                            .get(old)
                            .name
                            .starts_with("minecraft:infested_")
                        && self.ui.rules.bool("block_drops")
                    {
                        let at = [
                            f64::from(pos[0]) + 0.5,
                            f64::from(pos[1]),
                            f64::from(pos[2]) + 0.5,
                        ];
                        if let Some(id) = self.spawn_mob("minecraft:silverfish", at, false) {
                            self.mob_poof(id);
                        }
                    }
                    // Block.spawnDestroyParticles: fire breaks leave nothing.
                    if !self.res.facts().get(old).has_class("BaseFireBlock") {
                        self.level_event_at(2001, pos, old.0 as i32);
                    }
                    if drop {
                        let mut drops = world_items!(self).block_drops_in(
                            self.pack.assets.registry,
                            old,
                            None,
                            &mut self.world.random,
                            Some((&mut self.rand_sequences, self.level.seed)),
                        );
                        self.native_block_drops(pos, old, None, None, None, &mut drops);
                        for d in drops {
                            self.pop_resource(pos, d);
                        }
                    }
                    // Level.destroyBlock: setBlock(fluidState.createLegacyBlock(), 3),
                    // so water stays water and a waterlogged block leaves its water.
                    let b = self.res.behaviours();
                    let left = b.legacy_block(b.fluid(old));
                    if left != old {
                        self.level_set(pos, left)?;
                    }
                }
                MobEvent::Particles { kind, at, count } => {
                    // Mob particle bursts (hearts, smoke, splashes) around a point.
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        if kind.starts_with("item_") {
                            self.ensure_item_particle_sprites();
                        }
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        if ty == horizon_client::particles::ParticleType::SonicBoom {
                            self.lan_sonic_boom(&options, at);
                        }
                        // Count 0: one particle exactly there, at rest.
                        if count == 0 {
                            self.particles.add(&options, at, [0.0; 3]);
                        }
                        for _ in 0..count {
                            let r = &mut self.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.particles.add(&options, p, v);
                        }
                    }
                }
                MobEvent::AroundSelf {
                    mob,
                    event,
                    kind,
                    feet,
                    width,
                    height,
                    burst,
                } => {
                    self.lan_entity_event(mob as i32, event, feet);
                    self.particles_around(kind, feet, [width, height], burst)
                }
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) {
                        let options = horizon_client::particles::ParticleOptions::simple(ty);
                        self.particles.add(&options, at, velocity);
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    let options = horizon_client::particles::ParticleOptions::colored(
                        horizon_client::particles::ParticleType::EntityEffect,
                        [r, g, b, 1.0],
                    );
                    self.particles.add(&options, at, [0.0; 3]);
                }
                MobEvent::Experience { at, amount } => self.spawn_breeding_orb(at, amount),
                MobEvent::AxolotlSupportPlayer { mob } => {
                    let previous = self
                        .potions
                        .effects
                        .get("regeneration")
                        .map_or(0, |e| e.duration);
                    if (0..=2399).contains(&previous) {
                        self.potions
                            .effects
                            .add(horizon_client::effects::EffectInstance::new(
                                "regeneration",
                                (previous + 100).min(2400),
                                0,
                            ));
                    }
                    self.potions
                        .effects
                        .list
                        .retain(|e| e.id != "mining_fatigue");
                    let source = self
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(|m| self.mob_entity(m));
                    self.trigger_effects_changed_by(source);
                }
                MobEvent::DolphinTreasure { mob } => {
                    // ServerLevel.findNearestMapStructure(#dolphin_located, 50).
                    let Some(at) = self.mobs.iter().find(|m| m.id == mob).map(|m| {
                        [
                            m.position.x.floor() as i32,
                            m.position.y.floor() as i32,
                            m.position.z.floor() as i32,
                        ]
                    }) else {
                        continue;
                    };
                    let names = self.structure_tag("minecraft:dolphin_located");
                    let found = self
                        .end
                        .locator()
                        .filter(|_| !names.is_empty())
                        .and_then(|t| t.locate(&names, at, 50));
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.home = found.map(|(_, p)| p);
                    }
                }
                MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                } => {
                    self.give_player_effect(effect, ticks, amplifier);
                }
                MobEvent::Ignite { target, seconds } => match target {
                    Who::Player => {
                        if !self.creative
                            && let Some(p) = self.player.as_mut()
                        {
                            p.ignite_for_ticks(seconds * 20)?;
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                            m.ignite_for_ticks(seconds * 20);
                        }
                    }
                },
                MobEvent::Breed {
                    parent,
                    partner,
                    llama,
                } => self.breed(parent, partner, llama),
                MobEvent::LockGaze { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::animal_brain::lock_gaze(m, partner);
                    }
                }
                MobEvent::BreedTarget { mob, partner } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::animal_brain::set_breed_partner(m, partner);
                    }
                }
                MobEvent::Retreat { mob, from, at } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::piglin::receive_retreat(m, from, at);
                    }
                }
                MobEvent::StartRiding { rider, vehicle } => {
                    horizon_client::mobs::riders::start_riding(&mut self.mobs, rider, vehicle);
                }
                // Applied after the fish's tick (`apply_brain_events`).
                MobEvent::StartFollowing { .. } | MobEvent::SchoolSize { .. } => {}
                MobEvent::LeashBreak { mob, drop, sound } => self.break_leash(mob, drop, sound),
                MobEvent::NotifyLeashHolder { holder } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == holder) {
                        horizon_client::mobs::happy_ghast::notify_leash_holder(m);
                    }
                }
                MobEvent::Vibration { from, to, ticks } => {
                    self.mob_vibration_particle(from, to, ticks)
                }
                MobEvent::DigParticles { at } => self.warden_dig_particles(at),
                MobEvent::BlockBreakEffect { pos, block } => {
                    let state = self.world.block(block[0], block[1], block[2]);
                    self.destroy_particles(pos, state);
                }
                MobEvent::Darkness { at, radius } => self.apply_darkness(at, radius),
                MobEvent::CreakingHurt {
                    home,
                    by_player,
                    id,
                } => self.creaking_hurt(home, id, by_player),
                MobEvent::CreakingTearDown { center, spread } => {
                    self.creaking_tear_down(center, spread)
                }
                MobEvent::CureZombieVillager { mob } => self.cure_zombie_villager(mob),
                MobEvent::Reinforcements { mob, target } => self.call_reinforcements(mob, target),
                MobEvent::VillagerMeet { mob, other } => self.villager_meet(mob, other),
                MobEvent::VillagerOffers { mob } => {
                    if let Some(i) = self.mobs.iter().position(|m| m.id == mob) {
                        self.ensure_offers(i);
                    }
                }
                MobEvent::VillagerBirth {
                    parent,
                    partner,
                    child,
                } => self.villager_birth(parent, partner, child),
                MobEvent::WorkAtComposter { mob, pos } => self.villager_work_at_composter(mob, pos),
                MobEvent::GolemCheck { mob, needed } => self.villager_golem_check(mob, needed),
                MobEvent::Firework { at, color, flight } => {
                    self.villager_firework(at, color, flight)
                }
                MobEvent::ThrowGift { table, from, to } => self.throw_gift(&table, from, to),
                MobEvent::PatrolTarget { mob, target } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        horizon_client::mobs::raider::set_patrol_target(m, target);
                    }
                }
                MobEvent::RaiderTarget {
                    mob,
                    target,
                    aggressive,
                } => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.target = Some(target);
                        m.ai.aggressive |= aggressive;
                    }
                }
                MobEvent::PandaSneezed { mob } => self.panda_sneezed(mob),
                MobEvent::FallToPassengers {
                    mob,
                    distance,
                    modifier,
                } => self.fall_to_passengers(mob, distance, modifier),
                MobEvent::ShedScute { mob, at, pitch } => {
                    // The mob already checked `shouldDropLoot` (a baby, `mob_drops`).
                    let table = "minecraft:gameplay/armadillo_shed";
                    let drops = self.sequence_loot(table, table, None);
                    for d in drops {
                        self.spawn_at_location(Vec3::new(at[0], at[1], at[2]), 0.0, d);
                    }
                    self.audio.play(
                        "minecraft:entity.armadillo.scute_drop",
                        Some(at),
                        1.0,
                        pitch,
                    );
                    let ctx = self.who_context(Some(Who::Mob(mob)), None);
                    self.post_game_event("minecraft:entity_place", at, ctx);
                }
                MobEvent::GiftLoot { mob, table, at } => {
                    // Turtle.ageBoundaryReached and Panda.afterSneeze: only
                    // with mob loot on.
                    if matches!(
                        table,
                        "minecraft:gameplay/turtle_grow" | "minecraft:gameplay/panda_sneeze"
                    ) && !self.ui.rules.bool("mob_drops")
                    {
                        continue;
                    }
                    self.drop_gift_loot(mob, table, at);
                }
            }
        }
        Ok(())
    }
    /// `TurtleEggBlock.randomTick` at the last hatch stage: one baby turtle
    /// per egg, a day from grown, with the egg as its home (the sound, the
    /// crumbs and the block's removal came from the level).
    pub(super) fn hatch_turtles(
        &mut self,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
    ) {
        let eggs = self
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(1);
        for i in 0..eggs {
            let at = [
                f64::from(pos[0]) + 0.3 + f64::from(i) * 0.2,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.3,
            ];
            if let Some(id) = self.spawn_mob("minecraft:turtle", at, true)
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                m.extra.age = -24000;
                m.extra.home = Some(pos);
                m.yaw = 0.0;
                m.body_yaw = 0.0;
                m.head_yaw = 0.0;
            }
        }
    }
    fn apply_setup(&mut self, id: u32, setup: &SpawnSetup) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if setup.baby {
            m.extra.age = -24000;
        }
        if setup.nausea {
            m.add_effect("minecraft:nausea", 200, 0);
        }
        if setup.size > 0 {
            m.extra.size = setup.size;
        }
        m.yaw = setup.yaw;
        m.body_yaw = setup.yaw;
        m.head_yaw = setup.yaw;
        m.velocity = Vec3::new(setup.velocity[0], setup.velocity[1], setup.velocity[2]);
        if let Some(h) = setup.health {
            m.health = h;
        }
        if let Some(v) = setup.variant {
            m.extra.variant = v;
        }
        if let Some(owner) = setup.owner_mob {
            m.extra.counter2 = owner as i32;
        }
        if let Some(home) = setup.home {
            m.extra.home = Some(home);
        }
        if let Some(life) = setup.life {
            m.extra.egg_time = life;
        }
    }
    /// `Animal.finalizeSpawnChildFromBreeding`'s experience orb: only when the
    /// `mob_drops` game rule is on.
    fn spawn_breeding_orb(&mut self, at: [f64; 3], amount: u32) {
        if self.ui.rules.bool("mob_drops") {
            self.spawn_orbs(at, amount);
        }
    }
    /// `DyeColor.getMixedColor`: the colour of the crafting recipe that mixes
    /// the two dyes, else one of the two picked at random.
    fn dye_mix(&mut self, a: u8, b: u8) -> u8 {
        if let Some(mixed) = self.dye_color_mix_in_recipes(a, b) {
            return mixed;
        }
        if self.world.random.next_boolean() {
            a
        } else {
            b
        }
    }
    /// `DyeColor.findColorMixInRecipes`: the dye colour a two-dye crafting
    /// recipe makes, when its result is a dye.
    fn dye_color_mix_in_recipes(&self, a: u8, b: u8) -> Option<u8> {
        let dye = |c: u8| {
            let name = horizon_client::mobs::render::DYE_NAMES[usize::from(c) & 15];
            Some(ItemStack::new(&format!("minecraft:{name}_dye"), 1))
        };
        let grid = [dye(a), dye(b), None, None];
        let (_, result) = self.res.recipes().find(&grid, 2)?;
        let name = result
            .item
            .strip_prefix("minecraft:")?
            .strip_suffix("_dye")?;
        let index = horizon_client::mobs::render::DYE_NAMES
            .iter()
            .position(|n| *n == name)?;
        u8::try_from(index).ok()
    }
    /// `ServerPlayer.tellNeutralMobsThatIDied`: the mobs within 32 blocks
    /// across and 10 up and down get `NeutralMob.playerDied`, when the
    /// `forgive_dead_players` rule is on. A mob's `Who::Player` is the guest
    /// it is bound to in `bindings` (game/lan_mobs.rs), else the host's
    /// player, so only the mobs whose binding is `conn` (`None` for the
    /// host's player) hear it.
    pub(super) fn tell_neutral_mobs_player_died(
        &mut self,
        feet: [f64; 3],
        conn: Option<u32>,
        bindings: &horizon_client::fast_hash::FxHashMap<u32, u32>,
    ) {
        // A client of someone else's world has the server's mobs only to show.
        if self.remote.is_some() || !self.ui.rules.bool("forgive_dead_players") {
            return;
        }
        let block = feet.map(f64::floor);
        let area = Aabb::new(
            Vec3::new(block[0], block[1], block[2]),
            Vec3::new(block[0] + 1.0, block[1] + 1.0, block[2] + 1.0),
        )
        .inflate(Vec3::new(32.0, 10.0, 32.0));
        for m in self.mobs.iter_mut() {
            if bindings.get(&m.id).copied() == conn && m.bounds().intersects(area) {
                horizon_client::mobs::ai::forgive_dead_player(m);
            }
        }
    }
    /// Animal.spawnChildFromBreeding
    fn breed(
        &mut self,
        parent: u32,
        partner: u32,
        llama: Option<Box<horizon_client::mobs::llama::Offspring>>,
    ) {
        let Some(p) = self.mobs.iter().find(|m| m.id == parent).cloned() else {
            return;
        };
        // Animal.finalizeSpawnChildFromBreeding: the player who fed the parent,
        // or else the one who fed the partner.
        if let Some(cause) = breeding_credit(&mut self.love_cause, parent, partner) {
            let parent = self.mob_entity(&p);
            let partner_view = self
                .mobs
                .iter()
                .find(|m| m.id == partner)
                .map(|m| self.mob_entity(m))
                .unwrap_or_else(|| parent.clone());
            let mut child = parent.clone();
            child.baby = true;
            // TurtleBreedGoal.breed triggers it without a child.
            let turtle = matches!(p.species(), Species::Turtle | Species::Frog);
            if matches!(
                (
                    p.species(),
                    self.mobs
                        .iter()
                        .find(|m| m.id == partner)
                        .map(|m| m.species())
                ),
                (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse))
            ) {
                child.kind = "minecraft:mule".into();
            }
            let who = cause.map_or(
                super::commands::Target::Player,
                super::commands::Target::Guest,
            );
            self.for_player(who, |g| {
                g.stats.custom("animals_bred", 1);
                g.trigger_bred(&parent, &partner_view, (!turtle).then_some(&child));
            });
        }
        if let Some(q) = self.mobs.iter_mut().find(|m| m.id == partner) {
            q.extra.age = 6000;
            q.extra.in_love = 0;
        }
        let q = self.mobs.iter().find(|m| m.id == partner).cloned();
        // Animal.finalizeSpawnChildFromBreeding: entity event 18 for the parent.
        self.mob_event_particles(parent, 18);
        // Sniffer.spawnChildFromBreeding: an egg instead of a baby.
        if p.species() == Species::Sniffer {
            self.lay_sniffer_egg(&p);
            return;
        }
        // TurtleBreedGoal.breed: the goal gave the mother her egg; only the
        // experience is left, and only with mob loot on.
        if matches!(p.species(), Species::Turtle | Species::Frog) {
            // One `new ExperienceOrb` of 1 to 7 from the mother's random (not
            // split up as `ExperienceOrb.award` does).
            if self.ui.rules.bool("mob_drops")
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == parent)
            {
                let xp = 1 + m.random.next_int_bound(7).unwrap_or(0) as u32;
                self.add_orb([p.position.x, p.position.y, p.position.z], xp);
            }
            return;
        }
        // (Mooshroom draws its own coin in getOffspringVariant.)
        let coin = p.species() != Species::Mooshroom
            && self.world.random.next_int_bound(2).unwrap_or(0) == 0;
        // Axolotl.getBreedOffspring: useRareVariant (one in 1200) makes it blue,
        // else either parent's variant. The draws come from the mother's random.
        let axolotl = if p.species() == Species::Axolotl {
            let other = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if m.random.next_int_bound(1200).unwrap_or(1) == 0 {
                    // Axolotl.Variant.getRareSpawnVariant: the one blue, still a draw.
                    let _ = m.random.next_int_bound(1);
                    4
                } else if m.random.next_boolean() {
                    p.extra.variant
                } else {
                    other
                }
            })
        } else {
            None
        };
        // MushroomCow.getOffspringVariant, from the mother's random: the other
        // colour 1 time in 1024 when both parents share one, otherwise either
        // parent's colour by a coin flip.
        let mooshroom = if p.species() == Species::Mooshroom {
            let mate = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            let own = p.extra.variant;
            self.mobs.iter_mut().find(|m| m.id == parent).map(|m| {
                if own == mate && m.random.next_int_bound(1024).unwrap_or(1) == 0 {
                    if own == 1 { 0 } else { 1 }
                } else if m.random.next_boolean() {
                    own
                } else {
                    mate
                }
            })
        } else {
            None
        };
        // A horse and a donkey have a mule.
        let kind = match (p.species(), q.as_ref().map(|q| q.species())) {
            (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse)) => {
                "minecraft:mule".to_owned()
            }
            _ => p.kind.id.clone(),
        };
        if let Some(id) = self.spawn_mob(&kind, [p.position.x, p.position.y, p.position.z], true)
            && let Some(child) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            child.extra.age = -24000;
            // No Mob.finalizeSpawn either: no follow range bonus, right-handed.
            spawning::bred_offspring(child);
            // getBreedOffspring creates the baby without finalizeSpawn: it keeps
            // the default (`classic`) sound variant.
            child.extra.sound_variant = None;
            // Fox.getBreedOffspring runs no finalizeSpawn: nothing in the
            // baby fox's mouth.
            if p.species() == Species::Fox {
                child.extra.main_hand = None;
            }
            // Fox.FoxBreedGoal.breed: the baby trusts the players who fed its
            // parents into love.
            if p.species() == Species::Fox {
                horizon_client::mobs::fox::trust_breeders(
                    child,
                    p.extra.love_cause,
                    q.as_ref().and_then(|q| q.extra.love_cause),
                );
            }
            // Goat.getBreedOffspring: GoatAi.initMemories, then the baby
            // screams if the parent a coin toss picks does, or on a 2% roll.
            if p.species() == Species::Goat {
                child.extra.lost_horns = [false; 2];
                horizon_client::mobs::finalize::goat_init_memories(child, &mut self.world.random);
                let pick_parent = self.world.random.next_boolean();
                let parent_screams = if pick_parent {
                    p.extra.screaming
                } else {
                    q.as_ref().unwrap_or(&p).extra.screaming
                };
                child.extra.screaming = parent_screams || self.world.random.next_double() < 0.02;
            }
            // A llama's baby was drawn in the parent's tick, with the
            // experience.
            if let Some(o) = llama {
                horizon_client::mobs::llama::apply_offspring(child, &o);
                return;
            }
            if horizon_client::mobs::equine::is_equine(p.species()) {
                let other = q.clone().unwrap_or_else(|| p.clone());
                let mut r = self.world.random.clone();
                horizon_client::mobs::equine::inherit(child, &p, &other, &mut r);
                if horizon_client::mobs::llama::is_llama(p.species()) {
                    horizon_client::mobs::llama::offspring(child, &p, &other, &mut r);
                }
                // Horse.getBreedOffspring: coat from either parent or new.
                if p.species() == Species::Horse && other.species() == Species::Horse {
                    let c = r.next_int_bound(9).unwrap_or(0);
                    child.extra.variant = if c < 4 {
                        p.extra.variant
                    } else if c < 8 {
                        other.extra.variant
                    } else {
                        r.next_int_bound(7).unwrap_or(0) as u8
                    };
                    let mk = r.next_int_bound(5).unwrap_or(0);
                    child.extra.variant2 = if mk < 2 {
                        p.extra.variant2
                    } else if mk < 4 {
                        other.extra.variant2
                    } else {
                        r.next_int_bound(5).unwrap_or(0) as u8
                    };
                }
                self.world.random = r;
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            if p.species() == Species::Cat {
                // Cat.getBreedOffspring.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    child.extra.tame = true;
                    child.extra.owner = p.extra.owner;
                    // The mix needs all of `self`, so it waits until `child`'s
                    // borrow of the mob list is over.
                    let mixed = self.dye_mix(p.extra.color, other.extra.color);
                    if let Some(c) = self.mobs.iter_mut().find(|m| m.id == id) {
                        c.extra.color = mixed;
                    }
                }
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            if p.species() == Species::Wolf {
                // Wolf.getBreedOffspring: a parent's variant; tame parents
                // give a tame baby with their collar colours mixed.
                let other = q.as_ref().unwrap_or(&p);
                child.extra.variant = if coin {
                    p.extra.variant
                } else {
                    other.extra.variant
                };
                if p.extra.tame {
                    horizon_client::mobs::tame::set_tame(child, true);
                    child.extra.owner = p.extra.owner;
                    // The mix needs all of `self`, so it waits until `child`'s
                    // borrow of the mob list is over.
                    let mixed = self.dye_mix(p.extra.color, other.extra.color);
                    if let Some(c) = self.mobs.iter_mut().find(|m| m.id == id) {
                        c.extra.color = mixed;
                    }
                }
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            // Panda.getBreedOffspring: genes from both parents.
            if p.species() == Species::Panda {
                horizon_client::mobs::panda::inherit(child, &p, q.as_ref().unwrap_or(&p));
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            // Axolotl.getBreedOffspring: the drawn variant, and the baby is persistent.
            if let Some(variant) = axolotl {
                child.extra.variant = variant;
                child.persistent = true;
                let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
                self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
                return;
            }
            // Nautilus.getBreedOffspring: a tame parent's baby is tame too.
            if p.species() == Species::Nautilus && p.extra.tame {
                child.extra.tame = true;
                child.extra.owner = p.extra.owner;
                child.persistent = true;
            }
            // Offspring inherit a parent's variant (pigs/cows/chickens) or mixed wool.
            child.extra.variant = p.extra.variant;
            // Fox, Pig, Cow and Chicken.getBreedOffspring: either parent's variant.
            if matches!(
                p.species(),
                Species::Fox | Species::Pig | Species::Cow | Species::Chicken
            ) && !coin
            {
                child.extra.variant = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            }
            if let Some(variant) = mooshroom {
                child.extra.variant = variant;
            }
            // Sheep.getBreedOffspring: the lamb's wool mixes both parents' dyes.
            if p.species() == Species::Sheep {
                let other = q.as_ref().unwrap_or(&p);
                let mixed = self.dye_mix(p.extra.color, other.extra.color);
                if let Some(c) = self.mobs.iter_mut().find(|m| m.id == id) {
                    c.extra.color = mixed;
                }
            }
        }
        let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
        self.spawn_breeding_orb([p.position.x, p.position.y, p.position.z], xp);
    }
    /// `MobEffectUtil.addEffectToPlayersAround` for an elder guardian:
    /// mining fatigue III for five minutes unless a stronger or longer one
    /// is on, with the curse sound and the elder's apparition.
    fn elder_curse(&mut self, at: [f64; 3]) {
        let mut cursed = Vec::new();
        self.for_each_player(|g| {
            if g.elder_curse_player(at) {
                cursed.push(g.lan_acting);
            }
        });
        // A guest's client shows the apparition and plays the curse itself
        // (ClientboundGameEventPacket.GUARDIAN_ELDER_EFFECT).
        if let Some(host) = self.lan.as_ref() {
            for conn in cursed.into_iter().flatten() {
                host.hub.send_to(
                    conn,
                    &horizon_protocol::packets::play::Clientbound::GameEvent(
                        horizon_protocol::packets::play::GameEvent {
                            event: horizon_client::lan::hub::game_event::GUARDIAN_ELDER_EFFECT,
                            param: 1.0,
                        },
                    ),
                );
            }
        }
    }
    /// The curse on the player in place; whether it took.
    fn elder_curse_player(&mut self, at: [f64; 3]) -> bool {
        if self.creative || self.spectator() {
            return false;
        }
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let d = p.position.subtract(Vec3::new(at[0], at[1], at[2]));
        if d.x * d.x + d.y * d.y + d.z * d.z >= 2500.0 || p.health.dead_or_dying() {
            return false;
        }
        // The player's effects are kept by their bare name. A curse as strong
        // that won't end within the next minute stays (endsWithin(1199)).
        let active = self.potions.effects.get("mining_fatigue");
        if active.is_some_and(|e| e.amplifier >= 2 && !e.ends_within(1199)) {
            return false;
        }
        let at = [p.position.x, p.position.y, p.position.z];
        self.give_player_effect("minecraft:mining_fatigue", 6000, 2);
        if self.lan_acting.is_none() {
            self.audio
                .play("minecraft:entity.elder_guardian.curse", Some(at), 1.0, 1.0);
            self.creatures.elder_apparition = super::mob_blocks::APPARITION_TICKS;
        }
        true
    }
    /// Damage of a given type to the player from a point.
    pub(super) fn mob_hits_player_kind(&mut self, amount: f32, from: [f64; 3], kind: &str) {
        self.mob_hits_player(amount, from, kind);
    }
    /// Damage whose causing mob is known (`cause`, or none): not guessed
    /// from the mob nearest to `from`.
    pub(super) fn mob_hits_player_caused(
        &mut self,
        amount: f32,
        from: [f64; 3],
        kind: &str,
        cause: Option<u32>,
    ) {
        self.hurt_player_from(amount, Some(from), kind, Some(cause));
    }
    pub(super) fn mob_hits_player(
        &mut self,
        amount: f32,
        from: [f64; 3],
        damage_type: &str,
    ) -> bool {
        self.hurt_player_from(amount, Some(from), damage_type, None)
    }
    /// `hurtServer` on the player. `from` is the source's position (none: no
    /// direction), `known`: the causing mob when it is given (`/damage`),
    /// else the melee attacker is the mob nearest to `from`.
    pub(super) fn hurt_player_from(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        damage_type: &str,
        known: Option<Option<u32>>,
    ) -> bool {
        let post_attack_if_alive = std::mem::take(&mut self.combat.post_attack_if_alive);
        let rocket = std::mem::take(&mut self.combat.rocket_hit);
        // ServerPlayer.canHarmPlayer refused another player's projectile.
        if self.creative || self.combat.refuse_player_hit {
            self.combat.forget_noted(false);
            return false;
        }
        // A projectile entity dealt it: `#is_projectile`, or a firework
        // rocket (its `fireworks` type is not in the tag).
        let projectile = rocket
            || self
                .res
                .stations()
                .enchantments
                .damage_tags
                .get("minecraft:is_projectile")
                .is_some_and(|t| t.contains(damage_type));
        let blocking = self.player_blocking(from, damage_type, false);
        let raised = blocking.is_some();
        // Melee: the attacker stands at `from` (another player's hit has no mob behind it).
        let attacker = match known {
            Some(known) => known,
            None => from
                .filter(|_| !projectile && damage_type != "minecraft:player_attack")
                .and_then(|from| {
                    self.mobs
                        .iter()
                        .filter(|m| m.alive())
                        .min_by(|a, b| {
                            let d = |m: &horizon_client::mobs::Mob| {
                                (m.position.x - from[0]).powi(2) + (m.position.z - from[2]).powi(2)
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .map(|m| m.id)
                }),
        };
        // The causing entity for the death message.
        let owner = self.combat.projectile_owner.take();
        if let Some(a) = attacker.or(owner) {
            self.note_attacker(damage_type, a);
        }
        if let Some((id, name)) = self.combat.projectile_attacker.take() {
            self.note_named_attacker(damage_type, id, name);
        }
        // The projectile is the direct entity: named when it has no owner.
        if let Some(name) = self.combat.projectile_direct.take() {
            self.note_direct_entity(damage_type, name);
        }
        // DamageSource.scalesWithDifficulty: a mob (or its projectile's
        // owner) caused the hit, not a player.
        let living_cause = attacker.or(owner).is_some();
        // The native damage hook's causing entity: the one given, or the
        // owner of the projectile that hit (`projectile_owner` outlives a
        // projectile's other hits), not the guessed melee attacker.
        let owner = owner.filter(|_| projectile);
        let causing = known.flatten().or(owner).map_or(0, |id| id as i32);
        let mut outcome = horizon_client::health::BlockOutcome::default();
        // The hit's own push, held back until after the shield's (`hurtServer`
        // runs `applyItemBlocking` before `dealDefaultKnockback`).
        let mut default_push: Option<([f64; 2], HurtSource<'_>)> = None;
        if let Some(player) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(damage_type)
            && let Ok(o) = player.hurt_caused_by(
                &self.motion.health,
                self.world.health_rules,
                id,
                amount,
                blocking,
                living_cause,
                causing,
            )
            && {
                outcome = o;
                o.hurt
            }
        {
            // LivingEntity.hurtServer's dealDefaultKnockback: not inside the
            // hurt cooldown (a stronger hit there only adds its difference).
            if o.knockback {
                // No source position: a random direction.
                let (xd, zd) = from.map_or((0.0, 0.0), |from| {
                    (from[0] - player.position.x, from[2] - player.position.z)
                });
                // dealDefaultKnockback's `indicateDamage`, unless blocked.
                if o.blocked <= 0.0 {
                    player.health.indicate_damage(xd, zd, player.yaw);
                }
                let source = HurtSource {
                    damage_type,
                    causing,
                    direct: 0,
                };
                default_push = Some(([xd, zd], source));
            }
            // The thorns hurt the entity that caused the hit, if there is one.
            let killed = self.player.as_ref().is_some_and(|p| {
                super::wither::killed(true, p.health.dead_or_dying(), p.health.totem_used)
            });
            if let Some(a) = attacker.or(owner)
                && !(post_attack_if_alive && killed)
            {
                self.thorns(a);
            }
        }
        // LivingEntity.hurtServer: ENTITY_HURT_PLAYER, unless it returned
        // early (invulnerable, or inside the hurt cooldown).
        // A hit a native mod canceled records no damage (hurtServer never ran).
        self.combat.forget_noted(outcome.canceled);
        if outcome.hurt || outcome.sound {
            let source = attacker
                .or(owner)
                .and_then(|id| self.mobs.iter().find(|m| m.id == id))
                .map(|m| self.mob_entity(m));
            self.trigger_entity_hurt_player(&horizon_client::advancements::DamageView {
                kind: damage_type.to_owned(),
                // Arrows and the like name no direct entity here; a
                // rocket's burst names the rocket.
                direct: if rocket {
                    Some(horizon_client::advancements::EntityView {
                        kind: "minecraft:firework_rocket".to_owned(),
                        position: from.unwrap_or_default(),
                        ..Default::default()
                    })
                } else {
                    source.clone().filter(|_| !projectile)
                },
                is_direct: !projectile,
                source,
                original: amount,
                actual: amount - outcome.blocked,
                blocked: outcome.blocked > 0.0,
            });
        }
        if raised {
            let source = HurtSource {
                damage_type,
                causing,
                direct: 0,
            };
            self.after_block(outcome, (attacker, projectile), source);
        }
        // `LivingEntity.dealDefaultKnockback`: after the item's block push, so
        // a half-blocked hit pushes the defender back the way vanilla does.
        if let Some(([xd, zd], source)) = default_push {
            self.knock_back_player(f64::from(0.4_f32), xd, zd, source);
            if let Some(p) = self.player.as_mut()
                && let Some(r) = p.hurt_log.last_mut()
            {
                r.knockback = Some([p.velocity.x, p.velocity.y, p.velocity.z]);
            }
        }
        outcome.hurt
    }
    /// `LivingEntity.knockback` on the player: pushed `power` away from the
    /// direction (`xd`, `zd`), less its knockback resistance, and up only from
    /// the ground.
    /// The native knockback hook runs once the resistance is applied, with
    /// `source`, the hit's.
    pub(super) fn knock_back_player(
        &mut self,
        power: f64,
        xd: f64,
        zd: f64,
        source: HurtSource<'_>,
    ) {
        if let Some(p) = self.player.as_mut() {
            knock_back(p, &mut self.world.random, power, (xd, zd), source);
        }
    }
    /// A status effect from a mob, arrow or splash potion (`addEffect`).
    pub(super) fn give_player_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        if self.creative || self.spectator() {
            return;
        }
        let id = effect.strip_prefix("minecraft:").unwrap_or(effect);
        self.potions
            .effects
            .add(horizon_client::effects::EffectInstance::new(
                id, ticks, amplifier,
            ));
    }
    /// `WitherRoseBlock.entityInside` for the player: on a server level that
    /// isn't peaceful, `getBeeInteractionEffect` (Wither, 40 ticks) unless the
    /// player is invulnerable to wither damage (creative, spectator). A
    /// server's player gets it from the server.
    pub(super) fn wither_rose_contact(&mut self) {
        if self.remote.is_some() || self.ui.difficulty == 0 {
            return;
        }
        self.give_player_effect("minecraft:wither", 40, 0);
    }
    pub(super) fn hurt_player_by(&mut self, damage_type: &str, amount: f32) {
        let mut canceled = false;
        if !self.creative
            && let Some(player) = self.player.as_mut()
            && let Some(id) = self.motion.health.damage.id(damage_type)
            && let Ok(o) = player.hurt_blocking(
                &self.motion.health,
                self.world.health_rules,
                id,
                amount,
                None,
            )
        {
            canceled = o.canceled;
        }
        self.combat.forget_noted(canceled);
    }
    /// `Player.aiStep` touching arrows in its box grown by (1, 0.5, 1):
    /// `AbstractArrow.playerTouch`, then `Player.take`'s pickup sound at the
    /// arrow (`ClientPacketListener.handleTakeItemEntity`).
    pub(super) fn touch_arrows(&mut self) {
        use horizon_client::arrows::Touch;
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        // Player.hasInfiniteMaterials.
        let creative = self.creative;
        let mut projectiles = std::mem::take(&mut self.creatures.projectiles);
        let mut picked = Vec::new();
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let at = shot.position;
            let bounds = Aabb::new(
                Vec3::new(at.x - 0.25, at.y, at.z - 0.25),
                Vec3::new(at.x + 0.25, at.y + 0.5, at.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                // Inventory.add: a creative player takes what does not fit.
                Touch::Give(stack) => self.give(stack).is_none() || creative,
            };
            if taken {
                let r = &mut self.world.random;
                let pitch = (r.next_float() - r.next_float()) * 1.4 + 2.0;
                self.audio.play(
                    "minecraft:entity.item.pickup",
                    Some([at.x, at.y, at.z]),
                    0.2,
                    pitch,
                );
                picked.push(shot.clone());
            }
            !taken
        });
        self.creatures.projectiles = projectiles;
        for shot in picked {
            self.add_pickup(
                super::pickups::Taken::Arrow(Box::new(shot)),
                super::pickups::Collector::Player,
            );
        }
    }

    fn tick_projectiles(&mut self) -> Result<()> {
        let mut projectiles = std::mem::take(&mut self.creatures.projectiles);
        let mut hits = Vec::new();
        let mut deflections: Vec<[f64; 3]> = Vec::new();
        let mut arrow_events = Vec::new();
        let mut trails = Vec::new();
        let mut keep_flags = vec![true; projectiles.len()];
        // The impacts the mods may cancel (`native_projectile_impact`), on a server.
        let hear = self.native_hears_impacts();
        let mut impacts: Vec<Impact> = Vec::new();
        {
            let targets = self.projectile_targets();
            let breezes: Vec<u32> = self
                .mobs
                .iter()
                .filter(|m| m.alive() && m.species() == Species::Breeze)
                .map(|m| m.id)
                .collect();
            // Drawn only when a breeze could deflect a projectile: an empty
            // level must not move the level random every tick.
            let jitter = if breezes.is_empty() || projectiles.is_empty() {
                0.0
            } else {
                self.world.random.next_float()
            };
            let w = self.mob_world_base(&[]);
            let rain = self.arrow_rain();
            let scene = self.arrow_scene(&rain);
            // Arrows also hit minecarts (VehicleEntity is pickable).
            let mut arrow_targets = targets.clone();
            arrow_targets.extend(self.carts.iter().map(|c| (Who::Mob(c.id), c.bounds())));
            // Mobs' projectiles also hit the guests (game/lan_mobs.rs).
            let guests = self.lan_projectile_targets();
            let mut arrow_targets_guests = arrow_targets.clone();
            arrow_targets_guests.extend(guests.iter().copied());
            let mut targets_guests = targets.clone();
            targets_guests.extend(guests);
            let pw = ProjWorld { w: &w, targets };
            let pw_guests = ProjWorld {
                w: &w,
                targets: targets_guests,
            };
            let guest_entities = self.lan_guest_entities();
            // ThrownEnderpearl.tick: a dead player's pearl is discarded in
            // place of its tick while ender_pearls_vanish_on_death is on.
            let vanish = self.ui.rules.bool("ender_pearls_vanish_on_death");
            let host_dead = self.player_view().is_some_and(|p| !p.alive);
            let dead_guests: Vec<u32> = self.lan.as_ref().map_or_else(Vec::new, |h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| g.dead)
                    .map(|g| g.conn)
                    .collect()
            });
            for i in 0..projectiles.len() {
                // A wind charge is pickable, so a pearl, an arrow or a trident can
                // strike it (`Projectile.isPickable`, `Projectile.onHit`).
                let winds = if matches!(projectiles[i].kind, ProjectileKind::EnderPearl)
                    || projectiles[i].arrow.is_some()
                {
                    wind_charge_hitboxes(&projectiles, &keep_flags, i)
                } else {
                    Vec::new()
                };
                let p = &mut projectiles[i];
                if vanish
                    && matches!(p.kind, ProjectileKind::EnderPearl)
                    && p.owner == Some(Who::Player)
                    && p.guest.map_or(host_dead, |c| dead_guests.contains(&c))
                {
                    keep_flags[i] = false;
                    continue;
                }
                // The projectile as it was, to fly on if a mod cancels its impact.
                let before = (hear && !p.stuck).then(|| p.clone());
                let mob_shot = !matches!(p.owner, Some(Who::Player));
                // A player's arrows, snowballs and eggs meet every player; a
                // guest's flies from that guest (Projectile.getOwner), not
                // from the host, until it has left them.
                let pvp = !mob_shot && super::lan_pvp::meets_players(&p.kind);
                let shooter = p
                    .guest
                    .filter(|_| pvp)
                    .and_then(|c| guest_entities.iter().find(|(g, _)| *g == c))
                    .map(|&(_, id)| Who::Mob(id));
                if shooter.is_some() {
                    p.owner = shooter;
                }
                let (keep, hit) = match (p.arrow.is_some(), mob_shot || pvp, winds.is_empty()) {
                    (true, false, true) => horizon_client::arrows::tick(p, &scene, &arrow_targets),
                    (true, true, true) => {
                        horizon_client::arrows::tick(p, &scene, &arrow_targets_guests)
                    }
                    (true, with_guests, false) => {
                        let mut targets = if with_guests {
                            arrow_targets_guests.clone()
                        } else {
                            arrow_targets.clone()
                        };
                        targets.extend(winds);
                        horizon_client::arrows::tick(p, &scene, &targets)
                    }
                    (false, false, true) => p.tick(&pw),
                    (false, true, true) => p.tick(&pw_guests),
                    (false, pvp_shot, false) => {
                        let mut targets = if pvp_shot {
                            pw_guests.targets.clone()
                        } else {
                            pw.targets.clone()
                        };
                        targets.extend(winds);
                        p.tick(&ProjWorld {
                            w: if pvp_shot { pw_guests.w } else { pw.w },
                            targets,
                        })
                    }
                };
                if shooter.is_some() {
                    p.owner = Some(Who::Player);
                }
                // AbstractHurtingProjectile.tick ends with createParticleTrail.
                if super::remote_projectiles::has_smoke_trail(&p.kind) {
                    trails.push(p.position);
                }
                // ThrownEnderpearl.onAboveBubbleColumn / onInsideBubbleColumn.
                if keep && hit == Hit::None && matches!(p.kind, ProjectileKind::EnderPearl) {
                    let (at, h) = (p.position, 0.125);
                    horizon_client::motion::bubble_column_push(
                        w.world,
                        w.registry,
                        w.facts,
                        w.collision,
                        Aabb::new(
                            Vec3::new(at.x - h, at.y, at.z - h),
                            Vec3::new(at.x + h, at.y + 2.0 * h, at.z + h),
                        ),
                        &mut p.velocity,
                    );
                }
                let events_from = arrow_events.len();
                if let Some(a) = p.arrow.as_mut() {
                    arrow_events.extend(a.events.drain(..).map(|e| (p.position, e)));
                }
                let wind = matches!(
                    p.kind,
                    ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                );
                // Entity.deflection: a breeze turns every projectile but wind charges away.
                if let Hit::Entity {
                    who: Who::Mob(id),
                    at,
                } = hit
                    && !wind
                    && breezes.contains(&id)
                {
                    super::wind::reverse(p, jitter);
                    if p.arrow.is_some() {
                        horizon_client::arrows::finish_entity_hit(p, &scene, EntityHit::Deflected);
                        let a = p.arrow.as_mut().expect("an arrow");
                        arrow_events.extend(a.events.drain(..).map(|e| (p.position, e)));
                    }
                    deflections.push([at.x, at.y, at.z]);
                    continue;
                }
                // AbstractWindCharge.tick: out of the world's top it bursts.
                if wind && keep && p.position.y > 350.0 {
                    hits.push((
                        p.clone(),
                        Hit::Block {
                            pos: [0; 3],
                            at: p.position,
                            face: [0; 3],
                        },
                    ));
                    keep_flags[i] = false;
                    continue;
                }
                if hit != Hit::None {
                    if let Some(before) = before {
                        impacts.push(Impact {
                            index: i,
                            before,
                            hit,
                            hit_slot: hits.len(),
                            events: events_from..arrow_events.len(),
                        });
                    }
                    hits.push((p.clone(), hit));
                }
                keep_flags[i] = keep;
                // Projectile.onHit: the wind charge it hit turns before its own tick.
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(j) = wind_charge_hit(id)
                {
                    let (owner, owner_uuid, guest) = (p.owner, p.owner_uuid, p.guest);
                    let look = self.owner_look(owner, guest);
                    let wind = &mut projectiles[j];
                    super::deflect::aim_deflect(wind, look, owner, owner_uuid, guest);
                }
            }
        }
        let listed = keep_flags.clone();
        let mut idx = 0;
        projectiles.retain(|_| {
            let k = keep_flags[idx];
            idx += 1;
            k
        });
        if !impacts.is_empty() {
            self.native_settle_impacts(
                &mut projectiles,
                &listed,
                impacts,
                &mut hits,
                &mut arrow_events,
            );
        }
        self.smoke_trails(trails);
        for at in deflections {
            self.audio
                .play("minecraft:entity.breeze.deflect", Some(at), 1.0, 1.0);
        }
        for (at, event) in arrow_events {
            self.arrow_event(at, event);
        }
        self.creatures.projectiles.append(&mut projectiles);
        for (p, hit) in hits {
            // A player's arrow on another player lands as the victim's hurt.
            if let Some(victim) = self.projectile_pvp_victim(&p, hit) {
                self.projectile_hits_player(&p, hit, victim)?;
                continue;
            }
            // A guest's projectile lands as theirs: a pearl takes them, what
            // it hurts answers to them. One hitting the host (`Who::Player`
            // here) stays the host's hit.
            match p.guest {
                Some(conn)
                    if !matches!(
                        hit,
                        Hit::Entity {
                            who: Who::Player,
                            ..
                        }
                    ) =>
                {
                    self.guest_projectile_hit(conn, &p, hit)?;
                }
                _ => self.projectile_hit(&p, hit)?,
            }
        }
        self.flush_lan_explosions();
        self.flush_wind_bursts()
    }
    /// `LivingEntity.hurtServer`'s `setLastHurtByPlayer` for a projectile a
    /// player owns (a fireball they turned back): the kill is theirs, so a
    /// guest's (the host on its server thread too) reaches their statistics
    /// and `player_killed_entity`, as the arrow's hit does.
    fn player_hurt_mob(&mut self, p: &Projectile, id: u32) {
        if p.owner == Some(Who::Player) {
            self.last_hurt_by_player.insert(id, self.lan_acting);
        }
    }
    /// `projectile_hit` for a projectile the guest `conn` shot, run as them;
    /// as the host's when they have left.
    fn guest_projectile_hit(&mut self, conn: u32, p: &Projectile, hit: Hit) -> Result<()> {
        let Some(mut host) = self.lan.take() else {
            return self.projectile_hit(p, hit);
        };
        let result = self.as_guest(&mut host, conn, |g| g.projectile_hit(p, hit));
        // The mob hit now answers to the guest (LivingEntity.lastHurtByPlayer).
        if result.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.mobs.iter().any(|m| m.id == id)
        {
            host.mob_players.insert(id, conn);
        }
        self.lan = Some(host);
        match result {
            Some(r) => r,
            None => self.projectile_hit(p, hit),
        }
    }
    /// What an arrow or a thrown trident hits an entity of type `victim`
    /// for (`AbstractArrow.onHitEntity`, `ThrownTrident.onHitEntity`): a
    /// trident's 8 through `EnchantmentHelper.modifyDamage` of the trident
    /// (Impaling's `minecraft:damage` on `#sensitive_to_impaling`).
    fn shot_damage(&mut self, shot: &mut Projectile, victim: Option<&str>) -> f32 {
        if !matches!(shot.kind, ProjectileKind::Trident) {
            return arrow_damage(shot, &mut self.world.random) + shot.bonus.damage;
        }
        let Some(trident) = shot.arrow.as_ref().map(|a| a.pickup_item.clone()) else {
            return 8.0;
        };
        let ctx = horizon_client::enchant::EffectContext {
            this: victim,
            attacker: (shot.owner == Some(Who::Player)).then_some("minecraft:player"),
            direct_attacker: Some("minecraft:trident"),
            damage_type: Some("minecraft:trident"),
            items: Some(world_items!(self)),
            ..Default::default()
        };
        world_stations!(self).enchantments.modify_value(
            Some(&trident),
            "minecraft:damage",
            &ctx,
            &mut self.world.random,
            8.0,
        )
    }
    /// `BellBlock.onProjectileHit`: a projectile that strikes the bell proper
    /// rings it, and a player's shot counts as the player's ring.
    fn bell_projectile_hit(&mut self, p: &Projectile, pos: [i32; 3], at: Vec3, face: [i32; 3]) {
        let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face) else {
            return;
        };
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let rang = self
            .with_level(|b, level, ticks| b.bell_projectile_hit(level, ticks, block, face, at.y));
        if rang {
            let _ = self.process_level_events();
            if p.owner == Some(Who::Player) {
                self.stats.custom("bell_ring", 1);
            }
        }
    }
    /// `AmethystBlock.onProjectileHit`: a projectile that strikes an amethyst
    /// block rings `block.amethyst_block.chime` there, pitched by the level random.
    fn amethyst_projectile_hit(&mut self, pos: [i32; 3]) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if &*self.res.facts().get(state).name != "minecraft:amethyst_block" {
            return;
        }
        let pitch = 0.5 + self.world.random.next_float() * 1.2;
        self.level_events
            .push(horizon_world::level::LevelEvent::Sound {
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                sound: "minecraft:block.amethyst_block.chime".into(),
                volume: 1.0,
                pitch,
            });
        let _ = self.process_level_events();
    }
    /// `onProjectileHit` of chorus flowers, decorated pots and speleothems
    /// (pointed dripstone, sulfur spikes): an impact projectile
    /// (`#impact_projectiles`) breaks the block when `Projectile.mayInteract`
    /// and `Projectile.mayBreak` allow it, which takes the
    /// `projectiles_can_break_blocks` rule and a shooter that may break blocks
    /// (`canBreakBlockInAdventureMode`: never a mob, never an
    /// adventure player). A pot is cracked first (flags 260), so it shatters.
    fn projectile_breaks_block(&mut self, p: &Projectile, pos: [i32; 3]) {
        use horizon_client::command::args::GameMode;
        let impact = matches!(
            p.kind,
            ProjectileKind::Arrow { .. }
                | ProjectileKind::Trident
                | ProjectileKind::Snowball
                | ProjectileKind::Thrown(_)
                | ProjectileKind::Egg
                | ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        );
        let shooter_may_break = match p.owner {
            Some(Who::Player) => self.mode != GameMode::Adventure,
            Some(Who::Mob(_)) => false,
            None => true,
        };
        if !impact || !shooter_may_break || !self.ui.rules.bool("projectiles_can_break_blocks") {
            return;
        }
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        match &*self.res.facts().get(state).name {
            "minecraft:chorus_flower" => {}
            // SpeleothemBlock.onProjectileHit (pointed dripstone and sulfur spikes): a
            // thrown trident faster than 0.6 breaks it.
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
                if matches!(p.kind, ProjectileKind::Trident)
                    && p.arrow.as_ref().is_some_and(|a| a.hit_speed > 0.6) => {}
            "minecraft:decorated_pot" => {
                let registry = &self.pack.assets.registry;
                let Some(mut pot) = registry.state(state).cloned() else {
                    return;
                };
                pot.properties.insert("cracked".into(), "true".into());
                let Ok(cracked) = registry.resolve(&pot) else {
                    return;
                };
                self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
            }
            _ => return,
        }
        let _ = self.destroy_block_dropping(pos);
    }
    /// What an arrow's tick asked for: its particles and sounds.
    fn arrow_event(&mut self, at: Vec3, event: horizon_client::arrows::ArrowEvent) {
        use horizon_client::arrows::{ArrowEvent, ArrowParticle, ArrowSound, BlockReaction};
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let kind = |k| match k {
            ArrowParticle::Bubble => ParticleType::Bubble,
            ArrowParticle::Crit => ParticleType::Crit,
            ArrowParticle::Splash => ParticleType::Splash,
        };
        match event {
            ArrowEvent::Particle {
                kind: k,
                position,
                velocity,
            } => self.particles.add(
                &ParticleOptions::simple(kind(k)),
                [position.x, position.y, position.z],
                [velocity.x, velocity.y, velocity.z],
            ),
            // Arrow.tick: `broadcastEntityEvent(this, 0)`. The window and the
            // guests burst the particles themselves (`remote_arrow_puff`).
            ArrowEvent::PotionPuff { id, .. } => {
                self.lan_entity_event(id as i32, 0, [at.x, at.y, at.z]);
            }
            ArrowEvent::Sound {
                sound,
                volume,
                pitch,
            } => {
                let name = match sound {
                    ArrowSound::Splash => "minecraft:entity.generic.splash",
                    ArrowSound::HitGround => "minecraft:entity.arrow.hit",
                    ArrowSound::CrossbowHit => "minecraft:item.crossbow.hit",
                    ArrowSound::Extinguish => "minecraft:entity.generic.extinguish_fire",
                    ArrowSound::TridentHitGround => "minecraft:item.trident.hit_ground",
                    ArrowSound::TridentHit => "minecraft:item.trident.hit",
                };
                self.audio
                    .play(name, Some([at.x, at.y, at.z]), volume, pitch);
            }
            ArrowEvent::BubbleColumnParticles { pos } => self.bubble_column_particles(pos),
            ArrowEvent::Block { pos, reaction } => match reaction {
                BlockReaction::PrimeTnt => {
                    self.spawn_primed_tnt(pos, false, None, None);
                    self.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, false));
                    let _ = self.process_level_events();
                }
                BlockReaction::Light(lit) => {
                    let _ = self.level_set([pos.x, pos.y, pos.z], lit);
                }
            },
            ArrowEvent::SplashEvent => {
                self.projectile_event("minecraft:splash", None, [at.x, at.y, at.z]);
            }
            // finish_arrow_hit drops the pickup, which it holds.
            ArrowEvent::ShootEvent | ArrowEvent::Drop => {}
        }
    }
    /// `Level.isRainingAt` for arrows.
    /// What a projectile can hit: the mobs, the player, the dragon's parts and
    /// the End crystals (under reserved ids, game/dragon.rs).
    pub(super) fn projectile_targets(&self) -> Vec<(Who, Aabb)> {
        let mut targets: Vec<(Who, Aabb)> = self
            .mobs
            .iter()
            .filter(|m| m.alive())
            .map(|m| (Who::Mob(m.id), m.bounds()))
            .collect();
        if let Some(p) = self.player_view()
            && p.alive
        {
            targets.push((Who::Player, p.bounds));
        }
        targets.extend(
            self.special_targets()
                .into_iter()
                .map(|(id, b)| (Who::Mob(id), b)),
        );
        targets
    }
    pub(super) fn arrow_rain(&self) -> impl Fn(BlockPos) -> bool + '_ {
        move |p: BlockPos| self.raining_at(p)
    }
    /// The level as arrows see it.
    pub(super) fn arrow_scene<'s>(
        &'s self,
        rain: &'s dyn Fn(BlockPos) -> bool,
    ) -> horizon_client::arrows::ArrowScene<'s, 's> {
        horizon_client::arrows::ArrowScene {
            world: &self.world,
            registry: self.pack.assets.registry,
            facts: self.res.facts(),
            collision: self.collision,
            fluids: &self.motion.fluids,
            raining: rain,
            mob_griefing: self.ui.rules.bool("mob_griefing"),
            tnt_explodes: self.ui.rules.bool("tnt_explodes"),
        }
    }
    /// The rest of the tick of an arrow that hit an entity
    /// (`arrows::finish_entity_hit`): its sounds and drop, and back among the
    /// projectiles if it flies on.
    fn finish_arrow_hit(&mut self, mut arrow: Projectile, outcome: EntityHit) {
        let keep = {
            let rain = self.arrow_rain();
            horizon_client::arrows::finish_entity_hit(&mut arrow, &self.arrow_scene(&rain), outcome)
        };
        let Some(a) = arrow.arrow.as_mut() else {
            return;
        };
        let (events, item) = (std::mem::take(&mut a.events), a.pickup_item.clone());
        for event in events {
            if event == horizon_client::arrows::ArrowEvent::Drop {
                self.spawn_at_location(arrow.position, f64::from(0.1f32), item.clone());
            } else {
                self.arrow_event(arrow.position, event);
            }
        }
        if keep {
            self.add_projectile(arrow);
        }
    }
    /// `TargetBlock.onProjectileHit`: a target block takes its power from
    /// where the projectile landed and resets after 20 ticks for an arrow, 8
    /// for any other projectile. A player's shot counts as a target hit.
    fn target_block_hit(&mut self, p: &Projectile, pos: [i32; 3], at: Vec3, face: [i32; 3]) {
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let arrow = p.arrow.is_some();
        let location = [at.x, at.y, at.z];
        let hit = self.with_level(|b, level, ticks| {
            b.target_projectile_hit(level, ticks, block, location, face, arrow)
        });
        if let Some(strength) = hit {
            let _ = self.process_level_events();
            if p.owner == Some(Who::Player) && (p.guest.is_none() || p.guest == self.lan_acting) {
                self.stats.custom("target_hit", 1);
                let kind = if p
                    .arrow
                    .as_ref()
                    .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow")
                {
                    "minecraft:spectral_arrow"
                } else {
                    p.kind.id()
                };
                let projectile = horizon_client::advancements::EntityView {
                    kind: kind.to_owned(),
                    position: [p.position.x, p.position.y, p.position.z],
                    ..Default::default()
                };
                self.trigger_target_hit(&projectile, strength);
            }
        }
    }
    pub(super) fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, at, face } = hit
            && face != [0; 3]
        {
            self.bell_projectile_hit(p, pos, at, face);
            self.amethyst_projectile_hit(pos);
            self.projectile_breaks_block(p, pos);
        }
        self.combat.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.combat.projectile_direct = match hit {
            Hit::Entity {
                who: Who::Player, ..
            } => {
                let spectral = p
                    .arrow
                    .as_ref()
                    .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow");
                let id = if spectral {
                    "minecraft:spectral_arrow"
                } else {
                    p.kind.id()
                };
                Some(Text::translate(
                    &format!("entity.{}", id.replace(':', ".")),
                    &[],
                ))
            }
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, face, .. } = hit
            && face != [0; 3]
        {
            self.target_block_hit(p, pos, at, face);
        }
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && self.lan_projectile_hit(p, id, at)
        {
            return Ok(());
        }
        if matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        ) {
            return self.wind_charge_hit(p, hit);
        }
        // A wither skull's hit on a part or crystal goes through
        // `wither_skull_hit` below, as on any entity, and then explodes.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && id >= super::dragon::SPECIAL
            && !matches!(p.kind, ProjectileKind::WitherSkull { .. })
        {
            let by_player = p.owner == Some(Who::Player);
            if p.arrow.is_some() {
                // AbstractArrow.onHitEntity: spent if the part or crystal took
                // it, else turned back (a perched dragon sets it alight first).
                let mut arrow = p.clone();
                let amount = self.shot_damage(&mut arrow, None);
                let outcome = match self.hit_special(
                    id,
                    amount,
                    by_player,
                    horizon_client::mobs::DamageKind::Arrow,
                    Some(&p.kind),
                ) {
                    super::dragon::Hurt::Landed { .. } => EntityHit::Hurt,
                    super::dragon::Hurt::Refused => EntityHit::Refused,
                    super::dragon::Hurt::Shrugged => {
                        if let Some(a) = arrow.arrow.as_mut() {
                            a.ignite_for_seconds(1.0);
                        }
                        EntityHit::Refused
                    }
                };
                self.finish_arrow_hit(arrow, outcome);
                return Ok(());
            }
            let amount = match p.kind {
                ProjectileKind::SmallFireball => 5.0,
                ProjectileKind::LargeFireball { .. } => 6.0,
                ProjectileKind::Thrown(t) => t.damage,
                _ => 0.0,
            };
            let kind = match p.kind {
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                    horizon_client::mobs::DamageKind::Fireball
                }
                _ => horizon_client::mobs::DamageKind::Thrown,
            };
            self.hit_special(id, amount, by_player, kind, Some(&p.kind));
            // ThrowableProjectile.onHit goes on after onHitEntity: a pearl
            // still takes its owner to where it broke (into the crystal's
            // blast), an egg still hatches, a snowball still breaks. A large
            // fireball still explodes (LargeFireball.onHit), whatever the
            // part or crystal made of its hit (both refuse fire), and the
            // blast takes the crystal.
            if !matches!(
                p.kind,
                ProjectileKind::EnderPearl
                    | ProjectileKind::Egg
                    | ProjectileKind::Snowball
                    | ProjectileKind::Thrown(_)
                    | ProjectileKind::LargeFireball { .. }
            ) {
                return Ok(());
            }
        }
        if p.arrow.is_some()
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && let Some(i) = self.carts.iter().position(|c| c.id == id)
        {
            // AbstractArrow.onHitEntity on a minecart.
            let mut arrow = p.clone();
            let amount = self.shot_damage(&mut arrow, None);
            let burning = p.arrow.as_ref().is_some_and(|a| a.on_fire());
            let creative = p.owner == Some(Who::Player) && self.creative;
            self.arrow_hits_cart(i, amount, p.velocity, burning, creative);
            self.finish_arrow_hit(arrow, EntityHit::Hurt);
            return Ok(());
        }
        let atp = [at.x, at.y, at.z];
        // Projectile.onHit: the landing is heard.
        self.projectile_event("minecraft:projectile_land", p.owner, atp);
        let source_kind = match p.kind {
            ProjectileKind::Arrow { .. } | ProjectileKind::Trident => DamageKind::Arrow,
            ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                DamageKind::Fireball
            }
            _ => DamageKind::Thrown,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
        };
        let aim = self.attacker_aim(source.attacker);
        match (&p.kind, hit) {
            // ThrownTrident.onHitEntity: 8 damage (Impaling on aquatic mobs),
            // dealt whether or not the hurt lands, then it bounces off.
            (ProjectileKind::Trident, Hit::Entity { who, .. }) => {
                let mut trident = p.clone();
                let victim = match who {
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.kind.id.clone()),
                    Who::Player => Some("minecraft:player".to_owned()),
                };
                let amount = self.shot_damage(&mut trident, victim.as_deref());
                let hurt = match who {
                    Who::Player => self.mob_hits_player(
                        amount,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:trident",
                    ),
                    Who::Mob(id) => self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                        m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z])
                    }),
                };
                let outcome = if hurt {
                    EntityHit::Hurt
                } else {
                    EntityHit::Refused
                };
                self.burning_arrow_primes(p, who);
                self.finish_arrow_hit(trident, outcome);
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Trident, Hit::Block { .. }) => {}
            // AbstractArrow.onHitEntity on a wind charge: `hurtOrSimulate` fails
            // (the charge turned in `Projectile.onHit`, which already ran), so the
            // arrow takes `deflect(REVERSE, 0.2)` instead of vanishing.
            (
                ProjectileKind::Arrow { .. },
                Hit::Entity {
                    who: Who::Mob(id), ..
                },
            ) if wind_charge_hit(id).is_some() => {
                let mut arrow = p.clone();
                arrow_damage(&mut arrow, &mut self.world.random);
                self.finish_arrow_hit(arrow, EntityHit::Refused);
            }
            (ProjectileKind::Arrow { .. }, Hit::Entity { who, .. }) => {
                let mut arrow = p.clone();
                let amount = arrow_damage(&mut arrow, &mut self.world.random) + p.bonus.damage;
                let player_fire = self.player.as_ref().map(|pl| pl.fire_ticks);
                let mut dodged = false;
                // AbstractArrow.onHitEntity: a burning arrow sets its target alight.
                if p.arrow.as_ref().is_some_and(|a| a.on_fire()) {
                    self.handle_mob_events(vec![MobEvent::Ignite {
                        target: who,
                        seconds: 5,
                    }])?;
                }
                let hurt = match who {
                    Who::Player => {
                        let hurt = self.mob_hits_player(
                            amount,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:arrow",
                        );
                        // AbstractArrow.onHitEntity: a hit that hurt leaves a
                        // non-piercing arrow stuck in the body.
                        if hurt && p.arrow.as_ref().is_some_and(|a| a.pierce == 0) {
                            self.combat.stuck.add_arrow();
                        }
                        hurt
                    }
                    Who::Mob(id) => {
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z])
                        });
                        dodged = !hurt
                            && self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
                                m.species() == Species::Enderman
                                    && horizon_client::mobs::enderman::dodges(m, &source)
                            });
                        if p.owner == Some(Who::Player) {
                            self.last_hurt_by_player.insert(id, self.lan_acting);
                            if hurt && let Some(m) = self.mobs.iter().find(|m| m.id == id) {
                                // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                                let victim = self.mob_entity(m);
                                let spectral = p.arrow.as_ref().is_some_and(|a| {
                                    &*a.pickup_item.item == "minecraft:spectral_arrow"
                                });
                                let direct = horizon_client::advancements::EntityView {
                                    kind: if spectral {
                                        "minecraft:spectral_arrow"
                                    } else {
                                        "minecraft:arrow"
                                    }
                                    .into(),
                                    position: [p.position.x, p.position.y, p.position.z],
                                    ..Default::default()
                                };
                                self.player_hit(&victim, "minecraft:arrow", Some(direct), amount);
                            }
                            // AbstractArrow.onHitEntity: KILLED_BY_ARROW for a kill
                            // (piercing arrows are not modelled).
                            if hurt
                                && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                                && !m.alive()
                            {
                                let victim = self.mob_entity(m);
                                let weapon =
                                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref());
                                self.trigger_killed_by_arrow(&[victim], weapon);
                            }
                        }
                        hurt
                    }
                };
                self.burning_arrow_primes(p, who);
                // doKnockback: Punch pushes along the flight, less the target's
                // knockback resistance.
                let flight = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
                let resistance = match who {
                    Who::Player => self
                        .player
                        .as_mut()
                        .and_then(|pl| pl.attribute("minecraft:knockback_resistance").ok())
                        .unwrap_or(0.0),
                    Who::Mob(id) => self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map_or(0.0, |m| m.attrs.knockback_resistance),
                };
                let knockback = if !hurt {
                    0.0
                } else if let Some(weapon) =
                    p.arrow.as_ref().and_then(|a| a.fired_from_weapon.as_ref())
                {
                    let victim = match who {
                        Who::Player => Some("minecraft:player"),
                        Who::Mob(id) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                    };
                    let owner = match p.owner {
                        Some(Who::Player) => Some("minecraft:player"),
                        Some(Who::Mob(id)) => self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .map(|m| m.kind.id.as_str()),
                        None => None,
                    };
                    let arrow_type = if p
                        .arrow
                        .as_ref()
                        .is_some_and(|a| &*a.pickup_item.item == "minecraft:spectral_arrow")
                    {
                        "minecraft:spectral_arrow"
                    } else {
                        "minecraft:arrow"
                    };
                    let ctx = horizon_client::enchant::EffectContext {
                        this: victim,
                        attacker: owner,
                        direct_attacker: Some(arrow_type),
                        damage_type: Some("minecraft:arrow"),
                        items: Some(self.res.items()),
                        ..Default::default()
                    };
                    self.res.stations().enchantments.modify_value(
                        Some(weapon),
                        "minecraft:knockback",
                        &ctx,
                        &mut self.world.random,
                        0.0,
                    )
                } else {
                    p.bonus.punch as f32
                };
                let push = f64::from(knockback) * 0.6 * (1.0 - resistance).max(0.0);
                if hurt && push > 0.0 && flight > 1e-6 {
                    let k = push / flight;
                    self.handle_mob_events(vec![MobEvent::Push {
                        target: who,
                        velocity: [p.velocity.x * k, 0.1, p.velocity.z * k],
                    }])?;
                }
                if hurt {
                    self.arrow_post_hurt_effects(p, who);
                }
                match (hurt, who) {
                    (true, _) => self.finish_arrow_hit(arrow, EntityHit::Hurt),
                    // What the player took no damage from (a raised shield)
                    // keeps its fire as it was and turns the arrow back.
                    (false, Who::Player) => {
                        if let (Some(pl), Some(ticks)) = (self.player.as_mut(), player_fire) {
                            pl.set_remaining_fire_ticks(ticks);
                        }
                        self.finish_arrow_hit(arrow, EntityHit::Refused);
                    }
                    // EnderMan.hurtServer teleported away: the arrow flies on.
                    (false, Who::Mob(_)) if dodged => {
                        self.finish_arrow_hit(arrow, EntityHit::Dodged);
                    }
                    // Mobs' other refusals don't follow hurtServer yet: the arrow is gone.
                    (false, Who::Mob(_)) => {}
                }
            }
            // AbstractArrow.onHitBlock plays its sound itself (ArrowEvent).
            (ProjectileKind::Arrow { .. }, Hit::Block { .. }) => {}
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if m.species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    m.damage_projectile(amount, source, aim, [p.velocity.x, p.velocity.z]);
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:snowball");
            }
            // a mod's copy of Snowball: its damage to anything it hits.
            (ProjectileKind::Thrown(t), hit) => {
                match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => {
                        let push = [p.velocity.x, p.velocity.z];
                        let hurt = self
                            .mobs
                            .iter_mut()
                            .find(|m| m.id == id)
                            .is_some_and(|m| m.damage_projectile(t.damage, source, aim, push));
                        if hurt {
                            self.player_hurt_mob(p, id);
                        }
                        // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                        if hurt
                            && p.owner == Some(Who::Player)
                            && let Some(m) = self.mobs.iter().find(|m| m.id == id)
                        {
                            let victim = self.mob_entity(m);
                            let direct = horizon_client::advancements::EntityView {
                                kind: t.id.into(),
                                position: [p.position.x, p.position.y, p.position.z],
                                ..Default::default()
                            };
                            self.player_hit(&victim, "minecraft:thrown", Some(direct), t.damage);
                        }
                    }
                    // Player.hurtServer takes no 0 damage.
                    Hit::Entity {
                        who: Who::Player, ..
                    } if t.damage > 0.0 => {
                        let from = [p.previous.x, p.previous.y, p.previous.z];
                        self.mob_hits_player(t.damage, from, "minecraft:thrown");
                    }
                    _ => {}
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, t.item);
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.damage_projectile(0.0, source, aim, [p.velocity.x, p.velocity.z]);
                }
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob("minecraft:chicken", atp, true)
                            && let Some(c) = self.mobs.iter_mut().find(|m| m.id == id)
                        {
                            c.extra.age = -24000;
                        }
                    }
                }
                self.thrown_break_event(p.id, atp);
                self.item_burst(atp, "minecraft:egg");
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.damage_projectile(0.0, source, aim, [p.velocity.x, p.velocity.z]);
                }
                if p.owner == Some(Who::Player) && self.player.is_some() {
                    // ThrownEnderpearl: 5% endermite where the player was
                    // (when monsters spawn), teleport, 5 damage.
                    if self.world.random.next_float() < 0.05
                        && self.ui.rules.bool("spawn_monsters")
                        && self.ui.difficulty != 0
                        && let Some(pl) = self.player.as_ref()
                    {
                        let from = pl.position;
                        self.spawn_mob("minecraft:endermite", [from.x, from.y, from.z], false);
                    }
                    // To where the pearl started its last tick (oldPosition).
                    let to = p.previous;
                    if let Some(pl) = self.player.as_mut() {
                        pl.position = to;
                        pl.velocity = Vec3::ZERO;
                        pl.fall_distance = 0.0;
                        pl.teleported = true;
                    }
                    self.previous_position = to;
                    self.hurt_player_by("minecraft:ender_pearl", 5.0);
                    self.audio.play(
                        "minecraft:entity.player.teleport",
                        Some([to.x, to.y, to.z]),
                        1.0,
                        1.0,
                    );
                }
            }
            (ProjectileKind::SmallFireball, hit) => match hit {
                Hit::Entity { who, .. } => match who {
                    Who::Player => {
                        if !self.creative {
                            // SmallFireball.onHitEntity: remember the fire, ignite,
                            // and restore it when the hurt is refused (a raised shield).
                            let remaining = self.player.as_ref().map(|p| p.fire_ticks);
                            if let Some(p) = self.player.as_mut() {
                                p.ignite_for_ticks(100)?;
                            }
                            let hurt = self.mob_hits_player(
                                5.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                            if !hurt
                                && let Some(remaining) = remaining
                                && let Some(pl) = self.player.as_mut()
                            {
                                pl.set_remaining_fire_ticks(remaining);
                            }
                        }
                    }
                    Who::Mob(id) => {
                        let hurt = self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                            let remaining = m.fire_ticks;
                            if !m.kind.flags.fire_immune {
                                m.ignite_for_ticks(100);
                            }
                            let hurt =
                                m.damage_projectile(5.0, source, aim, [p.velocity.x, p.velocity.z]);
                            if !hurt {
                                m.fire_ticks = remaining;
                            }
                            hurt
                        });
                        if hurt {
                            self.player_hurt_mob(p, id);
                        }
                    }
                },
                Hit::Block { pos, face, .. } => {
                    // SmallFireball.onHitBlock: a mob's only with mobGriefing;
                    // BaseFireBlock.getState placed with updates (the fire
                    // ticks, ages and goes out where it can't stay).
                    let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                    let mob = matches!(p.owner, Some(Who::Mob(_)));
                    if (!mob || self.ui.rules.bool("mob_griefing"))
                        && self
                            .res
                            .facts()
                            .is_air(self.world.block(fire[0], fire[1], fire[2]))
                    {
                        let at = BlockPos::new(fire[0], fire[1], fire[2]);
                        let state =
                            self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                        self.level_set(fire, state)?;
                    }
                }
                Hit::None => {}
            },
            (ProjectileKind::LargeFireball { power }, hit) => {
                // Fireball explosions are caused by the shooter. A direct hit
                // on the player takes `projectile_owner`, so read the owner
                // from the fireball.
                let owner = match p.owner {
                    Some(Who::Mob(id)) => Some(id),
                    _ => None,
                };
                if let Hit::Entity { who, .. } = hit {
                    match who {
                        Who::Player => {
                            self.mob_hits_player(
                                6.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                "minecraft:fireball",
                            );
                        }
                        Who::Mob(id) => {
                            if self.mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                                m.damage_projectile(6.0, source, aim, [p.velocity.x, p.velocity.z])
                            }) {
                                self.player_hurt_mob(p, id);
                            }
                        }
                    }
                }
                self.combat.explosion_owner = owner;
                self.combat.explosion_in_water = p.touching_water;
                // The blast's source names the owner, so a dragon it kills
                // remembers the player who shot it.
                self.set_blast_by_player(p.owner == Some(Who::Player));
                let result = self.explode(atp, *power as f32);
                self.set_blast_by_player(false);
                self.combat.explosion_owner = None;
                self.combat.explosion_in_water = false;
                result?;
            }
            (
                ProjectileKind::Potion {
                    effect,
                    ticks,
                    amplifier,
                    ..
                },
                _,
            ) => {
                // AbstractThrownPotion.applySplash: within 4 blocks, scaled by distance.
                if let Some(pv) = self.player_view() {
                    let d2 = (pv.position.x - at.x).powi(2)
                        + (pv.position.y - at.y).powi(2)
                        + (pv.position.z - at.z).powi(2);
                    if d2 < 16.0 && !self.creative {
                        let scale = 1.0 - d2.sqrt() / 4.0;
                        let t = (f64::from(*ticks) * scale + 0.5) as i32;
                        match *effect {
                            "minecraft:instant_damage" => {
                                // indirectMagic(potion, owner), built where the
                                // hurt happens: the thrower, or the potion
                                // itself when it has none. A status effect
                                // deals no damage, so it records no source.
                                let owner = self
                                    .combat
                                    .projectile_owner
                                    .filter(|id| self.mob_display_name(*id).is_some());
                                match owner {
                                    Some(owner) => {
                                        self.note_attacker("minecraft:indirect_magic", owner)
                                    }
                                    None => self.note_direct_entity(
                                        "minecraft:indirect_magic",
                                        Text::translate("entity.minecraft.splash_potion", &[]),
                                    ),
                                }
                                self.hurt_player_by(
                                    "minecraft:indirect_magic",
                                    6.0 * (1 << *amplifier) as f32 * scale as f32,
                                )
                            }
                            e if t > 20 => self.give_player_effect(e, t, *amplifier),
                            _ => {}
                        }
                    }
                }
                // A witch's healing potions reach the raiders around the splash.
                if matches!(
                    *effect,
                    "minecraft:instant_health" | "minecraft:regeneration"
                ) {
                    let center = at;
                    for m in &mut self.mobs {
                        let d2 = m.position.distance_squared(center);
                        if !m.alive()
                            || d2 >= 16.0
                            || !horizon_client::mobs::raider::is_raider(m.species())
                        {
                            continue;
                        }
                        let scale = 1.0 - d2.sqrt() / 4.0;
                        if *effect == "minecraft:instant_health" {
                            let heal = (4 << *amplifier) as f64 * scale;
                            m.health = (m.health + heal as f32).min(m.attrs.max_health);
                        } else {
                            let t = (f64::from(*ticks) * scale + 0.5) as i32;
                            if t > 20 {
                                m.add_effect(effect, t, *amplifier);
                            }
                        }
                    }
                }
                self.audio.play(
                    "minecraft:entity.splash_potion.break",
                    Some(atp),
                    1.0,
                    self.world.random.next_float() * 0.1 + 0.9,
                );
            }
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    self.mob_hits_player(
                        1.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:spit",
                    );
                }
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        m.damage_projectile(1.0, source, aim, [p.velocity.x, p.velocity.z]);
                    }
                }
            },
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => match who {
                Who::Player => {
                    // Only a hit that hurt (`wasHurt`) brings the levitation.
                    if !self.creative
                        && self.mob_hits_player(
                            4.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:mob_projectile",
                        )
                    {
                        self.give_player_effect("minecraft:levitation", 200, 0);
                    }
                }
                Who::Mob(id) => {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                        && m.damage_projectile(4.0, source, aim, [p.velocity.x, p.velocity.z])
                    {
                        m.add_effect("minecraft:levitation", 200, 0);
                        // Shulker.hurtServer: a shulker hit by a bullet.
                        horizon_client::mobs::shulker::hit_by_bullet(m);
                    }
                }
            },
            (ProjectileKind::WitherSkull { .. }, hit) => {
                let who = match hit {
                    Hit::Entity { who, .. } => Some(who),
                    _ => None,
                };
                self.wither_skull_hit(p, who, atp)?;
            }
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.audio
                    .play("minecraft:entity.shulker_bullet.hit", Some(atp), 1.0, 1.0);
                if let Some(ty) = horizon_client::particles::ParticleType::from_name("explosion") {
                    let options = horizon_client::particles::ParticleOptions::simple(ty);
                    for _ in 0..2 {
                        let r = &mut self.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.particles.add(&options, at, [0.0; 3]);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// `Snowball`/`ThrownEgg.onHit`: entity event 3 for the guests, before the
    /// projectile is discarded (the clients lose it when the tracker next runs).
    fn thrown_break_event(&mut self, id: u32, at: [f64; 3]) {
        if id != 0 {
            self.lan_entity_event(id as i32, 3, at);
        }
    }
    /// `Snowball`/`ThrownEgg.handleEntityEvent(3)`: eight crumbs of the
    /// thrown item where it broke, at rest for a snowball, scattered by the
    /// client entity's random (the particle engine's here) for an egg.
    pub(super) fn item_burst(&mut self, at: [f64; 3], item: &str) {
        let options = self.item_particle(&horizon_client::items::ItemStack::new(item, 1));
        let egg = item == "minecraft:egg";
        for _ in 0..8 {
            let v = if egg {
                let r = self.particles.random();
                [
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                    (f64::from(r.next_float()) - 0.5) * 0.08,
                ]
            } else {
                [0.0; 3]
            };
            self.particles.add(&options, at, v);
        }
    }
    /// `AgeableMob.mobInteract`: whether a click with `item` on `m` gets to
    /// the golden dandelion check (`canUseGoldenDandelion`, which a tadpole
    /// calls with `isBaby` true). A panda that is scared or on its back and an
    /// armadillo in its shell answer in their own `mobInteract` first.
    pub(super) fn age_lock_takes(&self, m: &Mob, item: &str) -> bool {
        let species = m.species();
        let baby = if species == Species::Tadpole {
            true
        } else if horizon_client::mobs::finalize::ageable(species) {
            m.is_baby()
                && !(species == Species::Panda
                    && (horizon_client::mobs::panda::scared(m)
                        || horizon_client::mobs::panda::is_on_back(m)))
                && !(species == Species::Armadillo && horizon_client::mobs::armadillo::scared(m))
        } else {
            false
        };
        let cannot = self
            .res
            .mob_data()
            .entity_has("minecraft:cannot_be_age_locked", &m.kind.id);
        horizon_client::mobs::ai::can_use_golden_dandelion(
            item,
            baby,
            m.extra.age_lock_particle_timer,
            cannot,
        )
    }
    /// `AgeableMob.setAgeLocked`: a golden dandelion on mob `index` flips its
    /// age lock (a tadpole's or an ageable mob's own `setAgeLockedData`) and is
    /// used up; a locked mob stays (`setPersistenceRequired`), and the lock's
    /// use or unuse sound plays at its block for everyone near.
    pub(super) fn set_age_locked(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        if m.species() == Species::Tadpole {
            horizon_client::mobs::aquatic::set_age_locked_data(m);
        } else {
            horizon_client::mobs::ai::set_age_locked_data(m);
        }
        self.consume_held();
        let m = &mut self.mobs[index];
        let locked = m.extra.age_locked;
        if locked {
            m.persistent = true;
        }
        // Level.playSound(null, blockPosition, ..): the middle of the block.
        let at = [
            m.position.x.floor() + 0.5,
            m.position.y.floor() + 0.5,
            m.position.z.floor() + 0.5,
        ];
        let chunk = (
            m.position.x.floor() as i32 >> 4,
            m.position.z.floor() as i32 >> 4,
        );
        self.world.modified.insert(chunk);
        let sound = if locked {
            "minecraft:item.golden_dandelion.use"
        } else {
            "minecraft:item.golden_dandelion.unuse"
        };
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// Right click on a mob (Mob.interact / mobInteract), on the mob under the
    /// crosshair or on the one a guest's interact packet named: returns true
    /// when consumed.
    pub(super) fn interact_mob(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        let Some((index, distance)) = (match target {
            // The id the packet named, under the filters the crosshair pick
            // applies (`mob_hit`): a mob in its death animation is not clicked
            // by either path, so naming one in a packet must not reach the
            // feed, shear, leash or name paths either.
            Some(t) => Self::named_entity(&self.mobs, t.id, |m| m.id)
                .filter(|&i| {
                    let m = &self.mobs[i];
                    named_mob_pickable(m.alive(), m.id, self.riding, self.remote_camera_id())
                })
                .map(|i| (i, 0.0)),
            None => self.mob_hit(),
        }) else {
            return Ok(false);
        };
        // Only a client's own pick is stopped by a block: the range check passed.
        if target.is_none()
            && let Some(hit) = self.pick_unfiltered()
            && let Some(eye) = self.eye_position()
        {
            let d = ((hit.point[0] - eye[0]).powi(2)
                + (hit.point[1] - eye[1]).powi(2)
                + (hit.point[2] - eye[2]).powi(2))
            .sqrt();
            if d < distance {
                return Ok(false);
            }
        }
        if self.name_mob(index) {
            return Ok(true);
        }
        if self.spawn_egg_offspring(index) {
            self.swing_arm();
            return Ok(true);
        }
        {
            let item = self.held().map(|h| h.item.to_string()).unwrap_or_default();
            let used = self.held().cloned();
            if self.interact_leash(index, &item) {
                if let Some(stack) = &used {
                    let target = self.mob_entity(&self.mobs[index]);
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &target,
                    );
                }
                self.swing_arm();
                return Ok(true);
            }
        }
        let held = self.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        // Bucketable.bucketMobPickup: a water bucket scoops up a fish, a
        // tadpole or an axolotl, with the data its bucket carries.
        if item == "minecraft:water_bucket"
            && self.mobs[index].alive()
            && let Some((bucket, sound)) = bucketable(self.mobs[index].species())
        {
            let mut filled = ItemStack::new(bucket, 1);
            filled.extra = Some(Box::new(horizon_client::entity_nbt::bucket_components(
                &self.mobs[index],
            )));
            self.audio
                .play(sound, Some(self.mobs[index].center()), 1.0, 1.0);
            self.trigger_item("minecraft:filled_bucket", &filled);
            // Leashable.dropLeash, then discard.
            let id = self.mobs[index].id;
            self.break_leash(id, true, false);
            self.mobs[index].removed = true;
            if self.creative {
                if let Some(rest) = self.give(filled) {
                    self.spawn_item(rest, self.eye_position().unwrap_or([0.0; 3]), [0.0; 3], 40);
                }
            } else {
                *self.held_mut() = Some(filled);
            }
            self.swing_arm();
            return Ok(true);
        }
        // AgeableMob.mobInteract: a golden dandelion on a baby freezes its
        // growth, or lets it grow again.
        if self.mobs[index].alive() && self.age_lock_takes(&self.mobs[index], &item) {
            self.set_age_locked(index);
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        let creative = self.creative;
        // The hand's slot (Player.interactOn passes the hand).
        let selected = self.held_slot();
        // Panda.mobInteract: bamboo, or a click on a panda lying on its back.
        if self.mobs[index].species() == Species::Panda && self.mobs[index].alive() {
            let taken = self.interact_panda(index, &item);
            if taken {
                self.swing_arm();
            }
            return Ok(taken);
        }
        if self.interact_zombie_villager(index, &item) {
            return Ok(true);
        }
        if self.interact_trader(index, &item)? {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_allay(index, held.as_ref()) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_sulfur_cube(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_armadillo(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_copper_golem(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_shear_mob(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // AbstractHorse.mobInteract: feed, equip or ride.
        if self.interact_mount(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // PiglinAi.mobInteract: hand over something gold to admire.
        if self.mobs[index].alive() && self.give_piglin(index, &item) {
            self.swing_arm();
            if !creative && let Some(stack) = self.stack_in_mut(selected).as_mut() {
                stack.count -= 1;
                if stack.count == 0 {
                    *self.stack_in_mut(selected) = None;
                }
            }
            return Ok(true);
        }
        // Creeper.mobInteract: any item in #creeper_igniters, damaged when it
        // is damageable and used up otherwise.
        let creeper_igniter = self
            .res
            .items()
            .item_tags
            .get("minecraft:creeper_igniters")
            .is_some_and(|t| t.contains(&*item));
        let fish = self
            .res
            .items()
            .item_tags
            .get("minecraft:fishes")
            .is_some_and(|t| t.contains(&*item));
        let damageable = self
            .res
            .items()
            .get(&item)
            .is_some_and(|i| i.max_damage > 0);
        let parrot_poisonous = self
            .res
            .items()
            .item_tags
            .get("minecraft:parrot_poisonous_food")
            .is_some_and(|t| t.contains(&*item));
        let player_at = self.eye_position();
        let player_feet = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let acting = self.acting_player_uuid();
        let sneaking = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft)
            || self.keys.contains(&winit::keyboard::KeyCode::ShiftRight);
        let held_damage = held.as_ref().map_or(0, |h| h.damage as i32);
        let held_extra = held.as_ref().and_then(|h| h.extra.clone());
        let m = &mut self.mobs[index];
        if !m.alive() {
            return Ok(false);
        }
        let species = m.species();
        let center = m.center();
        // Wolf armor the owner takes off with shears or leaves a dropped stack of.
        let mut armor_drop: Option<ItemStack> = None;
        let mut consume = false;
        let mut give: Option<&str> = None;
        let mut sound: Option<&str> = None;
        // Sounds played as vanilla plays them: (event, volume, pitch) at the feet.
        let mut eat: Option<(&'static str, f32, f32)> = None;
        let feet = [m.position.x, m.position.y, m.position.z];
        let mut handled = true;
        let mut drops: Vec<(String, u32)> = Vec::new();
        // The shearing table rolled, for the mods (none for a mooshroom).
        let mut drops_table = String::new();
        let mob = (m.id, super::native_hooks::mob_kind(m));
        // `Sheep.shear`'s drops add a spread on top of the ItemEntity's own
        // motion; `MushroomCow.shear`'s do not.
        let mut wool = false;
        let is_food = m.kind.food.contains(&item);
        let baby_turtle = species == Species::Turtle && m.is_baby();
        // MushroomCow.getEffectsFromItemStack: the item's block supplies them.
        let flower = if species == Species::Mooshroom {
            horizon_client::flowers::stew_effect(&item)
        } else {
            None
        };
        let mut give_stew: Option<Vec<(Arc<str>, i32)>> = None;
        // `Some(true)` for the four spell particles, `Some(false)` for the two
        // smoke puffs of a cow that already holds effects.
        let mut stew_particles: Option<bool> = None;
        // `Ocelot.mobInteract`: whether the fish won its trust.
        let mut ocelot_trust: Option<bool> = None;
        match species {
            Species::Cow | Species::Mooshroom if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some("entity.cow.milk");
            }
            // Goat.mobInteract: a bucket on an adult gives milk.
            Species::Goat if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some(if m.extra.screaming {
                    "entity.goat.screaming.milk"
                } else {
                    "entity.goat.milk"
                });
            }
            // MushroomCow.mobInteract: a bowl gives back what a flower left as
            // a suspicious stew, or plain stew when it holds no effects.
            Species::Mooshroom if item == "minecraft:bowl" && !m.is_baby() => {
                consume = true;
                match m.extra.stew_effects.take() {
                    Some(effects) => {
                        give = Some("minecraft:suspicious_stew");
                        give_stew = Some(effects);
                        // `playSound(.., 1.0F, 1.0F)` at the feet.
                        eat = Some(("entity.mooshroom.suspicious_milk", 1.0, 1.0));
                    }
                    None => {
                        give = Some("minecraft:mushroom_stew");
                        sound = Some("entity.mooshroom.milk");
                    }
                }
            }
            Species::Mooshroom if item == "minecraft:shears" && !m.is_baby() => {
                // MushroomCow.shear: becomes a cow and drops five mushrooms.
                let mushroom = if m.extra.variant == 1 {
                    "minecraft:brown_mushroom"
                } else {
                    "minecraft:red_mushroom"
                };
                drops.push((mushroom.into(), 5));
                m.removed = true;
                let setup = SpawnSetup {
                    yaw: m.yaw,
                    persistent: true,
                    ..Default::default()
                };
                let pos = [m.position.x, m.position.y, m.position.z];
                sound = Some("entity.mooshroom.shear");
                self.handle_mob_events(vec![MobEvent::SpawnKind {
                    kind: "minecraft:cow".into(),
                    at: pos,
                    setup,
                }])?;
                self.damage_held_tool();
            }
            // MushroomCow.mobInteract: a brown adult takes the flower's effects
            // unless it holds some already, when it only puffs smoke.
            Species::Mooshroom if m.extra.variant == 1 && !m.is_baby() && flower.is_some() => {
                if m.extra.stew_effects.is_some() {
                    stew_particles = Some(false);
                } else {
                    let (effect, ticks) = flower.expect("the guard checked");
                    m.extra.stew_effects = Some(vec![(Arc::from(effect), ticks)]);
                    consume = true;
                    eat = Some(("entity.mooshroom.eat", 2.0, 1.0));
                    stew_particles = Some(true);
                }
            }
            Species::Sheep if item == "minecraft:shears" && !m.extra.sheared && !m.is_baby() => {
                m.extra.sheared = true;
                let color = horizon_client::mobs::render::DYE_NAMES[m.extra.color as usize & 15];
                drops_table = format!("minecraft:shearing/sheep/{color}");
                let stacks = world_items!(self).loot_table(
                    &drops_table,
                    false,
                    None,
                    &mut self.world.random,
                );
                drops.extend(stacks.into_iter().map(|s| (s.item.to_string(), s.count)));
                wool = true;
                sound = Some("entity.sheep.shear");
                self.damage_held_tool();
            }
            Species::Sheep if item.ends_with("_dye") => {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                    && m.extra.color as usize != c
                    && !m.extra.sheared
                {
                    m.extra.color = c as u8;
                    consume = true;
                    sound = Some("item.dye.use");
                } else {
                    handled = false;
                }
            }
            // Entity.interact: shears take a tamed wolf's armor off for its owner.
            Species::Wolf
                if item == "minecraft:shears"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.head.is_some()
                    && !sneaking =>
            {
                let mut stack = ItemStack::new("minecraft:wolf_armor", 1);
                stack.damage = m.extra.body_damage.max(0) as u32;
                stack.extra = m.extra.body_extra.take();
                armor_drop = Some(stack);
                m.extra.head = None;
                m.extra.body_damage = 0;
                eat = Some(("item.armor.unequip_wolf", 1.0, 1.0));
                self.damage_held_tool();
            }
            // Wolf.mobInteract: heal, dye the collar or sit/stand when tame;
            // a bone tames a wild one a third of the time.
            Species::Wolf if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(2.0, |f| f.0 as f32 * 2.0);
                // TamableAnimal.feed(.., 2.0F, 2.0F): heals twice the nutrition,
                // then playEatingSound, which wolves leave empty.
                m.health = (m.health + heal).min(m.attrs.max_health);
                consume = true;
            }
            // Wolf.mobInteract: the owner puts wolf armor on a tamed adult.
            Species::Wolf
                if item == "minecraft:wolf_armor"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.head.is_none()
                    && !m.is_baby() =>
            {
                m.extra.head = Some(item.as_str().into());
                m.extra.body_damage = held_damage;
                m.extra.body_extra = held_extra;
                consume = true;
                eat = Some(("item.armor.equip_wolf", 1.0, 1.0));
            }
            // Wolf.mobInteract: an armadillo scute repairs a sitting wolf's
            // armor by an eighth of its durability.
            Species::Wolf
                if item == "minecraft:armadillo_scute"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.sitting
                    && m.extra.head.is_some()
                    && m.extra.body_damage > 0 =>
            {
                m.extra.body_damage = (m.extra.body_damage
                    - (horizon_client::mobs::WOLF_ARMOR_DURABILITY as f32 * 0.125) as i32)
                    .max(0);
                consume = true;
                eat = Some(("item.wolf_armor.repair", 1.0, 1.0));
            }
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                consume = true;
            }
            // Wolf.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Wolf
                if m.extra.tame && m.extra.owner == Some(acting) && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                m.ai.jumping = false;
                m.nav_stop();
                m.ai.target = None;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Wolf
                if !m.extra.tame
                    && item == "minecraft:bone"
                    && !horizon_client::mobs::tame::angry(m) =>
            {
                consume = true;
                let tamed = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if tamed {
                    horizon_client::mobs::tame::set_tame(m, true);
                    m.extra.owner = Some(acting);
                    m.persistent = true;
                    m.nav_stop();
                    m.ai.target = None;
                    m.extra.sitting = true;
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Cat.mobInteract: dye, heal or sit when tame; fish may tame a wild one.
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && horizon_client::mobs::render::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = horizon_client::mobs::render::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                m.persistent = true;
                consume = true;
            }
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && is_food
                    && m.health < m.attrs.max_health =>
            {
                let heal = self
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(1.0, |f| f.0 as f32);
                // TamableAnimal.feed.
                m.health = (m.health + heal).min(m.attrs.max_health);
                consume = true;
                eat = eating_sound(m, &mut self.world.random);
            }
            // Cat.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Cat
                if m.extra.tame && m.extra.owner == Some(acting) && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Parrot.mobInteract: the owner makes a tame parrot on the ground
            // sit or stand (poisonous food is handled apart).
            Species::Parrot
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.on_ground
                    && !parrot_poisonous =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Cat if !m.extra.tame && is_food => {
                consume = true;
                m.persistent = true;
                let tamed = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                    m.extra.sitting = true;
                }
                eat = eating_sound(m, &mut self.world.random);
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
                let m = &self.mobs[index];
                self.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // IronGolem.mobInteract: an iron ingot repairs 25 health.
            Species::IronGolem
                if item == "minecraft:iron_ingot" && m.health < m.attrs.max_health =>
            {
                m.health = (m.health + 25.0).min(m.attrs.max_health);
                consume = true;
                sound = Some("entity.iron_golem.repair");
            }
            Species::Creeper if creeper_igniter => {
                m.extra.flag = true;
                // FIRECHARGE_USE for a fire charge, else FLINTANDSTEEL_USE,
                // at the creeper's feet with a pitch from its own random.
                let event = if item == "minecraft:fire_charge" {
                    "item.firecharge.use"
                } else {
                    "item.flintandsteel.use"
                };
                eat = Some((event, 1.0, m.random.next_float() * 0.4 + 0.8));
                if damageable {
                    self.damage_held_tool();
                } else {
                    consume = true;
                }
            }
            // Parrot.mobInteract: #parrot_poisonous_food (a cookie) poisons the
            // parrot for 45 s and then hurts it with a player attack of
            // Float.MAX_VALUE, which an invulnerable one shrugs off.
            Species::Parrot
                if self
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:parrot_poisonous_food")
                    .is_some_and(|t| t.contains(&*item)) =>
            {
                consume = true;
                m.add_effect("minecraft:poison", 900, 0);
                let id = m.id;
                let hurt = m.damage(
                    f32::MAX,
                    horizon_client::mobs::Damage::player(player_at.unwrap_or(center)),
                );
                // The feeder gets the kill, as for a normal player attack.
                if hurt {
                    self.last_hurt_by_player.insert(id, self.lan_acting);
                }
            }
            // Parrot.mobInteract: parrot food may tame a wild one (one time in
            // ten); a parrot never takes it as Animal food (isFood is false).
            Species::Parrot if !m.extra.tame && is_food => {
                consume = true;
                if !m.silent {
                    let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
                    eat = Some(("entity.parrot.eat", 1.0, pitch));
                }
                let tamed = m.random.next_int_bound(10).unwrap_or(1) == 0;
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                }
            }
            // Dolphin.mobInteract: a fish makes a baby grow by a tenth and an
            // adult remember it (`gotFish`), to swim to a treasure.
            Species::Dolphin if fish => {
                eat = Some(("entity.dolphin.eat", 1.0, 1.0));
                if horizon_client::mobs::ai::can_age_up(m) {
                    let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                    horizon_client::mobs::ai::age_up(m, secs, true);
                } else {
                    m.extra.flag = true;
                }
                consume = true;
            }
            // Tadpole.feed: frog food ages it by a tenth of the time left to
            // grow into a frog (it grows on its next tick once past 24000).
            Species::Tadpole if is_food && !m.extra.age_locked => {
                consume = true;
                let left = 24_000 - m.extra.age;
                m.extra.age += ((left / 20) as f32 * 0.1) as i32 * 20;
                // One happy villager particle at rest about the body.
                self.particles_around_mob(
                    self.mobs[index].id,
                    "happy_villager",
                    horizon_client::particles::Burst::HORSE_EATING,
                );
            }
            // Bee.mobInteract: a wither rose or an open eyeblossom is eaten for
            // its `getBeeInteractionEffect` instead of breeding the bee.
            Species::Bee
                if is_food
                    && matches!(
                        &*item,
                        "minecraft:wither_rose" | "minecraft:open_eyeblossom"
                    ) =>
            {
                if &*item == "minecraft:wither_rose" {
                    m.add_effect("minecraft:wither", 40, 0);
                } else {
                    m.add_effect("minecraft:poison", 25, 0);
                }
                consume = true;
            }
            // Ocelot.mobInteract: raw fish from a hand within 3 blocks, while it comes for the
            // food (`temptGoal.isRunning`), is used up; one time in three it trusts the player
            // (entity event 41, hearts), otherwise smoke (event 40). Anything else is
            // `Animal.mobInteract`.
            Species::Ocelot
                if is_food
                    && !m.extra.trusting
                    && m.ai.goals.is_running("OcelotTemptGoal")
                    && player_feet.is_some_and(|p| {
                        let d = [
                            p[0] - m.position.x,
                            p[1] - m.position.y,
                            p[2] - m.position.z,
                        ];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 9.0
                    }) =>
            {
                consume = true;
                let trusts = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if trusts {
                    m.extra.trusting = true;
                }
                ocelot_trust = Some(trusts);
            }
            // HappyGhast.canFallInLove: never, so only a ghastling takes food.
            _ if is_food
                && m.kind.flags.animal
                && species != Species::Parrot
                && (species != Species::HappyGhast || m.is_baby()) =>
            {
                // Animal.mobInteract: breed adults, speed up babies, then
                // playEatingSound (silent unless the species overrides it).
                match feed_animal(m, &mut self.world.random, Some(acting)) {
                    Some(sound) => {
                        consume = true;
                        eat = sound;
                        // Axolotl.usePlayerItem: a tropical fish bucket is
                        // emptied into a water bucket in its place.
                        if species == Species::Axolotl && &*item == "minecraft:tropical_fish_bucket"
                        {
                            give = Some("minecraft:water_bucket");
                        }
                        // Animal.setInLove: entity event 18 (a baby that
                        // only grew is not in love).
                        if self.mobs[index].extra.in_love > 0 {
                            self.love_cause.insert(self.mobs[index].id, self.lan_acting);
                            self.mob_event_particles(self.mobs[index].id, 18);
                        }
                    }
                    // Animal.mobInteract still consumes the client click when
                    // an adult cannot breed, without swinging or taking food.
                    None => return Ok(true),
                }
            }
            _ => handled = false,
        }
        if !handled {
            return Ok(false);
        }
        if let Some(trusts) = ocelot_trust {
            // `spawnTrustingParticles` on the clients: entity event 41 (hearts) or 40 (smoke).
            self.mob_event_particles(self.mobs[index].id, if trusts { 41 } else { 40 });
        }
        if let Some(fresh) = stew_particles {
            // `SpellParticleOption.create(EFFECT, -1, 1)` is the plain white
            // effect particle; the smoke is the already-fed answer.
            let ty = if fresh {
                ParticleType::Effect
            } else {
                ParticleType::Smoke
            };
            let options = ParticleOptions::simple(ty);
            let at = [feet[0], feet[1] + 0.5, feet[2]];
            let (spread, count) = ([0.5, 0.0, 0.5], if fresh { 4 } else { 2 });
            // shortcut: scalar speed models DEFAULT randomization; 26.3 sends
            // ALTERNATIVE_WITH_SPEED with a per-axis maximum speed.
            self.particles
                .send_particles(&options, at, spread, 0.2, count);
            self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options,
                state: None,
                at,
                spread,
                speed: 0.2,
                count: count as i32,
                randomization: 0,
            });
        }
        // PlayerInteractTrigger: the item used on the mob, once the interaction took.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        // Turtle.ageBoundaryReached: food that finishes a baby's growth.
        if baby_turtle && !self.mobs[index].is_baby() {
            self.handle_mob_events(vec![MobEvent::GiftLoot {
                mob: self.mobs[index].id,
                table: "minecraft:gameplay/turtle_grow",
                at: feet,
            }])?;
        }
        self.swing_arm();
        if let Some(s) = sound {
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio
                .play(&format!("minecraft:{s}"), Some(center), 1.0, pitch);
        }
        if let Some((s, volume, pitch)) = eat {
            self.audio
                .play(&format!("minecraft:{s}"), Some(feet), volume, pitch);
        }
        if consume
            && !creative
            && let Some(stack) = self.stack_in_mut(selected).as_mut()
        {
            stack.count -= 1;
            if stack.count == 0 {
                *self.stack_in_mut(selected) = None;
            }
        }
        if let Some(g) = give {
            let mut stack = ItemStack {
                item: g.into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            if let Some(effects) = give_stew {
                stack.extra = Some(Box::new(horizon_client::item_data::StackData {
                    stew_effects: effects,
                    ..Default::default()
                }));
            }
            if self.stack_in(selected).is_none() {
                *self.stack_in_mut(selected) = Some(stack);
            } else if let Some(rest) = self.give(stack) {
                let at = self.eye_position().unwrap_or(center);
                self.spawn_item(rest, at, [0.0, 0.0, 0.0], 40);
            }
        }
        if let Some(stack) = armor_drop {
            // Mob.shearItem: PLAYER_SHEARED_EQUIPMENT with the armor and the wolf.
            let target = self.mob_entity(&self.mobs[index]);
            self.trigger_item_entity("minecraft:player_sheared_equipment", &stack, &target);
            self.spawn_item(stack, [center[0], center[1] + 0.5, center[2]], [0.0; 3], 10);
        }
        let mut drops: Vec<ItemStack> = drops
            .into_iter()
            .map(|(item, count)| ItemStack::new(&item, count))
            .collect();
        if !drops.is_empty() {
            self.native_use_drops(
                &drops_table,
                feet,
                None,
                Some((mob.0, &mob.1)),
                held.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            for _ in 0..stack.count {
                // `spawnAtLocation(stack, 1.0)`: an ItemEntity a block above
                // the feet that starts with the constructor's own motion, and
                // `setDefaultPickUpDelay`. Sheep.shear adds its spread on top;
                // MushroomCow.shear builds the entity by hand instead, so its
                // mushrooms keep the constructor's motion and no pickup delay.
                let mut v = dispenser::new_item_motion();
                if wool {
                    v[0] += (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                    v[1] += self.world.random.next_float() as f64 * 0.05;
                    v[2] += (self.world.random.next_float() - self.world.random.next_float())
                        as f64
                        * 0.1;
                }
                self.spawn_item(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    [feet[0], feet[1] + 1.0, feet[2]],
                    v,
                    if wool { 10 } else { 0 },
                );
            }
        }
        Ok(true)
    }
    /// `SnowGolem.mobInteract` and `Bogged.mobInteract` with shears: the
    /// pumpkin or the mushrooms come off (the shearing loot table), the shear
    /// game event is posted and the tool takes one damage.
    fn interact_shear_mob(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if item != "minecraft:shears" || !m.alive() || m.extra.sheared {
            return false;
        }
        let (sound, table, height) = match m.species() {
            Species::SnowGolem => (
                "minecraft:entity.snow_golem.shear",
                "minecraft:shearing/snow_golem",
                m.eye_height(),
            ),
            Species::Bogged => (
                "minecraft:entity.bogged.shear",
                "minecraft:shearing/bogged",
                f64::from(m.height()),
            ),
            _ => return false,
        };
        let at = m.position;
        let mob = (m.id, super::native_hooks::mob_kind(m));
        let tool = self.held().cloned();
        self.audio.play(sound, Some([at.x, at.y, at.z]), 1.0, 1.0);
        self.mobs[index].extra.sheared = true;
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let origin = [at.x, at.y, at.z];
        self.native_use_drops(
            table,
            origin,
            None,
            Some((mob.0, &mob.1)),
            tool.as_ref(),
            &mut stacks,
        );
        for stack in stacks {
            self.spawn_at_location(at, height, stack);
        }
        let ctx = self.who_context(Some(horizon_client::mobs::Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        true
    }
    /// `Armadillo.mobInteract`: a brush brushes a scute off an adult
    /// (`brushOffScute`) and loses 16 durability.
    fn interact_armadillo(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if item != "minecraft:brush"
            || m.species() != Species::Armadillo
            || !m.alive()
            || m.is_baby()
        {
            return false;
        }
        let (id, at) = (m.id, m.position);
        let feet = [at.x, at.y, at.z];
        let tool = self.held().cloned();
        let table = "minecraft:brush/armadillo";
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let kind = super::native_hooks::mob_kind(&self.mobs[index]);
        self.native_use_drops(
            table,
            feet,
            None,
            Some((id, &kind)),
            tool.as_ref(),
            &mut stacks,
        );
        for d in stacks {
            self.spawn_at_location(at, 0.0, d);
        }
        self.audio
            .play("minecraft:entity.armadillo.brush", Some(feet), 1.0, 1.0);
        let ctx = self.who_context(Some(Who::Mob(id)), None);
        self.post_game_event("minecraft:entity_interact", feet, ctx);
        // ItemStack.hurtAndBreak(16).
        if !self.creative
            && let Some(stack) = tool
        {
            let max = self
                .res
                .items()
                .get(&stack.item)
                .map_or(0, |i| i.max_damage);
            if max > 0 {
                self.damage_held(16);
            }
        }
        true
    }
    /// `ItemStack.hurtAndBreak(1, ...)` on the held tool: Unbreaking, break sound and stat.
    pub(super) fn damage_held_tool(&mut self) {
        self.damage_held(1);
    }
    /// Throwables and spawn eggs (SnowballItem/EggItem/EnderpearlItem/SpawnEggItem.use).
    pub(super) fn throw_item(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let kind = match item {
            "minecraft:snowball" => ProjectileKind::Snowball,
            "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => ProjectileKind::Egg,
            "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
            "minecraft:wind_charge" => {
                let thrown = self.throw_wind_charge()?;
                if thrown {
                    self.apply_use_cooldown(stack);
                }
                return Ok(thrown);
            }
            _ if item.ends_with("_spawn_egg") => return self.use_spawn_egg(stack),
            _ => match horizon_client::projectiles::thrown_by_item(item) {
                Some(t) => ProjectileKind::Thrown(t),
                None => return Ok(false),
            },
        };
        let Some(eye) = self.eye_position() else {
            return Ok(false);
        };
        let Some(p) = self.player.as_ref() else {
            return Ok(false);
        };
        // Projectile.shootFromRotation adds getKnownMovement: what the thrower
        // last moved, not a velocity the server kept for a guest.
        let on_ground = p.ground.on_ground;
        let velocity = self.known_player_movement();
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut proj = Projectile::new(kind.clone(), start, Some(Who::Player));
        proj.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            1.5,
            1.0,
            velocity,
            on_ground,
        );
        self.add_projectile(proj);
        self.player_projectile_shot();
        // ItemStack.use's applyAfterUseComponentSideEffects: the pearl's
        // second is the `use_cooldown` component, as for any thrown item.
        self.apply_use_cooldown(stack);
        let sound = match kind {
            ProjectileKind::Snowball | ProjectileKind::Thrown(_) => {
                "minecraft:entity.snowball.throw"
            }
            ProjectileKind::Egg => "minecraft:entity.egg.throw",
            _ => "minecraft:entity.ender_pearl.throw",
        };
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        self.audio.play(sound, Some(eye), 0.5, pitch);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.held_mut() = None;
            }
        }
        Ok(true)
    }
    /// `Mob.checkAndHandleImportantInteractions` with a spawn egg of mob
    /// `index`'s own kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`): a baby
    /// at the mob's feet, taking the parent's variant as `getBreedOffspring`
    /// does, and one egg used. False (PASS) when the mob has no baby form.
    fn spawn_egg_offspring(&mut self, index: usize) -> bool {
        let Some(stack) = self.held().cloned() else {
            return false;
        };
        let parent = &self.mobs[index];
        if !parent.alive()
            || !horizon_client::mobs::finalize::spawn_egg_is_for(&stack.item, &parent.kind.id)
            || !horizon_client::mobs::drops::can_be_baby(parent.species())
            // WanderingTrader.getBreedOffspring returns null: no offspring, the
            // click carries on to trading.
            || parent.species() == Species::WanderingTrader
        {
            return false;
        }
        let (kind, at) = (
            parent.kind.id.clone(),
            [parent.position.x, parent.position.y, parent.position.z],
        );
        let (species, variant, color) =
            (parent.species(), parent.extra.variant, parent.extra.color);
        let mob_type = parent.kind.clone();
        let target = self.mob_entity(&self.mobs[index]);
        let id = if species == Species::Piglin {
            // SpawnEggItem.spawnOffspringFromSpawnEgg: a non-AgeableMob is
            // constructed, then setBaby(true), without finalizeSpawn. Giving
            // it adult spawn gear first leaves the baby holding a weapon.
            self.next_entity += 1;
            let id = self.next_entity;
            self.mobs
                .push(Mob::new(id, mob_type, Vec3::new(at[0], at[1], at[2]), 0.0));
            id
        } else {
            let Some(id) = self.spawn_mob_spawned(&kind, at) else {
                return false;
            };
            id
        };
        if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::mobs::finalize::spawn_egg_offspring(child, species, variant, color);
        }
        if species == Species::Piglin {
            self.native_mobs_joined(self.mobs.len() - 1, false);
        }
        // `Mob.checkAndHandleImportantInteractions` then calls
        // `onOffspringSpawnedFromEgg`: `Fox`'s makes the baby trust the player
        // who used the egg (a fox egg used on a block trusts nobody).
        if species == Species::Fox {
            let spawner = self.acting_player_uuid();
            if let Some(child) = self.mobs.iter_mut().find(|m| m.id == id) {
                horizon_client::mobs::fox::add_trusted_entity(child, spawner);
            }
        }
        self.trigger_item_entity("minecraft:player_interacted_with_entity", &stack, &target);
        self.consume_held();
        true
    }
    /// `SpawnEggItem.use`: the SOURCE_ONLY ray must hit a liquid block.
    pub(super) fn spawn_egg_fluid_hit(&self) -> Option<super::Hit> {
        let hit = self.bucket_hit()?;
        matches!(
            &*self.block_name(hit.pos),
            "minecraft:water" | "minecraft:lava"
        )
        .then_some(hit)
    }
    /// Shared peaceful-mode eligibility for spawn eggs.
    pub(super) fn spawn_egg_allowed(&self, item: &str) -> bool {
        let kind = item.trim_end_matches("_spawn_egg");
        self.ui.difficulty != 0
            || self
                .res
                .mob_types()
                .get(kind)
                .is_none_or(|t| horizon_client::mobs::spawn_rules::allowed_in_peaceful(t.species))
    }
    /// `SpawnEggItem.useOn` for a block, or `use` for a fluid source.
    fn use_spawn_egg(&mut self, stack: &ItemStack) -> Result<bool> {
        let fluid = matches!(self.guest_use, Some(super::GuestUse::Item(_)));
        let Some(hit) = (if fluid {
            self.spawn_egg_fluid_hit()
        } else {
            self.pick()
        }) else {
            return Ok(false);
        };
        let kind = stack.item.trim_end_matches("_spawn_egg").to_owned();
        if !self.spawn_egg_allowed(&stack.item) {
            self.use_failed = true;
            return Ok(true);
        }
        if fluid
            && !self.may_build()
            && !self.can_place_on_in_adventure([
                hit.pos[0] - hit.face[0],
                hit.pos[1] - hit.face[1],
                hit.pos[2] - hit.face[2],
            ])
        {
            self.use_failed = true;
            return Ok(true);
        }
        // SpawnEggItem.useOn on a spawner sets its mob (and an op's egg data).
        let (spawner_id, data) = horizon_client::block_entity_item::spawn_egg_on_spawner(
            stack,
            self.acting_player_is_op(),
        );
        if let Some(done) = self.use_spawn_egg_on_spawner(hit.pos, &spawner_id, data.as_ref()) {
            if done {
                self.swing_arm();
                self.stats.add("used", &stack.item, 1);
                self.consume_held();
            }
            return Ok(true);
        }
        // useOn: a clicked block without a collision shape (vines, grass)
        // takes the mob itself, any other the one on the clicked face.
        let p = if fluid || self.collision_empty(hit.pos) {
            hit.pos
        } else {
            [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ]
        };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob_spawned(&kind, at) else {
            return Ok(false);
        };
        // SpawnEggItem.spawnMob: `EntityType.create` finalizes the mob, then
        // `PostSpawnProcessor` puts the egg's components on it.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            horizon_client::entity_nbt::apply_item_components(m, stack);
        }
        self.apply_spawn_egg_entity_data(id, stack);
        self.post_block_game_event("minecraft:entity_place", p, Some(Who::Player), None);
        self.stats.add("used", &stack.item, 1);
        self.swing_arm();
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                *self.held_mut() = None;
            }
        }
        Ok(true)
    }
    /// `EntityType.updateCustomEntityTag`: the egg's `minecraft:entity_data` on
    /// the mob it spawned, once the guards let it through, as
    /// `TypedEntityData.loadInto`: the mob's own compound, the data merged over
    /// it, loaded back into the same mob (`/data merge entity`'s loader).
    pub(super) fn apply_spawn_egg_entity_data(&mut self, id: u32, stack: &ItemStack) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let kind = self.mobs[i].kind.id.clone();
        let op = self.acting_player_is_op();
        let Some(data) = horizon_client::block_entity_item::spawn_egg_entity_data(stack, &kind, op)
        else {
            return;
        };
        let own = self.mob_tree_nbt(i);
        let tag = horizon_client::data_cmd::merged(&own, data);
        // Nothing to report: the mob's own compound always loads back.
        let _ = self.apply_entity_nbt(super::commands::Target::Mob(id), tag);
    }
    /// A beam target's centre (half its height), interpolated.
    fn beam_target(&self, who: Option<Who>, alpha: f64) -> Option<glam::DVec3> {
        match who? {
            Who::Player => {
                let p = self.player.as_ref()?;
                let (a, b) = (self.previous_position, p.position);
                Some(glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(p.dimensions.height) * 0.5,
                    a.z + (b.z - a.z) * alpha,
                ))
            }
            Who::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                Some(glam::DVec3::new(
                    m.previous.x + (m.position.x - m.previous.x) * alpha,
                    m.previous.y
                        + (m.position.y - m.previous.y) * alpha
                        + f64::from(m.height()) * 0.5,
                    m.previous.z + (m.position.z - m.previous.z) * alpha,
                ))
            }
        }
    }
    /// `Guardian.aiStep` on a client: two bubbles a tick trailing a guardian
    /// that swims along, and bubbles up the beam to its target. The server
    /// never draws them, so the mob's own random stays untouched.
    pub(super) fn guardian_bubbles(&mut self) {
        use horizon_client::mobs::guardian;
        use horizon_client::particles::{ParticleOptions, ParticleType};
        let mut swimming = Vec::new();
        let mut beams = Vec::new();
        for m in &self.mobs {
            if !m.alive() || !matches!(m.species(), Species::Guardian | Species::ElderGuardian) {
                continue;
            }
            let at = [m.position.x, m.position.y, m.position.z];
            if m.extra.flag2 && m.fluids.in_water {
                let (width, height, _) = m.dims();
                swimming.push((at, width, height, m.view_vector()));
            }
            if let Some((_, scale)) = guardian::beam(m, 0.0)
                && let Some(t) = self.beam_target(m.ai.target, 1.0)
            {
                beams.push(([at[0], m.eye_y(), at[2]], [t.x, t.y, t.z], f64::from(scale)));
            }
        }
        let bubble = ParticleOptions::simple(ParticleType::Bubble);
        for (at, width, height, view) in swimming {
            for _ in 0..2 {
                let (x, y, z) = {
                    let r = self.particles.random();
                    (r.next_double(), r.next_double(), r.next_double())
                };
                let p = [
                    at[0] + width * (2.0 * x - 1.0) * 0.5 - view[0] * 1.5,
                    at[1] + height * y - view[1] * 1.5,
                    at[2] + width * (2.0 * z - 1.0) * 0.5 - view[2] * 1.5,
                ];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
        for (eye, target, scale) in beams {
            let d = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length <= 0.0 {
                continue;
            }
            let mut dist = self.particles.random().next_double();
            while dist < length {
                let r = self.particles.random();
                dist += 1.8 - scale + r.next_double() * (1.7 - scale);
                let f = dist / length;
                let p = [eye[0] + d[0] * f, eye[1] + d[1] * f, eye[2] + d[2] * f];
                self.particles.add(&bubble, p, [0.0; 3]);
            }
        }
    }
    /// Mob models and projectiles for the entity pass.
    pub(super) fn mob_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let atlas = &self.gui.atlas;
        let lookup = |name: &str| atlas.sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let scale = self.view_scale();
        self.rendered_mobs.set(0);
        // What custom entity model animations read of the frame.
        horizon_client::mobs::cem_anim::set_frame(horizon_client::mobs::cem_anim::Frame {
            time: self.world.game_time as f64 + alpha,
            day_time: self.world.day_time as f64 + alpha,
            dimension: match self.dim {
                Dim::Overworld => 0.0,
                Dim::Nether => -1.0,
                Dim::End => 1.0,
            },
            player: self.player.as_ref().map_or(camera.to_array(), |p| {
                [p.position.x, p.position.y, p.position.z]
            }),
            player_rot: [self.pitch, self.yaw],
            paused: self.pause,
            first_person: self.camera_type.first_person(),
            third_person_reversed: self.camera_type
                == super::perspective::CameraType::ThirdPersonFront,
            ..Default::default()
        });
        for mob in &self.mobs {
            let entity_start = out.len();
            self.mark_motion(entity_start, (1 << 32) | u64::from(mob.id));
            let outline = self.mob_outline(mob);
            // Entity.DATA_SHARED_FLAGS_ID invisible: only the name tag draws (and
            // the outline of a glowing mob, LivingEntityRenderer.getRenderType).
            // A player keeps its armour, elytra and held items
            // (`LivingEntityRenderer.submit` skips only the body).
            let is_player = &*mob.kind.id == "minecraft:player";
            let layers_only = mob.invisible && outline.is_none() && is_player;
            if mob.invisible && outline.is_none() && !layers_only {
                continue;
            }
            // LevelRenderer: the entity the view looks out of is not drawn
            // in first person.
            if self.hides_as_camera(mob.id) {
                continue;
            }
            let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
            // Entity.shouldRenderAtSqrDistance: 64 times the bounding box's
            // mean edge, scaled by the view scale. RemotePlayer's box counts
            // ten times.
            let (w, h, _) = mob.dims();
            let edge = (w + h + w) / 3.0
                * if &*mob.kind.id == "minecraft:player" {
                    10.0
                } else {
                    1.0
                };
            let range = edge * 64.0 * scale;
            if d.length_squared() >= range * range {
                continue;
            }
            // EntityRenderer.shouldRender: the culling box, grown by 0.5
            // (GuardianRenderer also draws while its beam is out).
            if let Some(view) = &self.view_frustum
                && horizon_client::mobs::guardian::beam(mob, alpha as f32).is_none()
            {
                let lo =
                    d.as_vec3() - glam::Vec3::new(w as f32 / 2.0 + 0.5, 0.5, w as f32 / 2.0 + 0.5);
                let hi = d.as_vec3()
                    + glam::Vec3::new(w as f32 / 2.0 + 0.5, h as f32 + 0.5, w as f32 / 2.0 + 0.5);
                if !view.intersects(lo, hi) {
                    continue;
                }
            }
            self.mark_entity(entity_start, &mob.kind.id);
            self.rendered_mobs.set(self.rendered_mobs.get() + 1);
            let light = self.world.light(
                mob.position.x.floor() as i32,
                (mob.position.y + mob.eye_height()).floor() as i32,
                mob.position.z.floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            // EntityRenderer.getBlockLightLevel: a burning mob has 15 block
            // light, and so do the Allay, Blaze, Magma Cube, Vex and Wither
            // renderers' overrides; the sky light stays the level's.
            if mob.fire_ticks > 0
                || matches!(
                    mob.species(),
                    Species::Allay
                        | Species::Blaze
                        | Species::MagmaCube
                        | Species::Vex
                        | Species::Wither
                )
            {
                packed = (packed & 0xFF) | ((15 * 16) << 8);
            }
            // GlowSquidRenderer.getBlockLightLevel: the body brightens as the
            // dark ticks run out (the block light is the second byte).
            if mob.species() == Species::GlowSquid {
                let lit = (packed >> 12) & 15;
                let block = horizon_client::mobs::aquatic::glow_squid_block_light(mob, lit);
                packed = (packed & 0xFF) | ((block * 16) << 8);
            }
            // A native mod's own model in place of the vanilla one
            // (`native_mob_submit`), before any of the vanilla model's parts
            // (a sulfur cube's swallowed block) are drawn.
            // A player's own hook (`native_player_submit`) is asked for the
            // other players in view.
            let who = is_player
                .then(|| self.remote_player_name(mob.id))
                .flatten()
                .map(|name| NativePlayerRender { name, local: false });
            let modded = if layers_only {
                false
            } else if let Some(who) = who {
                self.mod_player_geometry(mob, who, alpha as f32, camera, packed, out)
            } else {
                !is_player && self.mod_mob_geometry(mob, alpha as f32, camera, packed, out)
            };
            if modded {
                if let Some(rgb) = outline {
                    self.mark_outline(&out[entity_start..], rgb);
                    if mob.invisible {
                        out.truncate(entity_start);
                        continue;
                    }
                }
                if outline.is_none() {
                    self.cull_entity_vertices(out, entity_start);
                }
                continue;
            }
            if mob.species() == Species::SulfurCube
                && !horizon_client::mobs::trader::is_invisible(mob)
            {
                self.sulfur_cube_body_geometry(
                    mob,
                    alpha as f32,
                    horizon_client::mobs::render::base_transform(mob, alpha as f32, camera),
                    packed,
                    out,
                );
            }
            // Other players on a server: PlayerRenderer's HumanoidArmorLayer.
            let armor = if is_player {
                self.worn_armor_layers(mob)
            } else {
                Vec::new()
            };
            let body = out.len();
            let render = if layers_only {
                horizon_client::mobs::render::render_layers
            } else {
                horizon_client::mobs::render::render_with
            };
            render(
                mob,
                alpha as f32,
                camera,
                &lookup,
                packed,
                self.res.mob_data(),
                &armor,
                out,
            );
            // HDR output: mobs that are light shine by their kind, as blocks do (entity.wgsl
            // fs_hdr_mark); a tier's window lifts only their bright texels.
            if let Some(tier) = match mob.species() {
                Species::Blaze => Some(horizon_client::mesher::BRIGHT),
                Species::MagmaCube | Species::GlowSquid => Some(horizon_client::mesher::LOW),
                _ => None,
            } {
                for v in &mut out[body..] {
                    v.layer |= u32::from(tier) << 16;
                }
            }
            if is_player {
                // PlayerRenderer's CapeLayer, WingsLayer, ParrotOnShoulderLayer
                // and SpinAttackEffectLayer. Only the cape hides with the body;
                // the others have no `isInvisible` check.
                let chest = mob.extra.chest.as_deref();
                if !layers_only {
                    self.player_cape_geometry(mob, chest, camera, alpha as f32, packed, out);
                }
                if let Some((_, name)) = self
                    .remote_player_names()
                    .into_iter()
                    .find(|(id, _)| *id == mob.id)
                {
                    self.player_ears_geometry(mob, &name, camera, alpha as f32, packed, out);
                }
                self.player_elytra_geometry(mob, chest, camera, alpha as f32, packed, out);
                self.shoulder_parrots_geometry(mob, camera, alpha as f32, packed, out);
                self.spin_attack_geometry(mob, camera, alpha as f32, packed, out);
                self.stuck_in_body_geometry(mob, mob.id as i32, camera, alpha as f32, packed, out);
            }
            // CustomHeadLayer (a pumpkin, skull or head item) included.
            self.mob_block_geometry(mob, camera, alpha, packed, out);
            // `native_mob_layer`: a native mod's layer, over the vanilla ones.
            self.mod_mob_layer_geometry(mob, who, alpha as f32, camera, packed, out);
            if let Some(rgb) = outline {
                self.mark_outline(&out[entity_start..], rgb);
                if mob.invisible {
                    out.truncate(entity_start);
                    continue;
                }
            }
            // Glowing outlines and a guardian's beam remain visible independently
            // of whether the body is hidden behind terrain.
            if outline.is_none()
                && horizon_client::mobs::guardian::beam(mob, alpha as f32).is_none()
            {
                self.cull_entity_vertices(out, entity_start);
            }
            // GuardianRenderer: the laser to the locked target.
            if let Some((time, scale)) = horizon_client::mobs::guardian::beam(mob, alpha as f32)
                && let Some(target) = self.beam_target(mob.ai.target, alpha)
                && let Some(uv) = lookup("entity/guardian/guardian_beam")
            {
                let a = alpha;
                let eye = glam::DVec3::new(
                    mob.previous.x + (mob.position.x - mob.previous.x) * a,
                    mob.previous.y + (mob.position.y - mob.previous.y) * a + mob.eye_height(),
                    mob.previous.z + (mob.position.z - mob.previous.z) * a,
                ) - camera;
                horizon_client::mobs::guardian::beam_geometry(
                    eye.as_vec3(),
                    (target - camera).as_vec3(),
                    time,
                    scale,
                    uv,
                    out,
                );
            }
        }
        // What follows isn't a tracked mob: close the last one's span.
        self.mark_motion(out.len(), 0);
        self.elder_apparition_geometry(alpha, out);
        self.spawner_geometry(camera, alpha, out);
        // Projectiles: the baked vanilla arrow model, the rest as item billboards.
        let (right, up) = thrown_item_axes(self.camera_type, self.yaw, self.pitch);
        let live = self.creatures.projectiles.iter().map(|a| {
            let at = [
                a.previous.x + (a.position.x - a.previous.x) * alpha,
                a.previous.y + (a.position.y - a.previous.y) * alpha,
                a.previous.z + (a.position.z - a.previous.z) * alpha,
            ];
            (a, at)
        });
        // ItemPickupParticle: a picked-up arrow flying to its collector.
        let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
            super::pickups::Taken::Arrow(a) => Some((&**a, super::pickups::position(p, alpha))),
            _ => None,
        });
        // A glowing projectile's outline: what it drew up to the next one.
        let mut glowing = None;
        for (a, at) in live.chain(taken) {
            if let Some((start, rgb)) = glowing.take() {
                self.mark_outline(&out[start..], rgb);
            }
            if a.id != 0 && self.entity_glowing(a.id) {
                glowing = Some((out.len(), super::glowing::team_color(self.mob_team(a.id))));
            }
            let p = glam::DVec3::from_array(at) - camera;
            let c = p.as_vec3();
            let light = self.world.light(
                a.position.x.floor() as i32,
                a.position.y.floor() as i32,
                a.position.z.floor() as i32,
            );
            let mut packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            if a.is_fireball() {
                packed = (15 * 16) | ((15 * 16) << 8);
            }
            if a.is_arrow() {
                let Some(model) = self.arrow_model.as_ref() else {
                    continue;
                };
                // SpectralArrowRenderer, and TippableArrowRenderer's `getColor() > 0`.
                let texture = if a.is_spectral_arrow() {
                    "entity/projectiles/arrow_spectral"
                } else if a.arrow.as_ref().is_some_and(|s| s.effect_color > 0) {
                    "entity/projectiles/arrow_tipped"
                } else {
                    "entity/projectiles/arrow"
                };
                let Some(r) = atlas.sprites.get(texture).copied() else {
                    continue;
                };
                // ArrowRenderer: `Mth.lerp(partialTick, state.yRotO, state.yRot)`
                // and the same for the pitch, so a turning arrow is drawn
                // between the two ticks it turned over.
                let alpha32 = alpha as f32;
                let yaw = a.previous_yaw + alpha32 * (a.yaw - a.previous_yaw);
                let pitch = a.previous_pitch + alpha32 * (a.pitch - a.previous_pitch);
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z(pitch.to_radians());
                // ArrowRenderer: `state.shake = shakeTime - partialTicks`; ArrowModel.setupAnim
                // rolls the root while the shake timer runs.
                let shake = a
                    .arrow
                    .as_ref()
                    .map_or(0.0, |s| s.shake as f32 - alpha as f32);
                let roll = if shake > 0.0 {
                    -(shake * 3.0).msin() * shake * (std::f32::consts::PI / 180.0)
                } else {
                    0.0
                };
                let pose_for = |part: &horizon_client::mobs::Part| {
                    let mut pose = part.pose;
                    if std::ptr::eq(part, model) {
                        pose[5] += roll;
                    }
                    pose
                };
                horizon_client::mobs::render_model(
                    model,
                    m,
                    &pose_for,
                    [r.u0, r.v0, r.u1, r.v1],
                    packed,
                    0,
                    out,
                );
                if a.arrow.as_ref().is_some_and(|s| s.on_fire()) {
                    flame_geometry(atlas, c, 0.5, 0.5, self.yaw, light, out);
                }
                continue;
            }
            // ThrownTridentRenderer.
            if matches!(a.kind, ProjectileKind::Trident) {
                let Some(model) = self.trident_model.as_ref() else {
                    continue;
                };
                let Some(r) = atlas.sprites.get("entity/trident/trident").copied() else {
                    continue;
                };
                let m = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_rotation_y((a.yaw - 90.0).to_radians())
                    * glam::Mat4::from_rotation_z((a.pitch + 90.0).to_radians());
                let poses = horizon_client::mobs::anim::Poses::new(model);
                horizon_client::mobs::render::draw(
                    model,
                    m,
                    &poses,
                    [r.u0, r.v0, r.u1, r.v1],
                    0xFFFF_FFFF,
                    packed,
                    0,
                    out,
                );
                continue;
            }
            // WindChargeRenderer.
            if matches!(
                a.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            ) {
                self.wind_charge_geometry(a, c, packed, alpha as f32, out);
                continue;
            }
            // ShulkerBulletRenderer: the tumbling bullet model, then a larger
            // translucent copy of it (colour 0x26FFFFFF); always 15 block light.
            if matches!(a.kind, ProjectileKind::ShulkerBullet { .. }) {
                let model = &self.entity_models.shulker_bullet;
                if let (Some(model), Some(r)) =
                    (model, atlas.sprites.get("entity/shulker/spark").copied())
                {
                    use horizon_core::geometry::trig;
                    let tc = a.age as f32 + alpha as f32;
                    let mut poses = horizon_client::mobs::anim::Poses::new(model);
                    poses.yrot("main", a.yaw.to_radians());
                    poses.xrot("main", a.pitch.to_radians());
                    let light = (packed & 0xFF) | ((15 * 16) << 8);
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_translation(glam::Vec3::new(0.0, 0.15, 0.0))
                        * glam::Mat4::from_rotation_y(
                            (trig::sin(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_x(
                            (trig::cos(f64::from(tc * 0.1)) * 180.0).to_radians(),
                        )
                        * glam::Mat4::from_rotation_z(
                            (trig::sin(f64::from(tc * 0.15)) * 360.0).to_radians(),
                        )
                        * glam::Mat4::from_scale(glam::Vec3::new(-0.5, -0.5, 0.5));
                    let uv = [r.u0, r.v0, r.u1, r.v1];
                    horizon_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        uv,
                        0xFFFF_FFFF,
                        light,
                        0,
                        out,
                    );
                    // The translucent shell blends over the opaque core
                    // (`RenderTypes.entityTranslucent`).
                    let shell = out.len();
                    horizon_client::mobs::render::draw(
                        model,
                        base * glam::Mat4::from_scale(glam::Vec3::splat(1.5)),
                        &poses,
                        uv,
                        0x26FF_FFFF,
                        light,
                        0,
                        out,
                    );
                    horizon_client::gfx::mark_oit(&mut out[shell..]);
                }
                continue;
            }
            // WitherSkullRenderer: the skull model facing its flight.
            if let Some(dangerous) = super::wither::skull_kind(a) {
                let model = &self.entity_models.wither_skull;
                let tex = if dangerous {
                    "entity/wither/wither_invulnerable"
                } else {
                    "entity/wither/wither"
                };
                if let (Some(model), Some(r)) = (model, atlas.sprites.get(tex).copied()) {
                    let mut poses = horizon_client::mobs::anim::Poses::new(model);
                    poses.yrot("head", a.yaw.to_radians());
                    poses.xrot("head", a.pitch.to_radians());
                    let base = glam::Mat4::from_translation(c)
                        * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
                    horizon_client::mobs::render::draw(
                        model,
                        base,
                        &poses,
                        [r.u0, r.v0, r.u1, r.v1],
                        0xFFFF_FFFF,
                        packed,
                        0,
                        out,
                    );
                }
                continue;
            }
            if let Some(stack) = a.drawn_item() {
                let owner = super::item_draw::owner_at(at, 0.0, a.kind.id());
                let state = self.resolve_item(&stack, DisplayContext::Ground, Some(&owner), 0);
                let pose = glam::Mat4::from_translation(c)
                    * glam::Mat4::from_quat(self.camera_orientation());
                self.submit_item_world(&state, pose, packed, out);
                continue;
            }
            let Some(name) = a.item_sprite() else {
                continue;
            };
            let Some(r) = atlas.sprites.get(name).copied() else {
                continue;
            };
            let lift = is_item_billboard(&a.kind);
            let quad = thrown_item_quad(c, right, up, a.sprite_scale(), lift);
            for (v, uv) in quad
                .iter()
                .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
            {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
        if let Some((start, rgb)) = glowing {
            self.mark_outline(&out[start..], rgb);
        }
    }
}
/// `ThrownItemRenderer.submit` turns the item by `CameraRenderState.orientation`
/// (`Camera.setRotation`): the billboard's right and up axes are the camera's.
/// They stay continuous at pitch ±90, and in third-person front view they are
/// the turned-round camera's, not the player's.
fn thrown_item_axes(
    camera_type: super::perspective::CameraType,
    yaw: f32,
    pitch: f32,
) -> (glam::Vec3, glam::Vec3) {
    let orientation = super::camera_rotation(camera_type, yaw, pitch);
    (orientation * glam::Vec3::X, orientation * glam::Vec3::Y)
}
/// The four corners of the item billboard, in the order its UVs are laid out.
/// `ThrownItemRenderer` only scales and billboards it; the lift is
/// `item/generated`'s ground display transform, `translation [0, 2, 0]` of 16
/// units = 0.125 blocks, which is `0.125 * scale` == `0.5 * s`.
fn thrown_item_quad(
    at: glam::Vec3,
    right: glam::Vec3,
    up: glam::Vec3,
    scale: f32,
    lift: bool,
) -> [glam::Vec3; 4] {
    let s = 0.25 * scale;
    let rise = if lift { 0.5 * s } else { 0.0 };
    let centre = at + up * rise;
    [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|k| centre + right * k.0 * s + up * k.1 * s)
}
/// Whether vanilla draws the kind as `ThrownItemRenderer`'s item billboard, the
/// only ones that take that lift. Llama spit shares this quad but
/// `LlamaSpitRenderer` is a model, with its own offset.
fn is_item_billboard(kind: &ProjectileKind) -> bool {
    matches!(
        kind,
        ProjectileKind::Snowball
            | ProjectileKind::Egg
            | ProjectileKind::EnderPearl
            | ProjectileKind::SmallFireball
            | ProjectileKind::LargeFireball { .. }
    )
}
/// `Bucketable.getBucketItemStack` and `getPickupSound` of a mob that a water
/// bucket picks up.
fn bucketable(species: Species) -> Option<(&'static str, &'static str)> {
    let fish = "minecraft:item.bucket.fill_fish";
    Some(match species {
        Species::Cod => ("minecraft:cod_bucket", fish),
        Species::Salmon => ("minecraft:salmon_bucket", fish),
        Species::Pufferfish => ("minecraft:pufferfish_bucket", fish),
        Species::TropicalFish => ("minecraft:tropical_fish_bucket", fish),
        Species::Tadpole => (
            "minecraft:tadpole_bucket",
            "minecraft:item.bucket.fill_tadpole",
        ),
        Species::Axolotl => (
            "minecraft:axolotl_bucket",
            "minecraft:item.bucket.fill_axolotl",
        ),
        _ => return None,
    })
}
/// The damage type id of a mob's damage source (`DamageSources.explosion`: a
/// blast with a causing entity is `player_explosion` whoever it is, an
/// exploding creeper too).
pub(super) fn source_damage_type(d: Damage) -> &'static str {
    damage_type(d.kind, d.attacker.is_some())
}
/// Whether the causing entity deals `kind` itself, so it is the source's
/// direct entity too: a melee hit (`DamageSources.mobAttack`, `playerAttack`,
/// a mace's or a spear's), a sting, a sonic boom, thorns.
pub(super) fn melee(kind: horizon_client::mobs::DamageKind) -> bool {
    use horizon_client::mobs::DamageKind as K;
    matches!(
        kind,
        K::PlayerAttack
            | K::MaceSmash
            | K::Spear
            | K::MobAttack
            | K::Sting
            | K::SonicBoom
            | K::Thorns
    )
}
/// The damage type id of a Horizon damage kind. `caused`: the source has a
/// causing entity (only a blast asks).
pub(super) fn damage_type(kind: horizon_client::mobs::DamageKind, caused: bool) -> &'static str {
    use horizon_client::mobs::DamageKind as K;
    match kind {
        K::PlayerAttack => "minecraft:player_attack",
        K::MaceSmash => "minecraft:mace_smash",
        K::Spear => "minecraft:spear",
        K::MobAttack => "minecraft:mob_attack",
        K::Arrow => "minecraft:arrow",
        K::Thrown => "minecraft:thrown",
        K::Fireball => "minecraft:fireball",
        K::Explosion if caused => "minecraft:player_explosion",
        K::Explosion => "minecraft:explosion",
        K::Fall => "minecraft:fall",
        K::FallingBlock => "minecraft:falling_block",
        K::FallingStalactite => "minecraft:falling_stalactite",
        K::OnFire => "minecraft:on_fire",
        K::InFire => "minecraft:in_fire",
        K::Campfire => "minecraft:campfire",
        K::Lava => "minecraft:lava",
        K::Drown => "minecraft:drown",
        K::DryOut => "minecraft:dry_out",
        K::Magic => "minecraft:magic",
        K::IndirectMagic => "minecraft:indirect_magic",
        K::Sting => "minecraft:sting",
        K::Cactus => "minecraft:cactus",
        K::SweetBerryBush => "minecraft:sweet_berry_bush",
        K::Generic => "minecraft:generic",
        K::InWall => "minecraft:in_wall",
        K::Thorns => "minecraft:thorns",
        K::SonicBoom => "minecraft:sonic_boom",
        K::Freeze => "minecraft:freeze",
        K::OutOfWorld => "minecraft:out_of_world",
        K::Cramming => "minecraft:cramming",
        K::FallingAnvil => "minecraft:falling_anvil",
        K::Fireworks => "minecraft:fireworks",
        K::GenericKill => "minecraft:generic_kill",
        K::LightningBolt => "minecraft:lightning_bolt",
        K::OutsideBorder => "minecraft:outside_border",
        K::Spit => "minecraft:spit",
        K::Stalagmite => "minecraft:stalagmite",
        K::Starve => "minecraft:starve",
        K::HotFloor => "minecraft:hot_floor",
        K::SulfurCubeHot => "minecraft:sulfur_cube_hot",
        K::WindCharge => "minecraft:wind_charge",
        K::Wither => "minecraft:wither",
        K::WitherSkull => "minecraft:wither_skull",
    }
}
/// A Horizon item stack as a loot stack (id, count, damage, enchantments).
fn to_loot(stack: &ItemStack) -> horizon_world::loot::LootStack {
    horizon_world::loot::LootStack {
        item: stack.item.to_string(),
        count: stack.count as i32,
        damage: stack.damage as i32,
        enchantments: stack
            .enchantments()
            .iter()
            .map(|(e, l)| (e.to_string(), *l as i32))
            .collect(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components: Vec::new(),
    }
}
/// A generated loot stack as a Horizon item stack.
fn from_loot(stack: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(stack.item.as_str()),
        count: stack.count.max(1) as u32,
        damage: stack.damage.max(0) as u32,
        extra: None,
    };
    for (id, level) in &stack.enchantments {
        item.enchant(id, (*level).max(0) as u32);
    }
    if let Some(potion) = &stack.potion {
        item = item.with_potion(potion);
    }
    if let Some(amplifier) = stack.ominous_amplifier.filter(|a| *a > 0) {
        item.data_mut().ominous_amplifier = Some(amplifier as u8);
    }
    for (id, text) in &stack.names {
        item.data_mut().set_loot_name(id, text);
    }
    for (id, value) in &stack.components {
        item.data_mut().set_loot_component(id, value);
    }
    item
}
impl Game<'_> {
    /// The rest of `NetherPortalBlock.randomTick` after a won roll: with a
    /// player close enough for spawning, a zombified piglin spawns on the block
    /// under the portal column when it is a valid spawn.
    pub(super) fn portal_spawn(&mut self, pos: [i32; 3]) {
        // ChunkMap.anyPlayerCloseEnoughForSpawning: a non-spectator within 128
        // blocks of the chunk centre.
        let centre = [
            f64::from((pos[0] >> 4) * 16 + 8),
            f64::from((pos[2] >> 4) * 16 + 8),
        ];
        let close = self.level_players().iter().any(|p| {
            let (dx, dz) = (p.position.x - centre[0], p.position.z - centre[1]);
            !p.spectator && dx * dx + dz * dz < 16384.0
        });
        if !close {
            return;
        }
        let mut y = pos[1];
        while &*self
            .res
            .facts()
            .get(self.world.block(pos[0], y, pos[2]))
            .name
            == "minecraft:nether_portal"
        {
            y -= 1;
        }
        // isValidSpawn: a sturdy top face (portal frames emit no light).
        let below = self.res.facts().get(self.world.block(pos[0], y, pos[2]));
        if below.sturdy_full & (1 << 1) == 0 {
            return;
        }
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(y + 1),
            f64::from(pos[2]) + 0.5,
        ];
        // setPortalCooldown: the new piglin does not travel at once.
        if let Some(id) = self.spawn_mob("minecraft:zombified_piglin", at, false) {
            self.set_portal_cooldown(id);
        }
    }
}
/// `LivingEntity.knockback` on `p`: [`Game::knock_back_player`].
fn knock_back(
    p: &mut horizon_client::motion::PlayerMotion,
    random: &mut horizon_core::random::LegacyRandom,
    power: f64,
    (xd, zd): (f64, f64),
    source: HurtSource<'_>,
) {
    let resistance = p.attribute("minecraft:knockback_resistance").unwrap_or(0.0);
    // The native hook runs first, on the raw strength (`LivingKnockBackEvent`), then the
    // resistance scales what it left.
    let mut knock = Knock {
        strength: power,
        x: xd,
        z: zd,
    };
    if knockback_canceled(Hurting::Player(p.entity_id, source), &mut knock) {
        return;
    }
    let Knock {
        strength: power,
        x: mut xd,
        z: mut zd,
    } = knock;
    let power = power * (1.0 - resistance);
    if power <= 0.0 {
        return;
    }
    while xd * xd + zd * zd < f64::from(1.0e-5_f32) {
        xd = (random.next_double() - random.next_double()) * 0.01;
        zd = (random.next_double() - random.next_double()) * 0.01;
    }
    let l = (xd * xd + zd * zd).sqrt();
    let v = p.velocity;
    p.velocity = Vec3::new(
        v.x / 2.0 - xd / l * power,
        if p.ground.on_ground {
            (v.y / 2.0 + power).min(0.4)
        } else {
            v.y
        },
        v.z / 2.0 - zd / l * power,
    );
}

#[cfg(test)]
mod tests {
    use super::super::perspective::CameraType;
    use super::thrown_item_axes;
    use super::thrown_item_quad;
    use super::{ProjectileKind, is_item_billboard};

    /// The component-wise lowest and highest corner of the quad.
    fn bounds(corners: &[glam::Vec3; 4]) -> (glam::Vec3, glam::Vec3) {
        (
            corners.iter().copied().reduce(glam::Vec3::min).unwrap(),
            corners.iter().copied().reduce(glam::Vec3::max).unwrap(),
        )
    }

    #[test]
    fn thrown_item_billboard_lifts_by_the_generated_ground_transform() {
        // The kinds vanilla draws with `ThrownItemRenderer`: the only lifted ones.
        assert!(is_item_billboard(&ProjectileKind::Snowball));
        assert!(is_item_billboard(&ProjectileKind::Egg));
        assert!(is_item_billboard(&ProjectileKind::EnderPearl));
        assert!(is_item_billboard(&ProjectileKind::SmallFireball));
        let large = ProjectileKind::LargeFireball { power: 1 };
        assert!(is_item_billboard(&large));
        // A projectile at y = 100 with right = +X, up = +Y: the sprite sits
        // 0.125 blocks above the entity (2 of `item/generated`'s 16 units) and
        // stays 0.5 blocks wide, so the quad's y runs 99.875 .. 100.375.
        let at = glam::Vec3::Y * 100.0;
        let (right, up) = (glam::Vec3::X, glam::Vec3::Y);
        let lifted = is_item_billboard(&ProjectileKind::Snowball);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.y, hi.y), (99.875, 100.375));
        assert_eq!((lo.x, hi.x), (-0.25, 0.25));
        // A small fireball's 0.75 scale: the same quad scaled down, lifted by
        // 0.09375, half extent 0.1875.
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 0.75, lifted));
        assert_eq!((lo.y, hi.y), (99.90625, 100.28125));
        // A large fireball's 3.0: lift 0.375, half extent 0.75.
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 3.0, lifted));
        assert_eq!((lo.y, hi.y), (99.625, 101.125));
        // Llama spit shares the quad here but is a model in vanilla
        // (`LlamaSpitRenderer`): no lift at all.
        let lifted = is_item_billboard(&ProjectileKind::LlamaSpit);
        assert!(!lifted);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.y, hi.y), (99.75, 100.25));
        // The rise follows the billboard's own up axis, not world up.
        let (right, up) = (glam::Vec3::Z, glam::Vec3::X);
        let lifted = is_item_billboard(&ProjectileKind::Egg);
        let (lo, hi) = bounds(&thrown_item_quad(at, right, up, 1.0, lifted));
        assert_eq!((lo.x, hi.x), (-0.125, 0.375));
        assert_eq!((lo.y, hi.y), (100.0, 100.0));
    }

    #[test]
    fn thrown_item_billboard_does_not_flip_at_straight_up_or_down() {
        for yaw in [0.0f32, 37.5, 90.0, 180.0, -135.0] {
            for (edge, near) in [(-90.0f32, -89.9f32), (90.0, 89.9)] {
                let (right_edge, up_edge) = thrown_item_axes(CameraType::FirstPerson, yaw, edge);
                let (right_near, up_near) = thrown_item_axes(CameraType::FirstPerson, yaw, near);
                assert!(
                    right_edge.dot(right_near) > 0.99 && up_edge.dot(up_near) > 0.99,
                    "yaw {yaw}, pitch {edge}: {right_edge}/{up_edge} vs {right_near}/{up_near} at {near}"
                );
            }
        }
    }

    #[test]
    fn thrown_item_billboard_faces_the_front_camera() {
        // Camera.setup turns the front camera round (yaw + 180, -pitch): its
        // right is the player's left.
        let (right, up) = thrown_item_axes(CameraType::FirstPerson, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
        assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
        let (right, up) = thrown_item_axes(CameraType::ThirdPersonFront, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::X, 1e-5), "{right}");
        assert!(up.abs_diff_eq(glam::Vec3::Y, 1e-5), "{up}");
        let (right, _) = thrown_item_axes(CameraType::ThirdPersonBack, 0.0, 0.0);
        assert!(right.abs_diff_eq(glam::Vec3::NEG_X, 1e-5), "{right}");
    }
}

/// Whether the killing blow's causing entity (`DamageSource.getEntity`) was a
/// mob, such as a tame wolf whose owner gets the kill credit.
fn killed_by_mob(m: &Mob) -> bool {
    m.last_damage
        .is_some_and(|(d, _)| matches!(d.attacker, Some(Who::Mob(_))))
}

/// `Animal.finalizeSpawnChildFromBreeding`'s player: the one who fed the
/// parent, or else the one who fed the partner (`getLoveCause`). `Some(None)`
/// is the host, `Some(Some(conn))` a guest. Both animals are spent.
fn breeding_credit(
    love_cause: &mut std::collections::HashMap<u32, Option<u32>>,
    parent: u32,
    partner: u32,
) -> Option<Option<u32>> {
    let from_parent = love_cause.remove(&parent);
    let from_partner = love_cause.remove(&partner);
    from_parent.or(from_partner)
}

#[cfg(test)]
mod breeding_tests {
    use super::*;

    #[test]
    fn the_player_who_fed_either_animal_is_credited() {
        let mut love = std::collections::HashMap::from([(1, Some(7)), (2, Some(9))]);
        // The parent's feeder first, and both animals are spent.
        assert_eq!(breeding_credit(&mut love, 1, 2), Some(Some(7)));
        assert!(love.is_empty());
        // Only the partner was fed: still credited.
        let mut love = std::collections::HashMap::from([(2, None)]);
        assert_eq!(breeding_credit(&mut love, 1, 2), Some(None));
        // Neither was fed by a player (a bred pair loaded from a save).
        assert_eq!(breeding_credit(&mut love, 1, 2), None);
    }
}

#[cfg(test)]
mod damage_type_tests {
    use super::*;

    /// `DamageSources.explosion`: a blast with a causing entity is
    /// `player_explosion` whoever caused it, so an exploding creeper's too.
    #[test]
    fn a_creeper_blast_is_a_player_explosion() {
        let blast = |attacker| Damage {
            kind: DamageKind::Explosion,
            attacker,
            from: Some([0.0; 3]),
        };
        let creeper = blast(Some(Who::Mob(7)));
        assert_eq!(source_damage_type(creeper), "minecraft:player_explosion");
        // No causing entity.
        assert_eq!(source_damage_type(blast(None)), "minecraft:explosion");
        // The other kinds do not ask.
        let bite = Damage {
            kind: DamageKind::MobAttack,
            ..creeper
        };
        assert_eq!(source_damage_type(bite), "minecraft:mob_attack");
    }
}

#[cfg(test)]
mod pearl_tests {
    use super::*;

    /// A pearl's water test is `Entity.isInWater`: `wasTouchingWater` from the
    /// last `updateFluidInteraction`, water whose surface reaches its 0.25
    /// box (#3072), not the block at its feet.
    #[test]
    fn a_pearl_feels_the_water_its_box_touches() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("entity-types.json").exists()
            || !root.join("movement-properties.nbt").exists()
        {
            return;
        }
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let render = assets.reload(&root).unwrap();
        let registry = assets.registry;
        let facts = horizon_client::blocks::BlockFacts::load(&root, registry).unwrap();
        let motion = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
        let data = horizon_client::mobs::MobData::load(&root, &facts);
        let biomes = horizon_client::biome::Biomes::load(&root).unwrap();
        let items = horizon_client::items::Items::load(
            &root,
            registry,
            &facts,
            &horizon_world::loot::PackOverlay::NONE,
        )
        .unwrap();
        let collision = horizon_client::collisions::load(&root, registry).unwrap();
        let mut world = horizon_client::world::World::test_scene(&assets, &render, 3).unwrap();
        // A pool of still water one block deep at y = 4, from x = 20 on (clear of the
        // scene's pyramid): its
        // surface is at 4 + 8/9.
        let water = registry.default_state("minecraft:water").unwrap();
        for x in 20..=40 {
            for z in 0..=2 {
                world.set(x, 4, z, water).unwrap();
            }
        }
        let lid_openers = std::collections::HashMap::new();
        let w = MobWorld {
            teams: Default::default(),
            world: &world,
            registry,
            facts: &facts,
            fluid_tables: &motion.fluids,
            movement: &motion.blocks,
            collision: &collision,
            data: &data,
            player: None,
            nearest_player: None,
            others: &[],
            leash_boats: &[],
            others_grid: None,
            others_index: None,
            daylight: true,
            monsters_burn: false,
            game_time: 1000,
            day_time: 1000,
            raining: false,
            thundering: false,
            nether: false,
            block_tags: &items.block_tags,
            landing: &motion.landing,
            biomes: &biomes,
            mob_griefing: true,
            universal_anger: false,
            mob_drops: true,
            max_entity_cramming: 24,
            difficulty: spawning::LocalDifficulty::new(2, 1000, 0, 0.0),
            end: false,
            world_seed: 0,
            sea_level: 63,
            items: Some(&items),
            lid_openers: &lid_openers,
            mod_effects: None,
        };
        let pw = ProjWorld {
            w: &w,
            targets: Vec::new(),
        };
        let pearl = |x: f64, y: f64, velocity: Vec3| {
            let mut p = Projectile::new(ProjectileKind::EnderPearl, Vec3::new(x, y, 1.5), None);
            p.velocity = velocity;
            p
        };
        // Skimming the pool: its feet are in the water block, its box above
        // the surface. ThrowableProjectile.applyInertia keeps the air drag.
        let mut p = pearl(25.5, 4.95, Vec3::new(0.3, 0.0, 0.0));
        p.tick(&pw);
        assert!(p.position.y > 4.9, "the pearl dipped: {p:?}");
        p.tick(&pw);
        assert!(
            (p.velocity.x - 0.3 * 0.99 * 0.99).abs() < 1e-9,
            "a pearl above the surface slowed as in water: {p:?}"
        );
        // At the pool's edge: its feet over the dry block at x = 19, a side
        // of its box in the water at x = 20. The water drag after the first
        // tick, which still reads `wasTouchingWater` as false.
        let mut p = pearl(19.95, 4.3, Vec3::ZERO);
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * 0.99)).abs() < 1e-9,
            "the first tick has the air drag: {p:?}"
        );
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * 0.99 - 0.03) * 0.8).abs() < 1e-9,
            "a pearl whose box is in the water kept the air drag: {p:?}"
        );
        // Thrown under water, a pearl moves with the air drag on its first
        // tick and the water drag after it.
        let mut p = pearl(30.5, 4.2, Vec3::ZERO);
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * 0.99)).abs() < 1e-9,
            "the first tick under water has the air drag: {p:?}"
        );
        p.tick(&pw);
        assert!(
            (p.velocity.y - (-0.03 * 0.99 - 0.03) * 0.8).abs() < 1e-9,
            "the second tick under water has the water drag: {p:?}"
        );
    }
}

#[cfg(test)]
mod knockback_tests {
    use super::*;
    use horizon_client::health::KNOCKBACK;

    /// What the hook of these tests saw: the player, its source (as damage
    /// type, causing, direct) and the push.
    type Pushes = std::sync::Mutex<Vec<(i32, (String, i32, i32), Knock)>>;
    static PUSHES: Pushes = std::sync::Mutex::new(Vec::new());

    /// Sets the native knockback hook of these tests (global: it stays set for
    /// the process) for players 9201 to 9203: it records the push, cancels
    /// 9201's and doubles 9202's strength and turns its direction around.
    fn hook(pushed: Hurting<'_>, knock: &mut Knock) -> bool {
        let Hurting::Player(entity @ 9201..=9203, s) = pushed else {
            return false;
        };
        let source = (s.damage_type.to_owned(), s.causing, s.direct);
        PUSHES.lock().unwrap().push((entity, source, *knock));
        match entity {
            9201 => return true,
            9202 => {
                knock.strength *= 2.0;
                knock.z = -knock.z;
            }
            _ => {}
        }
        false
    }

    /// `LivingEntity.knockback` on a player: the hook runs after the
    /// knockback resistance and before the motion changes; a mod's strength or
    /// direction pushes, a cancel pushes nothing, and without a mod in the way
    /// the push is vanilla's. Needs the prepared data in
    /// `.local/horizon-client/26.3`; skipped without it.
    #[test]
    fn the_native_knockback_hook_changes_or_cancels_a_players_push() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("movement-properties.nbt").exists() {
            return;
        }
        let installed = *KNOCKBACK.get_or_init(|| hook);
        assert!(std::ptr::fn_addr_eq(
            installed,
            hook as fn(Hurting<'_>, &mut Knock) -> bool
        ));
        let assets = horizon_client::assets::Assets::load(&root).unwrap();
        let resources = horizon_client::motion::MotionResources::load(&root, &assets).unwrap();
        let mut random = horizon_core::random::LegacyRandom::new(1);
        let hit = HurtSource {
            damage_type: "minecraft:player_attack",
            causing: 5,
            direct: 5,
        };
        let mut push = |entity, resistance: f64| {
            let mut p = horizon_client::motion::PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
            p.entity_id = entity;
            p.ground.on_ground = true;
            if resistance != 0.0 {
                let attribute = "minecraft:knockback_resistance";
                let set = p.set_modifier(attribute, "horizon:test", resistance, "add_value", true);
                set.unwrap();
            }
            knock_back(&mut p, &mut random, 0.4, (0.0, 3.0), hit);
            (p.velocity.x, p.velocity.y, p.velocity.z)
        };
        // No mod in the way: vanilla's push, away from z = 3.
        assert_eq!(push(9200, 0.0), (0.0, 0.4, -0.4));
        // Canceled: no motion. The hook saw the push and the hit.
        assert_eq!(push(9201, 0.0), (0.0, 0.0, 0.0));
        let made = Knock {
            strength: 0.4,
            x: 0.0,
            z: 3.0,
        };
        let named = ("minecraft:player_attack".to_owned(), 5, 5);
        let seen = |entity| -> Vec<_> {
            let seen = PUSHES.lock().unwrap();
            seen.iter().filter(|p| p.0 == entity).cloned().collect()
        };
        assert_eq!(seen(9201), [(9201, named, made)]);
        // Twice the strength, direction turned around.
        assert_eq!(push(9202, 0.0), (0.0, 0.4, 0.8));
        // Before the resistance (`LivingKnockBackEvent`): a 0.5 resistance still hands the hook
        // the raw strength, and the push that follows is halved.
        assert_eq!(push(9203, 0.5), (0.0, 0.2, -0.2));
        assert_eq!(seen(9203)[0].2.strength, 0.4);
    }
}
