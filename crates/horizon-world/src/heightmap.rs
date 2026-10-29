//! Packed column heights and the six Java 26.3 heightmap predicates.
//!
//! Block behavior supplies air/fluid facts and resolved block-tag membership.
//! Heightmaps do not infer those facts from collision shapes or block names.
use horizon_core::storage::{PackedStorage, StorageError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HeightmapType {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapType {
    pub const ALL: [Self; 6] = [
        Self::WorldSurfaceWg,
        Self::WorldSurface,
        Self::OceanFloorWg,
        Self::OceanFloor,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::WorldSurfaceWg => "WORLD_SURFACE_WG",
            Self::WorldSurface => "WORLD_SURFACE",
            Self::OceanFloorWg => "OCEAN_FLOOR_WG",
            Self::OceanFloor => "OCEAN_FLOOR",
            Self::MotionBlocking => "MOTION_BLOCKING",
            Self::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
        }
    }

    pub fn send_to_client(self) -> bool {
        matches!(
            self,
            Self::WorldSurface | Self::MotionBlocking | Self::MotionBlockingNoLeaves
        )
    }

    pub fn keep_after_worldgen(self) -> bool {
        !matches!(self, Self::WorldSurfaceWg | Self::OceanFloorWg)
    }

    pub fn is_opaque(self, state: impl HeightmapState) -> bool {
        match self {
            Self::WorldSurfaceWg | Self::WorldSurface => !state.is_air(),
            Self::OceanFloorWg | Self::OceanFloor => state.blocks_motion_in_heightmap(),
            Self::MotionBlocking => state.blocks_motion_in_heightmap() || state.has_fluid(),
            Self::MotionBlockingNoLeaves => {
                state.blocks_motion_in_heightmap_no_leaves() || state.has_fluid()
            }
        }
    }
}

pub trait HeightmapState: Copy {
    fn is_air(self) -> bool;
    /// Exact AIR block identity, distinct from cave/void air.
    fn is_air_block(self) -> bool;
    fn has_fluid(self) -> bool;
    fn blocks_motion_in_heightmap(self) -> bool;
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool;
}

pub trait HeightmapChunk {
    type State: HeightmapState;
    fn min_y(&self) -> i32;
    fn height(&self) -> i32;
    fn highest_section_y(&self) -> i32;
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeightmapError {
    InvalidBounds,
    MismatchedChunk,
    Storage(StorageError),
}
impl From<StorageError> for HeightmapError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl std::fmt::Display for HeightmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HeightmapError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heightmap {
    kind: HeightmapType,
    min_y: i32,
    height: i32,
    data: PackedStorage,
}

impl Heightmap {
    /// Bounded to the vanilla codec's positive, aligned vertical ranges.
    pub fn new(kind: HeightmapType, min_y: i32, height: i32) -> Result<Self, HeightmapError> {
        if !(-2032..=2031).contains(&min_y)
            || !(16..=4064).contains(&height)
            || min_y % 16 != 0
            || height % 16 != 0
            || min_y + height > 2032
        {
            return Err(HeightmapError::InvalidBounds);
        }
        let bits = (32 - (height as u32).leading_zeros()) as u8;
        Ok(Self {
            kind,
            min_y,
            height,
            data: PackedStorage::new(bits, 256)?,
        })
    }

    pub fn kind(&self) -> HeightmapType {
        self.kind
    }
    pub fn raw_data(&self) -> &[u64] {
        self.data.words()
    }

    fn index(x: i32, z: i32) -> usize {
        x.wrapping_add(z.wrapping_mul(16)) as usize
    }

    pub fn first_available(&self, x: i32, z: i32) -> Result<i32, HeightmapError> {
        Ok((self.data.get(Self::index(x, z))? as i32).wrapping_add(self.min_y))
    }

    /// Whether any column of the local square `x0..=x1` by `z0..=z1` (each
    /// within 0..16) has its first available y at or above `y`.
    pub fn any_first_available_at_least(
        &self,
        [x0, x1]: [i32; 2],
        [z0, z1]: [i32; 2],
        y: i32,
    ) -> bool {
        debug_assert!(0 <= x0 && x1 < 16 && 0 <= z0 && z1 < 16);
        (z0..=z1).any(|z| {
            (x0..=x1)
                .any(|x| (self.data.read(Self::index(x, z)) as i32).wrapping_add(self.min_y) >= y)
        })
    }

    pub fn highest_taken(&self, x: i32, z: i32) -> Result<i32, HeightmapError> {
        Ok(self.first_available(x, z)?.wrapping_sub(1))
    }

    fn set_height(&mut self, x: i32, z: i32, y: i32) -> Result<(), HeightmapError> {
        self.data
            .set(Self::index(x, z), y.wrapping_sub(self.min_y) as u32)?;
        Ok(())
    }

    fn check_chunk(&self, chunk: &impl HeightmapChunk) -> Result<(), HeightmapError> {
        if chunk.min_y() != self.min_y || chunk.height() != self.height {
            return Err(HeightmapError::MismatchedChunk);
        }
        Ok(())
    }

    /// The caller writes the new state to the chunk before updating its maps.
    pub fn update<C: HeightmapChunk>(
        &mut self,
        chunk: &C,
        x: i32,
        y: i32,
        z: i32,
        state: C::State,
    ) -> Result<bool, HeightmapError> {
        self.check_chunk(chunk)?;
        let first = self.first_available(x, z)?;
        if y <= first.wrapping_sub(2) {
            return Ok(false);
        }
        if self.kind.is_opaque(state) {
            if y >= first {
                self.set_height(x, z, y.wrapping_add(1))?;
                return Ok(true);
            }
        } else if y == first.wrapping_sub(1) {
            let mut scan_y = y.wrapping_sub(1);
            while scan_y >= self.min_y {
                if self.kind.is_opaque(chunk.block_state(x, scan_y, z)) {
                    self.set_height(x, z, scan_y.wrapping_add(1))?;
                    return Ok(true);
                }
                scan_y -= 1;
            }
            self.set_height(x, z, self.min_y)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Matching lengths preserve all bits, including padding and unused slots.
    /// A length mismatch primes this registered map from the chunk instead.
    pub fn set_raw_data(
        &mut self,
        chunk: &impl HeightmapChunk,
        words: &[u64],
    ) -> Result<(), HeightmapError> {
        self.check_chunk(chunk)?;
        if words.len() == self.data.words().len() {
            self.data = PackedStorage::from_words(self.data.bits(), 256, words.to_vec())?;
            Ok(())
        } else {
            Self::prime(chunk, &mut [self])
        }
    }

    /// Preserves caller map order and scans X, then Z, then descending Y.
    /// An unfound column retains its previous value, including during re-priming.
    pub fn prime(
        chunk: &impl HeightmapChunk,
        maps: &mut [&mut Self],
    ) -> Result<(), HeightmapError> {
        if maps.is_empty() {
            return Ok(());
        }
        for map in maps.iter() {
            map.check_chunk(chunk)?;
        }
        let upper = chunk.highest_section_y().wrapping_add(16);
        let mut remaining = Vec::with_capacity(maps.len());
        for x in 0..16 {
            for z in 0..16 {
                remaining.clear();
                remaining.extend(0..maps.len());
                let mut y = upper.wrapping_sub(1);
                while y >= chunk.min_y() {
                    let state = chunk.block_state(x, y, z);
                    if !state.is_air_block() {
                        let mut i = 0;
                        while i < remaining.len() {
                            let map = &mut maps[remaining[i]];
                            if map.kind.is_opaque(state) {
                                map.set_height(x, z, y.wrapping_add(1))?;
                                remaining.remove(i);
                            } else {
                                i += 1;
                            }
                        }
                        if remaining.is_empty() {
                            break;
                        }
                    }
                    y -= 1;
                }
            }
        }
        Ok(())
    }
}
