//! Query interface for placement against a generated world.
use crate::worldgen::feature_sort::FeatureIdentity;
use crate::{heightmap::HeightmapType, worldgen::anchor::WorldGenerationContext};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementQueryError {
    Unavailable,
    WorkLimit,
    Failed(String),
}
impl std::fmt::Display for PlacementQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkLimit => f.write_str("predicate work limit exceeded"),
            Self::Unavailable => f.write_str("placement world query is unavailable"),
            Self::Failed(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for PlacementQueryError {}
/// Classification for layered placement: air states and exact WATER/LAVA blocks
/// are empty; exact BEDROCK is excluded from ground; every other block is ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerBlock {
    Empty,
    Bedrock,
    Ground,
}
pub trait PlacementContext {
    /// StructureTemplate.updateShapeAtEdge for one outer face of a placed
    /// voxel shape: update the state at `position` against its neighbor in
    /// `direction`, then the neighbor against the (possibly new) state, writing
    /// changes with `flags & !1`. Worlds without block shape-update behavior
    /// accept this as a no-op.
    fn update_shape_at_face(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
        _flags: i32,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// Block.updateFromNeighbourShapes for the state at `position`, written
    /// with `flags` when it changes, then LevelAccessor.updateNeighborsAt for
    /// the new block (a no-op outside a live level). Worlds without block
    /// shape-update behavior accept this as a no-op.
    fn update_from_neighbour_shapes(
        &mut self,
        _position: [i32; 3],
        _flags: i32,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// WorldGenRegion.ensureCanWrite: horizontal write radius and upgrade height.
    fn ensure_can_write(&mut self, _position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(true)
    }
    /// BulkSectionAccess read: None outside the world's section range.
    fn section_block(
        &mut self,
        position: [i32; 3],
    ) -> Result<Option<crate::registry::BlockStateId>, PlacementQueryError> {
        if self.outside_build_height(position[1])? {
            return Ok(None);
        }
        self.block_state(position).map(Some)
    }
    /// Raw LevelChunkSection write: no heightmap, light, POI or neighbor effects.
    fn set_section_block(
        &mut self,
        _position: [i32; 3],
        _state: crate::registry::BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// ChunkAccess.setBlockState(pos, state, false) in generation (a
    /// ProtoChunk write with its heightmaps, no neighbour or block-entity
    /// effects), as mods' structure processors write past their template.
    /// Worlds without generation chunks write the section.
    fn set_chunk_block(
        &mut self,
        position: [i32; 3],
        state: crate::registry::BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.set_section_block(position, state)
    }
    /// BlockState.isCollisionShapeFullBlock for an already-read state.
    fn collision_full_block(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Add an End crystal entity (structure spawn) at an entity position.
    fn spawn_end_crystal(
        &mut self,
        _position: [f64; 3],
        _yaw: f32,
        _beam_target: Option<[i32; 3]>,
        _invulnerable: bool,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// WorldGenLevel.getSeed: the world seed for seed-derived feature noise.
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Set the entity type of the spawner block entity at `position`, when one
    /// exists. The caller has already drawn the native entity choice.
    fn configure_spawner(
        &mut self,
        _position: [i32; 3],
        _entity: &str,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// Store worldgen beehive occupants (ticks in hive per bee) in the nest
    /// block entity at `position`. Worlds without block-entity storage drop them.
    fn store_beehive_occupants(
        &mut self,
        _position: [i32; 3],
        _ticks_in_hive: &[i32],
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// Test the already-read state's collision shape in an empty entity context.
    fn state_collision_empty(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Look up a randomizable container, then consume a loot seed only when one
    /// exists. Missing or unrelated block entities are successful no-ops.
    fn set_block_entity_loot_table(
        &mut self,
        _position: [i32; 3],
        _table: &str,
        _random: &mut impl horizon_core::random::RandomSource,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Look up the block entity at this position and set its exit only if it is
    /// an End gateway. Missing or other entities are successful no-ops. The
    /// owning world applies the gateway's dirty notification after the update.
    fn configure_end_gateway(
        &mut self,
        _position: [i32; 3],
        _exit: [i32; 3],
        _exact: bool,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Use the previously resolved biome, retaining its temperature cache.
    fn biome_should_freeze(
        &mut self,
        _biome: crate::worldgen::biome::BiomeId,
        _position: [i32; 3],
        _require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_should_snow(
        &mut self,
        _biome: crate::worldgen::biome::BiomeId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Block light only, independently of sky light and sky darkening.
    fn block_brightness(&mut self, _position: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn fluid_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::fluid::FluidState, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Request destruction with no responsible entity. The world adapter owns
    /// replacement, drops, block entities and updates; false is a rejected request.
    fn destroy_block_without_entity(
        &mut self,
        _position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// World sea level, independently of the generator's placement context.
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn schedule_fluid_tick(
        &mut self,
        _position: [i32; 3],
        _fluid: crate::fluid::FluidKind,
        _delay: i32,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Resolve the containing chunk and mark this position for postprocessing.
    fn mark_postprocessing(&mut self, _position: [i32; 3]) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Test the current fluid at this position against the world's water tag.
    fn water_at(&mut self, _position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// `BlockState.canOcclude()` of a state.
    fn can_occlude(
        &mut self,
        _state: crate::registry::BlockStateId,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Multiface attachment: neighbor support, then empty-context collision.
    fn attachment_at(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Draw from the world's random stream, independently of the feature RNG.
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn next_world_float(&mut self) -> Result<f32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn next_world_int(&mut self, _bound: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Schedule the block type identified by its registry default state.
    fn schedule_block_tick(
        &mut self,
        _position: [i32; 3],
        _block: crate::registry::BlockStateId,
        _delay: i32,
    ) -> Result<(), PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }

    /// Evaluate this candidate state's survival without placing it.
    fn block_survives(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Evaluate FULL support using an already-read state, without reading it again.
    fn state_face_sturdy(
        &mut self,
        _state: crate::registry::BlockStateId,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Read the block state and evaluate FULL support on the requested face.
    fn face_sturdy(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Block.canSupportCenter: CENTER support on the requested face.
    fn center_sturdy(
        &mut self,
        _position: [i32; 3],
        _direction: horizon_core::geometry::Direction,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Test entity obstruction of a unit-block shape with no excluded entity.
    fn unobstructed_block(&mut self, _position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn biome_identity(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::worldgen::biome::BiomeId, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn top_feature(&self) -> Option<FeatureIdentity> {
        None
    }
    /// Resolve the biome at this position and test its generation settings for
    /// the supplied placed feature. Registry equality/ownership belongs here.
    fn biome_allows_feature(
        &mut self,
        _position: [i32; 3],
        _feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn block_state(
        &mut self,
        _position: [i32; 3],
    ) -> Result<crate::registry::BlockStateId, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Read the state at the requested position and test its block-tag identity.
    fn block_matches_tag(
        &mut self,
        _position: [i32; 3],
        _tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn inside_build_height(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(position[1] >= self.min_y()? && position[1] <= self.max_y()?)
    }
    fn outside_build_height(&mut self, y: i32) -> Result<bool, PlacementQueryError> {
        Ok(y < self.min_y()? || y > self.max_y()?)
    }
    fn layer_block(&mut self, _position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    /// Add an entity from its NBT (structure templates and pieces; `Pos` is
    /// absolute). Worlds without entity storage drop it.
    fn add_entity(&mut self, _nbt: crate::nbt::Compound) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    /// Store block-entity data loaded at a just-placed block (structure
    /// templates). Worlds without block-entity storage drop it.
    fn set_block_entity(
        &mut self,
        _position: [i32; 3],
        _nbt: crate::nbt::Compound,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    fn generation(&self) -> WorldGenerationContext;
    /// Recreate the generation bounds for a block-predicate invocation. World
    /// adapters may perform fallible level/generator queries here; ordinary
    /// placement retains its already-created immutable generation context.
    fn predicate_generation(&mut self) -> Result<WorldGenerationContext, PlacementQueryError> {
        Ok(self.generation())
    }
    /// World minimum, which can differ from the generator's minimum.
    fn min_y(&mut self) -> Result<i32, PlacementQueryError>;
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError>;
    /// Whether any column of the square `x..=x + size`, `z..=z + size` has
    /// `height(kind)` at least `y`, read faster than column by column; `None`
    /// when the world can't (callers then ask `height` for each column).
    fn any_height_at_least(
        &mut self,
        _kind: HeightmapType,
        _corner: [i32; 2],
        _size: i32,
        _y: i32,
    ) -> Result<Option<bool>, PlacementQueryError> {
        Ok(None)
    }
}
impl PlacementContext for WorldGenerationContext {
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y.wrapping_add(self.height).wrapping_sub(1))
    }
    fn generation(&self) -> WorldGenerationContext {
        *self
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.min_y)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}
