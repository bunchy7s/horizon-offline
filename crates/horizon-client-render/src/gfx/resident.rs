//! Resident sections, stored densely and grouped by chunk column for the
//! per-frame culling walk: a column whose box misses the frustum or lies past
//! the render distance rejects all of its sections with one test, and the
//! sections that remain are reached by index rather than by hashing keys.
#[cfg(test)]
use super::Frustum;
use super::{Cull, SectionKey};
use crate::{fast_hash::FxHashMap, streaming::in_view_distance};
use glam::{DVec3, Vec3};

/// One chunk column's resident sections.
struct Column {
    x: i32,
    z: i32,
    /// Section y (from the world's bottom) and slot, in no particular order.
    sections: Vec<(i32, u32)>,
    /// The lowest and highest section y of `sections`.
    low: i32,
    high: i32,
}

impl Column {
    fn bounds(&mut self) {
        self.low = self.sections.iter().map(|s| s.0).min().unwrap_or(0);
        self.high = self.sections.iter().map(|s| s.0).max().unwrap_or(0);
    }
}

/// Values of type `T` per section key, in a dense list with stable keys and
/// slots that move only when a section is removed.
pub struct Resident<T> {
    /// The minimum Y of section index 0 (the level's column).
    min_y: i32,
    items: Vec<(SectionKey, T)>,
    bounds: Vec<Option<[Vec3; 2]>>,
    slots: FxHashMap<SectionKey, u32>,
    columns: Vec<Column>,
    column_index: FxHashMap<(i32, i32), u32>,
}

impl<T> Default for Resident<T> {
    fn default() -> Self {
        Self {
            min_y: crate::chunk::MIN_Y,
            items: Vec::new(),
            bounds: Vec::new(),
            slots: FxHashMap::default(),
            columns: Vec::new(),
            column_index: FxHashMap::default(),
        }
    }
}

impl<T> Resident<T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn contains(&self, key: SectionKey) -> bool {
        self.slots.contains_key(&key)
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|(_, v)| v)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items.iter_mut().map(|(_, v)| v)
    }
    /// `key`'s value, when it is resident.
    pub fn find(&self, key: SectionKey) -> Option<&T> {
        self.slots.get(&key).map(|&slot| self.get(slot))
    }
    pub fn find_mut(&mut self, key: SectionKey) -> Option<&mut T> {
        let slot = *self.slots.get(&key)?;
        Some(self.get_mut(slot))
    }
    /// The section in `slot` (`Visible::slot`).
    pub fn get(&self, slot: u32) -> &T {
        &self.items[slot as usize].1
    }
    pub fn get_mut(&mut self, slot: u32) -> &mut T {
        &mut self.items[slot as usize].1
    }
    /// Sections from now on count from `min_y` (a level of another column).
    pub fn set_min_y(&mut self, min_y: i32) {
        self.min_y = min_y;
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.bounds.clear();
        self.slots.clear();
        self.columns.clear();
        self.column_index.clear();
    }
    /// Adds or replaces `key`'s value with default 16x16x16 bounds.
    #[cfg(test)]
    pub fn insert(&mut self, key: SectionKey, value: T) {
        self.insert_with_bounds(key, value, None);
    }
    /// Adds or replaces `key`'s value with optional tight geometry bounds.
    pub fn insert_with_bounds(&mut self, key: SectionKey, value: T, bounds: Option<[Vec3; 2]>) {
        if let Some(&slot) = self.slots.get(&key) {
            self.items[slot as usize].1 = value;
            self.bounds[slot as usize] = bounds;
            return;
        }
        let slot = self.items.len() as u32;
        self.items.push((key, value));
        self.bounds.push(bounds);
        self.slots.insert(key, slot);
        let column = match self.column_index.get(&(key.0, key.2)) {
            Some(&c) => &mut self.columns[c as usize],
            None => {
                self.column_index
                    .insert((key.0, key.2), self.columns.len() as u32);
                self.columns.push(Column {
                    x: key.0,
                    z: key.2,
                    sections: Vec::new(),
                    low: key.1,
                    high: key.1,
                });
                self.columns.last_mut().expect("just pushed")
            }
        };
        column.sections.push((key.1, slot));
        column.low = column.low.min(key.1);
        column.high = column.high.max(key.1);
    }
    pub fn remove(&mut self, key: SectionKey) -> Option<T> {
        let slot = self.slots.remove(&key)?;
        let c = self.column_index[&(key.0, key.2)] as usize;
        let column = &mut self.columns[c];
        let at = column
            .sections
            .iter()
            .position(|s| s.1 == slot)
            .expect("section in its column");
        column.sections.swap_remove(at);
        if column.sections.is_empty() {
            self.column_index.remove(&(key.0, key.2));
            self.columns.swap_remove(c);
            if let Some(moved) = self.columns.get(c) {
                self.column_index.insert((moved.x, moved.z), c as u32);
            }
        } else {
            column.bounds();
        }
        let _ = self.bounds.swap_remove(slot as usize);
        let (_, value) = self.items.swap_remove(slot as usize);
        // The last item now sits in `slot`: point its key and column at it.
        if let Some(&(moved, _)) = self.items.get(slot as usize) {
            let last = self.items.len() as u32;
            self.slots.insert(moved, slot);
            let column = &mut self.columns[self.column_index[&(moved.0, moved.2)] as usize];
            let entry = column
                .sections
                .iter_mut()
                .find(|s| s.1 == last)
                .expect("moved section in its column");
            entry.1 = slot;
        }
        Some(value)
    }
}

/// A section in view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visible {
    /// Its centre's squared distance from the camera, as bits: non-negative
    /// floats order like their bits, so sorting compares integers.
    pub distance: u64,
    pub key: SectionKey,
    pub slot: u32,
    /// Its camera-relative origin.
    pub origin: [f32; 3],
}

/// The camera-relative origin of the section at `key`.
fn origin(key: SectionKey, min_y: i32, camera: DVec3) -> DVec3 {
    DVec3::new(
        f64::from(key.0 * 16),
        f64::from(min_y + key.1 * 16),
        f64::from(key.2 * 16),
    ) - camera
}

/// The camera's chunk column.
fn camera_column(camera: DVec3) -> (i32, i32) {
    (
        (camera.x.floor() as i32) >> 4,
        (camera.z.floor() as i32) >> 4,
    )
}

/// The world AABB (min, max) of a section from its origin and optional tight bounds.
#[inline]
fn section_box(min: Vec3, bounds: Option<[Vec3; 2]>) -> (Vec3, Vec3) {
    match bounds {
        Some([bmin, bmax]) => (min + bmin, min + bmax),
        None => (min, min + Vec3::splat(16.0)),
    }
}

/// Whether the section at `key` is drawn: its column within the render
/// distance of the camera's (`SectionOcclusionGraph.isInViewDistance`), and
/// partly inside the frustum. `None` when it is not, else its squared centre
/// distance and origin. `Resident::visible` walks columns instead.
#[cfg(test)]
fn in_view(key: SectionKey, camera: DVec3, radius: i32, frustum: &Frustum) -> Option<(f64, Vec3)> {
    in_view_with_bounds(key, camera, radius, frustum, None)
}

/// `in_view` with optional tight bounds.
#[cfg(test)]
fn in_view_with_bounds(
    key: SectionKey,
    camera: DVec3,
    radius: i32,
    frustum: &Frustum,
    bounds: Option<[Vec3; 2]>,
) -> Option<(f64, Vec3)> {
    if !in_view_distance(camera_column(camera), radius, key.0, key.2) {
        return None;
    }
    in_frustum(key, crate::chunk::MIN_Y, camera, frustum, false, bounds)
}

/// `in_view` without the distance test; `inside`: the section's column is
/// entirely inside the frustum, so the section is too.
#[cfg(test)]
fn in_frustum(
    key: SectionKey,
    min_y: i32,
    camera: DVec3,
    frustum: &impl Cull,
    inside: bool,
    bounds: Option<[Vec3; 2]>,
) -> Option<(f64, Vec3)> {
    let origin = origin(key, min_y, camera);
    let min = origin.as_vec3();
    if !inside {
        let (bmin, bmax) = section_box(min, bounds);
        if !frustum.intersects(bmin, bmax) {
            return None;
        }
    }
    Some(((origin + DVec3::splat(8.0)).length_squared(), min))
}

impl<T> Resident<T> {
    /// Appends the sections `in_view` and `reachable` to `out`, nearest
    /// first (ties by key, so the order never depends on storage), and the
    /// sections in the frustum of columns past the render distance that
    /// `beyond` keeps (distant terrain still leaves them out; the occlusion
    /// graph never reaches them).
    ///
    /// Columns are tested first, with the same distance test (it depends
    /// only on x and z) and a box spanning their sections: every section box
    /// lies inside it and the frustum test rounds monotonically, so a column
    /// the frustum rejects holds no section it would accept, and one
    /// entirely inside holds none it would reject (its sections skip the
    /// test). `radius` is the render distance in chunks.
    pub fn visible(
        &self,
        camera: DVec3,
        radius: i32,
        frustum: &impl Cull,
        reachable: impl Fn(SectionKey) -> bool,
        beyond: impl Fn(i32, i32) -> bool,
        out: &mut Vec<Visible>,
    ) {
        let from = camera_column(camera);
        for column in &self.columns {
            let near = in_view_distance(from, radius, column.x, column.z);
            if !near && !beyond(column.x, column.z) {
                continue;
            }
            let min = origin((column.x, column.low, column.z), self.min_y, camera).as_vec3();
            let top = origin((column.x, column.high, column.z), self.min_y, camera).as_vec3();
            let max = top + Vec3::splat(16.0);
            let inside = frustum.contains(min, max);
            if !inside && !frustum.intersects(min, max) {
                continue;
            }
            let ox = f64::from(column.x * 16) - camera.x;
            let oz = f64::from(column.z * 16) - camera.z;
            let min_x = ox as f32;
            let min_z = oz as f32;
            let dx8 = ox + 8.0;
            let dz8 = oz + 8.0;
            let xz_sq = dx8 * dx8 + dz8 * dz8;

            if inside {
                for &(y, slot) in &column.sections {
                    let key = (column.x, y, column.z);
                    if near && !reachable(key) {
                        continue;
                    }
                    let oy = f64::from(self.min_y + y * 16) - camera.y;
                    let min = Vec3::new(min_x, oy as f32, min_z);
                    let dy8 = oy + 8.0;
                    let distance = xz_sq + dy8 * dy8;
                    out.push(Visible {
                        distance: distance.to_bits(),
                        key,
                        slot,
                        origin: min.to_array(),
                    });
                }
            } else {
                for &(y, slot) in &column.sections {
                    let key = (column.x, y, column.z);
                    if near && !reachable(key) {
                        continue;
                    }
                    let oy = f64::from(self.min_y + y * 16) - camera.y;
                    let min = Vec3::new(min_x, oy as f32, min_z);
                    let (bmin, bmax) = section_box(min, self.bounds[slot as usize]);
                    if !frustum.intersects(bmin, bmax) {
                        continue;
                    }
                    let dy8 = oy + 8.0;
                    let distance = xz_sq + dy8 * dy8;
                    out.push(Visible {
                        distance: distance.to_bits(),
                        key,
                        slot,
                        origin: min.to_array(),
                    });
                }
            }
        }
        out.sort_unstable_by_key(|v| (v.distance, v.key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn column_walk_finds_exactly_the_sections_in_view() {
        let mut r = Resident::default();
        for x in -12i32..=12 {
            for z in -12..=12 {
                for y in 0..24 {
                    // Leave holes so columns have uneven bounds.
                    if (x * 7 + y * 3 + z * 5).rem_euclid(4) != 0 {
                        // Mix of sections with fallback None and randomized tight bounds.
                        let bounds = if (x + y + z).rem_euclid(2) == 0 {
                            let y_min = ((x + z).rem_euclid(4)) as f32;
                            let y_max = y_min + 2.0 + ((y.rem_euclid(5)) as f32);
                            Some([Vec3::new(0.0, y_min, 0.0), Vec3::new(16.0, y_max, 16.0)])
                        } else {
                            None
                        };
                        r.insert_with_bounds((x, y, z), (), bounds);
                    }
                }
            }
        }
        let reachable = |k: SectionKey| (k.0 + k.1 + k.2).rem_euclid(9) != 0;
        let cameras = [
            (DVec3::new(0.5, 70.0, 0.5), 0.0, 0.0),
            (DVec3::new(-37.25, 130.5, 21.75), 135.0, 35.0),
            (DVec3::new(100.0, -60.0, -3.0), 270.0, -80.0),
            (DVec3::new(8.0, 64.0, 8.0), 45.0, 89.0),
        ];
        for (camera, yaw, pitch) in cameras {
            for radius in [2, 6, 10] {
                let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
                let view = crate::camera::view_matrix(yaw, pitch);
                let frustum = Frustum {
                    planes: super::super::frustum(projection * view),
                };
                let mut fast = Vec::new();
                // Columns past the distance kept for distant terrain.
                let beyond = |x: i32, z: i32| (x + z).rem_euclid(3) == 0;
                r.visible(camera, radius, &frustum, reachable, beyond, &mut fast);
                let mut slow: Vec<(SectionKey, f64, [f32; 3])> = r
                    .items
                    .iter()
                    .enumerate()
                    .filter_map(|(slot, &(k, _))| {
                        let near = in_view_distance(camera_column(camera), radius, k.0, k.2);
                        if near {
                            in_view_with_bounds(k, camera, radius, &frustum, r.bounds[slot])
                                .filter(|_| reachable(k))
                        } else if beyond(k.0, k.2) {
                            in_frustum(
                                k,
                                crate::chunk::MIN_Y,
                                camera,
                                &frustum,
                                false,
                                r.bounds[slot],
                            )
                        } else {
                            None
                        }
                        .map(|(d, o)| (k, d, o.to_array()))
                    })
                    .collect();
                slow.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                let fast: Vec<_> = fast
                    .iter()
                    .map(|v| {
                        assert_eq!(r.items[v.slot as usize].0, v.key);
                        (v.key, f64::from_bits(v.distance), v.origin)
                    })
                    .collect();
                assert!(!slow.is_empty());
                assert_eq!(fast, slow, "{camera} {yaw} {pitch} {radius}");
            }
        }
    }

    #[test]
    fn columns_entirely_inside_skip_no_section() {
        // Short columns ahead of the camera: most lie entirely inside.
        let mut r = Resident::default();
        for x in -12i32..=12 {
            for z in 1..=12 {
                for y in 3..6 {
                    if (x + y + z).rem_euclid(3) != 0 {
                        r.insert((x, y, z), ());
                    }
                }
            }
        }
        let camera = DVec3::new(0.5, -8.0, 0.5);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * crate::camera::view_matrix(0.0, 5.0)),
        };
        let inside = r
            .columns
            .iter()
            .filter(|c| {
                let min = origin((c.x, c.low, c.z), crate::chunk::MIN_Y, camera).as_vec3();
                let top = origin((c.x, c.high, c.z), crate::chunk::MIN_Y, camera).as_vec3();
                frustum.contains(min, top + Vec3::splat(16.0))
            })
            .count();
        assert!(inside > 50 && inside < r.columns.len(), "{inside}");
        let mut fast = Vec::new();
        r.visible(camera, 12, &frustum, |_| true, |_, _| false, &mut fast);
        let mut slow: Vec<_> = r
            .items
            .iter()
            .filter_map(|&(k, _)| in_view(k, camera, 12, &frustum).map(|(d, _)| (d, k)))
            .collect();
        slow.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let fast: Vec<_> = fast
            .iter()
            .map(|v| (f64::from_bits(v.distance), v.key))
            .collect();
        assert_eq!(fast, slow);
    }

    /// Every key reaches its value through its slot and column, and every
    /// column's bounds and sections match the keys.
    fn check(r: &Resident<u32>, model: &HashMap<SectionKey, u32>) {
        assert_eq!(r.len(), model.len());
        let mut seen = 0;
        for column in &r.columns {
            assert!(!column.sections.is_empty());
            let ys: Vec<i32> = column.sections.iter().map(|s| s.0).collect();
            assert_eq!(column.low, *ys.iter().min().unwrap());
            assert_eq!(column.high, *ys.iter().max().unwrap());
            for &(y, slot) in &column.sections {
                let (key, value) = &r.items[slot as usize];
                assert_eq!(*key, (column.x, y, column.z));
                assert_eq!(model[key], *value);
                seen += 1;
            }
        }
        assert_eq!(seen, model.len());
        for key in model.keys() {
            assert!(r.contains(*key));
        }
    }

    #[test]
    fn slots_and_columns_follow_inserts_and_removals() {
        let mut r = Resident::default();
        let mut model = HashMap::new();
        // A deterministic mix of inserts, replacements and removals.
        let mut seed = 0x2545_f491_u32;
        for step in 0..4000u32 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let key = (
                (seed % 5) as i32 - 2,
                ((seed >> 4) % 6) as i32 - 1,
                ((seed >> 8) % 5) as i32 - 2,
            );
            if (seed >> 12).is_multiple_of(3) {
                assert_eq!(r.remove(key), model.remove(&key));
            } else {
                r.insert(key, step);
                model.insert(key, step);
            }
            check(&r, &model);
        }
        r.clear();
        assert_eq!(r.len(), 0);
        assert!(r.columns.is_empty());
    }

    /// The old walk (every key of a hash map, sorted by distance, then two
    /// lookups per visible section) against `visible` on a render distance
    /// 32 world. Run with `cargo test --profile fast -p horizon-client --lib
    /// -- --ignored column_walk_speed --nocapture`.
    #[test]
    #[ignore = "timing, not a check"]
    fn column_walk_speed() {
        use std::time::{Duration, Instant};
        let mut r = Resident::default();
        let mut map = HashMap::new();
        for x in -33i32..=33 {
            for z in -33i32..=33 {
                for y in 0..24 {
                    if (x * 7 + y * 3 + z * 5).rem_euclid(4) != 0 {
                        r.insert((x, y, z), [x, y, z]);
                        map.insert((x, y, z), [x, y, z]);
                    }
                }
            }
        }
        let camera = DVec3::new(8.5, 100.0, 8.5);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 1024.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * crate::camera::view_matrix(30.0, 35.0)),
        };
        let (mut old, mut new) = (Duration::MAX, Duration::MAX);
        let (mut list, mut keys) = (Vec::new(), Vec::new());
        let mut sum = 0i64;
        for _ in 0..200 {
            let t = Instant::now();
            keys.clear();
            keys.extend(map.keys().filter_map(|&k| {
                in_view(k, camera, 32, &frustum).map(|(d, o)| (k, d, o.to_array()))
            }));
            keys.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
            for (k, _, _) in &keys {
                sum += i64::from(map[k][1] + map[k][0]);
            }
            old = old.min(t.elapsed());
            let t = Instant::now();
            list.clear();
            r.visible(camera, 32, &frustum, |_| true, |_, _| false, &mut list);
            for v in &list {
                sum += i64::from(r.get(v.slot)[1] + r.get(v.slot)[0]);
            }
            new = new.min(t.elapsed());
        }
        assert_eq!(keys.len(), list.len());
        println!(
            "{} resident, {} visible: old walk {old:?}, column walk {new:?} (checksum {sum})",
            r.len(),
            list.len()
        );
    }

    #[test]
    fn tight_mesh_bounds_culls_section_outside_frustum() {
        let mut r = Resident::default();
        let key = (0, 0, 0);
        // Section geometry only occupies local Y in [0.0, 2.0]
        let tight = Some([Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, 2.0, 16.0)]);
        r.insert_with_bounds(key, (), tight);

        let camera = DVec3::new(8.0, f64::from(crate::chunk::MIN_Y + 15), -2.0);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);
        let view = crate::camera::view_matrix(0.0, -60.0);
        let frustum = Frustum {
            planes: super::super::frustum(projection * view),
        };

        // Tight bounds culls the section whose geometry is entirely outside the frustum
        let mut out = Vec::new();
        r.visible(camera, 4, &frustum, |_| true, |_, _| false, &mut out);
        assert!(
            out.is_empty(),
            "tight bounds should cull section whose geometry is below frustum"
        );

        // With fallback 16x16x16 bounds, the section intersects the frustum and is visible
        r.insert_with_bounds(key, (), None);
        out.clear();
        r.visible(camera, 4, &frustum, |_| true, |_, _| false, &mut out);
        assert_eq!(
            out.len(),
            1,
            "full 16x16x16 bounds intersect frustum and are accepted"
        );
    }

    #[test]
    #[ignore = "benchmark"]
    fn tight_bounds_benchmark() {
        use std::time::Instant;

        let mut r_fallback = Resident::default();
        let mut r_tight = Resident::default();

        let radius = 16;
        for x in -radius..=radius {
            for z in -radius..=radius {
                // Section 0..4: underground rock
                for y in 0..4 {
                    r_fallback.insert((x, y, z), ());
                    r_tight.insert_with_bounds((x, y, z), (), None);
                }
                // Section 4: surface terrain with grass / hills (e.g. Y in [0, 4] or [0, 6])
                let height = 2.0 + (((x * 3 + z * 5).rem_euclid(5)) as f32) * 1.5;
                let surface_bounds =
                    Some([Vec3::new(0.0, 0.0, 0.0), Vec3::new(16.0, height, 16.0)]);
                r_fallback.insert((x, 4, z), ());
                r_tight.insert_with_bounds((x, 4, z), (), surface_bounds);
            }
        }

        println!("\n=== Tight Bounds Culling Benchmark ===");
        println!("Total Resident Sections: {}", r_tight.len());

        let test_views = [
            ("Looking level across terrain (pitch 0°)", 0.0, 0.0),
            ("Looking slightly up (pitch -15°)", 45.0, -15.0),
            ("Looking up towards sky (pitch -35°)", 90.0, -35.0),
            ("Looking down at ground (pitch 25°)", 180.0, 25.0),
            ("Diagonal view (yaw 135°, pitch -10°)", 135.0, -10.0),
        ];

        let camera = DVec3::new(8.0, f64::from(crate::chunk::MIN_Y + 4 * 16 + 10), 8.0);
        let projection = super::super::world_projection(70.0, 16.0 / 9.0, 512.0);

        for (desc, yaw, pitch) in test_views {
            let view = crate::camera::view_matrix(yaw, pitch);
            let frustum = Frustum {
                planes: super::super::frustum(projection * view),
            };

            let mut out_fallback = Vec::new();
            let mut out_tight = Vec::new();

            r_fallback.visible(
                camera,
                radius,
                &frustum,
                |_| true,
                |_, _| false,
                &mut out_fallback,
            );
            r_tight.visible(
                camera,
                radius,
                &frustum,
                |_| true,
                |_, _| false,
                &mut out_tight,
            );

            let culled_extra = out_fallback.len().saturating_sub(out_tight.len());
            let pct_saved = if !out_fallback.is_empty() {
                (culled_extra as f64 / out_fallback.len() as f64) * 100.0
            } else {
                0.0
            };

            // Timing over 1,000 passes
            let iters = 1_000;
            let t0 = Instant::now();
            for _ in 0..iters {
                out_fallback.clear();
                r_fallback.visible(
                    camera,
                    radius,
                    &frustum,
                    |_| true,
                    |_, _| false,
                    &mut out_fallback,
                );
            }
            let dur_fallback = t0.elapsed();

            let t1 = Instant::now();
            for _ in 0..iters {
                out_tight.clear();
                r_tight.visible(
                    camera,
                    radius,
                    &frustum,
                    |_| true,
                    |_, _| false,
                    &mut out_tight,
                );
            }
            let dur_tight = t1.elapsed();

            println!(
                "{desc}\n  Fallback 16x16: {} sections | {:?}\n  Tight Bounds:   {} sections | {:?}\n  Reduction:      -{} sections ({:.1}% culled)\n",
                out_fallback.len(),
                dur_fallback / iters,
                out_tight.len(),
                dur_tight / iters,
                culled_extra,
                pct_saved
            );
        }
    }
}
