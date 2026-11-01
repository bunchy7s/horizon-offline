//! `BlendingData` and `Blender`: the terrain, biomes and carving of a new
//! chunk next to chunks an older version generated (`blending_data` in the
//! save), so a world upgraded from before 1.18 meets its old terrain without
//! a seam.
use super::biome::BiomeId;
use horizon_core::{
    density::{DensityBlender, DensitySampler, SampleError, SamplerContext},
    noise::{DensityVolume, Noise, NormalNoise},
    position::ChunkPos,
    random::XoroshiroRandom,
};
use std::sync::Arc;

const QUARTS_PER_SECTION: i32 = 4;
const MAX_INSIDE: i32 = QUARTS_PER_SECTION - 1;
const MAX_OUTSIDE: i32 = QUARTS_PER_SECTION;
const COLUMN_INSIDE_COUNT: i32 = 2 * MAX_INSIDE + 1;
const COLUMN_OUTSIDE_COUNT: i32 = 2 * MAX_OUTSIDE + 1;
/// `BlendingData.CELL_COLUMN_COUNT`: the length of a saved `heights` array.
pub const CELL_COLUMN_COUNT: usize = (COLUMN_INSIDE_COUNT + COLUMN_OUTSIDE_COUNT) as usize;
/// `BlendingData.NO_VALUE` (`Float.MAX_VALUE`).
pub const NO_VALUE: f32 = f32::MAX;
/// `Blender.HEIGHT_BLENDING_RANGE_CELLS`: `QuartPos.fromSection(7) - 1`.
const HEIGHT_BLENDING_RANGE_CELLS: i32 = 4 * 7 - 1;
/// `Blender.HEIGHT_BLENDING_RANGE_CHUNKS`: `QuartPos.toSection(cells + 3)`.
pub const HEIGHT_BLENDING_RANGE_CHUNKS: i32 = (HEIGHT_BLENDING_RANGE_CELLS + 3) >> 2;
/// `Blender.DENSITY_BLENDING_RANGE_CHUNKS`: `QuartPos.toSection(5)`.
const DENSITY_BLENDING_RANGE_CHUNKS: i32 = 5 >> 2;

/// `CompositeDirection.Direction8` in its declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction8 {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}
impl Direction8 {
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
    ];
    pub fn step(self) -> (i32, i32) {
        match self {
            Self::North => (0, -1),
            Self::NorthEast => (1, -1),
            Self::East => (1, 0),
            Self::SouthEast => (1, 1),
            Self::South => (0, 1),
            Self::SouthWest => (-1, 1),
            Self::West => (-1, 0),
            Self::NorthWest => (-1, -1),
        }
    }
    fn bit(self) -> u8 {
        1 << Self::ALL.iter().position(|d| *d == self).unwrap()
    }
}

/// The directions of `Direction8` a chunk's neighbour is in, as a set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides(u8);
impl Sides {
    pub fn insert(&mut self, side: Direction8) {
        self.0 |= side.bit();
    }
    pub fn contains(self, side: Direction8) -> bool {
        self.0 & side.bit() != 0
    }
}

/// What `BlendingData` reads of an old chunk (`ChunkAccess`), with x and z
/// relative to the chunk and y absolute.
pub trait OldChunk {
    /// `hasPrimedHeightmap(WORLD_SURFACE_WG)`.
    fn has_surface_heightmap(&self) -> bool;
    /// `getHeight(WORLD_SURFACE_WG, x, z)`: the first free y above the surface.
    fn surface_height(&self, x: i32, z: i32) -> i32;
    /// The block is one of `BlendingData.SURFACE_BLOCKS`.
    fn is_surface_block(&self, x: i32, y: i32, z: i32) -> bool;
    /// `BlendingData.isGround`: not air, leaves, logs or a huge mushroom, and
    /// with a collision shape.
    fn is_ground(&self, x: i32, y: i32, z: i32) -> bool;
    /// `getNoiseBiome` at the quart column `(qx, qz)` of the chunk (0..4)
    /// and absolute quart height `qy`.
    fn noise_biome(&self, qx: i32, qy: i32, qz: i32) -> BiomeId;
}

/// `BlendingData`: an old chunk's border heights, densities and biomes.
#[derive(Debug)]
pub struct BlendingData {
    min_section: i32,
    max_section: i32,
    heights: [f32; CELL_COLUMN_COUNT],
    densities: [Option<Vec<f32>>; CELL_COLUMN_COUNT],
    biomes: [Option<Vec<BiomeId>>; CELL_COLUMN_COUNT],
}

fn inside_index(x: i32, z: i32) -> usize {
    (MAX_INSIDE - x + z) as usize
}
fn outside_index(x: i32, z: i32) -> usize {
    (COLUMN_INSIDE_COUNT + x + MAX_OUTSIDE - z) as usize
}
fn zero_if_negative(value: i32) -> i32 {
    value & !(value >> 31)
}
fn index_x(index: usize) -> i32 {
    let index = index as i32;
    if index < COLUMN_INSIDE_COUNT {
        return zero_if_negative(MAX_INSIDE - index);
    }
    MAX_OUTSIDE - zero_if_negative(MAX_OUTSIDE - (index - COLUMN_INSIDE_COUNT))
}
fn index_z(index: usize) -> i32 {
    let index = index as i32;
    if index < COLUMN_INSIDE_COUNT {
        return zero_if_negative(index - MAX_INSIDE);
    }
    MAX_OUTSIDE - zero_if_negative(index - COLUMN_INSIDE_COUNT - MAX_OUTSIDE)
}
fn floor_f32(value: f32) -> i32 {
    let truncated = value as i32;
    if value < truncated as f32 {
        truncated - 1
    } else {
        truncated
    }
}

impl BlendingData {
    /// `BlendingData.unpack`: the saved `blending_data` (`min_section`,
    /// `max_section` and the optional `heights`).
    pub fn unpack(min_section: i32, max_section: i32, heights: Option<&[f32]>) -> Option<Self> {
        let mut all = [NO_VALUE; CELL_COLUMN_COUNT];
        if let Some(heights) = heights {
            if heights.len() != CELL_COLUMN_COUNT {
                return None;
            }
            all.copy_from_slice(heights);
        }
        Some(Self {
            min_section,
            max_section,
            heights: all,
            densities: Default::default(),
            biomes: Default::default(),
        })
    }
    /// `BlendingData.pack`: the saved form, `heights` only when one is set.
    pub fn pack(&self) -> (i32, i32, Option<[f32; CELL_COLUMN_COUNT]>) {
        let has_height = self.heights.iter().any(|h| *h != NO_VALUE);
        (
            self.min_section,
            self.max_section,
            has_height.then_some(self.heights),
        )
    }
    /// The lowest block y of the area the old generation covered.
    pub fn min_y(&self) -> i32 {
        self.min_section << 4
    }
    /// Its highest block y.
    pub fn max_y(&self) -> i32 {
        (self.max_section << 4) - 1
    }
    fn sections_count(&self) -> i32 {
        self.max_section - self.min_section
    }
    fn height_blocks(&self) -> i32 {
        self.sections_count() << 4
    }
    fn cell_count_per_column(&self) -> usize {
        (self.sections_count() * 2) as usize
    }
    fn quart_count_per_column(&self) -> usize {
        (self.sections_count() * 4) as usize
    }
    fn column_min_y(&self) -> i32 {
        self.min_cell_y() + 1
    }
    fn min_cell_y(&self) -> i32 {
        self.min_section * 2
    }
    fn cell_y_index(&self, cell_y: i32) -> i32 {
        cell_y - self.column_min_y()
    }

    /// `getOrUpdateBlendingData` + `calculateData`: the data with the border
    /// columns read from `chunk`, which has `new_sides` (the neighbours that
    /// are not old) around it.
    pub fn calculated(mut self, chunk: &dyn OldChunk, new_sides: Sides) -> Self {
        let has = |d| new_sides.contains(d);
        if has(Direction8::North) || has(Direction8::West) || has(Direction8::NorthWest) {
            self.add_values_for_column(inside_index(0, 0), chunk, 0, 0);
        }
        if has(Direction8::North) {
            for i in 1..QUARTS_PER_SECTION {
                self.add_values_for_column(inside_index(i, 0), chunk, 4 * i, 0);
            }
        }
        if has(Direction8::West) {
            for i in 1..QUARTS_PER_SECTION {
                self.add_values_for_column(inside_index(0, i), chunk, 0, 4 * i);
            }
        }
        if has(Direction8::East) {
            for i in 1..QUARTS_PER_SECTION {
                self.add_values_for_column(outside_index(MAX_OUTSIDE, i), chunk, 15, 4 * i);
            }
        }
        if has(Direction8::South) {
            for i in 0..QUARTS_PER_SECTION {
                self.add_values_for_column(outside_index(i, MAX_OUTSIDE), chunk, 4 * i, 15);
            }
        }
        if has(Direction8::East) && has(Direction8::NorthEast) {
            self.add_values_for_column(outside_index(MAX_OUTSIDE, 0), chunk, 15, 0);
        }
        if has(Direction8::East) && has(Direction8::South) && has(Direction8::SouthEast) {
            self.add_values_for_column(outside_index(MAX_OUTSIDE, MAX_OUTSIDE), chunk, 15, 15);
        }
        self
    }

    fn add_values_for_column(&mut self, index: usize, chunk: &dyn OldChunk, x: i32, z: i32) {
        if self.heights[index] == NO_VALUE {
            self.heights[index] = self.height_at_xz(chunk, x, z) as f32;
        }
        self.densities[index] =
            Some(self.density_column(chunk, x, z, floor_f32(self.heights[index])));
        self.biomes[index] = Some(self.biome_column(chunk, x, z));
    }

    fn height_at_xz(&self, chunk: &dyn OldChunk, x: i32, z: i32) -> i32 {
        let max_y = self.max_y();
        let mut height = if chunk.has_surface_heightmap() {
            chunk.surface_height(x, z).min(max_y)
        } else {
            max_y
        };
        let min_y = self.min_y();
        while height > min_y {
            if chunk.is_surface_block(x, height, z) {
                return height;
            }
            height -= 1;
        }
        min_y
    }

    fn density_column(&self, chunk: &dyn OldChunk, x: i32, z: i32, height: i32) -> Vec<f32> {
        let count = self.cell_count_per_column();
        let mut densities = vec![-1.0f32; count];
        let mut y = self.max_y() + 1;
        let read1 = |y: &mut i32| {
            *y -= 1;
            if chunk.is_ground(x, *y, z) {
                1.0f32
            } else {
                -1.0f32
            }
        };
        let read7 = |y: &mut i32| {
            let mut sum = 0.0f32;
            for _ in 0..7 {
                sum += read1(y);
            }
            sum
        };
        let mut last7 = read7(&mut y);
        for cell in (0..count - 1).rev() {
            let one = read1(&mut y);
            let current7 = read7(&mut y);
            densities[cell] = (last7 + one + current7) / 15.0f32;
            last7 = current7;
        }
        let highest = self.cell_y_index(height.div_euclid(8));
        if highest >= 0 && (highest as usize) < count - 1 {
            let in_cell = (height as f32 + 0.5f32) % 8.0f32 / 8.0f32;
            let amplitude = (1.0f32 - in_cell) / in_cell;
            let max = amplitude.max(1.0f32) * 0.25f32;
            densities[highest as usize + 1] = -amplitude / max;
            densities[highest as usize] = 1.0f32 / max;
        }
        densities
    }

    fn biome_column(&self, chunk: &dyn OldChunk, x: i32, z: i32) -> Vec<BiomeId> {
        let min_quart = self.min_y() >> 2;
        (0..self.quart_count_per_column())
            .map(|i| chunk.noise_biome(x >> 2, i as i32 + min_quart, z >> 2))
            .collect()
    }

    /// `getHeight(cellX, cellY, cellZ)`.
    fn height(&self, cell_x: i32, _cell_y: i32, cell_z: i32) -> f32 {
        if cell_x == MAX_OUTSIDE || cell_z == MAX_OUTSIDE {
            self.heights[outside_index(cell_x, cell_z)]
        } else if cell_x != 0 && cell_z != 0 {
            NO_VALUE
        } else {
            self.heights[inside_index(cell_x, cell_z)]
        }
    }
    fn column_density(&self, column: Option<&Vec<f32>>, cell_y: i32) -> f32 {
        let Some(column) = column else {
            return NO_VALUE;
        };
        let index = self.cell_y_index(cell_y);
        if index >= 0 && (index as usize) < column.len() {
            column[index as usize] * 0.1f32
        } else {
            NO_VALUE
        }
    }
    /// `getDensity(cellX, cellY, cellZ)`.
    fn density(&self, cell_x: i32, cell_y: i32, cell_z: i32) -> f32 {
        if cell_y == self.min_cell_y() {
            0.1f32
        } else if cell_x == MAX_OUTSIDE || cell_z == MAX_OUTSIDE {
            self.column_density(
                self.densities[outside_index(cell_x, cell_z)].as_ref(),
                cell_y,
            )
        } else if cell_x != 0 && cell_z != 0 {
            NO_VALUE
        } else {
            self.column_density(
                self.densities[inside_index(cell_x, cell_z)].as_ref(),
                cell_y,
            )
        }
    }
    fn iterate_biomes(
        &self,
        min_cell_x: i32,
        quart_y: i32,
        min_cell_z: i32,
        mut consumer: impl FnMut(i32, i32, BiomeId),
    ) {
        let min_quart = self.min_y() >> 2;
        let max_quart = self.max_y() >> 2;
        if quart_y < min_quart || quart_y > max_quart {
            return;
        }
        let quart_index = (quart_y - min_quart) as usize;
        for (i, column) in self.biomes.iter().enumerate() {
            if let Some(column) = column {
                consumer(
                    min_cell_x + index_x(i),
                    min_cell_z + index_z(i),
                    column[quart_index],
                );
            }
        }
    }
    fn iterate_heights(
        &self,
        min_cell_x: i32,
        min_cell_z: i32,
        mut consumer: impl FnMut(i32, i32, f32),
    ) {
        for (i, height) in self.heights.iter().enumerate() {
            if *height != NO_VALUE {
                consumer(min_cell_x + index_x(i), min_cell_z + index_z(i), *height);
            }
        }
    }
    fn iterate_densities(
        &self,
        min_cell_x: i32,
        min_cell_z: i32,
        from_cell_y: i32,
        to_cell_y: i32,
        mut consumer: impl FnMut(i32, i32, i32, f32),
    ) {
        let min_cell_y = self.column_min_y();
        let min_index = 0.max(from_cell_y - min_cell_y);
        let max_index = (self.cell_count_per_column() as i32).min(to_cell_y - min_cell_y);
        for (i, column) in self.densities.iter().enumerate() {
            let Some(column) = column else { continue };
            let (x, z) = (min_cell_x + index_x(i), min_cell_z + index_z(i));
            for y in min_index..max_index {
                consumer(x, y + min_cell_y, z, column[y as usize] * 0.1f32);
            }
        }
    }
}

/// The old chunks around a chunk being generated: what `WorldGenRegion`
/// tells `Blender.of` and the carving filter.
pub trait OldWorld: Send + Sync {
    /// `WorldGenRegion.isOldChunkAround`: a chunk within `range` of `center`
    /// (a square) is old, as far as the storage can tell without reading it.
    fn is_old_chunk_around(&self, center: ChunkPos, range: i32) -> bool;
    /// The blending data of the old chunk at `x, z`, its border columns
    /// calculated, or None for a chunk that is not old.
    fn blending(&self, x: i32, z: i32) -> Option<Arc<BlendingData>>;
    /// The biomes an old chunk keeps (what the chunk stores, not what the
    /// generator would make there), which its neighbours' surfaces read.
    fn old_biomes(&self, x: i32, z: i32) -> Option<Arc<super::biome_chunk::BiomeChunk<BiomeId>>>;
    /// The structure starts an old chunk saved (`structures.starts`): each
    /// structure's name and the union of its pieces' bounding boxes
    /// (`[min x, min y, min z, max x, max y, max z]`). What the chunk saved
    /// is what `getStructureAt` and `/locate` see there, not what this
    /// generator would make.
    fn saved_starts(&self, _x: i32, _z: i32) -> Vec<SavedStart> {
        Vec::new()
    }
}

/// A structure start an old chunk saved (`structures.starts.<name>`).
#[derive(Clone, Debug, PartialEq)]
pub struct SavedStart {
    pub name: String,
    /// The union of the pieces' `BB`: `[min x, min y, min z, max x, max y, max z]`.
    pub bounds: Option<[i32; 6]>,
    /// The start's `Children`, as saved.
    pub pieces: Vec<crate::nbt::Tag>,
}

/// `Blender.BlendingOutput`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendingOutput {
    pub alpha: f32,
    pub offset: f32,
}

/// `Blender`: empty (`Blender.empty()`) when no old chunk is near.
#[derive(Debug, Default)]
pub struct Blender {
    height_and_biome: OpenMap<(ChunkPos, Arc<BlendingData>)>,
    density: OpenMap<(ChunkPos, Arc<BlendingData>)>,
    carving: Option<OldChunkDistance>,
}

fn shift_noise() -> &'static dyn Noise {
    static NOISE: std::sync::OnceLock<horizon_core::noise::NoiseStack> = std::sync::OnceLock::new();
    NOISE.get_or_init(|| {
        NormalNoise::create_parity(-3, &[1.0, 1.0, 1.0, 0.0])
            .expect("the default shift noise")
            .create(&mut XoroshiroRandom::new(42))
            .expect("the default shift noise")
    })
}

#[allow(clippy::neg_cmp_op_on_partial_ord)] // vanilla's `!(a > b)`: a NaN passes
impl Blender {
    /// `Blender.empty`.
    pub fn empty() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.height_and_biome.is_empty() && self.density.is_empty() && self.carving.is_none()
    }
    /// `Blender.of`: the blender of the chunk at `center`.
    pub fn of(world: &dyn OldWorld, center: ChunkPos) -> Self {
        let range = HEIGHT_BLENDING_RANGE_CHUNKS;
        if !world.is_old_chunk_around(center, range) {
            return Self::empty();
        }
        let around = |dx: i32, dz: i32| world.blending(center.x + dx, center.z + dz);
        let mut height_and_biome = OpenMap::default();
        let mut density = OpenMap::default();
        let max_dist_sq = (range + 1) * (range + 1);
        for dx in -range..=range {
            for dz in -range..=range {
                if dx * dx + dz * dz > max_dist_sq {
                    continue;
                }
                let Some(data) = around(dx, dz) else { continue };
                let pos = ChunkPos::new(center.x + dx, center.z + dz);
                height_and_biome.insert(pos.pack(), (pos, data.clone()));
                if dx.abs() <= DENSITY_BLENDING_RANGE_CHUNKS
                    && dz.abs() <= DENSITY_BLENDING_RANGE_CHUNKS
                {
                    density.insert(pos.pack(), (pos, data));
                }
            }
        }
        let carving = OldChunkDistance::around(world, center);
        if height_and_biome.is_empty() && density.is_empty() && carving.is_none() {
            return Self::empty();
        }
        Self {
            height_and_biome,
            density,
            carving,
        }
    }

    /// `Blender.blendOffsetAndFactor(x, z)` for a block column.
    pub fn blend_offset_and_factor(&self, block_x: i32, block_z: i32) -> BlendingOutput {
        if self.is_empty() {
            return BlendingOutput {
                alpha: 1.0,
                offset: 0.0,
            };
        }
        let cell_x = block_x >> 2;
        let cell_z = block_z >> 2;
        let fixed = self.blending_value(cell_x, 0, cell_z, BlendingData::height);
        if fixed != NO_VALUE {
            return BlendingOutput {
                alpha: 0.0,
                offset: height_to_offset(f64::from(fixed)) as f32,
            };
        }
        let mut total_weight = 0.0f64;
        let mut weighted_heights = 0.0f64;
        let mut closest = f64::INFINITY;
        for (pos, data) in self.height_and_biome.iter() {
            data.iterate_heights(pos.x << 2, pos.z << 2, |test_x, test_z, height| {
                let distance = length2(f64::from(cell_x - test_x), f64::from(cell_z - test_z));
                if !(distance > f64::from(HEIGHT_BLENDING_RANGE_CELLS)) {
                    if distance < closest {
                        closest = distance;
                    }
                    let weight = 1.0 / (distance * distance * distance * distance);
                    weighted_heights += f64::from(height) * weight;
                    total_weight += weight;
                }
            });
        }
        if closest == f64::INFINITY {
            return BlendingOutput {
                alpha: 1.0,
                offset: 0.0,
            };
        }
        let average = weighted_heights / total_weight;
        let mut alpha = (closest / f64::from(HEIGHT_BLENDING_RANGE_CELLS + 1)).clamp(0.0, 1.0);
        alpha = 3.0 * alpha * alpha - 2.0 * alpha * alpha * alpha;
        BlendingOutput {
            alpha: alpha as f32,
            offset: height_to_offset(average) as f32,
        }
    }

    /// `Blender.blendDensity`.
    pub fn blend_density(&self, block_x: i32, block_y: i32, block_z: i32, noise: f32) -> f32 {
        if self.is_empty() {
            return noise;
        }
        let cell_x = block_x >> 2;
        let cell_y = block_y.div_euclid(8);
        let cell_z = block_z >> 2;
        let fixed = self.blending_value(cell_x, cell_y, cell_z, BlendingData::density);
        if fixed != NO_VALUE {
            return fixed;
        }
        let mut total_weight = 0.0f64;
        let mut weighted = 0.0f64;
        let mut closest = f64::INFINITY;
        for (pos, data) in self.density.iter() {
            data.iterate_densities(
                pos.x << 2,
                pos.z << 2,
                cell_y - 1,
                cell_y + 1,
                |test_x, test_y, test_z, density| {
                    let distance = length3(
                        f64::from(cell_x - test_x),
                        f64::from((cell_y - test_y) * 2),
                        f64::from(cell_z - test_z),
                    );
                    if !(distance > 2.0) {
                        if distance < closest {
                            closest = distance;
                        }
                        let weight = 1.0 / (distance * distance * distance * distance);
                        weighted += f64::from(density) * weight;
                        total_weight += weight;
                    }
                },
            );
        }
        if closest == f64::INFINITY {
            return noise;
        }
        let average = weighted / total_weight;
        let alpha = (closest / 3.0).clamp(0.0, 1.0);
        (average + alpha * (f64::from(noise) - average)) as f32
    }

    fn blending_value(
        &self,
        cell_x: i32,
        cell_y: i32,
        cell_z: i32,
        getter: fn(&BlendingData, i32, i32, i32) -> f32,
    ) -> f32 {
        let chunk_x = cell_x >> 2;
        let chunk_z = cell_z >> 2;
        let min_x = cell_x & 3 == 0;
        let min_z = cell_z & 3 == 0;
        let mut value = self.value_in(getter, chunk_x, chunk_z, cell_x, cell_y, cell_z);
        if value == NO_VALUE {
            if min_x && min_z {
                value = self.value_in(getter, chunk_x - 1, chunk_z - 1, cell_x, cell_y, cell_z);
            }
            if value == NO_VALUE {
                if min_x {
                    value = self.value_in(getter, chunk_x - 1, chunk_z, cell_x, cell_y, cell_z);
                }
                if value == NO_VALUE && min_z {
                    value = self.value_in(getter, chunk_x, chunk_z - 1, cell_x, cell_y, cell_z);
                }
            }
        }
        value
    }
    fn value_in(
        &self,
        getter: fn(&BlendingData, i32, i32, i32) -> f32,
        chunk_x: i32,
        chunk_z: i32,
        cell_x: i32,
        cell_y: i32,
        cell_z: i32,
    ) -> f32 {
        match self
            .height_and_biome
            .get(ChunkPos::new(chunk_x, chunk_z).pack())
        {
            Some((_, data)) => getter(
                data,
                cell_x - (chunk_x << 2),
                cell_y,
                cell_z - (chunk_z << 2),
            ),
            None => NO_VALUE,
        }
    }

    /// `Blender.getBiomeResolver`: the old chunks' biome at a quart position,
    /// or None where the new generator's biome stands.
    pub fn blend_biome(&self, quart_x: i32, quart_y: i32, quart_z: i32) -> Option<BiomeId> {
        let mut closest = f32::INFINITY;
        let mut biome = None;
        for (pos, data) in self.height_and_biome.iter() {
            data.iterate_biomes(pos.x << 2, quart_y, pos.z << 2, |test_x, test_z, found| {
                let distance = length2_f32((quart_x - test_x) as f32, (quart_z - test_z) as f32);
                if !(distance > HEIGHT_BLENDING_RANGE_CELLS as f32) && distance < closest {
                    biome = Some(found);
                    closest = distance;
                }
            });
        }
        if closest == f32::INFINITY {
            return None;
        }
        let shift = shift_noise().get3(f64::from(quart_x), 0.0, f64::from(quart_z)) * 12.0f32;
        let alpha = ((closest + shift) / (HEIGHT_BLENDING_RANGE_CELLS + 1) as f32).clamp(0.0, 1.0);
        if alpha > 0.5 { None } else { biome }
    }

    /// `Blender.getCarvingFilter`: whether a block is within 4 blocks of an
    /// old chunk's area, after the shift noise moves it.
    pub fn carving_filter(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let distance = self.carving.as_ref()?;
        let noise = shift_noise();
        let sx = f64::from(x)
            + 0.5
            + f64::from(noise.get3(f64::from(x), f64::from(y), f64::from(z)) * 4.0f32);
        let sy = f64::from(y)
            + 0.5
            + f64::from(noise.get3(f64::from(y), f64::from(z), f64::from(x)) * 4.0f32);
        let sz = f64::from(z)
            + 0.5
            + f64::from(noise.get3(f64::from(z), f64::from(x), f64::from(y)) * 4.0f32);
        Some(distance.distance(sx, sy, sz) < 4.0)
    }
    pub fn has_carving_filter(&self) -> bool {
        self.carving.is_some()
    }

    /// `Blender.OutputBuffer` for the columns of `volume`: the alpha and
    /// offset fields of the density graph.
    pub fn output_buffer(self: &Arc<Self>, volume: DensityVolume) -> OutputBuffer {
        let min_x = volume.minimum()[0] >> 2;
        let min_z = volume.minimum()[2] >> 2;
        let max_x = (volume.maximum()[0] >> 2) + 1;
        let max_z = (volume.maximum()[2] >> 2) + 1;
        let count_x = max_x - min_x + 1;
        let count_z = max_z - min_z + 1;
        let mut alphas = vec![0.0f32; (count_x * count_z) as usize];
        let mut offsets = alphas.clone();
        for z in 0..count_z {
            for x in 0..count_x {
                let output = self.blend_offset_and_factor((min_x + x) << 2, (min_z + z) << 2);
                alphas[(x + z * count_x) as usize] = output.alpha;
                offsets[(x + z * count_x) as usize] = output.offset;
            }
        }
        OutputBuffer {
            blender: self.clone(),
            min_cell_x: min_x,
            min_cell_z: min_z,
            count_x,
            count_z,
            alphas,
            offsets,
        }
    }
}

/// fastutil's `Long2ObjectOpenHashMap`, for the order it walks its entries in:
/// `Blender` adds floating point sums in that order, and a sum's last bits
/// depend on it.
#[derive(Debug)]
struct OpenMap<V> {
    n: usize,
    keys: Vec<i64>,
    values: Vec<Option<V>>,
    /// The value of the key 0, which the table keeps apart (at index `n`).
    null_value: Option<V>,
    size: usize,
    max_fill: usize,
}
impl<V> Default for OpenMap<V> {
    fn default() -> Self {
        let n = Self::array_size(16);
        Self {
            n,
            keys: vec![0; n],
            values: (0..n).map(|_| None).collect(),
            null_value: None,
            size: 0,
            max_fill: Self::max_fill(n),
        }
    }
}
impl<V> OpenMap<V> {
    /// `HashCommon.arraySize(expected, 0.75F)`.
    fn array_size(expected: usize) -> usize {
        let wanted = (expected as f32 / 0.75f32).ceil() as u64;
        wanted.next_power_of_two().max(2) as usize
    }
    /// `HashCommon.maxFill(n, 0.75F)`.
    fn max_fill(n: usize) -> usize {
        ((n as f32 * 0.75f32).ceil() as usize).min(n - 1)
    }
    /// `HashCommon.mix(long)`.
    fn mix(key: i64) -> usize {
        let mut h = (key as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= h >> 32;
        (h ^ (h >> 16)) as u32 as usize
    }
    fn is_empty(&self) -> bool {
        self.size == 0
    }
    fn get(&self, key: i64) -> Option<&V> {
        if key == 0 {
            return self.null_value.as_ref();
        }
        let mask = self.n - 1;
        let mut pos = Self::mix(key) & mask;
        while self.keys[pos] != 0 {
            if self.keys[pos] == key {
                return self.values[pos].as_ref();
            }
            pos = (pos + 1) & mask;
        }
        None
    }
    fn insert(&mut self, key: i64, value: V) {
        if key == 0 {
            if self.null_value.is_some() {
                self.null_value = Some(value);
                return;
            }
            self.null_value = Some(value);
        } else {
            let mask = self.n - 1;
            let mut pos = Self::mix(key) & mask;
            while self.keys[pos] != 0 {
                if self.keys[pos] == key {
                    self.values[pos] = Some(value);
                    return;
                }
                pos = (pos + 1) & mask;
            }
            self.keys[pos] = key;
            self.values[pos] = Some(value);
        }
        let grow = self.size >= self.max_fill;
        self.size += 1;
        if grow {
            self.rehash(Self::array_size(self.size + 1));
        }
    }
    fn rehash(&mut self, new_n: usize) {
        let mask = new_n - 1;
        let mut keys = vec![0i64; new_n];
        let mut values: Vec<Option<V>> = (0..new_n).map(|_| None).collect();
        let real = self.size - usize::from(self.null_value.is_some());
        let mut i = self.n;
        for _ in 0..real {
            loop {
                i -= 1;
                if self.keys[i] != 0 {
                    break;
                }
            }
            let mut pos = Self::mix(self.keys[i]) & mask;
            while keys[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            keys[pos] = self.keys[i];
            values[pos] = self.values[i].take();
        }
        self.n = new_n;
        self.keys = keys;
        self.values = values;
        self.max_fill = Self::max_fill(new_n);
    }
    /// The entries as `forEach` visits them: the key 0 first, then the table
    /// from its end to its start.
    fn iter(&self) -> impl Iterator<Item = &V> {
        self.null_value.iter().chain(
            (0..self.n)
                .rev()
                .filter(|&pos| self.keys[pos] != 0)
                .filter_map(|pos| self.values[pos].as_ref()),
        )
    }
}

fn height_to_offset(height: f64) -> f64 {
    let target_y = height + 0.5;
    let modulo = (target_y % 8.0 + 8.0) % 8.0;
    1.0 * (32.0 * (target_y - 128.0) - 3.0 * (target_y - 120.0) * modulo + 3.0 * modulo * modulo)
        / (128.0 * (32.0 - 3.0 * modulo))
}
fn length2(x: f64, z: f64) -> f64 {
    (x * x + z * z).sqrt()
}
fn length2_f32(x: f32, z: f32) -> f32 {
    (x * x + z * z).sqrt()
}
fn length3(x: f64, y: f64, z: f64) -> f64 {
    (x * x + y * y + z * z).sqrt()
}

impl DensityBlender for Blender {
    fn is_empty(&self) -> bool {
        Blender::is_empty(self)
    }
    fn blend_density(&self, [x, y, z]: [i32; 3], input: f32) -> f32 {
        Blender::blend_density(self, x, y, z, input)
    }
}

/// `Blender.OutputBuffer`: the alpha and offset of each quart column of a
/// volume, and of any other column on demand.
#[derive(Debug)]
pub struct OutputBuffer {
    blender: Arc<Blender>,
    min_cell_x: i32,
    min_cell_z: i32,
    count_x: i32,
    count_z: i32,
    alphas: Vec<f32>,
    offsets: Vec<f32>,
}
impl OutputBuffer {
    fn index(&self, block_x: i32, block_z: i32) -> Option<usize> {
        let x = (block_x >> 2) - self.min_cell_x;
        let z = (block_z >> 2) - self.min_cell_z;
        (x >= 0 && z >= 0 && x < self.count_x && z < self.count_z)
            .then(|| (x + z * self.count_x) as usize)
    }
    pub fn alpha(&self, block_x: i32, block_z: i32) -> f32 {
        match self.index(block_x, block_z) {
            Some(i) => self.alphas[i],
            None => self.blender.blend_offset_and_factor(block_x, block_z).alpha,
        }
    }
    pub fn offset(&self, block_x: i32, block_z: i32) -> f32 {
        match self.index(block_x, block_z) {
            Some(i) => self.offsets[i],
            None => {
                self.blender
                    .blend_offset_and_factor(block_x, block_z)
                    .offset
            }
        }
    }
}

/// `OutputBuffer.createAlphaSampler` / `createOffsetSampler`.
#[derive(Debug)]
pub struct BlendFieldSampler {
    pub buffer: Arc<OutputBuffer>,
    pub offset: bool,
}
impl DensitySampler for BlendFieldSampler {
    fn sample_value(
        &self,
        _: &mut SamplerContext,
        [x, _, z]: [i32; 3],
    ) -> Result<f32, SampleError> {
        Ok(if self.offset {
            self.buffer.offset(x, z)
        } else {
            self.buffer.alpha(x, z)
        })
    }
}

/// `Blender.makeOldChunkDistanceGetter`: the distance to the nearest old
/// chunk's box (the chunk itself and its eight neighbours).
#[derive(Debug)]
struct OldChunkDistance {
    /// `(offset x, offset z, centre y, radius y)` per old chunk.
    boxes: Vec<(f64, f64, f64, f64)>,
}
impl OldChunkDistance {
    fn around(world: &dyn OldWorld, center: ChunkPos) -> Option<Self> {
        let mut boxes = Vec::new();
        let mut add = |offset: (f64, f64), data: &BlendingData| {
            let radius_y = f64::from(data.height_blocks()) / 2.0;
            boxes.push((
                offset.0,
                offset.1,
                f64::from(data.min_y()) + radius_y,
                radius_y,
            ));
        };
        // `centerBlendingData` first, then the neighbours in `Direction8` order.
        if let Some(data) = world.blending(center.x, center.z) {
            add((0.0, 0.0), &data);
        }
        for side in Direction8::ALL {
            let (dx, dz) = side.step();
            if let Some(data) = world.blending(center.x + dx, center.z + dz) {
                add((f64::from(dx * 16), f64::from(dz * 16)), &data);
            }
        }
        (!boxes.is_empty()).then_some(Self { boxes })
    }
    fn distance(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut closest = f64::INFINITY;
        for &(offset_x, offset_z, center_y, radius_y) in &self.boxes {
            let dx = ((x - 8.0 - offset_x).abs() - 8.0).max(0.0);
            let dy = ((y - center_y).abs() - radius_y).max(0.0);
            let dz = ((z - 8.0 - offset_z).abs() - 8.0).max(0.0);
            let distance = length3(dx, dy, dz);
            if distance < closest {
                closest = distance;
            }
        }
        closest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_indices_walk_the_border_of_the_chunk() {
        // `getX`/`getZ` undo `getInsideIndex`/`getOutsideIndex`.
        for x in 0..4 {
            assert_eq!(
                (index_x(inside_index(x, 0)), index_z(inside_index(x, 0))),
                (x, 0)
            );
        }
        for z in 0..4 {
            assert_eq!(
                (index_x(inside_index(0, z)), index_z(inside_index(0, z))),
                (0, z)
            );
        }
        for i in 0..=4 {
            assert_eq!(
                (index_x(outside_index(i, 4)), index_z(outside_index(i, 4))),
                (i, 4)
            );
            assert_eq!(
                (index_x(outside_index(4, i)), index_z(outside_index(4, i))),
                (4, i)
            );
        }
        assert_eq!(CELL_COLUMN_COUNT, 16);
        assert_eq!(HEIGHT_BLENDING_RANGE_CHUNKS, 7);
    }

    #[test]
    fn the_open_map_keeps_every_entry_as_it_grows() {
        let mut map = OpenMap::default();
        for i in 0..200i64 {
            map.insert(
                ChunkPos::new((i % 17) as i32 - 8, (i / 17) as i32 - 5).pack(),
                i,
            );
        }
        assert_eq!(map.iter().count(), 200);
        assert_eq!(map.get(ChunkPos::new(0, 0).pack()), Some(&(8 + 5 * 17)));
        let mut seen: Vec<i64> = map.iter().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..200).collect::<Vec<_>>());
    }

    #[test]
    fn a_blender_without_old_chunks_changes_nothing() {
        let blender = Blender::empty();
        assert_eq!(
            blender.blend_offset_and_factor(5, 9),
            BlendingOutput {
                alpha: 1.0,
                offset: 0.0
            }
        );
        assert_eq!(blender.blend_density(1, 2, 3, 0.25), 0.25);
        assert!(blender.blend_biome(1, 2, 3).is_none());
    }
}
