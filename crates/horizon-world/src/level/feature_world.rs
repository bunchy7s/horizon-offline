//! The live level as a feature-placement world (`ServerLevel` passed to
//! `Feature.place`): reads and writes go through the behaviour library, so
//! features placed by saplings, fungi and bone meal get vanilla's
//! `Level.setBlock` update semantics for the flags they request.
use super::{LevelAccess, Sim, data::state_flags, update::rel};
use crate::{
    fluid::{FluidKind, FluidState},
    heightmap::HeightmapType,
    registry::BlockStateId,
    worldgen::{
        anchor::WorldGenerationContext,
        biome::BiomeId,
        feature::FeatureWorld,
        live::LiveFeatures,
        placement::{LayerBlock, PlacementContext, PlacementQueryError},
    },
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};

pub(crate) struct LevelFeatureWorld<'s, 'a, L: LevelAccess + ?Sized> {
    pub sim: &'s mut Sim<'a, L>,
    pub features: &'s LiveFeatures,
    /// The entities a structure template placed (`add_entity`), for the caller
    /// to spawn.
    pub entities: Vec<crate::nbt::Compound>,
}

fn pos([x, y, z]: [i32; 3]) -> BlockPos {
    BlockPos::new(x, y, z)
}

impl<L: LevelAccess + ?Sized> LevelFeatureWorld<'_, '_, L> {
    fn sea(&self) -> i32 {
        match self.sim.level.dimension() {
            super::Dimension::Nether => 32,
            super::Dimension::End => 0,
            super::Dimension::Overworld => 63,
        }
    }
    fn in_tag(&self, state: BlockStateId, tag: &str) -> bool {
        let name = tag.trim_start_matches('#');
        self.sim
            .b
            .tag(name)
            .is_some_and(|set| set.contains(self.sim.b.block(state)))
    }
}

impl<L: LevelAccess + ?Sized> FeatureWorld for LevelFeatureWorld<'_, '_, L> {
    type Error = PlacementQueryError;
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        Ok(self.sim.state(pos(position)))
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, Self::Error> {
        Ok(self.sim.set_block(pos(position), state, flags))
    }
}

impl<L: LevelAccess + ?Sized> PlacementContext for LevelFeatureWorld<'_, '_, L> {
    /// `BlockEntity.loadWithComponents` on the block a template just placed.
    fn set_block_entity(
        &mut self,
        position: [i32; 3],
        nbt: crate::nbt::Compound,
    ) -> Result<(), PlacementQueryError> {
        self.sim.level.load_block_entity(pos(position), nbt);
        Ok(())
    }
    fn add_entity(&mut self, nbt: crate::nbt::Compound) -> Result<(), PlacementQueryError> {
        self.entities.push(nbt);
        Ok(())
    }
    /// `StructureTemplate.updateShapeAtEdge` for one face.
    fn update_shape_at_face(
        &mut self,
        position: [i32; 3],
        direction: Direction,
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let place = pos(position);
        let neighbour_pos = rel(place, direction);
        let state = self.sim.state(place);
        let neighbour = self.sim.state(neighbour_pos);
        let new_state = self
            .sim
            .update_shape(state, place, direction, neighbour_pos, neighbour);
        if new_state != state {
            self.sim.set_block(place, new_state, flags & !1);
        }
        let new_neighbour = self.sim.update_shape(
            neighbour,
            neighbour_pos,
            direction.opposite(),
            place,
            new_state,
        );
        if new_neighbour != neighbour {
            self.sim.set_block(neighbour_pos, new_neighbour, flags & !1);
        }
        Ok(())
    }
    /// `StructureTemplate.placeInWorld`'s pass over a placed block:
    /// `Block.updateFromNeighbourShapes`, written with `flags` when it
    /// changes, then `ServerLevel.updateNeighborsAt` for the new block.
    fn update_from_neighbour_shapes(
        &mut self,
        position: [i32; 3],
        flags: i32,
    ) -> Result<(), PlacementQueryError> {
        let place = pos(position);
        let state = self.sim.state(place);
        let new_state = self.sim.update_from_neighbour_shapes(state, place);
        if new_state != state {
            self.sim.set_block(place, new_state, flags);
        }
        let block = self.sim.b.block(new_state);
        self.sim.update_neighbors_at(place, block);
        Ok(())
    }
    fn set_section_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.sim.level.set_raw(pos(position), state);
        Ok(())
    }
    /// LevelChunk.setBlockState without neighbour or shape updates.
    fn set_chunk_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.sim.set_block(pos(position), state, 2 | 16);
        Ok(())
    }
    fn collision_full_block(
        &mut self,
        state: BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.b.full_collision(state))
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        Ok(self.sim.level.world_seed())
    }
    fn state_collision_empty(
        &mut self,
        state: BlockStateId,
        _position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.b.has_flag(state, state_flags::COLLISION_EMPTY))
    }
    fn can_occlude(&mut self, state: BlockStateId) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.b.has_flag(state, state_flags::CAN_OCCLUDE))
    }
    /// BeehiveDecorator's bees go into the nest's block entity (the tree
    /// checked the nest is there).
    fn store_beehive_occupants(
        &mut self,
        position: [i32; 3],
        ticks_in_hive: &[i32],
    ) -> Result<(), PlacementQueryError> {
        let nbt = crate::worldgen::tree::beehive_occupants(ticks_in_hive);
        self.sim.level.load_block_entity(pos(position), nbt);
        Ok(())
    }
    /// Live levels hold no loot-table containers placed by these features.
    fn set_block_entity_loot_table(
        &mut self,
        _position: [i32; 3],
        _table: &str,
        _random: &mut impl RandomSource,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    fn block_brightness(&mut self, position: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Ok(self.sim.level.block_light(pos(position)))
    }
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        Ok(self.sim.fluid_at(pos(position)))
    }
    fn destroy_block_without_entity(
        &mut self,
        position: [i32; 3],
        drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        Ok(self
            .sim
            .destroy_block(pos(position), drop_resources, super::flags::UPDATE_LIMIT))
    }
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea())
    }
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.sim.schedule_fluid_tick(pos(position), fluid, delay);
        Ok(())
    }
    /// Live chunks are already post-processed.
    fn mark_postprocessing(&mut self, _position: [i32; 3]) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        let kind = self.sim.fluid_at(pos(position)).kind();
        Ok(self.sim.b.is_water_fluid(kind))
    }
    fn attachment_at(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let neighbour = self.sim.state(rel(pos(position), direction));
        Ok(self.sim.b.multiface_can_attach(neighbour, direction))
    }
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.random().next_boolean())
    }
    fn next_world_float(&mut self) -> Result<f32, PlacementQueryError> {
        Ok(self.sim.random().next_float())
    }
    fn next_world_int(&mut self, bound: i32) -> Result<i32, PlacementQueryError> {
        self.sim
            .random()
            .next_int_bound(bound)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))
    }
    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        let block = self.sim.b.block(block);
        self.sim.schedule_tick(pos(position), block, delay);
        Ok(())
    }
    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.can_survive(state, pos(position)))
    }
    fn state_face_sturdy(
        &mut self,
        state: BlockStateId,
        _position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        Ok(self.sim.b.is_face_sturdy(state, direction))
    }
    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.sim.state(pos(position));
        Ok(self.sim.b.is_face_sturdy(state, direction))
    }
    fn center_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.sim.state(pos(position));
        Ok(self.sim.b.is_face_sturdy_center(state, direction))
    }
    fn unobstructed_block(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        let stone = self.sim.b.default_of(self.sim.b.named.stone);
        Ok(self.sim.level.is_unobstructed(pos(position), stone))
    }
    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        self.sim
            .level
            .biome_name(pos(position))
            .and_then(|name| self.features.biome_id(name))
            .ok_or(PlacementQueryError::Unavailable)
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self.sim.state(pos(position)))
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.sim.state(pos(position));
        Ok(self.in_tag(state, tag))
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sim.level.max_y() - 1)
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        let state = self.sim.state(pos(position));
        let b = self.sim.b;
        let block = b.block(state);
        Ok(
            if b.is_air(state) || block == b.named.water || block == b.named.lava {
                LayerBlock::Empty
            } else if b.table.name(state) == "minecraft:bedrock" {
                LayerBlock::Bedrock
            } else {
                LayerBlock::Ground
            },
        )
    }
    fn generation(&self) -> WorldGenerationContext {
        let min_y = self.sim.level.min_y();
        WorldGenerationContext::new(min_y, self.sim.level.max_y() - min_y, self.sea())
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sim.level.min_y())
    }
    /// `Level.getHeight(type, x, z)`: one above the highest matching block.
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        let (min, max) = (self.sim.level.min_y(), self.sim.level.max_y());
        for y in (min..max).rev() {
            let state = self.sim.state(BlockPos::new(x, y, z));
            let fluid = self.sim.b.fluid(state).kind() != FluidKind::Empty;
            let matches = match kind {
                HeightmapType::WorldSurface | HeightmapType::WorldSurfaceWg => {
                    !self.sim.b.is_air(state)
                }
                HeightmapType::OceanFloor | HeightmapType::OceanFloorWg => {
                    self.in_tag(state, "minecraft:blocks_motion_in_heightmap")
                }
                HeightmapType::MotionBlocking => {
                    fluid || self.in_tag(state, "minecraft:blocks_motion_in_heightmap")
                }
                HeightmapType::MotionBlockingNoLeaves => {
                    fluid || self.in_tag(state, "minecraft:blocks_motion_in_heightmap_no_leaves")
                }
            };
            if matches {
                return Ok(y + 1);
            }
        }
        Ok(min)
    }
}
