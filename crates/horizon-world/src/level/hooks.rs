//! `neighborChanged`, `onPlace`, `affectNeighborsAfterRemoval` and
//! `updateIndirectNeighbourShapes` for the overriding vanilla classes.
use super::{
    LevelAccess, LevelEvent, Sim,
    classes::Class,
    states::BlockId,
    update::{below, rel},
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `BlockState.handleNeighborChanged`.
    pub fn neighbor_changed(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        changed: BlockId,
        moved: bool,
    ) {
        let b = self.b;
        let block = b.block(state);
        let chain = &b.data(state).chain;
        for &class in chain.iter() {
            match class {
                Class::AbstractSkull => self.skull_neighbor_changed(pos, state),
                Class::BaseRail => self.rail_neighbor_changed(pos, state, changed, moved),
                Class::Bell => self.bell_neighbor_changed(pos, state),
                Class::BigDripleaf => {
                    if self.has_neighbor_signal(pos) {
                        self.big_dripleaf_reset_tilt(pos, state);
                    }
                }
                Class::Command => {
                    // CommandBlock.setPoweredAndUpdate
                    let signal = self.has_neighbor_signal(pos);
                    if let Some((was_powered, automatic)) =
                        self.level.set_command_block_powered(pos, signal)
                        && signal
                        && !was_powered
                        && !automatic
                        && !b.is_block(state, b.named.chain_command_block)
                    {
                        self.schedule_tick(pos, block, 1);
                    }
                }
                Class::CopperBulb => self.copper_bulb_check(pos, state),
                Class::Crafter => self.crafter_neighbor_changed(pos, state),
                Class::Diode => self.diode_neighbor_changed(pos, state),
                Class::Dispenser => self.dispenser_neighbor_changed(pos, state),
                Class::Door => self.door_neighbor_changed(pos, state, changed),
                Class::FenceGate => self.gate_neighbor_changed(pos, state),
                Class::FrostedIce => {
                    if b.block_data(changed).is(Class::FrostedIce)
                        && self.frosted_ice_fewer_neighbours_than(pos, 2)
                    {
                        self.frosted_ice_melt(pos, state);
                    }
                }
                Class::Hopper => self.hopper_check_powered(pos, state),
                Class::Liquid => self.liquid_block_changed(pos, state),
                Class::Note => self.note_neighbor_changed(pos, state),
                Class::PistonBase => self.piston_check_if_extend(pos, state),
                Class::PistonHead => {
                    if self.piston_head_can_survive(state, pos) {
                        let base = rel(pos, b.facing(state).opposite());
                        self.neighbor_changed_at(base, changed);
                    }
                }
                Class::RedstoneLamp => self.lamp_neighbor_changed(pos, state),
                Class::RedstoneTorch => self.torch_neighbor_changed(pos, state),
                Class::RedstoneWire => {
                    if self.can_survive(state, pos) {
                        self.wire_update_power(pos, state);
                    } else {
                        self.drop_resources(state, pos);
                        self.remove_block(pos, false);
                    }
                }
                Class::Shelf => self.shelf_neighbor_changed(pos, state),
                Class::Sponge => self.sponge_try_absorb(pos),
                Class::Tnt => self.tnt_check(pos),
                Class::TrapDoor => self.trapdoor_neighbor_changed(pos, state),
                _ => continue,
            }
            let _ = block;
            return;
        }
    }

    /// `BlockState.onPlace`.
    pub fn on_place(&mut self, state: BlockStateId, pos: BlockPos, old: BlockStateId, moved: bool) {
        let b = self.b;
        let block = b.block(state);
        let same = b.block(old) == block;
        let chain = &b.data(state).chain;
        for &class in chain.iter() {
            match class {
                Class::Fire => {
                    self.base_fire_on_place(pos, state, same);
                    let delay = 30 + self.random().next_int_bound(10).unwrap_or(0);
                    self.schedule_tick(pos, block, delay);
                }
                Class::BaseFire => self.base_fire_on_place(pos, state, same),
                Class::DetectorRail | Class::BaseRail => {
                    if !same {
                        let updated = self.rail_update_state(pos, state, moved);
                        if class == Class::DetectorRail {
                            self.detector_rail_check_pressed(pos, updated);
                        }
                    }
                }
                Class::Brushable => {
                    self.schedule_tick(pos, block, 2);
                }
                Class::Composter => {
                    if b.table.int(state, b.props.level) == 7 {
                        self.schedule_tick(pos, block, 20);
                    }
                }
                Class::CopperBulb => {
                    if !same {
                        self.copper_bulb_check(pos, state);
                    }
                }
                Class::CoralFan | Class::CoralPlant | Class::CoralWallFan => {
                    self.coral_try_schedule_die_tick(state, pos)
                }
                Class::Diode => self.diode_update_neighbors_in_front(pos, state),
                Class::FallingBlock => {
                    let delay = if b.data(state).is(Class::DragonEgg) {
                        5
                    } else {
                        2
                    };
                    self.schedule_tick(pos, block, delay);
                }
                Class::Frogspawn => {
                    let delay = self.random().next_int_between(3600, 12000).unwrap_or(3600);
                    self.schedule_tick(pos, block, delay);
                }
                Class::FrostedIce => {
                    let delay = self
                        .random()
                        .next_int_between_inclusive(60, 120)
                        .unwrap_or(60);
                    self.schedule_tick(pos, block, delay);
                }
                Class::Hopper => {
                    if !same {
                        self.hopper_check_powered(pos, state);
                    }
                }
                Class::LightningRod => {
                    if !same
                        && b.table.bool(state, b.props.powered)
                        && !self.ticks.has_block_tick(pos, block)
                    {
                        self.schedule_tick(pos, block, 8);
                    }
                }
                Class::Liquid => self.liquid_block_changed(pos, state),
                Class::PistonBase => {
                    if !same && self.moving_piston(pos).is_none() {
                        self.piston_check_if_extend(pos, state);
                    }
                }
                Class::Observer => {
                    if !same
                        && b.table.bool(state, b.props.powered)
                        && !self.ticks.has_block_tick(pos, block)
                    {
                        let next = b.table.set_bool(state, b.props.powered, false);
                        self.set_block(pos, next, 18);
                        self.observer_update_in_front(pos, next);
                    }
                }
                Class::RedstoneTorch => self.torch_notify_neighbors(pos, state),
                Class::RedstoneWire => {
                    if !same {
                        self.wire_update_power(pos, state);
                        for direction in [Direction::Up, Direction::Down] {
                            self.update_neighbors_at(rel(pos, direction), block);
                        }
                        self.wire_update_neighbors_of_neighboring_wires(pos);
                    }
                }
                Class::Scaffolding => {
                    self.schedule_tick(pos, block, 1);
                }
                Class::Shelf => self.shelf_on_place(pos, state, old),
                // CarvedPumpkinBlock.onPlace: a pumpkin placed by any means may
                // complete a golem; the caller builds it.
                Class::CarvedPumpkin if !same => self.emit(LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "golem",
                }),
                Class::SnifferEgg => {
                    let boosted = b.is_block(self.below_state(pos), b.named.moss_block);
                    if boosted {
                        self.level_event(3009, pos, 0);
                    }
                    self.game_event_state(pos, "minecraft:block_place", state);
                    let delay = if boosted { 12000 } else { 24000 } / 3;
                    let jitter = self.random().next_int_bound(300).unwrap_or(0);
                    self.schedule_tick(pos, block, delay + jitter);
                }
                Class::SculkSensor => {
                    if !same
                        && b.table.int(state, b.props.power) > 0
                        && !self.ticks.has_block_tick(pos, block)
                    {
                        self.set_block(pos, b.table.set_int(state, b.props.power, 0), 18);
                    }
                }
                Class::Sponge => {
                    if !same {
                        self.sponge_try_absorb(pos);
                    }
                }
                Class::WetSponge => {
                    if self.level.dimension() == super::Dimension::Nether {
                        self.set_block_and_update(pos, b.default_of(b.named.sponge));
                        self.level_event(2009, pos, 0);
                        let pitch = (1.0 + self.random().next_float() * 0.2) * 0.7;
                        self.emit(super::LevelEvent::Sound {
                            pos,
                            sound: "minecraft:block.wet_sponge.dries".into(),
                            volume: 1.0,
                            pitch,
                        });
                    }
                }
                Class::Target => {
                    if !same
                        && b.table.int(state, b.props.power) > 0
                        && !self.ticks.has_block_tick(pos, block)
                    {
                        self.set_block(pos, b.table.set_int(state, b.props.power, 0), 18);
                    }
                }
                Class::Tnt => {
                    if !same {
                        self.tnt_check(pos);
                    }
                }
                Class::TripWire => {
                    if !same {
                        self.tripwire_update_source(pos, state);
                    }
                }
                Class::TurtleEgg => {
                    if b.in_set(self.below_state(pos), &b.tags.sand) {
                        self.level_event(2012, pos, 15);
                    }
                }
                _ => continue,
            }
            return;
        }
    }

    /// `BlockState.affectNeighborsAfterRemoval`.
    pub fn affect_neighbors_after_removal(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        moved: bool,
    ) {
        let b = self.b;
        let block = b.block(state);
        let chain = &b.data(state).chain;
        for &class in chain.iter() {
            match class {
                Class::AbstractFurnace
                | Class::Barrel
                | Class::BrewingStand
                | Class::Chest
                | Class::ChiseledBookShelf
                | Class::Crafter
                | Class::CreakingHeart
                | Class::DecoratedPot
                | Class::Dispenser
                | Class::Hopper
                | Class::Jukebox
                | Class::ShulkerBox
                | Class::CopperGolemStatue => self.update_neighbour_for_output_signal(pos, block),
                Class::Shelf => {
                    self.update_neighbour_for_output_signal(pos, block);
                    self.shelf_after_powering_down(pos, state);
                }
                Class::BasePressurePlate => {
                    if !moved && self.plate_signal_for_state(state) > 0 {
                        self.plate_update_neighbours(pos, block);
                    }
                }
                Class::BaseRail => {
                    if !moved {
                        if self.rail_shape(state).starts_with("ascending") {
                            self.update_neighbors_at(super::update::above(pos), block);
                        }
                        if !b.data(state).is(Class::Rail) {
                            self.update_neighbors_at(pos, block);
                            self.update_neighbors_at(below(pos), block);
                        }
                    }
                }
                Class::Button | Class::Lever => {
                    if !moved && b.table.bool(state, b.props.powered) {
                        self.attached_update_neighbours(pos, state);
                    }
                }
                Class::LightningRod => {
                    if b.table.bool(state, b.props.powered) {
                        let facing = b.facing(state);
                        self.update_neighbors_at(rel(pos, facing.opposite()), block);
                    }
                }
                Class::PistonHead => {
                    let base_pos = rel(pos, b.facing(state).opposite());
                    let base = self.state(base_pos);
                    if self.piston_head_fitting_base(state, base) {
                        self.destroy_block(base_pos, true, super::flags::UPDATE_LIMIT);
                    }
                }
                Class::Lectern => {
                    if b.table.bool(state, b.props.powered) {
                        self.update_neighbors_at(below(pos), block);
                    }
                }
                Class::Diode => {
                    if !moved {
                        self.diode_update_neighbors_in_front(pos, state);
                    }
                }
                Class::Observer => {
                    if b.table.bool(state, b.props.powered) && self.ticks.has_block_tick(pos, block)
                    {
                        let off = b.table.set_bool(state, b.props.powered, false);
                        self.observer_update_in_front(pos, off);
                    }
                }
                Class::RedstoneTorch => {
                    if !moved {
                        self.torch_notify_neighbors(pos, state);
                    }
                }
                Class::RedstoneWire => {
                    if !moved {
                        for direction in Direction::ALL {
                            self.update_neighbors_at(rel(pos, direction), block);
                        }
                        self.wire_update_power(pos, state);
                        self.wire_update_neighbors_of_neighboring_wires(pos);
                    }
                }
                Class::SculkSensor => {
                    if b.table.text(state, b.table_phase_key()) == "active" {
                        self.update_neighbors_at(pos, block);
                        self.update_neighbors_at(below(pos), block);
                    }
                }
                Class::TripWire => {
                    if !moved {
                        let powered = b.table.set_bool(state, b.props.powered, true);
                        self.tripwire_update_source(pos, powered);
                    }
                }
                Class::TripWireHook => {
                    if !moved {
                        self.tripwire_hook_removed(pos, state);
                    }
                }
                _ => continue,
            }
            return;
        }
    }

    /// `BlockState.updateIndirectNeighbourShapes` (only redstone wire overrides it).
    pub fn update_indirect_neighbour_shapes(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        flags: i32,
        limit: i32,
    ) {
        if self.b.is_block(state, self.b.named.redstone_wire) {
            self.wire_update_indirect(state, pos, flags, limit);
        }
    }

    /// `ServerPlayerGameMode.destroyBlock`: `playerWillDestroy` then
    /// `removeBlock`. Drops are the caller's (tool-dependent loot).
    pub(crate) fn player_destroy(
        &mut self,
        pos: BlockPos,
        creative: bool,
        correct_tool: bool,
    ) -> bool {
        let b = self.b;
        let p = b.props;
        let state = self.state(pos);
        if b.is_air(state) {
            return false;
        }
        let data = b.data(state);
        // DoorBlock also clears the bottom half when the tool would not drop it.
        if ((creative && data.is(Class::DoublePlant))
            || (data.is(Class::Door) && (creative || !correct_tool)))
            && b.table.text(state, p.half) == "upper"
        {
            let bottom = below(pos);
            let bottom_state = self.state(bottom);
            if b.is_block(bottom_state, b.block(state))
                && b.table.text(bottom_state, p.half) == "lower"
            {
                let replacement = if b.is_water_fluid(b.fluid(bottom_state).kind()) {
                    b.default_of(b.named.water)
                } else {
                    b.air()
                };
                self.set_block(bottom, replacement, 35);
                self.level_event(2001, bottom, bottom_state.0 as i32);
            }
        }
        if creative && data.is(Class::AbstractBed) && b.table.text(state, p.part) == "foot" {
            let head_pos = rel(pos, b.facing(state));
            let head = self.state(head_pos);
            if b.is_block(head, b.block(state)) && b.table.text(head, p.part) == "head" {
                self.set_block(head_pos, b.air(), 35);
                self.level_event(2001, head_pos, head.0 as i32);
            }
        }
        // PistonHeadBlock.playerWillDestroy: in creative, destroy the base piston
        // without drops so removing the head does not drop a piston item.
        if creative && data.is(Class::PistonHead) {
            let base_pos = rel(pos, b.facing(state).opposite());
            let base = self.state(base_pos);
            if self.piston_head_fitting_base(state, base) {
                self.destroy_block(base_pos, false, super::flags::UPDATE_LIMIT);
            }
        }
        // TntBlock.playerWillDestroy: unstable TNT primes unless instabuild.
        if !creative && data.is(Class::Tnt) && b.table.bool(state, p.unstable) {
            self.prime_tnt(pos);
        }
        // BaseFireBlock emits extinguish and suppresses generic destroy particles/sound.
        if data.is(Class::BaseFire) {
            self.level_event(1009, pos, 0);
        } else {
            self.level_event(2001, pos, state.0 as i32);
        }
        self.game_event(pos, "minecraft:block_destroy");
        self.remove_block(pos, false)
    }
    /// `IceBlock.playerDestroy` (after the drops): mined ice turns into water
    /// when it sits over a solid block or a liquid, unless the tool prevents
    /// ice melting (silk touch). In the nether, where water evaporates, the
    /// block stays air.
    pub(crate) fn ice_destroyed_by_player(
        &mut self,
        pos: BlockPos,
        destroyed: BlockStateId,
        prevents_melting: bool,
    ) {
        let b = self.b;
        if !b.data(destroyed).is(Class::Ice)
            || prevents_melting
            || self.level.dimension() == super::Dimension::Nether
        {
            return;
        }
        let below = self.below_state(pos);
        if b.in_set(below, &b.tags.blocks_motion) || b.liquid(below) {
            let water = b.default_of(b.named.water);
            self.set_block_and_update(pos, water);
        }
    }
    /// `entityInside` for entity-sensing blocks.
    pub(crate) fn entity_inside(&mut self, pos: BlockPos) {
        let b = self.b;
        let state = self.state(pos);
        let data = b.data(state);
        if data.is(Class::BasePressurePlate) {
            let signal = self.plate_signal_for_state(state);
            if signal == 0 {
                self.plate_check_pressed(pos, state, signal);
            }
        } else if data.is(Class::DetectorRail) {
            if !b.table.bool(state, b.props.powered) {
                self.detector_rail_check_pressed(pos, state);
            }
        } else if data.is(Class::Button) {
            // ButtonBlock.entityInside: a wooden button an arrow is inside.
            if data.set_type.as_ref().is_some_and(|s| s.arrows)
                && !b.table.bool(state, b.props.powered)
            {
                self.button_check_pressed(pos, state);
            }
        } else if data.is(Class::TripWire)
            && !b.table.bool(state, b.props.powered)
            && !self.ticks.has_block_tick(pos, b.block(state))
        {
            self.tripwire_check_pressed(pos);
        }
    }

    // ---- misc helpers ----

    /// `BaseFireBlock.onPlace`: a fire inside an empty obsidian frame (in the
    /// overworld or the nether) lights it, otherwise one that cannot survive
    /// goes out.
    fn base_fire_on_place(&mut self, pos: BlockPos, state: BlockStateId, same: bool) {
        if same {
            return;
        }
        if matches!(
            self.level.dimension(),
            super::Dimension::Overworld | super::Dimension::Nether
        ) && let Some(shape) = self.find_empty_portal_shape(pos, true)
        {
            self.create_portal_blocks(&shape);
            return;
        }
        if !self.can_survive(state, pos) {
            self.remove_block(pos, false);
        }
    }
    /// `SpongeBlock.tryAbsorbWater`.
    pub(crate) fn sponge_try_absorb(&mut self, start: BlockPos) {
        if self.sponge_remove_water(start) {
            let b = self.b;
            self.set_block(start, b.default_of(b.named.wet_sponge), 2);
            self.emit(super::LevelEvent::Sound {
                pos: start,
                sound: "minecraft:block.sponge.absorb".into(),
                volume: 1.0,
                pitch: 1.0,
            });
        }
    }
    fn sponge_remove_water(&mut self, start: BlockPos) -> bool {
        use std::collections::{HashSet, VecDeque};
        let b = self.b;
        let mut queue = VecDeque::from([(start, 0)]);
        let mut visited = HashSet::new();
        let mut count = 0;
        while let Some((pos, depth)) = queue.pop_front() {
            if !visited.insert(pos) {
                continue;
            }
            let accept = if pos == start {
                true
            } else {
                let state = self.state(pos);
                if !b.is_water_fluid(self.fluid_at(pos).kind()) {
                    false
                } else if self.pickup_block(pos, state) {
                    true
                } else {
                    if b.instance_of(state, Class::Liquid) {
                        self.set_block_and_update(pos, b.air());
                    } else {
                        let n = b.named;
                        let block = b.block(state);
                        if block != n.kelp
                            && block != n.kelp_plant
                            && block != n.seagrass
                            && block != n.tall_seagrass
                        {
                            continue;
                        }
                        self.drop_resources(state, pos);
                        self.set_block_and_update(pos, b.air());
                    }
                    true
                }
            };
            if !accept {
                continue;
            }
            count += 1;
            if count >= 65 {
                return count > 1;
            }
            if depth < 6 {
                for direction in Direction::ALL {
                    queue.push_back((rel(pos, direction), depth + 1));
                }
            }
        }
        count > 1
    }
    fn frosted_ice_fewer_neighbours_than(&self, pos: BlockPos, limit: i32) -> bool {
        let mut count = 0;
        for direction in Direction::ALL {
            if self
                .b
                .data(self.rel_state(pos, direction))
                .is(Class::FrostedIce)
            {
                count += 1;
                if count >= limit {
                    return false;
                }
            }
        }
        true
    }
    /// `FrostedIceBlock.melt` → `IceBlock.melt`.
    pub(crate) fn frosted_ice_melt(&mut self, pos: BlockPos, state: BlockStateId) {
        self.ice_melt(pos, state);
    }
    /// `IceBlock.melt`.
    pub(crate) fn ice_melt(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        if self.level.dimension() == super::Dimension::Nether {
            self.remove_block(pos, false);
        } else {
            let water = b.default_of(b.named.water);
            self.set_block_and_update(pos, water);
            self.neighbor_changed_at(pos, b.named.water);
            self.game_event_state(pos, "minecraft:block_destroy", state);
        }
    }
    /// `BigDripleafBlock.entityInside` on a server level: an entity standing on
    /// a flat leaf (`canEntityTilt`: on the ground, its feet above the leaf's
    /// lowest top, 11/16, less 4/16: 0.6875) tilts it UNSTABLE unless the leaf
    /// is powered; `setTiltAndScheduleTick` with no sound (UNSTABLE causes no
    /// vibration) and the next state in 10 ticks.
    pub(crate) fn big_dripleaf_entity_inside(
        &mut self,
        pos: BlockPos,
        on_ground: bool,
        feet_y: f64,
    ) {
        let b = self.b;
        let state = self.state(pos);
        if !b.data(state).is(Class::BigDripleaf)
            || b.table.text(state, b.props.tilt) != "none"
            || !(on_ground && feet_y > f64::from(pos.y) + f64::from(0.6875_f32))
            || self.has_neighbor_signal(pos)
        {
            return;
        }
        self.set_block(pos, b.table.set_text(state, b.props.tilt, "unstable"), 2);
        self.schedule_tick(pos, b.block(state), 10);
    }
    /// `BigDripleafBlock.resetTilt`.
    pub(crate) fn big_dripleaf_reset_tilt(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let next = b.table.set_text(state, b.props.tilt, "none");
        self.set_block(pos, next, 2);
        if b.table.text(state, b.props.tilt) != "none" {
            // setTilt: NONE causes a vibration when the tilt changes.
            self.game_event(pos, "minecraft:block_change");
            let pitch = self.random().next_float() * 0.4 + 0.8;
            self.emit(super::LevelEvent::Sound {
                pos,
                sound: "minecraft:block.big_dripleaf.tilt_up".into(),
                volume: 1.0,
                pitch,
            });
        }
    }
}

impl super::Behaviours {
    pub(crate) fn table_phase_key(&self) -> super::states::PropKey {
        self.props.sculk_sensor_phase
    }
}
