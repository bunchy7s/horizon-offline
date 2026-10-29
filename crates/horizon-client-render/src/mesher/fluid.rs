//! Fluid surfaces following `FluidRenderer.tesselate`: corner heights, face
//! occlusion against neighbour shapes, flow-rotated top UVs, the water overlay
//! beside glass and leaves, back faces, cardinal shading and max(self, above)
//! light.
use super::{
    Info, MeshTables, Padded, SectionMesh, TerrainVertex, lighting, pack_coords, pack_rgb, pi,
};
use horizon_core::{
    geometry::{Axis, Direction, VoxelShape, trig},
    position::BlockPos,
};
use horizon_world::{
    fluid::{FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
};
use std::{convert::Infallible, sync::OnceLock};

/// `FluidRenderer.MAX_FLUID_HEIGHT`.
const MAX_HEIGHT: f32 = 0.888_888_9;
const EPSILON: f32 = 0.001;

/// For the occlusion face toward `direction`: the height from the bottom over
/// which its slice flush with the `direction` side covers the whole width
/// (vertical faces: 1 when the whole square is covered, else 0). This is what
/// `Shapes.blockOccludes` decides for a fluid box of a given height.
pub(super) fn coverage(shape: &VoxelShape, direction: Direction) -> f32 {
    let (axis, u, v) = match direction {
        Direction::Down | Direction::Up => (Axis::Y, Axis::X, Axis::Z),
        Direction::North | Direction::South => (Axis::Z, Axis::X, Axis::Y),
        Direction::West | Direction::East => (Axis::X, Axis::Z, Axis::Y),
    };
    let positive = matches!(
        direction,
        Direction::Up | Direction::South | Direction::East
    );
    let rects: Vec<[f64; 4]> = shape
        .boxes()
        .iter()
        .filter(|b| {
            if positive {
                b.max(axis) >= 1.0 - 1e-7
            } else {
                b.min(axis) <= 1e-7
            }
        })
        .map(|b| [b.min(u), b.max(u), b.min(v), b.max(v)])
        .collect();
    let mut cuts: Vec<f64> = rects
        .iter()
        .flat_map(|r| [r[2], r[3]])
        .filter(|c| (0.0..=1.0).contains(c))
        .chain([0.0, 1.0])
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut covered = 0.0;
    for w in cuts.windows(2) {
        let middle = (w[0] + w[1]) / 2.0;
        let mut spans: Vec<(f64, f64)> = rects
            .iter()
            .filter(|r| r[2] <= middle && middle <= r[3])
            .map(|r| (r[0], r[1]))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach = 0.0f64;
        for (lo, hi) in spans {
            if lo > reach + 1e-7 {
                break;
            }
            reach = reach.max(hi);
        }
        if reach < 1.0 - 1e-7 {
            break;
        }
        covered = w[1];
    }
    if matches!(direction, Direction::Down | Direction::Up) && covered < 1.0 {
        return 0.0;
    }
    covered as f32
}
/// `isFaceOccludedByState` for a fluid box `height` tall against `state`'s
/// face opposite `direction`.
fn occluded_by(direction: Direction, height: f32, state: &Info) -> bool {
    let opposite = direction.opposite() as usize;
    match state.face[opposite] {
        0 => false,
        1 => direction != Direction::Up || height == 1.0,
        _ => match direction {
            Direction::Up => {
                f64::from(height) >= 1.0 - 1e-7 && state.cover[Direction::Down as usize] >= 1.0
            }
            Direction::Down => state.cover[Direction::Up as usize] >= 1.0,
            _ => f64::from(height) <= f64::from(state.cover[opposite]) + 1e-7,
        },
    }
}
/// `isFaceOccludedBySelf`: the cell's own occlusion face toward `direction`.
fn occluded_by_self(state: &Info, direction: Direction) -> bool {
    match state.face[direction as usize] {
        0 => false,
        1 => true,
        _ => state.cover[direction as usize] >= 1.0,
    }
}
/// `Mth.atan2`: the client's table-driven arctangent.
fn atan2(mut y: f64, mut x: f64) -> f64 {
    static TABLES: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
    let (asin, cos) = TABLES.get_or_init(|| {
        (0..257)
            .map(|i| {
                let a = (f64::from(i) / 256.0).asin();
                (a, a.cos())
            })
            .unzip()
    });
    let frac_bias = f64::from_bits(4_805_340_802_404_319_232);
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let negative_y = y < 0.0;
    if negative_y {
        y = -y;
    }
    let negative_x = x < 0.0;
    if negative_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    // Mth.fastInvSqrt
    let half = 0.5 * d2;
    let mut r = f64::from_bits((6_910_469_410_427_058_090i64 - (d2.to_bits() as i64 >> 1)) as u64);
    r *= 1.5 - half * r * r;
    x *= r;
    y *= r;
    let yp = frac_bias + y;
    let index = yp.to_bits() as i32 as usize;
    let s_phi = yp - frac_bias;
    let sd = y * cos[index] - x * s_phi;
    let mut theta = asin[index] + (6.0 + sd * sd) * sd * 0.166_666_666_666_666_66;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if negative_x {
        theta = std::f64::consts::PI - theta;
    }
    if negative_y {
        theta = -theta;
    }
    theta
}
/// The padded section as `FlowingFluid.getFlow` sees it (section-relative positions).
struct FlowView<'a> {
    pad: &'a Padded,
    info: &'a [Info],
}
impl FlowView<'_> {
    fn id(&self, p: BlockPos) -> u32 {
        self.pad.blocks[pi(p.x, p.y, p.z)]
    }
}
impl FluidFlowWorld for FlowView<'_> {
    type Error = Infallible;
    type Block = u32;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Infallible> {
        Ok(self.info[self.id(p) as usize].fluid)
    }
    fn block(&mut self, p: BlockPos) -> Result<u32, Infallible> {
        Ok(self.id(p))
    }
    fn blocks_fluid_flow(&self, state: u32) -> bool {
        self.info[state as usize].blocks_flow
    }
    fn is_ice_class(&self, state: u32) -> bool {
        self.info[state as usize].ice
    }
    fn is_face_sturdy(
        &mut self,
        state: u32,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Infallible> {
        Ok(self.info[state as usize].sturdy & 1 << d as u8 != 0)
    }
}
/// Emits one fluid cell at section-relative `[x, y, z]`.
pub(super) fn tesselate(
    tables: &MeshTables,
    nether: bool,
    pad: &Padded,
    water_color: u32,
    out: &mut SectionMesh,
    [x, y, z]: [i32; 3],
) {
    use horizon_assets::model::Direction as D;
    let info = &tables.info;
    let at = |p: [i32; 3]| &info[pad.blocks[pi(p[0], p[1], p[2])] as usize];
    let me = at([x, y, z]);
    let kind = me.fluid.kind();
    let same = |s: &Info| s.fluid.kind().same_family(kind);
    let neighbor = |d: Direction| {
        let [dx, dy, dz] = d.normal();
        at([x + dx, y + dy, z + dz])
    };
    let (down, up) = (neighbor(Direction::Down), neighbor(Direction::Up));
    let should_render = |d: Direction| !same(neighbor(d)) && !occluded_by_self(me, d);
    let render_up = !same(up);
    let render_down =
        should_render(Direction::Down) && !occluded_by(Direction::Down, MAX_HEIGHT, down);
    let sides = [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ]
    .map(should_render);
    if !render_up && !render_down && !sides.iter().any(|&s| s) {
        return;
    }
    let water = matches!(kind, FluidKind::Water | FluidKind::FlowingWater);
    let tint = if water {
        water_color & 0xFF_FFFF
    } else {
        0xFF_FFFF
    };
    // Lava emits light level 15.
    let emission = if water { 0 } else { 15 };
    let color = |by: f32| {
        let rgb = [16, 8, 0].map(|shift| lighting::scale((tint >> shift & 255) as i32, by) as u32);
        crate::mesher::emitting(pack_rgb(rgb), emission)
    };
    let height = |p: [i32; 3]| -> f32 {
        let s = at(p);
        if same(s) {
            if same(at([p[0], p[1] + 1, p[2]])) {
                1.0
            } else {
                s.fluid.own_height()
            }
        } else if !s.legacy_solid {
            0.0
        } else {
            -1.0
        }
    };
    let own = height([x, y, z]);
    let (mut ne, mut nw, mut se, mut sw) = (1.0f32, 1.0f32, 1.0f32, 1.0f32);
    if own < 1.0 {
        let n = height([x, y, z - 1]);
        let s = height([x, y, z + 1]);
        let e = height([x + 1, y, z]);
        let w = height([x - 1, y, z]);
        // calculateAverageHeight(self, height2, height1, corner)
        let average = |h2: f32, h1: f32, corner: [i32; 3]| -> f32 {
            if h1 >= 1.0 || h2 >= 1.0 {
                return 1.0;
            }
            let mut sum = [0f32; 2];
            let mut add = |h: f32| {
                if h >= 0.8 {
                    sum[0] += h * 10.0;
                    sum[1] += 10.0;
                } else if h >= 0.0 {
                    sum[0] += h;
                    sum[1] += 1.0;
                }
            };
            if h1 > 0.0 || h2 > 0.0 {
                let hc = height(corner);
                if hc >= 1.0 {
                    return 1.0;
                }
                add(hc);
            }
            add(own);
            add(h1);
            add(h2);
            sum[0] / sum[1]
        };
        ne = average(n, e, [x + 1, y, z - 1]);
        nw = average(n, w, [x - 1, y, z - 1]);
        se = average(s, e, [x + 1, y, z + 1]);
        sw = average(s, w, [x - 1, y, z + 1]);
    }
    let light = |p: [i32; 3]| {
        let above = [p[0], p[1] + 1, p[2]];
        pack_coords(lighting::max_coords(
            lighting::coords(pad, at(p), p),
            lighting::coords(pad, at(above), above),
        ))
    };
    let (still, flow, overlay) = if water {
        (
            tables.water_still,
            tables.water_flow,
            Some(tables.water_overlay),
        )
    } else {
        (tables.lava_still, tables.lava_flow, None)
    };
    let target = if water {
        &mut out.translucent
    } else {
        &mut out.solid
    };
    let mut quad =
        |v: [([f32; 3], [f32; 2]); 4], layer: u16, color: u32, light: u16, back: bool| {
            let vertex = |(pos, uv)| TerrainVertex::new(pos, uv, layer, color, light);
            target.extend(v.map(vertex));
            if back {
                target.extend([v[0], v[3], v[2], v[1]].map(vertex));
            }
        };
    let (fx, fy, fz) = (x as f32, y as f32, z as f32);
    let bottom = if render_down { EPSILON } else { 0.0 };
    if render_up && !occluded_by(Direction::Up, nw.min(sw).min(se.min(ne)), up) {
        nw -= EPSILON;
        sw -= EPSILON;
        se -= EPSILON;
        ne -= EPSILON;
        let flow_vector = flow_at(
            &mut FlowView { pad, info },
            me.fluid,
            BlockPos::new(x, y, z),
        )
        .unwrap_or_else(|e| match e {});
        let (layer, uv) = if flow_vector.x == 0.0 && flow_vector.z == 0.0 {
            (still, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
        } else {
            let angle =
                atan2(flow_vector.z, flow_vector.x) as f32 - std::f64::consts::FRAC_PI_2 as f32;
            let s = trig::sin(f64::from(angle)) * 0.25;
            let c = trig::cos(f64::from(angle)) * 0.25;
            (
                flow,
                [
                    [0.5 + (-c - s), 0.5 + (-c + s)],
                    [0.5 + (-c + s), 0.5 + (c + s)],
                    [0.5 + (c + s), 0.5 + (c - s)],
                    [0.5 + (c - s), 0.5 + (-c - s)],
                ],
            )
        };
        // FluidState.shouldRenderBackwardUpFace over the 3x3 cells above.
        let backward = (-1..=1).any(|ox| {
            (-1..=1).any(|oz| {
                let s = at([x + ox, y + 1, z + oz]);
                !same(s) && !s.solid
            })
        });
        quad(
            [
                ([fx, fy + nw, fz], uv[0]),
                ([fx, fy + sw, fz + 1.0], uv[1]),
                ([fx + 1.0, fy + se, fz + 1.0], uv[2]),
                ([fx + 1.0, fy + ne, fz], uv[3]),
            ],
            layer,
            color(lighting::cardinal(nether, D::Up)),
            light([x, y, z]),
            backward,
        );
    }
    if render_down {
        let y0 = fy + bottom;
        quad(
            [
                ([fx, y0, fz], [0.0, 0.0]),
                ([fx + 1.0, y0, fz], [1.0, 0.0]),
                ([fx + 1.0, y0, fz + 1.0], [1.0, 1.0]),
                ([fx, y0, fz + 1.0], [0.0, 1.0]),
            ],
            still,
            color(lighting::cardinal(nether, D::Down)),
            light([x, y - 1, z]),
            false,
        );
    }
    let side_light = light([x, y, z]);
    let up_shade = lighting::cardinal(nether, D::Up);
    for (i, d) in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ]
    .into_iter()
    .enumerate()
    {
        let (h0, h1, [x0, z0], [x1, z1]) = match d {
            Direction::North => (nw, ne, [fx, fz + EPSILON], [fx + 1.0, fz + EPSILON]),
            Direction::South => (
                se,
                sw,
                [fx + 1.0, fz + 1.0 - EPSILON],
                [fx, fz + 1.0 - EPSILON],
            ),
            Direction::West => (sw, nw, [fx + EPSILON, fz + 1.0], [fx + EPSILON, fz]),
            _ => (
                ne,
                se,
                [fx + 1.0 - EPSILON, fz],
                [fx + 1.0 - EPSILON, fz + 1.0],
            ),
        };
        let face_state = neighbor(d);
        if !sides[i] || occluded_by(d, h0.max(h1), face_state) {
            continue;
        }
        let (layer, is_overlay) = match overlay {
            Some(layer) if face_state.overlay => (layer, true),
            _ => (flow, false),
        };
        let side = if matches!(d, Direction::North | Direction::South) {
            lighting::cardinal(nether, D::North)
        } else {
            lighting::cardinal(nether, D::West)
        };
        let (v0, v1) = ((1.0 - h0) * 0.5, (1.0 - h1) * 0.5);
        quad(
            [
                ([x0, fy + h0, z0], [0.0, v0]),
                ([x1, fy + h1, z1], [0.5, v1]),
                ([x1, fy + bottom, z1], [0.5, 0.5]),
                ([x0, fy + bottom, z0], [0.0, 0.5]),
            ],
            layer,
            color(up_shade * side),
            side_light,
            !is_overlay,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atan2_follows_the_table_approximation() {
        for (y, x) in [
            (1.0, 0.0),
            (0.0, 1.0),
            (-0.5, 0.25),
            (0.3, -0.9),
            (-1.0, -1.0),
            (0.1, 0.7),
        ] {
            let exact = f64::atan2(y, x);
            assert!((atan2(y, x) - exact).abs() < 1e-6, "{y} {x}");
        }
        assert!(atan2(f64::NAN, 1.0).is_nan());
    }
    #[test]
    fn coverage_of_full_and_partial_faces() {
        assert_eq!(coverage(&VoxelShape::block(), Direction::North), 1.0);
        assert_eq!(coverage(&VoxelShape::block(), Direction::Up), 1.0);
        assert_eq!(coverage(&VoxelShape::empty(), Direction::East), 0.0);
        // A bottom slab covers the lower half of each side and the whole floor.
        let slab = VoxelShape::from_cube([1, 2, 1], &[true, false]).unwrap();
        assert_eq!(coverage(&slab, Direction::South), 0.5);
        assert_eq!(coverage(&slab, Direction::Down), 1.0);
        assert_eq!(coverage(&slab, Direction::Up), 0.0);
    }
}
