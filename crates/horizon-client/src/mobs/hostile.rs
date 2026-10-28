//! Creepers (swelling, explosions) and spiders (wall climbing, daylight
//! neutrality, leaping).
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    goals::{
        Avoid, AvoidEntity, Cx, Float, Goal, HurtByTarget, LeapAtTarget, LookAtPlayer, MOVE,
        MeleeAttack, NearestTarget, RandomLookAround, Stroll, goal_clone,
    },
};
use horizon_core::random::RandomSource;

pub fn register(m: &mut Mob) {
    use Species as S;
    match m.species() {
        S::Creeper => {
            let g = &mut m.ai.goals;
            g.add(1, Float);
            g.add(2, Swell::default());
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Ocelot]), 6.0, 1.0, 1.2),
            );
            g.add(
                3,
                AvoidEntity::new(Avoid::Species(vec![S::Cat]), 6.0, 1.0, 1.2),
            );
            g.add(4, MeleeAttack::new(1.0, false));
            g.add(5, Stroll::avoiding_water(0.8));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            m.ai.targets.add(1, NearestTarget::player(true));
            m.ai.targets.add(2, HurtByTarget::new());
            // Creeper DATA_SWELL_DIR starts at -1.
            m.extra.counter = -1;
        }
        S::Spider | S::CaveSpider => {
            let g = &mut m.ai.goals;
            g.add(1, Float);
            g.add(
                2,
                AvoidEntity::new(Avoid::Species(vec![S::Armadillo]), 6.0, 1.0, 1.2),
            );
            g.add(3, LeapAtTarget::new(0.4));
            g.add(4, SpiderAttack(MeleeAttack::new(1.0, true)));
            g.add(5, Stroll::avoiding_water(0.8));
            g.add(6, LookAtPlayer::new(8.0));
            g.add(6, RandomLookAround::default());
            m.ai.targets.add(1, HurtByTarget::new());
            m.ai.targets
                .add(2, NearestTarget::player(true).gate(dark_enough));
            m.ai.targets.add(
                3,
                NearestTarget::species(&[S::IronGolem], true).gate(dark_enough),
            );
            m.ai.nav.kind = super::nav::NavKind::WallClimber;
        }
        S::Silverfish => {
            let g = &mut m.ai.goals;
            g.add(1, Float);
            g.add(3, SilverfishWakeUpFriends);
            g.add(4, MeleeAttack::new(1.0, false));
            g.add(5, SilverfishMergeWithStone::default());
            m.ai.targets.add(1, HurtByTarget::new().alert());
            m.ai.targets.add(2, NearestTarget::player(true));
        }
        _ => {}
    }
}
/// `Silverfish.hurtServer`: a hit by an entity (or magic damage) starts the
/// countdown of its wake-up goal, kept in `extra.counter`.
pub fn silverfish_hurt(m: &mut Mob, source: &Damage) {
    if (source.attacker.is_some() || source.kind == DamageKind::Magic) && m.extra.counter == 0 {
        m.extra.counter = super::goals::reduced(20);
    }
}
/// `Silverfish.SilverfishWakeUpFriendsGoal`: 20 ticks after being hurt, the
/// infested blocks within 10 blocks sideways and 5 up or down break (or turn
/// back into stone with mob griefing off), until a coin flip stops it.
#[derive(Clone, Debug)]
struct SilverfishWakeUpFriends;
impl Goal for SilverfishWakeUpFriends {
    goal_clone!();
    fn name(&self) -> &'static str {
        "SilverfishWakeUpFriendsGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        m.extra.counter > 0
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.counter -= 1;
        if m.extra.counter > 0 {
            return;
        }
        let base = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        // The loops' offsets go 0, 1, -1, 2, -2 and so on.
        let offsets = |limit: i32| std::iter::once(0).chain((1..=limit).flat_map(|i| [i, -i]));
        for y in offsets(5) {
            for x in offsets(10) {
                for z in offsets(10) {
                    let pos = [base[0] + x, base[1] + y, base[2] + z];
                    if !cx
                        .w
                        .block_name(pos[0], pos[1], pos[2])
                        .starts_with("minecraft:infested_")
                    {
                        continue;
                    }
                    if cx.w.mob_griefing {
                        cx.events.push(MobEvent::BreakBlock { pos, drop: true });
                    } else {
                        cx.events.push(MobEvent::Infest {
                            pos,
                            infested: false,
                            poof: None,
                        });
                    }
                    if m.random.next_boolean() {
                        return;
                    }
                }
            }
        }
    }
}
/// `Silverfish.SilverfishMergeWithStoneGoal`: a stroll that, now and then
/// with mob griefing on, hides in a stone block next to the silverfish.
#[derive(Clone, Debug)]
struct SilverfishMergeWithStone {
    stroll: Stroll,
    merge: Option<[i32; 3]>,
}
impl Default for SilverfishMergeWithStone {
    fn default() -> Self {
        Self {
            stroll: Stroll::new(1.0).interval(10),
            merge: None,
        }
    }
}
impl Goal for SilverfishMergeWithStone {
    goal_clone!();
    fn name(&self) -> &'static str {
        "SilverfishMergeWithStoneGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if m.ai.target.is_some() || !m.ai.nav.is_done() {
            return false;
        }
        if cx.w.mob_griefing
            && m.random
                .next_int_bound(super::goals::reduced(10))
                .unwrap_or(1)
                == 0
        {
            // Direction.getRandom: down, up, north, south, west, east.
            const DIRECTIONS: [[i32; 3]; 6] = [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ];
            let d = DIRECTIONS[m.random.next_int_bound(6).unwrap_or(0) as usize];
            let pos = [
                m.position.x.floor() as i32 + d[0],
                (m.position.y + 0.5).floor() as i32 + d[1],
                m.position.z.floor() as i32 + d[2],
            ];
            if super::spawn_selection::infested_host(cx.w.block_name(pos[0], pos[1], pos[2])) {
                self.merge = Some(pos);
                return true;
            }
        }
        self.merge = None;
        self.stroll.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.merge.is_none() && self.stroll.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        let Some(pos) = self.merge else {
            return self.stroll.start(m, cx);
        };
        if super::spawn_selection::infested_host(cx.w.block_name(pos[0], pos[1], pos[2])) {
            cx.events.push(MobEvent::Infest {
                pos,
                infested: true,
                poof: Some(m.id),
            });
            m.removed = true;
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.stroll.stop(m, cx);
    }
}
/// Entity.getLightLevelDependentMagicValue at the eye.
pub fn mob_brightness(m: &Mob, w: &MobWorld) -> f32 {
    w.brightness(
        m.position.x.floor() as i32,
        m.eye_y().floor() as i32,
        m.position.z.floor() as i32,
    )
}
fn dark_enough(m: &Mob, w: &MobWorld) -> bool {
    mob_brightness(m, w) < 0.5
}
/// Spider.SpiderAttackGoal: gives up in bright light 1 in 100 ticks.
#[derive(Clone, Debug)]
pub struct SpiderAttack(MeleeAttack);
impl Goal for SpiderAttack {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // SpiderAttackGoal.canUse: not while it carries a rider.
        self.0.can_use(m, cx) && m.extra.rider.is_none()
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if mob_brightness(m, cx.w) >= 0.5 && m.random.next_int_bound(100).unwrap_or(1) == 0 {
            m.ai.target = None;
            return false;
        }
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.start(m, cx)
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx)
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.tick(m, cx)
    }
}
/// SwellGoal: fuse within 3 blocks, defuse beyond 7 or out of sight.
#[derive(Clone, Debug, Default)]
pub struct Swell {
    target: Option<Who>,
}
impl Goal for Swell {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let close = m
            .target_info(cx.w)
            .is_some_and(|t| t.alive && m.distance_sqr(t.position) < 9.0);
        m.extra.counter > 0 || close
    }
    fn start(&mut self, m: &mut Mob, _: &mut Cx) {
        m.nav_stop();
        self.target = m.ai.target;
    }
    fn stop(&mut self, _: &mut Mob, _: &mut Cx) {
        self.target = None;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        let dir = match self
            .target
            .and_then(|t| cx.w.target_info(t).map(|i| (t, i)))
        {
            Some((who, t)) if t.alive => {
                if m.distance_sqr(t.position) > 49.0 || !m.sees(cx.w, who) {
                    -1
                } else {
                    1
                }
            }
            _ => -1,
        };
        m.extra.counter = dir;
    }
}
/// Creeper.tick on a client of a server: the synced fuse direction
/// (DATA_SWELL_DIR, DATA_IS_IGNITED) swells the creeper the renderer draws;
/// the server explodes it.
pub fn creeper_client_tick(m: &mut Mob) {
    if !m.alive() {
        return;
    }
    m.previous_swell = m.swell;
    if m.extra.flag {
        m.extra.counter = 1;
    }
    m.swell = (m.swell + m.extra.counter).clamp(0, 30);
}
/// Creeper.tick: advance the fuse and explode at 30.
pub fn creeper_tick(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    if m.extra.flag {
        // Ignited by flint and steel.
        m.extra.counter = 1;
    }
    let dir = m.extra.counter;
    if dir > 0 && m.swell == 0 {
        events.push(MobEvent::Sound(
            "entity.creeper.primed",
            m.center(),
            1.0,
            0.5,
        ));
    }
    m.swell = (m.swell + dir).max(0);
    if m.swell >= 30 {
        m.swell = 30;
        let radius = if m.extra.charged { 6.0 } else { 3.0 };
        let name = match &m.custom_name {
            Some(name) => crate::text::Text::literal(name.clone()),
            None => crate::text::Text::translate(
                &format!("entity.{}", m.kind.id.replace(':', ".")),
                &[],
            ),
        };
        events.push(MobEvent::ExplosionSource {
            mob: m.id,
            charged: m.extra.charged,
            name,
        });
        events.push(MobEvent::Explode(
            [m.position.x, m.position.y, m.position.z],
            radius,
            m.species(),
        ));
        m.removed = true;
    }
}
