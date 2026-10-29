//! Entity death drops for differential tests against vanilla
//! (tools/test_entity_death_loot_parity.py): reads the cases of
//! `VanillaEntityDeathLootOracle` and prints what `mobs::drops::die` drops.
//!
//! usage: entity_death_loot_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    mobs::{Mob, MobTypes, Species, cube, drops, gear, raider, render::DYE_NAMES, sulfur_cube},
    random_sequences::RandomSequences,
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use horizon_world::loot::{LootDamage, LootData, LootEntity, LootStack};
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path, sync::Arc};

/// `id[;enchantment:level...]`.
fn stack(spec: &str) -> LootStack {
    let mut parts = spec.split(';');
    let mut s = LootStack {
        item: parts.next().unwrap_or("minecraft:air").to_owned(),
        count: 1,
        damage: 0,
        enchantments: Vec::new(),
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: Vec::new(),
        names: Vec::new(),
        components: Vec::new(),
    };
    for e in parts {
        let (name, level) = e.rsplit_once(':').unwrap_or((e, "1"));
        let name = if name.contains(':') {
            name.to_owned()
        } else {
            format!("minecraft:{name}")
        };
        s.enchantments.push((name, level.parse().unwrap_or(1)));
    }
    s
}

fn line(s: &LootStack) -> String {
    let mut out = format!("drop {} {}", s.item, s.count);
    if s.damage != 0 {
        let _ = write!(out, " damage={}", s.damage);
    }
    for (e, l) in &s.enchantments {
        let _ = write!(out, " ench={e}:{l}");
    }
    if let Some(p) = &s.potion {
        let _ = write!(out, " potion={p}");
    }
    if let Some(a) = s.ominous_amplifier {
        let _ = write!(out, " ominous={a}");
    }
    out
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: entity_death_loot_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let types = MobTypes::load(root)?;
    let loot = LootData::load(root)?;
    let mut out = String::new();
    for raw in input.lines() {
        if raw.trim().is_empty() || raw.starts_with('#') {
            continue;
        }
        let mut c = BTreeMap::new();
        let mut name = "";
        for token in raw.split_whitespace() {
            match token.split_once('=') {
                Some((k, v)) => {
                    c.insert(k, v);
                }
                None => name = token,
            }
        }
        let get =
            |k: &str, d: &'static str| -> String { c.get(k).copied().unwrap_or(d).to_owned() };
        let _ = writeln!(out, "== {name}");
        let Some(kind) = types.types.get(c["type"]) else {
            let _ = writeln!(out, "error unsupported {}", c["type"]);
            continue;
        };
        let kind = Arc::new(kind.clone());
        let world_seed: i64 = get("seed", "0").parse()?;
        let kills: i64 = get("kills", "1").parse()?;
        let rand: i64 = get("rand", "0").parse()?;
        let mut sequences = RandomSequences::default();
        for k in 0..kills {
            let _ = writeln!(out, "kill {k}");
            let mut m = Mob::new(1, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
            // EntityType.create(LOAD) gives no equipment.
            for slot in drops::SLOTS {
                drops::clear_slot(&mut m, slot);
            }
            if get("baby", "0") == "1" && drops::can_be_baby(m.species()) {
                m.extra.age = -24000;
            }
            if get("fire", "0") == "1" {
                m.fire_ticks = 100;
            }
            // A cube made with EntityType.create(LOAD) was never sized.
            if matches!(m.species(), Species::Slime | Species::MagmaCube) {
                m.extra.size = 0;
            }
            if let Some(size) = c.get("size") {
                let size: i32 = size.parse()?;
                if m.species() == Species::SulfurCube {
                    sulfur_cube::set_size(&mut m, size, true);
                } else {
                    cube::set_size(&mut m, size);
                }
            }
            if let Some(color) = c.get("color") {
                m.extra.color = DYE_NAMES.iter().position(|d| d == color).unwrap_or(0) as u8;
            }
            m.extra.sheared = get("sheared", "0") == "1";
            if get("leader", "0") == "1" {
                raider::state_mut(&mut m).patrol_leader = true;
            }
            if get("captain", "0") == "1" {
                m.extra.head = Some(Arc::from(raider::BANNER));
            }
            let mut chances = [drops::DEFAULT_DROP_CHANCE; 8];
            for (i, slot) in drops::SLOTS.iter().enumerate() {
                if let Some(spec) = c.get(slot) {
                    let s = stack(spec);
                    let item: Option<Arc<str>> = Some(Arc::from(s.item.as_str()));
                    let index = match *slot {
                        "mainhand" => {
                            m.extra.main_hand = item;
                            Some(gear::MAIN)
                        }
                        "offhand" => {
                            m.extra.off_hand = item;
                            Some(gear::OFF)
                        }
                        "head" => {
                            m.extra.head = item;
                            Some(gear::HEAD)
                        }
                        "body" if drops::head_is_body(m.species()) => {
                            m.extra.head = item;
                            None
                        }
                        "saddle" => {
                            m.set_saddle(Some(horizon_client::items::ItemStack::new(&s.item, 1)));
                            None
                        }
                        "chest" => {
                            m.extra.chest = item;
                            Some(gear::CHEST)
                        }
                        "legs" => {
                            m.extra.legs = item;
                            Some(gear::LEGS)
                        }
                        "feet" => {
                            m.extra.feet = item;
                            Some(gear::FEET)
                        }
                        _ => None,
                    };
                    if let Some(index) = index
                        && !s.enchantments.is_empty()
                    {
                        let levels = s
                            .enchantments
                            .iter()
                            .map(|(e, l)| (Arc::from(e.as_str()), *l as u32))
                            .collect();
                        gear::set(&mut m, index, levels);
                    }
                }
                if let Some(chance) = c.get(format!("chance_{slot}").as_str()) {
                    chances[i] = chance.parse()?;
                }
            }
            m.extra.drop_chances = Some(Box::new(chances));
            m.random = LegacyRandom::new(rand + k);
            // The killer, the last player to hurt the victim and the damage source.
            let killer_id = get("killer", "none");
            let weapon = c.get("weapon").map(|w| stack(w));
            let armed = |kind: &str| {
                let mut e = LootEntity::new(kind);
                if let Some(w) = &weapon {
                    e.equipment.push(("mainhand".into(), w.clone()));
                }
                e
            };
            let player = (killer_id == "player" || get("pkill", "0") == "1").then(|| {
                if killer_id == "player" {
                    armed("minecraft:player")
                } else {
                    LootEntity::new("minecraft:player")
                }
            });
            let killer = match killer_id.as_str() {
                "player" => player.clone(),
                "none" => None,
                id => {
                    let mut e = armed(id);
                    e.frog_variant = c.get("frog").map(|f| f.to_string());
                    e.powered = get("charged", "0") == "1";
                    Some(e)
                }
            };
            let pkill = get("pkill", if killer_id == "player" { "1" } else { "0" }) == "1";
            let projectile = |kind: &str| LootEntity {
                kind: kind.to_owned(),
                ..LootEntity::default()
            };
            let damage =
                |kind: &str, source: Option<LootEntity>, direct: Option<LootEntity>| LootDamage {
                    kind: kind.to_owned(),
                    source,
                    direct,
                };
            let source = match get("source", "attack").as_str() {
                "attack" => match &killer {
                    None => damage("minecraft:generic", None, None),
                    Some(k) if k.kind == "minecraft:player" => {
                        damage("minecraft:player_attack", Some(k.clone()), Some(k.clone()))
                    }
                    Some(k) => damage("minecraft:mob_attack", Some(k.clone()), Some(k.clone())),
                },
                "arrow" => damage(
                    "minecraft:arrow",
                    killer.clone(),
                    Some(projectile("minecraft:arrow")),
                ),
                "fireball" => match &killer {
                    Some(k) => damage(
                        "minecraft:fireball",
                        Some(k.clone()),
                        Some(projectile("minecraft:fireball")),
                    ),
                    None => damage(
                        "minecraft:unattributed_fireball",
                        Some(projectile("minecraft:fireball")),
                        Some(projectile("minecraft:fireball")),
                    ),
                },
                "explosion" => match &killer {
                    Some(k) => damage(
                        "minecraft:player_explosion",
                        Some(k.clone()),
                        Some(k.clone()),
                    ),
                    None => damage("minecraft:explosion", None, None),
                },
                "lightning" => damage("minecraft:lightning_bolt", None, None),
                "fire" => damage("minecraft:on_fire", None, None),
                _ => damage("minecraft:generic", None, None),
            };
            let death = drops::Death {
                loot: &loot,
                damage: Some(source),
                player: player.filter(|_| pkill),
                luck: get("luck", "0").parse()?,
                vehicle: c.get("vehicle").map(|v| LootEntity::new(v)),
                mob_drops: true,
                world_seed,
            };
            let (items, _) = drops::die(&mut m, &death, &mut sequences);
            for d in items {
                let _ = match d {
                    drops::Drop::Item(s) => writeln!(out, "{}", line(&s)),
                    drops::Drop::Orb(v) => writeln!(out, "orb {v}"),
                };
            }
        }
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
