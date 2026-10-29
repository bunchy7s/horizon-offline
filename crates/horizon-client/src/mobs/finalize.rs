//! `Mob.finalizeSpawn` and its overrides, for every spawn path (natural
//! spawning, chunk generation, spawners, trial spawners, spawn eggs,
//! commands, structures, patrols, raids, jockeys, conversions...).
//!
//! Every draw vanilla takes from `level.getRandom()` comes from the random
//! the caller passes (the level's random for a running level, the region's
//! for chunk generation); the few that come from the entity's own random
//! (`Zombie.handleAttributes`, `Piglin.createSpawnWeapon`, salmon sizes) come
//! from `Mob::random`. The order of the draws follows vanilla's class chain:
//! each species runs its own part before or after its superclass as vanilla
//! does. Jockeys a mob brings (`EntityType.create` inside finalizeSpawn) are
//! made through [`FinalizeLevel::create`], finalized with the same random and
//! returned in [`Ctx::created`].
//!
//! Variant state kept in [`super::Extra`]: `variant` is the registry or enum
//! index of the species' variant (cat, wolf, frog, farm animals by climate,
//! zombie nautilus, horse, llama, fox, rabbit, parrot, axolotl, salmon,
//! tropical fish pattern, panda main gene, villager type), `variant2` the
//! second part (horse markings, llama strength, panda hidden gene, tropical
//! fish base colour), `variant3` the tropical fish pattern colour, and
//! `sound_variant` the index in the species' sound variant registry.
use super::{Mob, Species, farm::Climate, gear, spawning::LocalDifficulty};
use crate::{enchant::Enchantments, items::Items};
use horizon_core::{
    geometry::Aabb,
    random::{LegacyRandom, RandomSource},
};
use serde_json::Value;
use std::{fs, path::Path, sync::Arc};

/// `EntitySpawnReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    Natural,
    ChunkGeneration,
    Spawner,
    Structure,
    Breeding,
    MobSummoned,
    Jockey,
    Event,
    Conversion,
    Reinforcement,
    Triggered,
    Bucket,
    SpawnItemUse,
    Command,
    Dispenser,
    Patrol,
    TrialSpawner,
    Load,
    DimensionTravel,
}
impl Reason {
    pub const ALL: [Self; 19] = [
        Self::Natural,
        Self::ChunkGeneration,
        Self::Spawner,
        Self::Structure,
        Self::Breeding,
        Self::MobSummoned,
        Self::Jockey,
        Self::Event,
        Self::Conversion,
        Self::Reinforcement,
        Self::Triggered,
        Self::Bucket,
        Self::SpawnItemUse,
        Self::Command,
        Self::Dispenser,
        Self::Patrol,
        Self::TrialSpawner,
        Self::Load,
        Self::DimensionTravel,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Natural => "NATURAL",
            Self::ChunkGeneration => "CHUNK_GENERATION",
            Self::Spawner => "SPAWNER",
            Self::Structure => "STRUCTURE",
            Self::Breeding => "BREEDING",
            Self::MobSummoned => "MOB_SUMMONED",
            Self::Jockey => "JOCKEY",
            Self::Event => "EVENT",
            Self::Conversion => "CONVERSION",
            Self::Reinforcement => "REINFORCEMENT",
            Self::Triggered => "TRIGGERED",
            Self::Bucket => "BUCKET",
            Self::SpawnItemUse => "SPAWN_ITEM_USE",
            Self::Command => "COMMAND",
            Self::Dispenser => "DISPENSER",
            Self::Patrol => "PATROL",
            Self::TrialSpawner => "TRIAL_SPAWNER",
            Self::Load => "LOAD",
            Self::DimensionTravel => "DIMENSION_TRAVEL",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
}

/// The level as finalizeSpawn reads it.
pub trait FinalizeLevel {
    /// The biome id at a block.
    fn biome(&self, pos: [i32; 3]) -> String;
    /// The biome at a block is in a biome tag (`minecraft:...`).
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool;
    /// The structures with a piece at a block.
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String>;
    /// `EnvironmentAttributes.MOON_PHASE` at a block (0 full moon to 7).
    fn moon_phase(&self, pos: [i32; 3]) -> usize;
    /// `ServerLevel.isSpawningMonsters`.
    fn spawning_monsters(&self) -> bool;
    /// `Level.noCollision(AABB)`.
    fn no_collision(&self, bounds: Aabb) -> bool;
    /// The first chicken (in entity order) whose box meets `bounds` and
    /// that is not being ridden (`Zombie.finalizeSpawn`'s chicken jockeys).
    fn free_chicken(&self, bounds: Aabb) -> Option<u32>;
    /// A chicken found by `free_chicken` carries the zombie `rider`
    /// (`setChickenJockey(true)` and `startRiding`).
    fn ride_chicken(&mut self, chicken: u32, rider: u32);
    /// `getGameTime`.
    fn game_time(&self) -> i64;
    /// `SpecialDates.isHalloween`.
    fn halloween(&self) -> bool;
    /// `EntityType.create(level, reason)`: a new mob of a type (an id and an
    /// own random from the caller), or None where it cannot exist (monsters
    /// in peaceful).
    fn create(&mut self, kind: &str, reason: Reason) -> Option<Mob>;
}

/// `AgeableMob.AgeableMobGroupData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ageable {
    pub size: i32,
    pub should_spawn_baby: bool,
    pub baby_chance: f32,
}
impl Ageable {
    pub fn new(should_spawn_baby: bool, baby_chance: f32) -> Self {
        Self {
            size: 0,
            should_spawn_baby,
            baby_chance,
        }
    }
    /// `AgeableMobGroupData(boolean)`.
    pub fn babies(should_spawn_baby: bool) -> Self {
        Self::new(should_spawn_baby, 0.05)
    }
    /// `AgeableMobGroupData(float)`.
    pub fn chance(baby_chance: f32) -> Self {
        Self::new(true, baby_chance)
    }
}
/// `SpawnGroupData`: what the mobs of a pack share.
#[derive(Clone, Debug, PartialEq)]
pub enum Group {
    Ageable(Ageable),
    /// `Zombie.ZombieGroupData`, and `Husk.HuskGroupData` with `tried_camel`.
    Zombie {
        baby: bool,
        jockey: bool,
        husk: Option<bool>,
    },
    /// `Spider.SpiderEffectsGroupData`.
    Spider(Option<&'static str>),
    Horse(Ageable, u8),
    Llama(Ageable, u8),
    Fox(Ageable, u8),
    Rabbit(Ageable, u8),
    Axolotl(Ageable, [u8; 2]),
    Wolf(Ageable, u8),
    /// `AbstractSchoolingFish.SchoolSpawnGroupData` (the leader's id), and
    /// `TropicalFishGroupData` with its variant.
    School(u32, Option<(u8, u8, u8)>),
}
impl Group {
    fn ageable(&mut self) -> Option<&mut Ageable> {
        match self {
            Self::Ageable(a)
            | Self::Horse(a, _)
            | Self::Llama(a, _)
            | Self::Fox(a, _)
            | Self::Rabbit(a, _)
            | Self::Axolotl(a, _)
            | Self::Wolf(a, _) => Some(a),
            _ => None,
        }
    }
}

/// A variant registry entry's spawn conditions (`SpawnPrioritySelectors`).
#[derive(Clone, Debug)]
enum Condition {
    Biome(Vec<String>),
    Structure(Vec<String>),
    Moon(Option<f64>, Option<f64>),
}
#[derive(Clone, Debug)]
pub struct VariantRegistry {
    /// Entry names in registry order (sorted by id).
    pub names: Vec<String>,
    selectors: Vec<(usize, Option<Condition>, i32)>,
}
impl VariantRegistry {
    fn load(dir: &Path, structure_tags: &dyn Fn(&str) -> Vec<String>) -> Self {
        let mut names: Vec<String> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".json"))
                    .map(str::to_owned)
            })
            .collect();
        names.sort();
        let mut selectors = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let Ok(bytes) = fs::read(dir.join(format!("{name}.json"))) else {
                continue;
            };
            let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            for s in v["spawn_conditions"].as_array().into_iter().flatten() {
                let priority = s["priority"].as_i64().unwrap_or(0) as i32;
                let c = &s["condition"];
                let list = |v: &Value| -> Vec<String> {
                    match v {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(a) => a
                            .iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect(),
                        _ => Vec::new(),
                    }
                };
                let condition = match c["type"].as_str() {
                    Some("minecraft:biome") => Some(Condition::Biome(list(&c["biomes"]))),
                    Some("minecraft:structure") => Some(Condition::Structure(
                        list(&c["structures"])
                            .into_iter()
                            .flat_map(|s| match s.strip_prefix('#') {
                                Some(tag) => structure_tags(tag),
                                None => vec![s],
                            })
                            .collect(),
                    )),
                    Some("minecraft:moon_brightness") => {
                        let r = &c["range"];
                        Some(match r.as_f64() {
                            Some(x) => Condition::Moon(Some(x), Some(x)),
                            None => Condition::Moon(r["min"].as_f64(), r["max"].as_f64()),
                        })
                    }
                    _ => None,
                };
                selectors.push((i, condition, priority));
            }
        }
        Self { names, selectors }
    }
    pub fn index(&self, name: &str) -> Option<usize> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        self.names.iter().position(|n| n == name)
    }
    /// `VariantUtils.selectVariantToSpawn`: the entries of the highest
    /// priority whose condition holds, one of them at random.
    pub fn select<L: FinalizeLevel + ?Sized>(
        &self,
        level: &L,
        pos: [i32; 3],
        random: &mut impl RandomSource,
    ) -> Option<usize> {
        let mut unpacked: Vec<&(usize, Option<Condition>, i32)> = self.selectors.iter().collect();
        unpacked.sort_by_key(|s| std::cmp::Reverse(s.2));
        let mut highest = i32::MIN;
        let mut selected = Vec::new();
        for (entry, condition, priority) in unpacked {
            if *priority < highest {
                continue;
            }
            let ok = match condition {
                None => true,
                Some(Condition::Biome(list)) => list.iter().any(|b| match b.strip_prefix('#') {
                    Some(tag) => level.biome_tag(tag, pos),
                    None => level.biome(pos) == *b,
                }),
                Some(Condition::Structure(list)) => {
                    let here = level.structures_with_piece_at(pos);
                    here.iter().any(|s| list.contains(s))
                }
                Some(Condition::Moon(min, max)) => {
                    let b = f64::from(super::spawning::MOON_BRIGHTNESS[level.moon_phase(pos) & 7]);
                    min.is_none_or(|m| b >= m) && max.is_none_or(|m| b <= m)
                }
            };
            if ok {
                highest = *priority;
                selected.push(*entry);
            }
        }
        if selected.is_empty() {
            return None;
        }
        Some(selected[random.next_int_bound(selected.len() as i32).unwrap_or(0) as usize])
    }
    /// `Registry.getRandom`: any entry.
    pub fn random(&self, random: &mut impl RandomSource) -> usize {
        if self.names.is_empty() {
            return 0;
        }
        random.next_int_bound(self.names.len() as i32).unwrap_or(0) as usize
    }
}

/// The registries finalizeSpawn reads.
pub struct SpawnRegistries {
    pub pig: VariantRegistry,
    pub cow: VariantRegistry,
    pub chicken: VariantRegistry,
    pub cat: VariantRegistry,
    pub wolf: VariantRegistry,
    pub frog: VariantRegistry,
    pub zombie_nautilus: VariantRegistry,
    pub pig_sound: VariantRegistry,
    pub cow_sound: VariantRegistry,
    pub chicken_sound: VariantRegistry,
    pub cat_sound: VariantRegistry,
    pub wolf_sound: VariantRegistry,
}
impl SpawnRegistries {
    /// Loads the variant registries from a resources directory
    /// (`data/minecraft/<registry>/`).
    pub fn load(root: &Path) -> Self {
        let data = root.join("data/minecraft");
        let structure_tag = |tag: &str| -> Vec<String> {
            let path = tag.strip_prefix("minecraft:").unwrap_or(tag);
            let file = data.join(format!("tags/worldgen/structure/{path}.json"));
            let Ok(bytes) = fs::read(file) else {
                return Vec::new();
            };
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v["values"].as_array().cloned())
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(str::to_owned))
                .collect()
        };
        let r = |name: &str| VariantRegistry::load(&data.join(name), &structure_tag);
        Self {
            pig: r("pig_variant"),
            cow: r("cow_variant"),
            chicken: r("chicken_variant"),
            cat: r("cat_variant"),
            wolf: r("wolf_variant"),
            frog: r("frog_variant"),
            zombie_nautilus: r("zombie_nautilus_variant"),
            pig_sound: r("pig_sound_variant"),
            cow_sound: r("cow_sound_variant"),
            chicken_sound: r("chicken_sound_variant"),
            cat_sound: r("cat_sound_variant"),
            wolf_sound: r("wolf_sound_variant"),
        }
    }
}

/// What finalizeSpawn needs besides the level: registries, enchantments.
pub struct SpawnData<'a> {
    pub registries: &'a SpawnRegistries,
    pub enchantments: &'a Enchantments,
    pub items: &'a Items,
    /// `EntityTypes.CAMEL_HUSK` dimensions (width, height).
    pub camel_husk_size: (f32, f32),
}

/// A finalizeSpawn call and the entities it made.
pub struct Ctx<'a, L: FinalizeLevel + ?Sized, R: RandomSource> {
    pub level: &'a mut L,
    pub random: &'a mut R,
    pub difficulty: LocalDifficulty,
    pub data: &'a SpawnData<'a>,
    /// Jockeys and mounts made along the way, finalized, with their riding
    /// links set (`Extra::mount` / `Extra::rider`).
    pub created: Vec<Mob>,
}

// ---- attributes -----------------------------------------------------------

/// The attributes finalizeSpawn changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attr {
    MaxHealth,
    MovementSpeed,
    FollowRange,
    AttackDamage,
    KnockbackResistance,
    SpawnReinforcements,
    JumpStrength,
    Armor,
    BurningTime,
}
impl Attr {
    pub const ALL: [Self; 9] = [
        Self::MaxHealth,
        Self::MovementSpeed,
        Self::FollowRange,
        Self::AttackDamage,
        Self::KnockbackResistance,
        Self::SpawnReinforcements,
        Self::JumpStrength,
        Self::Armor,
        Self::BurningTime,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::MaxHealth => "max_health",
            Self::MovementSpeed => "movement_speed",
            Self::FollowRange => "follow_range",
            Self::AttackDamage => "attack_damage",
            Self::KnockbackResistance => "knockback_resistance",
            Self::SpawnReinforcements => "spawn_reinforcements",
            Self::JumpStrength => "jump_strength",
            Self::Armor => "armor",
            Self::BurningTime => "burning_time",
        }
    }
    /// `RangedAttribute` bounds.
    fn range(self) -> (f64, f64) {
        match self {
            Self::MaxHealth => (1.0, 1024.0),
            Self::MovementSpeed => (0.0, 1024.0),
            Self::FollowRange => (0.0, 2048.0),
            Self::AttackDamage => (0.0, 2048.0),
            Self::KnockbackResistance | Self::SpawnReinforcements => (0.0, 1.0),
            Self::JumpStrength => (0.0, 32.0),
            Self::Armor => (0.0, 30.0),
            Self::BurningTime => (0.0, 1024.0),
        }
    }
}
/// `AttributeModifier.Operation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}
/// A permanent attribute modifier (`AttributeModifier`).
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub attr: Attr,
    pub id: &'static str,
    pub amount: f64,
    pub op: Op,
}
/// Attribute base values that differ from the type's and the permanent
/// modifiers finalizeSpawn added, in the order vanilla adds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Attributes {
    pub bases: Vec<(Attr, f64)>,
    pub modifiers: Vec<Modifier>,
}
/// The type's default value of an attribute, if the mob has it.
pub fn default_value(m: &Mob, a: Attr) -> Option<f64> {
    m.kind
        .attributes
        .get(&format!("minecraft:{}", a.name()))
        .copied()
}
pub fn base_value(m: &Mob, a: Attr) -> f64 {
    m.extra
        .attributes
        .as_ref()
        .and_then(|s| s.bases.iter().find(|(x, _)| *x == a).map(|(_, v)| *v))
        .or_else(|| default_value(m, a))
        .unwrap_or(if a == Attr::BurningTime { 1.0 } else { 0.0 })
}
/// The transient modifiers of the mob's state (`SPEED_MODIFIER_BABY`).
pub fn transient(m: &Mob, a: Attr) -> Option<(f64, Op)> {
    if a != Attr::MovementSpeed || !m.is_baby() {
        return None;
    }
    match m.species() {
        Species::Zombie
        | Species::Husk
        | Species::Drowned
        | Species::ZombieVillager
        | Species::ZombifiedPiglin => Some((0.5, Op::AddMultipliedBase)),
        Species::Piglin => Some((f64::from(0.2f32), Op::AddMultipliedBase)),
        _ => None,
    }
}
/// The attribute modifiers of the mob's effects (`MobEffect.addAttributeModifier`,
/// scaled by the amplifier plus one): strength and speed.
fn effect_modifiers(m: &Mob, a: Attr) -> impl Iterator<Item = (f64, Op)> + '_ {
    m.extra
        .effects
        .iter()
        .filter_map(move |&(effect, _, amplifier)| {
            let level = f64::from(amplifier + 1);
            match (effect.trim_start_matches("minecraft:"), a) {
                ("strength", Attr::AttackDamage) => Some((3.0 * level, Op::AddValue)),
                ("speed", Attr::MovementSpeed) => {
                    Some((f64::from(0.2f32) * level, Op::AddMultipliedTotal))
                }
                _ => None,
            }
        })
}
/// `AttributeInstance.getValue`: the base, then the modifiers by operation.
pub fn value(m: &Mob, a: Attr) -> f64 {
    compute(m, a, true, &[])
}
/// `value` with more modifiers, such as the ones worn equipment grants.
pub fn value_with(m: &Mob, a: Attr, extra: &[(f64, Op)]) -> f64 {
    compute(m, a, true, extra)
}
fn compute(m: &Mob, a: Attr, with_transient: bool, extra: &[(f64, Op)]) -> f64 {
    let mut base = base_value(m, a);
    let empty = Vec::new();
    let mods = m.extra.attributes.as_ref().map_or(&empty, |s| &s.modifiers);
    let mut ops: Vec<(f64, Op)> = mods
        .iter()
        .filter(|x| x.attr == a)
        .map(|x| (x.amount, x.op))
        .collect();
    if with_transient {
        ops.extend(transient(m, a));
        ops.extend(effect_modifiers(m, a));
    }
    ops.extend(extra.iter().copied());
    for (amount, op) in &ops {
        if *op == Op::AddValue {
            base += amount;
        }
    }
    let mut result = base;
    for (amount, op) in &ops {
        if *op == Op::AddMultipliedBase {
            result += base * amount;
        }
    }
    for (amount, op) in &ops {
        if *op == Op::AddMultipliedTotal {
            result *= 1.0 + amount;
        }
    }
    let (lo, hi) = a.range();
    if result.is_nan() {
        lo
    } else {
        result.clamp(lo, hi)
    }
}
/// Copies the attribute values into the fields the game reads.
fn refresh(m: &mut Mob, a: Attr) {
    match a {
        Attr::MaxHealth => m.attrs.max_health = value(m, a) as f32,
        Attr::MovementSpeed => m.attrs.speed = base_with_modifiers(m, a),
        Attr::FollowRange => {
            m.attrs.follow_range = value(m, a);
            m.update_max_visited_nodes();
        }
        Attr::AttackDamage => m.attrs.attack_damage = value(m, a) as f32,
        Attr::KnockbackResistance => m.attrs.knockback_resistance = value(m, a),
        Attr::SpawnReinforcements => m.attrs.spawn_reinforcements = value(m, a),
        Attr::JumpStrength => m.attrs.jump_strength = value(m, a),
        Attr::Armor | Attr::BurningTime => {}
    }
}
/// The value without the transient baby modifier (`speed_bonus` applies
/// that one to movement speed).
fn base_with_modifiers(m: &Mob, a: Attr) -> f64 {
    compute(m, a, false, &[])
}
/// `AttributeInstance.setBaseValue`.
pub fn set_base(m: &mut Mob, a: Attr, v: f64) {
    let s = m.extra.attributes.get_or_insert_with(Box::default);
    match s.bases.iter_mut().find(|(x, _)| *x == a) {
        Some(b) => b.1 = v,
        None => s.bases.push((a, v)),
    }
    refresh(m, a);
}
pub fn has_modifier(m: &Mob, a: Attr, id: &str) -> bool {
    m.extra
        .attributes
        .as_ref()
        .is_some_and(|s| s.modifiers.iter().any(|x| x.attr == a && x.id == id))
}
/// The amount of a permanent modifier, if the mob has it.
pub fn modifier_amount(m: &Mob, a: Attr, id: &str) -> Option<f64> {
    m.extra
        .attributes
        .as_ref()
        .and_then(|s| s.modifiers.iter().find(|x| x.attr == a && x.id == id))
        .map(|x| x.amount)
}
/// `addOrReplacePermanentModifier`.
pub fn add_modifier(m: &mut Mob, a: Attr, id: &'static str, amount: f64, op: Op) {
    let s = m.extra.attributes.get_or_insert_with(Box::default);
    let new = Modifier {
        attr: a,
        id,
        amount,
        op,
    };
    match s.modifiers.iter_mut().find(|x| x.attr == a && x.id == id) {
        Some(x) => *x = new,
        None => s.modifiers.push(new),
    }
    refresh(m, a);
}
/// `Zombie.handleAttributes`, from the zombie's own random.
pub fn zombie_attributes(m: &mut Mob, special: f32, reason: Reason) {
    let r = &mut m.random;
    let reinforcements = if m.kind.species == Species::ZombifiedPiglin {
        0.0
    } else {
        r.next_double() * f64::from(0.1f32)
    };
    let knockback = r.next_double() * f64::from(0.05f32);
    let follow = r.next_double() * 1.5 * f64::from(special);
    let leader = r.next_float() < special * 0.05;
    let leader_rolls = leader.then(|| (r.next_double() * 0.25 + 0.5, r.next_double() * 3.0 + 1.0));
    set_base(m, Attr::SpawnReinforcements, reinforcements);
    add_modifier(
        m,
        Attr::KnockbackResistance,
        "minecraft:random_spawn_bonus",
        knockback,
        Op::AddValue,
    );
    if follow > 1.0 {
        add_modifier(
            m,
            Attr::FollowRange,
            "minecraft:zombie_random_spawn_bonus",
            follow,
            Op::AddMultipliedTotal,
        );
    }
    if let Some((reinforce, health)) = leader_rolls {
        add_modifier(
            m,
            Attr::SpawnReinforcements,
            "minecraft:leader_zombie_bonus",
            reinforce,
            Op::AddValue,
        );
        add_modifier(
            m,
            Attr::MaxHealth,
            "minecraft:leader_zombie_bonus",
            health,
            Op::AddMultipliedTotal,
        );
        if !matches!(
            reason,
            Reason::Conversion | Reason::Load | Reason::DimensionTravel
        ) {
            m.health = max_health(m);
        }
        super::doors::set_can_break_doors(m, true);
    }
}
pub fn max_health(m: &Mob) -> f32 {
    value(m, Attr::MaxHealth) as f32
}

// ---- species facts --------------------------------------------------------

/// `AgeableMob` subclasses.
pub fn ageable(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Pig
            | S::Cow
            | S::Mooshroom
            | S::Sheep
            | S::Chicken
            | S::Rabbit
            | S::Horse
            | S::Donkey
            | S::Mule
            | S::Llama
            | S::TraderLlama
            | S::Goat
            | S::Panda
            | S::PolarBear
            | S::Fox
            | S::Wolf
            | S::Cat
            | S::Ocelot
            | S::Parrot
            | S::Bee
            | S::Turtle
            | S::Armadillo
            | S::Frog
            | S::Axolotl
            | S::Squid
            | S::GlowSquid
            | S::Dolphin
            | S::Villager
            | S::WanderingTrader
            | S::Slime
            | S::MagmaCube
            | S::SulfurCube
            | S::Hoglin
            | S::Strider
            | S::Camel
            | S::CamelHusk
            | S::Sniffer
            | S::HappyGhast
            | S::Nautilus
            | S::ZombieNautilus
            | S::ZombieHorse
            | S::SkeletonHorse
    )
}
fn zombie(s: Species) -> bool {
    matches!(
        s,
        Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::ZombifiedPiglin
    )
}
fn skeleton(s: Species) -> bool {
    matches!(
        s,
        Species::Skeleton
            | Species::Stray
            | Species::Bogged
            | Species::Parched
            | Species::WitherSkeleton
    )
}
fn raider(s: Species) -> bool {
    super::raider::is_raider(s)
}
/// `EntityType.isAllowedInPeaceful`: the types built with `notInPeaceful`.
pub fn allowed_in_peaceful(kind: &str) -> bool {
    !matches!(
        kind.strip_prefix("minecraft:").unwrap_or(kind),
        "blaze"
            | "bogged"
            | "breeze"
            | "cave_spider"
            | "creaking"
            | "creeper"
            | "drowned"
            | "elder_guardian"
            | "enderman"
            | "endermite"
            | "evoker"
            | "ghast"
            | "giant"
            | "guardian"
            | "hoglin"
            | "husk"
            | "illusioner"
            | "magma_cube"
            | "parched"
            | "phantom"
            | "piglin_brute"
            | "pillager"
            | "ravager"
            | "silverfish"
            | "skeleton"
            | "slime"
            | "spider"
            | "stray"
            | "vex"
            | "vindicator"
            | "warden"
            | "witch"
            | "wither"
            | "wither_skeleton"
            | "zoglin"
            | "zombie"
            | "zombie_villager"
            | "zombified_piglin"
    )
}
/// `getBabyStartAge`.
fn baby_start_age(s: Species) -> i32 {
    if s == Species::Sniffer {
        super::sniffer::BABY_START_AGE
    } else {
        -24000
    }
}
/// The age a non-ageable baby (zombies, piglins, zoglins) keeps.
pub const NON_AGEABLE_BABY: i32 = i32::MIN / 2;

/// `AgeableMob.setAge`, with each class's `ageBoundaryReached`.
pub fn set_age(m: &mut Mob, age: i32) {
    let old = m.extra.age;
    m.extra.age = age;
    if (old < 0) != (age < 0) {
        age_boundary_reached(m);
    }
}
fn age_boundary_reached(m: &mut Mob) {
    let baby = m.is_baby();
    match m.species() {
        Species::Goat => set_base(m, Attr::AttackDamage, if baby { 1.0 } else { 2.0 }),
        Species::Hoglin => set_base(m, Attr::AttackDamage, if baby { 0.5 } else { 6.0 }),
        Species::SulfurCube if !baby => sulfur_size(m, 2),
        _ => {}
    }
}
/// `SpawnEggItem.spawnsEntity`: the spawn egg is for the mob's own kind, so
/// it is used on it (`Mob.checkAndHandleImportantInteractions`).
pub fn spawn_egg_is_for(egg: &str, kind: &str) -> bool {
    egg.strip_suffix("_spawn_egg") == Some(kind)
}

/// `SpawnEggItem.spawnOffspringFromSpawnEgg`: the offspring of a spawn egg
/// used on a mob of the same kind is a baby of that kind: it starts with
/// the age of a baby and keeps the parent's variant and colour.
pub fn spawn_egg_offspring(child: &mut Mob, species: Species, variant: u8, color: u8) {
    set_baby(child, true);
    child.extra.variant = variant;
    if species == Species::Sheep {
        child.extra.color = color;
    }
}
/// `setBaby`: AgeableMob through its age, zombies, piglins and zoglins.
pub fn set_baby(m: &mut Mob, baby: bool) {
    let s = m.species();
    if ageable(s) {
        set_age(m, if baby { baby_start_age(s) } else { 0 });
        return;
    }
    match s {
        _ if zombie(s) || s == Species::Piglin => {
            m.extra.age = if baby { NON_AGEABLE_BABY } else { 0 };
        }
        Species::Zoglin => {
            m.extra.age = if baby { NON_AGEABLE_BABY } else { 0 };
            if baby {
                set_base(m, Attr::AttackDamage, 0.5);
            }
        }
        _ => {}
    }
}

// ---- equipment ------------------------------------------------------------

/// `EquipmentSlot` names (the `drops::SLOTS` order).
pub const SLOT_NAMES: [&str; 8] = super::drops::SLOTS;
const MAINHAND: usize = 0;
const OFFHAND: usize = 1;
const FEET: usize = 2;
const LEGS: usize = 3;
const CHEST: usize = 4;
const HEAD: usize = 5;
const SADDLE: usize = 7;
/// The `gear` slot of an `EquipmentSlot`.
fn gear_slot(slot: usize) -> usize {
    match slot {
        MAINHAND => gear::MAIN,
        OFFHAND => gear::OFF,
        FEET => gear::FEET,
        LEGS => gear::LEGS,
        CHEST => gear::CHEST,
        _ => gear::HEAD,
    }
}
pub fn item(m: &Mob, slot: usize) -> Option<Arc<str>> {
    match slot {
        MAINHAND => m.extra.main_hand.clone(),
        OFFHAND => m.extra.off_hand.clone(),
        FEET => m.extra.feet.clone(),
        LEGS => m.extra.legs.clone(),
        CHEST => m.extra.chest.clone(),
        HEAD => m.extra.head.clone(),
        SADDLE => m.extra.saddled.then(|| Arc::from("minecraft:saddle")),
        _ => None,
    }
}
/// `setItemSlot` with a fresh stack (enchantments cleared).
pub fn set_item(m: &mut Mob, slot: usize, id: &str) {
    let v = Some(Arc::from(id));
    match slot {
        MAINHAND => m.extra.main_hand = v,
        OFFHAND => m.extra.off_hand = v,
        FEET => m.extra.feet = v,
        LEGS => m.extra.legs = v,
        CHEST => m.extra.chest = v,
        HEAD => m.extra.head = v,
        SADDLE => {
            m.extra.saddled = true;
            m.extra.saddle_stack = None;
            return;
        }
        _ => return,
    }
    gear::set(m, gear_slot(slot), Vec::new());
}
/// The enchantments on what `m` has in `slot`.
pub fn slot_enchantments(m: &Mob, slot: usize) -> gear::Levels {
    gear::gear(m).map_or_else(Vec::new, |g| g.slots[gear_slot(slot)].clone())
}
/// Replace the enchantments of what `m` has in `slot` (after `set_item`).
pub fn set_slot_enchantments(m: &mut Mob, slot: usize, levels: gear::Levels) {
    gear::set(m, gear_slot(slot), levels);
}
/// `EquipmentUser.equip` for one rolled stack: `setItemSlot` puts the item in
/// `slot` with its own enchantments and trim (set_components), then the
/// slot's drop chance is stored when the table names one.
pub fn equip_stack(
    m: &mut Mob,
    slot: usize,
    stack: &horizon_world::loot::LootStack,
    drop_chance: Option<f32>,
) {
    set_item(m, slot, &stack.item);
    if slot <= HEAD {
        let levels = stack
            .enchantments
            .iter()
            .map(|(id, level)| (Arc::from(id.as_str()), (*level).max(0) as u32))
            .collect();
        gear::set(m, gear_slot(slot), levels);
        let trim = stack
            .components
            .iter()
            .find(|(id, _)| id == "minecraft:trim")
            .and_then(|(_, v)| {
                Some((
                    Arc::from(v["material"].as_str()?),
                    Arc::from(v["pattern"].as_str()?),
                ))
            });
        gear::set_trim(m, gear_slot(slot), trim);
    }
    if let Some(chance) = drop_chance {
        set_drop_chance(m, slot, chance);
    }
}
/// `Mob.setDropChance`.
pub fn set_drop_chance(m: &mut Mob, slot: usize, chance: f32) {
    let c = m
        .extra
        .drop_chances
        .get_or_insert_with(|| Box::new([super::drops::DEFAULT_DROP_CHANCE; 8]));
    c[slot] = chance;
}
/// The drop chance stored for a slot (`DropChances.byEquipment`).
pub fn stored_drop_chance(m: &Mob, slot: usize) -> f32 {
    m.extra
        .drop_chances
        .as_ref()
        .map_or(super::drops::DEFAULT_DROP_CHANCE, |c| c[slot])
}

impl<L: FinalizeLevel + ?Sized, R: RandomSource> Ctx<'_, L, R> {
    fn float(&mut self) -> f32 {
        self.random.next_float()
    }
    fn int(&mut self, bound: i32) -> i32 {
        self.random.next_int_bound(bound).unwrap_or(0)
    }
    fn special(&self) -> f32 {
        self.difficulty.special()
    }
    /// `UniformInt.sample`.
    fn uniform(&mut self, min: i32, max: i32) -> i32 {
        self.random
            .next_int_between_inclusive(min, max)
            .unwrap_or(min)
    }

    // ---- Mob ----------------------------------------------------------

    /// `Mob.finalizeSpawn`: the follow range spawn bonus and left-handedness.
    fn mob(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        if !has_modifier(m, Attr::FollowRange, "minecraft:random_spawn_bonus") {
            let bonus = self.random.triangle(0.0, 0.114_850_000_000_000_01);
            add_modifier(
                m,
                Attr::FollowRange,
                "minecraft:random_spawn_bonus",
                bonus,
                Op::AddMultipliedBase,
            );
        }
        m.extra.left_handed = self.float() < 0.05;
        g
    }
    /// `AgeableMob.finalizeSpawn`.
    fn ageable(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let mut g = g.unwrap_or(Group::Ageable(Ageable::babies(true)));
        if let Some(a) = g.ageable() {
            let (should, size, chance) = (a.should_spawn_baby, a.size, a.baby_chance);
            if should && size > 0 && self.float() <= chance {
                let start = baby_start_age(m.species());
                set_age(m, start);
            }
            if let Some(a) = g.ageable() {
                a.size += 1;
            }
        }
        self.mob(m, Some(g))
    }
    /// `Mob.populateDefaultEquipmentSlots`: a set of armour from the head
    /// down, of one material.
    fn armor(&mut self, m: &mut Mob) {
        if self.float() >= 0.15 * self.special() {
            return;
        }
        let mut kind = self.int(3);
        for _ in 1..=3 {
            if self.float() < 0.1087 {
                kind += 1;
            }
        }
        let partial = if self.difficulty.hard { 0.1 } else { 0.25 };
        const MATERIAL: [&str; 6] = [
            "leather",
            "copper",
            "golden",
            "chainmail",
            "iron",
            "diamond",
        ];
        let mut first = true;
        // EQUIPMENT_POPULATION_ORDER: head, chest, legs, feet.
        for (slot, piece) in [
            (HEAD, "helmet"),
            (CHEST, "chestplate"),
            (LEGS, "leggings"),
            (FEET, "boots"),
        ] {
            if !first && self.float() < partial {
                break;
            }
            first = false;
            if item(m, slot).is_none()
                && let Some(material) = MATERIAL.get(kind as usize)
            {
                set_item(m, slot, &format!("minecraft:{material}_{piece}"));
            }
        }
    }
    /// `populateDefaultEquipmentSlots` of each class.
    fn populate(&mut self, m: &mut Mob) {
        use Species as S;
        match m.species() {
            S::Zombie | S::Husk | S::ZombieVillager => {
                self.armor(m);
                let chance = if self.difficulty.hard { 0.05 } else { 0.01 };
                if self.float() < chance {
                    let weapon = match self.int(6) {
                        0 => "minecraft:iron_sword",
                        1 => "minecraft:iron_spear",
                        _ => "minecraft:iron_shovel",
                    };
                    set_item(m, MAINHAND, weapon);
                }
            }
            S::Drowned => {
                if f64::from(self.float()) > 0.9 {
                    let weapon = if self.int(16) < 10 {
                        "minecraft:trident"
                    } else {
                        "minecraft:fishing_rod"
                    };
                    set_item(m, MAINHAND, weapon);
                }
            }
            S::ZombifiedPiglin => {
                let weapon = if self.int(20) == 0 {
                    "minecraft:golden_spear"
                } else {
                    "minecraft:golden_sword"
                };
                set_item(m, MAINHAND, weapon);
            }
            S::WitherSkeleton => set_item(m, MAINHAND, "minecraft:stone_sword"),
            s if skeleton(s) => {
                self.armor(m);
                set_item(m, MAINHAND, "minecraft:bow");
            }
            S::Pillager => set_item(m, MAINHAND, "minecraft:crossbow"),
            S::Vindicator => {
                if super::raider::state(m).is_none_or(|s| s.raid_id.is_none()) {
                    set_item(m, MAINHAND, "minecraft:iron_axe");
                }
            }
            S::Piglin => {
                if !m.is_baby() {
                    for (slot, piece) in [
                        (HEAD, "helmet"),
                        (CHEST, "chestplate"),
                        (LEGS, "leggings"),
                        (FEET, "boots"),
                    ] {
                        if self.float() < 0.1 {
                            set_item(m, slot, &format!("minecraft:golden_{piece}"));
                        }
                    }
                }
            }
            S::PiglinBrute => set_item(m, MAINHAND, "minecraft:golden_axe"),
            S::Vex => {
                set_item(m, MAINHAND, "minecraft:iron_sword");
                set_drop_chance(m, MAINHAND, 0.0);
            }
            S::Fox => {
                if self.float() < 0.2 {
                    let odds = self.float();
                    let held = if odds < 0.05 {
                        "minecraft:emerald"
                    } else if odds < 0.2 {
                        "minecraft:egg"
                    } else if odds < 0.4 {
                        if self.random.next_boolean() {
                            "minecraft:rabbit_foot"
                        } else {
                            "minecraft:rabbit_hide"
                        }
                    } else if odds < 0.6 {
                        "minecraft:wheat"
                    } else if odds < 0.8 {
                        "minecraft:leather"
                    } else {
                        "minecraft:feather"
                    };
                    set_item(m, MAINHAND, held);
                }
            }
            _ => self.armor(m),
        }
    }
    /// `Mob.populateDefaultEquipmentEnchantments`: the weapon one time in
    /// four and each piece of armour one time in two, scaled by the local
    /// difficulty, through the `mob_spawn_equipment` provider.
    fn enchant(&mut self, m: &mut Mob) {
        if m.species() == Species::WitherSkeleton {
            return;
        }
        self.enchant_slot(m, MAINHAND, 0.25);
        // Pillager.enchantSpawnedWeapon: one crossbow in 300 pierces
        // (`pillager_spawn_crossbow`).
        if m.species() == Species::Pillager
            && self.int(300) == 0
            && item(m, MAINHAND).as_deref() == Some("minecraft:crossbow")
        {
            gear::upgrade(m, gear::MAIN, "minecraft:piercing", 1);
        }
        // EquipmentSlot.VALUES order: feet, legs, chest, head.
        for slot in [FEET, LEGS, CHEST, HEAD] {
            self.enchant_slot(m, slot, 0.5);
        }
    }
    fn enchant_slot(&mut self, m: &mut Mob, slot: usize, chance: f32) {
        let Some(id) = item(m, slot) else { return };
        if self.float() >= chance * self.special() {
            return;
        }
        self.enchant_by_cost(m, slot, &id);
    }
    /// `EnchantmentsByCostWithDifficulty.enchant` (`mob_spawn_equipment`:
    /// cost 5 to 5 + 17 x the special multiplier, `#on_mob_spawn_equipment`).
    fn enchant_by_cost(&mut self, m: &mut Mob, slot: usize, id: &str) {
        let top = 5 + (self.special() * 17.0) as i32;
        let cost = self.random.next_int_between_inclusive(5, top).unwrap_or(5);
        let e = self.data.enchantments;
        let pool = e.tag("minecraft:on_mob_spawn_equipment");
        let picked = e.select(self.data.items, self.random, id, cost, pool);
        for (enchantment, level) in picked {
            gear::upgrade(m, gear_slot(slot), &enchantment, level);
        }
    }
    fn halloween(&mut self, m: &mut Mob) {
        if m.extra.head.is_none() && self.level.halloween() && self.float() < 0.25 {
            let head = if self.float() < 0.1 {
                "minecraft:jack_o_lantern"
            } else {
                "minecraft:carved_pumpkin"
            };
            set_item(m, HEAD, head);
            set_drop_chance(m, HEAD, 0.0);
        }
    }

    // ---- jockeys ------------------------------------------------------

    fn create(&mut self, kind: &str, reason: Reason, at: &Mob) -> Option<Mob> {
        let mut j = self.level.create(kind, reason)?;
        // snapTo(x, y, z, yRot, 0).
        j.position = at.position;
        j.previous = at.position;
        j.yaw = at.yaw;
        j.previous_yaw = at.yaw;
        j.body_yaw = at.yaw;
        j.head_yaw = at.yaw;
        j.head_pitch = 0.0;
        Some(j)
    }
    /// `startRiding`: the rider leaves the vehicle it was on.
    fn ride(&mut self, rider: &mut Mob, vehicle: &mut Mob) {
        if let Some(old) = rider.extra.mount
            && let Some(v) = self.created.iter_mut().find(|c| c.id == old)
            && v.extra.rider == Some(rider.id)
        {
            v.extra.rider = None;
        }
        rider.extra.mount = Some(vehicle.id);
        rider.extra.passenger = true;
        if vehicle.extra.rider.is_none() {
            vehicle.extra.rider = Some(rider.id);
        }
    }

    // ---- the classes --------------------------------------------------

    fn zombie(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.mob(m, g);
        let special = self.special();
        if reason != Reason::Conversion {
            m.extra.pick_up_loot = self.float() < 0.55 * special;
        }
        if g.is_none() {
            g = Some(Group::Zombie {
                baby: self.float() < 0.05,
                jockey: true,
                husk: None,
            });
        }
        if let Some(Group::Zombie { baby, jockey, .. }) = g {
            if baby {
                set_baby(m, true);
                if jockey {
                    if f64::from(self.float()) < 0.05 {
                        let search = m
                            .bounds()
                            .inflate(horizon_core::geometry::Vec3::new(5.0, 3.0, 5.0));
                        if let Some(chicken) = self.level.free_chicken(search) {
                            self.level.ride_chicken(chicken, m.id);
                            m.extra.mount = Some(chicken);
                            m.extra.passenger = true;
                        }
                    } else if f64::from(self.float()) < 0.05
                        && let Some(mut chicken) =
                            self.create("minecraft:chicken", Reason::Jockey, m)
                    {
                        self.finalize(&mut chicken, Reason::Jockey, None);
                        chicken.extra.jockey = true;
                        self.ride(m, &mut chicken);
                        self.created.push(chicken);
                    }
                }
            }
            let can_break_doors = self.float() < special * 0.1;
            super::doors::set_can_break_doors(m, can_break_doors);
            if reason != Reason::Conversion {
                self.populate(m);
                self.enchant(m);
            }
        }
        self.halloween(m);
        zombie_attributes(m, special, reason);
        g
    }
    fn husk(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.zombie(m, g, reason);
        if reason != Reason::Conversion {
            m.extra.pick_up_loot = self.float() < 0.55 * self.special();
        }
        if let Some(Group::Zombie { baby, jockey, .. }) = g {
            g = Some(Group::Zombie {
                baby,
                jockey,
                husk: Some(reason != Reason::Natural),
            });
        }
        if let Some(Group::Zombie {
            husk: Some(tried @ false),
            ..
        }) = &mut g
        {
            let (bx, by, bz) = (
                m.position.x.floor(),
                m.position.y.floor(),
                m.position.z.floor(),
            );
            let (w, h) = self.data.camel_husk_size;
            let half = w / 2.0;
            let bounds = Aabb::new(
                horizon_core::geometry::Vec3::new(
                    bx + 0.5 - f64::from(half),
                    by,
                    bz + 0.5 - f64::from(half),
                ),
                horizon_core::geometry::Vec3::new(
                    bx + 0.5 + f64::from(half),
                    by + f64::from(h),
                    bz + 0.5 + f64::from(half),
                ),
            );
            if self.level.no_collision(bounds) {
                *tried = true;
                if self.float() < 0.1 {
                    set_item(m, MAINHAND, "minecraft:iron_spear");
                    if let Some(mut camel) =
                        self.level.create("minecraft:camel_husk", Reason::Natural)
                    {
                        // setPos: the husk's position, the camel's own rotation.
                        camel.position = m.position;
                        camel.previous = m.position;
                        self.finalize(&mut camel, reason, None);
                        self.ride(m, &mut camel);
                        if let Some(mut parched) =
                            self.create("minecraft:parched", Reason::Natural, m)
                        {
                            self.finalize(&mut parched, reason, None);
                            self.ride(&mut parched, &mut camel);
                            self.created.push(camel);
                            self.created.push(parched);
                        } else {
                            self.created.push(camel);
                        }
                    }
                }
            }
        }
        g
    }
    fn drowned(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let g = self.zombie(m, g, reason);
        if m.extra.off_hand.is_none() && self.float() < 0.03 {
            set_item(m, OFFHAND, "minecraft:nautilus_shell");
            set_drop_chance(m, OFFHAND, 2.0);
        }
        let at = [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ];
        if matches!(reason, Reason::Natural | Reason::Structure)
            && m.extra.main_hand.as_deref() == Some("minecraft:trident")
            && self.float() < 0.5
            && !m.is_baby()
            && !self
                .level
                .biome_tag("minecraft:more_frequent_drowned_spawns", at)
            && let Some(mut nautilus) = self.create("minecraft:zombie_nautilus", Reason::Jockey, m)
        {
            if reason == Reason::Structure {
                nautilus.persistent = true;
            }
            self.finalize(&mut nautilus, reason, None);
            self.ride(m, &mut nautilus);
            self.created.push(nautilus);
        }
        g
    }
    fn skeleton(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let g = self.mob(m, g);
        self.populate(m);
        self.enchant(m);
        m.extra.pick_up_loot = self.float() < 0.55 * self.special();
        self.halloween(m);
        if m.species() == Species::WitherSkeleton {
            set_base(m, Attr::AttackDamage, 4.0);
        }
        g
    }
    fn spider(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let mut g = self.mob(m, g);
        if self.int(100) == 0
            && let Some(mut skeleton) = self.create("minecraft:skeleton", Reason::Jockey, m)
        {
            self.finalize(&mut skeleton, reason, None);
            self.ride(&mut skeleton, m);
            self.created.push(skeleton);
        }
        if g.is_none() {
            let mut effect = None;
            if self.difficulty.hard && self.float() < 0.1 * self.special() {
                effect = Some(match self.int(5) {
                    0 | 1 => "minecraft:speed",
                    2 => "minecraft:strength",
                    3 => "minecraft:regeneration",
                    _ => "minecraft:invisibility",
                });
            }
            g = Some(Group::Spider(effect));
        }
        if let Some(Group::Spider(Some(effect))) = g {
            m.add_effect(effect, -1, 0);
        }
        g
    }
    /// `Raider` then `PatrollingMonster.finalizeSpawn`.
    fn raider(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        let witch = m.species() == Species::Witch;
        super::raider::state_mut(m).can_join_raid = !witch || reason != Reason::Natural;
        if !matches!(reason, Reason::Patrol | Reason::Event | Reason::Structure)
            && self.float() < 0.06
            && super::raider::can_be_leader(m.species())
        {
            super::raider::state_mut(m).patrol_leader = true;
        }
        if super::raider::state(m).is_some_and(|s| s.patrol_leader) {
            set_item(m, HEAD, super::raider::BANNER);
            set_drop_chance(m, HEAD, 2.0);
        }
        if reason == Reason::Patrol {
            super::raider::state_mut(m).patrolling = true;
        }
        self.mob(m, g)
    }
    fn piglin(&mut self, m: &mut Mob, g: Option<Group>, reason: Reason) -> Option<Group> {
        if reason != Reason::Structure {
            if self.float() < 0.2 {
                set_baby(m, true);
            } else if !m.is_baby() {
                // createSpawnWeapon, from the piglin's own random.
                let r = &mut m.random;
                let weapon = if f64::from(r.next_float()) < 0.5 {
                    "minecraft:crossbow"
                } else if r.next_int_bound(10).unwrap_or(1) == 0 {
                    "minecraft:golden_spear"
                } else {
                    "minecraft:golden_sword"
                };
                set_item(m, MAINHAND, weapon);
            }
        }
        // PiglinAi.initMemories: HUNTED_RECENTLY for 30 to 120 seconds.
        let hunt = self.uniform(600, 2400);
        set_memory(m, "hunted_recently", 1, i64::from(hunt));
        self.populate(m);
        self.enchant(m);
        self.mob(m, g)
    }
    fn strider(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        if m.is_baby() {
            return self.ageable(m, g);
        }
        let mut g = g;
        if self.int(30) == 0 && self.level.spawning_monsters() {
            if let Some(mut jockey) = self.create("minecraft:zombified_piglin", Reason::Jockey, m) {
                let baby = self.float() < 0.05;
                g = Some(self.strider_jockey(
                    m,
                    &mut jockey,
                    Some(Group::Zombie {
                        baby,
                        jockey: false,
                        husk: None,
                    }),
                ));
                set_item(&mut jockey, MAINHAND, "minecraft:warped_fungus_on_a_stick");
                set_item(m, SADDLE, "minecraft:saddle");
                set_drop_chance(m, SADDLE, 2.0);
                self.created.push(jockey);
            }
        } else if self.int(10) == 0 {
            if let Some(mut jockey) = self.create("minecraft:strider", Reason::Jockey, m) {
                set_age(&mut jockey, -24000);
                g = Some(self.strider_jockey(m, &mut jockey, None));
                self.created.push(jockey);
            }
        } else {
            g = Some(Group::Ageable(Ageable::chance(0.5)));
        }
        self.ageable(m, g)
    }
    /// `Strider.spawnJockey`.
    fn strider_jockey(&mut self, m: &mut Mob, jockey: &mut Mob, g: Option<Group>) -> Group {
        jockey.position = m.position;
        jockey.yaw = m.yaw;
        self.finalize(jockey, Reason::Jockey, g);
        self.ride(jockey, m);
        Group::Ageable(Ageable::chance(0.0))
    }
    /// `AbstractHorse.finalizeSpawn` with each `randomizeAttributes`.
    fn horse(&mut self, m: &mut Mob, g: Option<Group>) -> Option<Group> {
        let g = g.or(Some(Group::Ageable(Ageable::chance(0.2))));
        use Species as S;
        match m.species() {
            S::Horse => {
                let health = self.horse_health();
                set_base(m, Attr::MaxHealth, f64::from(health));
                let speed = self.horse_speed();
                set_base(m, Attr::MovementSpeed, speed);
                let jump = self.horse_jump();
                set_base(m, Attr::JumpStrength, jump);
            }
            S::Donkey | S::Mule | S::Llama | S::TraderLlama => {
                let health = self.horse_health();
                set_base(m, Attr::MaxHealth, f64::from(health));
            }
            S::SkeletonHorse => {
                let jump = self.horse_jump();
                set_base(m, Attr::JumpStrength, jump);
            }
            S::ZombieHorse => {
                let d = 0.066_666_666_666_666_67;
                let jump = 0.5
                    + self.random.next_double() * d
                    + self.random.next_double() * d
                    + self.random.next_double() * d;
                set_base(m, Attr::JumpStrength, jump);
                let speed = (9.0
                    + self.random.next_double()
                    + self.random.next_double()
                    + self.random.next_double())
                    / f64::from(42.16f32);
                set_base(m, Attr::MovementSpeed, speed);
            }
            _ => {}
        }
        self.ageable(m, g)
    }
    fn horse_health(&mut self) -> f32 {
        15.0 + self.int(8) as f32 + self.int(9) as f32
    }
    fn horse_speed(&mut self) -> f64 {
        (f64::from(0.45f32)
            + self.random.next_double() * 0.3
            + self.random.next_double() * 0.3
            + self.random.next_double() * 0.3)
            * 0.25
    }
    fn horse_jump(&mut self) -> f64 {
        f64::from(0.4f32)
            + self.random.next_double() * 0.2
            + self.random.next_double() * 0.2
            + self.random.next_double() * 0.2
    }
    fn villager_type(&mut self, m: &mut Mob) {
        let at = block(m);
        m.extra.variant = villager_type(&self.level.biome(at));
    }
    /// `VariantUtils.selectVariantToSpawn` into `variant`, by registry name.
    fn variant(&mut self, m: &mut Mob, registry: fn(&SpawnRegistries) -> &VariantRegistry) {
        let r = registry(self.data.registries);
        let at = block(m);
        if let Some(i) = r.select(&*self.level, at, self.random) {
            let name = r.names[i].clone();
            m.extra.variant = variant_index(m.species(), &name);
        }
    }
    fn sound(&mut self, m: &mut Mob, registry: fn(&SpawnRegistries) -> &VariantRegistry) {
        let r = registry(self.data.registries);
        m.extra.sound_variant = Some(r.random(self.random) as u8);
    }

    /// `finalizeSpawn` of the mob's class.
    pub fn finalize(&mut self, m: &mut Mob, reason: Reason, g: Option<Group>) -> Option<Group> {
        use Species as S;
        let s = m.species();
        match s {
            S::Pig | S::Cow | S::Chicken => {
                let (v, sound): (
                    fn(&SpawnRegistries) -> &VariantRegistry,
                    fn(&SpawnRegistries) -> &VariantRegistry,
                ) = match s {
                    S::Pig => (|r| &r.pig, |r| &r.pig_sound),
                    S::Cow => (|r| &r.cow, |r| &r.cow_sound),
                    _ => (|r| &r.chicken, |r| &r.chicken_sound),
                };
                self.variant(m, v);
                self.sound(m, sound);
                self.ageable(m, g)
            }
            S::Cat => {
                let g = self.ageable(m, g);
                self.variant(m, |r| &r.cat);
                self.sound(m, |r| &r.cat_sound);
                g
            }
            S::Wolf => {
                let g = match g {
                    Some(Group::Wolf(a, v)) => {
                        m.extra.variant = v;
                        Some(Group::Wolf(a, v))
                    }
                    other => {
                        let r = &self.data.registries.wolf;
                        let at = block(m);
                        match r.select(&*self.level, at, self.random) {
                            Some(i) => {
                                let v = variant_index(s, &r.names[i]);
                                m.extra.variant = v;
                                Some(Group::Wolf(Ageable::babies(false), v))
                            }
                            None => other,
                        }
                    }
                };
                self.sound(m, |r| &r.wolf_sound);
                self.ageable(m, g)
            }
            S::Frog => {
                self.variant(m, |r| &r.frog);
                let jump = self.uniform(100, 140);
                set_memory(m, "long_jump_cooldown", i64::from(jump), i64::MAX);
                self.ageable(m, g)
            }
            S::ZombieNautilus => {
                self.variant(m, |r| &r.zombie_nautilus);
                let cooldown = self.uniform(2400, 3600);
                set_memory(m, "attack_target_cooldown", i64::from(cooldown), i64::MAX);
                m.extra.counter = cooldown;
                self.ageable(m, g)
            }
            S::Nautilus => {
                let cooldown = self.uniform(2400, 3600);
                set_memory(m, "attack_target_cooldown", i64::from(cooldown), i64::MAX);
                m.extra.counter = cooldown;
                self.ageable(m, g)
            }
            S::Sheep => {
                let climate = if self
                    .level
                    .biome_tag("minecraft:spawns_warm_variant_farm_animals", block(m))
                {
                    Climate::Warm
                } else if self
                    .level
                    .biome_tag("minecraft:spawns_cold_variant_farm_animals", block(m))
                {
                    Climate::Cold
                } else {
                    Climate::Temperate
                };
                m.extra.color = super::farm::sheep_color(climate, self.random);
                self.ageable(m, g)
            }
            S::Rabbit => {
                let at = block(m);
                let roll = self.int(100);
                let mut variant = if self.level.biome_tag("minecraft:spawns_white_rabbits", at) {
                    if roll < 80 { 1 } else { 3 }
                } else if self.level.biome_tag("minecraft:spawns_gold_rabbits", at) {
                    4
                } else if roll < 50 {
                    0
                } else if roll < 90 {
                    5
                } else {
                    2
                };
                let g = match g {
                    Some(Group::Rabbit(a, v)) => {
                        variant = v;
                        Group::Rabbit(a, v)
                    }
                    _ => Group::Rabbit(Ageable::chance(1.0), variant),
                };
                m.extra.variant = variant;
                self.ageable(m, Some(g))
            }
            S::Horse => {
                let (g, variant) = match g {
                    Some(Group::Horse(a, v)) => (Group::Horse(a, v), v),
                    _ => {
                        let v = self.int(7) as u8;
                        (Group::Horse(Ageable::babies(true), v), v)
                    }
                };
                let markings = self.int(5) as u8;
                m.extra.variant = variant;
                m.extra.variant2 = markings;
                self.horse(m, Some(g))
            }
            S::Llama | S::TraderLlama => {
                let mut g = g;
                if s == S::TraderLlama {
                    if reason == Reason::Event {
                        set_age(m, 0);
                    }
                    if g.is_none() {
                        g = Some(Group::Ageable(Ageable::babies(false)));
                    }
                }
                let max = if self.float() < 0.04 { 5 } else { 3 };
                m.extra.variant2 = (1 + self.int(max)) as u8;
                let g = match g {
                    Some(Group::Llama(a, v)) => {
                        m.extra.variant = v;
                        Group::Llama(a, v)
                    }
                    _ => {
                        let v = self.int(4) as u8;
                        m.extra.variant = v;
                        Group::Llama(Ageable::babies(true), v)
                    }
                };
                self.horse(m, Some(g))
            }
            S::Donkey | S::Mule | S::SkeletonHorse => self.horse(m, g),
            S::ZombieHorse => {
                if reason == Reason::Natural
                    && let Some(mut zombie) = self.create("minecraft:zombie", Reason::Jockey, m)
                {
                    self.finalize(&mut zombie, reason, None);
                    set_item(&mut zombie, MAINHAND, "minecraft:iron_spear");
                    self.ride(&mut zombie, m);
                    self.created.push(zombie);
                }
                self.horse(m, g)
            }
            S::Camel | S::CamelHusk => {
                let now = self.level.game_time();
                m.extra.pose_change_tick = (now - 52 - 1).max(0);
                super::camel::stand_up_instantly(m);
                self.horse(m, g)
            }
            S::Goat => {
                let jump = self.uniform(600, 1200);
                set_memory(m, "long_jump_cooldown", i64::from(jump), i64::MAX);
                let ram = self.uniform(600, 6000);
                set_memory(m, "ram_cooldown", i64::from(ram), i64::MAX);
                m.extra.screaming = self.random.next_double() < 0.02;
                age_boundary_reached(m);
                if !m.is_baby() && self.float() < 0.1 {
                    let left = self.random.next_boolean();
                    m.extra.lost_horns[usize::from(!left)] = true;
                }
                self.ageable(m, g)
            }
            S::Panda => {
                let main = panda_gene(self.int(16));
                let hidden = panda_gene(self.int(16));
                m.extra.variant = main;
                m.extra.variant2 = hidden;
                super::panda::set_attributes(m);
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.2)))))
            }
            S::PolarBear | S::Ocelot => {
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(1.0)))))
            }
            S::Fox => {
                let at = block(m);
                let mut variant = u8::from(self.level.biome_tag("minecraft:spawns_snow_foxes", at));
                let mut baby = false;
                let g = match g {
                    Some(Group::Fox(a, v)) => {
                        variant = v;
                        baby = a.size >= 2;
                        Group::Fox(a, v)
                    }
                    _ => Group::Fox(Ageable::babies(false), variant),
                };
                m.extra.variant = variant;
                if baby {
                    set_age(m, -24000);
                }
                self.populate(m);
                self.ageable(m, Some(g))
            }
            S::Parrot => {
                m.extra.variant = self.int(5) as u8;
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::babies(false)))))
            }
            S::Axolotl => {
                if reason == Reason::Bucket {
                    return g;
                }
                let mut baby = false;
                let g = match g {
                    Some(Group::Axolotl(a, types)) => {
                        baby = a.size >= 2;
                        Group::Axolotl(a, types)
                    }
                    _ => {
                        let a = self.int(4) as u8;
                        let b = self.int(4) as u8;
                        Group::Axolotl(Ageable::babies(false), [a, b])
                    }
                };
                if let Group::Axolotl(_, types) = g {
                    m.extra.variant = types[self.int(2) as usize];
                }
                if baby {
                    set_age(m, -24000);
                }
                self.ageable(m, Some(g))
            }
            S::Squid | S::GlowSquid => {
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.05)))))
            }
            S::Dolphin => {
                m.air = 4800;
                m.head_pitch = 0.0;
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::chance(0.1)))))
            }
            S::Turtle => {
                m.extra.home = Some(block(m));
                self.ageable(m, g)
            }
            S::Villager | S::WanderingTrader => {
                if s == S::Villager {
                    if reason == Reason::Breeding {
                        m.extra.variant2 = 0;
                    }
                    self.villager_type(m);
                }
                self.ageable(m, g.or(Some(Group::Ageable(Ageable::babies(false)))))
            }
            S::Hoglin => {
                if self.float() < 0.2 {
                    set_baby(m, true);
                }
                self.ageable(m, g)
            }
            S::Zoglin => {
                if self.float() < 0.2 {
                    set_baby(m, true);
                }
                self.mob(m, g)
            }
            S::Strider => self.strider(m, g),
            S::Slime | S::MagmaCube | S::SulfurCube => {
                let g = if s == S::SulfurCube {
                    g
                } else {
                    g.or(Some(Group::Ageable(Ageable::babies(false))))
                };
                let g = self.ageable(m, g);
                if s == S::SulfurCube {
                    let size = if m.is_baby() { 1 } else { 2 };
                    sulfur_size(m, size);
                } else {
                    let mut scale = self.int(3);
                    if scale < 2 && self.float() < 0.5 * self.special() {
                        scale += 1;
                    }
                    cube_size(m, 1 << scale);
                }
                g
            }
            S::Cod | S::Salmon | S::TropicalFish => {
                if s == S::Salmon {
                    // A weighted list (small 30, medium 50, large 15) on the
                    // salmon's own random.
                    let r = m.random.next_int_bound(95).unwrap_or(0);
                    m.extra.variant = if r < 30 {
                        0
                    } else if r < 80 {
                        1
                    } else {
                        2
                    };
                }
                let given = g;
                self.mob(m, None);
                let g = match given {
                    None => Group::School(m.id, None),
                    Some(Group::School(leader, v)) => {
                        m.extra.leader = Some(leader);
                        Group::School(leader, v)
                    }
                    Some(other) => other,
                };
                if s != S::TropicalFish {
                    return Some(g);
                }
                let (g, (p, b, c)) = match g {
                    Group::School(leader, Some(v)) => (Group::School(leader, Some(v)), v),
                    other => {
                        if f64::from(self.float()) < 0.9 {
                            let v = super::aquatic::COMMON_TROPICAL[self.int(22) as usize];
                            (Group::School(m.id, Some(v)), v)
                        } else {
                            m.extra.lone = true;
                            let p = self.int(12) as u8;
                            let b = self.int(16) as u8;
                            let c = self.int(16) as u8;
                            (other, (p, b, c))
                        }
                    }
                };
                m.extra.variant = p;
                m.extra.variant2 = b;
                m.extra.variant3 = c;
                Some(g)
            }
            S::Zombie | S::ZombifiedPiglin => self.zombie(m, g, reason),
            S::ZombieVillager => {
                self.villager_type(m);
                // ZombieVillager's constructor picks the profession from the
                // fresh entity's random, before anything seeds it: an
                // independent draw that leaves the mob's own random alone.
                let mut fresh = LegacyRandom::new(
                    (m.random.state() as i64).wrapping_mul(0x5851_F42D_4C95_7F2D) ^ i64::from(m.id),
                );
                m.extra.variant2 = fresh.next_int_bound(15).unwrap_or(0) as u8;
                self.zombie(m, g, reason)
            }
            S::Husk => self.husk(m, g, reason),
            S::Drowned => self.drowned(m, g, reason),
            s if skeleton(s) => self.skeleton(m, g),
            S::Spider => self.spider(m, g, reason),
            // CaveSpider.finalizeSpawn returns at once.
            S::CaveSpider => g,
            S::Pillager => {
                self.populate(m);
                self.enchant(m);
                self.raider(m, g, reason)
            }
            S::Vindicator => {
                let g = self.raider(m, g, reason);
                self.populate(m);
                self.enchant(m);
                g
            }
            S::Illusioner => {
                set_item(m, MAINHAND, "minecraft:bow");
                self.raider(m, g, reason)
            }
            s if raider(s) => self.raider(m, g, reason),
            S::Piglin => self.piglin(m, g, reason),
            S::PiglinBrute => {
                let home = block(m);
                m.extra.home = Some(home);
                set_memory(m, "home", 0, i64::MAX);
                self.populate(m);
                self.mob(m, g)
            }
            S::Vex => {
                self.populate(m);
                self.enchant(m);
                self.mob(m, g)
            }
            S::Warden => {
                super::warden::finalize(m, reason == Reason::Triggered, &mut Vec::new());
                set_memory(m, "dig_cooldown", 0, 1200);
                if reason == Reason::Triggered {
                    set_memory(m, "is_emerging", 0, super::warden::EMERGE_DURATION);
                }
                self.mob(m, g)
            }
            S::Phantom => {
                let at = block(m);
                m.extra.home = Some([at[0], at[1] + 5, at[2]]);
                m.extra.size = 0;
                self.mob(m, g)
            }
            S::Shulker => {
                m.yaw = 0.0;
                m.head_yaw = 0.0;
                m.previous_yaw = 0.0;
                m.previous_head_yaw = 0.0;
                self.mob(m, g)
            }
            _ if ageable(s) => self.ageable(m, g),
            _ => self.mob(m, g),
        }
    }
}

/// `Mob.finalizeSpawn` of a fresh mob. Returns the group data for the next
/// mob of the pack.
pub fn finalize<L: FinalizeLevel + ?Sized, R: RandomSource>(
    cx: &mut Ctx<'_, L, R>,
    m: &mut Mob,
    reason: Reason,
    g: Option<Group>,
) -> Option<Group> {
    cx.finalize(m, reason, g)
}

fn block(m: &Mob) -> [i32; 3] {
    [
        m.position.x.floor() as i32,
        m.position.y.floor() as i32,
        m.position.z.floor() as i32,
    ]
}
/// A brain memory finalizeSpawn sets: value and ticks until it expires.
fn set_memory(m: &mut Mob, name: &'static str, value: i64, ttl: i64) {
    match m.extra.memories.iter_mut().find(|x| x.0 == name) {
        Some(x) => *x = (name, value, ttl),
        None => m.extra.memories.push((name, value, ttl)),
    }
}
/// `Panda.Gene.getRandom` (enum order: normal, lazy, worried, playful,
/// brown, weak, aggressive).
pub fn panda_gene(roll: i32) -> u8 {
    match roll {
        0 => 1,
        1 => 2,
        2 => 3,
        4 => 6,
        r if r < 9 => 5,
        r if r < 11 => 4,
        _ => 0,
    }
}
/// `Panda.Gene.getVariantFromGenes`: a recessive gene shows when both match.
pub fn panda_variant(main: u8, hidden: u8) -> u8 {
    let recessive = matches!(main, 4 | 5);
    if recessive && main != hidden { 0 } else { main }
}
/// `AbstractCubeMob.setSize(size, true)` for slimes and magma cubes.
/// `SulfurCube.setSize(size, true)`, with the attribute bases it sets.
fn sulfur_size(m: &mut Mob, size: i32) {
    super::sulfur_cube::set_size(m, size, true);
    let size = m.extra.size;
    set_base(m, Attr::MaxHealth, f64::from(4 * size));
    set_base(
        m,
        Attr::MovementSpeed,
        f64::from(0.2f32 + 0.1f32 * size as f32),
    );
    m.health = max_health(m);
}
fn cube_size(m: &mut Mob, size: i32) {
    super::cube::set_size(m, size);
}
/// The index the game keeps for a variant registry entry.
pub fn variant_index(s: Species, name: &str) -> u8 {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let i = match s {
        Species::Cat => super::tame::CAT_VARIANTS.iter().position(|v| *v == name),
        Species::Wolf => super::tame::WOLF_VARIANTS.iter().position(|v| v.0 == name),
        Species::ZombieNautilus => super::nautilus::VARIANTS.iter().position(|v| *v == name),
        _ => ["temperate", "warm", "cold"]
            .iter()
            .position(|v| *v == name),
    };
    i.unwrap_or(0) as u8
}
/// The variant data component a mob reports (`Entity.get`, e.g.
/// `Wolf.get(DataComponents.WOLF_VARIANT)`): component id and registry entry.
pub fn variant_component(m: &Mob) -> Option<(&'static str, String)> {
    species_variant_component(m.species(), m.extra.variant)
}
/// [`variant_component`] of a species with the game's variant index.
pub fn species_variant_component(s: Species, variant: u8) -> Option<(&'static str, String)> {
    let component = match s {
        Species::Wolf => "minecraft:wolf/variant",
        Species::Cat => "minecraft:cat/variant",
        Species::Frog => "minecraft:frog/variant",
        Species::Pig => "minecraft:pig/variant",
        Species::Cow => "minecraft:cow/variant",
        Species::Chicken => "minecraft:chicken/variant",
        Species::ZombieNautilus => "minecraft:zombie_nautilus/variant",
        _ => return None,
    };
    Some((component, format!("minecraft:{}", variant_name(s, variant))))
}
/// The `*_sound_variant` registry of a species, in registry order.
pub fn sound_variants(s: Species) -> &'static [&'static str] {
    match s {
        Species::Pig => &["big", "classic", "mini"],
        Species::Cow => &["classic", "moody"],
        Species::Chicken => &["classic", "picky"],
        Species::Cat => &["classic", "royal"],
        Species::Wolf => &["angry", "big", "classic", "cute", "grumpy", "puglin", "sad"],
        _ => &[],
    }
}
/// A sound of the mob's `*_sound_variant` set (`Pig.getSoundSet`,
/// `Wolf.getSoundSet`, `Cat.getSoundSet`, ...): `kind` is the sound's key
/// (`ambient`, `hurt`, `step`, `growl`, `purreow`...). Babies use the species'
/// baby set (cows have none); an adult's set is `entity.<species>.*` for
/// `classic` and `entity.<species>_<variant>.*` otherwise, except the
/// footsteps that every variant shares (not the cow's). None for a species
/// without sound variants.
pub fn variant_sound(m: &Mob, kind: &str) -> Option<&'static str> {
    use std::{collections::HashMap, sync::Mutex};
    let species = m.species();
    let name = match species {
        Species::Pig => "pig",
        Species::Cow => "cow",
        Species::Chicken => "chicken",
        Species::Wolf => "wolf",
        Species::Cat => "cat",
        _ => return None,
    };
    let variant = m
        .extra
        .sound_variant
        .and_then(|i| sound_variants(species).get(i as usize))
        .copied()
        .unwrap_or("classic");
    let id = if m.is_baby() && species != Species::Cow {
        format!("entity.baby_{name}.{kind}")
    } else if variant == "classic" || (kind == "step" && species != Species::Cow) {
        format!("entity.{name}.{kind}")
    } else {
        format!("entity.{name}_{variant}.{kind}")
    };
    // A handful of ids in all: keep each one as a `&'static str`.
    static IDS: Mutex<Option<HashMap<String, &'static str>>> = Mutex::new(None);
    let mut ids = IDS.lock().ok()?;
    let ids = ids.get_or_insert_with(HashMap::new);
    Some(
        *ids.entry(id)
            .or_insert_with_key(|k| Box::leak(k.clone().into_boxed_str())),
    )
}
/// The registry name of the game's variant index.
pub fn variant_name(s: Species, index: u8) -> &'static str {
    let i = index as usize;
    match s {
        Species::Cat => super::tame::CAT_VARIANTS.get(i).copied().unwrap_or("tabby"),
        Species::Wolf => super::tame::WOLF_VARIANTS.get(i).map_or("pale", |v| v.0),
        Species::ZombieNautilus => super::nautilus::VARIANTS
            .get(i)
            .copied()
            .unwrap_or("temperate"),
        _ => ["temperate", "warm", "cold"]
            .get(i)
            .copied()
            .unwrap_or("temperate"),
    }
}
/// `VillagerType.byBiome` (index into `render::VILLAGER_TYPES`: plains,
/// desert, jungle, savanna, snow, swamp, taiga).
pub fn villager_type(biome: &str) -> u8 {
    match biome.strip_prefix("minecraft:").unwrap_or(biome) {
        "badlands" | "desert" | "eroded_badlands" | "wooded_badlands" => 1,
        "bamboo_jungle" | "jungle" | "sparse_jungle" => 2,
        "savanna_plateau" | "savanna" | "windswept_savanna" => 3,
        "deep_frozen_ocean" | "frozen_ocean" | "frozen_river" | "ice_spikes" | "snowy_beach"
        | "snowy_taiga" | "snowy_plains" | "grove" | "snowy_slopes" | "frozen_peaks"
        | "jagged_peaks" => 4,
        "swamp" | "mangrove_swamp" => 5,
        "old_growth_spruce_taiga"
        | "old_growth_pine_taiga"
        | "windswept_gravelly_hills"
        | "windswept_hills"
        | "taiga"
        | "windswept_forest" => 6,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panda_genes_follow_the_vanilla_table() {
        assert_eq!(panda_gene(0), 1);
        assert_eq!(panda_gene(3), 5);
        assert_eq!(panda_gene(4), 6);
        assert_eq!(panda_gene(9), 4);
        assert_eq!(panda_gene(15), 0);
        assert_eq!(panda_variant(4, 0), 0);
        assert_eq!(panda_variant(4, 4), 4);
        assert_eq!(panda_variant(6, 0), 6);
    }
    /// #2269: the offspring of a spawn egg used on a mob is a baby of that
    /// kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`), and only the mob's
    /// own kind takes the egg.
    #[test]
    fn a_spawn_egg_offspring_is_a_baby() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let data = crate::mobs::MobTypes::load(&root).unwrap();
        let kind = std::sync::Arc::new(data.types.get("minecraft:cow").unwrap().clone());
        let mut child = crate::mobs::Mob::new(
            2,
            kind,
            horizon_core::geometry::Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        assert!(!child.is_baby());
        spawn_egg_offspring(&mut child, Species::Cow, 3, 0);
        // The offspring is a baby of the mob's kind.
        assert!(child.is_baby());
        // It keeps the parent's variant.
        assert_eq!(child.extra.variant, 3);
        // The egg is for the mob's own kind.
        assert!(spawn_egg_is_for("minecraft:cow_spawn_egg", "minecraft:cow"));
        assert!(!spawn_egg_is_for(
            "minecraft:pig_spawn_egg",
            "minecraft:cow"
        ));
        assert!(!spawn_egg_is_for("minecraft:stick", "minecraft:cow"));
    }
    #[test]
    fn reasons_round_trip() {
        for r in Reason::ALL {
            assert_eq!(Reason::parse(r.name()), Some(r));
        }
    }
    #[test]
    fn villager_types_by_biome() {
        assert_eq!(villager_type("minecraft:badlands"), 1);
        assert_eq!(villager_type("minecraft:grove"), 4);
        assert_eq!(villager_type("minecraft:plains"), 0);
    }
}
