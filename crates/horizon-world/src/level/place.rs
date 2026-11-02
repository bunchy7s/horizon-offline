//! Block placement: `BlockItem.place`, `BlockPlaceContext`,
//! `canBeReplaced(state, context)` and `getStateForPlacement` for every
//! overriding class.
use super::{
    Behaviours, LevelAccess, LevelEvent, Sim, Ticks,
    classes::Class,
    data::ItemKind,
    query::{axis_of, clockwise, counter_clockwise, is_horizontal},
    states::BlockId,
    update::{HORIZONTAL, above, below, rel},
};
use crate::{fluid::FluidState, registry::BlockStateId};
use horizon_core::{
    geometry::{Axis, Direction, Vec3, trig},
    position::BlockPos,
    random::RandomSource,
};

/// A player's use-item-on-block action (`UseOnContext` inputs).
#[derive(Clone, Debug)]
pub struct PlaceContext {
    /// The block that was hit (`BlockHitResult.getBlockPos`).
    pub clicked_pos: BlockPos,
    /// The face that was hit.
    pub face: Direction,
    /// Absolute hit location.
    pub hit: Vec3,
    /// `BlockHitResult.isInside`.
    pub inside: bool,
    /// Player view rotation in degrees (`getYRot`, `getXRot`).
    pub yaw: f32,
    pub pitch: f32,
    /// `isSecondaryUseActive` (sneaking).
    pub sneaking: bool,
    /// The item in hand (registry name, e.g. `minecraft:oak_stairs`).
    pub item: String,
    /// `Player.canUseGameMasterBlocks` (creative operator); gates command,
    /// structure, jigsaw and test blocks.
    pub can_use_game_master_blocks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceOutcome {
    /// The block was placed (the caller consumes the item).
    Placed { pos: BlockPos, state: BlockStateId },
    /// `InteractionResult.FAIL`.
    Failed,
}

/// `BlockPlaceContext` after resolving the replaced position: what
/// `Block.getStateForPlacement` reads, and what a mod block is handed when it
/// chooses the state it is placed in
/// (`LevelAccess::before_state_for_placement`).
#[derive(Clone, Debug)]
pub struct PlacementContext {
    /// Where the block would go (`getClickedPos`): the clicked block itself
    /// when it can be replaced, else the block beside it.
    pub pos: BlockPos,
    /// The block that was clicked (`getHitResult().getBlockPos`).
    pub hit_pos: BlockPos,
    /// The face that was clicked (`getClickedFace`).
    pub face: Direction,
    /// Where the click landed (`getClickLocation`).
    pub hit: Vec3,
    /// `BlockHitResult.isInside`: the click hit the inside of the clicked
    /// block's shape.
    pub inside: bool,
    /// The placer's rotation, in degrees (`getRotation`).
    pub yaw: f32,
    pub pitch: f32,
    /// `isSecondaryUseActive`: the player placing it is sneaking.
    pub sneaking: bool,
    /// The item in the placer's hand (`getItemInHand().getItem()`), as its
    /// registry name: `minecraft:oak_stairs`.
    pub item: String,
    /// Whether the clicked block is replaceable (`replacingClickedOnBlock`),
    /// i.e. whether the block goes into the clicked position.
    pub replace_clicked: bool,
}

/// `Direction.orderedByNearest(entity)`.
pub(crate) fn ordered_by_nearest(yaw: f32, pitch: f32) -> [Direction; 6] {
    let pitch_rad = pitch * (std::f32::consts::PI / 180.0);
    let yaw_rad = -yaw * (std::f32::consts::PI / 180.0);
    let pitch_sin = trig::sin(f64::from(pitch_rad));
    let pitch_cos = trig::cos(f64::from(pitch_rad));
    let yaw_sin = trig::sin(f64::from(yaw_rad));
    let yaw_cos = trig::cos(f64::from(yaw_rad));
    let x_pos = yaw_sin > 0.0;
    let y_pos = pitch_sin < 0.0;
    let z_pos = yaw_cos > 0.0;
    let x_yaw = if x_pos { yaw_sin } else { -yaw_sin };
    let y_mag = if y_pos { -pitch_sin } else { pitch_sin };
    let z_yaw = if z_pos { yaw_cos } else { -yaw_cos };
    let x_mag = x_yaw * pitch_cos;
    let z_mag = z_yaw * pitch_cos;
    let ax = if x_pos {
        Direction::East
    } else {
        Direction::West
    };
    let ay = if y_pos {
        Direction::Up
    } else {
        Direction::Down
    };
    let az = if z_pos {
        Direction::South
    } else {
        Direction::North
    };
    let make = |a: Direction, b: Direction, c: Direction| {
        [a, b, c, c.opposite(), b.opposite(), a.opposite()]
    };
    if x_yaw > z_yaw {
        if y_mag > x_mag {
            make(ay, ax, az)
        } else if z_mag > y_mag {
            make(ax, az, ay)
        } else {
            make(ax, ay, az)
        }
    } else if y_mag > z_mag {
        make(ay, az, ax)
    } else if x_mag > y_mag {
        make(az, ax, ay)
    } else {
        make(az, ay, ax)
    }
}

/// `Direction.fromYRot`.
pub(crate) fn from_y_rot(yaw: f64) -> Direction {
    let value = ((yaw / 90.0 + 0.5).floor() as i64 & 3) as i32;
    super::query::from_2d(value)
}

/// `RotationSegment.convertToSegment(float)`.
pub(crate) fn rotation_segment(degrees: f32) -> i32 {
    ((degrees * (16.0 / 360.0) + 0.5).floor() as i32) & 15
}
/// `RotationSegment.convertToSegment(Direction)`.
fn rotation_segment_of(direction: Direction) -> i32 {
    if !is_horizontal(direction) {
        0
    } else {
        super::query::data_2d(direction) << 2
    }
}

impl PlacementContext {
    fn horizontal_direction(&self) -> Direction {
        from_y_rot(f64::from(self.yaw))
    }
    fn nearest_looking_directions(&self) -> [Direction; 6] {
        let mut directions = ordered_by_nearest(self.yaw, self.pitch);
        if self.replace_clicked {
            return directions;
        }
        let target = self.face.opposite();
        if let Some(index) = directions.iter().position(|d| *d == target) {
            if index > 0 {
                directions.copy_within(0..index, 1);
                directions[0] = target;
            }
        }
        directions
    }
    fn nearest_looking_direction(&self) -> Direction {
        ordered_by_nearest(self.yaw, self.pitch)[0]
    }
    fn nearest_looking_vertical(&self) -> Direction {
        if self.pitch < 0.0 {
            Direction::Up
        } else {
            Direction::Down
        }
    }
    fn click_y(&self) -> f64 {
        self.hit.y - f64::from(self.pos.y)
    }
    /// `UseOnContext.getClickedPos` vs `BlockPlaceContext.getClickedPos`.
    fn half_from_click(&self) -> bool {
        // true = bottom
        self.face != Direction::Down && (self.face == Direction::Up || self.click_y() <= 0.5)
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn item_is(&self, ctx: &PlacementContext, block: BlockId) -> bool {
        !ctx.item.is_empty() && *self.b.block_data(block).item == *ctx.item
    }
    /// `BlockState.canBeReplaced(BlockPlaceContext)`.
    pub(crate) fn can_be_replaced(&self, state: BlockStateId, ctx: &PlacementContext) -> bool {
        let b = self.b;
        let p = b.props;
        let block = b.block(state);
        let data = b.data(state);
        let default = |sim: &Self| {
            b.is_replaceable(state) && (ctx.item.is_empty() || !sim.item_is(ctx, block))
        };
        for class in &data.chain {
            return match class {
                Class::Candle => {
                    (!ctx.sneaking && self.item_is(ctx, block) && b.table.int(state, p.candles) < 4)
                        || default(self)
                }
                Class::SeaPickle => {
                    (!ctx.sneaking && self.item_is(ctx, block) && b.table.int(state, p.pickles) < 4)
                        || default(self)
                }
                Class::TurtleEgg => {
                    (!ctx.sneaking && self.item_is(ctx, block) && b.table.int(state, p.eggs) < 4)
                        || default(self)
                }
                Class::FlowerBed => {
                    (!ctx.sneaking
                        && self.item_is(ctx, block)
                        && b.table.int(state, p.flower_amount) < 4)
                        || default(self)
                }
                Class::LeafLitter => {
                    (!ctx.sneaking
                        && self.item_is(ctx, block)
                        && b.table.int(state, p.segment_amount) < 4)
                        || default(self)
                }
                Class::GrowingPlantBody => {
                    let (head, _) = self.growing_plant_pair(block);
                    default(self) && !self.item_is(ctx, head)
                }
                Class::Multiface => {
                    !self.item_is(ctx, block)
                        || Direction::ALL
                            .into_iter()
                            .any(|d| !self.multiface_has_face(state, d))
                }
                Class::PitcherCrop => false,
                Class::Scaffolding => self.item_is(ctx, block),
                Class::Slab => {
                    let kind = b.table.text(state, p.kind);
                    if kind == "double" || !self.item_is(ctx, block) {
                        false
                    } else if ctx.replace_clicked {
                        let above = ctx.hit.y - f64::from(ctx.hit_pos.y) > 0.5;
                        if kind == "bottom" {
                            ctx.face == Direction::Up || (above && is_horizontal(ctx.face))
                        } else {
                            ctx.face == Direction::Down || (!above && is_horizontal(ctx.face))
                        }
                    } else {
                        true
                    }
                }
                Class::SnowLayer => {
                    let layers = b.table.int(state, p.layers);
                    if !self.item_is(ctx, block) || layers >= 8 {
                        layers == 1
                    } else if ctx.replace_clicked {
                        ctx.face == Direction::Up
                    } else {
                        true
                    }
                }
                Class::Vine => {
                    let clicked = self.state(ctx.pos);
                    if b.is_block(clicked, block) {
                        self.vine_count_faces(clicked) < 5
                    } else {
                        default(self)
                    }
                }
                _ => continue,
            };
        }
        default(self)
    }

    fn fluid_is_water_source(&self, pos: BlockPos) -> bool {
        self.fluid_at(pos) == FluidState::WATER || {
            let f = self.fluid_at(pos);
            f.kind() == crate::fluid::FluidKind::Water
        }
    }

    /// `Block.getStateForPlacement(context)` for `block`, after the level's
    /// hook (`LevelAccess::before_state_for_placement`), which a mod block
    /// answers for itself.
    pub(crate) fn state_for_placement(
        &mut self,
        block: BlockId,
        ctx: &PlacementContext,
    ) -> Option<BlockStateId> {
        let default = self.b.default_of(block);
        let fluid = self.fluid_at(ctx.pos);
        let mut answer = None;
        self.level
            .before_state_for_placement(default, ctx, fluid, &mut answer);
        match answer {
            // A mod's answer, for a state of the block being placed, as
            // vanilla's `getStateForPlacement` returns its own `defaultBlockState`
            // set out. Another block's answer, or an id past the state table
            // (which would panic the server), leaves vanilla's state as it is.
            Some(Some(new))
                if (new.0 as usize) < self.b.table.state_count() && self.b.block(new) == block =>
            {
                return Some(new);
            }
            // Vanilla's `null` return: the block goes nowhere.
            Some(None) => return None,
            _ => {}
        }
        self.placement_at(block, 0, ctx)
    }

    fn placement_at(
        &mut self,
        block: BlockId,
        depth: usize,
        ctx: &PlacementContext,
    ) -> Option<BlockStateId> {
        let b = self.b;
        let chain = &b.block_data(block).chain;
        for (index, class) in chain.iter().copied().enumerate().skip(depth) {
            if let Some(result) = self.placement_class(class, index, block, ctx) {
                return result;
            }
        }
        Some(self.b.default_of(block))
    }

    #[allow(clippy::option_option)]
    fn placement_class(
        &mut self,
        class: Class,
        depth: usize,
        block: BlockId,
        ctx: &PlacementContext,
    ) -> Option<Option<BlockStateId>> {
        let b = self.b;
        let p = b.props;
        let t = &b.tags;
        let n = b.named;
        let table = &b.table;
        let default = b.default_of(block);
        let pos = ctx.pos;
        let water_source = self.fluid_is_water_source(pos);
        let wl = |state: BlockStateId| table.set_bool(state, p.waterlogged, water_source);
        macro_rules! sup {
            () => {
                self.placement_at(block, depth + 1, ctx)
            };
        }
        let horizontal = ctx.horizontal_direction();
        Some(match class {
            Class::AbstractBed => {
                let relative = rel(pos, horizontal);
                if self.can_be_replaced(self.state(relative), ctx) {
                    Some(table.set_dir(default, p.facing, horizontal))
                } else {
                    None
                }
            }
            Class::AbstractFurnace
            | Class::Beehive
            | Class::CarvedPumpkin
            | Class::ChiseledBookShelf
            | Class::Diode
            | Class::GlazedTerracotta
            | Class::Loom
            | Class::Stonecutter
            | Class::Vault => Some(table.set_dir(default, p.facing, horizontal.opposite())),
            Class::Repeater => {
                let state = sup!()?;
                let locked = self.diode_is_locked(pos, state);
                Some(table.set_bool(state, p.locked, locked))
            }
            Class::EndPortalFrame => Some(table.set_bool(
                table.set_dir(default, p.facing, horizontal.opposite()),
                p.eye,
                false,
            )),
            Class::Lectern => Some(table.set_bool(
                table.set_dir(default, p.facing, horizontal.opposite()),
                p.has_book,
                false,
            )),
            Class::AbstractSkull => {
                let signal = self.has_neighbor_signal(pos);
                Some(table.set_bool(default, p.powered, signal))
            }
            Class::Skull => {
                let state = sup!()?;
                Some(table.set_int(state, p.rotation, rotation_segment(ctx.yaw)))
            }
            Class::WallSkull => {
                let mut state = sup!()?;
                for direction in ctx.nearest_looking_directions() {
                    if is_horizontal(direction) {
                        state = table.set_dir(state, p.facing, direction.opposite());
                        if !self.can_be_replaced(self.state(rel(pos, direction)), ctx) {
                            return Some(Some(state));
                        }
                    }
                }
                None
            }
            Class::AmethystCluster => Some(table.set_dir(wl(default), p.facing, ctx.face)),
            Class::Anvil => Some(table.set_dir(default, p.facing, clockwise(horizontal))),
            Class::BambooStalk => {
                if self.fluid_at(pos).kind() != crate::fluid::FluidKind::Empty {
                    return Some(None);
                }
                let below_state = self.below_state(pos);
                if !b.in_set(below_state, &t.supports_bamboo) {
                    None
                } else if b.is_block(below_state, n.bamboo_sapling) {
                    Some(table.set_int(default, p.age, 0))
                } else if b.is_block(below_state, n.bamboo) {
                    let age = if table.int(below_state, p.age) > 0 {
                        1
                    } else {
                        0
                    };
                    Some(table.set_int(default, p.age, age))
                } else {
                    let above_state = self.above_state(pos);
                    if b.is_block(above_state, n.bamboo) {
                        Some(table.set_int(default, p.age, table.int(above_state, p.age)))
                    } else {
                        Some(b.default_of(n.bamboo_sapling))
                    }
                }
            }
            Class::Banner => {
                Some(table.set_int(default, p.rotation, rotation_segment(ctx.yaw + 180.0)))
            }
            Class::StandingSign => Some(wl(table.set_int(
                default,
                p.rotation,
                rotation_segment(ctx.yaw + 180.0),
            ))),
            Class::Barrel | Class::Command | Class::Dispenser => Some(table.set_dir(
                default,
                p.facing,
                ctx.nearest_looking_direction().opposite(),
            )),
            Class::Observer => {
                Some(table.set_dir(default, p.facing, ctx.nearest_looking_direction()))
            }
            Class::PistonBase => Some(table.set_bool(
                table.set_dir(
                    default,
                    p.facing,
                    ctx.nearest_looking_direction().opposite(),
                ),
                p.extended,
                false,
            )),
            Class::Barrier
            | Class::HeavyCore
            | Class::SculkSensor
            | Class::SculkShrieker
            | Class::Chain
            | Class::MangroveRoots
            | Class::WaterloggedTransparent => {
                let state = if class == Class::Chain
                    || class == Class::MangroveRoots
                    || class == Class::WaterloggedTransparent
                {
                    sup!()?
                } else {
                    default
                };
                Some(wl(state))
            }
            Class::CalibratedSculkSensor => {
                let state = sup!()?;
                Some(table.set_dir(state, p.facing, horizontal))
            }
            Class::BaseCoralPlantType | Class::Conduit => {
                let fluid = self.fluid_at(pos);
                Some(table.set_bool(
                    default,
                    p.waterlogged,
                    b.is_water_fluid(fluid.kind()) && fluid.is_full(),
                ))
            }
            Class::BaseCoralWallFan
            | Class::WallBanner
            | Class::WallSign
            | Class::WallTorch
            | Class::ShelfMushroom
            | Class::TripWireHook
            | Class::Ladder
            | Class::Cocoa => {
                if class == Class::Ladder && !ctx.replace_clicked {
                    let behind = self.state(rel(pos, ctx.face.opposite()));
                    if b.is_block(behind, block) && b.facing(behind) == ctx.face {
                        return Some(None);
                    }
                }
                let mut state = match class {
                    Class::BaseCoralWallFan => sup!()?,
                    Class::TripWireHook => {
                        table.set_bool(table.set_bool(default, p.powered, false), p.attached, false)
                    }
                    _ => default,
                };
                for direction in ctx.nearest_looking_directions() {
                    if is_horizontal(direction) {
                        let facing = if class == Class::Cocoa {
                            direction
                        } else {
                            direction.opposite()
                        };
                        state = table.set_dir(state, p.facing, facing);
                        if self.can_survive(state, pos) {
                            return Some(Some(match class {
                                Class::WallSign | Class::Ladder => wl(state),
                                _ => state,
                            }));
                        }
                    }
                }
                None
            }
            Class::RedstoneWallTorch => {
                let wall = b.default_of(table.block_by_name("minecraft:wall_torch")?);
                let mut found = None;
                for direction in ctx.nearest_looking_directions() {
                    if is_horizontal(direction) {
                        let state = table.set_dir(wall, p.facing, direction.opposite());
                        if self.can_survive(state, pos) {
                            found = Some(state);
                            break;
                        }
                    }
                }
                found.map(|state| table.set_dir(default, p.facing, b.facing(state)))
            }
            Class::WallHangingSign => {
                let mut state = default;
                for direction in ctx.nearest_looking_directions() {
                    if is_horizontal(direction) && axis_of(direction) != axis_of(ctx.face) {
                        state = table.set_dir(state, p.facing, direction.opposite());
                        if self.can_survive(state, pos)
                            && self.wall_hanging_sign_can_place(state, pos)
                        {
                            return Some(Some(wl(state)));
                        }
                    }
                }
                None
            }
            Class::CeilingHangingSign => {
                let above_pos = above(pos);
                let above_state = self.state(above_pos);
                let below_hanging = b
                    .tag("minecraft:all_hanging_signs")
                    .is_some_and(|tag| b.in_set(above_state, tag));
                let direction = from_y_rot(f64::from(ctx.yaw));
                let mut attached_to_middle =
                    !b.collision_face_full(above_state, Direction::Down) || ctx.sneaking;
                if below_hanging && !ctx.sneaking {
                    if b.instance_of(above_state, Class::WallHangingSign) {
                        if axis_of(b.facing(above_state)) == axis_of(direction) {
                            attached_to_middle = false;
                        }
                    } else if table.has(above_state, p.rotation) {
                        let segment = table.int(above_state, p.rotation);
                        let above_direction = match segment {
                            0 => Some(Direction::North),
                            4 => Some(Direction::East),
                            8 => Some(Direction::South),
                            12 => Some(Direction::West),
                            _ => None,
                        };
                        if above_direction.is_some_and(|d| axis_of(d) == axis_of(direction)) {
                            attached_to_middle = false;
                        }
                    }
                }
                let segment = if !attached_to_middle {
                    rotation_segment_of(direction.opposite())
                } else {
                    rotation_segment(ctx.yaw + 180.0)
                };
                Some(wl(table.set_int(
                    table.set_bool(default, p.attached, attached_to_middle),
                    p.rotation,
                    segment,
                )))
            }
            Class::BaseFire | Class::Fire | Class::SoulFire => Some(self.fire_state_for(pos)),
            Class::BaseRail => {
                let east_west = matches!(horizontal, Direction::East | Direction::West);
                let shape = if east_west {
                    "east_west"
                } else {
                    "north_south"
                };
                Some(wl(table.set_text(default, p.shape, shape)))
            }
            Class::Bell => {
                let face = ctx.face;
                if axis_of(face) == Axis::Y {
                    let attachment = if face == Direction::Down {
                        "ceiling"
                    } else {
                        "floor"
                    };
                    let state = table.set_dir(
                        table.set_text(default, p.attachment, attachment),
                        p.facing,
                        horizontal,
                    );
                    if self.can_survive(state, pos) {
                        Some(state)
                    } else {
                        None
                    }
                } else {
                    let double = match axis_of(face) {
                        Axis::X => {
                            self.sturdy(rel(pos, Direction::West), Direction::East)
                                && self.sturdy(rel(pos, Direction::East), Direction::West)
                        }
                        _ => {
                            self.sturdy(rel(pos, Direction::North), Direction::South)
                                && self.sturdy(rel(pos, Direction::South), Direction::North)
                        }
                    };
                    let mut state = table.set_text(
                        table.set_dir(default, p.facing, face.opposite()),
                        p.attachment,
                        if double { "double_wall" } else { "single_wall" },
                    );
                    if self.can_survive(state, pos) {
                        return Some(Some(state));
                    }
                    let floor = self.sturdy(below(pos), Direction::Up);
                    state = table.set_text(
                        state,
                        p.attachment,
                        if floor { "floor" } else { "ceiling" },
                    );
                    if self.can_survive(state, pos) {
                        Some(state)
                    } else {
                        None
                    }
                }
            }
            Class::BigDripleaf => {
                let below_state = self.below_state(pos);
                let dripleaf_part = b.is_block(below_state, n.big_dripleaf)
                    || b.is_block(below_state, n.big_dripleaf_stem);
                let facing = if dripleaf_part {
                    b.facing(below_state)
                } else {
                    horizontal.opposite()
                };
                let source = self.fluid_at(pos) == FluidState::WATER;
                Some(table.set_dir(
                    table.set_bool(default, p.waterlogged, source),
                    p.facing,
                    facing,
                ))
            }
            Class::Campfire => {
                let water = self.fluid_at(pos).kind() == crate::fluid::FluidKind::Water;
                let smoke = self.is_smoke_source(self.below_state(pos));
                let state = table.set_bool(default, p.waterlogged, water);
                let state = table.set_bool(state, p.signal_fire, smoke);
                let state = table.set_bool(state, p.lit, !water);
                Some(table.set_dir(state, p.facing, horizontal))
            }
            Class::Candle => {
                let current = self.state(pos);
                if b.is_block(current, block) {
                    Some(table.cycle(current, p.candles))
                } else {
                    let state = sup!()?;
                    Some(wl(state))
                }
            }
            Class::SeaPickle => {
                let current = self.state(pos);
                if b.is_block(current, block) {
                    Some(table.set_int(
                        current,
                        p.pickles,
                        (table.int(current, p.pickles) + 1).min(4),
                    ))
                } else {
                    let state = sup!()?;
                    Some(wl(state))
                }
            }
            Class::TurtleEgg => {
                let current = self.state(pos);
                if b.is_block(current, block) {
                    Some(table.set_int(current, p.eggs, (table.int(current, p.eggs) + 1).min(4)))
                } else {
                    sup!()
                }
            }
            Class::SnowLayer => {
                let current = self.state(pos);
                if b.is_block(current, block) {
                    Some(table.set_int(
                        current,
                        p.layers,
                        (table.int(current, p.layers) + 1).min(8),
                    ))
                } else {
                    sup!()
                }
            }
            Class::FlowerBed | Class::LeafLitter => {
                let current = self.state(pos);
                let key = if class == Class::FlowerBed {
                    p.flower_amount
                } else {
                    p.segment_amount
                };
                if b.is_block(current, block) {
                    Some(table.set_int(current, key, (table.int(current, key) + 1).min(4)))
                } else {
                    Some(table.set_dir(default, p.facing, horizontal.opposite()))
                }
            }
            Class::Chest => {
                let mut kind = "single";
                let mut facing = horizontal.opposite();
                let face = ctx.face;
                if is_horizontal(face) && ctx.sneaking {
                    if let Some(neighbour_facing) =
                        self.chest_candidate_partner_facing(default, pos, face.opposite())
                    {
                        if axis_of(neighbour_facing) != axis_of(face) {
                            facing = neighbour_facing;
                            kind = if counter_clockwise(facing) == face.opposite() {
                                "right"
                            } else {
                                "left"
                            };
                        }
                    }
                }
                if kind == "single" && !ctx.sneaking {
                    if self.chest_candidate_partner_facing(default, pos, clockwise(facing))
                        == Some(facing)
                    {
                        kind = "left";
                    } else if self.chest_candidate_partner_facing(
                        default,
                        pos,
                        counter_clockwise(facing),
                    ) == Some(facing)
                    {
                        kind = "right";
                    }
                }
                let state =
                    wl(table.set_text(table.set_dir(default, p.facing, facing), p.kind, kind));
                Some(self.copper_chest_least_oxidized(state, pos))
            }
            Class::ChorusPlant => Some(self.chorus_plant_with_connections(pos, default)),
            // A content mod's connecting block (alone, PipeBlock places as
            // Block does).
            Class::Pipe if depth == 0 => Some(self.pipe_with_connections(pos, default)),
            Class::ConcretePowder => {
                let replaced = self.state(pos);
                if b.is_water_fluid(b.fluid(replaced).kind()) || self.concrete_touches_liquid(pos) {
                    Some(b.data(default).link.map_or(default, |c| b.default_of(c)))
                } else {
                    sup!()
                }
            }
            Class::CopperGolemStatue | Class::EnderChest | Class::Shelf => {
                let mut state = wl(table.set_dir(default, p.facing, horizontal.opposite()));
                if class == Class::Shelf {
                    let signal = self.has_neighbor_signal(pos);
                    state = table.set_bool(state, p.powered, signal);
                }
                Some(state)
            }
            Class::Coral => {
                if !self.coral_scan_for_water(pos) {
                    let delay = 60 + self.random().next_int_bound(40).unwrap_or(0);
                    self.schedule_tick(pos, block, delay);
                }
                Some(default)
            }
            Class::Crafter => {
                let front = ctx.nearest_looking_direction().opposite();
                let top = match front {
                    Direction::Down => horizontal.opposite(),
                    Direction::Up => horizontal,
                    _ => Direction::Up,
                };
                let orientation = format!(
                    "{}_{}",
                    super::states::direction_name(front),
                    super::states::direction_name(top)
                );
                let signal = self.has_neighbor_signal(pos);
                Some(table.set_bool(
                    table.set_text(default, p.orientation, &orientation),
                    p.triggered,
                    signal,
                ))
            }
            Class::Jigsaw => {
                let front = ctx.face;
                let top = if axis_of(front) == Axis::Y {
                    horizontal.opposite()
                } else {
                    Direction::Up
                };
                let orientation = format!(
                    "{}_{}",
                    super::states::direction_name(front),
                    super::states::direction_name(top)
                );
                Some(table.set_text(default, p.orientation, &orientation))
            }
            Class::CreakingHeart => {
                let state =
                    table.set_text(default, p.axis, super::states::axis_name(axis_of(ctx.face)));
                Some(self.heart_placement(state, ctx.pos))
            }
            Class::RotatedPillar | Class::InfestedRotatedPillar => {
                Some(table.set_text(default, p.axis, super::states::axis_name(axis_of(ctx.face))))
            }
            Class::DecoratedPot => {
                let state = table.set_dir(default, p.facing, horizontal);
                Some(table.set_bool(wl(state), p.cracked, false))
            }
            Class::Door => {
                if pos.y < self.level.max_y() - 1
                    && self.can_be_replaced(self.state(above(pos)), ctx)
                {
                    let powered =
                        self.has_neighbor_signal(pos) || self.has_neighbor_signal(above(pos));
                    let hinge = self.door_hinge(ctx);
                    let state = table.set_dir(default, p.facing, horizontal);
                    let state = table.set_text(state, p.hinge, hinge);
                    let state =
                        table.set_bool(table.set_bool(state, p.powered, powered), p.open, powered);
                    Some(table.set_text(state, p.half, "lower"))
                } else {
                    None
                }
            }
            Class::DoublePlant => {
                if pos.y < self.level.max_y() - 1
                    && self.can_be_replaced(self.state(above(pos)), ctx)
                {
                    sup!()
                } else {
                    None
                }
            }
            Class::PitcherCrop => Some(default),
            Class::SmallDripleaf => {
                let state = sup!()?;
                let state = table.set_dir(state, p.facing, horizontal.opposite());
                let water = self.is_water_at(pos);
                Some(
                    table
                        .with_bool(state, p.waterlogged, water)
                        .unwrap_or(state),
                )
            }
            Class::TallSeagrass => {
                let state = sup!()?;
                let fluid = self.fluid_at(above(pos));
                if b.is_water_fluid(fluid.kind()) && fluid.is_full() {
                    Some(state)
                } else {
                    None
                }
            }
            Class::DriedGhast => {
                let state = sup!()?;
                Some(table.set_dir(wl(state), p.facing, horizontal.opposite()))
            }
            Class::EndRod => {
                let behind = self.state(rel(pos, ctx.face.opposite()));
                if b.is_block(behind, block) && b.facing(behind) == ctx.face {
                    Some(table.set_dir(default, p.facing, ctx.face.opposite()))
                } else {
                    Some(table.set_dir(default, p.facing, ctx.face))
                }
            }
            Class::LightningRod => Some(wl(table.set_dir(default, p.facing, ctx.face))),
            Class::ShulkerBox => Some(table.set_dir(default, p.facing, ctx.face)),
            Class::FaceAttachedHorizontalDirectional => {
                for direction in ctx.nearest_looking_directions() {
                    let state = if axis_of(direction) == Axis::Y {
                        let face = if direction == Direction::Up {
                            "ceiling"
                        } else {
                            "floor"
                        };
                        table.set_dir(table.set_text(default, p.face, face), p.facing, horizontal)
                    } else {
                        table.set_dir(
                            table.set_text(default, p.face, "wall"),
                            p.facing,
                            direction.opposite(),
                        )
                    };
                    if self.can_survive(state, pos) {
                        return Some(Some(state));
                    }
                }
                None
            }
            Class::Farmland => {
                if !self.can_survive(default, pos) {
                    Some(b.default_of(b.data(default).link.unwrap_or(n.dirt)))
                } else {
                    sup!()
                }
            }
            Class::Path => {
                if !self.can_survive(default, pos) {
                    Some(b.default_of(n.dirt))
                } else {
                    sup!()
                }
            }
            Class::Fence => {
                let mut state = sup!()?;
                for direction in HORIZONTAL {
                    let neighbour = self.rel_state(pos, direction);
                    let sturdy = b.is_face_sturdy(neighbour, direction.opposite());
                    let connect =
                        self.fence_connects_to(default, neighbour, sturdy, direction.opposite());
                    state = table.set_bool(state, self.direction_prop(direction), connect);
                }
                Some(wl(state))
            }
            Class::IronBars => {
                let mut state = default;
                for direction in [
                    Direction::North,
                    Direction::South,
                    Direction::West,
                    Direction::East,
                ] {
                    let neighbour = self.rel_state(pos, direction);
                    let sturdy = b.is_face_sturdy(neighbour, direction.opposite());
                    state = table.set_bool(
                        state,
                        self.direction_prop(direction),
                        self.bars_attach_to(neighbour, sturdy),
                    );
                }
                Some(wl(state))
            }
            Class::Wall => {
                let sides = [
                    Direction::North,
                    Direction::East,
                    Direction::South,
                    Direction::West,
                ]
                .map(|direction| {
                    let neighbour = self.rel_state(pos, direction);
                    self.wall_connects_to(
                        neighbour,
                        b.is_face_sturdy(neighbour, direction.opposite()),
                        direction.opposite(),
                    )
                });
                let top_pos = above(pos);
                let top = self.state(top_pos);
                Some(self.wall_shape(wl(default), top_pos, top, sides))
            }
            Class::FenceGate => {
                let open = self.has_neighbor_signal(pos);
                let wall = |sim: &Self, d: Direction| b.in_set(sim.rel_state(pos, d), &t.walls);
                let in_wall = match axis_of(horizontal) {
                    Axis::Z => wall(self, Direction::West) || wall(self, Direction::East),
                    _ => wall(self, Direction::North) || wall(self, Direction::South),
                };
                let state = table.set_dir(default, p.facing, horizontal);
                let state = table.set_bool(table.set_bool(state, p.open, open), p.powered, open);
                Some(table.set_bool(state, p.in_wall, in_wall))
            }
            Class::GrowingPlant => {
                let growth = b.block_data(block).growth_direction;
                let (head, body) = self.growing_plant_pair(block);
                let next = self.rel_state(pos, growth);
                if !b.is_block(next, head) && !b.is_block(next, body) {
                    if b.block_data(block).is(Class::GrowingPlantHead) {
                        let age = self.random().next_int_bound(25).unwrap_or(0);
                        Some(table.set_int(default, p.age, age))
                    } else {
                        Some(default)
                    }
                } else {
                    Some(b.default_of(body))
                }
            }
            Class::Kelp => {
                let fluid = self.fluid_at(pos);
                if b.is_water_fluid(fluid.kind()) && fluid.is_full() {
                    sup!()
                } else {
                    None
                }
            }
            Class::Seagrass => {
                let fluid = self.fluid_at(pos);
                if b.is_water_fluid(fluid.kind()) && fluid.is_full() {
                    sup!()
                } else {
                    None
                }
            }
            Class::HangingRoots => {
                let state = sup!()?;
                Some(wl(state))
            }
            Class::Hopper => {
                let direction = ctx.face.opposite();
                let facing = if axis_of(direction) == Axis::Y {
                    Direction::Down
                } else {
                    direction
                };
                Some(table.set_bool(table.set_dir(default, p.facing, facing), p.enabled, true))
            }
            Class::HugeMushroom => {
                let mut state = default;
                for direction in Direction::ALL {
                    let same = b.is_block(self.rel_state(pos, direction), block);
                    state = table.set_bool(state, self.direction_prop(direction), !same);
                }
                Some(state)
            }
            Class::Lantern => {
                for direction in ctx.nearest_looking_directions() {
                    if axis_of(direction) == Axis::Y {
                        let state = table.set_bool(default, p.hanging, direction == Direction::Up);
                        if self.can_survive(state, pos) {
                            return Some(Some(wl(state)));
                        }
                    }
                }
                None
            }
            Class::Leaves => {
                let state = wl(table.set_bool(default, p.persistent, true));
                let mut distance = 7;
                for direction in Direction::ALL {
                    distance =
                        distance.min(self.leaf_distance_at(self.rel_state(pos, direction)) + 1);
                    if distance == 1 {
                        break;
                    }
                }
                Some(table.set_int(state, p.distance, distance))
            }
            Class::MangrovePropagule => {
                let state = sup!()?;
                Some(table.set_int(wl(state), p.age, 4))
            }
            Class::MossyCarpet => Some(self.mossy_carpet_updated_state(default, pos, true)),
            Class::Multiface => {
                let old = self.state(pos);
                for direction in ctx.nearest_looking_directions() {
                    if let Some(state) = self.multiface_placement(block, old, pos, direction) {
                        return Some(Some(state));
                    }
                }
                None
            }
            Class::Note => Some(self.note_block_instrument(pos, default)),
            Class::RedstoneLamp => {
                let signal = self.has_neighbor_signal(pos);
                Some(table.set_bool(default, p.lit, signal))
            }
            Class::RedstoneWire => {
                let cross = self.wire_cross_state(0);
                Some(self.wire_connection_state(cross, pos))
            }
            Class::Scaffolding => {
                let distance = self.scaffolding_distance(pos);
                let bottom = self.scaffolding_is_bottom(pos, distance);
                let water = self.fluid_at(pos).kind() == crate::fluid::FluidKind::Water;
                let state = table.set_bool(default, p.waterlogged, water);
                Some(table.set_bool(table.set_int(state, p.distance, distance), p.bottom, bottom))
            }
            Class::Slab => {
                let replaced = self.state(pos);
                if b.is_block(replaced, block) {
                    return Some(Some(table.set_bool(
                        table.set_text(replaced, p.kind, "double"),
                        p.waterlogged,
                        false,
                    )));
                }
                let result = wl(table.set_text(default, p.kind, "bottom"));
                if ctx.half_from_click() {
                    Some(result)
                } else {
                    Some(table.set_text(result, p.kind, "top"))
                }
            }
            Class::Stair => {
                let half = if ctx.half_from_click() {
                    "bottom"
                } else {
                    "top"
                };
                let state =
                    wl(table.set_text(table.set_dir(default, p.facing, horizontal), p.half, half));
                let shape = self.stairs_shape(state, pos);
                Some(table.set_text(state, p.shape, shape))
            }
            Class::TrapDoor => {
                let mut state = if !ctx.replace_clicked && is_horizontal(ctx.face) {
                    let top = ctx.hit.y - f64::from(pos.y) > 0.5;
                    table.set_text(
                        table.set_dir(default, p.facing, ctx.face),
                        p.half,
                        if top { "top" } else { "bottom" },
                    )
                } else {
                    let half = if ctx.face == Direction::Up {
                        "bottom"
                    } else {
                        "top"
                    };
                    table.set_text(
                        table.set_dir(default, p.facing, horizontal.opposite()),
                        p.half,
                        half,
                    )
                };
                if self.has_neighbor_signal(pos) {
                    state = table.set_bool(table.set_bool(state, p.open, true), p.powered, true);
                }
                Some(wl(state))
            }
            Class::Snowy => {
                let snowy = b.in_set(self.above_state(pos), &t.snow);
                Some(table.set_bool(default, p.snowy, snowy))
            }
            Class::Speleothem => {
                let default_tip = ctx.nearest_looking_vertical().opposite();
                let tip = if self.is_valid_speleothem_placement(block, pos, default_tip) {
                    default_tip
                } else if self.is_valid_speleothem_placement(block, pos, default_tip.opposite()) {
                    default_tip.opposite()
                } else {
                    return Some(None);
                };
                let thickness = self.speleothem_thickness_for(block, pos, tip, !ctx.sneaking);
                let state = table.set_dir(default, p.vertical_direction, tip);
                Some(wl(table.set_text(state, p.thickness, thickness)))
            }
            Class::TripWire => {
                let mut state = default;
                for direction in HORIZONTAL {
                    let connect =
                        self.tripwire_should_connect_to(self.rel_state(pos, direction), direction);
                    state = table.set_bool(state, self.direction_prop(direction), connect);
                }
                Some(state)
            }
            Class::Vine => {
                let clicked = self.state(pos);
                let clicked_vine = b.is_block(clicked, block);
                let result = if clicked_vine { clicked } else { default };
                for direction in ctx.nearest_looking_directions() {
                    if direction != Direction::Down {
                        let key = self.direction_prop(direction);
                        let occupied = clicked_vine && table.bool(clicked, key);
                        if !occupied && self.vine_can_support_at(result, pos, direction) {
                            return Some(Some(table.set_bool(result, key, true)));
                        }
                    }
                }
                if clicked_vine { Some(result) } else { None }
            }
            Class::PotentSulfur => Some(self.potent_sulfur_state(default, pos)),
            Class::Nylium => sup!(),
            _ => return None,
        })
    }

    /// `CopperChestBlock.getLeastOxidizedChestOfConnectedBlocks`: a copper chest
    /// joining another takes the less weathered of the two blocks (both unwaxed
    /// when only one was waxed), keeping its own properties.
    pub(crate) fn copper_chest_least_oxidized(
        &self,
        state: BlockStateId,
        pos: BlockPos,
    ) -> BlockStateId {
        let b = self.b;
        if !b.instance_of(state, Class::CopperChest)
            || b.table.text(state, b.props.kind) == "single"
        {
            return state;
        }
        let connected = self.rel_state(pos, self.chest_connected_direction(state));
        if !b.instance_of(connected, Class::CopperChest) {
            return state;
        }
        let name = |s: BlockStateId| b.table.name(s).trim_start_matches("minecraft:").to_owned();
        let weather = |n: &str| {
            let n = n.trim_start_matches("waxed_");
            ["exposed_", "weathered_", "oxidized_"]
                .iter()
                .position(|p| n.starts_with(p))
                .map_or(0, |i| i + 1)
        };
        let (own, other) = (name(state), name(connected));
        let (mut own_block, mut other_block) = (own.clone(), other.clone());
        if own.starts_with("waxed_") != other.starts_with("waxed_") {
            own_block = own.trim_start_matches("waxed_").to_owned();
            other_block = other.trim_start_matches("waxed_").to_owned();
        }
        let least = if weather(&own) <= weather(&other) {
            own_block
        } else {
            other_block
        };
        match b.table.block_by_name(&format!("minecraft:{least}")) {
            Some(block) => b.table.with_properties_of(block, state),
            None => state,
        }
    }
    /// `CopperChestBlock.getFromCopperBlock`: the copper chest a copper block
    /// becomes under a copper golem's pumpkin, facing `facing`.
    pub(crate) fn copper_chest_from_block(
        &self,
        copper: BlockStateId,
        facing: Direction,
        pos: BlockPos,
    ) -> Option<BlockStateId> {
        let b = self.b;
        let block = b.table.name(copper).trim_start_matches("minecraft:");
        let chest = match block {
            "copper_block" => "copper_chest".to_owned(),
            "waxed_copper_block" => "waxed_copper_chest".to_owned(),
            n if n.ends_with("_copper") => format!("{n}_chest"),
            _ => "copper_chest".to_owned(),
        };
        let block = b.table.block_by_name(&format!("minecraft:{chest}"))?;
        let default = b.default_of(block);
        let kind = if self.chest_candidate_partner_facing(default, pos, clockwise(facing))
            == Some(facing)
        {
            "left"
        } else if self.chest_candidate_partner_facing(default, pos, counter_clockwise(facing))
            == Some(facing)
        {
            "right"
        } else {
            "single"
        };
        let state = b.table.set_text(
            b.table.set_dir(default, b.props.facing, facing),
            b.props.kind,
            kind,
        );
        Some(self.copper_chest_least_oxidized(state, pos))
    }
    fn chest_candidate_partner_facing(
        &self,
        chest: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> Option<Direction> {
        let state = self.rel_state(pos, direction);
        if self.chest_can_connect_to(chest, state)
            && self.b.table.text(state, self.b.props.kind) == "single"
        {
            Some(self.b.facing(state))
        } else {
            None
        }
    }
    /// A content mod's connecting block (a `PipeBlock` alone in its chain):
    /// joined on each side to itself and to its `connects_to` tag.
    pub(crate) fn pipe_with_connections(&self, pos: BlockPos, state: BlockStateId) -> BlockStateId {
        let mut result = state;
        for direction in Direction::ALL {
            let connect = self.pipe_connects(state, self.rel_state(pos, direction));
            result = self
                .b
                .table
                .with_bool(result, self.direction_prop(direction), connect)
                .unwrap_or(result);
        }
        result
    }
    /// Whether connecting block `state` joins `neighbour`.
    pub(crate) fn pipe_connects(&self, state: BlockStateId, neighbour: BlockStateId) -> bool {
        let b = self.b;
        b.is_block(neighbour, b.block(state))
            || b.data(state)
                .support_tag
                .as_ref()
                .is_some_and(|t| b.in_set(neighbour, t))
    }
    /// `ChorusPlantBlock.getStateWithConnections`.
    pub(crate) fn chorus_plant_with_connections(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> BlockStateId {
        let b = self.b;
        let block = b.block(state);
        let flower = b.named.chorus_flower;
        let mut result = state;
        for direction in Direction::ALL {
            let neighbour = self.rel_state(pos, direction);
            let mut connect = b.is_block(neighbour, block) || b.is_block(neighbour, flower);
            if direction == Direction::Down {
                connect |= b.in_set(neighbour, &b.tags.supports_chorus_plant);
            }
            result = b
                .table
                .with_bool(result, self.direction_prop(direction), connect)
                .unwrap_or(result);
        }
        result
    }
    fn wall_hanging_sign_can_place(&self, state: BlockStateId, pos: BlockPos) -> bool {
        let b = self.b;
        let facing = b.facing(state);
        let attach = |side: Direction, face: Direction| {
            let attach_state = self.rel_state(pos, side);
            if b.tag("minecraft:wall_hanging_signs")
                .is_some_and(|tag| b.in_set(attach_state, tag))
            {
                axis_of(b.facing(attach_state)) == axis_of(facing)
            } else {
                b.is_face_sturdy(attach_state, face)
            }
        };
        attach(clockwise(facing), counter_clockwise(facing))
            || attach(counter_clockwise(facing), clockwise(facing))
    }
    fn multiface_placement(
        &self,
        block: BlockId,
        old: BlockStateId,
        pos: BlockPos,
        direction: Direction,
    ) -> Option<BlockStateId> {
        let b = self.b;
        if b.is_block(old, block) && self.multiface_has_face(old, direction) {
            return None;
        }
        if !b.multiface_can_attach(self.rel_state(pos, direction), direction) {
            return None;
        }
        let state = if b.is_block(old, block) {
            old
        } else if b.fluid(old) == FluidState::WATER {
            b.table
                .set_bool(b.default_of(block), b.props.waterlogged, true)
        } else {
            b.default_of(block)
        };
        Some(
            b.table
                .set_bool(state, self.direction_prop(direction), true),
        )
    }
    fn vine_can_support_at(&self, vine: BlockStateId, pos: BlockPos, direction: Direction) -> bool {
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
        let above_state = self.above_state(pos);
        b.is_block(above_state, b.block(vine))
            && b.table.bool(above_state, self.direction_prop(direction))
    }
    fn speleothem_thickness_for(
        &self,
        block: BlockId,
        pos: BlockPos,
        tip: Direction,
        merge: bool,
    ) -> &'static str {
        let b = self.b;
        let p = b.props;
        let base = tip.opposite();
        let is_with = |state: BlockStateId, direction: Direction| {
            b.tag("minecraft:speleothems")
                .is_some_and(|tag| b.in_set(state, tag))
                && b.table.dir(state, p.vertical_direction) == Some(direction)
        };
        let front = self.rel_state(pos, tip);
        if is_with(front, base) && b.is_block(front, block) {
            if !merge && b.table.text(front, p.thickness) != "tip_merge" {
                "tip"
            } else {
                "tip_merge"
            }
        } else if !is_with(front, tip) {
            "tip"
        } else {
            let thickness = b.table.text(front, p.thickness);
            if thickness != "tip" && thickness != "tip_merge" {
                if !is_with(self.rel_state(pos, base), tip) {
                    "base"
                } else {
                    "middle"
                }
            } else {
                "frustum"
            }
        }
    }
    /// `DoorBlock.getHinge`.
    fn door_hinge(&self, ctx: &PlacementContext) -> &'static str {
        let b = self.b;
        let pos = ctx.pos;
        let place = ctx.horizontal_direction();
        let above_pos = above(pos);
        let left = counter_clockwise(place);
        let right = clockwise(place);
        let left_state = self.rel_state(pos, left);
        let left_above = self.rel_state(above_pos, left);
        let right_state = self.rel_state(pos, right);
        let right_above = self.rel_state(above_pos, right);
        let full = |s: BlockStateId| b.full_collision(s);
        let balance = -i32::from(full(left_state)) - i32::from(full(left_above))
            + i32::from(full(right_state))
            + i32::from(full(right_above));
        let lower_door = |s: BlockStateId| {
            b.instance_of(s, Class::Door) && b.table.text(s, b.props.half) == "lower"
        };
        let door_left = lower_door(left_state);
        let door_right = lower_door(right_state);
        if (door_left && !door_right) || balance > 0 {
            return "right";
        }
        if (door_right && !door_left) || balance < 0 {
            return "left";
        }
        let step_x = place.normal()[0];
        let step_z = place.normal()[2];
        let click_x = ctx.hit.x - f64::from(pos.x);
        let click_z = ctx.hit.z - f64::from(pos.z);
        if (step_x >= 0 || click_z >= 0.5)
            && (step_x <= 0 || click_z <= 0.5)
            && (step_z >= 0 || click_x <= 0.5)
            && (step_z <= 0 || click_x >= 0.5)
        {
            "left"
        } else {
            "right"
        }
    }

    /// `Block.setPlacedBy` for the blocks that place companions.
    fn set_placed_by(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let data = b.data(state);
        let block = b.block(state);
        if data.is(Class::AbstractBed) {
            let other = rel(pos, b.facing(state));
            self.set_block_and_update(other, b.table.set_text(state, p.part, "head"));
        } else if data.is(Class::Crafter) {
            if b.table.bool(state, p.triggered) {
                self.schedule_tick(pos, block, 4);
            }
        } else if data.is(Class::Diode) {
            if self.diode_should_turn_on(pos, state) {
                self.schedule_tick(pos, block, 1);
            }
        } else if data.is(Class::Door) {
            self.set_block_and_update(above(pos), b.table.set_text(state, p.half, "upper"));
        } else if data.is(Class::DriedGhast) {
            // DriedGhastBlock.setPlacedBy: the place sound, the water one when waterlogged.
            let sound = if b.table.bool(state, p.waterlogged) {
                "minecraft:block.dried_ghast.place_in_water"
            } else {
                "minecraft:block.dried_ghast.place"
            };
            self.sound(pos, &sound.into(), 1.0, 1.0);
        } else if data.is(Class::PitcherCrop) {
        } else if data.is(Class::SmallDripleaf) {
            let above_pos = above(pos);
            let upper = b.table.set_dir(
                b.table.set_text(b.default_of(block), p.half, "upper"),
                p.facing,
                b.facing(state),
            );
            let water = self.is_water_at(above_pos);
            let upper = b
                .table
                .with_bool(upper, p.waterlogged, water)
                .unwrap_or(upper);
            self.set_block_and_update(above_pos, upper);
        } else if data.is(Class::DoublePlant) {
            let above_pos = above(pos);
            let upper = b.table.set_text(b.default_of(block), p.half, "upper");
            let water = self.is_water_at(above_pos);
            let upper = b
                .table
                .with_bool(upper, p.waterlogged, water)
                .unwrap_or(upper);
            self.set_block_and_update(above_pos, upper);
        } else if data.is(Class::PistonBase) {
            self.piston_check_if_extend(pos, state);
        } else if data.is(Class::TripWireHook) {
            self.tripwire_hook_calculate(pos, state, false, false, -1, None);
        } else if data.is(Class::MossyCarpet) {
            let topper = self.mossy_carpet_topper(pos);
            if !b.is_air(topper) {
                self.set_block_and_update(above(pos), topper);
            }
        }
    }
    /// `MossyCarpetBlock.createTopperWithSideChance(level, pos, random::nextBoolean)`.
    fn mossy_carpet_topper(&mut self, pos: BlockPos) -> BlockStateId {
        let b = self.b;
        let p = b.props;
        let block = b.block(self.state(pos));
        let above_pos = above(pos);
        let above_state = self.state(above_pos);
        let is_carpet = b.is_block(above_state, block);
        if (!is_carpet || !b.table.bool(above_state, p.bottom))
            && (is_carpet || b.is_replaceable(above_state))
        {
            let carpet = b.table.set_bool(b.default_of(block), p.bottom, false);
            let updated = self.mossy_carpet_updated_state(carpet, above_pos, true);
            let mut result = updated;
            for direction in HORIZONTAL {
                let key = self.direction_prop(direction);
                if b.table.text(result, key) != "none" && !self.random().next_boolean() {
                    result = b.table.set_text(result, key, "none");
                }
            }
            if self.mossy_carpet_has_faces(result) && result != above_state {
                return result;
            }
        }
        b.air()
    }

    /// `BlockItem.place` (and its subclasses' overrides).
    /// `BlockPlaceContext` resolution, `updatePlacementContext` and the item's
    /// `getPlacementState` (survival and obstruction checks included).
    pub(crate) fn placement(
        &mut self,
        input: &PlaceContext,
    ) -> Option<(PlacementContext, super::data::ItemBinding, BlockStateId)> {
        let b = self.b;
        let binding = b.item_binding(&input.item).cloned()?;
        let mut ctx = PlacementContext {
            pos: input.clicked_pos,
            hit_pos: input.clicked_pos,
            face: input.face,
            hit: input.hit,
            inside: input.inside,
            yaw: input.yaw,
            pitch: input.pitch,
            sneaking: input.sneaking,
            item: input.item.clone(),
            replace_clicked: true,
        };
        ctx.replace_clicked = self.can_be_replaced(self.state(input.clicked_pos), &ctx);
        if !ctx.replace_clicked {
            ctx.pos = rel(input.clicked_pos, input.face);
        }
        if !ctx.replace_clicked && !self.can_be_replaced(self.state(ctx.pos), &ctx) {
            return None;
        }
        if binding.kind == ItemKind::Scaffolding {
            ctx = self.scaffolding_update_context(&ctx, binding.block)?;
        }
        if binding.game_master && !input.can_use_game_master_blocks {
            return None;
        }
        let state = match binding.kind {
            ItemKind::StandingAndWall | ItemKind::HangingSign => {
                let wall = binding
                    .wall_block
                    .and_then(|wall| self.state_for_placement(wall, &ctx));
                let attachment = binding.attachment.unwrap_or(Direction::Down);
                let mut chosen = None;
                for direction in ctx.nearest_looking_directions() {
                    if direction == attachment.opposite() {
                        continue;
                    }
                    let candidate = if direction == attachment {
                        self.state_for_placement(binding.block, &ctx)
                    } else {
                        wall
                    };
                    if let Some(candidate) = candidate {
                        let hanging_ok = binding.kind != ItemKind::HangingSign
                            || !b.instance_of(candidate, Class::WallHangingSign)
                            || self.wall_hanging_sign_can_place(candidate, ctx.pos);
                        if hanging_ok && self.can_survive(candidate, ctx.pos) {
                            chosen = Some(candidate);
                            break;
                        }
                    }
                }
                chosen.filter(|state| self.level.is_unobstructed(ctx.pos, *state))
            }
            _ => {
                let must_survive = binding.kind != ItemKind::Scaffolding;
                self.state_for_placement(binding.block, &ctx)
                    .filter(|state| {
                        (!must_survive || self.can_survive(*state, ctx.pos))
                            && self.level.is_unobstructed(ctx.pos, *state)
                    })
            }
        };
        state.map(|state| (ctx, binding, state))
    }

    /// `BlockItem.place` (and its subclasses' overrides).
    pub(crate) fn place(&mut self, input: &PlaceContext) -> PlaceOutcome {
        let b = self.b;
        let Some((ctx, binding, state)) = self.placement(input) else {
            return PlaceOutcome::Failed;
        };
        if !self.is_in_valid_bounds(ctx.pos) {
            return PlaceOutcome::Failed;
        }

        if binding.kind == ItemKind::DoubleHigh {
            let above_pos = above(ctx.pos);
            let replacement = if self.is_water_at(above_pos) {
                b.default_of(b.named.water)
            } else {
                b.air()
            };
            self.set_block(above_pos, replacement, 27);
        }
        if !self.set_block(ctx.pos, state, 11) {
            return PlaceOutcome::Failed;
        }
        let placed = self.state(ctx.pos);
        if b.block(placed) == b.block(state) {
            self.set_placed_by(ctx.pos, placed);
        }
        let placed = self.state(ctx.pos);
        let sounds = b.data(placed).sounds.clone();
        self.emit(LevelEvent::Sound {
            pos: ctx.pos,
            sound: sounds.place.clone(),
            volume: (sounds.volume + 1.0) / 2.0,
            pitch: sounds.pitch * 0.8,
        });
        self.game_event_state(ctx.pos, "minecraft:block_place", placed);
        PlaceOutcome::Placed {
            pos: ctx.pos,
            state: placed,
        }
    }
    /// `ScaffoldingBlockItem.updatePlacementContext`.
    fn scaffolding_update_context(
        &self,
        ctx: &PlacementContext,
        block: BlockId,
    ) -> Option<PlacementContext> {
        let b = self.b;
        let pos = ctx.pos;
        if !b.is_block(self.state(pos), block) {
            return if self.scaffolding_distance(pos) == 7 {
                None
            } else {
                Some(ctx.clone())
            };
        }
        let direction = if ctx.sneaking {
            if ctx.inside {
                ctx.face.opposite()
            } else {
                ctx.face
            }
        } else if ctx.face == Direction::Up {
            ctx.horizontal_direction()
        } else {
            Direction::Up
        };
        let mut horizontal_distance = 0;
        let mut placement = rel(pos, direction);
        while horizontal_distance < 7 {
            if !self.is_in_valid_bounds(placement) {
                break;
            }
            let replaced = self.state(placement);
            if !b.is_block(replaced, block) {
                if self.can_be_replaced(replaced, ctx) {
                    let [dx, dy, dz] = direction.normal();
                    return Some(PlacementContext {
                        pos: placement,
                        hit_pos: placement,
                        face: direction,
                        hit: Vec3::new(
                            f64::from(placement.x) + 0.5 + f64::from(dx) * 0.5,
                            f64::from(placement.y) + 0.5 + f64::from(dy) * 0.5,
                            f64::from(placement.z) + 0.5 + f64::from(dz) * 0.5,
                        ),
                        inside: false,
                        replace_clicked: true,
                        ..ctx.clone()
                    });
                }
                break;
            }
            placement = rel(placement, direction);
            if is_horizontal(direction) {
                horizontal_distance += 1;
            }
        }
        None
    }
}

impl Behaviours {
    /// `BlockItem.place` for the item in `ctx`: resolves the replaced
    /// position, computes `getStateForPlacement`, checks survival and entity
    /// obstruction, writes the block (flags 11), runs `setPlacedBy` (second
    /// halves of doors, beds, tall plants) and reports the place sound.
    pub fn place_block<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        ctx: &PlaceContext,
    ) -> PlaceOutcome {
        self.sim(level, ticks).place(ctx)
    }
    /// `DirectionalPlaceContext.canPlace`: whether the block at the clicked
    /// position can be replaced by the item (a dispenser places only there).
    pub fn directional_can_place<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        ctx: &PlaceContext,
    ) -> bool {
        let sim = self.sim(level, ticks);
        let inner = PlacementContext {
            pos: ctx.clicked_pos,
            hit_pos: ctx.clicked_pos,
            face: ctx.face,
            hit: ctx.hit,
            inside: ctx.inside,
            yaw: ctx.yaw,
            pitch: ctx.pitch,
            sneaking: ctx.sneaking,
            item: ctx.item.clone(),
            replace_clicked: true,
        };
        sim.can_be_replaced(sim.state(ctx.clicked_pos), &inner)
    }
    /// The position and state `BlockItem.place` would place for `ctx`, without
    /// writing anything (None when vanilla's placement fails).
    pub fn placement_state<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        ctx: &PlaceContext,
    ) -> Option<(BlockPos, BlockStateId)> {
        self.sim(level, ticks)
            .placement(ctx)
            .map(|(ctx, _, state)| (ctx.pos, state))
    }
    /// `getStateForPlacement` of `block` for `ctx` without placing anything
    /// (`None` when vanilla refuses the placement). The position is resolved
    /// as `BlockPlaceContext` does.
    pub fn state_for_placement<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        block: BlockId,
        input: &PlaceContext,
    ) -> Option<BlockStateId> {
        let mut sim = self.sim(level, ticks);
        let mut ctx = PlacementContext {
            pos: input.clicked_pos,
            hit_pos: input.clicked_pos,
            face: input.face,
            hit: input.hit,
            inside: input.inside,
            yaw: input.yaw,
            pitch: input.pitch,
            sneaking: input.sneaking,
            item: input.item.clone(),
            replace_clicked: true,
        };
        ctx.replace_clicked = sim.can_be_replaced(sim.state(input.clicked_pos), &ctx);
        if !ctx.replace_clicked {
            ctx.pos = rel(input.clicked_pos, input.face);
        }
        sim.state_for_placement(block, &ctx)
    }
}
