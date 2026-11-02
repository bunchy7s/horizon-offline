//! `ExperimentalRedstoneWireEvaluator`: calculate first, then write and notify in
//! first-discovery order. A wire retains the orientation with which it was found.
use super::*;
use crate::level::orientation::{Orientation, SideBias};
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
struct Evaluator {
    off: VecDeque<BlockPos>,
    on: VecDeque<BlockPos>,
    indices: HashMap<BlockPos, usize>,
    wires: Vec<(BlockPos, i32, Orientation)>,
}
impl Evaluator {
    fn set_power(&mut self, pos: BlockPos, power: i32, orientation: Orientation) {
        if let Some(&index) = self.indices.get(&pos) {
            self.wires[index].1 = power;
        } else {
            self.indices.insert(pos, self.wires.len());
            self.wires.push((pos, power, orientation));
        }
    }
    fn power<L: LevelAccess + ?Sized>(&self, sim: &Sim<'_, L>, pos: BlockPos) -> i32 {
        self.indices
            .get(&pos)
            .map_or_else(|| sim.wire_power_of(sim.state(pos)), |&i| self.wires[i].1)
    }
    fn incoming<L: LevelAccess + ?Sized>(&self, sim: &Sim<'_, L>, pos: BlockPos) -> i32 {
        let mut signal = 0;
        for direction in HORIZONTAL {
            let neighbor_pos = rel(pos, direction);
            let neighbor = sim.state(neighbor_pos);
            signal = signal.max(self.power(sim, neighbor_pos));
            let above_pos = above(pos);
            if sim.is_redstone_conductor(neighbor, neighbor_pos)
                && !sim.is_redstone_conductor(sim.state(above_pos), above_pos)
            {
                signal = signal.max(self.power(sim, above(neighbor_pos)));
            } else if !sim.is_redstone_conductor(neighbor, neighbor_pos) {
                signal = signal.max(self.power(sim, below(neighbor_pos)));
            }
        }
        (signal - 1).max(0)
    }
    fn enqueue<L: LevelAccess + ?Sized>(
        &mut self,
        sim: &Sim<'_, L>,
        pos: BlockPos,
        from_power: i32,
        orientation: Orientation,
        turn_off: bool,
    ) {
        if !sim.b.is_block(sim.state(pos), sim.b.named.redstone_wire) {
            return;
        }
        let power = self.power(sim, pos);
        if power < from_power - 1 && !self.on.contains(&pos) {
            self.on.push_back(pos);
            self.set_power(pos, power, orientation);
        }
        if turn_off && power > from_power && !self.off.contains(&pos) {
            self.off.push_back(pos);
            self.set_power(pos, power, orientation);
        }
    }
    fn propagate<L: LevelAccess + ?Sized>(
        &mut self,
        sim: &Sim<'_, L>,
        pos: BlockPos,
        power: i32,
        orientation: Orientation,
        turn_off: bool,
    ) {
        for direction in orientation.horizontal_directions() {
            self.enqueue(
                sim,
                rel(pos, direction),
                power,
                orientation.with_front(direction),
                turn_off,
            );
        }
        for vertical in orientation.vertical_directions() {
            let offset = rel(pos, vertical);
            let solid = sim.is_redstone_conductor(sim.state(offset), offset);
            for direction in orientation.horizontal_directions() {
                let neighbor = rel(pos, direction);
                if (vertical == Direction::Up && !solid)
                    || (vertical == Direction::Down
                        && !sim.is_redstone_conductor(sim.state(neighbor), neighbor))
                {
                    self.enqueue(
                        sim,
                        rel(offset, direction),
                        power,
                        orientation.with_front(direction),
                        turn_off,
                    );
                }
            }
        }
    }
    fn calculate<L: LevelAccess + ?Sized>(
        &mut self,
        sim: &Sim<'_, L>,
        initial: BlockPos,
        orientation: Orientation,
    ) {
        let state = sim.state(initial);
        if sim.b.is_block(state, sim.b.named.redstone_wire) {
            self.set_power(initial, sim.wire_power_of(state), orientation);
            self.off.push_back(initial);
        } else {
            self.propagate(sim, initial, 0, orientation, true);
        }
        while let Some(pos) = self.off.pop_front() {
            let (_, old, orientation) = self.wires[self.indices[&pos]];
            let block = sim.wire_block_signal(pos);
            let new = block.max(self.incoming(sim, pos));
            let power = if new < old {
                if block > 0 && !self.on.contains(&pos) {
                    self.on.push_back(pos);
                }
                0
            } else {
                new
            };
            if power != old {
                self.set_power(pos, power, orientation);
            }
            self.propagate(sim, pos, power, orientation, old > new);
        }
        while let Some(pos) = self.on.pop_front() {
            let (_, old, orientation) = self.wires[self.indices[&pos]];
            let new = sim.wire_block_signal(pos).max(self.incoming(sim, pos));
            if new > old {
                self.set_power(pos, new, orientation);
            } else {
                assert_eq!(new, old, "Turning off wire while trying to turn it on");
            }
            self.propagate(sim, pos, new, orientation, false);
        }
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub(super) fn experimental_wire_update(
        &mut self,
        pos: BlockPos,
        incoming: Option<Orientation>,
        update_shape: bool,
    ) {
        let orientation = incoming
            .unwrap_or_else(|| Orientation::random(self.random()))
            .with_up(Direction::Up)
            .with_side_bias(SideBias::Left);
        let mut evaluator = Evaluator::default();
        evaluator.calculate(self, pos, orientation);
        let mut changed = Vec::new();
        for (index, (pos, power, orientation)) in evaluator.wires.into_iter().enumerate() {
            let state = self.state(pos);
            if self.b.is_block(state, self.b.named.redstone_wire)
                && self.wire_power_of(state) != power
            {
                let flags = if update_shape && index == 0 {
                    2
                } else {
                    2 | 128
                };
                self.set_block(
                    pos,
                    self.b.table.set_int(state, self.b.props.power, power),
                    flags,
                );
                changed.push((pos, orientation));
            }
        }
        let wire = self.b.named.redstone_wire;
        for (pos, orientation) in changed {
            let state = self.state(pos);
            for direction in orientation.directions() {
                let connected = if axis_of(direction) == Axis::Y {
                    direction == Direction::Down
                } else {
                    self.wire_connected(state, direction)
                };
                if !connected {
                    continue;
                }
                let neighbor = rel(pos, direction);
                let neighbor_state = self.state(neighbor);
                let next = orientation.with_front_preserve_up(direction);
                self.neighbor_changed_full_oriented(
                    neighbor_state,
                    neighbor,
                    wire,
                    Some(next),
                    false,
                );
                if self.is_redstone_conductor(neighbor_state, neighbor) {
                    for other in next.directions() {
                        if other != direction.opposite() {
                            self.neighbor_changed_at_oriented(
                                rel(neighbor, other),
                                wire,
                                Some(next.with_front_preserve_up(other)),
                            );
                        }
                    }
                }
            }
        }
    }
}
