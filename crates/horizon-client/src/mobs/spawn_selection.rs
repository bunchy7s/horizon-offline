//! `NaturalSpawner`: which mob spawns where and when. The spawn state
//! (`createState`: category counts, the spawn potential's charges, the
//! per-player local caps), the filtered categories and their global and local
//! caps, `spawnForChunk` and `spawnCategoryForPosition` (the three packs of up
//! to four attempts, the distance and respawn checks, `getRandomSpawnMobAt`
//! over `mobsAt`: the nether fortress bounds, structure spawn overrides by
//! piece or bounding box, then the biome's natural mob spawns), the per-type
//! checks of `isValidSpawnPostitionForType` and `isValidPositionForMob`
//! (each class's `checkSpawnRules` walk target value, `checkSpawnObstruction`,
//! `removeWhenFarAway`, cluster and group sizes), and
//! `spawnMobsForChunkGeneration`. Everything reads the level through
//! [`NaturalLevel`] so the game and the differential test world
//! (tools/test_natural_spawn_parity.py) run the same code; random draws follow
//! vanilla's order.
use super::Species;
use super::spawn_rules::{self, Reason, SpawnLevel};
use crate::biome::SpawnEntry;
use crate::fast_hash::FxHashMap;
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

pub use crate::biome::{SpawnCost, parse_costs, parse_entries};

/// `MobCategory`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Monster,
    Creature,
    Ambient,
    Axolotls,
    UndergroundWaterCreature,
    WaterCreature,
    WaterAmbient,
    Misc,
}
impl Category {
    pub const ALL: [Self; 8] = [
        Self::Monster,
        Self::Creature,
        Self::Ambient,
        Self::Axolotls,
        Self::UndergroundWaterCreature,
        Self::WaterCreature,
        Self::WaterAmbient,
        Self::Misc,
    ];
    /// `NaturalSpawner.SPAWNING_CATEGORIES`: all but MISC, in enum order.
    pub const SPAWNING: [Self; 7] = [
        Self::Monster,
        Self::Creature,
        Self::Ambient,
        Self::Axolotls,
        Self::UndergroundWaterCreature,
        Self::WaterCreature,
        Self::WaterAmbient,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Monster => "monster",
            Self::Creature => "creature",
            Self::Ambient => "ambient",
            Self::Axolotls => "axolotls",
            Self::UndergroundWaterCreature => "underground_water_creature",
            Self::WaterCreature => "water_creature",
            Self::WaterAmbient => "water_ambient",
            Self::Misc => "misc",
        }
    }
    pub fn from_name(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .unwrap_or(Self::Misc)
    }
    fn index(self) -> usize {
        self as usize
    }
    /// `getMaxInstancesPerChunk`.
    pub fn max(self) -> i32 {
        match self {
            Self::Monster => 70,
            Self::Creature => 10,
            Self::Ambient => 15,
            Self::Axolotls | Self::UndergroundWaterCreature | Self::WaterCreature => 5,
            Self::WaterAmbient => 20,
            Self::Misc => -1,
        }
    }
    pub fn friendly(self) -> bool {
        self != Self::Monster
    }
    pub fn persistent(self) -> bool {
        matches!(self, Self::Creature | Self::Misc)
    }
    pub fn despawn_distance(self) -> i32 {
        if self == Self::WaterAmbient { 64 } else { 128 }
    }
}

/// `PathfinderMob.checkSpawnRules`: the class's `getWalkTargetValue` at the
/// spawn position must not be negative (plain `Mob`s accept any position).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WalkTarget {
    NotPathfinder,
    Zero,
    Animal,
    Mooshroom,
    Turtle,
    Monster,
    Giant,
    Guardian,
    Silverfish,
    Strider,
    Hoglin,
    Bee,
    HappyGhast,
}
fn walk_target(s: Species) -> WalkTarget {
    use Species as S;
    use WalkTarget as W;
    match s {
        S::Bat | S::Slime | S::MagmaCube | S::SulfurCube | S::Ghast | S::Phantom => {
            W::NotPathfinder
        }
        // Monster subclasses that override it with 0.
        S::Enderman | S::Creaking | S::Pillager | S::Warden => W::Zero,
        S::Giant => W::Giant,
        S::Guardian | S::ElderGuardian => W::Guardian,
        S::Silverfish => W::Silverfish,
        S::Zombie
        | S::Husk
        | S::Drowned
        | S::ZombieVillager
        | S::ZombifiedPiglin
        | S::Skeleton
        | S::Stray
        | S::Bogged
        | S::Parched
        | S::WitherSkeleton
        | S::Creeper
        | S::Spider
        | S::CaveSpider
        | S::Endermite
        | S::Witch
        | S::Vindicator
        | S::Evoker
        | S::Illusioner
        | S::Ravager
        | S::Piglin
        | S::PiglinBrute
        | S::Zoglin
        | S::Blaze
        | S::Vex
        | S::Breeze
        | S::Wither => W::Monster,
        S::Mooshroom => W::Mooshroom,
        S::Turtle => W::Turtle,
        S::Strider => W::Strider,
        S::Hoglin => W::Hoglin,
        S::Bee => W::Bee,
        S::HappyGhast => W::HappyGhast,
        // Animal subclasses with their own value.
        S::Axolotl | S::Nautilus | S::ZombieNautilus => W::Zero,
        S::Pig
        | S::Cow
        | S::Sheep
        | S::Chicken
        | S::Rabbit
        | S::Horse
        | S::Donkey
        | S::Mule
        | S::Llama
        | S::TraderLlama
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk
        | S::Goat
        | S::Panda
        | S::PolarBear
        | S::Fox
        | S::Wolf
        | S::Cat
        | S::Ocelot
        | S::Parrot
        | S::Armadillo
        | S::Frog
        | S::Sniffer => W::Animal,
        // Other PathfinderMobs: water animals, golems, villagers, allays...
        _ => W::Zero,
    }
}
/// Each class's `checkSpawnObstruction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Obstruction {
    /// `Mob`: no liquid in the box and no entity blocking it.
    NoLiquidUnobstructed,
    Unobstructed,
    Ocelot,
    NoLiquid,
}
fn obstruction(s: Species) -> Obstruction {
    use Species as S;
    match s {
        S::Squid
        | S::GlowSquid
        | S::Dolphin
        | S::Axolotl
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole
        | S::Nautilus
        | S::ZombieNautilus
        | S::Guardian
        | S::ElderGuardian
        | S::Strider
        | S::Drowned => Obstruction::Unobstructed,
        S::Ocelot => Obstruction::Ocelot,
        S::Ravager => Obstruction::NoLiquid,
        // ZombifiedPiglin's override is the Mob default's two tests; iron
        // golems and wardens never spawn naturally.
        _ => Obstruction::NoLiquidUnobstructed,
    }
}
/// `Mob.removeWhenFarAway` of a freshly created mob.
fn removed_when_far(s: Species) -> bool {
    use Species as S;
    match s {
        // Animal, golems, villagers, traders, allays and wardens never;
        // cats and ocelots only past 2400 ticks; chickens as jockeys.
        S::Axolotl
        | S::Cod
        | S::Salmon
        | S::TropicalFish
        | S::Pufferfish
        | S::Tadpole
        | S::CamelHusk
        | S::ZombieHorse
        | S::Hoglin
        | S::Nautilus
        | S::ZombieNautilus => true,
        S::Villager
        | S::WanderingTrader
        | S::IronGolem
        | S::SnowGolem
        | S::Shulker
        | S::Allay
        | S::Warden => false,
        _ => !is_animal(s),
    }
}
fn is_animal(s: Species) -> bool {
    use Species as S;
    matches!(walk_target(s), WalkTarget::Animal)
        || matches!(
            s,
            S::Axolotl
                | S::Nautilus
                | S::ZombieNautilus
                | S::Mooshroom
                | S::Turtle
                | S::Strider
                | S::Hoglin
                | S::Bee
                | S::HappyGhast
        )
}
/// `Mob.getMaxSpawnClusterSize`.
pub fn max_cluster_size(s: Species) -> u32 {
    use Species as S;
    match s {
        S::Horse
        | S::Donkey
        | S::Mule
        | S::Llama
        | S::TraderLlama
        | S::SkeletonHorse
        | S::ZombieHorse
        | S::Camel
        | S::CamelHusk => 6,
        S::Cod | S::TropicalFish | S::Pufferfish | S::Tadpole | S::Wolf => 8,
        S::Salmon => 5,
        S::Ghast | S::HappyGhast | S::Pillager => 1,
        _ => 4,
    }
}
/// `EntityType.canSpawnFarFromPlayer`: creatures, misc, pillagers, shulkers.
fn spawns_far_from_player(s: Species, category: Category) -> bool {
    matches!(category, Category::Creature | Category::Misc)
        || matches!(s, Species::Pillager | Species::Shulker)
}

/// What natural spawning reads from a level beyond the spawn rules.
/// An entity type's species, width, height and category.
pub type MobType = (Species, f32, f32, Category);

pub trait NaturalLevel: SpawnLevel {
    fn min_y(&self) -> i32;
    /// `getMaxY()`: the highest buildable block.
    fn max_y(&self) -> i32;
    /// `DimensionType.ambientLight`.
    fn ambient_light(&self) -> f32;
    fn has_ceiling(&self) -> bool;
    /// The entity type's species, width, height and category.
    fn mob_type(&self, entity: &str) -> Option<MobType>;
    /// The `natural_mob_spawns` lists by category at a position.
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>>;
    /// The `natural_mob_spawns` spawn cost of a type at a position.
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost>;
    /// The `creature_world_gen_spawn_probability` attribute at a position.
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32;
    /// Every structure whose references reach the position's chunk and
    /// whose id `wanted` takes, in the chunk's reference order.
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt>;
    fn structure_spawns(&self) -> &StructureSpawns;
    /// Where the players `getNearestPlayer(x, y, z, -1, false)` measures to
    /// are: every one that is not a spectator.
    fn nearest_player_positions(&self) -> &[[f64; 3]];
    /// The world spawn point, when it is in this dimension.
    fn respawn(&self) -> Option<[i32; 3]>;
    /// `ServerLevel.canSpawnEntitiesInChunk`.
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool;
    /// `noCollision(null, box)`: blocks under an empty collision context and
    /// entities that can be collided with.
    fn no_collision(&self, bounds: Aabb) -> bool;
    /// `isUnobstructed(entity)`: no entity blocking building overlaps.
    fn unobstructed(&self, bounds: Aabb) -> bool;
    /// The `spawn_mobs` game rule.
    fn spawn_mobs(&self) -> bool;
    /// `getHeight(heightmap, x, z)`.
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32;
    /// `isPathfindable(PathComputationType.LAND)` of the block.
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool;
}

/// The heightmap types `SpawnPlacements` registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heightmap {
    WorldSurface,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}
impl Heightmap {
    pub fn from_name(name: &str) -> Self {
        match name {
            "WORLD_SURFACE" => Self::WorldSurface,
            "OCEAN_FLOOR" => Self::OceanFloor,
            "MOTION_BLOCKING" => Self::MotionBlocking,
            _ => Self::MotionBlockingNoLeaves,
        }
    }
}

/// `Heightmap.Types`' predicates: WORLD_SURFACE (not air), OCEAN_FLOOR
/// (`#blocks_motion_in_heightmap`), MOTION_BLOCKING (that or a fluid) and
/// MOTION_BLOCKING_NO_LEAVES (`#blocks_motion_in_heightmap_no_leaves` or a fluid).
pub fn heightmap_blocks<L: SpawnLevel>(l: &L, kind: Heightmap, x: i32, y: i32, z: i32) -> bool {
    let state = l.state(x, y, z);
    let f = l.facts().get(state);
    let fluid = f.fluid != crate::blocks::Fluid::None;
    match kind {
        Heightmap::WorldSurface => !l.facts().is_air(state),
        Heightmap::OceanFloor => l.block_tag("minecraft:blocks_motion_in_heightmap", &f.name),
        Heightmap::MotionBlocking => {
            fluid || l.block_tag("minecraft:blocks_motion_in_heightmap", &f.name)
        }
        Heightmap::MotionBlockingNoLeaves => {
            fluid || l.block_tag("minecraft:blocks_motion_in_heightmap_no_leaves", &f.name)
        }
    }
}
/// `LevelReader.getLightLevelDependentMagicValue`.
fn magic_light<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> f32 {
    let raw = l
        .block_light(x, y, z)
        .max(l.sky_light(x, y, z) - l.sky_darken());
    let v = raw as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + l.ambient_light() * (1.0 - curved)
}
fn cost_from_light<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> f32 {
    magic_light(l, x, y, z) - 0.5
}
fn name_at<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> &str {
    &l.facts().get(l.state(x, y, z)).name
}
fn fluid_at<L: NaturalLevel>(l: &L, x: i32, y: i32, z: i32) -> crate::blocks::Fluid {
    l.facts().get(l.state(x, y, z)).fluid
}
/// `checkSpawnRules(level, NATURAL)` of a freshly created mob at a block.
pub(crate) fn mob_spawn_rules<L: NaturalLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    use crate::blocks::Fluid;
    let value = match walk_target(s) {
        WalkTarget::NotPathfinder | WalkTarget::Zero => 0.0,
        WalkTarget::Animal => {
            if name_at(l, x, y - 1, z) == "minecraft:grass_block" {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Mooshroom => {
            if name_at(l, x, y - 1, z) == "minecraft:mycelium" {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Turtle => {
            if fluid_at(l, x, y, z) == Fluid::Water
                || l.block_tag("minecraft:sand", name_at(l, x, y - 1, z))
            {
                10.0
            } else {
                cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Monster => -cost_from_light(l, x, y, z),
        WalkTarget::Giant => cost_from_light(l, x, y, z),
        WalkTarget::Guardian => {
            if fluid_at(l, x, y, z) == Fluid::Water {
                10.0 + cost_from_light(l, x, y, z)
            } else {
                -cost_from_light(l, x, y, z)
            }
        }
        WalkTarget::Silverfish => {
            if infested_host(name_at(l, x, y - 1, z)) {
                10.0
            } else {
                -cost_from_light(l, x, y, z)
            }
        }
        // A new strider is not in lava yet.
        WalkTarget::Strider => {
            if fluid_at(l, x, y, z) == Fluid::Lava {
                10.0
            } else {
                0.0
            }
        }
        // A new hoglin remembers no repellent.
        WalkTarget::Hoglin => {
            if name_at(l, x, y - 1, z) == "minecraft:crimson_nylium" {
                10.0
            } else {
                0.0
            }
        }
        WalkTarget::Bee => {
            if l.facts().is_air(l.state(x, y, z)) {
                10.0
            } else {
                0.0
            }
        }
        WalkTarget::HappyGhast => {
            let air = |dy| l.facts().is_air(l.state(x, y + dy, z));
            if !air(0) {
                0.0
            } else if air(-1) && !air(-2) {
                10.0
            } else {
                5.0
            }
        }
    };
    value >= 0.0
}
/// `InfestedBlock.isCompatibleHostBlock`.
pub fn infested_host(block: &str) -> bool {
    matches!(
        block,
        "minecraft:stone"
            | "minecraft:cobblestone"
            | "minecraft:stone_bricks"
            | "minecraft:mossy_stone_bricks"
            | "minecraft:cracked_stone_bricks"
            | "minecraft:chiseled_stone_bricks"
            | "minecraft:deepslate"
    )
}
/// `LevelReader.containsAnyLiquid`: the blocks from the box's minimum corner
/// to the block holding its maximum corner.
pub fn contains_any_liquid<L: SpawnLevel>(l: &L, b: Aabb) -> bool {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (x0, y0, z0) = (
        lo.x.floor() as i32,
        lo.y.floor() as i32,
        lo.z.floor() as i32,
    );
    let (x1, y1, z1) = (
        hi.x.floor() as i32,
        hi.y.floor() as i32,
        hi.z.floor() as i32,
    );
    (x0..=x1).any(|x| {
        (y0..=y1).any(|y| {
            (z0..=z1).any(|z| l.facts().get(l.state(x, y, z)).fluid != crate::blocks::Fluid::None)
        })
    })
}
/// `EntityDimensions.makeBoundingBox`: the half width is a float.
pub fn spawn_box(x: f64, y: f64, z: f64, width: f32, height: f32) -> Aabb {
    let half = f64::from(width / 2.0);
    Aabb::new(
        Vec3::new(x - half, y, z - half),
        Vec3::new(x + half, y + f64::from(height), z + half),
    )
}
/// `EntityType.spawnDimensionsScale`: cube mobs are checked for room at
/// their largest spawn size.
fn spawn_dimensions_scale(s: Species) -> f32 {
    match s {
        Species::Slime | Species::MagmaCube => 4.0,
        Species::SulfurCube => 2.0,
        _ => 1.0,
    }
}
/// `EntityType.getSpawnAABB`.
pub fn spawn_aabb(s: Species, x: f64, y: f64, z: f64, width: f32, height: f32) -> Aabb {
    let scale = spawn_dimensions_scale(s);
    let half = f64::from(scale * width / 2.0);
    Aabb::new(
        Vec3::new(x - half, y, z - half),
        Vec3::new(x + half, y + f64::from(scale * height), z + half),
    )
}
/// `checkSpawnObstruction` of a freshly created mob with the given box.
pub(crate) fn spawn_obstruction<L: NaturalLevel>(
    s: Species,
    l: &L,
    bounds: Aabb,
    spawned: &[Aabb],
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    let unobstructed =
        |l: &L| l.unobstructed(bounds) && !spawned.iter().any(|b| b.intersects(bounds));
    match obstruction(s) {
        Obstruction::NoLiquidUnobstructed => !contains_any_liquid(l, bounds) && unobstructed(l),
        Obstruction::Unobstructed => unobstructed(l),
        Obstruction::NoLiquid => !contains_any_liquid(l, bounds),
        Obstruction::Ocelot => {
            if unobstructed(l) && !contains_any_liquid(l, bounds) {
                if y < l.sea_level() {
                    return false;
                }
                let below = name_at(l, x, y - 1, z);
                if below == "minecraft:grass_block" || l.block_tag("minecraft:leaves", below) {
                    return true;
                }
            }
            false
        }
    }
}

/// A structure's spawn override for one category.
#[derive(Clone, Debug)]
pub struct Override {
    /// `bounding_box: piece` (else the whole structure's box).
    pub piece: bool,
    pub spawns: Vec<SpawnEntry>,
}
/// Every structure's `spawn_overrides`, by structure id then category.
#[derive(Clone, Debug, Default)]
pub struct StructureSpawns {
    /// Fx-hashed: natural spawning asks for every structure reaching every
    /// position it tries.
    pub overrides: FxHashMap<String, FxHashMap<String, Override>>,
}
impl StructureSpawns {
    /// The overrides of the resource root first asked for, loaded once.
    pub fn shared(root: &Path) -> &'static Self {
        static SHARED: std::sync::OnceLock<StructureSpawns> = std::sync::OnceLock::new();
        SHARED.get_or_init(|| Self::load(root))
    }
    pub fn load(root: &Path) -> Self {
        let mut overrides = FxHashMap::default();
        let dir = root.join("data/minecraft/worldgen/structure");
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(v) = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            else {
                continue;
            };
            let map: FxHashMap<String, Override> = v["spawn_overrides"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(category, o)| {
                    (
                        category.clone(),
                        Override {
                            piece: o["bounding_box"].as_str() == Some("piece"),
                            spawns: parse_entries(&o["spawns"]),
                        },
                    )
                })
                .collect();
            if !map.is_empty() {
                overrides.insert(format!("minecraft:{stem}"), map);
            }
        }
        Self { overrides }
    }
    pub fn get(&self, structure: &str, category: &str) -> Option<&Override> {
        self.overrides.get(structure)?.get(category)
    }
}

/// A structure whose references reach a position's chunk.
#[derive(Clone, Debug)]
pub struct StructureAt {
    pub id: String,
    /// A piece of one of its starts holds the position.
    pub in_piece: bool,
    /// One of its starts' bounding boxes holds the position.
    pub in_bounds: bool,
}

/// `NetherFortressStructure.FORTRESS_ENEMIES`.
pub fn fortress_enemies() -> &'static [SpawnEntry] {
    static ENEMIES: std::sync::LazyLock<Vec<SpawnEntry>> = std::sync::LazyLock::new(|| {
        let e = |entity: &str, weight, min, max, constant| SpawnEntry {
            entity: entity.to_owned(),
            weight,
            min,
            max,
            constant,
        };
        vec![
            e("minecraft:blaze", 10, 2, 3, false),
            e("minecraft:zombified_piglin", 5, 4, 4, true),
            e("minecraft:wither_skeleton", 8, 5, 5, true),
            e("minecraft:skeleton", 2, 5, 5, true),
            e("minecraft:magma_cube", 3, 4, 4, true),
        ]
    });
    &ENEMIES
}

/// `NaturalSpawner.mobsAt`: the fortress bounds on nether bricks, then
/// `ChunkGenerator.getMobsAt` (the first referenced structure whose override
/// for the category holds the position), else the biome's list. Borrowed from
/// the level: a spawn attempt looks it up up to twice.
pub fn mobs_at<L: NaturalLevel>(
    l: &L,
    category: Category,
    x: i32,
    y: i32,
    z: i32,
) -> &[SpawnEntry] {
    // Only a fortress or a structure overriding this category's spawns can
    // change the list, so only theirs are looked up (the references reaching
    // most chunks are mineshafts, which override nothing).
    let overrides = l.structure_spawns();
    let monster = category == Category::Monster;
    let structures = l.structures_at(x, y, z, &|id| {
        (monster && id == "minecraft:fortress") || overrides.get(id, category.name()).is_some()
    });
    if category == Category::Monster
        && name_at(l, x, y - 1, z) == "minecraft:nether_bricks"
        && structures
            .iter()
            .any(|s| s.id == "minecraft:fortress" && s.in_bounds)
    {
        return fortress_enemies();
    }
    for s in &structures {
        if let Some(o) = overrides.get(&s.id, category.name())
            && (if o.piece { s.in_piece } else { s.in_bounds })
        {
            return &o.spawns;
        }
    }
    l.biome_spawns(x, y, z)
        .and_then(|b| b.get(category.name()))
        .map_or(&[], Vec::as_slice)
}

/// `WeightedList.getRandom`.
pub fn weighted_pick(list: &[SpawnEntry], random: &mut impl RandomSource) -> Option<SpawnEntry> {
    let total: i64 = list.iter().map(|e| i64::from(e.weight)).sum();
    if total <= 0 {
        return None;
    }
    let mut selection = i64::from(random.next_int_bound(total as i32).ok()?);
    for e in list {
        selection -= i64::from(e.weight);
        if selection < 0 {
            return Some(e.clone());
        }
    }
    None
}

/// `SpawnerData.count().sample(random)`: a constant draws nothing.
pub fn sample_count(entry: &SpawnEntry, random: &mut impl RandomSource) -> u32 {
    if entry.constant {
        entry.min
    } else {
        let span = entry.max.saturating_sub(entry.min) as i32 + 1;
        entry.min + random.next_int_bound(span).unwrap_or(0) as u32
    }
}
/// `SpawnerData.equals`: the type and the count provider (the weight is not
/// part of it; a constant and a one-value uniform differ).
fn same_spawner(a: &SpawnEntry, b: &SpawnEntry) -> bool {
    a.entity == b.entity && a.constant == b.constant && a.min == b.min && a.max == b.max
}

/// An entity counted by `NaturalSpawner.createState`.
#[derive(Clone, Debug)]
pub struct Counted<'a> {
    pub entity: &'a str,
    pub category: Category,
    /// `blockPosition()`.
    pub block: [i32; 3],
    pub is_mob: bool,
    /// `isPersistenceRequired() || requiresCustomPersistence()`.
    pub persistent: bool,
}
/// A player as the spawn caps see it.
#[derive(Clone, Copy, Debug)]
pub struct SpawnPlayer {
    pub position: [f64; 3],
    pub spectator: bool,
}

/// `NaturalSpawner.SpawnState` with its `PotentialCalculator` and
/// `LocalMobCapCalculator`.
#[derive(Clone, Debug)]
pub struct SpawnState {
    spawnable_chunks: i32,
    counts: [i32; 8],
    charges: Vec<([i32; 3], f64)>,
    players: Vec<SpawnPlayer>,
    players_near: FxHashMap<(i32, i32), Vec<usize>>,
    player_counts: FxHashMap<usize, [i32; 8]>,
    last_checked: Option<([i32; 3], String, f64)>,
}
impl SpawnState {
    /// `NaturalSpawner.createState`: every non-persistent entity of a
    /// spawning category in a loaded chunk adds its biome's spawn charge, its
    /// category count and (for mobs) the local caps of the players near it.
    pub fn new<L: NaturalLevel>(
        l: &L,
        spawnable_chunks: i32,
        players: Vec<SpawnPlayer>,
        entities: &[Counted],
        chunk_loaded: impl Fn(i32, i32) -> bool,
    ) -> Self {
        let mut state = Self {
            spawnable_chunks,
            counts: [0; 8],
            charges: Vec::new(),
            players,
            players_near: FxHashMap::default(),
            player_counts: FxHashMap::default(),
            last_checked: None,
        };
        for e in entities {
            if e.is_mob && e.persistent || e.category == Category::Misc {
                continue;
            }
            let [x, y, z] = e.block;
            if !chunk_loaded(x >> 4, z >> 4) {
                continue;
            }
            if let Some(cost) = l.spawn_cost(e.entity, x, y, z) {
                state.add_charge(e.block, cost.charge);
            }
            if e.is_mob {
                state.add_local(x >> 4, z >> 4, e.category);
            }
            state.counts[e.category.index()] += 1;
        }
        state
    }
    fn add_charge(&mut self, pos: [i32; 3], charge: f64) {
        if charge != 0.0 {
            self.charges.push((pos, charge));
        }
    }
    /// `ChunkMap.getPlayersCloseForSpawning`: none past the natural spawn
    /// chunk tracker's range of 8 chunks from every player, else the
    /// non-spectators within 128 blocks of the chunk's centre.
    fn players_near<'a>(
        near: &'a mut FxHashMap<(i32, i32), Vec<usize>>,
        players: &[SpawnPlayer],
        cx: i32,
        cz: i32,
    ) -> &'a [usize] {
        near.entry((cx, cz)).or_insert_with(|| {
            let tracked = players.iter().any(|p| {
                let (px, pz) = (
                    (p.position[0].floor() as i32) >> 4,
                    (p.position[2].floor() as i32) >> 4,
                );
                (px - cx).abs().max((pz - cz).abs()) <= 8
            });
            if !tracked {
                return Vec::new();
            }
            let (x, z) = (f64::from(cx * 16 + 8), f64::from(cz * 16 + 8));
            players
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    let (dx, dz) = (x - p.position[0], z - p.position[2]);
                    !p.spectator && dx * dx + dz * dz < 16384.0
                })
                .map(|(i, _)| i)
                .collect()
        })
    }
    fn add_local(&mut self, cx: i32, cz: i32, category: Category) {
        for &p in Self::players_near(&mut self.players_near, &self.players, cx, cz) {
            self.player_counts.entry(p).or_insert([0; 8])[category.index()] += 1;
        }
    }
    /// `canSpawnForCategoryGlobal`.
    pub fn can_spawn_global(&self, category: Category) -> bool {
        let max = category.max() * self.spawnable_chunks / 289;
        self.counts[category.index()] < max
    }
    /// `canSpawnForCategoryLocal` (`LocalMobCapCalculator.canSpawn`).
    pub fn can_spawn_local(&mut self, category: Category, cx: i32, cz: i32) -> bool {
        let near = Self::players_near(&mut self.players_near, &self.players, cx, cz);
        near.iter().any(|p| {
            self.player_counts
                .get(p)
                .is_none_or(|c| c[category.index()] < category.max())
        })
    }
    /// `getFilteredSpawningCategories`.
    pub fn filtered_categories(
        &self,
        spawn_enemies: bool,
        spawn_persistent: bool,
    ) -> Vec<Category> {
        Category::SPAWNING
            .into_iter()
            .filter(|&c| {
                (spawn_enemies || c.friendly())
                    && (spawn_persistent || !c.persistent())
                    && self.can_spawn_global(c)
            })
            .collect()
    }
    /// `SpawnState.canSpawn`: the spawn potential's energy change must stay
    /// within the type's budget where it has a spawn cost.
    fn can_spawn<L: NaturalLevel>(&mut self, l: &L, entity: &str, pos: [i32; 3]) -> bool {
        match l.spawn_cost(entity, pos[0], pos[1], pos[2]) {
            None => {
                self.last_checked = Some((pos, entity.to_owned(), 0.0));
                true
            }
            Some(cost) => {
                self.last_checked = Some((pos, entity.to_owned(), cost.charge));
                self.energy_change(pos, cost.charge) <= cost.energy_budget
            }
        }
    }
    /// `PotentialCalculator.getPotentialEnergyChange`.
    fn energy_change(&self, pos: [i32; 3], charge: f64) -> f64 {
        if charge == 0.0 {
            return 0.0;
        }
        let mut change = 0.0;
        for (p, c) in &self.charges {
            let d = [
                f64::from(p[0] - pos[0]),
                f64::from(p[1] - pos[1]),
                f64::from(p[2] - pos[2]),
            ];
            let dist = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            change += if dist == 0.0 {
                f64::INFINITY
            } else {
                c / dist.sqrt()
            };
        }
        change * charge
    }
    /// `SpawnState.afterSpawn`.
    fn after_spawn<L: NaturalLevel>(
        &mut self,
        l: &L,
        entity: &str,
        category: Category,
        pos: [i32; 3],
    ) {
        let charge = match &self.last_checked {
            Some((p, t, c)) if *p == pos && t == entity => *c,
            _ => l
                .spawn_cost(entity, pos[0], pos[1], pos[2])
                .map_or(0.0, |c| c.charge),
        };
        self.add_charge(pos, charge);
        self.counts[category.index()] += 1;
        self.add_local(pos[0] >> 4, pos[2] >> 4, category);
    }
}

/// What a spawn pass did, in vanilla's order (for differential tests).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// `getFilteredSpawningCategories`.
    Categories(Vec<Category>),
    /// `canSpawnForCategoryLocal` for the chunk.
    Local(Category, bool),
    /// `getRandomPosWithin`'s start.
    Start(Category, [i32; 3]),
    /// An attempt at a position (the nearest player lookup).
    Try(i32, i32),
    /// The type passed `isValidSpawnPostitionForType`; the spawn state's
    /// potential test answered.
    Test(String, [i32; 3], bool),
    /// A mob spawned.
    Spawn(String, [f64; 3], f32),
}
/// A mob the spawner is about to add: the level runs `finalizeSpawn` and
/// adds it, returning its finalized box and whether a tropical fish schools.
#[derive(Clone, Debug)]
pub struct SpawnRequest<'a> {
    pub entity: &'a str,
    pub species: Species,
    pub position: [f64; 3],
    pub yaw: f32,
    /// Mobs of this group spawned before (the `SpawnGroupData`'s size).
    pub group_index: u32,
    pub reason: Reason,
}
#[derive(Clone, Copy, Debug)]
pub struct Spawned {
    pub bounds: Aabb,
    /// `TropicalFish.isSchool`.
    pub school: bool,
}

/// `NaturalSpawner.spawnForChunk` for the categories already filtered.
#[allow(clippy::too_many_arguments)]
pub fn spawn_for_chunk<L: NaturalLevel, R: RandomSource>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    categories: &[Category],
    chunk: (i32, i32),
    spawn: &mut dyn FnMut(&L, &SpawnRequest) -> Option<Spawned>,
    trace: &mut dyn FnMut(Event),
) {
    for &category in categories {
        let local = state.can_spawn_local(category, chunk.0, chunk.1);
        trace(Event::Local(category, local));
        if local {
            spawn_category_for_chunk(l, random, state, category, chunk, spawn, trace);
        }
    }
}
/// `NaturalSpawner.spawnCategoryForChunk`: `getRandomPosWithin` (a column of
/// the chunk, a height up to one above the world surface).
pub fn spawn_category_for_chunk<L: NaturalLevel, R: RandomSource>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    category: Category,
    chunk: (i32, i32),
    spawn: &mut dyn FnMut(&L, &SpawnRequest) -> Option<Spawned>,
    trace: &mut dyn FnMut(Event),
) {
    let x = chunk.0 * 16 + random.next_int_bound(16).unwrap_or(0);
    let z = chunk.1 * 16 + random.next_int_bound(16).unwrap_or(0);
    let top = l.heightmap(Heightmap::WorldSurface, x, z) + 1;
    let min = l.min_y();
    let y = min + random.next_int_bound(top - min + 1).unwrap_or(0);
    trace(Event::Start(category, [x, y, z]));
    if y > min {
        spawn_category_for_position(l, random, state, category, chunk, [x, y, z], spawn, trace);
    }
}
/// `NaturalSpawner.spawnCategoryForPosition` with the spawn state's
/// `canSpawn` and `afterSpawn`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_category_for_position<L: NaturalLevel, R: RandomSource>(
    l: &L,
    random: &mut R,
    state: &mut SpawnState,
    category: Category,
    chunk: (i32, i32),
    start: [i32; 3],
    spawn: &mut dyn FnMut(&L, &SpawnRequest) -> Option<Spawned>,
    trace: &mut dyn FnMut(Event),
) {
    let [x0, y, z0] = start;
    if l.mob_data().state(l.state(x0, y, z0)).flags & super::data::REDSTONE_CONDUCTOR != 0 {
        return;
    }
    let mut spawned: Vec<Aabb> = Vec::new();
    let mut cluster = 0u32;
    let mut nearest = Nearest::new(start);
    for _ in 0..3 {
        let (mut x, mut z) = (x0, z0);
        // The group's entry, its type and where it was picked.
        let mut current: Option<(SpawnEntry, Option<MobType>, [i32; 3])> = None;
        let mut group_index = 0u32;
        let mut max = (random.next_float() * 4.0).ceil() as i32;
        let mut ll = 0;
        while ll < max {
            ll += 1;
            x += random.next_int_bound(6).unwrap_or(0) - random.next_int_bound(6).unwrap_or(0);
            z += random.next_int_bound(6).unwrap_or(0) - random.next_int_bound(6).unwrap_or(0);
            let (xx, zz) = (f64::from(x) + 0.5, f64::from(z) + 0.5);
            trace(Event::Try(x, z));
            let Some(distance) = nearest.distance_sqr(l, xx, f64::from(y), zz) else {
                continue;
            };
            if !right_distance(l, chunk, [x, y, z], distance) {
                continue;
            }
            if current.is_none() {
                let list = if category == Category::WaterAmbient
                    && l.biome_tag("minecraft:reduce_water_ambient_spawns", x, y, z)
                    && random.next_float() < 0.98
                {
                    None
                } else {
                    weighted_pick(mobs_at(l, category, x, y, z), random)
                };
                let Some(entry) = list else {
                    break;
                };
                max = sample_count(&entry, random) as i32;
                let kind = l.mob_type(&entry.entity);
                current = Some((entry, kind, [x, y, z]));
            }
            let (entry, kind, picked_at) = current.as_ref().unwrap_or_else(|| unreachable!());
            // The list the entry was picked from holds it where it was picked.
            let listed = *picked_at == [x, y, z];
            let valid = valid_position_for_type(
                l,
                random,
                category,
                entry,
                *kind,
                listed,
                [x, y, z],
                distance,
            );
            let Some((species, width, height, type_category)) = (*kind).filter(|_| valid) else {
                continue;
            };
            let passed = state.can_spawn(l, &entry.entity, [x, y, z]);
            trace(Event::Test(entry.entity.clone(), [x, y, z], passed));
            if !passed {
                continue;
            }
            // getMobForSpawn (a type the game cannot create, which would end
            // the pass, never passes isValidSpawnPostitionForType).
            let yaw = random.next_float() * 360.0;
            let bounds = spawn_box(xx, f64::from(y), zz, width, height);
            let despawn = f64::from(type_category.despawn_distance().pow(2));
            let far = distance > despawn && removed_when_far(species);
            if far
                || !mob_spawn_rules(species, l, x, y, z)
                || !spawn_obstruction(species, l, bounds, &spawned, x, y, z)
            {
                continue;
            }
            let request = SpawnRequest {
                entity: &entry.entity,
                species,
                position: [xx, f64::from(y), zz],
                yaw,
                group_index,
                reason: Reason::Natural,
            };
            let Some(result) = spawn(l, &request) else {
                continue;
            };
            group_index += 1;
            spawned.push(result.bounds);
            cluster += 1;
            state.after_spawn(l, &entry.entity, type_category, [x, y, z]);
            trace(Event::Spawn(
                entry.entity.clone(),
                [xx, f64::from(y), zz],
                yaw,
            ));
            if cluster >= max_cluster_size(species) {
                return;
            }
            // TropicalFish.isMaxGroupSizeReached.
            if species == Species::TropicalFish && !result.school {
                break;
            }
        }
    }
}
/// `getNearestPlayer(x, y, z, -1, false)`'s squared distance for the
/// attempts of one start. The spawn checks only compare it with 24² and the
/// despawn distances (128² at most), so it is measured only to the players
/// that can be within 128 blocks of an attempt near the start: when none is,
/// the nearest reads as infinitely far, which every check answers as it
/// answers the real distance. An attempt that wanders farther than
/// `MARGIN` from the start measures every player.
struct Nearest {
    /// The players that can be within 128 blocks, once an attempt asked.
    nearby: Option<Vec<[f64; 3]>>,
    start: (f64, f64),
}
impl Nearest {
    const REACH: f64 = 128.0;
    const MARGIN: f64 = 64.0;
    fn new(start: [i32; 3]) -> Self {
        Self {
            nearby: None,
            start: (f64::from(start[0]) + 0.5, f64::from(start[2]) + 0.5),
        }
    }
    fn distance_sqr<L: NaturalLevel>(&mut self, l: &L, x: f64, y: f64, z: f64) -> Option<f64> {
        let all = l.nearest_player_positions();
        if all.is_empty() {
            return None;
        }
        let distance = |p: &[f64; 3]| (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
        let (sx, sz) = self.start;
        if (x - sx).abs() > Self::MARGIN || (z - sz).abs() > Self::MARGIN {
            return all.iter().map(distance).reduce(f64::min);
        }
        // The others are farther than REACH, across, from every point within
        // MARGIN of the start.
        let nearby = self.nearby.get_or_insert_with(|| {
            all.iter()
                .filter(|p| {
                    let dx = ((p[0] - sx).abs() - Self::MARGIN).max(0.0);
                    let dz = ((p[2] - sz).abs() - Self::MARGIN).max(0.0);
                    dx * dx + dz * dz <= Self::REACH * Self::REACH
                })
                .copied()
                .collect()
        });
        Some(nearby.iter().map(distance).fold(f64::INFINITY, f64::min))
    }
}
/// `isRightDistanceToPlayerAndSpawnPoint`.
fn right_distance<L: NaturalLevel>(l: &L, chunk: (i32, i32), pos: [i32; 3], distance: f64) -> bool {
    if distance <= 576.0 {
        return false;
    }
    if let Some(r) = l.respawn() {
        let d = [
            f64::from(r[0]) + 0.5 - (f64::from(pos[0]) + 0.5),
            f64::from(r[1]) + 0.5 - f64::from(pos[1]),
            f64::from(r[2]) + 0.5 - (f64::from(pos[2]) + 0.5),
        ];
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 24.0 * 24.0 {
            return false;
        }
    }
    let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
    (cx, cz) == chunk || l.can_spawn_in_chunk(cx, cz)
}
/// `isValidSpawnPostitionForType`, for the entry's type (`mob_type`).
/// `listed`: the position is where the entry was picked from `mobsAt`.
#[allow(clippy::too_many_arguments)]
fn valid_position_for_type<L: NaturalLevel, R: RandomSource>(
    l: &L,
    random: &mut R,
    category: Category,
    entry: &SpawnEntry,
    kind: Option<MobType>,
    listed: bool,
    pos: [i32; 3],
    distance: f64,
) -> bool {
    let Some((species, width, height, type_category)) = kind else {
        return false;
    };
    if type_category == Category::Misc {
        return false;
    }
    let despawn = f64::from(type_category.despawn_distance().pow(2));
    if !spawns_far_from_player(species, type_category) && distance > despawn {
        return false;
    }
    let [x, y, z] = pos;
    if !listed
        && !mobs_at(l, category, x, y, z)
            .iter()
            .any(|e| same_spawner(e, entry))
    {
        return false;
    }
    if !spawn_rules::is_spawn_position_ok(species, l, x, y, z) {
        return false;
    }
    if !spawn_rules::check_spawn_rules(species, l, Reason::Natural, random, x, y, z) {
        return false;
    }
    l.no_collision(spawn_aabb(
        species,
        f64::from(x) + 0.5,
        f64::from(y),
        f64::from(z) + 0.5,
        width,
        height,
    ))
}

/// What chunk generation spawning did (for differential tests).
#[derive(Clone, Debug, PartialEq)]
pub enum GenEvent {
    /// An attempt at a column (the heightmap lookup).
    Column(i32, i32),
    Spawn(String, [f64; 3], f32),
}
/// `NaturalSpawner.spawnMobsForChunkGeneration`: creature packs placed on
/// the chunk's surface. `rules` is the level's own random, which the spawn
/// predicates draw from.
#[allow(clippy::too_many_arguments)]
pub fn spawn_mobs_for_chunk_generation<L: NaturalLevel, R: RandomSource, Q: RandomSource>(
    l: &L,
    source: [i32; 3],
    chunk: (i32, i32),
    random: &mut R,
    rules: &mut Q,
    spawn: &mut dyn FnMut(&L, &SpawnRequest) -> Option<Spawned>,
    trace: &mut dyn FnMut(GenEvent),
) {
    if !l.spawn_mobs() {
        return;
    }
    let [sx, sy, sz] = source;
    let mobs = l
        .biome_spawns(sx, sy, sz)
        .and_then(|b| b.get("creature"))
        .cloned()
        .unwrap_or_default();
    if mobs.iter().map(|e| e.weight).sum::<u32>() == 0 {
        return;
    }
    let probability = l.creature_probability(sx, sy, sz);
    let (xo, zo) = (chunk.0 * 16, chunk.1 * 16);
    while random.next_float() < probability {
        let Some(entry) = weighted_pick(&mobs, random) else {
            continue;
        };
        let count = sample_count(&entry, random);
        let mut group_index = 0u32;
        let mut spawned: Vec<Aabb> = Vec::new();
        let mut x = xo + random.next_int_bound(16).unwrap_or(0);
        let mut z = zo + random.next_int_bound(16).unwrap_or(0);
        let (start_x, start_z) = (x, z);
        let kind = l.mob_type(&entry.entity);
        for _ in 0..count {
            let mut success = false;
            let mut attempts = 0;
            while !success && attempts < 4 {
                attempts += 1;
                let Some((species, width, height, _)) = kind else {
                    break;
                };
                let pos = top_non_colliding(l, &entry.entity, x, z);
                trace(GenEvent::Column(x, z));
                if spawn_rules::is_spawn_position_ok(species, l, pos[0], pos[1], pos[2]) {
                    let fx = f64::from(x).clamp(
                        f64::from(xo) + f64::from(width),
                        f64::from(xo) + 16.0 - f64::from(width),
                    );
                    let fz = f64::from(z).clamp(
                        f64::from(zo) + f64::from(width),
                        f64::from(zo) + 16.0 - f64::from(width),
                    );
                    let fy = f64::from(pos[1]);
                    let (bx, bz) = (fx.floor() as i32, fz.floor() as i32);
                    if !l.no_collision(spawn_aabb(species, fx, fy, fz, width, height))
                        || !spawn_rules::check_spawn_rules(
                            species,
                            l,
                            Reason::ChunkGeneration,
                            rules,
                            bx,
                            pos[1],
                            bz,
                        )
                    {
                        continue;
                    }
                    let yaw = random.next_float() * 360.0;
                    let bounds = spawn_box(fx, fy, fz, width, height);
                    if mob_spawn_rules(species, l, bx, pos[1], bz)
                        && spawn_obstruction(species, l, bounds, &spawned, bx, pos[1], bz)
                    {
                        let request = SpawnRequest {
                            entity: &entry.entity,
                            species,
                            position: [fx, fy, fz],
                            yaw,
                            group_index,
                            reason: Reason::ChunkGeneration,
                        };
                        if let Some(result) = spawn(l, &request) {
                            group_index += 1;
                            spawned.push(result.bounds);
                            trace(GenEvent::Spawn(entry.entity.clone(), [fx, fy, fz], yaw));
                            success = true;
                        }
                    }
                }
                x += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
                z += random.next_int_bound(5).unwrap_or(0) - random.next_int_bound(5).unwrap_or(0);
                while x < xo || x >= xo + 16 || z < zo || z >= zo + 16 {
                    x = start_x + random.next_int_bound(5).unwrap_or(0)
                        - random.next_int_bound(5).unwrap_or(0);
                    z = start_z + random.next_int_bound(5).unwrap_or(0)
                        - random.next_int_bound(5).unwrap_or(0);
                }
            }
        }
    }
}
/// `getTopNonCollidingPos`: the type's heightmap, under the ceiling in
/// dimensions that have one, then the placement type's adjustment.
fn top_non_colliding<L: NaturalLevel>(l: &L, entity: &str, x: i32, z: i32) -> [i32; 3] {
    let heightmap = Heightmap::from_name(l.mob_data().heightmap(entity));
    let mut y = l.heightmap(heightmap, x, z);
    if l.has_ceiling() {
        loop {
            y -= 1;
            if l.facts().is_air(l.state(x, y, z)) {
                break;
            }
        }
        loop {
            y -= 1;
            if !(l.facts().is_air(l.state(x, y, z)) && y > l.min_y()) {
                break;
            }
        }
    }
    if l.mob_data().placement(entity) == super::data::Placement::OnGround
        && l.pathfindable_land(x, y - 1, z)
    {
        y -= 1;
    }
    [x, y, z]
}
