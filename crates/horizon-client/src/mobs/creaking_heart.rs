//! The creaking heart's block entity (`CreakingHeartBlockEntity`): at night
//! an awake heart in a pale oak trunk spawns a creaking that protects it
//! (`SpawnUtil.trySpawnMob`), lets it go by day, when it strays or when a
//! player is stuck in it, and when that creaking is hurt by a player emits
//! trails and spreads resin over the logs. The game and the mob AI probe
//! both tick hearts through this module; the host applies the block changes
//! and plays what it reports.
use super::{Mob, MobEvent, MobType, MobWorld, creaking};
use crate::{collisions::SceneCollisions, containers::CreakingHeart};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::registry::BlockStateId;
use std::sync::Arc;

/// `Creaking.CREAKING_ORANGE` and `CREAKING_GRAY`.
pub const ORANGE: u32 = 16_545_810;
pub const GRAY: u32 = 6_250_335;
/// `CreakingHeartBlockEntity.DISTANCE_CREAKING_TOO_FAR`.
const TOO_FAR: f64 = 34.0;
/// `Direction.values()`: down, up, north, south, west, east.
const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
const FACE_NAMES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

/// What a heart's tick did, for its host to apply.
#[derive(Clone, Debug)]
pub enum HeartEvent {
    /// `Level.setBlockAndUpdate`.
    SetBlock([i32; 3], BlockStateId),
    /// `Level.playSound(null, ...)`: heard by everyone.
    Sound(&'static str, [f64; 3], f32, f32),
    /// A `TrailParticleOption` sent from `at` towards `target`.
    Trail {
        at: [f64; 3],
        target: [f64; 3],
        color: u32,
        duration: i32,
    },
    /// `updateNeighbourForOutputSignal`: comparators read the new signal.
    OutputChanged([i32; 3]),
    /// A mob's own sounds and effects (the spawned creaking's voice, a
    /// creaking falling apart).
    Mob(MobEvent),
}

/// The mobs a heart sees and the creakings it spawns.
pub trait HeartHost {
    fn mob(&self, id: u32) -> Option<&Mob>;
    fn mob_mut(&mut self, id: u32) -> Option<&mut Mob>;
    /// The loaded mob with this UUID.
    fn mob_for_uuid(&self, uuid: [i32; 4]) -> Option<u32>;
    fn uuid_for(&mut self, id: u32) -> [i32; 4];
    /// The creaking's entity type.
    fn creaking_type(&self) -> Option<Arc<MobType>>;
    /// A new mob of the type (the next entity id, its random seeded).
    fn create_mob(&mut self, kind: Arc<MobType>, at: Vec3) -> Mob;
    /// `Level.addFreshEntity`.
    fn add_mob(&mut self, m: Mob);
    /// `ServerLevel.isSpawningMonsters` (rules and difficulty).
    fn spawning_monsters(&self) -> bool;
}

/// Block reads during a tick see the tick's own changes.
struct Blocks<'a, 'w> {
    w: &'a MobWorld<'w>,
    changed: Vec<([i32; 3], BlockStateId)>,
}
impl Blocks<'_, '_> {
    fn get(&self, p: [i32; 3]) -> BlockStateId {
        self.changed
            .iter()
            .rev()
            .find(|(q, _)| *q == p)
            .map_or_else(|| self.w.world.block(p[0], p[1], p[2]), |(_, s)| *s)
    }
    fn set(&mut self, p: [i32; 3], s: BlockStateId, out: &mut Vec<HeartEvent>) {
        self.changed.push((p, s));
        out.push(HeartEvent::SetBlock(p, s));
    }
    fn name(&self, s: BlockStateId) -> &str {
        &self.w.facts.get(s).name
    }
    fn prop(&self, s: BlockStateId, key: &str) -> Option<String> {
        self.w
            .registry
            .state(s)
            .and_then(|st| st.properties.get(key).cloned())
    }
    fn with_prop(&self, s: BlockStateId, key: &str, value: &str) -> Option<BlockStateId> {
        let mut st = self.w.registry.state(s)?.clone();
        st.properties.insert(key.to_owned(), value.to_owned());
        self.w.registry.resolve(&st).ok()
    }
    fn is_pale_oak_log(&self, p: [i32; 3]) -> bool {
        let s = self.get(p);
        self.w.has_tag("minecraft:pale_oak_logs", self.name(s))
    }
}

/// `EnvironmentAttributes.CREAKING_ACTIVE`: the overworld's night, from
/// 12600 to 23400.
pub fn creaking_active(w: &MobWorld) -> bool {
    !w.nether && !w.end && (12600..23401).contains(&w.day_time.rem_euclid(24000))
}
/// `CreakingHeartBlock.hasRequiredLogs`: pale oak logs on both ends of the
/// heart's axis, along the same axis.
fn has_required_logs(b: &Blocks, state: BlockStateId, pos: [i32; 3]) -> bool {
    let axis = b.prop(state, "axis").unwrap_or_else(|| "y".into());
    let dirs: [usize; 2] = match axis.as_str() {
        "x" => [4, 5],
        "z" => [2, 3],
        _ => [0, 1],
    };
    dirs.iter().all(|&d| {
        let n = [
            pos[0] + DIRECTIONS[d][0],
            pos[1] + DIRECTIONS[d][1],
            pos[2] + DIRECTIONS[d][2],
        ];
        let s = b.get(n);
        b.w.has_tag("minecraft:pale_oak_logs", b.name(s))
            && b.prop(s, "axis").as_deref() == Some(axis.as_str())
    })
}

/// `getCreakingProtector`: the protector's entity, resolving a UUID read
/// from a save (forgotten 30 ticks after the heart came back unresolved).
pub fn protector(heart: &mut CreakingHeart, host: &impl HeartHost) -> Option<u32> {
    let (id, uuid) = heart.creaking?;
    if let Some(id) = id {
        if host.mob(id).is_some_and(|m| !m.removed) {
            return Some(id);
        }
        // setCreakingInfo(uuid)
        heart.creaking = Some((None, uuid));
        heart.ticks_existed = 0;
    }
    if let Some(id) = host.mob_for_uuid(uuid) {
        heart.creaking = Some((Some(id), uuid));
        return Some(id);
    }
    if heart.ticks_existed >= 30 {
        heart.creaking = None;
    }
    None
}
fn distance_to_creaking(heart: &mut CreakingHeart, pos: [i32; 3], host: &impl HeartHost) -> f64 {
    protector(heart, host)
        .and_then(|id| host.mob(id))
        .map_or(0.0, |m| {
            let at = Vec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.5,
            );
            m.distance_sqr(at).sqrt()
        })
}
/// `computeAnalogOutputSignal`.
fn output_signal(heart: &mut CreakingHeart, pos: [i32; 3], host: &impl HeartHost) -> i32 {
    if heart.creaking.is_none() || protector(heart, host).is_none() {
        return 0;
    }
    let scaled = distance_to_creaking(heart, pos, host).clamp(0.0, 32.0) / 32.0;
    15 - (scaled * 15.0).floor() as i32
}
/// `removeProtector`: the creaking falls apart (`tearDown`), or with a
/// damage source (`by_player`) twitches to death.
pub fn remove_protector(
    heart: &mut CreakingHeart,
    host: &mut impl HeartHost,
    by_player: bool,
    out: &mut Vec<HeartEvent>,
) {
    let Some(id) = protector(heart, host) else {
        return;
    };
    if let Some(m) = host.mob_mut(id) {
        let mut events = Vec::new();
        if by_player {
            // creakingDeathEffects, setTearingDown, setHealth(0).
            creaking::set_tearing_down(m);
            m.health = 0.0;
            m.voice("entity.creaking.twitch", &mut events);
        } else {
            creaking::tear_down(m, &mut events);
        }
        out.extend(events.into_iter().map(HeartEvent::Mob));
    }
    heart.creaking = None;
}
/// `emitParticles`: trails between the protector and the heart.
fn emit_particles(
    heart: &mut CreakingHeart,
    pos: [i32; 3],
    count: i32,
    towards: bool,
    lr: &mut LegacyRandom,
    host: &impl HeartHost,
    out: &mut Vec<HeartEvent>,
) {
    let Some(b) = protector(heart, host)
        .and_then(|id| host.mob(id))
        .map(Mob::bounds)
    else {
        return;
    };
    let (lo, hi) = (b.min_position(), b.max_position());
    for _ in 0..count {
        let mut source = [
            lo.x + lr.next_double() * (hi.x - lo.x),
            lo.y + lr.next_double() * (hi.y - lo.y),
            lo.z + lr.next_double() * (hi.z - lo.z),
        ];
        let mut destination = [
            f64::from(pos[0]) + lr.next_double(),
            f64::from(pos[1]) + lr.next_double(),
            f64::from(pos[2]) + lr.next_double(),
        ];
        if towards {
            std::mem::swap(&mut source, &mut destination);
        }
        let duration = lr.next_int_bound(40).unwrap_or(0) + 10;
        out.push(HeartEvent::Trail {
            at: source,
            target: destination,
            color: if towards { ORANGE } else { GRAY },
            duration,
        });
    }
}
/// `Util.shuffledCopy(Direction.values(), random)`.
fn shuffled_directions(lr: &mut LegacyRandom) -> [usize; 6] {
    let mut order = [0, 1, 2, 3, 4, 5];
    for i in (2..=6).rev() {
        let j = lr.next_int_bound(i as i32).unwrap_or(0) as usize;
        order.swap(i - 1, j);
    }
    order
}
fn offset(p: [i32; 3], d: usize) -> [i32; 3] {
    [
        p[0] + DIRECTIONS[d][0],
        p[1] + DIRECTIONS[d][1],
        p[2] + DIRECTIONS[d][2],
    ]
}
/// `spreadResin`: a resin clump on the first log face found within two steps
/// of pale oak logs (`BlockPos.breadthFirstTraversal(pos, 2, 64, ...)`).
fn spread_resin(
    b: &mut Blocks,
    pos: [i32; 3],
    lr: &mut LegacyRandom,
    out: &mut Vec<HeartEvent>,
) -> Option<[i32; 3]> {
    let mut visited: Vec<[i32; 3]> = Vec::new();
    let mut queue: std::collections::VecDeque<([i32; 3], i32)> = Default::default();
    queue.push_back((pos, 0));
    let mut count = 0;
    while let Some((current, depth)) = queue.pop_front() {
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);
        // The node visitor.
        if b.is_pale_oak_log(current) {
            for d in shuffled_directions(lr) {
                let n = offset(current, d);
                let mut neighbour = b.get(n);
                let opposite = d ^ 1;
                let facts = b.w.facts.get(neighbour);
                if b.w.facts.is_air(neighbour) {
                    neighbour = b.w.registry.default_state("minecraft:resin_clump")?;
                } else if &*facts.name == "minecraft:water"
                    && facts.fluid_level == 8
                    && !facts.fluid_falling
                {
                    let clump = b.w.registry.default_state("minecraft:resin_clump")?;
                    neighbour = b.with_prop(clump, "waterlogged", "true")?;
                }
                if b.name(neighbour) == "minecraft:resin_clump"
                    && b.prop(neighbour, FACE_NAMES[opposite]).as_deref() != Some("true")
                {
                    let placed = b.with_prop(neighbour, FACE_NAMES[opposite], "true")?;
                    b.set(n, placed, out);
                    return Some(n);
                }
            }
        }
        count += 1;
        if count >= 64 {
            return None;
        }
        if depth < 2 {
            // The neighbour provider: adjacent pale oak logs.
            for d in shuffled_directions(lr) {
                let n = offset(current, d);
                if b.is_pale_oak_log(n) {
                    queue.push_back((n, depth + 1));
                }
            }
        }
    }
    None
}
/// `creakingHurt`: a player hurt the protector.
pub fn creaking_hurt(
    heart: &mut CreakingHeart,
    pos: [i32; 3],
    w: &MobWorld,
    lr: &mut LegacyRandom,
    host: &mut impl HeartHost,
    out: &mut Vec<HeartEvent>,
) {
    let Some(id) = protector(heart, host) else {
        return;
    };
    if heart.emitter > 0 {
        return;
    }
    emit_particles(heart, pos, 20, false, lr, host, out);
    let mut b = Blocks {
        w,
        changed: Vec::new(),
    };
    let state = b.get(pos);
    if b.prop(state, "creaking_heart_state").as_deref() == Some("awake") {
        let clumps = lr.next_int_between_inclusive(2, 3).unwrap_or(2);
        for _ in 0..clumps {
            if let Some(at) = spread_resin(&mut b, pos, lr, out) {
                out.push(HeartEvent::Sound("block.resin.place", centre(at), 1.0, 1.0));
            }
        }
    }
    heart.emitter = 100;
    heart.emitter_target = host.mob(id).map(Mob::center);
}
fn centre(p: [i32; 3]) -> [f64; 3] {
    [
        f64::from(p[0]) + 0.5,
        f64::from(p[1]) + 0.5,
        f64::from(p[2]) + 0.5,
    ]
}

/// `SpawnUtil.Strategy.ON_TOP_OF_COLLIDER_NO_LEAVES`.
fn can_spawn_on(b: &Blocks, pos: [i32; 3], state: BlockStateId, above: BlockStateId) -> bool {
    let mut scene = SceneCollisions {
        world: b.w.world,
        binding: b.w.collision,
        subject: None,
    };
    let above_pos = [pos[0], pos[1] + 1, pos[2]];
    let above_empty =
        b.w.collision
            .collision_shape(&mut scene, above, above_pos)
            .is_ok_and(|s| s.is_empty());
    above_empty
        && !b.w.has_tag("minecraft:leaves", b.name(state))
        && b.w.facts.get(state).sturdy_full & (1 << 1) != 0
}
/// `SpawnUtil.trySpawnMob(CREAKING, SPAWNER, level, pos, 5, 16, 8,
/// ON_TOP_OF_COLLIDER_NO_LEAVES, true)` and `spawnProtector`.
fn spawn_protector(
    pos: [i32; 3],
    w: &MobWorld,
    lr: &mut LegacyRandom,
    host: &mut impl HeartHost,
    out: &mut Vec<HeartEvent>,
) -> Option<u32> {
    let kind = host.creaking_type()?;
    let b = Blocks {
        w,
        changed: Vec::new(),
    };
    for _ in 0..5 {
        let dx = lr.next_int_between_inclusive(-16, 16).unwrap_or(0);
        let dz = lr.next_int_between_inclusive(-16, 16).unwrap_or(0);
        let mut at = [pos[0] + dx, pos[1] + 8, pos[2] + dz];
        if !w.world.loaded(at[0], at[2]) {
            continue;
        }
        // moveToPossibleSpawnPosition
        let mut above = b.get(at);
        let mut found = false;
        for _ in (-8..=8).rev() {
            at[1] -= 1;
            let current = b.get(at);
            if can_spawn_on(&b, at, current, above) {
                at[1] += 1;
                found = true;
                break;
            }
            above = current;
        }
        if !found {
            continue;
        }
        // noCollision(getSpawnAABB)
        let half = f64::from(kind.width as f32 / 2.0);
        let height = f64::from(kind.height as f32);
        let (x, y, z) = (
            f64::from(at[0]) + 0.5,
            f64::from(at[1]),
            f64::from(at[2]) + 0.5,
        );
        let bounds = Aabb::new(
            Vec3::new(x - half, y, z - half),
            Vec3::new(x + half, y + height, z + half),
        );
        let mut scene = SceneCollisions {
            world: w.world,
            binding: w.collision,
            subject: None,
        };
        if !scene.unobstructed(bounds).unwrap_or(false) {
            continue;
        }
        // EntityType.create: a random yaw, then Mob.finalizeSpawn.
        let mut m = host.create_mob(kind.clone(), Vec3::new(x, y, z));
        let yaw = wrap_degrees(lr.next_float() * 360.0);
        m.yaw = yaw;
        m.previous_yaw = yaw;
        m.head_yaw = yaw;
        m.body_yaw = yaw;
        finalize_spawn(&mut m, lr);
        // Mob.checkSpawnRules (always true for a creaking) and
        // checkSpawnObstruction (no liquid in the box).
        if liquid_in(w, m.bounds()) {
            continue;
        }
        let id = m.id;
        // SpawnUtil: addFreshEntityWithPassengers, then playAmbientSound.
        let mut events = Vec::new();
        if !creaking::is_active(&m) {
            m.voice("entity.creaking.ambient", &mut events);
        }
        creaking::set_transient(&mut m, pos);
        // spawnProtector's caller: the creaking's spawn sound.
        m.voice("entity.creaking.spawn", &mut events);
        host.add_mob(m);
        out.extend(events.into_iter().map(HeartEvent::Mob));
        out.push(HeartEvent::Sound(
            "block.creaking_heart.spawn",
            centre(pos),
            1.0,
            1.0,
        ));
        return Some(id);
    }
    None
}
/// `Mob.finalizeSpawn`: the follow range's random spawn bonus and the
/// handedness, from the level random.
pub fn finalize_spawn(m: &mut Mob, lr: &mut LegacyRandom) {
    let bonus = lr.triangle(0.0, 0.114_850_000_000_000_01);
    // ADD_MULTIPLIED_BASE on the base value.
    m.attrs.follow_range += m.kind.follow_range * bonus;
    m.update_max_visited_nodes();
    let _left_handed = lr.next_float() < 0.05;
}
fn wrap_degrees(angle: f32) -> f32 {
    let mut w = angle % 360.0;
    if w >= 180.0 {
        w -= 360.0;
    }
    if w < -180.0 {
        w += 360.0;
    }
    w
}
fn liquid_in(w: &MobWorld, b: Aabb) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    for x in lo.x.floor() as i32..=(hi.x.ceil() as i32 - 1) {
        for y in lo.y.floor() as i32..=(hi.y.ceil() as i32 - 1) {
            for z in lo.z.floor() as i32..=(hi.z.ceil() as i32 - 1) {
                if w.fluid(x, y, z) != crate::blocks::Fluid::None {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether this tick of the heart reaches its update (`updateCreakingState`
/// and the protector checks), the only part of [`server_tick`] that reads the
/// level's other mobs: about once a second, so a host may build them only then.
pub fn tick_updates(heart: &CreakingHeart) -> bool {
    heart.ticker < 0
}

/// `CreakingHeartBlockEntity.serverTick` (hearts that are not uprooted).
pub fn server_tick(
    heart: &mut CreakingHeart,
    pos: [i32; 3],
    w: &MobWorld,
    lr: &mut LegacyRandom,
    host: &mut impl HeartHost,
    out: &mut Vec<HeartEvent>,
) {
    heart.ticks_existed += 1;
    let signal = output_signal(heart, pos, host);
    if heart.output_signal != signal {
        heart.output_signal = signal;
        out.push(HeartEvent::OutputChanged(pos));
    }
    if heart.emitter > 0 {
        if heart.emitter > 50 {
            emit_particles(heart, pos, 1, true, lr, host, out);
            emit_particles(heart, pos, 1, false, lr, host, out);
        }
        if heart.emitter % 10 == 0 && heart.emitter_target.is_some() {
            if let Some(m) = protector(heart, host).and_then(|id| host.mob(id)) {
                heart.emitter_target = Some(m.center());
            }
            if let Some(target) = heart.emitter_target {
                let c = centre(pos);
                let progress = 0.2 + 0.8 * f64::from(100 - heart.emitter) / 100.0;
                let sound = [
                    (c[0] - target[0]) * progress + target[0],
                    (c[1] - target[1]) * progress + target[1],
                    (c[2] - target[2]) * progress + target[2],
                ];
                // playSound(null, BlockPos.containing(soundLocation), ...)
                let at = centre([
                    sound[0].floor() as i32,
                    sound[1].floor() as i32,
                    sound[2].floor() as i32,
                ]);
                let volume = heart.emitter as f32 / 2.0 / 100.0 + 0.5;
                out.push(HeartEvent::Sound(
                    "block.creaking_heart.hurt",
                    at,
                    volume,
                    1.0,
                ));
            }
        }
        heart.emitter -= 1;
    }
    let ticker = heart.ticker;
    heart.ticker -= 1;
    if ticker >= 0 {
        return;
    }
    heart.ticker = lr.next_int_bound(5).unwrap_or(0) + 20;
    let mut b = Blocks {
        w,
        changed: Vec::new(),
    };
    let state = b.get(pos);
    // updateCreakingState
    let active = creaking_active(w);
    let wanted = if !has_required_logs(&b, state, pos) && heart.creaking.is_none() {
        "uprooted"
    } else if active {
        "awake"
    } else {
        "dormant"
    };
    let mut updated = state;
    if b.prop(state, "creaking_heart_state").as_deref() != Some(wanted)
        && let Some(next) = b.with_prop(state, "creaking_heart_state", wanted)
    {
        updated = next;
        b.set(pos, next, out);
        if wanted == "uprooted" {
            return;
        }
    }
    if heart.creaking.is_none() {
        let awake = b.prop(updated, "creaking_heart_state").as_deref() == Some("awake");
        if awake && host.spawning_monsters() {
            // getNearestPlayer(pos, 32, false): no spectators.
            let (x, y, z) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            let near = w.player.as_ref().is_some_and(|p| {
                !p.spectator
                    && (p.position.x - x).powi(2)
                        + (p.position.y - y).powi(2)
                        + (p.position.z - z).powi(2)
                        < 32.0 * 32.0
            });
            if near && let Some(id) = spawn_protector(pos, w, lr, host, out) {
                let uuid = host.uuid_for(id);
                heart.creaking = Some((Some(id), uuid));
            }
        }
    } else if let Some(id) = protector(heart, host) {
        let persistent = host.mob(id).is_some_and(|m| m.persistent);
        // playerIsStuckInYou reads the creaking's NEAREST_PLAYERS.
        let stuck = host.mob_mut(id).is_some_and(|m| {
            let listed = super::monster_brain::nearest_players(m).contains(&super::Who::Player);
            let eye = w.player.as_ref().filter(|_| listed).map(|p| p.eye);
            creaking::player_is_stuck(m, eye)
        });
        if !active && !persistent || distance_to_creaking(heart, pos, host) > TOO_FAR || stuck {
            remove_protector(heart, host, false, out);
        }
    }
}
