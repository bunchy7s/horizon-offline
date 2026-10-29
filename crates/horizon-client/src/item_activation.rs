//! The item that spins up to the screen when it activates (the totem of
//! undying): `net.minecraft.client.player.ItemActivation`, set by
//! `LocalPlayer.displayItemActivation`, ticked by `GameRenderer.tick`, and
//! posed by `ScreenEffectRenderer.renderItemActivationAnimation`.
use crate::items::ItemStack;
use glam::{Mat4, Vec3};
use horizon_core::{geometry::trig::Mth, random::RandomSource};

/// `ItemActivation.ANIMATION_LENGTH`, in ticks.
pub const ANIMATION_LENGTH: i32 = 40;

#[derive(Clone, Debug, Default)]
pub struct ItemActivation {
    item: Option<ItemStack>,
    ticks: i32,
    off_x: f32,
    off_y: f32,
}

impl ItemActivation {
    /// `ItemActivation.tick`: the item goes when the count reaches 0.
    pub fn tick(&mut self) {
        if self.ticks > 0 {
            self.ticks -= 1;
            if self.ticks == 0 {
                self.item = None;
            }
        }
    }
    /// `ItemActivation.reset` (`GameRenderer.resetData`).
    pub fn reset(&mut self) {
        self.item = None;
        self.ticks = 0;
    }
    /// `ItemActivation.activate`: a copy of the stack, the full length, and
    /// two random offsets in [-1, 1) for where on the screen it flies.
    pub fn activate(&mut self, item: ItemStack, random: &mut impl RandomSource) {
        self.item = Some(item);
        self.ticks = ANIMATION_LENGTH;
        self.off_x = random.next_float() * 2.0 - 1.0;
        self.off_y = random.next_float() * 2.0 - 1.0;
    }
    /// `ItemActivation.isActive`: the item to draw, while it shows.
    pub fn active(&self) -> Option<&ItemStack> {
        self.item.as_ref().filter(|_| self.ticks > 0)
    }
    /// The item's view-space pose this frame, before its `FIXED` display
    /// transform; `aspect` is the window's width over its height.
    pub fn pose(&self, partial: f32, aspect: f32) -> Option<Mat4> {
        self.active()?;
        Some(animation_pose(
            self.ticks, partial, self.off_x, self.off_y, aspect,
        ))
    }
}

/// `ScreenEffectRenderer.renderItemActivationAnimation`: a quintic over the
/// 40 ticks brings the item from 10 blocks out to 1 and back while it turns
/// 900 degrees about Y and rocks 6 degrees about X and Z, drawn with the
/// hand's projection and no camera transform.
pub fn animation_pose(ticks: i32, partial: f32, off_x: f32, off_y: f32, aspect: f32) -> Mat4 {
    let tick = ANIMATION_LENGTH - ticks;
    let t = (tick as f32 + partial) / 40.0;
    let t2 = t * t;
    let t3 = t * t2;
    let curve = 10.25 * t3 * t2 - 24.95 * t2 * t2 + 25.5 * t3 - 13.8 * t2 + 4.0 * t;
    let angle = curve * std::f32::consts::PI;
    let x = off_x * 0.3 * aspect;
    let y = off_y * 0.3;
    let swing = (angle * 2.0).msin().abs();
    Mat4::from_translation(Vec3::new(x * swing, y * swing, -10.0 + 9.0 * angle.msin()))
        * Mat4::from_scale(Vec3::splat(0.8))
        * Mat4::from_rotation_y((900.0 * angle.msin().abs()).to_radians())
        * Mat4::from_rotation_x((6.0 * (t * 8.0).mcos()).to_radians())
        * Mat4::from_rotation_z((6.0 * (t * 8.0).mcos()).to_radians())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    fn totem() -> ItemStack {
        ItemStack::new("minecraft:totem_of_undying", 1)
    }

    #[test]
    fn it_shows_for_forty_ticks() {
        let mut a = ItemActivation::default();
        assert!(a.active().is_none());
        a.activate(totem(), &mut LegacyRandom::new(0));
        for _ in 0..39 {
            a.tick();
            assert!(a.active().is_some());
        }
        a.tick();
        assert!(a.active().is_none());
        assert!(a.pose(0.0, 1.0).is_none());
    }

    #[test]
    fn the_offsets_are_two_floats_from_the_players_random() {
        // java.util.Random(0): nextFloat() 0.73096776, then 0.831441.
        let mut a = ItemActivation::default();
        a.activate(totem(), &mut LegacyRandom::new(0));
        assert!((a.off_x - 0.461_935_5).abs() < 1e-6, "{}", a.off_x);
        assert!((a.off_y - 0.662_882).abs() < 1e-6, "{}", a.off_y);
    }

    #[test]
    fn it_comes_from_ten_blocks_out_to_one_and_back() {
        let at = |ticks, partial| {
            animation_pose(ticks, partial, 0.5, -0.5, 16.0 / 9.0)
                .w_axis
                .truncate()
        };
        // First tick: straight ahead, 10 out.
        assert_eq!(at(40, 0.0), Vec3::new(0.0, 0.0, -10.0));
        // Halfway the curve is 0.4984375: almost PI/2, so about 1 out and
        // the sideways swing (|sin 2a|) nearly gone.
        let mid = at(20, 0.0);
        assert!((mid.z - -1.0).abs() < 1e-3, "{mid}");
        let a = 0.498_437_5 * std::f32::consts::PI;
        let swing = (a * 2.0).msin().abs();
        assert!((mid.x - 0.5 * 0.3 * 16.0 / 9.0 * swing).abs() < 1e-6);
        assert!((mid.y - -0.5 * 0.3 * swing).abs() < 1e-6);
        // A quarter in (curve 0.4484863): already near, swung to the side.
        let quarter = at(30, 0.0);
        let a = 0.448_486_33 * std::f32::consts::PI;
        assert!((quarter.z - (-10.0 + 9.0 * a.msin())).abs() < 1e-5);
        let swing = (a * 2.0).msin().abs();
        assert!((quarter.x - 0.5 * 0.3 * 16.0 / 9.0 * swing).abs() < 1e-6);
        // Partial ticks move it smoothly between ticks.
        let between = at(39, 0.5).z;
        assert!(between > at(39, 0.0).z && between < at(38, 0.0).z);
    }

    #[test]
    fn it_is_scaled_to_four_fifths() {
        let m = animation_pose(25, 0.3, 0.0, 0.0, 1.0);
        let (scale, _, _) = m.to_scale_rotation_translation();
        assert!((scale - Vec3::splat(0.8)).abs().max_element() < 1e-5);
    }
}
