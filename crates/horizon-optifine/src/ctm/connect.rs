//! Which tile a connected face gets. A face's texture directions ([`Axes`]) say where its left,
//! right, up and down neighbours are; [`neighbours`] turns the ones that join into a bit mask;
//! each method then reads its tile from the mask ([`ctm_index`], [`compact_quarters`],
//! [`horizontal`], [`overlay_tiles`]...) or from the position ([`repeat_index`],
//! [`random_index`]).
//!
//! Nothing here knows OptiFine's files, so other connected-texture formats (Fusion's) share it:
//! they build the same mask and read the same tables.
//!
//! The tile tables are OptiFine's documented templates (`ctm_template.png`,
//! `ctm_compact_template.png`, `horizontal_template.png`, `vertical_template.png`,
//! `overlay_template.png`), read off the images.

use super::Symmetry;

pub type Vec3 = [i32; 3];

const fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

const fn neg(a: Vec3) -> Vec3 {
    [-a[0], -a[1], -a[2]]
}

/// In `i64`: a coordinate times -1 can leave `i32`.
const fn dot(a: Vec3, b: Vec3) -> i64 {
    a[0] as i64 * b[0] as i64 + a[1] as i64 * b[1] as i64 + a[2] as i64 * b[2] as i64
}

/// Block faces in `Direction` order, the bit order of [`super::faces`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

const EAST: Vec3 = [1, 0, 0];
const WEST: Vec3 = [-1, 0, 0];
const UP: Vec3 = [0, 1, 0];
const SOUTH: Vec3 = [0, 0, 1];
const NORTH: Vec3 = [0, 0, -1];

impl Face {
    pub const ALL: [Face; 6] = [
        Face::Down,
        Face::Up,
        Face::North,
        Face::South,
        Face::West,
        Face::East,
    ];

    pub const fn normal(self) -> Vec3 {
        match self {
            Face::Down => [0, -1, 0],
            Face::Up => UP,
            Face::North => NORTH,
            Face::South => SOUTH,
            Face::West => WEST,
            Face::East => EAST,
        }
    }

    /// The face's [`super::faces`] bit.
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// A block state's `axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// Where a face's texture points in the world: its right (+u) and its up (-v).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Axes {
    pub right: Vec3,
    pub up: Vec3,
}

impl Axes {
    /// Where a quad's own texture points, from its corners' positions and UVs, snapped to the
    /// block's axes (Continuity's `orient=texture`): a turned or mirrored model's face turns or
    /// mirrors its neighbours too. `None` for a quad whose UVs don't span it.
    pub fn texture(positions: &[[f32; 3]; 4], uv: &[[f32; 2]; 4]) -> Option<Self> {
        let edge = |k: usize| [0, 1, 2].map(|c| positions[k][c] - positions[0][c]);
        let step = |k: usize| [uv[k][0] - uv[0][0], uv[k][1] - uv[0][1]];
        let (e1, e2, d1, d2) = (edge(1), edge(3), step(1), step(3));
        // The edges in UV terms: e = du * (d/du) + dv * (d/dv), solved for the two gradients.
        let det = d1[0] * d2[1] - d2[0] * d1[1];
        if !det.is_finite() || det.abs() < 1e-6 {
            return None;
        }
        let along_u = [0, 1, 2].map(|c| (e1[c] * d2[1] - e2[c] * d1[1]) / det);
        let along_v = [0, 1, 2].map(|c| (e2[c] * d1[0] - e1[c] * d2[0]) / det);
        // Only a direction close to one of the block's axes (within about 25°): a plant's 45°
        // planes have none.
        let snap = |v: [f32; 3]| {
            let k = (0..3).max_by(|&a, &b| v[a].abs().total_cmp(&v[b].abs()))?;
            let length = v.iter().map(|c| c * c).sum::<f32>().sqrt();
            let mut out = [0; 3];
            out[k] = if v[k] > 0.0 { 1 } else { -1 };
            (v[k].abs() > 0.9 * length).then_some(out)
        };
        // v grows downward: the texture's up is against it.
        let (right, up) = (snap(along_u)?, neg(snap(along_v)?));
        (dot(right, up) == 0).then_some(Self { right, up })
    }

    /// On a vanilla cube: `BlockElement`'s default UVs (OptiFine's `ctm`, Continuity's
    /// `orient=none`).
    pub const fn cube(face: Face) -> Self {
        let (right, up) = match face {
            Face::Down => (EAST, SOUTH),
            Face::Up => (EAST, NORTH),
            Face::North => (WEST, UP),
            Face::South => (EAST, UP),
            Face::West => (SOUTH, UP),
            Face::East => (NORTH, UP),
        };
        Self { right, up }
    }

    /// On a cube column turned to `axis` as vanilla's blockstates turn one (`x=90, y=90` for
    /// `x`, `x=90` for `z`): OptiFine's horizontal and vertical methods, Continuity's
    /// `orient=state_axis`.
    pub fn column(face: Face, axis: Axis) -> Self {
        let turn = |[x, y, z]: Vec3| match axis {
            Axis::Y => [x, y, z],
            // x=90 takes up to north.
            Axis::Z => [x, z, -y],
            // Then y=90 takes north to east.
            Axis::X => [y, z, x],
        };
        let model = Face::ALL
            .into_iter()
            .find(|f| turn(f.normal()) == face.normal())
            .expect("a quarter turn maps faces onto faces");
        let cube = Self::cube(model);
        Self {
            right: turn(cube.right),
            up: turn(cube.up),
        }
    }
}

/// Neighbour bits of a face's mask, in texture directions.
pub const U: u8 = 1;
pub const UR: u8 = 2;
pub const R: u8 = 4;
pub const DR: u8 = 8;
pub const D: u8 = 16;
pub const DL: u8 = 32;
pub const L: u8 = 64;
pub const UL: u8 = 128;

/// The neighbours of a face that join it, by `joins(offset from the block)`: all eight, so
/// overlays see the corners whatever the sides. With `inner_seams`, a neighbour whose own face
/// is covered (the block in front of it joins too) doesn't count, which draws a seam on inner
/// corners.
pub fn neighbours(
    axes: Axes,
    normal: Vec3,
    inner_seams: bool,
    mut joins: impl FnMut(Vec3) -> bool,
) -> u8 {
    let (r, u) = (axes.right, axes.up);
    let (l, d) = (neg(r), neg(u));
    let offsets = [
        (U, u),
        (UR, add(u, r)),
        (R, r),
        (DR, add(d, r)),
        (D, d),
        (DL, add(d, l)),
        (L, l),
        (UL, add(u, l)),
    ];
    let mut mask = 0;
    for (bit, offset) in offsets {
        if joins(offset) && !(inner_seams && joins(add(offset, normal))) {
            mask |= bit;
        }
    }
    mask
}

/// The 47 tiles of `ctm_template.png`, as the neighbours each one joins: its sides, and a
/// corner only where both sides beside it join (a lone corner draws nothing).
const TEMPLATE: [u8; 47] = [
    0,
    R,
    R | L,
    L,
    R | D,
    D | L,
    U | R | D,
    R | D | L,
    U | UR | R | D | L,
    U | R | DR | D | L,
    U | R | D | DL | L | UL,
    U | UR | R | D | L | UL,
    D,
    R | DR | D,
    R | DR | D | DL | L,
    D | DL | L,
    U | R,
    U | L,
    U | R | L,
    U | D | L,
    U | R | D | L | UL,
    U | R | D | DL | L,
    U | R | DR | D | DL | L,
    U | UR | R | DR | D | L,
    U | D,
    U | UR | R | DR | D,
    U | UR | R | DR | D | DL | L | UL,
    U | D | DL | L | UL,
    U | R | DR | D,
    R | D | DL | L,
    U | UR | R | D,
    R | DR | D | L,
    U | UR | R | D | DL | L | UL,
    U | UR | R | DR | D | L | UL,
    U | UR | R | D | DL | L,
    U | R | DR | D | L | UL,
    U,
    U | UR | R,
    U | UR | R | L | UL,
    U | L | UL,
    U | UR | R | L,
    U | D | L | UL,
    U | R | L | UL,
    U | D | DL | L,
    U | R | DR | D | DL | L | UL,
    U | UR | R | DR | D | DL | L,
    U | R | D | L,
];

/// A mask without the corners whose sides don't both join.
pub(crate) const fn reduce(mask: u8) -> u8 {
    let mut out = mask & (U | R | D | L);
    let corners = [(UR, U | R), (DR, D | R), (DL, D | L), (UL, U | L)];
    let mut i = 0;
    while i < corners.len() {
        let (corner, sides) = corners[i];
        if mask & corner != 0 && mask & sides == sides {
            out |= corner;
        }
        i += 1;
    }
    out
}

const CTM_INDEX: [u8; 256] = {
    let mut table = [0; 256];
    let mut mask = 0;
    while mask < 256 {
        let reduced = reduce(mask as u8);
        let mut tile = 0;
        while TEMPLATE[tile] != reduced {
            tile += 1;
        }
        table[mask] = tile as u8;
        mask += 1;
    }
    table
};

/// The `ctm` tile (0-46) for a neighbour mask.
pub fn ctm_index(mask: u8) -> usize {
    CTM_INDEX[mask as usize] as usize
}

/// `ctm_compact`'s tile (0-4) for each quarter of the face: top-left, top-right, bottom-left,
/// bottom-right. Tile 0 has every border, 1 none, 2 its left and right ones (joined up and
/// down), 3 its top and bottom ones, 4 only the inner corners.
pub fn compact_quarters(mask: u8) -> [u8; 4] {
    let quarter = |vertical: u8, horizontal: u8, corner: u8| match (
        mask & vertical != 0,
        mask & horizontal != 0,
    ) {
        (false, false) => 0,
        (true, false) => 2,
        (false, true) => 3,
        (true, true) if mask & corner != 0 => 1,
        (true, true) => 4,
    };
    [
        quarter(U, L, UL),
        quarter(U, R, UR),
        quarter(D, L, DL),
        quarter(D, R, DR),
    ]
}

/// `horizontal`'s tile: 0 the left end of a row, 1 its middle, 2 its right end, 3 alone.
pub fn horizontal(mask: u8) -> usize {
    match (mask & L != 0, mask & R != 0) {
        (false, true) => 0,
        (true, true) => 1,
        (true, false) => 2,
        (false, false) => 3,
    }
}

/// `vertical`'s tile: 0 the bottom of a column, 1 its middle, 2 its top, 3 alone.
pub fn vertical(mask: u8) -> usize {
    match (mask & U != 0, mask & D != 0) {
        (true, false) => 0,
        (true, true) => 1,
        (false, true) => 2,
        (false, false) => 3,
    }
}

/// `horizontal+vertical`'s 7 tiles: a face alone in its row takes `vertical`'s first three as
/// tiles 4-6.
pub fn horizontal_vertical(mask: u8) -> usize {
    match (horizontal(mask), vertical(mask)) {
        (3, 3) => 3,
        (3, v) => 4 + v,
        (h, _) => h,
    }
}

/// `vertical+horizontal`'s 7 tiles: a face alone in its column takes `horizontal`'s first three
/// as tiles 4-6.
pub fn vertical_horizontal(mask: u8) -> usize {
    match (vertical(mask), horizontal(mask)) {
        (3, 3) => 3,
        (3, h) => 4 + h,
        (v, _) => v,
    }
}

/// `overlay`'s tiles (0-16) for a face whose neighbours in `mask` hold the block drawn over it:
/// one tile for the sides (two when they face each other: the template has no tile for those),
/// and one for each corner with neither side beside it.
pub fn overlay_tiles(mask: u8) -> impl Iterator<Item = usize> {
    let sides: &[usize] = match (mask & U != 0, mask & R != 0, mask & D != 0, mask & L != 0) {
        (false, false, false, false) => &[],
        (true, false, false, false) => &[15],
        (false, true, false, false) => &[7],
        (false, false, true, false) => &[1],
        (false, false, false, true) => &[9],
        (true, true, false, false) => &[10],
        (true, false, false, true) => &[11],
        (false, true, true, false) => &[3],
        (false, false, true, true) => &[4],
        (true, false, true, false) => &[15, 1],
        (false, true, false, true) => &[9, 7],
        (true, true, true, false) => &[12],
        (true, false, true, true) => &[6],
        (true, true, false, true) => &[13],
        (false, true, true, true) => &[5],
        (true, true, true, true) => &[8],
    };
    let corners = [
        (UR, U | R, 14),
        (DR, D | R, 0),
        (DL, D | L, 2),
        (UL, U | L, 16),
    ];
    let corners = corners
        .into_iter()
        .filter(move |&(corner, beside, _)| mask & corner != 0 && mask & beside == 0)
        .map(|(_, _, tile)| tile);
    sides.iter().copied().chain(corners)
}

/// `repeat`'s tile: the block's place in a `width` x `height` pattern laid along the face's
/// texture directions, row by row from the top.
pub fn repeat_index(pos: Vec3, axes: Axes, width: u32, height: u32) -> usize {
    let (w, h) = (width.max(1) as i64, height.max(1) as i64);
    let u = dot(pos, axes.right).rem_euclid(w);
    let v = (-dot(pos, axes.up)).rem_euclid(h);
    (v * w + u) as usize
}

/// The face whose choice `face` shares under `symmetry`: itself, the first of its opposite
/// pair, or one face for all six.
pub fn symmetric_face(face: Face, symmetry: Symmetry) -> Face {
    match symmetry {
        Symmetry::None => face,
        Symmetry::Opposite => Face::ALL[face as usize & !1],
        Symmetry::All => Face::Down,
    }
}

/// `random`'s tile (below `count`): a fixed choice for the block at `pos` and its (symmetric)
/// face, so a world looks the same every time it loads. `weights` is one per tile; any other
/// length (none given) picks every tile alike. `loops` rehashes the choice (`randomLoops`).
pub fn random_index(pos: Vec3, face: Face, loops: u32, weights: &[u32], count: usize) -> usize {
    // The golden-ratio step keeps a zero input (the origin's down face) off `mix`'s fixed point.
    let step = |h: u32, v: u32| mix((h ^ v).wrapping_add(0x9e37_79b9));
    let mut h = step(0, pos[0] as u32);
    h = step(h, pos[1] as u32);
    h = step(h, pos[2] as u32);
    h = step(h, face as u32);
    for _ in 0..loops {
        h = step(h, 0);
    }
    let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
    if weights.len() != count || total == 0 {
        return h as usize % count.max(1);
    }
    let mut pick = u64::from(h) % total;
    weights
        .iter()
        .position(|&w| match pick.checked_sub(u64::from(w)) {
            Some(rest) => {
                pick = rest;
                false
            }
            None => true,
        })
        .expect("pick is below the weights' total")
}

/// A 32-bit finalizer (Chris Wellons' `lowbias32`): every input bit moves every output bit.
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctm_tiles_from_the_template() {
        // Read off ctm_template.png by hand: the tile's borders are the sides that don't join.
        assert_eq!(ctm_index(0), 0);
        assert_eq!(ctm_index(R), 1);
        assert_eq!(ctm_index(R | L), 2);
        assert_eq!(ctm_index(L), 3);
        assert_eq!(ctm_index(D), 12);
        assert_eq!(ctm_index(U | D), 24);
        assert_eq!(ctm_index(U), 36);
        assert_eq!(ctm_index(255), 26);
        assert_eq!(ctm_index(U | R | D | L), 46);
        // The top-left of a 3x3 square, and a corner whose sides don't both join.
        assert_eq!(ctm_index(R | DR | D), 13);
        assert_eq!(ctm_index(U | UR | DR | DL | UL), 36);
        // One missing corner of a full square.
        assert_eq!(ctm_index(!UR), 44);
    }

    #[test]
    fn compact_quarters_from_the_template() {
        // ctm_compact_template.png: 0 every border, 1 none, 2 left and right borders, 3 top and
        // bottom ones, 4 the inner corners only.
        assert_eq!(compact_quarters(0), [0; 4]);
        assert_eq!(compact_quarters(255), [1; 4]);
        assert_eq!(compact_quarters(U | D), [2; 4]);
        assert_eq!(compact_quarters(L | R), [3; 4]);
        assert_eq!(compact_quarters(U | R | D | L), [4; 4]);
        // The left end of a row, and a bottom-left corner of a square.
        assert_eq!(compact_quarters(R), [0, 3, 0, 3]);
        assert_eq!(compact_quarters(U | UR | R), [2, 1, 0, 3]);
    }

    #[test]
    fn rows_and_columns() {
        assert_eq!([R, R | L, L, 0].map(horizontal), [0, 1, 2, 3]);
        assert_eq!([U, U | D, D, 0].map(vertical), [0, 1, 2, 3]);
        // Joined left: horizontal wins. Alone in the row: the column's tiles, from 4.
        assert_eq!(horizontal_vertical(L | U), 2);
        assert_eq!([U, U | D, D, 0].map(horizontal_vertical), [4, 5, 6, 3]);
        assert_eq!([R, R | L, L, 0].map(vertical_horizontal), [4, 5, 6, 3]);
        assert_eq!(vertical_horizontal(U | R), 0);
    }

    #[test]
    fn overlay_tiles_from_the_template() {
        let tiles = |mask| overlay_tiles(mask).collect::<Vec<_>>();
        assert_eq!(tiles(0), [0usize; 0]);
        assert_eq!(tiles(D), [1]);
        assert_eq!(tiles(DR), [0]);
        assert_eq!(tiles(D | DR), [1], "a corner beside a side is the side's");
        assert_eq!(tiles(U | D), [15, 1]);
        assert_eq!(tiles(U | L | DR), [11, 0]);
        assert_eq!(tiles(255), [8]);
        assert_eq!(tiles(UR | UL), [14, 16]);
    }

    #[test]
    fn face_directions() {
        // A north face seen from the north: right is west.
        assert_eq!(Axes::cube(Face::North).right, WEST);
        assert_eq!(Axes::column(Face::North, Axis::Y), Axes::cube(Face::North));
        // An x log: its grain runs east-west on the top face, its end turned on the east one.
        let top = Axes::column(Face::Up, Axis::X);
        assert_eq!((top.right, top.up), (SOUTH, EAST));
        let end = Axes::column(Face::East, Axis::X);
        assert_eq!((end.right, end.up), (SOUTH, [0, -1, 0]));
        // A z log: north face is the end.
        let end = Axes::column(Face::North, Axis::Z);
        assert_eq!((end.right, end.up), (EAST, [0, -1, 0]));
    }

    #[test]
    fn every_turned_face_keeps_its_texture_facing_out() {
        // A texture's right x up is the face's normal on a cube, and turning keeps it.
        let cross = |a: Vec3, b: Vec3| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        for face in Face::ALL {
            for axis in [Axis::X, Axis::Y, Axis::Z] {
                let a = Axes::column(face, axis);
                assert_eq!(cross(a.right, a.up), face.normal(), "{face:?} {axis:?}");
            }
        }
    }

    #[test]
    fn neighbour_masks() {
        let axes = Axes::cube(Face::North);
        let normal = Face::North.normal();
        // Glass at the block's west (its north face's right) and above it.
        let glass = |o: Vec3| o == WEST || o == UP || o == add(WEST, UP);
        assert_eq!(neighbours(axes, normal, false, glass), U | UR | R);
        // A block in front of the west neighbour covers its face: an inner seam.
        let covered = |o: Vec3| glass(o) || o == add(WEST, NORTH);
        assert_eq!(neighbours(axes, normal, false, covered), U | UR | R);
        assert_eq!(neighbours(axes, normal, true, covered), U | UR);
        // A covered corner counts as covered too.
        let corner = |o: Vec3| glass(o) || o == add(add(WEST, UP), NORTH);
        assert_eq!(neighbours(axes, normal, true, corner), U | R);
    }

    #[test]
    fn symmetry() {
        assert_eq!(symmetric_face(Face::South, Symmetry::None), Face::South);
        assert_eq!(symmetric_face(Face::South, Symmetry::Opposite), Face::North);
        assert_eq!(symmetric_face(Face::East, Symmetry::Opposite), Face::West);
        assert_eq!(symmetric_face(Face::Up, Symmetry::Opposite), Face::Down);
        assert_eq!(symmetric_face(Face::East, Symmetry::All), Face::Down);
    }

    #[test]
    fn repeat_patterns() {
        let north = Axes::cube(Face::North);
        // Along the north face the pattern runs west (its right), rows downward.
        assert_eq!(repeat_index([0, 0, 0], north, 2, 2), 0);
        assert_eq!(repeat_index([-1, 0, 0], north, 2, 2), 1);
        assert_eq!(repeat_index([0, -1, 0], north, 2, 2), 2);
        assert_eq!(repeat_index([-3, 5, 7], north, 3, 2), 3);
    }

    #[test]
    fn random_choices_are_fixed_and_weighted() {
        // Pinned, so a world keeps its tiles across versions (lowbias32 worked by hand in Python).
        assert_eq!(random_index([12, 64, -7], Face::Up, 0, &[], 5), 3);
        assert_eq!(random_index([0, 0, 0], Face::Down, 0, &[], 5), 4);
        assert_eq!(random_index([0, 0, 0], Face::Down, 1, &[], 4), 2);
        // Weights that don't fit the tiles pick every tile alike, and never past them.
        for x in 0..50 {
            assert!(random_index([x, 0, 0], Face::Up, 0, &[1, 1, 1], 2) < 2);
        }
        // Weights 1:3 over a 100x100 floor: about a quarter take tile 0.
        let mut zeros = 0;
        for x in 0..100 {
            for z in 0..100 {
                zeros += (random_index([x, 0, z], Face::Up, 0, &[1, 3], 2) == 0) as u32;
            }
        }
        assert!((2200..2800).contains(&zeros), "{zeros}");
        // A zero weight is never picked.
        for x in 0..200 {
            assert_ne!(random_index([x, 1, 2], Face::Up, 3, &[2, 0, 1], 3), 1);
        }
        // Loops pick another pattern.
        let differs = (0..50)
            .filter(|&x| {
                random_index([x, 0, 0], Face::Up, 0, &[], 4)
                    != random_index([x, 0, 0], Face::Up, 1, &[], 4)
            })
            .count();
        assert!(differs > 20, "{differs}");
    }

    #[test]
    fn a_quads_own_texture_directions() {
        // A north face with the cube's UVs: its texture points as the cube's does.
        let north = [
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let standard = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert_eq!(
            Axes::texture(&north, &standard),
            Some(Axes::cube(Face::North))
        );
        // Mirrored (a vanilla pane face): right turns to east.
        let mirrored = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        assert_eq!(
            Axes::texture(&north, &mirrored),
            Some(Axes {
                right: EAST,
                up: UP
            })
        );
        // A top face turned a quarter (y=90): the texture's up points east, its right south.
        let top = [
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let turned = [[0.0, 1.0], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
        assert_eq!(
            Axes::texture(&top, &turned),
            Some(Axes {
                right: SOUTH,
                up: EAST
            })
        );
        // UVs that don't span the quad, or that run diagonally across it.
        assert_eq!(Axes::texture(&north, &[[0.5, 0.5]; 4]), None);
        let diagonal = [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]];
        assert_eq!(Axes::texture(&north, &diagonal), None);
        assert_eq!(Axes::texture(&north, &[[f32::NAN; 2]; 4]), None);
        // A plant's plane, turned 45° about y: no block axis to connect along.
        let plant = [
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
        ];
        assert_eq!(Axes::texture(&plant, &standard), None);
    }
}
