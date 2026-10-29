//! The world border (`WorldBorder`): the extent geometry of
//! `horizon_core::geometry::border` (static or moving, with the previous size
//! for partial ticks) plus the damage and warning settings, and how strongly
//! the warning vignette shows.
use horizon_core::geometry::border::BorderGeometry;
pub use horizon_core::geometry::border::BorderStatus;

/// `WorldBorder.MAX_SIZE` (the float 5.999997E7 as a double).
pub const MAX_SIZE: f64 = 59_999_968.0;

/// `BorderStatus.getColor`: the wall colour.
pub fn status_color(status: BorderStatus) -> u32 {
    match status {
        BorderStatus::Growing => 0x40FF80,
        BorderStatus::Shrinking => 0xFF3030,
        BorderStatus::Stationary => 0x20A0FF,
    }
}

/// `Math.min(double, double)`, including the signed-zero rule.
pub(crate) fn jmin(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        b
    } else if a <= b {
        a
    } else {
        b
    }
}
/// `Math.max(double, double)`, including the signed-zero rule.
pub(crate) fn jmax(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        b
    } else if a >= b {
        a
    } else {
        b
    }
}
/// `Mth.clamp(double, double, double)`.
pub(crate) fn jclamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo { lo } else { jmin(v, hi) }
}

#[derive(Clone, Debug)]
pub struct WorldBorder {
    geometry: BorderGeometry,
    pub damage_per_block: f64,
    pub safe_zone: f64,
    pub warning_distance: i32,
    pub warning_time: i32,
}
impl Default for WorldBorder {
    fn default() -> Self {
        Self {
            geometry: BorderGeometry::new([0.0, 0.0], MAX_SIZE, 29_999_984)
                .expect("default border"),
            damage_per_block: 0.2,
            safe_zone: 5.0,
            warning_distance: 5,
            warning_time: 300,
        }
    }
}
impl PartialEq for WorldBorder {
    fn eq(&self, other: &Self) -> bool {
        self.center() == other.center()
            && self.size() == other.size()
            && self.target_size() == other.target_size()
            && self.remaining_ticks() == other.remaining_ticks()
            && self.absolute_max_size() == other.absolute_max_size()
            && self.damage_per_block == other.damage_per_block
            && self.safe_zone == other.safe_zone
            && self.warning_distance == other.warning_distance
            && self.warning_time == other.warning_time
    }
}
impl WorldBorder {
    /// Its extent, for collisions.
    pub fn geometry(&self) -> &BorderGeometry {
        &self.geometry
    }
    pub fn center(&self) -> [f64; 2] {
        self.geometry.center()
    }
    /// `setCenter` (non-finite centres are ignored).
    pub fn set_center(&mut self, center: [f64; 2]) {
        let _ = self.geometry.set_center(center);
    }
    pub fn absolute_max_size(&self) -> i32 {
        self.geometry.absolute_max()
    }
    /// `setAbsoluteMaxSize`.
    pub fn set_absolute_max_size(&mut self, limit: i32) {
        let _ = self.geometry.set_absolute_max(limit);
    }
    /// `getSize`.
    pub fn size(&self) -> f64 {
        self.geometry.size()
    }
    /// `getLerpTarget`.
    pub fn target_size(&self) -> f64 {
        self.geometry.lerp_target()
    }
    /// `getLerpTime`: ticks left in a size change (0 when none).
    pub fn remaining_ticks(&self) -> i64 {
        self.geometry.lerp_time()
    }
    /// `getLerpSpeed`: blocks per tick.
    pub fn lerp_speed(&self) -> f64 {
        self.geometry.lerp_speed()
    }
    /// `getStatus`.
    pub fn status(&self) -> BorderStatus {
        self.geometry.status()
    }
    pub fn set_size(&mut self, size: f64) {
        let _ = self.geometry.set_size(size.clamp(1.0, MAX_SIZE));
    }
    /// `lerpSizeBetween(from, to, ticks)` (what the server and the border
    /// packets do).
    pub fn lerp_between(&mut self, from: f64, to: f64, ticks: i64) {
        let _ = self.geometry.lerp_size_between(from, to, ticks);
    }
    /// A size change from the current size (`/worldborder set|add` with a time).
    pub fn lerp_to(&mut self, to: f64, ticks: i64) {
        if ticks <= 0 {
            self.set_size(to);
        } else {
            self.lerp_between(self.size(), to.clamp(1.0, MAX_SIZE), ticks);
        }
    }
    /// `tick`.
    pub fn tick(&mut self) {
        let _ = self.geometry.tick();
    }
    /// `getMinX(partial)`, `getMinZ`, `getMaxX`, `getMaxZ`.
    pub fn bounds_at(&self, partial: f32) -> [f64; 4] {
        self.geometry.bounds(partial)
    }
    /// `getMinX()` etc. (partial tick 0: the previous size while moving).
    pub fn bounds(&self) -> [f64; 4] {
        self.bounds_at(0.0)
    }
    /// `getDistanceToBorder(x, z)`: negative outside.
    pub fn distance(&self, x: f64, z: f64) -> f64 {
        self.geometry.distance_to_border(x, z)
    }
    pub fn contains(&self, x: f64, z: f64) -> bool {
        self.geometry.is_within_bounds(x, z, 0.0)
    }
    /// `Hud.extractVignette`: how strongly the vignette turns red at
    /// (x, z), 0 when outside the warning distance.
    pub fn warning_strength(&self, x: f64, z: f64) -> f32 {
        crate::hud_overlays::border_warning(
            self.distance(x, z) as f32,
            self.warning_distance,
            self.warning_time,
            self.lerp_speed(),
            self.target_size() - self.size(),
        )
    }
    /// Clamp a point so a body of half-width `half` stays inside.
    pub fn clamp(&self, x: f64, z: f64, half: f64) -> [f64; 2] {
        let [min_x, min_z, max_x, max_z] = self.bounds();
        [
            x.clamp(min_x + half, (max_x - half).max(min_x + half)),
            z.clamp(min_z + half, (max_z - half).max(min_z + half)),
        ]
    }
    pub fn pack(&self) -> horizon_world::nbt::Tag {
        use horizon_world::nbt::Tag;
        Tag::compound([
            ("center_x", Tag::Double(self.center()[0].to_bits())),
            ("center_z", Tag::Double(self.center()[1].to_bits())),
            ("size", Tag::Double(self.size().to_bits())),
            (
                "damage_per_block",
                Tag::Double(self.damage_per_block.to_bits()),
            ),
            ("safe_zone", Tag::Double(self.safe_zone.to_bits())),
            ("warning_blocks", Tag::Int(self.warning_distance)),
            ("warning_time", Tag::Int(self.warning_time)),
            ("lerp_target", Tag::Double(self.target_size().to_bits())),
            ("lerp_time", Tag::Long(self.remaining_ticks())),
        ])
    }
    pub fn unpack(tag: &horizon_world::nbt::Tag) -> Self {
        use horizon_world::nbt::Tag;
        let d = |k: &str, default: f64| match tag.get(k) {
            Some(Tag::Double(b)) => f64::from_bits(*b),
            _ => default,
        };
        let i = |k: &str, default: i32| match tag.get(k) {
            Some(Tag::Int(v)) => *v,
            _ => default,
        };
        let size = d("size", MAX_SIZE).clamp(1.0, MAX_SIZE);
        let mut b = Self {
            damage_per_block: d("damage_per_block", 0.2),
            safe_zone: d("safe_zone", 5.0),
            warning_distance: i("warning_blocks", 5),
            warning_time: i("warning_time", 300),
            ..Self::default()
        };
        b.set_center([d("center_x", 0.0), d("center_z", 0.0)]);
        b.set_size(size);
        if let Some(Tag::Long(t)) = tag.get("lerp_time")
            && *t > 0
        {
            let to = d("lerp_target", size);
            b.lerp_between(size, to.clamp(1.0, MAX_SIZE), *t);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_and_distance() {
        let mut b = WorldBorder::default();
        b.set_size(100.0);
        assert_eq!(b.distance(0.0, 0.0), 50.0);
        assert_eq!(b.distance(60.0, 0.0), -10.0);
        assert!(b.contains(49.9, -49.9) && !b.contains(50.0, 0.0));
        b.lerp_to(200.0, 10);
        assert_eq!(b.size(), 100.0);
        assert_eq!(b.status(), BorderStatus::Growing);
        for _ in 0..5 {
            b.tick();
        }
        assert_eq!(b.size(), 150.0);
        assert_eq!(b.remaining_ticks(), 5);
        // Bounds at partial tick 0 still use the previous tick's size.
        assert_eq!(b.bounds()[2], 70.0);
        assert_eq!(b.bounds_at(1.0)[2], 75.0);
        for _ in 0..5 {
            b.tick();
        }
        assert_eq!((b.size(), b.remaining_ticks()), (200.0, 0));
        // An instant change and clamping a body inside.
        b.lerp_to(10.0, 0);
        assert_eq!(b.size(), 10.0);
        assert_eq!(b.status(), BorderStatus::Stationary);
        assert_eq!(b.clamp(100.0, -100.0, 0.3), [4.7, -4.7]);
        b.set_center([10.0, 0.0]);
        assert_eq!(b.bounds(), [5.0, -5.0, 15.0, 5.0]);
    }
    #[test]
    fn saved_form_round_trips() {
        let mut b = WorldBorder::default();
        b.set_size(500.0);
        b.set_center([3.5, -7.0]);
        b.damage_per_block = 1.5;
        b.warning_distance = 9;
        let back = WorldBorder::unpack(&b.pack());
        assert_eq!(back, b);
    }
}
