//! The world border wall (`WorldBorderRenderer`, `WorldBorderRenderState`):
//! which border the camera is near, the wall's fade and status colour, the
//! cached vertex buffer of the four walls, the scrolling texture transform
//! and which walls are drawn. Kept free of GPU types so the parity fixture
//! can compare every value with vanilla's renderer.
use crate::worldborder::{WorldBorder, jclamp, jmax, jmin, status_color};

/// `WorldBorderRenderer.RENDERING_OFFSET`.
const OFFSET: f64 = 0.01;

/// `WorldBorderRenderState`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BorderRenderState {
    pub min_x: f64,
    pub max_x: f64,
    pub min_z: f64,
    pub max_z: f64,
    /// `BorderStatus.getColor`, set only when the wall is visible.
    pub tint: u32,
    pub alpha: f64,
}
impl BorderRenderState {
    /// `closestBorder`: the four walls by distance from (x, z), nearest
    /// first (a stable sort of north, south, west, east), as
    /// `Direction.get2DDataValue` side indices and distances.
    pub fn closest_border(&self, x: f64, z: f64) -> [(usize, f64); 4] {
        let mut sides = [
            (2, z - self.min_z),
            (0, self.max_z - z),
            (1, x - self.min_x),
            (3, self.max_x - x),
        ];
        // Comparator.comparingDouble: Double.compare order, stable.
        sides.sort_by(|a, b| java_double_compare(a.1, b.1));
        sides
    }
}
/// `Double.compare`.
fn java_double_compare(a: f64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if a < b {
        Less
    } else if a > b {
        Greater
    } else {
        let bits = |v: f64| {
            if v.is_nan() {
                0x7ff8_0000_0000_0000u64 as i64
            } else {
                v.to_bits() as i64
            }
        };
        bits(a).cmp(&bits(b))
    }
}

/// `WorldBorderRenderer.extract`.
pub fn extract(
    border: &WorldBorder,
    partial: f32,
    camera: [f64; 3],
    render_distance: f64,
    state: &mut BorderRenderState,
) {
    let [min_x, min_z, max_x, max_z] = border.bounds_at(partial);
    state.min_x = min_x;
    state.max_x = max_x;
    state.min_z = min_z;
    state.max_z = max_z;
    let (x, z) = (camera[0], camera[2]);
    let deep_inside = x < max_x - render_distance
        && x > min_x + render_distance
        && z < max_z - render_distance
        && z > min_z + render_distance;
    let far_outside = x < min_x - render_distance
        || x > max_x + render_distance
        || z < min_z - render_distance
        || z > max_z + render_distance;
    if !deep_inside && !far_outside {
        let alpha = 1.0 - border.distance(x, z) / render_distance;
        state.alpha = jclamp(horizon_core::jmath::pow(alpha, 4.0), 0.0, 1.0);
        state.tint = status_color(border.status());
    } else {
        state.alpha = 0.0;
    }
}

/// One `POSITION_TEX` vertex of the wall buffer: position relative to the
/// buffer origin and texture coordinates.
pub type BorderVertex = [f32; 5];

/// What `WorldBorderRenderer.prepareDynamicTransforms` writes: the colour
/// modulator, the model offset of the buffer origin from the camera and
/// the texture matrix translation (both u and v).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderTransforms {
    pub color: [f32; 4],
    pub model_offset: [f32; 3],
    pub texture_offset: f32,
}

/// `WorldBorderRenderer`'s cached wall buffer.
#[derive(Clone, Debug)]
pub struct BorderRenderer {
    needs_rebuild: bool,
    /// Set when `prepare` rebuilt the buffer (cleared by [`Self::take_rebuilt`]).
    rebuilt: bool,
    last_min_x: f64,
    last_min_z: f64,
    last_border: [f64; 4],
    /// Four quads: south, west, north, east (`get2DDataValue` order).
    pub vertices: [BorderVertex; 16],
}
impl Default for BorderRenderer {
    fn default() -> Self {
        Self {
            needs_rebuild: true,
            rebuilt: false,
            last_min_x: 0.0,
            last_min_z: 0.0,
            last_border: [0.0; 4],
            vertices: [[0.0; 5]; 16],
        }
    }
}
/// `Mth.floor(double)`.
fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < f64::from(i) { i - 1 } else { i }
}
/// `Mth.ceil(double)`.
fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > f64::from(i) { i + 1 } else { i }
}
/// `Mth.frac(double)`.
fn frac(v: f64) -> f64 {
    v - v.floor()
}
impl BorderRenderer {
    /// `invalidate`: rebuild on the next prepare (e.g. after a resource reload).
    pub fn invalidate(&mut self) {
        self.needs_rebuild = true;
    }
    /// `prepare`: rebuilds the buffer when the border moved; returns whether
    /// the wall is drawn this frame.
    pub fn prepare(
        &mut self,
        state: &BorderRenderState,
        camera: [f64; 3],
        render_distance: f64,
        depth_far: f32,
    ) -> bool {
        if state.alpha <= 0.0 {
            return false;
        }
        let half_height = depth_far;
        let v0 = (-frac(camera[1] * 0.5)) as f32;
        let v1 = v0 + half_height;
        let border = [state.min_x, state.min_z, state.max_x, state.max_z];
        if self.needs_rebuild || border != self.last_border {
            self.rebuild(
                state,
                render_distance,
                camera[2],
                camera[0],
                half_height,
                v1,
                v0,
            );
        }
        true
    }
    /// `rebuildWorldBorderBuffer`.
    #[allow(clippy::too_many_arguments)]
    fn rebuild(
        &mut self,
        state: &BorderRenderState,
        render_distance: f64,
        camera_z: f64,
        camera_x: f64,
        h: f32,
        v1: f32,
        v0: f32,
    ) {
        let border_min_x = state.min_x + OFFSET;
        let border_max_x = state.max_x - OFFSET;
        let border_min_z = state.min_z + OFFSET;
        let border_max_z = state.max_z - OFFSET;
        let min_z = jmax(f64::from(floor(camera_z - render_distance)), border_min_z);
        let max_z = jmin(f64::from(ceil(camera_z + render_distance)), border_max_z);
        let u0z = (floor(min_z) & 1) as f32 * 0.5;
        let u1z = (max_z - min_z) as f32 / 2.0;
        let min_x = jmax(f64::from(floor(camera_x - render_distance)), border_min_x);
        let max_x = jmin(f64::from(ceil(camera_x + render_distance)), border_max_x);
        let u0x = (floor(min_x) & 1) as f32 * 0.5;
        let u1x = (max_x - min_x) as f32 / 2.0;
        let (dx, dz) = ((max_x - min_x) as f32, (max_z - min_z) as f32);
        let south = (border_max_z - min_z) as f32;
        let east = (border_max_x - min_x) as f32;
        self.vertices = [
            [0.0, -h, south, u0x, v1],
            [dx, -h, south, u1x + u0x, v1],
            [dx, h, south, u1x + u0x, v0],
            [0.0, h, south, u0x, v0],
            [0.0, -h, 0.0, u0z, v1],
            [0.0, -h, dz, u1z + u0z, v1],
            [0.0, h, dz, u1z + u0z, v0],
            [0.0, h, 0.0, u0z, v0],
            [dx, -h, 0.0, u0x, v1],
            [0.0, -h, 0.0, u1x + u0x, v1],
            [0.0, h, 0.0, u1x + u0x, v0],
            [dx, h, 0.0, u0x, v0],
            [east, -h, dz, u0z, v1],
            [east, -h, 0.0, u1z + u0z, v1],
            [east, h, 0.0, u1z + u0z, v0],
            [east, h, dz, u0z, v0],
        ];
        self.last_border = [state.min_x, state.min_z, state.max_x, state.max_z];
        self.last_min_x = min_x;
        self.last_min_z = min_z;
        self.needs_rebuild = false;
        self.rebuilt = true;
    }
    /// Whether the buffer was rebuilt since the last call (it must be
    /// uploaded again).
    pub fn take_rebuilt(&mut self) -> bool {
        std::mem::take(&mut self.rebuilt)
    }
    /// `prepareDynamicTransforms` at `Util.getMillis()` = `millis`.
    pub fn transforms(
        &self,
        state: &BorderRenderState,
        camera: [f64; 3],
        millis: i64,
    ) -> BorderTransforms {
        let channel = |shift: u32| ((state.tint >> shift) & 0xFF) as f32 / 255.0;
        let offset = (millis % 3000) as f32 / 3000.0;
        BorderTransforms {
            color: [channel(16), channel(8), channel(0), state.alpha as f32],
            model_offset: [
                (self.last_min_x - camera[0]) as f32,
                (-camera[1]) as f32,
                (self.last_min_z - camera[2]) as f32,
            ],
            texture_offset: offset,
        }
    }
    /// `draw`: the walls within the render distance, nearest first, as side
    /// indices into [`Self::vertices`] (four vertices each).
    pub fn sides(state: &BorderRenderState, camera: [f64; 3], render_distance: f64) -> Vec<usize> {
        state
            .closest_border(camera[0], camera[2])
            .into_iter()
            .filter(|&(_, d)| d < render_distance)
            .map(|(side, _)| side)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_wall_is_drawn_and_tinted() {
        let mut border = WorldBorder::default();
        border.set_size(100.0);
        let mut state = BorderRenderState::default();
        let camera = [40.0, 64.0, 0.0];
        extract(&border, 0.0, camera, 128.0, &mut state);
        assert!(state.alpha > 0.0);
        assert_eq!(state.tint, 0x20A0FF);
        let mut r = BorderRenderer::default();
        assert!(r.prepare(&state, camera, 128.0, 512.0));
        // East wall first (10 blocks away), then north and south, then west.
        assert_eq!(
            BorderRenderer::sides(&state, camera, 128.0),
            vec![3, 2, 0, 1]
        );
        let t = r.transforms(&state, camera, 4500);
        assert_eq!(t.texture_offset, 0.5);
        // Far inside a huge border, nothing is drawn.
        border.set_size(10_000.0);
        extract(&border, 0.0, camera, 128.0, &mut state);
        assert_eq!(state.alpha, 0.0);
        assert!(!r.prepare(&state, camera, 128.0, 512.0));
    }
}
