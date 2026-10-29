//! Client-ticked block-entity animation state, as vanilla's block entities
//! keep it: `ChestLidController` (chests, trapped and copper chests, ender
//! chests), `EnchantingTableBlockEntity.bookAnimationTick`,
//! `ShulkerBoxBlockEntity` (open count, animation status, progress),
//! `BellBlockEntity` (ring ticks, shake direction, resonation),
//! `ConduitBlockEntity.clientTick` (tick count, shape check, rotation,
//! hunting, nautilus particles), `SkullBlockEntity.animation` (powered dragon
//! and piglin heads) and `DecoratedPotBlockEntity` wobbles.
//!
//! [`BlockEntityAnims`] holds the state per position and turns it into the
//! renderer's [`BlockAnim`]. Its tick methods take what the level answers
//! (blocks, entities, game time) as closures, so the parity fixture can drive
//! them with scripted inputs (`tools/test_block_entity_anim_parity.py`).
use crate::block_entity_render::BlockAnim;
use crate::fast_hash::FxHashMap;
use horizon_core::{
    geometry::trig::{self, Mth},
    random::{LegacyRandom, RandomSource},
};

/// `Direction` names by `get3DDataValue`.
pub const DIRECTIONS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

fn step(direction: u8) -> [i32; 3] {
    match direction {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}

/// `Direction.from3DDataValue`: `BY_3D_DATA[Mth.abs(data % 6)]`.
pub fn from_3d_data_value(data: i32) -> u8 {
    (data % 6).unsigned_abs() as u8
}

/// The 3D data value of a direction name.
pub fn direction_index(name: &str) -> Option<u8> {
    DIRECTIONS.iter().position(|d| *d == name).map(|i| i as u8)
}

/// `ChestLidController`, shared by `ChestBlockEntity` (chests, trapped and
/// copper chests) and `EnderChestBlockEntity`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChestLid {
    pub should_be_open: bool,
    pub openness: f32,
    pub o_openness: f32,
}

impl ChestLid {
    /// `triggerEvent(1, openCount)`: the lid opens while anyone has it open.
    pub fn trigger_event(&mut self, b0: i32, b1: i32) -> bool {
        if b0 == 1 {
            self.should_be_open = b1 > 0;
            true
        } else {
            false
        }
    }
    /// `tickLid` (the client ticker `lidAnimateTick`).
    pub fn tick(&mut self) {
        self.o_openness = self.openness;
        if !self.should_be_open && self.openness > 0.0 {
            self.openness = (self.openness - 0.1).max(0.0);
        } else if self.should_be_open && self.openness < 1.0 {
            self.openness = (self.openness + 0.1).min(1.0);
        }
    }
    /// `getOpenness(a)`: `Mth.lerp(a, oOpenness, openness)`.
    pub fn get_openness(&self, a: f32) -> f32 {
        self.o_openness + a * (self.openness - self.o_openness)
    }
}

/// `ChestBlock.opennessCombiner`: a double chest opens as far as its more
/// open half (`Math.max(first, second)`, first being the `right` half).
pub fn combined_openness(first: f32, second: Option<f32>) -> f32 {
    match second {
        Some(second) => java_max(first, second),
        None => first,
    }
}

/// `Math.max(float, float)`.
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && a.to_bits() != b.to_bits() {
        0.0
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `ChestRenderer.submit` and `ChestModel.setupAnim`: the lid and lock
/// `xRot` for a render-state openness.
pub fn chest_lid_x_rot(open: f32) -> f32 {
    let open = 1.0 - open;
    let open = 1.0 - open * open * open;
    -(open * std::f32::consts::FRAC_PI_2)
}

/// `EnchantingTableBlockEntity` book fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EnchantingBook {
    pub time: i32,
    pub flip: f32,
    pub o_flip: f32,
    pub flip_t: f32,
    pub flip_a: f32,
    pub open: f32,
    pub o_open: f32,
    pub rot: f32,
    pub o_rot: f32,
    pub t_rot: f32,
}

/// The `while (a >= PI) a -= 2PI; while (a < -PI) a += 2PI` loops.
fn wrap_pi(mut a: f32) -> f32 {
    use std::f32::consts::PI;
    while a >= PI {
        a -= 2.0 * PI;
    }
    while a < -PI {
        a += 2.0 * PI;
    }
    a
}

/// `Mth.floor(float)`.
fn mth_floor(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 { i - 1 } else { i }
}

/// `Mth.frac(float)`.
pub fn mth_frac(v: f32) -> f32 {
    v - mth_floor(v) as f32
}

/// `EnchantTableRenderState` (`EnchantTableRenderer.extractRenderState`) and
/// the `BookModel.State` its `submit` builds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BookRender {
    pub flip: f32,
    pub open: f32,
    pub time: f32,
    pub y_rot: f32,
    /// `BookModel.State.forAnimation`: openness, pageFlip1, pageFlip2.
    pub openness: f32,
    pub page_flip1: f32,
    pub page_flip2: f32,
}

impl EnchantingBook {
    /// `bookAnimationTick`. `players` are the feet positions of the level's
    /// non-spectator players (`getNearestPlayer(centre, 3.0, false)`);
    /// `random` is the block entity class's shared static `RANDOM`.
    pub fn tick(&mut self, pos: [i32; 3], players: &[[f64; 3]], random: &mut impl RandomSource) {
        self.o_open = self.open;
        self.o_rot = self.rot;
        let c = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let mut best = -1.0;
        let mut nearest = None;
        for p in players {
            let (dx, dy, dz) = (p[0] - c[0], p[1] - c[1], p[2] - c[2]);
            let dist = dx * dx + dy * dy + dz * dz;
            if dist < 9.0 && (best == -1.0 || dist < best) {
                best = dist;
                nearest = Some(*p);
            }
        }
        if let Some(p) = nearest {
            let xd = p[0] - c[0];
            let zd = p[2] - c[2];
            self.t_rot = trig::atan2(zd, xd) as f32;
            self.open += 0.1;
            if self.open < 0.5 || random.next_int_bound(40).unwrap_or(1) == 0 {
                let old = self.flip_t;
                loop {
                    let a = random.next_int_bound(4).unwrap_or(0);
                    let b = random.next_int_bound(4).unwrap_or(0);
                    self.flip_t += (a - b) as f32;
                    if old != self.flip_t {
                        break;
                    }
                }
            }
        } else {
            self.t_rot += 0.02;
            self.open -= 0.1;
        }
        self.rot = wrap_pi(self.rot);
        self.t_rot = wrap_pi(self.t_rot);
        let dir = wrap_pi(self.t_rot - self.rot);
        self.rot += dir * 0.4;
        self.open = self.open.clamp(0.0, 1.0);
        self.time += 1;
        self.o_flip = self.flip;
        let diff = ((self.flip_t - self.flip) * 0.4).clamp(-0.2, 0.2);
        self.flip_a += (diff - self.flip_a) * 0.9;
        self.flip += self.flip_a;
    }
    /// `EnchantTableRenderer.extractRenderState` and the book state of its
    /// `submit`.
    pub fn render(&self, a: f32) -> BookRender {
        let flip = self.o_flip + a * (self.flip - self.o_flip);
        let open = self.o_open + a * (self.open - self.o_open);
        let time = self.time as f32 + a;
        let y_rot = self.o_rot + wrap_pi(self.rot - self.o_rot) * a;
        let ff1 = mth_frac(flip + 0.25) * 1.6 - 0.3;
        let ff2 = mth_frac(flip + 0.75) * 1.6 - 0.3;
        BookRender {
            flip,
            open,
            time,
            y_rot,
            openness: ((time * 0.02).msin() * 0.1 + 1.25) * open,
            page_flip1: ff1.clamp(0.0, 1.0),
            page_flip2: ff2.clamp(0.0, 1.0),
        }
    }
}

/// `ShulkerBoxBlockEntity.AnimationStatus`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShulkerStatus {
    #[default]
    Closed,
    Opening,
    Opened,
    Closing,
}

/// `ShulkerBoxBlockEntity` lid state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shulker {
    pub open_count: i32,
    pub status: ShulkerStatus,
    pub progress: f32,
    pub progress_old: f32,
}

/// Side effects of one [`Shulker::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShulkerTick {
    /// `doNeighborUpdates` calls (shape and neighbour updates around the box).
    pub neighbor_updates: u32,
    /// `moveCollidedEntities`: the swept lid box (min x, y, z, max x, y, z);
    /// entities in it move out along the facing.
    pub push: Option<[f64; 6]>,
}

/// `Shulker.getProgressDeltaAabb(size, direction, from, to, position)`.
pub fn progress_delta_aabb(
    size: f32,
    direction: u8,
    from: f32,
    to: f32,
    position: [f64; 3],
) -> [f64; 6] {
    let size_d = f64::from(size);
    let mut b = [
        -size_d * 0.5,
        0.0,
        -size_d * 0.5,
        size_d * 0.5,
        size_d,
        size_d * 0.5,
    ];
    let max = f64::from(from.max(to));
    let min = f64::from(from.min(to));
    let s = step(direction);
    for axis in 0..3 {
        // expandTowards
        let a = f64::from(s[axis]) * max * size_d;
        if a < 0.0 {
            b[axis] += a;
        } else if a > 0.0 {
            b[axis + 3] += a;
        }
    }
    for axis in 0..3 {
        // contract
        let a = -f64::from(s[axis]) * (1.0 + min) * size_d;
        if a < 0.0 {
            b[axis] -= a;
        } else if a > 0.0 {
            b[axis + 3] -= a;
        }
    }
    // new AABB keeps min <= max; then move to the position.
    let mut out = [0.0; 6];
    for axis in 0..3 {
        out[axis] = b[axis].min(b[axis + 3]) + position[axis];
        out[axis + 3] = b[axis].max(b[axis + 3]) + position[axis];
    }
    out
}

impl Shulker {
    /// `triggerEvent(1, openCount)`.
    pub fn trigger_event(&mut self, b0: i32, b1: i32) -> bool {
        if b0 != 1 {
            return false;
        }
        self.open_count = b1;
        if b1 == 0 {
            self.status = ShulkerStatus::Closing;
        }
        if b1 == 1 {
            self.status = ShulkerStatus::Opening;
        }
        true
    }
    /// `startOpen`/`stopOpen` for a non-spectator user: the new open count to
    /// send as block event 1, and whether this was the first opener (open) or
    /// the last closer (close), which play the sound and game event.
    pub fn start_open(&mut self) -> (i32, bool) {
        if self.open_count < 0 {
            self.open_count = 0;
        }
        self.open_count += 1;
        (self.open_count, self.open_count == 1)
    }
    pub fn stop_open(&mut self) -> (i32, bool) {
        self.open_count -= 1;
        (self.open_count, self.open_count <= 0)
    }
    /// `updateAnimation` (`ShulkerBoxBlockEntity.tick`); `facing` is the
    /// block's facing as a 3D data value.
    pub fn tick(&mut self, pos: [i32; 3], facing: u8) -> ShulkerTick {
        let mut out = ShulkerTick::default();
        self.progress_old = self.progress;
        match self.status {
            ShulkerStatus::Closed => self.progress = 0.0,
            ShulkerStatus::Opening => {
                self.progress += 0.1;
                if self.progress_old == 0.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress >= 1.0 {
                    self.status = ShulkerStatus::Opened;
                    self.progress = 1.0;
                    out.neighbor_updates += 1;
                }
                let bottom = [
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]),
                    f64::from(pos[2]) + 0.5,
                ];
                out.push = Some(progress_delta_aabb(
                    1.0,
                    facing,
                    self.progress_old,
                    self.progress,
                    bottom,
                ));
            }
            ShulkerStatus::Opened => self.progress = 1.0,
            ShulkerStatus::Closing => {
                self.progress -= 0.1;
                if self.progress_old == 1.0 {
                    out.neighbor_updates += 1;
                }
                if self.progress <= 0.0 {
                    self.status = ShulkerStatus::Closed;
                    self.progress = 0.0;
                    out.neighbor_updates += 1;
                }
            }
        }
        out
    }
    /// `ShulkerBoxBlock.canOpen`: a closed box only opens when its lid has room
    /// (`level.noCollision` of the half-block lid box, deflated by 1e-6).
    pub fn lid_box(pos: [i32; 3], facing: u8) -> [f64; 6] {
        let bottom = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let mut b = progress_delta_aabb(1.0, facing, 0.0, 0.5, bottom);
        for axis in 0..3 {
            b[axis] += 1.0e-6;
            b[axis + 3] -= 1.0e-6;
        }
        b
    }
}

/// What the bell's tick asks about an entity it gathered when rung.
#[derive(Clone, Copy, Debug)]
pub struct BellEntity {
    pub position: [f64; 3],
    /// `isAlive() && !isRemoved()`.
    pub alive: bool,
    /// In `EntityTypeTags.RAIDERS`.
    pub raider: bool,
}

/// `BellBlockEntity`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bell {
    pub ticks: i32,
    pub shaking: bool,
    /// `clickDirection` as a 3D data value.
    pub click_direction: Option<u8>,
    pub last_ring_timestamp: i64,
    /// `nearbyEntities`: the living entities around the bell when it was last
    /// gathered, as caller-defined handles.
    pub nearby: Option<Vec<u64>>,
    pub resonating: bool,
    pub resonation_ticks: i32,
}

/// Side effects of one [`Bell::tick`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BellTick {
    /// Resonation started: `BELL_RESONATE` (played by the server).
    pub resonate: bool,
    /// Resonation ended (`ResonationEndAction`): the client's
    /// `showBellParticles` entity-effect particles (position and colour) and
    /// the server's `makeRaidersGlow` targets.
    pub ended: Option<(Vec<([f64; 3], i32)>, Vec<u64>)>,
}

/// `Vec3i.closerToCenterThan`: `p` is within `distance` of the block's centre.
pub fn closer_to_center_than(pos: [i32; 3], p: [f64; 3], distance: f64) -> bool {
    let dx = f64::from(pos[0]) + 0.5 - p[0];
    let dy = f64::from(pos[1]) + 0.5 - p[1];
    let dz = f64::from(pos[2]) + 0.5 - p[2];
    dx * dx + dy * dy + dz * dz < distance * distance
}

impl Bell {
    /// `triggerEvent(1, direction)`. `gather` lists the living entities whose
    /// position is inside the bell's block box inflated by 48
    /// (`updateEntities`, refreshed at most every 60 ticks).
    pub fn trigger_event(
        &mut self,
        b0: i32,
        b1: i32,
        game_time: i64,
        gather: impl FnOnce() -> Vec<u64>,
    ) -> bool {
        if b0 != 1 {
            return false;
        }
        if game_time > self.last_ring_timestamp + 60 || self.nearby.is_none() {
            self.last_ring_timestamp = game_time;
            self.nearby = Some(gather());
        }
        self.resonation_ticks = 0;
        self.click_direction = Some(from_3d_data_value(b1));
        self.ticks = 0;
        self.shaking = true;
        true
    }
    /// `BellBlockEntity.tick` with the client's resonation end action.
    pub fn tick(&mut self, pos: [i32; 3], entity: impl Fn(u64) -> Option<BellEntity>) -> BellTick {
        let mut out = BellTick::default();
        if self.shaking {
            self.ticks += 1;
        }
        if self.ticks >= 50 {
            self.shaking = false;
            self.ticks = 0;
        }
        let nearby = self.nearby.clone().unwrap_or_default();
        let raider_in = |e: &BellEntity, distance: f64| {
            e.alive && closer_to_center_than(pos, e.position, distance) && e.raider
        };
        if self.ticks >= 5
            && self.resonation_ticks == 0
            && nearby
                .iter()
                .filter_map(|&h| entity(h))
                .any(|e| raider_in(&e, 32.0))
        {
            self.resonating = true;
            out.resonate = true;
        }
        if self.resonating {
            if self.resonation_ticks < 40 {
                self.resonation_ticks += 1;
            } else {
                // showBellParticles / makeRaidersGlow.
                let mut color = 16_700_985;
                let views: Vec<(u64, BellEntity)> = nearby
                    .iter()
                    .filter_map(|&h| Some((h, entity(h)?)))
                    .collect();
                let count = views
                    .iter()
                    .filter(|(_, e)| closer_to_center_than(pos, e.position, 48.0))
                    .count() as i32;
                let per = ((count - 21) / -2).clamp(3, 15);
                let mut particles = Vec::new();
                let mut glow = Vec::new();
                for (h, e) in &views {
                    if !raider_in(e, 48.0) {
                        continue;
                    }
                    glow.push(*h);
                    let (bx, bz) = (f64::from(pos[0]), f64::from(pos[2]));
                    let dist = ((e.position[0] - bx) * (e.position[0] - bx)
                        + (e.position[2] - bz) * (e.position[2] - bz))
                        .sqrt();
                    let x3 = bx + f64::from(0.5f32) + 1.0 / dist * (e.position[0] - bx);
                    let z3 = bz + f64::from(0.5f32) + 1.0 / dist * (e.position[2] - bz);
                    for _ in 0..per {
                        color += 5;
                        particles.push(([x3, f64::from(pos[1]) + 0.5, z3], color));
                    }
                }
                out.ended = Some((particles, glow));
                self.resonating = false;
            }
        }
        out
    }
}

/// The conduit's `destroyTarget` as the client sees it.
#[derive(Clone, Copy, Debug)]
pub struct ConduitTarget {
    pub position: [f64; 3],
    pub eye_y: f64,
    pub width: f32,
    pub height: f32,
}

/// `ConduitBlockEntity` (client side).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conduit {
    pub tick_count: i32,
    pub active_rotation: f32,
    pub is_active: bool,
    pub is_hunting: bool,
    pub effect_blocks: Vec<[i32; 3]>,
}

/// One nautilus particle: position and velocity.
pub type Particle = ([f64; 3], [f64; 3]);

impl Conduit {
    /// `updateShape`: water all around, then the frame blocks
    /// (`BlockTags.CONDUIT_EFFECT_BLOCK`) in the three 5x5 rings.
    fn update_shape(
        &mut self,
        pos: [i32; 3],
        is_water: &impl Fn([i32; 3]) -> bool,
        is_frame: &impl Fn([i32; 3]) -> bool,
    ) -> bool {
        self.effect_blocks.clear();
        for ox in -1..=1 {
            for oy in -1..=1 {
                for oz in -1..=1 {
                    if !is_water([pos[0] + ox, pos[1] + oy, pos[2] + oz]) {
                        return false;
                    }
                }
            }
        }
        for ox in -2i32..=2 {
            for oy in -2i32..=2 {
                for oz in -2i32..=2 {
                    let (ax, ay, az) = (ox.abs(), oy.abs(), oz.abs());
                    if (ax > 1 || ay > 1 || az > 1)
                        && (ox == 0 && (ay == 2 || az == 2)
                            || oy == 0 && (ax == 2 || az == 2)
                            || oz == 0 && (ax == 2 || ay == 2))
                    {
                        let p = [pos[0] + ox, pos[1] + oy, pos[2] + oz];
                        if is_frame(p) {
                            self.effect_blocks.push(p);
                        }
                    }
                }
            }
        }
        self.effect_blocks.len() >= 16
    }
    /// `ConduitBlockEntity.clientTick`: returns the nautilus particles of
    /// `animationTick`, drawn from the client level's random.
    pub fn client_tick(
        &mut self,
        pos: [i32; 3],
        game_time: i64,
        is_water: impl Fn([i32; 3]) -> bool,
        is_frame: impl Fn([i32; 3]) -> bool,
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        self.tick_count += 1;
        if game_time % 40 == 0 {
            self.is_active = self.update_shape(pos, &is_water, &is_frame);
            self.is_hunting = self.effect_blocks.len() >= 42;
        }
        let particles = self.animation_tick(pos, target, random);
        if self.is_active {
            self.active_rotation += 1.0;
        }
        particles
    }
    fn animation_tick(
        &self,
        pos: [i32; 3],
        target: Option<ConduitTarget>,
        random: &mut impl RandomSource,
    ) -> Vec<Particle> {
        let mut out = Vec::new();
        let hh = f64::from(((self.tick_count + 35) as f32 * 0.1).msin() / 2.0 + 0.5);
        let hh = (hh * hh + hh) * f64::from(0.3f32);
        let end = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.5 + hh,
            f64::from(pos[2]) + 0.5,
        ];
        for p in &self.effect_blocks {
            if random.next_int_bound(50).unwrap_or(1) == 0 {
                let dx = -0.5 + random.next_float() + (p[0] - pos[0]) as f32;
                let dy = -2.0 + random.next_float() + (p[1] - pos[1]) as f32;
                let dz = -0.5 + random.next_float() + (p[2] - pos[2]) as f32;
                out.push((end, [f64::from(dx), f64::from(dy), f64::from(dz)]));
            }
        }
        if let Some(t) = target {
            let randx = (-0.5 + random.next_float()) * (3.0 + t.width);
            let randy = -1.0 + random.next_float() * t.height;
            let randz = (-0.5 + random.next_float()) * (3.0 + t.width);
            out.push((
                [t.position[0], t.eye_y, t.position[2]],
                [f64::from(randx), f64::from(randy), f64::from(randz)],
            ));
        }
        out
    }
}

/// What `ConduitBlockEntity.serverTick`'s 40-tick check decided.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ConduitCheck {
    /// The shape changed: `CONDUIT_ACTIVATE` (true) or `CONDUIT_DEACTIVATE`.
    pub toggled: Option<bool>,
    /// The frame blocks counted, while the conduit is active: `applyEffects`
    /// and `updateAndAttackTarget` follow (the attack needs 42 or more).
    pub frames: Option<usize>,
}

/// `ConduitBlockEntity` (server side): what `serverTick` keeps between ticks.
#[derive(Clone, Debug, Default)]
pub struct ConduitServer {
    shape: Conduit,
    /// `destroyTarget`: the mob id of the enemy being struck.
    pub target: Option<u32>,
    next_ambient_sound: i64,
}

impl ConduitServer {
    /// The `gameTime % 40` part of `serverTick`: the shape is checked again.
    pub fn check(
        &mut self,
        pos: [i32; 3],
        game_time: i64,
        is_water: impl Fn([i32; 3]) -> bool,
        is_frame: impl Fn([i32; 3]) -> bool,
    ) -> Option<ConduitCheck> {
        if game_time % 40 != 0 {
            return None;
        }
        let active = self.shape.update_shape(pos, &is_water, &is_frame);
        let toggled = (active != self.shape.is_active).then_some(active);
        self.shape.is_active = active;
        Some(ConduitCheck {
            toggled,
            frames: active.then_some(self.shape.effect_blocks.len()),
        })
    }
    /// The sounds of an active conduit at the end of `serverTick`: whether
    /// `CONDUIT_AMBIENT` (every 80 ticks) and `CONDUIT_AMBIENT_SHORT` (every
    /// 60 to 99 ticks) play.
    pub fn ambient(&mut self, game_time: i64, random: &mut impl RandomSource) -> (bool, bool) {
        if !self.shape.is_active {
            return (false, false);
        }
        let short = game_time > self.next_ambient_sound;
        if short {
            self.next_ambient_sound =
                game_time + 60 + i64::from(random.next_int_bound(40).unwrap_or(0));
        }
        (game_time % 80 == 0, short)
    }
}

/// `SkullBlockEntity` animation (client ticker of dragon and piglin heads).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Skull {
    pub animating: bool,
    pub tick_count: i32,
}
impl Skull {
    /// `SkullBlockEntity.animation`.
    pub fn tick(&mut self, powered: bool) {
        if powered {
            self.animating = true;
            self.tick_count += 1;
        } else {
            self.animating = false;
        }
    }
}

/// `DecoratedPotBlockEntity.WobbleStyle`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WobbleStyle {
    Positive,
    Negative,
}
impl WobbleStyle {
    pub fn duration(self) -> i32 {
        match self {
            Self::Positive => 7,
            Self::Negative => 10,
        }
    }
}

/// `DecoratedPotBlockEntity` wobble fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PotWobble {
    pub started_at: i64,
    pub style: Option<WobbleStyle>,
}
impl PotWobble {
    /// `triggerEvent(1, style)`.
    pub fn trigger_event(&mut self, event: i32, data: i32, game_time: i64) -> bool {
        if event == 1 && (0..2).contains(&data) {
            self.started_at = game_time;
            self.style = Some(if data == 0 {
                WobbleStyle::Positive
            } else {
                WobbleStyle::Negative
            });
            true
        } else {
            false
        }
    }
    /// `DecoratedPotRenderer.extractRenderState`'s `wobbleProgress`.
    pub fn progress(&self, game_time: i64, partial: f32) -> f32 {
        match self.style {
            Some(style) => {
                ((game_time - self.started_at) as f32 + partial) / style.duration() as f32
            }
            None => 0.0,
        }
    }
}

/// `TheEndGatewayBlockEntity` fields the beam reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EndGateway {
    pub age: i64,
    pub teleport_cooldown: i32,
}
impl EndGateway {
    /// `beamAnimationTick` (the client ticker).
    pub fn tick(&mut self) {
        self.age = self.age.wrapping_add(1);
        if self.teleport_cooldown > 0 {
            self.teleport_cooldown -= 1;
        }
    }
    /// `portalTick` on the server copy: whether it called `triggerCooldown`,
    /// whose block event 1 reaches the client copy.
    pub fn portal_tick(&mut self) -> bool {
        let cooling = self.teleport_cooldown > 0;
        self.age = self.age.wrapping_add(1);
        if cooling {
            self.teleport_cooldown -= 1;
            false
        } else if self.age % 2400 == 0 {
            self.teleport_cooldown = 40;
            true
        } else {
            false
        }
    }
    /// `EndGatewayBlock.entityInside` on the server copy: an entity that can
    /// use the portal starts the cooldown unless it is already running.
    pub fn enter(&mut self) -> bool {
        if self.teleport_cooldown > 0 {
            return false;
        }
        self.teleport_cooldown = 40;
        true
    }
    /// `triggerEvent`: event 1 starts the 40 tick cooldown.
    pub fn trigger_event(&mut self, event: i32, _data: i32) -> bool {
        if event == 1 {
            self.teleport_cooldown = 40;
            true
        } else {
            false
        }
    }
}

/// The client ticker a block entity runs (`getTicker` on a client level),
/// or [`Ticker::Pot`] for decorated pots, which keep wobble state unticked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ticker {
    /// `ChestBlockEntity` / `EnderChestBlockEntity.lidAnimateTick`.
    Lid,
    /// `EnchantingTableBlockEntity.bookAnimationTick`.
    Book,
    /// `ShulkerBoxBlockEntity.tick`, with the box's facing.
    Shulker {
        facing: u8,
    },
    /// `BellBlockEntity.clientTick`.
    Bell,
    /// `ConduitBlockEntity.clientTick`.
    Conduit,
    /// `SkullBlockEntity.animation` (dragon and piglin heads only).
    Skull {
        powered: bool,
    },
    Pot,
    /// `TheEndGatewayBlockEntity.beamAnimationTick`.
    Gateway,
}

/// The [`Ticker`] of a block state, by block name and property lookup.
pub fn ticker(name: &str, prop: impl Fn(&str) -> Option<String>) -> Option<Ticker> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let unwaxed = name.strip_prefix("waxed_").unwrap_or(name);
    Some(match unwaxed {
        "chest"
        | "trapped_chest"
        | "ender_chest"
        | "copper_chest"
        | "exposed_copper_chest"
        | "weathered_copper_chest"
        | "oxidized_copper_chest" => Ticker::Lid,
        "enchanting_table" => Ticker::Book,
        "bell" => Ticker::Bell,
        "conduit" => Ticker::Conduit,
        "decorated_pot" => Ticker::Pot,
        "end_gateway" => Ticker::Gateway,
        "dragon_head" | "dragon_wall_head" | "piglin_head" | "piglin_wall_head" => Ticker::Skull {
            powered: prop("powered").as_deref() == Some("true"),
        },
        n if n.ends_with("shulker_box") => Ticker::Shulker {
            facing: prop("facing")
                .as_deref()
                .and_then(direction_index)
                .unwrap_or(1),
        },
        _ => return None,
    })
}

/// The animated block entities of a level, by position.
#[derive(Clone, Debug)]
pub struct BlockEntityAnims {
    pub chests: FxHashMap<[i32; 3], ChestLid>,
    pub books: FxHashMap<[i32; 3], EnchantingBook>,
    /// `EnchantingTableBlockEntity.RANDOM`, shared by every table.
    pub book_random: LegacyRandom,
    pub shulkers: FxHashMap<[i32; 3], Shulker>,
    pub bells: FxHashMap<[i32; 3], Bell>,
    pub conduits: FxHashMap<[i32; 3], Conduit>,
    pub skulls: FxHashMap<[i32; 3], Skull>,
    pub pots: FxHashMap<[i32; 3], PotWobble>,
    pub gateways: FxHashMap<[i32; 3], EndGateway>,
    /// The integrated server's copies of the gateways (`portalTick`).
    pub gateway_servers: FxHashMap<[i32; 3], EndGateway>,
    /// The integrated server's copies of the conduits (`serverTick`).
    pub conduit_servers: FxHashMap<[i32; 3], ConduitServer>,
    /// The client level's random (conduit particles).
    pub random: LegacyRandom,
    /// [`Ticker`] by block state id (see [`BlockEntityAnims::classify`]).
    pub kinds: Vec<Option<Ticker>>,
    /// The drawn block entities with a ticker, sorted by position, and the
    /// renderer's block entity revision they were read at.
    pub present: Vec<([i32; 3], Ticker)>,
    pub present_revision: Option<u64>,
}
impl Default for BlockEntityAnims {
    fn default() -> Self {
        Self {
            chests: FxHashMap::default(),
            books: FxHashMap::default(),
            book_random: LegacyRandom::new(0x626f6f6b),
            shulkers: FxHashMap::default(),
            bells: FxHashMap::default(),
            conduits: FxHashMap::default(),
            skulls: FxHashMap::default(),
            pots: FxHashMap::default(),
            gateways: FxHashMap::default(),
            gateway_servers: FxHashMap::default(),
            conduit_servers: FxHashMap::default(),
            random: LegacyRandom::new(0x6c6576656c),
            kinds: Vec::new(),
            present: Vec::new(),
            present_revision: None,
        }
    }
}
impl BlockEntityAnims {
    /// Fills [`Self::kinds`] for every state of the registry.
    pub fn classify(&mut self, registry: &horizon_world::registry::BlockRegistry) {
        self.kinds = (0..registry.len() as u32)
            .map(|id| {
                registry
                    .state(horizon_world::registry::BlockStateId(id))
                    .and_then(|s| ticker(&s.name, |k| s.properties.get(k).cloned()))
            })
            .collect();
        self.present_revision = None;
    }
    /// The ticker of a block state.
    pub fn ticker(&self, state: u32) -> Option<Ticker> {
        self.kinds.get(state as usize).copied().flatten()
    }
    /// Drops the state of block entities that are gone or changed kind:
    /// `present` holds the loaded block entities' tickers, sorted by position.
    pub fn retain_present(&mut self, present: &[([i32; 3], Ticker)]) {
        let is = |p: &[i32; 3], f: fn(&Ticker) -> bool| {
            present
                .binary_search_by_key(p, |(q, _)| *q)
                .is_ok_and(|i| f(&present[i].1))
        };
        self.chests.retain(|p, _| is(p, |t| *t == Ticker::Lid));
        self.books.retain(|p, _| is(p, |t| *t == Ticker::Book));
        self.shulkers
            .retain(|p, _| is(p, |t| matches!(t, Ticker::Shulker { .. })));
        self.bells.retain(|p, _| is(p, |t| *t == Ticker::Bell));
        self.conduits
            .retain(|p, _| is(p, |t| *t == Ticker::Conduit));
        self.skulls
            .retain(|p, _| is(p, |t| matches!(t, Ticker::Skull { .. })));
        self.pots.retain(|p, _| is(p, |t| *t == Ticker::Pot));
        self.gateways
            .retain(|p, _| is(p, |t| *t == Ticker::Gateway));
    }
    /// Drops the state of positions whose block entity is gone.
    pub fn retain(&mut self, keep: impl Fn([i32; 3]) -> bool) {
        self.chests.retain(|p, _| keep(*p));
        self.books.retain(|p, _| keep(*p));
        self.shulkers.retain(|p, _| keep(*p));
        self.bells.retain(|p, _| keep(*p));
        self.conduits.retain(|p, _| keep(*p));
        self.skulls.retain(|p, _| keep(*p));
        self.pots.retain(|p, _| keep(*p));
        self.gateways.retain(|p, _| keep(*p));
        self.gateway_servers.retain(|p, _| keep(*p));
        self.conduit_servers.retain(|p, _| keep(*p));
    }
    /// `getOpenNess(partial)` of the chest or ender chest at `pos`.
    pub fn lid(&self, pos: [i32; 3], partial: f32) -> f32 {
        self.chests
            .get(&pos)
            .map_or(0.0, |l| l.get_openness(partial))
    }
    /// The renderer's view of the block entity at `pos`; `sherds` are the
    /// pot's decorations (back, left, right, front), when it is a pot.
    pub fn block_anim(
        &self,
        pos: [i32; 3],
        game_time: i64,
        partial: f32,
        sherds: [Option<String>; 4],
    ) -> BlockAnim {
        let mut anim = BlockAnim {
            sherds,
            // Gateways not ticked yet render settled (no beam).
            gateway: self
                .gateways
                .get(&pos)
                .map_or((i64::MAX, 0), |g| (g.age, g.teleport_cooldown)),
            ..BlockAnim::default()
        };
        if let Some(b) = self.books.get(&pos) {
            anim.book = *b;
        }
        if let Some(s) = self.shulkers.get(&pos) {
            anim.shulker = (s.progress, s.progress_old);
        }
        if let Some(b) = self.bells.get(&pos) {
            anim.bell_ticks = b.ticks;
            anim.bell_shake = b
                .shaking
                .then_some(b.click_direction)
                .flatten()
                .map(|d| DIRECTIONS[usize::from(d)]);
        }
        if let Some(c) = self.conduits.get(&pos) {
            anim.conduit = (c.tick_count, c.is_active, c.is_hunting, c.active_rotation);
        }
        if let Some(s) = self.skulls.get(&pos) {
            anim.skull = (s.animating, s.tick_count);
        }
        if let Some(w) = self.pots.get(&pos)
            && w.style.is_some()
        {
            // extractRenderState never copies lastWobbleStyle into the render
            // state, so DecoratedPotRenderer.submit always takes the
            // non-POSITIVE branch (with the style's own duration).
            anim.wobble = Some((false, w.progress(game_time, partial)));
        }
        anim
    }
}
