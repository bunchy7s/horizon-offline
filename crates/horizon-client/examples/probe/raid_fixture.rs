//! Pillager patrols and raids for differential tests against vanilla
//! (tools/test_raid_parity.py): reads the cases of `VanillaRaidOracle` and
//! prints what `horizon_client::patrol` and `horizon_client::raid` do, in the
//! oracle's order and format.
//!
//! usage: raid_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    assets::Assets,
    blocks::BlockFacts,
    items::Items,
    mobs::{
        Species,
        data::MobData,
        spawn_rules::{self, Dimension, SpawnLevel},
        spawn_selection::{self, Heightmap},
        spawning::LocalDifficulty,
    },
    patrol::{self, GameplayAttributes, Member, PatrolLevel, PatrolSpawner},
    raid::{self, Action, BarName, RaidLevel, RaidPlayer, RaiderInfo, Raids},
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::loot::PackOverlay;
use horizon_world::{chunk::BlockState, nbt::Tag, registry::BlockStateId};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::Path,
};

#[derive(Clone, Copy)]
struct Bx([i32; 6]);
impl Bx {
    fn parse(s: &str) -> Self {
        let v: Vec<i32> = s.split(',').map(|n| n.parse().unwrap_or(0)).collect();
        Self([v[0], v[1], v[2], v[3], v[4], v[5]])
    }
    fn has(&self, x: i32, y: i32, z: i32) -> bool {
        let b = self.0;
        x >= b[0] && x <= b[3] && y >= b[1] && y <= b[4] && z >= b[2] && z <= b[5]
    }
}

struct Shared<'a> {
    facts: &'a BlockFacts,
    data: &'a MobData,
    block_tags: &'a HashMap<String, HashSet<String>>,
    attributes: &'a GameplayAttributes,
    air: BlockStateId,
}

struct Player {
    position: [f64; 3],
    mode: String,
    alive: bool,
}
impl Player {
    fn block(&self) -> [i32; 3] {
        [
            self.position[0].floor() as i32,
            self.position[1].floor() as i32,
            self.position[2].floor() as i32,
        ]
    }
}

struct World<'a> {
    s: &'a Shared<'a>,
    blocks: Vec<(Bx, BlockStateId)>,
    lights: Vec<(Bx, i32, i32)>,
    biome: String,
    sky: i32,
    light: i32,
    darken: i32,
    difficulty: i32,
    dimension: Dimension,
    dimension_id: String,
    unloaded: HashSet<(i32, i32)>,
    no_ticking: HashSet<(i32, i32)>,
    villages: Vec<[i32; 3]>,
    pois: Vec<[i32; 3]>,
    player: Option<Player>,
    clock: i64,
    inhabited: i64,
    moon: f32,
    patrols: bool,
    raids: bool,
}
impl World<'_> {
    fn lights_at(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        self.lights
            .iter()
            .rev()
            .find(|(b, ..)| b.has(x, y, z))
            .map_or((self.sky, self.light), |(_, s, b)| (*s, *b))
    }
    fn min_y(&self) -> i32 {
        if self.dimension == Dimension::Overworld {
            -64
        } else {
            0
        }
    }
    fn max_y(&self) -> i32 {
        if self.dimension == Dimension::Overworld {
            319
        } else {
            255
        }
    }
    fn loaded(&self, cx: i32, cz: i32) -> bool {
        !self.unloaded.contains(&(cx, cz))
    }
    fn height(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        if !self.loaded(x >> 4, z >> 4) {
            return self.min_y();
        }
        (self.min_y()..=self.max_y())
            .rev()
            .find(|&y| spawn_selection::heightmap_blocks(self, kind, x, y, z))
            .map_or(self.min_y(), |y| y + 1)
    }
    fn attribute(&self, key: &str) -> bool {
        self.s
            .attributes
            .value(key, &self.dimension_id, &self.biome, self.clock)
    }
}
impl SpawnLevel for World<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.blocks
            .iter()
            .rev()
            .find(|(b, _)| b.has(x, y, z))
            .map_or(self.s.air, |(_, s)| *s)
    }
    fn facts(&self) -> &BlockFacts {
        self.s.facts
    }
    fn mob_data(&self) -> &MobData {
        self.s.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.s
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }
    fn biome_tag(&self, tag: &str, _: i32, _: i32, _: i32) -> bool {
        self.s
            .data
            .biome_tags
            .get(tag)
            .is_some_and(|s| s.contains(&self.biome))
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.lights_at(x, y, z).0
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.lights_at(x, y, z).1
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn thundering(&self) -> bool {
        false
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.height(Heightmap::WorldSurface, x, z)
    }
    fn peaceful(&self) -> bool {
        self.difficulty == 0
    }
    fn dimension(&self) -> Dimension {
        self.dimension
    }
    fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
        0.0
    }
    fn world_seed(&self) -> i64 {
        0
    }
    fn player_near(&self, _: f64, _: f64, _: f64, _: f64) -> bool {
        false
    }
}
impl PatrolLevel for World<'_> {
    fn spawn_patrols(&self) -> bool {
        self.patrols
    }
    fn bright_outside(&self) -> bool {
        self.dimension == Dimension::Overworld && self.darken < 4
    }
    fn patrol_players(&self) -> Vec<([i32; 3], bool)> {
        self.player
            .iter()
            .map(|p| (p.block(), p.mode == "spectator"))
            .collect()
    }
    fn sections_to_village(&self, pos: [i32; 3]) -> i32 {
        let s = [pos[0] >> 4, pos[1] >> 4, pos[2] >> 4];
        self.villages
            .iter()
            .map(|v| {
                (v[0] - s[0])
                    .abs()
                    .max((v[1] - s[1]).abs())
                    .max((v[2] - s[2]).abs())
            })
            .fold(7, i32::min)
    }
    fn has_chunk(&self, cx: i32, cz: i32) -> bool {
        self.loaded(cx, cz)
    }
    fn can_pillager_patrol_spawn(&self, _: [i32; 3]) -> bool {
        self.attribute(patrol::CAN_PILLAGER_PATROL_SPAWN)
    }
    fn effective_difficulty(&self, pos: [i32; 3]) -> f32 {
        let loaded = self.loaded(pos[0] >> 4, pos[2] >> 4);
        LocalDifficulty::new(
            self.difficulty as u8,
            self.clock,
            if loaded { self.inhabited } else { 0 },
            if loaded { self.moon } else { 0.0 },
        )
        .effective
    }
    fn motion_blocking_no_leaves(&self, x: i32, z: i32) -> i32 {
        self.height(Heightmap::MotionBlockingNoLeaves, x, z)
    }
}
impl RaidLevel for World<'_> {
    fn difficulty(&self) -> i32 {
        self.difficulty
    }
    fn has_chunk_at(&self, pos: [i32; 3]) -> bool {
        self.loaded(pos[0] >> 4, pos[2] >> 4)
    }
    fn has_chunks_at(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        patrol::has_chunks_at(self, x0, z0, x1, z1)
    }
    fn sections_to_village(&self, pos: [i32; 3]) -> i32 {
        PatrolLevel::sections_to_village(self, pos)
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        self.height(Heightmap::WorldSurface, x, z)
    }
    fn is_position_entity_ticking(&self, pos: [i32; 3]) -> bool {
        let c = (pos[0] >> 4, pos[2] >> 4);
        self.loaded(c.0, c.1) && !self.no_ticking.contains(&c)
    }
    fn ravager_can_spawn(&self, [x, y, z]: [i32; 3]) -> bool {
        spawn_rules::is_spawn_position_ok(Species::Ravager, self, x, y, z)
            || self.s.facts.get(self.state(x, y - 1, z)).name.as_ref() == "minecraft:snow"
                && self.s.facts.is_air(self.state(x, y, z))
    }
    fn player(&self) -> Option<RaidPlayer> {
        self.player.as_ref().map(|p| RaidPlayer {
            position: p.position,
            block: p.block(),
            alive: p.alive,
        })
    }
}

fn parse_state(spec: &str) -> BlockState {
    let (name, props) = match spec.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    BlockState {
        name: name.to_owned(),
        properties: props
            .split(',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}
fn bits64(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn bits32(v: f32) -> String {
    format!("{:x}", v.to_bits())
}
fn ints(s: &str) -> Vec<i32> {
    s.split(',').map(|n| n.parse().unwrap_or(0)).collect()
}
fn difficulty_id(name: &str) -> i32 {
    match name {
        "peaceful" => 0,
        "easy" => 1,
        "hard" => 3,
        _ => 2,
    }
}

/// A raider of the scenario, by creation order.
struct Raider {
    kind: String,
    health: f32,
    position: [i32; 3],
    tick_count: i32,
    no_action_time: i32,
    ticks_outside: i32,
    removed: bool,
    in_level: bool,
    patrol_leader: bool,
    can_join: bool,
    wave: i32,
    raid: Option<i32>,
}
fn max_health(kind: &str) -> f32 {
    match kind {
        "minecraft:witch" => 26.0,
        "minecraft:ravager" => 100.0,
        _ => 24.0,
    }
}
fn kind_of(t: raid::RaiderType) -> &'static str {
    t.entity()
}

const PLAYER_UUID: [i32; 4] = [0, 0x1234, 0, 0x5678];

/// `ClientboundSoundPacket`: positions travel as eighths of a block.
fn packet_coord(v: f64) -> f64 {
    f64::from(((v * 8.0) as i32) as f32 / 8.0)
}

fn raid_state(raids: &Raids, player_in: &dyn Fn(i32) -> bool) -> String {
    let mut s = String::new();
    for id in raids.ids() {
        let r = &raids.map[&id];
        let name = match r.bar.name {
            BarName::Raid => "Raid".to_owned(),
            BarName::Remaining(n) => format!("Raid - Raiders Remaining: {n}"),
            BarName::Victory => "Raid - Victory".to_owned(),
            BarName::Defeat => "Raid - Defeat".to_owned(),
        };
        let leaders: Vec<String> = r.leaders.keys().map(|k| k.to_string()).collect();
        let _ = write!(
            s,
            " ; r{id} {} act={} st={} om={} gs={} th={} alive={} c={},{},{} wp={} cel={} lead=[{}] bar={},{},{},{}",
            r.status.name(),
            i32::from(r.active),
            i32::from(r.started),
            r.omen_level,
            r.groups_spawned,
            bits32(r.total_health),
            ALIVE.with(|a| a.borrow().get(&id).copied().unwrap_or(0)),
            r.center[0],
            r.center[1],
            r.center[2],
            r.wave_spawn_pos
                .map_or("-".to_owned(), |p| format!("{},{},{}", p[0], p[1], p[2])),
            r.celebration_ticks,
            leaders.join(", "),
            bits32(r.bar.progress),
            name,
            i32::from(r.bar.visible),
            i32::from(r.bar.player && player_in(id)),
        );
    }
    for id in raids.ids() {
        let r = &raids.map[&id];
        let _ = write!(
            s,
            " ; t{id} {} {} {}",
            r.ticks_active, r.cooldown_ticks, r.post_raid_ticks
        );
    }
    s
}
thread_local! {
    static ALIVE: RefCell<HashMap<i32, usize>> = RefCell::new(HashMap::new());
}

/// NBT as the oracle's canonical text.
fn canon(tag: &Tag) -> String {
    match tag {
        Tag::Byte(v) => format!("{v}b"),
        Tag::Short(v) => format!("{v}s"),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => format!("{v}L"),
        Tag::Float(b) => format!("f{b:x}"),
        Tag::Double(b) => format!("d{b:x}"),
        Tag::String(s) => format!("\"{}\"", s.to_string_lossy()),
        Tag::ByteArray(v) => {
            let mut s = "[B;".to_owned();
            for b in v {
                let _ = write!(s, "{},", *b as i8);
            }
            s + "]"
        }
        Tag::IntArray(v) => {
            let mut s = "[I;".to_owned();
            for b in v {
                let _ = write!(s, "{b},");
            }
            s + "]"
        }
        Tag::LongArray(v) => {
            let mut s = "[L;".to_owned();
            for b in v {
                let _ = write!(s, "{b},");
            }
            s + "]"
        }
        Tag::List(items) => {
            let mut s = "[".to_owned();
            for t in items {
                s += &canon(t);
                s.push(',');
            }
            s + "]"
        }
        Tag::Compound(map) => {
            let mut keys: Vec<String> = map.keys().map(|k| k.to_string_lossy()).collect();
            keys.sort();
            let mut s = "{".to_owned();
            for k in keys {
                let _ = write!(s, "{k}:{},", canon(tag.get(&k).unwrap_or(&Tag::End)));
            }
            s + "}"
        }
        Tag::End => "?".to_owned(),
    }
}

fn run_codec(c: &BTreeMap<&str, &str>) -> Result<String, String> {
    let hex = c.get("snbt").copied().unwrap_or("");
    let text: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8(text).map_err(|e| e.to_string())?;
    let (tag, _) = horizon_world::nbt::parse_snbt(&text)?;
    let raids = Raids::from_nbt(&tag, 0);
    let fresh = raids.dirty && raids.map.is_empty() && raids.next_id == 1 && raids.tick == 0 && {
        // A fresh Raids is dirty; a decoded one never is.
        true
    };
    let mut log = vec![if fresh { "fresh" } else { "decoded" }.to_owned()];
    log.push(canon(&raids.to_nbt()));
    for id in raids.ids() {
        let r = &raids.map[&id];
        log.push(format!(
            "bar {id} {} {} {}",
            bits32(r.bar.progress),
            i32::from(r.bar.visible),
            i32::from(raids.dirty)
        ));
    }
    Ok(log.join(" ; "))
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: raid_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let data = MobData::load(root, &facts);
    let items = Items::tags_only(root, &PackOverlay::NONE)?;
    let attributes = GameplayAttributes::load(root);
    let shared = Shared {
        facts: &facts,
        data: &data,
        block_tags: &items.block_tags,
        attributes: &attributes,
        air: facts.air,
    };
    let mut out = String::new();
    for raw in input.lines() {
        if raw.trim().is_empty() {
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
        let mode = get("mode", "raid");
        if mode == "codec" {
            let line = match run_codec(&c) {
                Ok(l) => l,
                Err(e) => format!("error {e}"),
            };
            let _ = writeln!(out, "{name} {line}");
            continue;
        }
        let indexed = |prefix: &str| -> Vec<&str> {
            (0..)
                .map_while(|i| c.get(format!("{prefix}{i}").as_str()).copied())
                .collect()
        };
        let mut blocks = Vec::new();
        for k in indexed("k") {
            let (b, s) = k.split_once('/').ok_or("bad block box")?;
            blocks.push((Bx::parse(b), registry.resolve(&parse_state(s))?));
        }
        let lights = indexed("l")
            .into_iter()
            .filter_map(|l| {
                let (b, v) = l.split_once('/')?;
                let (s, k) = v.split_once(',')?;
                Some((Bx::parse(b), s.parse().ok()?, k.parse().ok()?))
            })
            .collect();
        let chunks = |key: &str| -> HashSet<(i32, i32)> {
            c.get(key)
                .map(|u| {
                    u.split(';')
                        .filter_map(|p| {
                            let (x, z) = p.split_once(',')?;
                            Some((x.parse().ok()?, z.parse().ok()?))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let triples = |key: &str| -> Vec<[i32; 3]> {
            c.get(key)
                .map(|u| {
                    u.split(';')
                        .map(ints)
                        .filter(|v| v.len() == 3)
                        .map(|v| [v[0], v[1], v[2]])
                        .collect()
                })
                .unwrap_or_default()
        };
        let dimension_id = get("dim", "minecraft:overworld");
        let dimension = match dimension_id.as_str() {
            "minecraft:the_nether" => Dimension::Nether,
            "minecraft:the_end" => Dimension::End,
            _ => Dimension::Overworld,
        };
        let player = c.get("p0").map(|p| {
            let mut parts = p.split('/');
            let v: Vec<f64> = parts
                .next()
                .unwrap_or("")
                .split(',')
                .map(|n| n.parse().unwrap_or(0.0))
                .collect();
            Player {
                position: [v[0], v[1], v[2]],
                mode: parts.next().unwrap_or("survival").to_owned(),
                alive: true,
            }
        });
        let mut world = World {
            s: &shared,
            blocks,
            lights,
            biome: get("biome", "minecraft:plains"),
            sky: get("sky", "15").parse()?,
            light: get("light", "0").parse()?,
            darken: get("darken", "0").parse()?,
            difficulty: difficulty_id(&get("difficulty", "normal")),
            dimension,
            dimension_id: dimension_id.clone(),
            unloaded: chunks("unloaded"),
            no_ticking: chunks("noticking"),
            villages: triples("vil"),
            pois: triples("poi"),
            player,
            clock: get("clock", "200000").parse()?,
            inhabited: get("inhabited", "0").parse()?,
            moon: get("moon", "0.0").parse()?,
            patrols: get("patrols", "1") != "0",
            raids: get("raids", "1") != "0",
        };
        let ticks: i32 = get("ticks", "1").parse()?;
        let mut log: Vec<String> = Vec::new();
        if mode == "patrol" {
            let mut random = LegacyRandom::new(get("seed", "0").parse()?);
            let mut spawner = PatrolSpawner {
                next_tick: get("next", "0").parse()?,
            };
            let enemies = get("enemies", "1") != "0";
            let mut spawn = |l: &World, m: Member, _: &mut LegacyRandom| -> bool {
                // EntityType.create: no monsters in peaceful.
                if l.difficulty == 0 {
                    return false;
                }
                let mut s = format!(
                    "spawn minecraft:pillager {} {} {}",
                    m.pos[0], m.pos[1], m.pos[2]
                );
                if m.leader {
                    // The patrol target is drawn from the pillager's own random
                    // before it is placed: around the world origin.
                    let mut own = LegacyRandom::new(m.pos[0] as i64 ^ 0x5DEE);
                    let t = patrol::patrol_target([0, 0, 0], &mut own);
                    let origin =
                        t[1] == 0 && (-500..500).contains(&t[0]) && (-500..500).contains(&t[2]);
                    s += " leader banner patrolling";
                    s += if origin {
                        " target"
                    } else {
                        " target-elsewhere"
                    };
                } else {
                    s += " patrolling";
                }
                s += " wave=0 join";
                log.push(s);
                true
            };
            for _ in 0..ticks {
                spawner.tick(&world, enemies, &mut random, &mut spawn);
            }
            log.push(format!(
                "next {} {}",
                spawner.next_tick,
                random.next_int_bound(1000).unwrap_or(0)
            ));
            let _ = writeln!(out, "{name} {}", log.join(" ; "));
            continue;
        }

        // raid
        let seed: i64 = get("rseed", "1").parse()?;
        let mut raids = Raids::default();
        let mut raiders: Vec<Raider> = Vec::new();
        let mut events: BTreeMap<i32, Vec<String>> = BTreeMap::new();
        for e in indexed("ev") {
            let (t, ev) = e.split_once(':').ok_or("bad event")?;
            events.entry(t.parse()?).or_default().push(ev.to_owned());
        }
        let raid_at = |raids: &Raids, pos: [i32; 3]| raids.nearby(pos).map(|(id, _)| id);
        let living = |raiders: &[Raider], id: i32| -> f32 {
            raiders
                .iter()
                .filter(|r| r.raid == Some(id))
                .map(|r| r.health)
                .fold(0.0, |a, h| a + h)
        };
        let mut last_state = String::new();
        for t in 0..=ticks {
            for ev in events.get(&t).cloned().unwrap_or_default() {
                log.push(format!("ev {ev}"));
                let a: Vec<&str> = ev.split(':').collect();
                let idx =
                    |i: usize| -> usize { a.get(i).and_then(|v| v.parse().ok()).unwrap_or(0) };
                if ["hurt", "kill", "move", "age", "idle", "gone", "discard"].contains(&a[0])
                    && idx(1) >= raiders.len()
                {
                    log.push("noraider".into());
                    continue;
                }
                if a[0] == "killall" || a[0] == "hurtall" {
                    for i in 0..raiders.len() {
                        let Some(id) = raiders[i].raid else { continue };
                        if raiders[i].removed {
                            continue;
                        }
                        if a[0] == "hurtall" {
                            if raids.get(id).is_some_and(|r| r.active) {
                                let l = living(&raiders, id);
                                if let Some(r) = raids.get_mut(id) {
                                    r.update_bossbar(l);
                                }
                            }
                            let amount: f32 = a[1].parse()?;
                            let max = max_health(&raiders[i].kind);
                            raiders[i].health = (raiders[i].health - amount).clamp(0.0, max);
                            continue;
                        }
                        raiders[i].raid = None;
                        let after = living(&raiders, id);
                        if let Some(r) = raids.get_mut(id) {
                            if raiders[i].patrol_leader {
                                r.remove_leader(raiders[i].wave);
                            }
                            if a.len() > 1 && world.player.is_some() {
                                r.add_hero(PLAYER_UUID);
                            }
                            r.remove_from_raid(raiders[i].health, false, after);
                        }
                        raiders[i].health = 0.0;
                        raiders[i].removed = true;
                        raiders[i].in_level = false;
                    }
                    continue;
                }
                match a[0] {
                    "omen" => {
                        let v = ints(a[2]);
                        let p = world.player.as_mut().ok_or("omen needs p0")?;
                        p.position = [
                            f64::from(v[0]) + 0.5,
                            f64::from(v[1]),
                            f64::from(v[2]) + 0.5,
                        ];
                        let pos = p.block();
                        let spectator = p.mode == "spectator";
                        // Raids.createOrExtendRaid.
                        if spectator || !world.raids || !world.attribute(patrol::CAN_START_RAID) {
                            log.push("noraid".into());
                            continue;
                        }
                        let center = raid::raid_center(pos, &world.pois);
                        let known: HashSet<i32> = raids.ids().into_iter().collect();
                        let (id, triggered) = raids.create_or_extend(
                            center,
                            world.difficulty,
                            Some(a[1].parse()?),
                            0,
                        );
                        if !known.contains(&id)
                            && let Some(r) = raids.get_mut(id)
                        {
                            r.reseed(seed + i64::from(id) * 1000);
                        }
                        if triggered {
                            log.push("stat minecraft:raid_trigger".into());
                        }
                    }
                    "setomen" | "stop" => {
                        let at = world.player.as_ref().map(Player::block).unwrap_or_default();
                        if let Some(id) = raid_at(&raids, at)
                            && let Some(r) = raids.get_mut(id)
                        {
                            if a[0] == "stop" {
                                r.stop();
                            } else {
                                r.omen_level = a[1].parse()?;
                            }
                        }
                    }
                    "pmove" => {
                        let v: Vec<f64> =
                            a[1].split(',').map(|n| n.parse().unwrap_or(0.0)).collect();
                        if let Some(p) = world.player.as_mut() {
                            p.position = [v[0], v[1], v[2]];
                        }
                    }
                    "pdead" | "palive" => {
                        if let Some(p) = world.player.as_mut() {
                            p.alive = a[0] == "palive";
                        }
                    }
                    "diff" => world.difficulty = difficulty_id(a[1]),
                    "rules" => world.raids = a[1] == "1",
                    "unload" | "load" | "notick" | "tick" => {
                        let v = ints(a[1]);
                        let key = (v[0], v[1]);
                        match a[0] {
                            "unload" => {
                                world.unloaded.insert(key);
                            }
                            "load" => {
                                world.unloaded.remove(&key);
                            }
                            "notick" => {
                                world.no_ticking.insert(key);
                            }
                            _ => {
                                world.no_ticking.remove(&key);
                            }
                        }
                    }
                    "vil" => {
                        let v = ints(a[1]);
                        world.villages.push([v[0], v[1], v[2]]);
                    }
                    "unvil" => {
                        let v = ints(a[1]);
                        world.villages.retain(|x| x[..] != v[..]);
                    }
                    "hurt" => {
                        let i = idx(1);
                        if let Some(id) = raiders[i].raid
                            && let Some(r) = raids.get_mut(id)
                            && r.active
                        {
                            r.update_bossbar(living(&raiders, id));
                        }
                        let amount: f32 = a[2].parse()?;
                        // LivingEntity.setHealth clamps to [0, max].
                        let max = max_health(&raiders[i].kind);
                        raiders[i].health = (raiders[i].health - amount).clamp(0.0, max);
                    }
                    "kill" => {
                        let i = idx(1);
                        if let Some(id) = raiders[i].raid {
                            raiders[i].raid = None;
                            let after = living(&raiders, id);
                            if let Some(r) = raids.get_mut(id) {
                                if raiders[i].patrol_leader {
                                    r.remove_leader(raiders[i].wave);
                                }
                                if a.len() > 2 && world.player.is_some() {
                                    r.add_hero(PLAYER_UUID);
                                }
                                r.remove_from_raid(raiders[i].health, false, after);
                            }
                        }
                        raiders[i].health = 0.0;
                        raiders[i].removed = true;
                        raiders[i].in_level = false;
                    }
                    "move" => {
                        let v = ints(a[2]);
                        raiders[idx(1)].position = [v[0], v[1], v[2]];
                    }
                    "age" => raiders[idx(1)].tick_count = a[2].parse()?,
                    "idle" => raiders[idx(1)].no_action_time = a[2].parse()?,
                    "gone" => raiders[idx(1)].in_level = false,
                    "discard" => raiders[idx(1)].removed = true,
                    "join" | "loadmob" => {
                        let v = ints(a[2]);
                        let kind = format!("minecraft:{}", a[1]);
                        let health = max_health(&kind);
                        raiders.push(Raider {
                            kind,
                            health,
                            position: [v[0], v[1], v[2]],
                            tick_count: 0,
                            no_action_time: 0,
                            ticks_outside: 0,
                            removed: false,
                            in_level: true,
                            patrol_leader: false,
                            can_join: false,
                            wave: 0,
                            raid: None,
                        });
                        let i = raiders.len() - 1;
                        if a[0] == "join" {
                            raiders[i].can_join = true;
                            if let Some(id) = raid_at(&raids, raiders[i].position) {
                                let wave = raids.map[&id].groups_spawned;
                                raiders[i].raid = Some(id);
                                let after = living(&raiders, id);
                                let r = raids.get_mut(id).ok_or("raid")?;
                                r.add_wave_mob(health, true, after);
                                let x = &mut raiders[i];
                                x.wave = wave;
                                x.ticks_outside = 0;
                            }
                        } else {
                            let wave: i32 = a[3].parse()?;
                            raiders[i].wave = wave;
                            if a.len() > 4 {
                                raiders[i].patrol_leader = true;
                            }
                            let id: i32 = a.get(5).copied().unwrap_or("2").parse()?;
                            if raids.get(id).is_some() {
                                raiders[i].raid = Some(id);
                                let after = living(&raiders, id);
                                let r = raids.get_mut(id).ok_or("raid")?;
                                r.add_wave_mob(health, false, after);
                                if raiders[i].patrol_leader {
                                    r.set_leader(wave, i as u32);
                                }
                            }
                        }
                    }
                    other => return Err(format!("event {other}").into()),
                }
            }
            if t == 0 {
                continue;
            }
            let before: Vec<(Option<i32>, i32)> =
                raiders.iter().map(|r| (r.raid, r.ticks_outside)).collect();
            let members = |id: i32| -> Vec<RaiderInfo> {
                raiders
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.raid == Some(id))
                    .map(|(i, r)| RaiderInfo {
                        id: i as u32,
                        wave: r.wave,
                        health: r.health,
                        position: r.position,
                        tick_count: r.tick_count,
                        no_action_time: r.no_action_time,
                        ticks_outside: r.ticks_outside,
                        removed: r.removed,
                        other_dimension: false,
                        in_level: r.in_level,
                        patrol_leader: r.patrol_leader,
                    })
                    .collect()
            };
            let actions = raids.tick(&world, world.raids, &members);
            for (id, action) in actions {
                match action {
                    Action::Spawn { pos, wave, spawns } => {
                        let spawn =
                            |kind: &str, leader: bool, raiders: &mut Vec<Raider>| -> usize {
                                raiders.push(Raider {
                                    kind: kind.to_owned(),
                                    health: max_health(kind),
                                    position: [pos[0], pos[1] + 1, pos[2]],
                                    tick_count: 0,
                                    no_action_time: 0,
                                    ticks_outside: 0,
                                    removed: false,
                                    in_level: true,
                                    patrol_leader: leader,
                                    can_join: true,
                                    wave,
                                    raid: Some(id),
                                });
                                raiders.len() - 1
                            };
                        let mut lines = Vec::new();
                        for s in &spawns {
                            let i = spawn(kind_of(s.kind), s.leader, &mut raiders);
                            if s.leader
                                && let Some(r) = raids.get_mut(id)
                            {
                                r.set_leader(wave, i as u32);
                            }
                            let mut line = format!(
                                "spawn {} {} {} {}",
                                kind_of(s.kind),
                                pos[0],
                                pos[1] + 1,
                                pos[2]
                            );
                            if s.leader {
                                line += " leader banner patrolling";
                            }
                            let _ = write!(line, " wave={wave} join");
                            lines.push(line);
                            if let Some(rider) = s.rider {
                                spawn(kind_of(rider), false, &mut raiders);
                                // joinRaid adds the rider to the level before it mounts.
                                lines.push(format!(
                                    "spawn {} {} {} {} wave={wave} join",
                                    kind_of(rider),
                                    pos[0],
                                    pos[1] + 1,
                                    pos[2]
                                ));
                            }
                        }
                        log.extend(lines);
                    }
                    Action::Horn { pos, seed } => {
                        if let Some(p) = world.player.as_ref() {
                            let in_raid = raids.get(id).is_some_and(|r| r.bar.player);
                            let (rx, rz) = (f64::from(pos[0]) + 0.5, f64::from(pos[2]) + 0.5);
                            let (px, py, pz) = (p.position[0], p.position[1], p.position[2]);
                            let d = ((rx - px) * (rx - px) + (rz - pz) * (rz - pz)).sqrt();
                            let x = px + 13.0 / d * (rx - px);
                            let z = pz + 13.0 / d * (rz - pz);
                            if d <= 64.0 || in_raid {
                                log.push(format!(
                                    "horn {seed} {} {} {}",
                                    bits64(packet_coord(x)),
                                    bits64(packet_coord(py)),
                                    bits64(packet_coord(z))
                                ));
                            }
                        }
                    }
                    Action::Remove { id: mob } => {
                        raiders[mob as usize].raid = None;
                    }
                    Action::SetOutside { id: mob, ticks } => {
                        raiders[mob as usize].ticks_outside = ticks;
                    }
                    Action::Victory { heroes, amplifier } => {
                        for h in heroes {
                            if h == PLAYER_UUID
                                && let Some(p) = world.player.as_ref()
                                && p.mode != "spectator"
                            {
                                log.push(format!(
                                    "effect minecraft:hero_of_the_village {} {} false false true",
                                    raid::HERO_DURATION,
                                    // MobEffectInstance clamps the amplifier.
                                    amplifier.clamp(0, 255)
                                ));
                                log.push("stat minecraft:raid_win".into());
                            }
                        }
                    }
                }
            }
            for (i, r) in raiders.iter().enumerate() {
                if let Some((was, outside)) = before.get(i) {
                    if was.is_some() && r.raid.is_none() {
                        log.push(format!("leave {i}"));
                    }
                    if *outside != r.ticks_outside {
                        log.push(format!("out {i} {}", r.ticks_outside));
                    }
                }
            }
            ALIVE.with(|a| {
                let mut a = a.borrow_mut();
                a.clear();
                for r in &raiders {
                    if let Some(id) = r.raid {
                        *a.entry(id).or_default() += 1;
                    }
                }
            });
            let state = raid_state(&raids, &|_| true);
            let key: String = state
                .split(" ; ")
                .filter(|p| {
                    !(p.starts_with('t') && p[1..].starts_with(|c: char| c.is_ascii_digit()))
                })
                .collect::<Vec<_>>()
                .join(" ; ");
            if key != last_state || t % 1000 == 0 || t == ticks {
                log.push(format!("t{t}{state}"));
            }
            last_state = key;
        }
        let _ = writeln!(out, "{name} {}", log.join(" ; "));
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
