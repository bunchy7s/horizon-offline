//! Piglins, piglin brutes, hoglins and zoglins: gold-wearing players are
//! left alone, gold is admired and bartered, piglins and hoglins zombify
//! outside the Nether, hoglins headbutt their targets into the air.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, HurtByTarget, LOOK, LookAtPlayer, MOVE, MeleeAttack,
        NearestTarget, Prey, RandomLookAround, Stroll, TARGET, TargetState, goal_clone,
        target_continue,
    },
};
use horizon_core::geometry::{Vec3, trig::Mth};
use horizon_core::random::{LegacyRandom, RandomSource};
use std::f32::consts::PI;

/// `PiglinAi.ADMIRE_DURATION`.
pub const ADMIRE: i32 = 119;
/// `PiglinAi.eat`: the `ATE_RECENTLY` memory lasts 200 ticks (kept in
/// `extra.ints[0]` of a piglin).
pub const ATE_RECENTLY: i32 = 200;
/// `AbstractPiglin.CONVERSION_TIME` (and the hoglin's).
const CONVERSION: i32 = 300;
/// `PiglinAi.CELEBRATION_TIME`: how long the `CELEBRATE_LOCATION` and
/// `DANCING` memories last.
const CELEBRATION: i32 = 300;

fn hostile_to_player(m: &Mob, w: &MobWorld) -> bool {
    if m.is_baby() {
        return false;
    }
    match m.species() {
        Species::Piglin => m.extra.anger > 0 || w.player.as_ref().is_some_and(|p| !p.gold_armor),
        _ => true,
    }
}
pub fn register_piglin(m: &mut Mob) {
    let crossbow = m.extra.main_hand.as_deref() == Some("minecraft:crossbow");
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, Admire);
    g.add(
        2,
        AvoidEntity::new(
            Avoid::Species(vec![Species::ZombifiedPiglin, Species::Zoglin]),
            6.0,
            1.0,
            1.2,
        ),
    );
    if crossbow {
        g.add(3, super::illager::crossbow_goal(1.0, 8.0));
    }
    g.add(3, MeleeAttack::new(1.0, true));
    g.add(4, PiglinAvoidRepellent);
    g.add(4, GoToWantedItem);
    g.add(4, Celebrate);
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    g.add(7, RandomLookAround::default());
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets
        .add(2, NearestTarget::player(true).when(hostile_to_player));
    m.ai.targets.add(3, HuntHoglin::default());
    // `PiglinAi.initMemories`: the first hunt waits too.
    m.extra.ints[0] = time_between_hunts(m);
}
/// `PiglinAi.TIME_BETWEEN_HUNTS`: 30 to 120 seconds.
fn time_between_hunts(m: &mut Mob) -> i32 {
    m.random
        .next_int_between_inclusive(600, 2400)
        .unwrap_or(600)
}
/// `StartHuntingHoglin`: an adult piglin that is not angry, has not hunted
/// recently and sees no adult piglin that has, goes for the nearest adult
/// hoglin it sees; then it and the adult piglins near it wait before the next
/// hunt, and the others join in (`broadcastAngerTarget`).
#[derive(Clone, Debug)]
struct HuntHoglin {
    found: Option<u32>,
    st: TargetState,
}
impl Default for HuntHoglin {
    fn default() -> Self {
        let mut st = TargetState::default();
        st.unseen_memory = 60;
        Self { found: None, st }
    }
}
impl Goal for HuntHoglin {
    goal_clone!();
    fn flags(&self) -> u8 {
        TARGET
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.is_baby() || m.extra.ints[0] > 0 || m.extra.anger > 0 || m.ai.target.is_some() {
            return false;
        }
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        if cx
            .w
            .nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby && s.hunted_recently
            })
            .next()
            .is_some()
        {
            return false;
        }
        let hoglins: Vec<(u32, Vec3)> =
            cx.w.nearby(pos, range, |s| s.species == Species::Hoglin && !s.baby)
                .map(|s| (s.id, s.position))
                .collect();
        self.found = hoglins
            .into_iter()
            .find(|&(hoglin, at)| {
                m.distance_sqr(at) <= range * range && m.sees(cx.w, Who::Mob(hoglin))
            })
            .map(|(hoglin, _)| hoglin);
        self.found.is_some()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        target_continue(&mut self.st, true, m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(hoglin) = self.found else { return };
        m.ai.target = Some(Who::Mob(hoglin));
        m.extra.ints[0] = time_between_hunts(m);
        let range = m.attrs.follow_range;
        let pos = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let id = m.id;
        let others: Vec<u32> =
            cx.w.nearby(pos, range, |s| {
                s.id != id && s.species == Species::Piglin && !s.baby
            })
            .map(|s| s.id)
            .collect();
        for mob in others {
            let ticks = time_between_hunts(m);
            cx.events.push(MobEvent::HuntHoglin {
                mob,
                target: hoglin,
                ticks,
            });
        }
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.ai.target = None;
    }
}
pub fn register_brute(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(2, MeleeAttack::new(1.0, true));
    g.add(5, Stroll::avoiding_water(0.6));
    g.add(6, LookAtPlayer::new(8.0));
    let mut hurt = HurtByTarget::new().alert();
    hurt.ignore = vec![Species::Piglin, Species::PiglinBrute];
    m.ai.targets.add(1, hurt);
    m.ai.targets.add(2, NearestTarget::player(true));
    m.ai.targets
        .add(3, NearestTarget::species(&[Species::WitherSkeleton], true));
}
pub fn register_hoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    g.add(1, BabyPanic(super::goals::Panic::new(1.25)));
    // HoglinAi: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(3, super::goals::Breed::new(1.0));
    g.add(3, HoglinAvoidRepellent);
    g.add(4, super::goals::FollowParent::new(1.1));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new().alert());
    // `HoglinAi.findNearestValidAttackTarget`: nothing while pacified.
    m.ai.targets.add(
        2,
        NearestTarget::player(true).when(|m, _| !m.is_baby() && m.extra.ints[0] <= 0),
    );
}
pub fn register_zoglin(m: &mut Mob) {
    let g = &mut m.ai.goals;
    g.add(0, Float);
    // Zoglin.initMemories: MeleeAttack.create(40) for adults, 15 for babies.
    g.add(2, MeleeAttack::new(1.0, true).intervals(40, 15));
    g.add(5, Stroll::avoiding_water(0.4));
    g.add(6, LookAtPlayer::new(8.0));
    m.ai.targets.add(1, HurtByTarget::new());
    m.ai.targets.add(2, NearestTarget::player(true));
    // Zoglin.isTargetable: anything but creepers and other zoglins.
    m.ai.targets.add(
        3,
        NearestTarget::new(
            Prey::Filter(|s| !matches!(s.species, Species::Creeper | Species::Zoglin)),
            true,
        )
        .interval(10),
    );
}
/// `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, true)` of the hoglin's
/// idle activity: closer than 8 blocks to a repellent, walk away from it.
#[derive(Clone, Debug)]
struct HoglinAvoidRepellent;
impl Goal for HoglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(path, 1.0);
    }
}
/// Baby hoglins flee what hurts them; adults stand their ground.
#[derive(Clone, Debug)]
struct BabyPanic(super::goals::Panic);
impl Goal for BabyPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        m.is_baby() && self.0.can_use(m, cx)
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
}
/// `PiglinAi.avoidRepellent`: `SetWalkTargetAwayFrom.pos(NEAREST_REPELLENT, 1.0, 8, false)`
/// of the idle activity. Closer than 8 blocks to a repellent and not walking
/// anywhere, walk away from it.
#[derive(Clone, Debug)]
struct PiglinAvoidRepellent;
impl Goal for PiglinAvoidRepellent {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(r) = m.extra.repellent else {
            return false;
        };
        // The idle activity: not fighting or admiring, and no walk target yet.
        if m.ai.target.is_some() || m.extra.using_item > 0 || !m.ai.nav.is_done() {
            return false;
        }
        // Vec3.atBottomCenterOf.
        let avoid = [
            f64::from(r[0]) + 0.5,
            f64::from(r[1]),
            f64::from(r[2]) + 0.5,
        ];
        if m.distance_sqr(Vec3::new(avoid[0], avoid[1], avoid[2])) >= 64.0 {
            return false;
        }
        for _ in 0..10 {
            if let Some(p) = m.land_random_pos_away(cx.w, 16, 7, avoid) {
                let at = [
                    p[0].floor() as i32,
                    p[1].floor() as i32,
                    p[2].floor() as i32,
                ];
                m.ai.pending_path = m.create_path_to(cx.w, at, 0);
                return m.ai.pending_path.is_some();
            }
        }
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        !m.ai.nav.is_done()
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        let path = m.ai.pending_path.take();
        m.nav_move_to_path(path, 1.0);
    }
}
/// Admiring a gold item held in the off hand: stand and look at it.
#[derive(Clone, Debug)]
struct Admire;
impl Goal for Admire {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.using_item > 0
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
    }
}
/// The celebrate activity: `GoToTargetLocation` on the spot a killed target
/// fell (within 2 blocks, or 4 and slower while dancing).
#[derive(Clone, Debug)]
struct Celebrate;
impl Celebrate {
    fn walk(m: &mut Mob, cx: &mut Cx) {
        let Some(at) = m.extra.home else { return };
        let (close, speed) = if m.extra.scratch[0] > 0 {
            (4.0, 0.6)
        } else {
            (2.0, 1.0)
        };
        let center = Vec3::new(
            f64::from(at[0]) + 0.5,
            f64::from(at[1]),
            f64::from(at[2]) + 0.5,
        );
        if m.distance_sqr(center) >= close * close {
            m.nav_move_to(cx.w, [center.x, center.y, center.z], speed);
        }
    }
}
impl Goal for Celebrate {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.scratch[1] > 0 && m.extra.home.is_some() && m.extra.using_item == 0
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        Self::walk(m, cx);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.ai.nav.is_done() {
            Self::walk(m, cx);
        }
    }
}
/// `GoToWantedItem`: walk to the gold the game spotted.
#[derive(Clone, Debug)]
struct GoToWantedItem;
impl Goal for GoToWantedItem {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(p) = m.extra.point else {
            return false;
        };
        m.extra.using_item == 0 && m.nav_move_to(cx.w, p, 1.0)
    }
    fn can_continue(&mut self, m: &mut Mob, _: &mut Cx) -> bool {
        m.extra.point.is_some() && !m.ai.nav.is_done()
    }
    fn stop(&mut self, m: &mut Mob, _: &mut Cx) {
        m.extra.point = None;
    }
}
/// `HoglinSpecificSensor.findNearestRepellent`: the nearest `#hoglin_repellents`
/// block within 8 blocks sideways and 4 up or down.
fn find_hoglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:hoglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |_, name| tag.contains(name))
}
/// The hoglin's repellent memories: the sensor (every 20 ticks), the end of
/// `PACIFIED` (`ints[0]`), a hit ending it (`HoglinAi.wasHurtBy`, `ints[1]`
/// remembers the hit seen) and `BecomePassiveIfMemoryPresent(NEAREST_REPELLENT, 200)`.
fn hoglin_repellent(m: &mut Mob, w: &MobWorld) {
    if m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    if m.ai.last_hurt_by.is_some() && m.ai.last_hurt_by_time != m.extra.ints[1] {
        m.extra.ints[1] = m.ai.last_hurt_by_time;
        m.extra.ints[0] = 0;
    }
    if m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_hoglin_repellent(m, w);
    }
    if m.extra.ints[0] <= 0 && m.extra.repellent.is_some() {
        m.extra.ints[0] = 200;
        m.ai.target = None;
    }
}
/// `StartCelebratingIfTargetDead` for a piglin (`scratch[0]` dancing,
/// `scratch[1]` ticks of celebration left, `scratch[2]` the hit seen, `home`
/// the `CELEBRATE_LOCATION`): a killed target starts a celebration at its
/// spot, and a hoglin's death makes the piglin dance one time in ten.
/// `PiglinAi.wasHurtBy` ends both. Runs before the target goals drop the
/// dead target.
pub fn celebrate(m: &mut Mob, w: &MobWorld) {
    if m.extra.scratch[1] > 0 {
        m.extra.scratch[1] -= 1;
        if m.extra.scratch[1] == 0 {
            m.extra.scratch[0] = 0;
            m.extra.home = None;
        }
    }
    if let Some(by) = m.ai.last_hurt_by
        && m.ai.last_hurt_by_time != m.extra.scratch[2]
    {
        m.extra.scratch[2] = m.ai.last_hurt_by_time;
        // PiglinAi.wasHurtBy: another piglin's hit ends nothing.
        let piglin =
            matches!(by, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Piglin));
        if !piglin {
            m.extra.scratch[0] = 0;
            m.extra.scratch[1] = 0;
            m.extra.home = None;
        }
    }
    if m.extra.scratch[1] > 0 {
        return;
    }
    let Some(target) = m.ai.target else { return };
    let Some(t) = w.target_info(target).filter(|t| !t.alive) else {
        return;
    };
    let hoglin =
        matches!(target, Who::Mob(id) if w.seen(id).is_some_and(|s| s.species == Species::Hoglin));
    // PiglinAi.wantsToDance: a fresh random seeded with the game time, not
    // the piglin's own.
    if hoglin && LegacyRandom::new(w.game_time).next_float() < 0.1 {
        m.extra.scratch[0] = 1;
    }
    m.extra.scratch[1] = CELEBRATION;
    m.extra.home = Some([
        t.position.x.floor() as i32,
        t.position.y.floor() as i32,
        t.position.z.floor() as i32,
    ]);
}
/// `Hoglin.isConverting` and `AbstractPiglin.isConverting` on a client: in a
/// dimension that is not piglin safe (`nether` is false), a mob that is not
/// immune (`flag2`) and has AI shakes. `counter2 > 0` draws the shake, as it
/// does on the server while the zombification counts up.
pub fn client_converting(m: &mut Mob, nether: bool) {
    m.extra.counter2 = i32::from(!nether && !m.extra.flag2 && !m.no_ai);
}
/// Per-tick piglin and hoglin state: admiring, anger, zombification and the
/// hoglin attack animation. Returns a conversion when it happens.
pub fn ai_step(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    if m.species() == Species::Hoglin && m.alive() {
        hoglin_repellent(m, w);
    }
    if m.species() == Species::Piglin {
        if m.extra.ints[0] > 0 {
            m.extra.ints[0] -= 1;
        }
        // `StopAttackingIfTargetInvalid`: a hoglin handed over by a hunt
        // broadcast has no target goal of its own to drop it when it dies.
        if let Some(who @ Who::Mob(_)) = m.ai.target
            && w.target_info(who)
                .is_none_or(|t| !t.alive || !t.can_be_seen_as_enemy)
        {
            m.ai.target = None;
        }
    }
    if m.extra.anger > 0 {
        m.extra.anger -= 1;
        if m.extra.anger == 0 && m.ai.target == Some(Who::Player) && !hostile_to_player(m, w) {
            m.ai.target = None;
        }
    }
    if m.species() == Species::Piglin && m.extra.ints[0] > 0 {
        m.extra.ints[0] -= 1;
    }
    // Admiring disabled after being hit by a player.
    if m.extra.egg_time > 0 {
        m.extra.egg_time -= 1;
    }
    if m.extra.using_item > 0 {
        m.extra.using_item -= 1;
        if m.extra.using_item == 0 {
            events.push(MobEvent::FinishAdmiring { mob: m.id });
        }
    }
    // Zombification outside the Nether (piglins without immunity, hoglins).
    if !w.nether && !m.extra.flag2 && m.alive() {
        m.extra.counter2 += 1;
        if m.extra.counter2 > CONVERSION {
            let (kind, sound) = match m.species() {
                Species::Hoglin => ("minecraft:zoglin", "entity.hoglin.converted_to_zombified"),
                Species::PiglinBrute => (
                    "minecraft:zombified_piglin",
                    "entity.piglin_brute.converted_to_zombified",
                ),
                _ => (
                    "minecraft:zombified_piglin",
                    "entity.piglin.converted_to_zombified",
                ),
            };
            events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
            events.push(MobEvent::SpawnKind {
                kind: kind.into(),
                at: [m.position.x, m.position.y, m.position.z],
                setup: super::SpawnSetup {
                    baby: m.is_baby(),
                    yaw: m.yaw,
                    persistent: m.persistent,
                    nausea: true,
                    ..Default::default()
                },
            });
            m.removed = true;
        }
    } else {
        m.extra.counter2 = 0;
    }
    // PiglinSpecificSensor (every 20 ticks): the nearest repelling block.
    if m.species() == Species::Piglin && m.alive() && m.age.wrapping_add(m.id).is_multiple_of(20) {
        m.extra.repellent = find_piglin_repellent(m, w);
    }
}
/// `PiglinSpecificSensor.findNearestRepellent`: the nearest `#piglin_repellents`
/// block within 8 blocks sideways and 4 up or down; a soul campfire only
/// counts while lit.
fn find_piglin_repellent(m: &Mob, w: &MobWorld) -> Option<[i32; 3]> {
    let tag = w.block_tags.get("minecraft:piglin_repellents")?;
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    w.nearest_block(at, 8, 4, |p, name| {
        tag.contains(name)
            && (name != "minecraft:soul_campfire"
                || w.prop(p[0], p[1], p[2], "lit") == Some("true"))
    })
}
/// `HoglinBase.throwTarget`: a shove away and up of the attack knockback less
/// the target's knockback resistance (none at all when that is used up).
fn throw_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    let resistance = match who {
        Who::Mob(id) => cx.w.seen(id).map_or(0.0, |s| s.knockback_resistance),
        Who::Player => 0.0,
    };
    let power = m.kind.attack_knockback - resistance;
    let Some(t) = cx.w.target_info(who) else {
        return;
    };
    if power <= 0.0 {
        return;
    }
    let away = Vec3::new(
        t.position.x - m.position.x,
        0.0,
        t.position.z - m.position.z,
    );
    // `yRot` takes the random whole number of degrees as radians, as vanilla does.
    let angle = (m.random.next_int_bound(21).unwrap_or(10) - 10) as f32;
    let scale = power * f64::from(m.random.next_float() * 0.5 + 0.2);
    let push = away.normalize().scale(scale).rotate_y(angle);
    let up = power * f64::from(m.random.next_float()) * 0.5;
    cx.events.push(MobEvent::Push {
        target: who,
        velocity: [push.x, up, push.z],
    });
}
/// `HoglinBase.hurtAndThrowTarget`.
pub fn hoglin_hurt_target(m: &mut Mob, who: Who, cx: &mut Cx) {
    m.extra.anim_ticks = 10;
    // Entity event 4: clients start the headbutt.
    cx.events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: [m.position.x, m.position.y, m.position.z],
    });
    let attack = m.attrs.attack_damage;
    let amount = if !m.is_baby() && attack as i32 > 0 {
        attack / 2.0 + m.random.next_int_bound(attack as i32).unwrap_or(0) as f32
    } else {
        attack
    };
    let from = [m.position.x, m.position.y, m.position.z];
    match who {
        Who::Player => cx.events.push(MobEvent::AttackPlayer(amount, from)),
        Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
            target: id,
            amount,
            source: super::Damage::mob(m.id, from),
        }),
    }
    if !m.is_baby() {
        throw_target(m, who, cx);
    }
    let sound = if m.species() == Species::Zoglin {
        "entity.zoglin.attack"
    } else {
        "entity.hoglin.attack"
    };
    cx.events.push(MobEvent::Sound(sound, m.center(), 1.0, 1.0));
}
/// `AbstractPiglinModel.setupAnim`: the ears flap around their default angle
/// (30 degrees for adults, 5 for babies).
pub fn ears(p: &mut Poses, s: &State) {
    use horizon_core::geometry::trig::DEG_TO_RAD;
    let default = if s.baby { 5.0 } else { 30.0 } * DEG_TO_RAD;
    let freq = s.age * 0.1 + s.pos * 0.5;
    let amp = 0.08 + s.speed * 0.4;
    p.zrot("left_ear", -default - (freq * 1.2).mcos() * amp);
    p.zrot("right_ear", default + freq.mcos() * amp);
}
/// Piglins' and brutes' ears flap (`AbstractPiglinModel`), arms follow the
/// arm pose (`PiglinModel`).
pub fn piglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    // Piglin.getArmPose: dancing comes first.
    let pose = if m.extra.scratch[0] > 0 {
        PiglinPose::Dancing
    } else if m.extra.using_item > 0 {
        PiglinPose::AdmiringItem
    } else if m.extra.flag {
        PiglinPose::CrossbowCharge
    } else if m.extra.main_hand.as_deref() == Some("minecraft:crossbow") && m.ai.aggressive {
        PiglinPose::CrossbowHold
    } else if m.ai.aggressive && m.extra.main_hand.is_some() {
        PiglinPose::MeleeWeapon
    } else {
        PiglinPose::Default
    };
    piglin_pose(
        p,
        s,
        pose,
        25.0,
        m.extra.using_item as f32,
        m.main_arm_left(),
    );
}
/// `PiglinArmPose`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PiglinPose {
    Default,
    Dancing,
    MeleeWeapon,
    CrossbowHold,
    CrossbowCharge,
    AdmiringItem,
}
/// `PiglinModel.setupAnim`; `left` when the piglin's main arm is the left one.
pub fn piglin_pose(
    p: &mut Poses,
    s: &State,
    pose: PiglinPose,
    max_charge: f32,
    ticks: f32,
    left: bool,
) {
    use super::anim::{ArmPose, Hum, crossbow_charge, crossbow_hold, humanoid};
    let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
    h.weapon_swing = pose == PiglinPose::MeleeWeapon;
    h.left_main = left;
    h.charge = (max_charge, ticks);
    humanoid(p, s, &h);
    ears(p, s);
    let (hy, hx) = (s.y_rot.to_radians(), s.x_rot.to_radians());
    let rad = |v: f64| v as f32;
    let deg = horizon_core::geometry::trig::DEG_TO_RAD;
    match pose {
        PiglinPose::Dancing => {
            let dance = s.age / 60.0;
            p.zrot(
                "right_ear",
                rad(std::f64::consts::PI / 6.0) + deg * (dance * 30.0).msin() * 10.0,
            );
            p.zrot(
                "left_ear",
                rad(-std::f64::consts::PI / 6.0) - deg * (dance * 30.0).mcos() * 10.0,
            );
            p.get("head")[0] += (dance * 10.0).msin();
            p.get("head")[1] += (dance * 40.0).msin() + 0.4;
            let z = deg * (70.0 + (dance * 40.0).mcos() * 10.0);
            p.zrot("right_arm", z);
            p.zrot("left_arm", -z);
            p.get("right_arm")[1] += (dance * 40.0).msin() * 0.5 - 0.5;
            p.get("left_arm")[1] += (dance * 40.0).msin() * 0.5 + 0.5;
            p.get("body")[1] += (dance * 40.0).msin() * 0.35;
        }
        // holdWeaponHigh
        PiglinPose::MeleeWeapon if s.swing == 0.0 => {
            p.xrot(if left { "left_arm" } else { "right_arm" }, -1.8)
        }
        PiglinPose::CrossbowHold => crossbow_hold(p, hx, hy, !left),
        PiglinPose::CrossbowCharge => crossbow_charge(p, max_charge, ticks, !left),
        PiglinPose::AdmiringItem => {
            p.xrot("head", 0.5);
            p.yrot("head", 0.0);
            if left {
                p.yrot("right_arm", -0.5);
                p.xrot("right_arm", -0.9);
            } else {
                p.yrot("left_arm", 0.5);
                p.xrot("left_arm", -0.9);
            }
        }
        _ => {}
    }
}
/// `Hoglin.handleEntityEvent` and `Zoglin`'s event 4 on a client: the
/// headbutt, which `aiStep` then counts down.
pub fn client_event(m: &mut Mob, event: u8) {
    if event == 4 {
        m.extra.anim_ticks = 10;
    }
}
/// The headbutt's countdown (`Hoglin.aiStep`, `Zoglin.aiStep`) on a client.
pub fn client_tick(m: &mut Mob) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
}
/// `HoglinModel.setupAnim`.
pub fn hoglin_anim(m: &Mob, s: &State, p: &mut Poses) {
    let (pos, sp) = (s.pos, s.speed);
    p.zrot("right_ear", -PI * 2.0 / 9.0 - sp * pos.msin());
    p.zrot("left_ear", PI * 2.0 / 9.0 + sp * pos.msin());
    p.yrot("head", s.y_rot.to_radians());
    let remaining = m.extra.anim_ticks as f32;
    let f = 1.0 - (10.0 - 2.0 * remaining).abs() / 10.0;
    p.xrot("head", 0.872_664_63 + (-PI / 9.0 - 0.872_664_63) * f);
    if s.baby {
        p.get("head")[1] += f * 2.5;
    }
    p.xrot("right_front_leg", pos.mcos() * 1.2 * sp);
    p.xrot("left_front_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("right_hind_leg", (pos + PI).mcos() * 1.2 * sp);
    p.xrot("left_hind_leg", pos.mcos() * 1.2 * sp);
}
