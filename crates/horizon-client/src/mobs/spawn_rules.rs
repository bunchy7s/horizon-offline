//! `SpawnPlacements`: each entity type's placement type
//! (`SpawnPlacementTypes.isSpawnPositionOk`) and spawn predicate
//! (`checkSpawnRules`), read through [`SpawnLevel`] so the game's world and a
//! differential test world answer the same questions. Random draws follow
//! vanilla's order.
use super::{Species, data};
use crate::blocks::{BlockFacts, Fluid};
use horizon_core::random::{RandomSource, slime_chunk_random};
use horizon_world::registry::BlockStateId;

/// `EntitySpawnReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Natural,
    ChunkGeneration,
    Spawner,
    TrialSpawner,
    Reinforcement,
    Patrol,
    Structure,
    Event,
    Command,
    SpawnItemUse,
    MobSummoned,
    Conversion,
    Breeding,
    Triggered,
    Jockey,
    Dispenser,
    Bucket,
    Load,
    DimensionTravel,
}
impl Reason {
    pub fn is_spawner(self) -> bool {
        matches!(self, Self::Spawner | Self::TrialSpawner)
    }
    pub fn ignores_light(self) -> bool {
        self == Self::TrialSpawner
    }
}
/// The dimension type's monster spawn light rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    Overworld,
    Nether,
    End,
}
impl Dimension {
    /// `monster_spawn_block_light_limit`.
    fn block_light_limit(self) -> i32 {
        match self {
            Self::Nether => 15,
            _ => 0,
        }
    }
    /// `monster_spawn_light_level.sample`: uniform 0..=7 in the overworld.
    fn light_test(self, random: &mut impl RandomSource) -> i32 {
        match self {
            Self::Overworld => random.next_int_bound(8).unwrap_or(0),
            Self::Nether => 7,
            Self::End => 15,
        }
    }
}

/// What spawn checks read from a level.
pub trait SpawnLevel {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId;
    fn facts(&self) -> &BlockFacts;
    fn mob_data(&self) -> &data::MobData;
    fn block_tag(&self, tag: &str, block: &str) -> bool;
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool;
    /// `getBrightness(LightLayer.SKY / BLOCK)`.
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32;
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32;
    fn sky_darken(&self) -> i32;
    fn thundering(&self) -> bool;
    fn sea_level(&self) -> i32;
    /// `getHeight(WORLD_SURFACE, x, z)`: above the highest non-air block.
    fn world_surface(&self, x: i32, z: i32) -> i32;
    fn peaceful(&self) -> bool;
    fn dimension(&self) -> Dimension;
    /// The `gameplay/surface_slime_spawn_chance` environment attribute.
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32;
    fn world_seed(&self) -> i64;
    /// `getNearestPlayer(x, y, z, distance, true) != null`: a survival or
    /// adventure player that close.
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool;
}

fn name<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> &str {
    &l.facts().get(l.state(x, y, z)).name
}
fn fluid<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> Fluid {
    l.facts().get(l.state(x, y, z)).fluid
}
fn flags<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> u16 {
    l.mob_data().state(l.state(x, y, z)).flags
}
fn tag<L: SpawnLevel>(l: &L, tag: &str, x: i32, y: i32, z: i32) -> bool {
    l.block_tag(tag, name(l, x, y, z))
}
/// `getRawBrightness(pos, darkening)`.
fn raw_brightness<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32, darkening: i32) -> i32 {
    l.block_light(x, y, z).max(l.sky_light(x, y, z) - darkening)
}
fn can_see_sky<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    l.sky_light(x, y, z) >= 15
}
/// `LevelReader.canSeeSkyFromBelowWater`.
fn can_see_sky_from_below_water<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    let sea = l.sea_level();
    if y >= sea {
        return can_see_sky(l, x, y, z);
    }
    if !can_see_sky(l, x, sea, z) {
        return false;
    }
    ((y + 1)..sea).all(|yy| {
        let f = l.facts().get(l.state(x, yy, z));
        f.dampening == 0 || f.fluid != Fluid::None
    })
}

/// `EntityType.fireImmune`.
pub fn fire_immune(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Blaze
            | S::Ghast
            | S::MagmaCube
            | S::Shulker
            | S::Strider
            | S::Vex
            | S::Warden
            | S::Wither
            | S::WitherSkeleton
            | S::Zoglin
            | S::ZombifiedPiglin
    )
}
/// `EntityType.isAllowedInPeaceful`.
pub fn allowed_in_peaceful(s: Species) -> bool {
    use Species as S;
    !matches!(
        s,
        S::Blaze
            | S::Bogged
            | S::Breeze
            | S::CaveSpider
            | S::Creaking
            | S::Creeper
            | S::Drowned
            | S::ElderGuardian
            | S::Enderman
            | S::Endermite
            | S::Evoker
            | S::Ghast
            | S::Giant
            | S::Guardian
            | S::Hoglin
            | S::Husk
            | S::Illusioner
            | S::MagmaCube
            | S::Parched
            | S::Phantom
            | S::PiglinBrute
            | S::Pillager
            | S::Ravager
            | S::Silverfish
            | S::Skeleton
            | S::Slime
            | S::Spider
            | S::Stray
            | S::Vex
            | S::Vindicator
            | S::Warden
            | S::Witch
            | S::Wither
            | S::WitherSkeleton
            | S::Zoglin
            | S::Zombie
            | S::ZombieVillager
            | S::ZombifiedPiglin
    )
}
/// The block tag an entity type is `immuneTo`.
fn immune_tag(s: Species) -> Option<&'static str> {
    use Species as S;
    match s {
        S::Fox => Some("minecraft:fox_immune_to"),
        S::PolarBear => Some("minecraft:polar_bear_immune_to"),
        S::SnowGolem => Some("minecraft:snow_golem_immune_to"),
        S::Stray => Some("minecraft:stray_immune_to"),
        S::Wither => Some("minecraft:wither_immune_to"),
        S::WitherSkeleton => Some("minecraft:wither_skeleton_immune_to"),
        _ => None,
    }
}
/// `BlockState.isValidSpawn(level, pos, type)` for the block at a position.
pub fn is_valid_spawn<L: SpawnLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    if name(l, x, y, z) == "minecraft:magma_block" {
        return fire_immune(s);
    }
    let bit = match s {
        Species::Ocelot | Species::Parrot => data::VALID_SPAWN_LEAVES,
        Species::PolarBear => data::VALID_SPAWN_ICE,
        _ => data::VALID_SPAWN,
    };
    flags(l, x, y, z) & bit != 0
}
/// `NaturalSpawner.isValidEmptySpawnBlock` for an entity type.
pub fn is_valid_empty_spawn<L: SpawnLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    // The exported flag is the zombie's answer; only blocks a zombie finds
    // dangerous can answer differently for another type.
    if flags(l, x, y, z) & data::VALID_EMPTY_SPAWN != 0 {
        return true;
    }
    let f = l.facts().get(l.state(x, y, z));
    let block = &*f.name;
    let burning = l.block_tag("minecraft:fire", block)
        || block == "minecraft:lava"
        || block == "minecraft:magma_block"
        || (block == "minecraft:campfire" || block == "minecraft:soul_campfire") && f.emission > 0
        || block == "minecraft:lava_cauldron";
    let other = matches!(
        block,
        "minecraft:wither_rose"
            | "minecraft:sweet_berry_bush"
            | "minecraft:cactus"
            | "minecraft:powder_snow"
    );
    if !burning && !other {
        return false;
    }
    let immune = immune_tag(s).is_some_and(|t| l.block_tag(t, block));
    let dangerous = !immune && (burning && !fire_immune(s) || other);
    !dangerous
        && !f.full_collision
        && f.fluid == Fluid::None
        && !l.block_tag("minecraft:prevent_mob_spawning_inside", block)
}
/// `SpawnPlacements.isSpawnPositionOk`.
pub fn is_spawn_position_ok<L: SpawnLevel>(s: Species, l: &L, x: i32, y: i32, z: i32) -> bool {
    match l.mob_data().species_placement(s) {
        data::Placement::OnGround => {
            is_valid_spawn(s, l, x, y - 1, z)
                && is_valid_empty_spawn(s, l, x, y, z)
                && is_valid_empty_spawn(s, l, x, y + 1, z)
        }
        data::Placement::InWater => {
            fluid(l, x, y, z) == Fluid::Water
                && flags(l, x, y + 1, z) & data::REDSTONE_CONDUCTOR == 0
        }
        data::Placement::InLava => fluid(l, x, y, z) == Fluid::Lava,
        data::Placement::NoRestrictions => true,
    }
}
/// `Monster.isDarkEnoughToSpawn`.
pub fn is_dark_enough<L: SpawnLevel>(
    l: &L,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    if l.sky_light(x, y, z) > random.next_int_bound(32).unwrap_or(0) {
        return false;
    }
    let dim = l.dimension();
    let limit = dim.block_light_limit();
    if limit < 15 && l.block_light(x, y, z) > limit {
        return false;
    }
    let darken = if l.thundering() { 10 } else { l.sky_darken() };
    let brightness = raw_brightness(l, x, y, z, darken);
    brightness <= dim.light_test(random)
}
/// `Animal.isBrightEnoughToSpawn`.
fn bright<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    raw_brightness(l, x, y, z, 0) > 8
}
/// `Mob.checkMobSpawnRules`.
fn mob_rules<L: SpawnLevel>(s: Species, l: &L, reason: Reason, x: i32, y: i32, z: i32) -> bool {
    reason.is_spawner() || is_valid_spawn(s, l, x, y - 1, z)
}
/// `Monster.checkMonsterSpawnRules`.
fn monster_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    (reason.ignores_light() || is_dark_enough(l, random, x, y, z))
        && mob_rules(s, l, reason, x, y, z)
}
/// `Animal.checkAnimalSpawnRules` with the given spawnable-on tag.
fn animal_rules<L: SpawnLevel>(l: &L, reason: Reason, on: &str, x: i32, y: i32, z: i32) -> bool {
    let bright = reason.ignores_light() || bright(l, x, y, z);
    tag(l, on, x, y - 1, z) && bright
}
/// `WaterAnimal.checkSurfaceWaterAnimalSpawnRules`.
fn surface_water<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> bool {
    let sea = l.sea_level();
    y >= sea - 13
        && y <= sea
        && fluid(l, x, y - 1, z) == Fluid::Water
        && name(l, x, y + 1, z) == "minecraft:water"
}

/// `SpawnPlacements.checkSpawnRules`.
pub fn check_spawn_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    use Species as S;
    if !allowed_in_peaceful(s) && l.peaceful() {
        return false;
    }
    let below = |l: &L| name(l, x, y - 1, z).to_owned();
    match s {
        S::Axolotl => tag(l, "minecraft:axolotls_spawnable_on", x, y - 1, z),
        S::Cod | S::Pufferfish | S::Salmon | S::Dolphin | S::Squid => surface_water(l, x, y, z),
        S::Drowned => {
            if fluid(l, x, y - 1, z) != Fluid::Water && !reason.is_spawner() {
                return false;
            }
            let frequent = l.biome_tag("minecraft:more_frequent_drowned_spawns", x, y, z);
            let can_spawn = !l.peaceful()
                && (reason.ignores_light() || is_dark_enough(l, random, x, y, z))
                && (reason.is_spawner() || fluid(l, x, y, z) == Fluid::Water);
            if !can_spawn || !reason.is_spawner() && reason != Reason::Reinforcement {
                if frequent {
                    random.next_int_bound(15).unwrap_or(1) == 0 && can_spawn
                } else {
                    random.next_int_bound(40).unwrap_or(1) == 0
                        && y < l.sea_level() - 5
                        && can_spawn
                }
            } else {
                true
            }
        }
        S::Guardian | S::ElderGuardian => {
            (random.next_int_bound(20).unwrap_or(1) == 0
                || !can_see_sky_from_below_water(l, x, y, z))
                && !l.peaceful()
                && (reason.is_spawner() || fluid(l, x, y, z) == Fluid::Water)
                && fluid(l, x, y - 1, z) == Fluid::Water
        }
        S::TropicalFish => {
            fluid(l, x, y - 1, z) == Fluid::Water
                && name(l, x, y + 1, z) == "minecraft:water"
                && (l.biome_tag(
                    "minecraft:allows_tropical_fish_spawns_at_any_height",
                    x,
                    y,
                    z,
                ) || surface_water(l, x, y, z))
        }
        S::Armadillo => {
            tag(l, "minecraft:armadillo_spawnable_on", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Bat => {
            if y >= l.world_surface(x, z) || random.next_boolean() {
                return false;
            }
            if raw_brightness(l, x, y, z, l.sky_darken()) > random.next_int_bound(4).unwrap_or(0) {
                return false;
            }
            tag(l, "minecraft:bats_spawnable_on", x, y - 1, z) && mob_rules(s, l, reason, x, y, z)
        }
        S::Blaze | S::Breeze | S::Zoglin => mob_rules(s, l, reason, x, y, z),
        S::Camel => tag(l, "minecraft:camels_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::CamelHusk | S::Husk | S::Parched => {
            monster_rules(s, l, reason, random, x, y, z)
                && (reason.is_spawner() || can_see_sky(l, x, y, z))
        }
        S::Chicken
        | S::Cow
        | S::Donkey
        | S::HappyGhast
        | S::Horse
        | S::Llama
        | S::Mule
        | S::Pig
        | S::Sheep
        | S::Cat
        | S::Panda
        | S::TraderLlama => animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z),
        S::Endermite | S::Silverfish => {
            if !mob_rules(s, l, reason, x, y, z) {
                return false;
            }
            reason.is_spawner()
                || !l.player_near(
                    f64::from(x) + 0.5,
                    f64::from(y) + 0.5,
                    f64::from(z) + 0.5,
                    5.0,
                )
        }
        S::IronGolem
        | S::SnowGolem
        | S::Villager
        | S::Phantom
        | S::Shulker
        | S::WanderingTrader => mob_rules(s, l, reason, x, y, z),
        S::Frog => tag(l, "minecraft:frogs_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Ghast => {
            !l.peaceful()
                && random.next_int_bound(20).unwrap_or(1) == 0
                && mob_rules(s, l, reason, x, y, z)
        }
        S::GlowSquid => {
            y <= l.sea_level() - 33
                && raw_brightness(l, x, y, z, 0) == 0
                && name(l, x, y, z) == "minecraft:water"
        }
        S::Goat => tag(l, "minecraft:goats_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::MagmaCube => !l.peaceful(),
        S::SulfurCube => true,
        S::Mooshroom => {
            tag(l, "minecraft:mooshrooms_spawnable_on", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Nautilus => {
            let sea = l.sea_level();
            y >= sea - 25
                && y <= sea - 5
                && fluid(l, x, y - 1, z) == Fluid::Water
                && name(l, x, y + 1, z) == "minecraft:water"
        }
        S::Ocelot => random.next_int_bound(3).unwrap_or(0) != 0,
        S::Parrot => tag(l, "minecraft:parrots_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Hoglin | S::Piglin => below(l) != "minecraft:nether_wart_block",
        S::Pillager => l.block_light(x, y, z) <= 8 && mob_rules(s, l, reason, x, y, z),
        S::PolarBear => {
            if l.biome_tag("minecraft:polar_bears_spawn_on_alternate_blocks", x, y, z) {
                bright(l, x, y, z)
                    && tag(
                        l,
                        "minecraft:polar_bears_spawnable_on_alternate",
                        x,
                        y - 1,
                        z,
                    )
            } else {
                animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z)
            }
        }
        S::Rabbit => tag(l, "minecraft:rabbits_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::SkeletonHorse => {
            if reason.is_spawner() {
                reason.ignores_light() || bright(l, x, y, z)
            } else {
                animal_rules(l, reason, "minecraft:animals_spawnable_on", x, y, z)
            }
        }
        S::Slime => slime_rules(s, l, reason, random, x, y, z),
        S::Stray => {
            let mut sky = y + 1;
            while name(l, x, sky, z) == "minecraft:powder_snow" {
                sky += 1;
            }
            monster_rules(s, l, reason, random, x, y, z)
                && (reason.is_spawner() || can_see_sky(l, x, sky - 1, z))
        }
        S::Strider => {
            let mut up = y + 1;
            while fluid(l, x, up, z) == Fluid::Lava {
                up += 1;
            }
            l.facts().is_air(l.state(x, up, z))
        }
        S::Turtle => {
            y < l.sea_level() + 4 && tag(l, "minecraft:sand", x, y - 1, z) && bright(l, x, y, z)
        }
        S::Wolf => tag(l, "minecraft:wolves_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::ZombifiedPiglin => !l.peaceful() && below(l) != "minecraft:nether_wart_block",
        S::Fox => tag(l, "minecraft:foxes_spawnable_on", x, y - 1, z) && bright(l, x, y, z),
        S::Bogged
        | S::CaveSpider
        | S::Creeper
        | S::Enderman
        | S::Giant
        | S::Skeleton
        | S::Spider
        | S::Witch
        | S::Wither
        | S::WitherSkeleton
        | S::Creaking
        | S::Zombie
        | S::ZombieHorse
        | S::ZombieVillager
        | S::Evoker
        | S::Illusioner
        | S::Ravager
        | S::Vex
        | S::Vindicator
        | S::Warden => monster_rules(s, l, reason, random, x, y, z),
        // Types without a SpawnPlacements registration.
        _ => true,
    }
}
/// `Slime.checkSlimeSpawnRules`.
fn slime_rules<L: SpawnLevel>(
    s: Species,
    l: &L,
    reason: Reason,
    random: &mut impl RandomSource,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    if l.peaceful() {
        return false;
    }
    if reason.is_spawner() {
        return mob_rules(s, l, reason, x, y, z);
    }
    if l.biome_tag("minecraft:allows_surface_slime_spawns", x, y, z) && y > 50 && y < 70 {
        let chance = l.surface_slime_chance(x, y, z);
        if random.next_float() < chance
            && raw_brightness(l, x, y, z, l.sky_darken()) <= random.next_int_bound(8).unwrap_or(0)
        {
            return mob_rules(s, l, reason, x, y, z);
        }
    }
    let slime_chunk = slime_chunk_random(x >> 4, z >> 4, l.world_seed(), 987_234_911)
        .next_int_bound(10)
        .unwrap_or(1)
        == 0;
    if random.next_int_bound(10).unwrap_or(1) == 0 && slime_chunk && y < 40 {
        return mob_rules(s, l, reason, x, y, z);
    }
    false
}

impl SpawnLevel for super::MobWorld<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.world.block(x, y, z)
    }
    fn facts(&self) -> &BlockFacts {
        self.facts
    }
    fn mob_data(&self) -> &data::MobData {
        self.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.has_tag(tag, block)
    }
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        let Some(chunk) = self.world.chunks.get(&(x >> 4, z >> 4)) else {
            return false;
        };
        let biome = &self.biomes.get(chunk.biome(x, y, z)).name;
        self.data
            .biome_tags
            .get(tag.trim_start_matches('#'))
            .is_some_and(|members| members.contains(biome))
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        if self.nether || self.end {
            return 0;
        }
        i32::from(self.world.light(x, y, z) >> 4)
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        i32::from(self.world.light(x, y, z) & 15)
    }
    fn sky_darken(&self) -> i32 {
        super::MobWorld::sky_darken(self)
    }
    fn thundering(&self) -> bool {
        self.thundering
    }
    fn sea_level(&self) -> i32 {
        self.sea_level
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |id| self.facts.is_air(id))
    }
    fn peaceful(&self) -> bool {
        self.difficulty.effective == 0.0
    }
    fn dimension(&self) -> Dimension {
        if self.nether {
            Dimension::Nether
        } else if self.end {
            Dimension::End
        } else {
            Dimension::Overworld
        }
    }
    fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
        let phase = self.day_time.div_euclid(24000).rem_euclid(8) as usize;
        0.5 * super::spawning::MOON_BRIGHTNESS[phase]
    }
    fn world_seed(&self) -> i64 {
        self.world_seed
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.player.as_ref().is_some_and(|p| {
            let d = (p.position.x - x).powi(2)
                + (p.position.y - y).powi(2)
                + (p.position.z - z).powi(2);
            p.alive && !p.creative && !p.spectator && d < distance * distance
        })
    }
}
