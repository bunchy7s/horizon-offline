//! Ravagers (`Ravager`): charging beasts that trample leaves, bite for 12,
//! get stunned by a raised shield half the time and answer with a roar
//! that hurts and flings everything within 4 blocks.
use super::{
    Damage, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{Float, HurtByTarget, LookAtPlayer, MeleeAttack, NearestTarget, Stroll},
    path::PathType,
};
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::f32::consts::PI;

// Ravager state: `counter` attackTick, `counter2` stunnedTick,
// `anim_ticks` roarTick.

pub fn register(m: &mut Mob) {
    m.set_malus(PathType::Leaves, 0.0);
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(4, MeleeAttack::new(1.0, true));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(6.0));
    g.add(10, LookAtPlayer::mobs(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![
        Species::Pillager,
        Species::Vindicator,
        Species::Evoker,
        Species::Illusioner,
        Species::Witch,
        Species::Ravager,
    ];
    m.ai.targets.add(2, hurt);
    m.ai.targets.add(3, NearestTarget::player(true));
    m.ai.targets.add(
        4,
        NearestTarget::new(
            super::goals::Prey::Filter(|s| {
                matches!(s.species, Species::Villager | Species::WanderingTrader) && !s.baby
            }),
            true,
        ),
    );
    m.ai.targets
        .add(4, NearestTarget::species(&[Species::IronGolem], true));
}
/// `Ravager.isImmobile`: biting, stunned or roaring.
pub fn immobile(m: &Mob) -> bool {
    m.extra.counter > 0 || m.extra.counter2 > 0 || m.extra.anim_ticks > 0
}
/// `Ravager.doHurtTarget`: the bite animation (entity event 4 for clients).
pub fn on_attack(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.counter = 10;
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: [m.position.x, m.position.y, m.position.z],
    });
    events.push(MobEvent::Sound(
        "entity.ravager.attack",
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `Ravager.blockedByItem`: a raised shield stuns it half the time (else
/// flings the defender); returns whether it was stunned.
pub fn blocked(m: &mut Mob, events: &mut Vec<MobEvent>) -> bool {
    if m.extra.anim_ticks != 0 {
        return false;
    }
    if m.random.next_double() < 0.5 {
        m.extra.counter2 = 40;
        // Entity event 39: the stun starts on clients.
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 39,
            at: [m.position.x, m.position.y, m.position.z],
        });
        events.push(MobEvent::Sound(
            "entity.ravager.stunned",
            [m.position.x, m.position.y, m.position.z],
            1.0,
            1.0,
        ));
        true
    } else {
        false
    }
}
/// `Ravager.aiStep`: speed toward 0.35 while hunting, leaf and crop trampling, and
/// the bite, stun and roar clocks.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    let max = if immobile(m) {
        0.0
    } else if m.ai.target.is_some() {
        0.35
    } else {
        0.3
    };
    m.attrs.speed = if max == 0.0 {
        0.0
    } else {
        m.attrs.speed + (max - m.attrs.speed) * 0.1
    };
    if m.horizontal_collision && w.mob_griefing {
        let b = m
            .bounds()
            .inflate(horizon_core::geometry::Vec3::new(0.2, 0.2, 0.2));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut destroyed = false;
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let name = w.block_name(x, y, z);
                    if name.ends_with("_leaves") {
                        events.push(MobEvent::BreakBlock {
                            pos: [x, y, z],
                            drop: true,
                        });
                        destroyed = true;
                    }
                }
            }
        }
        if !destroyed && m.on_ground {
            m.ai.jumping = true;
        }
    }
    // Entity.checkInsideBlocks -> CropBlock.entityInside: a ravager walking
    // through a crop destroys it, drops and all, with mobGriefing.
    if w.mob_griefing {
        let b = m
            .bounds()
            .deflate(horizon_core::geometry::Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    if matches!(
                        w.block_name(x, y, z),
                        "minecraft:wheat"
                            | "minecraft:carrots"
                            | "minecraft:potatoes"
                            | "minecraft:beetroots"
                            | "minecraft:torchflower_crop"
                    ) {
                        events.push(MobEvent::BreakBlock {
                            pos: [x, y, z],
                            drop: true,
                        });
                    }
                }
            }
        }
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
        if m.extra.anim_ticks == 10 {
            roar(m, w, events);
        }
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
        if m.random.next_int_bound(6).unwrap_or(1) == 0 {
            let yaw = m.body_yaw.to_radians();
            let width = f64::from(m.width());
            events.push(MobEvent::Particles {
                kind: "entity_effect",
                at: [
                    m.position.x - width * f64::from(yaw.msin()),
                    m.position.y + f64::from(m.height()) - 0.3,
                    m.position.z + width * f64::from(yaw.mcos()),
                ],
                count: 0,
            });
        }
        if m.extra.counter2 == 0 {
            events.push(MobEvent::Sound(
                "entity.ravager.roar",
                [m.position.x, m.position.y, m.position.z],
                1.0,
                1.0,
            ));
            m.extra.anim_ticks = 20;
        }
    }
}
/// `Ravager.handleEntityEvent` 4 and 39 on a client.
pub fn entity_event(m: &mut Mob, event: u8) {
    match event {
        4 => m.extra.counter = 10,
        39 => m.extra.counter2 = 40,
        _ => {}
    }
}
/// `Ravager.aiStep`'s clocks on a client: the bite and roar run down, and
/// the end of the stun starts the roar (the sounds, particles and the roar's
/// damage are the server's).
pub fn client_tick(m: &mut Mob) {
    if !m.alive() {
        return;
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.extra.counter > 0 {
        m.extra.counter -= 1;
    }
    if m.extra.counter2 > 0 {
        m.extra.counter2 -= 1;
        if m.extra.counter2 == 0 {
            m.extra.anim_ticks = 20;
        }
    }
}
/// `Ravager.roar`: 6 damage to everything but illagers within 4 blocks,
/// and a strong knockback (players too, through the client).
fn roar(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let area = m
        .bounds()
        .inflate(horizon_core::geometry::Vec3::new(4.0, 4.0, 4.0));
    let from = [m.position.x, m.position.y, m.position.z];
    let fling = |p: horizon_core::geometry::Vec3| {
        let (xd, zd) = (p.x - m.position.x, p.z - m.position.z);
        let dd = (xd * xd + zd * zd).max(0.001);
        [xd / dd * 4.0, 0.2, zd / dd * 4.0]
    };
    if let Some(p) = &w.player
        && p.alive
        && p.bounds.intersects(area)
    {
        events.push(MobEvent::AttackPlayer(6.0, from));
        events.push(MobEvent::Push {
            target: Who::Player,
            velocity: fling(p.position),
        });
    }
    for s in w.others_near(area) {
        if s.id == m.id || !s.alive || s.species == Species::Ravager || !s.bounds.intersects(area) {
            continue;
        }
        let illager = matches!(
            s.species,
            Species::Pillager | Species::Vindicator | Species::Evoker | Species::Illusioner
        );
        if !illager {
            events.push(MobEvent::AttackMob {
                target: s.id,
                amount: 6.0,
                source: Damage::mob(m.id, from),
            });
        }
        events.push(MobEvent::Push {
            target: Who::Mob(s.id),
            velocity: fling(s.position),
        });
    }
    // Entity event 69: a puff of smoke, and the client's own push. A client
    // of this server (the window of the server thread) gets the event;
    // this window draws the puff itself.
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 69,
        at: from,
    });
    events.push(MobEvent::Particles {
        kind: "poof",
        at: [
            m.position.x,
            m.position.y + f64::from(m.height()) / 2.0,
            m.position.z,
        ],
        count: 40,
    });
}
/// `RavagerModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let stunned = if m.extra.counter2 > 0 {
        m.extra.counter2 as f32 - s.partial
    } else {
        0.0
    };
    let attack = if m.extra.counter > 0 {
        m.extra.counter as f32 - s.partial
    } else {
        0.0
    };
    if attack > 0.0 {
        // Mth.triangleWave(attack, 10).
        let wave = ((attack % 10.0 - 5.0).abs() - 2.5) / 2.5;
        let scaled = (1.0 + wave) * 0.5;
        let head = scaled * scaled * scaled * 12.0;
        let neck_x_rot = p.get("neck")[3];
        let neck = p.get("neck");
        neck[2] = -6.5 + head;
        neck[1] = -7.0 - head * neck_x_rot.msin();
        let mouth = if attack > 5.0 {
            ((-4.0 + attack) / 4.0).msin() * PI * 0.4
        } else {
            PI / 20.0 * (PI * attack / 10.0).msin()
        };
        p.xrot("mouth", mouth);
    } else {
        let is_stunned = stunned > 0.0;
        let x_rot: f32 = if is_stunned { 0.219_911_49 } else { 0.0 };
        let neck = p.get("neck");
        let y_offset = -neck[3].msin();
        neck[0] = 0.0;
        neck[1] = -7.0 - y_offset;
        neck[2] = 5.5;
        neck[3] = x_rot;
        p.xrot("mouth", PI * if is_stunned { 0.05 } else { 0.01 });
        if is_stunned {
            p.get("neck")[0] = ((f64::from(stunned) / 40.0) * 10.0).sin() as f32 * 3.0;
        } else if m.extra.anim_ticks > 0 {
            let roar = (20.0 - m.extra.anim_ticks as f32 + s.partial) / 20.0;
            p.xrot("mouth", PI / 2.0 * (roar * PI * 0.25).msin());
        }
    }
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let (pos, leg) = (s.pos, 0.4 * s.speed);
    p.xrot("right_hind_leg", (pos * 0.6662).mcos() * leg);
    p.xrot("left_hind_leg", (pos * 0.6662 + PI).mcos() * leg);
    p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * leg);
    p.xrot("left_front_leg", (pos * 0.6662).mcos() * leg);
}
