//! The local player's hand: the arm swing (`LivingEntity.SwingState`,
//! `getModifiedSwingDuration`), the first-person equip heights
//! (`FirstPersonHandsAndItems.tick` and `itemUsed`), and the click handling
//! that starts swings: `Minecraft.startAttack`, `continueAttack`,
//! `startUseItem` and `handleKeybinds`' attack and use part, with
//! `MultiPlayerGameMode`'s block breaking (`startDestroyBlock`,
//! `continueDestroyBlock`, `stopDestroyBlock`).
//!
//! What those read and do in the world is behind [`HandWorld`].

/// The ranged item state read by the first-person hand renderer.
#[derive(Clone, Copy, Debug)]
pub enum ItemPose {
    Swing,
    /// A STAB swing (`SwingAnimationType` 2): a spear's jab.
    Stab,
    Using,
    Eat {
        remaining: f32,
        duration: f32,
        partial: f32,
    },
    Trident {
        ticks_used: f32,
    },
    /// Riptide's spin (`isAutoSpinAttack`): the item held up like a fork.
    SpinAttack,
    Bow {
        ticks_used: f32,
    },
    /// A spear being charged (`ItemUseAnimation.SPEAR`): the time held and
    /// the ticks since its last hit's feedback.
    Spear {
        kinetic: crate::spear::Kinetic,
        time_held: f32,
        ticks_since_hit: f32,
    },
    Crossbow {
        ticks_used: Option<f32>,
        charge_duration: f32,
        charged: bool,
    },
}

/// `submitArmWithItem`: the pose of an item in the hand on the `left` or
/// right arm, before the model transform. Only the main hand's charged
/// crossbow is turned aside.
pub fn item_pose(left: bool, inverse_height: f32, attack: f32, pose: ItemPose) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    use horizon_core::geometry::trig::Mth;
    let invert = if left { -1.0 } else { 1.0 };
    let pi = std::f32::consts::PI;
    let mut m = Mat4::from_translation(Vec3::new(
        invert * 0.56,
        -0.52 - inverse_height * 0.6,
        -0.72,
    ));
    if let ItemPose::Eat {
        remaining,
        duration,
        partial,
    } = pose
    {
        let time = remaining - partial + 1.0;
        let fraction = time / duration;
        let mut eat = Mat4::IDENTITY;
        if fraction < 0.8 {
            eat *=
                Mat4::from_translation(Vec3::new(0.0, ((time / 4.0 * pi).mcos() * 0.1).abs(), 0.0));
        }
        let jiggle = 1.0 - horizon_core::jmath::pow(f64::from(fraction), 27.0) as f32;
        eat *= Mat4::from_translation(Vec3::new(jiggle * 0.6 * invert, jiggle * -0.5, 0.0));
        eat *= Mat4::from_rotation_y((invert * jiggle * 90.0).to_radians());
        eat *= Mat4::from_rotation_x((jiggle * 10.0).to_radians());
        eat *= Mat4::from_rotation_z((invert * jiggle * 30.0).to_radians());
        return eat * m;
    }
    if matches!(pose, ItemPose::Using) {
        return m;
    }
    // The spear's own place in the hand, without the equip height.
    if let ItemPose::Spear {
        kinetic,
        time_held,
        ticks_since_hit,
    } = pose
    {
        return Mat4::from_translation(Vec3::new(invert * 0.56, -0.52, -0.72))
            * crate::spear::first_person_use(&kinetic, ticks_since_hit, time_held, !left);
    }
    if matches!(pose, ItemPose::Stab) {
        return m * crate::spear::first_person_attack(attack, !left);
    }
    if matches!(pose, ItemPose::SpinAttack) {
        return m
            * Mat4::from_translation(Vec3::new(invert * -0.4, 0.8, 0.3))
            * Mat4::from_rotation_y((invert * 65.0f32).to_radians())
            * Mat4::from_rotation_z((invert * -85.0f32).to_radians());
    }
    let charging = match pose {
        ItemPose::Bow { ticks_used } => {
            let power = ticks_used / 20.0;
            Some((
                ticks_used,
                ((power * power + power * 2.0) / 3.0).min(1.0),
                false,
            ))
        }
        ItemPose::Crossbow {
            ticks_used: Some(ticks),
            charge_duration,
            charged: false,
        } => Some((ticks, (ticks / charge_duration).min(1.0), true)),
        ItemPose::Trident { ticks_used } => Some((ticks_used, (ticks_used / 10.0).min(1.0), false)),
        _ => None,
    };
    if let Some((ticks, power, crossbow)) = charging {
        let trident = matches!(pose, ItemPose::Trident { .. });
        let (x, y, z, pitch, yaw) = if trident {
            (-0.5, 0.7, 0.1, -55.0f32, 35.3f32)
        } else if crossbow {
            (-0.4785682, -0.094387, 0.05731531, -11.935f32, 65.3f32)
        } else {
            (-0.2785682, 0.18344387, 0.15731531, -13.935f32, 35.3f32)
        };
        m *= Mat4::from_translation(Vec3::new(invert * x, y, z));
        m *= Mat4::from_rotation_x(pitch.to_radians());
        m *= Mat4::from_rotation_y((invert * yaw).to_radians());
        m *= Mat4::from_rotation_z((invert * -9.785f32).to_radians());
        if power > 0.1 {
            let shake = ((ticks - 0.1) * 1.3).msin() * (power - 0.1);
            m *= Mat4::from_translation(Vec3::new(0.0, shake * 0.004, 0.0));
        }
        m *= Mat4::from_translation(Vec3::new(
            0.0,
            0.0,
            power * if trident { 0.2 } else { 0.04 },
        ));
        m *= Mat4::from_scale(Vec3::new(1.0, 1.0, 1.0 + power * 0.2));
        m *= Mat4::from_rotation_y((invert * -45.0f32).to_radians());
    } else {
        let root = attack.sqrt();
        m *= Mat4::from_translation(Vec3::new(
            invert * -0.4 * (root * pi).msin(),
            0.2 * (root * std::f32::consts::TAU).msin(),
            -0.2 * (attack * pi).msin(),
        ));
        m *= Mat4::from_rotation_y(
            (invert * (45.0 - (attack * attack * pi).msin() * 20.0)).to_radians(),
        );
        m *= Mat4::from_rotation_z((invert * (root * pi).msin() * -20.0).to_radians());
        m *= Mat4::from_rotation_x(((root * pi).msin() * -80.0).to_radians());
        m *= Mat4::from_rotation_y((invert * -45.0f32).to_radians());
        if matches!(pose, ItemPose::Crossbow { charged: true, .. }) && attack < 0.001 {
            m *= Mat4::from_translation(Vec3::new(invert * -0.641864, 0.0, 0.0));
            m *= Mat4::from_rotation_y((invert * 10.0f32).to_radians());
        }
    }
    m
}

/// How `submitArmWithItem` holds a stack with a `minecraft:map_id`: in both
/// hands when it is the main hand's and the off hand is empty
/// (`renderTwoHandedMap`), else in its own hand (`renderOneHandedMap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapHold {
    TwoHanded,
    OneHanded,
}
impl MapHold {
    pub fn of(hand: Hand, off_hand_empty: bool) -> Self {
        if hand == Hand::Main && off_hand_empty {
            Self::TwoHanded
        } else {
            Self::OneHanded
        }
    }
}

/// `renderPlayerArm` up to `renderPlayerHand`: the pose of the bare `left`
/// or right arm.
pub fn player_arm_pose(left: bool, inverse_height: f32, attack: f32) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    use horizon_core::geometry::trig::Mth;
    let f = if left { -1.0 } else { 1.0 };
    let pi = std::f32::consts::PI;
    let root = attack.sqrt();
    let x = -0.3 * (root * pi).msin();
    let y = 0.4 * (root * std::f32::consts::TAU).msin();
    let z = -0.4 * (attack * pi).msin();
    let z_rot = (attack * attack * pi).msin();
    let y_rot = (root * pi).msin();
    let deg = |d: f32| d.to_radians();
    Mat4::from_translation(Vec3::new(
        f * (x + 0.640_000_05),
        y - 0.6 + inverse_height * -0.6,
        z + -0.719_999_97,
    )) * Mat4::from_rotation_y(deg(f * 45.0))
        * Mat4::from_rotation_y(deg(f * y_rot * 70.0))
        * Mat4::from_rotation_z(deg(f * z_rot * -20.0))
        * Mat4::from_translation(Vec3::new(-f, 3.6, 3.5))
        * Mat4::from_rotation_z(deg(f * 120.0))
        * Mat4::from_rotation_x(deg(200.0))
        * Mat4::from_rotation_y(deg(f * -135.0))
        * Mat4::from_translation(Vec3::new(f * 5.6, 0.0, 0.0))
}

/// `renderTwoHandedMap` up to its arms: the swing, the tilt from the view
/// pitch (`calculateMapTilt`) and the turn towards the player.
pub fn two_handed_map_base(pitch: f32, inverse_height: f32, attack: f32) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    use horizon_core::geometry::trig::Mth;
    let pi = std::f32::consts::PI;
    let y_swing = -0.2 * (attack * pi).msin();
    let z_swing = -0.4 * (attack.sqrt() * pi).msin();
    let tilt = (1.0 - pitch / 45.0 + 0.1).clamp(0.0, 1.0);
    let tilt = -(tilt * pi).mcos() * 0.5 + 0.5;
    Mat4::from_translation(Vec3::new(0.0, -y_swing / 2.0, z_swing))
        * Mat4::from_translation(Vec3::new(
            0.0,
            0.04 + inverse_height * -1.2 + tilt * -0.5,
            -0.72,
        ))
        * Mat4::from_rotation_x((tilt * -85.0).to_radians())
}

/// The arms under a held map, as (left arm, pose before `renderPlayerHand`):
/// `renderOneHandedMap`'s arm turned 10 degrees outwards, or
/// `renderTwoHandedMap`'s two `renderMapHand` arms under the map's
/// `two_handed_map_base`. Vanilla skips them for an invisible player.
pub fn map_arm_poses(
    hold: MapHold,
    left: bool,
    pitch: f32,
    inverse_height: f32,
    attack: f32,
) -> Vec<(bool, glam::Mat4)> {
    use glam::{Mat4, Vec3};
    let deg = |d: f32| d.to_radians();
    match hold {
        MapHold::OneHanded => {
            let f = if left { -1.0 } else { 1.0 };
            let m = Mat4::from_translation(Vec3::new(f * 0.125, -0.125, 0.0))
                * Mat4::from_rotation_z(deg(f * 10.0))
                * player_arm_pose(left, inverse_height, attack);
            vec![(left, m)]
        }
        MapHold::TwoHanded => {
            let base = two_handed_map_base(pitch, inverse_height, attack)
                * Mat4::from_rotation_y(deg(90.0));
            [false, true]
                .into_iter()
                .map(|left| {
                    let f = if left { -1.0 } else { 1.0 };
                    let hand = Mat4::from_rotation_y(deg(92.0))
                        * Mat4::from_rotation_x(deg(45.0))
                        * Mat4::from_rotation_z(deg(f * -41.0))
                        * Mat4::from_translation(Vec3::new(f * 0.3, -1.1, 0.45));
                    (left, base * hand)
                })
                .collect()
        }
    }
}

/// `renderOneHandedMap` up to `renderMap`: the pose of a map in the hand on
/// the `left` or right arm (its arm: [`map_arm_poses`]).
pub fn one_handed_map_pose(left: bool, inverse_height: f32, attack: f32) -> glam::Mat4 {
    use glam::{Mat4, Vec3};
    use horizon_core::geometry::trig::Mth;
    let invert = if left { -1.0 } else { 1.0 };
    let pi = std::f32::consts::PI;
    let root = attack.sqrt();
    let x_swing = (root * pi).msin();
    Mat4::from_translation(Vec3::new(invert * 0.125, -0.125, 0.0))
        * Mat4::from_translation(Vec3::new(
            invert * 0.51,
            -0.08 + inverse_height * -1.2,
            -0.75,
        ))
        * Mat4::from_translation(Vec3::new(
            invert * -0.5 * x_swing,
            0.4 * (root * std::f32::consts::TAU).msin() - 0.3 * x_swing,
            -0.3 * (attack * pi).msin(),
        ))
        * Mat4::from_rotation_x((x_swing * -45.0).to_radians())
        * Mat4::from_rotation_y((invert * x_swing * -30.0).to_radians())
}

/// `FirstPersonHandsAndItems.evaluateWhichHandsToRender`: whether the main
/// and the off hand show, from what each holds (item and whether it is a
/// charged crossbow) and the item in use with its hand. A bow or crossbow
/// being drawn hides the other hand, and a charged crossbow in the main hand
/// hides the off hand unless the off hand is the one in use.
pub fn hands_to_render(held: [Option<(&str, bool)>; 2], using: Option<(Hand, &str)>) -> [bool; 2] {
    let bow_like = |id: &str| matches!(id, "minecraft:bow" | "minecraft:crossbow");
    let charged_crossbow =
        |h: Option<(&str, bool)>| h.is_some_and(|(id, c)| c && id == "minecraft:crossbow");
    if !held.iter().flatten().any(|&(id, _)| bow_like(id)) {
        return [true, true];
    }
    match using {
        None => [true, !charged_crossbow(held[0])],
        Some((hand, id)) if bow_like(id) => [hand == Hand::Main, hand == Hand::Off],
        Some((hand, _)) => [true, !(hand == Hand::Main && charged_crossbow(held[1]))],
    }
}

/// `InteractionHand`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
}
impl Hand {
    pub const BOTH: [Hand; 2] = [Hand::Main, Hand::Off];
}

/// `SwingAnimation`: its type (`SwingAnimationType` ordinal) and duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwingAnimation {
    pub kind: u8,
    pub duration: i32,
}
impl SwingAnimation {
    /// `SwingAnimation.DEFAULT`: WHACK for 6 ticks.
    pub const DEFAULT: SwingAnimation = SwingAnimation {
        kind: 1,
        duration: 6,
    };
}

/// `LivingEntity.SwingDescription`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwingDescription {
    pub hand: Hand,
    pub animation: SwingAnimation,
    pub duration_ticks: i32,
}

/// `LivingEntity.SwingState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SwingState {
    pub current: Option<SwingDescription>,
    pub ticks: i32,
    pub old_animation: f32,
    pub animation: f32,
}
impl SwingState {
    /// `startIfAble`: a swing restarts unless the current one is in its
    /// first half (past its first tick).
    pub fn start_if_able(&mut self, hand: Hand, animation: SwingAnimation, duration: i32) -> bool {
        if let Some(c) = self.current
            && self.ticks <= c.duration_ticks / 2
            && self.ticks > 0
        {
            return false;
        }
        let swing = SwingDescription {
            hand,
            animation,
            duration_ticks: duration,
        };
        if self.current != Some(swing) {
            self.old_animation = 0.0;
            self.animation = 0.0;
        }
        self.current = Some(swing);
        self.ticks = 0;
        true
    }
    /// `tick` (from `LivingEntity.baseTick`).
    pub fn tick(&mut self) {
        self.old_animation = self.animation;
        if let Some(c) = self.current {
            if c.duration_ticks > 0 {
                self.animation = (self.ticks as f32 / c.duration_ticks as f32).min(1.0);
            }
            let ticks = self.ticks;
            self.ticks += 1;
            if ticks > c.duration_ticks {
                self.current = None;
                self.animation = 0.0;
            }
        } else {
            self.animation = 0.0;
        }
    }
    /// `getAnimation(partialTicks)`: a restarted swing keeps moving forward.
    pub fn animation(&self, partial: f32) -> f32 {
        if self.animation >= self.old_animation {
            crate::camera::lerp(partial, self.old_animation, self.animation)
        } else {
            crate::camera::lerp(partial, self.old_animation, self.animation + 1.0)
        }
    }
    pub fn swinging(&self) -> bool {
        self.current.is_some()
    }
    /// The hand of the current swing (`state.attackHand`), main by default.
    pub fn attack_hand(&self) -> Hand {
        self.current.map_or(Hand::Main, |c| c.hand)
    }
}

/// `LivingEntity.getModifiedSwingDuration`: `dig_speed` is
/// `MobEffectUtil.getDigSpeedAmplification` while haste or conduit power is
/// active, `fatigue` the mining fatigue amplifier.
pub fn swing_duration(
    animation: SwingAnimation,
    dig_speed: Option<i32>,
    fatigue: Option<i32>,
) -> i32 {
    let duration = animation.duration;
    if let Some(amplification) = dig_speed {
        duration - (1 + amplification)
    } else if let Some(amplifier) = fatigue {
        duration + (1 + amplifier) * 2
    } else {
        duration
    }
}

/// `FirstPersonHandsAndItems`' equip heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HandHeights {
    pub main: f32,
    pub old_main: f32,
    pub off: f32,
    pub old_off: f32,
}
impl Default for HandHeights {
    fn default() -> Self {
        HandHeights {
            main: 0.0,
            old_main: 0.0,
            off: 0.0,
            old_off: 0.0,
        }
    }
}
impl HandHeights {
    /// `FirstPersonHandsAndItems.tick`'s heights. `changed` says per hand
    /// whether the visible item differs from the held one once
    /// `shouldInstantlyReplaceVisibleItem` has run; `swap_scale` is
    /// `getItemSwapScale(1.0F)`. Returns whether each hand's visible item
    /// is replaced at the end.
    pub fn tick(&mut self, hands_busy: bool, swap_scale: f32, changed: [bool; 2]) -> [bool; 2] {
        self.old_main = self.main;
        self.old_off = self.off;
        if hands_busy {
            self.main = (self.main - 0.4).clamp(0.0, 1.0);
            self.off = (self.off - 0.4).clamp(0.0, 1.0);
        } else {
            let attack = swap_scale;
            let main_target = if changed[0] {
                0.0
            } else {
                attack * attack * attack
            };
            let off_target = if changed[1] { 0.0 } else { 1.0 };
            self.main += (main_target - self.main).clamp(-0.4, 0.4);
            self.off += (off_target - self.off).clamp(-0.4, 0.4);
        }
        [self.main < 0.1, self.off < 0.1]
    }
    /// `itemUsed`: the hand drops to the bottom and rises again.
    pub fn item_used(&mut self, hand: Hand) {
        match hand {
            Hand::Main => self.main = 0.0,
            Hand::Off => self.off = 0.0,
        }
    }
    /// `Mth.lerp(partialTicks, oMainHandHeight, mainHandHeight)` and the
    /// off hand's.
    pub fn interpolated(&self, partial: f32) -> [f32; 2] {
        [
            crate::camera::lerp(partial, self.old_main, self.main),
            crate::camera::lerp(partial, self.old_off, self.off),
        ]
    }
}

/// `LocalPlayer.xBob` and `yBob`: the hands lag behind the view when it
/// turns. `LocalPlayer.applyInput` eases them halfway to the rotation every
/// tick; `FirstPersonHandsAndItemsRenderer` turns the hands by a tenth of
/// the gap left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HandSway {
    pub x_bob: f32,
    pub y_bob: f32,
    pub x_bob_o: f32,
    pub y_bob_o: f32,
}
impl HandSway {
    /// `LocalPlayer.applyInput` with the player's `getXRot` and `getYRot`.
    pub fn tick(&mut self, x_rot: f32, y_rot: f32) {
        self.y_bob_o = self.y_bob;
        self.x_bob_o = self.x_bob;
        self.x_bob += (x_rot - self.x_bob) * 0.5;
        self.y_bob += (y_rot - self.y_bob) * 0.5;
    }
    /// The degrees the hands turn about X, then Y, before either is drawn:
    /// `(viewXRot - xBob) * 0.1` and `(viewYRot - yBob) * 0.1`, the bobs
    /// interpolated as `extractRenderState` does.
    pub fn rotation(&self, partial: f32, view_x_rot: f32, view_y_rot: f32) -> [f32; 2] {
        let x_bob = crate::camera::lerp(partial, self.x_bob_o, self.x_bob);
        let y_bob = crate::camera::lerp(partial, self.y_bob_o, self.y_bob);
        [(view_x_rot - x_bob) * 0.1, (view_y_rot - y_bob) * 0.1]
    }
}

/// `HitResult` at the crosshair (`Minecraft.hitResult`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Miss,
    Block { pos: [i32; 3], face: u8 },
    Entity,
}

/// `InteractionResult` as `startUseItem` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseResult {
    /// `Success` with its `SwingSource`: whether the client swings.
    Success {
        swing: bool,
    },
    Pass,
    Fail,
}

impl UseResult {
    pub fn should_swing(self, before: u32, after: u32) -> bool {
        matches!(self, Self::Success { swing: true }) && before == after
    }
}
pub use horizon_items::use_animation::consumable_animation;
/// What the click handling reads and does in the world.
pub trait HandWorld {
    fn hit(&mut self) -> Hit;
    fn is_air(&mut self, pos: [i32; 3]) -> bool;
    /// `MultiPlayerGameMode.localPlayerMode`.
    fn creative(&self) -> bool;
    fn spectator(&self) -> bool;
    /// `Abilities.instabuild`.
    fn instabuild(&self) -> bool;
    fn hands_busy(&self) -> bool;
    fn using_item(&self) -> bool;
    /// `ItemStack.isItemEnabled` for the item in a hand.
    fn item_enabled(&mut self, hand: Hand) -> bool;
    fn held_empty(&mut self, hand: Hand) -> bool;
    fn held_count(&mut self, hand: Hand) -> i32;
    /// `Player.hasInfiniteMaterials`.
    fn infinite_materials(&self) -> bool;
    /// `Player.cannotAttackWithItem(heldItem, 0)`.
    fn cannot_attack(&mut self) -> bool;
    /// The main-hand item is a `PIERCING_WEAPON`.
    fn piercing_weapon(&mut self) -> bool;
    fn attack_animation(&mut self) -> SwingAnimation;
    fn interact_animation(&mut self, hand: Hand) -> SwingAnimation;
    /// `MultiPlayerGameMode.piercingAttack`.
    fn piercing_attack(&mut self, animation: SwingAnimation);
    /// `Player.attack` on the targeted entity when the item's attack range
    /// allows it; false when out of range.
    fn attack_entity(&mut self) -> bool;
    /// `Player.blockActionRestricted` for the game mode.
    fn block_action_restricted(&mut self, pos: [i32; 3]) -> bool;
    /// `WorldBorder.isWithinBounds`.
    fn within_border(&mut self, pos: [i32; 3]) -> bool;
    /// `BlockState.attack` on the first hit of a break.
    fn attack_block(&mut self, pos: [i32; 3]);
    /// `BlockState.getDestroyProgress` for the player.
    fn destroy_progress(&mut self, pos: [i32; 3]) -> f32;
    /// `MultiPlayerGameMode.destroyBlock`.
    fn destroy_block(&mut self, pos: [i32; 3]) -> bool;
    /// A key for the main-hand stack, kept as `destroyingItem`.
    fn main_hand_key(&mut self) -> String;
    /// `ItemStack.isSameItemSameComponents(selected, destroyingItem)` for
    /// the stack `main_hand_key` gave when the break started. Vanilla keeps
    /// a reference to that stack, so it compares what it holds now.
    fn same_destroying_item(&mut self, key: &str) -> bool {
        self.main_hand_key() == key
    }
    /// Breaking progress shown (`destroyBlockProgress`), or cleared with `None`.
    fn destroy_stage(&mut self, pos: [i32; 3], stage: Option<i32>);
    /// `player.swing(hand, animation, false)`.
    fn swing(&mut self, hand: Hand, animation: SwingAnimation);
    /// `Player.resetAttackStrengthTicker`.
    fn reset_attack_strength(&mut self);
    /// The targeted entity's block position is inside the world border.
    fn entity_within_border(&mut self) -> bool;
    /// The use attempts of `startUseItem`: on the targeted entity (`None`
    /// when it is out of interaction range), the targeted block, and the
    /// item alone.
    fn use_on_entity(&mut self, hand: Hand) -> Option<UseResult>;
    fn use_on_block(&mut self, hand: Hand) -> UseResult;
    fn use_item(&mut self, hand: Hand) -> UseResult;
    /// `LocalPlayer.itemUsed`.
    fn item_used(&mut self, hand: Hand);
    /// `MultiPlayerGameMode.releaseUsingItem`.
    fn release_using(&mut self);
}

/// `MultiPlayerGameMode`'s block breaking.
#[derive(Clone, Debug, PartialEq)]
pub struct Destroying {
    pub delay: i32,
    pub active: bool,
    pub pos: [i32; 3],
    pub face: u8,
    pub item: String,
    pub progress: f32,
    pub ticks: f32,
}
impl Default for Destroying {
    /// `destroyBlockPos` starts at (-1, -1, -1) and `destroyingItem` empty
    /// (an empty key).
    fn default() -> Self {
        Destroying {
            delay: 0,
            active: false,
            pos: [-1; 3],
            face: 0,
            item: String::new(),
            progress: 0.0,
            ticks: 0.0,
        }
    }
}
impl Destroying {
    /// `getDestroyStage`.
    pub fn stage(&self) -> i32 {
        if self.progress > 0.0 {
            (self.progress * 10.0) as i32
        } else {
            -1
        }
    }
    fn same_target(&self, w: &mut impl HandWorld, pos: [i32; 3]) -> bool {
        pos == self.pos && w.same_destroying_item(&self.item)
    }
    /// `startDestroyBlock`.
    pub fn start(&mut self, w: &mut impl HandWorld, pos: [i32; 3], face: u8) -> bool {
        if w.block_action_restricted(pos) || !w.within_border(pos) {
            return false;
        }
        if w.instabuild() {
            w.destroy_block(pos);
            self.delay = 5;
        } else if !self.active || !self.same_target(w, pos) {
            let not_air = !w.is_air(pos);
            if not_air && self.progress == 0.0 {
                w.attack_block(pos);
            }
            if not_air && w.destroy_progress(pos) >= 1.0 {
                w.destroy_block(pos);
            } else {
                self.active = true;
                self.pos = pos;
                self.face = face;
                self.item = w.main_hand_key();
                self.progress = 0.0;
                self.ticks = 0.0;
                w.destroy_stage(pos, Some(self.stage()));
            }
        }
        true
    }
    /// `stopDestroyBlock`.
    pub fn stop(&mut self, w: &mut impl HandWorld) {
        if self.active {
            self.active = false;
            self.progress = 0.0;
            w.destroy_stage(self.pos, None);
            w.reset_attack_strength();
        }
    }
    /// `continueDestroyBlock`: true while the hand keeps swinging.
    pub fn continue_destroy(&mut self, w: &mut impl HandWorld, pos: [i32; 3], face: u8) -> bool {
        if self.delay > 0 {
            self.delay -= 1;
            return true;
        }
        if w.instabuild() && w.within_border(pos) {
            self.delay = 5;
            w.destroy_block(pos);
            return true;
        }
        if !self.same_target(w, pos) {
            return self.start(w, pos, face);
        }
        if w.is_air(pos) {
            self.active = false;
            return false;
        }
        self.progress += w.destroy_progress(pos);
        self.ticks += 1.0;
        if self.progress >= 1.0 {
            self.active = false;
            w.destroy_block(pos);
            self.progress = 0.0;
            self.ticks = 0.0;
            self.delay = 5;
        } else if self.face != face {
            self.face = face;
        }
        w.destroy_stage(self.pos, Some(self.stage()));
        true
    }
}

/// `Minecraft`'s attack and use handling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clicks {
    pub miss_time: i32,
    pub right_click_delay: i32,
    pub destroying: Destroying,
}
impl Clicks {
    /// `Minecraft.startAttack`: whether the click ended the attack (broke
    /// the block outright).
    pub fn start_attack(&mut self, w: &mut impl HandWorld) -> bool {
        if self.miss_time > 0 || w.hands_busy() {
            return false;
        }
        if w.spectator() {
            return true;
        }
        if !w.item_enabled(Hand::Main) || w.cannot_attack() {
            return false;
        }
        let animation = w.attack_animation();
        if w.piercing_weapon() {
            w.piercing_attack(animation);
            return true;
        }
        let mut end = false;
        let mut miss = false;
        match w.hit() {
            // MultiPlayerGameMode.attack.
            Hit::Entity => {
                if w.attack_entity() {
                    w.reset_attack_strength();
                    if w.instabuild() {
                        self.destroying.delay = 5;
                    }
                }
            }
            Hit::Block { pos, face } if !w.is_air(pos) => {
                self.destroying.start(w, pos, face);
                end = w.is_air(pos);
            }
            Hit::Block { .. } | Hit::Miss => miss = true,
        }
        if miss {
            if !w.creative() {
                self.miss_time = 10;
            }
            w.reset_attack_strength();
        }
        w.swing(Hand::Main, animation);
        end
    }
    /// `Minecraft.continueAttack`.
    pub fn continue_attack(&mut self, w: &mut impl HandWorld, down: bool) {
        if !down {
            self.miss_time = 0;
        }
        if self.miss_time > 0 || w.using_item() || w.piercing_weapon() {
            return;
        }
        let animation = w.attack_animation();
        match w.hit() {
            Hit::Block { pos, face } if down => {
                if !w.is_air(pos) && self.destroying.continue_destroy(w, pos, face) {
                    w.swing(Hand::Main, animation);
                }
            }
            _ => self.destroying.stop(w),
        }
    }
    /// `Minecraft.startUseItem`.
    pub fn start_use_item(&mut self, w: &mut impl HandWorld) {
        if self.destroying.active {
            return;
        }
        self.right_click_delay = 4;
        if w.hands_busy() {
            return;
        }
        for hand in Hand::BOTH {
            if !w.item_enabled(hand) {
                return;
            }
            let animation = w.interact_animation(hand);
            match w.hit() {
                Hit::Entity => {
                    if !w.entity_within_border() {
                        return;
                    }
                    if let Some(UseResult::Success { swing }) = w.use_on_entity(hand) {
                        if swing {
                            w.swing(hand, animation);
                        }
                        return;
                    }
                }
                Hit::Block { .. } => {
                    let count = w.held_count(hand);
                    match w.use_on_block(hand) {
                        UseResult::Success { swing } => {
                            if swing {
                                w.swing(hand, animation);
                                if !w.held_empty(hand)
                                    && (w.held_count(hand) != count || w.infinite_materials())
                                {
                                    w.item_used(hand);
                                }
                            }
                            return;
                        }
                        UseResult::Fail => return,
                        UseResult::Pass => {}
                    }
                }
                Hit::Miss => {}
            }
            if !w.held_empty(hand)
                && let UseResult::Success { swing } = w.use_item(hand)
            {
                if swing {
                    w.swing(hand, animation);
                }
                w.item_used(hand);
                return;
            }
        }
    }
    /// The attack and use part of `Minecraft.tick`: `rightClickDelay`, the
    /// clicks `handleKeybinds` consumes, held use and attack, then
    /// `missTime`. `attack_down` is `keyAttack.isDown()` with no screen and
    /// the mouse grabbed.
    pub fn tick(
        &mut self,
        w: &mut impl HandWorld,
        attack_clicks: u32,
        use_clicks: u32,
        attack_down: bool,
        use_down: bool,
    ) {
        if self.right_click_delay > 0 {
            self.right_click_delay -= 1;
        }
        let mut instant = false;
        if w.using_item() {
            if !use_down {
                w.release_using();
            }
        } else {
            for _ in 0..attack_clicks {
                instant |= self.start_attack(w);
            }
            for _ in 0..use_clicks {
                self.start_use_item(w);
            }
        }
        if use_down && self.right_click_delay == 0 && !w.using_item() {
            self.start_use_item(w);
        }
        self.continue_attack(w, !instant && attack_down);
        if self.miss_time > 0 {
            self.miss_time -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drawing_a_bow_or_holding_a_charged_crossbow_hides_a_hand() {
        let (bow, apple) = (
            Some(("minecraft:bow", false)),
            Some(("minecraft:apple", false)),
        );
        let charged = Some(("minecraft:crossbow", true));
        assert_eq!(hands_to_render([apple, apple], None), [true, true]);
        assert_eq!(hands_to_render([bow, apple], None), [true, true]);
        assert_eq!(hands_to_render([charged, apple], None), [true, false]);
        assert_eq!(hands_to_render([apple, charged], None), [true, true]);
        let drawing = |hand| Some((hand, "minecraft:bow"));
        assert_eq!(
            hands_to_render([bow, apple], drawing(Hand::Main)),
            [true, false]
        );
        assert_eq!(
            hands_to_render([apple, bow], drawing(Hand::Off)),
            [false, true]
        );
        let eating = |hand| Some((hand, "minecraft:apple"));
        assert_eq!(
            hands_to_render([apple, charged], eating(Hand::Main)),
            [true, false]
        );
        assert_eq!(
            hands_to_render([charged, apple], eating(Hand::Off)),
            [true, true]
        );
    }
    #[test]
    fn a_swing_lasts_its_duration_and_restarts_after_half() {
        let mut s = SwingState::default();
        assert!(s.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 6));
        let mut seen = Vec::new();
        for _ in 0..9 {
            s.tick();
            seen.push(s.animation);
        }
        assert_eq!(
            seen,
            [
                0.0,
                1.0 / 6.0,
                2.0 / 6.0,
                0.5,
                4.0 / 6.0,
                5.0 / 6.0,
                1.0,
                0.0,
                0.0
            ]
        );
        assert!(!s.swinging());
        s.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 6);
        s.tick();
        s.tick();
        assert!(!s.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 6));
        s.tick();
        s.tick();
        assert!(s.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 6));
        s.tick();
        // Restarting the same swing wraps forward rather than snapping back.
        assert_eq!(
            s.animation(0.5),
            (s.old_animation + s.animation + 1.0) / 2.0
        );
    }
    #[test]
    fn haste_shortens_and_fatigue_lengthens_swings() {
        let d = SwingAnimation::DEFAULT;
        assert_eq!(swing_duration(d, None, None), 6);
        assert_eq!(swing_duration(d, Some(1), Some(0)), 4);
        assert_eq!(swing_duration(d, None, Some(2)), 12);
    }
}

#[cfg(test)]
mod item_pose_tests {
    use super::*;
    #[test]
    fn a_map_is_two_handed_only_in_the_main_hand_with_an_empty_off_hand() {
        assert_eq!(MapHold::of(Hand::Main, true), MapHold::TwoHanded);
        assert_eq!(MapHold::of(Hand::Main, false), MapHold::OneHanded);
        assert_eq!(MapHold::of(Hand::Off, true), MapHold::OneHanded);
        assert_eq!(MapHold::of(Hand::Off, false), MapHold::OneHanded);
    }
    #[test]
    fn one_handed_maps_follow_render_one_handed_map() {
        // PoseStack.last().pose() from vanilla's renderOneHandedMap
        // transforms (column-major), for (left, inverse height, attack).
        let cases: [(bool, f32, f32, [f32; 16]); 3] = [
            (
                false,
                0.0,
                0.0,
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.635, -0.205,
                    -0.75, 1.0,
                ],
            ),
            (
                true,
                0.25,
                0.3,
                [
                    0.86894435,
                    -0.34686038,
                    -0.35302067,
                    0.0,
                    0.0,
                    0.713303,
                    -0.70085573,
                    0.0,
                    0.4949098,
                    0.6090046,
                    0.61982065,
                    0.0,
                    -0.1406033,
                    -0.9197687,
                    -0.9926984,
                    1.0,
                ],
            ),
            (
                false,
                0.6,
                0.85,
                [
                    0.9919303,
                    0.024030307,
                    0.12448644,
                    0.0,
                    0.0,
                    0.9818737,
                    -0.18953651,
                    0.0,
                    -0.12678458,
                    0.18800701,
                    0.97395027,
                    0.0,
                    0.5136029,
                    -1.1862278,
                    -0.88621765,
                    1.0,
                ],
            ),
        ];
        for (left, inverse_height, attack, want) in cases {
            let got = one_handed_map_pose(left, inverse_height, attack).to_cols_array();
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g - w).abs() < 1e-5,
                    "{left} {inverse_height} {attack}: {got:?}"
                );
            }
        }
    }
    #[test]
    fn maps_draw_the_arms_under_them_as_vanilla() {
        // PoseStack.last().pose() (column-major) where renderOneHandedMap's
        // renderPlayerArm and renderTwoHandedMap's renderMapHand call
        // renderPlayerHand, replayed with vanilla's PoseStack, Axis and Mth.
        let near = |got: glam::Mat4, want: [f32; 16], what: &str| {
            let got = got.to_cols_array();
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() < 1e-5, "{what}: {got:?}");
            }
        };
        // (left, inverse height, attack, arm)
        #[rustfmt::skip]
        let one: [(bool, f32, f32, [f32; 16]); 3] = [
            (false, 0.0, 0.0, [
                -0.2350201, -0.7860474, -0.5717472, 0.0,
                0.24694103, 0.52063686, -0.8172866, 0.0,
                0.9400987, -0.33326668, 0.07174723, 0.0,
                0.65913033, -1.1543381, -0.73980355, 1.0,
            ]),
            (true, 0.25, 0.3, [
                -0.4967606, 0.84289914, -0.20676011, 0.0,
                0.7202682, 0.2674863, -0.6400506, 0.0,
                -0.4841926, -0.46687466, -0.7399901, 0.0,
                -0.59715486, -1.4160433, -0.7328731, 1.0,
            ]),
            (false, 0.6, 0.85, [
                -0.47358897, -0.8403256, -0.2637542, 0.0,
                0.081554025, 0.25634, -0.96313995, 0.0,
                0.87696207, -0.47764266, -0.052867785, 0.0,
                0.61887383, -1.6482887, -0.7229744, 1.0,
            ]),
        ];
        for (left, inverse_height, attack, want) in one {
            let arms = map_arm_poses(MapHold::OneHanded, left, 0.0, inverse_height, attack);
            assert_eq!(arms.len(), 1, "one arm under a one-handed map");
            assert_eq!(arms[0].0, left, "the arm of the hand holding it");
            near(
                arms[0].1,
                want,
                &format!("one {left} {inverse_height} {attack}"),
            );
        }
        // (pitch, inverse height, attack, right arm, left arm)
        #[rustfmt::skip]
        let two: [(f32, f32, f32, [f32; 16], [f32; 16]); 2] = [
            (0.0, 0.0, 0.0, [
                -0.7380599, 0.44766384, 0.50484115, 0.0,
                -0.67428374, -0.46198538, -0.57611704, 0.0,
                -0.024677532, -0.76561517, 0.64282537, 0.0,
                0.5091893, -0.16204369, 0.3544525, 1.0,
            ], [
                -0.77043974, -0.3951866, -0.50025004, 0.0,
                0.637035, -0.5076031, -0.58010805, 0.0,
                -0.024677532, -0.76561517, 0.64282537, 0.0,
                -0.48071152, -0.12760738, 0.35746527, 1.0,
            ]),
            (30.0, 0.2, 0.5, [
                -0.7380599, -0.11453738, 0.6649426, 0.0,
                -0.67428374, 0.16124588, -0.72065324, 0.0,
                -0.024677532, -0.98024535, -0.1962398, 0.0,
                0.5091893, -0.9508533, -0.13439482, 1.0,
            ], [
                -0.77043974, 0.14373614, -0.6210978, 0.0,
                0.637035, 0.13586378, -0.75876695, 0.0,
                -0.024677532, -0.98024535, -0.1962398, 0.0,
                -0.48071152, -0.9316926, -0.1056232, 1.0,
            ]),
        ];
        for (pitch, inverse_height, attack, right, left) in two {
            let arms = map_arm_poses(MapHold::TwoHanded, false, pitch, inverse_height, attack);
            assert_eq!(arms.len(), 2, "both arms under a two-handed map");
            assert_eq!((arms[0].0, arms[1].0), (false, true), "right, then left");
            near(arms[0].1, right, &format!("two right {pitch}"));
            near(arms[1].1, left, &format!("two left {pitch}"));
        }
    }
    #[test]
    fn ranged_draw_uses_native_charge_curves_and_caps() {
        for (ticks, stretch) in [
            (0.0, 1.0),
            (10.0, 1.0 + 0.2 * 1.25 / 3.0),
            (20.0, 1.2),
            (40.0, 1.2),
        ] {
            let m = item_pose(false, 0.0, 0.0, ItemPose::Bow { ticks_used: ticks });
            assert!((m.determinant() - stretch).abs() < 0.000001);
        }
        for duration in [25.0, 20.0, 15.0, 10.0] {
            let m = item_pose(
                false,
                0.0,
                0.0,
                ItemPose::Crossbow {
                    ticks_used: Some(duration),
                    charge_duration: duration,
                    charged: false,
                },
            );
            assert!((m.determinant() - 1.2).abs() < 0.000001);
        }
    }
    #[test]
    fn left_handed_ranged_poses_mirror_the_right_hand() {
        let reflect = glam::Mat4::from_scale(glam::Vec3::new(-1.0, 1.0, 1.0));
        for pose in [
            ItemPose::Swing,
            ItemPose::Bow { ticks_used: 10.0 },
            ItemPose::Crossbow {
                ticks_used: Some(10.0),
                charge_duration: 25.0,
                charged: false,
            },
            ItemPose::Crossbow {
                ticks_used: None,
                charge_duration: 25.0,
                charged: true,
            },
        ] {
            let left = item_pose(true, 0.2, 0.0, pose).to_cols_array();
            let right = (reflect * item_pose(false, 0.2, 0.0, pose) * reflect).to_cols_array();
            for (a, b) in left.into_iter().zip(right) {
                assert!((a - b).abs() < 0.000001);
            }
        }
    }

    #[test]
    fn drawing_bow_has_its_own_pose() {
        assert_ne!(
            item_pose(false, 0.0, 0.0, ItemPose::Bow { ticks_used: 10.0 }),
            item_pose(false, 0.0, 0.0, ItemPose::Swing)
        );
    }
    #[test]
    fn charging_crossbow_has_its_own_pose() {
        assert_ne!(
            item_pose(
                false,
                0.0,
                0.0,
                ItemPose::Crossbow {
                    ticks_used: Some(10.0),
                    charge_duration: 25.0,
                    charged: false
                }
            ),
            item_pose(false, 0.0, 0.0, ItemPose::Swing)
        );
    }
    #[test]
    fn charged_crossbow_is_aimed_while_idle() {
        assert_ne!(
            item_pose(
                false,
                0.0,
                0.0,
                ItemPose::Crossbow {
                    ticks_used: None,
                    charge_duration: 25.0,
                    charged: true
                }
            ),
            item_pose(false, 0.0, 0.0, ItemPose::Swing)
        );
    }
}

#[cfg(test)]
mod continuous_use_tests {
    use super::*;
    #[test]
    fn eating_and_throwing_have_use_poses_not_attack_poses() {
        let swing = item_pose(false, 0.0, 0.3, ItemPose::Swing);
        assert_ne!(
            swing,
            item_pose(
                false,
                0.0,
                0.3,
                ItemPose::Eat {
                    remaining: 10.0,
                    duration: 32.0,
                    partial: 0.5
                }
            )
        );
        assert_ne!(
            swing,
            item_pose(false, 0.0, 0.3, ItemPose::Trident { ticks_used: 5.0 })
        );
        assert_ne!(swing, item_pose(false, 0.0, 0.3, ItemPose::Using));
    }
}

#[cfg(test)]
mod action_result_tests {
    use super::*;
    #[test]
    fn only_successful_actions_swing_once() {
        assert!(UseResult::Success { swing: true }.should_swing(2, 2));
        for result in [
            UseResult::Fail,
            UseResult::Pass,
            UseResult::Success { swing: false },
        ] {
            assert!(!result.should_swing(2, 2));
        }
        assert!(!UseResult::Success { swing: true }.should_swing(2, 3));
    }
    #[test]
    fn consumption_pose_requires_eat_or_drink_animation() {
        use horizon_world::consumable::UseAnimation::*;
        assert_eq!(consumable_animation(&serde_json::json!({})), Eat);
        assert_eq!(
            consumable_animation(&serde_json::json!({"animation":"drink"})),
            Drink
        );
        for animation in ["none", "block", "bow", "brush"] {
            assert_eq!(
                consumable_animation(&serde_json::json!({"animation":animation})),
                None
            );
        }
    }
}
