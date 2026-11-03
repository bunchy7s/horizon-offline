//! Conservative, budgeted occlusion against full opaque block cross-sections.
//!
//! A covered projection rectangle proves that an entire bounding box is hidden.
//! Testing a few rays would miss openings between those rays.
use std::cell::Cell;

const EPSILON: f64 = 1.0e-5;

pub struct Culler {
    pub camera: [f64; 3],
    remaining: Cell<usize>,
}

impl Culler {
    pub fn new(camera: [f64; 3], budget: usize) -> Self {
        Self {
            camera,
            remaining: Cell::new(budget),
        }
    }

    /// Remove only this entity's completed camera-relative mesh when hidden.
    pub fn cull_vertices(
        &self,
        vertices: &mut Vec<crate::mesher::Vertex>,
        start: usize,
        opaque: impl FnMut([i32; 3]) -> bool,
    ) -> bool {
        let hidden = self.hidden(vertices, start, opaque);
        if hidden {
            vertices.truncate(start);
        }
        hidden
    }

    /// Whether this entity's completed camera-relative mesh (`vertices[start..]`) is hidden.
    pub fn hidden(
        &self,
        vertices: &[crate::mesher::Vertex],
        start: usize,
        opaque: impl FnMut([i32; 3]) -> bool,
    ) -> bool {
        if self.remaining.get() == 0 {
            return false;
        }
        let Some((min, max)) = Self::bounds(self.camera, vertices, start) else {
            return false;
        };
        self.occluded(min, max, opaque)
    }

    /// Camera-relative bounds grown to include floating-point and rasterization
    /// edges. The asynchronous path sends these small queries to its worker.
    pub fn bounds(
        camera: [f64; 3],
        vertices: &[crate::mesher::Vertex],
        start: usize,
    ) -> Option<([f64; 3], [f64; 3])> {
        if start >= vertices.len() {
            return None;
        }
        let mut min = [f64::INFINITY; 3];
        let mut max = [f64::NEG_INFINITY; 3];
        for vertex in &vertices[start..] {
            for a in 0..3 {
                let value = f64::from(vertex.pos[a]) + camera[a];
                if !value.is_finite() {
                    return None;
                }
                min[a] = min[a].min(value);
                max[a] = max[a].max(value);
            }
        }
        // Cover floating-point and rasterization edges as well as the vertices.
        Some((min.map(|v| v - 0.05), max.map(|v| v + 0.05)))
    }

    /// Returns true only when a complete opaque block plane covers the bounds.
    /// Unknown blocks, invalid bounds and an exhausted lookup budget stay visible.
    pub fn occluded(
        &self,
        min: [f64; 3],
        max: [f64; 3],
        mut opaque: impl FnMut([i32; 3]) -> bool,
    ) -> bool {
        if (0..3).any(|a| {
            !self.camera[a].is_finite()
                || !min[a].is_finite()
                || !max[a].is_finite()
                || min[a] > max[a]
        }) {
            return false;
        }
        let mut best = (0, 0.0, 0.0);
        for a in 0..3 {
            let (gap, sign) = if self.camera[a] < min[a] {
                (min[a] - self.camera[a], 1.0)
            } else {
                (self.camera[a] - max[a], -1.0)
            };
            if gap > best.1 {
                best = (a, gap, sign);
            }
        }
        let (axis, gap, sign) = best;
        if !gap.is_finite() || gap <= EPSILON {
            return false;
        }
        let across = [(axis + 1) % 3, (axis + 2) % 3];
        let Some(mut cell) = floor_cell(self.camera[axis]) else {
            return false;
        };
        let step = sign as i32;
        if (f64::from(cell) + 0.5 - self.camera[axis]) * sign <= EPSILON {
            let Some(next) = cell.checked_add(step) else {
                return false;
            };
            cell = next;
        }
        loop {
            let depth = (f64::from(cell) + 0.5 - self.camera[axis]) * sign;
            if depth >= gap - EPSILON || self.remaining.get() == 0 {
                return false;
            }
            let mut lo = [f64::INFINITY; 2];
            let mut hi = [f64::NEG_INFINITY; 2];
            for corner in 0..8 {
                let point: [f64; 3] = std::array::from_fn(|a| {
                    if corner & (1 << a) == 0 {
                        min[a]
                    } else {
                        max[a]
                    }
                });
                let t = depth / ((point[axis] - self.camera[axis]) * sign);
                for (i, &a) in across.iter().enumerate() {
                    let p = self.camera[a] + t * (point[a] - self.camera[a]);
                    if !p.is_finite() {
                        return false;
                    }
                    lo[i] = lo[i].min(p);
                    hi[i] = hi[i].max(p);
                }
            }
            let bounds = [
                floor_cell(lo[0] - EPSILON),
                floor_cell(hi[0] + EPSILON),
                floor_cell(lo[1] - EPSILON),
                floor_cell(hi[1] + EPSILON),
            ];
            let [Some(x0), Some(x1), Some(y0), Some(y1)] = bounds else {
                return false;
            };
            let mut pos = [0; 3];
            pos[axis] = cell;
            pos[across[0]] = x0 + ((i64::from(x1) - i64::from(x0)) / 2) as i32;
            pos[across[1]] = y0 + ((i64::from(y1) - i64::from(y0)) / 2) as i32;
            self.remaining.set(self.remaining.get() - 1);
            if opaque(pos) {
                let area = (i64::from(x1) - i64::from(x0) + 1)
                    .checked_mul(i64::from(y1) - i64::from(y0) + 1)
                    .and_then(|n| usize::try_from(n).ok());
                let Some(area) = area.filter(|&n| n <= self.remaining.get()) else {
                    return false;
                };
                // Reserve the worst-case lookups before walking the rectangle.
                self.remaining.set(self.remaining.get() - area);
                let covered = (x0..=x1).all(|x| {
                    (y0..=y1).all(|y| {
                        pos[across[0]] = x;
                        pos[across[1]] = y;
                        opaque(pos)
                    })
                });
                if covered {
                    return true;
                }
            }
            let Some(next) = cell.checked_add(step) else {
                return false;
            };
            cell = next;
        }
    }
}

fn floor_cell(value: f64) -> Option<i32> {
    let n = value.floor();
    (n.is_finite() && n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX)).then_some(n as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_mesh_culling_preserves_prior_geometry_and_protruding_parts() {
        let vertex = |pos| crate::mesher::Vertex {
            pos,
            uv: [0.0; 2],
            layer: 0,
            color: u32::MAX,
            light: 0,
            normal: [0; 4],

            glint_uv: [0.0, 0.0],
        };
        let prior = vertex([0.0; 3]);
        let body = [vertex([8.0, -1.0, -1.0]), vertex([10.0, 1.0, 1.0])];
        let wall = |p: [i32; 3]| p[0] == 4 && (-1..=0).contains(&p[1]);
        let mut hidden = vec![prior];
        hidden.extend(body);
        assert!(Culler::new([0.0; 3], 1_000).cull_vertices(&mut hidden, 1, wall));
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].pos, prior.pos);

        // A held item or animated part beyond the wall keeps the whole model.
        let mut exposed = vec![prior];
        exposed.extend(body);
        exposed.push(vertex([8.0, 5.0, 0.0]));
        let before = exposed.clone();
        assert!(!Culler::new([0.0; 3], 1_000).cull_vertices(&mut exposed, 1, wall));
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&exposed),
            bytemuck::cast_slice::<_, u8>(&before)
        );
        assert!(!Culler::new([0.0; 3], 0).cull_vertices(&mut exposed, 1, |_| panic!()));
    }

    #[test]
    fn opaque_walls_hide_bounds_in_all_directions() {
        for axis in 0..3 {
            for sign in [-1.0, 1.0] {
                let mut min = [-2.0; 3];
                let mut max = [2.0; 3];
                min[axis] = if sign > 0.0 { 8.0 } else { -10.0 };
                max[axis] = if sign > 0.0 { 10.0 } else { -8.0 };
                let wall = if sign > 0.0 { 4 } else { -5 };
                assert!(Culler::new([0.0; 3], 1_000).occluded(min, max, |p| p[axis] == wall));
            }
        }
    }

    #[test]
    fn windows_slits_and_partial_walls_stay_visible() {
        let min = [8.0, -4.0, -4.0];
        let max = [10.0, 4.0, 4.0];
        for opening in 0..3 {
            assert!(!Culler::new([0.0; 3], 1_000).occluded(min, max, |p| {
                p[0] == 4
                    && match opening {
                        0 => !(p[1] == 1 && p[2] == 1),
                        1 => p[1] != 1,
                        _ => p[1] < 0,
                    }
            }));
        }
        assert!(!Culler::new([0.0; 3], 1_000).occluded(min, max, |_| false));
    }

    #[test]
    fn budgets_are_shared_and_exhaustion_fails_open() {
        let culler = Culler::new([0.0; 3], 3);
        let calls = Cell::new(0);
        for _ in 0..2 {
            assert!(!culler.occluded([8.0; 3], [10.0; 3], |_| {
                calls.set(calls.get() + 1);
                false
            }));
        }
        assert_eq!(calls.get(), 3);
        assert!(!Culler::new([0.0; 3], 0).occluded([8.0; 3], [10.0; 3], |_| panic!()));
        assert!(!Culler::new([0.0; 3], 1).occluded([8.0; 3], [10.0; 3], |_| true));
    }

    #[test]
    fn camera_inside_invalid_and_target_crossing_wall_stay_visible() {
        let culler = Culler::new([0.0; 3], 1_000);
        assert!(!culler.occluded([-1.0; 3], [1.0; 3], |_| panic!()));
        assert!(!culler.occluded([f64::NAN; 3], [1.0; 3], |_| panic!()));
        assert!(!culler.occluded([2.0; 3], [1.0; 3], |_| panic!()));
        assert!(!culler.occluded([4.0, -1.0, -1.0], [8.0, 1.0, 1.0], |p| p[0] == 4));
        assert!(!Culler::new([f64::INFINITY; 3], 1_000).occluded(
            [8.0; 3],
            [10.0; 3],
            |_| panic!()
        ));
        assert!(!Culler::new([f64::MAX; 3], 1_000).occluded(
            [-f64::MAX; 3],
            [-1.0; 3],
            |_| panic!()
        ));
    }
}
