//! Tamable animals (`TamableAnimal`): wolves with their variants, taming,
//! sitting, following and defending the owner, begging, anger and shaking
//! off water (`Wolf`, `WolfModel`).
use super::{
    Damage, Mob, MobEvent, MobWorld, PlayerView, Species, Who,
    anim::{Poses, State},
    goals::{
        Breed, Cx, Float, Goal, HurtByTarget, JUMP, LOOK, LeapAtTarget, LookAtPlayer, MOVE,
        MeleeAttack, NearestTarget, Panic, Prey, RandomLookAround, Stroll, TARGET, goal_clone,
        reduced,
    },
};
use horizon_core::geometry::trig::Mth;
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    random::RandomSource,
};
use std::f32::consts::PI;

/// Wolf variants: registry name and texture file (`WolfVariants`).
pub const WOLF_VARIANTS: [(&str, &str); 9] = [
    ("pale", "wolf"),
    ("spotted", "wolf_spotted"),
    ("snowy", "wolf_snowy"),
    ("black", "wolf_black"),
    ("ashen", "wolf_ashen"),
    ("rusty", "wolf_rusty"),
    ("woods", "wolf_woods"),
    ("chestnut", "wolf_chestnut"),
    ("striped", "wolf_striped"),
];
/// The wolf variant a biome spawns (its high-priority `BiomeCheck`).
pub fn wolf_variant_for(biome: &str, has_tag: &dyn Fn(&str) -> bool) -> u8 {
    if has_tag("minecraft:is_savanna") {
        1
    } else if biome == "minecraft:grove" {
        2
    } else if biome == "minecraft:old_growth_pine_taiga" {
        3
    } else if biome == "minecraft:snowy_taiga" {
        4
    } else if has_tag("minecraft:is_jungle") {
        5
    } else if biome == "minecraft:forest" {
        6
    } else if biome == "minecraft:old_growth_spruce_taiga" {
        7
    } else if has_tag("minecraft:is_badlands") {
        8
    } else {
        0
    }
}
/// The default collar: red.
pub const RED: u8 = 14;

// AnimState slots: begging interest and the shake animation.
pub const INTEREST: usize = 0;
pub const SHAKE: usize = 1;

pub fn register(m: &mut Mob) {
    m.extra.color = RED;
    let food = m.kind.food.clone();
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(1, TamePanic(Panic::new(1.5)));
    g.add(2, SitWhenOrdered);
    g.add(3, AvoidLlamas::new());
    g.add(4, LeapAtTarget::new(0.4));
    g.add(5, MeleeAttack::new(1.0, true));
    g.add(6, FollowOwner::new(1.0, 10.0, 2.0));
    g.add(7, Breed::new(1.0));
    g.add(8, Stroll::avoiding_water(1.0));
    g.add(9, Beg { food, ticks: 0 });
    g.add(10, LookAtPlayer::new(8.0));
    g.add(10, RandomLookAround::default());
    m.ai.targets.add(1, OwnerHurtBy { timestamp: 0 });
    m.ai.targets.add(2, OwnerHurt { timestamp: 0 });
    m.ai.targets.add(3, HurtByTarget::new().alert());
    m.ai.targets
        .add(4, NearestTarget::player(true).when(angry_at_player));
    m.ai.targets.add(
        5,
        NearestTarget::new(Prey::Filter(wolf_prey), false).when(|m, _| !m.extra.tame),
    );
    m.ai.targets.add(
        6,
        NearestTarget::new(Prey::Filter(baby_turtle_on_land), false).when(|m, _| !m.extra.tame),
    );
    m.ai.targets.add(
        7,
        NearestTarget::species(
            &[
                Species::Skeleton,
                Species::Stray,
                Species::Bogged,
                Species::WitherSkeleton,
            ],
            false,
        ),
    );
}
/// `Parrot.registerGoals`: a tamed parrot sits when told to and follows its
/// owner from five blocks away (the wander and the shoulder come as they are).
pub fn register_parrot(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, TamePanic(Panic::new(1.25)));
    g.add(0, Float);
    g.add(1, LookAtPlayer::new(8.0));
    g.add(2, SitWhenOrdered);
    g.add(2, FollowOwner::new(1.0, 5.0, 1.0));
    g.add(2, Stroll::avoiding_water(1.0));
    g.add(3, LandOnOwnersShoulder);
}
/// `LandOnOwnersShoulderGoal`: a standing parrot, 100 ticks old, lands on
/// the shoulder of the owner it touches; the host does the landing.
#[derive(Clone, Debug)]
struct LandOnOwnersShoulder;
impl Goal for LandOnOwnersShoulder {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // ShoulderRidingEntity.canSitOnShoulder: rideCooldownCounter > 100.
        is_owner(m, cx.w)
            && owner(m, cx.w).is_some_and(|(_, _, spectator)| !spectator)
            && !m.extra.sitting
            && m.age > 100
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if !m.extra.sitting
            && m.extra.leash.is_none()
            && is_owner(m, cx.w)
            && cx
                .w
                .player
                .as_ref()
                .is_some_and(|p| m.bounds().intersects(p.bounds))
        {
            cx.events.push(MobEvent::ParrotToShoulder { mob: m.id });
        }
    }
}
/// `Wolf.PREY_SELECTOR`: sheep, rabbits and foxes.
fn wolf_prey(s: &super::Seen) -> bool {
    matches!(s.species, Species::Sheep | Species::Rabbit | Species::Fox)
}
fn baby_turtle_on_land(s: &super::Seen) -> bool {
    s.species == Species::Turtle && s.baby
}
/// `NeutralMob.isAngryAt(player)`: the player is the anger's target.
fn angry_at_player(m: &Mob, _: &MobWorld) -> bool {
    m.extra.angry_at == Some(Who::Player)
}
/// `Wolf.isAngry`.
pub fn angry(m: &Mob) -> bool {
    m.extra.anger > 0
}
/// `TamableAnimal.applyTamingSideEffects` for wolves: 40 health when tame.
pub fn set_tame(m: &mut Mob, tame: bool) {
    m.extra.tame = tame;
    if tame {
        m.attrs.max_health = 40.0;
        m.health = 40.0;
    } else {
        m.attrs.max_health = 8.0;
    }
}
/// `TamableAnimal.getOwner`: the player the saved owner UUID names, when it
/// is the one this mob sees; anyone else nearby is not its owner.
fn owner_view<'a>(m: &Mob, w: &'a MobWorld) -> Option<&'a PlayerView> {
    if !m.extra.tame {
        return None;
    }
    let p = w.player.as_ref()?;
    (m.extra.owner == Some(p.uuid)).then_some(p)
}
/// The owner's position, eye height and spectator flag when tame.
fn owner(m: &Mob, w: &MobWorld) -> Option<(Vec3, f64, bool)> {
    let p = owner_view(m, w)?;
    Some((p.position, p.eye[1], p.spectator))
}
/// Whether the player the mob sees is the one saved as its owner (with
/// guests on, that player is whoever stands nearest).
fn is_owner(m: &Mob, w: &MobWorld) -> bool {
    w.player
        .as_ref()
        .is_some_and(|p| owned_by(m.extra.owner, p.uuid))
}
/// Whether `uuid` is the saved owner. A host with no UUID yet owns what it
/// tamed under the placeholder `[0, 0, 0, 1]`.
pub fn owned_by(owner: Option<[i32; 4]>, uuid: [i32; 4]) -> bool {
    owner.is_some_and(|o| o == uuid || (o == [0, 0, 0, 1] && uuid == [0; 4]))
}
/// `TamableAnimal.unableToMoveToOwner`.
fn unable_to_move_to_owner(m: &Mob, w: &MobWorld) -> bool {
    m.extra.sitting || owner_view(m, w).is_some_and(|p| p.spectator)
}
/// `TamableAnimal.tryToTeleportToOwner`: ten tries around the owner.
fn teleport_to_owner(m: &mut Mob, w: &MobWorld) {
    let Some((at, _, _)) = owner(m, w) else {
        return;
    };
    let base = [
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    ];
    for _ in 0..10 {
        let xd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        let zd = m.random.next_int_bound(7).unwrap_or(0) - 3;
        if xd.abs() < 2 && zd.abs() < 2 {
            continue;
        }
        let yd = m.random.next_int_bound(3).unwrap_or(0) - 1;
        let p = [base[0] + xd, base[1] + yd, base[2] + zd];
        if can_teleport_to(m, w, p) {
            m.position = Vec3::new(
                f64::from(p[0]) + 0.5,
                f64::from(p[1]),
                f64::from(p[2]) + 0.5,
            );
            m.previous = m.position;
            m.nav_stop();
            return;
        }
    }
}
/// `TamableAnimal.canTeleportTo`: walkable, not on leaves, no collision.
fn can_teleport_to(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    let below = w.facts.get(w.world.block(p[0], p[1] - 1, p[2]));
    if !below.full_collision || below.name.ends_with("_leaves") {
        return false;
    }
    let here = w.facts.get(w.world.block(p[0], p[1], p[2]));
    if here.fluid != crate::blocks::Fluid::None {
        return false;
    }
    let half = f64::from(m.width()) / 2.0;
    let (x, y, z) = (
        f64::from(p[0]) + 0.5,
        f64::from(p[1]),
        f64::from(p[2]) + 0.5,
    );
    w.no_collision(Aabb::new(
        Vec3::new(x - half, y, z - half),
        Vec3::new(x + half, y + f64::from(m.height()), z + half),
    ))
}
fn owner_distance_sqr(m: &Mob, w: &MobWorld) -> Option<f64> {
    owner(m, w).map(|(at, _, _)| m.distance_sqr(at))
}

/// `TamableAnimal.TamableAnimalPanicGoal`: panic from fire and freezing,
/// teleporting to a far owner.
#[derive(Clone, Debug)]
struct TamePanic(Panic);
impl Goal for TamePanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // PanicGoal with PANIC_ENVIRONMENTAL_CAUSES: only burning or freezing.
        m.fire_ticks > 0 && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if !unable_to_move_to_owner(m, cx.w)
            && owner_distance_sqr(m, cx.w).is_some_and(|d| d >= 144.0)
        {
            teleport_to_owner(m, cx.w);
        }
        self.0.tick(m, cx);
    }
}
/// `SitWhenOrderedToGoal`.
#[derive(Clone, Debug)]
struct SitWhenOrdered;
impl Goal for SitWhenOrdered {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.sitting && !m.extra.tame {
            return false;
        }
        if m.fluids.in_water || !m.on_ground {
            return false;
        }
        match owner_distance_sqr(m, cx.w) {
            None => true,
            Some(d) => {
                let attacked = owner_view(m, cx.w).is_some_and(|p| p.hurt_by.is_some());
                !(d < 144.0 && attacked) && m.extra.sitting
            }
        }
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.sitting
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        m.extra.flag = false;
    }
}
/// `Wolf.WolfAvoidEntityGoal`: untamed wolves flee llamas.
#[derive(Clone, Debug)]
struct AvoidLlamas(super::goals::AvoidEntity);
impl AvoidLlamas {
    fn new() -> Self {
        Self(super::goals::AvoidEntity::new(
            super::goals::Avoid::Species(vec![Species::Llama, Species::TraderLlama]),
            24.0,
            1.5,
            1.5,
        ))
    }
}
/// `Wolf.WolfAvoidEntityGoal`: a wild wolf runs from a llama, the more
/// likely the stronger it is (`strength >= random.nextInt(5)`), and drops
/// its target while it does.
impl Goal for AvoidLlamas {
    goal_clone!();
    fn name(&self) -> &'static str {
        "WolfAvoidEntityGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !self.0.can_use(m, cx) {
            return false;
        }
        // Seen.profession is the llama's strength (`variant2`).
        let Some(strength) = self
            .0
            .found
            .and_then(|id| cx.w.seen(id))
            .map(|s| s.profession)
        else {
            return false;
        };
        !m.extra.tame && i32::from(strength) >= m.random.next_int_bound(5).unwrap_or(0)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.ai.target = None;
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.ai.target = None;
        self.0.tick(m, cx)
    }
}
/// `FollowOwnerGoal(speed, startDistance, stopDistance)`.
#[derive(Clone, Debug)]
struct FollowOwner {
    speed: f64,
    start: f64,
    stop: f64,
    recalc: i32,
}
impl FollowOwner {
    fn new(speed: f64, start: f64, stop: f64) -> Self {
        Self {
            speed,
            start,
            stop,
            recalc: 0,
        }
    }
}
impl Goal for FollowOwner {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(d) = owner_distance_sqr(m, cx.w) else {
            return false;
        };
        !unable_to_move_to_owner(m, cx.w) && d >= self.start * self.start
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.nav.is_done() || unable_to_move_to_owner(m, cx.w) {
            return false;
        }
        owner_distance_sqr(m, cx.w).is_some_and(|d| d > self.stop * self.stop)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        self.recalc = 0;
        m.set_malus(super::path::PathType::Water, 0.0);
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        let water = super::path::PathType::Water.malus();
        m.set_malus(super::path::PathType::Water, water);
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some((at, eye, _)) = owner(m, cx.w) else {
            return;
        };
        let far = m.distance_sqr(at) >= 144.0;
        if !far {
            let max = m.max_head_x_rot();
            m.look_at_speed([at.x, eye, at.z], 10.0, max);
        }
        self.recalc -= 1;
        if self.recalc <= 0 {
            self.recalc = reduced(10);
            if far {
                teleport_to_owner(m, cx.w);
            } else {
                m.nav_move_to_entity(cx.w, [at.x, at.y, at.z], 0, self.speed);
            }
        }
    }
}
/// `OwnerHurtByTargetGoal`: attack what hurt the owner.
#[derive(Clone, Debug)]
struct OwnerHurtBy {
    timestamp: i64,
}
impl Goal for OwnerHurtBy {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.tame || m.extra.sitting {
            return false;
        }
        let Some((who, at)) = owner_view(m, cx.w).and_then(|p| p.hurt_by) else {
            return false;
        };
        at != self.timestamp && wants_to_attack(m, cx.w, who)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target
            .and_then(|t| cx.w.target_info(t))
            .is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((who, at)) = owner_view(m, cx.w).and_then(|p| p.hurt_by) {
            m.ai.target = Some(who);
            self.timestamp = at;
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
/// `OwnerHurtTargetGoal`: attack what the owner attacks.
#[derive(Clone, Debug)]
struct OwnerHurt {
    timestamp: i64,
}
impl Goal for OwnerHurt {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.tame || m.extra.sitting {
            return false;
        }
        let Some((who, at)) = owner_view(m, cx.w).and_then(|p| p.last_hurt) else {
            return false;
        };
        at != self.timestamp && wants_to_attack(m, cx.w, who)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.ai.target
            .and_then(|t| cx.w.target_info(t))
            .is_some_and(|t| t.alive)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some((who, at)) = owner_view(m, cx.w).and_then(|p| p.last_hurt) {
            m.ai.target = Some(who);
            self.timestamp = at;
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
/// `Wolf.wantsToAttack`: never creepers, ghasts, tame cats and parrots, or a
/// tame wolf of the same owner.
fn wants_to_attack(m: &Mob, w: &MobWorld, who: Who) -> bool {
    let Who::Mob(id) = who else {
        return false;
    };
    if id == m.id {
        return false;
    }
    let Some(s) = w.seen(id) else {
        return false;
    };
    match s.species {
        Species::Creeper | Species::Ghast => false,
        Species::Wolf => !s.tame || s.owner != m.extra.owner,
        Species::Cat | Species::Parrot => !s.tame,
        Species::Horse | Species::Donkey | Species::Mule | Species::Llama => !s.tame,
        _ => true,
    }
}
/// `BegGoal`: tilt the head at a player holding something interesting.
#[derive(Clone, Debug)]
struct Beg {
    food: std::sync::Arc<[String]>,
    ticks: i32,
}
impl Beg {
    fn interesting(&self, w: &MobWorld) -> bool {
        let Some(p) = &w.player else {
            return false;
        };
        let held = |i: &Option<std::sync::Arc<str>>| {
            // `BegGoal.playerHoldingInteresting`: a bone or food, tame or not.
            i.as_deref()
                .is_some_and(|i| i == "minecraft:bone" || self.food.iter().any(|f| f == i))
        };
        p.alive && !p.spectator && (held(&p.main_hand) || held(&p.off_hand))
    }
}
impl Goal for Beg {
    goal_clone!();
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.player
            .as_ref()
            .is_some_and(|p| m.distance_sqr(p.position) <= 64.0)
            && self.interesting(cx.w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        cx.w.player
            .as_ref()
            .is_some_and(|p| p.alive && m.distance_sqr(p.position) <= 64.0)
            && self.ticks > 0
            && self.interesting(cx.w)
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = true;
        self.ticks = reduced(40 + m.random.next_int_bound(40).unwrap_or(0));
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(p) = &cx.w.player {
            let max = m.max_head_x_rot();
            m.look_at_speed([p.position.x, p.eye[1], p.position.z], 10.0, max);
        }
        self.ticks -= 1;
    }
}
/// `NeutralMob.isValidPlayerTarget`: a player in survival or adventure, off
/// peaceful.
fn valid_player_target(w: &MobWorld) -> bool {
    w.player
        .as_ref()
        .is_some_and(|p| !p.creative && !p.spectator)
        && w.difficulty.id != 0
}
/// `NeutralMob.stopBeingAngry`.
fn stop_being_angry(m: &mut Mob) {
    // setLastHurtByMob(null) also stamps the time.
    m.ai.last_hurt_by = None;
    m.ai.last_hurt_by_time = m.age as i32;
    m.ai.target = None;
    m.extra.angry_at = None;
    m.extra.anger = 0;
}
/// `NeutralMob.updatePersistentAnger(level, true)` from `Wolf.aiStep`: any
/// target (the attacker, a pack-mate's attacker the `HurtByTargetGoal` alert
/// gave it, prey, the owner's enemy) is the anger's target, and the 20 to 39
/// seconds of `PERSISTENT_ANGER_TIME` start over every tick it is held.
/// Without a target the anger runs out (`extra.anger` counts down to the end
/// time), and anger at a mob ends when that mob dies.
fn update_persistent_anger(m: &mut Mob, w: &MobWorld) {
    let angry_at = m.extra.angry_at;
    if let Some(target @ Who::Mob(id)) = m.ai.target
        && w.seen(id).is_none_or(|s| !s.alive)
        && angry_at == Some(target)
    {
        stop_being_angry(m);
        return;
    }
    // Mob.getTarget: a player in creative or spectator mode is no target.
    let target = m.ai.target.filter(|t| {
        *t != Who::Player
            || w.player
                .as_ref()
                .is_some_and(|p| !p.creative && !p.spectator)
    });
    if let Some(target) = target {
        m.extra.angry_at = Some(target);
        // Wolf.startPersistentAngerTimer: TimeUtil.rangeOfSeconds(20, 39).
        m.extra.anger = 400 + m.random.next_int_bound(381).unwrap_or(0);
    } else if m.extra.anger > 0 {
        m.extra.anger -= 1;
    }
    if angry_at.is_some()
        && m.extra.anger == 0
        && !(target == Some(Who::Player) && valid_player_target(w))
    {
        stop_being_angry(m);
    }
    // The anger's target as it was when the tick began, once it is found: a
    // player who is not here yet (a reloaded wolf ticks before the player
    // joins) keeps the anger.
    if angry_at == Some(Who::Player)
        && w.player
            .as_ref()
            .is_some_and(|p| p.creative || p.spectator || w.difficulty.id == 0)
    {
        stop_being_angry(m);
    }
}
/// `Wolf.tick` / `aiStep`: anger, begging tilt and shaking off water.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    update_persistent_anger(m, w);
    if !m.alive() {
        return;
    }
    // interestedAngle eases toward 1 while begging.
    let goal = if m.extra.flag { 1.0 } else { 0.0 };
    let v = &mut m.anim.v;
    v[INTEREST] += (goal - v[INTEREST]) * 0.4;
    // isWet (extra.flag2), isShaking (counter2 > 0).
    let rain = super::ai::in_rain(m, w);
    let shaking = m.extra.counter2 > 0;
    if m.fluids.in_water || rain {
        m.extra.flag2 = true;
        if shaking {
            m.extra.counter2 = 0;
            m.anim.v[SHAKE] = 0.0;
            m.anim.vo[SHAKE] = 0.0;
        }
    } else if shaking {
        if m.anim.v[SHAKE] == 0.0 {
            let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
            events.push(MobEvent::Sound("entity.wolf.shake", m.center(), 0.4, pitch));
        }
        m.anim.v[SHAKE] += 0.05;
        if m.anim.vo[SHAKE] >= 2.0 {
            m.extra.flag2 = false;
            m.extra.counter2 = 0;
            m.anim.v[SHAKE] = 0.0;
            m.anim.vo[SHAKE] = 0.0;
        }
        if m.anim.v[SHAKE] > 0.4 {
            let count = (((m.anim.v[SHAKE] - 0.4) * PI).msin() * 7.0) as u32;
            if count > 0 {
                events.push(MobEvent::Particles {
                    kind: "splash",
                    at: [m.position.x, m.position.y + 0.8, m.position.z],
                    count,
                });
            }
        }
    } else if m.extra.flag2 && m.ai.nav.is_done() && m.on_ground {
        // Wolf.aiStep: start shaking when wet, standing and not pathing.
        m.extra.counter2 = 1;
        m.anim.v[SHAKE] = 0.0;
        m.anim.vo[SHAKE] = 0.0;
    }
}
/// `Wolf.hurtServer`: stop sitting. The anger comes with the target
/// (`HurtByTargetGoal`, then `updatePersistentAnger`).
pub fn after_hurt(m: &mut Mob, _source: &Damage) {
    m.extra.sitting = false;
}
/// `Wolf.getTailAngle`.
fn tail_angle(m: &Mob) -> f32 {
    if angry(m) {
        1.539_380_4
    } else if m.extra.tame {
        let max = m.attrs.max_health;
        let ratio = (max - m.health) / max;
        (0.55 - ratio * 0.4) * PI
    } else {
        PI / 5.0
    }
}
/// `WolfRenderState.getBodyRollAngle`.
fn body_roll(shake: f32, offset: f32) -> f32 {
    let p = ((shake + offset) / 1.8).clamp(0.0, 1.0);
    (p * PI).msin() * (p * PI * 11.0).msin() * 0.15 * PI
}
/// `WolfModel.setupAnim` with the adult and baby overrides.
pub fn anim(m: &Mob, s: &State, p: &mut Poses) {
    let (pos, speed) = (s.pos, s.speed);
    let baby = m.is_baby();
    if angry(m) {
        p.yrot("tail", 0.0);
    } else {
        p.yrot("tail", (pos * 0.6662).mcos() * 1.4 * speed);
    }
    if m.extra.sitting {
        let age = if baby { 0.5 } else { 1.0 };
        let add = |p: &mut Poses, part: &str, x: f32, y: f32, z: f32| {
            let v = p.get(part);
            v[0] += x;
            v[1] += y;
            v[2] += z;
        };
        add(p, "body", 0.0, 4.0 * age, -2.0 * age);
        p.xrot("body", PI / 4.0);
        add(p, "tail", 0.0, 9.0 * age, -2.0 * age);
        add(p, "right_hind_leg", 0.0, 6.7 * age, -5.0 * age);
        p.xrot("right_hind_leg", PI * 3.0 / 2.0);
        add(p, "left_hind_leg", 0.0, 6.7 * age, -5.0 * age);
        p.xrot("left_hind_leg", PI * 3.0 / 2.0);
        p.xrot("right_front_leg", 5.811_947);
        add(p, "right_front_leg", 0.01 * age, 1.0 * age, 0.0);
        p.xrot("left_front_leg", 5.811_947);
        add(p, "left_front_leg", -0.01 * age, 1.0 * age, 0.0);
        if baby {
            let x = p.get("body")[3] - PI / 2.0;
            p.xrot("body", x);
        } else if p.has("upper_body") {
            p.get("upper_body")[1] += 2.0;
            p.xrot("upper_body", PI * 2.0 / 5.0);
            p.yrot("upper_body", 0.0);
        }
    } else {
        p.xrot("right_hind_leg", (pos * 0.6662).mcos() * 1.4 * speed);
        p.xrot("left_hind_leg", (pos * 0.6662 + PI).mcos() * 1.4 * speed);
        p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * 1.4 * speed);
        p.xrot("left_front_leg", (pos * 0.6662).mcos() * 1.4 * speed);
    }
    // shakeOffWater
    let shake = m.anim.lerp(SHAKE, s.partial);
    let roll = m.anim.lerp(INTEREST, s.partial) * 0.15 * PI;
    p.zrot("body", body_roll(shake, -0.16));
    if baby {
        p.zrot("head", roll + body_roll(shake, 0.0));
        p.zrot("tail", body_roll(shake, -0.2));
    } else {
        if p.has("real_head") {
            p.zrot("real_head", roll + body_roll(shake, 0.0));
        }
        if p.has("upper_body") {
            p.zrot("upper_body", body_roll(shake, -0.08));
        }
        if p.has("real_tail") {
            p.zrot("real_tail", body_roll(shake, -0.2));
        }
    }
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    p.xrot("tail", tail_angle(m));
}
/// `Wolf.getWetShade` as a vertex colour.
pub fn tint(m: &Mob, partial: f32) -> u32 {
    if !m.extra.flag2 {
        return 0xFFFF_FFFF;
    }
    let shade = (0.75 + m.anim.lerp(SHAKE, partial) / 2.0 * 0.25).min(1.0);
    let c = (shade * 255.0) as u32;
    0xFF00_0000 | c << 16 | c << 8 | c
}
/// `Wolf.getTexture`.
pub fn texture(m: &Mob) -> String {
    let file = WOLF_VARIANTS[(m.extra.variant as usize).min(8)].1;
    let state = if m.extra.tame {
        "_tame"
    } else if angry(m) {
        "_angry"
    } else {
        ""
    };
    let baby = if m.is_baby() { "_baby" } else { "" };
    format!("entity/wolf/{file}{state}{baby}")
}

/// Cat variants (`CatVariants`): registry name, texture file.
pub const CAT_VARIANTS: [&str; 11] = [
    "tabby",
    "black",
    "red",
    "siamese",
    "british_shorthair",
    "calico",
    "persian",
    "ragdoll",
    "white",
    "jellie",
    "all_black",
];
// Cat AnimState slots.
const LIE: usize = 0;
const LIE_TAIL: usize = 1;
const RELAX: usize = 2;

/// `Cat.registerGoals` (with `reassessTameGoals`' player avoidance).
pub fn register_cat(m: &mut Mob) {
    m.extra.color = RED;
    let food = m.kind.food.clone();
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(1, TamePanic(Panic::new(1.5)));
    g.add(2, SitWhenOrdered);
    g.add(3, CatRelaxOnOwner::default());
    g.add(4, CatTempt(super::goals::Tempt::new(0.6, food, true)));
    g.add(
        4,
        super::goals::AvoidEntity::new(super::goals::Avoid::Player, 16.0, 0.8, 1.33)
            .when(|m| !m.extra.tame),
    );
    g.add(
        5,
        CatLieOnBlock(super::goals::MoveToBlock {
            vertical_start: -2,
            ..super::goals::MoveToBlock::new(1.1, 8, 6)
        }),
    );
    g.add(6, FollowOwner::new(1.0, 10.0, 5.0));
    g.add(7, CatSitOnBlock(super::goals::MoveToBlock::new(0.8, 8, 1)));
    g.add(8, LeapAtTarget::new(0.3));
    g.add(9, OcelotAttack { attack_time: 0 });
    g.add(10, Breed::new(0.8));
    g.add(11, Stroll::avoiding_water(0.8));
    g.add(12, LookAtPlayer::new(10.0));
    m.ai.targets.add(
        1,
        NearestTarget::species(&[Species::Rabbit], false).when(|m, _| !m.extra.tame),
    );
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Filter(baby_turtle_on_land), false).when(|m, _| !m.extra.tame),
    );
}
/// `Ocelot.registerGoals`, then `reassessTrustingGoals` adds the player
/// avoidance last. Trust (`Ocelot.mobInteract`, the `Trusting` tag) is not
/// ported yet, so every ocelot is wild: it flees players and can be scared
/// off while tempted.
pub fn register_ocelot(m: &mut Mob) {
    let food = m.kind.food.clone();
    let mut stroll = Stroll::avoiding_water(0.8);
    stroll.water_avoiding = Some(1.000_000_1e-5);
    let g = &mut m.ai.goals;
    g.add(1, Float);
    g.add(3, OcelotTempt(super::goals::Tempt::new(0.6, food, true)));
    g.add(7, LeapAtTarget::new(0.3));
    g.add(8, OcelotAttack { attack_time: 0 });
    g.add(9, Breed::new(0.8));
    g.add(10, stroll);
    g.add(11, LookAtPlayer::new(10.0));
    g.add(4, OcelotAvoid::new());
    m.ai.targets
        .add(1, NearestTarget::species(&[Species::Chicken], false));
    m.ai.targets.add(
        1,
        NearestTarget::new(Prey::Filter(baby_turtle_on_land), false),
    );
}
/// `Ocelot.OcelotTemptGoal(ocelot, 0.6, ocelot food, true)`.
#[derive(Clone, Debug)]
struct OcelotTempt(super::goals::Tempt);
impl Goal for OcelotTempt {
    goal_clone!();
    fn name(&self) -> &'static str {
        "OcelotTemptGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
/// `Ocelot.OcelotAvoidEntityGoal(ocelot, Player.class, 16.0F, 0.8, 1.33)`.
#[derive(Clone, Debug)]
struct OcelotAvoid(super::goals::AvoidEntity);
impl OcelotAvoid {
    fn new() -> Self {
        Self(super::goals::AvoidEntity::new(
            super::goals::Avoid::Player,
            16.0,
            0.8,
            1.33,
        ))
    }
}
impl Goal for OcelotAvoid {
    goal_clone!();
    fn name(&self) -> &'static str {
        "OcelotAvoidEntityGoal"
    }
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
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
/// `Cat.isInSittingPose`: ordered to sit or sitting on a block.
pub fn cat_sitting(m: &Mob) -> bool {
    m.extra.sitting || m.extra.flag2
}
/// `Cat.CatTemptGoal`: only wild cats come for fish.
#[derive(Clone, Debug)]
struct CatTempt(super::goals::Tempt);
impl Goal for CatTempt {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        !m.extra.tame && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        self.0.every_tick()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        // Cat.tick: a wild cat being tempted begs every 100 ticks.
        if m.age.is_multiple_of(100) {
            cx.events.push(MobEvent::Sound(
                super::finalize::variant_sound(m, "beg_for_food")
                    .unwrap_or("entity.cat.beg_for_food"),
                m.center(),
                1.0,
                1.0,
            ));
        }
        self.0.tick(m, cx)
    }
}
fn has_tag(w: &MobWorld, p: [i32; 3], tag: &str) -> bool {
    let name = w.facts.get(w.world.block(p[0], p[1], p[2])).name.clone();
    w.block_tags.get(tag).is_some_and(|t| t.contains(&*name))
}
fn empty_above(w: &MobWorld, p: [i32; 3]) -> bool {
    w.facts
        .get(w.world.block(p[0], p[1] + 1, p[2]))
        .name
        .as_ref()
        == "minecraft:air"
}
/// Whether `p` is the player that owns `m` (a cat without a recorded owner
/// belongs to whoever it sees).
pub fn is_owner_of(m: &Mob, p: &super::PlayerView) -> bool {
    m.extra.owner.is_none_or(|o| o == p.uuid)
}
/// `Cat.CatRelaxOnOwnerGoal`: while the owner sleeps, the cat lies down beside
/// them, and may leave a morning gift when they wake up.
#[derive(Clone, Debug, Default)]
struct CatRelaxOnOwner {
    goal: Option<[i32; 3]>,
    on_bed_ticks: i32,
}
impl CatRelaxOnOwner {
    /// `spaceIsOccupied`: another cat lying or relaxing within two blocks.
    fn occupied(&self, m: &Mob, w: &MobWorld) -> bool {
        let Some(g) = self.goal else { return true };
        let area = Aabb::new(
            Vec3::new(
                f64::from(g[0]) - 2.0,
                f64::from(g[1]) - 2.0,
                f64::from(g[2]) - 2.0,
            ),
            Vec3::new(
                f64::from(g[0]) + 3.0,
                f64::from(g[1]) + 3.0,
                f64::from(g[2]) + 3.0,
            ),
        );
        w.others.iter().any(|o| {
            o.species == Species::Cat && o.id != m.id && o.lying && o.bounds.intersects(area)
        })
    }
    /// The owner's sleeping state, when this cat may relax on them: the
    /// player it sees has to be its owner (`Cat.getOwner`).
    fn sleeping_owner<'w>(m: &Mob, w: &'w MobWorld) -> Option<&'w super::PlayerView> {
        w.player
            .as_ref()
            .filter(|p| m.extra.tame && !m.extra.sitting && p.sleeping && is_owner_of(m, p))
    }
}
impl Goal for CatRelaxOnOwner {
    goal_clone!();
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let w = cx.w;
        let Some(p) = Self::sleeping_owner(m, w) else {
            return false;
        };
        if m.distance_sqr(p.position) > 100.0 {
            return false;
        }
        let at = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if !has_tag(w, at, "minecraft:cats_can_lie_on") {
            return false;
        }
        // The bed's foot: against `BedBlock.FACING`.
        let back = match w.prop(at[0], at[1], at[2], "facing") {
            Some("north") => [0, 0, 1],
            Some("south") => [0, 0, -1],
            Some("east") => [-1, 0, 0],
            Some("west") => [1, 0, 0],
            Some("up") => [0, -1, 0],
            Some("down") => [0, 1, 0],
            _ => [0, 0, 0],
        };
        self.goal = Some([at[0] + back[0], at[1] + back[1], at[2] + back[2]]);
        !self.occupied(m, w)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        Self::sleeping_owner(m, cx.w).is_some() && self.goal.is_some() && !self.occupied(m, cx.w)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(g) = self.goal {
            m.extra.flag2 = false;
            m.nav_move_to(
                cx.w,
                [f64::from(g[0]), f64::from(g[1]), f64::from(g[2])],
                1.1,
            );
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.flag = false;
        // The `gameplay/cat_waking_up_gift_chance` of the `day` timeline: 0.7
        // from tick 23667 of the day to tick 362 of the next, 0 in between.
        let day = cx.w.day_time.rem_euclid(24000);
        let chance = if !cx.w.nether && !cx.w.end && !(362..23667).contains(&day) {
            0.7
        } else {
            0.0
        };
        if cx
            .w
            .player
            .as_ref()
            .is_some_and(|p| is_owner_of(m, p) && p.sleep_timer >= 100)
            && m.random.next_float() < chance
        {
            give_morning_gift(m, cx);
        }
        self.on_bed_ticks = 0;
        m.extra.relax_one = false;
        m.nav_stop();
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (Some(p), Some(g)) = (cx.w.player.as_ref(), self.goal) else {
            return;
        };
        m.extra.flag2 = false;
        m.nav_move_to(
            cx.w,
            [f64::from(g[0]), f64::from(g[1]), f64::from(g[2])],
            1.1,
        );
        if m.distance_sqr(p.position) < 2.5 {
            self.on_bed_ticks += 1;
            if self.on_bed_ticks > reduced(16) {
                m.extra.flag = true;
                m.extra.relax_one = false;
            } else {
                m.look_at_speed(p.eye, 45.0, 45.0);
                m.extra.relax_one = true;
            }
        } else {
            m.extra.flag = false;
        }
    }
}
/// `CatRelaxOnOwnerGoal.giveMorningGift`: hop to a spot within five blocks
/// (`randomTeleport`, avoiding `#dangerous_for_teleportation`) and drop a
/// `gameplay/cat_morning_gift` item at its feet.
fn give_morning_gift(m: &mut Mob, cx: &mut Cx) {
    let w = cx.w;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    let x = at[0] + m.random.next_int_bound(11).unwrap_or(5) - 5;
    let y = at[1] + m.random.next_int_bound(5).unwrap_or(2) - 2;
    let z = at[2] + m.random.next_int_bound(11).unwrap_or(5) - 5;
    let tagged = |p: [i32; 3], tag| has_tag(w, p, tag);
    let mut ground = y - 1;
    while ground > crate::chunk::MIN_Y
        && !tagged([x, ground, z], "minecraft:entities_can_teleport_to")
    {
        ground -= 1;
    }
    if w.world.loaded(x, z) && ground > crate::chunk::MIN_Y {
        let feet = f64::from(ground + 1);
        let half = f64::from(m.width()) / 2.0;
        let b = Aabb::new(
            Vec3::new(f64::from(x) - half, feet, f64::from(z) - half),
            Vec3::new(
                f64::from(x) + half,
                feet + f64::from(m.height()),
                f64::from(z) + half,
            ),
        );
        let dangerous = |p| tagged(p, "minecraft:dangerous_for_teleportation");
        let in_box = |f: &dyn Fn([i32; 3]) -> bool| {
            (b.min(Axis::X).floor() as i32..=b.max(Axis::X).floor() as i32).any(|bx| {
                (b.min(Axis::Y).floor() as i32..=b.max(Axis::Y).floor() as i32).any(|by| {
                    (b.min(Axis::Z).floor() as i32..=b.max(Axis::Z).floor() as i32)
                        .any(|bz| f([bx, by, bz]))
                })
            })
        };
        let fluids = w.fluids(b, feet);
        if !dangerous([x, ground, z])
            && w.no_collision(b)
            && !(fluids.in_water || fluids.in_lava)
            && !in_box(&dangerous)
        {
            m.position = Vec3::new(f64::from(x), feet, f64::from(z));
            m.previous = m.position;
            m.velocity = Vec3::ZERO;
        }
    }
    let yaw = m.body_yaw * (PI / 180.0);
    let here = [
        m.position.x.floor() - f64::from(yaw.msin()),
        m.position.y.floor(),
        m.position.z.floor() + f64::from(yaw.mcos()),
    ];
    cx.events.push(MobEvent::DropGift {
        table: "minecraft:gameplay/cat_morning_gift",
        at: here,
    });
}
/// `CatLieOnBlockGoal`: tame cats lie on beds and other warm blocks.
#[derive(Clone, Debug)]
struct CatLieOnBlock(super::goals::MoveToBlock);
fn can_lie_on(w: &MobWorld, p: [i32; 3]) -> bool {
    empty_above(w, p) && has_tag(w, p, "minecraft:cats_can_lie_on")
}
impl Goal for CatLieOnBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.extra.tame
            && !m.extra.sitting
            && !m.extra.flag
            && self.0.can_use(m, cx.w, reduced(40), can_lie_on)
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(cx.w, can_lie_on)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
        m.extra.flag2 = false;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag = false;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx.w);
        m.extra.flag2 = false;
        m.extra.flag = self.0.reached;
    }
}
/// `CatSitOnBlockGoal`: unopened chests, lit furnaces and the foot of beds.
#[derive(Clone, Debug)]
struct CatSitOnBlock(super::goals::MoveToBlock);
fn can_sit_on(w: &MobWorld, p: [i32; 3]) -> bool {
    if !empty_above(w, p) || !has_tag(w, p, "minecraft:cats_can_sit_on") {
        return false;
    }
    let f = w.facts.get(w.world.block(p[0], p[1], p[2]));
    match f.name.as_ref() {
        // The lit state is the only light source a furnace has.
        "minecraft:furnace" => f.emission > 0,
        // `ChestBlockEntity.getOpenCount(level, pos) < 1`: players' openers
        // and the copper golems' own count.
        "minecraft:chest" => {
            let key = (p[0], p[1], p[2]);
            !w.lid_openers.contains_key(&key)
                && w.world.chest_openers.get(&key).is_none_or(|o| o.0 < 1)
        }
        // A bed's foot only (`BedBlock.PART != HEAD`).
        _ => w.prop(p[0], p[1], p[2], "part") != Some("head"),
    }
}
impl Goal for CatSitOnBlock {
    goal_clone!();
    fn flags(&self) -> u8 {
        JUMP | MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !m.extra.tame || m.extra.sitting {
            return false;
        }
        // MoveToBlockGoal.canUse draws the next delay only once the last one
        // has run out.
        if self.0.next_start > 0 {
            self.0.next_start -= 1;
            return false;
        }
        let next = reduced(200 + m.random.next_int_bound(200).unwrap_or(0));
        self.0.can_use(m, cx.w, next, can_sit_on)
    }
    fn can_continue(&mut self, _: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(cx.w, can_sit_on)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx.w);
        m.extra.flag2 = false;
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.flag2 = false;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx.w);
        m.extra.flag2 = self.0.reached;
    }
}
/// `OcelotAttackGoal`: stalk slowly, dash in, swipe every second.
#[derive(Clone, Debug)]
struct OcelotAttack {
    attack_time: i32,
}
impl Goal for OcelotAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.ai.target.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(t) = m.ai.target.and_then(|t| cx.w.target_info(t)) else {
            return false;
        };
        t.alive
            && m.distance_sqr(t.position) <= 225.0
            && (!m.ai.nav.is_done() || m.ai.target.is_some())
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(who) = m.ai.target else {
            return;
        };
        let Some(t) = cx.w.target_info(who) else {
            return;
        };
        m.look_at_speed([t.position.x, t.eye_y, t.position.z], 30.0, 30.0);
        let reach = f64::from(m.width() * 2.0);
        let melee = reach * reach;
        let d2 = m.distance_sqr(t.position);
        let speed = if d2 > melee && d2 < 16.0 {
            1.33
        } else if d2 < 225.0 {
            0.6
        } else {
            0.8
        };
        m.nav_move_to_entity(cx.w, [t.position.x, t.position.y, t.position.z], 0, speed);
        self.attack_time = (self.attack_time - 1).max(0);
        if d2 <= melee && self.attack_time <= 0 {
            self.attack_time = 20;
            super::ai::do_hurt_target(m, who, cx);
        }
    }
}
/// `Cat.tick`: lying and relaxing amounts and the purr.
pub fn cat_ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    cat_amounts(m);
    if (m.extra.flag || m.extra.relax_one) && m.age.is_multiple_of(5) {
        let volume = 0.6 + 0.4 * (m.random.next_float() - m.random.next_float());
        let sound = super::finalize::variant_sound(m, "purr").unwrap_or("entity.cat.purr");
        events.push(MobEvent::Sound(sound, m.center(), volume, 1.0));
    }
}
/// The lying and relaxing amounts of `Cat.tick` (also a client's, from the
/// synched flags).
pub fn cat_amounts(m: &mut Mob) {
    let lying = m.extra.flag;
    let relaxing = m.extra.relax_one;
    let v = &mut m.anim.v;
    if lying {
        v[LIE] = (v[LIE] + 0.15).min(1.0);
        v[LIE_TAIL] = (v[LIE_TAIL] + 0.08).min(1.0);
    } else {
        v[LIE] = (v[LIE] - 0.22).max(0.0);
        v[LIE_TAIL] = (v[LIE_TAIL] - 0.13).max(0.0);
    }
    v[RELAX] = if relaxing {
        (v[RELAX] + 0.1).min(1.0)
    } else {
        (v[RELAX] - 0.13).max(0.0)
    };
}
/// `Cat.finalizeSpawn` variant: any but all-black, which a full moon allows.
pub fn cat_variant(m: &mut Mob, w_moon_full: bool) -> u8 {
    let n = if w_moon_full { 11 } else { 10 };
    m.random.next_int_bound(n).unwrap_or(0) as u8
}
pub fn cat_texture(m: &Mob) -> String {
    let v = CAT_VARIANTS[(m.extra.variant as usize).min(10)];
    let baby = if m.is_baby() { "_baby" } else { "" };
    format!("entity/cat/cat_{v}{baby}")
}
/// `CatRenderer.setupRotations`: lying cats roll onto their side.
pub fn cat_rotations(m: &Mob, partial: f32) -> glam::Mat4 {
    let lie = m.anim.lerp(LIE, partial);
    if lie <= 0.0 {
        return glam::Mat4::IDENTITY;
    }
    glam::Mat4::from_translation(glam::Vec3::new(0.4 * lie, 0.15 * lie, 0.1 * lie))
        * glam::Mat4::from_rotation_z((90.0 * lie).to_radians())
}
/// `Mth.rotLerp`: degrees-wrapping, applied by the feline models to radians.
fn rot_lerp(t: f32, from: f32, to: f32) -> f32 {
    from + t * super::living::wrap(to - from)
}
/// `FelineRenderState` values the feline models read.
#[derive(Clone, Copy, Debug, Default)]
pub struct Feline {
    pub crouching: bool,
    pub sprinting: bool,
    pub sitting: bool,
    pub lie_down: f32,
    pub lie_down_tail: f32,
    pub relax_one: f32,
}
/// `Cat.customServerAiStep` and `Ocelot.customServerAiStep`: crouch while
/// stalking (the move control's 0.6), sprint while dashing (1.33). Sprinting
/// adds `LivingEntity.SPEED_MODIFIER_SPRINTING` (`Mob::movement_speed`) to the
/// move control's tick that follows.
pub fn feline_server_ai_step(m: &mut Mob) {
    let speed = m.ai.mov.has_wanted().then_some(m.ai.mov.speed_modifier);
    m.extra.pose = if speed == Some(0.6) {
        super::sync::CROUCHING
    } else {
        super::sync::STANDING
    };
    m.extra.sprinting = speed == Some(1.33);
}
/// `AdultFelineModel` / `BabyFelineModel.setupAnim` for cats and ocelots.
pub fn cat_anim(m: &Mob, s: &State, p: &mut Poses) {
    // FelineRenderState: Entity.isCrouching (the pose) and isSprinting.
    let st = Feline {
        crouching: m.extra.pose == super::sync::CROUCHING,
        sprinting: m.extra.sprinting,
        sitting: cat_sitting(m),
        lie_down: m.anim.lerp(LIE, s.partial),
        lie_down_tail: m.anim.lerp(LIE_TAIL, s.partial),
        relax_one: m.anim.lerp(RELAX, s.partial),
    };
    feline(p, s, &st);
}
pub fn feline(p: &mut Poses, s: &State, st: &Feline) {
    let baby = s.baby;
    let age = if baby { 0.5 } else { 1.0 };
    let (crouching, sprinting, sitting) = (st.crouching, st.sprinting, st.sitting);
    let add = |p: &mut Poses, part: &str, x: f32, y: f32, z: f32| {
        let v = p.get(part);
        v[0] += x;
        v[1] += y;
        v[2] += z;
    };
    if crouching {
        add(p, "body", 0.0, age, 0.0);
        add(p, "head", 0.0, 2.0 * age, 0.0);
        add(p, "tail1", 0.0, age, 0.0);
        add(p, "tail2", 0.0, -4.0 * age, 2.0 * age);
        p.xrot("tail1", PI / 2.0);
        p.xrot("tail2", PI / 2.0);
    } else if sprinting {
        let y1 = p.get("tail1")[1];
        p.get("tail2")[1] = y1;
        add(p, "tail2", 0.0, 0.0, 2.0 * age);
        p.xrot("tail1", PI / 2.0);
        p.xrot("tail2", PI / 2.0);
    }
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    let (pos, sp) = (s.pos, s.speed);
    if !sitting {
        if !baby {
            p.xrot("body", PI / 2.0);
        }
        if sprinting {
            p.xrot("left_hind_leg", (pos * 0.6662).mcos() * sp);
            p.xrot("right_hind_leg", (pos * 0.6662 + 0.3).mcos() * sp);
            p.xrot("left_front_leg", (pos * 0.6662 + PI + 0.3).mcos() * sp);
            p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * sp);
            p.xrot("tail2", 1.727_876_1 + PI / 10.0 * pos.mcos() * sp);
        } else {
            p.xrot("left_hind_leg", (pos * 0.6662).mcos() * sp);
            p.xrot("right_hind_leg", (pos * 0.6662 + PI).mcos() * sp);
            p.xrot("left_front_leg", (pos * 0.6662 + PI).mcos() * sp);
            p.xrot("right_front_leg", (pos * 0.6662).mcos() * sp);
            let swing = if crouching { 0.471_238_94 } else { PI / 4.0 };
            p.xrot("tail2", 1.727_876_1 + swing * pos.mcos() * sp);
        }
    } else if baby {
        let x = p.get("body")[3] - 0.436_332_32;
        p.xrot("body", x);
        add(p, "body", 0.0, 1.25, 0.0);
        add(p, "head", 0.0, 0.0, 0.75);
        let t = p.get("tail1")[3] + 0.545_415_4;
        p.xrot("tail1", t);
        add(p, "tail1", 0.0, 4.0, -0.9);
        add(p, "left_hind_leg", 0.0, 0.0, -0.9);
        add(p, "right_hind_leg", 0.0, 0.0, -0.9);
    } else {
        p.xrot("body", PI / 4.0);
        add(p, "body", 0.0, -4.0 * age, 5.0 * age);
        add(p, "head", 0.0, -3.3 * age, age);
        add(p, "tail1", 0.0, 8.0 * age, -2.0 * age);
        add(p, "tail2", 0.0, 2.0 * age, -0.8 * age);
        p.xrot("tail1", 1.727_876_1);
        p.xrot("tail2", 2.670_354);
        p.xrot("left_front_leg", -PI / 20.0);
        add(p, "left_front_leg", 0.0, 2.0 * age, -2.0 * age);
        p.xrot("right_front_leg", -PI / 20.0);
        add(p, "right_front_leg", 0.0, 2.0 * age, -2.0 * age);
        p.xrot("left_hind_leg", -PI / 2.0);
        add(p, "left_hind_leg", 0.0, 3.0 * age, -4.0 * age);
        p.xrot("right_hind_leg", -PI / 2.0);
        add(p, "right_hind_leg", 0.0, 3.0 * age, -4.0 * age);
    }
    let lie = st.lie_down;
    let lie_tail = st.lie_down_tail;
    if lie > 0.0 && baby {
        add(p, "body", 1.0, 0.0, 0.0);
        let hx = p.get("head")[3];
        p.xrot("head", rot_lerp(lie, hx, PI / 18.0));
        let hz = p.get("head")[5];
        p.zrot("head", rot_lerp(lie, hz, -PI * 5.0 / 12.0));
        add(p, "head", 1.5, 0.75, -0.5);
        p.xrot("right_front_leg", -PI / 4.0);
        add(p, "right_front_leg", 3.5, -0.5, 0.0);
        p.xrot("left_front_leg", -PI / 2.0);
        add(p, "left_front_leg", 1.5, -1.0, -2.0);
        p.xrot("right_hind_leg", PI * 2.0 / 9.0);
        p.yrot("right_hind_leg", PI / 9.0);
        p.zrot("right_hind_leg", -PI / 9.0);
        add(p, "right_hind_leg", 2.5, -0.25, 0.5);
        add(p, "left_hind_leg", 1.5, 0.0, -1.0);
        let t = p.get("tail1");
        let (tx, ty, tz) = (t[3], t[4], t[5]);
        p.xrot("tail1", tx + rot_lerp(lie_tail, tx, -PI / 6.0));
        p.yrot("tail1", ty + rot_lerp(lie_tail, ty, 0.0));
        p.zrot("tail1", tz + rot_lerp(lie_tail, tz, -PI / 18.0));
        add(p, "tail1", 1.0, 0.5, -0.25);
    } else if lie > 0.0 {
        let (hz, hy) = (p.get("head")[5], p.get("head")[4]);
        p.zrot("head", rot_lerp(lie, hz, -1.270_796_3));
        p.yrot("head", rot_lerp(lie, hy, 1.270_796_3));
        p.xrot("left_front_leg", -1.270_796_3);
        p.xrot("right_front_leg", -0.470_796_35);
        p.zrot("right_front_leg", -0.2);
        add(p, "right_front_leg", age, 0.0, 0.0);
        p.xrot("left_hind_leg", -0.4);
        p.xrot("right_hind_leg", 0.5);
        p.zrot("right_hind_leg", -0.5);
        add(p, "right_hind_leg", 0.8 * age, 2.0 * age, 0.0);
        let (t1, t2) = (p.get("tail1")[3], p.get("tail2")[3]);
        p.xrot("tail1", rot_lerp(lie_tail, t1, 0.8));
        p.xrot("tail2", rot_lerp(lie_tail, t2, -0.4));
    }
    let relax = st.relax_one;
    if relax > 0.0 {
        let hx = p.get("head")[3];
        p.xrot("head", rot_lerp(relax, hx, -0.581_776_44));
    }
}
