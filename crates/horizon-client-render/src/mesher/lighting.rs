//! Vertex light and colour of model quads, following `BlockModelLighter`
//! (smooth light and ambient occlusion, including the partial-face weights)
//! and `ModelBlockRenderer`'s integer colour arithmetic (`ARGB.gray`,
//! `scaleRGB`, `multiply`).
use super::{Info, Padded, pi};
use horizon_assets::model::Direction as D;

/// `LightCoordsUtil.FULL_BRIGHT`.
pub(super) const FULL_BRIGHT: u32 = 0x00F0_00F0;

/// `CardinalLighting.DEFAULT`/`NETHER` by face.
pub(super) fn cardinal(nether: bool, face: D) -> f32 {
    match face {
        D::Down if nether => 0.9,
        D::Up if nether => 0.9,
        D::Down => 0.5,
        D::Up => 1.0,
        D::North | D::South => 0.8,
        D::West | D::East => 0.6,
    }
}
fn normal(face: D) -> [i32; 3] {
    match face {
        D::Down => [0, -1, 0],
        D::Up => [0, 1, 0],
        D::North => [0, 0, -1],
        D::South => [0, 0, 1],
        D::West => [-1, 0, 0],
        D::East => [1, 0, 0],
    }
}
pub(super) fn step(p: [i32; 3], face: D) -> [i32; 3] {
    let n = normal(face);
    [p[0] + n[0], p[1] + n[1], p[2] + n[2]]
}
fn state<'a>(pad: &Padded, info: &'a [Info], p: [i32; 3]) -> &'a Info {
    &info[pad.blocks[pi(p[0], p[1], p[2])] as usize]
}
/// `LightCoordsUtil.getLightCoords`: light at `p` as seen through `state`
/// (block level << 4 | sky level << 20).
pub(super) fn coords(pad: &Padded, state: &Info, p: [i32; 3]) -> u32 {
    packed_coords(state, pad.light[pi(p[0], p[1], p[2])])
}
/// `coords` of a stored light byte (sky level << 4 | block level).
pub(super) fn packed_coords(state: &Info, packed: u8) -> u32 {
    if state.emissive {
        return FULL_BRIGHT;
    }
    let block = u32::from((packed & 15).max(state.emission));
    block << 4 | u32::from(packed >> 4) << 20
}
/// `LightCoordsUtil.max`.
pub(super) fn max_coords(a: u32, b: u32) -> u32 {
    (a >> 4 & 15).max(b >> 4 & 15) << 4 | (a >> 20 & 15).max(b >> 20 & 15) << 20
}
/// `LightCoordsUtil.lightCoordsWithEmission` (a quad's element light emission).
pub(super) fn with_emission(coords: u32, emission: u8) -> u32 {
    if emission == 0 {
        return coords;
    }
    let e = u32::from(emission);
    (coords >> 4 & 15).max(e) << 4 | (coords >> 20 & 15).max(e) << 20
}
/// `LightCoordsUtil.smoothBlend`: zero samples take the centre; samples without
/// sky light borrow the centre's sky.
pub(super) fn smooth_blend(mut a: u32, mut b: u32, mut c: u32, center: u32) -> u32 {
    if center >> 20 & 15 > 2 || center >> 4 & 15 > 2 {
        for n in [&mut a, &mut b, &mut c] {
            if *n == 0 {
                *n = center;
            } else if *n >> 20 & 15 == 0 {
                *n |= center & 0x00FF_0000;
            }
        }
    }
    (a + b + c + center) >> 2 & 0x00FF_00FF
}
/// `LightCoordsUtil.smoothWeightedBlend` (Java float sums and int narrowing).
fn weighted_blend(c: [u32; 4], w: [f32; 4]) -> u32 {
    let channel = |shift: u32| {
        let v = |i: usize| (c[i] >> shift & 0xFF) as f32 * w[i];
        (v(0) + v(1) + v(2) + v(3)) as i32 as u32 & 0xFF
    };
    channel(0) | channel(16) << 16
}
/// `ARGB.gray` channel: `Mth.floor(brightness * 255)`.
pub(super) fn gray(brightness: f32) -> i32 {
    (brightness * 255.0).floor() as i32
}
/// One channel of `ARGB.scaleRGB(color, float)`.
pub(super) fn scale(channel: i32, by: f32) -> i32 {
    ((channel as f32 * by) as i32).clamp(0, 255)
}
// SizeInfo indices into the face shape.
const DOWN: u8 = 0;
const UP: u8 = 1;
const NORTH: u8 = 2;
const SOUTH: u8 = 3;
const WEST: u8 = 4;
const EAST: u8 = 5;
const FLIP_DOWN: u8 = 6;
const FLIP_UP: u8 = 7;
const FLIP_NORTH: u8 = 8;
const FLIP_SOUTH: u8 = 9;
const FLIP_WEST: u8 = 10;
const FLIP_EAST: u8 = 11;
struct Adjacency {
    corners: [D; 4],
    /// Face-shape factor pairs for each vertex's four corner weights.
    weights: [[u8; 8]; 4],
    /// Output vertex for each of the four computed corners.
    remap: [usize; 4],
}
const fn adjacency(face: D) -> Adjacency {
    use D::*;
    match face {
        Down => Adjacency {
            corners: [West, East, North, South],
            weights: [
                [
                    FLIP_WEST, SOUTH, FLIP_WEST, FLIP_SOUTH, WEST, FLIP_SOUTH, WEST, SOUTH,
                ],
                [
                    FLIP_WEST, NORTH, FLIP_WEST, FLIP_NORTH, WEST, FLIP_NORTH, WEST, NORTH,
                ],
                [
                    FLIP_EAST, NORTH, FLIP_EAST, FLIP_NORTH, EAST, FLIP_NORTH, EAST, NORTH,
                ],
                [
                    FLIP_EAST, SOUTH, FLIP_EAST, FLIP_SOUTH, EAST, FLIP_SOUTH, EAST, SOUTH,
                ],
            ],
            remap: [0, 1, 2, 3],
        },
        Up => Adjacency {
            corners: [East, West, North, South],
            weights: [
                [
                    EAST, SOUTH, EAST, FLIP_SOUTH, FLIP_EAST, FLIP_SOUTH, FLIP_EAST, SOUTH,
                ],
                [
                    EAST, NORTH, EAST, FLIP_NORTH, FLIP_EAST, FLIP_NORTH, FLIP_EAST, NORTH,
                ],
                [
                    WEST, NORTH, WEST, FLIP_NORTH, FLIP_WEST, FLIP_NORTH, FLIP_WEST, NORTH,
                ],
                [
                    WEST, SOUTH, WEST, FLIP_SOUTH, FLIP_WEST, FLIP_SOUTH, FLIP_WEST, SOUTH,
                ],
            ],
            remap: [2, 3, 0, 1],
        },
        North => Adjacency {
            corners: [Up, Down, East, West],
            weights: [
                [UP, FLIP_WEST, UP, WEST, FLIP_UP, WEST, FLIP_UP, FLIP_WEST],
                [UP, FLIP_EAST, UP, EAST, FLIP_UP, EAST, FLIP_UP, FLIP_EAST],
                [
                    DOWN, FLIP_EAST, DOWN, EAST, FLIP_DOWN, EAST, FLIP_DOWN, FLIP_EAST,
                ],
                [
                    DOWN, FLIP_WEST, DOWN, WEST, FLIP_DOWN, WEST, FLIP_DOWN, FLIP_WEST,
                ],
            ],
            remap: [3, 0, 1, 2],
        },
        South => Adjacency {
            corners: [West, East, Down, Up],
            weights: [
                [UP, FLIP_WEST, FLIP_UP, FLIP_WEST, FLIP_UP, WEST, UP, WEST],
                [
                    DOWN, FLIP_WEST, FLIP_DOWN, FLIP_WEST, FLIP_DOWN, WEST, DOWN, WEST,
                ],
                [
                    DOWN, FLIP_EAST, FLIP_DOWN, FLIP_EAST, FLIP_DOWN, EAST, DOWN, EAST,
                ],
                [UP, FLIP_EAST, FLIP_UP, FLIP_EAST, FLIP_UP, EAST, UP, EAST],
            ],
            remap: [0, 1, 2, 3],
        },
        West => Adjacency {
            corners: [Up, Down, North, South],
            weights: [
                [
                    UP, SOUTH, UP, FLIP_SOUTH, FLIP_UP, FLIP_SOUTH, FLIP_UP, SOUTH,
                ],
                [
                    UP, NORTH, UP, FLIP_NORTH, FLIP_UP, FLIP_NORTH, FLIP_UP, NORTH,
                ],
                [
                    DOWN, NORTH, DOWN, FLIP_NORTH, FLIP_DOWN, FLIP_NORTH, FLIP_DOWN, NORTH,
                ],
                [
                    DOWN, SOUTH, DOWN, FLIP_SOUTH, FLIP_DOWN, FLIP_SOUTH, FLIP_DOWN, SOUTH,
                ],
            ],
            remap: [3, 0, 1, 2],
        },
        East => Adjacency {
            corners: [Down, Up, North, South],
            weights: [
                [
                    FLIP_DOWN, SOUTH, FLIP_DOWN, FLIP_SOUTH, DOWN, FLIP_SOUTH, DOWN, SOUTH,
                ],
                [
                    FLIP_DOWN, NORTH, FLIP_DOWN, FLIP_NORTH, DOWN, FLIP_NORTH, DOWN, NORTH,
                ],
                [
                    FLIP_UP, NORTH, FLIP_UP, FLIP_NORTH, UP, FLIP_NORTH, UP, NORTH,
                ],
                [
                    FLIP_UP, SOUTH, FLIP_UP, FLIP_SOUTH, UP, FLIP_SOUTH, UP, SOUTH,
                ],
            ],
            remap: [1, 2, 3, 0],
        },
    }
}
const ADJACENCY: [Adjacency; 6] = [
    adjacency(D::Down),
    adjacency(D::Up),
    adjacency(D::North),
    adjacency(D::South),
    adjacency(D::West),
    adjacency(D::East),
];
/// `prepareQuadShape`: the quad's bounds, whether it is partial (smaller than
/// the block face) and whether it is cubic (flush with the block boundary, or
/// flat on a full collision shape).
struct Shape {
    bounds: [f32; 12],
    partial: bool,
    cubic: bool,
}
fn shape(positions: &[[f32; 3]; 4], face: D, full: bool) -> Shape {
    let (mut min, mut max) = ([32f32; 3], [-32f32; 3]);
    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    let mut bounds = [0f32; 12];
    for (index, value) in [
        (WEST, min[0]),
        (EAST, max[0]),
        (DOWN, min[1]),
        (UP, max[1]),
        (NORTH, min[2]),
        (SOUTH, max[2]),
    ] {
        bounds[index as usize] = value;
        bounds[index as usize + 6] = 1.0 - value;
    }
    let (lo, hi) = (1.0e-4f32, 0.9999f32);
    let partial = |a: usize, b: usize| min[a] >= lo || min[b] >= lo || max[a] <= hi || max[b] <= hi;
    let (partial, cubic) = match face {
        D::Down => (partial(0, 2), min[1] == max[1] && (min[1] < lo || full)),
        D::Up => (partial(0, 2), min[1] == max[1] && (max[1] > hi || full)),
        D::North => (partial(0, 1), min[2] == max[2] && (min[2] < lo || full)),
        D::South => (partial(0, 1), min[2] == max[2] && (max[2] > hi || full)),
        D::West => (partial(1, 2), min[0] == max[0] && (min[0] < lo || full)),
        D::East => (partial(1, 2), min[0] == max[0] && (max[0] > hi || full)),
    };
    Shape {
        bounds,
        partial,
        cubic,
    }
}
/// `prepareQuadFlat` for a quad without a cull face: light from the
/// neighbour when the quad is flush with the face, else from the block itself.
pub(super) fn flat(
    pad: &Padded,
    own: &Info,
    pos: [i32; 3],
    positions: &[[f32; 3]; 4],
    face: D,
) -> u32 {
    coords(
        pad,
        own,
        flat_position(pos, positions, face, None, own.full),
    )
}
/// Where `tesselateFlat` reads the light of a quad of a block at `pos`: the
/// neighbour on its cull face, else `prepareQuadFlat`'s choice (the neighbour
/// when the quad is cubic, else the block itself). `full` is the block's
/// `isCollisionShapeFullBlock`.
pub(super) fn flat_position(
    pos: [i32; 3],
    positions: &[[f32; 3]; 4],
    face: D,
    cull: Option<D>,
    full: bool,
) -> [i32; 3] {
    match cull {
        Some(d) => step(pos, d),
        None if shape(positions, face, full).cubic => step(pos, face),
        None => pos,
    }
}
/// `prepareQuadAmbientOcclusion`: per-vertex AO brightness (before `ARGB.gray`)
/// and smooth light coords, by vertex index.
pub(super) fn ambient_occlusion(
    pad: &Padded,
    info: &[Info],
    own: &Info,
    pos: [i32; 3],
    positions: &[[f32; 3]; 4],
    face: D,
) -> ([f32; 4], [u32; 4]) {
    let shape = shape(positions, face, own.full);
    let base = if shape.cubic { step(pos, face) } else { pos };
    let adjacency = &ADJACENCY[face as usize];
    let corner = |i: usize| step(base, adjacency.corners[i]);
    let sample = |p: [i32; 3]| {
        let s = state(pad, info, p);
        (coords(pad, s, p), s.shade)
    };
    let (light0, shade0) = sample(corner(0));
    let (light1, shade1) = sample(corner(1));
    let (light2, shade2) = sample(corner(2));
    let (light3, shade3) = sample(corner(3));
    let permeable = |i: usize| state(pad, info, step(corner(i), face)).permeable;
    let (p0, p1, p2, p3) = (permeable(0), permeable(1), permeable(2), permeable(3));
    let diagonal = |a: usize, b: usize, open: bool| {
        if open {
            sample(step(corner(a), adjacency.corners[b]))
        } else {
            (light0, shade0)
        }
    };
    let (light02, shade02) = diagonal(0, 2, p2 || p0);
    let (light03, shade03) = diagonal(0, 3, p3 || p0);
    let (light12, shade12) = diagonal(1, 2, p2 || p1);
    let (light13, shade13) = diagonal(1, 3, p3 || p1);
    let next = step(pos, face);
    let next_state = state(pad, info, next);
    let light_center = if shape.cubic || !next_state.solid {
        coords(pad, next_state, next)
    } else {
        coords(pad, own, pos)
    };
    let shade_center = if shape.cubic {
        state(pad, info, base).shade
    } else {
        own.shade
    };
    let ao = [
        (shade3 + shade0 + shade03 + shade_center) * 0.25,
        (shade2 + shade0 + shade02 + shade_center) * 0.25,
        (shade2 + shade1 + shade12 + shade_center) * 0.25,
        (shade3 + shade1 + shade13 + shade_center) * 0.25,
    ];
    let blended = [
        smooth_blend(light3, light0, light03, light_center),
        smooth_blend(light2, light0, light02, light_center),
        smooth_blend(light2, light1, light12, light_center),
        smooth_blend(light3, light1, light13, light_center),
    ];
    let mut brightness = [0f32; 4];
    let mut light = [0u32; 4];
    for (k, &vertex) in adjacency.remap.iter().enumerate() {
        if shape.partial {
            let f = |i: usize| shape.bounds[adjacency.weights[k][i] as usize];
            let w = [f(0) * f(1), f(2) * f(3), f(4) * f(5), f(6) * f(7)];
            brightness[vertex] =
                (ao[0] * w[0] + ao[1] * w[1] + ao[2] * w[2] + ao[3] * w[3]).clamp(0.0, 1.0);
            light[vertex] = weighted_blend(blended, w);
        } else {
            brightness[vertex] = ao[k];
            light[vertex] = blended[k];
        }
    }
    (brightness, light)
}

#[cfg(test)]
mod tests {
    use super::*;
    const fn packed(block: u32, sky: u32) -> u32 {
        block << 4 | sky << 20
    }
    #[test]
    fn smooth_blend_replaces_dark_samples_with_the_centre() {
        let center = packed(2, 15);
        // A zero sample takes the centre; a sky-less sample borrows its sky.
        let blended = smooth_blend(0, packed(8, 0), packed(4, 15), center);
        assert_eq!(blended & 0xFF, (32 + 128 + 64 + 32) / 4);
        assert_eq!(blended >> 16 & 0xFF, 240);
        // A dark centre leaves samples untouched.
        assert_eq!(
            smooth_blend(0, 0, packed(15, 15), packed(1, 1)) & 0xFF,
            (240 + 16) / 4
        );
    }
    #[test]
    fn weighted_blend_narrows_like_java_int_casts() {
        let c = [packed(15, 15); 4];
        assert_eq!(weighted_blend(c, [0.25; 4]), packed(15, 15));
        // Negative weights (geometry past the block) wrap through the 0xFF mask.
        assert_eq!(
            weighted_blend([packed(1, 0); 4], [-0.25; 4]) & 0xFF,
            (-16i32 as u32) & 0xFF
        );
    }
    #[test]
    fn colour_arithmetic_matches_argb_helpers() {
        assert_eq!(gray(1.0), 255);
        assert_eq!(gray(0.8), 204);
        assert_eq!(scale(255, 0.5), 127);
        assert_eq!(scale(gray(0.6), 0.8), 122);
        assert_eq!(cardinal(true, D::Down), 0.9);
        assert_eq!(cardinal(false, D::Down), 0.5);
    }
    #[test]
    fn emission_raises_both_channels_and_drops_fractions() {
        assert_eq!(with_emission(0x0087_0013, 0), 0x0087_0013);
        assert_eq!(with_emission(0x0087_0013, 3), packed(3, 8));
        assert_eq!(max_coords(packed(3, 9), packed(7, 2)), packed(7, 9));
    }
    #[test]
    fn flat_light_reads_the_neighbour_a_face_looks_at() {
        let pos = [4, 64, -2];
        let full_top = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        // A cull face samples its neighbour, whatever the quad's shape.
        let inset = full_top.map(|[x, _, z]| [x, 0.75, z]);
        assert_eq!(
            flat_position(pos, &inset, D::Up, Some(D::Up), false),
            [4, 65, -2]
        );
        // A flush quad without a cull face samples its neighbour too: a block
        // drawn at a cell that is now opaque (a moving block, #3002) still
        // takes the light around it.
        assert_eq!(
            flat_position(pos, &full_top, D::Up, None, false),
            [4, 65, -2]
        );
        let west = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(flat_position(pos, &west, D::West, None, false), [3, 64, -2]);
        // An inset quad (a piston head's arm) takes the block's own light.
        assert_eq!(flat_position(pos, &inset, D::Up, None, false), pos);
        assert_eq!(flat_position(pos, &inset, D::Up, None, true), [4, 65, -2]);
    }
    #[test]
    fn quad_shape_classifies_partial_and_cubic_faces() {
        let full_top = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        let s = shape(&full_top, D::Up, false);
        assert!(s.cubic && !s.partial);
        let slab_top = full_top.map(|[x, _, z]| [x, 0.5, z]);
        let s = shape(&slab_top, D::Up, false);
        assert!(!s.cubic && !s.partial);
        assert!(shape(&slab_top, D::Up, true).cubic);
        let small = slab_top.map(|[x, y, z]| [x * 0.5, y, z]);
        let s = shape(&small, D::Up, false);
        assert!(s.partial);
        assert_eq!(s.bounds[EAST as usize], 0.5);
        assert_eq!(s.bounds[FLIP_EAST as usize], 0.5);
    }
}
