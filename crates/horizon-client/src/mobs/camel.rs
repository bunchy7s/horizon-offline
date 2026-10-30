//! Camels (`Camel`): ridden by two, sitting down and standing up now and
//! then, dashing instead of jumping, eating cactus, always "tame".
use super::{
    Mob, MobEvent,
    anim::{Poses, State},
    goals::{
        Breed, Cx, Float, FollowParent, Goal, LookAtPlayer, MOVE, Panic, RandomLookAround, Stroll,
        Tempt, goal_clone,
    },
};
use horizon_core::geometry::trig::Mth;
use horizon_core::random::RandomSource;
use std::collections::HashMap;

// Camel state: `egg_time` the mob age of the last pose change (it may be
// negative: a camel finalized at age 0 stood up 53 ticks "ago"),
// `camel_sitting` whether it sits, `anim_ticks` the dash cooldown, `flag2` dashing, `ints[0]` the
// idle animation timeout. AnimState states: sit, sit pose, stand up, idle,
// dash.
const SIT_DOWN: i64 = 40;
const STAND_UP: i64 = 52;
pub const DASH_COOLDOWN: i32 = 55;

pub fn register(m: &mut Mob) {
    m.extra.tame = true;
    let tempt = m.kind.tempt.clone();
    let g = &mut m.ai.goals;
    // CamelAi speed modifiers (times the 0.09 movement speed attribute).
    g.add(0, Float);
    g.add(1, CamelPanic(Panic::new(4.0)));
    g.add(2, Breed::new(1.0));
    g.add(3, Tempt::new(2.5, tempt, false));
    g.add(4, FollowParent::new(2.5));
    g.add(5, Stroll::new(2.0));
    g.add(5, RandomSitting);
    g.add(6, LookAtPlayer::new(6.0));
    g.add(7, RandomLookAround::default());
    // finalizeSpawn: fully standing.
    stand_up_instantly(m);
}
fn pose_time(m: &Mob) -> i64 {
    i64::from(m.age) - i64::from(m.extra.egg_time)
}
pub fn sitting(m: &Mob) -> bool {
    m.extra.camel_sitting
}
fn in_transition(m: &Mob) -> bool {
    pose_time(m) < if sitting(m) { SIT_DOWN } else { STAND_UP }
}
/// `Camel.getPoseTime` of a client mob from the `LAST_POSE_CHANGE_TICK` it
/// was sent and the client's game time, and whether that value says it sits.
fn client_pose(m: &Mob, game_time: i64) -> (i64, bool) {
    let tick = m.extra.pose_change_tick;
    (game_time - tick.abs(), tick < 0)
}
/// The camel's `LAST_POSE_CHANGE_TICK` as the server sends it: in game time
/// (a mob counts its own age), negative while sitting.
pub fn synced_pose_tick(m: &Mob, game_time: i64) -> i64 {
    let tick = (game_time - pose_time(m)).max(0);
    if sitting(m) { -tick.max(1) } else { tick }
}
/// `Entity.onSyncedDataUpdated` for the pose tick of a client mob: its sign
/// is whether it sits. `client_tick` keeps `egg_time` (in the mob's age, for
/// `refuse_to_move`) with it.
pub fn set_synced_pose_tick(m: &mut Mob, tick: i64) {
    m.extra.pose_change_tick = tick;
    m.extra.camel_sitting = tick < 0;
}
/// Put a camel loaded from `LastPoseTick` (kept in `pose_change_tick`) on
/// the mob's own clock: the pose changed `game_time - |tick|` ticks ago. A
/// save without the tag leaves it standing for good.
pub fn finish_load(m: &mut Mob, game_time: i64) {
    let tick = m.extra.pose_change_tick;
    if tick == 0 {
        return stand_up_instantly(m);
    }
    let since = (game_time - tick.abs()).clamp(0, i64::from(i32::MAX / 2));
    m.extra.egg_time = (i64::from(m.age) - since) as i32;
}
/// `Camel.refuseToMove`: sitting or getting up or down.
pub fn refuse_to_move(m: &Mob) -> bool {
    sitting(m) || in_transition(m)
}
fn sit_down(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if !sitting(m) {
        sound(m, voice(m, 0), events);
        m.extra.camel_sitting = true;
        m.extra.egg_time = m.age as i32;
    }
}
pub fn stand_up(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if sitting(m) {
        sound(m, voice(m, 1), events);
        m.extra.camel_sitting = false;
        m.extra.egg_time = m.age as i32;
    }
}
/// `standUpInstantly`.
pub fn stand_up_instantly(m: &mut Mob) {
    m.extra.camel_sitting = false;
    m.extra.egg_time = (i64::from(m.age) - STAND_UP - 1) as i32;
}
/// `is_camel`: camels and camel husks (`CamelHusk extends Camel`).
pub fn is_camel(s: super::Species) -> bool {
    matches!(s, super::Species::Camel | super::Species::CamelHusk)
}
/// The sit, stand, dash ready, dash and eat sounds (`CamelHusk` overrides each).
pub fn voice(m: &Mob, which: usize) -> &'static str {
    let husk = m.species() == super::Species::CamelHusk;
    match (which, husk) {
        (0, false) => "entity.camel.sit",
        (0, true) => "entity.camel_husk.sit",
        (1, false) => "entity.camel.stand",
        (1, true) => "entity.camel_husk.stand",
        (2, false) => "entity.camel.dash_ready",
        (2, true) => "entity.camel_husk.dash_ready",
        (3, false) => "entity.camel.dash",
        (3, true) => "entity.camel_husk.dash",
        (_, false) => "entity.camel.eat",
        (_, true) => "entity.camel_husk.eat",
    }
}
fn sound(m: &Mob, name: &'static str, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(
        name,
        [m.position.x, m.position.y, m.position.z],
        1.0,
        1.0,
    ));
}
/// `CamelAi.CamelPanic`: standing up at once to flee.
#[derive(Clone, Debug)]
struct CamelPanic(Panic);
impl Goal for CamelPanic {
    goal_clone!();
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // CamelHusk.isMobControlled: not with a mob on its back either.
        m.rider.is_none()
            && !(m.species() == super::Species::CamelHusk && m.extra.rider.is_some())
            && self.0.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        stand_up_instantly(m);
        self.0.start(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.0.stop(m, cx);
    }
}
/// `CamelAi.RandomSitting`: after 20 seconds in a pose, sit or stand.
#[derive(Clone, Debug)]
struct RandomSitting;
impl Goal for RandomSitting {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        // checkExtraStartConditions: not leashed, and no controlling
        // passenger, a player's or a mob's (a husk on its back).
        !m.fluids.in_water
            && pose_time(m) >= 400
            && m.extra.leash.is_none()
            && m.on_ground
            && m.rider.is_none()
            && !super::riders::mob_controlled(m, cx.w)
            && m.random.next_int_bound(4).unwrap_or(1) == 0
    }
    fn can_continue(&mut self, _m: &mut Mob, _cx: &mut Cx) -> bool {
        false
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if sitting(m) {
            stand_up(m, cx.events);
        } else if !super::goals::should_panic(m) {
            sit_down(m, cx.events);
            m.ai.nav.stop();
        }
    }
}
/// `Camel.tick`: the dash clock, water stands it up; keyframe states.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if m.extra.flag2 && m.extra.anim_ticks < 50 && (m.on_ground || m.fluids.in_water) {
        m.extra.flag2 = false;
    }
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
        if m.extra.anim_ticks == 0 {
            sound(m, voice(m, 2), events);
        }
    }
    if sitting(m) && m.fluids.in_water {
        stand_up_instantly(m);
    }
    if refuse_to_move(m) {
        m.velocity.x = 0.0;
        m.velocity.z = 0.0;
        m.ai.xxa = 0.0;
        m.ai.zza = 0.0;
        m.ai.nav.stop();
    }
    animation_states(m, pose_time(m), sitting(m));
}
/// `Camel.tick` on a client that is not the server: the dash cooldown, which
/// the dash flag starts, and the keyframe states from the synced pose tick.
pub fn client_tick(m: &mut Mob, game_time: i64) {
    if m.extra.anim_ticks > 0 {
        m.extra.anim_ticks -= 1;
    }
    let (pose_time, sitting) = client_pose(m, game_time);
    m.extra.camel_sitting = sitting;
    m.extra.egg_time = (i64::from(m.age) - pose_time) as i32;
    animation_states(m, pose_time, sitting);
}
/// `Camel.onSyncedDataUpdated(DASH)`: a dash starts the cooldown unless it
/// runs. Not on the first tick.
pub fn set_dashing(m: &mut Mob, dashing: bool) {
    if m.age > 0 && m.extra.anim_ticks == 0 {
        m.extra.anim_ticks = DASH_COOLDOWN;
    }
    m.extra.flag2 = dashing;
}
/// `Camel.setupAnimationStates` for a pose time and whether it sits.
fn animation_states(m: &mut Mob, pose_time: i64, sitting: bool) {
    let age = m.age as f32;
    if m.extra.ints[0] <= 0 {
        m.extra.ints[0] = m.random.next_int_bound(40).unwrap_or(0) + 80;
        m.anim.states[3].start(age);
    } else {
        m.extra.ints[0] -= 1;
    }
    // isCamelVisuallySitting: (poseTime < 0) != sitting.
    let visually_sitting = (pose_time < 0) != sitting;
    let sitting_down = sitting && (0..SIT_DOWN).contains(&pose_time);
    let transition = (0..if sitting { SIT_DOWN } else { STAND_UP }).contains(&pose_time);
    let dashing = m.extra.flag2;
    let st = &mut m.anim.states;
    if visually_sitting {
        st[2].stop();
        st[4].stop();
        if sitting_down {
            st[0].start_if_stopped(age);
            st[1].stop();
        } else {
            st[0].stop();
            st[1].start_if_stopped(age);
        }
    } else {
        st[0].stop();
        st[1].stop();
        st[4].animate_when(dashing, age);
        st[2].animate_when(transition, age);
    }
}
/// `Camel.executeRidersJump`: the dash along the look, once cooled down.
pub fn dash(m: &mut Mob, amount: f32, events: &mut Vec<MobEvent>) {
    if m.extra.anim_ticks > 0 || !m.on_ground || refuse_to_move(m) {
        return;
    }
    let yaw = m.yaw.to_radians();
    let (dx, dz) = (f64::from(-yaw.msin()), f64::from(yaw.mcos()));
    let speed = m.movement_speed();
    let a = f64::from(amount);
    m.velocity.x += dx * 22.2222 * a * speed;
    m.velocity.z += dz * 22.2222 * a * speed;
    m.velocity.y += 1.4285 * a * 0.42;
    m.extra.anim_ticks = DASH_COOLDOWN;
    m.extra.flag2 = true;
    sound(m, voice(m, 3), events);
}
/// `CamelModel.setupAnim`.
pub fn anim(
    m: &Mob,
    s: &State,
    p: &mut Poses,
    animations: &HashMap<String, super::keyframes::Animation>,
) {
    let mut y = s.y_rot.clamp(-30.0, 30.0);
    let mut x = s.x_rot.clamp(-25.0, 45.0);
    let cooldown = (m.extra.anim_ticks as f32 - s.partial).max(0.0);
    if cooldown > 0.0 {
        x = (x + 45.0 * cooldown / 55.0).clamp(-25.0, 70.0);
    }
    y = y.to_radians();
    p.yrot("head", y);
    p.xrot("head", x.to_radians());
    let prefix = if m.is_baby() {
        "CamelBabyAnimation.CAMEL_BABY_"
    } else {
        "CamelAnimation.CAMEL_"
    };
    let get = |n: &str| animations.get(&format!("{prefix}{n}"));
    if let Some(walk) = get("WALK") {
        walk.apply_walk(s.pos, s.speed, 2.0, 2.5, p);
    }
    let age = s.age;
    for (state, name) in [
        (&m.anim.states[0], "SIT"),
        (&m.anim.states[1], "SIT_POSE"),
        (&m.anim.states[2], "STANDUP"),
        (&m.anim.states[3], "IDLE"),
        (&m.anim.states[4], "DASH"),
    ] {
        if let (Some(ms), Some(a)) = (state.millis(age), get(name)) {
            a.apply(ms, 1.0, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{Anim, Brain, Flags, MobType, Part, Species};
    use crate::mobs::{leash::Holder, world::with_test_world};
    use horizon_core::geometry::Vec3;
    use serde_json::json;
    use std::sync::Arc;

    /// `CamelAi.RandomSitting.checkExtraStartConditions`: a camel sits down
    /// only with no controlling passenger and no lead. A husk on its back
    /// controls it (`Mob.getControllingPassenger`), a slime does not
    /// (`#non_controlling_rider`), so a camel carrying a husk never sits
    /// with it floating above (#3199).
    #[test]
    fn a_camel_with_a_husk_on_its_back_never_sits_down() {
        // Whether the goal starts in 64 tries (each passes 1 in 4).
        let sits = |rider: Option<&str>, leashed: bool| {
            let mut seen = Vec::new();
            let mut camel = None;
            with_test_world(&[], |_, types| {
                let mob = |id: u32, name: &str| {
                    let kind = std::sync::Arc::new(types.types[name].clone());
                    Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0)
                };
                let mut m = mob(1, "minecraft:camel");
                if let Some(name) = rider {
                    let mut r = mob(2, name);
                    r.extra.mount = Some(m.id);
                    m.extra.rider = Some(r.id);
                    seen.push(r.seen(None));
                }
                if leashed {
                    m.extra.leash = Some(Holder::Mob(3));
                }
                camel = Some(m);
            });
            let mut m = camel?;
            m.age = 1000;
            m.extra.egg_time = 0;
            m.on_ground = true;
            let mut started = false;
            with_test_world(&seen, |w, _| {
                let mut lr = horizon_core::random::LegacyRandom::new(0);
                let mut events = Vec::new();
                let mut cx = Cx {
                    w,
                    lr: &mut lr,
                    events: &mut events,
                };
                started = (0..64).any(|_| RandomSitting.can_use(&mut m, &mut cx));
            });
            Some(started)
        };
        let Some(alone) = sits(None, false) else {
            return;
        };
        assert!(alone, "a camel with nothing on its back sits down");
        assert_eq!(sits(Some("minecraft:husk"), false), Some(false));
        assert_eq!(sits(Some("minecraft:slime"), false), Some(true));
        assert_eq!(sits(None, true), Some(false), "a leashed camel");
    }

    fn camel() -> Mob {
        let model = Arc::new(Part::parse("root", &json!({"children": {"head": {}}})));
        let kind = Arc::new(MobType {
            id: "minecraft:camel".into(),
            species: Species::Camel,
            width: 1.7,
            height: 2.375,
            eye_height: 2.275,
            category: "creature".into(),
            max_health: 32.0,
            speed: 0.09,
            flying_speed: 0.4,
            attack_damage: 0.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 1.5,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Arc::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.45,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 120,
            flags: Flags {
                animal: true,
                ..Default::default()
            },
            model,
            layers: HashMap::new(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Camel,
            sounds: [""; 3],
            step_sound: None,
            food: Arc::default(),
            tempt: Arc::default(),
        });
        Mob::new(1, kind, horizon_core::geometry::Vec3::ZERO, 0.0)
    }
    fn types(m: &Mob) -> HashMap<String, Arc<super::super::MobType>> {
        HashMap::from([(m.kind.id.clone(), m.kind.clone())])
    }

    #[test]
    fn a_fresh_camel_can_move_at_once() {
        // Camel.finalizeSpawn: standUpInstantly, also at age 0.
        let mut m = camel();
        register(&mut m);
        assert_eq!(m.age, 0);
        assert!(!sitting(&m));
        assert!(!refuse_to_move(&m));
        stand_up_instantly(&mut m);
        assert!(!refuse_to_move(&m));
    }

    #[test]
    fn sitting_and_standing_up_take_their_transition_time() {
        let mut m = camel();
        register(&mut m);
        m.age = 100;
        let mut events = Vec::new();
        sit_down(&mut m, &mut events);
        assert!(sitting(&m) && refuse_to_move(&m));
        assert_eq!(pose_time(&m), 0);
        m.age += 40;
        assert!(sitting(&m) && refuse_to_move(&m));
        stand_up(&mut m, &mut events);
        assert!(!sitting(&m) && refuse_to_move(&m));
        m.age += 52;
        assert!(!refuse_to_move(&m));
    }

    #[test]
    fn last_pose_tick_is_a_time_not_a_sign() {
        let mut m = camel();
        register(&mut m);
        m.age = 30;
        // Stood up at game time 1000; now 1030.
        m.extra.egg_time = 0;
        assert_eq!(synced_pose_tick(&m, 1030), 1000);
        m.extra.camel_sitting = true;
        assert_eq!(synced_pose_tick(&m, 1030), -1000);
    }

    #[test]
    fn a_loaded_camel_keeps_its_pose_time() {
        let mut m = camel();
        register(&mut m);
        let types = types(&m);
        let load = |tick: i64, now: i64| {
            let (tag, _) = horizon_world::nbt::parse_snbt(&format!(
                "{{id:\"minecraft:camel\",Pos:[0.0d,0.0d,0.0d],LastPoseTick:{tick}L}}"
            ))
            .unwrap();
            let mut m = crate::entity_nbt::mob_from_nbt(&tag, &types, 2).unwrap();
            finish_load(&mut m, now);
            m
        };
        // Standing for a long time: free to move at once.
        let m = load(1000, 5000);
        assert!(!sitting(&m) && !refuse_to_move(&m));
        // Stood up 5 ticks ago: still getting up.
        let m = load(1000, 1005);
        assert!(!sitting(&m) && refuse_to_move(&m));
        assert_eq!(pose_time(&m), 5);
        // Sat 5 ticks ago: pose time 5, not 401.
        let m = load(-1000, 1005);
        assert!(sitting(&m));
        assert_eq!(pose_time(&m), 5);
        // No tag: standing.
        let (tag, _) =
            horizon_world::nbt::parse_snbt("{id:\"minecraft:camel\",Pos:[0.0d,0.0d,0.0d]}")
                .unwrap();
        let mut m = crate::entity_nbt::mob_from_nbt(&tag, &types, 2).unwrap();
        finish_load(&mut m, 5000);
        assert!(!refuse_to_move(&m));
    }
}
