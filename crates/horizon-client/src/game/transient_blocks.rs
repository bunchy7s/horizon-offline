//! `ClientboundAddTransientBlockPacket`: a block the server places that the
//! client shows for a moment, drawn as a moving block until the section mesh
//! that holds it has been rebuilt (a landed falling block would otherwise
//! leave a gap between the entity going and the mesh catching up).
use super::Game;
use horizon_client::mesher::Vertex;
use horizon_world::registry::BlockStateId;
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

/// `TransientBlock.DEFAULT_TIME_TO_LIVE`, in seconds.
const DEFAULT_TIME_TO_LIVE: f64 = 1.0;

/// `TransientBlock`: a block queued by the packet.
struct Queued {
    pos: [i32; 3],
    state: BlockStateId,
    time_to_live: f64,
}

/// `TransientBlockRenderState`: a block on show.
struct Shown {
    state: BlockStateId,
    live_until: Instant,
    /// `createTimeNs`, on the mesher's dispatch clock (`Streaming::sequence`):
    /// `SectionOcclusionGraph.schedulePropagationFrom` removes the block once a
    /// mesh of its section whose compile task started after this is uploaded.
    created: u64,
}

/// `LevelExtractor.transientBlockQueue` and `LevelRenderer.transientBlocks`.
#[derive(Default)]
pub(super) struct TransientBlocks {
    queue: VecDeque<Queued>,
    /// By block position, so a second one at the same place replaces it.
    shown: HashMap<[i32; 3], Shown>,
}

impl TransientBlocks {
    /// `LevelExtractor.queueTransientBlock`.
    fn queue(&mut self, pos: [i32; 3], state: BlockStateId) {
        self.queue.push_back(Queued {
            pos,
            state,
            time_to_live: DEFAULT_TIME_TO_LIVE,
        });
    }

    /// `LevelExtractor.drainTransientBlockQueue`: queued blocks go on show
    /// for their time to live from `now`.
    fn drain(&mut self, now: Instant, created: u64) {
        while let Some(block) = self.queue.pop_front() {
            self.shown.insert(
                block.pos,
                Shown {
                    state: block.state,
                    live_until: now + Duration::from_secs_f64(block.time_to_live),
                    created,
                },
            );
        }
    }

    pub(super) fn clear(&mut self) {
        self.queue.clear();
        self.shown.clear();
    }
}

/// `performTransientBlockRemovals`: a section's uploaded mesh shows a block created at `created`
/// when its compile task started after it (`state.createTimeNs > compileTaskStartTimeNs` keeps it).
fn shows_block(created: u64, compile_start: Option<u64>, upload_pending: bool) -> bool {
    !upload_pending && compile_start.is_some_and(|start| created < start)
}
impl Game<'_> {
    /// `ClientPacketListener.handleAddTransientBlockPacket`.
    pub(super) fn add_transient_block(&mut self, pos: [i32; 3], state: i32) {
        self.transient_blocks.queue(pos, BlockStateId(state as u32));
    }

    /// `LevelExtractor.drainTransientBlockQueue`, once a frame.
    pub(super) fn drain_transient_block_queue(&mut self) {
        self.transient_blocks
            .drain(Instant::now(), self.streaming.sequence());
    }

    /// `LevelRenderer.submitTransientBlocks`: each block whose time has not
    /// run out, drawn as a moving block at its position, camera-relative.
    /// One whose section mesh has been rebuilt since it came is dropped
    /// (`performTransientBlockRemovals`).
    pub(super) fn transient_block_geometry(&mut self, camera: glam::DVec3, out: &mut Vec<Vertex>) {
        if self.transient_blocks.shown.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut shown = std::mem::take(&mut self.transient_blocks.shown);
        shown.retain(|pos, block| {
            if now >= block.live_until {
                return false;
            }
            // performTransientBlockRemovals: the section's uploaded mesh began compiling
            // at or after the block was created, so it shows the block.
            !self.section_of(*pos).is_some_and(|section| {
                shows_block(
                    block.created,
                    self.streaming.compile_start(section),
                    self.section_upload_pending(section),
                )
            })
        });
        for (pos, block) in &shown {
            let at = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
            self.moving_block_vertices(
                block.state,
                horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
                at - camera,
                out,
            );
        }
        self.transient_blocks.shown = shown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_dropped_once_a_mesh_compiled_after_it_is_uploaded() {
        // No mesh yet, or one that began before the block existed: kept.
        assert!(!shows_block(5, None, false));
        assert!(!shows_block(5, Some(5), false));
        assert!(!shows_block(5, Some(3), false));
        // A later compile start shows it, once the mesh is uploaded.
        assert!(shows_block(5, Some(6), false));
        assert!(!shows_block(5, Some(6), true));
    }

    /// `drainTransientBlockQueue`: a block goes on show for a second from the
    /// frame that drains it, and a second at the same place replaces it.
    #[test]
    fn queued_blocks_go_on_show_for_a_second() {
        let mut blocks = TransientBlocks::default();
        blocks.queue([1, 2, 3], BlockStateId(5));
        assert!(blocks.shown.is_empty());
        let now = Instant::now();
        blocks.drain(now, 7);
        assert!(blocks.queue.is_empty());
        assert_eq!(blocks.shown[&[1, 2, 3]].created, 7);
        assert_eq!(
            blocks.shown[&[1, 2, 3]].live_until,
            now + Duration::from_secs(1)
        );
        blocks.queue([1, 2, 3], BlockStateId(6));
        blocks.drain(now, 9);
        assert_eq!(blocks.shown.len(), 1);
        assert_eq!(blocks.shown[&[1, 2, 3]].state, BlockStateId(6));
    }
}
