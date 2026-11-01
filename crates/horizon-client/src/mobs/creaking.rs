//! Creakings (`Creaking`): they freeze while any player looks at them, wake
//! up when one looks from within 12 blocks, then hunt the nearest visible
//! player and hit every 40 ticks (the brain, `CreakingAi`, is in
//! `monster_brain`). One bound to a creaking heart takes no damage (it only
//! sways and hurts the heart's resin spreader), dies when the heart no
//! longer protects it and falls apart with the heart.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    keyframes::Animation,
    monster_brain,
    path::PathType,
};
use crate::containers::BlockEntity;
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::HashMap;

/// `Extra.ints` slots.
const INVULNERABLE: usize = 0;
const ATTACKING: usize = 1;
const STUCK: usize = 2;
const TEARING_DOWN: usize = 3;
const EYES: usize = 4;
const NEXT_FLICKER: usize = 5;
/// `Extra.counter` bits: hurt while protected, and by a player.
const HURT: i32 = 1;
const HURT_BY_PLAYER: i32 = 2;

/// `Creaking.ACTIVATION_RANGE_SQ`.
const ACTIVATION_RANGE_SQ: f64 = 144.0;
/// `Creaking.INVULNERABILITY_ANIMATION_DURATION` and the attack animation.
const INVULNERABILITY_TICKS: i32 = 8;
const ATTACK_TICKS: i32 = 15;
/// `Creaking.TWITCH_DEATH_DURATION`.
const TWITCH_TICKS: i32 = 45;

/// `Creaking`'s constructor: no goals (the brain thinks), a navigation that
/// floats.
pub fn register(m: &mut Mob) {
    m.ai.nav.can_float = true;
}
/// Bound to a heart (`getHomePos() != null`).
pub fn heart_bound(m: &Mob) -> bool {
    is_creaking(m) && m.extra.home.is_some()
}
pub fn tearing_down(m: &Mob) -> bool {
    is_creaking(m) && m.extra.ints[TEARING_DOWN] != 0
}
/// `Creaking.canMove`: false while a player looks at it.
pub fn can_move(m: &Mob) -> bool {
    !m.extra.flag2
}
/// `Creaking.isActive`.
pub fn is_active(m: &Mob) -> bool {
    m.extra.flag
}
/// `Creaking.setTransient`: bind to the heart at `pos` and avoid damage.
pub fn set_transient(m: &mut Mob, pos: [i32; 3]) {
    m.extra.home = Some(pos);
    m.set_malus(PathType::Damaging, 8.0);
    m.set_malus(PathType::PowderSnow, 8.0);
    m.set_malus(PathType::Lava, 8.0);
    m.set_malus(PathType::Fire, 0.0);
    m.set_malus(PathType::FireInNeighbor, 0.0);
}
/// `Creaking.setTearingDown`: the heart was broken by someone.
pub fn set_tearing_down(m: &mut Mob) {
    m.extra.ints[TEARING_DOWN] = 1;
}
/// `Creaking.hasGlowingEyes` / `isActive`: the eyes layer shows.
pub fn eyes_glowing(m: &Mob) -> bool {
    if tearing_down(m) {
        m.extra.ints[EYES] != 0
    } else {
        is_active(m)
    }
}
/// Ticks of death animation before the mob is removed (`tickDeath`).
pub fn death_limit(m: &Mob) -> i32 {
    if heart_bound(m) && tearing_down(m) {
        TWITCH_TICKS + 1
    } else {
        20
    }
}
/// `Creaking.tearDown`: crumbles away.
pub fn tear_down(m: &mut Mob, events: &mut Vec<MobEvent>) {
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    events.push(MobEvent::CreakingTearDown {
        center: [
            (lo.x + hi.x) / 2.0,
            (lo.y + hi.y) / 2.0,
            (lo.z + hi.z) / 2.0,
        ],
        spread: [
            (hi.x - lo.x) * 0.3,
            (hi.y - lo.y) * 0.3,
            (hi.z - lo.z) * 0.3,
        ],
    });
    m.voice("entity.creaking.death", events);
    m.removed = true;
}
/// `Creaking.hurtServer` for a heart-bound creaking: no damage, only the
/// invulnerability animation and the heart's reaction. Returns whether the
/// normal damage path should run.
pub fn accepts_damage(m: &mut Mob, source: &Damage) -> bool {
    if !heart_bound(m) || source.kind.bypasses_invulnerability() {
        return true;
    }
    if source.is_fire() || !m.alive() || m.extra.ints[INVULNERABLE] > 0 {
        return false;
    }
    let by_player = source.attacker == Some(Who::Player);
    // The direct entity must be living or a projectile (or a player must be to blame).
    let direct = matches!(
        source.kind,
        DamageKind::PlayerAttack
            | DamageKind::MaceSmash
            | DamageKind::Spear
            | DamageKind::MobAttack
            | DamageKind::Arrow
            | DamageKind::Thrown
            | DamageKind::Fireball
            | DamageKind::WitherSkull
            | DamageKind::WindCharge
            | DamageKind::Thorns
    );
    if !direct && !by_player {
        return false;
    }
    m.extra.ints[INVULNERABLE] = INVULNERABILITY_TICKS;
    m.extra.counter |= HURT | if by_player { HURT_BY_PLAYER } else { 0 };
    false
}
/// The rest of `Creaking.hurtServer` for a heart-bound creaking, run by the
/// host right after the damage: whether its heart still protects it, and
/// whether a player is to blame (the heart then calls `creakingHurt`). The
/// caller plays [`sway`] after the heart's reaction.
pub fn take_hurt(m: &mut Mob, w: &MobWorld) -> Option<([i32; 3], bool)> {
    if m.extra.counter & HURT == 0 {
        return None;
    }
    let by_player = m.extra.counter & HURT_BY_PLAYER != 0;
    m.extra.counter &= !(HURT | HURT_BY_PLAYER);
    let home = m.extra.home?;
    protected(m, w).then_some((home, by_player))
}
/// `Mob.playHurtSound` of a heart-bound creaking: the ambient sound clock
/// restarts and it sways.
pub fn sway(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.ambient_timer = -m.kind.ambient_interval;
    m.voice("entity.creaking.sway", events);
}
/// `Creaking.doHurtTarget`: the attack animation (the sound is the
/// `playAttackSound` that ends `Mob.doHurtTarget`).
pub fn on_attack(m: &mut Mob, events: &mut Vec<MobEvent>) {
    m.extra.ints[ATTACKING] = ATTACK_TICKS;
    broadcast(m, 4, events);
}
/// `Level.broadcastEntityEvent` for a creaking (guests run `entity_event`).
fn broadcast(m: &Mob, event: i8, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event,
        at: [m.position.x, m.position.y, m.position.z],
    });
}
/// `Creaking.handleEntityEvent` on a client: 66 the invulnerability flash,
/// 4 the attack swing (the sounds are the server's).
pub fn entity_event(m: &mut Mob, event: u8) {
    match event {
        66 => m.extra.ints[INVULNERABLE] = INVULNERABILITY_TICKS,
        4 => m.extra.ints[ATTACKING] = ATTACK_TICKS,
        _ => {}
    }
}
/// The heart this creaking is bound to protects it (`isProtector`); a link
/// read from a save is trusted until the heart resolves it.
fn protected(m: &Mob, w: &MobWorld) -> bool {
    let Some(home) = m.extra.home else {
        return true;
    };
    match w.world.block_entities.get(&(home[0], home[1], home[2])) {
        Some(BlockEntity::CreakingHeart(h)) => match h.creaking {
            Some((Some(id), _)) => id == m.id,
            Some((None, _)) => true,
            None => false,
        },
        _ => false,
    }
}
/// `Creaking.tick` and the start of `aiStep`: protection, timers, freezing.
pub fn update(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    // Creaking.tick: without its heart's protection it dies.
    if heart_bound(m) && m.alive() && !protected(m, w) {
        m.health = 0.0;
    }
    // hurtServer set the invulnerability clock since the last tick: entity
    // event 66.
    if m.extra.ints[INVULNERABLE] == INVULNERABILITY_TICKS {
        broadcast(m, 66, events);
    }
    animate(m);
    if m.extra.counter & HURT != 0 {
        // hurtServer: sway, and the heart reacts to a player's hit.
        if protected(m, w) {
            let by_player = m.extra.counter & HURT_BY_PLAYER != 0;
            if let Some(home) = m.extra.home {
                events.push(MobEvent::CreakingHurt {
                    home,
                    by_player,
                    id: m.id,
                });
            }
            sway(m, events);
        }
        m.extra.counter &= !(HURT | HURT_BY_PLAYER);
    }
    if !m.alive() {
        return;
    }
    let now = check_can_move(m, w, events);
    if now != can_move(m) {
        if now {
            m.voice("entity.creaking.unfreeze", events);
        } else {
            stop_in_place(m);
            m.voice("entity.creaking.freeze", events);
        }
        m.extra.flag2 = !now;
    }
}
/// The clocks and keyframe states `Creaking.aiStep`, `setupAnimationStates`
/// and `checkEyeBlink` run (the server's and a client's).
pub fn animate(m: &mut Mob) {
    for slot in [INVULNERABLE, ATTACKING] {
        if m.extra.ints[slot] > 0 {
            m.extra.ints[slot] -= 1;
        }
    }
    let age = m.age as f32;
    m.anim.states[0].animate_when(m.extra.ints[ATTACKING] > 0, age);
    m.anim.states[1].animate_when(m.extra.ints[INVULNERABLE] > 0, age);
    m.anim.states[2].animate_when(tearing_down(m), age);
    // checkEyeBlink: the eyes of a creaking falling apart flicker.
    if m.death_time > m.extra.ints[NEXT_FLICKER] {
        let glowing = m.extra.ints[EYES] != 0;
        let (lo, hi) = if glowing {
            (2, 8)
        } else {
            (m.death_time / 4, m.death_time / 2)
        };
        let span = (hi - lo + 1).max(1);
        m.extra.ints[NEXT_FLICKER] = m.death_time + lo + m.random.next_int_bound(span).unwrap_or(0);
        m.extra.ints[EYES] = i32::from(!glowing);
    }
}
/// `Mob.stopInPlace`.
pub fn stop_in_place(m: &mut Mob) {
    m.nav_stop();
    m.ai.xxa = 0.0;
    m.ai.yya = 0.0;
    m.ai.zza = 0.0;
    m.ai.speed = 0.0;
    m.velocity = horizon_core::geometry::Vec3::ZERO;
    m.extra.leash_spin = 0.0;
}
/// `Creaking.activate`.
fn activate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    monster_brain::set_attack_target(m, Some(Who::Player));
    m.voice("entity.creaking.activate", events);
    m.extra.flag = true;
}
/// `Creaking.deactivate`.
fn deactivate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    monster_brain::set_attack_target(m, None);
    m.voice("entity.creaking.deactivate", events);
    m.extra.flag = false;
}
/// `LivingEntity.isLookingAtMe(player, 0.5, false, true, eyeY, y + 0.5,
/// (eyeY + y) / 2)`: within the player's view cone and seen through
/// transparent blocks.
fn looked_at_by(m: &Mob, w: &MobWorld) -> bool {
    let Some(p) = &w.player else { return false };
    let look = normalize(p.look);
    let eye = m.eye_y();
    [eye, m.position.y + 0.5, (eye + m.position.y) / 2.0]
        .into_iter()
        .any(|y| {
            let d = normalize([
                m.position.x - p.position.x,
                y - p.eye[1],
                m.position.z - p.position.z,
            ]);
            let dot = look[0] * d[0] + look[1] * d[1] + look[2] * d[2];
            if dot <= 0.5 {
                return false;
            }
            // Player.hasLineOfSight(this, VISUAL, NONE, y).
            let from = Vec3::new(p.position.x, p.eye[1], p.position.z);
            let to = Vec3::new(m.position.x, y, m.position.z);
            to.distance(from) <= 128.0 && w.visual_clear_between(from, to)
        })
}
/// `Vec3.normalize`.
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < f64::from(1.0e-5_f32) {
        [0.0; 3]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}
/// `Creaking.checkCanMove`: false while looked at; also wakes and calms it.
fn check_can_move(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    let players = monster_brain::nearest_players(m);
    let active = is_active(m);
    if players.is_empty() {
        if active {
            deactivate(m, events);
        }
        return true;
    }
    let mut potential = false;
    if let Some(p) = w.player.as_ref().filter(|_| players.contains(&Who::Player)) {
        // canAttack (not creative, spectator or dead) and not allied.
        if p.alive && !p.creative && !p.spectator {
            potential = true;
            // GAZE_DISGUISE_EQUIPMENT hides the player once the creaking is awake.
            let disguised = p.head.as_deref() == Some("minecraft:carved_pumpkin");
            if (!active || !disguised) && looked_at_by(m, w) {
                if active {
                    return false;
                }
                if m.distance_sqr(p.position) < ACTIVATION_RANGE_SQ {
                    activate(m, events);
                    return false;
                }
            }
        }
    }
    if !potential && active {
        deactivate(m, events);
    }
    true
}
/// `Creaking.playerIsStuckInYou`: the player's eyes inside it for more than 4 ticks.
pub fn player_is_stuck(m: &mut Mob, eye: Option<[f64; 3]>) -> bool {
    let Some(eye) = eye else {
        m.extra.ints[STUCK] = 0;
        return false;
    };
    let b = m.bounds();
    let (lo, hi) = (b.min_position(), b.max_position());
    // AABB.contains: the far faces are outside.
    let inside = eye[0] >= lo.x
        && eye[0] < hi.x
        && eye[1] >= lo.y
        && eye[1] < hi.y
        && eye[2] >= lo.z
        && eye[2] < hi.z;
    if inside {
        m.extra.ints[STUCK] += 1;
        m.extra.ints[STUCK] > 4
    } else {
        m.extra.ints[STUCK] = 0;
        false
    }
}
/// `CreakingModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let get = |n: &str| animations.get(&format!("CreakingAnimation.CREAKING_{n}"));
    if can_move(m)
        && let Some(walk) = get("WALK")
    {
        walk.apply_walk(s.pos, s.speed, 1.0, 1.0, p);
    }
    for (state, name) in [
        (&m.anim.states[0], "ATTACK"),
        (&m.anim.states[1], "INVULNERABLE"),
        (&m.anim.states[2], "DEATH"),
    ] {
        if let (Some(ms), Some(a)) = (state.millis(s.age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
}
/// The creaking species exists in the species table.
pub fn is_creaking(m: &Mob) -> bool {
    m.species() == Species::Creaking
}
