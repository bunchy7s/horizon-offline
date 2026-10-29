//! The warden (`Warden`, `AngerManagement`): anger kept per entity,
//! vibration listening, the sonic boom and melee attack, the heartbeat and
//! tendril animations, the model and its emissive layers. Its brain
//! (`WardenAi`) is in `warden_brain`.
use super::{
    Damage, DamageKind, Mob, MobEvent, MobWorld, Species, Who,
    anim::{Poses, State},
    keyframes::Animation,
    path::PathType,
    warden_brain::{self, Brain},
};
use crate::sculk::{self, Context, Info};
use horizon_core::{
    geometry::{Vec3, trig::Mth},
    random::{LegacyRandom, RandomSource},
};
use std::{collections::HashMap, f32::consts::PI};

pub const EMERGE_DURATION: i64 = warden_brain::EMERGE_DURATION as i64;
pub const DIGGING_COOLDOWN: i64 = warden_brain::DIGGING_COOLDOWN;
const DEFAULT_ANGER: i32 = 35;
const PROJECTILE_ANGER: i32 = 10;
const ON_HURT_ANGER_BOOST: i32 = 20;
const MAX_ANGER: i32 = 150;
const DARKNESS_RADIUS: f64 = 20.0;
/// Darkness lasts 260 ticks and is renewed within the last 200.
pub const DARKNESS_DURATION: i32 = 260;
pub const DARKNESS_REAPPLY: i32 = 200;

// AnimState keyframe slots.
pub const ROAR: usize = 0;
pub const SNIFF: usize = 1;
pub const EMERGE: usize = 2;
pub const DIGGING: usize = 3;
pub const ATTACK: usize = 4;
pub const SONIC_BOOM: usize = 5;
// AnimState values: tendril and heart animation (ticks, 10 at the start).
pub const TENDRIL: usize = 0;
pub const HEART: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pose {
    #[default]
    Standing,
    Emerging,
    Digging,
    Roaring,
    Sniffing,
}
impl Pose {
    fn code(self) -> i32 {
        match self {
            Self::Standing => 0,
            Self::Emerging => 1,
            Self::Digging => 2,
            Self::Roaring => 3,
            Self::Sniffing => 4,
        }
    }
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Emerging,
            2 => Self::Digging,
            3 => Self::Roaring,
            4 => Self::Sniffing,
            _ => Self::Standing,
        }
    }
}
/// `Pose.id` of the poses a warden takes (`sync::pose_id`).
pub fn pose_id(m: &Mob) -> u8 {
    match pose(m) {
        Pose::Standing => 0,
        Pose::Roaring => 11,
        Pose::Sniffing => 12,
        Pose::Emerging => 13,
        Pose::Digging => 14,
    }
}
/// `Entity.onSyncedDataUpdated(DATA_POSE)` on a client.
pub(super) fn set_pose_id(m: &mut Mob, id: u8) {
    set_pose(
        m,
        match id {
            11 => Pose::Roaring,
            12 => Pose::Sniffing,
            13 => Pose::Emerging,
            14 => Pose::Digging,
            _ => Pose::Standing,
        },
    );
}
/// The warden's pose (kept in `Extra.counter` so it can be read while the
/// brain is being ticked).
pub fn pose(m: &Mob) -> Pose {
    Pose::from_code(m.extra.counter)
}
pub fn digging_or_emerging(m: &Mob) -> bool {
    matches!(pose(m), Pose::Digging | Pose::Emerging)
}
/// `Entity.setPose`, with `onSyncedDataUpdated` starting the keyframes.
pub(super) fn set_pose(m: &mut Mob, new: Pose) {
    if pose(m) == new {
        return;
    }
    m.extra.counter = new.code();
    let age = m.age as f32;
    match new {
        Pose::Emerging => m.anim.states[EMERGE].start(age),
        Pose::Digging => m.anim.states[DIGGING].start(age),
        Pose::Roaring => m.anim.states[ROAR].start(age),
        Pose::Sniffing => m.anim.states[SNIFF].start(age),
        Pose::Standing => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AngerLevel {
    Calm,
    Agitated,
    Angry,
}
impl AngerLevel {
    pub fn by_anger(anger: i32) -> Self {
        if anger >= 80 {
            Self::Angry
        } else if anger >= 40 {
            Self::Agitated
        } else {
            Self::Calm
        }
    }
    pub fn minimum(self) -> i32 {
        match self {
            Self::Calm => 0,
            Self::Agitated => 40,
            Self::Angry => 80,
        }
    }
    pub fn is_angry(self) -> bool {
        self == Self::Angry
    }
    pub fn ambient_sound(self) -> &'static str {
        match self {
            Self::Calm => "entity.warden.ambient",
            Self::Agitated => "entity.warden.agitated",
            Self::Angry => "entity.warden.angry",
        }
    }
    pub fn listening_sound(self) -> &'static str {
        match self {
            Self::Calm => "entity.warden.listening",
            _ => "entity.warden.listening_angry",
        }
    }
}

/// `AngerManagement`: anger per suspect, highest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AngerManagement {
    suspects: Vec<Who>,
    anger: HashMap<Who, i32>,
    highest: i32,
}
impl AngerManagement {
    /// One second: every suspect calms by one; the invalid are forgotten.
    pub fn tick(&mut self, valid: impl Fn(Who) -> bool) {
        let mut dropped = Vec::new();
        for (who, anger) in self.anger.iter_mut() {
            if *anger > 1 && valid(*who) {
                *anger -= 1;
            } else {
                dropped.push(*who);
            }
        }
        for who in dropped {
            self.anger.remove(&who);
            self.suspects.retain(|s| *s != who);
        }
        self.sort_and_update_highest();
    }
    fn sort_and_update_highest(&mut self) {
        // AngerManagement.Sorter: the angry first, then players, then the
        // angriest; a stable sort keeps the order of ties.
        let anger = self.anger.clone();
        let level = |w: &Who| anger.get(w).copied().unwrap_or(0);
        self.suspects.sort_by(|a, b| {
            let (aa, ab) = (level(a), level(b));
            let (angry_a, angry_b) = (
                AngerLevel::by_anger(aa).is_angry(),
                AngerLevel::by_anger(ab).is_angry(),
            );
            if angry_a != angry_b {
                return if angry_a {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            let (player_a, player_b) = (*a == Who::Player, *b == Who::Player);
            if player_a != player_b {
                return if player_a {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            ab.cmp(&aa)
        });
        self.highest = match self.suspects.len() {
            0 => 0,
            1 => level(&self.suspects[0]),
            _ => self.suspects.iter().map(&level).max().unwrap_or(0),
        };
    }
    /// `increaseAnger`: the entity's new anger (at most 150).
    pub fn increase(&mut self, who: Who, amount: i32) -> i32 {
        let new_suspect = !self.anger.contains_key(&who);
        let current = (self.anger.get(&who).copied().unwrap_or(0) + amount).min(MAX_ANGER);
        self.anger.insert(who, current);
        if new_suspect {
            self.suspects.push(who);
        }
        self.sort_and_update_highest();
        current
    }
    pub fn clear(&mut self, who: Who) {
        self.anger.remove(&who);
        self.suspects.retain(|s| *s != who);
        self.sort_and_update_highest();
    }
    /// `getActiveAnger`: the target's anger, or the highest when there is none.
    pub fn active_anger(&self, target: Option<Who>) -> i32 {
        match target {
            None => self.highest,
            Some(t) => self.anger.get(&t).copied().unwrap_or(0),
        }
    }
    /// `getActiveEntity`: the first suspect that can still be targeted.
    pub fn active_entity(&self, can_target: impl Fn(Who) -> bool) -> Option<Who> {
        self.suspects.iter().copied().find(|w| can_target(*w))
    }
    pub fn anger_of(&self, who: Who) -> i32 {
        self.anger.get(&who).copied().unwrap_or(0)
    }
    pub fn suspects(&self) -> &[Who] {
        &self.suspects
    }
}

/// The warden's state: anger, its vibration listener and its brain.
#[derive(Clone, Debug, Default)]
pub struct Warden {
    pub anger: AngerManagement,
    pub vibration: sculk::Data,
    pub brain: Brain,
    /// `getClientAngerLevel`, synchronised once a second.
    pub client_anger: i32,
    /// Attackers noted by `hurtServer` and not yet reacted to.
    hurt_by: Vec<(Option<Who>, bool)>,
}
impl Warden {
    pub fn target(&self) -> Option<Who> {
        self.brain.mem.attack_target.value()
    }
}

pub fn register(m: &mut Mob) {
    m.extra.warden = Some(Box::default());
    m.ai.nav.can_float = true;
    m.set_malus(PathType::UnpassableRail, 0.0);
    m.set_malus(PathType::Damaging, 8.0);
    m.set_malus(PathType::PowderSnow, 8.0);
    m.set_malus(PathType::Lava, 8.0);
    m.set_malus(PathType::Fire, 0.0);
    m.set_malus(PathType::FireInNeighbor, 0.0);
    // Brain.Provider.makeBrain: the sensors start on a random phase.
    delay_sensors(m);
}
/// `Sensor.randomlyDelayStart` for the warden's two sensors, from its random.
pub fn delay_sensors(m: &mut Mob) {
    if let Some(mut st) = m.extra.warden.take() {
        st.brain.delay_sensors(&mut m.random);
        m.extra.warden = Some(st);
    }
}
/// A brain whose `RunOne` shuffles with `seed` (vanilla seeds it from the
/// clock; harnesses seed both sides alike), its sensors delayed again.
pub fn init_brain(m: &mut Mob, seed: i64) {
    if let Some(st) = m.extra.warden.as_mut() {
        let mem = std::mem::take(&mut st.brain.mem);
        st.brain = Brain::new(seed);
        st.brain.mem = mem;
    }
    delay_sensors(m);
}
/// `Warden.finalizeSpawn`: a minute until it digs back down; a summoned
/// warden climbs out of the ground first.
pub fn finalize(m: &mut Mob, triggered: bool, events: &mut Vec<MobEvent>) {
    if m.extra.warden.is_none() {
        return;
    }
    warden_brain::mem(m)
        .dig_cooldown
        .set_ttl((), DIGGING_COOLDOWN);
    if triggered {
        set_pose(m, Pose::Emerging);
        warden_brain::mem(m)
            .is_emerging
            .set_ttl((), EMERGE_DURATION);
        sound(m, "entity.warden.agitated", 5.0, 1.0, events);
    }
}
fn at(m: &Mob) -> [f64; 3] {
    [m.position.x, m.position.y, m.position.z]
}
fn sound(m: &Mob, name: &'static str, volume: f32, pitch: f32, events: &mut Vec<MobEvent>) {
    events.push(MobEvent::Sound(name, at(m), volume, pitch));
}
/// `LivingEntity.getVoicePitch` for an adult.
fn voice_pitch(m: &mut Mob) -> f32 {
    (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0
}
fn closer_than(m: &Mob, p: Vec3, horizontal: f64, vertical: f64) -> bool {
    let (dx, dy, dz) = (p.x - m.position.x, p.y - m.position.y, p.z - m.position.z);
    dx * dx + dz * dz < horizontal * horizontal && dy * dy < vertical * vertical
}

// ---- entity facts ----

/// `Warden.canTargetEntity`: living, neither creative nor spectator, not a
/// warden, alive.
pub fn can_target(w: &MobWorld, who: Who) -> bool {
    match who {
        Who::Player => w
            .player
            .as_ref()
            .is_some_and(|p| p.alive && !p.creative && !p.spectator),
        Who::Mob(id) => w
            .seen(id)
            .is_some_and(|s| s.alive && s.species != Species::Warden),
    }
}
fn position_of(w: &MobWorld, who: Who) -> Option<Vec3> {
    w.target_info(who).map(|t| t.position)
}

// ---- anger ----

/// `Warden.increaseAngerAt`.
pub(super) fn increase_anger(
    m: &mut Mob,
    w: &MobWorld,
    who: Option<Who>,
    amount: i32,
    play_sound: bool,
    events: &mut Vec<MobEvent>,
) {
    let Some(who) = who else { return };
    if m.no_ai || !can_target(w, who) {
        return;
    }
    warden_brain::set_dig_cooldown(m);
    let Some(st) = m.extra.warden.as_mut() else {
        return;
    };
    let maybe_switch = st.target() != Some(Who::Player);
    let new = st.anger.increase(who, amount);
    if who == Who::Player && maybe_switch && AngerLevel::by_anger(new).is_angry() {
        st.brain.mem.attack_target.clear();
    }
    if play_sound && pose(m) != Pose::Roaring {
        let level = anger_level(m);
        let pitch = voice_pitch(m);
        sound(m, level.listening_sound(), 10.0, pitch, events);
    }
}
/// `Warden.getEntityAngryAt`.
pub(super) fn entity_angry_at(m: &Mob, w: &MobWorld) -> Option<Who> {
    let st = m.extra.warden.as_ref()?;
    if anger_level(m).is_angry() {
        st.anger.active_entity(|e| can_target(w, e))
    } else {
        None
    }
}
/// `Warden.getAngerLevel`.
pub fn anger_level(m: &Mob) -> AngerLevel {
    m.extra.warden.as_ref().map_or(AngerLevel::Calm, |s| {
        AngerLevel::by_anger(s.anger.active_anger(s.target()))
    })
}

// ---- damage ----

/// `Warden.isInvulnerableTo`: while digging or emerging.
pub fn accepts_damage(m: &Mob, source: &Damage) -> bool {
    !(digging_or_emerging(m) && !source.kind.bypasses_invulnerability())
}
/// `Warden.hurtServer`'s reaction, handled at the warden's next tick.
pub fn hurt(m: &mut Mob, source: &Damage) {
    if m.no_ai || digging_or_emerging(m) {
        return;
    }
    let direct = !matches!(
        source.kind,
        DamageKind::Arrow | DamageKind::Thrown | DamageKind::Fireball
    );
    if let Some(st) = m.extra.warden.as_mut() {
        st.hurt_by.push((source.attacker, direct));
    }
}
/// The rest of `Warden.hurtServer`.
fn handle_hurt(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let hurt = m
        .extra
        .warden
        .as_mut()
        .map(|st| std::mem::take(&mut st.hurt_by))
        .unwrap_or_default();
    for (attacker, direct) in hurt {
        increase_anger(
            m,
            w,
            attacker,
            AngerLevel::Angry.minimum() + ON_HURT_ANGER_BOOST,
            false,
            events,
        );
        if !warden_brain::mem(m).attack_target.present()
            && let Some(who) = attacker
            && let Some(p) = position_of(w, who)
            && (direct || m.distance_sqr(p) < 25.0)
        {
            warden_brain::set_attack_target(m, who);
        }
    }
}
/// `Warden.doPush`: touching something angers it (once a second).
pub fn do_push(m: &mut Mob, w: &MobWorld, who: Who, events: &mut Vec<MobEvent>) {
    if m.no_ai || m.extra.warden.is_none() || warden_brain::mem(m).touch_cooldown.present() {
        return;
    }
    warden_brain::mem(m).touch_cooldown.set_ttl((), 20);
    increase_anger(m, w, Some(who), DEFAULT_ANGER, true, events);
    if let Some(t) = w.target_info(who) {
        let block = [
            t.position.x.floor() as i32,
            t.position.y.floor() as i32,
            t.position.z.floor() as i32,
        ];
        warden_brain::set_disturbance(m, w, block);
    }
}
/// `Warden.doHurtTarget`: the attack keyframes (entity event 4), the impact
/// sound, a sonic boom cooldown, then `Mob.doHurtTarget`.
pub(super) fn do_hurt_target(
    m: &mut Mob,
    target: Who,
    w: &MobWorld,
    lr: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    m.anim.states[ROAR].stop();
    m.anim.states[ATTACK].start(m.age as f32);
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 4,
        at: at(m),
    });
    let pitch = voice_pitch(m);
    sound(m, "entity.warden.attack_impact", 10.0, pitch, events);
    warden_brain::mem(m).sonic_boom_cooldown.set_ttl((), 40);
    let mut cx = warden_brain::cx(w, lr, events);
    super::ai::do_hurt_target(m, target, &mut cx);
}

/// The brain's attack target (`Warden.getTarget` before `asValidTarget`).
pub fn brain_target(m: &Mob) -> Option<Who> {
    warden_brain::mem_ref(m).and_then(|mem| mem.attack_target.value())
}

// ---- vibrations ----

/// `Warden.VibrationUser.canReceiveVibration`.
pub fn can_receive_vibration(m: &Mob, _now: i64, ctx: &Context) -> bool {
    let Some(st) = m.extra.warden.as_ref() else {
        return false;
    };
    !m.no_ai
        && m.alive()
        && !st.brain.mem.vibration_cooldown.present()
        && !digging_or_emerging(m)
        && (!ctx.living || ctx.targetable)
}
/// `VibrationSystem.Listener.handleGameEvent` for the warden: a free
/// listener within 16 blocks of its eyes schedules what it can hear.
/// `occluded` is `VibrationSystem.Listener.isOccluded`. Returns true when a
/// sneaking player's step was avoided (AVOID_VIBRATION) instead.
pub fn listen(
    m: &mut Mob,
    now: i64,
    event: &'static str,
    pos: [f64; 3],
    ctx: &Context,
    occluded: impl Fn([f64; 3], [f64; 3]) -> bool,
) -> bool {
    let eye = [m.position.x, m.position.y + m.kind.eye_height, m.position.z];
    let Some(st) = m.extra.warden.as_ref() else {
        return false;
    };
    if st.vibration.current.is_some() {
        return false;
    }
    let block = |p: [f64; 3]| p.map(|v| i64::from(v.floor() as i32));
    let (source, listener) = (block(pos), block(eye));
    let d = [0, 1, 2].map(|i| source[i] - listener[i]);
    if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] > 256 {
        return false;
    }
    // isValidVibration: the warden can trigger avoid vibration.
    let listens = sculk::warden_can_listen(event);
    if listens
        && ctx.source == Some(Who::Player)
        && !ctx.spectator
        && ctx.stepping_carefully
        && sculk::ignored_when_sneaking(event)
    {
        return true;
    }
    if !sculk::valid_vibration(listens, event, ctx)
        || !can_receive_vibration(m, i64::from(m.age), ctx)
        || occluded(pos, eye)
    {
        return false;
    }
    let d = [pos[0] - eye[0], pos[1] - eye[1], pos[2] - eye[2]];
    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() as f32;
    if let Some(st) = m.extra.warden.as_mut() {
        st.vibration.schedule(
            Info {
                event,
                distance,
                pos,
                source: ctx.source,
                owner: ctx.owner,
            },
            now,
        );
    }
    false
}
/// `Level.gameEvent` for the mobs' own movement events from `from` on, as
/// the mob AI probe has no sculk layer: every warden's listener hears them.
pub fn hear_mob_events(mobs: &mut [Mob], events: &[MobEvent], from: usize, w: &MobWorld) {
    for e in &events[from..] {
        let MobEvent::GameEvent {
            event,
            at,
            source,
            state,
        } = *e
        else {
            continue;
        };
        let who = mobs.iter().find(|m| m.id == source);
        let name = &w.facts.get(state).name;
        let ctx = Context {
            source: Some(Who::Mob(source)),
            dampening_state: w.has_tag("minecraft:dampens_vibrations", name),
            dampens: who.is_some_and(|m| m.species() == Species::Warden),
            targetable: who.is_some_and(|m| m.alive() && m.species() != Species::Warden),
            living: true,
            ..Context::default()
        };
        for m in mobs.iter_mut().filter(|m| m.extra.warden.is_some()) {
            listen(m, w.game_time, event, at, &ctx, |a, b| {
                sculk::is_occluded(a, b, |p| {
                    let block = w.world.block(p.x, p.y, p.z);
                    w.has_tag(
                        "minecraft:occludes_vibration_signals",
                        &w.facts.get(block).name,
                    )
                })
            });
        }
    }
}
/// `Warden.VibrationUser.onReceiveVibration`.
fn receive_vibration(m: &mut Mob, w: &MobWorld, info: Info, events: &mut Vec<MobEvent>) {
    if !m.alive() {
        return;
    }
    warden_brain::mem(m).vibration_cooldown.set_ttl((), 40);
    // Entity event 61: the tendrils twitch.
    m.anim.v[TENDRIL] = 10.0;
    events.push(MobEvent::EntityEvent {
        mob: m.id,
        event: 61,
        at: at(m),
    });
    let pitch = voice_pitch(m);
    sound(m, "entity.warden.tendril_clicks", 5.0, pitch, events);
    let origin = [
        info.pos[0].floor() as i32,
        info.pos[1].floor() as i32,
        info.pos[2].floor() as i32,
    ];
    let mut suspicious = origin;
    if let Some(owner) = info.owner {
        if let Some(p) = position_of(w, owner)
            && m.distance_sqr(p) < 900.0
        {
            if warden_brain::mem(m).recent_projectile.present() {
                if can_target(w, owner)
                    && let Some(t) = w.target_info(owner)
                {
                    suspicious = [
                        t.position.x.floor() as i32,
                        t.position.y.floor() as i32,
                        t.position.z.floor() as i32,
                    ];
                }
                increase_anger(m, w, Some(owner), DEFAULT_ANGER, true, events);
            } else {
                increase_anger(m, w, Some(owner), PROJECTILE_ANGER, true, events);
            }
        }
        warden_brain::mem(m).recent_projectile.set_ttl((), 100);
    } else {
        increase_anger(m, w, info.source, DEFAULT_ANGER, true, events);
    }
    if !anger_level(m).is_angry() {
        let active = m
            .extra
            .warden
            .as_ref()
            .and_then(|st| st.anger.active_entity(|e| can_target(w, e)));
        if info.owner.is_some() || active.is_none() || active == info.source {
            warden_brain::set_disturbance(m, w, suspicious);
        }
    }
}

// ---- the server tick ----

/// `Warden.tick` before `super.tick()`: the vibration ticker, and
/// persistence keeping the dig cooldown.
pub fn pre_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    if m.extra.warden.is_none() {
        return;
    }
    let tick = m
        .extra
        .warden
        .as_mut()
        .map(|st| {
            st.vibration
                .tick(w.game_time, true, false, |d| d.floor() as i32)
        })
        .unwrap_or_default();
    if let Some((info, travel)) = tick.started {
        let eye = [m.position.x, m.position.y + m.kind.eye_height, m.position.z];
        events.push(MobEvent::Vibration {
            from: info.pos,
            to: eye,
            ticks: travel,
        });
    }
    if let Some(info) = tick.received {
        receive_vibration(m, w, info, events);
    }
    if m.persistent || m.custom_name.is_some() {
        warden_brain::set_dig_cooldown(m);
    }
}
/// `Warden.customServerAiStep`: the brain, the darkness pulse, anger and
/// the activity.
pub fn server_ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if m.extra.warden.is_none() {
        return;
    }
    handle_hurt(m, w, events);
    warden_brain::tick(m, w, level_random, events);
    if (i64::from(m.age) + i64::from(m.id)) % 120 == 0 {
        events.push(MobEvent::Darkness {
            at: at(m),
            radius: DARKNESS_RADIUS,
        });
    }
    if m.age.is_multiple_of(20)
        && let Some(st) = m.extra.warden.as_mut()
    {
        st.anger.tick(|who| can_target(w, who));
        st.client_anger = st.anger.active_anger(st.target());
    }
    if let Some(st) = m.extra.warden.as_mut() {
        st.brain.update_activity();
    }
}
/// `SonicBoom.tick`: face the target; once the sound delay is over, the
/// beam, the sound, the damage and the knockback.
pub(super) fn tick_sonic_boom(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) {
    let Some(target) = warden_brain::mem(m).attack_target.value() else {
        return;
    };
    if let Some(t) = w.target_info(target) {
        m.look_at([t.position.x, t.position.y, t.position.z]);
    }
    let mm = warden_brain::mem(m);
    if mm.sonic_boom_sound_delay.present() || mm.sonic_boom_sound_cooldown.present() {
        return;
    }
    mm.sonic_boom_sound_cooldown.set_ttl((), 60 - 34);
    if !can_target(w, target) {
        return;
    }
    let Some(t) = w.target_info(target) else {
        return;
    };
    if !closer_than(m, t.position, 15.0, 20.0) {
        return;
    }
    // The chest attachment: 1.6 up and 0.0 forward.
    let source = [m.position.x, m.position.y + 1.6, m.position.z];
    let eye = [t.position.x, t.eye_y, t.position.z];
    let delta = [eye[0] - source[0], eye[1] - source[1], eye[2] - source[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    let normal = if length < 1.0e-5 {
        [0.0; 3]
    } else {
        [delta[0] / length, delta[1] / length, delta[2] / length]
    };
    let steps = length.floor() as i32 + 7;
    for i in 1..steps {
        let d = f64::from(i);
        events.push(MobEvent::Particles {
            kind: "sonic_boom",
            at: [
                source[0] + normal[0] * d,
                source[1] + normal[1] * d,
                source[2] + normal[2] * d,
            ],
            count: 0,
        });
    }
    sound(m, "entity.warden.sonic_boom", 3.0, 1.0, events);
    events.push(MobEvent::SonicBoom {
        target,
        attacker: m.id,
        from: at(m),
        direction: normal,
    });
}
/// `Warden.tick` on the client: the heartbeat (pitched by the client's copy
/// of the warden: a level event sound here), the tendril and heart
/// animations, digging crumbs, and keyframes that ended.
pub fn ai_step(m: &mut Mob, events: &mut Vec<MobEvent>) {
    if heartbeat(m) {
        events.push(MobEvent::LevelEventSound {
            sound: "entity.warden.heartbeat",
            at: at(m),
            volume: 5.0,
            spread: 0.2,
        });
    }
    animate(m, events);
}
/// The heart's beat on its clock, from the client's anger level: whether
/// this tick has one (its sound is the caller's).
fn heartbeat(m: &mut Mob) -> bool {
    let Some(st) = m.extra.warden.as_ref() else {
        return false;
    };
    let delay = heart_beat_delay(st.client_anger);
    let beat = i64::from(m.age) % i64::from(delay) == 0;
    if beat {
        m.anim.v[HEART] = 10.0;
    }
    beat
}
/// `Warden.tick` on a client that is not the server: the heart and tendril
/// animations and the keyframes that ended. Its sounds and particles come
/// from the server.
pub fn client_tick(m: &mut Mob) {
    m.anim.vo = m.anim.v;
    heartbeat(m);
    animate(m, &mut Vec::new());
}
fn animate(m: &mut Mob, events: &mut Vec<MobEvent>) {
    // The animation values count down after the sync of the last tick.
    for slot in [TENDRIL, HEART] {
        if m.anim.v[slot] > 0.0 {
            m.anim.v[slot] -= 1.0;
        }
    }
    let digging_clock = |m: &Mob, slot: usize| {
        m.anim.states[slot]
            .millis(m.age as f32)
            .is_some_and(|ms| (ms as f32) < 4500.0)
    };
    match pose(m) {
        Pose::Emerging if digging_clock(m, EMERGE) => {
            events.push(MobEvent::DigParticles { at: at(m) })
        }
        Pose::Digging if digging_clock(m, DIGGING) => {
            events.push(MobEvent::DigParticles { at: at(m) })
        }
        _ => {}
    }
    // Poses that ended stop their keyframes.
    let p = pose(m);
    if p != Pose::Roaring {
        m.anim.states[ROAR].stop();
    }
    if p != Pose::Sniffing {
        m.anim.states[SNIFF].stop();
    }
    if p != Pose::Emerging {
        m.anim.states[EMERGE].stop();
    }
    if p != Pose::Digging {
        m.anim.states[DIGGING].stop();
    }
    if m.anim.states[ATTACK]
        .millis(m.age as f32)
        .is_some_and(|ms| ms > 1000)
    {
        m.anim.states[ATTACK].stop();
    }
    if m.anim.states[SONIC_BOOM]
        .millis(m.age as f32)
        .is_some_and(|ms| ms > 3500)
    {
        m.anim.states[SONIC_BOOM].stop();
    }
}
/// Test aid: the minute of calm ran out.
pub fn expire_dig_cooldown(m: &mut Mob) {
    if m.extra.warden.is_some() {
        let mm = warden_brain::mem(m);
        mm.dig_cooldown.clear();
        mm.attack_target.clear();
        mm.roar_target.clear();
    }
}
/// The warden's state for the control socket.
pub fn describe(m: &Mob) -> Option<serde_json::Value> {
    let st = m.extra.warden.as_ref()?;
    let anger: Vec<String> = st
        .anger
        .suspects()
        .iter()
        .map(|w| format!("{w:?}={}", st.anger.anger_of(*w)))
        .collect();
    let (activities, memories) = warden_brain::describe(m).unwrap_or_default();
    Some(serde_json::json!({
        "pose": format!("{:?}", pose(m)),
        "activities": activities,
        "memories": memories,
        "anger_level": format!("{:?}", anger_level(m)),
        "anger": anger,
        "vibration": st.vibration.current.map(|i| i.event),
    }))
}
/// `Warden.getHeartBeatDelay`.
pub fn heart_beat_delay(client_anger: i32) -> i32 {
    let anger = client_anger as f32 / AngerLevel::Angry.minimum() as f32;
    40 - (anger.clamp(0.0, 1.0) * 30.0).floor() as i32
}
/// `Warden.getAmbientSound`: none while roaring, digging or emerging.
pub fn ambient_sound(m: &Mob) -> Option<&'static str> {
    if pose(m) == Pose::Roaring || digging_or_emerging(m) {
        None
    } else {
        Some(anger_level(m).ambient_sound())
    }
}
/// `getTendrilAnimation` / `getHeartAnimation` at the partial tick.
pub fn tendril_animation(m: &Mob, partial: f32) -> f32 {
    m.anim.lerp(TENDRIL, partial) / 10.0
}
pub fn heart_animation(m: &Mob, partial: f32) -> f32 {
    m.anim.lerp(HEART, partial) / 10.0
}

// ---- model ----

/// `WardenModel.setupAnim`.
pub fn anim(m: &Mob, s: &State, p: &mut Poses, animations: &HashMap<String, Animation>) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
    // animateWalk
    let speed_modifier = (3.0 * s.speed).min(0.5);
    let adjusted = s.pos * 0.8662;
    let (cos, sin) = (adjusted.mcos(), adjusted.msin());
    let speed_min = speed_modifier.min(0.35);
    let head_z = p.get("head")[5] + 0.3 * sin * speed_modifier;
    p.zrot("head", head_z);
    let head_x = p.get("head")[3] + 1.2 * (adjusted + PI / 2.0).mcos() * speed_min;
    p.xrot("head", head_x);
    p.zrot("body", 0.1 * sin * speed_modifier);
    p.xrot("body", cos * speed_min);
    p.xrot("left_leg", cos * speed_modifier);
    p.xrot("right_leg", (adjusted + PI).mcos() * speed_modifier);
    p.xrot("left_arm", -(0.8 * cos * speed_modifier));
    p.zrot("left_arm", 0.0);
    p.xrot("right_arm", -(0.8 * sin * speed_modifier));
    p.zrot("right_arm", 0.0);
    for (arm, x) in [("left_arm", 13.0), ("right_arm", -13.0)] {
        let a = p.get(arm);
        a[4] = 0.0;
        a[2] = 1.0;
        a[0] = x;
        a[1] = -13.0;
    }
    // animateIdlePose
    let scaled = s.age * 0.1;
    let (wobble_cos, wobble_sin) = (scaled.mcos(), scaled.msin());
    let head_z = p.get("head")[5] + 0.06 * wobble_cos;
    p.zrot("head", head_z);
    let head_x = p.get("head")[3] + 0.06 * wobble_sin;
    p.xrot("head", head_x);
    let body_z = p.get("body")[5] + 0.025 * wobble_sin;
    p.zrot("body", body_z);
    let body_x = p.get("body")[3] + 0.025 * wobble_cos;
    p.xrot("body", body_x);
    // animateTendrils
    let tendril = tendril_animation(m, s.partial)
        * ((f64::from(s.age) * 2.25).cos() * std::f64::consts::PI * f64::from(0.1f32)) as f32;
    p.xrot("left_tendril", tendril);
    p.xrot("right_tendril", -tendril);
    for (slot, name) in [
        (ATTACK, "WARDEN_ATTACK"),
        (SONIC_BOOM, "WARDEN_SONIC_BOOM"),
        (DIGGING, "WARDEN_DIG"),
        (EMERGE, "WARDEN_EMERGE"),
        (ROAR, "WARDEN_ROAR"),
        (SNIFF, "WARDEN_SNIFF"),
    ] {
        if let (Some(ms), Some(a)) = (
            m.anim.states[slot].millis(s.age),
            animations.get(&format!("WardenAnimation.{name}")),
        ) {
            a.apply(ms, 1.0, p);
        }
    }
}
/// The emissive layers of `WardenRenderer`: the bioluminescent skin, the
/// pulsating spots, the tendrils and the heart, as (model, texture, alpha).
pub fn emissive_layers(m: &Mob, partial: f32) -> [(&'static str, &'static str, f32); 5] {
    let age = m.age as f32 + partial;
    [
        (
            "warden#bioluminescent",
            "entity/warden/warden_bioluminescent_layer",
            1.0,
        ),
        (
            "warden#pulsating_spots",
            "entity/warden/warden_pulsating_spots_1",
            ((age * 0.045).mcos() * 0.25).max(0.0),
        ),
        (
            "warden#pulsating_spots",
            "entity/warden/warden_pulsating_spots_2",
            ((age * 0.045 + PI).mcos() * 0.25).max(0.0),
        ),
        (
            "warden#tendrils",
            "entity/warden/warden",
            tendril_animation(m, partial),
        ),
        (
            "warden#heart",
            "entity/warden/warden_heart",
            heart_animation(m, partial),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anger_levels_follow_the_thresholds() {
        assert_eq!(AngerLevel::by_anger(0), AngerLevel::Calm);
        assert_eq!(AngerLevel::by_anger(39), AngerLevel::Calm);
        assert_eq!(AngerLevel::by_anger(40), AngerLevel::Agitated);
        assert_eq!(AngerLevel::by_anger(79), AngerLevel::Agitated);
        assert_eq!(AngerLevel::by_anger(80), AngerLevel::Angry);
        assert!(AngerLevel::Angry.is_angry());
        assert_eq!(AngerLevel::Calm.ambient_sound(), "entity.warden.ambient");
        assert_eq!(
            AngerLevel::Agitated.listening_sound(),
            "entity.warden.listening_angry"
        );
    }

    #[test]
    fn anger_accumulates_to_a_cap_and_decays_each_second() {
        let mut a = AngerManagement::default();
        assert_eq!(a.increase(Who::Player, 35), 35);
        assert_eq!(a.increase(Who::Player, 35), 70);
        assert_eq!(a.increase(Who::Player, 100), 150);
        assert_eq!(a.active_anger(None), 150);
        a.tick(|_| true);
        assert_eq!(a.anger_of(Who::Player), 149);
        // An invalid suspect is forgotten.
        a.increase(Who::Mob(3), 10);
        a.tick(|w| w == Who::Player);
        assert_eq!(a.anger_of(Who::Mob(3)), 0);
        assert_eq!(a.suspects(), &[Who::Player]);
        // Anger of one point is gone after the next second.
        let mut b = AngerManagement::default();
        b.increase(Who::Mob(1), 1);
        b.tick(|_| true);
        assert!(b.suspects().is_empty());
    }

    #[test]
    fn the_angriest_player_comes_first() {
        let mut a = AngerManagement::default();
        a.increase(Who::Mob(1), 90);
        a.increase(Who::Player, 50);
        a.increase(Who::Mob(2), 120);
        // Angry suspects first, the angrier mob before the weaker one...
        assert_eq!(a.suspects(), &[Who::Mob(2), Who::Mob(1), Who::Player]);
        // ...until the player is angry as well: players lead ties of level.
        a.increase(Who::Player, 40);
        assert_eq!(a.suspects()[0], Who::Player);
        assert_eq!(a.active_entity(|w| w != Who::Player), Some(Who::Mob(2)));
        assert_eq!(a.active_anger(Some(Who::Mob(1))), 90);
        assert_eq!(a.active_anger(Some(Who::Mob(9))), 0);
        a.clear(Who::Player);
        assert_eq!(a.suspects(), &[Who::Mob(2), Who::Mob(1)]);
        assert_eq!(a.active_anger(None), 120);
    }

    #[test]
    fn a_single_suspect_sets_the_highest_anger() {
        let mut a = AngerManagement::default();
        a.increase(Who::Mob(5), 44);
        assert_eq!(a.active_anger(None), 44);
        a.clear(Who::Mob(5));
        assert_eq!(a.active_anger(None), 0);
    }

    #[test]
    fn the_heart_beats_faster_when_angry() {
        assert_eq!(heart_beat_delay(0), 40);
        assert_eq!(heart_beat_delay(40), 25);
        assert_eq!(heart_beat_delay(80), 10);
        assert_eq!(heart_beat_delay(150), 10);
    }

    #[test]
    fn the_pose_codes_round_trip() {
        for p in [
            Pose::Standing,
            Pose::Emerging,
            Pose::Digging,
            Pose::Roaring,
            Pose::Sniffing,
        ] {
            assert_eq!(Pose::from_code(p.code()), p);
        }
    }
}
