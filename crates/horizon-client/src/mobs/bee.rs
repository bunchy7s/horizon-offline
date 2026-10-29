//! Bees (`Bee`): flying about (`FlyingMoveControl`, `BeeWanderGoal`), finding
//! flowers and hovering over them until they carry nectar
//! (`BeePollinateGoal`), `NeutralMob` anger at whoever hurts them, stinging
//! once and dying of it, and the hive and flower bookkeeping: finding a hive
//! with room nearby (`BeeLocateHiveGoal`), flying home at night, in the rain
//! or with nectar (`BeeGoToHiveGoal`) and going in (`BeeEnterHiveGoal`, done
//! by the host: [`MobEvent::EnterHive`]); the hive lets them out
//! (`crate::beehive`).
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    control::{LookKind, MoveKind},
    goals::{
        Breed, Cx, Float, FollowParent, Goal, HurtByTarget, MOVE, MeleeAttack, NearestTarget,
        Tempt, goal_clone, reduced,
    },
    nav::NavKind,
    path::PathType,
};
use crate::beehive::BeehiveExt;
use horizon_core::random::{LegacyRandom, RandomSource};
use std::collections::HashMap;

const FLAG_ROLL: u8 = 2;
const FLAG_HAS_STUNG: u8 = 4;
const FLAG_HAS_NECTAR: u8 = 8;

/// The bee's own fields (`Bee`'s instance state and its synced flags).
#[derive(Clone, Debug)]
pub struct Bee {
    /// `DATA_FLAGS_ID`: rolling, has stung, has nectar.
    pub flags: u8,
    /// `DATA_ANGER_END_TIME`, -1 when calm.
    pub anger_end: i64,
    /// `persistentAngerTarget`.
    pub angry_at: Option<Who>,
    pub roll: f32,
    pub roll_o: f32,
    pub time_since_sting: i32,
    pub ticks_without_nectar: i32,
    pub stay_out_of_hive: i32,
    pub crops_grown: i32,
    /// `remainingCooldownBeforeLocatingNewHive` and `...NewFlower`.
    pub hive_cooldown: i32,
    pub flower_cooldown: i32,
    pub flower: Option<[i32; 3]>,
    pub hive: Option<[i32; 3]>,
    pub under_water: i32,
    /// `BeePollinateGoal.pollinating`, read by the navigation and look control.
    pub pollinating: bool,
    /// The cooldowns `ValidateHiveGoal` and `ValidateFlowerGoal` draw when built.
    pub validate_hive_cooldown: i32,
    pub validate_flower_cooldown: i32,
    /// `BeeGoToHiveGoal.blacklistedTargets`: the last three hives it gave up on.
    pub hive_blacklist: Vec<[i32; 3]>,
}
impl Default for Bee {
    fn default() -> Self {
        Self {
            flags: 0,
            anger_end: -1,
            angry_at: None,
            roll: 0.0,
            roll_o: 0.0,
            time_since_sting: 0,
            ticks_without_nectar: 0,
            stay_out_of_hive: 0,
            crops_grown: 0,
            hive_cooldown: 0,
            flower_cooldown: 0,
            flower: None,
            hive: None,
            under_water: 0,
            pollinating: false,
            validate_hive_cooldown: 0,
            validate_flower_cooldown: 0,
            hive_blacklist: Vec::new(),
        }
    }
}

fn st(m: &mut Mob) -> &mut Bee {
    m.extra.bee.get_or_insert_with(Default::default)
}
pub fn get(m: &Mob) -> Option<&Bee> {
    m.extra.bee.as_deref()
}
fn flag(m: &Mob, f: u8) -> bool {
    get(m).is_some_and(|b| b.flags & f != 0)
}
fn set_flag(m: &mut Mob, f: u8, value: bool) {
    let b = st(m);
    if value {
        b.flags |= f;
    } else {
        b.flags &= !f;
    }
}
pub fn has_nectar(m: &Mob) -> bool {
    flag(m, FLAG_HAS_NECTAR)
}
pub fn has_stung(m: &Mob) -> bool {
    flag(m, FLAG_HAS_STUNG)
}
pub fn rolling(m: &Mob) -> bool {
    flag(m, FLAG_ROLL)
}
/// A client's bee from the server's `DATA_FLAGS_ID` and `DATA_ANGER_END_TIME`
/// (the texture and the roll read them).
pub fn set_synced(m: &mut Mob, flags: Option<u8>, anger_end: Option<i64>) {
    let b = st(m);
    if let Some(flags) = flags {
        b.flags = flags;
    }
    if let Some(anger_end) = anger_end {
        b.anger_end = anger_end;
    }
}
/// `Bee.setHasNectar`: nectar resets the time without it.
pub fn set_has_nectar(m: &mut Mob, value: bool) {
    if value {
        st(m).ticks_without_nectar = 0;
    }
    set_flag(m, FLAG_HAS_NECTAR, value);
}
pub fn set_has_stung(m: &mut Mob, value: bool) {
    set_flag(m, FLAG_HAS_STUNG, value);
}
pub fn pollinating(m: &Mob) -> bool {
    m.species() == Species::Bee && get(m).is_some_and(|b| b.pollinating)
}
/// `BeeLookControl.resetXRotOnTick`: the pitch stays put while pollinating.
fn set_pollinating(m: &mut Mob, value: bool) {
    st(m).pollinating = value;
    m.ai.look.kind = if value {
        LookKind::Fixed
    } else {
        LookKind::Default
    };
}
/// `Mth.nextInt(random, min, max)`.
fn next_int(m: &mut Mob, min: i32, max: i32) -> i32 {
    m.random.next_int_between_inclusive(min, max).unwrap_or(min)
}

/// `Bee(EntityType, Level)` with `registerGoals` and `createNavigation`.
pub fn register(m: &mut Mob) {
    m.ai.mov.kind = MoveKind::Flying {
        max_turn: 20.0,
        hovers: true,
    };
    m.ai.nav.kind = NavKind::Flying;
    m.ai.nav.can_open_doors = false;
    m.set_required_path_length(48.0);
    m.set_malus(PathType::Fire, -1.0);
    m.set_malus(PathType::Water, -1.0);
    m.set_malus(PathType::WaterBorder, 16.0);
    m.set_malus(PathType::Cocoa, -1.0);
    m.set_malus(PathType::Fence, -1.0);
    let tempt = m.kind.tempt.clone();
    let g = &mut m.ai.goals;
    g.add(0, BeeAttack(MeleeAttack::new(f64::from(1.4_f32), true)));
    g.add(1, EnterHive);
    g.add(2, Breed::new(1.0));
    g.add(3, Tempt::new(1.25, tempt, false));
    g.add(3, ValidateHive { last: -1 });
    g.add(3, ValidateFlower { last: -1 });
    g.add(4, Pollinate::default());
    g.add(5, FollowParent::new(1.25));
    g.add(5, LocateHive);
    g.add(5, GoToHive::default());
    g.add(6, GoToKnownFlower::default());
    g.add(7, GrowCrop);
    g.add(8, Wander);
    g.add(9, Float);
    // FloatGoal's constructor lets the navigation float.
    m.ai.nav.can_float = true;
    let t = &mut m.ai.targets;
    t.add(1, HurtByTarget::new().alert().bee());
    t.add(2, BecomeAngryTarget::new());
    // ResetUniversalAngerTargetGoal only runs with the universalAnger rule.
    m.extra.bee = Some(Box::default());
    draw_cooldowns(m);
}
/// The random draws of the constructor: the validate goals' cooldowns (built
/// in `registerGoals`), then the field initialiser of the flower cooldown.
/// The parity probe draws them again once the random is seeded.
pub fn draw_cooldowns(m: &mut Mob) {
    let hive = next_int(m, 20, 40);
    let flower = next_int(m, 20, 40);
    let locate = next_int(m, 20, 60);
    let b = st(m);
    b.validate_hive_cooldown = hive;
    b.validate_flower_cooldown = flower;
    b.flower_cooldown = locate;
}

// --- NeutralMob -------------------------------------------------------------

/// `NeutralMob.isAngry`.
pub fn angry(m: &Mob, w: &MobWorld) -> bool {
    get(m).is_some_and(|b| b.anger_end > 0 && b.anger_end - w.game_time > 0)
}
/// `NeutralMob.stopBeingAngry`.
pub fn stop_being_angry(m: &mut Mob) {
    m.ai.last_hurt_by = None;
    m.ai.last_hurt_by_time = m.age as i32;
    m.ai.target = None;
    let b = st(m);
    b.angry_at = None;
    b.anger_end = -1;
}
/// `Bee.startPersistentAngerTimer`: 20 to 39 seconds.
fn start_anger_timer(m: &mut Mob, w: &MobWorld) {
    let ticks = next_int(m, 400, 780);
    st(m).anger_end = w.game_time + i64::from(ticks);
}
fn valid_player_target(w: &MobWorld) -> bool {
    w.player
        .as_ref()
        .is_some_and(|p| !p.creative && !p.spectator)
}
/// `NeutralMob.updatePersistentAnger(level, false)`.
fn update_persistent_anger(m: &mut Mob, w: &MobWorld) {
    let previous = m.ai.target;
    let angry_at = get(m).and_then(|b| b.angry_at);
    let dead_mob = |who: Who| match who {
        Who::Mob(id) => w.seen(id).is_none_or(|s| !s.alive),
        Who::Player => false,
    };
    if let Some(p) = previous
        && dead_mob(p)
        && angry_at == Some(p)
    {
        stop_being_angry(m);
        return;
    }
    if let Some(target) = m.ai.target
        && angry_at != Some(target)
    {
        st(m).angry_at = Some(target);
        start_anger_timer(m, w);
    }
    if angry_at.is_some() && !angry(m, w) {
        stop_being_angry(m);
    }
    if angry_at == Some(Who::Player) && !valid_player_target(w) {
        stop_being_angry(m);
    }
}
/// `beeCanTarget`: angry and still armed.
fn can_target(m: &Mob, w: &MobWorld) -> bool {
    angry(m, w) && !has_stung(m)
}

// --- the bee's ticks ----------------------------------------------------------

/// `Bee.tick` after `super.tick()`: nectar drips (positions drawn from the
/// level's random even on the server), then the roll.
pub fn tick(m: &mut Mob, level_random: &mut LegacyRandom) {
    if has_nectar(m) && get(m).is_some_and(|b| b.crops_grown < 10) && m.random.next_float() < 0.05 {
        let mut i = 0;
        while i < m.random.next_int_bound(2).unwrap_or(0) + 1 {
            let _ = (level_random.next_double(), level_random.next_double());
            i += 1;
        }
    }
    let roll = rolling(m);
    let b = st(m);
    b.roll_o = b.roll;
    b.roll = if roll {
        (b.roll + 0.2).min(1.0)
    } else {
        (b.roll - 0.24).max(0.0)
    };
}
/// `Bee.aiStep` after `super.aiStep()`.
pub fn ai_step(m: &mut Mob, w: &MobWorld) {
    let b = st(m);
    for c in [
        &mut b.stay_out_of_hive,
        &mut b.hive_cooldown,
        &mut b.flower_cooldown,
    ] {
        if *c > 0 {
            *c -= 1;
        }
    }
    let close = m
        .target_info(w)
        .is_some_and(|t| m.distance_sqr(t.position) < 4.0);
    let roll = angry(m, w) && !has_stung(m) && close;
    set_flag(m, FLAG_ROLL, roll);
    if m.age.is_multiple_of(20) && !hive_valid(m, w) {
        st(m).hive = None;
    }
}
/// `Bee.customServerAiStep`: drowning, the sting's slow death, the time
/// without nectar and the anger.
pub fn server_ai_step(m: &mut Mob, cx: &mut Cx) {
    let stung = has_stung(m);
    let in_water = m.fluids.in_water;
    let b = st(m);
    b.under_water = if in_water { b.under_water + 1 } else { 0 };
    if b.under_water > 20 {
        m.damage(1.0, Damage::of(DamageKind::Drown));
    }
    if stung {
        let b = st(m);
        b.time_since_sting += 1;
        let t = b.time_since_sting;
        if t % 5 == 0
            && m.random
                .next_int_bound((1200 - t).clamp(1, 1200))
                .unwrap_or(1)
                == 0
        {
            let health = m.health;
            m.damage(health, Damage::of(DamageKind::Generic));
        }
    }
    if !has_nectar(m) {
        st(m).ticks_without_nectar += 1;
    }
    update_persistent_anger(m, cx.w);
}
/// `Bee.hurtServer`: a hit ends pollinating.
pub fn hurt(m: &mut Mob) {
    set_pollinating(m, false);
}
/// `Bee.doHurtTarget`: a sting for the attack damage, poison on normal (10 s)
/// and hard (18 s), then the bee has stung and calms down.
pub fn sting(m: &mut Mob, who: Who, cx: &mut Cx) {
    let from = [m.position.x, m.position.y, m.position.z];
    let amount = (m.attrs.attack_damage as i32) as f32;
    match who {
        Who::Player => {
            // Bee.doHurtTarget: the poison only when the sting hurt, and
            // only on normal (10 s) and hard (18 s).
            let seconds = match cx.w.difficulty.id {
                2 => 10,
                3 => 18,
                _ => 0,
            };
            if seconds > 0 {
                cx.events.push(MobEvent::AttackPlayerWithEffect {
                    amount,
                    from,
                    effect: "minecraft:poison",
                    ticks: seconds * 20,
                    amplifier: 0,
                });
            } else {
                cx.events.push(MobEvent::AttackPlayer(amount, from));
            }
        }
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source: Damage {
                kind: DamageKind::Sting,
                attacker: Some(Who::Mob(m.id)),
                from: Some(from),
            },
        }),
    }
    set_has_stung(m, true);
    stop_being_angry(m);
    if !m.silent {
        cx.events
            .push(MobEvent::Sound("entity.bee.sting", from, 1.0, 1.0));
    }
}

// --- places -------------------------------------------------------------------

fn block_pos(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// `Bee.closerThan`: `Vec3i.closerThan` from the bee's block.
fn closer_than(m: &Mob, p: [i32; 3], dist: i32) -> bool {
    let b = block_pos(m);
    let d = |i: usize| f64::from(p[i] - b[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2) < f64::from(dist * dist)
}
fn too_far(m: &Mob, p: [i32; 3]) -> bool {
    !closer_than(m, p, 48)
}
/// `Bee.getBeehiveBlockEntity`: the hive's block entity, unless too far.
fn hive_entity<'a>(m: &Mob, w: &'a MobWorld) -> Option<&'a crate::beehive::Beehive> {
    let h = get(m)?.hive?;
    if too_far(m, h) {
        return None;
    }
    match w.world.block_entities.get(&(h[0], h[1], h[2])) {
        Some(crate::containers::BlockEntity::Beehive(hive)) => Some(hive),
        _ => None,
    }
}
fn hive_valid(m: &Mob, w: &MobWorld) -> bool {
    hive_entity(m, w).is_some()
}
/// `BeehiveBlockEntity.isFireNearby`: fire (not soul fire) next to the hive.
fn hive_near_fire(m: &Mob, w: &MobWorld) -> bool {
    let Some(h) = get(m).and_then(|b| b.hive).filter(|_| hive_valid(m, w)) else {
        return false;
    };
    (-1..=1).any(|dx| {
        (-1..=1).any(|dy| {
            (-1..=1).any(|dz| w.block_name(h[0] + dx, h[1] + dy, h[2] + dz) == "minecraft:fire")
        })
    })
}
/// `EnvironmentAttributes.BEES_STAY_IN_HIVE`: the overworld's night, or rain.
pub fn bees_stay_in_hive(w: &MobWorld) -> bool {
    let night = !w.nether && !w.end && (12542..23460).contains(&w.day_time.rem_euclid(24000));
    night || w.raining
}
fn tired_of_looking_for_nectar(m: &Mob) -> bool {
    get(m).is_some_and(|b| b.ticks_without_nectar > 3600)
}
/// `Bee.wantsToEnterHive`.
fn wants_to_enter_hive(m: &Mob, w: &MobWorld) -> bool {
    let Some(b) = get(m) else { return false };
    if b.stay_out_of_hive > 0 || b.pollinating || has_stung(m) || m.ai.target.is_some() {
        return false;
    }
    (has_nectar(m) || tired_of_looking_for_nectar(m) || bees_stay_in_hive(w))
        && !hive_near_fire(m, w)
}
/// `Bee.setHivePos`.
pub fn set_hive(m: &mut Mob, hive: Option<[i32; 3]>) {
    st(m).hive = hive;
}
/// `Bee.setSavedFlowerPos`.
pub fn set_flower(m: &mut Mob, flower: Option<[i32; 3]>) {
    st(m).flower = flower;
}
/// `Bee.dropOffNectar`.
pub fn drop_off_nectar(m: &mut Mob) {
    set_has_nectar(m, false);
    st(m).crops_grown = 0;
}
/// `Bee.setStayOutOfHiveCountdown`: the bee will not enter a hive for 400
/// ticks (the hive passes that when a sedated hive lets its bees out).
pub fn set_stay_out_of_hive_countdown(m: &mut Mob) {
    st(m).stay_out_of_hive = 400;
}
fn drop_hive(m: &mut Mob) {
    let b = st(m);
    b.hive = None;
    b.hive_cooldown = 200;
}
fn drop_flower(m: &mut Mob) {
    st(m).flower = None;
    let cooldown = next_int(m, 20, 60);
    st(m).flower_cooldown = cooldown;
}
/// `Bee.attractsBees`.
pub fn attracts_bees(w: &MobWorld, p: [i32; 3]) -> bool {
    let name = w.block_name(p[0], p[1], p[2]);
    if !w.has_tag("minecraft:bee_attractive", name) {
        return false;
    }
    if w.prop(p[0], p[1], p[2], "waterlogged") == Some("true") {
        return false;
    }
    name != "minecraft:sunflower" || w.prop(p[0], p[1], p[2], "half") == Some("upper")
}
/// `BlockPos.withinBoxByManhattanDistance`: nearest first by Manhattan
/// distance, then x, y, and +z before -z.
fn manhattan_box(o: [i32; 3], rx: i32, ry: i32, rz: i32) -> Vec<[i32; 3]> {
    let mut out = Vec::new();
    for depth in 0..=rx + ry + rz {
        let max_x = rx.min(depth);
        for x in -max_x..=max_x {
            let max_y = ry.min(depth - x.abs());
            for y in -max_y..=max_y {
                let z = depth - x.abs() - y.abs();
                if z <= rz {
                    out.push([o[0] + x, o[1] + y, o[2] + z]);
                    if z != 0 {
                        out.push([o[0] + x, o[1] + y, o[2] - z]);
                    }
                }
            }
        }
    }
    out
}
/// `Bee.pathfindRandomlyTowards`.
fn pathfind_randomly_towards(m: &mut Mob, w: &MobWorld, p: [i32; 3]) {
    let target = [
        f64::from(p[0]) + 0.5,
        f64::from(p[1]),
        f64::from(p[2]) + 0.5,
    ];
    let here = block_pos(m);
    let y_delta = target[1] as i32 - here[1];
    let y_adjust = if y_delta > 2 {
        4
    } else if y_delta < -2 {
        -4
    } else {
        0
    };
    let dist = (p[0] - here[0]).abs() + (p[1] - here[1]).abs() + (p[2] - here[2]).abs();
    let (h, v) = if dist < 15 {
        (dist / 2, dist / 2)
    } else {
        (6, 8)
    };
    let max_rad = f64::from((std::f64::consts::PI / 10.0) as f32);
    if let Some(at) = m.air_random_pos_towards(w, h, v, y_adjust, target, max_rad) {
        m.ai.nav.max_visited_multiplier = 0.5;
        m.nav_move_to(w, at, 1.0);
    }
}

// --- goals ----------------------------------------------------------------------

/// `BaseBeeGoal`: none of these run while the bee is angry.
fn calm(m: &Mob, w: &MobWorld) -> bool {
    !angry(m, w)
}

/// `BeeAttackGoal`: a melee attack while angry and armed.
#[derive(Clone, Debug)]
struct BeeAttack(MeleeAttack);
impl Goal for BeeAttack {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeAttackGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx) && can_target(m, cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx) && can_target(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx);
    }
}

/// `BeeBecomeAngryTargetGoal`: the player it is angry at, while armed.
#[derive(Clone, Debug)]
struct BecomeAngryTarget(NearestTarget);
impl BecomeAngryTarget {
    fn new() -> Self {
        Self(
            NearestTarget::player(true)
                .interval(10)
                .gate(can_target)
                // NeutralMob.isAngryAt: the persistent anger target.
                .when(|m, _| get(m).is_some_and(|b| b.angry_at == Some(Who::Player))),
        )
    }
}
impl Goal for BecomeAngryTarget {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeBecomeAngryTargetGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        can_target(m, cx.w) && m.ai.target.is_some() && self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}

/// `BeeEnterHiveGoal`: into the hive beside it, or forget it when full.
#[derive(Clone, Debug)]
struct EnterHive;
impl Goal for EnterHive {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeEnterHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let close = get(m).and_then(|b| b.hive).is_some_and(|h| {
            let d = |i: usize, v: f64| f64::from(h[i]) + 0.5 - v;
            let (x, y, z) = (d(0, m.position.x), d(1, m.position.y), d(2, m.position.z));
            x * x + y * y + z * z < 4.0
        });
        let mut enter = false;
        if close && wants_to_enter_hive(m, cx.w) {
            match hive_entity(m, cx.w).map(|h| h.full()) {
                Some(false) => enter = true,
                Some(true) => st(m).hive = None,
                None => {}
            }
        }
        enter && calm(m, cx.w)
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(hive) = get(m).and_then(|b| b.hive).filter(|_| hive_valid(m, cx.w)) {
            cx.events.push(MobEvent::EnterHive { mob: m.id, hive });
        }
    }
}

/// `ValidateHiveGoal`: now and then, forget a hive that is gone.
#[derive(Clone, Debug)]
struct ValidateHive {
    last: i64,
}
impl Goal for ValidateHive {
    goal_clone!();
    fn name(&self) -> &'static str {
        "ValidateHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let cooldown = get(m).map_or(0, |b| b.validate_hive_cooldown);
        cx.w.game_time > self.last + i64::from(cooldown) && calm(m, cx.w)
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(h) = get(m).and_then(|b| b.hive)
            && cx.w.world.loaded(h[0], h[2])
            && !hive_valid(m, cx.w)
        {
            drop_hive(m);
        }
        self.last = cx.w.game_time;
    }
}

/// `ValidateFlowerGoal`: now and then, forget a flower that is gone.
#[derive(Clone, Debug)]
struct ValidateFlower {
    last: i64,
}
impl Goal for ValidateFlower {
    goal_clone!();
    fn name(&self) -> &'static str {
        "ValidateFlowerGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let cooldown = get(m).map_or(0, |b| b.validate_flower_cooldown);
        cx.w.game_time > self.last + i64::from(cooldown) && calm(m, cx.w)
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(f) = get(m).and_then(|b| b.flower)
            && cx.w.world.loaded(f[0], f[2])
            && !attracts_bees(cx.w, f)
        {
            drop_flower(m);
        }
        self.last = cx.w.game_time;
    }
}

/// `BeePollinateGoal`: fly to a flower within five blocks and hover about
/// over it; after 400 ticks there the bee carries nectar.
#[derive(Clone, Debug, Default)]
struct Pollinate {
    successful: i32,
    last_sound: i32,
    hover: Option<[f64; 3]>,
    ticks: i32,
    /// `unreachableFlowerCache`: flowers and the game time until which they
    /// count as out of reach.
    unreachable: HashMap<[i32; 3], i64>,
}
impl Pollinate {
    fn long_enough(&self) -> bool {
        self.successful > 400
    }
    /// `findNearbyFlower`: the first reachable flower, nearest first.
    fn find_flower(&mut self, m: &mut Mob, w: &MobWorld) -> Option<[i32; 3]> {
        let mut cache = HashMap::new();
        let mut found = None;
        for p in manhattan_box(block_pos(m), 5, 5, 5) {
            if !attracts_bees(w, p) {
                continue;
            }
            let until = self.unreachable.get(&p).copied().unwrap_or(i64::MIN);
            if w.game_time < until {
                cache.insert(p, until);
                continue;
            }
            if m.create_path_to(w, p, 1).is_some_and(|path| path.reached) {
                found = Some(p);
                break;
            }
            cache.insert(p, w.game_time + 600);
        }
        self.unreachable = cache;
        found
    }
    fn set_wanted(&self, m: &mut Mob) {
        if let Some(h) = self.hover {
            m.ai.mov.set_wanted(h[0], h[1], h[2], f64::from(0.35_f32));
        }
    }
    fn offset(m: &mut Mob) -> f64 {
        f64::from((m.random.next_float() * 2.0 - 1.0) * 0.333_333_34)
    }
}
impl Goal for Pollinate {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeePollinateGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let usable =
            if get(m).is_some_and(|b| b.flower_cooldown > 0) || has_nectar(m) || cx.w.raining {
                false
            } else if let Some(p) = self.find_flower(m, cx.w) {
                st(m).flower = Some(p);
                let at = [
                    f64::from(p[0]) + 0.5,
                    f64::from(p[1]) + 0.5,
                    f64::from(p[2]) + 0.5,
                ];
                m.nav_move_to(cx.w, at, f64::from(1.2_f32));
                true
            } else {
                let cooldown = next_int(m, 20, 60);
                st(m).flower_cooldown = cooldown;
                false
            };
        usable && calm(m, cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let go = if !pollinating(m) || get(m).is_none_or(|b| b.flower.is_none()) || cx.w.raining {
            false
        } else if self.long_enough() {
            m.random.next_float() < 0.2
        } else {
            true
        };
        go && calm(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.successful = 0;
        self.ticks = 0;
        self.last_sound = 0;
        set_pollinating(m, true);
        st(m).ticks_without_nectar = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        if self.long_enough() {
            set_has_nectar(m, true);
        }
        set_pollinating(m, false);
        m.nav_stop();
        st(m).flower_cooldown = 200;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, _cx: &mut Cx) {
        let Some(f) = get(m).and_then(|b| b.flower) else {
            return;
        };
        self.ticks += 1;
        if self.ticks > 600 {
            drop_flower(m);
            set_pollinating(m, false);
            st(m).flower_cooldown = 200;
            return;
        }
        let flower = [
            f64::from(f[0]) + 0.5,
            f64::from(f[1]) + f64::from(0.6_f32),
            f64::from(f[2]) + 0.5,
        ];
        if m.distance_to(flower) > 1.0 {
            self.hover = Some(flower);
            self.set_wanted(m);
            return;
        }
        let hover = *self.hover.get_or_insert(flower);
        let arrived = m.distance_to(hover) <= 0.1;
        if !arrived && self.ticks > 600 {
            drop_flower(m);
            return;
        }
        let mut set_wanted = true;
        if arrived {
            if m.random.next_int_bound(25).unwrap_or(1) == 0 {
                let x = flower[0] + Self::offset(m);
                let z = flower[2] + Self::offset(m);
                self.hover = Some([x, flower[1], z]);
                m.nav_stop();
            } else {
                set_wanted = false;
            }
            m.look_at(flower);
        }
        if set_wanted {
            self.set_wanted(m);
        }
        self.successful += 1;
        if m.random.next_float() < 0.05 && self.successful > self.last_sound + 60 {
            self.last_sound = self.successful;
            if !m.silent {
                let at = [m.position.x, m.position.y, m.position.z];
                m.outbox
                    .push(MobEvent::Sound("entity.bee.pollinate", at, 1.0, 1.0));
            }
        }
    }
}

/// `BeeLocateHiveGoal`: the nearest bee home with room within 20 blocks,
/// skipping the ones `BeeGoToHiveGoal` gave up on (unless that is all).
#[derive(Clone, Debug)]
struct LocateHive;
impl Goal for LocateHive {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeLocateHiveGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let b = get(m);
        let usable = b.is_some_and(|b| b.hive_cooldown == 0 && b.hive.is_none())
            && wants_to_enter_hive(m, cx.w);
        usable && calm(m, cx.w)
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        st(m).hive_cooldown = 200;
        let hives = hives_with_space(m, cx.w);
        let b = st(m);
        let pick = hives
            .iter()
            .find(|h| !b.hive_blacklist.contains(h))
            .copied();
        if pick.is_none() && !hives.is_empty() {
            b.hive_blacklist.clear();
        }
        b.hive = pick.or_else(|| hives.first().copied()).or(b.hive);
    }
}

/// `findNearbyHivesWithSpace`: the bee homes (`PoiTypeTags.BEE_HOME`)
/// within 20 blocks whose hive has room, nearest first.
fn hives_with_space(m: &Mob, w: &MobWorld) -> Vec<[i32; 3]> {
    let here = block_pos(m);
    let dist = |p: &[i32; 3]| {
        let d = |i: usize| i64::from(p[i] - here[i]);
        d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
    };
    let mut hives: Vec<[i32; 3]> = w
        .world
        .block_entities
        .iter()
        .filter_map(|(&(x, y, z), e)| match e {
            crate::containers::BlockEntity::Beehive(h) if !h.full() => Some([x, y, z]),
            _ => None,
        })
        .filter(|p| {
            dist(p) <= 20 * 20 && w.has_tag("minecraft:beehives", w.block_name(p[0], p[1], p[2]))
        })
        .collect();
    hives.sort_by_key(|p| (dist(p), *p));
    hives
}

/// `BeeGoToHiveGoal`: home when it wants to go in.
#[derive(Clone, Debug, Default)]
struct GoToHive {
    travelling: i32,
    last_path: Option<super::path::Path>,
    stuck: i32,
}
impl GoToHive {
    fn reached(m: &Mob, p: [i32; 3]) -> bool {
        closer_than(m, p, 2)
            || m.ai
                .nav
                .path
                .as_ref()
                .is_some_and(|path| path.target == p && path.reached && path.is_done())
    }
    fn drop_and_blacklist(&mut self, m: &mut Mob) {
        let b = st(m);
        if let Some(h) = b.hive {
            b.hive_blacklist.push(h);
            while b.hive_blacklist.len() > 3 {
                b.hive_blacklist.remove(0);
            }
        }
        drop_hive(m);
    }
    fn pathfind_directly(m: &mut Mob, w: &MobWorld, p: [i32; 3]) -> bool {
        let close_enough = if closer_than(m, p, 3) { 1 } else { 2 };
        m.ai.nav.max_visited_multiplier = 10.0;
        let path = m.create_path_to(w, p, close_enough);
        m.nav_move_to_path(path, 1.0);
        m.ai.nav.path.as_ref().is_some_and(|path| path.reached)
    }
}
impl Goal for GoToHive {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeGoToHiveGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let usable = get(m).and_then(|b| b.hive).is_some_and(|h| {
            !too_far(m, h)
                && m.ai.home.is_none()
                && wants_to_enter_hive(m, cx.w)
                && !Self::reached(m, h)
                && cx
                    .w
                    .has_tag("minecraft:beehives", cx.w.block_name(h[0], h[1], h[2]))
        });
        usable && calm(m, cx.w)
    }
    fn start(&mut self, _m: &mut Mob, _cx: &mut Cx) {
        self.travelling = 0;
        self.stuck = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.travelling = 0;
        self.stuck = 0;
        m.nav_stop();
        m.ai.nav.max_visited_multiplier = 1.0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(h) = get(m).and_then(|b| b.hive) else {
            return;
        };
        self.travelling += 1;
        if self.travelling > reduced(2400) {
            self.drop_and_blacklist(m);
        } else if !m.ai.nav.is_in_progress() {
            if !closer_than(m, h, 16) {
                if too_far(m, h) {
                    drop_hive(m);
                } else {
                    pathfind_randomly_towards(m, cx.w, h);
                }
            } else if !Self::pathfind_directly(m, cx.w, h) {
                self.drop_and_blacklist(m);
            } else if let Some(last) = &self.last_path
                && m.ai
                    .nav
                    .path
                    .as_ref()
                    .is_some_and(|p| p.same_as(Some(last)))
            {
                self.stuck += 1;
                if self.stuck > 60 {
                    drop_hive(m);
                    self.stuck = 0;
                }
            } else {
                self.last_path = m.ai.nav.path.clone();
            }
        }
    }
}

/// `BeeGoToKnownFlowerGoal`: back to the remembered flower after 600 ticks
/// without nectar.
#[derive(Clone, Debug, Default)]
struct GoToKnownFlower {
    travelling: i32,
}
impl Goal for GoToKnownFlower {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeGoToKnownFlowerGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let usable = get(m).is_some_and(|b| {
            b.flower.is_some_and(|f| {
                m.ai.home.is_none() && b.ticks_without_nectar > 600 && !closer_than(m, f, 2)
            })
        });
        usable && calm(m, cx.w)
    }
    fn start(&mut self, _m: &mut Mob, _cx: &mut Cx) {
        self.travelling = 0;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        self.travelling = 0;
        m.nav_stop();
        m.ai.nav.max_visited_multiplier = 1.0;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(f) = get(m).and_then(|b| b.flower) else {
            return;
        };
        self.travelling += 1;
        if self.travelling > reduced(2400) {
            drop_flower(m);
        } else if !m.ai.nav.is_in_progress() {
            if too_far(m, f) {
                drop_flower(m);
            } else {
                pathfind_randomly_towards(m, cx.w, f);
            }
        }
    }
}

/// `BeeGrowCropGoal`: a bee with nectar and a hive grows crops below it.
#[derive(Clone, Debug)]
struct GrowCrop;
impl Goal for GrowCrop {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeGrowCropGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // The 0.3 roll comes only after the crop count: it draws randomness.
        let usable = get(m).is_none_or(|b| b.crops_grown < 10)
            && m.random.next_float() >= 0.3
            && has_nectar(m)
            && hive_valid(m, cx.w);
        usable && calm(m, cx.w)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.random.next_int_bound(reduced(30)).unwrap_or(1) != 0 {
            return;
        }
        let here = block_pos(m);
        for i in 1..=2 {
            let p = [here[0], here[1] - i, here[2]];
            if let Some(state) = grown(cx.w, p) {
                cx.events.push(MobEvent::SetBlock { pos: p, state });
                st(m).crops_grown += 1;
            }
        }
    }
}
/// The state a bee grows a `bee_growables` block to, if it can grow.
fn grown(w: &MobWorld, p: [i32; 3]) -> Option<horizon_world::registry::BlockStateId> {
    let name = w.block_name(p[0], p[1], p[2]);
    if !w.has_tag("minecraft:bee_growables", name) {
        return None;
    }
    let state = w.registry.state(w.world.block(p[0], p[1], p[2]))?;
    let (key, max) = match name {
        "minecraft:beetroots" => ("age", 3),
        // TorchflowerCropBlock.getStateForAge(2): the flower itself.
        "minecraft:torchflower_crop" => {
            let age: i32 = state.properties.get("age")?.parse().ok()?;
            return if age + 1 >= 2 {
                w.registry.default_state("minecraft:torchflower")
            } else {
                let mut s = state.clone();
                s.properties.insert("age".into(), (age + 1).to_string());
                w.registry.id(&s)
            };
        }
        "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" => ("age", 7),
        "minecraft:melon_stem" | "minecraft:pumpkin_stem" => ("age", 7),
        "minecraft:sweet_berry_bush" => ("age", 3),
        // CaveVinesBlock.performBonemeal: berries.
        "minecraft:cave_vines" | "minecraft:cave_vines_plant" => {
            if state.properties.get("berries").map(String::as_str) == Some("true") {
                return None;
            }
            let mut s = state.clone();
            s.properties.insert("berries".into(), "true".into());
            return w.registry.id(&s);
        }
        _ => return None,
    };
    let age: i32 = state.properties.get(key)?.parse().ok()?;
    if age >= max {
        return None;
    }
    let mut s = state.clone();
    s.properties.insert(key.into(), (age + 1).to_string());
    w.registry.id(&s)
}

/// `BeeWanderGoal`: hover to a random spot ahead (or back towards a far hive).
#[derive(Clone, Debug)]
struct Wander;
impl Goal for Wander {
    goal_clone!();
    fn name(&self) -> &'static str {
        "BeeWanderGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.ai.nav.is_done() && m.random.next_int_bound(10).unwrap_or(1) == 0
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.ai.nav.is_in_progress()
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        // Back towards a valid hive farther than 24 blocks, else ahead.
        let far_hive = get(m)
            .and_then(|b| b.hive)
            .filter(|&h| hive_valid(m, cx.w) && !closer_than(m, h, 24));
        let dir = match far_hive {
            Some(h) => {
                let v = [
                    f64::from(h[0]) + 0.5 - m.position.x,
                    f64::from(h[1]) + 0.5 - m.position.y,
                    f64::from(h[2]) + 0.5 - m.position.z,
                ];
                // Vec3.normalize.
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if len < 1.0E-5 {
                    [0.0; 3]
                } else {
                    v.map(|c| c / len)
                }
            }
            None => m.view_vector(),
        };
        let max_rad = f64::from(std::f32::consts::FRAC_PI_2);
        let at = m
            .hover_random_pos(cx.w, 8, 7, dir[0], dir[2], max_rad, 3, 1)
            .or_else(|| m.air_and_water_random_pos(cx.w, 8, 4, -2, dir[0], dir[2]));
        if let Some(at) = at {
            let p = [
                at[0].floor() as i32,
                at[1].floor() as i32,
                at[2].floor() as i32,
            ];
            let path = m.create_path_to(cx.w, p, 1);
            m.nav_move_to_path(path, 1.0);
        }
    }
}

/// `FlyingPathNavigation.isStableDestination` for bees: anything above
/// something not air.
pub fn stable_destination(w: &MobWorld, p: [i32; 3]) -> bool {
    !w.facts.is_air(w.world.block(p[0], p[1] - 1, p[2]))
}
/// `Bee.addAdditionalSaveData` (and NeutralMob's `anger_end_time`).
pub fn to_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    use horizon_world::nbt::Tag;
    let b = get(m).cloned().unwrap_or_default();
    for (key, pos) in [("hive_pos", b.hive), ("flower_pos", b.flower)] {
        match pos {
            Some(p) => {
                map.insert(key.into(), Tag::IntArray(p.to_vec()));
            }
            None => {
                map.remove(&key.into());
            }
        }
    }
    map.insert("HasNectar".into(), Tag::Byte(i8::from(has_nectar(m))));
    map.insert("HasStung".into(), Tag::Byte(i8::from(has_stung(m))));
    map.insert(
        "TicksSincePollination".into(),
        Tag::Int(b.ticks_without_nectar),
    );
    map.insert("CannotEnterHiveTicks".into(), Tag::Int(b.stay_out_of_hive));
    map.insert("CropsGrownSincePollination".into(), Tag::Int(b.crops_grown));
    map.insert("anger_end_time".into(), Tag::Long(b.anger_end));
}
/// `Bee.readAdditionalSaveData`.
pub fn from_nbt(m: &mut Mob, tag: &horizon_world::nbt::Tag) {
    use horizon_world::nbt::Tag;
    let int = |k: &str| match tag.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    let pos = |k: &str| match tag.get(k) {
        Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
        _ => None,
    };
    set_has_nectar(m, int("HasNectar").unwrap_or(0) != 0);
    set_has_stung(m, int("HasStung").unwrap_or(0) != 0);
    let anger_end = match tag.get("anger_end_time") {
        Some(Tag::Long(v)) => *v,
        _ => -1,
    };
    let b = st(m);
    b.ticks_without_nectar = int("TicksSincePollination").unwrap_or(0);
    b.stay_out_of_hive = int("CannotEnterHiveTicks").unwrap_or(0);
    b.crops_grown = int("CropsGrownSincePollination").unwrap_or(0);
    b.hive = pos("hive_pos");
    b.flower = pos("flower_pos");
    b.anger_end = anger_end;
}
