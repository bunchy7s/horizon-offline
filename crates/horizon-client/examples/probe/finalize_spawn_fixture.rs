//! `Mob.finalizeSpawn` for differential tests against vanilla
//! (tools/test_finalize_spawn_parity.py): reads the cases of
//! `VanillaFinalizeSpawnOracle`, runs `mobs::finalize` with the same level
//! random and entity randoms, and prints every entity of the case in the
//! oracle's format.
//!
//! usage: finalize_spawn_fixture RESOURCES INPUT OUTPUT HALLOWEEN
use horizon_client::{
    assets::Assets,
    blocks::BlockFacts,
    enchant::Enchantments,
    items::Items,
    mobs::{
        Mob, MobTypes, Species,
        data::MobData,
        finalize::{self, Attr, Ctx, FinalizeLevel, Reason, SpawnData, SpawnRegistries},
        gear,
        render::{DYE_NAMES, PROFESSIONS, VILLAGER_TYPES},
        spawning::LocalDifficulty,
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::loot::PackOverlay;
use std::{collections::HashMap, fmt::Write as _, fs, path::Path, sync::Arc};

struct Level<'a> {
    types: &'a MobTypes,
    data: &'a MobData,
    biome: String,
    structure: Option<(String, [i32; 6])>,
    phase: usize,
    monsters: bool,
    peaceful: bool,
    time: i64,
    halloween: bool,
    roof: Option<i32>,
    eseed: i64,
    /// Every entity of the case in creation order (case chickens first).
    created: usize,
    /// The level's entities that can carry a zombie: (id, bounds, ridden).
    chickens: Vec<(u32, Aabb, bool)>,
    rides: Vec<(u32, u32)>,
}
impl FinalizeLevel for Level<'_> {
    fn biome(&self, _pos: [i32; 3]) -> String {
        self.biome.clone()
    }
    fn biome_tag(&self, tag: &str, _pos: [i32; 3]) -> bool {
        self.data.biome_has(tag, &self.biome)
    }
    fn structures_with_piece_at(&self, pos: [i32; 3]) -> Vec<String> {
        match &self.structure {
            Some((id, b))
                if pos[0] >= b[0]
                    && pos[0] <= b[3]
                    && pos[1] >= b[1]
                    && pos[1] <= b[4]
                    && pos[2] >= b[2]
                    && pos[2] <= b[5] =>
            {
                vec![id.clone()]
            }
            _ => Vec::new(),
        }
    }
    fn moon_phase(&self, _pos: [i32; 3]) -> usize {
        self.phase
    }
    fn spawning_monsters(&self) -> bool {
        self.monsters
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        // A floor under the spawn point and an optional roof layer.
        self.roof.is_none_or(|y| {
            let (lo, hi) = (
                bounds.min(horizon_core::geometry::Axis::Y),
                bounds.max(horizon_core::geometry::Axis::Y),
            );
            !(lo < f64::from(y) + 1.0 && hi > f64::from(y))
        })
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.chickens
            .iter()
            .find(|(_, b, ridden)| !ridden && b.intersects(bounds))
            .map(|c| c.0)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        if let Some(c) = self.chickens.iter_mut().find(|c| c.0 == chicken) {
            c.2 = true;
        }
        self.rides.push((chicken, rider));
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn halloween(&self) -> bool {
        self.halloween
    }
    fn create(&mut self, kind: &str, _reason: Reason) -> Option<Mob> {
        if self.peaceful && !finalize::allowed_in_peaceful(kind) {
            return None;
        }
        let t = Arc::new(self.types.types.get(kind)?.clone());
        let index = self.created;
        self.created += 1;
        let mut m = Mob::new(index as u32 + 1, t, Vec3::ZERO, 0.0);
        m.random = LegacyRandom::new(self.eseed.wrapping_add(index as i64));
        Some(m)
    }
}

fn hex64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn hex32(v: f32) -> String {
    format!("{:x}", v.to_bits())
}
const HORSE: [&str; 7] = [
    "white",
    "creamy",
    "chestnut",
    "brown",
    "black",
    "gray",
    "dark_brown",
];
const MARKINGS: [&str; 5] = ["none", "white", "white_field", "white_dots", "black_dots"];
const RABBITS: [&str; 6] = ["brown", "white", "black", "white_splotched", "gold", "salt"];
const PARROTS: [&str; 5] = ["red_blue", "blue", "green", "yellow_blue", "gray"];
const AXOLOTLS: [&str; 5] = ["lucy", "wild", "gold", "cyan", "blue"];
const SALMON: [&str; 3] = ["small", "medium", "large"];
const GENES: [&str; 7] = [
    "normal",
    "lazy",
    "worried",
    "playful",
    "brown",
    "weak",
    "aggressive",
];
const MEMORIES: [&str; 7] = [
    "long_jump_cooling_down",
    "ram_cooldown_ticks",
    "attack_target_cooldown",
    "hunted_recently",
    "dig_cooldown",
    "is_emerging",
    "home",
];

fn variant(m: &Mob, regs: &SpawnRegistries, v: &mut Vec<String>) {
    use Species as S;
    let s = m.species();
    let named = |reg: &finalize::VariantRegistry, i: Option<u8>| {
        let name = i.map_or("classic", |i| {
            reg.names.get(i as usize).map_or("?", |n| n.as_str())
        });
        format!("minecraft:{name}")
    };
    let var = format!("minecraft:{}", finalize::variant_name(s, m.extra.variant));
    match s {
        S::Pig => {
            v.push(var);
            v.push(named(&regs.pig_sound, m.extra.sound_variant));
        }
        S::Cow => {
            v.push(var);
            v.push(named(&regs.cow_sound, m.extra.sound_variant));
        }
        S::Mooshroom => v.push(if m.extra.variant == 0 { "red" } else { "brown" }.into()),
        S::Chicken => {
            v.push(var);
            v.push(named(&regs.chicken_sound, m.extra.sound_variant));
            v.push(format!("jockey={}", m.extra.jockey));
        }
        S::Cat => {
            v.push(var);
            v.push(named(&regs.cat_sound, m.extra.sound_variant));
        }
        S::Wolf => {
            v.push(var);
            v.push(named(&regs.wolf_sound, m.extra.sound_variant));
        }
        S::Frog | S::ZombieNautilus => v.push(var),
        S::Sheep => v.push(DYE_NAMES[m.extra.color as usize & 15].into()),
        S::Horse => v.push(format!(
            "{}/{}",
            HORSE[m.extra.variant as usize % 7],
            MARKINGS[m.extra.variant2 as usize % 5]
        )),
        S::Llama | S::TraderLlama => v.push(format!(
            "{}/{}",
            horizon_client::mobs::llama::VARIANTS[m.extra.variant as usize % 4],
            m.extra.variant2
        )),
        S::SkeletonHorse => v.push(format!("trap={}", m.extra.trap)),
        S::Fox => v.push(if m.extra.variant == 1 { "snow" } else { "red" }.into()),
        S::Rabbit => v.push(RABBITS[m.extra.variant as usize % 6].into()),
        S::Parrot => v.push(PARROTS[m.extra.variant as usize % 5].into()),
        S::Axolotl => v.push(AXOLOTLS[m.extra.variant as usize % 5].into()),
        S::TropicalFish => {
            let p = u32::from(m.extra.variant);
            let pattern = if p < 6 { p << 8 } else { 1 | (p - 6) << 8 };
            let packed = (pattern & 0xFFFF)
                | (u32::from(m.extra.variant2) & 0xFF) << 16
                | (u32::from(m.extra.variant3) & 0xFF) << 24;
            v.push((packed as i32).to_string());
        }
        S::Salmon => v.push(SALMON[m.extra.variant as usize % 3].into()),
        S::Panda => v.push(format!(
            "{}/{}",
            GENES[m.extra.variant as usize % 7],
            GENES[m.extra.variant2 as usize % 7]
        )),
        S::Goat => v.push(format!(
            "scream={} horns={},{}",
            m.extra.screaming, !m.extra.lost_horns[0], !m.extra.lost_horns[1]
        )),
        S::Slime | S::MagmaCube | S::SulfurCube => v.push(format!("size={}", m.extra.size.max(1))),
        S::Villager => v.push(format!(
            "minecraft:{}/minecraft:{}/{}",
            VILLAGER_TYPES[m.extra.variant as usize % 7],
            PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()],
            m.extra.variant3.max(1)
        )),
        S::ZombieVillager => v.push(format!(
            "minecraft:{}/{}",
            VILLAGER_TYPES[m.extra.variant as usize % 7],
            m.extra.variant3.max(1)
        )),
        S::Phantom => v.push(format!("size={}", m.extra.size)),
        S::Turtle => v.push(format!(
            "home={}",
            m.extra
                .home
                .map_or("-".into(), |h| format!("{},{},{}", h[0], h[1], h[2]))
        )),
        S::Dolphin => v.push(format!("air={}", m.air)),
        _ => {}
    }
    if matches!(
        s,
        S::Zombie | S::Husk | S::Drowned | S::ZombieVillager | S::ZombifiedPiglin
    ) {
        v.push(format!("doors={}", m.extra.can_break_doors));
    }
    if horizon_client::mobs::raider::is_raider(s) {
        let st = horizon_client::mobs::raider::state(m);
        v.push(format!(
            "leader={} patrolling={}",
            st.is_some_and(|s| s.patrol_leader),
            st.is_some_and(|s| s.patrolling)
        ));
        v.push(format!("raid={}", st.is_some_and(|s| s.can_join_raid)));
    }
    if s == S::Strider {
        v.push(format!("saddled={}", m.extra.saddled));
    }
}

fn describe(m: &Mob, all: &[Mob], regs: &SpawnRegistries) -> String {
    let mut t = vec![m.kind.id.clone()];
    let vehicle = m
        .extra
        .mount
        .and_then(|v| all.iter().position(|o| o.id == v))
        .map_or("-".to_owned(), |i| i.to_string());
    t.push(format!("veh={vehicle}"));
    if finalize::ageable(m.species()) {
        t.push(format!("age={}", m.extra.age));
    } else {
        t.push(format!("baby={}", m.is_baby()));
    }
    t.push(format!("hp={}", hex32(m.health)));
    t.push(format!("lh={}", m.extra.left_handed));
    t.push(format!("loot={}", m.can_pick_up_loot()));
    t.push(format!("persist={}", m.persistent));
    let mut eq = Vec::new();
    for (slot, name) in finalize::SLOT_NAMES.iter().enumerate() {
        let Some(id) = finalize::item(m, slot) else {
            continue;
        };
        let mut text = id.to_string();
        let g = match slot {
            0 => Some(gear::MAIN),
            1 => Some(gear::OFF),
            2 => Some(gear::FEET),
            3 => Some(gear::LEGS),
            4 => Some(gear::CHEST),
            5 => Some(gear::HEAD),
            _ => None,
        };
        if let (Some(g), Some(levels)) = (g, gear::gear(m)) {
            let mut list: Vec<String> = levels.slots[g]
                .iter()
                .map(|(e, l)| format!("{e}={l}"))
                .collect();
            list.sort();
            if !list.is_empty() {
                let _ = write!(text, "[{}]", list.join(","));
            }
        }
        if slot == 5 && horizon_client::mobs::raider::is_captain(m) {
            text.push_str("*4");
        }
        eq.push(format!("{name}:{text}"));
    }
    t.push(format!(
        "eq={}",
        if eq.is_empty() {
            "-".into()
        } else {
            eq.join(";")
        }
    ));
    let drops: Vec<String> = finalize::SLOT_NAMES
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let d = finalize::stored_drop_chance(m, i);
            (d != 0.085).then(|| format!("{name}:{}", hex32(d)))
        })
        .collect();
    t.push(format!(
        "drop={}",
        if drops.is_empty() {
            "-".into()
        } else {
            drops.join(";")
        }
    ));
    let attrs: Vec<String> = [
        Attr::MaxHealth,
        Attr::MovementSpeed,
        Attr::FollowRange,
        Attr::AttackDamage,
        Attr::KnockbackResistance,
        Attr::SpawnReinforcements,
        Attr::JumpStrength,
    ]
    .into_iter()
    .filter(|a| finalize::default_value(m, *a).is_some())
    .map(|a| format!("{}:{}", a.name(), hex64(finalize::value(m, a))))
    .collect();
    t.push(format!("a={}", attrs.join(",")));
    let mut fx: Vec<String> = m
        .extra
        .effects
        .iter()
        .map(|(e, d, a)| format!("{e}:{a}:{d}"))
        .collect();
    fx.sort();
    t.push(format!(
        "fx={}",
        if fx.is_empty() {
            "-".into()
        } else {
            fx.join(",")
        }
    ));
    let mem: Vec<String> = MEMORIES
        .iter()
        .filter_map(|name| {
            let (_, value, ttl) = m.extra.memories.iter().find(|x| x.0 == *name)?;
            let text = match *name {
                "hunted_recently" => "true".to_owned(),
                "dig_cooldown" | "is_emerging" => "INSTANCE".to_owned(),
                "home" => m
                    .extra
                    .home
                    .map_or("-".into(), |h| format!("{},{},{}", h[0], h[1], h[2])),
                _ => value.to_string(),
            };
            Some(format!("{name}:{text}:{ttl}"))
        })
        .collect();
    if !mem.is_empty() {
        t.push(format!("mem={}", mem.join(",")));
    }
    let mut v = Vec::new();
    variant(m, regs, &mut v);
    if !v.is_empty() {
        t.push(format!("v={}", v.join(" ").replace(' ', "_")));
    }
    t.join(" ")
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: finalize_spawn_fixture RESOURCES INPUT OUTPUT HALLOWEEN")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let halloween = args.get(4).is_some_and(|h| h == "true");
    let assets = Assets::load(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let mob_data = MobData::load(root, &facts);
    let items = Items::tags_only(root, &PackOverlay::NONE)?;
    let enchantments = Enchantments::load(root);
    let types = MobTypes::load(root)?;
    let registries = SpawnRegistries::load(root);
    let camel = &types.types["minecraft:camel_husk"];
    let data = SpawnData {
        registries: &registries,
        enchantments: &enchantments,
        items: &items,
        camel_husk_size: (camel.width as f32, camel.height as f32),
    };
    let mut out = format!("halloween {halloween}\n");
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let mut c: HashMap<&str, &str> = HashMap::new();
        let mut name = "";
        for token in line.split_whitespace() {
            match token.split_once('=') {
                Some((k, v)) => {
                    c.insert(k, v);
                }
                None => name = token,
            }
        }
        let get =
            |k: &str, d: &'static str| -> String { c.get(k).copied().unwrap_or(d).to_owned() };
        let p: Vec<f64> = get("pos", "0,0,0")
            .split(',')
            .map(|x| x.parse().unwrap_or(0.0))
            .collect();
        let pos = Vec3::new(p[0], p[1], p[2]);
        let block = [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ];
        let base = match get("difficulty", "normal").as_str() {
            "peaceful" => 0,
            "easy" => 1,
            "hard" => 3,
            _ => 2,
        };
        let phase: usize = get("phase", "0").parse().unwrap_or(0);
        let time: i64 = get("time", "0").parse().unwrap_or(0);
        let inhabited: i64 = get("inhabited", "0").parse().unwrap_or(0);
        let difficulty = LocalDifficulty::new(
            base,
            time,
            inhabited,
            horizon_client::mobs::spawning::MOON_BRIGHTNESS[phase & 7],
        );
        let mut level = Level {
            types: &types,
            data: &mob_data,
            biome: get("biome", "minecraft:plains"),
            structure: c.get("structure").map(|s| {
                (
                    (*s).to_owned(),
                    [
                        block[0] - 8,
                        block[1] - 8,
                        block[2] - 8,
                        block[0] + 8,
                        block[1] + 8,
                        block[2] + 8,
                    ],
                )
            }),
            phase,
            monsters: get("monsters", "1") != "0",
            peaceful: base == 0,
            time,
            halloween,
            roof: c
                .get("roof")
                .and_then(|r| r.parse::<i32>().ok())
                .map(|r| block[1] + r),
            eseed: get("eseed", "0").parse().unwrap_or(0),
            created: 0,
            chickens: Vec::new(),
            rides: Vec::new(),
        };
        let mut all: Vec<Mob> = Vec::new();
        let chickens: usize = get("chickens", "0").parse().unwrap_or(0);
        for i in 0..chickens {
            if let Some(mut chicken) = level.create("minecraft:chicken", Reason::Load) {
                chicken.position = Vec3::new(pos.x + i as f64, pos.y, pos.z);
                level.chickens.push((chicken.id, chicken.bounds(), false));
                all.push(chicken);
            }
        }
        let kind = get("type", "minecraft:pig");
        let reason = Reason::parse(&get("reason", "NATURAL")).unwrap_or(Reason::Natural);
        let pack: usize = get("pack", "1").parse().unwrap_or(1);
        let mut random = LegacyRandom::new(get("seed", "0").parse().unwrap_or(0));
        let mut group = None;
        for _ in 0..pack {
            let Some(mut mob) = level.create(&kind, reason) else {
                break;
            };
            mob.position = pos;
            mob.previous = pos;
            let mut cx = Ctx {
                level: &mut level,
                random: &mut random,
                difficulty,
                data: &data,
                created: Vec::new(),
                jockey_rider: None,
            };
            group = finalize::finalize(&mut cx, &mut mob, reason, group);
            let created = std::mem::take(&mut cx.created);
            all.push(mob);
            all.extend(created);
        }
        for (chicken, rider) in std::mem::take(&mut level.rides) {
            if let Some(c) = all.iter_mut().find(|m| m.id == chicken) {
                c.extra.jockey = true;
                c.extra.rider = Some(rider);
            }
        }
        all.sort_by_key(|m| m.id);
        let mut parts: Vec<String> = all.iter().map(|m| describe(m, &all, &registries)).collect();
        for m in &mut all {
            parts.push(format!("rng {}", m.random.next_long()));
        }
        parts.push(format!("next {}", random.next_long()));
        let _ = writeln!(out, "{name} {}", parts.join(" | "));
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
