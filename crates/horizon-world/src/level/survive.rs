//! `canSurvive` and `VegetationBlock.mayPlaceOn` for every overriding class.
//! Rules mirror the natively-validated `block_survival` port; dispatch follows
//! the Java class chain so `super.canSurvive` reaches the next override.
use super::{
    LevelAccess, Sim,
    classes::Class,
    query::is_horizontal,
    update::{HORIZONTAL, above, below, rel},
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `BlockState.canSurvive(level, pos)`.
    pub fn can_survive(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let mut survives = None;
        self.level.before_can_survive(pos, state, &mut survives);
        survives.unwrap_or_else(|| self.can_survive_at(0, state, pos))
    }
    fn can_survive_at(&self, depth: usize, state: BlockStateId, pos: BlockPos) -> bool {
        let chain = &self.b.data(state).chain;
        for (index, class) in chain.iter().enumerate().skip(depth) {
            if let Some(result) = self.can_survive_class(*class, index, state, pos) {
                return result;
            }
        }
        true
    }
    fn can_survive_class(
        &self,
        class: Class,
        depth: usize,
        state: BlockStateId,
        pos: BlockPos,
    ) -> Option<bool> {
        let b = self.b;
        let p = &b.props;
        let t = &b.tags;
        let n = &b.named;
        let block = b.block(state);
        Some(match class {
            Class::AmethystCluster | Class::ShelfMushroom | Class::BaseCoralWallFan => {
                let facing = b.facing(state);
                self.sturdy(rel(pos, facing.opposite()), facing)
            }
            Class::BambooSapling | Class::BambooStalk => {
                b.in_set(self.below_state(pos), &t.supports_bamboo)
            }
            Class::Banner | Class::Cake | Class::CandleCake | Class::StandingSign => {
                b.is_solid(self.below_state(pos))
            }
            Class::BaseCoralPlantType | Class::LeafLitter => self.sturdy(below(pos), Direction::Up),
            Class::BasePressurePlate => {
                let below = below(pos);
                self.can_support_rigid_block(below) || self.can_support_center(below, Direction::Up)
            }
            Class::BaseRail => self.can_support_rigid_block(below(pos)),
            Class::BaseTorch | Class::Candle => self.can_support_center(below(pos), Direction::Up),
            Class::Bell => {
                let connection = self.bell_connected_direction(state).opposite();
                if connection == Direction::Up {
                    self.can_support_center(above(pos), Direction::Down)
                } else {
                    self.sturdy(rel(pos, connection), connection.opposite())
                }
            }
            Class::BigDripleaf => {
                let below = self.below_state(pos);
                b.is_block(below, block)
                    || b.is_block(below, n.big_dripleaf_stem)
                    || b.in_set(below, &t.supports_big_dripleaf)
            }
            Class::BigDripleafStem => {
                let below = self.below_state(pos);
                let above = self.above_state(pos);
                (b.is_block(below, block) || b.in_set(below, &t.supports_big_dripleaf))
                    && (b.is_block(above, block) || b.is_block(above, n.big_dripleaf))
            }
            Class::BubbleColumn => {
                let below = self.below_state(pos);
                b.is_block(below, block)
                    || b.in_set(below, &t.enables_bubble_column_push_up)
                    || b.in_set(below, &t.enables_bubble_column_drag_down)
            }
            Class::Cactus => {
                for direction in HORIZONTAL {
                    let at = rel(pos, direction);
                    if b.is_solid(self.state(at)) || b.is_lava_fluid(self.fluid_at(at).kind()) {
                        return Some(false);
                    }
                }
                let below = self.below_state(pos);
                (b.is_block(below, block) || b.in_set(below, &t.supports_cactus))
                    && !b.liquid(self.above_state(pos))
            }
            Class::Carpet => !self.is_empty_block(below(pos)),
            Class::CeilingHangingSign => {
                b.is_face_sturdy_center(self.above_state(pos), Direction::Down)
            }
            Class::ChorusFlower => {
                let below = self.below_state(pos);
                if b.is_block(below, n.chorus_plant) || b.in_set(below, &t.supports_chorus_flower) {
                    return Some(true);
                }
                if !b.is_air(below) {
                    return Some(false);
                }
                let mut one = false;
                for direction in HORIZONTAL {
                    let neighbour = self.rel_state(pos, direction);
                    if b.is_block(neighbour, n.chorus_plant) {
                        if one {
                            return Some(false);
                        }
                        one = true;
                    } else if !b.is_air(neighbour) {
                        return Some(false);
                    }
                }
                one
            }
            Class::ChorusPlant => {
                let below_state = self.below_state(pos);
                let vertical = !b.is_air(self.above_state(pos)) && !b.is_air(below_state);
                for direction in HORIZONTAL {
                    let neighbour_pos = rel(pos, direction);
                    if b.is_block(self.state(neighbour_pos), block) {
                        if vertical {
                            return Some(false);
                        }
                        let under = self.below_state(neighbour_pos);
                        if b.is_block(under, block) || b.in_set(under, &t.supports_chorus_plant) {
                            return Some(true);
                        }
                    }
                }
                b.is_block(below_state, block) || b.in_set(below_state, &t.supports_chorus_plant)
            }
            Class::Cocoa => b.in_set(self.rel_state(pos, b.facing(state)), &t.supports_cocoa),
            Class::Crop => {
                self.raw_brightness(pos, 0) >= 8 && self.can_survive_at(depth + 1, state, pos)
            }
            Class::Diode => b.is_face_sturdy_rigid(self.below_state(pos), Direction::Up),
            Class::Door => {
                let below_state = self.below_state(pos);
                if b.table.text(state, p.half) == "lower" {
                    b.is_face_sturdy(below_state, Direction::Up)
                } else {
                    b.is_block(below_state, block)
                }
            }
            Class::DoublePlant => {
                if b.table.text(state, p.half) != "upper" {
                    return Some(self.can_survive_at(depth + 1, state, pos));
                }
                let below_state = self.below_state(pos);
                b.is_block(below_state, block) && b.table.text(below_state, p.half) == "lower"
            }
            Class::SmallDripleaf => {
                if b.table.text(state, p.half) == "upper" {
                    return Some(self.can_survive_at(depth + 1, state, pos));
                }
                let below = below(pos);
                self.may_place_on(state, self.state(below), below)
            }
            Class::TallSeagrass => {
                if b.table.text(state, p.half) == "upper" {
                    let below_state = self.below_state(pos);
                    return Some(
                        b.is_block(below_state, block)
                            && b.table.text(below_state, p.half) == "lower",
                    );
                }
                let fluid = self.fluid_at(pos);
                self.can_survive_at(depth + 1, state, pos)
                    && b.is_water_fluid(fluid.kind())
                    && fluid.is_full()
            }
            Class::PitcherCrop => {
                let lower =
                    b.is_block(state, n.pitcher_crop) && b.table.text(state, p.half) == "lower";
                if lower && self.raw_brightness(pos, 0) < 8 {
                    false
                } else {
                    self.can_survive_at(depth + 1, state, pos)
                }
            }
            Class::FaceAttachedHorizontalDirectional => {
                let direction = self.face_attached_connected(state).opposite();
                self.sturdy(rel(pos, direction), direction.opposite())
            }
            Class::Grindstone => true,
            Class::PistonHead => self.piston_head_can_survive(state, pos),
            Class::Farmland => {
                let above = self.above_state(pos);
                !b.is_solid(above) || b.in_set(above, &t.maintains_farmland)
            }
            Class::Fire => {
                self.sturdy(below(pos), Direction::Up) || self.is_valid_fire_location(pos)
            }
            Class::SoulFire => b.in_set(self.below_state(pos), &t.soul_fire_base_blocks),
            Class::Frogspawn => self.frogspawn_may_place_on(below(pos)),
            Class::GrowingPlant => {
                let data = b.data(state);
                let growth = data.growth_direction;
                let attached_pos = rel(pos, growth.opposite());
                let attached = self.state(attached_pos);
                if !self.growing_plant_can_attach_to(state, attached) {
                    return Some(false);
                }
                let (head, body) = self.growing_plant_pair(block);
                b.is_block(attached, head)
                    || b.is_block(attached, body)
                    || b.is_face_sturdy(attached, growth)
            }
            Class::HangingMoss => {
                let neighbour = self.above_state(pos);
                b.multiface_can_attach(neighbour, Direction::Up) || b.is_block(neighbour, block)
            }
            Class::HangingRoots => self.sturdy(above(pos), Direction::Down),
            Class::Ladder => {
                let facing = b.facing(state);
                self.sturdy(rel(pos, facing.opposite()), facing)
            }
            Class::Lantern => {
                let direction = if b.table.bool(state, p.hanging) {
                    Direction::Up
                } else {
                    Direction::Down
                };
                self.can_support_center(rel(pos, direction), direction.opposite())
            }
            Class::MangrovePropagule => {
                if b.table.bool(state, p.hanging) {
                    b.in_set(
                        self.above_state(pos),
                        &t.supports_hanging_mangrove_propagule,
                    )
                } else {
                    self.can_survive_at(depth + 1, state, pos)
                }
            }
            Class::MossyCarpet => {
                let below_state = self.below_state(pos);
                if b.table.bool(state, p.bottom) {
                    !b.is_air(below_state)
                } else {
                    b.is_block(below_state, block) && b.table.bool(below_state, p.bottom)
                }
            }
            Class::Multiface => {
                let mut any = false;
                for direction in Direction::ALL {
                    if self.multiface_has_face(state, direction) {
                        if !b.multiface_can_attach(self.rel_state(pos, direction), direction) {
                            return Some(false);
                        }
                        any = true;
                    }
                }
                any
            }
            Class::Mushroom => {
                let below_pos = below(pos);
                let below_state = self.state(below_pos);
                if b.in_set(below_state, &t.overrides_mushroom_light_requirement) {
                    true
                } else {
                    self.raw_brightness(pos, 0) < 13
                        && self.may_place_on(state, below_state, below_pos)
                }
            }
            Class::Path => {
                let above = self.above_state(pos);
                !b.is_solid(above) || b.instance_of(above, Class::FenceGate)
            }
            Class::RedstoneWallTorch | Class::WallTorch => {
                let facing = b.facing(state);
                self.sturdy(rel(pos, facing.opposite()), facing)
            }
            Class::RedstoneWire => {
                // `RedstoneWireBlock.canSurvive`'s `canSurviveOn`, through
                // `support_state` so a retracting piston's moving base still
                // holds its dust (#4728).
                let below = below(pos);
                self.sturdy(below, Direction::Up) || b.is_block(self.state(below), n.hopper)
            }
            Class::Scaffolding => self.scaffolding_distance(pos) < 7,
            Class::SeaPickle => {
                let below = below(pos);
                self.may_place_on(state, self.state(below), below)
            }
            Class::SnowLayer => {
                let below_state = self.below_state(pos);
                if b.in_set(below_state, &t.cannot_support_snow_layer) {
                    false
                } else if b.in_set(below_state, &t.support_override_snow_layer) {
                    true
                } else {
                    self.shape_info_at(below_state, below(pos)).full_faces
                        & (1 << Direction::Up as u8)
                        != 0
                        || (b.is_block(below_state, block)
                            && b.table.int(below_state, p.layers) == 8)
                }
            }
            Class::Speleothem => {
                let tip = b
                    .table
                    .dir(state, p.vertical_direction)
                    .unwrap_or(Direction::Up);
                self.is_valid_speleothem_placement(block, pos, tip)
            }
            Class::SporeBlossom => {
                self.can_support_center(above(pos), Direction::Down) && !self.is_water_at(pos)
            }
            Class::SugarCane => {
                let below_state = self.below_state(pos);
                if b.is_block(below_state, block) {
                    return Some(true);
                }
                if b.in_set(below_state, &t.supports_sugar_cane) {
                    let below = below(pos);
                    for direction in HORIZONTAL {
                        let at = rel(below, direction);
                        let fluid = self.fluid_at(at);
                        if b.fluid_in(fluid.kind(), b.fluid_tags.supports_sugar_cane_adjacently)
                            || b.in_set(self.state(at), &t.supports_sugar_cane_adjacently)
                        {
                            return Some(true);
                        }
                    }
                }
                false
            }
            Class::TripWireHook => {
                let facing = b.facing(state);
                is_horizontal(facing) && self.sturdy(rel(pos, facing.opposite()), facing)
            }
            Class::Vegetation => {
                let below = below(pos);
                self.may_place_on(state, self.state(below), below)
            }
            Class::Vine => {
                let updated = self.vine_updated_state(state, pos);
                self.vine_count_faces(updated) > 0
            }
            Class::WallBanner | Class::WallSign => {
                b.is_solid(self.rel_state(pos, b.facing(state).opposite()))
            }
            _ => return None,
        })
    }

    /// `VegetationBlock.mayPlaceOn` (virtual) for `state`'s block.
    pub fn may_place_on(
        &self,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
    ) -> bool {
        let mut may = None;
        self.level
            .before_may_place_on(state, ground, ground_pos, &mut may);
        if let Some(may) = may {
            return may;
        }
        let b = self.b;
        let t = &b.tags;
        let data = b.data(state);
        for class in &data.chain {
            let result = match class {
                Class::AttachedStem | Class::NetherFungus | Class::NetherRoots | Class::Stem => {
                    data.support_tag
                        .as_ref()
                        .is_some_and(|tag| b.in_set(ground, tag))
                }
                Class::Azalea => b.in_set(ground, &t.supports_azalea),
                Class::CactusFlower => {
                    let below = self.state(ground_pos);
                    b.in_set(below, &t.support_override_cactus_flower)
                        || b.is_face_sturdy_center(below, Direction::Up)
                }
                Class::Crop | Class::PitcherCrop => b.in_set(ground, &t.supports_crops),
                Class::DryVegetation => b.in_set(ground, &t.supports_dry_vegetation),
                Class::LilyPad => {
                    let fluid = self.fluid_at(ground_pos);
                    let above = self.fluid_at(above(ground_pos));
                    (b.fluid_in(fluid.kind(), b.fluid_tags.supports_lily_pad)
                        || b.in_set(ground, &t.supports_lily_pad))
                        && above.kind() == crate::fluid::FluidKind::Empty
                }
                Class::MangrovePropagule => b.in_set(ground, &t.supports_mangrove_propagule),
                Class::Mushroom => b.solid_render(ground),
                Class::NetherSprouts => b.in_set(ground, &t.supports_nether_sprouts),
                Class::NetherWart => b.in_set(ground, &t.supports_nether_wart),
                Class::SeaPickle => {
                    self.shape_info_at(ground, ground_pos).nonempty_faces
                        & (1 << Direction::Up as u8)
                        != 0
                        || b.is_face_sturdy(ground, Direction::Up)
                }
                Class::Seagrass | Class::TallSeagrass => {
                    b.is_face_sturdy(ground, Direction::Up)
                        && !b.in_set(ground, &t.cannot_support_seagrass)
                }
                Class::SmallDripleaf => {
                    b.in_set(ground, &t.supports_small_dripleaf)
                        || (self.fluid_at(above(ground_pos)) == crate::fluid::FluidState::WATER
                            && b.in_set(ground, &t.supports_vegetation))
                }
                Class::WitherRose => b.in_set(ground, &t.supports_wither_rose),
                Class::Vegetation => b.in_set(ground, &t.supports_vegetation),
                _ => continue,
            };
            return result;
        }
        false
    }

    fn frogspawn_may_place_on(&self, pos: BlockPos) -> bool {
        let b = self.b;
        let fluid = self.fluid_at(pos);
        let above = self.fluid_at(above(pos));
        (b.fluid_in(fluid.kind(), b.fluid_tags.supports_frogspawn)
            || b.in_set(self.state(pos), &b.tags.supports_frogspawn))
            && above.kind() == crate::fluid::FluidKind::Empty
    }

    pub(crate) fn bell_connected_direction(&self, state: BlockStateId) -> Direction {
        match self.b.table.text(state, self.b.props.attachment) {
            "floor" => Direction::Up,
            "ceiling" => Direction::Down,
            _ => self.b.facing(state).opposite(),
        }
    }
    /// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`.
    pub(crate) fn face_attached_connected(&self, state: BlockStateId) -> Direction {
        match self.b.table.text(state, self.b.props.face) {
            "ceiling" => Direction::Down,
            "floor" => Direction::Up,
            _ => self.b.facing(state),
        }
    }
    pub(crate) fn multiface_has_face(&self, state: BlockStateId, direction: Direction) -> bool {
        self.b.table.bool(state, self.direction_prop(direction))
    }
    pub(crate) fn direction_prop(&self, direction: Direction) -> super::states::PropKey {
        let p = &self.b.props;
        match direction {
            Direction::Down => p.down,
            Direction::Up => p.up,
            Direction::North => p.north,
            Direction::South => p.south,
            Direction::West => p.west,
            Direction::East => p.east,
        }
    }
    /// Head and body blocks of a growing-plant family.
    pub(crate) fn growing_plant_pair(
        &self,
        block: super::states::BlockId,
    ) -> (super::states::BlockId, super::states::BlockId) {
        let name = &*self.b.table.info(block).name;
        let head = name.strip_suffix("_plant").unwrap_or(name);
        let head = self.b.table.block_by_name(head).unwrap_or(block);
        let body = self
            .b
            .table
            .block_by_name(&format!("{}_plant", &*self.b.table.info(head).name))
            .unwrap_or(block);
        (head, body)
    }
    /// `GrowingPlantBlock.canAttachTo` (kelp refuses `cannot_support_kelp`).
    fn growing_plant_can_attach_to(&self, state: BlockStateId, attached: BlockStateId) -> bool {
        let b = self.b;
        if b.instance_of(state, Class::Kelp) || b.instance_of(state, Class::KelpPlant) {
            !b.in_set(attached, &b.tags.cannot_support_kelp)
        } else {
            true
        }
    }
    /// `FireBlock.isValidFireLocation`.
    pub(crate) fn is_valid_fire_location(&self, pos: BlockPos) -> bool {
        Direction::ALL
            .into_iter()
            .any(|direction| self.can_burn(self.rel_state(pos, direction)))
    }
    /// `FireBlock.canBurn`: ignite odds above zero (waterlogged states never burn).
    pub(crate) fn can_burn(&self, state: BlockStateId) -> bool {
        self.ignite_odds(state) > 0
    }
    pub(crate) fn ignite_odds(&self, state: BlockStateId) -> i32 {
        self.fire_odds(state).0
    }
    pub(crate) fn burn_odds(&self, state: BlockStateId) -> i32 {
        self.fire_odds(state).1
    }
    /// `FireBlock.getIgniteOdds` and `getBurnOdds`: the level's answer, else
    /// `FireBlock.bootStrap`'s table.
    fn fire_odds(&self, state: BlockStateId) -> (i32, i32) {
        if self.b.is_waterlogged(state) {
            return (0, 0);
        }
        let mut odds = None;
        self.level.before_fire_odds(state, &mut odds);
        odds.unwrap_or_else(|| {
            let data = self.b.data(state);
            (data.ignite_odds, data.burn_odds)
        })
    }
    /// `CampfireBlock.isSmokeSource`: a hay bale, unless the level answers.
    pub(crate) fn is_smoke_source(&self, state: BlockStateId) -> bool {
        let mut smoke = None;
        self.level.before_smoke_source(state, &mut smoke);
        smoke.unwrap_or_else(|| {
            self.b
                .table
                .block_by_name("minecraft:hay_block")
                .is_some_and(|hay| self.b.is_block(state, hay))
        })
    }
    /// `ScaffoldingBlock.getDistance`.
    pub(crate) fn scaffolding_distance(&self, pos: BlockPos) -> i32 {
        let b = self.b;
        let below_pos = below(pos);
        let below_state = self.state(below_pos);
        let mut distance = 7;
        if b.is_block(below_state, b.named.scaffolding) {
            distance = b.table.int(below_state, b.props.distance);
        } else if b.is_face_sturdy(below_state, Direction::Up) {
            return 0;
        }
        for direction in HORIZONTAL {
            let neighbour = self.rel_state(pos, direction);
            if b.is_block(neighbour, b.named.scaffolding) {
                distance = distance.min(b.table.int(neighbour, b.props.distance) + 1);
                if distance == 1 {
                    break;
                }
            }
        }
        distance
    }
    /// `SpeleothemBlock.isValidSpeleothemPlacement`.
    pub(crate) fn is_valid_speleothem_placement(
        &self,
        block: super::states::BlockId,
        pos: BlockPos,
        tip: Direction,
    ) -> bool {
        let behind_pos = rel(pos, tip.opposite());
        let behind = self.state(behind_pos);
        self.b.is_face_sturdy(behind, tip)
            || (self.b.is_block(behind, block)
                && self.b.table.dir(behind, self.b.props.vertical_direction) == Some(tip))
    }
    /// `VineBlock.getUpdatedState`.
    pub(crate) fn vine_updated_state(
        &self,
        mut state: BlockStateId,
        pos: BlockPos,
    ) -> BlockStateId {
        let b = self.b;
        let p = &b.props;
        let above_pos = above(pos);
        if b.table.bool(state, p.up) {
            let accept = b.multiface_can_attach(self.state(above_pos), Direction::Down);
            state = b.table.set_bool(state, p.up, accept);
        }
        let mut above_state = None;
        for direction in HORIZONTAL {
            let key = self.direction_prop(direction);
            if b.table.bool(state, key) {
                let mut support = self.vine_can_support_at_face(state, pos, direction);
                if !support {
                    let above = *above_state.get_or_insert_with(|| self.state(above_pos));
                    support = b.is_block(above, b.block(state)) && b.table.bool(above, key);
                }
                state = b.table.set_bool(state, key, support);
            }
        }
        state
    }
    fn vine_can_support_at_face(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> bool {
        let b = self.b;
        if direction == Direction::Down {
            return false;
        }
        if b.multiface_can_attach(self.rel_state(pos, direction), direction) {
            return true;
        }
        if direction == Direction::Up {
            return false;
        }
        let above = self.above_state(pos);
        b.is_block(above, b.block(state)) && b.table.bool(above, self.direction_prop(direction))
    }
    pub(crate) fn vine_count_faces(&self, state: BlockStateId) -> usize {
        [
            Direction::Up,
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ]
        .into_iter()
        .filter(|d| self.b.table.bool(state, self.direction_prop(*d)))
        .count()
    }
}
