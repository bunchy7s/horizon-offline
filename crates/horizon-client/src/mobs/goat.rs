//! Goats (`Goat`, `GoatAi`): the lowered head (`lowerHeadTick`, entity
//! events 58 and 59) and the ram activity: `PrepareRamNearestTarget` picks
//! the nearest living thing it can see, walks to a spot four to seven
//! blocks away in a straight line and waits there 20 ticks with its head
//! down; `RamTarget` then charges at it, knocks it back, or breaks a horn
//! on a log, stone or ore. Both are one goal here. The long jump activity is
//! the frog's goal (frog.rs) with the goat's numbers.
use super::{
    Damage, Mob, MobEvent, MobWorld, Who,
    goals::{Cx, Goal, LOOK, MOVE, goal_clone},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// `GoatAi.TIME_BETWEEN_RAMS` and `TIME_BETWEEN_RAMS_SCREAMER`.
const RAMS: (i32, i32) = (600, 6000);
const RAMS_SCREAMER: (i32, i32) = (100, 300);
const RAM_PREPARE_TIME: i64 = 20;
const MIN_RAM_DISTANCE: i32 = 4;
const MAX_RAM_DISTANCE: i32 = 7;
/// `PrepareRamNearestTarget.TIME_OUT_DURATION` and `RamTarget`'s.
const PREPARE_TIME_OUT: i32 = 160;
const RAM_TIME_OUT: i32 = 200;
/// `NEAREST_VISIBLE_LIVING_ENTITIES` reaches 16 blocks.
const SENSE_RANGE: f64 = 16.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Done,
    Prepare,
    /// `RAM_TARGET` is set; `RamTarget` starts on the next tick.
    RamStart,
    Ram,
}
#[derive(Clone, Copy, Debug)]
struct Candidate {
    start: [i32; 3],
    target_pos: [i32; 3],
    target: Who,
}

/// The lowered head and the ram behaviours' state. `RAM_COOLDOWN_TICKS` is a
/// brain memory (`ram_cooldown_ticks`), saved with the others.
#[derive(Clone, Debug, Default)]
pub struct State {
    lower_head_tick: i32,
    lowering_head: bool,
    phase: Phase,
    candidate: Option<Candidate>,
    reached_at: Option<i32>,
    /// `RAM_TARGET`, and the direction of the charge.
    ram_target: Option<Vec3>,
    ram_direction: Vec3,
    time: i32,
    /// `LivingEntity.discardFriction`, set through the long jump.
    pub discard_friction: bool,
}

pub fn register(m: &mut Mob) {
    m.ai.goals.add(2, Ram);
    // `GoatAi.initLongJumpActivity`: the long jump activity comes after the
    // ram activity and before the idle one.
    m.ai.goals.add(2, super::frog::LongJump::default());
}
/// The goat's long jump activity is the one in use: `GoatAi` tries the ram
/// activity first, so the ram cooldown runs, and the long jump activity wants
/// no tempting player, no breed target and no walk target (the navigation is
/// idle).
pub(super) fn long_jump_activity(m: &Mob, w: &MobWorld) -> bool {
    ram_cooldown(m) > 0 && m.ai.nav.is_done() && !tempted(m, w) && !breeding(m, w)
}
/// `TemptingSensor.forAnimal`: a player within 10 blocks holds goat food.
fn tempted(m: &Mob, w: &MobWorld) -> bool {
    let holds = |item: &Option<std::sync::Arc<str>>| {
        item.as_deref()
            .is_some_and(|i| m.kind.food.iter().any(|f| f.as_str() == i))
    };
    w.player.as_ref().is_some_and(|p| {
        p.alive
            && !p.spectator
            && m.distance_sqr(p.position) <= 100.0
            && (holds(&p.main_hand) || holds(&p.off_hand))
            && m.rider.is_none()
    })
}
/// `AnimalMakeLove.checkExtraStartConditions`: the goat is in love and an
/// adult goat in love is near, so `BREED_TARGET` is set or about to be.
fn breeding(m: &Mob, w: &MobWorld) -> bool {
    let id = m.id;
    m.extra.in_love > 0
        && w.nearby(m.position, 8.0, move |s| {
            s.id != id && s.species == super::Species::Goat && s.in_love && !s.baby
        })
        .next()
        .is_some()
}

/// `Goat.handleEntityEvent`: 58 lowers the head, 59 raises it.
pub fn entity_event(m: &mut Mob, event: u8) {
    match event {
        58 => m.extra.goat.lowering_head = true,
        59 => m.extra.goat.lowering_head = false,
        _ => {}
    }
}
/// `Entity.setPose` and the `refreshDimensions` it triggers, for a goat of a
/// server: the pose's box (`Goat.getDefaultDimensions` scales a long jump's
/// by 0.7), and a box that grew (the goat lands) is moved to a free spot
/// (`fudgePositionAfterSizeChange`).
pub fn set_pose(m: &mut Mob, w: &MobWorld, id: u8) {
    let (old_w, old_h, _) = m.dims();
    m.extra.pose = id;
    m.fudge_position_after_size_change(w, old_w, old_h);
}
/// `Goat.aiStep`'s `lowerHeadTick`; a client runs it too.
pub fn tick(m: &mut Mob) {
    let g = &mut m.extra.goat;
    g.lower_head_tick += if g.lowering_head { 1 } else { -2 };
    g.lower_head_tick = g.lower_head_tick.clamp(0, 20);
    // `CountDownCooldownTicks(RAM_COOLDOWN_TICKS)`: one less each tick, erased
    // once it runs out.
    let cooldown = ram_cooldown(m);
    if cooldown > 0 {
        set_ram_cooldown(m, cooldown - 1);
    }
    // `CountDownCooldownTicks(LONG_JUMP_COOLDOWN_TICKS)` of the core activity.
    super::frog::count_down_cooldown(m);
}
/// `RAM_COOLDOWN_TICKS`: 0 when absent.
pub fn ram_cooldown(m: &Mob) -> i32 {
    m.extra
        .memories
        .iter()
        .find(|x| x.0 == RAM_COOLDOWN)
        .map_or(0, |x| x.1 as i32)
}
fn set_ram_cooldown(m: &mut Mob, ticks: i32) {
    let memories = &mut m.extra.memories;
    match memories.iter().position(|x| x.0 == RAM_COOLDOWN) {
        Some(i) if ticks > 0 => memories[i].1 = i64::from(ticks),
        Some(i) => {
            memories.remove(i);
        }
        None if ticks > 0 => memories.push((RAM_COOLDOWN, i64::from(ticks), i64::MAX)),
        None => {}
    }
}
const RAM_COOLDOWN: &str = "ram_cooldown_ticks";
/// `Goat.getRammingXHeadRot`.
pub fn ramming_x_head_rot(m: &Mob) -> f32 {
    let max = if m.is_baby() { 52.5_f32 } else { 30.0 };
    m.extra.goat.lower_head_tick as f32 / 20.0 * max * (std::f32::consts::PI / 180.0)
}
/// A server's `level.broadcastEntityEvent(goat, 58 or 59)`, sent when it
/// changes what the goat does.
fn lower_head(m: &mut Mob, events: &mut Vec<MobEvent>, lower: bool) {
    if m.extra.goat.lowering_head != lower {
        let event: i8 = if lower { 58 } else { 59 };
        entity_event(m, event as u8);
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::EntityEvent {
            mob: m.id,
            event,
            at,
        });
    }
}
fn between(m: &mut Mob, (min, max): (i32, i32)) -> i32 {
    m.random.next_int_between_inclusive(min, max).unwrap_or(min)
}
fn rams(m: &Mob) -> (i32, i32) {
    if m.extra.screaming {
        RAMS_SCREAMER
    } else {
        RAMS
    }
}
fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
fn play(m: &Mob, events: &mut Vec<MobEvent>, sound: &'static str, pitch: f32) {
    if !m.silent {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::Sound(sound, at, 1.0, pitch));
    }
}

/// `PrepareRamNearestTarget.isWalkableBlock`.
fn walkable(m: &Mob, w: &MobWorld, p: [i32; 3]) -> bool {
    m.stable_destination(w, p) && m.malus(w.path_type_static(p[0], p[1], p[2])) == 0.0
}
/// `PrepareRamNearestTarget.calculateRammingStartPosition`.
fn ramming_start(m: &mut Mob, w: &MobWorld, target: [i32; 3]) -> Option<[i32; 3]> {
    if !walkable(m, w, target) {
        return None;
    }
    // Direction.Plane.HORIZONTAL: north, east, south, west.
    let mut starts = Vec::new();
    for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
        let mut p = target;
        for _ in 0..MAX_RAM_DISTANCE {
            p = [p[0] + dx, p[1], p[2] + dz];
            if !walkable(m, w, p) {
                p = [p[0] - dx, p[1], p[2] - dz];
                break;
            }
        }
        let manhattan =
            (p[0] - target[0]).abs() + (p[1] - target[1]).abs() + (p[2] - target[2]).abs();
        if manhattan >= MIN_RAM_DISTANCE {
            starts.push(p);
        }
    }
    let here = block_of(m.position);
    let sqr = |p: &[i32; 3]| {
        (0..3)
            .map(|i| i64::from(p[i] - here[i]).pow(2))
            .sum::<i64>()
    };
    starts.sort_by_key(sqr);
    starts
        .into_iter()
        .find(|&p| m.create_path_to(w, p, 0).is_some_and(|path| path.reached))
}
/// `PrepareRamNearestTarget.chooseRamPosition`.
fn choose(m: &mut Mob, w: &MobWorld, target: Who) {
    let g = &mut m.extra.goat;
    g.reached_at = None;
    let Some(t) = w.target_info(target) else {
        g.candidate = None;
        return;
    };
    let target_pos = block_of(t.position);
    m.extra.goat.candidate = ramming_start(m, w, target_pos).map(|start| Candidate {
        start,
        target_pos,
        target,
    });
}
/// The nearest living thing the goat sees that `RAM_TARGET_CONDITIONS`
/// allow: not a goat, a player who is not creative or a spectator.
fn nearest_target(m: &mut Mob, w: &MobWorld) -> Option<Who> {
    let mut best: Option<(Who, f64)> = None;
    if let Some(p) = &w.player
        && p.alive
        && !p.creative
        && !p.spectator
        && !p.allies.contains(&m.id)
        && m.distance_sqr(p.position) <= SENSE_RANGE * SENSE_RANGE
        && m.sees(w, Who::Player)
    {
        best = Some((Who::Player, m.distance_sqr(p.position)));
    }
    let id = m.id;
    let mobs: Vec<(u32, f64)> = w
        .nearby(m.position, SENSE_RANGE, |s| {
            s.id != id && s.species != super::Species::Goat && !s.playing_dead
        })
        .map(|s| (s.id, m.distance_sqr(s.position)))
        .collect();
    for (mob, d) in mobs {
        if best.is_none_or(|b| d < b.1) && m.sees(w, Who::Mob(mob)) {
            best = Some((Who::Mob(mob), d));
        }
    }
    best.map(|b| b.0)
}

/// `PrepareRamNearestTarget` and `RamTarget` of the ram activity.
#[derive(Clone, Debug)]
struct Ram;
impl Goal for Ram {
    goal_clone!();
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, m: &mut Mob, _cx: &mut Cx) -> bool {
        // The ram activity needs the cooldown to be over (it also waits for
        // no tempting player and no breed target, which goats lack here).
        ram_cooldown(m) <= 0
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let g = &m.extra.goat;
        match g.phase {
            Phase::Done => false,
            Phase::Prepare => {
                g.time <= PREPARE_TIME_OUT
                    && g.candidate
                        .is_some_and(|c| cx.w.target_info(c.target).is_some_and(|t| t.alive))
            }
            Phase::RamStart | Phase::Ram => g.ram_target.is_some() && g.time <= RAM_TIME_OUT,
        }
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.goat.phase = Phase::Prepare;
        m.extra.goat.time = 0;
        m.extra.goat.reached_at = None;
        m.extra.goat.candidate = None;
        if let Some(target) = nearest_target(m, cx.w) {
            choose(m, cx.w, target);
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        match m.extra.goat.phase {
            // PrepareRamNearestTarget.stop: no ram target, so the head comes
            // up and the cooldown starts.
            Phase::Prepare => {
                lower_head(m, cx.events, false);
                let wait = rams(m).0;
                set_ram_cooldown(m, wait);
            }
            // RamTarget.finishRam (also what its timeout leaves undone).
            Phase::RamStart | Phase::Ram => finish_ram(m, cx.events),
            Phase::Done => {}
        }
        m.extra.goat.phase = Phase::Done;
        m.extra.goat.candidate = None;
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.extra.goat.time += 1;
        match m.extra.goat.phase {
            Phase::Prepare => prepare_tick(m, cx),
            Phase::RamStart => {
                // RamTarget.start: charge from here at the ram target.
                let (Some(target), here) = (m.extra.goat.ram_target, block_of(m.position)) else {
                    return;
                };
                let d = Vec3::new(
                    f64::from(here[0]) - target.x,
                    0.0,
                    f64::from(here[2]) - target.z,
                );
                m.extra.goat.ram_direction = d.normalize();
                m.extra.goat.phase = Phase::Ram;
                m.extra.goat.time = 0;
                m.nav_move_to_entity(cx.w, [target.x, target.y, target.z], 0, 3.0);
                ram_tick(m, cx);
            }
            Phase::Ram => ram_tick(m, cx),
            Phase::Done => {}
        }
    }
}
/// `PrepareRamNearestTarget.tick`.
fn prepare_tick(m: &mut Mob, cx: &mut Cx) {
    let Some(c) = m.extra.goat.candidate else {
        return;
    };
    let Some(t) = cx.w.target_info(c.target) else {
        return;
    };
    m.look_at([t.position.x, t.eye_y, t.position.z]);
    let now = m.age as i32;
    let at = [
        f64::from(c.start[0]) + 0.5,
        f64::from(c.start[1]),
        f64::from(c.start[2]) + 0.5,
    ];
    if m.ai.nav.is_done() && block_of(m.position) != c.start {
        m.nav_move_to_entity(cx.w, at, 0, 1.25);
    }
    if block_of(t.position) != c.target_pos {
        // The target moved: the head comes up and a new spot is chosen.
        lower_head(m, cx.events, false);
        m.nav_stop();
        choose(m, cx.w, c.target);
    } else if block_of(m.position) == c.start {
        lower_head(m, cx.events, true);
        let since = *m.extra.goat.reached_at.get_or_insert(now);
        if i64::from(now - since) >= RAM_PREPARE_TIME {
            // getEdgeOfBlock.
            let (x, z) = (
                f64::from(c.target_pos[0])
                    + 0.5
                    + 0.5 * f64::from(sign(c.target_pos[0] - c.start[0])),
                f64::from(c.target_pos[2])
                    + 0.5
                    + 0.5 * f64::from(sign(c.target_pos[2] - c.start[2])),
            );
            m.extra.goat.ram_target = Some(Vec3::new(x, f64::from(c.target_pos[1]), z));
            let sound = if m.extra.screaming {
                "entity.goat.screaming.prepare_ram"
            } else {
                "entity.goat.prepare_ram"
            };
            let pitch = m.voice_pitch();
            play(m, cx.events, sound, pitch);
            m.extra.goat.candidate = None;
            m.extra.goat.phase = Phase::RamStart;
        }
    }
}
fn sign(v: i32) -> i32 {
    v.signum()
}
/// `RamTarget.tick`.
fn ram_tick(m: &mut Mob, cx: &mut Cx) {
    let impact = if m.extra.screaming {
        "entity.goat.screaming.ram_impact"
    } else {
        "entity.goat.ram_impact"
    };
    let from = [m.position.x, m.position.y, m.position.z];
    if let Some(target) = rammed(m, cx.w) {
        // hurtServer by `noAggroMobAttack`, then the knockback.
        let amount = m.attrs.attack_damage;
        match target {
            Who::Player => cx.events.push(MobEvent::DamagePlayer {
                amount,
                from,
                kind: "minecraft:mob_attack_no_aggro",
            }),
            Who::Mob(id) => cx.events.push(MobEvent::AttackMob {
                target: id,
                amount,
                source: Damage::mob(m.id, from),
            }),
        }
        let level = |effect: &str| {
            m.extra
                .effects
                .iter()
                .find(|e| e.0 == effect)
                .map_or(0, |e| e.2 + 1)
        };
        let boost = 0.25 * (level("minecraft:speed") - level("minecraft:slowness")) as f32;
        let factor = (m.ai.speed * 1.65).clamp(0.2, 3.0) + boost;
        let force = if m.is_baby() { 1.0 } else { 2.5 };
        let d = m.extra.goat.ram_direction;
        // The knockback's source is `mobAttack`, not the hurt's `noAggroMobAttack`.
        cx.events.push(MobEvent::Knockback {
            target,
            power: f64::from(factor * force),
            x: d.x,
            z: d.z,
            source: Damage::mob(m.id, from),
        });
        finish_ram(m, cx.events);
        play(m, cx.events, impact, 1.0);
    } else if rammed_horn_block(m, cx.w) {
        play(m, cx.events, impact, 1.0);
        if drop_horn(m, cx.events) {
            play(m, cx.events, "entity.goat.horn_break", 1.0);
        }
        finish_ram(m, cx.events);
    } else if m.ai.nav.is_done() || m.extra.goat.ram_target.is_none() {
        // The walk target is gone: the charge ended where it could.
        finish_ram(m, cx.events);
    }
}
/// The first living thing in the goat's box that `RAM_TARGET_CONDITIONS`
/// allow.
fn rammed(m: &Mob, w: &MobWorld) -> Option<Who> {
    let bounds = m.bounds();
    if let Some(p) = &w.player
        && p.alive
        && !p.creative
        && !p.spectator
        && !p.allies.contains(&m.id)
        && p.bounds.intersects(bounds)
    {
        return Some(Who::Player);
    }
    w.others_near(bounds)
        .find(|s| {
            s.id != m.id
                && s.alive
                && s.species != super::Species::Goat
                && !s.playing_dead
                && s.bounds.intersects(bounds)
        })
        .map(|s| Who::Mob(s.id))
}
/// `RamTarget.hasRammedHornBreakingBlock`: the block one step ahead, or the
/// one above it, snaps a horn.
fn rammed_horn_block(m: &Mob, w: &MobWorld) -> bool {
    let ahead = Vec3::new(m.velocity.x, 0.0, m.velocity.z).normalize();
    let at = block_of(m.position.add_vector(ahead));
    let snaps =
        |p: [i32; 3]| w.has_tag("minecraft:snaps_goat_horn", w.block_name(p[0], p[1], p[2]));
    snaps(at) || snaps([at[0], at[1] + 1, at[2]])
}
/// `Goat.dropHorn`: one of the horns it still has comes off and drops.
fn drop_horn(m: &mut Mob, events: &mut Vec<MobEvent>) -> bool {
    if m.is_baby() {
        return false;
    }
    let [left_lost, right_lost] = m.extra.lost_horns;
    if left_lost && right_lost {
        return false;
    }
    let lose_left = if left_lost {
        false
    } else if right_lost {
        true
    } else {
        m.random.next_boolean()
    };
    m.extra.lost_horns[usize::from(!lose_left)] = true;
    // `Mth.randomBetween(this.random, ..)` for the push, drawn after createHorn.
    let mut between = |min: f32, max: f32| f64::from(m.random.next_float() * (max - min) + min);
    let motion = [between(-0.2, 0.2), between(0.3, 0.7), between(-0.2, 0.2)];
    events.push(MobEvent::DropGoatHorn {
        mob: m.id,
        screaming: m.extra.screaming,
        at: [m.position.x, m.position.y, m.position.z],
        motion,
    });
    true
}
/// `Goat.createHorn`: the instrument is a random pick (by a random seeded
/// with the goat's UUID hash) from `#regular_goat_horns` or, for a screaming
/// goat, `#screaming_goat_horns`.
pub fn create_horn(uuid: [i32; 4], screaming: bool) -> &'static str {
    const REGULAR: [&str; 4] = [
        "minecraft:ponder_goat_horn",
        "minecraft:sing_goat_horn",
        "minecraft:seek_goat_horn",
        "minecraft:feel_goat_horn",
    ];
    const SCREAMING: [&str; 4] = [
        "minecraft:admire_goat_horn",
        "minecraft:call_goat_horn",
        "minecraft:yearn_goat_horn",
        "minecraft:dream_goat_horn",
    ];
    // The UUID's Java hash code: the two halves xored, then folded to an int.
    let [a, b, c, d] = uuid;
    let mut random = horizon_core::random::LegacyRandom::new(i64::from(a ^ b ^ c ^ d));
    let list = if screaming { SCREAMING } else { REGULAR };
    list[random.next_int_bound(list.len() as i32).unwrap_or(0) as usize]
}
/// `RamTarget.finishRam`.
fn finish_ram(m: &mut Mob, events: &mut Vec<MobEvent>) {
    lower_head(m, events, false);
    let wait = rams(m);
    let cooldown = between(m, wait);
    set_ram_cooldown(m, cooldown);
    m.extra.goat.ram_target = None;
    m.extra.goat.phase = Phase::Done;
}
