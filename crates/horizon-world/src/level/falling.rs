//! Gravity blocks: `FallingBlock.tick`, `FallingBlockEntity.fall` and the
//! landing branch of `FallingBlockEntity.tick`. The falling entity itself
//! (motion, collision, entity damage) is simulated by the caller.
use super::{Behaviours, LevelAccess, LevelEvent, Sim, Ticks, classes::Class, update::below};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};

/// Outcome of [`Behaviours::falling_block_landed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// The block was placed at the position (concrete powder may have
    /// solidified); discard the entity.
    Placed(BlockStateId),
    /// The block could not be placed: discard the entity and, if the entity
    /// drops items, spawn the block's item (vanilla `spawnAtLocation`).
    Broken,
    /// The landing position holds a moving piston: keep falling.
    Continue,
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `FallingBlockEntity.fall`.
    pub(crate) fn falling_block_fall(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        hurt_per_distance: Option<f32>,
    ) {
        let b = self.b;
        let carried = b
            .table
            .with_bool(state, b.props.waterlogged, false)
            .unwrap_or(state);
        let legacy = b.legacy_block(b.fluid(state));
        self.set_block_and_update(pos, legacy);
        self.emit(LevelEvent::FallingBlock {
            pos,
            state: carried,
            hurt_per_distance,
        });
    }
    /// `FallingBlock.tick` (and `SpeleothemBlock`'s falling stalactites).
    pub(crate) fn falling_block_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.is_free(self.below_state(pos)) && pos.y >= self.level.min_y() {
            self.falling_block_fall(pos, state, None);
        }
    }
    /// `SpeleothemBlock.spawnFallingStalactite`.
    pub(crate) fn spawn_falling_stalactite(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let is_stalactite = |s: BlockStateId| {
            b.tag("minecraft:speleothems")
                .is_some_and(|tag| b.in_set(s, tag))
                && b.table.dir(s, p.vertical_direction) == Some(Direction::Down)
        };
        let mut fall_pos = pos;
        let mut fall_state = state;
        while is_stalactite(fall_state) {
            let thickness = b.table.text(fall_state, p.thickness);
            let tip = thickness == "tip" || thickness == "tip_merge";
            let damage = tip.then_some((1 + pos.y - fall_pos.y).max(6) as f32);
            self.falling_block_fall(fall_pos, fall_state, damage);
            if tip {
                break;
            }
            fall_pos = below(fall_pos);
            fall_state = self.state(fall_pos);
        }
    }
}

impl Behaviours {
    /// The landing branch of `FallingBlockEntity.tick`, called by the caller's
    /// falling-block entity once it is on the ground (or stuck in water as
    /// concrete powder). `state` is the carried block state.
    pub fn falling_block_landed<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        stuck_in_water: bool,
    ) -> Landing {
        let mut sim = self.sim(level, ticks);
        let current = sim.state(pos);
        if self.is_block(current, self.named.moving_piston) {
            return Landing::Continue;
        }
        let concrete = self.instance_of(state, Class::ConcretePowder);
        // DirectionalPlaceContext(level, pos, DOWN, ItemStack.EMPTY, UP).
        let ctx = super::place::Ctx {
            pos,
            hit_pos: pos,
            face: Direction::Up,
            hit: horizon_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y),
                f64::from(pos.z) + 0.5,
            ),
            inside: false,
            yaw: 0.0,
            pitch: 90.0,
            sneaking: false,
            item: String::new(),
            replace_clicked: true,
        };
        let may_replace = sim.can_be_replaced(current, &ctx);
        let would_continue = sim.is_free(sim.below_state(pos)) && (!concrete || !stuck_in_water);
        let would_survive = sim.can_survive(state, pos) && !would_continue;
        if !(may_replace && would_survive) {
            if self.instance_of(state, Class::Anvil) {
                sim.level_event(1029, pos, 0);
            }
            return Landing::Broken;
        }
        let mut placed = state;
        if self.table.has(placed, self.props.waterlogged)
            && self.is_water_fluid(sim.fluid_at(pos).kind())
        {
            placed = self.table.set_bool(placed, self.props.waterlogged, true);
        }
        if !sim.set_block_and_update(pos, placed) {
            if self.instance_of(state, Class::Anvil) {
                sim.level_event(1029, pos, 0);
            }
            return Landing::Broken;
        }
        // Fallable.onLand
        if concrete {
            if self.is_water_fluid(self.fluid(current).kind()) || sim.concrete_touches_liquid(pos) {
                if let Some(solid) = self.data(state).link {
                    sim.set_block_and_update(pos, self.default_of(solid));
                }
            }
        } else if self.instance_of(state, Class::Anvil) {
            sim.level_event(1031, pos, 0);
        }
        Landing::Placed(sim.state(pos))
    }
}
