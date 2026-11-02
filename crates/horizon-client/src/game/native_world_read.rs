//! Read-only block and tag queries for native server hooks.
use super::{Dim, Game};
use horizon_client::mod_api::NativeLifecycleContext;
use horizon_client::{
    items::{ItemStack, Items},
    world::World,
};
use horizon_world::{chunk::BlockState, registry::BlockRegistry};

/// Why a server block or tag cannot be read.
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NativeBlockReadError {
    /// The context is stale or the call is outside its server tick hook.
    InactiveContext,
    /// Tag queries require an authoritative server.
    NotServer,
    /// The world has not loaded its tag tables yet.
    TagsUnavailable,
    /// The dimension is unknown or is not currently open on the server.
    UnavailableDimension,
    OutOfBounds,
    Unloaded,
    InvalidState,
}

impl Game<'_> {
    /// The world lock is installed after its tag tables are built.
    fn native_tag_items(&self) -> Result<&Items, NativeBlockReadError> {
        if self.options.execution_role.is_client() {
            return Err(NativeBlockReadError::NotServer);
        }
        if self.session_lock.is_none() {
            return Err(NativeBlockReadError::TagsUnavailable);
        }
        Ok(self.items())
    }

    /// Test a block state's name in an active server tick hook. API 28.
    #[allow(dead_code)]
    pub(super) fn native_block_in_tag(
        &self,
        context: &NativeLifecycleContext,
        state: &BlockState,
        tag: &str,
    ) -> Result<bool, NativeBlockReadError> {
        if !self.native_lifecycle.permits_read(context) {
            return Err(NativeBlockReadError::InactiveContext);
        }
        self.native_server_block_in_tag(state, tag)
    }

    /// Test an item stack in an active server tick hook. API 28.
    #[allow(dead_code)]
    pub(super) fn native_item_in_tag(
        &self,
        context: &NativeLifecycleContext,
        stack: &ItemStack,
        tag: &str,
    ) -> Result<bool, NativeBlockReadError> {
        if !self.native_lifecycle.permits_read(context) {
            return Err(NativeBlockReadError::InactiveContext);
        }
        self.native_server_item_in_tag(stack, tag)
    }

    /// Test a block state's name against the server's merged block tags.
    #[allow(dead_code)]
    pub(super) fn native_server_block_in_tag(
        &self,
        state: &BlockState,
        tag: &str,
    ) -> Result<bool, NativeBlockReadError> {
        Ok(self.native_tag_items()?.block_in_tag(tag, &state.name))
    }

    /// Test an item stack against the server's merged item tags.
    #[allow(dead_code)]
    pub(super) fn native_server_item_in_tag(
        &self,
        stack: &ItemStack,
        tag: &str,
    ) -> Result<bool, NativeBlockReadError> {
        let items = self.native_tag_items()?;
        let item = if stack.count == 0 {
            "minecraft:air"
        } else {
            &stack.item
        };
        Ok(items.item_in_tag(tag, item))
    }

    /// Read a loaded block in a server tick hook, without loading a chunk.
    /// The returned name and properties are owned and remain valid after the hook.
    #[allow(dead_code)]
    pub(super) fn native_server_block(
        &self,
        context: &NativeLifecycleContext,
        dimension: &str,
        pos: [i32; 3],
    ) -> Result<BlockState, NativeBlockReadError> {
        if !self.native_lifecycle.permits_read(context) {
            return Err(NativeBlockReadError::InactiveContext);
        }
        let dim = match dimension {
            "minecraft:overworld" => Dim::Overworld,
            "minecraft:the_nether" => Dim::Nether,
            "minecraft:the_end" => Dim::End,
            _ => return Err(NativeBlockReadError::UnavailableDimension),
        };
        let world = if dim == self.dim {
            Some(&self.world)
        } else {
            self.levels.parked_world(dim)
        }
        .ok_or(NativeBlockReadError::UnavailableDimension)?;
        read_block(world, self.pack.assets.registry, pos)
    }
}

/// Resolve an existing world state, preserving unloaded and invalid states.
fn read_block(
    world: &World,
    registry: &BlockRegistry,
    [x, y, z]: [i32; 3],
) -> Result<BlockState, NativeBlockReadError> {
    if !(-30_000_000..30_000_000).contains(&x)
        || !(-30_000_000..30_000_000).contains(&z)
        || !(world.shape.min_y..world.shape.max_y()).contains(&y)
    {
        return Err(NativeBlockReadError::OutOfBounds);
    }
    if !world.loaded(x, z) {
        return Err(NativeBlockReadError::Unloaded);
    }
    registry
        .state(world.block(x, y, z))
        .cloned()
        .ok_or(NativeBlockReadError::InvalidState)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::chunk::Chunk;
    use horizon_world::registry::BlockStateId;
    use std::sync::Arc;

    #[test]
    fn reads_only_loaded_in_bounds_states() {
        let facts = br#"{"defaults":{"minecraft:air":0,"test:pillar":1},"states":[
            ["air",true,false,"","minecraft:air"],
            ["pillar",false,false,"",{"id":"test:pillar","properties":{"axis":"x"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(&facts[..]).unwrap();
        let mut world = World::new(BlockStateId(0), 0);
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        world.set(-1, 64, 1, BlockStateId(1)).unwrap();
        // The Nether's build range starts at y 0 (its level's shape).
        let mut nether = World::new(BlockStateId(0), 0);
        nether.shape = horizon_client::chunk::LevelShape::NETHER;
        nether.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        let block = read_block(&world, &registry, [-1, 64, 1]).unwrap();
        assert_eq!(block.name, "test:pillar");
        assert_eq!(block.properties["axis"], "x");
        assert_eq!(
            read_block(&world, &registry, [-1, 65, 1]).unwrap().name,
            "minecraft:air"
        );
        assert_eq!(
            read_block(&world, &registry, [0, 64, 1]),
            Err(NativeBlockReadError::Unloaded)
        );
        assert_eq!(
            read_block(&nether, &registry, [-1, -1, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        assert_eq!(
            read_block(&world, &registry, [30_000_000, 64, 1]),
            Err(NativeBlockReadError::OutOfBounds)
        );
        world.set(-1, 64, 1, BlockStateId(99)).unwrap();
        assert_eq!(
            read_block(&world, &registry, [-1, 64, 1]),
            Err(NativeBlockReadError::InvalidState)
        );
        assert_eq!(world.chunks.len(), 1, "reads must not load chunks");
    }
}
