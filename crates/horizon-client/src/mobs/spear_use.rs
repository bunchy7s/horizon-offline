//! Mobs using a spear (`KineticWeapon`): `startUsingItem` of the main hand,
//! the stab of every tick of the use (`KineticWeapon.damageEntities`) and
//! what makes a mob use it: `SpearUseGoal` of zombies and zombified piglins,
//! and the fight activity's `SpearApproach`, `SpearAttack` and `SpearRetreat`
//! of piglins.
//! A mob's `using_item` counts the ticks of the use up from 1, which the
//! living entity flags tell the clients (`mob_meta`), so the arm and the
//! spear are drawn charging for them as well.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    brain::Timed,
    gear,
    goals::{Cx, Goal, LOOK, MOVE, goal_clone, reduced},
};
use crate::spear::{Kinetic, REACH, hits_in_reach};
use horizon_core::geometry::{Aabb, Vec3, trig::Mth};

/// The kinetic weapon in the main hand (`getMainHandItem().get(KINETIC_WEAPON)`).
pub fn main_hand_kinetic(m: &Mob) -> Option<Kinetic> {
    m.extra.main_hand.as_deref().and_then(Kinetic::of)
}

/// `LivingEntity.isUsingItem`. A piglin admiring what it holds in its off
/// hand counts `using_item` down as well; vanilla keeps that in a memory.
pub fn is_using_item(m: &Mob) -> bool {
    m.extra.using_item > 0 && !(m.species() == Species::Piglin && m.extra.off_hand.is_some())
}

/// `LivingEntity.startUsingItem(MAIN_HAND)`: the use of a kinetic weapon
/// starts with a new `recentKineticEnemies`.
pub fn start_using_item(m: &mut Mob) {
    if m.extra.main_hand.is_none() || is_using_item(m) {
        return;
    }
    m.extra.using_item = 1;
    m.extra.using_off_hand = false;
    if main_hand_kinetic(m).is_some() {
        m.extra.stabbed = Some(Vec::new());
    }
}

/// `LivingEntity.stopUsingItem`: `recentKineticEnemies` is gone.
pub fn stop_using_item(m: &mut Mob) {
    if is_using_item(m) {
        m.extra.using_item = 0;
    }
    m.extra.stabbed = None;
}

/// `LivingEntity.updatingUsingItem` of a kinetic weapon, in `LivingEntity.tick`
/// before the AI: the stab of this tick, then one more tick used. A use whose
/// weapon left the main hand ends. `known_speed` is what the mob moved last
/// tick (`Entity.lastKnownSpeed`).
pub fn update_using_item(m: &mut Mob, w: &MobWorld, known_speed: Vec3, events: &mut Vec<MobEvent>) {
    if m.extra.stabbed.is_none() {
        return;
    }
    let Some(kinetic) = main_hand_kinetic(m).filter(|_| is_using_item(m)) else {
        stop_using_item(m);
        return;
    };
    // `ItemStack.onUseTick` gets the ticks left; the use began at 1.
    let ticks_used = m.extra.using_item - 1;
    damage_entities(m, &kinetic, ticks_used, w, known_speed, events);
    if is_using_item(m) {
        m.extra.using_item += 1;
    }
}

/// What a stab can meet: who, where and how fast it moves per second.
struct Candidate {
    who: Who,
    bounds: Aabb,
    speed: Vec3,
}

/// The entities a stab may hit (`PiercingWeapon.canHitEntity`): the player
/// the mob sees and the other living mobs near it, but not the mount it sits
/// on or the rider it carries.
fn candidates(m: &Mob, w: &MobWorld) -> Vec<Candidate> {
    let mut out = Vec::new();
    if let Some(p) = w.player.as_ref().filter(|p| p.alive && !p.spectator) {
        out.push(Candidate {
            who: Who::Player,
            bounds: p.bounds,
            speed: p.velocity.scale(20.0),
        });
    }
    let r = 6.0;
    let area = m.bounds().inflate(Vec3::new(r, r, r));
    for s in w.others_near(area) {
        if s.id == m.id || !s.alive || m.extra.mount == Some(s.id) || m.extra.rider == Some(s.id) {
            continue;
        }
        out.push(Candidate {
            who: Who::Mob(s.id),
            bounds: s.bounds,
            speed: s.known_speed.scale(20.0),
        });
    }
    out
}

/// `Entity.getHeadLookAngle`: `calculateViewVector(xRot, yHeadRot)`.
fn head_look_angle(m: &Mob) -> Vec3 {
    let to_rad = std::f32::consts::PI / 180.0;
    let x = m.head_pitch * to_rad;
    let y = -m.head_yaw * to_rad;
    let (y_cos, y_sin, x_cos, x_sin) = (y.mcos(), y.msin(), x.mcos(), x.msin());
    Vec3::new(
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    )
}

/// `KineticWeapon.getMotion`: the known speed per second of the attacker, or
/// of the mount it rides.
fn motion(m: &Mob, w: &MobWorld, known_speed: Vec3) -> Vec3 {
    m.extra
        .mount
        .and_then(|id| w.seen(id))
        .map_or(known_speed, |s| s.known_speed)
        .scale(20.0)
}

/// `KineticWeapon.damageEntities` for a mob: from `delay_ticks` of the use
/// on, every entity along the view that the speed allows is stabbed, once in
/// `contact_cooldown_ticks` each. A mob's conditions are scaled by 0.2 and
/// its reach by the `mob_factor` of the spear's `AttackRange`.
fn damage_entities(
    m: &mut Mob,
    k: &Kinetic,
    ticks_used: i32,
    w: &MobWorld,
    known_speed: Vec3,
    events: &mut Vec<MobEvent>,
) {
    if ticks_used < k.delay_ticks {
        return;
    }
    let ticks_used = ticks_used - k.delay_ticks;
    let look = m.look_angle();
    let look = Vec3::new(look[0], look[1], look[2]);
    let attacker_speed = look.dot(motion(m, w, known_speed));
    // `AttackRange.effectiveMinRange` and `effectiveMaxRange` of a mob.
    let reach = (
        REACH.min * REACH.mob_factor,
        REACH.max * REACH.mob_factor,
        REACH.hitbox_margin,
    );
    let targets = candidates(m, w);
    let boxes: Vec<(usize, Aabb)> = targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.bounds))
        .collect();
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    let hits = hits_in_reach(
        &boxes,
        eye,
        head_look_angle(m),
        m.velocity,
        reach,
        |from, to| w.clip_block(from, to),
    );
    let now = w.game_time;
    let action_factor = f64::from(0.2_f32);
    // The attack damage attribute's base value.
    let base = m.attrs.attack_damage;
    let mut affected = false;
    for (i, _) in hits {
        let target = &targets[i];
        let recent = m.extra.stabbed.as_ref().is_some_and(|stabbed| {
            stabbed.iter().any(|&(who, at)| {
                who == target.who && now - at < i64::from(k.contact_cooldown_ticks)
            })
        });
        if recent {
            continue;
        }
        // `LivingEntity.rememberStabbedEntity`.
        if let Some(stabbed) = m.extra.stabbed.as_mut() {
            match stabbed.iter_mut().find(|(who, _)| *who == target.who) {
                Some(entry) => entry.1 = now,
                None => stabbed.push((target.who, now)),
            }
        }
        let relative = (attacker_speed - look.dot(target.speed)).max(0.0);
        let dismounts = k
            .dismount
            .test(ticks_used, attacker_speed, relative, action_factor);
        let pushes = k
            .knockback
            .test(ticks_used, attacker_speed, relative, action_factor);
        let damages = k
            .damage
            .test(ticks_used, attacker_speed, relative, action_factor);
        if dismounts || pushes || damages {
            let amount = base + (relative * f64::from(k.damage_multiplier)).floor() as i32 as f32;
            affected |= stab_attack(
                m,
                target.who,
                amount,
                (damages, pushes, dismounts),
                w,
                events,
            );
        }
    }
    if affected {
        // `Level.broadcastEntityEvent(mob, 2)`: the hit is felt and heard.
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event: 2,
            at: [m.position.x, m.position.y, m.position.z],
        });
    }
}

/// `LivingEntity.stabAttack` of a mob on `target`, with what the conditions
/// allow (`deals_damage`, `deals_knockback`, `dismounts`): whether it was
/// affected. The damage and the pushes are the game's events; a hit that
/// the target's damage cooldown refuses still counts as landed here.
fn stab_attack(
    m: &mut Mob,
    target: Who,
    base_damage: f32,
    (deals_damage, deals_knockback, dismounts): (bool, bool, bool),
    w: &MobWorld,
    events: &mut Vec<MobEvent>,
) -> bool {
    let species = match target {
        Who::Mob(id) => w.seen(id).map(|s| s.species),
        Who::Player => None,
    };
    // `EnchantmentHelper.modifyDamage`.
    let damage = base_damage + gear::melee_bonus(m, species);
    let from = [m.position.x, m.position.y, m.position.z];
    // `weaponItem.getDamageSource(this)`: the spear's, for the hurt and the pushes.
    let source = Damage {
        kind: DamageKind::Spear,
        attacker: Some(Who::Mob(m.id)),
        from: Some(from),
    };
    let mut affected = deals_knockback;
    if deals_damage {
        match target {
            Who::Player => events.push(MobEvent::DamagePlayer {
                amount: damage,
                from,
                kind: "minecraft:spear",
            }),
            Who::Mob(id) => events.push(MobEvent::AttackMob {
                target: id,
                amount: damage,
                source,
            }),
        }
        affected = true;
        // `EnchantmentHelper.doPostAttackEffects`: Fire Aspect.
        let burn = gear::fire_aspect(m);
        if burn > 0 {
            events.push(MobEvent::Ignite {
                target,
                seconds: burn,
            });
        }
    }
    if deals_knockback {
        cause_extra_knockback(m, target, 0.4_f32, source, events);
        // `LivingEntity.getKnockback`: the Knockback enchantment, halved.
        let power = gear::knockback(m) as f32 / 2.0;
        cause_extra_knockback(m, target, power, source, events);
    }
    if dismounts && w.target_info(target).is_some_and(|t| t.passenger) {
        affected = true;
        events.push(MobEvent::StabDismount { target });
    }
    affected
}

/// `LivingEntity.causeExtraKnockback`: a push along the attacker's facing,
/// which slows the attacker.
fn cause_extra_knockback(
    m: &mut Mob,
    target: Who,
    power: f32,
    source: Damage,
    events: &mut Vec<MobEvent>,
) {
    if power <= 0.0 {
        return;
    }
    let to_rad = std::f32::consts::PI / 180.0;
    events.push(MobEvent::Knockback {
        target,
        power: f64::from(power),
        x: f64::from((m.yaw * to_rad).msin()),
        z: f64::from(-(m.yaw * to_rad).mcos()),
        source,
    });
    m.velocity = Vec3::new(m.velocity.x * 0.6, m.velocity.y, m.velocity.z * 0.6);
}

/// `Mob.chargeSpeedModifier` of the mount: a zombie horse runs 1.4 times as
/// fast with a charging rider, a camel husk 4 times.
fn charge_speed_modifier(m: &Mob, w: &MobWorld) -> f32 {
    match m.extra.mount.and_then(|id| w.seen(id)).map(|s| s.species) {
        Some(Species::ZombieHorse) => 1.4,
        Some(Species::CamelHusk) => 4.0,
        _ => 1.0,
    }
}

/// `Entity.isPassenger` as the 2 blocks it adds to the charge distances.
fn passenger_distance(m: &Mob) -> i32 {
    if m.extra.mount.is_some() { 2 } else { 0 }
}

/// `ableToAttack` of `SpearUseGoal` and the spear behaviours: a target and a
/// kinetic weapon in the main hand.
fn able_to_attack(m: &Mob, w: &MobWorld) -> bool {
    m.target_info(w).is_some() && main_hand_kinetic(m).is_some()
}

/// `SpearUseGoal.SpearUseState`: where a charge is.
#[derive(Clone, Debug)]
struct SpearUseState {
    engage_time: i32,
    fleeing_time: i32,
    away_pos: Option<[f64; 3]>,
    done: bool,
}
impl SpearUseState {
    /// `SpearUseState.<init>`: not engaged and not fleeing yet.
    fn new() -> Self {
        Self {
            engage_time: -1,
            fleeing_time: -1,
            away_pos: None,
            done: false,
        }
    }
    /// `SpearUseState.notEngagedYet`.
    fn not_engaged_yet(&self) -> bool {
        self.engage_time < 0
    }
    /// `SpearUseState.startEngagement`: the spear stays down for `spear_down_time` goal ticks.
    fn start_engagement(&mut self, spear_down_time: i32) {
        self.engage_time = spear_down_time;
    }
    /// `SpearUseState.tickAndCheckEngagement`: whether the time ran out.
    fn tick_and_check_engagement(&mut self) -> bool {
        if self.engage_time > 0 {
            self.engage_time -= 1;
            if self.engage_time == 0 {
                return true;
            }
        }
        false
    }
    /// `SpearUseState.tickAndCheckFleeing`: whether it fled for too long.
    fn tick_and_check_fleeing(&mut self) -> bool {
        if self.fleeing_time > 0 {
            self.fleeing_time += 1;
            if self.fleeing_time > SpearUse::MAX_FLEEING_TIME {
                self.done = true;
                return true;
            }
        }
        false
    }
}

/// `SpearUseGoal`: with a spear in hand and a target, walk up to the
/// approach distance, lower the spear and charge until its use time ends,
/// back off 9 to 11 blocks and start over.
#[derive(Clone, Debug)]
pub struct SpearUse {
    speed_modifier_when_charging: f64,
    speed_modifier_when_repositioning: f64,
    approach_distance_sq: f32,
    target_in_range_radius_sq: f32,
    state: Option<SpearUseState>,
}
impl SpearUse {
    /// `SpearUseGoal.MAX_FLEEING_TIME`: `reducedTickDelay(100)`.
    const MAX_FLEEING_TIME: i32 = 50;

    pub fn new(
        speed_modifier_when_charging: f64,
        speed_modifier_when_repositioning: f64,
        approach_distance: f32,
        target_in_range_radius: f32,
    ) -> Self {
        Self {
            speed_modifier_when_charging,
            speed_modifier_when_repositioning,
            approach_distance_sq: approach_distance * approach_distance,
            target_in_range_radius_sq: target_in_range_radius * target_in_range_radius,
            state: None,
        }
    }

    /// `SpearUseGoal.getKineticWeaponUseDuration`.
    fn kinetic_weapon_use_duration(m: &Mob) -> i32 {
        reduced(main_hand_kinetic(m).map_or(0, |k| k.damage_use_duration()))
    }

    /// The part of `SpearUseGoal.tick` that works on the state.
    fn step(&self, state: &mut SpearUseState, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else { return };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        let target_dist_sqr = m.distance_sqr(t.position);
        let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
        let mount_distance = passenger_distance(m);
        let at = [t.position.x, t.eye_y, t.position.z];
        m.turn_to(at, 30.0, 30.0);
        m.look_at_speed(at, 30.0, 30.0);
        if state.not_engaged_yet() {
            if target_dist_sqr > f64::from(self.approach_distance_sq) {
                m.nav_move_to_entity(
                    cx.w,
                    [t.position.x, t.position.y, t.position.z],
                    1,
                    speed_modifier * self.speed_modifier_when_repositioning,
                );
                return;
            }
            state.start_engagement(Self::kinetic_weapon_use_duration(m));
            start_using_item(m);
        }
        if state.tick_and_check_engagement() {
            stop_using_item(m);
            let distance = target_dist_sqr.sqrt();
            state.away_pos = m.land_random_pos_away_between(
                cx.w,
                0.0_f64.max(f64::from(9 + mount_distance) - distance),
                1.0_f64.max(f64::from(11 + mount_distance) - distance),
                7,
                [t.position.x, t.position.y, t.position.z],
            );
            state.fleeing_time = 1;
        }
        if !state.tick_and_check_fleeing() {
            if let Some(away) = state.away_pos {
                m.nav_move_to(
                    cx.w,
                    away,
                    speed_modifier * self.speed_modifier_when_repositioning,
                );
                if m.ai.nav.is_done() {
                    if state.fleeing_time > 0 {
                        state.done = true;
                        return;
                    }
                    state.away_pos = None;
                }
            } else {
                m.nav_move_to_entity(
                    cx.w,
                    [t.position.x, t.position.y, t.position.z],
                    1,
                    speed_modifier * self.speed_modifier_when_charging,
                );
                if target_dist_sqr < f64::from(self.target_in_range_radius_sq) || m.ai.nav.is_done()
                {
                    let distance = target_dist_sqr.sqrt();
                    state.away_pos = m.land_random_pos_away_between(
                        cx.w,
                        f64::from(6 + mount_distance) - distance,
                        f64::from(7 + mount_distance) - distance,
                        7,
                        [t.position.x, t.position.y, t.position.z],
                    );
                }
            }
        }
    }
}
impl Goal for SpearUse {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    /// `SpearUseGoal.canUse`: able to attack and not using an item.
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        able_to_attack(m, cx.w) && !is_using_item(m)
    }
    /// `SpearUseGoal.canContinueToUse`.
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.state.as_ref().is_some_and(|s| !s.done) && able_to_attack(m, cx.w)
    }
    /// `SpearUseGoal.start`: aggressive, with a new state.
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.aggressive = true;
        self.state = Some(SpearUseState::new());
    }
    /// `SpearUseGoal.stop`: the navigation stops, the spear goes down.
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        m.ai.aggressive = false;
        self.state = None;
        stop_using_item(m);
    }
    /// `SpearUseGoal.tick`.
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(mut state) = self.state.take() {
            self.step(&mut state, m, cx);
            self.state = Some(state);
        }
    }
}

/// `SpearAttack.SpearStatus`: where a piglin is in its charge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpearStatus {
    Approach,
    Charging,
    Retreat,
}

/// The spear memories of a piglin's brain (`SPEAR_STATUS` and the like). None
/// of them is saved: they have no codec.
#[derive(Clone, Debug, Default)]
pub struct Memories {
    /// `SPEAR_STATUS`.
    pub status: Option<SpearStatus>,
    /// `SPEAR_ENGAGE_TIME`: ticks of the charge left.
    pub engage_time: Option<i32>,
    /// `SPEAR_CHARGE_POSITION`: where the charge goes to reposition.
    pub charge_position: Option<[f64; 3]>,
    /// `SPEAR_FLEEING_TIME`: ticks spent backing off.
    pub fleeing_time: Option<i32>,
    /// `SPEAR_FLEEING_POSITION`: where it backs off to.
    pub fleeing_position: Option<[f64; 3]>,
}

/// A piglin's `LOOK_TARGET` of an `EntityTracker(target, true)`: its eyes
/// follow the target's eyes.
fn look_at_target(m: &mut Mob, eye: [f64; 3]) {
    m.look_at(eye);
}

/// `SpearApproach.checkExtraStartConditions`: able to attack and not using an
/// item. The behaviour also needs `SPEAR_STATUS` to be absent.
fn approach_check_extra_start_conditions(m: &Mob, w: &MobWorld) -> bool {
    able_to_attack(m, w) && !is_using_item(m)
}

/// `SpearApproach.start`: aggressive, and the status is APPROACH.
fn approach_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.status = Some(SpearStatus::Approach);
}

/// `SpearApproach.canStillUse`: able to attack and still farther from the
/// target than the approach distance (`farEnough`).
fn approach_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    let Some(t) = m.target_info(w) else {
        return false;
    };
    able_to_attack(m, w)
        && m.distance_sqr(t.position) > f64::from(PiglinSpear::APPROACH_DISTANCE_SQ)
}

/// `SpearApproach.tick`: look at the target and walk up to it.
fn approach_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    m.nav_move_to_entity(
        cx.w,
        [t.position.x, t.position.y, t.position.z],
        1,
        speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
    );
}

/// `SpearApproach.stop`: the walk ends and the charge is next.
fn approach_stop(m: &mut Mob) {
    m.nav_stop();
    m.extra.piglin.spear.status = Some(SpearStatus::Charging);
}

/// `SpearAttack.checkExtraStartConditions`: the status is CHARGING, able to
/// attack and not using an item.
fn attack_check_extra_start_conditions(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.status.unwrap_or(SpearStatus::Approach) == SpearStatus::Charging
        && able_to_attack(m, w)
        && !is_using_item(m)
}

/// `SpearAttack.getKineticWeaponUseDuration`.
fn attack_kinetic_weapon_use_duration(m: &Mob) -> i32 {
    main_hand_kinetic(m).map_or(0, |k| k.damage_use_duration())
}

/// `SpearAttack.start`: aggressive, the charge lasts the weapon's use time,
/// and the spear goes up.
fn attack_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.engage_time = Some(attack_kinetic_weapon_use_duration(m));
    m.extra.piglin.spear.charge_position = None;
    start_using_item(m);
}

/// `SpearAttack.canStillUse`: time left to charge and able to attack.
fn attack_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.engage_time.unwrap_or(0) > 0 && able_to_attack(m, w)
}

/// `SpearAttack.tick`: look at the target, count the charge down and keep
/// moving: run at the target, and once within 2 blocks of it (or at the end
/// of the path) pick a spot 6 to 7 blocks past it to reposition to, then run
/// at the target again from there.
fn attack_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let target_dist_sqr = m.distance_sqr(t.position);
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    let mount_distance = passenger_distance(m);
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    let left = m.extra.piglin.spear.engage_time.unwrap_or(0) - 1;
    m.extra.piglin.spear.engage_time = Some(left);
    if let Some(away) = m.extra.piglin.spear.charge_position {
        m.nav_move_to(
            cx.w,
            away,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
        );
        if m.ai.nav.is_done() {
            m.extra.piglin.spear.charge_position = None;
        }
    } else {
        m.nav_move_to_entity(
            cx.w,
            [t.position.x, t.position.y, t.position.z],
            1,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_CHARGING,
        );
        if target_dist_sqr < f64::from(PiglinSpear::TARGET_IN_RANGE_RADIUS_SQ) || m.ai.nav.is_done()
        {
            let distance = target_dist_sqr.sqrt();
            m.extra.piglin.spear.charge_position = m.land_random_pos_away_between(
                cx.w,
                f64::from(6 + mount_distance) - distance,
                f64::from(7 + mount_distance) - distance,
                7,
                [t.position.x, t.position.y, t.position.z],
            );
        }
    }
}

/// `SpearAttack.stop`: the walk ends, the spear goes down and the retreat is next.
fn attack_stop(m: &mut Mob) {
    m.nav_stop();
    stop_using_item(m);
    m.extra.piglin.spear.charge_position = None;
    m.extra.piglin.spear.engage_time = None;
    m.extra.piglin.spear.status = Some(SpearStatus::Retreat);
}

/// `SpearRetreat.checkExtraStartConditions`: able to attack, not using an
/// item, the status RETREAT and a spot 9 to 11 blocks from the target to back
/// off to (2 more on a mount), which becomes `SPEAR_FLEEING_POSITION`.
fn retreat_check_extra_start_conditions(m: &mut Mob, cx: &mut Cx) -> bool {
    if !able_to_attack(m, cx.w) || is_using_item(m) {
        return false;
    }
    if m.extra.piglin.spear.status.unwrap_or(SpearStatus::Approach) != SpearStatus::Retreat {
        return false;
    }
    let Some(t) = m.target_info(cx.w) else {
        return false;
    };
    let mount_distance = passenger_distance(m);
    let distance = m.distance_sqr(t.position).sqrt();
    let Some(away) = m.land_random_pos_away_between(
        cx.w,
        0.0_f64.max(f64::from(9 + mount_distance) - distance),
        1.0_f64.max(f64::from(11 + mount_distance) - distance),
        7,
        [t.position.x, t.position.y, t.position.z],
    ) else {
        return false;
    };
    m.extra.piglin.spear.fleeing_position = Some(away);
    true
}

/// `SpearRetreat.start`: aggressive, with no time spent backing off yet.
fn retreat_start(m: &mut Mob) {
    m.ai.aggressive = true;
    m.extra.piglin.spear.fleeing_time = Some(0);
}

/// `SpearRetreat.canStillUse`: under 100 ticks backing off, with a spot to
/// back off to, a walk going on and able to attack.
fn retreat_can_still_use(m: &Mob, w: &MobWorld) -> bool {
    m.extra.piglin.spear.fleeing_time.unwrap_or(100) < 100
        && m.extra.piglin.spear.fleeing_position.is_some()
        && !m.ai.nav.is_done()
        && able_to_attack(m, w)
}

/// `SpearRetreat.tick`: look at the target, count the time and walk to the spot.
fn retreat_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(t) = m.target_info(cx.w) else { return };
    let speed_modifier = f64::from(charge_speed_modifier(m, cx.w));
    look_at_target(m, [t.position.x, t.eye_y, t.position.z]);
    let time = m.extra.piglin.spear.fleeing_time.unwrap_or(0) + 1;
    m.extra.piglin.spear.fleeing_time = Some(time);
    if let Some(away) = m.extra.piglin.spear.fleeing_position {
        m.nav_move_to(
            cx.w,
            away,
            speed_modifier * PiglinSpear::SPEED_MODIFIER_WHEN_REPOSITIONING,
        );
    }
}

/// `SpearRetreat.stop`: the walk ends, the spear is down and the charge starts over.
fn retreat_stop(m: &mut Mob) {
    m.nav_stop();
    m.ai.aggressive = false;
    stop_using_item(m);
    m.extra.piglin.spear.fleeing_time = None;
    m.extra.piglin.spear.fleeing_position = None;
    m.extra.piglin.spear.status = None;
}

/// The spear behaviours of a piglin's fight activity, `SpearApproach`,
/// `SpearAttack` and `SpearRetreat`: each starts on the `SPEAR_STATUS` the
/// one before it leaves. The goal runs them in the order of `Brain.tick`:
/// the ones not running try to start, then the running ones tick or stop.
/// Fight is the activity while the piglin has a target; a piglin with a
/// spear never melees (`NoMeleeWithSpear`).
#[derive(Clone, Debug)]
pub struct PiglinSpear {
    approach: Timed,
    attack: Timed,
    retreat: Timed,
}
impl PiglinSpear {
    /// `new SpearApproach(1.0, 10.0F)`: the squared approach distance.
    const APPROACH_DISTANCE_SQ: f32 = 10.0 * 10.0;
    /// `new SpearAttack(1.0, 1.0, 2.0F)`: the speed charging and the squared
    /// radius around the target that ends the run at it.
    const SPEED_MODIFIER_WHEN_CHARGING: f64 = 1.0;
    const TARGET_IN_RANGE_RADIUS_SQ: f32 = 2.0 * 2.0;
    /// The `speedModifierWhenRepositioning` of the three behaviours.
    const SPEED_MODIFIER_WHEN_REPOSITIONING: f64 = 1.0;

    pub fn new() -> Self {
        Self {
            // `Behavior(Map)`: a duration of 60 ticks (which `timedOut` ignores).
            approach: Timed::new(60, 60),
            attack: Timed::new(60, 60),
            // `super(Map, 100)`: a timeout of 100 ticks.
            retreat: Timed::new(100, 100),
        }
    }

    /// Whether the piglin is fighting (it has a target) with a kinetic weapon.
    fn fighting_with_spear(m: &Mob) -> bool {
        m.ai.target.is_some() && main_hand_kinetic(m).is_some()
    }
}
impl Default for PiglinSpear {
    fn default() -> Self {
        Self::new()
    }
}
impl Goal for PiglinSpear {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        Self::fighting_with_spear(m)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        Self::fighting_with_spear(m)
    }
    fn every_tick(&self) -> bool {
        true
    }
    /// `Behavior.doStop` of the behaviours still running when the fight ends.
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        if self.approach.running {
            self.approach.running = false;
            approach_stop(m);
        }
        if self.attack.running {
            self.attack.running = false;
            attack_stop(m);
        }
        if self.retreat.running {
            self.retreat.running = false;
            retreat_stop(m);
        }
    }
    /// `Brain.tick`'s `startEachNonRunningBehavior`, then `tickEachRunningBehavior`.
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let now = cx.w.game_time;
        if !self.approach.running
            && m.extra.piglin.spear.status.is_none()
            && approach_check_extra_start_conditions(m, cx.w)
        {
            self.approach.begin(cx.lr, now);
            approach_start(m);
        }
        if !self.attack.running
            && m.extra.piglin.spear.status.is_some()
            && attack_check_extra_start_conditions(m, cx.w)
        {
            self.attack.begin(cx.lr, now);
            attack_start(m);
        }
        if !self.retreat.running
            && m.extra.piglin.spear.status.is_some()
            && retreat_check_extra_start_conditions(m, cx)
        {
            self.retreat.begin(cx.lr, now);
            retreat_start(m);
        }
        if self.approach.running {
            if approach_can_still_use(m, cx.w) {
                approach_tick(m, cx);
            } else {
                self.approach.running = false;
                approach_stop(m);
            }
        }
        if self.attack.running {
            if attack_can_still_use(m, cx.w) {
                attack_tick(m, cx);
            } else {
                self.attack.running = false;
                attack_stop(m);
            }
        }
        if self.retreat.running {
            if !self.retreat.timed_out(now) && retreat_can_still_use(m, cx.w) {
                retreat_tick(m, cx);
            } else {
                self.retreat.running = false;
                retreat_stop(m);
            }
        }
    }
}

/// `MeleeAttack.isHoldingUsableNonMeleeWeapon`: a piglin holding a kinetic
/// weapon (`canUseNonMeleeWeapon`) never melees; its spear behaviours attack.
#[derive(Clone, Debug)]
pub struct NoMeleeWithSpear<G>(pub G);
/// A kinetic weapon or a crossbow in the main hand: the melee goal stays off.
pub(super) fn holds_non_melee_weapon(m: &Mob) -> bool {
    main_hand_kinetic(m).is_some() || m.extra.main_hand.as_deref() == Some("minecraft:crossbow")
}
impl<G: Goal + Clone + 'static> Goal for NoMeleeWithSpear<G> {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn name(&self) -> &'static str {
        self.0.name()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !holds_non_melee_weapon(m) && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !holds_non_melee_weapon(m) && self.0.can_continue(m, cx)
    }
    fn interruptable(&self) -> bool {
        self.0.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
