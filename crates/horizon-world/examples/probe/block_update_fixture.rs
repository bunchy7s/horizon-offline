//! Rust side of the block update, random tick and redstone parity harnesses
//! (`tools/test_block_update_parity.py`, `test_random_tick_parity.py`,
//! `test_redstone_parity.py`): replays `VanillaBlockUpdateOracle` cases against
//! `Behaviours` over a map-backed level and prints the same result text.
use horizon_core::{
    geometry::{Aabb, Direction},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    level::{
        Behaviours, Dimension, EntityQuery, GameRules, LevelAccess, LevelEvent, Ticks,
        data::load_registry,
        removal::{PreRemove, drop_item_stack, pre_remove},
    },
    registry::BlockStateId,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
    io::{BufRead, Write},
    path::Path,
};

struct Cells {
    cells: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    time: i64,
    sky: i32,
    block: i32,
    raining: bool,
    thundering: bool,
    events: Vec<LevelEvent>,
    command_blocks: HashMap<BlockPos, bool>,
    lecterns_with_books: std::rc::Rc<std::collections::HashSet<BlockStateId>>,
    /// `preRemoveSideEffects` of each state's block entity.
    removals: std::rc::Rc<Vec<PreRemove>>,
}
impl LevelAccess for Cells {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.cells.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        let old = self.block_state(pos);
        if state == self.air {
            self.cells.remove(&pos);
        } else {
            self.cells.insert(pos, state);
        }
        Some(old)
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    /// Fresh block entities hold nothing, but a lectern whose state has a book
    /// reads page progress 1 of an empty book: signal 14.
    fn block_entity_analog_signal(
        &self,
        _pos: BlockPos,
        state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        if self.lecterns_with_books.contains(&state) {
            14
        } else {
            0
        }
    }
    /// A chest's fresh block entity: nobody has it open.
    fn recheck_container_openers(&mut self, _pos: BlockPos) -> Option<(i32, i32)> {
        Some((0, 0))
    }
    /// A fresh command block entity: unpowered and not automatic.
    fn set_command_block_powered(&mut self, pos: BlockPos, powered: bool) -> Option<(bool, bool)> {
        Some((
            self.command_blocks.insert(pos, powered).unwrap_or(false),
            false,
        ))
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn block_light(&self, _pos: BlockPos) -> i32 {
        self.block
    }
    fn sky_light(&self, _pos: BlockPos) -> i32 {
        self.sky
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn is_thundering(&self) -> bool {
        self.thundering
    }
    fn is_raining_at(&self, _pos: BlockPos) -> bool {
        self.raining
    }
    fn dimension(&self) -> Dimension {
        Dimension::Overworld
    }
    fn game_rules(&self) -> GameRules {
        GameRules::default()
    }
    fn count_entities(&self, _area: Aabb, _query: EntityQuery) -> usize {
        0
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
    /// The oracle level's block entities are fresh: containers spill only
    /// empty slots (which still draw their spot), a lectern whose state has a
    /// book drops its empty book, a fresh shrieker has no warning level.
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        let kind = self.removals.get(old.0 as usize).copied();
        if side_effects {
            let slots = match kind {
                Some(PreRemove::Contents { slots, .. }) => slots,
                Some(PreRemove::Campfire) => 4,
                Some(PreRemove::Lectern) if self.lecterns_with_books.contains(&old) => {
                    self.events.push(LevelEvent::BlockEvent {
                        pos,
                        state: old,
                        kind: "spawn_item",
                    });
                    0
                }
                _ => 0,
            };
            for _ in 0..slots {
                drop_item_stack(&mut self.random, pos, 0);
            }
        }
        // BlockEntity.setRemoved: a jukebox stops its (absent) song, a beacon
        // plays its deactivation.
        match kind {
            Some(PreRemove::Jukebox) => {
                self.events.push(LevelEvent::GameEvent {
                    pos,
                    event: "minecraft:jukebox_stop_play",
                    state: Some(old),
                });
                self.events.push(LevelEvent::LevelEvent {
                    id: 1011,
                    pos,
                    data: 0,
                });
            }
            Some(PreRemove::Beacon) => self.events.push(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.beacon.deactivate".into(),
                volume: 1.0,
                pitch: 1.0,
            }),
            _ => {}
        }
        0
    }
}

fn pos_of(value: &Value) -> BlockPos {
    let c: Vec<i32> = value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i32)
        .collect();
    BlockPos::new(c[0], c[1], c[2])
}

fn p(pos: BlockPos) -> String {
    format!("{},{},{}", pos.x, pos.y, pos.z)
}

fn summary(level: &Cells, before: &HashMap<BlockPos, BlockStateId>, out: &mut String) {
    let mut all: BTreeMap<(i32, i32, i32), ()> = BTreeMap::new();
    for pos in before.keys().chain(level.cells.keys()) {
        all.insert((pos.x, pos.y, pos.z), ());
    }
    for (x, y, z) in all.keys() {
        let pos = BlockPos::new(*x, *y, *z);
        let a = before.get(&pos).copied().unwrap_or(level.air);
        let b = level.block_state(pos);
        if a != b {
            let _ = writeln!(out, "set {} {}", p(pos), b.0);
        }
    }
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = crate::args().skip(1);
    let resources = args.next().expect("resource directory");
    let cases = args.next().expect("case file");
    let output = args.next().expect("output file");
    let resources = Path::new(&resources);
    let registry = load_registry(resources)?;
    let behaviours = Behaviours::load(&registry, resources)?;
    let air = registry.default_state("minecraft:air").unwrap();
    let lecterns_with_books: std::rc::Rc<std::collections::HashSet<BlockStateId>> =
        std::rc::Rc::new(
            (0..registry.len() as u32)
                .map(BlockStateId)
                .filter(|&id| {
                    registry.state(id).is_some_and(|s| {
                        s.name == "minecraft:lectern"
                            && s.properties.get("has_book").is_some_and(|v| v == "true")
                    })
                })
                .collect(),
        );
    let removals: std::rc::Rc<Vec<PreRemove>> = std::rc::Rc::new(
        (0..registry.len() as u32)
            .map(|id| {
                registry
                    .state(BlockStateId(id))
                    .map_or(PreRemove::Nothing, |s| pre_remove(&s.name))
            })
            .collect(),
    );
    let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
    for (index, line) in std::io::BufReader::new(std::fs::File::open(cases)?)
        .lines()
        .enumerate()
    {
        let case: Value = serde_json::from_str(&line?)?;
        let mut cells = HashMap::new();
        for cell in case["cells"].as_array().unwrap() {
            let c = cell.as_array().unwrap();
            let state = BlockStateId(c[3].as_i64().unwrap() as u32);
            if state != air {
                cells.insert(
                    BlockPos::new(
                        c[0].as_i64().unwrap() as i32,
                        c[1].as_i64().unwrap() as i32,
                        c[2].as_i64().unwrap() as i32,
                    ),
                    state,
                );
            }
        }
        let before = cells.clone();
        let light = case.get("light").and_then(|v| v.as_array());
        let mut level = Cells {
            cells,
            air,
            random: LegacyRandom::new(case["seed"].as_i64().unwrap()),
            time: case.get("time").and_then(|v| v.as_i64()).unwrap_or(0),
            sky: light.map_or(15, |l| l[0].as_i64().unwrap() as i32),
            block: light.map_or(0, |l| l[1].as_i64().unwrap() as i32),
            raining: case
                .get("raining")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            thundering: case
                .get("thundering")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            events: Vec::new(),
            command_blocks: HashMap::new(),
            lecterns_with_books: lecterns_with_books.clone(),
            removals: removals.clone(),
        };
        let mut ticks = Ticks::new(0);
        for x in -3..=3 {
            for z in -3..=3 {
                ticks.add_empty_chunk(ChunkPos::new(x, z))?;
            }
        }
        let mut result = format!("case {index}\n");
        let origin = pos_of(&case["pos"]);
        match case["mode"].as_str().unwrap() {
            "shape" => {
                let dir = Direction::ALL[case["dir"].as_u64().unwrap() as usize];
                let neighbour = BlockStateId(case["neighbor"].as_i64().unwrap() as u32);
                let state = level.block_state(origin);
                let s = behaviours.update_shape(
                    &mut level,
                    &mut ticks,
                    state,
                    origin,
                    dir,
                    {
                        let [x, y, z] = dir.normal();
                        origin.offset(x, y, z)
                    },
                    neighbour,
                );
                let _ = writeln!(result, "shape {}", s.0);
            }
            "random" => behaviours.random_tick(&mut level, &mut ticks, origin),
            "tick" => behaviours.scheduled_block_tick(&mut level, &mut ticks, origin),
            "ftick" => behaviours.scheduled_fluid_tick(&mut level, &mut ticks, origin),
            "neighbor" => {
                let from = pos_of(&case["from"]);
                let changed = behaviours.block(level.block_state(from));
                behaviours.handle_neighbor_changed(&mut level, &mut ticks, origin, changed);
            }
            "run" => {
                for op in case["ops"].as_array().unwrap() {
                    match op["op"].as_str().unwrap() {
                        "set" => {
                            behaviours.set_block(
                                &mut level,
                                &mut ticks,
                                pos_of(&op["pos"]),
                                BlockStateId(op["state"].as_i64().unwrap() as u32),
                                op["flags"].as_i64().unwrap() as i32,
                            );
                        }
                        "ticks" => {
                            let snap = op.get("snap").and_then(|v| v.as_bool()).unwrap_or(false);
                            for _ in 0..op["n"].as_i64().unwrap() {
                                level.time += 1;
                                behaviours.tick_scheduled(&mut level, &mut ticks);
                                if snap {
                                    let _ = writeln!(result, "tick {}", level.time);
                                    summary(&level, &before, &mut result);
                                }
                            }
                        }
                        other => panic!("unknown op {other}"),
                    }
                }
            }
            other => panic!("unknown mode {other}"),
        }
        summary(&level, &before, &mut result);
        // Pending scheduled ticks, sorted as the oracle sorts its rows.
        let mut rows: Vec<String> = Vec::new();
        for x in -3..=3 {
            for z in -3..=3 {
                let chunk = ChunkPos::new(x, z);
                if let Some(c) = ticks.blocks.chunk(chunk) {
                    for t in c.queued() {
                        let block = horizon_world::level::BlockId(t.kind.0 as u16);
                        rows.push(format!(
                            "bt {} {} {} {}",
                            p(t.pos),
                            behaviours.table.info(block).name,
                            t.trigger_tick - level.time,
                            t.priority as i32
                        ));
                    }
                }
                if let Some(c) = ticks.fluids.chunk(chunk) {
                    for t in c.queued() {
                        let name = ["empty", "flowing_water", "water", "flowing_lava", "lava"]
                            [t.kind.0 as usize];
                        rows.push(format!(
                            "ft {} minecraft:{} {} {}",
                            p(t.pos),
                            name,
                            t.trigger_tick - level.time,
                            t.priority as i32
                        ));
                    }
                }
            }
        }
        rows.sort();
        for r in rows {
            result.push_str(&r);
            result.push('\n');
        }
        for event in &level.events {
            match event {
                LevelEvent::LevelEvent { id, pos, data } => {
                    let _ = writeln!(result, "L {} {} {}", id, p(*pos), data);
                }
                LevelEvent::Sound { pos, sound, .. } => {
                    let _ = writeln!(result, "S {} {}", p(*pos), sound);
                }
                LevelEvent::DropResources { state, .. } => {
                    let name = behaviours.table.name(*state);
                    let short = name.strip_prefix("minecraft:").unwrap_or(name);
                    // `Blocks.wallVariant` reuses the standing block's loot table.
                    let short = short.replacen("_wall_", "_", 1);
                    let short = short.strip_prefix("wall_").unwrap_or(&short);
                    let short = short.replacen("wall_", "", 1);
                    let _ = writeln!(result, "D minecraft:blocks/{short}");
                }
                LevelEvent::FallingBlock { .. } => {
                    let _ = writeln!(result, "E minecraft:falling_block");
                }
                LevelEvent::PrimeTnt { .. } => {
                    let _ = writeln!(result, "E minecraft:tnt");
                }
                LevelEvent::BlockEvent {
                    kind: "spawn_item", ..
                } => {
                    let _ = writeln!(result, "E minecraft:item");
                }
                LevelEvent::GameEvent { pos, event, .. } => {
                    let _ = writeln!(result, "G {} {}", p(*pos), event);
                }
                // Dispensing from the fixture's empty block entities, as the
                // client does: the fail event, plus `block_activate` for a
                // dispenser; a crafter without a recipe only fails.
                LevelEvent::BlockEvent {
                    kind: "dispense",
                    pos,
                    state,
                } => match behaviours.table.name(*state) {
                    "minecraft:crafter" => {
                        let _ = writeln!(result, "L 1050 {} 0", p(*pos));
                    }
                    "minecraft:dispenser" => {
                        let _ = writeln!(result, "L 1001 {} 0", p(*pos));
                        let _ = writeln!(result, "G {} minecraft:block_activate", p(*pos));
                    }
                    _ => {
                        let _ = writeln!(result, "L 1001 {} 0", p(*pos));
                    }
                },
                // DriedGhastBlock.spawnGhastling, as the client does it.
                LevelEvent::BlockEvent {
                    kind: "spawn_ghastling",
                    ..
                } => {
                    let _ = writeln!(result, "E minecraft:happy_ghast");
                    let _ = writeln!(result, "S entity minecraft:entity.ghastling.spawn");
                }
                // A fresh shrieker has warning level 0 and so never responds,
                // and no player is near enough for a portal to spawn a piglin.
                // A fresh command block has no command and is neither powered
                // nor always active: `CommandBlock.tick` changes nothing. A
                // fresh suspicious block was never brushed, so
                // `BrushableBlockEntity.checkReset` has nothing to reset. A
                // fresh structure block has no structure name, so its trigger
                // neither saves nor loads. A test block's trigger and reset
                // live on its block entity: `game/test_blocks.rs` runs them.
                // `Block.pushEntitiesUp` lifts only the entities in the new
                // block's box, and these cases have none: nothing moves.
                LevelEvent::BlockEvent {
                    kind:
                        "shrieker_respond" | "portal_spawn" | "command_block" | "brushable"
                        | "structure_block" | "test_block" | "test_block_tick" | "push_entities_up",
                    ..
                } => {}
                LevelEvent::BlockEvent { kind, .. } => {
                    let _ = writeln!(result, "X {kind}");
                }
                // The client copy of a bell's ring: no level change.
                LevelEvent::BlockEntityEvent { .. } => {}
                LevelEvent::GrowTree { .. } => {
                    let _ = writeln!(result, "X grow_tree");
                }
                LevelEvent::PlaceFeature { feature, .. } => {
                    let _ = writeln!(result, "X feature {feature}");
                }
                LevelEvent::Player { .. } => {
                    let _ = writeln!(result, "X player");
                }
            }
        }
        for e in &ticks.block_events {
            let _ = writeln!(
                result,
                "B {} {} {} {}",
                p(e.pos),
                behaviours.table.info(e.block).name,
                e.param_a,
                e.param_b
            );
        }
        let _ = writeln!(result, "rng {}", level.random.next_long());
        out.write_all(result.as_bytes())?;
    }
    Ok(())
}
