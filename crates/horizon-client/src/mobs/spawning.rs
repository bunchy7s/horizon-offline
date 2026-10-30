//! Natural spawning rules: `SpawnPlacements` placement types and each
//! species' spawn predicate (`checkAnimalSpawnRules`, `checkMonsterSpawnRules`,
//! water animals, bats...), and `finalizeSpawn` (variants, colours, babies,
//! equipment).
use super::{Mob, MobWorld, Species, farm::Climate};
use horizon_core::random::RandomSource;

/// Biome facts finalizeSpawn and the spawn rules read.
#[derive(Clone, Copy, Debug, Default)]
pub struct BiomeFacts {
    pub climate: Climate,
    /// Biome temperature at the position (strays in cold biomes, snow foxes).
    pub temperature: f32,
    pub snowy: bool,
    /// `#minecraft:spawns_snow_foxes` (`Fox.Variant.byBiome`: snow, else red).
    pub snow_fox: bool,
    pub swamp: bool,
    pub ocean: bool,
    pub mushroom: bool,
    pub desert: bool,
    pub jungle: bool,
    /// Frog variant from `spawns_warm/cold_variant_frogs` (0 temperate, 1 warm, 2 cold).
    pub frog: u8,
    /// Wolf variant index (`tame::WOLF_VARIANTS`).
    pub wolf: u8,
    /// `MoonBrightnessCheck(>= 0.9)`: the full moon.
    pub full_moon: bool,
    /// `spawns_coral_variant_zombie_nautilus` (the warm ocean).
    pub coral_nautilus: bool,
}
/// `SpawnPlacements.isSpawnPositionOk` for the species.
pub fn placement_ok(species: Species, w: &MobWorld, x: i32, y: i32, z: i32) -> bool {
    super::spawn_rules::is_spawn_position_ok(species, w, x, y, z)
}
/// Mob.finalizeSpawn for natural, chunk-generation and summoned mobs.
/// `DifficultyInstance`: the local difficulty at a position.
#[derive(Clone, Copy, Debug)]
pub struct LocalDifficulty {
    /// The world difficulty id, 0 peaceful to 3 hard.
    pub base: u8,
    pub effective: f32,
    /// The world difficulty is normal.
    pub normal: bool,
    /// The world difficulty is hard.
    pub hard: bool,
    /// `Difficulty.getId`: 0 peaceful to 3 hard.
    pub id: u8,
}
/// `DimensionType.MOON_BRIGHTNESS_PER_PHASE`.
pub const MOON_BRIGHTNESS: [f32; 8] = [1.0, 0.75, 0.5, 0.25, 0.0, 0.25, 0.5, 0.75];
impl LocalDifficulty {
    /// `calculateDifficulty` from the difficulty id (0 peaceful to 3 hard),
    /// the overworld clock, the chunk's inhabited time and the moon.
    pub fn new(base: u8, total: i64, inhabited: i64, moon: f32) -> Self {
        let hard = base == 3;
        let normal = base == 2;
        if base == 0 {
            return Self {
                base,
                effective: 0.0,
                normal,
                hard,
                id: base,
            };
        }
        let global = ((total as f32 - 72000.0) / 1_440_000.0).clamp(0.0, 1.0) * 0.25;
        let mut local =
            (inhabited as f32 / 3_600_000.0).clamp(0.0, 1.0) * if hard { 1.0 } else { 0.75 };
        local += (moon * 0.25).clamp(0.0, global);
        if base == 1 {
            local *= 0.5;
        }
        Self {
            base,
            effective: f32::from(base) * (0.75 + global + local),
            normal,
            hard,
            id: base,
        }
    }
    /// `getSpecialMultiplier`.
    pub fn special(&self) -> f32 {
        if self.effective < 2.0 {
            0.0
        } else if self.effective > 4.0 {
            1.0
        } else {
            (self.effective - 2.0) / 2.0
        }
    }
}
/// The armour and toughness each vanilla armour piece adds.
pub fn armor_stats(item: &str) -> (f32, f32) {
    let Some(name) = item.strip_prefix("minecraft:") else {
        return (0.0, 0.0);
    };
    let slot = ["helmet", "chestplate", "leggings", "boots"]
        .iter()
        .position(|p| name.ends_with(p));
    let Some(slot) = slot else { return (0.0, 0.0) };
    let (points, toughness): ([f32; 4], f32) = match name.split('_').next() {
        Some("leather") => ([1.0, 3.0, 2.0, 1.0], 0.0),
        Some("copper") => ([2.0, 4.0, 3.0, 1.0], 0.0),
        Some("golden") => ([2.0, 5.0, 3.0, 1.0], 0.0),
        Some("chainmail") => ([2.0, 5.0, 4.0, 1.0], 0.0),
        Some("iron") => ([2.0, 6.0, 5.0, 2.0], 0.0),
        Some("turtle") => ([2.0, 0.0, 0.0, 0.0], 0.0),
        Some("diamond") => ([3.0, 8.0, 6.0, 3.0], 2.0),
        Some("netherite") => ([3.0, 8.0, 6.0, 3.0], 3.0),
        _ => return (0.0, 0.0),
    };
    (points[slot], toughness)
}
/// The armour and toughness a horse armor item adds in the body slot
/// (`ArmorMaterial.createAttributes(ArmorType.BODY)`).
pub fn horse_armor_stats(item: &str) -> (f32, f32) {
    match item.strip_prefix("minecraft:") {
        Some("leather_horse_armor") => (3.0, 0.0),
        Some("copper_horse_armor") => (4.0, 0.0),
        Some("iron_horse_armor") => (5.0, 0.0),
        Some("golden_horse_armor") => (7.0, 0.0),
        Some("diamond_horse_armor") => (11.0, 2.0),
        Some("netherite_horse_armor") => (19.0, 3.0),
        _ => (0.0, 0.0),
    }
}
/// `Mob.populateDefaultEquipmentSlots`: sometimes a set of armour, from the
/// head down, of one random material.
fn populate_armor(m: &mut Mob, d: &LocalDifficulty) {
    if m.random.next_float() >= 0.15 * d.special() {
        return;
    }
    let mut kind = m.random.next_int_bound(3).unwrap_or(0);
    for _ in 1..=3 {
        if m.random.next_float() < 0.1087 {
            kind += 1;
        }
    }
    let partial = if d.hard { 0.1 } else { 0.25 };
    let material = [
        "leather",
        "copper",
        "golden",
        "chainmail",
        "iron",
        "diamond",
    ][kind as usize];
    for (i, piece) in ["helmet", "chestplate", "leggings", "boots"]
        .into_iter()
        .enumerate()
    {
        if i > 0 && m.random.next_float() < partial {
            break;
        }
        let slot = match i {
            0 => &mut m.extra.head,
            1 => &mut m.extra.chest,
            2 => &mut m.extra.legs,
            _ => &mut m.extra.feet,
        };
        if slot.is_none() {
            *slot = Some(format!("minecraft:{material}_{piece}").into());
        }
    }
}
pub fn finalize(m: &mut Mob, biome: &BiomeFacts, difficulty: &LocalDifficulty) {
    use Species as S;
    let climate_index = match biome.climate {
        Climate::Temperate => 0,
        Climate::Warm => 1,
        Climate::Cold => 2,
    };
    // `Registry.getRandom` of the `*_sound_variant` registry (pig, cow,
    // chicken, cat and wolf), drawn by their finalizeSpawn.
    let sounds = super::finalize::sound_variants(m.species()).len();
    if sounds > 0 {
        m.extra.sound_variant = Some(m.random.next_int_bound(sounds as i32).unwrap_or(0) as u8);
    }
    match m.species() {
        // Warden.finalizeSpawn: a minute before it digs back down.
        S::Warden => super::warden::finalize(m, false, &mut Vec::new()),
        S::Pig | S::Cow | S::Chicken => m.extra.variant = climate_index,
        S::Sheep => m.extra.color = super::farm::sheep_color(biome.climate, &mut m.random),
        S::Mooshroom => m.extra.variant = 0,
        // Turtle.finalizeSpawn: home is where it spawned.
        S::Turtle => {
            m.extra.home = Some([
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ])
        }
        S::Frog => m.extra.variant = biome.frog,
        S::Wolf => m.extra.variant = biome.wolf,
        S::Cat => m.extra.variant = super::tame::cat_variant(m, biome.full_moon),
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager => {
            // Zombie.finalizeSpawn: pick up loot 55% of the time (scaled by
            // difficulty), 5% babies (normal difficulty).
            m.extra.pick_up_loot = m.random.next_float() < 0.55 * difficulty.special();
            if m.random.next_float() < 0.05 {
                m.extra.age = i32::MIN / 2;
            }
            // setCanBreakDoors(nextFloat() < special * 0.1), before the gear.
            let can_break_doors = m.random.next_float() < difficulty.special() * 0.1;
            super::doors::set_can_break_doors(m, can_break_doors);
            if m.species() != S::Drowned {
                populate_armor(m, difficulty);
            }
            let weapon = if difficulty.hard { 0.05 } else { 0.01 };
            if m.species() != S::Drowned && m.random.next_float() < weapon {
                let r = m.random.next_int_bound(6).unwrap_or(0);
                m.extra.set_main_hand(Some(
                    match r {
                        0 => "minecraft:iron_sword",
                        1 => "minecraft:iron_spear",
                        _ => "minecraft:iron_shovel",
                    }
                    .into(),
                ));
            }
            if m.species() == S::Drowned && m.random.next_float() > 0.9 {
                m.extra.set_main_hand(Some(
                    if m.random.next_int_bound(16).unwrap_or(1) < 10 {
                        "minecraft:trident"
                    } else {
                        "minecraft:fishing_rod"
                    }
                    .into(),
                ));
            }
            if m.species() == S::ZombieVillager {
                m.extra.variant = villager_type(biome);
                m.extra.variant2 = m.random.next_int_bound(15).unwrap_or(0) as u8;
            }
        }
        // ZombifiedPiglin.populateDefaultEquipmentSlots: a golden sword, or
        // one time in 20 a golden spear.
        S::ZombifiedPiglin => {
            // Zombie.finalizeSpawn's setCanBreakDoors.
            let can_break_doors = m.random.next_float() < difficulty.special() * 0.1;
            super::doors::set_can_break_doors(m, can_break_doors);
            let spear = m.random.next_int_bound(20).unwrap_or(1) == 0;
            m.extra.set_main_hand(Some(
                if spear {
                    "minecraft:golden_spear"
                } else {
                    "minecraft:golden_sword"
                }
                .into(),
            ));
        }
        // AbstractSkeleton.populateDefaultEquipmentSlots: armour, then a bow.
        S::Skeleton | S::Stray | S::Bogged | S::Parched => {
            populate_armor(m, difficulty);
            m.extra.set_main_hand(Some("minecraft:bow".into()));
            m.extra.pick_up_loot = m.random.next_float() < 0.55 * difficulty.special();
        }
        // WitherSkeleton.populateDefaultEquipmentSlots: a stone sword only.
        S::WitherSkeleton => {
            m.extra.set_main_hand(Some("minecraft:stone_sword".into()));
            m.extra.pick_up_loot = m.random.next_float() < 0.55 * difficulty.special();
        }
        S::Pillager => {
            m.extra.set_main_hand(Some("minecraft:crossbow".into()));
            m.extra.pick_up_loot = true;
        }
        S::Horse | S::Donkey | S::Mule | S::SkeletonHorse | S::ZombieHorse => {
            super::equine::finalize(m)
        }
        S::Llama | S::TraderLlama => super::llama::finalize(m),
        // Camel.finalizeSpawn: standing, with fixed attributes.
        S::Camel | S::CamelHusk => super::camel::stand_up_instantly(m),
        S::Nautilus | S::ZombieNautilus => super::nautilus::finalize(m, biome.coral_nautilus),
        S::SulfurCube => super::sulfur_cube::finalize(m),
        // Piglin.createSpawnWeapon: half crossbows, half golden swords, and
        // each piece of golden armour one time in ten.
        S::Piglin if !m.is_baby() => {
            m.extra.pick_up_loot = true;
            for i in 0..4 {
                if m.random.next_float() < 0.1 {
                    let (slot, piece) = match i {
                        0 => (&mut m.extra.head, "helmet"),
                        1 => (&mut m.extra.chest, "chestplate"),
                        2 => (&mut m.extra.legs, "leggings"),
                        _ => (&mut m.extra.feet, "boots"),
                    };
                    *slot = Some(format!("minecraft:golden_{piece}").into());
                }
            }
            let crossbow = m.random.next_float() < 0.5;
            m.extra.set_main_hand(Some(
                if crossbow {
                    "minecraft:crossbow"
                } else {
                    "minecraft:golden_sword"
                }
                .into(),
            ));
        }
        S::PiglinBrute => {
            m.extra.set_main_hand(Some("minecraft:golden_axe".into()));
            m.extra.pick_up_loot = true;
        }
        S::Illusioner => super::illusioner::finalize(m),
        S::Vindicator => m.extra.set_main_hand(Some("minecraft:iron_axe".into())),
        S::Villager => {
            m.extra.variant = villager_type(biome);
            m.extra.pick_up_loot = true;
        }
        // Fox.finalizeSpawn: the variant by biome, then the mouth's item.
        S::Fox => {
            m.extra.variant = u8::from(biome.snow_fox);
            m.extra.pick_up_loot = true;
            super::fox::populate_default_equipment(m);
        }
        S::Panda | S::Dolphin => m.extra.pick_up_loot = true,
        S::Goat => super::finalize::goat_spawn(m),
        S::TropicalFish => super::aquatic::finalize(m),
        // Salmon.finalizeSpawn: a weighted size (small 30, medium 50, large 15).
        S::Salmon => {
            let r = m.random.next_int_bound(95).unwrap_or(0);
            m.extra.variant = if r < 30 {
                0
            } else if r < 80 {
                1
            } else {
                2
            };
        }
        S::Slime | S::MagmaCube => super::cube::finalize(m),
        _ => {}
    }
    // Zombie.finalizeSpawn ends with handleAttributes: the reinforcement
    // chance (and a leader's bonus) that `Zombie.hurtServer` reads.
    if matches!(
        m.species(),
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager | S::ZombifiedPiglin
    ) {
        super::finalize::zombie_attributes(
            m,
            difficulty.special(),
            super::finalize::Reason::Natural,
        );
    }
}
/// VillagerType.byBiome
pub fn villager_type(b: &BiomeFacts) -> u8 {
    if b.desert {
        1
    } else if b.jungle {
        2
    } else if b.snowy {
        4
    } else if b.swamp {
        5
    } else if b.climate == Climate::Warm {
        3
    } else if b.climate == Climate::Cold && b.temperature < 0.4 {
        6
    } else {
        0
    }
}
