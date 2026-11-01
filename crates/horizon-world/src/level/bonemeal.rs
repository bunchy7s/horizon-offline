//! Bone meal: `BoneMealItem.useOn` / `growCrop` / `growWaterPlant` and every
//! vanilla `BonemealableBlock` (`isValidBonemealTarget`, `isBonemealSuccess`,
//! `performBonemeal`), ported in vanilla's random draw order.
use super::{
    Behaviours, LevelAccess, LevelEvent, Sim, Ticks,
    classes::{Class, Interface},
    feature_world::LevelFeatureWorld,
    growth::grower,
    update::{HORIZONTAL, above, below, rel},
};
use crate::{fluid::FluidState, registry::BlockStateId};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};

/// The `BonemealableBlock` implementation a block uses (most derived wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Sapling,
    Propagule,
    Mushroom,
    Fungus,
    Azalea,
    Grass,
    Nylium,
    Netherrack,
    FeaturePlacer,
    RootedDirt,
    MangroveLeaves,
    Crop,
    Stem,
    Pitcher,
    BambooSapling,
    Bamboo,
    Cocoa,
    SweetBerry,
    TallGrass,
    TallFlower,
    SpreadingBush,
    FlowerBed,
    CaveVines,
    PlantHead,
    CaveVinesPlant,
    PlantBody,
    Seagrass,
    SeaPickle,
    BigDripleaf,
    BigDripleafStem,
    SmallDripleaf,
    GlowLichen,
    HangingMoss,
    MossyCarpet,
    ShortDryGrass,
    TallDryGrass,
    ShelfMushroom,
}

fn kind_of(class: Class) -> Option<Kind> {
    Some(match class {
        Class::MangrovePropagule => Kind::Propagule,
        Class::Sapling => Kind::Sapling,
        Class::Mushroom => Kind::Mushroom,
        Class::NetherFungus => Kind::Fungus,
        Class::Azalea => Kind::Azalea,
        Class::Grass => Kind::Grass,
        Class::Nylium => Kind::Nylium,
        Class::Netherrack => Kind::Netherrack,
        Class::BonemealableFeaturePlacer => Kind::FeaturePlacer,
        Class::RootedDirt => Kind::RootedDirt,
        Class::MangroveLeaves => Kind::MangroveLeaves,
        Class::Crop => Kind::Crop,
        Class::Stem => Kind::Stem,
        Class::PitcherCrop => Kind::Pitcher,
        Class::BambooSapling => Kind::BambooSapling,
        Class::BambooStalk => Kind::Bamboo,
        Class::Cocoa => Kind::Cocoa,
        Class::SweetBerryBush => Kind::SweetBerry,
        Class::TallGrass => Kind::TallGrass,
        Class::TallFlower => Kind::TallFlower,
        Class::Bush | Class::FireflyBush => Kind::SpreadingBush,
        Class::FlowerBed => Kind::FlowerBed,
        Class::CaveVines => Kind::CaveVines,
        Class::GrowingPlantHead => Kind::PlantHead,
        Class::CaveVinesPlant => Kind::CaveVinesPlant,
        Class::GrowingPlantBody => Kind::PlantBody,
        Class::Seagrass => Kind::Seagrass,
        Class::SeaPickle => Kind::SeaPickle,
        Class::BigDripleaf => Kind::BigDripleaf,
        Class::BigDripleafStem => Kind::BigDripleafStem,
        Class::SmallDripleaf => Kind::SmallDripleaf,
        Class::GlowLichen => Kind::GlowLichen,
        Class::HangingMoss => Kind::HangingMoss,
        Class::MossyCarpet => Kind::MossyCarpet,
        Class::ShortDryGrass => Kind::ShortDryGrass,
        Class::TallDryGrass => Kind::TallDryGrass,
        Class::ShelfMushroom => Kind::ShelfMushroom,
        _ => return None,
    })
}

/// `Mth.nextInt(random, min, max)`.
fn next_int_between<R: RandomSource>(random: &mut R, min: i32, max: i32) -> i32 {
    if min >= max {
        min
    } else {
        random.next_int_bound(max - min + 1).unwrap_or(0) + min
    }
}

impl Behaviours {
    /// `/place feature`: `ConfiguredFeature.place` at `pos` with the level
    /// random. `None` when the feature is unknown or cannot run.
    pub fn place_feature<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        name: &str,
        pos: BlockPos,
    ) -> Option<bool> {
        self.live_features()?.configured(name)?;
        Some(self.sim(level, ticks).place_configured_feature(name, pos))
    }
    /// `StructureBlockEntity.placeStructure`'s `StructureTemplate.placeInWorld`
    /// on the live level: `pos` is both the position and the pivot, the
    /// template rots at `integrity` below 1 (`BlockRotProcessor`) drawing from
    /// `LegacyRandomSource(seed)`, the loot seeds from a second one, and
    /// `flags` are the `setBlock` flags. Returns whether the template placed
    /// and the entities it carried (their NBT, for the caller to spawn);
    /// `None` when the level has no structure resources.
    #[allow(clippy::too_many_arguments)]
    pub fn place_template<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        template: &crate::worldgen::structure::StructureTemplate,
        pos: BlockPos,
        settings: TemplateSettings,
    ) -> Option<(bool, Vec<crate::nbt::Compound>)> {
        use crate::worldgen::structure::processor::{PlaceSettings, Processor, place_in_world};
        let features = self.live_features()?;
        let resources = features.structure_resources()?;
        let mut sim = self.sim(level, ticks);
        let rot = Processor::BlockRot {
            rottable: None,
            integrity: settings.integrity,
        };
        let mut random = horizon_core::random::LegacyRandom::new(settings.seed);
        let mut loot_random = horizon_core::random::LegacyRandom::new(settings.seed);
        let mut place = PlaceSettings::new(settings.rotation);
        place.mirror = settings.mirror;
        place.known_shape = settings.known_shape;
        place.ignore_entities = settings.ignore_entities;
        if settings.integrity < 1.0 {
            place.processors.push(&rot);
            place.random = Some(&mut random);
        }
        let mut world = LevelFeatureWorld {
            sim: &mut sim,
            features,
            entities: Vec::new(),
        };
        let at = [pos.x, pos.y, pos.z];
        let placed = place_in_world(
            &mut world,
            &resources,
            template,
            at,
            at,
            &mut place,
            Some(&mut loot_random),
            settings.flags,
        )
        .ok()?;
        Some((placed, world.entities))
    }
    /// `BoneMealItem.useOn` for a player clicking `pos` on `face`. Returns
    /// whether bone meal was used (the caller then shrinks the stack unless
    /// the player is in creative mode). Reports level event 1505 (growth
    /// particles and sound) at the grown position.
    pub fn use_bone_meal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Direction,
    ) -> bool {
        let mut sim = self.sim(level, ticks);
        if sim.grow_crop(pos) {
            sim.level_event(1505, pos, 15);
            return true;
        }
        let clicked = sim.state(pos);
        let relative = rel(pos, face);
        if self.is_face_sturdy(clicked, face) && sim.grow_water_plant(relative, Some(face)) {
            sim.level_event(1505, relative, 15);
            return true;
        }
        false
    }
    /// `BoneMealItem.useOn` on a client's level, where nothing grows: whether
    /// it succeeds there (the server's packet does the growing). A sapling,
    /// an azalea or a mushroom is only a valid target on a `ServerLevel`
    /// (`isValidBonemealTarget`), so the client passes on them; `growWaterPlant`
    /// takes any full water block beside a sturdy clicked face.
    pub fn bone_meal_succeeds_on_client<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        face: Direction,
    ) -> bool {
        let mut ticks = Ticks::default();
        let mut sim = self.sim(level, &mut ticks);
        let state = sim.state(pos);
        // A mod block answers `isValidBonemealTarget` on both sides, so the
        // client predicts the use with the mod's own answer.
        let answer = sim.level.is_valid_bonemeal_target(pos, state);
        if answer == Some(true) {
            return true;
        }
        // Refused (`Some(false)`): growCrop fails, and only growWaterPlant
        // below can still take the meal.
        if answer.is_none()
            && let Some(kind) = self.bonemeal_kind(state)
            && !matches!(kind, Kind::Sapling | Kind::Azalea | Kind::Mushroom)
            && sim.bonemeal_valid(kind, pos, state)
        {
            return true;
        }
        let water = sim.state(rel(pos, face));
        self.is_face_sturdy(state, face)
            && self.is_block(water, self.named.water)
            && self.fluid(water).amount() == 8
    }
    /// `BoneMealItem.growCrop` (dispensers call this directly).
    pub fn grow_crop<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) -> bool {
        self.sim(level, ticks).grow_crop(pos)
    }
    /// `BoneMealItem.growWaterPlant` (dispensers pass no clicked face).
    pub fn grow_water_plant<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        face: Option<Direction>,
    ) -> bool {
        self.sim(level, ticks).grow_water_plant(pos, face)
    }
    /// The `BonemealableBlock.Type` particle spread of a block at level event
    /// 1505: Some(true) for `NEIGHBOR_SPREADER` (particles over a 3-block
    /// area above), Some(false) for `GROWER`, None when not bonemealable.
    pub fn bone_meal_spreads(&self, state: BlockStateId) -> Option<bool> {
        let kind = self.bonemeal_kind(state)?;
        Some(matches!(
            kind,
            Kind::Grass | Kind::Nylium | Kind::Netherrack | Kind::FeaturePlacer
        ))
    }
    fn bonemeal_kind(&self, state: BlockStateId) -> Option<Kind> {
        let data = self.data(state);
        if !data.implements(Interface::Bonemealable) {
            return None;
        }
        data.chain.iter().find_map(|&c| kind_of(c))
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn next_int(&mut self, bound: i32) -> i32 {
        self.random().next_int_bound(bound).unwrap_or(0)
    }
    fn block_named(&self, name: &str) -> Option<BlockStateId> {
        let b = self.b;
        b.table.block_by_name(name).map(|id| b.default_of(id))
    }
    fn in_tag(&self, state: BlockStateId, tag: &str) -> bool {
        self.b.tag(tag).is_some_and(|set| self.b.in_set(state, set))
    }

    /// `BoneMealItem.growCrop`: true when the block is a valid target (bone
    /// meal is consumed even when `isBonemealSuccess` fails).
    pub(crate) fn grow_crop(&mut self, pos: BlockPos) -> bool {
        let state = self.state(pos);
        // A mod block answers vanilla's three questions itself, ahead of the
        // `BonemealableBlock` its template would give it; a block nothing
        // answers for keeps its own, and vanilla's is what the port runs.
        match self.level.is_valid_bonemeal_target(pos, state) {
            Some(true) => {
                if self.level.is_bonemeal_success(pos, state) {
                    self.level.perform_bonemeal(self.ticks, pos, state);
                }
                return true;
            }
            // `growCrop` returns false and keeps the meal.
            Some(false) => return false,
            None => {}
        }
        let Some(kind) = self.b.bonemeal_kind(state) else {
            return false;
        };
        if !self.bonemeal_valid(kind, pos, state) {
            return false;
        }
        if self.bonemeal_success(kind, pos, state) {
            self.perform_bonemeal(kind, pos, state);
        }
        true
    }

    /// `BonemealableBlock.isValidBonemealTarget`.
    fn bonemeal_valid(&mut self, kind: Kind, pos: BlockPos, state: BlockStateId) -> bool {
        let b = self.b;
        let p = b.props;
        let up = above(pos);
        match kind {
            Kind::Sapling => {
                let Some(g) = b.data(state).feature.as_deref().and_then(grower) else {
                    return false;
                };
                if b.live_features().is_none() || !self.tree_can_grow(g, pos, state) {
                    return false;
                }
                let height = self.tree_minimum_height(g);
                self.is_inside_build_height(pos.y + height)
            }
            Kind::Propagule => !b.table.bool(state, p.hanging) || b.table.int(state, p.age) != 4,
            Kind::Mushroom => {
                let Some(feature) = b.data(state).feature.as_deref() else {
                    return false;
                };
                let Some(json) = b.live_features().and_then(|f| f.configured_json(feature)) else {
                    return false;
                };
                let huge = matches!(
                    json["type"].as_str(),
                    Some("minecraft:huge_red_mushroom" | "minecraft:huge_brown_mushroom")
                );
                let radius = json["foliage_radius"].as_i64().unwrap_or(2) as i32;
                huge && self.is_inside_build_height(pos.y + 4 + radius)
            }
            Kind::Fungus => {
                let below_state = self.below_state(pos);
                b.data(state)
                    .link
                    .is_some_and(|required| b.is_block(below_state, required))
                    && self.is_inside_build_height(pos.y + 1)
            }
            Kind::Azalea => {
                let Some(g) = grower("azalea") else {
                    return false;
                };
                let height = self.tree_minimum_height(g);
                self.is_inside_build_height(pos.y + height + 2)
                    && self.fluid_at(up) == FluidState::EMPTY
            }
            Kind::Grass | Kind::Nylium | Kind::BambooSapling => {
                b.is_air(self.state(up)) && self.is_inside_build_height(up.y)
            }
            Kind::Netherrack => {
                if !b.has_flag(self.state(up), super::data::state_flags::SKYLIGHT_DOWN) {
                    return false;
                }
                self.netherrack_nylium_near(pos) != (false, false)
            }
            Kind::FeaturePlacer => b.is_air(self.state(up)),
            Kind::RootedDirt => {
                b.is_air(self.state(below(pos))) && self.is_inside_build_height(pos.y - 1)
            }
            Kind::MangroveLeaves => b.is_air(self.state(below(pos))),
            Kind::Crop => b.table.int(state, p.age) < self.crop_max_age(state),
            Kind::Stem => b.table.int(state, p.age) != 7,
            Kind::Pitcher => match self.pitcher_lower(pos, state) {
                Some((lower_pos, lower)) => {
                    let age = b.table.int(lower, p.age) + 1;
                    self.pitcher_can_grow(lower_pos, lower, age)
                }
                None => false,
            },
            Kind::Bamboo => {
                let height_above = self.bamboo_height_above(pos);
                let height_below = self.bamboo_height_below(pos);
                let growth = pos.offset(0, height_above + 1, 0);
                height_above + height_below + 1 < 16
                    && b.table
                        .int(self.state(pos.offset(0, height_above, 0)), p.stage)
                        != 1
                    && self.is_inside_build_height(growth.y)
                    && self.is_empty_block(growth)
            }
            Kind::Cocoa => b.table.int(state, p.age) < 2,
            Kind::SweetBerry => b.table.int(state, p.age) < 3,
            Kind::TallGrass => {
                let Some(grown) = self.tall_grass_grown(state) else {
                    return false;
                };
                self.can_survive(grown, pos)
                    && self.is_empty_block(up)
                    && self.is_inside_build_height(up.y)
            }
            Kind::TallFlower | Kind::FlowerBed | Kind::SmallDripleaf | Kind::ShortDryGrass => true,
            Kind::SpreadingBush => self.spreadable_neighbour(pos, state, false).is_some(),
            Kind::TallDryGrass => {
                let Some(short) = self.block_named("minecraft:short_dry_grass") else {
                    return false;
                };
                self.spreadable_neighbour(pos, short, false).is_some()
            }
            Kind::PlantHead => {
                let growth = rel(pos, b.data(state).growth_direction);
                self.plant_can_grow_into(state, self.state(growth))
                    && self.is_inside_build_height(growth.y)
            }
            Kind::CaveVines | Kind::CaveVinesPlant => !b.table.bool(state, p.berries),
            Kind::PlantBody => match self.plant_head_pos(pos, state) {
                Some(head) => {
                    let head_state = self.state(head);
                    let growth = rel(head, b.data(state).growth_direction);
                    self.plant_can_grow_into(head_state, self.state(growth))
                        && self.is_inside_build_height(growth.y)
                }
                None => false,
            },
            Kind::Seagrass => b.is_block(self.state(up), b.named.water),
            Kind::SeaPickle => {
                b.table.bool(state, p.waterlogged)
                    && self.in_tag(self.below_state(pos), "minecraft:coral_blocks")
            }
            Kind::BigDripleaf => self.dripleaf_can_grow_into(up),
            Kind::BigDripleafStem => {
                let Some(leaf) = b.table.block_by_name("minecraft:big_dripleaf") else {
                    return false;
                };
                self.top_connected(pos, b.block(state), Direction::Up, leaf)
                    .is_some_and(|head| self.dripleaf_can_grow_into(above(head)))
            }
            Kind::GlowLichen => Direction::ALL
                .iter()
                .any(|&face| self.lichen_can_spread_in_any_direction(state, pos, face.opposite())),
            Kind::HangingMoss => {
                let grow = below(self.hanging_moss_tip(pos, state));
                b.is_air(self.state(grow)) && self.is_inside_build_height(grow.y)
            }
            Kind::MossyCarpet => {
                b.table.bool(state, p.bottom) && !b.is_air(self.mossy_topper_all_sides(pos))
            }
            Kind::ShelfMushroom => b.table.int(state, p.age) < 1,
        }
    }

    /// `BonemealableBlock.isBonemealSuccess`.
    fn bonemeal_success(&mut self, kind: Kind, _pos: BlockPos, state: BlockStateId) -> bool {
        let b = self.b;
        match kind {
            Kind::Sapling | Kind::Azalea => self.random().next_float() < 0.45,
            Kind::Propagule => {
                if b.table.bool(state, b.props.hanging) {
                    b.table.int(state, b.props.age) != 4
                } else {
                    self.random().next_float() < 0.45
                }
            }
            Kind::Mushroom | Kind::Fungus => self.random().next_float() < 0.4,
            _ => true,
        }
    }

    /// `BonemealableBlock.performBonemeal`.
    fn perform_bonemeal(&mut self, kind: Kind, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let up = above(pos);
        match kind {
            Kind::Sapling => self.advance_tree(pos, state),
            Kind::Propagule => {
                if b.table.bool(state, p.hanging) && b.table.int(state, p.age) != 4 {
                    self.set_block(pos, b.table.cycle(state, p.age), 2);
                } else {
                    self.advance_tree(pos, state);
                }
            }
            Kind::Mushroom => {
                self.grow_mushroom(pos, state);
            }
            Kind::Fungus => {
                if let Some(feature) = b.data(state).feature.clone() {
                    self.place_configured_feature(&feature, pos);
                }
            }
            Kind::Azalea => {
                if let Some(g) = grower("azalea") {
                    self.grow_tree(g, pos, state);
                }
            }
            Kind::Grass => self.grass_bonemeal(pos, state),
            Kind::Nylium => {
                if self.is_inside_build_height(up.y) {
                    self.place_configured_feature("minecraft:nylium_bonemeal", up);
                }
            }
            Kind::Netherrack => {
                let (warped, crimson) = self.netherrack_nylium_near(pos);
                let warped_state = self.block_named("minecraft:warped_nylium");
                let crimson_state = self.block_named("minecraft:crimson_nylium");
                let next = if warped && crimson {
                    if self.random().next_boolean() {
                        warped_state
                    } else {
                        crimson_state
                    }
                } else if warped {
                    warped_state
                } else if crimson {
                    crimson_state
                } else {
                    None
                };
                if let Some(next) = next {
                    self.set_block_and_update(pos, next);
                }
            }
            Kind::FeaturePlacer => {
                if let Some(feature) = b.data(state).feature.clone() {
                    self.place_configured_feature(&feature, up);
                }
            }
            Kind::RootedDirt => {
                if let Some(roots) = self.block_named("minecraft:hanging_roots") {
                    self.set_block_and_update(below(pos), roots);
                }
            }
            Kind::MangroveLeaves => {
                if let Some(propagule) = self.block_named("minecraft:mangrove_propagule") {
                    let hanging = b.table.set_bool(propagule, p.hanging, true);
                    self.set_block(below(pos), b.table.set_int(hanging, p.age, 0), 2);
                }
            }
            Kind::Crop => {
                let max = self.crop_max_age(state);
                let increase = self.crop_bonemeal_increase(state);
                let age = max.min(b.table.int(state, p.age) + increase);
                let next = self.crop_state_for_age(state, age);
                self.set_block(pos, next, 2);
            }
            Kind::Stem => {
                let increase = next_int_between(self.random(), 2, 5);
                let age = 7.min(b.table.int(state, p.age) + increase);
                let next = b.table.set_int(state, p.age, age);
                self.set_block(pos, next, 2);
                if age == 7 {
                    self.random_tick_block(next, pos);
                }
            }
            Kind::Pitcher => {
                if let Some((lower_pos, lower)) = self.pitcher_lower(pos, state) {
                    self.pitcher_grow(lower, lower_pos, 1);
                }
            }
            Kind::BambooSapling => self.bamboo_sapling_grow(pos),
            Kind::Bamboo => {
                let height_above = self.bamboo_height_above(pos);
                let height_below = self.bamboo_height_below(pos);
                let total = height_above + height_below + 1;
                let count = 1 + self.next_int(2);
                for i in 0..count {
                    let top = pos.offset(0, height_above + i, 0);
                    let top_state = self.state(top);
                    let growth = above(top);
                    if total + i >= 16
                        || b.table.int(top_state, p.stage) == 1
                        || !self.is_empty_block(growth)
                        || !self.is_inside_build_height(growth.y)
                    {
                        return;
                    }
                    self.bamboo_grow(top_state, top, total + i);
                }
            }
            Kind::Cocoa => {
                let age = b.table.int(state, p.age);
                self.set_block(pos, b.table.set_int(state, p.age, age + 1), 2);
            }
            Kind::SweetBerry => {
                let age = 3.min(b.table.int(state, p.age) + 1);
                self.set_block(pos, b.table.set_int(state, p.age, age), 2);
            }
            Kind::TallGrass => {
                if let Some(grown) = self.tall_grass_grown(state) {
                    self.double_plant_place_at(grown, pos, 2);
                }
            }
            Kind::TallFlower => self.pop_block_item(pos, state),
            Kind::SpreadingBush => {
                if let Some(at) = self.spreadable_neighbour(pos, state, true) {
                    self.set_block_and_update(at, b.default_of(b.block(state)));
                }
            }
            Kind::TallDryGrass => {
                if let Some(short) = self.block_named("minecraft:short_dry_grass")
                    && let Some(at) = self.spreadable_neighbour(pos, short, true)
                {
                    self.set_block_and_update(at, short);
                }
            }
            Kind::ShortDryGrass => {
                if let Some(tall) = self.block_named("minecraft:tall_dry_grass") {
                    self.set_block_and_update(pos, tall);
                }
            }
            Kind::FlowerBed => {
                let amount = b.table.int(state, p.flower_amount);
                if amount < 4 {
                    self.set_block(pos, b.table.set_int(state, p.flower_amount, amount + 1), 2);
                } else {
                    self.pop_block_item(pos, state);
                }
            }
            Kind::PlantHead => self.plant_head_bonemeal(pos, state),
            Kind::CaveVines | Kind::CaveVinesPlant => {
                self.set_block(pos, b.table.set_bool(state, p.berries, true), 2);
            }
            Kind::PlantBody => {
                if let Some(head) = self.plant_head_pos(pos, state) {
                    let head_state = self.state(head);
                    match b.bonemeal_kind(head_state) {
                        Some(Kind::CaveVines) => {
                            self.set_block(head, b.table.set_bool(head_state, p.berries, true), 2);
                        }
                        Some(Kind::PlantHead) => self.plant_head_bonemeal(head, head_state),
                        _ => {}
                    }
                }
            }
            Kind::Seagrass => {
                let lower = b.default_of(b.named.tall_seagrass);
                let upper = b.table.set_text(lower, p.half, "upper");
                self.set_block(pos, lower, 2);
                self.set_block(up, upper, 2);
            }
            Kind::SeaPickle => self.sea_pickle_bonemeal(pos, state),
            Kind::BigDripleaf => {
                if self.dripleaf_can_grow_into(up) {
                    let facing = b.facing(state);
                    let fluid = b.fluid(state);
                    self.dripleaf_stem_place(pos, fluid, facing);
                    let fluid_above = self.fluid_at(up);
                    self.dripleaf_place(up, fluid_above, facing);
                }
            }
            Kind::BigDripleafStem => {
                let Some(leaf) = b.table.block_by_name("minecraft:big_dripleaf") else {
                    return;
                };
                if let Some(head) = self.top_connected(pos, b.block(state), Direction::Up, leaf) {
                    let facing = b.facing(state);
                    let fluid = self.fluid_at(head);
                    self.dripleaf_stem_place(head, fluid, facing);
                    let fluid_above = self.fluid_at(above(head));
                    self.dripleaf_place(above(head), fluid_above, facing);
                }
            }
            Kind::SmallDripleaf => {
                if b.table.text(state, p.half) == "lower" {
                    let empty = b.legacy_block(self.fluid_at(up));
                    self.set_block(up, empty, 18);
                    let facing = b.facing(state);
                    self.dripleaf_place_random_height(pos, facing);
                } else {
                    let below_pos = below(pos);
                    let below_state = self.state(below_pos);
                    if b.bonemeal_kind(below_state) == Some(Kind::SmallDripleaf) {
                        self.perform_bonemeal(Kind::SmallDripleaf, below_pos, below_state);
                    } else {
                        // performBonemeal on a non-dripleaf below: vanilla reads
                        // FACING/HALF of that state and would throw; do nothing.
                    }
                }
            }
            Kind::GlowLichen => self.lichen_spread_random(pos, state),
            Kind::HangingMoss => {
                let tip = below(self.hanging_moss_tip(pos, state));
                if b.is_air(self.state(tip)) {
                    self.set_block_and_update(tip, b.table.set_bool(state, p.tip, true));
                }
            }
            Kind::MossyCarpet => {
                let topper = self.mossy_topper_all_sides(pos);
                if !b.is_air(topper) {
                    self.set_block_and_update(up, topper);
                }
            }
            Kind::ShelfMushroom => {
                let age = b.table.int(state, p.age);
                self.set_block(pos, b.table.set_int(state, p.age, age + 1), 2);
            }
        }
    }

    // ---- helpers ----

    /// `Block.popResource(level, pos, new ItemStack(this))`.
    fn pop_block_item(&mut self, pos: BlockPos, state: BlockStateId) {
        self.emit(LevelEvent::BlockEvent {
            pos,
            state,
            kind: "pop_block_item",
        });
    }
    /// `CropBlock.getMaxAge`.
    pub(crate) fn crop_max_age(&self, state: BlockStateId) -> i32 {
        let b = self.b;
        if b.instance_of(state, Class::TorchflowerCrop) {
            2
        } else {
            b.table.int_range(state, b.props.age).map_or(7, |r| r.1)
        }
    }
    /// `CropBlock.getStateForAge` (torchflower crops become torchflowers).
    pub(crate) fn crop_state_for_age(&self, state: BlockStateId, age: i32) -> BlockStateId {
        let b = self.b;
        if b.instance_of(state, Class::TorchflowerCrop)
            && age == 2
            && let Some(flower) = self.block_named("minecraft:torchflower")
        {
            return flower;
        }
        b.table
            .set_int(b.default_of(b.block(state)), b.props.age, age)
    }
    /// `CropBlock.getBonemealAgeIncrease` and its overrides.
    fn crop_bonemeal_increase(&mut self, state: BlockStateId) -> i32 {
        let b = self.b;
        if b.instance_of(state, Class::TorchflowerCrop) {
            1
        } else if b.instance_of(state, Class::Beetroot) {
            next_int_between(self.random(), 2, 5) / 3
        } else {
            next_int_between(self.random(), 2, 5)
        }
    }
    /// `PitcherCropBlock.getLowerHalf`.
    fn pitcher_lower(
        &self,
        pos: BlockPos,
        state: BlockStateId,
    ) -> Option<(BlockPos, BlockStateId)> {
        let b = self.b;
        let is_lower = |s: BlockStateId| {
            b.is_block(s, b.named.pitcher_crop) && b.table.text(s, b.props.half) == "lower"
        };
        if is_lower(state) {
            return Some((pos, state));
        }
        let below_pos = below(pos);
        let below_state = self.state(below_pos);
        is_lower(below_state).then_some((below_pos, below_state))
    }
    /// `PitcherCropBlock.canGrow`.
    fn pitcher_can_grow(&self, lower_pos: BlockPos, lower: BlockStateId, new_age: i32) -> bool {
        let b = self.b;
        let upper = self.state(above(lower_pos));
        b.table.int(lower, b.props.age) < 4
            && self.raw_brightness(lower_pos, 0) >= 8
            && self.is_inside_build_height(lower_pos.y + 1)
            && (new_age < 3 || b.is_air(upper) || b.is_block(upper, b.named.pitcher_crop))
    }
    /// `TallGrassBlock.getGrownBlock`.
    fn tall_grass_grown(&self, state: BlockStateId) -> Option<BlockStateId> {
        let name = if self.b.table.name(state) == "minecraft:fern" {
            "minecraft:large_fern"
        } else {
            "minecraft:tall_grass"
        };
        self.block_named(name)
    }
    /// `DoublePlantBlock.placeAt`.
    fn double_plant_place_at(&mut self, state: BlockStateId, lower: BlockPos, flags: i32) {
        let b = self.b;
        let p = b.props;
        let upper = above(lower);
        let lower_state = b.table.set_text(state, p.half, "lower");
        let lower_state = b
            .table
            .with_bool(lower_state, p.waterlogged, self.is_water_at(lower))
            .unwrap_or(lower_state);
        self.set_block(lower, lower_state, flags);
        let upper_state = b.table.set_text(state, p.half, "upper");
        let upper_state = b
            .table
            .with_bool(upper_state, p.waterlogged, self.is_water_at(upper))
            .unwrap_or(upper_state);
        self.set_block(upper, upper_state, flags);
    }
    /// `BonemealableBlock.getSpreadableNeighbourPos` (shuffled order when
    /// `shuffle`, as `findSpreadableNeighbourPos`).
    fn spreadable_neighbour(
        &mut self,
        pos: BlockPos,
        to_place: BlockStateId,
        shuffle: bool,
    ) -> Option<BlockPos> {
        let mut directions = HORIZONTAL;
        if shuffle {
            // Util.shuffle.
            for i in (2..=directions.len()).rev() {
                let swap = self.next_int(i as i32) as usize;
                directions.swap(i - 1, swap);
            }
        }
        directions
            .into_iter()
            .map(|d| rel(pos, d))
            .find(|&n| self.is_empty_block(n) && self.can_survive(to_place, n))
    }
    /// `NetherrackBlock`: (warped nylium nearby, crimson nylium nearby).
    fn netherrack_nylium_near(&self, pos: BlockPos) -> (bool, bool) {
        let (mut warped, mut crimson) = (false, false);
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    let state = self.state(pos.offset(x, y, z));
                    if !self.in_tag(state, "minecraft:nylium") {
                        continue;
                    }
                    match self.b.table.name(state) {
                        "minecraft:warped_nylium" => warped = true,
                        "minecraft:crimson_nylium" => crimson = true,
                        _ => {}
                    }
                    if warped && crimson {
                        return (true, true);
                    }
                }
            }
        }
        (warped, crimson)
    }
    /// `GrowingPlantHeadBlock.canGrowInto` of the head's block.
    fn plant_can_grow_into(&self, head: BlockStateId, state: BlockStateId) -> bool {
        let b = self.b;
        if b.instance_of(head, Class::Kelp) {
            b.is_block(state, b.named.water)
        } else {
            b.is_air(state)
        }
    }
    /// `GrowingPlantHeadBlock.performBonemeal`.
    fn plant_head_bonemeal(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let direction = b.data(state).growth_direction;
        let mut forward = rel(pos, direction);
        let mut next_age = (b.table.int(state, b.props.age) + 1).min(25);
        let count = if b.instance_of(state, Class::TwistingVines)
            || b.instance_of(state, Class::WeepingVines)
        {
            // NetherVines.getBlocksToGrowWhenBonemealed.
            let mut probability = 1.0;
            let mut count = 0;
            while self.random().next_double() < probability {
                probability *= 0.826;
                count += 1;
            }
            count
        } else {
            1
        };
        for _ in 0..count {
            if !self.plant_can_grow_into(state, self.state(forward))
                || !self.is_inside_build_height(forward.y)
            {
                break;
            }
            self.set_block_and_update(forward, b.table.set_int(state, b.props.age, next_age));
            forward = rel(forward, direction);
            next_age = (next_age + 1).min(25);
        }
    }
    /// `GrowingPlantBodyBlock.getHeadPos` (`BlockUtil.getTopConnectedBlock`).
    fn plant_head_pos(&self, pos: BlockPos, state: BlockStateId) -> Option<BlockPos> {
        let b = self.b;
        let name = b.table.name(state);
        let head = b.table.block_by_name(name.strip_suffix("_plant")?)?;
        self.top_connected(pos, b.block(state), b.data(state).growth_direction, head)
    }
    /// `BlockUtil.getTopConnectedBlock`.
    fn top_connected(
        &self,
        pos: BlockPos,
        body: super::states::BlockId,
        direction: Direction,
        head: super::states::BlockId,
    ) -> Option<BlockPos> {
        let mut forward = pos;
        loop {
            forward = rel(forward, direction);
            let state = self.state(forward);
            if !self.b.is_block(state, body) {
                return self.b.is_block(state, head).then_some(forward);
            }
            if !self.level.is_loaded(forward) || !self.is_in_valid_bounds(forward) {
                return None;
            }
        }
    }
    /// `HangingMossBlock.getTip`.
    fn hanging_moss_tip(&self, pos: BlockPos, state: BlockStateId) -> BlockPos {
        let block = self.b.block(state);
        let mut forward = pos;
        loop {
            forward = below(forward);
            if !self.b.is_block(self.state(forward), block) || forward.y < self.level.min_y() {
                return above(forward);
            }
        }
    }
    /// `MossyCarpetBlock.createTopperWithSideChance(level, pos, () -> true)`.
    fn mossy_topper_all_sides(&mut self, pos: BlockPos) -> BlockStateId {
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
            let topper = self.mossy_carpet_updated_state(carpet, above_pos, true);
            if self.mossy_carpet_has_faces(topper) && topper != above_state {
                return topper;
            }
        }
        b.air()
    }
    /// `SeaPickleBlock.performBonemeal`.
    fn sea_pickle_bonemeal(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let p = b.props;
        let (mut z_span, mut z_offset) = (1, 0);
        let x_start = pos.x - 2;
        for (count, x) in (0..5).enumerate() {
            for z in 0..z_span {
                let end_y = 2 + pos.y - 1;
                for y in (end_y - 2)..end_y {
                    let position = BlockPos::new(x_start + x, y, pos.z - z_offset + z);
                    if position != pos
                        && self.next_int(6) == 0
                        && b.is_block(self.state(position), b.named.water)
                        && self.in_tag(self.below_state(position), "minecraft:coral_blocks")
                    {
                        let pickles = self.next_int(4) + 1;
                        let pickle =
                            b.table
                                .set_int(b.default_of(b.block(state)), p.pickles, pickles);
                        self.set_block_and_update(position, pickle);
                    }
                }
            }
            if count < 2 {
                z_span += 2;
                z_offset += 1;
            } else {
                z_span -= 2;
                z_offset -= 1;
            }
        }
        self.set_block(pos, b.table.set_int(state, p.pickles, 4), 2);
    }
    /// `BigDripleafBlock.canGrowInto` / `canPlaceAt`.
    fn dripleaf_can_grow_into(&self, pos: BlockPos) -> bool {
        let b = self.b;
        let state = self.state(pos);
        self.is_inside_build_height(pos.y)
            && (b.is_air(state)
                || b.is_block(state, b.named.water)
                || b.table.name(state) == "minecraft:small_dripleaf")
    }
    fn dripleaf_set(&mut self, name: &str, pos: BlockPos, fluid: FluidState, facing: Direction) {
        let b = self.b;
        let Some(state) = self.block_named(name) else {
            return;
        };
        let state = b
            .table
            .set_bool(state, b.props.waterlogged, fluid == FluidState::WATER);
        let state = b.table.set_dir(state, b.props.facing, facing);
        self.set_block_and_update(pos, state);
    }
    fn dripleaf_place(&mut self, pos: BlockPos, fluid: FluidState, facing: Direction) {
        self.dripleaf_set("minecraft:big_dripleaf", pos, fluid, facing);
    }
    fn dripleaf_stem_place(&mut self, pos: BlockPos, fluid: FluidState, facing: Direction) {
        self.dripleaf_set("minecraft:big_dripleaf_stem", pos, fluid, facing);
    }
    /// `BigDripleafBlock.placeWithRandomHeight`.
    fn dripleaf_place_random_height(&mut self, bottom: BlockPos, facing: Direction) {
        let desired = next_int_between(self.random(), 2, 5);
        let mut pos = bottom;
        let mut height = 0;
        while height < desired && self.dripleaf_can_grow_into(pos) {
            height += 1;
            pos = above(pos);
        }
        let leaf_y = bottom.y + height - 1;
        pos = bottom;
        while pos.y < leaf_y {
            let fluid = self.fluid_at(pos);
            self.dripleaf_stem_place(pos, fluid, facing);
            pos = above(pos);
        }
        let fluid = self.fluid_at(pos);
        self.dripleaf_place(pos, fluid, facing);
    }

    // ---- MultifaceSpreader (glow lichen, default config) ----

    /// `SpreadType.getSpreadPos` for SAME_POSITION, SAME_PLANE, WRAP_AROUND.
    fn spread_positions(
        pos: BlockPos,
        spread: Direction,
        from: Direction,
    ) -> [(BlockPos, Direction); 3] {
        [
            (pos, spread),
            (rel(pos, spread), from),
            (rel(rel(pos, spread), from), spread.opposite()),
        ]
    }
    /// `DefaultSpreaderConfig.canSpreadInto`.
    fn lichen_can_spread_into(
        &self,
        block: super::states::BlockId,
        at: BlockPos,
        face: Direction,
    ) -> bool {
        let b = self.b;
        let existing = self.state(at);
        let replaceable = b.is_air(existing)
            || b.is_block(existing, block)
            || (b.is_block(existing, b.named.water) && b.fluid(existing).is_source());
        replaceable
            && !(b.is_block(existing, block) && self.multiface_has_face(existing, face))
            && b.multiface_can_attach(self.state(rel(at, face)), face)
    }
    /// `MultifaceSpreader.getSpreadFromFaceTowardDirection`.
    fn lichen_spread_target(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        from: Direction,
        spread: Direction,
    ) -> Option<(BlockPos, Direction)> {
        if crate::level::query::axis_of(spread) == crate::level::query::axis_of(from) {
            return None;
        }
        if !self.multiface_has_face(state, from) || self.multiface_has_face(state, spread) {
            return None;
        }
        let block = self.b.block(state);
        Self::spread_positions(pos, spread, from)
            .into_iter()
            .find(|&(at, face)| self.lichen_can_spread_into(block, at, face))
    }
    fn lichen_can_spread_in_any_direction(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        from: Direction,
    ) -> bool {
        Direction::ALL.iter().any(|&spread| {
            self.lichen_spread_target(state, pos, from, spread)
                .is_some()
        })
    }
    fn shuffled_directions(&mut self) -> [Direction; 6] {
        let mut directions = Direction::ALL;
        for i in (2..=directions.len()).rev() {
            let swap = self.next_int(i as i32) as usize;
            directions.swap(i - 1, swap);
        }
        directions
    }
    /// `MultifaceSpreader.spreadFromRandomFaceTowardRandomDirection`.
    fn lichen_spread_random(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let block = b.block(state);
        for from in self.shuffled_directions() {
            if !self.multiface_has_face(state, from) {
                continue;
            }
            for spread in self.shuffled_directions() {
                let Some((at, face)) = self.lichen_spread_target(state, pos, from, spread) else {
                    continue;
                };
                let old = self.state(at);
                // MultifaceBlock.getStateForPlacement then setBlock(..., 2).
                let base = if b.is_block(old, block) {
                    old
                } else if b.fluid(old) == FluidState::WATER {
                    b.table
                        .set_bool(b.default_of(block), b.props.waterlogged, true)
                } else {
                    b.default_of(block)
                };
                let placed = b.table.set_bool(base, self.direction_prop(face), true);
                if self.set_block(at, placed, 2) {
                    return;
                }
            }
        }
    }

    // ---- water plants and grass ----

    /// `BoneMealItem.growWaterPlant`.
    pub(crate) fn grow_water_plant(&mut self, pos: BlockPos, face: Option<Direction>) -> bool {
        let b = self.b;
        let start = self.state(pos);
        if !(b.is_block(start, b.named.water) && b.fluid(start).amount() == 8) {
            return false;
        }
        let seagrass = b.default_of(b.named.seagrass);
        let features = b.live_features();
        'attempts: for j in 0..128 {
            let mut test = pos;
            let mut grow = seagrass;
            for _ in 0..j / 16 {
                let dx = self.next_int(3) - 1;
                let dy = (self.next_int(3) - 1) * self.next_int(3) / 2;
                let dz = self.next_int(3) - 1;
                test = test.offset(dx, dy, dz);
                if b.full_collision(self.state(test)) {
                    continue 'attempts;
                }
            }
            let corals = match (features, self.level.biome_name(test)) {
                (Some(f), Some(biome)) => {
                    f.biome_in_tag(biome, "minecraft:produces_corals_from_bonemeal")
                }
                _ => false,
            };
            if corals {
                if j == 0 && face.is_some_and(|f| !matches!(f, Direction::Up | Direction::Down)) {
                    if let Some(coral) = self.random_tag_element("minecraft:wall_corals") {
                        grow = coral;
                        if b.table.has(grow, b.props.facing) {
                            grow = b.table.set_dir(grow, b.props.facing, face.unwrap());
                        }
                    }
                } else if self.next_int(4) == 0
                    && let Some(plant) = self.random_tag_element("minecraft:underwater_bonemeals")
                {
                    grow = plant;
                }
            }
            if self.in_tag(grow, "minecraft:wall_corals") && b.table.has(grow, b.props.facing) {
                let mut d = 0;
                while !self.can_survive(grow, test) && d < 4 {
                    let facing = HORIZONTAL[self.next_int(4) as usize];
                    grow = b.table.set_dir(grow, b.props.facing, facing);
                    d += 1;
                }
            }
            if self.can_survive(grow, test) {
                let test_state = self.state(test);
                if b.is_block(test_state, b.named.water) && b.fluid(test_state).amount() == 8 {
                    self.set_block_and_update(test, grow);
                } else if b.is_block(test_state, b.named.seagrass)
                    && b.is_block(self.state(above(test)), b.named.water)
                    && self.next_int(10) == 0
                {
                    self.perform_bonemeal(Kind::Seagrass, test, test_state);
                }
            }
        }
        true
    }
    /// `Registry.getRandomElementOf(tag, random)`: default state of a random member.
    fn random_tag_element(&mut self, tag: &str) -> Option<BlockStateId> {
        let features = self.b.live_features()?;
        let entries = features.block_tag_entries(tag);
        if entries.is_empty() {
            return None;
        }
        let index = self.next_int(entries.len() as i32) as usize;
        self.block_named(&entries[index])
    }
    /// `GrassBlock.performBonemeal`.
    fn grass_bonemeal(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let block = b.block(state);
        let start = above(pos);
        'attempts: for attempt in 0..128 {
            let mut test = start;
            for _ in 0..attempt / 16 {
                let dx = self.next_int(3) - 1;
                let dy = (self.next_int(3) - 1) * self.next_int(3) / 2;
                let dz = self.next_int(3) - 1;
                test = test.offset(dx, dy, dz);
                // stopBonemealSpread
                if !b.is_block(self.below_state(test), block) || b.full_collision(self.state(test))
                {
                    continue 'attempts;
                }
            }
            self.grass_bonemeal_effect(test);
        }
    }
    /// `GrassBlock.placeBonemealEffect`.
    fn grass_bonemeal_effect(&mut self, test: BlockPos) {
        let b = self.b;
        let test_state = self.state(test);
        if b.table.name(test_state) == "minecraft:short_grass"
            && self.random().next_float() < 0.1
            && self.bonemeal_valid(Kind::TallGrass, test, test_state)
        {
            self.perform_bonemeal(Kind::TallGrass, test, test_state);
        }
        if b.is_air(test_state) && self.is_inside_build_height(test.y) {
            if self.random().next_float() < 0.125 {
                let Some(features) = b.live_features() else {
                    return;
                };
                let Some(biome) = self
                    .level
                    .biome_name(test)
                    .and_then(|name| features.biome_id(name))
                else {
                    return;
                };
                let list = features.bone_meal_features(biome);
                if list.is_empty() {
                    return;
                }
                let feature = list[self.next_int(list.len() as i32) as usize].clone();
                self.place_configured_feature(&feature, test);
            } else {
                self.place_placed_feature("minecraft:grass_bonemeal", test);
            }
        }
    }
}

/// What `StructurePlaceSettings` a `place_template` call sets.
#[derive(Clone, Copy, Debug)]
pub struct TemplateSettings {
    pub mirror: crate::worldgen::structure::Mirror,
    pub rotation: crate::worldgen::structure::Rotation,
    pub ignore_entities: bool,
    pub known_shape: bool,
    /// `BlockRotProcessor`'s integrity; 1 places everything.
    pub integrity: f32,
    pub seed: i64,
    pub flags: i32,
}
