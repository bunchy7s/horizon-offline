use super::{
    Aabb, Axis, BlockHit, COLLISION_EPSILON, Direction, Vec3, clamp, floor_i32, java_max, java_min,
};
use crate::position::BlockPos;
use std::fmt;
use std::sync::{Arc, OnceLock};
mod create;
mod face;
mod join;
pub use join::ShapeOp;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShapeError {
    InvalidCoordinates,
    InvalidOccupancy,
    TooLarge,
    UnboundedBoolean,
    InvalidBounds,
}
impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid voxel shape: {self:?}")
    }
}
impl std::error::Error for ShapeError {}

/// Immutable discrete occupancy with exact double coordinate boundaries.
///
/// Greedy boxes are calculated once and reused for ray and proximity queries.
/// Cube shapes retain vanilla's arithmetic cell lookup instead of replacing it
/// with a binary search over coordinates (which differs at rounding boundaries).
#[derive(Clone, Debug)]
pub struct VoxelShape {
    identity: Arc<ShapeIdentity>,
    coords: Arc<[Vec<f64>; 3]>,
    sizes: [usize; 3],
    occupancy: Arc<[u64]>,
    cube: bool,
    coordinate_cubes: [bool; 3],
    boxes: Arc<Vec<Aabb>>,
    bounds: Option<Aabb>,
    discrete_min: [i32; 3],
    discrete_max: [i32; 3],
}

type FaceCache = [OnceLock<Result<VoxelShape, ShapeError>>; 6];
#[derive(Debug, Default)]
struct ShapeIdentity {
    faces: OnceLock<Box<FaceCache>>,
}

impl VoxelShape {
    pub fn from_grid(coords: [Vec<f64>; 3], occupied: &[bool]) -> Result<Self, ShapeError> {
        Self::build(coords, occupied, false, [false; 3])
    }
    pub fn from_cube(sizes: [usize; 3], occupied: &[bool]) -> Result<Self, ShapeError> {
        if sizes.contains(&0) {
            return Err(ShapeError::InvalidCoordinates);
        }
        let volume = volume(sizes)?;
        if occupied.len() != volume {
            return Err(ShapeError::InvalidOccupancy);
        }
        let coords = sizes.map(|size| (0..=size).map(|i| i as f64 / size as f64).collect());
        Self::build(coords, occupied, true, [true; 3])
    }
    fn build(
        coords: [Vec<f64>; 3],
        occupied: &[bool],
        cube: bool,
        coordinate_cubes: [bool; 3],
    ) -> Result<Self, ShapeError> {
        if coords.iter().any(|c| {
            c.is_empty()
                || c.len() > 1_048_577
                || c.iter().any(|v| v.is_nan())
                || c.windows(2).any(|p| p[0] >= p[1])
        }) {
            return Err(ShapeError::InvalidCoordinates);
        }
        let sizes = std::array::from_fn(|i| coords[i].len() - 1);
        let count = volume(sizes)?;
        if count != occupied.len() {
            return Err(ShapeError::InvalidOccupancy);
        }
        let mut occupancy = vec![0; count.div_ceil(64)];
        let mut discrete_min = sizes.map(|v| v as i32);
        let mut discrete_max = [0; 3];
        for (i, full) in occupied.iter().enumerate() {
            if *full {
                occupancy[i / 64] |= 1 << (i % 64);
                let position = [
                    (i / (sizes[1] * sizes[2])) as i32,
                    (i / sizes[2] % sizes[1]) as i32,
                    (i % sizes[2]) as i32,
                ];
                for axis in 0..3 {
                    discrete_min[axis] = discrete_min[axis].min(position[axis]);
                    discrete_max[axis] = discrete_max[axis].max(position[axis] + 1);
                }
            }
        }
        let mut shape = Self {
            identity: Arc::new(ShapeIdentity::default()),
            coords: Arc::new(coords),
            sizes,
            occupancy: occupancy.into(),
            cube,
            coordinate_cubes,
            boxes: Arc::new(Vec::new()),
            bounds: None,
            discrete_min,
            discrete_max,
        };
        shape.boxes = Arc::new(shape.decompose());
        shape.bounds = shape.boxes.iter().copied().reduce(Aabb::union_bounds);
        Ok(shape)
    }
    pub fn empty() -> Self {
        Self::empty_ref().clone()
    }
    /// The shared empty shape, without cloning it.
    pub fn empty_ref() -> &'static Self {
        static EMPTY: OnceLock<VoxelShape> = OnceLock::new();
        EMPTY.get_or_init(|| {
            Self::from_grid([vec![0.0], vec![0.0], vec![0.0]], &[]).expect("valid empty grid")
        })
    }
    /// `shape == Shapes.empty()`: the same as `same_identity(&empty())`
    /// without cloning the shared empty shape.
    pub fn is_empty_identity(&self) -> bool {
        self.same_identity(Self::empty_ref())
    }
    pub fn block() -> Self {
        Self::block_ref().clone()
    }
    fn block_ref() -> &'static Self {
        static BLOCK: OnceLock<VoxelShape> = OnceLock::new();
        BLOCK.get_or_init(|| Self::from_cube([1, 1, 1], &[true]).expect("valid unit cube"))
    }
    /// `shape == Shapes.block()`: the same as `same_identity(&block())`
    /// without cloning the shared unit cube.
    pub fn is_block(&self) -> bool {
        self.same_identity(Self::block_ref())
    }
    pub fn is_empty(&self) -> bool {
        self.bounds.is_none()
    }
    pub fn bounds(&self) -> Option<Aabb> {
        self.bounds
    }
    pub fn boxes(&self) -> &[Aabb] {
        &self.boxes
    }
    pub fn coordinates(&self, axis: Axis) -> &[f64] {
        &self.coords[axis as usize]
    }
    pub fn sizes(&self) -> [usize; 3] {
        self.sizes
    }
    pub fn is_cube(&self) -> bool {
        self.cube
    }
    /// Clones and operations returning an existing immutable shape preserve its
    /// identity. Separately constructed grids remain distinct even if equal.
    pub fn same_identity(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
    /// A separately constructed shape with the same immutable grid. Storage is
    /// shared, while identity-sensitive vanilla join shortcuts remain distinct.
    pub fn fresh_identity(&self) -> Self {
        let mut shape = self.clone();
        shape.identity = Arc::new(ShapeIdentity::default());
        shape
    }
    pub fn cube_coordinate_axes(&self) -> [bool; 3] {
        self.coordinate_cubes
    }
    /// An array shape can contain CubePointRange axes after a partial cube
    /// merge. Preserve that metadata for subsequent Boolean operations.
    pub fn from_mixed_grid(
        coords: [Vec<f64>; 3],
        occupied: &[bool],
        coordinate_cubes: [bool; 3],
    ) -> Result<Self, ShapeError> {
        for i in 0..3 {
            if coordinate_cubes[i] {
                let size = coords[i].len().saturating_sub(1);
                if size == 0
                    || coords[i]
                        .iter()
                        .enumerate()
                        .any(|(j, v)| v.to_bits() != (j as f64 / size as f64).to_bits())
                {
                    return Err(ShapeError::InvalidCoordinates);
                }
            }
        }
        Self::build(coords, occupied, false, coordinate_cubes)
    }
    pub fn min(&self, axis: Axis) -> f64 {
        let i = axis as usize;
        let index = self.discrete_min[i];
        if index >= self.sizes[i] as i32 {
            f64::INFINITY
        } else {
            self.coords[i][index as usize]
        }
    }
    pub fn max(&self, axis: Axis) -> f64 {
        let i = axis as usize;
        let index = self.discrete_max[i];
        if index <= 0 {
            f64::NEG_INFINITY
        } else {
            self.coords[i][index as usize]
        }
    }
    /// Discrete extrema are separate from occupancy: a quantized empty shape
    /// can retain nonempty axis ranges, and empty joins retain Java sentinels.
    pub fn discrete_bounds(&self) -> ([i32; 3], [i32; 3]) {
        (self.discrete_min, self.discrete_max)
    }
    /// Import reference extrema. Nonempty shapes must match their occupancy;
    /// empty shapes admit bounded ranges or the empty-join sentinel triple.
    pub fn with_discrete_bounds(
        mut self,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Result<Self, ShapeError> {
        let in_range = (0..3).all(|i| {
            min[i] >= 0
                && min[i] <= self.sizes[i] as i32
                && max[i] >= 0
                && max[i] <= self.sizes[i] as i32
        });
        let join_sentinel = min == [i32::MAX; 3] && max == [i32::MIN + 1; 3];
        if (!in_range && !join_sentinel)
            || (!self.is_empty() && (min != self.discrete_min || max != self.discrete_max))
        {
            return Err(ShapeError::InvalidBounds);
        }
        if min != self.discrete_min || max != self.discrete_max {
            self.identity = Arc::new(ShapeIdentity::default());
        }
        self.discrete_min = min;
        self.discrete_max = max;
        Ok(self)
    }
    /// Minimum occupied coordinate along an axis at the two cyclic
    /// perpendicular coordinates; an empty/outside column yields +infinity.
    pub fn min_at(&self, axis: Axis, first: f64, second: f64) -> f64 {
        let mut position = self.column_position(axis, first, second);
        for i in 0..self.sizes[axis as usize] {
            position[axis as usize] = i as i32;
            if self.is_full(position[0], position[1], position[2]) {
                return self.coords[axis as usize][i];
            }
        }
        f64::INFINITY
    }
    /// Maximum occupied coordinate at the same cyclic column; an empty/outside
    /// column yields -infinity. The maximum is the cell's upper face.
    pub fn max_at(&self, axis: Axis, first: f64, second: f64) -> f64 {
        let mut position = self.column_position(axis, first, second);
        for i in (0..self.sizes[axis as usize]).rev() {
            position[axis as usize] = i as i32;
            if self.is_full(position[0], position[1], position[2]) {
                return self.coords[axis as usize][i + 1];
            }
        }
        f64::NEG_INFINITY
    }
    fn column_position(&self, axis: Axis, first: f64, second: f64) -> [i32; 3] {
        let a = (axis as usize + 1) % 3;
        let b = (axis as usize + 2) % 3;
        let mut position = [0; 3];
        position[a] = self.find_index(Axis::ALL[a], first);
        position[b] = self.find_index(Axis::ALL[b], second);
        position
    }
    /// Vanilla movement converts cube coordinates to an array shape, even for
    /// a zero offset. Keep this distinction for subsequent cell lookups.
    pub fn moved(&self, offset: Vec3) -> Self {
        if self.is_empty() {
            return Self::empty();
        }
        let mut result = self.clone();
        result.identity = Arc::new(ShapeIdentity::default());
        result.cube = false;
        result.coordinate_cubes = [false; 3];
        let mut coords = (*self.coords).clone();
        for axis in Axis::ALL {
            for value in &mut coords[axis as usize] {
                *value += offset.component(axis);
            }
        }
        result.coords = Arc::new(coords);
        // The same occupancy decomposes into the same boxes, over coordinates
        // moved by the same additions.
        result.boxes = Arc::new(self.boxes.iter().map(|b| b.move_by(offset)).collect());
        result.bounds = self.bounds.map(|bounds| bounds.move_by(offset));
        result
    }
    pub fn is_full(&self, x: i32, y: i32, z: i32) -> bool {
        if x < 0
            || y < 0
            || z < 0
            || x as usize >= self.sizes[0]
            || y as usize >= self.sizes[1]
            || z as usize >= self.sizes[2]
        {
            return false;
        }
        full(
            &self.occupancy,
            index(self.sizes, x as usize, y as usize, z as usize),
        )
    }
    pub fn find_index(&self, axis: Axis, value: f64) -> i32 {
        let i = axis as usize;
        if self.cube {
            return floor_i32(clamp(
                value * self.sizes[i] as f64,
                -1.0,
                self.sizes[i] as f64,
            ));
        }
        // Upper-bound search, including the final outside cell index.
        let coords = &self.coords[i];
        let mut start = 0;
        let mut end = coords.len();
        while start < end {
            let middle = (start + end) / 2;
            if value < coords[middle] {
                end = middle;
            } else {
                start = middle + 1;
            }
        }
        start as i32 - 1
    }
    pub fn collide(&self, axis: Axis, aabb: Aabb, amount: f64) -> f64 {
        if self.is_empty() {
            return amount;
        }
        if amount.abs() < COLLISION_EPSILON {
            return 0.0;
        }
        let i = axis as usize;
        let p = Axis::ALL[(i + 1) % 3];
        let q = Axis::ALL[(i + 2) % 3];
        let p_start = self.find_index(p, aabb.min(p) + COLLISION_EPSILON).max(0);
        let p_end = (self.find_index(p, aabb.max(p) - COLLISION_EPSILON) + 1)
            .min(self.sizes[p as usize] as i32);
        let q_start = self.find_index(q, aabb.min(q) + COLLISION_EPSILON).max(0);
        let q_end = (self.find_index(q, aabb.max(q) - COLLISION_EPSILON) + 1)
            .min(self.sizes[q as usize] as i32);
        let occupied_layer = |layer| {
            (p_start..p_end).any(|u| {
                (q_start..q_end).any(|v| {
                    let mut cell = [0; 3];
                    cell[i] = layer;
                    cell[p as usize] = u;
                    cell[q as usize] = v;
                    self.is_full(cell[0], cell[1], cell[2])
                })
            })
        };
        if amount > 0.0 {
            let start = self.find_index(axis, aabb.max(axis) - COLLISION_EPSILON) + 1;
            for layer in start..self.sizes[i] as i32 {
                if occupied_layer(layer) {
                    let distance = self.coords[i][layer as usize] - aabb.max(axis);
                    return if distance >= -COLLISION_EPSILON {
                        java_min(amount, distance)
                    } else {
                        amount
                    };
                }
            }
        } else if amount < 0.0 {
            let start = self.find_index(axis, aabb.min(axis) + COLLISION_EPSILON);
            for layer in (0..start).rev() {
                if occupied_layer(layer) {
                    let distance = self.coords[i][layer as usize + 1] - aabb.min(axis);
                    return if distance <= COLLISION_EPSILON {
                        java_max(amount, distance)
                    } else {
                        amount
                    };
                }
            }
        }
        amount
    }
    pub fn collide_all<'a>(
        axis: Axis,
        aabb: Aabb,
        shapes: impl IntoIterator<Item = &'a Self>,
        mut amount: f64,
    ) -> f64 {
        for shape in shapes {
            if amount.abs() < COLLISION_EPSILON {
                return 0.0;
            }
            amount = shape.collide(axis, aabb, amount);
        }
        amount
    }
    pub fn clip(&self, start: Vec3, end: Vec3, block: BlockPos) -> Option<BlockHit> {
        if self.is_empty() {
            return None;
        }
        let delta = end.subtract(start);
        if delta.length_squared() < COLLISION_EPSILON {
            return None;
        }
        let nudged = start.add_vector(delta.scale(0.001));
        let x = self.find_index(Axis::X, nudged.x - block.x as f64);
        let y = self.find_index(Axis::Y, nudged.y - block.y as f64);
        let z = self.find_index(Axis::Z, nudged.z - block.z as f64);
        if self.is_full(x, y, z) {
            Some(BlockHit {
                location: nudged,
                direction: Direction::approximate_nearest(delta).opposite(),
                block,
                inside: true,
            })
        } else {
            Aabb::clip_boxes(self.boxes.iter().copied(), start, end, block)
        }
    }
    pub fn closest_point(&self, point: Vec3) -> Option<Vec3> {
        let mut best = None;
        for aabb in self.boxes.iter() {
            let candidate = Vec3::new(
                clamp(point.x, aabb.min(Axis::X), aabb.max(Axis::X)),
                clamp(point.y, aabb.min(Axis::Y), aabb.max(Axis::Y)),
                clamp(point.z, aabb.min(Axis::Z), aabb.max(Axis::Z)),
            );
            if best
                .is_none_or(|old| point.distance_squared(candidate) < point.distance_squared(old))
            {
                best = Some(candidate);
            }
        }
        best
    }
    fn decompose(&self) -> Vec<Aabb> {
        let mut remaining = self.occupancy.to_vec();
        let mut boxes = Vec::new();
        let [nx, ny, nz] = self.sizes;
        for y in 0..ny {
            for x in 0..nx {
                let mut run = None;
                for z in 0..=nz {
                    if z < nz && full(&remaining, index(self.sizes, x, y, z)) {
                        if run.is_none() {
                            run = Some(z);
                        }
                        continue;
                    }
                    let Some(z_start) = run.take() else { continue };
                    clear_strip(&mut remaining, self.sizes, x, y, z_start, z);
                    let mut x_end = x + 1;
                    while x_end < nx && strip_full(&remaining, self.sizes, x_end, y, z_start, z) {
                        clear_strip(&mut remaining, self.sizes, x_end, y, z_start, z);
                        x_end += 1;
                    }
                    let mut y_end = y + 1;
                    while y_end < ny
                        && (x..x_end)
                            .all(|u| strip_full(&remaining, self.sizes, u, y_end, z_start, z))
                    {
                        for u in x..x_end {
                            clear_strip(&mut remaining, self.sizes, u, y_end, z_start, z);
                        }
                        y_end += 1;
                    }
                    boxes.push(Aabb::new(
                        Vec3::new(
                            self.coords[0][x],
                            self.coords[1][y],
                            self.coords[2][z_start],
                        ),
                        Vec3::new(
                            self.coords[0][x_end],
                            self.coords[1][y_end],
                            self.coords[2][z],
                        ),
                    ));
                }
            }
        }
        boxes
    }
}

fn volume(sizes: [usize; 3]) -> Result<usize, ShapeError> {
    let count = sizes
        .into_iter()
        .try_fold(1usize, usize::checked_mul)
        .ok_or(ShapeError::TooLarge)?;
    if count > 1_048_576 {
        Err(ShapeError::TooLarge)
    } else {
        Ok(count)
    }
}
fn index(sizes: [usize; 3], x: usize, y: usize, z: usize) -> usize {
    (x * sizes[1] + y) * sizes[2] + z
}
fn full(bits: &[u64], i: usize) -> bool {
    bits[i / 64] & (1 << (i % 64)) != 0
}
fn strip_full(bits: &[u64], sizes: [usize; 3], x: usize, y: usize, min: usize, max: usize) -> bool {
    (min..max).all(|z| full(bits, index(sizes, x, y, z)))
}
fn clear_strip(bits: &mut [u64], sizes: [usize; 3], x: usize, y: usize, min: usize, max: usize) {
    for z in min..max {
        let i = index(sizes, x, y, z);
        bits[i / 64] &= !(1 << (i % 64));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_at_obstacle_and_reuse_boxes() {
        let shape = VoxelShape::block();
        let player = Aabb::new(Vec3::new(-0.6, 0.0, 0.2), Vec3::new(-0.2, 1.8, 0.8));
        assert_eq!(shape.collide(Axis::X, player, 1.0), 0.2);
        assert_eq!(shape.collide(Axis::Z, player, 1.0), 1.0);
        assert_eq!(shape.boxes().len(), 1);
        assert!(
            shape
                .clip(
                    Vec3::new(0.5, 0.5, 0.5),
                    Vec3::new(2.0, 0.5, 0.5),
                    BlockPos::default()
                )
                .unwrap()
                .inside
        );
        assert_eq!(
            VoxelShape::empty().collide(Axis::X, player, -0.0).to_bits(),
            (-0.0f64).to_bits()
        );
    }
    #[test]
    fn sparse_stair_decomposition() {
        let shape = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, true, false, false, true, true, true, true],
        )
        .unwrap();
        assert_eq!(shape.boxes().len(), 2);
        assert_eq!(
            shape.closest_point(Vec3::new(0.1, 0.9, 0.5)),
            Some(Vec3::new(0.1, 0.5, 0.5))
        );
        assert!(VoxelShape::from_grid(std::array::from_fn(|_| vec![0.0, 1.0]), &[]).is_err());
    }
    #[test]
    fn moved_boxes_are_the_moved_shapes_decomposition() {
        let stair = VoxelShape::from_cube(
            [2, 2, 2],
            &[true, true, false, false, true, true, true, true],
        )
        .unwrap();
        let slab = VoxelShape::from_grid([vec![0.0, 1.0], vec![0.0, 0.5], vec![0.0, 1.0]], &[true])
            .unwrap();
        let offsets = [
            Vec3::new(3.0, -64.0, -7.0),
            Vec3::new(-0.0, 0.0, -0.0),
            Vec3::new(1.0e-9, 319.999, -30_000_000.0),
            Vec3::new(f64::from(i32::MAX), f64::from(i32::MIN), 0.1),
        ];
        for shape in [VoxelShape::block(), stair, slab] {
            for offset in offsets {
                let moved = shape.moved(offset);
                let [nx, ny, nz] = shape.sizes();
                let occupied: Vec<bool> = (0..nx * ny * nz)
                    .map(|i| {
                        shape.is_full(
                            (i / (ny * nz)) as i32,
                            (i / nz % ny) as i32,
                            (i % nz) as i32,
                        )
                    })
                    .collect();
                let rebuilt = VoxelShape::from_grid(
                    Axis::ALL.map(|axis| moved.coordinates(axis).to_vec()),
                    &occupied,
                )
                .unwrap();
                assert_eq!(moved.boxes(), rebuilt.boxes());
                assert_eq!(moved.bounds(), rebuilt.bounds());
            }
        }
        assert!(VoxelShape::block().is_block());
        assert!(!VoxelShape::block().moved(Vec3::ZERO).is_block());
        assert!(!VoxelShape::block().fresh_identity().is_block());
    }
}
