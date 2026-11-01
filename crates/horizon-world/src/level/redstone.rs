//! Redstone: `SignalGetter`, redstone wire (default evaluator), torches,
//! diodes (repeaters, comparators), observers, lamps and the power-driven
//! responses of doors, trapdoors, gates and other consumers.
use super::{
    LevelAccess, LevelEvent, Sim,
    classes::Class,
    data::state_flags,
    query::{axis_of, clockwise, counter_clockwise},
    states::BlockId,
    update::{HORIZONTAL, above, below, rel},
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::{Axis, Direction, trig},
    position::BlockPos,
    random::RandomSource,
    ticks::TickPriority,
};

const SIGNAL_ORDER: [Direction; 6] = [
    Direction::Down,
    Direction::Up,
    Direction::North,
    Direction::South,
    Direction::West,
    Direction::East,
];

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn nibble(word: u32, direction: Direction) -> i32 {
        ((word >> (4 * direction as u32)) & 15) as i32
    }
    pub(crate) fn is_signal_source(&self, state: BlockStateId) -> bool {
        if self.b.is_block(state, self.b.named.redstone_wire) {
            return !self.wire_signal_off.get();
        }
        self.level
            .adjust_signal_source(state, self.b.has_flag(state, state_flags::SIGNAL_SOURCE))
    }
    /// `BlockState.getSignal(level, pos, direction)`.
    pub(crate) fn state_signal(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> i32 {
        let vanilla = self.vanilla_signal(state, pos, direction);
        self.level
            .adjust_signal(pos, state, direction, false, vanilla)
    }
    fn vanilla_signal(&self, state: BlockStateId, pos: BlockPos, direction: Direction) -> i32 {
        let b = self.b;
        // BlockBehaviour.getSignal is `ownSignal` for a TestBlock, which is
        // not an `isSignalSource`.
        if b.data(state).class == Class::TestBlock {
            return self.level.block_entity_signal(pos, state, direction);
        }
        if !b.has_flag(state, state_flags::SIGNAL_SOURCE) {
            return 0;
        }
        let data = b.data(state);
        match data.class {
            Class::RedstoneWire => self.wire_signal(state, pos, direction),
            Class::Comparator => {
                if b.facing(state) == direction && b.table.bool(state, b.props.powered) {
                    self.comparator_output(pos)
                } else {
                    0
                }
            }
            Class::TrappedChest | Class::Jukebox => {
                self.level.block_entity_signal(pos, state, direction)
            }
            _ => Self::nibble(b.facts(state).weak, direction),
        }
    }
    /// `BlockState.getDirectSignal(level, pos, direction)`.
    pub(crate) fn state_direct_signal(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> i32 {
        let vanilla = self.vanilla_direct_signal(state, pos, direction);
        self.level
            .adjust_signal(pos, state, direction, true, vanilla)
    }
    fn vanilla_direct_signal(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> i32 {
        let b = self.b;
        if !b.has_flag(state, state_flags::SIGNAL_SOURCE) {
            return 0;
        }
        match b.data(state).class {
            Class::RedstoneWire => {
                if self.wire_signal_off.get() {
                    0
                } else {
                    self.wire_signal(state, pos, direction)
                }
            }
            Class::Comparator => self.vanilla_signal(state, pos, direction),
            Class::TrappedChest => {
                if direction == Direction::Up {
                    self.level.block_entity_signal(pos, state, direction)
                } else {
                    0
                }
            }
            _ => Self::nibble(b.facts(state).strong, direction),
        }
    }
    pub(crate) fn comparator_output(&self, pos: BlockPos) -> i32 {
        self.ticks
            .comparator_outputs
            .get(&pos)
            .copied()
            .unwrap_or(0)
    }
    fn wire_signal(&self, state: BlockStateId, pos: BlockPos, direction: Direction) -> i32 {
        if self.wire_signal_off.get() || direction == Direction::Down {
            return 0;
        }
        let power = self.b.table.int(state, self.b.props.power);
        if power == 0 {
            return 0;
        }
        if direction != Direction::Up {
            let connected = self.wire_connection_state(state, pos);
            if self
                .b
                .table
                .text(connected, self.direction_prop(direction.opposite()))
                == "none"
            {
                return 0;
            }
        }
        power
    }
    /// `SignalGetter.getDirectSignal(pos, direction)`.
    pub fn direct_signal(&self, pos: BlockPos, direction: Direction) -> i32 {
        self.state_direct_signal(self.state(pos), pos, direction)
    }
    /// `SignalGetter.getDirectSignalTo`.
    pub fn direct_signal_to(&self, pos: BlockPos) -> i32 {
        let mut result = 0;
        for direction in SIGNAL_ORDER {
            result = result.max(self.direct_signal(rel(pos, direction), direction));
            if result >= 15 {
                return result;
            }
        }
        result
    }
    /// `SignalGetter.getSignal(pos, direction)`.
    pub fn signal(&self, pos: BlockPos, direction: Direction) -> i32 {
        let state = self.state(pos);
        let signal = self.state_signal(state, pos, direction);
        if self.is_redstone_conductor(state, pos) {
            signal.max(self.direct_signal_to(pos))
        } else {
            signal
        }
    }
    pub fn has_signal(&self, pos: BlockPos, direction: Direction) -> bool {
        self.signal(pos, direction) > 0
    }
    /// `SignalGetter.hasNeighborSignal`.
    pub fn has_neighbor_signal(&self, pos: BlockPos) -> bool {
        SIGNAL_ORDER
            .into_iter()
            .any(|direction| self.signal(rel(pos, direction), direction) > 0)
    }
    /// `SignalGetter.getBestNeighborSignal`.
    pub fn best_neighbor_signal(&self, pos: BlockPos) -> i32 {
        let mut best = 0;
        for direction in SIGNAL_ORDER {
            let signal = self.signal(rel(pos, direction), direction);
            if signal >= 15 {
                return 15;
            }
            best = best.max(signal);
        }
        best
    }
    /// `SignalGetter.getControlInputSignal`.
    fn control_input_signal(&self, pos: BlockPos, direction: Direction, only_diodes: bool) -> i32 {
        let b = self.b;
        let state = self.state(pos);
        if only_diodes {
            if b.instance_of(state, Class::Diode) {
                self.direct_signal(pos, direction)
            } else {
                0
            }
        } else if b.is_block(state, b.named.redstone_block) {
            15
        } else if b.is_block(state, b.named.redstone_wire) {
            b.table.int(state, b.props.power)
        } else if self.is_signal_source(state) {
            self.direct_signal(pos, direction)
        } else {
            0
        }
    }

    // ---- redstone wire ----

    fn should_wire_connect_to(&self, state: BlockStateId, direction: Option<Direction>) -> bool {
        let b = self.b;
        match b.data(state).class {
            Class::RedstoneWire => true,
            Class::Observer => direction == Some(b.facing(state)),
            Class::Repeater => {
                let facing = b.facing(state);
                direction == Some(facing) || direction == Some(facing.opposite())
            }
            _ => self.is_signal_source(state) && direction.is_some(),
        }
    }
    /// `RedstoneWireBlock.canSurvive` and `updateShape`'s down face: the block
    /// below is read through `support_state`, since a retracting piston's
    /// `moving_piston` block carries the extended base only through its block
    /// entity and its exported facts call every face unsupported (#4728).
    fn wire_can_survive_on(&self, pos: BlockPos) -> bool {
        let state = self.support_state(pos);
        self.b.is_face_sturdy(state, Direction::Up) || self.b.is_block(state, self.b.named.hopper)
    }
    fn wire_connecting_side(
        &self,
        pos: BlockPos,
        direction: Direction,
        can_connect_up: bool,
    ) -> &'static str {
        let b = self.b;
        let relative_pos = rel(pos, direction);
        let relative = self.state(relative_pos);
        if can_connect_up {
            let placeable_above =
                b.instance_of(relative, Class::TrapDoor) || self.wire_can_survive_on(relative_pos);
            if placeable_above && self.should_wire_connect_to(self.state(above(relative_pos)), None)
            {
                return if b.is_face_sturdy(relative, direction.opposite()) {
                    "up"
                } else {
                    "side"
                };
            }
        }
        if !self.should_wire_connect_to(relative, Some(direction))
            && (self.is_redstone_conductor(relative, relative_pos)
                || !self.should_wire_connect_to(self.state(below(relative_pos)), None))
        {
            "none"
        } else {
            "side"
        }
    }
    fn wire_can_connect_up(&self, pos: BlockPos) -> bool {
        !self.is_redstone_conductor(self.above_state(pos), pos)
    }
    fn wire_connected(&self, state: BlockStateId, direction: Direction) -> bool {
        self.b.table.text(state, self.direction_prop(direction)) != "none"
    }
    fn wire_is_cross(&self, state: BlockStateId) -> bool {
        HORIZONTAL
            .into_iter()
            .all(|d| self.wire_connected(state, d))
    }
    fn wire_is_dot(&self, state: BlockStateId) -> bool {
        !HORIZONTAL
            .into_iter()
            .any(|d| self.wire_connected(state, d))
    }
    pub(crate) fn wire_cross_state(&self, power: i32) -> BlockStateId {
        let b = self.b;
        let mut state = b.default_of(b.named.redstone_wire);
        for direction in HORIZONTAL {
            state = b
                .table
                .set_text(state, self.direction_prop(direction), "side");
        }
        b.table.set_int(state, b.props.power, power)
    }
    /// `RedstoneWireBlock.getConnectionState`.
    pub(crate) fn wire_connection_state(&self, state: BlockStateId, pos: BlockPos) -> BlockStateId {
        let b = self.b;
        let table = &b.table;
        let was_dot = self.wire_is_dot(state);
        let mut result = table.set_int(
            b.default_of(b.named.redstone_wire),
            b.props.power,
            table.int(state, b.props.power),
        );
        let can_up = self.wire_can_connect_up(pos);
        for direction in HORIZONTAL {
            if !self.wire_connected(result, direction) {
                let side = self.wire_connecting_side(pos, direction, can_up);
                result = table.set_text(result, self.direction_prop(direction), side);
            }
        }
        if was_dot && self.wire_is_dot(result) {
            return result;
        }
        let north = self.wire_connected(result, Direction::North);
        let south = self.wire_connected(result, Direction::South);
        let east = self.wire_connected(result, Direction::East);
        let west = self.wire_connected(result, Direction::West);
        let ns_empty = !north && !south;
        let ew_empty = !east && !west;
        if !west && ns_empty {
            result = table.set_text(result, b.props.west, "side");
        }
        if !east && ns_empty {
            result = table.set_text(result, b.props.east, "side");
        }
        if !north && ew_empty {
            result = table.set_text(result, b.props.north, "side");
        }
        if !south && ew_empty {
            result = table.set_text(result, b.props.south, "side");
        }
        result
    }
    pub(crate) fn redstone_wire_update_shape(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        npos: BlockPos,
        _neighbour: BlockStateId,
    ) -> BlockStateId {
        let b = self.b;
        match direction {
            Direction::Down => {
                if self.wire_can_survive_on(npos) {
                    state
                } else {
                    b.air()
                }
            }
            Direction::Up => self.wire_connection_state(state, pos),
            _ => {
                let side = self.wire_connecting_side(pos, direction, self.wire_can_connect_up(pos));
                let key = self.direction_prop(direction);
                if (side != "none") == self.wire_connected(state, direction)
                    && !self.wire_is_cross(state)
                {
                    b.table.set_text(state, key, side)
                } else {
                    let power = b.table.int(state, b.props.power);
                    let cross = b.table.set_text(self.wire_cross_state(power), key, side);
                    self.wire_connection_state(cross, pos)
                }
            }
        }
    }
    /// `RedstoneWireBlock.updateIndirectNeighbourShapes`.
    pub(crate) fn wire_update_indirect(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        flags: i32,
        limit: i32,
    ) {
        let b = self.b;
        let wire = b.named.redstone_wire;
        for direction in HORIZONTAL {
            if b.table.text(state, self.direction_prop(direction)) != "none"
                && !b.is_block(self.rel_state(pos, direction), wire)
            {
                let down = below(rel(pos, direction));
                if b.is_block(self.state(down), wire) {
                    let neighbour = rel(down, direction.opposite());
                    let neighbour_state = self.state(neighbour);
                    self.neighbor_shape_changed(
                        direction.opposite(),
                        down,
                        neighbour,
                        neighbour_state,
                        flags,
                        limit,
                    );
                }
                let up = above(rel(pos, direction));
                if b.is_block(self.state(up), wire) {
                    let neighbour = rel(up, direction.opposite());
                    let neighbour_state = self.state(neighbour);
                    self.neighbor_shape_changed(
                        direction.opposite(),
                        up,
                        neighbour,
                        neighbour_state,
                        flags,
                        limit,
                    );
                }
            }
        }
    }
    fn wire_block_signal(&self, pos: BlockPos) -> i32 {
        self.wire_signal_off.set(true);
        let signal = self.best_neighbor_signal(pos);
        self.wire_signal_off.set(false);
        signal
    }
    fn wire_power_of(&self, state: BlockStateId) -> i32 {
        if self.b.is_block(state, self.b.named.redstone_wire) {
            self.b.table.int(state, self.b.props.power)
        } else {
            0
        }
    }
    fn wire_incoming_signal(&self, pos: BlockPos) -> i32 {
        let mut signal = 0;
        for direction in HORIZONTAL {
            let neighbour_pos = rel(pos, direction);
            let neighbour = self.state(neighbour_pos);
            signal = signal.max(self.wire_power_of(neighbour));
            let above_pos = above(pos);
            if self.is_redstone_conductor(neighbour, neighbour_pos)
                && !self.is_redstone_conductor(self.state(above_pos), above_pos)
            {
                signal = signal.max(self.wire_power_of(self.state(above(neighbour_pos))));
            } else if !self.is_redstone_conductor(neighbour, neighbour_pos) {
                signal = signal.max(self.wire_power_of(self.state(below(neighbour_pos))));
            }
        }
        (signal - 1).max(0)
    }
    /// `DefaultRedstoneWireEvaluator.updatePowerStrength`.
    pub(crate) fn wire_update_power(&mut self, pos: BlockPos, state: BlockStateId) {
        let block_signal = self.wire_block_signal(pos);
        let target = if block_signal == 15 {
            15
        } else {
            block_signal.max(self.wire_incoming_signal(pos))
        };
        if self.b.table.int(state, self.b.props.power) != target {
            if self.state(pos) == state {
                let next = self.b.table.set_int(state, self.b.props.power, target);
                self.set_block(pos, next, 2);
            }
            // Vanilla iterates a HashSet of the position and its six
            // neighbours; BlockPos hash order is reproduced below.
            let wire = self.b.named.redstone_wire;
            let mut targets: Vec<BlockPos> = std::iter::once(pos)
                .chain(Direction::ALL.map(|d| rel(pos, d)))
                .collect();
            java_hash_set_order(&mut targets);
            for target_pos in targets {
                self.update_neighbors_at(target_pos, wire);
            }
        }
    }
    fn wire_check_corner(&mut self, pos: BlockPos) {
        let wire = self.b.named.redstone_wire;
        if self.b.is_block(self.state(pos), wire) {
            self.update_neighbors_at(pos, wire);
            for direction in Direction::ALL {
                self.update_neighbors_at(rel(pos, direction), wire);
            }
        }
    }
    pub(crate) fn wire_update_neighbors_of_neighboring_wires(&mut self, pos: BlockPos) {
        for direction in HORIZONTAL {
            self.wire_check_corner(rel(pos, direction));
        }
        for direction in HORIZONTAL {
            let target = rel(pos, direction);
            if self.is_redstone_conductor(self.state(target), target) {
                self.wire_check_corner(above(target));
            } else {
                self.wire_check_corner(below(target));
            }
        }
    }

    // ---- diodes ----

    pub(crate) fn diode_delay(&self, state: BlockStateId) -> i32 {
        if self.b.instance_of(state, Class::Repeater) {
            self.b.table.int(state, self.b.props.delay) * 2
        } else {
            2
        }
    }
    pub(crate) fn diode_is_locked(&self, pos: BlockPos, state: BlockStateId) -> bool {
        self.b.instance_of(state, Class::Repeater) && self.diode_alternate_signal(pos, state) > 0
    }
    fn diode_alternate_signal(&self, pos: BlockPos, state: BlockStateId) -> i32 {
        let facing = self.b.facing(state);
        let only_diodes = self.b.instance_of(state, Class::Repeater);
        let cw = clockwise(facing);
        let ccw = counter_clockwise(facing);
        self.control_input_signal(rel(pos, cw), cw, only_diodes)
            .max(self.control_input_signal(rel(pos, ccw), ccw, only_diodes))
    }
    fn diode_input_signal(&self, pos: BlockPos, state: BlockStateId) -> i32 {
        let b = self.b;
        let facing = b.facing(state);
        let target = rel(pos, facing);
        let input = self.signal(target, facing);
        let mut result = if input >= 15 {
            input
        } else {
            let target_state = self.state(target);
            input.max(self.wire_power_of(target_state))
        };
        if b.instance_of(state, Class::Comparator) {
            let target_state = self.state(target);
            if self.has_analog_output(target_state) {
                result = self.analog_output(target_state, target, facing.opposite());
            } else if result < 15 && self.is_redstone_conductor(target_state, target) {
                let further = rel(target, facing);
                let further_state = self.state(further);
                let block = self
                    .has_analog_output(further_state)
                    .then(|| self.analog_output(further_state, further, facing.opposite()));
                // ComparatorBlock.getInputSignal: an item frame behind the
                // conductor counts too, the larger signal winning.
                let frame = self.level.item_frame_analog_output(further, facing);
                if let Some(signal) = frame.into_iter().chain(block).max() {
                    result = signal;
                }
            }
        }
        result
    }
    pub(crate) fn diode_should_turn_on(&self, pos: BlockPos, state: BlockStateId) -> bool {
        if self.b.instance_of(state, Class::Comparator) {
            let input = self.diode_input_signal(pos, state);
            if input == 0 {
                return false;
            }
            let side = self.diode_alternate_signal(pos, state);
            input > side
                || (input == side && self.b.table.text(state, self.b.props.mode) == "compare")
        } else {
            self.diode_input_signal(pos, state) > 0
        }
    }
    fn diode_should_prioritize(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let direction = self.b.facing(state).opposite();
        let opposite = self.rel_state(pos, direction);
        self.b.instance_of(opposite, Class::Diode) && self.b.facing(opposite) != direction
    }
    pub(crate) fn diode_update_neighbors_in_front(&mut self, pos: BlockPos, state: BlockStateId) {
        let direction = self.b.facing(state);
        let opposite_pos = rel(pos, direction.opposite());
        let block = self.b.block(state);
        self.neighbor_changed_at(opposite_pos, block);
        self.update_neighbors_at_except(opposite_pos, block, direction);
    }
    fn comparator_calculate_output(&self, pos: BlockPos, state: BlockStateId) -> i32 {
        let input = self.diode_input_signal(pos, state);
        if input == 0 {
            return 0;
        }
        let alternate = self.diode_alternate_signal(pos, state);
        if alternate > input {
            0
        } else if self.b.table.text(state, self.b.props.mode) == "subtract" {
            input - alternate
        } else {
            input
        }
    }
    /// `DiodeBlock.checkTickOnNeighbor` (and the comparator override).
    pub(crate) fn diode_check_tick_on_neighbor(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.b.block(state);
        if self.b.instance_of(state, Class::Comparator) {
            if !self.ticks.will_block_tick_this_tick(pos, block) {
                let output = self.comparator_calculate_output(pos, state);
                let old = self.comparator_output(pos);
                if output != old
                    || self.b.table.bool(state, self.b.props.powered)
                        != self.diode_should_turn_on(pos, state)
                {
                    let priority = if self.diode_should_prioritize(pos, state) {
                        TickPriority::High
                    } else {
                        TickPriority::Normal
                    };
                    self.schedule_tick_priority(pos, block, 2, priority);
                }
            }
            return;
        }
        if self.diode_is_locked(pos, state) {
            return;
        }
        let on = self.b.table.bool(state, self.b.props.powered);
        let should = self.diode_should_turn_on(pos, state);
        if on != should && !self.ticks.will_block_tick_this_tick(pos, block) {
            let priority = if self.diode_should_prioritize(pos, state) {
                TickPriority::ExtremelyHigh
            } else if on {
                TickPriority::VeryHigh
            } else {
                TickPriority::High
            };
            let delay = self.diode_delay(state);
            self.schedule_tick_priority(pos, block, delay, priority);
        }
    }
    /// `DiodeBlock.tick` / `ComparatorBlock.tick`.
    pub(crate) fn diode_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if b.instance_of(state, Class::Comparator) {
            self.comparator_refresh(pos, state);
            return;
        }
        if self.diode_is_locked(pos, state) {
            return;
        }
        let on = b.table.bool(state, b.props.powered);
        let should = self.diode_should_turn_on(pos, state);
        if on && !should {
            self.set_block(pos, b.table.set_bool(state, b.props.powered, false), 2);
        } else if !on {
            self.set_block(pos, b.table.set_bool(state, b.props.powered, true), 2);
            if !should {
                let delay = self.diode_delay(state);
                self.schedule_tick_priority(pos, b.block(state), delay, TickPriority::VeryHigh);
            }
        }
    }
    pub(crate) fn comparator_refresh(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let output = self.comparator_calculate_output(pos, state);
        let old = self
            .ticks
            .comparator_outputs
            .insert(pos, output)
            .unwrap_or(0);
        if old != output || b.table.text(state, b.props.mode) == "compare" {
            let should = self.diode_should_turn_on(pos, state);
            let on = b.table.bool(state, b.props.powered);
            if on && !should {
                self.set_block(pos, b.table.set_bool(state, b.props.powered, false), 2);
            } else if !on && should {
                self.set_block(pos, b.table.set_bool(state, b.props.powered, true), 2);
            }
            self.diode_update_neighbors_in_front(pos, state);
        }
    }
    /// `DiodeBlock.neighborChanged`.
    pub(crate) fn diode_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.b.block(state);
        if !self.b.is_block(self.state(pos), block) {
            return;
        }
        if self.can_survive(state, pos) {
            self.diode_check_tick_on_neighbor(pos, state);
        } else {
            self.drop_resources(state, pos);
            self.remove_block(pos, false);
            for direction in Direction::ALL {
                self.update_neighbors_at(rel(pos, direction), block);
            }
        }
    }

    /// `BlockState.hasAnalogOutputSignal`, as the level answers it (a mod's block).
    pub(crate) fn has_analog_output(&self, state: BlockStateId) -> bool {
        let vanilla = self.b.has_flag(state, state_flags::ANALOG_OUTPUT);
        self.level.adjust_has_analog_output(state, vanilla)
    }
    /// `BlockState.getAnalogOutputSignal`, as the level answers it (a mod's block), 0 to 15.
    pub(crate) fn analog_output(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> i32 {
        let vanilla = self.vanilla_analog_output(state, pos, direction);
        self.level
            .adjust_analog_output(pos, state, direction, vanilla)
            .clamp(0, 15)
    }
    /// `getAnalogOutputSignal` for state-only sources; others ask the caller.
    fn vanilla_analog_output(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> i32 {
        let b = self.b;
        let p = b.props;
        match b.data(state).class {
            Class::Beehive => b.table.int(state, p.honey_level),
            Class::Cake => (7 - b.table.int(state, p.bites)) * 2,
            Class::CandleCake => 14,
            Class::Composter | Class::LayeredCauldron => b.table.int(state, p.level),
            Class::CopperBulb => {
                if b.table.bool(self.state(pos), p.lit) {
                    15
                } else {
                    0
                }
            }
            Class::EndPortalFrame => {
                if b.table.bool(state, p.eye) {
                    15
                } else {
                    0
                }
            }
            Class::LavaCauldron => 3,
            Class::RespawnAnchor => {
                let charges = b.table.int(state, p.charges);
                (charges as f32 / 4.0 * 15.0).floor() as i32
            }
            Class::CopperGolemStatue => {
                let pose = b.table.text(state, p.copper_golem_pose);
                ["standing", "sitting", "running", "star"]
                    .iter()
                    .position(|p| *p == pose)
                    .map_or(1, |i| i as i32 + 1)
            }
            _ => self.level.block_entity_analog_signal(pos, state, direction),
        }
    }

    // ---- torches ----

    fn torch_has_neighbor_signal(&self, pos: BlockPos, state: BlockStateId) -> bool {
        if self.b.instance_of(state, Class::RedstoneWallTorch) {
            let opposite = self.b.facing(state).opposite();
            self.has_signal(rel(pos, opposite), opposite)
        } else {
            self.has_signal(below(pos), Direction::Down)
        }
    }
    pub(crate) fn torch_notify_neighbors(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.b.block(state);
        for direction in Direction::ALL {
            self.update_neighbors_at(rel(pos, direction), block);
        }
    }
    pub(crate) fn torch_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let block = self.b.block(state);
        if self.b.table.bool(state, self.b.props.lit) == self.torch_has_neighbor_signal(pos, state)
            && !self.ticks.will_block_tick_this_tick(pos, block)
        {
            self.schedule_tick(pos, block, 2);
        }
    }
    fn torch_toggled_too_frequently(&mut self, pos: BlockPos, add: bool) -> bool {
        if add {
            let time = self.level.game_time();
            self.ticks.torch_toggles.push((pos, time));
        }
        self.ticks
            .torch_toggles
            .iter()
            .filter(|(p, _)| *p == pos)
            .take(8)
            .count()
            >= 8
    }
    pub(crate) fn torch_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let signal = self.torch_has_neighbor_signal(pos, state);
        let time = self.level.game_time();
        while self
            .ticks
            .torch_toggles
            .first()
            .is_some_and(|(_, when)| time - when > 60)
        {
            self.ticks.torch_toggles.remove(0);
        }
        if b.table.bool(state, b.props.lit) {
            if signal {
                self.set_block_and_update(pos, b.table.set_bool(state, b.props.lit, false));
                if self.torch_toggled_too_frequently(pos, true) {
                    self.level_event(1502, pos, 0);
                    let block = b.block(self.state(pos));
                    self.schedule_tick(pos, block, 160);
                }
            }
        } else if !signal && !self.torch_toggled_too_frequently(pos, false) {
            self.set_block_and_update(pos, b.table.set_bool(state, b.props.lit, true));
        }
    }

    // ---- observers ----

    pub(crate) fn observer_start_signal(&mut self, pos: BlockPos, block: BlockId) {
        if !self.ticks.has_block_tick(pos, block) {
            self.schedule_tick(pos, block, 2);
        }
    }
    pub(crate) fn observer_update_in_front(&mut self, pos: BlockPos, state: BlockStateId) {
        let direction = self.b.facing(state);
        let opposite = rel(pos, direction.opposite());
        let block = self.b.block(state);
        self.neighbor_changed_at(opposite, block);
        self.update_neighbors_at_except(opposite, block, direction);
    }
    pub(crate) fn observer_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if b.table.bool(state, b.props.powered) {
            self.set_block(pos, b.table.set_bool(state, b.props.powered, false), 2);
        } else {
            self.set_block(pos, b.table.set_bool(state, b.props.powered, true), 2);
            self.schedule_tick(pos, b.block(state), 2);
        }
        self.observer_update_in_front(pos, state);
    }

    // ---- buttons, levers, plates ----

    /// `ButtonBlock/LeverBlock.updateNeighbours`.
    pub(crate) fn attached_update_neighbours(&mut self, pos: BlockPos, state: BlockStateId) {
        let front = self.face_attached_connected(state).opposite();
        let block = self.b.block(state);
        self.update_neighbors_at(pos, block);
        self.update_neighbors_at(rel(pos, front), block);
    }
    pub(crate) fn plate_update_neighbours(&mut self, pos: BlockPos, block: BlockId) {
        self.update_neighbors_at(pos, block);
        self.update_neighbors_at(below(pos), block);
    }
    pub(crate) fn plate_signal_for_state(&self, state: BlockStateId) -> i32 {
        let b = self.b;
        if b.instance_of(state, Class::WeightedPressurePlate) {
            b.table.int(state, b.props.power)
        } else if b.table.bool(state, b.props.powered) {
            15
        } else {
            0
        }
    }
    fn plate_signal_strength(&self, pos: BlockPos, state: BlockStateId) -> i32 {
        use horizon_core::geometry::{Aabb, Vec3};
        let b = self.b;
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos.x) + 0.0625,
                f64::from(pos.y),
                f64::from(pos.z) + 0.0625,
            ),
            Vec3::new(
                f64::from(pos.x) + 0.9375,
                f64::from(pos.y) + 0.25,
                f64::from(pos.z) + 0.9375,
            ),
        );
        let data = b.data(state);
        if data.is(Class::WeightedPressurePlate) {
            let max = data.max_weight.max(1);
            let count = (self
                .level
                .count_entities(area, super::EntityQuery::Everything)
                as i32)
                .min(max);
            if count > 0 {
                (count.min(max) as f32 / max as f32 * 15.0).ceil() as i32
            } else {
                0
            }
        } else {
            let query = if data.set_type.as_ref().is_some_and(|s| s.mobs_only) {
                super::EntityQuery::Mobs
            } else {
                super::EntityQuery::Everything
            };
            if self.level.count_entities(area, query) > 0 {
                15
            } else {
                0
            }
        }
    }
    /// `BasePressurePlateBlock.checkPressed`.
    pub(crate) fn plate_check_pressed(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        old_signal: i32,
    ) {
        let b = self.b;
        let signal = self.plate_signal_strength(pos, state);
        let was = old_signal > 0;
        let is = signal > 0;
        let block = b.block(state);
        if old_signal != signal {
            let new = if b.instance_of(state, Class::WeightedPressurePlate) {
                b.table.set_int(state, b.props.power, signal)
            } else {
                b.table.set_bool(state, b.props.powered, signal > 0)
            };
            self.set_block(pos, new, 2);
            self.plate_update_neighbours(pos, block);
        }
        if let Some(set) = b.data(state).set_type.clone() {
            if !is && was {
                self.sound(pos, &set.plate_off, 1.0, 1.0);
                self.game_event(pos, "minecraft:block_deactivate");
            } else if is && !was {
                self.sound(pos, &set.plate_on, 1.0, 1.0);
                self.game_event(pos, "minecraft:block_activate");
            }
        }
        if is {
            let delay = if b.instance_of(state, Class::WeightedPressurePlate) {
                10
            } else {
                20
            };
            self.schedule_tick(pos, block, delay);
        }
    }
    /// `ButtonBlock.press`.
    pub(crate) fn button_press(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        self.set_block_and_update(pos, b.table.set_bool(state, b.props.powered, true));
        self.attached_update_neighbours(pos, state);
        let data = b.data(state);
        self.schedule_tick(pos, b.block(state), data.ticks_to_stay_pressed);
        if let Some(set) = data.set_type.clone() {
            self.sound(pos, &set.button_on, 1.0, 1.0);
        }
        self.game_event(pos, "minecraft:block_activate");
    }
    /// `ButtonBlock.tick`.
    pub(crate) fn button_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if b.table.bool(state, b.props.powered) {
            self.button_check_pressed(pos, state);
        }
    }
    /// `ButtonBlock.checkPressed`: a button whose type allows arrows is
    /// pressed while an arrow is inside its shape, and stays pressed (the
    /// release is scheduled again) as long as one is.
    pub(crate) fn button_check_pressed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let data = b.data(state);
        let should_be_pressed = data.set_type.as_ref().is_some_and(|s| s.arrows)
            && self
                .level
                .count_entities(self.button_bounds(pos, state), super::EntityQuery::Arrows)
                > 0;
        let was_pressed = b.table.bool(state, b.props.powered);
        if should_be_pressed != was_pressed {
            self.set_block_and_update(
                pos,
                b.table.set_bool(state, b.props.powered, should_be_pressed),
            );
            self.attached_update_neighbours(pos, state);
            if let Some(set) = data.set_type.clone() {
                let sound = if should_be_pressed {
                    &set.button_on
                } else {
                    &set.button_off
                };
                self.sound(pos, sound, 1.0, 1.0);
            }
            self.game_event(
                pos,
                if should_be_pressed {
                    "minecraft:block_activate"
                } else {
                    "minecraft:block_deactivate"
                },
            );
        }
        if should_be_pressed {
            self.schedule_tick(pos, b.block(state), data.ticks_to_stay_pressed);
        }
    }
    /// `ButtonBlock.getShape(...).bounds().move(pos)`: a 6 by 4 plate on the
    /// face it is attached to, 2 pixels thick (1 when pressed).
    fn button_bounds(&self, pos: BlockPos, state: BlockStateId) -> horizon_core::geometry::Aabb {
        use horizon_core::geometry::{Aabb, Vec3};
        let b = self.b;
        let thickness = if b.table.bool(state, b.props.powered) {
            1.0 / 16.0
        } else {
            2.0 / 16.0
        };
        let attached = self.face_attached_connected(state).opposite().normal();
        let thick = attached.iter().position(|&n| n != 0).unwrap_or(1);
        // The 6 pixel side lies across the button's facing.
        let wide = if b.facing(state).normal()[2] != 0 {
            0
        } else {
            2
        };
        let tall = 3 - thick - wide;
        let (mut lo, mut hi) = ([0.0; 3], [1.0; 3]);
        if attached[thick] > 0 {
            lo[thick] = 1.0 - thickness;
        } else {
            hi[thick] = thickness;
        }
        (lo[wide], hi[wide]) = (5.0 / 16.0, 11.0 / 16.0);
        (lo[tall], hi[tall]) = (6.0 / 16.0, 10.0 / 16.0);
        let origin = [f64::from(pos.x), f64::from(pos.y), f64::from(pos.z)];
        Aabb::new(
            Vec3::new(origin[0] + lo[0], origin[1] + lo[1], origin[2] + lo[2]),
            Vec3::new(origin[0] + hi[0], origin[1] + hi[1], origin[2] + hi[2]),
        )
    }
    /// `DaylightDetectorBlock.updateSignalStrength`: the effective sky light
    /// at the detector, shaped by the sun's angle (or inverted), as its
    /// power.
    pub(crate) fn daylight_detector_update_signal(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let mut target = (self.level.sky_light(pos) - self.level.sky_darken()).max(0);
        if b.table.bool(state, b.props.inverted) {
            target = 15 - target;
        } else if target > 0 {
            let mut sun_angle = self.level.sun_angle(pos) * trig::DEG_TO_RAD;
            let offset = if sun_angle < std::f32::consts::PI {
                0.0
            } else {
                std::f32::consts::TAU
            };
            sun_angle += (offset - sun_angle) * 0.2;
            // Math.round(float).
            let scaled = target as f32 * trig::cos(f64::from(sun_angle));
            target = (f64::from(scaled) + 0.5).floor() as i32;
        }
        let target = target.clamp(0, 15);
        if b.table.int(state, b.props.power) != target {
            self.set_block_and_update(pos, b.table.set_int(state, b.props.power, target));
        }
    }
    /// `LeverBlock.pull`.
    pub(crate) fn lever_pull(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let state = b.table.cycle(state, b.props.powered);
        self.set_block_and_update(pos, state);
        self.attached_update_neighbours(pos, state);
        let powered = b.table.bool(state, b.props.powered);
        self.emit(LevelEvent::Sound {
            pos,
            sound: "minecraft:block.lever.click".into(),
            volume: 0.3,
            pitch: if powered { 0.6 } else { 0.5 },
        });
        self.game_event(
            pos,
            if powered {
                "minecraft:block_activate"
            } else {
                "minecraft:block_deactivate"
            },
        );
    }

    // ---- consumers ----

    pub(crate) fn door_neighbor_changed(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        changed: BlockId,
    ) {
        let b = self.b;
        let p = b.props;
        let other = if b.table.text(state, p.half) == "lower" {
            Direction::Up
        } else {
            Direction::Down
        };
        let signal = self.has_neighbor_signal(pos) || self.has_neighbor_signal(rel(pos, other));
        if b.block(state) != changed && signal != b.table.bool(state, p.powered) {
            if signal != b.table.bool(state, p.open) {
                self.door_sound(pos, state, signal);
                self.game_event(
                    pos,
                    if signal {
                        "minecraft:block_open"
                    } else {
                        "minecraft:block_close"
                    },
                );
            }
            let next = b
                .table
                .set_bool(b.table.set_bool(state, p.powered, signal), p.open, signal);
            self.set_block(pos, next, 2);
        }
    }
    pub(crate) fn door_sound(&mut self, pos: BlockPos, state: BlockStateId, open: bool) {
        if let Some(set) = self.b.data(state).set_type.clone() {
            let pitch = self.random().next_float() * 0.1 + 0.9;
            let sound = if open {
                set.door_open.clone()
            } else {
                set.door_close.clone()
            };
            self.sound(pos, &sound, 1.0, pitch);
        }
    }
    pub(crate) fn trapdoor_sound(&mut self, pos: BlockPos, state: BlockStateId, open: bool) {
        if let Some(set) = self.b.data(state).set_type.clone() {
            let pitch = self.random().next_float() * 0.1 + 0.9;
            let sound = if open {
                set.trapdoor_open.clone()
            } else {
                set.trapdoor_close.clone()
            };
            self.sound(pos, &sound, 1.0, pitch);
            self.game_event(
                pos,
                if open {
                    "minecraft:block_open"
                } else {
                    "minecraft:block_close"
                },
            );
        }
    }
    pub(crate) fn trapdoor_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let signal = self.has_neighbor_signal(pos);
        if signal != b.table.bool(state, p.powered) {
            let mut state = state;
            if b.table.bool(state, p.open) != signal {
                state = b.table.set_bool(state, p.open, signal);
                self.trapdoor_sound(pos, state, signal);
            }
            self.set_block(pos, b.table.set_bool(state, p.powered, signal), 2);
            if b.is_waterlogged(state) {
                self.schedule_water_tick(pos);
            }
        }
    }
    pub(crate) fn gate_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let power = self.has_neighbor_signal(pos);
        if b.table.bool(state, p.powered) != power {
            let next = b
                .table
                .set_bool(b.table.set_bool(state, p.powered, power), p.open, power);
            self.set_block(pos, next, 2);
            if b.table.bool(state, p.open) != power {
                let data = b.data(state);
                let sound = if power {
                    data.gate_open.clone()
                } else {
                    data.gate_close.clone()
                };
                let pitch = self.random().next_float() * 0.1 + 0.9;
                if let Some(sound) = sound {
                    self.sound(pos, &sound, 1.0, pitch);
                }
                self.game_event(
                    pos,
                    if power {
                        "minecraft:block_open"
                    } else {
                        "minecraft:block_close"
                    },
                );
            }
        }
    }
    pub(crate) fn lamp_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let lit = b.table.bool(state, b.props.lit);
        if lit != self.has_neighbor_signal(pos) {
            if lit {
                self.schedule_tick(pos, b.block(state), 4);
            } else {
                self.set_block(pos, b.table.cycle(state, b.props.lit), 2);
            }
        }
    }
    pub(crate) fn lamp_tick(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if b.table.bool(state, b.props.lit) && !self.has_neighbor_signal(pos) {
            self.set_block(pos, b.table.cycle(state, b.props.lit), 2);
        }
    }
    /// `CopperBulbBlock.checkAndFlip`.
    pub(crate) fn copper_bulb_check(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let signal = self.has_neighbor_signal(pos);
        if signal != b.table.bool(state, p.powered) {
            let mut next = state;
            if !b.table.bool(state, p.powered) {
                next = b.table.cycle(next, p.lit);
                let sound = if b.table.bool(next, p.lit) {
                    "minecraft:block.copper_bulb.turn_on"
                } else {
                    "minecraft:block.copper_bulb.turn_off"
                };
                self.emit(LevelEvent::Sound {
                    pos,
                    sound: sound.into(),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
            self.set_block(pos, b.table.set_bool(next, p.powered, signal), 3);
        }
    }
    /// `NoteBlock.playNote`: queues the block event and emits the game event.
    pub(crate) fn note_play(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let instrument = b.table.text(state, b.props.instrument);
        let mob_head = super::data::INSTRUMENTS
            .iter()
            .position(|n| *n == instrument)
            .is_some_and(|i| super::data::instrument_works_above(i as u8));
        if mob_head || b.is_air(self.above_state(pos)) {
            self.block_event(pos, b.block(state), 0, 0);
            self.game_event(pos, "minecraft:note_block_play");
        }
    }
    /// `NoteBlock.triggerEvent`: the note's particle (tunable instruments)
    /// and sound.
    pub(crate) fn note_trigger_event(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let instrument = b.table.text(state, b.props.instrument).to_owned();
        let mob_head = super::data::INSTRUMENTS
            .iter()
            .position(|n| *n == instrument)
            .is_some_and(|i| super::data::instrument_works_above(i as u8));
        let note = b.table.int(state, b.props.note);
        let pitch = if mob_head {
            1.0
        } else {
            // `isTunable`: the client adds the NOTE particle above the block.
            self.emit(LevelEvent::BlockEvent {
                pos,
                state,
                kind: "note_particle",
            });
            horizon_core::jmath::pow(2.0, f64::from(note - 12) / 12.0) as f32
        };
        // NoteBlock.getCustomSoundId: the sound saved in the skull above is
        // read by the client; no skull or no sound means no sound.
        if instrument == "custom_head" {
            self.emit(LevelEvent::BlockEvent {
                pos,
                state,
                kind: "note_custom_sound",
            });
            return;
        }
        let sound = match instrument.as_str() {
            "dragon" => "minecraft:block.note_block.imitate.ender_dragon".to_owned(),
            name if mob_head => format!("minecraft:block.note_block.imitate.{name}"),
            name => format!("minecraft:block.note_block.{name}"),
        };
        self.emit(LevelEvent::Sound {
            pos,
            sound: sound.into(),
            volume: 3.0,
            pitch,
        });
    }
    pub(crate) fn note_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let signal = self.has_neighbor_signal(pos);
        if signal != b.table.bool(state, b.props.powered) {
            if signal {
                self.note_play(pos, state);
            }
            self.set_block_and_update(pos, b.table.set_bool(state, b.props.powered, signal));
        }
    }
    /// `TntBlock.prime` then `removeBlock`.
    pub(crate) fn tnt_check(&mut self, pos: BlockPos) {
        // TntBlock.prime succeeds only on a ServerLevel. A client's piston
        // replay must leave priming and block removal to the server.
        if !self.level.is_client_side() && self.has_neighbor_signal(pos) {
            self.prime_tnt(pos);
            self.remove_block(pos, false);
        }
    }
    pub(crate) fn dispenser_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let trigger = self.has_neighbor_signal(pos) || self.has_neighbor_signal(above(pos));
        let triggered = b.table.bool(state, b.props.triggered);
        if trigger && !triggered {
            self.schedule_tick(pos, b.block(state), 4);
            self.set_block(pos, b.table.set_bool(state, b.props.triggered, true), 2);
        } else if !trigger && triggered {
            self.set_block(pos, b.table.set_bool(state, b.props.triggered, false), 2);
        }
    }
    pub(crate) fn hopper_check_powered(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let enabled = !self.has_neighbor_signal(pos);
        if enabled != b.table.bool(state, b.props.enabled) {
            self.set_block(pos, b.table.set_bool(state, b.props.enabled, enabled), 2);
        }
    }
    pub(crate) fn skull_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let signal = self.has_neighbor_signal(pos);
        if signal != b.table.bool(state, b.props.powered) {
            self.set_block(pos, b.table.set_bool(state, b.props.powered, signal), 2);
        }
    }
    pub(crate) fn bell_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let signal = self.has_neighbor_signal(pos);
        if signal != b.table.bool(state, b.props.powered) {
            if signal {
                self.bell_ring(pos, state, b.facing(state));
            }
            self.set_block_and_update(pos, b.table.set_bool(state, b.props.powered, signal));
        }
    }
    /// `BellBlock.onHit` with `requireHitFromCorrectSide`: a hit on `face` at
    /// height `click_y` in the block rings the bell only when it struck the
    /// bell proper. False when the hit missed it.
    pub(crate) fn bell_hit(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        face: Direction,
        click_y: f64,
    ) -> bool {
        let b = self.b;
        let facing_axis = axis_of(b.facing(state));
        let proper = axis_of(face) != Axis::Y
            && click_y <= f64::from(0.8124f32)
            && match b.table.text(state, b.props.attachment) {
                "floor" => facing_axis == axis_of(face),
                "single_wall" | "double_wall" => facing_axis != axis_of(face),
                "ceiling" => true,
                _ => false,
            };
        if proper {
            self.bell_ring(pos, state, face);
        }
        proper
    }
    /// `BellBlock.attemptToRing`: `BellBlockEntity.onHit` queues block event 1
    /// with the struck direction, then the sound and game event.
    pub(crate) fn bell_ring(&mut self, pos: BlockPos, state: BlockStateId, direction: Direction) {
        self.block_event(pos, self.b.block(state), 1, direction as i32);
        self.emit(LevelEvent::Sound {
            pos,
            sound: "minecraft:block.bell.use".into(),
            volume: 2.0,
            pitch: 1.0,
        });
        self.game_event(pos, "minecraft:block_change");
    }
    pub(crate) fn crafter_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let trigger = self.has_neighbor_signal(pos);
        let triggered = b.table.bool(state, b.props.triggered);
        if trigger && !triggered {
            self.schedule_tick(pos, b.block(state), 4);
            self.set_block(pos, b.table.set_bool(state, b.props.triggered, true), 2);
        } else if !trigger && triggered {
            let next = b.table.set_bool(
                b.table.set_bool(state, b.props.triggered, false),
                b.props.crafting,
                false,
            );
            self.set_block(pos, next, 2);
        }
    }
    /// `TargetBlock.onProjectileHit` for a projectile that hit `face` of the
    /// block at `pos` at `location`: the output power from how near the
    /// face's centre it landed (1 to 15), reset after 20 ticks for an arrow
    /// and 8 for anything else, unless a reset is already scheduled. None
    /// when the block is not a target block, else the strength.
    pub(crate) fn target_projectile_hit(
        &mut self,
        pos: BlockPos,
        location: [f64; 3],
        face: [i32; 3],
        arrow: bool,
    ) -> Option<i32> {
        let b = self.b;
        let state = self.state(pos);
        if !b.data(state).is(Class::Target) {
            return None;
        }
        // `getRedstoneStrength`: the distance from the centre along the
        // face's two other axes (Mth.frac of the location, minus a half).
        let off = |v: f64| (v - v.floor() - 0.5).abs();
        let [dx, dy, dz] = [off(location[0]), off(location[1]), off(location[2])];
        let distance = if face[1] != 0 {
            dx.max(dz)
        } else if face[2] != 0 {
            dx.max(dy)
        } else {
            dy.max(dz)
        };
        let strength = ((15.0 * ((0.5 - distance) / 0.5).clamp(0.0, 1.0)).ceil() as i32).max(1);
        let block = b.block(state);
        if !self.ticks.has_block_tick(pos, block) {
            // `setOutputPower`.
            self.set_block_and_update(pos, b.table.set_int(state, b.props.power, strength));
            self.schedule_tick(pos, block, if arrow { 20 } else { 8 });
        }
        Some(strength)
    }
}

/// Order `positions` as a `java.util.HashSet<BlockPos>` of capacity 16 would
/// iterate them (bucket index of the spread hash, then insertion order).
fn java_hash_set_order(positions: &mut Vec<BlockPos>) {
    fn hash(pos: BlockPos) -> i32 {
        // Vec3i.hashCode: (y + z * 31) * 31 + x
        (pos.y.wrapping_add(pos.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(pos.x)
    }
    let mut unique: Vec<BlockPos> = Vec::new();
    for pos in positions.iter() {
        if !unique.contains(pos) {
            unique.push(*pos);
        }
    }
    let buckets = 16;
    let bucket = |pos: BlockPos| {
        let h = hash(pos);
        ((h ^ ((h as u32) >> 16) as i32) & (buckets - 1)) as usize
    };
    let mut ordered: Vec<(usize, usize, BlockPos)> = unique
        .iter()
        .enumerate()
        .map(|(i, p)| (bucket(*p), i, *p))
        .collect();
    ordered.sort_by_key(|(b, i, _)| (*b, *i));
    *positions = ordered.into_iter().map(|(_, _, p)| p).collect();
}
