//! Natural spawning decisions for differential tests against vanilla
//! (tools/test_natural_spawn_parity.py): reads the box-world cases of
//! `VanillaNaturalSpawnOracle` and prints what `mobs::spawn_selection` does in
//! the same order and format.
//!
//! The oracle runs `finalizeSpawn` with a fixed random (nextInt 0, nextFloat
//! 0.75); `neutral_box` is the box that leaves a mob with: polar bears,
//! ocelots and rabbits after the first of their group are babies (chance
//! 1.0), foxes and axolotls from the third, sulfur cubes have size 2, salmon
//! are small, slimes and magma cubes keep size 1, tropical fish school.
//!
//! Spawner cases (`mode=spawner`, tools/test_spawner_parity.py) tick a
//! `mobs::spawner::Spawner` loaded from the case's SNBT on the same world and
//! print its events tick by tick.
//!
//! Trial spawner cases (`mode=trial`, tools/test_trial_spawner_parity.py) tick
//! a `mobs::trial_spawner::TrialSpawner` with the game time advancing and
//! print its states, events, spawns and ejected items.
//!
//! Vault cases (`mode=vault`, tools/test_vault_parity.py) tick a
//! `vault::Vault` the same way, with players moving and using keys on it.
//!
//! usage: natural_spawn_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    assets::Assets,
    biome::{Biomes, SpawnEntry},
    blocks::BlockFacts,
    collisions::{self, LookupCollisions, no_collision_empty},
    items::{ItemStack, Items},
    mobs::{
        MobTypes, Species,
        data::MobData,
        spawn_rules::{Dimension, SpawnLevel},
        spawn_selection::{
            self, Category, Counted, Event, GenEvent, Heightmap, NaturalLevel, SpawnCost,
            SpawnPlayer, SpawnRequest, SpawnState, Spawned, StructureAt, StructureSpawns,
            spawn_box,
        },
        spawner::{
            SpawnDataExt, Spawner, SpawnerAdded, SpawnerEvent, SpawnerExt, SpawnerLevel,
            SpawnerSpawn,
        },
        species::{self, Baby},
        trial_spawner::{
            Omen, TRIAL_OMEN_TICKS_PER_LEVEL, TrialEvent, TrialPlayer, TrialSpawner,
            TrialSpawnerExt, TrialSpawnerLevel, TrialState,
        },
    },
    vault::{Vault, VaultEvent, VaultLevel, VaultPlayer, VaultState},
};
use horizon_core::{
    geometry::Aabb,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry, chunk::BlockState, nbt::Tag, registry::BlockStateId,
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
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
struct Start {
    id: String,
    pieces: Vec<Bx>,
    /// The start's bounding box (pieces encapsulated, inflated by 12 for
    /// terrain adaptation).
    bounds: Bx,
}

struct Shared<'a> {
    facts: &'a BlockFacts,
    data: &'a MobData,
    block_tags: &'a HashMap<String, HashSet<String>>,
    types: &'a MobTypes,
    biomes: &'a Biomes,
    structures: &'a StructureSpawns,
    collision: &'a BlockCollisionRegistry<'a>,
    air: BlockStateId,
    /// Every entity type id (`entity-types.json`).
    entity_types: &'a HashSet<String>,
}
struct World<'a> {
    s: &'a Shared<'a>,
    blocks: Vec<(Bx, BlockStateId)>,
    lights: Vec<(Bx, i32, i32)>,
    biomes: Vec<(Bx, String)>,
    biome: String,
    sky: i32,
    light: i32,
    darken: i32,
    thunder: bool,
    sea: i32,
    seed: i64,
    peaceful: bool,
    dimension: Dimension,
    dimension_id: String,
    slime: f32,
    starts: Vec<Start>,
    players: Vec<([f64; 3], String)>,
    /// Where the non-spectators among them are.
    nearest: Vec<[f64; 3]>,
    respawn: Option<[i32; 3]>,
    unloaded: HashSet<(i32, i32)>,
    spawn_mobs: bool,
    /// The `spawner_blocks_work` game rule.
    spawners_work: bool,
    /// Types and boxes of living entities (case entities, then spawned mobs).
    entities: RefCell<Vec<(String, Aabb)>>,
    /// Blocks set since the case started (a trial spawner's states).
    placed: RefCell<Vec<([i32; 3], BlockStateId)>>,
    /// The game time (trial cases advance it).
    time: Cell<i64>,
    /// The mobs a trial spawner added: UUID, position, height and box.
    trial_mobs: RefCell<Vec<([i32; 4], [f64; 3], f32, Aabb)>>,
    /// How many mobs a trial spawner added (their UUIDs count up).
    trial_count: Cell<i32>,
    /// Each player's omen and its amplifier (`/trial_omen:A`, `/bad_omen:A`).
    omens: RefCell<Vec<Option<(Omen, i32)>>>,
    /// Loot tables (trial cases).
    loot: Option<std::sync::Arc<horizon_world::loot::LootData>>,
}
impl World<'_> {
    fn biome_at(&self, x: i32, y: i32, z: i32) -> &str {
        self.biomes
            .iter()
            .rev()
            .find(|(b, _)| b.has(x, y, z))
            .map_or(&self.biome, |(_, n)| n)
    }
    fn biome_info(&self, x: i32, y: i32, z: i32) -> Option<&horizon_client::biome::BiomeInfo> {
        let i = *self.s.biomes.index.get(self.biome_at(x, y, z))?;
        Some(self.s.biomes.get(i))
    }
    fn lights_at(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        self.lights
            .iter()
            .rev()
            .find(|(b, ..)| b.has(x, y, z))
            .map_or((self.sky, self.light), |(_, s, b)| (*s, *b))
    }
}
impl SpawnLevel for World<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        if let Some((_, s)) = self
            .placed
            .borrow()
            .iter()
            .rev()
            .find(|(p, _)| *p == [x, y, z])
        {
            return *s;
        }
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
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.s
            .data
            .biome_tags
            .get(tag)
            .is_some_and(|s| s.contains(self.biome_at(x, y, z)))
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
        self.thunder
    }
    fn sea_level(&self) -> i32 {
        self.sea
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.heightmap(Heightmap::WorldSurface, x, z)
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
        self.players.iter().any(|(p, mode)| {
            let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
            (mode == "survival" || mode == "adventure") && d < distance * distance
        })
    }
}
impl NaturalLevel for World<'_> {
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
    fn ambient_light(&self) -> f32 {
        match self.dimension {
            Dimension::Overworld => 0.0,
            Dimension::Nether => 0.1,
            Dimension::End => 0.25,
        }
    }
    fn has_ceiling(&self) -> bool {
        self.dimension == Dimension::Nether
    }
    fn mob_type(&self, entity: &str) -> Option<(Species, f32, f32, Category)> {
        let t = self.s.types.types.get(entity)?;
        Some((
            t.species,
            t.width as f32,
            t.height as f32,
            Category::from_name(&t.category),
        ))
    }
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.biome_info(x, y, z).map(|b| &b.spawns)
    }
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        self.biome_info(x, y, z)?.spawn_costs.get(entity).copied()
    }
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome_info(x, y, z)
            .map_or(0.1, |b| b.creature_probability)
    }
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt> {
        let (cx, cz) = (x >> 4, z >> 4);
        let mut out: Vec<StructureAt> = Vec::new();
        for start in self.starts.iter().filter(|s| wanted(&s.id)) {
            let b = start.bounds.0;
            // Referenced: the start's box reaches the chunk's columns.
            if b[3] < cx * 16 || b[0] > cx * 16 + 15 || b[5] < cz * 16 || b[2] > cz * 16 + 15 {
                continue;
            }
            let in_piece = start.pieces.iter().any(|p| p.has(x, y, z));
            let in_bounds = start.bounds.has(x, y, z);
            match out.iter_mut().find(|s| s.id == start.id) {
                Some(s) => {
                    s.in_piece |= in_piece;
                    s.in_bounds |= in_bounds;
                }
                None => out.push(StructureAt {
                    id: start.id.clone(),
                    in_piece,
                    in_bounds,
                }),
            }
        }
        out
    }
    fn structure_spawns(&self) -> &StructureSpawns {
        self.s.structures
    }
    fn nearest_player_positions(&self) -> &[[f64; 3]] {
        &self.nearest
    }
    fn respawn(&self) -> Option<[i32; 3]> {
        self.respawn
    }
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        !self.unloaded.contains(&(cx, cz))
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        let lookup = |x: i32, y: i32, z: i32| self.state(x, y, z);
        let mut world = LookupCollisions {
            block: &lookup,
            binding: self.s.collision,
        };
        // getEntityCollisions(null, box): living shulkers are solid.
        let grown = bounds.inflate(horizon_core::geometry::Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut world, bounds)
            && !self
                .entities
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn unobstructed(&self, bounds: Aabb) -> bool {
        !self
            .entities
            .borrow()
            .iter()
            .any(|(_, b)| b.intersects(bounds))
    }
    fn spawn_mobs(&self) -> bool {
        self.spawn_mobs
    }
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        (self.min_y()..=self.max_y())
            .rev()
            .find(|&y| spawn_selection::heightmap_blocks(self, kind, x, y, z))
            .map_or(self.min_y(), |y| y + 1)
    }
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.s.data.state(self.state(x, y, z)).flags & horizon_client::mobs::data::LAND != 0
    }
}

impl SpawnerLevel for World<'_> {
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.players.iter().any(|(p, mode)| {
            let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
            mode != "spectator" && (range < 0.0 || d < range * range)
        })
    }
    fn spawner_blocks_work(&self) -> bool {
        self.spawners_work
    }
    fn entity_exists(&self, id: &str) -> bool {
        self.s.types.types.contains_key(id) || self.s.entity_types.contains(id)
    }
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize {
        self.entities
            .borrow()
            .iter()
            .filter(|(k, b)| k == id && b.intersects(bounds))
            .count()
    }
}

impl TrialSpawnerLevel for World<'_> {
    fn game_time(&self) -> i64 {
        self.time.get()
    }
    fn mob_spawning(&self) -> bool {
        self.spawn_mobs
    }
    fn players(&self) -> Vec<TrialPlayer> {
        self.players
            .iter()
            .enumerate()
            .map(|(i, (p, mode))| TrialPlayer {
                uuid: [0, 0, 0, i as i32 + 1],
                block: p.map(|v| v.floor() as i32),
                eye: [p[0], p[1] + f64::from(1.62f32), p[2]],
                creative: mode == "creative",
                spectator: mode == "spectator",
                position: *p,
                height: 1.8,
                alive: true,
                omen: self
                    .omens
                    .borrow()
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|(o, _)| o),
            })
            .collect()
    }
    fn visual_hit(&self, from: [f64; 3], to: [f64; 3]) -> Option<[i32; 3]> {
        let lookup = |x: i32, y: i32, z: i32| self.state(x, y, z);
        let mut scene = LookupCollisions {
            block: &lookup,
            binding: self.s.collision,
        };
        let v = |p: [f64; 3]| horizon_core::geometry::Vec3::new(p[0], p[1], p[2]);
        horizon_client::mobs::visual_clip(
            self.s.facts,
            self.s.collision,
            &mut scene,
            &lookup,
            v(from),
            v(to),
        )
        .flatten()
    }
    fn entity_block(&self, uuid: [i32; 4]) -> Option<[i32; 3]> {
        let mobs = self.trial_mobs.borrow();
        let (_, p, ..) = mobs.iter().find(|(u, ..)| *u == uuid)?;
        Some(p.map(|v| v.floor() as i32))
    }
    fn entity_box(&self, uuid: [i32; 4]) -> Option<([f64; 3], f32)> {
        let mobs = self.trial_mobs.borrow();
        let (_, p, h, _) = mobs.iter().find(|(u, ..)| *u == uuid)?;
        Some((*p, *h))
    }
    fn no_block_collision(&self, block: [i32; 3]) -> bool {
        let lookup = |x: i32, y: i32, z: i32| self.state(x, y, z);
        let mut scene = LookupCollisions {
            block: &lookup,
            binding: self.s.collision,
        };
        let state = self.state(block[0], block[1], block[2]);
        self.s
            .collision
            .collision_shape(&mut scene, state, block)
            .map_or(true, |shape| shape.is_empty())
    }
    fn loot_items(&self, table: &str, seed: i64) -> Vec<horizon_client::items::ItemStack> {
        let Some(loot) = &self.loot else {
            return Vec::new();
        };
        loot.roll_split(table, &mut LegacyRandom::new(seed))
            .iter()
            .map(horizon_client::structure_blocks::loot_item_with_components)
            .collect()
    }
    fn seed(&self) -> i64 {
        self.seed
    }
}

/// The box a mob is left with by `finalizeSpawn` under the oracle's fixed random.
fn neutral_box(l: &World, r: &SpawnRequest) -> Aabb {
    let t = &l.s.types.types[r.entity];
    let (mut w, mut h) = (t.width as f32, t.height as f32);
    let baby = match r.species {
        Species::PolarBear | Species::Ocelot | Species::Rabbit => r.group_index > 0,
        Species::Fox | Species::Axolotl => r.group_index >= 2,
        _ => false,
    };
    if baby {
        match species::def(r.species).baby {
            Baby::Abs(bw, bh, _) => (w, h) = (bw as f32, bh as f32),
            Baby::Scale(s, _) => (w, h) = (w * s as f32, h * s as f32),
            Baby::None => {}
        }
    }
    match r.species {
        Species::SulfurCube => (w, h) = (w * 2.0, h * 2.0),
        Species::Salmon => (w, h) = (w * 0.5, h * 0.5),
        _ => {}
    }
    let [x, y, z] = r.position;
    spawn_box(x, y, z, w, h)
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
fn bits(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
/// The yaw as the oracle prints it: a cube mob grown by `finalizeSpawn`
/// (sulfur cubes, size 2 under the fixed random) takes its constructor's
/// head rotation from the entity's unseeded random.
fn yaw_text(entity: &str, v: f32) -> String {
    if entity == "minecraft:sulfur_cube" {
        "head".to_owned()
    } else {
        format!("{:x}", v.to_bits())
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: natural_spawn_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let data = MobData::load(root, &facts);
    let items = Items::tags_only(root)?;
    // Trial spawner rewards: the loot tables and the configs.
    let loot = if input.contains("mode=trial") || input.contains("mode=vault") {
        horizon_client::mobs::trial_spawner::load_registry(root);
        horizon_client::structure_blocks::loot_data(root)
    } else {
        None
    };
    let types = MobTypes::load(root)?;
    let biomes = Biomes::load(root)?;
    let structures = StructureSpawns::load(root);
    let collision = collisions::load(root, registry)?;
    let entity_types: HashSet<String> = fs::read(root.join("entity-types.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| {
            v.as_object().map(|o| {
                o.keys()
                    .map(|k| horizon_client::command::args::normalize(k))
                    .collect()
            })
        })
        .unwrap_or_default();
    let shared = Shared {
        facts: &facts,
        data: &data,
        block_tags: &items.block_tags,
        types: &types,
        biomes: &biomes,
        structures: &structures,
        collision: &collision,
        air: facts.air,
        entity_types: &entity_types,
    };
    let mut adaptation: HashMap<String, bool> = HashMap::new();
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
        let biome_boxes = indexed("b")
            .into_iter()
            .filter_map(|l| {
                let (b, n) = l.split_once('/')?;
                Some((Bx::parse(b), n.to_owned()))
            })
            .collect();
        let mut starts = Vec::new();
        for s in indexed("s") {
            let (id, pieces) = s.split_once('/').ok_or("bad structure")?;
            let pieces: Vec<Bx> = pieces.split(';').map(Bx::parse).collect();
            let mut b = pieces[0].0;
            for p in &pieces[1..] {
                for i in 0..3 {
                    b[i] = b[i].min(p.0[i]);
                    b[i + 3] = b[i + 3].max(p.0[i + 3]);
                }
            }
            let adapts = *adaptation.entry(id.to_owned()).or_insert_with(|| {
                let path = root.join(format!(
                    "data/minecraft/worldgen/structure/{}.json",
                    id.trim_start_matches("minecraft:")
                ));
                fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v["terrain_adaptation"].as_str().map(|a| a != "none"))
                    .unwrap_or(false)
            });
            if adapts {
                for i in 0..3 {
                    b[i] -= 12;
                    b[i + 3] += 12;
                }
            }
            starts.push(Start {
                id: id.to_owned(),
                pieces,
                bounds: Bx(b),
            });
        }
        // A player's third field: an omen and its amplifier.
        let omens = indexed("p")
            .iter()
            .map(|p| {
                let (effect, amplifier) = p.split('/').nth(2)?.split_once(':')?;
                let omen = if effect == "trial_omen" {
                    Omen::Trial
                } else {
                    Omen::Bad
                };
                Some((omen, amplifier.parse().ok()?))
            })
            .collect();
        let players = indexed("p")
            .into_iter()
            .map(|p| {
                let mut parts = p.split('/');
                let v: Vec<f64> = parts
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .map(|n| n.parse().unwrap_or(0.0))
                    .collect();
                (
                    [v[0], v[1], v[2]],
                    parts.next().unwrap_or("survival").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let dimension_id = get("dim", "minecraft:overworld");
        let dimension = match dimension_id.as_str() {
            "minecraft:the_nether" => Dimension::Nether,
            "minecraft:the_end" => Dimension::End,
            _ => Dimension::Overworld,
        };
        let respawn = {
            let r = get("respawn", "0,-1000,0/minecraft:overworld");
            let (p, d) = r.split_once('/').ok_or("bad respawn")?;
            let v: Vec<i32> = p.split(',').map(|n| n.parse().unwrap_or(0)).collect();
            (d == dimension_id).then_some([v[0], v[1], v[2]])
        };
        let unloaded = c
            .get("unloaded")
            .map(|u| {
                u.split(';')
                    .filter_map(|p| {
                        let (x, z) = p.split_once(',')?;
                        Some((x.parse().ok()?, z.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Case entities: (type, position, persistent).
        let mut case_entities = Vec::new();
        for e in indexed("e") {
            let parts: Vec<&str> = e.split('/').collect();
            let v: Vec<f64> = parts[1]
                .split(',')
                .map(|n| n.parse().unwrap_or(0.0))
                .collect();
            // EntityType.create makes no monsters in peaceful.
            if get("difficulty", "normal") == "peaceful"
                && types.types.get(parts[0]).is_some_and(|t| {
                    !horizon_client::mobs::spawn_rules::allowed_in_peaceful(t.species)
                })
            {
                continue;
            }
            case_entities.push((
                parts[0].to_owned(),
                [v[0], v[1], v[2]],
                parts.get(2) == Some(&"p"),
            ));
        }
        let world = World {
            s: &shared,
            blocks,
            lights,
            biomes: biome_boxes,
            biome: get("biome", "minecraft:plains"),
            sky: get("sky", "15").parse()?,
            light: get("light", "0").parse()?,
            darken: get("darken", "0").parse()?,
            thunder: get("thunder", "0") == "1",
            sea: get("sea", "63").parse()?,
            seed: get("wseed", "0").parse()?,
            peaceful: get("difficulty", "normal") == "peaceful",
            dimension,
            dimension_id: dimension_id.clone(),
            slime: get("slime", "0.0").parse()?,
            starts,
            nearest: players
                .iter()
                .filter(|(_, mode)| mode != "spectator")
                .map(|(p, _)| *p)
                .collect(),
            players,
            respawn,
            unloaded,
            spawn_mobs: get("rules", "1") != "0",
            spawners_work: get("spawners", "1") != "0",
            placed: RefCell::new(Vec::new()),
            time: Cell::new(0),
            trial_mobs: RefCell::new(Vec::new()),
            trial_count: Cell::new(0),
            omens: RefCell::new(omens),
            loot: loot.clone(),
            entities: RefCell::new(
                case_entities
                    .iter()
                    .filter_map(|(id, p, _)| {
                        let t = types.types.get(id)?;
                        // Entity.setPos (a shulker sits at its block's centre).
                        let p = horizon_client::mobs::spawner::entity_position(t.species, *p);
                        Some((
                            id.clone(),
                            spawn_box(p[0], p[1], p[2], t.width as f32, t.height as f32),
                        ))
                    })
                    .collect(),
            ),
        };
        let _ = &world.dimension_id;
        let chunk = {
            let v: Vec<i32> = get("chunk", "0,0")
                .split(',')
                .map(|n| n.parse().unwrap_or(0))
                .collect();
            (v[0], v[1])
        };
        let mut random = LegacyRandom::new(get("seed", "0").parse()?);
        let mut log: Vec<String> = Vec::new();
        let mut spawn = |l: &World, r: &SpawnRequest| -> Option<Spawned> {
            let bounds = neutral_box(l, r);
            l.entities.borrow_mut().push((r.entity.to_owned(), bounds));
            Some(Spawned {
                bounds,
                school: true,
            })
        };
        let mode = get("mode", "chunk");
        if mode == "spawner" {
            spawner_case(&world, &get, &mut random, &mut log)?;
        } else if mode == "trial" {
            let loot = loot.as_ref().ok_or("no loot tables")?;
            trial_case(&world, registry, loot, &get, &mut random, &mut log)?;
        } else if mode == "vault" {
            let loot = loot.as_ref().ok_or("no loot tables")?;
            vault_case(world.players.clone(), loot, &get, &mut random, &mut log)?;
        } else if mode == "gen" {
            let mut gen_random = LegacyRandom::new(get("gseed", "0").parse()?);
            // The level random of the spawn predicates: no creature's predicate
            // draws from it (the oracle's answers fixed values).
            let mut rules = LegacyRandom::new(0);
            let mut spawns = Vec::new();
            let source = [chunk.0 * 16, world.max_y(), chunk.1 * 16];
            spawn_selection::spawn_mobs_for_chunk_generation(
                &world,
                source,
                chunk,
                &mut gen_random,
                &mut rules,
                &mut spawn,
                &mut |e| match e {
                    GenEvent::Column(x, z) => log.push(format!("col {x} {z}")),
                    GenEvent::Spawn(t, p, yaw) => spawns.push(format!(
                        "spawn {t} {} {} {} {}",
                        bits(p[0]),
                        bits(p[1]),
                        bits(p[2]),
                        yaw_text(&t, yaw)
                    )),
                },
            );
            log.extend(spawns);
            log.push(format!(
                "next {}",
                gen_random.next_int_bound(1000).unwrap_or(0)
            ));
        } else {
            let counted: Vec<Counted> = case_entities
                .iter()
                .filter_map(|(t, p, persistent)| {
                    let kind = types.types.get(t)?;
                    Some(Counted {
                        entity: t,
                        category: Category::from_name(&kind.category),
                        block: [
                            p[0].floor() as i32,
                            p[1].floor() as i32,
                            p[2].floor() as i32,
                        ],
                        is_mob: true,
                        persistent: *persistent,
                    })
                })
                .collect();
            let players = world
                .players
                .iter()
                .map(|(p, mode)| SpawnPlayer {
                    position: *p,
                    spectator: mode == "spectator",
                })
                .collect();
            let mut state = SpawnState::new(
                &world,
                get("chunkcount", "289").parse()?,
                players,
                &counted,
                |cx, cz| !world.unloaded.contains(&(cx, cz)),
            );
            let mut trace = |e: Event| {
                log.push(match e {
                    Event::Categories(list) => format!(
                        "cats {}",
                        list.iter().map(|c| c.name()).collect::<Vec<_>>().join(",")
                    ),
                    Event::Local(c, ok) => format!("local {} {ok}", c.name()),
                    Event::Start(c, p) => format!("start {} {} {} {}", c.name(), p[0], p[1], p[2]),
                    Event::Try(x, z) => format!("try {x} {z}"),
                    Event::Test(t, p, ok) => format!("test {t} {} {} {} {ok}", p[0], p[1], p[2]),
                    Event::Spawn(t, p, yaw) => format!(
                        "spawn {t} {} {} {} {}",
                        bits(p[0]),
                        bits(p[1]),
                        bits(p[2]),
                        yaw_text(&t, yaw)
                    ),
                })
            };
            if mode == "pos" {
                let category = Category::from_name(&get("cat", "monster"));
                let v: Vec<i32> = get("start", "0,0,0")
                    .split(',')
                    .map(|n| n.parse().unwrap_or(0))
                    .collect();
                spawn_selection::spawn_category_for_position(
                    &world,
                    &mut random,
                    &mut state,
                    category,
                    chunk,
                    [v[0], v[1], v[2]],
                    &mut spawn,
                    &mut trace,
                );
            } else {
                let categories = state
                    .filtered_categories(get("enemies", "1") == "1", get("persistent", "1") == "1");
                trace(Event::Categories(categories.clone()));
                spawn_selection::spawn_for_chunk(
                    &world,
                    &mut random,
                    &mut state,
                    &categories,
                    chunk,
                    &mut spawn,
                    &mut trace,
                );
            }
            log.push(format!("next {}", random.next_int_bound(1000).unwrap_or(0)));
        }
        let _ = writeln!(out, "{name} {}", log.join(" ; "));
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}

/// `SpawnerBlockEntity.serverTick` `ticks` times at `spos` for the spawner in
/// `snbt` (base64 SNBT), as the oracle logs it.
fn spawner_case(
    world: &World,
    get: &dyn Fn(&str, &'static str) -> String,
    random: &mut LegacyRandom,
    log: &mut Vec<String>,
) -> horizon_client::Result<()> {
    let v: Vec<i32> = get("spos", "0,0,0")
        .split(',')
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let pos = [v[0], v[1], v[2]];
    let text = horizon_client::skins::base64_decode(&get("snbt", "")).ok_or("bad snbt")?;
    let tag = horizon_world::nbt::parse_snbt(&String::from_utf8(text)?)?.0;
    let mut spawner = Spawner::load(&tag);
    let ticks: i32 = get("ticks", "1").parse()?;
    for tick in 0..ticks {
        let mut events = Vec::new();
        let mut spawned = std::collections::VecDeque::new();
        let mut spawn = |l: &World, r: &SpawnerSpawn| -> Option<SpawnerAdded> {
            let t = l.s.types.types.get(r.id)?;
            let [x, y, z] = r.position;
            let bounds = spawn_box(x, y, z, t.width as f32, t.height as f32);
            l.entities.borrow_mut().push((r.id.to_owned(), bounds));
            spawned.push_back(format!(
                "{tick} spawn {} {} {} {} {}",
                r.id,
                bits(x),
                bits(y),
                bits(z),
                // Shulker.finalizeSpawn faces it south.
                yaw_text(
                    r.id,
                    if r.finalize && r.id == "minecraft:shulker" {
                        0.0
                    } else {
                        r.yaw
                    }
                )
            ));
            Some(SpawnerAdded {
                id: spawned.len() as u32,
                bounds,
                mob: true,
            })
        };
        spawner.server_tick(world, random, pos, &mut spawn, &mut events);
        for e in events {
            match e {
                SpawnerEvent::BlockUpdated => log.push(format!("{tick} update")),
                SpawnerEvent::BlockEvent(id) => log.push(format!("{tick} bevent {id} 0")),
                // Each spawn's level event follows its tryAddFreshEntityWithPassengers.
                SpawnerEvent::LevelEvent(id) => {
                    log.extend(spawned.pop_front());
                    log.push(format!("{tick} levent {id} 0"));
                }
                SpawnerEvent::EntityPlace(_, p) => log.push(format!(
                    "{tick} gevent minecraft:entity_place {} {} {}",
                    p[0], p[1], p[2]
                )),
                SpawnerEvent::SpawnAnim(_) => log.push(format!("{tick} eevent 20")),
            }
        }
    }
    let next = spawner
        .next_data()
        .and_then(|d| d.id())
        .unwrap_or_else(|| "-".into());
    let pots: Vec<String> = spawner
        .potentials
        .iter()
        .map(|(d, w)| format!("{}*{w}", d.id().unwrap_or_else(|| "-".into())))
        .collect();
    log.push(format!(
        "end delay={} next={next} pots={}",
        spawner.spawn_delay as i16,
        pots.join(",")
    ));
    log.push(format!("next {}", random.next_int_bound(1000).unwrap_or(0)));
    Ok(())
}

/// `TrialSpawner.tickServer` `ticks` times from game time `t0` at `spos` on a
/// trial spawner block in state `state` (ominous with `ominous=1`) loaded from
/// `snbt` (base64 SNBT), as the oracle logs it. Every `kill` ticks the mobs it
/// spawned die.
fn trial_case(
    world: &World,
    registry: &horizon_world::registry::BlockRegistry,
    loot: &horizon_world::loot::LootData,
    get: &dyn Fn(&str, &'static str) -> String,
    random: &mut LegacyRandom,
    log: &mut Vec<String>,
) -> horizon_client::Result<()> {
    let v: Vec<i32> = get("spos", "0,0,0")
        .split(',')
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let pos = [v[0], v[1], v[2]];
    let block_state = |state: TrialState, ominous: bool| {
        parse_state(&format!(
            "minecraft:trial_spawner[trial_spawner_state={},ominous={ominous}]",
            state.name()
        ))
    };
    let mut state =
        TrialState::from_name(&get("state", "waiting_for_players")).ok_or("bad state")?;
    let mut ominous = get("ominous", "0") == "1";
    world
        .placed
        .borrow_mut()
        .push((pos, registry.resolve(&block_state(state, ominous))?));
    let text = horizon_client::skins::base64_decode(&get("snbt", "")).ok_or("bad snbt")?;
    let tag = horizon_world::nbt::parse_snbt(&String::from_utf8(text)?)?.0;
    let mut spawner = TrialSpawner::load(&tag);
    let ticks: i64 = get("ticks", "1").parse()?;
    let t0: i64 = get("t0", "0").parse()?;
    let kill: i64 = get("kill", "0").parse()?;
    let seed = world.seed;
    let mut sequences = horizon_client::random_sequences::RandomSequences::default();
    let center = pos.map(|v| f64::from(v) + 0.5);
    for tick in 0..ticks {
        world.time.set(t0 + tick);
        let mut events = Vec::new();
        let mut spawned = std::collections::VecDeque::new();
        let mut items = Vec::new();
        let mut spawn = |l: &World, r: &SpawnerSpawn| {
            let t = l.s.types.types.get(r.id)?;
            // Slime.readAdditionalSaveData: size Size + 1, clamped to 1..=127.
            let size = match r.entity.get(&horizon_world::nbt::JavaString::from("Size")) {
                Some(Tag::Byte(v)) => i32::from(*v),
                Some(Tag::Short(v)) => i32::from(*v),
                Some(Tag::Int(v)) => *v,
                Some(Tag::Long(v)) => *v as i32,
                _ => 0,
            };
            let size = if matches!(r.id, "minecraft:slime" | "minecraft:magma_cube") {
                size.saturating_add(1).clamp(1, 127)
            } else {
                1
            };
            let [x, y, z] = r.position;
            let scale = size as f32;
            let bounds = spawn_box(x, y, z, t.width as f32 * scale, t.height as f32 * scale);
            l.entities.borrow_mut().push((r.id.to_owned(), bounds));
            let uuid = [1, 0, 0, l.trial_count.get()];
            l.trial_count.set(l.trial_count.get() + 1);
            // Zombie.readAdditionalSaveData: IsBaby halves the box
            // (getAgeScale); finalizeSpawn's fixed random never makes one.
            let baby = match r
                .entity
                .get(&horizon_world::nbt::JavaString::from("IsBaby"))
            {
                Some(Tag::Byte(v)) => *v != 0,
                Some(Tag::Int(v)) => *v != 0,
                _ => false,
            };
            let height = t.height as f32 * scale * if baby { 0.5 } else { 1.0 };
            l.trial_mobs
                .borrow_mut()
                .push((uuid, r.position, height, bounds));
            spawned.push_back(format!(
                "{tick} spawn {} {} {} {} {}",
                r.id,
                bits(x),
                bits(y),
                bits(z),
                if size == 1 {
                    yaw_text(r.id, r.yaw)
                } else {
                    "head".to_owned()
                }
            ));
            Some(uuid)
        };
        let mut eject = |table: &str, random: &mut LegacyRandom| {
            let drops = match loot.random_sequence(table) {
                Some(s) => loot.roll_split(table, sequences.get(s, seed)),
                None => loot.roll_split(table, random),
            };
            for stack in &drops {
                random.next_double();
                let spread = 0.017_227_5 * 2.0;
                let v = [
                    random.triangle(0.0, spread),
                    random.triangle(0.2, spread),
                    random.triangle(0.0, spread),
                ];
                items.push(format!(
                    "{tick} item {} {} {} {} {} {} {} {}",
                    stack.item,
                    stack.count,
                    bits(center[0]),
                    bits(center[1] + 0.7 - 0.125),
                    bits(center[2]),
                    bits(v[0]),
                    bits(v[1]),
                    bits(v[2])
                ));
            }
            !drops.is_empty()
        };
        let next = spawner.server_tick(
            world,
            random,
            pos,
            state,
            ominous,
            &mut spawn,
            &mut eject,
            &mut events,
        );
        for e in events {
            match e {
                TrialEvent::BlockUpdated => log.push(format!("{tick} update")),
                TrialEvent::LevelEvent(id, p, data) => {
                    // A spawn's events follow its tryAddFreshEntityWithPassengers,
                    // a reward's its items.
                    match id {
                        3011 => log.extend(spawned.pop_front()),
                        3014 => log.append(&mut items),
                        _ => {}
                    }
                    match id {
                        3020 | 3012 => log.push(format!(
                            "{tick} levent {id} {data} at {},{},{}",
                            p[0], p[1], p[2]
                        )),
                        _ => log.push(format!("{tick} levent {id} {data}")),
                    }
                }
                TrialEvent::Sound(name) => log.push(format!(
                    "{tick} sound minecraft:{name} {} {} {} 1.0 1.0",
                    bits(center[0]),
                    bits(center[1]),
                    bits(center[2])
                )),
                TrialEvent::EntityPlace(_, p) => log.push(format!(
                    "{tick} gevent minecraft:entity_place {} {} {}",
                    p[0], p[1], p[2]
                )),
                TrialEvent::TrialOmen(uuid) => {
                    let i = (uuid[3] - 1) as usize;
                    let mut omens = world.omens.borrow_mut();
                    let amplifier = omens.get(i).copied().flatten().map_or(0, |(_, a)| a);
                    log.push(format!("{tick} effect remove minecraft:bad_omen"));
                    log.push(format!(
                        "{tick} effect add minecraft:trial_omen {} 0",
                        TRIAL_OMEN_TICKS_PER_LEVEL * (amplifier + 1)
                    ));
                    if let Some(o) = omens.get_mut(i) {
                        *o = Some((Omen::Trial, 0));
                    }
                }
                TrialEvent::Discard(uuid) => {
                    let mut mobs = world.trial_mobs.borrow_mut();
                    if let Some(i) = mobs.iter().position(|(u, ..)| *u == uuid) {
                        let (.., b) = mobs.remove(i);
                        world.entities.borrow_mut().retain(|(_, e)| *e != b);
                    }
                }
                TrialEvent::SoundAt(name, b, pitch) => log.push(format!(
                    "{tick} sound minecraft:{name} {} {} {} 1.0 {pitch:?}",
                    bits(f64::from(b[0]) + 0.5),
                    bits(f64::from(b[1]) + 0.5),
                    bits(f64::from(b[2]) + 0.5)
                )),
                TrialEvent::ItemSpawner {
                    item,
                    position,
                    delay,
                } => log.push(format!(
                    "{tick} ispawner {} {} {} {} {} {delay}",
                    item.item,
                    item.count,
                    bits(position[0]),
                    bits(position[1]),
                    bits(position[2])
                )),
                TrialEvent::OminousSet(set) => {
                    ominous = set;
                    world
                        .placed
                        .borrow_mut()
                        .push((pos, registry.resolve(&block_state(state, ominous))?));
                    log.push(format!("{tick} state {} {ominous}", state.name()));
                }
            }
        }
        if next != state || spawner.is_ominous != ominous {
            state = next;
            ominous = spawner.is_ominous;
            world
                .placed
                .borrow_mut()
                .push((pos, registry.resolve(&block_state(state, ominous))?));
            log.push(format!("{tick} state {} {ominous}", state.name()));
        }
        if kill > 0 && tick % kill == kill - 1 {
            let dead: Vec<Aabb> = world
                .trial_mobs
                .borrow_mut()
                .drain(..)
                .map(|(.., b)| b)
                .collect();
            world
                .entities
                .borrow_mut()
                .retain(|(_, b)| !dead.iter().any(|d| d == b));
            if !dead.is_empty() {
                log.push(format!("{tick} kill {}", dead.len()));
            }
        }
    }
    log.push(format!(
        "end state={} players={} mobs={} cooldown={} next_spawn={} total={} data={} eject={}",
        state.name(),
        spawner.detected_players.len(),
        spawner.current_mobs.len(),
        spawner.cooldown_ends_at,
        spawner.next_mob_spawns_at,
        spawner.total_mobs_spawned,
        spawner
            .next_spawn_data
            .as_ref()
            .and_then(|d| d.id())
            .unwrap_or_else(|| "-".into()),
        spawner.ejecting_loot_table.as_deref().unwrap_or("-")
    ));
    log.push(format!("next {}", random.next_int_bound(1000).unwrap_or(0)));
    Ok(())
}

/// What a vault reads: the game time and the case's players, moved by `move`.
struct VaultWorld {
    time: i64,
    players: Vec<([f64; 3], String)>,
}
impl VaultLevel for VaultWorld {
    fn game_time(&self) -> i64 {
        self.time
    }
    fn players(&self) -> Vec<VaultPlayer> {
        self.players
            .iter()
            .enumerate()
            .map(|(i, (p, mode))| VaultPlayer {
                uuid: [0, 0, 0, i as i32 + 1],
                block: p.map(|v| v.floor() as i32),
                spectator: mode == "spectator",
            })
            .collect()
    }
}
/// A UUID's low 64 bits, as the oracle prints players.
fn uuid_low(u: &[i32; 4]) -> String {
    ((i64::from(u[2]) << 32) | i64::from(u[3] as u32)).to_string()
}
/// The display item and the connected players as the oracle prints them.
fn vault_shared(v: &Vault) -> String {
    let mut connected: Vec<String> = v.connected_players.iter().map(uuid_low).collect();
    connected.sort();
    format!(
        "{} [{}]",
        v.display_item
            .as_ref()
            .map_or_else(|| "-".to_owned(), |s| format!("{}*{}", s.item, s.count)),
        connected.join(",")
    )
}

/// `VaultBlockEntity.Server.tick` `ticks` times from game time `t0` at `spos`
/// on a vault block in state `state` (ominous with `ominous=1`) loaded from
/// `snbt` (base64 SNBT), with the case's key uses (`use`) and player moves
/// (`move`) before each tick, as the oracle logs it.
fn vault_case(
    players: Vec<([f64; 3], String)>,
    loot: &horizon_world::loot::LootData,
    get: &dyn Fn(&str, &'static str) -> String,
    random: &mut LegacyRandom,
    log: &mut Vec<String>,
) -> horizon_client::Result<()> {
    let v: Vec<i32> = get("spos", "0,0,0")
        .split(',')
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    let pos = [v[0], v[1], v[2]];
    let mut state = VaultState::from_name(&get("state", "inactive")).ok_or("bad state")?;
    let ominous = get("ominous", "0") == "1";
    let text = horizon_client::skins::base64_decode(&get("snbt", "")).ok_or("bad snbt")?;
    let tag = horizon_world::nbt::parse_snbt(&String::from_utf8(text)?)?.0;
    let mut vault = Vault::load(&tag);
    let schedule = |key: &str| -> Vec<Vec<String>> {
        get(key, "")
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.split('/').map(str::to_owned).collect())
            .collect()
    };
    let (uses, moves) = (schedule("use"), schedule("move"));
    let ticks: i64 = get("ticks", "1").parse()?;
    let t0: i64 = get("t0", "0").parse()?;
    let seed: i64 = get("wseed", "0").parse()?;
    let mut sequences = horizon_client::random_sequences::RandomSequences::default();
    let mut world = VaultWorld { time: t0, players };
    let center = pos.map(|v| f64::from(v) + 0.5);
    for tick in 0..ticks {
        world.time = t0 + tick;
        let at = |s: &[String]| s[0].parse() == Ok(tick);
        for m in moves.iter().filter(|m| at(m)) {
            let v: Vec<f64> = m[2].split(',').map(|n| n.parse().unwrap_or(0.0)).collect();
            world.players[m[1].parse::<usize>()?].0 = [v[0], v[1], v[2]];
        }
        let mut events = Vec::new();
        let mut roll = |table: &str, random: &mut LegacyRandom, sequence: bool| {
            let drops = match loot.random_sequence(table).filter(|_| sequence) {
                Some(s) => loot.roll_split(table, sequences.get(s, seed)),
                None => loot.roll_split(table, random),
            };
            drops
                .iter()
                .map(horizon_client::structure_blocks::loot_item_with_components)
                .collect()
        };
        for u in uses.iter().filter(|u| at(u)) {
            let player: usize = u[1].parse()?;
            let mut stack = ItemStack::new(&u[2], u[3].parse()?);
            let used = vault.use_item_on(
                &world,
                random,
                pos,
                state,
                ominous,
                [0, 0, 0, player as i32 + 1],
                Some(&stack),
                &mut roll,
                &mut events,
            );
            if world.players[player].1 != "creative" {
                stack.count -= used.unwrap_or(0);
            }
            vault_events(&mut events, &mut state, ominous, tick, center, &vault, log);
            log.push(format!(
                "{tick} use {} {}",
                if used.is_some() { "ok" } else { "pass" },
                stack.count
            ));
        }
        vault.server_tick(&world, random, pos, state, ominous, &mut roll, &mut events);
        vault_events(&mut events, &mut state, ominous, tick, center, &vault, log);
    }
    let saved = vault.save();
    let config = saved
        .iter()
        .find(|(k, _)| *k == "config")
        .and_then(|(_, t)| match t {
            Tag::Compound(c) => {
                let mut keys: Vec<String> = c.keys().map(|k| k.to_string_lossy()).collect();
                keys.sort();
                Some(keys.join(","))
            }
            _ => None,
        })
        .unwrap_or_default();
    let eject: Vec<String> = vault
        .items_to_eject
        .iter()
        .map(|s| format!("{}*{}", s.item, s.count))
        .collect();
    log.push(format!(
        "end state={} rewarded={} resumes={} eject={} total={} shared={} config={config}",
        state.name(),
        vault
            .rewarded_players
            .iter()
            .map(uuid_low)
            .collect::<Vec<_>>()
            .join(","),
        vault.state_updating_resumes_at,
        eject.join(","),
        vault.total_ejections_needed,
        vault_shared(&vault),
    ));
    log.push(format!("next {}", random.next_int_bound(1000).unwrap_or(0)));
    Ok(())
}
/// Logs a vault's events and applies its state changes.
fn vault_events(
    events: &mut Vec<VaultEvent>,
    state: &mut VaultState,
    ominous: bool,
    tick: i64,
    center: [f64; 3],
    vault: &Vault,
    log: &mut Vec<String>,
) {
    for e in events.drain(..) {
        match e {
            VaultEvent::State(s) => {
                *state = s;
                log.push(format!("{tick} state {} {ominous}", s.name()));
            }
            VaultEvent::BlockUpdated => log.push(format!("{tick} update {}", vault_shared(vault))),
            VaultEvent::LevelEvent(id, data) => log.push(format!("{tick} levent {id} {data}")),
            VaultEvent::Sound(name, volume, pitch) => log.push(format!(
                "{tick} sound minecraft:{name} {} {} {} {volume:?} {pitch:?}",
                bits(center[0]),
                bits(center[1]),
                bits(center[2])
            )),
            VaultEvent::Eject(stack, v) => log.push(format!(
                "{tick} item {} {} {} {} {} {} {} {}",
                stack.item,
                stack.count,
                bits(center[0]),
                bits(center[1] + 0.7 - 0.125),
                bits(center[2]),
                bits(v[0]),
                bits(v[1]),
                bits(v[2])
            )),
        }
    }
}
