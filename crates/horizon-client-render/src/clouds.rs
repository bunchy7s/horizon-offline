//! Cloud visibility and flat/fancy cell geometry (`CloudRenderer`).
/// `LevelRenderer`: skip OFF clouds and colors with no packed alpha.
pub fn visible(mode: &str, alpha: f32) -> bool {
    mode != "false" && (alpha.clamp(0.0, 1.0) * 255.0) as u8 > 0
}

/// `buildFlatCell`: bottom plane, in both windings for the culled cloud pipeline.
pub fn flat_cell(x: f32, z: f32, bottom: f32) -> [[[f32; 3]; 4]; 2] {
    let p = [
        [x + 12.0, bottom, z],
        [x + 12.0, bottom, z + 12.0],
        [x, bottom, z + 12.0],
        [x, bottom, z],
    ];
    [p, [p[3], p[2], p[1], p[0]]]
}

/// Direction order is DOWN, UP, NORTH, SOUTH, WEST, EAST; inward faces use bits 6..12.
pub fn face_mask(x: i32, z: i32, bottom: f32, empty: [bool; 4]) -> u16 {
    let mut mask = 0;
    if bottom <= 0.0 {
        mask |= 1 << 1;
    }
    if bottom + 4.0 >= 0.0 {
        mask |= 1;
    }
    for (i, visible) in [z > 0, z < 0, x > 0, x < 0].into_iter().enumerate() {
        if empty[i] && visible {
            mask |= 1 << (i + 2);
        }
    }
    if x.abs() <= 1 && z.abs() <= 1 {
        mask |= 63 << 6;
    }
    mask
}

/// Exterior faces first, then reversed interior faces, in vanilla emission order.
pub fn faces(mask: u16) -> impl Iterator<Item = (u8, bool)> {
    [1, 0, 2, 3, 4, 5]
        .into_iter()
        .filter(move |d| mask & (1 << d) != 0)
        .map(|d| (d, false))
        .chain(
            (0..6)
                .filter(move |d| mask & (1 << (d + 6)) != 0)
                .map(|d| (d, true)),
        )
}

/// `CloudRenderer.prepare`: the cell radius for the Cloud Distance option
/// (in chunks), independent of the render distance.
pub fn radius_cells(cloud_range_chunks: i32) -> i32 {
    ((cloud_range_chunks * 16) as f32 / 12.0).ceil() as i32
}

/// CloudRenderer builds cells in expanding Manhattan rings, nearest first.
pub fn cells(radius: i32) -> impl Iterator<Item = (i32, i32)> {
    (0..=2 * radius).flat_map(move |ring| {
        (-ring..=ring).flat_map(move |x| {
            let z = ring - x.abs();
            let visible = z <= radius && x * x + z * z <= radius * radius;
            (visible && z != 0)
                .then_some((x, -z))
                .into_iter()
                .chain(visible.then_some((x, z)))
        })
    })
}

/// Per-frame cloud inputs. Resources are immutable for the renderer's lifetime:
/// resource-pack reloads replace the renderer and its cache together.
pub struct Input<'a> {
    pub texture: &'a (u32, u32, Vec<bool>),
    pub uv: [f32; 2],
    pub camera: glam::DVec3,
    pub time: i64,
    pub partial_tick: f64,
    pub range: i32,
    pub fast: bool,
    pub color: [f32; 4],
}

#[derive(Debug, PartialEq, Eq)]
struct MeshKey {
    cell_x: i64,
    cell_z: i64,
    radius: i32,
    fast: bool,
    // The two vertical face tests include both surfaces at the boundaries.
    top: bool,
    bottom: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniform {
    offset: [f32; 4],
    colors: [[f32; 4]; 4],
}

impl Input<'_> {
    fn state(&self) -> (MeshKey, Uniform) {
        let (w, h, _) = self.texture;
        let offset = self.time.rem_euclid(i64::from(*w) * 400) as f64 + self.partial_tick;
        let mut x = self.camera.x + offset * 0.030_000_001;
        let mut z = self.camera.z + 3.96;
        let (tw, th) = (f64::from(*w) * 12.0, f64::from(*h) * 12.0);
        x -= (x / tw).floor() * tw;
        z -= (z / th).floor() * th;
        let cell_x = (x / 12.0).floor() as i64;
        let cell_z = (z / 12.0).floor() as i64;
        let bottom = (192.33 - self.camera.y) as f32;
        let key = MeshKey {
            cell_x,
            cell_z,
            radius: radius_cells(self.range),
            fast: self.fast,
            top: bottom <= 0.0,
            bottom: bottom + 4.0 >= 0.0,
        };
        let colors = [1.0, 0.7, 0.8, 0.9].map(|shade| {
            let [r, g, b, a] = self.color;
            [r * shade, g * shade, b * shade, a]
                .map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8 as f32 / 255.0)
        });
        (
            key,
            Uniform {
                offset: [
                    -(x - cell_x as f64 * 12.0) as f32,
                    bottom,
                    -(z - cell_z as f64 * 12.0) as f32,
                    0.0,
                ],
                colors,
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct Mesh {
    key: Option<MeshKey>,
    pub vertices: Vec<crate::mesher::Vertex>,
}

impl Mesh {
    /// Returns true only when vertices must be uploaded. Sub-cell motion and
    /// color changes update only the uniform, as in CloudRenderer.prepare.
    pub fn prepare(&mut self, input: &Input<'_>) -> (bool, Uniform) {
        let (key, uniform) = input.state();
        if self.key.as_ref() == Some(&key) {
            return (false, uniform);
        }
        self.vertices.clear();
        let (w, h, cells) = input.texture;
        let (w, h) = (i64::from(*w), i64::from(*h));
        let (cell_x, cell_z, radius) = (key.cell_x, key.cell_z, key.radius);
        let bottom = uniform.offset[1];
        let uv = input.uv;
        let filled = |cx: i64, cz: i64| cells[(cz.rem_euclid(h) * w + cx.rem_euclid(w)) as usize];
        for (dx, dz) in self::cells(radius) {
            let (dx, dz) = (i64::from(dx), i64::from(dz));
            let (cx, cz) = (cell_x + dx, cell_z + dz);
            if !filled(cx, cz) {
                continue;
            }
            let x0 = dx as f32 * 12.0;
            let z0 = dz as f32 * 12.0;
            let (x1, z1, y0, y1) = (x0 + 12.0, z0 + 12.0, 0.0, 4.0);
            let mut quad = |p: [[f32; 3]; 4], shade: u32| {
                for v in p {
                    // Overlay 4: clouds (unlit, faded by the cloud fog end).
                    self.vertices.push(crate::mesher::Vertex {
                        normal: [0; 4],
                        pos: v,
                        uv,
                        layer: 4,
                        color: u32::MAX,
                        light: shade,

                        glint_uv: [0.0, 0.0],
                    });
                }
            };
            if input.fast {
                for p in flat_cell(x0, z0, 0.0) {
                    quad(p, 0);
                }
                continue;
            }
            let quads = [
                ([[x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y0, z0]], 1),
                ([[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]], 0),
                ([[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]], 2),
                ([[x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]], 2),
                ([[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]], 3),
                ([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], 3),
            ];
            let mask = face_mask(
                dx as i32,
                dz as i32,
                bottom,
                [
                    !filled(cx, cz - 1),
                    !filled(cx, cz + 1),
                    !filled(cx - 1, cz),
                    !filled(cx + 1, cz),
                ],
            );
            for (direction, inside) in faces(mask) {
                let (p, shade) = quads[usize::from(direction)];
                quad(if inside { [p[3], p[2], p[1], p[0]] } else { p }, shade);
            }
        }
        self.key = Some(key);
        (true, uniform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_mesh_tracks_geometry_but_not_subcell_motion_or_color() {
        let texture = (8, 8, (0..64).map(|i| i % 3 != 0).collect());
        let mut input = Input {
            texture: &texture,
            uv: [0.5; 2],
            camera: glam::DVec3::new(1.0, 100.0, 1.0),
            time: 0,
            partial_tick: 0.0,
            range: 2,
            fast: false,
            color: [1.0; 4],
        };
        let mut mesh = Mesh::default();
        assert!(mesh.prepare(&input).0);
        let vertices = bytemuck::cast_slice::<_, u8>(&mesh.vertices).to_vec();
        let pointer = mesh.vertices.as_ptr();
        for tick in 0..100 {
            input.time = tick;
            input.partial_tick = 0.5;
            input.camera.x = 2.0;
            input.camera.y = 101.0;
            input.color = [0.8, 0.7, 0.6, 0.5];
            let (rebuilt, uniform) = mesh.prepare(&input);
            assert!(!rebuilt);
            assert_eq!(mesh.vertices.as_ptr(), pointer);
            assert_eq!(bytemuck::cast_slice::<_, u8>(&mesh.vertices), vertices);
            assert_eq!(uniform.offset[1], (192.33 - input.camera.y) as f32);
            assert_eq!(
                uniform.colors[1],
                [0.8 * 0.7, 0.7 * 0.7, 0.6 * 0.7, 0.5].map(|v| (v * 255.0) as u8 as f32 / 255.0)
            );
            for color in uniform.colors {
                assert_eq!(color[3], 127.0 / 255.0);
            }
        }
        for (x, y, z, range, fast) in [
            (13.0, 101.0, 1.0, 2, false),
            (13.0, 101.0, -5.0, 2, false),
            (13.0, 192.33, -5.0, 2, false),
            (13.0, 196.34, -5.0, 2, false),
            (13.0, 196.34, -5.0, 3, false),
            (13.0, 196.34, -5.0, 3, true),
        ] {
            input.camera = glam::DVec3::new(x, y, z);
            input.range = range;
            input.fast = fast;
            assert!(mesh.prepare(&input).0);
            let mut fresh = Mesh::default();
            fresh.prepare(&input);
            assert_eq!(
                bytemuck::cast_slice::<_, u8>(&mesh.vertices),
                bytemuck::cast_slice::<_, u8>(&fresh.vertices)
            );
            assert!(!mesh.prepare(&input).0);
        }
        // Drift alone crosses a cell, even with a stationary camera.
        input.time += 400;
        assert!(mesh.prepare(&input).0);
        // Resource reload constructs a new renderer and cache, including empty packs.
        let mut reloaded = Mesh::default();
        let empty = (1, 1, vec![false]);
        input.texture = &empty;
        assert!(reloaded.prepare(&input).0);
        assert!(reloaded.vertices.is_empty());
        assert!(!reloaded.prepare(&input).0);
    }

    #[test]
    fn settings_and_packed_alpha_gate_clouds() {
        for mode in ["false", "fast", "true"] {
            assert_eq!(visible(mode, 1.0), mode != "false");
            assert!(!visible(mode, 0.0));
            assert!(!visible(mode, 0.5 / 255.0));
        }
        assert!(visible("fast", 1.0 / 255.0));
    }

    #[test]
    fn fast_cells_are_flat_at_the_bottom_and_visible_from_both_sides() {
        for bottom in [-10.0, 0.0, 10.0] {
            let [down, up] = flat_cell(-3.0, 7.0, bottom);
            assert_eq!(
                down,
                [
                    [9.0, bottom, 7.0],
                    [9.0, bottom, 19.0],
                    [-3.0, bottom, 19.0],
                    [-3.0, bottom, 7.0],
                ]
            );
            assert_eq!(up, [down[3], down[2], down[1], down[0]]);
            for (quad, sign) in [(down, -1.0), (up, 1.0)] {
                let [a, b, c] = [quad[0], quad[1], quad[2]].map(glam::Vec3::from_array);
                assert!((b - a).cross(c - a).y * sign > 0.0);
            }
        }
    }

    #[test]
    fn cells_are_nearest_first_and_unique() {
        let cells: Vec<_> = cells(2).collect();
        assert_eq!(cells[0], (0, 0));
        assert_eq!(cells.len(), 13);
        assert!(
            cells
                .windows(2)
                .all(|w| w[0].0.abs() + w[0].1.abs() <= w[1].0.abs() + w[1].1.abs())
        );
        assert_eq!(
            cells.iter().collect::<std::collections::HashSet<_>>().len(),
            cells.len()
        );
    }
    #[test]
    fn only_camera_facing_exterior_faces() {
        assert_eq!(face_mask(2, 2, 10.0, [true; 4]), 1 | 4 | 16);
        assert_eq!(face_mask(-2, -2, -5.0, [true; 4]), 2 | 8 | 32);
        assert_eq!(face_mask(2, 2, 0.0, [false; 4]), 3);
    }
    #[test]
    fn cloud_distance_sets_the_radius() {
        assert_eq!(radius_cells(2), 3);
        assert_eq!(radius_cells(12), 16);
        assert_eq!(radius_cells(128), 171);
    }
    #[test]
    fn connected_cells_have_no_exterior_walls() {
        assert_eq!(face_mask(2, 2, 10.0, [false; 4]), 1);
    }
    #[test]
    fn nearby_cells_keep_inward_faces_for_camera_inside_clouds() {
        assert_eq!(face_mask(0, 0, 0.0, [false; 4]), 3 | (63 << 6));
    }
}
