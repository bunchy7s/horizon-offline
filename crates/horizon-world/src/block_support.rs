//! Native support tests over an already-selected block support shape. Selecting
//! that shape from block/world/entity behavior is the caller's responsibility.
use horizon_core::geometry::{Aabb, Direction, ShapeError, ShapeOp, Vec3, VoxelShape};
use std::sync::OnceLock;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupportType {
    Full,
    Center,
    Rigid,
}
impl SupportType {
    pub fn is_supporting(
        self,
        shape: &VoxelShape,
        direction: Direction,
    ) -> Result<bool, ShapeError> {
        self.supported_by(&shape.face_shape(direction)?)
    }
    /// [`Self::is_supporting`] given the shape's face in that direction.
    fn supported_by(self, face: &VoxelShape) -> Result<bool, ShapeError> {
        let (required, operation) = match self {
            Self::Full => (VoxelShape::block(), ShapeOp::XOR),
            Self::Center => {
                static CENTER: OnceLock<VoxelShape> = OnceLock::new();
                (
                    CENTER
                        .get_or_init(|| {
                            VoxelShape::from_box(Aabb::new(
                                Vec3::new(0.4375, 0.0, 0.4375),
                                Vec3::new(0.5625, 0.625, 0.5625),
                            ))
                            .expect("valid support shape")
                        })
                        .clone(),
                    ShapeOp::ONLY_SECOND,
                )
            }
            Self::Rigid => {
                static RIGID: OnceLock<VoxelShape> = OnceLock::new();
                (
                    RIGID
                        .get_or_init(|| {
                            VoxelShape::block()
                                .join(
                                    &VoxelShape::from_box(Aabb::new(
                                        Vec3::new(0.125, 0.0, 0.125),
                                        Vec3::new(0.875, 1.0, 0.875),
                                    ))
                                    .expect("valid support inset"),
                                    ShapeOp::ONLY_FIRST,
                                )
                                .expect("valid support ring")
                        })
                        .clone(),
                    ShapeOp::ONLY_SECOND,
                )
            }
        };
        Ok(!face.join_is_not_empty(&required, operation)?)
    }
}

use crate::{
    collision::CachedCollisionRegistry,
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable},
};
use std::{collections::HashMap, io};
/// Cached branch of BlockState.isFaceSturdy. Dynamic support-shape selection
/// stays explicit as None, even when a context-free approximation looks obvious.
pub struct CachedBlockSupportRegistry<'r> {
    shapes: CachedCollisionRegistry<'r>,
    states: Vec<Option<[u8; 3]>>,
}
impl<'r> CachedBlockSupportRegistry<'r> {
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "support")?)
    }
    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        if table.get("Kind") != Some(&Tag::String("block_support".into())) {
            return Err(invalid("wrong cached support data kind"));
        }
        let registry = table.registry();
        let shapes = CachedCollisionRegistry::from_table(table)?;
        let mut cache = HashMap::new();
        let mut states = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let Some(shape) = shapes.cached_shape(BlockStateId(id as u32))? else {
                states.push(None);
                continue;
            };
            let identity = std::ptr::from_ref(shape);
            let flags = if let Some(flags) = cache.get(&identity) {
                *flags
            } else {
                let mut flags = [0; 3];
                for direction in Direction::ALL {
                    // Not kept on the shape: only these flags are.
                    let face = shape
                        .face_shape_transient(direction)
                        .map_err(|e| invalid(e.to_string()))?;
                    for (i, support) in [SupportType::Full, SupportType::Center, SupportType::Rigid]
                        .into_iter()
                        .enumerate()
                    {
                        if support
                            .supported_by(&face)
                            .map_err(|e| invalid(e.to_string()))?
                        {
                            flags[i] |= 1 << direction as u8;
                        }
                    }
                }
                cache.insert(identity, flags);
                flags
            };
            states.push(Some(flags));
        }
        Ok(Self { shapes, states })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.shapes.registry()
    }
    pub fn cached_state_count(&self) -> usize {
        self.shapes.cached_state_count()
    }
    pub fn cached_shape(&self, state: BlockStateId) -> io::Result<Option<&VoxelShape>> {
        self.shapes.cached_shape(state)
    }
    pub fn is_face_sturdy(
        &self,
        state: BlockStateId,
        direction: Direction,
        support: SupportType,
    ) -> io::Result<Option<bool>> {
        let flags = self
            .states
            .get(state.0 as usize)
            .ok_or_else(|| invalid("support state outside registry"))?;
        Ok(flags.map(|flags| {
            flags[match support {
                SupportType::Full => 0,
                SupportType::Center => 1,
                SupportType::Rigid => 2,
            }] & (1 << direction as u8)
                != 0
        }))
    }
}

use crate::dynamic_support::{DynamicSupport, DynamicSupportError, DynamicSupportWorld};
/// Unified cached/dynamic support dispatch. Block-entity queries remain owned by
/// the world, including the moving-piston entity's collision calculation.
pub struct BlockSupportRegistry<'r> {
    cached: CachedBlockSupportRegistry<'r>,
    dynamic: HashMap<BlockStateId, DynamicSupport>,
}
#[derive(Debug)]
pub enum SupportQueryError<E> {
    Data(io::Error),
    Dynamic(DynamicSupportError<E>),
    Unsupported(BlockStateId),
}
impl<E: std::fmt::Display> std::fmt::Display for SupportQueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(e) => e.fmt(f),
            Self::Dynamic(e) => e.fmt(f),
            Self::Unsupported(state) => write!(f, "unsupported block support state {}", state.0),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SupportQueryError<E> {}
impl<'r> BlockSupportRegistry<'r> {
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "support")?)
    }
    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        let registry = table.registry();
        let cached = CachedBlockSupportRegistry::from_table(table)?;
        let mut dynamic = HashMap::new();
        for id in 0..registry.len() {
            let id = BlockStateId(id as u32);
            if cached.cached_shape(id)?.is_none() {
                if let Some(behavior) = DynamicSupport::from_state(registry.state(id).unwrap())? {
                    dynamic.insert(id, behavior);
                }
            }
        }
        Ok(Self { cached, dynamic })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.cached.registry()
    }
    pub fn is_face_sturdy<W: DynamicSupportWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
        direction: Direction,
        support: SupportType,
    ) -> Result<bool, SupportQueryError<W::Error>> {
        if let Some(result) = self
            .cached
            .is_face_sturdy(state, direction, support)
            .map_err(SupportQueryError::Data)?
        {
            return Ok(result);
        }
        self.dynamic
            .get(&state)
            .ok_or(SupportQueryError::Unsupported(state))?
            .is_face_sturdy(world, position, direction, support)
            .map_err(SupportQueryError::Dynamic)
    }
}
