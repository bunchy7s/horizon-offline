//! Water mobs: squid and glow squid (tentacle propulsion), cod, salmon,
//! tropical fish and pufferfish (`AbstractFish`, puffing), and dolphins
//! (breathing, smooth swimming).
use super::{
    Mob, MobEvent, MobWorld, Species,
    anim::{Poses, State},
    control::MoveKind,
    goals::{
        Avoid, AvoidEntity, BreathAir, Cx, Goal, HurtByTarget, LookAtPlayer, MeleeAttack, Panic,
        RandomLookAround, Stroll, TryFindWater, goal_clone, reduced,
    },
    nav::NavKind,
};
use crate::blocks::Fluid;
use crate::particles::ParticleType;
use horizon_core::geometry::trig::Mth;
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::f32::consts::PI;

// Squid animation slots in AnimState.v.
const TENTACLE_MOVEMENT: usize = 0;
pub const TENTACLE_ANGLE: usize = 1;
const X_BODY_ROT: usize = 2;
const Z_BODY_ROT: usize = 3;
const TENTACLE_SPEED: usize = 4;
const ROTATE_SPEED: usize = 5;

/// `Squid.spawnInk`: the squirt sound and 30 ink particles shot from under
/// the squid, turned by its body rotation.
pub fn spawn_ink(m: &mut Mob) {
    let sound = if m.species() == Species::GlowSquid {
        "entity.glow_squid.squirt"
    } else {
        "entity.squid.squirt"
    };
    let kind = if m.species() == Species::GlowSquid {
        "glow_squid_ink"
    } else {
        "squid_ink"
    };
    // makeSound draws the voice pitch even for a silent squid.
    let pitch = m.voice_pitch();
    if !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        m.outbox.push(MobEvent::Sound(
            sound,
            at,
            super::ai::sound_volume(m),
            pitch,
        ));
    }
    // rotateVector: xRot(xBodyRotO), then yRot(-yBodyRotO).
    let to_radians = (std::f64::consts::PI / 180.0) as f32;
    let (x_rot, y_rot) = (
        m.anim.vo[X_BODY_ROT] * to_radians,
        -m.previous_body_yaw * to_radians,
    );
    let rotate = |v: Vec3| v.rotate_x(x_rot).rotate_y(y_rot);
    let pos = rotate(Vec3::new(0.0, -1.0, 0.0)).add_vector(m.position);
    let offset_scale: f32 = if m.is_baby() { 0.1 } else { 0.3 };
    for _ in 0..30 {
        let x = f64::from(m.random.next_float()) * 0.6 - 0.3;
        let z = f64::from(m.random.next_float()) * 0.6 - 0.3;
        let dir = rotate(Vec3::new(x, -1.0, z));
        let d = dir.scale(f64::from(offset_scale + m.random.next_float() * 2.0));
        // ServerLevel.sendParticles with a count of 0 and speed 0.1: one
        // particle moving at the float offset times the float speed.
        let speed = f64::from(0.1f32);
        m.outbox.push(MobEvent::ParticleAt {
            kind,
            at: [pos.x, pos.y + 0.5, pos.z],
            velocity: [
                speed * f64::from(d.x as f32),
                speed * f64::from(d.y as f32),
                speed * f64::from(d.z as f32),
            ],
        });
    }
}
pub fn register(m: &mut Mob) {
    use Species as S;
    // WaterAnimal and AgeableWaterCreature: water costs nothing, so swim
    // targets in water pass DefaultRandomPos's malus check.
    if matches!(
        m.species(),
        S::Cod
            | S::Salmon
            | S::TropicalFish
            | S::Pufferfish
            | S::Tadpole
            | S::Squid
            | S::GlowSquid
            | S::Dolphin
    ) {
        m.set_malus(super::path::PathType::Water, 0.0);
    }
    match m.species() {
        S::Squid | S::GlowSquid => {
            m.ai.goals.add(0, SquidRandomMovement);
            m.ai.goals.add(1, SquidFlee { ticks: 0 });
            m.ai.mov.kind = MoveKind::Custom;
            m.ai.nav.kind = NavKind::Water { breach: false };
            let speed = 1.0 / (m.random.next_float() + 1.0) * 0.2;
            m.anim.v[TENTACLE_SPEED] = speed;
        }
        S::Axolotl => {
            m.air = super::axolotl::MAX_AIR;
            // Native controller scaling is essential: the base speed attribute is 1.0.
            m.ai.mov.kind = MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.1,
                outside: 0.5,
                gravity: false,
            };
            m.ai.look.kind = super::control::LookKind::SmoothSwimming(20.0);
            m.ai.nav.kind = NavKind::Amphibious {
                prefers_shallow: false,
            };
            // `Axolotl()`: water costs nothing, so swim targets in water pass
            // DefaultRandomPos's malus check.
            m.set_malus(super::path::PathType::Water, 0.0);
            super::animal_brain::init(m, i64::from(m.id));
        }
        S::Tadpole => {
            // `Tadpole()`: `SmoothSwimmingMoveControl(this, 85, 10, 0.02, 0.1, true)`,
            // `SmoothSwimmingLookControl(this, 10)` and a water-bound navigation.
            m.ai.mov.kind = MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.02,
                outside: 0.1,
                gravity: true,
            };
            m.ai.look.kind = super::control::LookKind::SmoothSwimming(10.0);
            m.ai.nav.kind = NavKind::Water { breach: false };
            // A tadpole is an `AbstractFish`: its goals (panic 1.25, avoid the player, the fish
            // swim goal) run beside `TadpoleAi`'s brain (panic 2.0, looking at players, frog
            // food, swimming about).
            let g = &mut m.ai.goals;
            g.add(0, Panic::new(1.25));
            g.add(2, AvoidEntity::new(Avoid::Player, 8.0, 1.6, 1.4));
            g.add(4, FishSwim(Stroll::swimming(1.0, 40)));
            super::animal_brain::init(m, i64::from(m.id));
        }
        S::Cod | S::Salmon | S::TropicalFish | S::Pufferfish => {
            let puffer = m.species() == S::Pufferfish;
            let g = &mut m.ai.goals;
            g.add(0, Panic::new(1.25));
            g.add(2, AvoidEntity::new(Avoid::Player, 8.0, 1.6, 1.4));
            g.add(4, FishSwim(Stroll::swimming(1.0, 40)));
            if puffer {
                g.add(1, PufferfishPuff);
            } else {
                let next_start = FollowFlockLeader::next_start_tick(m);
                m.ai.goals.add(5, FollowFlockLeader::new(next_start));
            }
            m.ai.mov.kind = MoveKind::Custom;
            m.ai.nav.kind = NavKind::Water { breach: false };
        }
        S::Dolphin => {
            let g = &mut m.ai.goals;
            g.add(0, BreathAir);
            g.add(0, TryFindWater::default());
            g.add(1, super::dolphin::SwimToTreasure::default());
            g.add(2, super::dolphin::SwimWithPlayer::new(4.0));
            g.add(4, Stroll::swimming(1.0, 10));
            g.add(4, RandomLookAround::default());
            g.add(5, LookAtPlayer::new(6.0));
            g.add(5, super::dolphin::Jump::new(10));
            g.add(6, MeleeAttack::new(1.2, true));
            g.add(8, super::dolphin::FollowPlayerRidden::new(false));
            g.add(8, super::dolphin::FollowPlayerRidden::new(true));
            // `Dolphin.registerGoals`: AvoidEntityGoal(Guardian.class, 8, 1, 1).
            g.add(
                9,
                AvoidEntity::new(
                    Avoid::Species(vec![S::Guardian, S::ElderGuardian]),
                    8.0,
                    1.0,
                    1.0,
                ),
            );
            // `HurtByTargetGoal(this, Guardian.class)`: guardians' hits are ignored.
            m.ai.targets.add(
                1,
                HurtByTarget::new()
                    .alert()
                    .ignoring(&[S::Guardian, S::ElderGuardian]),
            );
            m.ai.mov.kind = MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.02,
                outside: 0.1,
                gravity: true,
            };
            m.ai.look.kind = super::control::LookKind::SmoothSwimming(10.0);
            m.ai.nav.kind = NavKind::Water { breach: true };
            m.air = 4800;
        }
        _ => {}
    }
}
/// Pre-tick updates (Pufferfish.tick puff state).
pub fn tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.species() != Species::Pufferfish || !m.alive() {
        return;
    }
    let puff = m.extra.size;
    if m.extra.counter > 0 {
        if puff == 0 {
            m.sound("entity.puffer_fish.blow_up", events);
            m.extra.size = 1;
        } else if m.extra.counter > 40 && puff == 1 {
            m.sound("entity.puffer_fish.blow_up", events);
            m.extra.size = 2;
        }
        m.extra.counter += 1;
    } else if puff != 0 {
        if m.extra.counter2 > 60 && puff == 2 {
            m.sound("entity.puffer_fish.blow_out", events);
            m.extra.size = 1;
        } else if m.extra.counter2 > 100 && puff == 1 {
            m.sound("entity.puffer_fish.blow_out", events);
            m.extra.size = 0;
        }
        m.extra.counter2 += 1;
    }
}
/// Pufferfish.getScale by puff state (the dimensions scale with it).
pub fn puff_scale(state: i32) -> f64 {
    match state {
        0 => 0.5,
        1 => 0.7,
        _ => 1.0,
    }
}
/// `Tadpole.setAgeLockedData`: a golden dandelion flips the lock, puts the
/// age back to 0 and starts the particle timer (40 ticks).
pub fn set_age_locked_data(m: &mut Mob) {
    m.extra.age_locked = !m.extra.age_locked;
    m.extra.age = 0;
    m.extra.age_lock_particle_timer = 40;
}
/// Species aiStep additions.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    use Species as S;
    match m.species() {
        S::Squid => squid_ai_step(m),
        S::GlowSquid => {
            squid_ai_step(m);
            glow_squid_ai_step(m, events);
        }
        S::Tadpole => {
            // Tadpole.aiStep: growing up into a frog after 24000 ticks.
            if m.extra.age >= 0 && !m.extra.age_locked {
                m.extra.age += 1;
                if m.extra.age >= 24_000 && !m.removed {
                    m.removed = true;
                    // Tadpole.ageUp: playSound at its feet, volume 0.15 (not
                    // when silent: Entity.playSound).
                    if !m.silent {
                        let at = [m.position.x, m.position.y, m.position.z];
                        events.push(MobEvent::Sound("entity.tadpole.grow_up", at, 0.15, 1.0));
                    }
                    // `ConversionParams.single(this, false, false)`; the game
                    // finalizes the frog and keeps it persistent.
                    events.push(MobEvent::Convert {
                        from: Box::new(m.clone()),
                        kind: "minecraft:frog",
                        params: super::conversion::Params {
                            keep_equipment: false,
                            preserve_can_pick_up_loot: false,
                        },
                    });
                }
            }
            if !m.fluids.in_water && m.on_ground && m.vertical_collision_below {
                let (a, b) = (m.random.next_float(), m.random.next_float());
                m.velocity = Vec3::new(
                    m.velocity.x + f64::from((a * 2.0 - 1.0) * 0.05),
                    m.velocity.y + 0.4,
                    m.velocity.z + f64::from((b * 2.0 - 1.0) * 0.05),
                );
                m.on_ground = false;
                m.sound("entity.tadpole.flop", events);
            }
            // Tadpole.aiStep: the age lock's particles, after the age.
            super::ai::make_age_locked_particle(m, events);
        }
        S::Cod | S::Salmon | S::TropicalFish | S::Pufferfish => {
            // AbstractFish.aiStep flops on land (applied before travel next tick).
            if !m.fluids.in_water && m.on_ground && m.vertical_collision_below {
                let (a, b) = (m.random.next_float(), m.random.next_float());
                m.velocity = Vec3::new(
                    m.velocity.x + f64::from((a * 2.0 - 1.0) * 0.05),
                    m.velocity.y + 0.4,
                    m.velocity.z + f64::from((b * 2.0 - 1.0) * 0.05),
                );
                m.on_ground = false;
                let flop = match m.species() {
                    S::Cod => "entity.cod.flop",
                    S::Salmon => "entity.salmon.flop",
                    S::TropicalFish => "entity.tropical_fish.flop",
                    _ => "entity.puffer_fish.flop",
                };
                m.sound(flop, events);
            }
            if m.species() == S::Pufferfish && m.alive() && m.extra.size > 0 {
                // Pufferfish.playerTouch / touch: sting with poison.
                if let Some(p) = &w.player
                    && p.alive
                    && !p.creative
                    && m.bounds()
                        .inflate(Vec3::new(0.3, 0.3, 0.3))
                        .intersects(p.bounds)
                    && m.ai.no_action_time % 10 == 0
                {
                    let amount = 1.0 + m.extra.size as f32;
                    // Only when the sting hurt the player.
                    events.push(MobEvent::AttackPlayerWithEffect {
                        amount,
                        from: [m.position.x, m.position.y, m.position.z],
                        effect: "minecraft:poison",
                        ticks: 60 * m.extra.size,
                        amplifier: 0,
                    });
                    m.sound("entity.puffer_fish.sting", events);
                }
            }
        }
        S::Dolphin => {
            // Dolphin.tick: moisture out of water, hop on land.
            if m.fluids.in_water || super::ai::in_rain(m, w) {
                m.extra.counter = 2400;
            } else {
                m.extra.counter -= 1;
                if m.extra.counter <= 0 {
                    m.damage(1.0, super::Damage::of(super::DamageKind::Drown));
                }
                if m.on_ground {
                    let (a, b) = (m.random.next_float(), m.random.next_float());
                    m.velocity = Vec3::new(
                        m.velocity.x + f64::from((a * 2.0 - 1.0) * 0.2),
                        m.velocity.y + 0.5,
                        m.velocity.z + f64::from((b * 2.0 - 1.0) * 0.2),
                    );
                    m.yaw = m.random.next_float() * 360.0;
                    m.on_ground = false;
                }
            }
            // LivingEntity air: dolphins drown underwater and refill instantly in air.
            if !m.fluids.eye_in_water {
                m.air = 4800;
            }
        }
        _ => {}
    }
}
/// Squid.aiStep: tentacle cycle, propulsion and body rotation.
fn squid_ai_step(m: &mut Mob) {
    squid_step(m, false);
}

/// Squid.aiStep also runs on remote entities: animate from packet velocity,
/// without running goals or replacing that velocity with the server's movement vector.
/// A glow squid's own `aiStep` follows it (the dark timer and the glow).
pub fn squid_client_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if matches!(m.species(), Species::Squid | Species::GlowSquid) {
        m.anim.vo = m.anim.v;
        squid_step(m, true);
    }
    if m.species() == Species::GlowSquid {
        glow_squid_ai_step(m, events);
    }
}

/// `GlowSquid.getDarkTicksRemaining`: `DATA_DARK_TICKS_REMAINING`, the ticks
/// left of the dimming a hit gave (kept in `extra.counter`).
pub fn dark_ticks(m: &Mob) -> i32 {
    m.extra.counter
}

/// `GlowSquid.setDarkTicks`: a server sends it to its clients as synched data.
pub fn set_dark_ticks(m: &mut Mob, ticks: i32) {
    m.extra.counter = ticks;
}

/// `GlowSquid.hurtServer`: a hit that went through dims the squid for 100
/// ticks (called after `Squid.hurtServer`'s ink).
pub fn hurt_server(m: &mut Mob) {
    set_dark_ticks(m, 100);
}

/// `GlowSquid.aiStep` after `super.aiStep()`: the dark timer counts down, and
/// a `glow` particle appears at `getRandomX(0.6)`, `getRandomY()`,
/// `getRandomZ(0.6)` (drawn in that order). A window that simulates the world
/// and a client of a server both draw it, so the server's own `ai_step` pushes
/// the event as well (as the nautilus bubbles do).
fn glow_squid_ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let ticks = dark_ticks(m);
    if ticks > 0 {
        set_dark_ticks(m, ticks - 1);
    }
    let (w, h) = (f64::from(m.width()), f64::from(m.height()));
    let r = &mut m.random;
    let x = m.position.x + w * ((2.0 * r.next_double() - 1.0) * 0.6);
    let y = m.position.y + h * r.next_double();
    let z = m.position.z + w * ((2.0 * r.next_double() - 1.0) * 0.6);
    events.push(MobEvent::ParticleAt {
        kind: ParticleType::Glow.name(),
        at: [x, y, z],
        velocity: [0.0; 3],
    });
}

/// `GlowSquidRenderer.getBlockLightLevel`: the block light a glow squid is
/// drawn with, `lit` being what `EntityRenderer.getBlockLightLevel` gives (15
/// when on fire). It glows brighter the fewer dark ticks are left.
pub fn glow_squid_block_light(m: &Mob, lit: u32) -> u32 {
    // Mth.clampedLerp(1.0F - darkTicks / 10.0F, 0.0F, 15.0F).
    let factor = 1.0f32 - dark_ticks(m) as f32 / 10.0;
    let glow = if factor < 0.0 {
        0
    } else if factor > 1.0 {
        15
    } else {
        (factor * 15.0) as u32
    };
    if glow == 15 { 15 } else { glow.max(lit) }
}

/// Squid.handleEntityEvent(19): begin the next server-synchronised tentacle cycle.
pub fn squid_cycle_event(m: &mut Mob) {
    if matches!(m.species(), Species::Squid | Species::GlowSquid) {
        m.anim.v[TENTACLE_MOVEMENT] = 0.0;
    }
}

fn squid_step(m: &mut Mob, client: bool) {
    let v = &mut m.anim.v;
    v[TENTACLE_MOVEMENT] += v[TENTACLE_SPEED];
    if f64::from(v[TENTACLE_MOVEMENT]) > std::f64::consts::TAU {
        if client {
            v[TENTACLE_MOVEMENT] = PI * 2.0;
        } else {
            v[TENTACLE_MOVEMENT] -= PI * 2.0;
            if m.random.next_int_bound(10).unwrap_or(1) == 0 {
                m.anim.v[TENTACLE_SPEED] = 1.0 / (m.random.next_float() + 1.0) * 0.2;
            }
            m.outbox.push(MobEvent::SquidCycle {
                mob: m.id,
                at: [m.position.x, m.position.y, m.position.z],
            });
        }
    }
    let v = &mut m.anim.v;
    if m.fluids.in_water {
        if v[TENTACLE_MOVEMENT] < PI {
            let scale = v[TENTACLE_MOVEMENT] / PI;
            v[TENTACLE_ANGLE] = (scale * scale * PI).msin() * PI * 0.25;
            if scale > 0.75 {
                if !client {
                    let mv = m.extra.vector;
                    m.velocity = Vec3::new(mv[0], mv[1], mv[2]);
                }
                m.anim.v[ROTATE_SPEED] = 1.0;
            } else {
                m.anim.v[ROTATE_SPEED] *= 0.8;
            }
        } else {
            v[TENTACLE_ANGLE] = 0.0;
            if !client {
                m.velocity = Vec3::new(m.velocity.x * 0.9, m.velocity.y * 0.9, m.velocity.z * 0.9);
            }
            m.anim.v[ROTATE_SPEED] *= 0.99;
        }
        let mv = m.velocity;
        let horizontal = (mv.x * mv.x + mv.z * mv.z).sqrt();
        m.body_yaw += (-(horizon_core::geometry::trig::atan2(mv.x, mv.z) as f32) * 57.295776
            - m.body_yaw)
            * 0.1;
        m.yaw = m.body_yaw;
        let v = &mut m.anim.v;
        v[Z_BODY_ROT] += PI * v[ROTATE_SPEED] * 1.5;
        v[X_BODY_ROT] += (-(horizon_core::geometry::trig::atan2(horizontal, mv.y) as f32)
            * 57.295776
            - v[X_BODY_ROT])
            * 0.1;
    } else {
        v[TENTACLE_ANGLE] = v[TENTACLE_MOVEMENT].msin().abs() * PI * 0.25;
        if !client {
            let yd = m.velocity.y - m.kind.gravity;
            m.velocity = Vec3::new(0.0, yd * 0.98, 0.0);
        }
        let v = &mut m.anim.v;
        v[X_BODY_ROT] += (-90.0 - v[X_BODY_ROT]) * 0.02;
    }
}
/// Species travel overrides. Returns true when handled.
pub fn travel(m: &mut Mob, w: &MobWorld, input: Vec3, events: &mut Vec<MobEvent>) -> bool {
    use Species as S;
    match m.species() {
        // Squid.travel: move by the current velocity only.
        S::Squid | S::GlowSquid => {
            m.do_move(w, events);
            true
        }
        // Axolotl.travelInWater: no fish sinking or generic water gravity.
        S::Axolotl if m.fluids.in_water => {
            m.move_relative(m.ai.speed, input);
            m.do_move(w, events);
            m.velocity = Vec3::new(m.velocity.x * 0.9, m.velocity.y * 0.9, m.velocity.z * 0.9);
            true
        }
        // AbstractFish.travelInWater / Dolphin.travelInWater.
        S::Cod | S::Salmon | S::TropicalFish | S::Pufferfish | S::Tadpole | S::Dolphin
            if m.fluids.in_water =>
        {
            let speed = if m.species() == S::Dolphin {
                m.ai.speed
            } else {
                0.01
            };
            m.move_relative(speed, input);
            m.do_move(w, events);
            m.velocity = Vec3::new(m.velocity.x * 0.9, m.velocity.y * 0.9, m.velocity.z * 0.9);
            if m.ai.target.is_none() {
                // `Vec3.add(0.0, -0.005, 0.0)`: adding 0.0 turns a -0.0 into 0.0.
                m.velocity =
                    Vec3::new(m.velocity.x + 0.0, m.velocity.y - 0.005, m.velocity.z + 0.0);
            }
            true
        }
        _ => false,
    }
}
/// FishMoveControl.tick (fish use it instead of the default move control).
pub fn fish_move_control(m: &mut Mob) {
    if !matches!(
        m.species(),
        Species::Cod | Species::Salmon | Species::TropicalFish | Species::Pufferfish
    ) {
        return;
    }
    if m.fluids.eye_in_water {
        m.velocity.y += 0.005;
    }
    if m.ai.mov.operation == super::control::Operation::MoveTo && !m.ai.nav.is_done() {
        let target = (m.ai.mov.speed_modifier * m.movement_speed()) as f32;
        let speed = m.ai.speed + (target - m.ai.speed) * 0.125;
        m.set_speed(speed);
        let [wx, wy, wz] = m.ai.mov.wanted;
        let (xd, yd, zd) = (wx - m.position.x, wy - m.position.y, wz - m.position.z);
        if yd != 0.0 {
            let dd = (xd * xd + yd * yd + zd * zd).sqrt();
            m.velocity.y += f64::from(m.ai.speed) * (yd / dd) * 0.1;
        }
        if xd != 0.0 || zd != 0.0 {
            let y_rot = horizon_core::geometry::trig::atan2_degrees(zd, xd) - 90.0;
            m.yaw = super::control::rotlerp(m.yaw, y_rot, 90.0);
            m.body_yaw = m.yaw;
        }
    } else {
        m.set_speed(0.0);
    }
}
/// Squid.SquidRandomMovementGoal
#[derive(Clone, Debug)]
pub struct SquidRandomMovement;
impl Goal for SquidRandomMovement {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _: &mut Cx) {
        if m.ai.no_action_time > 100 {
            m.extra.vector = [0.0; 3];
        } else if m
            .random
            .next_int_bound(super::goals::reduced(50))
            .unwrap_or(1)
            == 0
            || !m.fluids.in_water
            || m.extra.vector == [0.0; 3]
        {
            let angle = m.random.next_float() * PI * 2.0;
            let y = -0.1 + m.random.next_float() * 0.2;
            m.extra.vector = [
                f64::from(angle.mcos() * 0.2),
                f64::from(y),
                f64::from(angle.msin() * 0.2),
            ];
        }
    }
}
/// Squid.SquidFleeGoal (keeps vanilla's discarded normalize()).
#[derive(Clone, Debug)]
pub struct SquidFlee {
    ticks: i32,
}
impl Goal for SquidFlee {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(by) = m.ai.last_hurt_by.and_then(|w| cx.w.target_info(w)) else {
            return false;
        };
        m.fluids.in_water && m.distance_sqr(by.position) < 100.0
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.ticks = 0;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.ticks += 1;
        let Some(by) = m.ai.last_hurt_by.and_then(|w| cx.w.target_info(w)) else {
            return;
        };
        let mut f = [
            m.position.x - by.position.x,
            m.position.y - by.position.y,
            m.position.z - by.position.z,
        ];
        let at = [
            (m.position.x + f[0]).floor() as i32,
            (m.position.y + f[1]).floor() as i32,
            (m.position.z + f[2]).floor() as i32,
        ];
        let air = cx.w.facts.is_air(cx.w.world.block(at[0], at[1], at[2]));
        if cx.w.fluid(at[0], at[1], at[2]) == Fluid::Water || air {
            let length = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt();
            if length > 0.0 {
                let mut avoid = 3.0;
                if length > 5.0 {
                    avoid -= (length - 5.0) / 5.0;
                }
                if avoid > 0.0 {
                    f = f.map(|c| c * avoid);
                }
            }
            if air {
                f[1] = 0.0;
            }
            m.extra.vector = [f[0] / 20.0, f[1] / 20.0, f[2] / 20.0];
        }
    }
}
/// `AbstractSchoolingFish`: cod, salmon and tropical fish.
fn schooling(s: Species) -> bool {
    matches!(s, Species::Cod | Species::Salmon | Species::TropicalFish)
}
/// `getMaxSchoolSize`: `AbstractFish.getMaxSpawnClusterSize` (8), a salmon's
/// own 5.
fn max_school_size(s: Species) -> i32 {
    if s == Species::Salmon { 5 } else { 8 }
}
/// `isFollower`: its leader is alive.
fn follows(w: &MobWorld, leader: Option<u32>) -> bool {
    leader.and_then(|l| w.seen(l)).is_some_and(|s| s.alive)
}
/// `canBeFollowed`: it has followers and room for more.
fn can_be_followed(s: Species, followers: i32) -> bool {
    followers > 0 && followers + 1 < max_school_size(s)
}
/// `startFollowing` on a recruit, applied after the recruiter's tick. A fish
/// another leader took this tick (its snapshot was older) stays with it.
pub fn start_following(mobs: &mut [Mob], fish: u32, leader: u32) {
    let alive = |mobs: &[Mob], id: u32| mobs.iter().any(|m| m.id == id && m.alive() && !m.removed);
    let Some(current) = mobs.iter().find(|m| m.id == fish).map(|m| m.extra.leader) else {
        return;
    };
    if current.is_some_and(|l| alive(mobs, l)) {
        return;
    }
    if let Some(m) = mobs.iter_mut().find(|m| m.id == fish) {
        m.extra.leader = Some(leader);
    }
    if let Some(m) = mobs.iter_mut().find(|m| m.id == leader) {
        m.extra.followers += 1;
    }
}
/// `AbstractSchoolingFish.tick`: a leader with no fish of its kind within 8
/// forgets its school, checked once in 200 ticks on the level's random.
pub fn school_tick(m: &mut Mob, w: &MobWorld, level_random: &mut impl RandomSource) {
    if !schooling(m.species()) || m.extra.followers <= 0 {
        return;
    }
    if level_random.next_int_bound(200).unwrap_or(0) == 1 {
        let area = m.bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let species = m.species();
        let others = w
            .others_near(area)
            .filter(|s| {
                s.id != m.id && s.species == species && s.alive && s.bounds.intersects(area)
            })
            .count();
        if others == 0 {
            m.extra.followers = 0;
        }
    }
}
/// `AbstractFish.FishSwimGoal`: random swimming, which a schooling fish
/// that follows a leader leaves (`canRandomSwim`).
#[derive(Clone, Debug)]
pub struct FishSwim(Stroll);
impl Goal for FishSwim {
    goal_clone!();
    fn name(&self) -> &'static str {
        "FishSwimGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if schooling(m.species()) && follows(cx.w, m.extra.leader) {
            return false;
        }
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
/// `FollowFlockLeaderGoal`: a fish with no school, every ten seconds or so,
/// joins a school nearby that has room, or gathers the fish around it that
/// follow no one into a school of its own; a follower swims after its
/// leader while it is within 11 blocks.
#[derive(Clone, Debug)]
pub struct FollowFlockLeader {
    next_start: i32,
    recalc: i32,
}
impl FollowFlockLeader {
    pub fn new(next_start: i32) -> Self {
        Self {
            next_start,
            recalc: 0,
        }
    }
    /// `nextStartTick`, on the fish's random.
    pub fn next_start_tick(m: &mut Mob) -> i32 {
        reduced(200 + m.random.next_int_bound(200).unwrap_or(0) % 20)
    }
}
impl Goal for FollowFlockLeader {
    goal_clone!();
    fn set_start_delay(&mut self, ticks: i32) {
        self.next_start = ticks;
    }
    fn name(&self) -> &'static str {
        "FollowFlockLeaderGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.extra.followers > 0 {
            return false;
        }
        if follows(cx.w, m.extra.leader) {
            return true;
        }
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = Self::next_start_tick(m);
        // The fish of its kind around it that lead a school with room or
        // follow no one, in level order; itself as it is now.
        let species = m.species();
        let area = m.bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let mut near: Vec<&crate::mobs::Seen> =
            cx.w.others_near(area)
                .filter(|s| s.species == species && s.alive && s.bounds.intersects(area))
                .collect();
        // `getEntitiesOfClass`: the sections by x, z, y; in each, as they entered it.
        near.sort_by_key(|s| (s.section.0[0], s.section.0[2], s.section.0[1], s.section.1));
        let candidates: Vec<(u32, Option<u32>, i32)> = near
            .into_iter()
            .map(|s| {
                if s.id == m.id {
                    (s.id, m.extra.leader, m.extra.followers)
                } else {
                    (s.id, s.school.0, s.school.1)
                }
            })
            .filter(|&(_, leader, followers)| {
                can_be_followed(species, followers) || !follows(cx.w, leader)
            })
            .collect();
        let (leader, school_size) = candidates
            .iter()
            .find(|c| can_be_followed(species, c.2))
            .map_or((m.id, m.extra.followers + 1), |c| (c.0, c.2 + 1));
        // addFollowers: the limit counts the leader before it is skipped.
        let room = (max_school_size(species) - school_size).max(0) as usize;
        for &(id, ..) in candidates
            .iter()
            .filter(|c| !follows(cx.w, c.1))
            .take(room)
            .filter(|c| c.0 != leader)
        {
            if id == m.id {
                m.extra.leader = Some(leader);
                cx.events.push(MobEvent::SchoolSize { leader, by: 1 });
            } else {
                cx.events
                    .push(MobEvent::StartFollowing { fish: id, leader });
            }
        }
        follows(cx.w, m.extra.leader)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra
            .leader
            .and_then(|l| cx.w.seen(l))
            .is_some_and(|l| l.alive && m.distance_sqr(l.position) <= 121.0)
    }
    fn start(&mut self, _: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(leader) = m.extra.leader.take() {
            cx.events.push(MobEvent::SchoolSize { leader, by: -1 });
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.recalc -= 1;
        if self.recalc > 0 {
            return;
        }
        self.recalc = reduced(10);
        // pathToLeader
        if let Some(l) = m
            .extra
            .leader
            .and_then(|l| cx.w.seen(l))
            .filter(|l| l.alive)
        {
            let at = [l.position.x, l.position.y, l.position.z];
            m.nav_move_to_entity(cx.w, at, 1, 1.0);
        }
    }
}
/// Pufferfish.PufferfishPuffGoal: inflate when a mob or player is within 2.
#[derive(Clone, Debug)]
pub struct PufferfishPuff;
impl Goal for PufferfishPuff {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let area = m.bounds().inflate(Vec3::new(2.0, 2.0, 2.0));
        let player =
            cx.w.player
                .as_ref()
                .is_some_and(|p| p.alive && !p.creative && area.intersects(p.bounds));
        // TARGETING_CONDITIONS: not #not_scary_for_pufferfish, and alive.
        let mob = cx.w.others_near(area).any(|s| {
            s.id != m.id
                && s.alive
                && !matches!(
                    s.species,
                    Species::Cod
                        | Species::Salmon
                        | Species::TropicalFish
                        | Species::Pufferfish
                        | Species::Squid
                        | Species::GlowSquid
                        | Species::Dolphin
                        | Species::Turtle
                        | Species::Guardian
                        | Species::ElderGuardian
                        | Species::Tadpole
                        | Species::Nautilus
                        | Species::ZombieNautilus
                        | Species::SulfurCube
                )
                && area.intersects(s.bounds)
        });
        player || mob
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.counter = 1;
        m.extra.counter2 = 0;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.counter = 0;
    }
}
/// finalizeSpawn variants (tropical fish patterns and colours).
/// TropicalFish.COMMON_VARIANTS: (pattern index, base colour, pattern colour).
pub const COMMON_TROPICAL: [(u8, u8, u8); 22] = [
    (7, 1, 7),
    (6, 7, 7),
    (6, 7, 11),
    (11, 0, 7),
    (1, 11, 7),
    (0, 1, 0),
    (5, 6, 3),
    (9, 10, 4),
    (11, 0, 14),
    (5, 0, 4),
    (8, 0, 7),
    (11, 0, 1),
    (3, 9, 6),
    (4, 5, 3),
    (10, 14, 0),
    (2, 7, 14),
    (9, 14, 0),
    (6, 0, 4),
    (0, 14, 0),
    (1, 7, 0),
    (3, 9, 4),
    (6, 4, 4),
];
pub fn finalize(m: &mut Mob) {
    if m.species() == Species::TropicalFish {
        let (p, b, c) = if m.random.next_float() < 0.9 {
            COMMON_TROPICAL[m
                .random
                .next_int_bound(COMMON_TROPICAL.len() as i32)
                .unwrap_or(0) as usize]
        } else {
            (
                m.random.next_int_bound(12).unwrap_or(0) as u8,
                m.random.next_int_bound(16).unwrap_or(0) as u8,
                m.random.next_int_bound(16).unwrap_or(0) as u8,
            )
        };
        m.extra.variant = p;
        m.extra.variant2 = b;
        m.extra.variant3 = c;
    }
}
/// Model animation for the water mobs.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    use Species as S;
    let amp = if m.fluids.in_water { 1.0 } else { 1.5 };
    match m.species() {
        S::Squid | S::GlowSquid => {
            let angle = m.anim.lerp(TENTACLE_ANGLE, s.partial);
            for i in 0..8 {
                p.xrot(&format!("tentacle{i}"), angle);
            }
        }
        S::Cod => p.yrot("tail_fin", -amp * 0.45 * (0.6 * s.age).msin()),
        // TadpoleModel: a stronger tail swing out of water.
        S::Tadpole => {
            let a = if m.fluids.in_water { 1.0 } else { 1.5 };
            p.yrot("tail", -a * 0.25 * (0.3 * s.age).msin())
        }
        S::TropicalFish => p.yrot("tail", -amp * 0.45 * (0.6 * s.age).msin()),
        S::Salmon => {
            let (a, k) = if m.fluids.in_water {
                (1.0, 1.0)
            } else {
                (1.3, 1.7)
            };
            p.yrot("body_back", -a * 0.25 * (k * 0.6 * s.age).msin());
        }
        S::Pufferfish => {
            let fin = 0.2 - 0.4 * (s.age * 0.2).msin();
            if p.has("right_blue_fin") {
                p.zrot("right_blue_fin", -fin);
                p.zrot("left_blue_fin", fin);
            } else {
                p.zrot("right_fin", -fin);
                p.zrot("left_fin", fin);
            }
        }
        S::Dolphin => {
            p.xrot("body", s.x_rot.to_radians());
            p.yrot("body", s.y_rot.to_radians());
            let moving = m.velocity.x * m.velocity.x + m.velocity.z * m.velocity.z > 1.0e-7;
            if moving {
                p.get("body")[3] += -0.05 - 0.05 * (s.age * 0.3).mcos();
                p.xrot("tail", -0.1 * (s.age * 0.3).mcos());
                p.xrot("tail_fin", -0.2 * (s.age * 0.3).mcos());
            }
        }
        _ => {}
    }
}
/// Renderer setupRotations for the water mobs (replacing or extending the default).
pub fn rotations(m: &Mob, alpha: f32, body: f32) -> Option<glam::Mat4> {
    use Species as S;
    let age = m.age as f32 + alpha;
    match m.species() {
        S::Squid | S::GlowSquid => {
            let baby = m.is_baby();
            let x = m.anim.lerp(X_BODY_ROT, alpha);
            let z = m.anim.lerp(Z_BODY_ROT, alpha);
            Some(
                glam::Mat4::from_translation(glam::Vec3::new(
                    0.0,
                    if baby { 0.25 } else { 0.5 },
                    0.0,
                )) * glam::Mat4::from_rotation_y((180.0 - body).to_radians())
                    * glam::Mat4::from_rotation_x(x.to_radians())
                    * glam::Mat4::from_rotation_y(z.to_radians())
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        0.0,
                        if baby { -0.6 } else { -1.2 },
                        0.0,
                    )),
            )
        }
        S::Cod | S::Salmon | S::TropicalFish => {
            let (a, k) = if m.species() == S::Salmon && !m.fluids.in_water {
                (1.3, 1.7)
            } else {
                (1.0, 1.0)
            };
            let mut t = glam::Mat4::from_rotation_y((180.0 - body).to_radians())
                * glam::Mat4::from_rotation_y((a * 4.3 * (k * 0.6 * age).msin()).to_radians());
            if !m.fluids.in_water {
                let off = if m.species() == S::Cod {
                    glam::Vec3::new(0.1, 0.1, -0.1)
                } else {
                    glam::Vec3::new(0.2, 0.1, 0.0)
                };
                t *= glam::Mat4::from_translation(off) * glam::Mat4::from_rotation_z(PI / 2.0);
            }
            Some(t)
        }
        S::Pufferfish => Some(
            glam::Mat4::from_translation(glam::Vec3::new(0.0, (age * 0.05).mcos() * 0.08, 0.0))
                * glam::Mat4::from_rotation_y((180.0 - body).to_radians()),
        ),
        _ => None,
    }
}
#[cfg(test)]
mod school_tests {
    use super::*;
    use crate::mobs::sensor_scan_tests::kind;
    use std::sync::Arc;

    fn cod(id: u32) -> Mob {
        let mut kind = kind();
        let k = Arc::make_mut(&mut kind);
        k.id = "minecraft:cod".into();
        k.species = Species::Cod;
        Mob::new(id, kind, Vec3::ZERO, 0.0)
    }
    #[test]
    fn school_sizes() {
        assert_eq!(max_school_size(Species::Cod), 8);
        assert_eq!(max_school_size(Species::TropicalFish), 8);
        assert_eq!(max_school_size(Species::Salmon), 5);
        // canBeFollowed: followers and room (schoolSize below the maximum).
        assert!(!can_be_followed(Species::Cod, 0));
        assert!(can_be_followed(Species::Cod, 6));
        assert!(!can_be_followed(Species::Cod, 7));
        assert!(!can_be_followed(Species::Salmon, 4));
    }
    #[test]
    fn a_recruit_another_leader_took_stays_with_it() {
        let mut mobs = vec![cod(1), cod(2), cod(3)];
        start_following(&mut mobs, 3, 1);
        assert_eq!(mobs[2].extra.leader, Some(1));
        assert_eq!(mobs[0].extra.followers, 1);
        start_following(&mut mobs, 3, 2);
        assert_eq!(mobs[2].extra.leader, Some(1));
        assert_eq!(mobs[1].extra.followers, 0);
        // A dead leader's follower is free again.
        mobs[0].health = 0.0;
        start_following(&mut mobs, 3, 2);
        assert_eq!(mobs[2].extra.leader, Some(2));
        assert_eq!(mobs[1].extra.followers, 1);
    }
    #[test]
    fn schooling_fish_draw_their_start_delay() {
        let mut m = cod(1);
        let start = FollowFlockLeader::next_start_tick(&mut m);
        assert!((100..=110).contains(&start));
    }
}
