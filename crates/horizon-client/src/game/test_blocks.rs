//! Test blocks on the server: `handleSetTestBlock` and the mode a placed
//! test block starts with.
use super::{Game, lan::LanHost};
use horizon_client::{
    containers::BlockEntity,
    test_block::{Effects, TestBlock, TestBlockMode},
};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::play;

type Pos = (i32, i32, i32);

impl Game<'_> {
    fn test_block_mut(&mut self, pos: Pos) -> Option<&mut TestBlock> {
        match self.world.block_entities.get_mut(&pos)? {
            BlockEntity::TestBlock(b) => Some(b),
            _ => None,
        }
    }

    /// The `mode` of the block state at `pos`.
    pub(super) fn test_block_state_mode(&self, pos: Pos) -> Option<TestBlockMode> {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let mode = self
            .pack
            .assets
            .registry
            .state(state)?
            .properties
            .get("mode")?;
        TestBlockMode::of_serialized_name(mode)
    }

    /// `new TestBlockEntity(pos, state)`: the mode is the state's.
    pub(super) fn test_block_placed(&mut self, pos: Pos) {
        let Some(mode) = self.test_block_state_mode(pos) else {
            return;
        };
        if let Some(b) = self.test_block_mut(pos) {
            b.mode = mode;
        }
    }

    fn test_block_at(&self, pos: Pos) -> Option<&TestBlock> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::TestBlock(b) => Some(b),
            _ => None,
        }
    }

    /// `TestBlockEntity.hasTriggered`.
    pub(super) fn test_block_triggered(&self, pos: Pos) -> bool {
        self.test_block_at(pos).is_some_and(|b| b.triggered)
    }

    /// `TestBlockEntity.getMessage`.
    pub(super) fn test_block_message(&self, pos: Pos) -> String {
        self.test_block_at(pos)
            .map_or_else(String::new, |b| b.message.clone())
    }

    /// What a `TestBlockEntity` call asked of the level: its neighbours
    /// update (`level.updateNeighborsAt`) and the log line.
    fn apply_test_block_effects(&mut self, pos: Pos, effects: Effects) {
        if effects.update_neighbors {
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            let state = self.world.block(pos.0, pos.1, pos.2);
            self.with_level(|b, level, ticks| {
                let block = b.block(state);
                b.update_neighbors_at(level, ticks, at, block);
            });
        }
        if effects.log
            && let Some(b) = self.test_block_at(pos)
            && !b.message.trim().is_empty()
        {
            log::info!(
                "Test {} (at {}, {}, {}): {}",
                b.mode.serialized_name(),
                pos.0,
                pos.1,
                pos.2,
                b.message
            );
        }
    }

    /// `TestBlockEntity.trigger`.
    pub(super) fn trigger_test_block(&mut self, pos: Pos) {
        let Some(effects) = self.test_block_mut(pos).map(TestBlock::trigger) else {
            return;
        };
        self.apply_test_block_effects(pos, effects);
    }

    /// `TestBlockEntity.reset`, which `TestBlock.tick` runs.
    pub(super) fn reset_test_block(&mut self, pos: Pos) {
        let Some(effects) = self.test_block_mut(pos).map(TestBlock::reset) else {
            return;
        };
        self.apply_test_block_effects(pos, effects);
    }

    /// `TestBlock.neighborChanged`: a rising signal triggers the block.
    pub(super) fn test_block_neighbor_changed(&mut self, pos: Pos) {
        if self.test_block_at(pos).is_none() {
            return;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        let signal = self.with_level(|b, level, ticks| b.has_neighbor_signal(level, ticks, at));
        let Some(effects) = self
            .test_block_mut(pos)
            .and_then(|b| b.neighbor_changed(signal))
        else {
            return;
        };
        self.apply_test_block_effects(pos, effects);
    }

    /// `handleSetTestBlock` for a guest: only a creative player with game
    /// master commands edits the block.
    pub(super) fn lan_set_test_block(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetTestBlock,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        self.as_guest(host, conn, |game| {
            if allowed {
                game.apply_set_test_block(&p);
            }
        });
    }

    fn apply_set_test_block(&mut self, p: &play::SetTestBlock) {
        let pos = (p.pos[0], p.pos[1], p.pos[2]);
        if self.test_block_mut(pos).is_none() {
            return;
        }
        let mode = TestBlockMode::by_id(p.mode);
        // TestBlockEntity.setMode: the entity's and the block state's.
        if let Some(b) = self.test_block_mut(pos) {
            b.mode = mode;
        }
        self.set_mode_property(pos, "minecraft:test_block", mode.serialized_name());
        if let Some(b) = self.test_block_mut(pos) {
            b.message = p.message.clone();
        }
        self.mark_block_entity_updated(pos);
    }
}
