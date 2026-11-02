//! The `animations` of a custom entity model: compiled with the model when the packs load,
//! run per entity per frame after the vanilla animation has posed the parts, so `part.rx`
//! reads the vanilla pose and `head.rx = ...` overrides it.
//!
//! One slot array holds everything an expression reads: the entity's parameters
//! (`limb_swing`, `is_child`...) at fixed slots, then, in the order the expressions first name
//! them, 11 slots per model part (`tx ty tz rx ry rz sx sy sz visible visible_boxes`, as
//! `ModelPart` keeps them: pixels, radians, on vanilla's axes) and one per variable: the
//! entity's `var.*` / `varb.*`, which persist from frame to frame, the `global_var.*` /
//! `global_varb.*` every entity shares, and `render.*`, which nothing reads back yet.

use super::{Mob, Part, anim::Poses, anim::State};
use horizon_optifine::cem::expr::{Expr, compile};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// The model variables of a part, in slot order.
const PART_VARS: [&str; 11] = [
    "tx",
    "ty",
    "tz",
    "rx",
    "ry",
    "rz",
    "sx",
    "sy",
    "sz",
    "visible",
    "visible_boxes",
];

macro_rules! params {
    ($($name:literal => $p:ident),* $(,)?) => {
        /// The entity and render parameters, by slot.
        #[derive(Clone, Copy)]
        #[repr(u8)]
        enum Param { $($p),* }
        const PARAMS: &[(&str, Param)] = &[$(($name, Param::$p)),*];
    };
}
params! {
    "limb_swing" => LimbSwing, "limb_speed" => LimbSpeed, "age" => Age,
    "head_yaw" => HeadYaw, "head_pitch" => HeadPitch, "swing_progress" => SwingProgress,
    "health" => Health, "max_health" => MaxHealth, "hurt_time" => HurtTime,
    "death_time" => DeathTime, "pos_x" => PosX, "pos_y" => PosY, "pos_z" => PosZ,
    "rot_x" => RotX, "rot_y" => RotY, "id" => Id, "time" => Time, "day_time" => DayTime,
    "day_count" => DayCount, "frame_time" => FrameTime, "frame_counter" => FrameCounter,
    "dimension" => Dimension, "player_pos_x" => PlayerX, "player_pos_y" => PlayerY,
    "player_pos_z" => PlayerZ, "player_rot_x" => PlayerRotX, "player_rot_y" => PlayerRotY,
    "rule_index" => RuleIndex, "anger_time" => AngerTime, "anger_time_start" => AngerTimeStart,
    "distance" => Distance, "move_forward" => MoveForward, "move_strafing" => MoveStrafing,
    "height_above_ground" => HeightAboveGround, "fluid_depth" => FluidDepth,
    "fluid_depth_down" => FluidDepthDown, "fluid_depth_up" => FluidDepthUp,
    "is_aggressive" => Aggressive, "is_alive" => Alive, "is_burning" => Burning,
    "is_child" => Child, "is_glowing" => Glowing, "is_hurt" => Hurt, "is_in_hand" => InHand,
    "is_in_item_frame" => InItemFrame, "is_in_ground" => InGround, "is_in_gui" => InGui,
    "is_in_lava" => InLava, "is_in_water" => InWater, "is_invisible" => Invisible,
    "is_on_ground" => OnGround, "is_on_head" => OnHead, "is_on_shoulder" => OnShoulder,
    "is_ridden" => Ridden, "is_riding" => Riding, "is_sitting" => Sitting,
    "is_sneaking" => Sneaking, "is_sprinting" => Sprinting, "is_tamed" => Tamed,
    "is_wet" => Wet, "is_climbing" => Climbing, "is_blocking" => Blocking,
    "is_crawling" => Crawling, "is_jumping" => Jumping, "is_swimming" => Swimming,
    "is_right_handed" => RightHanded, "is_swinging_right_arm" => SwingingRight,
    "is_swinging_left_arm" => SwingingLeft, "is_first_person_hand" => FirstPersonHand,
    "is_using_item" => UsingItem, "is_holding_item_right" => HoldingRight,
    "is_holding_item_left" => HoldingLeft, "is_paused" => Paused, "is_hovered" => Hovered,
    "is_player_first_person" => FirstPerson, "is_player_third_person" => ThirdPerson,
    "is_player_third_person_reversed" => ThirdPersonReversed,
}

/// What every entity of a frame shares, set once per frame by the game.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The world's game time and day time in ticks, with the partial tick.
    pub time: f64,
    pub day_time: f64,
    pub frame_time: f32,
    pub frame_counter: u32,
    /// Overworld 0, nether -1, end 1.
    pub dimension: f32,
    pub player: [f64; 3],
    pub player_rot: [f32; 2],
    pub paused: bool,
    pub first_person: bool,
    pub third_person_reversed: bool,
}

impl Frame {
    /// Before the first frame: in the overworld at the origin, seen in first person.
    pub const DEFAULT: Self = Self {
        time: 0.0,
        day_time: 0.0,
        frame_time: 0.0,
        frame_counter: 0,
        dimension: 0.0,
        player: [0.0; 3],
        player_rot: [0.0; 2],
        paused: false,
        first_person: true,
        third_person_reversed: false,
    };
}

impl Default for Frame {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static FRAME: Mutex<Frame> = Mutex::new(Frame::DEFAULT);

/// `m`'s value, even if a thread panicked holding it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The frame the next animations see. Its `frame_time` and `frame_counter` come from the
/// calls themselves: the time since the previous one, and how many there were.
pub fn set_frame(mut frame: Frame) {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let now = std::time::Instant::now();
    let mut last = lock(&LAST);
    frame.frame_time = last.map_or(0.0, |t| (now - t).as_secs_f32());
    *last = Some(now);
    let mut current = lock(&FRAME);
    frame.frame_counter = current.frame_counter.wrapping_add(1);
    *current = frame;
}

fn frame() -> Frame {
    *lock(&FRAME)
}

thread_local!(static IN_GUI: Cell<bool> = const { Cell::new(false) });

/// Runs `draw` as a screen's preview of an entity (the inventory's player, a mount's box):
/// its animations see `is_in_gui = 1` and store no variables, unless the world didn't draw
/// the entity this frame or the last. Screens draw before the world in a frame, so a
/// preview runs under the previous frame's counter and time. `set_frame` must not be
/// called inside it.
pub fn in_gui<R>(draw: impl FnOnce() -> R) -> R {
    /// Puts the previous value back, even if `draw` panics.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_GUI.set(self.0);
        }
    }
    let _restore = Restore(IN_GUI.replace(true));
    draw()
}

/// A part an animation names: its `Poses` name and its first slot.
struct Bound {
    name: String,
    base: u32,
}

/// One compiled `target = expression`.
struct Step {
    slot: u32,
    expr: Expr,
}

/// The compiled `animations` of one model layer.
pub struct Animation {
    steps: Vec<Step>,
    parts: Vec<Bound>,
    /// The `var.*` and `varb.*` slots, kept per entity.
    vars: Vec<u32>,
    /// The `global_var.*` and `global_varb.*` slots and their index in `GLOBALS`.
    globals: Vec<(u32, u32)>,
    slots: usize,
    saved: Mutex<Saved>,
}

/// Each entity's variables, by entity id.
#[derive(Default)]
struct Saved {
    entities: HashMap<u32, Vars>,
    /// The frame of the last eviction, so that a full table is scanned once a frame at most.
    evicted: Option<u32>,
}

/// One entity's variables.
struct Vars {
    /// The frame counter of the last run.
    frame: u32,
    /// A run stored `now` in that frame.
    stored: bool,
    /// The last frame a draw in the world stored: a preview stores only when the world
    /// didn't draw the entity this frame or the last (the inventory's own player in first
    /// person, whom the world never draws).
    world: Option<u32>,
    /// The variables when that frame began: a model drawn again in the same frame (the
    /// illusioner's copies, a preview, the glowing pass) starts from them, not from the
    /// first draw's result.
    start: Box<[f32]>,
    /// The variables the frame's storing draw left.
    now: Box<[f32]>,
    /// The shared variables (`global_var.*`) when that frame began for this entity, read by
    /// every run of the frame like `start`.
    globals_start: Box<[f32]>,
}

/// The variables every entity shares (`global_var.*`): their names, then their values.
struct Globals {
    names: Vec<String>,
    values: Vec<f32>,
}

static GLOBALS: Mutex<Globals> = Mutex::new(Globals {
    names: Vec::new(),
    values: Vec::new(),
});

/// The index of the global variable `name`, registered on first sight.
fn global(name: &str) -> u32 {
    let mut g = lock(&GLOBALS);
    let i = match g.names.iter().position(|n| n == name) {
        Some(i) => i,
        None => {
            g.names.push(name.to_owned());
            g.values.push(0.0);
            g.names.len() - 1
        }
    };
    i as u32
}

/// How an animation finds the parts it names.
pub(super) struct Names<'a> {
    pub model: &'a Part,
    /// The OptiFine part name to the vanilla part's name.
    pub vanilla: &'a dyn Fn(&str) -> String,
    /// Custom bones' ids by their part names (`cem#3`).
    pub ids: &'a HashMap<String, String>,
}

impl Names<'_> {
    /// The part a model reference names: a part name or bone id, then bone ids below it.
    fn find(&self, reference: &str, this: &str, part: &str) -> Option<String> {
        let mut path = reference.split(':');
        let first = path.next()?;
        let mut at = match first {
            "this" => find(self.model, &|p| p.name == this)?,
            "part" => find(self.model, &|p| p.name == part)?,
            _ => {
                let vanilla = (self.vanilla)(first);
                find(self.model, &|p| p.name == vanilla).or_else(|| {
                    find(self.model, &|p| {
                        self.ids.get(&p.name).is_some_and(|i| i == first)
                    })
                })?
            }
        };
        for id in path {
            at = at
                .children
                .iter()
                .find_map(|c| find(c, &|p| self.ids.get(&p.name).is_some_and(|i| i == id)))?;
        }
        Some(at.name.clone())
    }
}

/// The first part of the tree, depth first, that passes `test`.
fn find<'a>(part: &'a Part, test: &dyn Fn(&Part) -> bool) -> Option<&'a Part> {
    if test(part) {
        return Some(part);
    }
    part.children.iter().find_map(|c| find(c, test))
}

/// Hands out slots as the expressions name things: the parameters are fixed at
/// `0..PARAMS.len()`, then each new part takes the next 11, each variable the next one.
struct Layout<'a, 'n> {
    names: &'a Names<'n>,
    next: u32,
    parts: Vec<Bound>,
    vars: Vec<(String, u32)>,
    globals: Vec<(String, u32)>,
    render: Vec<(String, u32)>,
}

/// The slot of `name` in `list`, the next free one on first sight.
fn named(list: &mut Vec<(String, u32)>, next: &mut u32, name: &str) -> u32 {
    if let Some((_, slot)) = list.iter().find(|(n, _)| n == name) {
        return *slot;
    }
    let slot = *next;
    *next += 1;
    list.push((name.to_owned(), slot));
    slot
}

impl Layout<'_, '_> {
    fn slot(&mut self, name: &str, this: &str, part: &str) -> Option<u32> {
        if let Some(i) = PARAMS.iter().position(|(n, _)| *n == name) {
            return Some(i as u32);
        }
        let (model, var) = name.rsplit_once('.')?;
        Some(match model {
            "var" | "varb" => named(&mut self.vars, &mut self.next, name),
            "global_var" | "global_varb" => named(&mut self.globals, &mut self.next, name),
            "render" => named(&mut self.render, &mut self.next, name),
            _ => {
                let var = PART_VARS.iter().position(|v| *v == var)? as u32;
                let found = self.names.find(model, this, part)?;
                let known = self.parts.iter().find(|b| b.name == found).map(|b| b.base);
                let base = match known {
                    Some(base) => base,
                    None => {
                        let base = self.next;
                        self.next += PART_VARS.len() as u32;
                        self.parts.push(Bound { name: found, base });
                        base
                    }
                };
                base + var
            }
        })
    }
}

/// The most expressions one model's animations may have; the rest are left out.
const MAX_EXPRESSIONS: usize = 4096;

/// Builds an animation from `entries`: each model entry's `this` and `part` names and its
/// (target, expression) pairs. An expression that doesn't compile is left out, with a warning.
pub(super) fn compile_animation(
    file: &str,
    names: &Names,
    entries: &[(String, String, &[(String, String)])],
) -> Option<Animation> {
    let mut layout = Layout {
        names,
        next: PARAMS.len() as u32,
        parts: Vec::new(),
        vars: Vec::new(),
        globals: Vec::new(),
        render: Vec::new(),
    };
    let mut steps = Vec::new();
    let mut count = 0;
    'entries: for (this, part, pairs) in entries {
        for (target, src) in pairs.iter() {
            count += 1;
            if count > MAX_EXPRESSIONS {
                log::warn!(
                    "Horizon: custom entity model {file}: more than {MAX_EXPRESSIONS} animation \
                     expressions, the rest are left out"
                );
                break 'entries;
            }
            let Some(slot) = layout.slot(target, this, part) else {
                log::warn!("Horizon: custom entity model {file}: unknown target {target}");
                continue;
            };
            let compiled = compile(src, &mut |name: &str| layout.slot(name, this, part));
            match compiled {
                Ok(expr) => steps.push(Step { slot, expr }),
                Err(e) => log::warn!("Horizon: custom entity model {file}: {target}: {e}"),
            }
        }
    }
    if steps.is_empty() {
        return None;
    }
    Some(Animation {
        steps,
        slots: layout.next as usize,
        parts: layout.parts,
        vars: layout.vars.into_iter().map(|(_, slot)| slot).collect(),
        globals: layout
            .globals
            .into_iter()
            .map(|(name, slot)| (slot, global(&name)))
            .collect(),
        saved: Mutex::default(),
    })
}

thread_local!(static SLOTS: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) });

/// Past this many entities, an animation forgets those it hasn't drawn for `STALE` frames,
/// and starts over at twice as many.
const SAVED: usize = 4096;
const STALE: u32 = 600;

fn flag(b: bool) -> f32 {
    f32::from(b)
}

impl Animation {
    /// Runs the animation for `m` over `poses`, which the vanilla animation has set.
    pub fn run(&self, m: &Mob, s: &State, poses: &mut Poses) {
        self.run_in(&frame(), m, s, poses);
    }

    /// `run` in `frame`.
    fn run_in(&self, frame: &Frame, m: &Mob, s: &State, poses: &mut Poses) {
        self.run_with(
            m.id,
            frame.frame_counter,
            |slots| params(m, s, frame, slots),
            poses,
        );
    }

    /// `run` for the entity `id` in the frame `counter`, whose parameters `fill` writes.
    fn run_with(&self, id: u32, counter: u32, fill: impl FnOnce(&mut [f32]), poses: &mut Poses) {
        let preview = IN_GUI.get();
        SLOTS.with_borrow_mut(|slots| {
            slots.clear();
            slots.resize(self.slots, 0.0);
            fill(slots);
            // What vanilla left: the pose, `visible` and `skipDraw`.
            for b in &self.parts {
                let base = b.base as usize;
                slots[base..base + 9].copy_from_slice(poses.get(&b.name));
                slots[base + 9] = flag(!poses.is_hidden_named(&b.name));
                slots[base + 10] = flag(!poses.skips_named(&b.name));
            }
            let mut saved = lock(&self.saved);
            // Each run of a frame starts from the variables the frame began with; only the
            // first one in the world keeps its result, so that drawing again (or in a
            // preview) changes nothing. A preview of an entity the world no longer draws
            // keeps its result too.
            let mut vars = None;
            if !self.vars.is_empty() || !self.globals.is_empty() {
                if saved.entities.len() >= SAVED && !saved.entities.contains_key(&id) {
                    saved.make_room(counter);
                }
                let globals = |into: &mut [f32]| {
                    if !self.globals.is_empty() {
                        let g = lock(&GLOBALS);
                        for (value, &(_, i)) in into.iter_mut().zip(&self.globals) {
                            *value = g.values[i as usize];
                        }
                    }
                };
                let v = saved.entities.entry(id).or_insert_with(|| {
                    // Seen for the first time: its variables began this frame at 0.
                    let zero: Box<[f32]> = vec![0.0; self.vars.len()].into_boxed_slice();
                    let mut globals_start = vec![0.0; self.globals.len()].into_boxed_slice();
                    globals(&mut globals_start);
                    Vars {
                        frame: counter,
                        stored: false,
                        world: None,
                        start: zero.clone(),
                        now: zero,
                        globals_start,
                    }
                });
                if v.frame != counter {
                    v.start.copy_from_slice(&v.now);
                    globals(&mut v.globals_start);
                    v.frame = counter;
                    v.stored = false;
                }
                for (&slot, value) in self.vars.iter().zip(v.start.iter()) {
                    slots[slot as usize] = *value;
                }
                for (&(slot, _), value) in self.globals.iter().zip(v.globals_start.iter()) {
                    slots[slot as usize] = *value;
                }
                vars = Some(v);
            }
            for step in &self.steps {
                let v = step.expr.eval(slots);
                // A part keeps the pose vanilla gave it, a variable its last value.
                if v.is_finite() {
                    slots[step.slot as usize] = v;
                }
            }
            if let Some(v) = vars
                && !v.stored
                && (!preview || v.world.is_none_or(|w| counter.wrapping_sub(w) > 1))
            {
                v.stored = true;
                if !preview {
                    v.world = Some(counter);
                }
                for (&slot, value) in self.vars.iter().zip(v.now.iter_mut()) {
                    *value = slots[slot as usize];
                }
                if !self.globals.is_empty() {
                    let mut g = lock(&GLOBALS);
                    for &(slot, i) in &self.globals {
                        g.values[i as usize] = slots[slot as usize];
                    }
                }
            }
            drop(saved);
            for b in &self.parts {
                let base = b.base as usize;
                // The slots began as the pose: writing back what no step changed keeps it.
                poses.get(&b.name).copy_from_slice(&slots[base..base + 9]);
                // Only a change is written: naming a part keeps what vanilla hid.
                let visible = slots[base + 9] != 0.0;
                if visible == poses.is_hidden_named(&b.name) {
                    poses.visible(&b.name, visible);
                }
                // `visible_boxes = 0` skips the part's own boxes; 1 leaves vanilla's skipDraw.
                if slots[base + 10] == 0.0 && !poses.skips_named(&b.name) {
                    poses.skip_draw(&b.name);
                }
            }
        });
    }
}

impl Saved {
    /// Before a new entity joins a full table, forgets the entities not drawn for a while,
    /// and everything if that is not enough.
    fn make_room(&mut self, counter: u32) {
        if self.evicted != Some(counter) {
            self.evicted = Some(counter);
            self.entities
                .retain(|_, v| counter.wrapping_sub(v.frame) <= STALE);
        }
        if self.entities.len() >= 2 * SAVED {
            self.entities.clear();
        }
    }
}

fn params(m: &Mob, s: &State, frame: &Frame, slots: &mut [f32]) {
    let max_health = super::finalize::max_health(m);
    let (dx, dy, dz) = (
        m.position.x - frame.player[0],
        m.position.y - frame.player[1],
        m.position.z - frame.player[2],
    );
    // The movement's direction against the head's: EMF's move_forward and move_strafing.
    let (forward, strafing) = {
        let (vx, vz) = (m.velocity.x as f32, m.velocity.z as f32);
        let len = (vx * vx + vz * vz).sqrt();
        if len < 1.0e-4 {
            (0.0, 0.0)
        } else {
            let yaw = m.head_yaw.to_radians();
            let (fx, fz) = (-yaw.sin(), yaw.cos());
            ((vx * fx + vz * fz) / len, (vx * -fz + vz * fx) / len)
        }
    };
    // `Entity.isVisuallySwimming` (`Pose.SWIMMING`, id 3): a player's or mannequin's pose comes
    // in `avatar_pose`, another mob's in `pose`. A drowned has no such pose, only the shared
    // swimming flag (`extra.flag`).
    let swim_pose = match &*m.kind.id {
        "minecraft:player" | "minecraft:mannequin" => {
            m.extra.avatar_pose == crate::pose::Pose::Swimming
        }
        _ => m.extra.pose == 3 || (m.species() == super::Species::Drowned && m.extra.flag),
    };
    // The arm the current swing moves: the main arm unless the swing named its hand.
    let swinging = m.ai.swinging || m.extra.swing_right_arm.is_some();
    let swing_right = m.extra.swing_right_arm.unwrap_or(!m.main_arm_left());
    // `HumanoidArm`'s items: the main hand is on the main arm.
    let (right_hand, left_hand) = if m.main_arm_left() {
        (&m.extra.off_hand, &m.extra.main_hand)
    } else {
        (&m.extra.main_hand, &m.extra.off_hand)
    };
    for (i, (_, p)) in PARAMS.iter().enumerate() {
        use Param::*;
        slots[i] = match p {
            LimbSwing => s.pos,
            LimbSpeed => s.speed,
            Age => s.age,
            HeadYaw => s.y_rot,
            HeadPitch => s.x_rot,
            SwingProgress => s.swing,
            Health => m.health,
            MaxHealth => max_health,
            HurtTime => m.hurt_time.max(0) as f32 - if m.hurt_time > 0 { s.partial } else { 0.0 },
            DeathTime => m.death_time as f32 + if m.death_time > 0 { s.partial } else { 0.0 },
            PosX => m.position.x as f32,
            PosY => m.position.y as f32,
            PosZ => m.position.z as f32,
            RotX => m.head_pitch,
            RotY => m.body_yaw,
            Id => m.id as f32,
            // OptiFine's `time` loops at 27720 ticks to keep float precision.
            Time => (frame.time % 27720.0) as f32,
            DayTime => (frame.day_time % 24000.0) as f32,
            DayCount => (frame.day_time / 24000.0).floor() as f32,
            FrameTime => frame.frame_time,
            FrameCounter => (frame.frame_counter % 27720) as f32,
            Dimension => frame.dimension,
            PlayerX => frame.player[0] as f32,
            PlayerY => frame.player[1] as f32,
            PlayerZ => frame.player[2] as f32,
            PlayerRotX => frame.player_rot[0],
            PlayerRotY => frame.player_rot[1],
            // Random models' rule (a later change); the model's own rule is 0 until then.
            RuleIndex => 0.0,
            // A client knows only a wolf's anger end time (as a remaining time taken when it
            // was synced, which doesn't count down) and never the start: 0 for now.
            AngerTime | AngerTimeStart => 0.0,
            Distance => (dx * dx + dy * dy + dz * dz).sqrt() as f32,
            MoveForward => forward,
            MoveStrafing => strafing,
            // How deep the entity is in its fluid, which is what `fluid_depth_down` measures
            // once it touches the bottom.
            FluidDepth | FluidDepthDown => m.fluids.water.max(m.fluids.lava) as f32,
            // Both need the blocks around the mob (the ground below, the fluid's surface
            // above), which the draw doesn't have: 0 for now.
            HeightAboveGround | FluidDepthUp => 0.0,
            // The synced `Mob.isAggressive`, which a client's mobs have too.
            Aggressive => flag(m.ai.aggressive),
            Alive => flag(m.alive()),
            Burning => flag(m.fire_ticks > 0),
            Child => flag(s.baby),
            Glowing => flag(m.glowing),
            Hurt => flag(m.hurt_time > 0),
            InWater => flag(m.fluids.in_water),
            InLava => flag(m.fluids.in_lava),
            // `isInWaterOrRain` without the rain: whether rain reaches the mob needs the
            // weather and the sky above it, which the draw doesn't have.
            Wet => flag(m.fluids.in_water),
            Invisible => flag(m.invisible),
            OnGround => flag(m.on_ground),
            Riding => flag(m.extra.passenger),
            // `Entity.isVehicle`: SetPassengers names its first rider.
            Ridden => flag(m.extra.rider.is_some() || m.rider.is_some()),
            Climbing => flag(m.climbing),
            Jumping => flag(!m.on_ground && m.velocity.y > 0.0),
            Paused => flag(frame.paused),
            InGui => flag(IN_GUI.get()),
            FirstPerson => flag(frame.first_person),
            ThirdPerson => flag(!frame.first_person && !frame.third_person_reversed),
            ThirdPersonReversed => flag(frame.third_person_reversed),
            RightHanded => flag(!m.main_arm_left()),
            // The tamed animals' DATA_FLAGS_ID, and the species that keep their own.
            Sitting => flag(match m.species() {
                super::Species::Fox => super::fox::flag(m, super::fox::SITTING),
                super::Species::Panda => super::panda::is_sitting(m),
                super::Species::Camel | super::Species::CamelHusk => super::camel::sitting(m),
                _ => m.extra.sitting,
            }),
            Tamed => flag(m.extra.tame),
            Sprinting => flag(m.extra.sprinting),
            Sneaking => flag(m.extra.crouching),
            // A player's `isBlocking`: the item in use blocks attacks.
            Blocking => flag(m.extra.blocking),
            // `isVisuallyCrawling`: the swimming pose out of water.
            Crawling => flag(swim_pose && !m.fluids.in_water),
            Swimming => flag(swim_pose),
            // DATA_LIVING_ENTITY_FLAGS' using bit (the client counts the ticks).
            UsingItem => flag(m.extra.using_item > 0),
            SwingingRight => flag(swinging && swing_right),
            SwingingLeft => flag(swinging && !swing_right),
            HoldingRight => flag(right_hand.is_some()),
            HoldingLeft => flag(left_hand.is_some()),
            // Where the model is drawn (a held, framed or worn item, a shoulder): the draw
            // doesn't pass it yet, so 0 until something reads them.
            InHand | InItemFrame | InGround | OnHead | OnShoulder | FirstPersonHand | Hovered => {
                0.0
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bone(name: &str, children: Vec<Part>) -> Part {
        Part {
            name: name.into(),
            pose: [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children,
        }
    }
    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }
    const LIMB_SWING: usize = Param::LimbSwing as usize;

    #[test]
    fn animations_override_poses_and_keep_variables_per_entity() {
        // A head (OptiFine's name for it maps to `real_head`) holding a custom bone `brim`.
        let model = bone(
            "root",
            vec![bone(
                "real_head",
                vec![bone("cem#0", vec![bone("cem#1", vec![])])],
            )],
        );
        let ids: HashMap<String, String> = [
            ("cem#0".into(), "hat".into()),
            ("cem#1".into(), "brim".into()),
        ]
        .into();
        let vanilla = |p: &str| {
            if p == "head" {
                "real_head".into()
            } else {
                p.to_owned()
            }
        };
        let names = Names {
            model: &model,
            vanilla: &vanilla,
            ids: &ids,
        };
        let head = pairs(&[
            ("var.steps", "var.steps + 1"),
            // Reads the vanilla pose (ty 2) before it changes.
            ("head.ty", "part.ty * 10"),
            ("head.rx", "sin(limb_swing) * 0.5"),
            ("hat:brim.tz", "var.steps"),
            ("brim.visible", "var.steps < 2"),
            ("this.sx", "nope + 1"),
        ]);
        let hat = pairs(&[("this.ry", "torad(90)"), ("render.shadow_size", "0.5")]);
        let entries = [
            (
                "real_head".to_owned(),
                "real_head".to_owned(),
                head.as_slice(),
            ),
            ("cem#0".to_owned(), "real_head".to_owned(), hat.as_slice()),
        ];
        let animation = compile_animation("test.jem", &names, &entries).unwrap();
        let frame = |id: u32, counter: u32, poses: &mut Poses| {
            animation.run_with(
                id,
                counter,
                |s| s[LIMB_SWING] = std::f32::consts::FRAC_PI_2,
                poses,
            );
        };
        let mut poses = Poses::new(&model);
        frame(7, 1, &mut poses);
        assert_eq!(poses.get("real_head")[..4], [1.0, 20.0, 3.0, 0.5]);
        assert_eq!(poses.get("cem#1")[2], 1.0);
        assert!((poses.get("cem#0")[4] - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        // The expression with an unknown name is left out: the scale stays.
        assert_eq!(poses.get("real_head")[6], 1.0);
        assert!(!poses.is_hidden(&bone("cem#1", vec![])));
        // The next frame starts from the vanilla pose again, with this entity's counter.
        let mut poses = Poses::new(&model);
        frame(7, 2, &mut poses);
        assert_eq!(poses.get("cem#1")[2], 2.0);
        assert!(poses.is_hidden(&bone("cem#1", vec![])));
        // Another entity counts on its own.
        let mut poses = Poses::new(&model);
        frame(8, 2, &mut poses);
        assert_eq!(poses.get("cem#1")[2], 1.0);
    }

    #[test]
    fn nothing_to_run_without_valid_expressions() {
        let model = bone("root", vec![]);
        let ids = HashMap::new();
        let names = Names {
            model: &model,
            vanilla: &|p| p.to_owned(),
            ids: &ids,
        };
        let bad = pairs(&[("missing.rx", "1"), ("root.rx", "1 +")]);
        let entries = [("root".to_owned(), "root".to_owned(), bad.as_slice())];
        assert!(compile_animation("test.jem", &names, &entries).is_none());
    }

    /// The animation `list` over a model whose parts keep their own names.
    fn animate(model: &Part, list: &[(&str, &str)]) -> Animation {
        let ids = HashMap::new();
        let names = Names {
            model,
            vanilla: &|p| p.to_owned(),
            ids: &ids,
        };
        let list = pairs(list);
        let entries = [("root".to_owned(), "root".to_owned(), list.as_slice())];
        compile_animation("test.jem", &names, &entries).unwrap()
    }

    #[test]
    fn naming_a_part_keeps_what_vanilla_hid() {
        let model = bone("root", vec![bone("head", vec![])]);
        let hidden = || {
            let mut poses = Poses::new(&model);
            poses.visible("head", false);
            poses.skip_draw("head");
            poses
        };
        // Only the rotation changes: the part stays hidden and its boxes skipped.
        let mut poses = hidden();
        animate(&model, &[("head.rx", "1")]).run_with(1, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("head")[3], 1.0);
        assert!(poses.is_hidden_named("head") && poses.skips_named("head"));
        // An animation that reads the flags sees vanilla's: hidden and skipped are 0.
        let mut poses = hidden();
        animate(
            &model,
            &[("head.tx", "head.visible + head.visible_boxes * 2")],
        )
        .run_with(1, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("head")[0], 0.0);
        // visible 1 shows the part; visible_boxes 1 leaves vanilla's skipDraw.
        let mut poses = hidden();
        animate(
            &model,
            &[("head.visible", "1"), ("head.visible_boxes", "1")],
        )
        .run_with(1, 1, |_| {}, &mut poses);
        assert!(!poses.is_hidden_named("head") && poses.skips_named("head"));
        // visible_boxes 0 skips the part's own boxes and leaves it visible.
        let mut poses = Poses::new(&model);
        animate(&model, &[("head.visible_boxes", "0")]).run_with(1, 1, |_| {}, &mut poses);
        assert!(poses.skips_named("head") && !poses.is_hidden_named("head"));
        // visible 0 hides a part vanilla showed.
        let mut poses = Poses::new(&model);
        animate(&model, &[("head.visible", "0")]).run_with(1, 1, |_| {}, &mut poses);
        assert!(poses.is_hidden_named("head") && !poses.skips_named("head"));
    }

    #[test]
    fn a_value_that_is_not_finite_keeps_the_vanilla_pose_and_the_variable() {
        let model = bone("root", vec![]);
        let mut poses = Poses::new(&model);
        animate(
            &model,
            &[
                ("root.tx", "sqrt(-1)"),
                ("root.ty", "5"),
                ("root.rx", "1 / 0"),
                ("var.k", "7"),
                ("var.k", "0 / 0"),
                ("var.k", "1 / 0"),
                ("root.tz", "var.k"),
            ],
        )
        .run_with(1, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("root")[..4], [1.0, 5.0, 7.0, 0.0]);
    }

    #[test]
    fn the_parameters_come_from_the_mob_and_its_state() {
        let mut m = Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            horizon_core::geometry::Vec3::new(3.0, 4.0, 0.0),
            0.0,
        );
        m.hurt_time = 5;
        m.fire_ticks = 3;
        m.on_ground = false;
        m.velocity = horizon_core::geometry::Vec3::new(0.0, 0.1, 0.0);
        let s = State {
            pos: 1.5,
            speed: 0.0,
            age: 0.0,
            y_rot: 30.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: true,
            partial: 0.25,
        };
        let model = bone("root", vec![]);
        let animation = animate(
            &model,
            &[
                ("root.tx", "hurt_time"),
                ("root.ty", "limb_swing"),
                ("root.tz", "head_yaw"),
                ("root.rx", "is_burning + is_jumping * 2 + is_child * 4"),
                ("root.ry", "distance"),
                (
                    "root.rz",
                    "is_player_third_person + is_player_first_person * 2",
                ),
            ],
        );
        let frame = Frame {
            player: [0.0, 0.0, 0.0],
            ..Frame::default()
        };
        let mut poses = Poses::new(&model);
        animation.run_in(&frame, &m, &s, &mut poses);
        assert_eq!(poses.get("root")[..6], [4.75, 1.5, 30.0, 7.0, 5.0, 2.0]);
    }

    /// A mob at the origin of the test species.
    fn mob() -> Mob {
        Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        )
    }

    /// A state with every value at rest.
    fn still() -> State {
        State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
        }
    }

    #[test]
    fn the_flags_come_from_the_synced_mob_fields() {
        let mut m = mob();
        m.ai.aggressive = true;
        m.extra.sitting = true;
        m.extra.tame = true;
        m.extra.sprinting = true;
        m.extra.crouching = true;
        // The player's main hand option set to left.
        m.extra.main_hand_left = true;
        m.extra.blocking = true;
        // The swimming pose out of water: crawling.
        m.extra.pose = 3;
        m.extra.using_item = 3;
        // Only the main hand holds something, and it is the left one.
        m.extra.set_main_hand(Some("minecraft:stick".into()));
        // A swing that names no hand moves the main arm: the left.
        m.ai.swinging = true;
        m.extra.rider = Some(7);
        let model = bone("root", vec![]);
        let animation = animate(
            &model,
            &[(
                "root.tx",
                "is_aggressive + is_sitting * 2 + is_tamed * 4 + is_sprinting * 8 \
                 + is_sneaking * 16 + is_right_handed * 32 + is_blocking * 64 \
                 + is_crawling * 128 + is_swimming * 256 + is_using_item * 512 \
                 + is_holding_item_right * 1024 + is_holding_item_left * 2048 \
                 + is_swinging_right_arm * 4096 + is_swinging_left_arm * 8192 \
                 + is_ridden * 16384",
            )],
        );
        let mut poses = Poses::new(&model);
        animation.run_in(&Frame::default(), &m, &still(), &mut poses);
        assert_eq!(
            poses.get("root")[0],
            (31 + 64 + 128 + 256 + 512 + 2048 + 8192 + 16384) as f32
        );
    }

    #[test]
    fn the_swimming_pose_in_water_swims_without_crawling() {
        let mut m = mob();
        m.extra.pose = 3;
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("root.tx", "is_swimming + is_crawling * 2")]);
        let mut read = |in_water: bool| {
            m.fluids.in_water = in_water;
            let mut poses = Poses::new(&model);
            animation.run_in(&Frame::default(), &m, &still(), &mut poses);
            poses.get("root")[0]
        };
        assert_eq!([read(true), read(false)], [1.0, 3.0]);
    }

    #[test]
    fn a_fox_sits_by_its_own_flags() {
        let mut m = mob();
        m.kind = std::sync::Arc::new(crate::mobs::MobType {
            species: crate::mobs::Species::Fox,
            ..(*m.kind).clone()
        });
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("root.tx", "is_sitting")]);
        let mut sitting = |flags: i32| {
            m.extra.ints[0] = flags;
            let mut poses = Poses::new(&model);
            animation.run_in(&Frame::default(), &m, &still(), &mut poses);
            poses.get("root")[0]
        };
        assert_eq!(sitting(crate::mobs::fox::SITTING), 1.0);
        assert_eq!(sitting(crate::mobs::fox::CROUCHING), 0.0);
    }

    #[test]
    fn expressions_past_the_cap_are_left_out() {
        let model = bone("root", vec![]);
        let list = vec![("root.tx", "1"); MAX_EXPRESSIONS + 1];
        assert_eq!(animate(&model, &list).steps.len(), MAX_EXPRESSIONS);
    }

    #[test]
    fn a_preview_leaves_the_world_entity_variables_alone() {
        let model = bone("root", vec![]);
        let animation = animate(
            &model,
            &[
                ("var.n", "var.n + 1 + is_in_gui * 100"),
                ("root.tx", "var.n"),
            ],
        );
        let (m, s) = (mob(), still());
        let tx = |counter: u32| {
            let frame = Frame {
                frame_counter: counter,
                ..Frame::default()
            };
            let mut poses = Poses::new(&model);
            animation.run_in(&frame, &m, &s, &mut poses);
            poses.get("root")[0]
        };
        // A preview in a frame the world drew, then one in a frame it didn't: each starts from
        // where its frame began (0, then the world's last 2) and stores nothing itself.
        assert_eq!(
            [tx(1), in_gui(|| tx(1)), tx(2), in_gui(|| tx(3)), tx(4)],
            [1.0, 101.0, 2.0, 103.0, 3.0]
        );
    }

    #[test]
    fn drawing_again_in_the_same_frame_does_not_advance_variables() {
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("var.n", "var.n + 1"), ("root.tx", "var.n")]);
        let mut poses = Poses::new(&model);
        let mut tx = |counter: u32| {
            animation.run_with(1, counter, |_| {}, &mut poses);
            poses.get("root")[0]
        };
        assert_eq!(
            [tx(10), tx(10), tx(11), tx(11), tx(12)],
            [1.0, 1.0, 2.0, 2.0, 3.0]
        );
    }

    #[test]
    fn global_variables_are_shared_by_every_entity() {
        let model = bone("root", vec![]);
        let animation = animate(
            &model,
            &[
                (
                    "global_var.cem_anim_test_shared",
                    "global_var.cem_anim_test_shared + 1",
                ),
                ("root.tx", "global_var.cem_anim_test_shared"),
            ],
        );
        let mut poses = Poses::new(&model);
        animation.run_with(1, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("root")[0], 1.0);
        animation.run_with(2, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("root")[0], 2.0);
        // Another model reading the same name sees the same value.
        let reader = animate(&model, &[("root.ty", "global_var.cem_anim_test_shared")]);
        reader.run_with(3, 1, |_| {}, &mut poses);
        assert_eq!(poses.get("root")[1], 2.0);
    }

    #[test]
    fn a_full_table_forgets_only_entities_not_seen_for_a_while() {
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("var.n", "var.n + 1"), ("root.tx", "var.n")]);
        let mut poses = Poses::new(&model);
        let mut tx = |id: u32, counter: u32| {
            animation.run_with(id, counter, |_| {}, &mut poses);
            poses.get("root")[0]
        };
        for id in 0..SAVED as u32 {
            tx(id, 1);
        }
        // Entity 0 comes back 699 frames later; a new one then fills the table.
        assert_eq!(tx(0, 700), 2.0);
        assert_eq!(tx(100_000, 701), 1.0);
        // Entity 0 was seen recently and keeps counting; entity 1 was forgotten.
        assert_eq!(tx(0, 702), 3.0);
        assert_eq!(tx(1, 702), 1.0);
    }

    #[test]
    fn drawing_an_entity_again_reads_and_stores_the_globals_once() {
        let model = bone("root", vec![]);
        let animation = animate(
            &model,
            &[
                (
                    "global_var.cem_anim_test_redraw",
                    "global_var.cem_anim_test_redraw + 1 + is_in_gui * 100",
                ),
                ("root.tx", "global_var.cem_anim_test_redraw"),
            ],
        );
        let mut poses = Poses::new(&model);
        let mut tx = |counter: u32| {
            // Only `is_in_gui` matters here: the rest of the parameters stay 0.
            let fill = |slots: &mut [f32]| slots[Param::InGui as usize] = flag(IN_GUI.get());
            animation.run_with(7, counter, fill, &mut poses);
            poses.get("root")[0]
        };
        // Two draws in the world and a preview in frame 1 read the same start; only the first
        // stores, so frame 2 counts on from 1, not from the preview's 101.
        assert_eq!(
            [tx(1), tx(1), in_gui(|| tx(1)), tx(2)],
            [1.0, 1.0, 101.0, 2.0]
        );
    }

    #[test]
    fn an_entity_only_a_preview_draws_still_counts() {
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("var.n", "var.n + 1"), ("root.tx", "var.n")]);
        let mut poses = Poses::new(&model);
        let mut tx = |counter: u32| {
            animation.run_with(5, counter, |_| {}, &mut poses);
            poses.get("root")[0]
        };
        // The inventory's own player in first person: the world never draws it.
        assert_eq!(
            [in_gui(|| tx(1)), in_gui(|| tx(2)), in_gui(|| tx(3))],
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn a_table_of_recent_entities_starts_over_at_twice_its_size() {
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("var.n", "var.n + 1"), ("root.tx", "var.n")]);
        let mut poses = Poses::new(&model);
        let mut tx = |id: u32, counter: u32| {
            animation.run_with(id, counter, |_| {}, &mut poses);
            poses.get("root")[0]
        };
        // Every entity is recent, so none can be forgotten until the table is twice full.
        for id in 0..2 * SAVED as u32 {
            tx(id, 1);
        }
        assert_eq!(tx(0, 2), 2.0);
        tx(1_000_000, 2);
        assert_eq!(tx(0, 3), 1.0);
    }

    /// Held by the tests that change `FRAME`.
    static FRAME_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn a_preview_runs_in_the_gui_and_the_world_frame_comes_back() {
        let _lock = lock(&FRAME_TEST);
        set_frame(Frame {
            time: 77.0,
            ..Frame::default()
        });
        let (m, s) = (mob(), still());
        let model = bone("root", vec![]);
        let animation = animate(&model, &[("root.tx", "is_in_gui"), ("root.ty", "time")]);
        let run = || {
            let mut poses = Poses::new(&model);
            animation.run(&m, &s, &mut poses);
            let p = *poses.get("root");
            [p[0], p[1]]
        };
        assert_eq!(run(), [0.0, 77.0]);
        assert_eq!(in_gui(run), [1.0, 77.0]);
        assert_eq!(run(), [0.0, 77.0]);
    }

    #[test]
    fn each_frame_counts_one_more() {
        let _lock = lock(&FRAME_TEST);
        let counter = || frame().frame_counter;
        let a = counter();
        set_frame(Frame::default());
        let b = counter();
        set_frame(Frame::default());
        let c = counter();
        assert_eq!([b.wrapping_sub(a), c.wrapping_sub(b)], [1, 1]);
    }
}
