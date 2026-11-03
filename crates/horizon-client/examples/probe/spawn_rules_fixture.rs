//! Spawn placement and spawn predicate answers for differential tests against
//! vanilla (tools/test_spawn_rules_parity.py): reads the column-world cases of
//! `VanillaSpawnRulesOracle` and prints `mobs::spawn_rules`' answers.
//!
//! usage: spawn_rules_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    assets::Assets,
    blocks::BlockFacts,
    items::Items,
    mobs::{
        MobTypes,
        data::MobData,
        spawn_rules::{self, Dimension, Reason, SpawnLevel},
    },
};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::loot::PackOverlay;
use horizon_world::{chunk::BlockState, registry::BlockStateId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::Path,
};

struct Column<'a> {
    facts: &'a BlockFacts,
    data: &'a MobData,
    block_tags: &'a HashMap<String, HashSet<String>>,
    air: BlockStateId,
    blocks: HashMap<i32, BlockStateId>,
    x0: i32,
    z0: i32,
    sky: i32,
    light: i32,
    darken: i32,
    thunder: bool,
    sea: i32,
    surface: i32,
    seed: i64,
    peaceful: bool,
    dimension: Dimension,
    biome: String,
    slime: f32,
    /// A player position and whether it is survival or adventure.
    player: Option<([f64; 3], bool)>,
}
impl SpawnLevel for Column<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        if x != self.x0 || z != self.z0 {
            return self.air;
        }
        self.blocks.get(&y).copied().unwrap_or(self.air)
    }
    fn facts(&self) -> &BlockFacts {
        self.facts
    }
    fn mob_data(&self) -> &MobData {
        self.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.block_tags.get(tag).is_some_and(|s| s.contains(block))
    }
    fn biome_tag(&self, tag: &str, _: i32, _: i32, _: i32) -> bool {
        self.data
            .biome_tags
            .get(tag)
            .is_some_and(|s| s.contains(&self.biome))
    }
    fn sky_light(&self, _: i32, _: i32, _: i32) -> i32 {
        self.sky
    }
    fn block_light(&self, _: i32, _: i32, _: i32) -> i32 {
        self.light
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn thundering(&self) -> bool {
        self.thunder
    }
    fn sea_level(&self) -> i32 {
        self.sea
    }
    fn world_surface(&self, _: i32, _: i32) -> i32 {
        self.surface
    }
    fn peaceful(&self) -> bool {
        self.peaceful
    }
    fn dimension(&self) -> Dimension {
        self.dimension
    }
    fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
        self.slime
    }
    fn world_seed(&self) -> i64 {
        self.seed
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.player.is_some_and(|(p, targetable)| {
            let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
            targetable && d < distance * distance
        })
    }
}

/// `name[key=value,...]`.
fn parse_state(spec: &str) -> BlockState {
    let (name, props) = match spec.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    let properties: BTreeMap<String, String> = props
        .split(',')
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    BlockState {
        name: name.to_owned(),
        properties: properties.into(),
    }
}

fn reason(name: &str) -> Reason {
    match name {
        "chunk_generation" => Reason::ChunkGeneration,
        "spawner" => Reason::Spawner,
        "trial_spawner" => Reason::TrialSpawner,
        "reinforcement" => Reason::Reinforcement,
        "patrol" => Reason::Patrol,
        "structure" => Reason::Structure,
        "event" => Reason::Event,
        "command" => Reason::Command,
        _ => Reason::Natural,
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: spawn_rules_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let data = MobData::load(root, &facts);
    let items = Items::tags_only(root, &PackOverlay::NONE)?;
    let types = MobTypes::load(root)?;
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
        let Some(kind) = types.types.get(c["type"]) else {
            let _ = writeln!(out, "{name} error unsupported {}", c["type"]);
            continue;
        };
        let y: i32 = get("y", "64").parse()?;
        let (x0, z0): (i32, i32) = (get("x", "8").parse()?, get("z", "8").parse()?);
        let mut blocks = HashMap::new();
        for (k, v) in &c {
            if let Some(offset) = k.strip_prefix("at") {
                let offset: i32 = offset.parse()?;
                blocks.insert(y + offset, registry.resolve(&parse_state(v))?);
            }
        }
        let player = c.get("player").map(|p| {
            let f: Vec<&str> = p.split(',').collect();
            let at = |i: usize, base: i32| f64::from(base) + f[i].parse::<f64>().unwrap_or(0.0);
            let mode = f.get(3).copied().unwrap_or("survival");
            (
                [at(0, x0), at(1, y), at(2, z0)],
                mode == "survival" || mode == "adventure",
            )
        });
        let level = Column {
            facts: &facts,
            data: &data,
            block_tags: &items.block_tags,
            air: facts.air,
            blocks,
            x0,
            z0,
            sky: get("sky", "15").parse()?,
            light: get("light", "0").parse()?,
            darken: get("darken", "0").parse()?,
            thunder: get("thunder", "0") == "1",
            sea: get("sea", "63").parse()?,
            surface: get("surface", "0").parse()?,
            seed: get("wseed", "0").parse()?,
            peaceful: get("difficulty", "normal") == "peaceful",
            dimension: match get("dim", "minecraft:overworld").as_str() {
                "minecraft:the_nether" => Dimension::Nether,
                "minecraft:the_end" => Dimension::End,
                _ => Dimension::Overworld,
            },
            biome: get("biome", "minecraft:plains"),
            slime: get("slime", "0.0").parse()?,
            player,
        };
        let mut random = LegacyRandom::new(get("seed", "0").parse()?);
        let s = kind.species;
        let ok = spawn_rules::is_spawn_position_ok(s, &level, x0, y, z0);
        let rules = spawn_rules::check_spawn_rules(
            s,
            &level,
            reason(&get("reason", "natural")),
            &mut random,
            x0,
            y,
            z0,
        );
        let next = random.next_int_bound(1000).unwrap_or(0);
        let _ = writeln!(out, "{name} {ok} {rules} {next}");
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
