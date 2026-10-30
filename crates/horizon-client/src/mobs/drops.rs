//! What a dying mob leaves behind: `LivingEntity.die`'s drop step. The
//! killer's `killedEntity` hook (a charged creeper's head drop), then
//! `dropAllDeathLoot`: the entity loot table (drawn from the world's random
//! sequence), `Mob.dropCustomDeathLoot` (equipment by drop chance, with the
//! entity random), the species' extra drops and the experience orbs.
use super::{Mob, Species, equine, gear, raider, render::DYE_NAMES, species};
use crate::random_sequences::RandomSequences;
use horizon_core::random::RandomSource;
use horizon_world::loot::{EntityLootParams, LootDamage, LootData, LootEntity, LootStack};

/// `DropChances.DEFAULT_EQUIPMENT_DROP_CHANCE`.
pub const DEFAULT_DROP_CHANCE: f32 = 0.085;
/// `EquipmentSlot.VALUES`.
pub use horizon_world::loot::EQUIPMENT_SLOTS as SLOTS;

/// One thing a death leaves in the world, in spawn order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drop {
    Item(LootStack),
    /// An experience orb of this value.
    Orb(i32),
}

/// How the mob died.
pub struct Death<'a> {
    pub loot: &'a LootData,
    /// The killing `DamageSource`.
    pub damage: Option<LootDamage>,
    /// The player who last hurt the mob, when `lastHurtByPlayerMemoryTime > 0`.
    pub player: Option<LootEntity>,
    /// That player's luck.
    pub luck: f32,
    /// The entity the mob rides.
    pub vehicle: Option<LootEntity>,
    /// The `doMobLoot` game rule.
    pub mob_drops: bool,
    pub world_seed: i64,
}

/// `instanceof Monster` (Monster.shouldDropLoot/shouldDropExperience).
pub fn is_monster(s: Species) -> bool {
    use Species as S;
    matches!(
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
            | S::Giant
            | S::Guardian
            | S::Husk
            | S::Illusioner
            | S::Parched
            | S::Piglin
            | S::PiglinBrute
            | S::Pillager
            | S::Ravager
            | S::Silverfish
            | S::Skeleton
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
/// `Mob.xpReward` as the constructors set it.
fn xp_reward(m: &Mob) -> i32 {
    use Species as S;
    match m.species() {
        S::Blaze | S::Breeze | S::Guardian | S::ElderGuardian | S::Evoker => 10,
        S::Vex | S::Endermite => 3,
        S::PiglinBrute | S::Ravager => 20,
        S::Wither => 50,
        S::Creaking => 0,
        // Hoglin.setBaby.
        S::Hoglin if m.is_baby() => 3,
        S::Ghast | S::Hoglin | S::Phantom | S::Shulker => 5,
        // Slime.setSize / MagmaCube.setSize (unsized cubes keep 0).
        S::Slime | S::MagmaCube => m.extra.size.max(0),
        s if is_monster(s) => 5,
        _ => 0,
    }
}
/// The mobs that extend `Animal`.
pub fn is_animal(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Armadillo
            | S::Axolotl
            | S::Bee
            | S::Camel
            | S::CamelHusk
            | S::Cat
            | S::Chicken
            | S::Cow
            | S::Donkey
            | S::Fox
            | S::Frog
            | S::HappyGhast
            | S::Goat
            | S::Hoglin
            | S::Horse
            | S::Llama
            | S::Mooshroom
            | S::Mule
            | S::Nautilus
            | S::Ocelot
            | S::Panda
            | S::Parrot
            | S::Pig
            | S::PolarBear
            | S::Rabbit
            | S::Sheep
            | S::SkeletonHorse
            | S::Sniffer
            | S::Strider
            | S::TraderLlama
            | S::Turtle
            | S::Wolf
            | S::ZombieHorse
            | S::ZombieNautilus
    )
}
/// AgeableWaterCreature and WaterAnimal.
fn is_water_creature(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Dolphin
            | S::GlowSquid
            | S::Squid
            | S::Cod
            | S::Salmon
            | S::Pufferfish
            | S::TropicalFish
            | S::Tadpole
    )
}
fn zombie_family(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager | S::ZombifiedPiglin
    )
}
fn is_raider(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Pillager | S::Vindicator | S::Evoker | S::Illusioner | S::Witch | S::Ravager
    )
}
/// Whether `setBaby(true)` makes the species a baby (AgeableMob and the
/// zombies, minus the species that are never babies).
pub fn can_be_baby(s: Species) -> bool {
    use Species as S;
    matches!(
        s,
        S::Armadillo
            | S::Axolotl
            | S::Bee
            | S::Camel
            | S::Cat
            | S::Chicken
            | S::Cow
            | S::Dolphin
            | S::Donkey
            | S::Drowned
            | S::Fox
            | S::HappyGhast
            | S::GlowSquid
            | S::Goat
            | S::Hoglin
            | S::Horse
            | S::Husk
            | S::Llama
            | S::Mooshroom
            | S::Mule
            | S::Nautilus
            | S::Ocelot
            | S::Panda
            | S::Pig
            | S::Piglin
            | S::PolarBear
            | S::Rabbit
            | S::Sheep
            | S::SkeletonHorse
            | S::Sniffer
            | S::Squid
            | S::Strider
            | S::SulfurCube
            | S::TraderLlama
            | S::Turtle
            | S::Villager
            | S::WanderingTrader
            | S::Wolf
            | S::Zoglin
            | S::Zombie
            | S::ZombieHorse
            | S::ZombieVillager
            | S::ZombifiedPiglin
    )
}
/// Whether the species keeps its body armour in `extra.head`.
pub fn head_is_body(s: Species) -> bool {
    species::body_slot(s) || equine::is_equine(s)
}

/// The item a mob has in an `EquipmentSlot` (by name), with its enchantments.
pub fn slot_item(m: &Mob, slot: &str) -> Option<LootStack> {
    let body = head_is_body(m.species());
    let (item, index) = match slot {
        "mainhand" => (m.extra.main_hand.as_deref(), Some(gear::MAIN)),
        "offhand" => (m.extra.off_hand.as_deref(), Some(gear::OFF)),
        "head" if !body => (m.extra.head.as_deref(), Some(gear::HEAD)),
        "chest" => (m.extra.chest.as_deref(), Some(gear::CHEST)),
        "legs" => (m.extra.legs.as_deref(), Some(gear::LEGS)),
        "feet" => (m.extra.feet.as_deref(), Some(gear::FEET)),
        "body" if body => (m.extra.head.as_deref(), None),
        "saddle" => {
            let saddle = m.saddle_item()?;
            let mut stack = loot_stack(&saddle.item, 1);
            stack.damage = saddle.damage as i32;
            for (e, l) in saddle.enchantments() {
                stack.enchantments.push((e.to_string(), *l as i32));
            }
            return Some(stack);
        }
        _ => (None, None),
    };
    // A panda keeps the whole stack it picked up in its mouth.
    let count = if slot == "mainhand" && m.species() == Species::Panda {
        super::panda::held_count(m)
    } else {
        1
    };
    let mut stack = loot_stack(item?, count as i32);
    if slot == "body" && m.species() == Species::Wolf {
        stack.damage = m.extra.body_damage;
    }
    if let (Some(index), Some(g)) = (index, gear::gear(m)) {
        for (e, l) in &g.slots[index] {
            stack.enchantments.push((e.to_string(), *l as i32));
        }
        if let Some((material, pattern)) = &g.trims[index] {
            let trim = serde_json::json!({ "material": &**material, "pattern": &**pattern });
            stack.components.push(("minecraft:trim".to_owned(), trim));
        }
    }
    Some(stack)
}
/// `setItemSlot(slot, EMPTY)`.
pub fn clear_slot(m: &mut Mob, slot: &str) {
    let body = head_is_body(m.species());
    let index = match slot {
        "mainhand" => {
            m.extra.set_main_hand(None);
            gear::MAIN
        }
        "offhand" => {
            m.extra.off_hand = None;
            gear::OFF
        }
        "head" if !body => {
            m.extra.head = None;
            gear::HEAD
        }
        "chest" => {
            m.extra.chest = None;
            gear::CHEST
        }
        "legs" => {
            m.extra.legs = None;
            gear::LEGS
        }
        "feet" => {
            m.extra.feet = None;
            gear::FEET
        }
        "body" if body => {
            m.extra.head = None;
            m.extra.body_damage = 0;
            m.extra.body_extra = None;
            return;
        }
        "saddle" => {
            m.set_saddle(None);
            return;
        }
        _ => return,
    };
    if m.extra.gear.is_some() {
        gear::set(m, index, Vec::new());
    }
}
fn loot_stack(item: &str, count: i32) -> LootStack {
    LootStack {
        item: item.to_owned(),
        count,
        damage: 0,
        enchantments: Vec::new(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components: Vec::new(),
    }
}
/// `Mob.dropChances.byEquipment(slot)`: the recorded chance, or the one the
/// game's spawn and equip paths give (saddles and body armour are put on by
/// `Equippable` and always drop; a captain's banner always drops; a vex
/// never drops its sword).
pub fn drop_chance(m: &Mob, slot: &str) -> f32 {
    let index = SLOTS.iter().position(|s| *s == slot).unwrap_or(0);
    if let Some(chances) = &m.extra.drop_chances {
        return chances[index];
    }
    match slot {
        "saddle" | "body" => 2.0,
        "head" if is_raider(m.species()) && m.extra.head.as_deref() == Some(raider::BANNER) => 2.0,
        "mainhand" if m.species() == Species::Vex => 0.0,
        _ => DEFAULT_DROP_CHANCE,
    }
}
/// The facts loot predicates read about a mob.
pub fn loot_entity(m: &Mob, vehicle: Option<LootEntity>) -> LootEntity {
    use Species as S;
    let s = m.species();
    let mut e = LootEntity::new(&m.kind.id);
    e.on_fire = m.fire_ticks > 0 && !m.kind.flags.fire_immune;
    e.baby = m.is_baby();
    e.vehicle = vehicle.map(Box::new);
    if matches!(s, S::Slime | S::MagmaCube | S::SulfurCube) {
        // An unsized cube has the synced default size 1.
        e.cube_size = Some(m.extra.size.max(1));
    }
    if s == S::Sheep {
        e.sheep = Some((
            DYE_NAMES[m.extra.color as usize & 15].to_owned(),
            m.extra.sheared,
        ));
    }
    if is_raider(s) {
        e.captain = Some(raider::is_captain(m));
    }
    if s == S::Frog {
        e.frog_variant = Some(
            ["minecraft:temperate", "minecraft:warm", "minecraft:cold"]
                [usize::from(m.extra.variant).min(2)]
            .to_owned(),
        );
    }
    if s == S::Chicken {
        e.chicken_variant = super::finalize::variant_component(m).map(|(_, id)| id);
    }
    e.powered = s == S::Creeper && m.extra.charged;
    e.equipment = SLOTS
        .iter()
        .filter_map(|slot| slot_item(m, slot).map(|i| (slot.to_string(), i)))
        .collect();
    e
}
/// `ExperienceOrb.getExperienceValue`.
pub fn orb_value(amount: i32) -> i32 {
    for v in [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3] {
        if amount >= v {
            return v;
        }
    }
    1
}

/// `LivingEntity.die`'s drops for `m` (equipment it loses is removed from
/// it). Returns the drops in spawn order and whether a charged creeper
/// killer dropped a head (`Creeper.droppedSkulls`).
pub fn die(m: &mut Mob, d: &Death, sequences: &mut RandomSequences) -> (Vec<Drop>, bool) {
    let mut out = Vec::new();
    let this = loot_entity(m, d.vehicle.clone());
    let mut skulls = false;
    // Creeper.killedEntity: a charged creeper's victim drops its head.
    if let Some(source) = d.damage.as_ref().and_then(|x| x.source.as_ref())
        && source.kind == "minecraft:creeper"
        && source.powered
        && d.mob_drops
    {
        let params = EntityLootParams {
            this: this.clone(),
            damage: d.damage.clone(),
            player: None,
            luck: 0.0,
        };
        let items = roll(d, "minecraft:charged_creeper/root", &params, sequences);
        skulls = items.iter().any(|i| !i.is_empty());
        out.extend(items.into_iter().filter(|i| !i.is_empty()).map(Drop::Item));
    }
    let killed_by_player = d.player.is_some();
    let s = m.species();
    // Fox.dropAllDeathLoot: whatever it holds in its mouth comes out first.
    if s == Species::Fox
        && let Some(stack) = slot_item(m, "mainhand")
    {
        out.push(Drop::Item(stack));
        clear_slot(m, "mainhand");
    }
    // Allay: what it carries (`ints[1]` of the held item) is its inventory.
    let allay_load = (s == Species::Allay)
        .then(|| {
            m.extra
                .main_hand
                .clone()
                .map(|item| (item, m.extra.ints[1]))
        })
        .flatten();
    let drops_loot = d.mob_drops && (is_monster(s) || !m.is_baby());
    if drops_loot {
        let table = format!(
            "minecraft:entities/{}",
            m.kind.id.trim_start_matches("minecraft:")
        );
        let params = EntityLootParams {
            this: this.clone(),
            damage: d.damage.clone(),
            player: d.player.clone().filter(|_| killed_by_player),
            luck: if killed_by_player { d.luck } else { 0.0 },
        };
        let items = roll(d, &table, &params, sequences);
        out.extend(items.into_iter().filter(|i| !i.is_empty()).map(Drop::Item));
        // Mob.dropCustomDeathLoot.
        let damage_params = EntityLootParams {
            this: this.clone(),
            damage: d.damage.clone(),
            player: None,
            luck: 0.0,
        };
        let living_attacker = d
            .damage
            .as_ref()
            .and_then(|x| x.source.as_ref())
            .is_some_and(|a| a.living);
        for slot in SLOTS {
            let stack = slot_item(m, slot);
            let mut chance = drop_chance(m, slot);
            if chance == 0.0 {
                continue;
            }
            let preserve = chance > 1.0;
            if living_attacker {
                chance = d.loot.equipment_drop_chance(&damage_params, chance);
            }
            let Some(mut stack) = stack else { continue };
            if d.loot.prevents_equipment_drop(&stack) || !(killed_by_player || preserve) {
                continue;
            }
            if m.random.next_float() >= chance {
                continue;
            }
            let max = d.loot.max_damage(&stack.item);
            if !preserve && max > 0 {
                let inner = m.random.next_int_bound((max - 3).max(1)).unwrap_or(0);
                stack.damage = max - m.random.next_int_bound(1 + inner).unwrap_or(0);
            }
            out.push(Drop::Item(stack));
            clear_slot(m, slot);
        }
        // WitherBoss.dropCustomDeathLoot.
        if s == Species::Wither {
            out.push(Drop::Item(loot_stack("minecraft:nether_star", 1)));
        }
    }
    // Allay.dropEquipment: its inventory, then the main-hand item unless it
    // prevents equipment drops.
    if let Some((item, carried)) = allay_load {
        if carried > 0 {
            out.push(Drop::Item(loot_stack(&item, carried)));
        }
        if let Some(stack) = slot_item(m, "mainhand")
            && !d.loot.prevents_equipment_drop(&stack)
        {
            out.push(Drop::Item(stack));
            clear_slot(m, "mainhand");
        }
    }
    // Piglin.dropCustomDeathLoot: everything in its inventory.
    if s == Species::Piglin {
        for stack in m.extra.inventory.drain(..) {
            let mut item = loot_stack(&stack.item, stack.count as i32);
            item.damage = stack.damage as i32;
            out.push(Drop::Item(item));
        }
    }
    // dropEquipment: AbstractHorse's inventory, then a chested horse's chest.
    if equine::is_equine(s) {
        for stack in m.extra.items.iter().flatten() {
            if stack.count > 0 {
                let mut item = loot_stack(&stack.item, stack.count as i32);
                item.damage = stack.damage as i32;
                out.push(Drop::Item(item));
            }
        }
        if m.extra.chested {
            out.push(Drop::Item(loot_stack("minecraft:chest", 1)));
            m.extra.chested = false;
        }
    }
    // dropExperience.
    if killed_by_player && drops_experience(m) && d.mob_drops {
        let mut amount = base_experience(m);
        while amount > 0 {
            let value = orb_value(amount);
            amount -= value;
            out.push(Drop::Orb(value));
        }
    }
    (out, skulls)
}
fn roll(
    d: &Death,
    table: &str,
    params: &EntityLootParams,
    sequences: &mut RandomSequences,
) -> Vec<LootStack> {
    match d.loot.random_sequence(table) {
        Some(id) => {
            let random = sequences.get(id, d.world_seed);
            d.loot.roll_entity(table, random, params)
        }
        None => Vec::new(),
    }
}
/// `LivingEntity.shouldDropExperience`: babies drop none, except monsters and
/// hoglins; tadpoles never do.
pub fn drops_experience(m: &Mob) -> bool {
    match m.species() {
        Species::Tadpole => false,
        Species::Hoglin => true,
        s if is_monster(s) => true,
        _ => !m.is_baby(),
    }
}
/// `getBaseExperienceReward` (no vanilla enchantment changes mob experience).
pub fn base_experience(m: &mut Mob) -> i32 {
    use Species as S;
    let s = m.species();
    match s {
        // Chicken.getBaseExperienceReward.
        S::Chicken if m.extra.jockey => 10,
        S::SulfurCube if m.is_baby() => 0,
        S::SulfurCube => 1 + m.random.next_int_bound(2).unwrap_or(0),
        // Hoglin / Piglin: the reward without the equipment bonus.
        S::Hoglin | S::Piglin => xp_reward(m),
        s if is_animal(s) || is_water_creature(s) => 1 + m.random.next_int_bound(3).unwrap_or(0),
        _ => {
            let mut reward = xp_reward(m);
            // Zombie.getBaseExperienceReward: babies are worth 2.5 times as much.
            if zombie_family(s) && m.is_baby() {
                reward = (reward as f32 * 2.5) as i32;
            }
            if reward > 0 {
                let mut result = reward;
                for slot in SLOTS {
                    // EquipmentSlot.canIncreaseExperience: all but the saddle.
                    if slot != "saddle"
                        && slot_item(m, slot).is_some()
                        && drop_chance(m, slot) <= 1.0
                    {
                        result += 1 + m.random.next_int_bound(3).unwrap_or(0);
                    }
                }
                result
            } else {
                reward
            }
        }
    }
}
