//! Rails (`BaseRailBlock`, `RailState`, powered/activator rails) and
//! tripwires (`TripWireBlock`, `TripWireHookBlock`).
use super::{
    LevelAccess, LevelEvent, Sim,
    classes::Class,
    states::BlockId,
    update::{above, below, rel},
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RailShape {
    NorthSouth,
    EastWest,
    AscendingEast,
    AscendingWest,
    AscendingNorth,
    AscendingSouth,
    SouthEast,
    SouthWest,
    NorthWest,
    NorthEast,
}
impl RailShape {
    fn parse(text: &str) -> Self {
        match text {
            "east_west" => Self::EastWest,
            "ascending_east" => Self::AscendingEast,
            "ascending_west" => Self::AscendingWest,
            "ascending_north" => Self::AscendingNorth,
            "ascending_south" => Self::AscendingSouth,
            "south_east" => Self::SouthEast,
            "south_west" => Self::SouthWest,
            "north_west" => Self::NorthWest,
            "north_east" => Self::NorthEast,
            _ => Self::NorthSouth,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::NorthSouth => "north_south",
            Self::EastWest => "east_west",
            Self::AscendingEast => "ascending_east",
            Self::AscendingWest => "ascending_west",
            Self::AscendingNorth => "ascending_north",
            Self::AscendingSouth => "ascending_south",
            Self::SouthEast => "south_east",
            Self::SouthWest => "south_west",
            Self::NorthWest => "north_west",
            Self::NorthEast => "north_east",
        }
    }
    fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
}

/// `RailState`.
struct Rail {
    pos: BlockPos,
    state: BlockStateId,
    straight: bool,
    connections: Vec<BlockPos>,
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub(crate) fn rail_shape(&self, state: BlockStateId) -> &str {
        self.b.table.text(state, self.b.props.shape)
    }
    fn is_rail(&self, state: BlockStateId) -> bool {
        self.b.in_set(state, &self.b.tags.rails) && self.b.instance_of(state, Class::BaseRail)
    }
    fn is_rail_at(&self, pos: BlockPos) -> bool {
        self.is_rail(self.state(pos))
    }
    fn rail_connections(pos: BlockPos, shape: RailShape) -> Vec<BlockPos> {
        let (n, s, w, e) = (
            rel(pos, Direction::North),
            rel(pos, Direction::South),
            rel(pos, Direction::West),
            rel(pos, Direction::East),
        );
        match shape {
            RailShape::NorthSouth => vec![n, s],
            RailShape::EastWest => vec![w, e],
            RailShape::AscendingEast => vec![w, above(e)],
            RailShape::AscendingWest => vec![above(w), e],
            RailShape::AscendingNorth => vec![above(n), s],
            RailShape::AscendingSouth => vec![n, above(s)],
            RailShape::SouthEast => vec![e, s],
            RailShape::SouthWest => vec![w, s],
            RailShape::NorthWest => vec![w, n],
            RailShape::NorthEast => vec![e, n],
        }
    }
    fn rail_new(&self, pos: BlockPos, state: BlockStateId) -> Rail {
        let shape = RailShape::parse(self.rail_shape(state));
        Rail {
            pos,
            state,
            straight: !self.b.data(state).is(Class::Rail),
            connections: Self::rail_connections(pos, shape),
        }
    }
    fn rail_get(&self, pos: BlockPos) -> Option<Rail> {
        for test in [pos, above(pos), below(pos)] {
            let state = self.state(test);
            if self.is_rail(state) {
                return Some(self.rail_new(test, state));
            }
        }
        None
    }
    fn rail_has_connection(rail: &Rail, pos: BlockPos) -> bool {
        rail.connections
            .iter()
            .any(|c| c.x == pos.x && c.z == pos.z)
    }
    fn rail_remove_soft_connections(&self, rail: &mut Rail) {
        let mut i = 0;
        while i < rail.connections.len() {
            match self.rail_get(rail.connections[i]) {
                Some(other) if Self::rail_has_connection(&other, rail.pos) => {
                    rail.connections[i] = other.pos;
                    i += 1;
                }
                _ => {
                    rail.connections.remove(i);
                }
            }
        }
    }
    fn rail_can_connect_to(rail: &Rail, other: &Rail) -> bool {
        Self::rail_has_connection(rail, other.pos) || rail.connections.len() != 2
    }
    fn rail_has_neighbor_rail(&self, rail: &Rail, pos: BlockPos) -> bool {
        match self.rail_get(pos) {
            Some(mut neighbour) => {
                self.rail_remove_soft_connections(&mut neighbour);
                Self::rail_can_connect_to(&neighbour, rail)
            }
            None => false,
        }
    }
    fn rail_count_potential_connections(&self, pos: BlockPos) -> i32 {
        super::update::HORIZONTAL
            .into_iter()
            .filter(|d| {
                let p = rel(pos, *d);
                self.is_rail_at(p) || self.is_rail_at(above(p)) || self.is_rail_at(below(p))
            })
            .count() as i32
    }
    fn rail_slopes(&self, pos: BlockPos, shape: Option<RailShape>) -> Option<RailShape> {
        let (n, s, w, e) = (
            rel(pos, Direction::North),
            rel(pos, Direction::South),
            rel(pos, Direction::West),
            rel(pos, Direction::East),
        );
        let mut shape = shape;
        if shape == Some(RailShape::NorthSouth) {
            if self.is_rail_at(above(n)) {
                shape = Some(RailShape::AscendingNorth);
            }
            if self.is_rail_at(above(s)) {
                shape = Some(RailShape::AscendingSouth);
            }
        }
        if shape == Some(RailShape::EastWest) {
            if self.is_rail_at(above(e)) {
                shape = Some(RailShape::AscendingEast);
            }
            if self.is_rail_at(above(w)) {
                shape = Some(RailShape::AscendingWest);
            }
        }
        shape
    }
    fn rail_set_shape(&self, state: BlockStateId, shape: RailShape) -> BlockStateId {
        self.b
            .table
            .set_text(state, self.b.props.shape, shape.name())
    }
    fn rail_connect_to(&mut self, rail: &mut Rail, other_pos: BlockPos) {
        rail.connections.push(other_pos);
        let pos = rail.pos;
        let n = Self::rail_has_connection(rail, rel(pos, Direction::North));
        let s = Self::rail_has_connection(rail, rel(pos, Direction::South));
        let w = Self::rail_has_connection(rail, rel(pos, Direction::West));
        let e = Self::rail_has_connection(rail, rel(pos, Direction::East));
        let mut shape = None;
        if n || s {
            shape = Some(RailShape::NorthSouth);
        }
        if w || e {
            shape = Some(RailShape::EastWest);
        }
        if !rail.straight {
            if s && e && !n && !w {
                shape = Some(RailShape::SouthEast);
            }
            if s && w && !n && !e {
                shape = Some(RailShape::SouthWest);
            }
            if n && w && !s && !e {
                shape = Some(RailShape::NorthWest);
            }
            if n && e && !s && !w {
                shape = Some(RailShape::NorthEast);
            }
        }
        let shape = self
            .rail_slopes(pos, shape)
            .unwrap_or(RailShape::NorthSouth);
        rail.state = self.rail_set_shape(rail.state, shape);
        self.set_block_and_update(pos, rail.state);
    }
    /// `RailState.place`; returns the resulting state.
    fn rail_place(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        has_signal: bool,
        first: bool,
        default: RailShape,
    ) -> BlockStateId {
        let mut rail = self.rail_new(pos, state);
        let n = self.rail_has_neighbor_rail(&rail, rel(pos, Direction::North));
        let s = self.rail_has_neighbor_rail(&rail, rel(pos, Direction::South));
        let w = self.rail_has_neighbor_rail(&rail, rel(pos, Direction::West));
        let e = self.rail_has_neighbor_rail(&rail, rel(pos, Direction::East));
        let mut shape = None;
        let ns = n || s;
        let we = w || e;
        if ns && !we {
            shape = Some(RailShape::NorthSouth);
        }
        if we && !ns {
            shape = Some(RailShape::EastWest);
        }
        let (se, sw, ne, nw) = (s && e, s && w, n && e, n && w);
        if !rail.straight {
            if se && !n && !w {
                shape = Some(RailShape::SouthEast);
            }
            if sw && !n && !e {
                shape = Some(RailShape::SouthWest);
            }
            if nw && !s && !e {
                shape = Some(RailShape::NorthWest);
            }
            if ne && !s && !w {
                shape = Some(RailShape::NorthEast);
            }
        }
        if shape.is_none() {
            if ns && we {
                shape = Some(default);
            } else if ns {
                shape = Some(RailShape::NorthSouth);
            } else if we {
                shape = Some(RailShape::EastWest);
            }
            if !rail.straight {
                if has_signal {
                    for (cond, s) in [
                        (se, RailShape::SouthEast),
                        (sw, RailShape::SouthWest),
                        (ne, RailShape::NorthEast),
                        (nw, RailShape::NorthWest),
                    ] {
                        if cond {
                            shape = Some(s);
                        }
                    }
                } else {
                    for (cond, s) in [
                        (nw, RailShape::NorthWest),
                        (ne, RailShape::NorthEast),
                        (sw, RailShape::SouthWest),
                        (se, RailShape::SouthEast),
                    ] {
                        if cond {
                            shape = Some(s);
                        }
                    }
                }
            }
        }
        let shape = self.rail_slopes(pos, shape).unwrap_or(default);
        rail.connections = Self::rail_connections(pos, shape);
        rail.state = self.rail_set_shape(rail.state, shape);
        if first || self.state(pos) != rail.state {
            self.set_block_and_update(pos, rail.state);
            for i in 0..rail.connections.len() {
                if let Some(mut neighbour) = self.rail_get(rail.connections[i]) {
                    self.rail_remove_soft_connections(&mut neighbour);
                    if Self::rail_can_connect_to(&neighbour, &rail) {
                        self.rail_connect_to(&mut neighbour, rail.pos);
                    }
                }
            }
        }
        rail.state
    }
    fn rail_update_dir(&mut self, pos: BlockPos, state: BlockStateId, first: bool) -> BlockStateId {
        let current = RailShape::parse(self.rail_shape(state));
        let signal = self.has_neighbor_signal(pos);
        self.rail_place(pos, state, signal, first, current)
    }
    /// `BaseRailBlock.updateState(state, level, pos, movedByPiston)`.
    pub(crate) fn rail_update_state(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        moved: bool,
    ) -> BlockStateId {
        let state = self.rail_update_dir(pos, state, true);
        if !self.b.data(state).is(Class::Rail) {
            let block = self.b.block(state);
            self.neighbor_changed_full(state, pos, block, moved);
        }
        state
    }
    pub(crate) fn rail_neighbor_changed(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        changed: BlockId,
        moved: bool,
    ) {
        let b = self.b;
        if !b.is_block(self.state(pos), b.block(state)) {
            return;
        }
        let shape = RailShape::parse(self.rail_shape(state));
        let support = |sim: &Self, p: BlockPos| sim.can_support_rigid_block(p);
        let remove = !support(self, below(pos))
            || match shape {
                RailShape::AscendingEast => !support(self, rel(pos, Direction::East)),
                RailShape::AscendingWest => !support(self, rel(pos, Direction::West)),
                RailShape::AscendingNorth => !support(self, rel(pos, Direction::North)),
                RailShape::AscendingSouth => !support(self, rel(pos, Direction::South)),
                _ => false,
            };
        if remove {
            self.drop_resources(state, pos);
            self.remove_block(pos, moved);
            return;
        }
        let data = b.data(state);
        if data.is(Class::PoweredRail) {
            self.powered_rail_update(pos, state);
        } else if data.is(Class::Rail) {
            let changed_default = b.default_of(changed);
            if self.is_signal_source_default(changed_default)
                && self.rail_count_potential_connections(pos) == 3
            {
                self.rail_update_dir(pos, state, false);
            }
        }
    }
    fn is_signal_source_default(&self, state: BlockStateId) -> bool {
        self.b
            .has_flag(state, super::data::state_flags::SIGNAL_SOURCE)
    }
    fn powered_rail_update(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let powered = b.table.bool(state, b.props.powered);
        let should = self.has_neighbor_signal(pos)
            || self.powered_rail_find(pos, state, true, 0)
            || self.powered_rail_find(pos, state, false, 0);
        if should != powered {
            self.set_block_and_update(pos, b.table.set_bool(state, b.props.powered, should));
            let block = b.block(state);
            self.update_neighbors_at(below(pos), block);
            if RailShape::parse(self.rail_shape(state)).is_slope() {
                self.update_neighbors_at(above(pos), block);
            }
        }
    }
    fn powered_rail_find(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        forward: bool,
        depth: i32,
    ) -> bool {
        if depth >= 8 {
            return false;
        }
        let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
        let mut check_below = true;
        let mut shape = RailShape::parse(self.rail_shape(state));
        match shape {
            RailShape::NorthSouth => z += if forward { 1 } else { -1 },
            RailShape::EastWest => x += if forward { -1 } else { 1 },
            RailShape::AscendingEast => {
                if forward {
                    x -= 1;
                } else {
                    x += 1;
                    y += 1;
                    check_below = false;
                }
                shape = RailShape::EastWest;
            }
            RailShape::AscendingWest => {
                if forward {
                    x -= 1;
                    y += 1;
                    check_below = false;
                } else {
                    x += 1;
                }
                shape = RailShape::EastWest;
            }
            RailShape::AscendingNorth => {
                if forward {
                    z += 1;
                } else {
                    z -= 1;
                    y += 1;
                    check_below = false;
                }
                shape = RailShape::NorthSouth;
            }
            RailShape::AscendingSouth => {
                if forward {
                    z += 1;
                    y += 1;
                    check_below = false;
                } else {
                    z -= 1;
                }
                shape = RailShape::NorthSouth;
            }
            _ => {}
        }
        let block = self.b.block(state);
        self.powered_rail_same_with_power(BlockPos::new(x, y, z), block, forward, depth, shape)
            || (check_below
                && self.powered_rail_same_with_power(
                    BlockPos::new(x, y - 1, z),
                    block,
                    forward,
                    depth,
                    shape,
                ))
    }
    fn powered_rail_same_with_power(
        &self,
        pos: BlockPos,
        block: BlockId,
        forward: bool,
        depth: i32,
        dir: RailShape,
    ) -> bool {
        let b = self.b;
        let state = self.state(pos);
        if !b.is_block(state, block) {
            return false;
        }
        let mine = RailShape::parse(self.rail_shape(state));
        if dir == RailShape::EastWest
            && matches!(
                mine,
                RailShape::NorthSouth | RailShape::AscendingNorth | RailShape::AscendingSouth
            )
        {
            return false;
        }
        if dir == RailShape::NorthSouth
            && matches!(
                mine,
                RailShape::EastWest | RailShape::AscendingEast | RailShape::AscendingWest
            )
        {
            return false;
        }
        if !b.table.bool(state, b.props.powered) {
            return false;
        }
        self.has_neighbor_signal(pos) || self.powered_rail_find(pos, state, forward, depth + 1)
    }

    // ---- tripwire ----

    /// `TripWireBlock.updateSource`.
    pub(crate) fn tripwire_update_source(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        for direction in [Direction::South, Direction::West] {
            for i in 1..42 {
                let [dx, dy, dz] = direction.normal();
                let test = pos.offset(dx * i, dy * i, dz * i);
                let block = self.state(test);
                if b.is_block(block, b.named.tripwire_hook) {
                    if b.facing(block) == direction.opposite() {
                        self.tripwire_hook_calculate(test, block, false, true, i, Some(state));
                    }
                    break;
                }
                if !b.is_block(block, b.named.tripwire) {
                    break;
                }
            }
        }
    }
    /// `DetectorRailBlock.checkPressed`.
    pub(crate) fn detector_rail_check_pressed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if !self.can_survive(state, pos) || !b.is_block(self.state(pos), b.block(state)) {
            return;
        }
        let was = b.table.bool(state, b.props.powered);
        // getInteractingMinecartOfType: the 0.2-inset search box.
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        let area = Aabb::new(
            Vec3::new(x + 0.2, y, z + 0.2),
            Vec3::new(x + 0.8, y + 0.8, z + 0.8),
        );
        let pressed = self
            .level
            .count_entities(area, super::EntityQuery::Minecart)
            > 0;
        let block = b.block(state);
        if pressed != was {
            let next = b.table.set_bool(state, b.props.powered, pressed);
            self.set_block_and_update(pos, next);
            let rail = self.rail_new(pos, next);
            for connection in rail.connections {
                let at = self.state(connection);
                self.neighbor_changed_full(at, connection, b.block(at), false);
            }
            self.update_neighbors_at(pos, block);
            self.update_neighbors_at(below(pos), block);
        }
        if pressed {
            self.schedule_tick(pos, block, 20);
        }
        self.update_neighbour_for_output_signal(pos, block);
    }
    pub(crate) fn tripwire_tick(&mut self, pos: BlockPos) {
        let b = self.b;
        let state = self.state(pos);
        if b.table.bool(state, b.props.powered) {
            self.tripwire_check_pressed(pos);
        }
    }
    /// `TripWireBlock.checkPressed` with the entities currently inside.
    pub(crate) fn tripwire_check_pressed(&mut self, pos: BlockPos) {
        let b = self.b;
        let state = self.state(pos);
        let was = b.table.bool(state, b.props.powered);
        let top = if b.table.bool(state, b.props.attached) {
            2.5 / 16.0
        } else {
            8.0 / 16.0
        };
        let area = Aabb::new(
            Vec3::new(
                f64::from(pos.x),
                f64::from(pos.y) + 1.0 / 16.0,
                f64::from(pos.z),
            ),
            Vec3::new(
                f64::from(pos.x) + 1.0,
                f64::from(pos.y) + top,
                f64::from(pos.z) + 1.0,
            ),
        );
        let pressed = self
            .level
            .count_entities(area, super::EntityQuery::Everything)
            > 0;
        let block = b.block(state);
        if pressed != was {
            let next = b.table.set_bool(state, b.props.powered, pressed);
            self.set_block_and_update(pos, next);
            self.tripwire_update_source(pos, next);
        }
        if pressed {
            self.schedule_tick(pos, block, 10);
        } else if was {
            self.schedule_tick(pos, block, 1);
        }
    }
    fn tripwire_hook_notify(&mut self, block: BlockId, pos: BlockPos, direction: Direction) {
        let front = direction.opposite();
        let orientation = self.initial_orientation(Some(front), Some(Direction::Up));
        self.update_neighbors_at_oriented(pos, block, orientation);
        self.update_neighbors_at_oriented(rel(pos, front), block, orientation);
    }
    fn tripwire_emit(
        &mut self,
        pos: BlockPos,
        attached: bool,
        powered: bool,
        was_attached: bool,
        was_powered: bool,
    ) {
        let (sound, volume, pitch, event) = if powered && !was_powered {
            (
                "minecraft:block.tripwire.click_on",
                0.4,
                0.6,
                "minecraft:block_activate",
            )
        } else if !powered && was_powered {
            (
                "minecraft:block.tripwire.click_off",
                0.4,
                0.5,
                "minecraft:block_deactivate",
            )
        } else if attached && !was_attached {
            (
                "minecraft:block.tripwire.attach",
                0.4,
                0.7,
                "minecraft:block_attach",
            )
        } else if !attached && was_attached {
            let pitch = 1.2 / (self.random().next_float() * 0.2 + 0.9);
            (
                "minecraft:block.tripwire.detach",
                0.4,
                pitch,
                "minecraft:block_detach",
            )
        } else {
            return;
        };
        self.emit(LevelEvent::Sound {
            pos,
            sound: sound.into(),
            volume,
            pitch,
        });
        self.game_event(pos, event);
    }
    /// `TripWireHookBlock.calculateState`.
    pub(crate) fn tripwire_hook_calculate(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        destroyed: bool,
        can_update: bool,
        wire_source: i32,
        wire_source_state: Option<BlockStateId>,
    ) {
        let b = self.b;
        let p = b.props;
        let Some(direction) = b.table.dir(state, p.facing) else {
            return;
        };
        let was_attached = b.table.bool(state, p.attached);
        let was_powered = b.table.bool(state, p.powered);
        let block = b.block(state);
        let mut attached = !destroyed;
        let mut powered = false;
        let mut receiver = 0;
        let mut wires: Vec<Option<BlockStateId>> = vec![None; 42];
        let [dx, dy, dz] = direction.normal();
        for i in 1..42 {
            let test = pos.offset(dx * i, dy * i, dz * i);
            let mut wire = self.state(test);
            if b.is_block(wire, b.named.tripwire_hook) {
                if b.facing(wire) == direction.opposite() {
                    receiver = i;
                }
                break;
            }
            if !b.is_block(wire, b.named.tripwire) && i != wire_source {
                wires[i as usize] = None;
                attached = false;
            } else {
                if i == wire_source {
                    wire = wire_source_state.unwrap_or(wire);
                }
                let armed = !b.table.bool(wire, p.disarmed);
                let wire_powered = b.table.bool(wire, p.powered);
                powered |= armed && wire_powered;
                wires[i as usize] = Some(wire);
                if i == wire_source {
                    self.schedule_tick(pos, block, 10);
                    attached &= armed;
                }
            }
        }
        attached &= receiver > 1;
        powered &= attached;
        let new_state = b.table.set_bool(
            b.table.set_bool(b.default_of(block), p.attached, attached),
            p.powered,
            powered,
        );
        if receiver > 0 {
            let test = pos.offset(dx * receiver, dy * receiver, dz * receiver);
            let opposite = direction.opposite();
            self.set_block_and_update(test, b.table.set_dir(new_state, p.facing, opposite));
            self.tripwire_hook_notify(block, test, opposite);
            if !b.is_block(self.state(pos), b.named.tripwire_hook) {
                self.tripwire_hook_removed(pos, new_state);
                return;
            }
            self.tripwire_emit(test, attached, powered, was_attached, was_powered);
        }
        self.tripwire_emit(pos, attached, powered, was_attached, was_powered);
        if !destroyed {
            self.set_block_and_update(pos, b.table.set_dir(new_state, p.facing, direction));
            if can_update {
                self.tripwire_hook_notify(block, pos, direction);
            }
        }
        if was_attached != attached {
            for i in 1..receiver {
                let test = pos.offset(dx * i, dy * i, dz * i);
                if let Some(wire) = wires[i as usize] {
                    let current = self.state(test);
                    if b.is_block(current, b.named.tripwire)
                        || b.is_block(current, b.named.tripwire_hook)
                    {
                        let next = b
                            .table
                            .with_bool(wire, p.attached, attached)
                            .unwrap_or(wire);
                        self.set_block_and_update(test, next);
                    }
                }
            }
        }
    }
    /// `TripWireHookBlock.onRemoved`.
    pub(crate) fn tripwire_hook_removed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let attached = b.table.bool(state, b.props.attached);
        let powered = b.table.bool(state, b.props.powered);
        if attached || powered {
            self.tripwire_hook_calculate(pos, state, true, false, -1, None);
        }
        if powered {
            let facing = b.facing(state);
            self.tripwire_hook_notify(b.block(state), pos, facing);
        }
    }
}
