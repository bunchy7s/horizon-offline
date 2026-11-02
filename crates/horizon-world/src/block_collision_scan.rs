//! Lazy block collision selection in vanilla cursor/query order. Providers own
//! chunk availability, state metadata, suffocation predicates and context dispatch.
use crate::{collision_context::CollisionContext, registry::BlockStateId};
use horizon_core::{
    geometry::{Aabb, ShapeError, ShapeOp, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};

#[derive(Clone, Copy, Debug)]
pub struct CollisionBlock {
    pub state: BlockStateId,
    pub large_collision_shape: bool,
    pub moving_piston: bool,
    /// The shape is empty under every context (air, plants): a scan skips
    /// the block without asking for its shape. Providers whose shapes come
    /// from elsewhere leave it false.
    pub empty_shape: bool,
}

pub trait BlockCollisionWorld {
    type Error;
    type Chunk: Copy;
    /// None is a missing collision chunk; native iteration retries missing chunks.
    fn collision_chunk(&mut self, position: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error>;
    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        position: BlockPos,
    ) -> Result<CollisionBlock, Self::Error>;
    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        position: BlockPos,
    ) -> Result<bool, Self::Error>;
    /// Query against the global collision world, not the cached chunk provider.
    /// Shapes must preserve canonical block identity (including imported shapes).
    fn context_shape(
        &mut self,
        block: CollisionBlock,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, Self::Error>;
    /// Whether a block of the chunk's section `section_y` (block y >> 4) may
    /// have a large collision shape or be a moving piston: the only blocks
    /// the scan keeps outside the box's own cells. False lets the scan pass
    /// that section's outer cells without reading them; true reads them all.
    fn may_reach_out(&mut self, _chunk: Self::Chunk, _section_y: i32) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[derive(Debug)]
pub enum CollisionScanError<E> {
    InvalidBounds,
    TooLarge,
    VisitBudget,
    World(E),
    Shape(ShapeError),
}
impl<E: std::fmt::Display> std::fmt::Display for CollisionScanError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBounds => f.write_str("collision scan bounds must be finite"),
            Self::TooLarge => {
                f.write_str("collision scan exceeds the positive Java-int cursor domain")
            }
            Self::VisitBudget => f.write_str("collision scan candidate budget exhausted"),
            Self::World(error) => error.fmt(f),
            Self::Shape(error) => error.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CollisionScanError<E> {}

#[derive(Debug)]
pub struct BlockCollisionHit {
    pub position: BlockPos,
    pub shape: VoxelShape,
}

#[derive(Clone, Copy, Debug)]
enum CellError {
    InvalidBounds,
    TooLarge,
}

/// The cursor a scan of `bounds` walks: its padded start, the box's extents in
/// cells and the cell count. `new` and [`scan_cells`] share it, so a caller
/// that sizes a budget from a box counts exactly what the scan will visit.
fn cells(bounds: Aabb) -> Result<(BlockPos, [i32; 3], usize), CellError> {
    let min = bounds.min_position();
    let max = bounds.max_position();
    if !min.is_finite() || !max.is_finite() {
        return Err(CellError::InvalidBounds);
    }
    let start = BlockPos::containing(Vec3::new(min.x - 1.0e-7, min.y - 1.0e-7, min.z - 1.0e-7))
        .offset(-1, -1, -1);
    let end = BlockPos::containing(Vec3::new(max.x + 1.0e-7, max.y + 1.0e-7, max.z + 1.0e-7))
        .offset(1, 1, 1);
    let sizes = [
        end.x.wrapping_sub(start.x).wrapping_add(1),
        end.y.wrapping_sub(start.y).wrapping_add(1),
        end.z.wrapping_sub(start.z).wrapping_add(1),
    ];
    if sizes.iter().any(|size| *size <= 0) {
        return Err(CellError::TooLarge);
    }
    let count = sizes
        .into_iter()
        .try_fold(1u64, |n, size| {
            n.checked_mul(size as u64).filter(|n| *n <= i32::MAX as u64)
        })
        .ok_or(CellError::TooLarge)? as usize;
    Ok((start, sizes, count))
}

/// The cells a scan of `bounds` visits, or `None` for a box `new` refuses
/// (`InvalidBounds`, `TooLarge`): the most a budget can usefully be.
pub fn scan_cells(bounds: Aabb) -> Option<usize> {
    cells(bounds).ok().map(|(_, _, count)| count)
}

pub struct BlockCollisionScan<'w, 'c, W: BlockCollisionWorld> {
    world: &'w mut W,
    context: CollisionContext<'c>,
    bounds: Aabb,
    /// `Shapes.create(box)`, built the first time a block's shape is not
    /// the unit cube (finite bounds always make one).
    entity_shape: Option<VoxelShape>,
    only_suffocating: bool,
    start: BlockPos,
    sizes: [usize; 3],
    index: usize,
    /// The cell at `index`, relative to `start` (x fastest, then y, then z).
    cursor: [usize; 3],
    count: usize,
    budget: usize,
    chunk: Option<(ChunkPos, W::Chunk)>,
    /// `may_reach_out` of the last section an outer cell was in.
    reach: Option<(ChunkPos, i32, bool)>,
    failed: bool,
}
impl<'w, 'c, W: BlockCollisionWorld> BlockCollisionScan<'w, 'c, W> {
    /// Finite bounds and a positive-Java-int candidate volume are supported.
    /// Budget counts every cursor cell, including skipped corners. After an error
    /// iteration terminates; already performed queries are not rolled back.
    pub fn new(
        world: &'w mut W,
        context: CollisionContext<'c>,
        bounds: Aabb,
        only_suffocating: bool,
        candidate_budget: usize,
    ) -> Result<Self, CollisionScanError<W::Error>> {
        let (start, sizes, count) = cells(bounds).map_err(|error| match error {
            CellError::InvalidBounds => CollisionScanError::InvalidBounds,
            CellError::TooLarge => CollisionScanError::TooLarge,
        })?;
        Ok(Self {
            world,
            context,
            bounds,
            entity_shape: None,
            only_suffocating,
            start,
            sizes: sizes.map(|s| s as usize),
            index: 0,
            cursor: [0; 3],
            count,
            budget: candidate_budget,
            chunk: None,
            reach: None,
            failed: false,
        })
    }

    /// Move the cursor `n` cells further along its row; `n` must not pass
    /// the row's end.
    fn skip(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let [width, height, _] = self.sizes;
        self.index += n;
        self.cursor[0] += n;
        if self.cursor[0] == width {
            self.cursor[0] = 0;
            self.cursor[1] += 1;
            if self.cursor[1] == height {
                self.cursor[1] = 0;
                self.cursor[2] += 1;
            }
        }
    }

    fn find_next(&mut self) -> Result<Option<BlockCollisionHit>, CollisionScanError<W::Error>> {
        while self.index < self.count {
            if self.index >= self.budget {
                return Err(CollisionScanError::VisitBudget);
            }
            let [width, height, depth] = self.sizes;
            let relative = self.cursor;
            self.index += 1;
            self.cursor[0] += 1;
            if self.cursor[0] == width {
                self.cursor[0] = 0;
                self.cursor[1] += 1;
                if self.cursor[1] == height {
                    self.cursor[1] = 0;
                    self.cursor[2] += 1;
                }
            }
            let kind = relative
                .into_iter()
                .zip(self.sizes)
                .filter(|(p, size)| *p == 0 || *p == size - 1)
                .count();
            if kind == 3 {
                continue;
            }
            let p = self
                .start
                .offset(relative[0] as i32, relative[1] as i32, relative[2] as i32);
            let chunk_position = p.chunk();
            let chunk = match self.chunk {
                Some((at, chunk)) if at == chunk_position => Some(chunk),
                _ => {
                    let chunk = self
                        .world
                        .collision_chunk(chunk_position)
                        .map_err(CollisionScanError::World)?;
                    self.chunk = chunk.map(|chunk| (chunk_position, chunk));
                    chunk
                }
            };
            let Some(chunk) = chunk else {
                continue;
            };
            // An outer cell only counts for a block reaching out of its own
            // cell: most sections hold none, so their outer cells go unread
            // (the result is the same, with a fifth of the block reads for a
            // mob's box).
            if kind != 0 {
                let section_y = p.y >> 4;
                let reaches = match self.reach {
                    Some((at, y, reaches)) if at == chunk_position && y == section_y => reaches,
                    _ => {
                        let reaches = self
                            .world
                            .may_reach_out(chunk, section_y)
                            .map_err(CollisionScanError::World)?;
                        self.reach = Some((chunk_position, section_y, reaches));
                        reaches
                    }
                };
                if !reaches {
                    // On a row along the box's bottom, top or sides every
                    // cell is outer: the rest of it in this chunk is passed
                    // at once (never beyond the budget, which still ends the
                    // scan where it would have).
                    if relative[1] == 0
                        || relative[1] == height - 1
                        || relative[2] == 0
                        || relative[2] == depth - 1
                    {
                        let rest = (width - 1 - relative[0])
                            .min(15 - (p.x & 15) as usize)
                            .min(self.budget.saturating_sub(self.index));
                        self.skip(rest);
                    }
                    continue;
                }
            }
            let block = self
                .world
                .collision_block(chunk, p)
                .map_err(CollisionScanError::World)?;
            if self.only_suffocating
                && !self
                    .world
                    .is_suffocating(chunk, block, p)
                    .map_err(CollisionScanError::World)?
            {
                continue;
            }
            if kind == 1 && !block.large_collision_shape || kind == 2 && !block.moving_piston {
                continue;
            }
            if block.empty_shape {
                continue;
            }
            let shape = self
                .world
                .context_shape(block, p, &self.context)
                .map_err(CollisionScanError::World)?;
            let offset = Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
            let moved = if shape.is_block() {
                // Native adds one in floating point, including at i32::MAX.
                let top = Vec3::new(offset.x + 1.0, offset.y + 1.0, offset.z + 1.0);
                if !self.bounds.intersects_corners(offset, top) {
                    continue;
                }
                shape.moved(offset)
            } else {
                // Shapes whose bounds miss the box share no volume with it:
                // joinIsNotEmpty(AND) is false without moving them.
                if !shape
                    .bounds()
                    .is_some_and(|b| b.move_by(offset).intersects(self.bounds))
                {
                    continue;
                }
                let moved = shape.moved(offset);
                let entity_shape = match &self.entity_shape {
                    Some(shape) => shape,
                    None => self.entity_shape.insert(
                        VoxelShape::from_box(self.bounds).map_err(CollisionScanError::Shape)?,
                    ),
                };
                if moved.is_empty()
                    || !moved
                        .join_is_not_empty(entity_shape, ShapeOp::AND)
                        .map_err(CollisionScanError::Shape)?
                {
                    continue;
                }
                moved
            };
            return Ok(Some(BlockCollisionHit {
                position: p,
                shape: moved,
            }));
        }
        Ok(None)
    }
}
impl<W: BlockCollisionWorld> Iterator for BlockCollisionScan<'_, '_, W> {
    type Item = Result<BlockCollisionHit, CollisionScanError<W::Error>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.find_next() {
            Ok(hit) => hit.map(Ok),
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}
impl<W: BlockCollisionWorld> std::iter::FusedIterator for BlockCollisionScan<'_, '_, W> {}
