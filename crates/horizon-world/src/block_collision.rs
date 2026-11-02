//! Cached/dynamic no-context collision queries and explicit contextual block rules.
//! Context-sensitive families override the no-context cache explicitly.
use crate::{
    block_collision_scan::CollisionBlock,
    collision::CachedCollisionRegistry,
    collision_context::CollisionContext,
    dynamic_support::{DynamicSupport, DynamicSupportError, DynamicSupportWorld},
    liquid_collision::{LiquidCollision, LiquidCollisionWorld},
    piston_shape::PistonBlockBindings,
    powder_snow_collision::{PowderSnowWorld, powder_snow_collision_shape},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::{Aabb, Axis, Direction, Vec3, VoxelShape};
use std::{collections::HashMap, io, sync::Arc};

#[derive(Clone, Copy, Debug)]
pub struct ShulkerCollisionState {
    pub previous_progress: f32,
    pub progress: f32,
}
impl ShulkerCollisionState {
    pub fn bounding_box(self, direction: Direction) -> Aabb {
        // Native getProgress(1) retains this interpolation, including rounding.
        let progress = 0.5f32 * (self.previous_progress + (self.progress - self.previous_progress));
        let maximum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).max(progress)
        } as f64;
        let minimum = if progress.is_nan() {
            progress
        } else {
            (-1.0f32).min(progress)
        } as f64;
        let [x, y, z] = direction.normal();
        Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5))
            .expand_towards(Vec3::new(
                f64::from(x) * maximum,
                f64::from(y) * maximum,
                f64::from(z) * maximum,
            ))
            .contract(Vec3::new(
                f64::from(-x) * (1.0 + minimum),
                f64::from(-y) * (1.0 + minimum),
                f64::from(-z) * (1.0 + minimum),
            ))
            .move_by(Vec3::new(0.5, 0.0, 0.5))
    }
}

pub trait DynamicCollisionWorld: DynamicSupportWorld {
    /// One block-entity lookup. None denotes absence or another entity type.
    fn shulker_collision_state(
        &mut self,
        position: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error>;
}

#[derive(Debug)]
pub enum CollisionQueryError<E> {
    Data(io::Error),
    Dynamic(DynamicSupportError<E>),
    Context(E),
    Unsupported(BlockStateId),
}
impl<E: std::fmt::Display> std::fmt::Display for CollisionQueryError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(error) => error.fmt(f),
            Self::Dynamic(error) => error.fmt(f),
            Self::Context(error) => error.fmt(f),
            Self::Unsupported(state) => write!(f, "unsupported block collision state {}", state.0),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for CollisionQueryError<E> {}

pub struct BlockCollisionRegistry<'r> {
    cached: Arc<CachedCollisionRegistry<'r>>,
    dynamic: HashMap<BlockStateId, DynamicSupport>,
    contextual: HashMap<BlockStateId, ContextualRule>,
    scan_flags: Vec<u8>,
    /// Piston and piston-head states for moving-piston collision; None in
    /// registries without pistons.
    pistons: Option<PistonBlockBindings<'r>>,
}
enum ContextualRule {
    Scaffolding(ScaffoldingCollision),
    Liquid(LiquidCollision),
    PowderSnow,
}
impl<'r> BlockCollisionRegistry<'r> {
    pub fn new(cached: Arc<CachedCollisionRegistry<'r>>) -> io::Result<Self> {
        let mut dynamic = HashMap::new();
        let mut contextual = HashMap::new();
        let mut scan_flags = Vec::with_capacity(cached.registry().len());
        for index in 0..cached.registry().len() {
            let id = BlockStateId(index as u32);
            let state = cached.registry().state(id).unwrap();
            let large = cached.cached_shape(id)?.is_none_or(|shape| {
                Axis::ALL
                    .into_iter()
                    .any(|axis| shape.min(axis) < 0.0 || shape.max(axis) > 1.0)
            });
            if let Some(rule) = ScaffoldingCollision::from_state(state)? {
                contextual.insert(id, ContextualRule::Scaffolding(rule));
            } else if let Some(rule) = LiquidCollision::from_state(state)? {
                contextual.insert(id, ContextualRule::Liquid(rule));
            } else if state.name == "minecraft:powder_snow" {
                contextual.insert(id, ContextualRule::PowderSnow);
            }
            // No contextual rule and an empty cached shape: empty everywhere.
            let empty = !contextual.contains_key(&id)
                && cached.cached_shape(id)?.is_some_and(VoxelShape::is_empty);
            scan_flags.push(
                u8::from(large)
                    | (u8::from(state.name == "minecraft:moving_piston") << 1)
                    | (u8::from(empty) << 2)
                    | (u8::from(contextual.contains_key(&id)) << 3),
            );
            if cached.cached_shape(id)?.is_none() {
                if let Some(rule) =
                    DynamicSupport::from_state(cached.registry().state(id).unwrap())?
                {
                    dynamic.insert(id, rule);
                }
            }
        }
        let pistons = PistonBlockBindings::new(cached.registry()).ok();
        Ok(Self {
            cached,
            dynamic,
            contextual,
            scan_flags,
            pistons,
        })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.cached.registry()
    }
    /// The piston states behind `PistonMovingBlockEntity.getCollisionShape`.
    pub fn pistons(&self) -> Option<&PistonBlockBindings<'r>> {
        self.pistons.as_ref()
    }
    pub fn cached_shapes(&self) -> &CachedCollisionRegistry<'r> {
        &self.cached
    }
    /// Immutable cursor-filter metadata, prepared once for this registry.
    pub fn collision_block(&self, state: BlockStateId) -> io::Result<CollisionBlock> {
        let flags = *self
            .scan_flags
            .get(state.0 as usize)
            .ok_or_else(|| crate::compression::invalid("collision scan state outside registry"))?;
        Ok(CollisionBlock {
            state,
            large_collision_shape: flags & 1 != 0,
            moving_piston: flags & 2 != 0,
            empty_shape: flags & 4 != 0,
        })
    }
    /// Whether `state` can have a collision shape: false for states whose shape is always
    /// empty, whatever the context. States outside the registry count.
    pub fn may_collide(&self, state: BlockStateId) -> bool {
        self.scan_flags
            .get(state.0 as usize)
            .is_none_or(|flags| flags & 4 == 0)
    }
    /// Whether a scan keeps `state` in the cells around its box: a large
    /// collision shape or a moving piston. States outside the registry count.
    pub fn reaches_out(&self, state: BlockStateId) -> bool {
        self.scan_flags
            .get(state.0 as usize)
            .is_none_or(|flags| flags & 3 != 0)
    }
    pub fn collision_shape<W: DynamicCollisionWorld>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<VoxelShape, CollisionQueryError<W::Error>> {
        if let Some(shape) = self
            .cached
            .cached_shape(state)
            .map_err(CollisionQueryError::Data)?
        {
            return Ok(shape.clone());
        }
        self.dynamic
            .get(&state)
            .ok_or(CollisionQueryError::Unsupported(state))?
            .collision_shape(world, position)
            .map_err(CollisionQueryError::Dynamic)
    }

    /// Ordinary entity/position/placement contexts. The remaining pinned block
    /// families share their no-context geometry; the native registry audit checks
    /// cached states with rejecting context/world providers before using that path.
    /// Experimental minecart position exclusions are a separate context layer.
    pub fn collision_shape_with_context<W, E>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: horizon_core::position::BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, CollisionQueryError<E>>
    where
        W: DynamicCollisionWorld<Error = E>
            + LiquidCollisionWorld<Error = E>
            + PowderSnowWorld<Error = E>,
    {
        // Most states have no contextual rule: their flag spares the lookup.
        let contextual = self
            .scan_flags
            .get(state.0 as usize)
            .is_none_or(|flags| flags & 8 != 0);
        match contextual.then(|| self.contextual.get(&state)).flatten() {
            Some(ContextualRule::Scaffolding(rule)) => Ok(rule.shape(context, position)),
            Some(ContextualRule::Liquid(rule)) => rule
                .shape(world, position, context)
                .map_err(CollisionQueryError::Context),
            Some(ContextualRule::PowderSnow) => {
                powder_snow_collision_shape(world, position, context)
                    .map_err(CollisionQueryError::Context)
            }
            None => self.collision_shape(world, state, [position.x, position.y, position.z]),
        }
    }
}

/// Entity-context scaffold selection, independent of world queries. Construction
/// validates the descriptor so missing properties cannot silently change behavior.
#[derive(Clone, Copy, Debug)]
pub struct ScaffoldingCollision {
    supported: bool,
    bottom: bool,
}
impl ScaffoldingCollision {
    pub fn from_state(state: &crate::chunk::BlockState) -> io::Result<Option<Self>> {
        if state.name != "minecraft:scaffolding" {
            return Ok(None);
        }
        let distance = state
            .properties
            .get("distance")
            .and_then(|v| crate::registry::java_integer(v))
            .filter(|v| (0..=7).contains(v))
            .ok_or_else(|| crate::compression::invalid("invalid scaffold distance"))?;
        let bottom = match state.properties.get("bottom").map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => return Err(crate::compression::invalid("invalid scaffold bottom")),
        };
        Ok(Some(Self {
            supported: distance == 0,
            bottom,
        }))
    }

    pub fn shape(
        &self,
        context: &crate::collision_context::CollisionContext<'_>,
        position: horizon_core::position::BlockPos,
    ) -> VoxelShape {
        use std::sync::OnceLock;
        if context.is_placement() {
            return VoxelShape::empty();
        }
        if context.is_above(&VoxelShape::block(), position, true) && !context.is_descending() {
            return crate::dynamic_support::scaffolding_stable_shape();
        }
        if !self.supported && self.bottom {
            static BELOW: OnceLock<VoxelShape> = OnceLock::new();
            let below = BELOW.get_or_init(|| {
                VoxelShape::block()
                    .moved(Vec3::new(0.0, -1.0, 0.0))
                    .optimized()
                    .expect("bounded scaffold below shape")
            });
            if context.is_above(below, position, true) {
                static BOTTOM: OnceLock<VoxelShape> = OnceLock::new();
                return BOTTOM
                    .get_or_init(|| {
                        VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.125, 1.0)))
                            .expect("bounded scaffold bottom")
                    })
                    .clone();
            }
        }
        VoxelShape::empty()
    }
}
