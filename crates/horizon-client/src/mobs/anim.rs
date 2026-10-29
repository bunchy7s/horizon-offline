//! Model animation: ports of each vanilla `EntityModel.setupAnim` family
//! (quadrupeds, humanoids, creepers, spiders, chickens...) applied to a map of
//! part poses before the model is drawn.
use super::{Mob, Part, keyframes::AnimationState, living::wrap};
use horizon_core::geometry::trig::Mth;
use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_6, PI};

/// The setupAnim implementation a model uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    Nautilus,
    HappyGhast,
    Sniffer,
    Breeze,
    Warden,
    Allay,
    Creaking,
    Camel,
    Wither,
    Vex,
    Ravager,
    Guardian,
    Shulker,
    Quadruped,
    Sheep,
    Chicken,
    Zombie,
    Drowned,
    ZombieVillager,
    Skeleton,
    Creeper,
    Spider,
    Enderman,
    Silverfish,
    Slime,
    MagmaCube,
    Witch,
    Villager,
    IronGolem,
    SnowGolem,
    CopperGolem,
    Illager,
    Piglin,
    ZombifiedPiglin,
    Hoglin,
    Wolf,
    Feline,
    Fox,
    Rabbit,
    Horse,
    Llama,
    Goat,
    Panda,
    PolarBear,
    Parrot,
    Bee,
    Turtle,
    Armadillo,
    Frog,
    Axolotl,
    Squid,
    Fish,
    Pufferfish,
    Dolphin,
    Bat,
    Phantom,
    Ghast,
    Blaze,
    Strider,
    /// The humanoid model of the earlier simple renderer.
    Humanoid {
        zombie_arms: bool,
    },
}
/// Client-side animation clocks advanced each tick.
#[derive(Clone, Debug, Default)]
pub struct AnimState {
    pub attack_anim: f32,
    pub attack_anim_o: f32,
    pub flap: f32,
    pub flap_o: f32,
    pub flap_speed: f32,
    pub flap_speed_o: f32,
    pub flapping: f32,
    /// Generic per-species keyframe animation states.
    pub states: [AnimationState; 6],
    /// Per-species animation values (tail wag, squid tentacles, squish...)
    /// with their previous-tick copies for interpolation.
    pub v: [f32; 8],
    pub vo: [f32; 8],
}
impl AnimState {
    pub fn begin_tick(&mut self) {
        self.attack_anim_o = self.attack_anim;
        self.vo = self.v;
    }
    /// Interpolated value `i` at the render partial tick.
    pub fn lerp(&self, i: usize, partial: f32) -> f32 {
        self.vo[i] + (self.v[i] - self.vo[i]) * partial
    }
    pub fn end_tick(&mut self, _swinging: bool) {}
}
/// Part poses for one draw, keyed by part name.
#[derive(Clone, Default)]
pub struct Poses {
    map: HashMap<String, [f32; 9]>,
    dummy: [f32; 9],
    /// Parts with `skipDraw` set: their own cubes are not drawn, children are.
    skipped: std::collections::HashSet<String>,
    /// Parts with `visible` cleared: neither they nor their children draw.
    hidden: std::collections::HashSet<String>,
}
impl Poses {
    pub fn new(model: &Part) -> Self {
        let mut poses = Self::default();
        poses.reset(model);
        poses
    }
    /// Refills this for another model, keeping the map and the sets' capacity:
    /// a mob draws its body and every equipment layer each frame.
    pub fn reset(&mut self, model: &Part) {
        fn walk(p: &Part, map: &mut HashMap<String, [f32; 9]>) {
            map.insert(p.name.clone(), p.pose);
            for c in &p.children {
                walk(c, map);
            }
        }
        self.map.clear();
        self.dummy = [0.0; 9];
        self.skipped.clear();
        self.hidden.clear();
        walk(model, &mut self.map);
    }
    pub fn get(&mut self, name: &str) -> &mut [f32; 9] {
        match self.map.get_mut(name) {
            Some(p) => p,
            None => &mut self.dummy,
        }
    }
    pub fn pose(&self, part: &Part) -> [f32; 9] {
        self.map.get(&part.name).copied().unwrap_or(part.pose)
    }
    pub fn skip_draw(&mut self, name: &str) {
        self.skipped.insert(name.to_owned());
    }
    pub fn skips(&self, part: &Part) -> bool {
        self.skipped.contains(&part.name)
    }
    pub fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
    pub fn xrot(&mut self, n: &str, v: f32) {
        self.get(n)[3] = v;
    }
    pub fn yrot(&mut self, n: &str, v: f32) {
        self.get(n)[4] = v;
    }
    pub fn zrot(&mut self, n: &str, v: f32) {
        self.get(n)[5] = v;
    }
    /// `ModelPart.visible = v`; a hidden part and its children do not draw.
    pub fn visible(&mut self, n: &str, v: bool) {
        if v {
            self.hidden.remove(n);
        } else if self.map.contains_key(n) {
            self.hidden.insert(n.to_owned());
        }
    }
    pub fn is_hidden(&self, part: &Part) -> bool {
        self.hidden.contains(&part.name)
    }
}
/// The LivingEntityRenderState values setupAnim reads.
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub pos: f32,
    pub speed: f32,
    pub age: f32,
    /// Head yaw relative to the body, degrees.
    pub y_rot: f32,
    /// Head pitch, degrees.
    pub x_rot: f32,
    pub swing: f32,
    pub baby: bool,
    pub partial: f32,
}
pub fn state(m: &Mob, alpha: f32) -> State {
    let body = m.previous_body_yaw + wrap(m.body_yaw - m.previous_body_yaw) * alpha;
    let head = m.previous_head_yaw + wrap(m.head_yaw - m.previous_head_yaw) * alpha;
    let mut swing = m.anim.attack_anim - m.anim.attack_anim_o;
    if swing < 0.0 {
        swing += 1.0;
    }
    let swing = m.anim.attack_anim_o + swing * alpha;
    // LivingEntityRenderer.extractRenderState: riding motion is not walking.
    let walking = m.alive() && !m.extra.passenger;
    State {
        pos: if walking {
            // WalkAnimationState.position(partial): babies' legs move 3 times as far.
            (m.limb_position - m.limb_speed * (1.0 - alpha)) * if m.is_baby() { 3.0 } else { 1.0 }
        } else {
            0.0
        },
        speed: if walking {
            (m.previous_limb_speed + (m.limb_speed - m.previous_limb_speed) * alpha).min(1.0)
        } else {
            0.0
        },
        age: m.age as f32 + alpha,
        y_rot: wrap(head - body),
        x_rot: m.previous_head_pitch + (m.head_pitch - m.previous_head_pitch) * alpha,
        swing,
        baby: m.is_baby(),
        partial: alpha,
    }
}
fn head(p: &mut Poses, s: &State) {
    p.xrot("head", s.x_rot.to_radians());
    p.yrot("head", s.y_rot.to_radians());
}
/// QuadrupedModel.setupAnim
pub(super) fn quadruped(p: &mut Poses, s: &State) {
    head(p, s);
    let (pos, sp) = (s.pos, s.speed);
    p.xrot("right_hind_leg", (pos * 0.6662).mcos() * 1.4 * sp);
    p.xrot("left_hind_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.xrot("right_front_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.xrot("left_front_leg", (pos * 0.6662).mcos() * 1.4 * sp);
}
/// AnimationUtils.bobModelPart
fn bob(p: &mut Poses, n: &str, age: f32, scale: f32) {
    let q = p.get(n);
    q[5] += scale * ((age * 0.09).mcos() * 0.05 + 0.05);
    q[3] += scale * ((age * 0.067).msin() * 0.05);
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArmPose {
    Empty,
    Item,
    Block,
    Bow,
    CrossbowHold,
    CrossbowCharge,
    /// Raising a trident to throw it (or to launch with Riptide).
    ThrowTrident,
    /// A spear held out (`HumanoidModel.ArmPose.SPEAR`), at rest or charging.
    Spear,
}
/// HumanoidModel.setupAnim with empty hands (endermen).
pub fn humanoid_pub(p: &mut Poses, s: &State, _m: &Mob) {
    humanoid(p, s, &Hum::new(ArmPose::Empty, ArmPose::Empty));
}
/// `AnimationUtils.swingWeaponDown`: the main arm swings down, the other
/// one hangs and bobs.
pub(super) fn swing_weapon_down(p: &mut Poses, swing: f32, age: f32, left_main: bool) {
    let attack2 = (swing * PI).msin();
    let attack = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
    let (main, off) = if left_main {
        ("left_arm", "right_arm")
    } else {
        ("right_arm", "left_arm")
    };
    p.zrot("right_arm", 0.0);
    p.zrot("left_arm", 0.0);
    p.yrot("right_arm", PI / 20.0);
    p.yrot("left_arm", -PI / 20.0);
    p.xrot(main, -1.884_955_8 + (age * 0.09).mcos() * 0.15);
    p.xrot(off, -0.0 + (age * 0.19).mcos() * 0.5);
    p.get(main)[3] += attack2 * 2.2 - attack * 0.4;
    p.get(off)[3] += attack2 * 1.2 - attack * 0.4;
    bob(p, "right_arm", age, 1.0);
    bob(p, "left_arm", age, -1.0);
}
/// What `HumanoidModel.setupAnim` reads besides the walk and head inputs.
#[derive(Clone, Copy, Debug)]
pub struct Hum {
    pub right: ArmPose,
    pub left: ArmPose,
    pub crouching: bool,
    pub passenger: bool,
    pub fall_flying: bool,
    pub fly_speed_scale: f32,
    /// `Some(aggressive)` for `ZombieModel`-style `animateZombieArms`.
    pub zombie: Option<bool>,
    /// The main hand holds something (`getMainHandItemStack() != EMPTY`).
    pub holding: bool,
    /// `maxCrossbowChargeDuration`, `ticksUsingItem`.
    pub charge: (f32, f32),
    /// `PiglinModel.setupAttackAnimation`: swing a held weapon down.
    pub weapon_swing: bool,
    /// `swimAmount` at this frame.
    pub swim: f32,
    /// `isUsingItem`: the swim stroke leaves the arms alone.
    pub using_item: bool,
    /// The arm the current swing moves (`Some(true)` for the right one).
    pub swing_right_arm: Option<bool>,
    /// The swing is a STAB (`SwingAnimationType` 2): a spear's jab.
    pub stab: bool,
    /// `mainArm` is the left one (`Mob.isLeftHanded`): a mob's swing uses it.
    pub left_main: bool,
    /// The spear each arm (right, left) holds, and how long that arm has
    /// been using it (`ticksUsingItem(arm)`, 0 when it is not).
    pub spear: [Option<crate::spear::Kinetic>; 2],
    pub spear_ticks: [f32; 2],
}
impl Hum {
    pub fn new(right: ArmPose, left: ArmPose) -> Self {
        Self {
            right,
            left,
            crouching: false,
            passenger: false,
            fall_flying: false,
            fly_speed_scale: 1.0,
            zombie: None,
            holding: false,
            charge: (0.0, 0.0),
            weapon_swing: false,
            swim: 0.0,
            using_item: false,
            swing_right_arm: None,
            stab: false,
            left_main: false,
            spear: [None; 2],
            spear_ticks: [0.0; 2],
        }
    }
}
impl ArmPose {
    fn two_handed(self) -> bool {
        matches!(self, Self::Bow | Self::CrossbowHold | Self::CrossbowCharge)
    }
    fn affects_offhand(self) -> bool {
        self.two_handed() || self == Self::ThrowTrident
    }
}
/// `AnimationUtils.animateCrossbowHold`.
pub(super) fn crossbow_hold(p: &mut Poses, hx: f32, hy: f32, in_right: bool) {
    let (hold, shoot) = if in_right {
        ("right_arm", "left_arm")
    } else {
        ("left_arm", "right_arm")
    };
    p.yrot(hold, if in_right { -0.3 } else { 0.3 } + hy);
    p.yrot(shoot, if in_right { 0.6 } else { -0.6 } + hy);
    p.xrot(hold, -PI / 2.0 + hx + 0.1);
    p.xrot(shoot, -1.5 + hx);
}
/// `AnimationUtils.animateCrossbowCharge`.
pub(super) fn crossbow_charge(p: &mut Poses, max: f32, ticks: f32, in_right: bool) {
    let (hold, pull) = if in_right {
        ("right_arm", "left_arm")
    } else {
        ("left_arm", "right_arm")
    };
    p.yrot(hold, if in_right { -0.8 } else { 0.8 });
    p.xrot(hold, -0.970_796_35);
    p.xrot(pull, -0.970_796_35);
    let use_ticks = ticks.clamp(0.0, max);
    let alpha = use_ticks / max;
    let lerp = |a: f32, p0: f32, p1: f32| p0 + a * (p1 - p0);
    p.yrot(
        pull,
        lerp(alpha, 0.4, 0.85) * if in_right { 1.0 } else { -1.0 },
    );
    let x = lerp(alpha, p.get(pull)[3], -PI / 2.0);
    p.xrot(pull, x);
}
/// `HumanoidModel.poseRightArm` / `poseLeftArm`.
fn pose_arm(p: &mut Poses, pose: ArmPose, right: bool, h: &Hum) {
    let arm = if right { "right_arm" } else { "left_arm" };
    let (hx, hy) = (p.get("head")[3], p.get("head")[4]);
    match pose {
        ArmPose::Empty => p.yrot(arm, 0.0),
        ArmPose::Item => {
            let x = p.get(arm)[3];
            p.xrot(arm, x * 0.5 - PI / 10.0);
            p.yrot(arm, 0.0);
        }
        ArmPose::Block => {
            let x = p.get(arm)[3] * 0.5 - 0.942_477_9 + hx.clamp(-1.396_263_4, 0.436_332_32);
            p.xrot(arm, x);
            p.yrot(
                arm,
                if right { -FRAC_PI_6 } else { FRAC_PI_6 } + hy.clamp(-FRAC_PI_6, FRAC_PI_6),
            );
        }
        ArmPose::Bow => {
            p.yrot("right_arm", -0.1 + hy - if right { 0.0 } else { 0.4 });
            p.yrot("left_arm", 0.1 + hy + if right { 0.4 } else { 0.0 });
            p.xrot("right_arm", -PI / 2.0 + hx);
            p.xrot("left_arm", -PI / 2.0 + hx);
        }
        ArmPose::ThrowTrident => {
            let x = p.get(arm)[3];
            p.xrot(arm, x * 0.5 - PI);
            p.yrot(arm, 0.0);
        }
        ArmPose::CrossbowHold => crossbow_hold(p, hx, hy, right),
        ArmPose::CrossbowCharge => crossbow_charge(p, h.charge.0, h.charge.1, right),
        // SpearAnimations.thirdPersonHandUse.
        ArmPose::Spear => {
            let side = usize::from(!right);
            let Some(kinetic) = h.spear[side] else {
                return;
            };
            let (x, y, z) = crate::spear::third_person_arm(
                &kinetic,
                h.spear_ticks[side],
                right,
                (hx, hy),
                h.fall_flying || h.swim > 0.0,
            );
            p.xrot(arm, x);
            p.yrot(arm, y);
            p.get(arm)[5] += z;
        }
    }
}
/// `HumanoidModel.setupAnim` (and `ZombieModel.setupAttackAnimation`).
pub(super) fn humanoid(p: &mut Poses, s: &State, h: &Hum) {
    head(p, s);
    if h.fall_flying {
        p.xrot("head", -std::f32::consts::FRAC_PI_4);
    }
    let (pos, sp) = (s.pos, s.speed / h.fly_speed_scale);
    p.xrot("right_arm", (pos * 0.6662 + PI).mcos() * 2.0 * sp * 0.5);
    p.xrot("left_arm", (pos * 0.6662).mcos() * 2.0 * sp * 0.5);
    p.xrot("right_leg", (pos * 0.6662).mcos() * 1.4 * sp);
    p.xrot("left_leg", (pos * 0.6662 + PI).mcos() * 1.4 * sp);
    p.yrot("right_leg", 0.005);
    p.yrot("left_leg", -0.005);
    p.zrot("right_leg", 0.005);
    p.zrot("left_leg", -0.005);
    if h.passenger {
        p.get("right_arm")[3] += -PI / 5.0;
        p.get("left_arm")[3] += -PI / 5.0;
        p.xrot("right_leg", -1.413_716_7);
        p.yrot("right_leg", PI / 10.0);
        p.zrot("right_leg", 0.078_539_82);
        p.xrot("left_leg", -1.413_716_7);
        p.yrot("left_leg", -PI / 10.0);
        p.zrot("left_leg", -0.078_539_82);
    }
    // The main arm is the right one: the off-hand pose goes first unless it
    // needs both hands.
    if h.left.two_handed() {
        pose_arm(p, h.right, true, h);
        if !h.right.affects_offhand() {
            pose_arm(p, h.left, false, h);
        }
    } else {
        pose_arm(p, h.left, false, h);
        if !h.left.affects_offhand() {
            pose_arm(p, h.right, true, h);
        }
    }
    // setupAttackAnimation: a WHACK or STAB swing of the swinging hand's arm.
    let hx = p.get("head")[3];
    let swing = s.swing;
    let swing_left = h.swing_right_arm.map_or(h.left_main, |right| !right);
    if swing > 0.0 && h.weapon_swing {
        swing_weapon_down(p, swing, s.age, h.left_main);
    } else if swing > 0.0 {
        let mut body_y = (swing.sqrt() * PI * 2.0).msin() * 0.2;
        if swing_left {
            body_y = -body_y;
        }
        p.yrot("body", body_y);
        let age_scale = if s.baby { 0.5 } else { 1.0 };
        {
            let r = p.get("right_arm");
            r[2] = body_y.msin() * 5.0 * age_scale;
            r[0] = -body_y.mcos() * 5.0 * age_scale;
        }
        {
            let l = p.get("left_arm");
            l[2] = -body_y.msin() * 5.0 * age_scale;
            l[0] = body_y.mcos() * 5.0 * age_scale;
        }
        let arm = if swing_left { "left_arm" } else { "right_arm" };
        if h.stab {
            // SpearAnimations.thirdPersonAttackHand: the arms keep their turn.
            p.get(arm)[3] += crate::spear::third_person_attack_arm(swing);
        } else {
            p.get("right_arm")[4] += body_y;
            p.get("left_arm")[4] += body_y;
            p.get("left_arm")[3] += body_y;
            // Ease.outQuart
            let eased = 1.0 - (1.0 - swing).powi(4);
            let aa = (eased * PI).msin();
            let bb = (swing * PI).msin() * -(hx - 0.7) * 0.75;
            let r = p.get(arm);
            r[3] -= aa * 1.2 + bb;
            r[4] += body_y * 2.0;
            r[5] += (swing * PI).msin() * -0.4;
        }
    }
    if let Some(aggressive) = h.zombie {
        // AnimationUtils.animateZombieArms
        let raise = !s.baby || !h.holding;
        let drop = if raise {
            -PI / if aggressive { 1.5 } else { 2.25 }
        } else {
            0.0
        };
        let ym = if raise { 1.0 } else { -1.0 } * (swing * PI).msin();
        let xm = ((1.0 - (1.0 - swing) * (1.0 - swing)) * PI).msin();
        let x = drop + ym * 1.2 - xm * 0.4;
        let y = 0.1 - ym * 0.6;
        // An arm raising a trident keeps its pose.
        if h.right != ArmPose::ThrowTrident {
            p.xrot("right_arm", x);
            p.yrot("right_arm", if raise { -y } else { y });
            p.zrot("right_arm", 0.0);
        }
        if h.left != ArmPose::ThrowTrident {
            p.xrot("left_arm", x);
            p.yrot("left_arm", if raise { y } else { -y });
            p.zrot("left_arm", 0.0);
        }
    }
    if h.crouching {
        p.xrot("body", 0.5);
        p.get("right_arm")[3] += 0.4;
        p.get("left_arm")[3] += 0.4;
        p.get("right_leg")[2] += 4.0;
        p.get("left_leg")[2] += 4.0;
        p.get("head")[1] += 4.2;
        p.get("body")[1] += 3.2;
        p.get("left_arm")[1] += 3.2;
        p.get("right_arm")[1] += 3.2;
    }
    bob(p, "right_arm", s.age, 1.0);
    bob(p, "left_arm", s.age, -1.0);
    if h.swim > 0.0 {
        swim(p, s.pos, h);
    }
}
/// `Mth.rotLerpRad`.
fn rot_lerp_rad(a: f32, from: f32, to: f32) -> f32 {
    let mut diff = to - from;
    while diff < -PI {
        diff += PI * 2.0;
    }
    while diff >= PI {
        diff -= PI * 2.0;
    }
    from + a * diff
}
/// `DrownedModel.setupSwimAnimation`: the arms reach up and sway, the legs
/// kick and the head lies straight.
fn drowned_swim(p: &mut Poses, s: &State, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let sway = amount * 0.35 * (0.1 * s.age).msin();
    let kick = amount * 0.55 * (0.1 * s.age).msin();
    let reach = -PI * 4.0 / 5.0;
    let (right_x, left_x) = (p.get("right_arm")[3], p.get("left_arm")[3]);
    let (right_z, left_z) = (p.get("right_arm")[5], p.get("left_arm")[5]);
    p.xrot("right_arm", rot_lerp_rad(amount, right_x, reach) + sway);
    p.xrot("left_arm", rot_lerp_rad(amount, left_x, reach) - sway);
    p.zrot("right_arm", rot_lerp_rad(amount, right_z, -0.15));
    p.zrot("left_arm", rot_lerp_rad(amount, left_z, 0.15));
    let (left_leg, right_leg) = (p.get("left_leg")[3], p.get("right_leg")[3]);
    p.xrot("left_leg", left_leg - kick);
    p.xrot("right_leg", right_leg + kick);
    p.xrot("head", 0.0);
}
/// `HumanoidModel.setupSwimAnimation`: the stroke of both arms over 26
/// walk-animation units, the head down and a slow flutter kick. The left arm
/// eases the short way round, the right one linearly, as in vanilla.
fn swim(p: &mut Poses, pos: f32, h: &Hum) {
    let amount = h.swim;
    if !h.fall_flying {
        let x = p.get("head")[3];
        p.xrot(
            "head",
            rot_lerp_rad(amount, x, -std::f32::consts::FRAC_PI_4),
        );
    }
    let swim_pos = pos % 26.0;
    let right_amount = if h.swing_right_arm == Some(true) {
        0.0
    } else {
        amount
    };
    let left_amount = if h.swing_right_arm == Some(false) {
        0.0
    } else {
        amount
    };
    let lerp = |a: f32, from: f32, to: f32| from + a * (to - from);
    let quadratic = |x: f32| -65.0 * x + x * x;
    if !h.using_item {
        // Targets (x, y, z) for the left and right arm.
        let targets = if swim_pos < 14.0 {
            let stroke = 1.870_796_4 * quadratic(swim_pos) / quadratic(14.0);
            Some(([0.0, PI, PI + stroke], [0.0, PI, PI - stroke]))
        } else if swim_pos < 22.0 {
            let t = (swim_pos - 14.0) / 8.0;
            let x = PI / 2.0 * t;
            Some((
                [x, PI, 5.012_389 - 1.870_796_4 * t],
                [x, PI, 1.270_796_3 + 1.870_796_4 * t],
            ))
        } else if swim_pos < 26.0 {
            let t = (swim_pos - 22.0) / 4.0;
            let x = PI / 2.0 - PI / 2.0 * t;
            Some(([x, PI, PI], [x, PI, PI]))
        } else {
            None
        };
        if let Some((left, right)) = targets {
            for (i, target) in left.into_iter().enumerate() {
                let angle = &mut p.get("left_arm")[3 + i];
                *angle = rot_lerp_rad(left_amount, *angle, target);
            }
            for (i, target) in right.into_iter().enumerate() {
                let angle = &mut p.get("right_arm")[3 + i];
                *angle = lerp(right_amount, *angle, target);
            }
        }
    }
    let x = p.get("left_leg")[3];
    p.xrot(
        "left_leg",
        lerp(amount, x, 0.3 * (pos * 0.333_333_34 + PI).mcos()),
    );
    let x = p.get("right_leg")[3];
    p.xrot(
        "right_leg",
        lerp(amount, x, 0.3 * (pos * 0.333_333_34).mcos()),
    );
}
/// Apply the mob's model animation to `p`.
pub fn setup(
    m: &Mob,
    s: &State,
    p: &mut Poses,
    w_anims: &HashMap<String, super::keyframes::Animation>,
) {
    if super::body_anim::anim(m, s, p, w_anims) {
        return;
    }
    match m.kind.anim {
        Anim::Camel => super::camel::anim(m, s, p, w_anims),
        Anim::Breeze => super::breeze::anim(m, s, p, w_anims),
        Anim::Sniffer => super::sniffer::anim(m, s, p, w_anims),
        Anim::CopperGolem => super::copper_golem::anim(m, s, p, w_anims),
        Anim::HappyGhast => super::happy_ghast::anim(m, s, p),
        Anim::Nautilus => super::nautilus::anim(m, s, p, w_anims),
        // body_anim::llama runs first.
        Anim::Llama => quadruped(p, s),
        Anim::Warden => super::warden::anim(m, s, p, w_anims),
        Anim::Allay => super::allay::anim(m, s, p),
        Anim::Creaking => super::creaking::anim(m, s, p, w_anims),
        Anim::Squid | Anim::Fish | Anim::Pufferfish | Anim::Dolphin => {
            super::aquatic::anim(m, s, p)
        }
        Anim::Quadruped | Anim::PolarBear | Anim::Panda | Anim::Goat => quadruped(p, s),
        Anim::Sheep => {
            quadruped(p, s);
            // SheepModel: lower the head while eating grass.
            let t = m.extra.anim_ticks;
            let partial = s.partial;
            let pos_scale = if t <= 0 {
                0.0
            } else if (4..=36).contains(&t) {
                1.0
            } else if t < 4 {
                (t as f32 - partial) / 4.0
            } else {
                -((t - 40) as f32 - partial) / 4.0
            };
            let angle = if t > 4 && t <= 36 {
                let f = ((t - 4) as f32 - partial) / 32.0;
                PI / 5.0 + 0.219_911_49 * (f * 28.7).msin()
            } else if t > 0 {
                PI / 5.0
            } else {
                s.x_rot.to_radians()
            };
            let age_scale = if s.baby { 0.5 } else { 1.0 };
            p.get("head")[1] += pos_scale * 9.0 * age_scale;
            p.xrot("head", angle);
        }
        Anim::Chicken => {
            let flap = m.anim.flap_o + (m.anim.flap - m.anim.flap_o) * s.partial;
            let flap_speed =
                m.anim.flap_speed_o + (m.anim.flap_speed - m.anim.flap_speed_o) * s.partial;
            let angle = (flap.msin() + 1.0) * flap_speed;
            p.xrot("right_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
            p.xrot("left_leg", (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed);
            p.zrot("right_wing", angle);
            p.zrot("left_wing", -angle);
            if !s.baby {
                head(p, s);
            }
        }
        Anim::Zombie
        | Anim::Drowned
        | Anim::ZombieVillager
        | Anim::ZombifiedPiglin
        | Anim::Humanoid { zombie_arms: true } => {
            // DrownedRenderer.getArmPose: an aggressive drowned raises the
            // trident in its main arm.
            let throwing = m.kind.anim == Anim::Drowned
                && m.ai.aggressive
                && m.extra.main_hand.as_deref() == Some("minecraft:trident");
            let main = if throwing {
                ArmPose::ThrowTrident
            } else {
                ArmPose::Empty
            };
            let mut h = if m.main_arm_left() {
                Hum::new(ArmPose::Empty, main)
            } else {
                Hum::new(main, ArmPose::Empty)
            };
            h.left_main = m.main_arm_left();
            h.zombie = Some(m.ai.aggressive);
            h.holding = m.extra.main_hand.is_some();
            h.crouching = m.extra.crouching;
            h.passenger = m.extra.passenger;
            humanoid(p, s, &h);
            if m.kind.anim == Anim::Drowned {
                drowned_swim(p, s, m.extra.swim.amount(s.partial));
            }
            if m.kind.anim == Anim::ZombifiedPiglin {
                super::piglin::ears(p, s);
            }
        }
        Anim::Skeleton | Anim::Humanoid { zombie_arms: false } => {
            let bow = m.extra.main_hand.as_deref() == Some("minecraft:bow");
            let player = matches!(m.kind.anim, Anim::Humanoid { .. });
            let right = if bow && m.ai.aggressive {
                ArmPose::Bow
            } else if player && m.extra.main_hand.is_some() {
                ArmPose::Item
            } else {
                ArmPose::Empty
            };
            let using_main = m.extra.using_item > 0 && !m.extra.using_off_hand;
            // AvatarRenderer.getArmPose: an arm holding a spear (at rest too).
            let spear_in = |hand: &Option<std::sync::Arc<str>>| {
                hand.as_deref()
                    .filter(|_| player)
                    .and_then(crate::spear::Kinetic::of)
            };
            let (main_spear, off_spear) =
                (spear_in(&m.extra.main_hand), spear_in(&m.extra.off_hand));
            let pose = if player {
                match m.extra.main_hand.as_deref() {
                    Some(_) if using_main && m.extra.blocking => ArmPose::Block,
                    Some("minecraft:bow") if using_main => ArmPose::Bow,
                    Some("minecraft:trident") if using_main => ArmPose::ThrowTrident,
                    Some("minecraft:crossbow") if using_main && !m.extra.charged => {
                        ArmPose::CrossbowCharge
                    }
                    Some("minecraft:crossbow") if m.extra.charged && s.swing == 0.0 => {
                        ArmPose::CrossbowHold
                    }
                    Some(_) if main_spear.is_some() => ArmPose::Spear,
                    Some(_) => ArmPose::Item,
                    None => ArmPose::Empty,
                }
            } else {
                right
            };
            let off = if player && m.extra.off_hand.is_some() && !pose.affects_offhand() {
                if off_spear.is_some() {
                    ArmPose::Spear
                } else if m.extra.using_item > 0 && m.extra.using_off_hand && m.extra.blocking {
                    ArmPose::Block
                } else {
                    ArmPose::Item
                }
            } else {
                ArmPose::Empty
            };
            let mut h = if m.main_arm_left() {
                Hum::new(off, pose)
            } else {
                Hum::new(pose, off)
            };
            h.charge = (m.extra.ints[5] as f32, m.extra.using_item as f32);
            h.fall_flying = m.extra.fall_flying;
            if h.fall_flying {
                let v = m.velocity;
                h.fly_speed_scale =
                    (((v.x * v.x + v.y * v.y + v.z * v.z) as f32 / 0.2).powi(3)).max(1.0);
            }
            h.crouching = m.extra.crouching;
            h.passenger = m.extra.passenger;
            h.swim = m.extra.swim.amount(s.partial);
            h.using_item = m.extra.using_item > 0;
            h.swing_right_arm = m.extra.swing_right_arm;
            h.stab = m.extra.swing_stab;
            h.left_main = m.main_arm_left();
            let main_side = usize::from(m.main_arm_left());
            h.spear[main_side] = main_spear;
            h.spear[1 - main_side] = off_spear;
            if m.extra.using_item > 0 {
                let using_side = if m.extra.using_off_hand {
                    1 - main_side
                } else {
                    main_side
                };
                h.spear_ticks[using_side] = m.extra.using_item as f32 + s.partial;
            }
            humanoid(p, s, &h);
            if player {
                // PlayerModel.setupAnim: the layers the player hid (PlayerModelPart).
                let hidden = m.extra.hidden_model_parts;
                for (bit, part) in [
                    (1, "jacket"),
                    (2, "left_sleeve"),
                    (3, "right_sleeve"),
                    (4, "left_pants"),
                    (5, "right_pants"),
                    (6, "hat"),
                ] {
                    p.visible(part, hidden & (1 << bit) == 0);
                }
            }
            if m.ai.aggressive && !bow {
                // SkeletonModel melee pose.
                let sw = s.swing;
                let s2 = (sw * PI).msin();
                let s1 = ((1.0 - (1.0 - sw) * (1.0 - sw)) * PI).msin();
                p.zrot("right_arm", 0.0);
                p.zrot("left_arm", 0.0);
                p.yrot("right_arm", -(0.1 - s2 * 0.6));
                p.yrot("left_arm", 0.1 - s2 * 0.6);
                p.xrot("right_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                p.xrot("left_arm", -PI / 2.0 - (s2 * 1.2 - s1 * 0.4));
                bob(p, "right_arm", s.age, 1.0);
                bob(p, "left_arm", s.age, -1.0);
            }
        }
        Anim::Witch => super::witch::anim(m, s, p),
        Anim::Villager => super::villager::anim(m, s, p),
        Anim::Wolf => super::tame::anim(m, s, p),
        Anim::IronGolem => super::golem::iron_anim(m, s, p),
        Anim::Illager => super::illager::anim(m, s, p),
        Anim::Piglin => super::piglin::piglin_anim(m, s, p),
        Anim::Phantom => super::phantom::anim(m, s, p),
        Anim::Guardian => super::guardian::anim(m, s, p),
        Anim::Vex => super::vex::anim(m, s, p),
        Anim::Wither => super::wither::anim(m, s, p),
        Anim::Ravager => super::ravager::anim(m, s, p),
        Anim::Shulker => super::shulker::anim(m, s, p),
        Anim::Horse => {
            super::equine::anim(m, s, p);
            // DonkeyModel: the chests show once fitted.
            if !m.extra.chested {
                for part in ["left_chest", "right_chest"] {
                    p.visible(part, false);
                }
            }
        }
        Anim::Hoglin => super::piglin::hoglin_anim(m, s, p),
        Anim::Blaze => super::nether::blaze_anim(s, p),
        Anim::Ghast => super::nether::ghast_anim(s, p),
        Anim::Feline => super::tame::cat_anim(m, s, p),
        Anim::Enderman => super::enderman::anim(m, s, p),
        Anim::Slime | Anim::MagmaCube => super::cube::anim(m, s, p),
        Anim::Creeper => {
            head(p, s);
            // CreeperModel binds its fields to the opposite-side parts, so the
            // left formulas drive the right parts and the reverse.
            p.xrot("left_hind_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
            p.xrot(
                "right_hind_leg",
                (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed,
            );
            p.xrot(
                "left_front_leg",
                (s.pos * 0.6662 + PI).mcos() * 1.4 * s.speed,
            );
            p.xrot("right_front_leg", (s.pos * 0.6662).mcos() * 1.4 * s.speed);
        }
        Anim::Spider => {
            head(p, s);
            let pos = s.pos * 0.6662;
            let sp = s.speed;
            let swing = |o: f32| -((pos * 2.0 + o).mcos() * 0.4) * sp;
            let step = |o: f32| ((pos + o).msin() * 0.4).abs() * sp;
            let legs = [
                ("hind", 0.0f32, 0.0f32),
                ("middle_hind", PI, PI),
                ("middle_front", PI / 2.0, PI / 2.0),
                ("front", PI * 3.0 / 2.0, PI * 3.0 / 2.0),
            ];
            for (n, so, to) in legs {
                let (sw, st) = (swing(so), step(to));
                p.get(&format!("right_{n}_leg"))[4] += sw;
                p.get(&format!("left_{n}_leg"))[4] -= sw;
                p.get(&format!("right_{n}_leg"))[5] += st;
                p.get(&format!("left_{n}_leg"))[5] -= st;
            }
        }
        _ => {
            if p.has("head") {
                head(p, s);
            }
            if p.has("right_hind_leg") {
                quadruped(p, s);
            }
        }
    }
}

#[cfg(test)]
mod flight_pose_tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    #[test]
    fn blocking_arm_follows_the_head_with_limits_in_both_hands() {
        for right in [false, true] {
            for (pitch, yaw, expected_pitch, expected_yaw) in [
                (0.0, 0.0, -0.742_477_9, 0.0),
                (2.0, 2.0, -0.306_145_58, FRAC_PI_6),
                (-2.0, -2.0, -2.138_741_3, -FRAC_PI_6),
            ] {
                let mut p = Poses {
                    map: ["head", "right_arm", "left_arm"]
                        .into_iter()
                        .map(|n| (n.to_string(), [0.0; 9]))
                        .collect(),
                    dummy: [0.0; 9],
                    skipped: Default::default(),
                    hidden: Default::default(),
                };
                let arm = if right { "right_arm" } else { "left_arm" };
                p.xrot("head", pitch);
                p.yrot("head", yaw);
                p.xrot(arm, 0.4);
                pose_arm(
                    &mut p,
                    ArmPose::Block,
                    right,
                    &Hum::new(ArmPose::Empty, ArmPose::Empty),
                );
                let pose = *p.get(arm);
                assert!((pose[3] - expected_pitch).abs() < 1e-6);
                let side = if right { -FRAC_PI_6 } else { FRAC_PI_6 };
                assert!((pose[4] - (side + expected_yaw)).abs() < 1e-6);
                assert_eq!(p.get(if right { "left_arm" } else { "right_arm" })[3], 0.0);
            }
        }
    }
    #[test]
    fn flight_slows_limbs_and_sets_the_head_pose() {
        let state = State {
            pos: 0.0,
            speed: 1.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
        };
        let make = || Poses {
            map: [
                "head",
                "right_arm",
                "left_arm",
                "right_leg",
                "left_leg",
                "body",
            ]
            .into_iter()
            .map(|n| (n.to_string(), [0.0; 9]))
            .collect(),
            dummy: [0.0; 9],
            skipped: Default::default(),
            hidden: Default::default(),
        };
        let mut ground = make();
        let mut flight = make();
        let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
        humanoid(&mut ground, &state, &h);
        h.fall_flying = true;
        h.fly_speed_scale = 8.0;
        humanoid(&mut flight, &state, &h);
        assert_eq!(flight.get("head")[3], -std::f32::consts::FRAC_PI_4);
        assert_eq!(flight.get("right_leg")[3], ground.get("right_leg")[3] / 8.0);
        // THROW_TRIDENT: the arm goes up over the head, and zombie arms leave it there.
        let mut throw = make();
        let mut h = Hum::new(ArmPose::ThrowTrident, ArmPose::Item);
        h.zombie = Some(true);
        humanoid(&mut throw, &state, &h);
        let still = (state.pos * 0.6662 + PI).mcos() * 2.0 * 0.5;
        assert_eq!(throw.get("right_arm")[3], still * 0.5 - PI);
        assert_eq!(throw.get("right_arm")[4], 0.0);
        assert_eq!(throw.get("left_arm")[3], -PI / 1.5);
    }

    #[test]
    fn swimming_strokes_the_arms_and_kicks() {
        let state = State {
            pos: 7.0,
            speed: 1.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 30.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
        };
        let make = || Poses {
            map: ["head", "right_arm", "left_arm", "right_leg", "left_leg"]
                .into_iter()
                .map(|n| (n.to_string(), [0.0; 9]))
                .collect(),
            dummy: [0.0; 9],
            skipped: Default::default(),
            hidden: Default::default(),
        };
        let same_angle = |a: f32, b: f32| ((a - b + PI).rem_euclid(PI * 2.0) - PI).abs() < 1e-5;
        let mut h = Hum::new(ArmPose::Empty, ArmPose::Empty);
        h.swim = 1.0;
        let mut p = make();
        humanoid(&mut p, &state, &h);
        // Mid-stroke: both arms sweep out from overhead by the quadratic.
        let stroke = 1.870_796_4 * (-65.0 * 7.0 + 49.0) / (-65.0 * 14.0 + 196.0);
        assert_eq!(p.get("right_arm")[5], PI - stroke);
        assert!(same_angle(p.get("left_arm")[5], PI + stroke));
        assert_eq!(p.get("right_arm")[4], PI);
        assert!(same_angle(p.get("head")[3], -std::f32::consts::FRAC_PI_4));
        assert!(same_angle(
            p.get("right_leg")[3],
            0.3 * (7.0f32 * 0.333_333_34).mcos()
        ));
        // Using an item keeps the arms; a swing frees its own arm only.
        let mut using = make();
        h.using_item = true;
        humanoid(&mut using, &state, &h);
        assert_eq!(using.get("right_arm")[4], 0.0);
        h.using_item = false;
        h.swing_right_arm = Some(true);
        let mut swinging = make();
        humanoid(
            &mut swinging,
            &State {
                swing: 0.5,
                ..state
            },
            &h,
        );
        assert!(swinging.get("right_arm")[4] < 1.0);
        assert!(same_angle(swinging.get("left_arm")[4], PI));
        // A STAB swing turns that arm's pitch alone, by the spear's curve.
        let mut stabbing = make();
        h.swim = 0.0;
        h.stab = true;
        let mut still = make();
        humanoid(
            &mut still,
            &state,
            &Hum::new(ArmPose::Empty, ArmPose::Empty),
        );
        let stab = State {
            swing: 0.2,
            ..state
        };
        humanoid(&mut stabbing, &stab, &h);
        let arm_pitch = crate::spear::third_person_attack_arm(0.2);
        assert!(same_angle(
            stabbing.get("right_arm")[3],
            still.get("right_arm")[3] + arm_pitch
        ));
        assert_eq!(stabbing.get("left_arm")[3], still.get("left_arm")[3]);
        // Out of the pose nothing changes.
        let mut dry = make();
        humanoid(&mut dry, &state, &Hum::new(ArmPose::Empty, ArmPose::Empty));
        assert_eq!(dry.get("head")[3], 30.0f32.to_radians());
    }

    /// A player: a humanoid model with a shield in its main hand.
    fn player() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.anim = Anim::Humanoid { zombie_arms: false };
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    #[test]
    fn a_raised_shield_holds_the_arm_across_the_body() {
        // AvatarRenderer.getArmPose: the used item's use animation decides
        // the arm's pose, and a shield (an item with `blocks_attacks`) uses
        // the BLOCK animation, so the arm is held up across the body
        // (#2289).
        let state = State {
            pos: 0.0,
            speed: 0.0,
            age: 0.0,
            y_rot: 0.0,
            x_rot: 0.0,
            swing: 0.0,
            baby: false,
            partial: 0.0,
        };
        let mut p = Poses {
            map: ["head", "right_arm", "left_arm"]
                .into_iter()
                .map(|n| (n.to_string(), [0.0; 9]))
                .collect(),
            dummy: [0.0; 9],
            skipped: Default::default(),
            hidden: Default::default(),
        };
        let mut m = player();
        m.extra.main_hand = Some("minecraft:shield".into());
        m.extra.using_item = 20;
        m.extra.blocking = true;
        setup(&m, &state, &mut p, &HashMap::new());
        // HumanoidModel.setupAnim's BLOCK pose.
        assert!((p.get("right_arm")[3] + 0.942_477_9).abs() < 1e-6);
        assert!((p.get("right_arm")[4] + FRAC_PI_6).abs() < 1e-6);
        // An item that does not block attacks leaves the arm holding it.
        let mut m = player();
        m.extra.main_hand = Some("minecraft:stick".into());
        m.extra.using_item = 20;
        let mut p = Poses {
            map: ["head", "right_arm", "left_arm"]
                .into_iter()
                .map(|n| (n.to_string(), [0.0; 9]))
                .collect(),
            dummy: [0.0; 9],
            skipped: Default::default(),
            hidden: Default::default(),
        };
        setup(&m, &state, &mut p, &HashMap::new());
        assert!((p.get("right_arm")[3] + PI / 10.0).abs() < 1e-6);
    }
}
