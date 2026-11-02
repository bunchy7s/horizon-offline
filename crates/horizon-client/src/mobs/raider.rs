//! Raiders (`PatrollingMonster` and `Raider`): the patrol state every
//! pillager, vindicator, evoker, witch and ravager carries, the long
//! distance patrol and hold-ground goals, the ominous banner a patrol leader
//! wears, and the saved fields.
use super::{
    Mob, MobEvent, MobWorld, Species, Who,
    goals::{Cx, Goal, LOOK, MOVE, goal_clone},
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::nbt::Tag;

/// The ominous banner is a white banner with the raid patterns; raiders wear
/// it as their head item.
pub const BANNER: &str = "minecraft:white_banner";

/// `PatrollingMonster` and `Raider` state (saved as `patrol_target`,
/// `PatrolLeader`, `Patrolling`, `Wave`, `CanJoinRaid`, `RaidId`).
#[derive(Clone, Debug, Default)]
pub struct State {
    pub patrol_leader: bool,
    pub patrolling: bool,
    pub patrol_target: Option<[i32; 3]>,
    /// Goal cooldown after a failed long distance path (game time).
    pub patrol_cooldown_until: i64,
    pub wave: i32,
    pub can_join_raid: bool,
    /// The raid this raider belongs to.
    pub raid_id: Option<i32>,
    pub ticks_outside_raid: i32,
    pub celebrating: bool,
    /// The raid is active (`hasActiveRaid`); set by the game every tick.
    pub active_raid: bool,
    /// A raid is active within 96 blocks (`ServerLevel.isRaided`); set by
    /// the game every tick for vindicators.
    pub raided: bool,
    /// The raid is won or lost (`Raid.isOver`), lost (`isLoss`).
    pub raid_over: bool,
    pub raid_loss: bool,
    pub raid_center: [i32; 3],
    /// Standing in or beside a village (`ServerLevel.isVillage`).
    pub in_village: bool,
    /// A dropped ominous banner this raider may fetch.
    pub banner_at: Option<[f64; 3]>,
    /// The item entity of `banner_at`.
    pub banner_item: u32,
    /// Banners with no path to them, and until when (game time):
    /// `unreachableBannerCache`.
    pub unreachable_banners: Vec<(u32, i64)>,
    /// `PathfindToRaidGoal.tick` ran its 20 tick recruitment: the game
    /// recruits the raiders around this one.
    pub recruit_due: bool,
    /// Beds around the raid's centre.
    pub homes: Option<std::sync::Arc<Vec<[i32; 3]>>>,
    /// A ravager carrying a raider, set every tick from `Extra::rider`.
    pub rider: bool,
    /// The raid counts this raider (`addWaveMob` ran since it joined or was
    /// loaded with its `RaidId`).
    pub registered: bool,
    /// `Raider.hurtServer` updates the raid's bar before the damage lands:
    /// when it was last hurt (see [`next_hurt_seq`]) and its health then.
    pub hurt_seq: u64,
    pub pre_hurt_health: f32,
}

static HURT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A counter ordering the hurts of raiders across the game.
pub fn next_hurt_seq() -> u64 {
    HURT_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
}
/// `Raider.hurtServer`: a raider in an active raid asks its raid to update
/// the bar with the health it has before this hit.
pub fn note_hurt(m: &mut Mob) {
    let health = m.health;
    if let Some(s) = m.extra.raider.as_deref_mut()
        && s.raid_id.is_some()
        && s.active_raid
    {
        s.hurt_seq = next_hurt_seq();
        s.pre_hurt_health = health;
    }
}
impl State {
    /// A ravager carrying a raider (`hasControllingPassenger`).
    pub fn has_rider(&self) -> bool {
        self.rider
    }
}

/// `Raider.class`: the species a goal that takes `Raider.class` filters out
/// (`HurtByTargetGoal(this, Raider.class)`). A vex is not a raider.
pub const RAIDERS: [Species; 6] = [
    Species::Pillager,
    Species::Vindicator,
    Species::Evoker,
    Species::Illusioner,
    Species::Witch,
    Species::Ravager,
];

/// The raider species (`EntityTypeTags.RAIDERS`).
pub fn is_raider(s: Species) -> bool {
    RAIDERS.contains(&s)
}
/// `canBeLeader`: witches and ravagers never lead.
pub fn can_be_leader(s: Species) -> bool {
    is_raider(s) && !matches!(s, Species::Witch | Species::Ravager)
}
pub fn state(m: &Mob) -> Option<&State> {
    m.extra.raider.as_deref()
}
pub fn state_mut(m: &mut Mob) -> &mut State {
    m.extra.raider.get_or_insert_with(Box::default)
}
/// `Raider.getCurrentRaid() != null`.
pub fn in_raid(m: &Mob) -> bool {
    state(m).is_some_and(|s| s.raid_id.is_some())
}
pub fn has_active_raid(m: &Mob) -> bool {
    state(m).is_some_and(|s| s.active_raid)
}
/// `Raider.isCaptain`: the banner on the head of a patrol leader.
///
/// The worn stack (`extra.armor[0]`) carries the ominous components so a
/// client that only got `SetEquipment` (no `PatrolLeader` NBT) still draws
/// the patterned banner (#4567).
pub fn is_captain(m: &Mob) -> bool {
    if m.extra.head.as_deref() != Some(BANNER) {
        return false;
    }
    state(m).is_some_and(|s| s.patrol_leader)
        || m.extra.armor[0]
            .as_ref()
            .and_then(|s| s.data())
            .is_some_and(|d| d.ominous_banner)
}

/// `PatrollingMonster.registerGoals` then `Raider.registerGoals`: the goals
/// every raider registers before its own.
pub fn register(m: &mut Mob) {
    if !is_raider(m.species()) {
        return;
    }
    m.extra.raider = Some(Box::default());
    m.ai.goals.add(4, LongDistancePatrol::new(0.7, 0.595));
    m.ai.goals.add(1, ObtainBanner::default());
    m.ai.goals.add(3, PathfindToRaid::default());
    m.ai.goals.add(4, MoveThroughVillage::new(1.05, 1.0));
    m.ai.goals.add(5, Celebration);
}

/// `Raid.getOminousBannerInstance`.
pub fn ominous_banner() -> crate::items::ItemStack {
    let mut stack = crate::items::ItemStack::new(BANNER, 1);
    stack.data_mut().ominous_banner = true;
    stack
}

/// `PatrollingMonster.setPatrolLeader`.
pub fn set_patrol_leader(m: &mut Mob, leader: bool) {
    let s = state_mut(m);
    s.patrol_leader = leader;
    s.patrolling = true;
}
/// `PatrollingMonster.setPatrolTarget`.
pub fn set_patrol_target(m: &mut Mob, target: [i32; 3]) {
    let s = state_mut(m);
    s.patrol_target = Some(target);
    s.patrolling = true;
}
/// `PatrollingMonster.findPatrolTarget`: somewhere within 500 blocks.
pub fn find_patrol_target(m: &mut Mob) {
    let at = [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ];
    find_patrol_target_from(m, at);
}
/// `findPatrolTarget` from a given block position.
pub fn find_patrol_target_from(m: &mut Mob, [x, y, z]: [i32; 3]) {
    let dx = -500 + m.random.next_int_bound(1000).unwrap_or(0);
    let dz = -500 + m.random.next_int_bound(1000).unwrap_or(0);
    let s = state_mut(m);
    s.patrol_target = Some([x + dx, y, z + dz]);
    s.patrolling = true;
}
/// The leader's ominous banner and its drop chance of 2.0 (always dropped).
///
/// Also keeps the full ominous stack in `armor[0]` so `SetEquipment` sends
/// the patterned banner to clients (`Raid.getOminousBannerInstance`).
pub fn wear_banner(m: &mut Mob) {
    m.extra.head = Some(BANNER.into());
    m.extra.armor[0] = Some(ominous_banner());
}
/// Why a mob was spawned (`EntitySpawnReason`), as far as raiders care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Natural spawning: one leader in 17, and witches stay out of raids.
    Natural,
    /// Commands, eggs and other spawns: one leader in 17.
    Other,
    Patrol,
    Event,
}
/// `PatrollingMonster.finalizeSpawn` and `Raider.finalizeSpawn`: a spawn
/// other than a patrol, raid or structure leads one time in 17, a patrol
/// leader wears the ominous banner, and every raider but a naturally spawned
/// witch can join a raid.
pub fn finalize(m: &mut Mob, reason: Reason) {
    if !is_raider(m.species()) {
        return;
    }
    let roll = matches!(reason, Reason::Natural | Reason::Other);
    if roll && m.random.next_float() < 0.06 && can_be_leader(m.species()) {
        state_mut(m).patrol_leader = true;
    }
    if state_mut(m).patrol_leader {
        wear_banner(m);
    }
    let witch = m.species() == Species::Witch;
    let s = state_mut(m);
    if reason == Reason::Patrol {
        s.patrolling = true;
    }
    s.can_join_raid = !witch || reason != Reason::Natural;
}

/// `PatrollingMonster.removeWhenFarAway`: patrollers and raiders in a raid
/// are only despawned beyond 128 blocks (never inside a raid).
pub fn keeps_alive(m: &Mob, distance_squared: f64) -> bool {
    match state(m) {
        Some(s) => s.raid_id.is_some() || s.patrolling && distance_squared <= 16384.0,
        None => false,
    }
}

/// `Heightmap.Types.MOTION_BLOCKING_NO_LEAVES` position above (x, z).
pub fn heightmap_pos(w: &MobWorld, x: i32, z: i32) -> [i32; 3] {
    let top = w.world.top_y();
    let y = (w.world.bottom_y()..top)
        .rev()
        .find(|&y| {
            let f = w.facts.get(w.world.block(x, y, z));
            (f.full_collision || f.fluid != crate::blocks::Fluid::None)
                && !f.name.ends_with("leaves")
        })
        .map_or(w.world.bottom_y(), |y| y + 1);
    [x, y, z]
}

/// `Vec3.yRot` (the angle is in radians, and the patrol goal passes 90).
fn y_rot(v: Vec3, angle: f32) -> Vec3 {
    let (cos, sin) = (f64::from(angle.cos()), f64::from(angle.sin()));
    Vec3::new(v.x * cos + v.z * sin, v.y, v.z * cos - v.x * sin)
}

/// `PatrollingMonster.LongDistancePatrolGoal`: walk toward the patrol target
/// in 10 block hops (curving around it), the leader dragging its companions.
#[derive(Clone, Debug)]
pub struct LongDistancePatrol {
    speed: f64,
    leader_speed: f64,
}
impl LongDistancePatrol {
    pub fn new(speed: f64, leader_speed: f64) -> Self {
        Self {
            speed,
            leader_speed,
        }
    }
}
/// Patrol companions within 16 blocks that are not in a raid.
fn companions(m: &Mob, w: &MobWorld) -> Vec<u32> {
    w.others
        .iter()
        .filter(|s| {
            s.id != m.id
                && s.alive
                && is_raider(s.species)
                && !s.active_raid
                && (s.position.x - m.position.x).abs() <= 16.0 + f64::from(m.width())
                && (s.position.y - m.position.y).abs() <= 16.0 + f64::from(m.height())
                && (s.position.z - m.position.z).abs() <= 16.0 + f64::from(m.width())
        })
        .map(|s| s.id)
        .collect()
}
impl Goal for LongDistancePatrol {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some(s) = state(m) else { return false };
        cx.w.game_time >= s.patrol_cooldown_until
            && s.patrolling
            && m.ai.target.is_none()
            && s.patrol_target.is_some()
            && !s.has_rider()
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if !m.ai.nav.is_done() {
            return;
        }
        let leader = state(m).is_some_and(|s| s.patrol_leader);
        let friends = companions(m, cx.w);
        let Some(target) = state(m).and_then(|s| s.patrol_target) else {
            return;
        };
        if state(m).is_some_and(|s| s.patrolling) && friends.is_empty() {
            state_mut(m).patrolling = false;
        } else if leader && close_to_center(target, m.position, 10.0) {
            find_patrol_target(m);
        } else {
            let to = Vec3::new(
                f64::from(target[0]) + 0.5,
                f64::from(target[1]),
                f64::from(target[2]) + 0.5,
            );
            let own = m.position;
            let distance = own.subtract(to);
            let to = y_rot(distance, 90.0).scale(0.4).add_vector(to);
            let hop = to.subtract(own).normalize().scale(10.0).add_vector(own);
            let at = heightmap_pos(cx.w, hop.x.floor() as i32, hop.z.floor() as i32);
            let speed = if leader {
                self.leader_speed
            } else {
                self.speed
            };
            let to = [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])];
            if !m.nav_move_to(cx.w, to, speed) {
                self.move_randomly(m, cx);
                state_mut(m).patrol_cooldown_until = cx.w.game_time + 200;
            } else if leader {
                for id in friends {
                    cx.events.push(MobEvent::PatrolTarget {
                        mob: id,
                        target: at,
                    });
                }
            }
        }
    }
}
impl LongDistancePatrol {
    fn move_randomly(&self, m: &mut Mob, cx: &mut Cx) -> bool {
        let (x, z) = (
            m.position.x.floor() as i32 - 8 + m.random.next_int_bound(16).unwrap_or(0),
            m.position.z.floor() as i32 - 8 + m.random.next_int_bound(16).unwrap_or(0),
        );
        let at = heightmap_pos(cx.w, x, z);
        m.nav_move_to(
            cx.w,
            [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])],
            self.speed,
        )
    }
}
/// `Vec3i.closerToCenterThan`.
fn close_to_center(p: [i32; 3], at: Vec3, distance: f64) -> bool {
    Vec3::new(
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    )
    .distance_squared(at)
        < distance * distance
}

/// `Raider.ObtainRaidLeaderBannerGoal`: walk to a fallen ominous banner when
/// the wave has no living leader (the game finds the banner and picks it up).
#[derive(Clone, Debug, Default)]
pub struct ObtainBanner {
    path_to_banner: Option<super::path::Path>,
}
impl Goal for ObtainBanner {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let Some((at, item)) = state(m).and_then(|s| s.banner_at.map(|at| (at, s.banner_item)))
        else {
            return false;
        };
        // A banner with no path to it is skipped for 600 ticks.
        let path = m.create_path_to_entity(cx.w, at, 1);
        if path.as_ref().is_some_and(|p| p.reached) {
            self.path_to_banner = path;
            return true;
        }
        let now = cx.w.game_time;
        let s = state_mut(m);
        s.unreachable_banners.retain(|&(_, until)| now < until);
        s.unreachable_banners.push((item, now + 600));
        false
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        !m.ai.nav.is_done() && state(m).is_some_and(|s| s.banner_at.is_some())
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.nav_move_to_path(cx.w, self.path_to_banner.take(), 1.15);
    }
}

/// `PathfindToRaidGoal`: outside the village, head for the raid's centre in
/// hops of up to 15 blocks, recruiting nearby raiders every 20 ticks.
#[derive(Clone, Debug, Default)]
pub struct PathfindToRaid {
    recruitment_tick: i32,
}
impl PathfindToRaid {
    fn valid(s: &State) -> bool {
        s.active_raid && !s.raid_over && !s.in_village
    }
}
impl Goal for PathfindToRaid {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.ai.target.is_none() && state(m).is_some_and(|s| !s.has_rider() && Self::valid(s))
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        state(m).is_some_and(Self::valid)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(c) = state(m).map(|s| s.raid_center) else {
            return;
        };
        if m.age as i32 > self.recruitment_tick {
            self.recruitment_tick = m.age as i32 + 20;
            if let Some(s) = m.extra.raider.as_deref_mut() {
                s.recruit_due = true;
            }
        }
        if m.ai.nav.is_done() {
            let to = [
                f64::from(c[0]) + 0.5,
                f64::from(c[1]),
                f64::from(c[2]) + 0.5,
            ];
            if let Some(at) =
                m.default_random_pos_towards(cx.w, 15, 4, to, std::f64::consts::FRAC_PI_2)
            {
                m.nav_move_to(cx.w, at, 1.0);
            }
        }
    }
}

/// `Raider.RaiderMoveThroughVillageGoal`: walk to a bed of the village not
/// visited lately, then onwards to the next one.
#[derive(Clone, Debug)]
pub struct MoveThroughVillage {
    speed: f64,
    distance: f64,
    poi: Option<[i32; 3]>,
    visited: Vec<[i32; 3]>,
    stuck: bool,
}
impl MoveThroughVillage {
    pub fn new(speed: f64, distance: f64) -> Self {
        Self {
            speed,
            distance,
            poi: None,
            visited: Vec::new(),
            stuck: false,
        }
    }
}
impl Goal for MoveThroughVillage {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        if self.visited.len() > 2 {
            self.visited.remove(0);
        }
        let Some(s) = state(m) else { return false };
        if !s.active_raid || s.raid_over || m.ai.target.is_some() {
            return false;
        }
        let Some(homes) = s.homes.clone() else {
            return false;
        };
        let near: Vec<[i32; 3]> = homes
            .iter()
            .copied()
            .filter(|p| !self.visited.contains(p) && close_to_center(*p, m.position, 48.0))
            .collect();
        if near.is_empty() {
            return false;
        }
        self.poi = Some(near[m.random.next_int_bound(near.len() as i32).unwrap_or(0) as usize]);
        true
    }
    fn can_continue(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        let Some(p) = self.poi else { return false };
        !m.ai.nav.is_done()
            && m.ai.target.is_none()
            && !close_to_center(p, m.position, f64::from(m.width()) + self.distance)
            && !self.stuck
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.ai.no_action_time = 0;
        if let Some(p) = self.poi {
            m.nav_move_to(
                cx.w,
                [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])],
                self.speed,
            );
        }
        self.stuck = false;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        if let Some(p) = self.poi
            && close_to_center(p, m.position, self.distance)
        {
            self.visited.push(p);
        }
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(p) = self.poi else { return };
        if !m.ai.nav.is_done() {
            return;
        }
        let to = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        let next = m
            .default_random_pos_towards(cx.w, 16, 7, to, std::f64::consts::PI / 10.0)
            .or_else(|| m.default_random_pos_towards(cx.w, 8, 7, to, std::f64::consts::FRAC_PI_2));
        match next {
            Some(at) => {
                m.nav_move_to(cx.w, at, self.speed);
            }
            None => self.stuck = true,
        }
    }
}

/// `Raider.RaiderCelebration`: after a lost village, cheer and jump.
#[derive(Clone, Debug)]
pub struct Celebration;
impl Goal for Celebration {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.alive()
            && m.ai.target.is_none()
            && state(m).is_some_and(|s| s.raid_loss && s.raid_id.is_some())
    }
    fn start(&mut self, m: &mut Mob, _cx: &mut Cx) {
        state_mut(m).celebrating = true;
    }
    fn stop(&mut self, m: &mut Mob, _cx: &mut Cx) {
        state_mut(m).celebrating = false;
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if m.random
            .next_int_bound(super::goals::reduced(100))
            .unwrap_or(1)
            == 0
        {
            let sound = match m.species() {
                Species::Pillager => "entity.pillager.celebrate",
                Species::Vindicator => "entity.vindicator.celebrate",
                Species::Evoker => "entity.evoker.celebrate",
                Species::Ravager => "entity.ravager.celebrate",
                _ => "entity.witch.celebrate",
            };
            m.voice(sound, cx.events);
        }
        if m.random
            .next_int_bound(super::goals::reduced(50))
            .unwrap_or(1)
            == 0
        {
            m.ai.jumping = true;
        }
    }
}

/// `Raider.HoldGroundAttackGoal`: while patrolling, stand still and let the
/// patrol shout when something shows up; attack once it comes close.
#[derive(Clone, Debug)]
pub struct HoldGroundAttack {
    hostile_radius_sqr: f64,
}
impl HoldGroundAttack {
    pub fn new(radius: f64) -> Self {
        Self {
            hostile_radius_sqr: radius * radius,
        }
    }
}
/// Raiders within 8 blocks, nearest first (`shoutTargeting`).
fn shout_range(m: &Mob, w: &MobWorld) -> Vec<u32> {
    w.others
        .iter()
        .filter(|s| {
            s.id != m.id
                && s.alive
                && is_raider(s.species)
                && (s.position.x - m.position.x).abs() <= 8.0 + f64::from(m.width())
                && (s.position.y - m.position.y).abs() <= 8.0 + f64::from(m.height())
                && (s.position.z - m.position.z).abs() <= 8.0 + f64::from(m.width())
                && s.position.distance_squared(m.position) <= 64.0
        })
        .map(|s| s.id)
        .collect()
}
impl Goal for HoldGroundAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        let Some(s) = state(m) else { return false };
        s.raid_id.is_none()
            && s.patrolling
            && m.ai.target.is_some()
            && !m.ai.aggressive
            && m.ai.last_hurt_by != Some(Who::Player)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.ai.nav.stop();
        if let Some(target) = m.ai.target {
            for id in shout_range(m, cx.w) {
                cx.events.push(MobEvent::RaiderTarget {
                    mob: id,
                    target,
                    aggressive: false,
                });
            }
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        if let Some(target) = m.ai.target {
            for id in shout_range(m, cx.w) {
                cx.events.push(MobEvent::RaiderTarget {
                    mob: id,
                    target,
                    aggressive: true,
                });
            }
            m.ai.aggressive = true;
        }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(t) = m.target_info(cx.w) else { return };
        if m.distance_sqr(t.position) > self.hostile_radius_sqr {
            m.look_at([t.position.x, t.eye_y, t.position.z]);
            if m.random.next_int_bound(50).unwrap_or(1) == 0 {
                let sound = m.kind.sounds[0];
                if !sound.is_empty() {
                    m.voice(sound, cx.events);
                }
            }
        } else {
            m.ai.aggressive = true;
        }
    }
}

/// The saved fields (`PatrollingMonster` and `Raider.addAdditionalSaveData`).
pub fn write_nbt(m: &Mob, map: &mut horizon_world::nbt::Compound) {
    let Some(s) = state(m) else { return };
    match s.patrol_target {
        Some(p) => {
            map.insert("patrol_target".into(), Tag::IntArray(p.to_vec()));
        }
        None => {
            map.remove(&"patrol_target".into());
        }
    }
    map.insert("PatrolLeader".into(), Tag::Byte(i8::from(s.patrol_leader)));
    map.insert("Patrolling".into(), Tag::Byte(i8::from(s.patrolling)));
    map.insert("Wave".into(), Tag::Int(s.wave));
    map.insert("CanJoinRaid".into(), Tag::Byte(i8::from(s.can_join_raid)));
    match s.raid_id {
        Some(id) => {
            map.insert("RaidId".into(), Tag::Int(id));
        }
        None => {
            map.remove(&"RaidId".into());
        }
    }
}
pub fn read_nbt(m: &mut Mob, tag: &Tag) {
    if !is_raider(m.species()) {
        return;
    }
    let int = |t: Option<&Tag>| match t {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        Some(Tag::Short(v)) => Some(i32::from(*v)),
        _ => None,
    };
    let flag = |t: Option<&Tag>| int(t).unwrap_or(0) != 0;
    let s = state_mut(m);
    s.patrol_target = match tag.get("patrol_target") {
        Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
        _ => None,
    };
    s.patrol_leader = flag(tag.get("PatrolLeader"));
    s.patrolling = flag(tag.get("Patrolling"));
    s.wave = int(tag.get("Wave")).unwrap_or(0);
    s.can_join_raid = flag(tag.get("CanJoinRaid"));
    s.raid_id = int(tag.get("RaidId"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raiders_and_leaders() {
        assert!(is_raider(Species::Ravager) && is_raider(Species::Witch));
        assert!(!is_raider(Species::Zombie));
        assert!(can_be_leader(Species::Pillager) && can_be_leader(Species::Evoker));
        assert!(!can_be_leader(Species::Witch) && !can_be_leader(Species::Ravager));
    }
    #[test]
    fn wear_banner_keeps_the_ominous_stack_for_clients() {
        use crate::mobs::sensor_scan_tests::kind;
        use std::sync::Arc;
        let mut k = (*kind()).clone();
        k.id = "minecraft:pillager".into();
        k.species = Species::Pillager;
        let mut m = Mob::new(1, Arc::new(k), Vec3::ZERO, 0.0);
        m.extra.raider = Some(Box::default());
        state_mut(&mut m).patrol_leader = true;
        wear_banner(&mut m);
        assert_eq!(m.extra.head.as_deref(), Some(BANNER));
        assert!(
            m.extra.armor[0]
                .as_ref()
                .and_then(|s| s.data())
                .is_some_and(|d| d.ominous_banner)
        );
        assert!(is_captain(&m));
        // Client that only saw SetEquipment: no PatrolLeader, still a captain.
        state_mut(&mut m).patrol_leader = false;
        assert!(is_captain(&m));
        m.extra.armor[0] = None;
        assert!(!is_captain(&m));
    }
    #[test]
    fn long_distance_patrol_turns_by_ninety_radians() {
        // PatrollingMonster passes 90.0F to Vec3.yRot (radians).
        let v = y_rot(Vec3::new(10.0, 3.0, 0.0), 90.0);
        let (c, s) = (90f64.cos(), 90f64.sin());
        assert!((v.x - 10.0 * c).abs() < 1e-4 && (v.z + 10.0 * s).abs() < 1e-4);
        assert_eq!(v.y, 3.0);
    }
    #[test]
    fn closer_to_center_uses_block_centres() {
        let at = Vec3::new(10.5, 64.5, 10.5);
        assert!(close_to_center([10, 64, 10], at, 1.0));
        assert!(!close_to_center([30, 64, 10], at, 10.0));
    }
}
