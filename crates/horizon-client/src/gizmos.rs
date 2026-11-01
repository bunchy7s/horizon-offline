//! Debug gizmos (26.3 `net.minecraft.gizmos`): coloured lines with a width
//! in pixels, as `GizmoFeatureRenderer.buildLines` draws them, and the two
//! debug renderers the F3 keys switch on: `ChunkBorderRenderer` (F3+G) and
//! `EntityHitboxDebugRenderer` (F3+B).

/// A line: world-space ends, ARGB colour, width in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub color: u32,
    pub width: f32,
}

/// The gizmos of a frame: depth-tested lines, and lines drawn over
/// everything (`GizmoProperties.setAlwaysOnTop`).
#[derive(Clone, Debug, Default)]
pub struct Gizmos {
    pub lines: Vec<Line>,
    pub on_top: Vec<Line>,
}

/// `CommonColors`.
pub const WHITE: u32 = 0xFFFF_FFFF;
pub const RED: u32 = 0xFFFF_0000;
pub const BLUE: u32 = 0xFF00_00FF;
pub const YELLOW: u32 = 0xFFFF_FF00;
/// `ChunkBorderRenderer.CELL_BORDER`, `YELLOW`, `MAJOR_LINES` and the outer
/// columns' `ARGB.colorFromFloat(0.5F, 1.0F, 0.0F, 0.0F)`.
pub const CELL_BORDER: u32 = 0xFF00_9B9B;
pub const CHUNK_YELLOW: u32 = 0xFFFF_FF00;
pub const MAJOR_LINES: u32 = 0xFF3F_3FFF;
pub const OUTER_COLUMNS: u32 = 0x7FFF_0000;
/// `ARGB.colorFromFloat(1.0F, 0.25F, 1.0F, 0.0F)`: dragon parts.
pub const DRAGON_PART: u32 = 0xFF3F_FF00;
/// `BlockEntityWithBoundingBoxRenderer.InvisibleBlockType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvisibleBlock {
    Air,
    StructureVoid,
    Barrier,
    Light,
}
impl InvisibleBlock {
    /// The marker's stroke: `colorFromFloat(1, 0.5, 0.5, 1)`, `(1, 1, 0.75,
    /// 0.75)`, `-65536` and `-256`.
    pub fn color(self) -> u32 {
        match self {
            Self::Air => 0xFF7F_7FFF,
            Self::StructureVoid => 0xFFFF_BFBF,
            Self::Barrier => 0xFFFF_0000,
            Self::Light => 0xFFFF_FF00,
        }
    }
}
/// `ARGB.colorFromFloat(1.0F, 0.9F, 0.9F, 0.9F)`: a block entity's box.
pub const BOUNDING_BOX: u32 = 0xFFE5_E5E5;
/// `GizmoStyle.stroke(argb)` and `Gizmos.arrow(start, end, argb)`.
pub const STROKE_WIDTH: f32 = 2.5;

impl Gizmos {
    /// `DebugCrosshairRenderer`: the world axes one block in front of the
    /// camera, `0.01 * gui_scale` long, X red, Y green, Z light blue, each a
    /// 2 pixel line over a 4 pixel black one. Drawn on top, in that order.
    pub fn debug_crosshair(&mut self, camera: [f64; 3], x_rot: f32, y_rot: f32, gui_scale: f32) {
        let forward = view_vector(x_rot, y_rot);
        let center: [f64; 3] = std::array::from_fn(|i| camera[i] + forward[i]);
        let length = f64::from(0.01 * gui_scale);
        let axes = [(0, 0xFFFF_0000), (1, 0xFF00_FF00), (2, 0xFF7F_7FFF)];
        let end = |axis: usize| {
            let mut p = center;
            p[axis] += length;
            p
        };
        for (axis, _) in axes {
            self.on_top.push(Line {
                start: center,
                end: end(axis),
                color: 0xFF00_0000,
                width: 4.0,
            });
        }
        for (axis, color) in axes {
            self.on_top.push(Line {
                start: center,
                end: end(axis),
                color,
                width: 2.0,
            });
        }
    }
    /// `Gizmos.line`.
    pub fn line(&mut self, start: [f64; 3], end: [f64; 3], color: u32, width: f32) {
        self.lines.push(Line {
            start,
            end,
            color,
            width,
        });
    }
    /// `CuboidGizmo.emit` with a stroke style: the twelve edges in its order.
    pub fn cuboid(&mut self, min: [f64; 3], max: [f64; 3], color: u32, width: f32) {
        let edges = cuboid_edges(min, max);
        self.lines.extend(edges.iter().map(|&(start, end)| Line {
            start,
            end,
            color,
            width,
        }));
    }
    /// `CuboidGizmo.emit` with a stroke style drawn on top of everything
    /// (`Gizmos.cuboid(aabb, style, true)`).
    pub fn cuboid_on_top(&mut self, min: [f64; 3], max: [f64; 3], color: u32, width: f32) {
        let edges = cuboid_edges(min, max);
        self.on_top.extend(edges.iter().map(|&(start, end)| Line {
            start,
            end,
            color,
            width,
        }));
    }
    /// `BlockEntityWithBoundingBoxRenderer.submit`: the box at `block` +
    /// `local` of `size` blocks (nothing when a side is empty), then the
    /// markers of the invisible blocks in it, indexed `z * sx * sy + y * sx + x`.
    pub fn bounding_box(
        &mut self,
        block: [i32; 3],
        local: [i32; 3],
        size: [i32; 3],
        invisible: Option<&[Option<InvisibleBlock>]>,
    ) {
        if size.iter().any(|s| *s < 1) {
            return;
        }
        let at = |i: usize| f64::from(block[i] + local[i]);
        let far = |i: usize| f64::from(block[i] + local[i] + size[i]);
        // `ARGB.colorFromFloat(1.0F, 0.9F, 0.9F, 0.9F)`.
        self.cuboid_on_top(
            [at(0), at(1), at(2)],
            [far(0), far(1), far(2)],
            BOUNDING_BOX,
            STROKE_WIDTH,
        );
        let Some(invisible) = invisible else {
            return;
        };
        let (sx, sy) = (size[0] as usize, size[1] as usize);
        for x in 0..size[0] {
            for y in 0..size[1] {
                for z in 0..size[2] {
                    let index = z as usize * sx * sy + y as usize * sx + x as usize;
                    let Some(Some(kind)) = invisible.get(index) else {
                        continue;
                    };
                    // Air gets bigger markers: 0.45 - 0.05 to 0.55 + 0.05.
                    let scale = if *kind == InvisibleBlock::Air {
                        0.05f32
                    } else {
                        0.0
                    };
                    let corner = |i: usize, d: i32, sign: f32| {
                        f64::from(
                            (block[i] + local[i] + d) as f32
                                + if sign < 0.0 {
                                    0.45 - scale
                                } else {
                                    0.55 + scale
                                },
                        )
                    };
                    let min = [corner(0, x, -1.0), corner(1, y, -1.0), corner(2, z, -1.0)];
                    let max = [corner(0, x, 1.0), corner(1, y, 1.0), corner(2, z, 1.0)];
                    self.cuboid(min, max, kind.color(), STROKE_WIDTH);
                }
            }
        }
    }
    /// `ArrowGizmo.emit`: the shaft, then four tips around the end.
    pub fn arrow(&mut self, start: [f64; 3], end: [f64; 3], color: u32, width: f32) {
        self.line(start, end, color, width);
        let d = glam::DVec3::from(end) - glam::DVec3::from(start);
        let dir = d.as_vec3().normalize();
        // Quaternionf.rotationTo((1, 0, 0), direction).
        let rotation = glam::Quat::from_rotation_arc(glam::Vec3::X, dir);
        // (float) Mth.clamp(distance * 0.1, 0.1, 1.0).
        let len = (d.length() * f64::from(0.1f32)).clamp(f64::from(0.1f32), 1.0) as f32;
        for tip in [
            glam::Vec3::new(-len, len, 0.0),
            glam::Vec3::new(-len, 0.0, len),
            glam::Vec3::new(-len, -len, 0.0),
            glam::Vec3::new(-len, 0.0, -len),
        ] {
            let t = rotation * tip;
            let from = [
                end[0] + f64::from(t.x),
                end[1] + f64::from(t.y),
                end[2] + f64::from(t.z),
            ];
            self.line(from, end, color, width);
        }
    }
    /// `ChunkBorderRenderer.emitGizmos` around the camera entity's block.
    pub fn chunk_borders(&mut self, camera_block: [i32; 3], min_y: i32, max_y: i32) {
        let x0 = f64::from(camera_block[0] & !15);
        let z0 = f64::from(camera_block[2] & !15);
        let section_y = camera_block[1] & !15;
        let (ymin, ymax) = (min_y as f32 as f64, (max_y + 1) as f32 as f64);
        let column = |g: &mut Self, x: f64, z: f64, color: u32, width: f32| {
            g.line([x, ymin, z], [x, ymax, z], color, width);
        };
        for x in (-16..=32).step_by(16) {
            for z in (-16..=32).step_by(16) {
                column(
                    self,
                    x0 + f64::from(x),
                    z0 + f64::from(z),
                    OUTER_COLUMNS,
                    4.0,
                );
            }
        }
        for x in (2..16).step_by(2) {
            let color = if x % 4 == 0 {
                CELL_BORDER
            } else {
                CHUNK_YELLOW
            };
            column(self, x0 + f64::from(x), z0, color, 1.0);
            column(self, x0 + f64::from(x), z0 + 16.0, color, 1.0);
        }
        for z in (2..16).step_by(2) {
            let color = if z % 4 == 0 {
                CELL_BORDER
            } else {
                CHUNK_YELLOW
            };
            column(self, x0, z0 + f64::from(z), color, 1.0);
            column(self, x0 + 16.0, z0 + f64::from(z), color, 1.0);
        }
        let ring = |g: &mut Self, y: f64, color: u32, width: f32| {
            g.line([x0, y, z0], [x0, y, z0 + 16.0], color, width);
            g.line([x0, y, z0 + 16.0], [x0 + 16.0, y, z0 + 16.0], color, width);
            g.line([x0 + 16.0, y, z0 + 16.0], [x0 + 16.0, y, z0], color, width);
            g.line([x0 + 16.0, y, z0], [x0, y, z0], color, width);
        };
        let mut y = min_y;
        while y <= max_y + 1 {
            let color = if y % 8 == 0 {
                CELL_BORDER
            } else {
                CHUNK_YELLOW
            };
            ring(self, f64::from(y as f32), color, 1.0);
            y += 2;
        }
        for x in [0.0, 16.0] {
            for z in [0.0, 16.0] {
                column(self, x0 + x, z0 + z, MAJOR_LINES, 4.0);
            }
        }
        // The camera's section, always on top.
        let min = [x0, f64::from(section_y), z0];
        let max = [x0 + 16.0, f64::from(section_y + 16), z0 + 16.0];
        self.on_top
            .extend(cuboid_edges(min, max).iter().map(|&(start, end)| Line {
                start,
                end,
                color: MAJOR_LINES,
                width: 1.0,
            }));
        let mut y = min_y;
        while y <= max_y + 1 {
            ring(self, f64::from(y), MAJOR_LINES, 4.0);
            y += 16;
        }
    }
    /// `EntityHitboxDebugRenderer.showHitboxes` for a client entity:
    /// `position` is the interpolated position, `size` (width, height).
    /// `eye` is the eye height and view rotations (x, y) for living
    /// entities; other entities still get the view arrow from `view`.
    pub fn hitbox(
        &mut self,
        position: [f64; 3],
        size: (f32, f32),
        eye_height: f32,
        living: bool,
        view: (f32, f32),
    ) {
        let half = f64::from(size.0) / 2.0;
        let min = [position[0] - half, position[1], position[2] - half];
        let max = [
            position[0] + half,
            position[1] + f64::from(size.1),
            position[2] + half,
        ];
        self.entity_box(min, max, position, eye_height, living, view);
    }
    /// The same for an entity whose box is not centred on its position
    /// (a painting or an item frame): `min` and `max` are the box.
    pub fn entity_box(
        &mut self,
        min: [f64; 3],
        max: [f64; 3],
        position: [f64; 3],
        eye_height: f32,
        living: bool,
        view: (f32, f32),
    ) {
        self.cuboid(min, max, WHITE, STROKE_WIDTH);
        if living {
            let eye = min[1] + f64::from(eye_height);
            let d = 0.009999999776482582;
            self.cuboid(
                [min[0], eye - d, min[2]],
                [max[0], eye + d, max[2]],
                RED,
                STROKE_WIDTH,
            );
        }
        let eye = [
            position[0],
            position[1] + f64::from(eye_height),
            position[2],
        ];
        let v = view_vector(view.0, view.1);
        let end = [
            eye[0] + v[0] * 2.0,
            eye[1] + v[1] * 2.0,
            eye[2] + v[2] * 2.0,
        ];
        self.arrow(eye, end, BLUE, STROKE_WIDTH);
    }
}

/// `CuboidGizmo.emit`'s fill: the six quads (+X, -X, -Z, +Z, +Y, -Y), each
/// counter-clockwise seen from outside.
pub fn cuboid_fill_quads(min: [f64; 3], max: [f64; 3]) -> [[[f64; 3]; 4]; 6] {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    [
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
    ]
}

/// `ARGB.colorFromFloat(0.375F, 1.0F, 0.0F, 0.0F)`: the fill of an error marker.
pub const ERROR_MARKER_FILL: u32 = 0x5FFF_0000;
/// `TestInstanceRenderer.submitErrorMarker`'s billboard text scale.
pub const ERROR_MARKER_TEXT_SCALE: f32 = 0.16;

/// `TestInstanceRenderer.submitErrorMarker`: the marker's box, `new
/// AABB(pos).inflate(0.02F)`, and the centre of its text,
/// `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.2, 0.5)`.
pub fn error_marker_shape(pos: [i32; 3]) -> ([f64; 3], [f64; 3], [f64; 3]) {
    let pad = f64::from(0.02f32);
    (
        pos.map(|v| f64::from(v) - pad),
        pos.map(|v| f64::from(v) + 1.0 + pad),
        [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.2,
            f64::from(pos[2]) + 0.5,
        ],
    )
}

/// `GizmoFeatureRenderer.buildTexts`: the world size of one font pixel,
/// `scale / 16`.
pub fn billboard_text_unit(scale: f32) -> f32 {
    scale / 16.0
}

/// The edges of `CuboidGizmo.emit`'s stroke, in order.
fn cuboid_edges(min: [f64; 3], max: [f64; 3]) -> [([f64; 3], [f64; 3]); 12] {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    [
        ([x0, y0, z0], [x1, y0, z0]),
        ([x0, y0, z0], [x0, y1, z0]),
        ([x0, y0, z0], [x0, y0, z1]),
        ([x1, y0, z0], [x1, y1, z0]),
        ([x1, y1, z0], [x0, y1, z0]),
        ([x0, y1, z0], [x0, y1, z1]),
        ([x0, y1, z1], [x0, y0, z1]),
        ([x0, y0, z1], [x1, y0, z1]),
        ([x1, y0, z1], [x1, y0, z0]),
        ([x0, y1, z1], [x1, y1, z1]),
        ([x1, y0, z1], [x1, y1, z1]),
        ([x1, y1, z0], [x1, y1, z1]),
    ]
}

/// `Entity.calculateViewVector(xRot, yRot)`.
pub fn view_vector(x_rot: f32, y_rot: f32) -> [f64; 3] {
    use horizon_core::geometry::trig::{cos, sin};
    let x = f64::from(x_rot * (std::f32::consts::PI / 180.0));
    let y = f64::from(-y_rot * (std::f32::consts::PI / 180.0));
    let (yc, ys, xc, xs) = (cos(y), sin(y), cos(x), sin(x));
    [f64::from(ys * xc), f64::from(-xs), f64::from(yc * xc)]
}

/// `GizmoFeatureRenderer.buildLines`: camera-relative ends, clipped at the
/// near plane in view space (z = -0.05), as (start, end, colour, width).
pub fn camera_relative(lines: &[Line], camera: [f64; 3], view: glam::Mat4) -> Vec<[f32; 8]> {
    let mut out = Vec::with_capacity(lines.len());
    for l in lines {
        let rel = |p: [f64; 3]| {
            glam::Vec3::new(
                (p[0] - camera[0]) as f32,
                (p[1] - camera[1]) as f32,
                (p[2] - camera[2]) as f32,
            )
        };
        let (mut start, mut end) = (rel(l.start), rel(l.end));
        let (sv, ev) = (view.transform_point3(start), view.transform_point3(end));
        let (start_behind, end_behind) = (sv.z > -0.05, ev.z > -0.05);
        if start_behind && end_behind {
            continue;
        }
        if start_behind || end_behind {
            let denom = ev.z - sv.z;
            if denom.abs() >= 1.0e-9 {
                let t = ((-0.05 - sv.z) / denom).clamp(0.0, 1.0);
                let cut = start.lerp(end, t);
                if start_behind {
                    start = cut;
                } else {
                    end = cut;
                }
            }
        }
        out.push([
            start.x,
            start.y,
            start.z,
            end.x,
            end.y,
            end.z,
            f32::from_bits(l.color),
            l.width,
        ]);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_error_marker_is_a_filled_box_with_text_above_it() {
        let (min, max, text) = error_marker_shape([10, 64, -3]);
        let pad = f64::from(0.02f32);
        assert_eq!(min, [10.0 - pad, 64.0 - pad, -3.0 - pad]);
        assert_eq!(max, [11.0 + pad, 65.0 + pad, -2.0 + pad]);
        assert_eq!(text, [10.5, 65.2, -2.5]);
        // 0.375 of 255 floors to 95; the colour is red.
        assert_eq!(ERROR_MARKER_FILL, 0x5FFF_0000);
        assert_eq!(billboard_text_unit(ERROR_MARKER_TEXT_SCALE), 0.01);
        let quads = cuboid_fill_quads(min, max);
        assert_eq!(quads.len(), 6);
        // Every quad is flat on a face of the box and counter-clockwise from outside.
        for q in quads {
            let n = (glam::DVec3::from(q[1]) - glam::DVec3::from(q[0]))
                .cross(glam::DVec3::from(q[2]) - glam::DVec3::from(q[1]));
            let centre = q
                .iter()
                .fold(glam::DVec3::ZERO, |a, p| a + glam::DVec3::from(*p))
                / 4.0;
            let middle = (glam::DVec3::from(min) + glam::DVec3::from(max)) / 2.0;
            assert!(n.dot(centre - middle) > 0.0);
        }
        assert_eq!(quads[0][0], [max[0], min[1], min[2]]);
    }

    use super::*;

    #[test]
    fn debug_crosshair_is_the_world_axes_ahead_of_the_camera() {
        let mut g = Gizmos::default();
        // Looking south (yaw 0) at GUI scale 2: one block towards +Z.
        g.debug_crosshair([0.5, 64.0, 0.5], 0.0, 0.0, 2.0);
        assert!(g.lines.is_empty());
        let ends: Vec<([f64; 3], u32, f32)> =
            g.on_top.iter().map(|l| (l.end, l.color, l.width)).collect();
        assert_eq!(g.on_top[0].start, [0.5, 64.0, 1.5]);
        // CROSSHAIR_SCALE * guiScale, in floats as vanilla scales.
        let l = f64::from(0.01f32 * 2.0);
        let (x, y, z) = (
            [0.5 + l, 64.0, 1.5],
            [0.5, 64.0 + l, 1.5],
            [0.5, 64.0, 1.5 + l],
        );
        assert_eq!(
            ends,
            [
                (x, 0xFF00_0000, 4.0),
                (y, 0xFF00_0000, 4.0),
                (z, 0xFF00_0000, 4.0),
                (x, 0xFFFF_0000, 2.0),
                (y, 0xFF00_FF00, 2.0),
                (z, 0xFF7F_7FFF, 2.0),
            ]
        );
    }

    #[test]
    fn chunk_borders_match_the_renderer() {
        let mut g = Gizmos::default();
        g.chunk_borders([37, 70, -5], -64, 319);
        // 4 * 4 outer columns, 2 * 7 + 2 * 7 inner columns, a ring every 2
        // blocks, 4 major columns, a major ring every 16.
        let rings = (-64..=320).step_by(2).count();
        let majors = (-64..=320).step_by(16).count();
        assert_eq!(g.lines.len(), 16 + 28 + rings * 4 + 4 + majors * 4);
        assert_eq!(g.on_top.len(), 12);
        let first = g.lines[0];
        assert_eq!(first.start, [16.0, -64.0, -32.0]);
        assert_eq!(first.end, [16.0, 320.0, -32.0]);
        assert_eq!((first.color, first.width), (OUTER_COLUMNS, 4.0));
        // The section box around y 70.
        assert_eq!(g.on_top[0].start, [32.0, 64.0, -16.0]);
        assert_eq!(g.on_top[11].end, [48.0, 80.0, 0.0]);
        // y 0 and 8 are cell-coloured rings, y 2 yellow.
        let ring_at = |y: f64| {
            g.lines
                .iter()
                .find(|l| l.start[1] == y && l.end[1] == y)
                .unwrap()
        };
        assert_eq!(ring_at(8.0).color, CELL_BORDER);
        assert_eq!(ring_at(2.0).color, CHUNK_YELLOW);
    }

    #[test]
    fn hitbox_box_eye_line_and_view_arrow() {
        let mut g = Gizmos::default();
        g.hitbox([0.5, 64.0, 0.5], (0.6, 1.8), 1.62, true, (0.0, 0.0));
        assert_eq!(g.lines.len(), 12 + 12 + 5);
        // EntityDimensions.makeBoundingBox: the float half width.
        let corner = 0.5 - f64::from(0.6f32 / 2.0);
        assert_eq!(g.lines[0].start, [corner, 64.0, corner]);
        assert_eq!(g.lines[12].color, RED);
        // Looking south (yRot 0): the arrow points +z.
        let arrow = g.lines[24];
        assert_eq!(arrow.color, BLUE);
        assert!((arrow.end[2] - 2.5).abs() < 1e-6 && (arrow.end[0] - 0.5).abs() < 1e-6);
        let mut g = Gizmos::default();
        g.hitbox([0.0, 0.0, 0.0], (0.25, 0.25), 0.2125, false, (0.0, 0.0));
        assert_eq!(g.lines.len(), 12 + 5);
    }

    #[test]
    fn lines_behind_the_camera_are_clipped_at_the_near_plane() {
        let view = glam::Mat4::IDENTITY; // looking down -z
        let lines = [
            Line {
                start: [0.0, 0.0, -2.0],
                end: [0.0, 0.0, 2.0],
                color: WHITE,
                width: 1.0,
            },
            Line {
                start: [0.0, 0.0, 1.0],
                end: [1.0, 0.0, 2.0],
                color: WHITE,
                width: 1.0,
            },
        ];
        let out = camera_relative(&lines, [0.0; 3], view);
        assert_eq!(out.len(), 1);
        assert!((out[0][5] + 0.05).abs() < 1e-6);
    }

    #[test]
    fn bounding_box_draws_the_box_on_top_and_markers_for_invisible_blocks() {
        let mut g = Gizmos::default();
        let mut invisible = vec![None; 2];
        invisible[1] = Some(InvisibleBlock::Barrier);
        g.bounding_box([10, 64, 10], [1, 0, 0], [2, 1, 1], Some(&invisible));
        // The box: twelve edges, from the corner to the far corner.
        assert_eq!(g.on_top.len(), 12);
        assert_eq!(g.on_top[0].start, [11.0, 64.0, 10.0]);
        assert_eq!(g.on_top[0].end, [13.0, 64.0, 10.0]);
        assert!(g.on_top.iter().all(|l| l.color == BOUNDING_BOX));
        // The barrier at index 1 (x = 1): 0.45 to 0.55 inside its block, red.
        assert_eq!(g.lines.len(), 12);
        let l = &g.lines[0];
        assert_eq!(l.color, 0xFFFF_0000);
        assert!((l.start[0] - 12.45).abs() < 1e-6 && (l.end[0] - 12.55).abs() < 1e-6);
        // An empty side draws nothing.
        let mut none = Gizmos::default();
        none.bounding_box([0; 3], [0; 3], [0, 3, 3], None);
        assert!(none.on_top.is_empty() && none.lines.is_empty());
    }

    #[test]
    fn air_markers_are_larger_than_the_others() {
        let mut g = Gizmos::default();
        g.bounding_box(
            [0; 3],
            [0; 3],
            [1, 1, 1],
            Some(&[Some(InvisibleBlock::Air)]),
        );
        let (min, max) = (g.lines[0].start[0], g.lines[0].end[0]);
        assert!((min - 0.4).abs() < 1e-6 && (max - 0.6).abs() < 1e-6);
        assert_eq!(g.lines[0].color, 0xFF7F_7FFF);
    }
}
