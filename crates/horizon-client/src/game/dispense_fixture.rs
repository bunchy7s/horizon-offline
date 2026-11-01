//! The Rust side of tools/test_dispenser_parity.py: with
//! `HORIZON_DISPENSE_FIXTURE=CASES,OUTPUT` the game loads its resources, runs
//! every case (a dispenser or dropper with items, blocks and target entities
//! around it) through `dispense_from` on a fresh headless world and prints what
//! changed in the format of `VanillaDispenserOracle`.
use super::{
    Game,
    dispenser::{Slot, body_in_head, mob_set_slot, mob_slot_item},
};
use horizon_client::{
    Result,
    chunk::Chunk,
    command::args::GameMode,
    containers::BlockEntity,
    gamerules::Value as RuleValue,
    item_data::{canon_stack, parse_stack},
    items::ItemStack,
    mobs::{
        Mob, Species,
        spawner::{SpawnDataExt, SpawnerExt},
    },
    motion::PlayerMotion,
    projectiles::ProjectileKind,
    world::World,
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{level::Ticks, registry::BlockStateId};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, fmt::Write as _, io::Write as _, sync::Arc};
use winit::keyboard::KeyCode;

thread_local! {
    /// Level and game events recorded while a fixture case runs.
    static LOG: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// Record a line if a fixture case is running.
pub(super) fn record(line: impl FnOnce() -> String) {
    LOG.with(|log| {
        if let Some(lines) = log.borrow_mut().as_mut() {
            lines.push(line());
        }
    });
}
/// Start recording level and game event lines (another fixture's case).
pub(super) fn start_recording() {
    LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
}
/// The lines recorded since `start_recording`; recording stops.
pub(super) fn stop_recording() -> Vec<String> {
    LOG.with(|log| log.borrow_mut().take()).unwrap_or_default()
}
/// `Level.levelEvent` lines for the level events the behaviour library reported.
pub(super) fn record_events(events: &[horizon_world::level::LevelEvent]) {
    for event in events {
        if let horizon_world::level::LevelEvent::LevelEvent { id, pos, data } = event {
            record(|| format!("L {id} {},{},{} {data}", pos.x, pos.y, pos.z));
        }
    }
}

/// Java's `String.format("%.6f")`: the exact value rounded half up (away
/// from zero) where Rust rounds a tie to even.
fn num(v: f64) -> String {
    let exact = format!("{:.40}", v.abs());
    let tie = exact
        .split_once('.')
        .is_some_and(|(_, f)| f[6..].starts_with('5') && f[7..].bytes().all(|b| b == b'0'));
    if tie {
        format!("{:.6}", v + 0.5e-6_f64.copysign(v))
    } else {
        format!("{v:.6}")
    }
}
fn floor_pos(p: [f64; 3]) -> String {
    format!(
        "{},{},{}",
        p[0].floor() as i32,
        p[1].floor() as i32,
        p[2].floor() as i32
    )
}

/// The targets a case put in front of the dispenser.
enum Target {
    Mob(u32),
    Stand(u32),
}

impl Game<'_> {
    /// Run every case of `spec` ("CASES,OUTPUT") and write the results.
    pub(super) fn dispense_fixture(&mut self, spec: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "fixture requires a server Game"
        );
        let (input, output) = spec
            .split_once(',')
            .ok_or("HORIZON_DISPENSE_FIXTURE=CASES,OUTPUT")?;
        // Saplings and feature placers grow from the live feature registry.
        self.install_live_features();
        let text = std::fs::read_to_string(input)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for (index, line) in text.lines().enumerate() {
            let case: Value = serde_json::from_str(line)?;
            writeln!(out, "case {index}")?;
            match self.dispense_case(&case) {
                Ok(lines) => {
                    for l in lines {
                        writeln!(out, "{l}")?;
                    }
                }
                Err(e) => writeln!(out, "failed {e}")?,
            }
        }
        LOG.with(|log| *log.borrow_mut() = None);
        Ok(())
    }

    fn fixture_reset(&mut self, seed: i64) {
        self.world = World::new(self.res.facts().air, seed);
        // The oracle level is plains everywhere (bone meal's flowers, freezing).
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        for cx in -2..=2 {
            for cz in -2..=2 {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    Arc::make_mut(section).biomes = horizon_core::paletted::PalettedContainer::new(
                        horizon_client::chunk::BIOME_STRATEGY,
                        plains,
                    );
                }
                self.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        self.block_ticks = Ticks::new(0);
        self.tick_chunks.clear();
        self.sync_tick_chunks();
        self.mobs.clear();
        self.dropped.clear();
        self.creatures.projectiles.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.rockets.clear();
        self.potions.clear_thrown();
        self.level_events.clear();
        self.orbs.clear();
        self.falling.clear();
        // Hopper cooldowns of an earlier case.
        self.hoppers.clear();
        self.ui.rules = Default::default();
        self.dim = super::Dim::Overworld;
        self.respawn_config = None;
        // The oracle level plays on normal difficulty.
        self.ui.difficulty = 2;
        // The oracle level has world seed 0 and fresh random sequences.
        self.level.seed = 0;
        self.rand_sequences = Default::default();
    }

    fn dispense_case(&mut self, case: &Value) -> Result<Vec<String>> {
        let seed = case["seed"].as_i64().unwrap_or(0);
        self.fixture_reset(seed);
        // The oracle level answers one sky and one block light everywhere.
        let light = case["light"].as_array();
        let level = |i: usize, default: i64| {
            light
                .and_then(|l| l.get(i))
                .and_then(Value::as_i64)
                .unwrap_or(default) as u8
        };
        let packed = (level(0, 15) << 4) | level(1, 0);
        for chunk in self.world.chunks.values_mut() {
            for section in &mut Arc::make_mut(chunk).sections {
                Arc::make_mut(section).light = horizon_client::chunk::Light::Uniform(packed);
            }
        }
        self.world.game_time = case["time"].as_i64().unwrap_or(0);
        match case["dim"].as_str() {
            Some("nether") => self.dim = super::Dim::Nether,
            Some("end") => self.dim = super::Dim::End,
            _ => {}
        }
        // The level's build range follows its dimension, as `enter_dimension` sets it.
        self.world.shape = self.dim.shape();
        // The oracle level's clock stands at day time 6000.
        self.world.day_time = 6000;
        if let Some(rules) = case["rules"].as_object() {
            for (name, value) in rules {
                if let Some(b) = value.as_bool() {
                    self.ui.rules.set(name, RuleValue::Bool(b));
                }
            }
        }
        let mut before: HashMap<(i32, i32, i32), u32> = HashMap::new();
        for cell in case["cells"].as_array().into_iter().flatten() {
            let p = (
                cell[0].as_i64().unwrap_or(0) as i32,
                cell[1].as_i64().unwrap_or(0) as i32,
                cell[2].as_i64().unwrap_or(0) as i32,
            );
            let state = BlockStateId(cell[3].as_u64().unwrap_or(0) as u32);
            self.world.set(p.0, p.1, p.2, state)?;
            before.insert(p, state.0);
            if let Some(entity) = BlockEntity::for_block(&self.res.facts().get(state).name) {
                self.world.block_entities.insert(p, entity);
            }
        }
        for item in case["items"].as_array().into_iter().flatten() {
            let p = (
                item[0].as_i64().unwrap_or(0) as i32,
                item[1].as_i64().unwrap_or(0) as i32,
                item[2].as_i64().unwrap_or(0) as i32,
            );
            let slot = item[3].as_u64().unwrap_or(0) as usize;
            let stack = parse_stack(item[4].as_str().unwrap_or("-"))?;
            self.fixture_fill(p, slot, stack);
        }
        // CrafterBlock.newBlockEntity copies the state's triggered flag;
        // CrafterBlockEntity.setSlotState disables only empty slots.
        let crafter_positions: Vec<(i32, i32, i32)> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Crafter(_)))
            .map(|(p, _)| *p)
            .collect();
        for p in crafter_positions {
            let state = self.world.block(p.0, p.1, p.2);
            let triggered = self
                .pack
                .assets
                .registry
                .state(state)
                .and_then(|s| s.properties.get("triggered"))
                .is_some_and(|v| v == "true");
            if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&p) {
                c.triggered = triggered;
                c.state_triggered = Some(triggered);
            }
        }
        for d in case["disabled"].as_array().into_iter().flatten() {
            let p = (
                d[0].as_i64().unwrap_or(0) as i32,
                d[1].as_i64().unwrap_or(0) as i32,
                d[2].as_i64().unwrap_or(0) as i32,
            );
            if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&p) {
                c.set_slot_state(d[3].as_u64().unwrap_or(0) as usize, false);
            }
        }
        let mut targets = Vec::new();
        for e in case["entities"].as_array().into_iter().flatten() {
            targets.push(self.fixture_target(e)?);
        }
        // Leashable.setLeashedTo: a lead from a target to an earlier one.
        for (e, t) in case["entities"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(&targets)
        {
            let (Some(holder), Target::Mob(id)) = (e["leash"].as_u64(), t) else {
                continue;
            };
            let holder = match targets.get(holder as usize) {
                Some(Target::Mob(h)) => horizon_client::mobs::leash::Holder::Mob(*h),
                _ => continue,
            };
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == *id) {
                m.extra.leash = Some(holder);
            }
        }
        // Creating the targets must not move the stream the dispenser draws from.
        self.world.random = LegacyRandom::new(seed);
        self.world.block_log = Some(Vec::new());
        self.audio.capture = Some(Vec::new());
        LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
        let old_mobs: Vec<u32> = self.mobs.iter().map(|m| m.id).collect();
        let old_stands: Vec<u32> = self.armor_stands.iter().map(|s| s.id).collect();

        let pos = &case["pos"];
        let origin = BlockPos::new(
            pos[0].as_i64().unwrap_or(0) as i32,
            pos[1].as_i64().unwrap_or(0) as i32,
            pos[2].as_i64().unwrap_or(0) as i32,
        );
        let state = self.world.block(origin.x, origin.y, origin.z);
        let mut used = Vec::new();
        if let Some(op) = case.get("remove") {
            used = self.fixture_remove(origin, op)?;
        } else if let Some(op) = case.get("use") {
            used = self.fixture_use(origin, op)?;
        } else if let Some(op) = case.get("use_item") {
            used = self.fixture_use_item(origin, op)?;
        } else if let Some(op) = case.get("respawn_at") {
            // ServerPlayer.findRespawnAndUseSpawnBlock for a respawn point at the origin.
            let config = super::respawn_anchor::RespawnConfig::wrapped(
                self.dim,
                [origin.x, origin.y, origin.z],
                op["yaw"].as_f64().map_or(0.0, |v| v as f32),
                op["forced"].as_bool().unwrap_or(false),
            );
            let found =
                self.find_respawn_and_use(config, op["consume"].as_bool().unwrap_or(true))?;
            used.push(match found {
                Some((p, yaw)) => format!(
                    "respawn {} {} {} {}",
                    num(p.x),
                    num(p.y),
                    num(p.z),
                    yaw.to_bits() as i32
                ),
                None => "respawn none".to_owned(),
            });
        } else if let Some(op) = case.get("attack") {
            used = self.fixture_attack(op, &targets)?;
        } else if case.get("hopper").is_some() {
            // HopperBlockEntity.pushItemsTick, once.
            self.hopper_try_move_items((origin.x, origin.y, origin.z));
            self.process_level_events()?;
        } else {
            self.dispense_from(origin, state)?;
            // CrafterBlockEntity.serverTick, the crafting animation running down.
            let mut crafters: Vec<(i32, i32, i32)> = self
                .world
                .block_entities
                .iter()
                .filter(|(_, e)| matches!(e, BlockEntity::Crafter(_)))
                .map(|(p, _)| *p)
                .collect();
            crafters.sort_unstable();
            for _ in 0..case["ticks"].as_u64().unwrap_or(0) {
                for &p in &crafters {
                    self.tick_crafter(p);
                }
            }
        }

        let mut out = Vec::new();
        let mut changed: Vec<(i32, i32, i32)> = self.world.block_log.take().unwrap_or_default();
        changed.sort_unstable();
        changed.dedup();
        for p in changed {
            let now = self.world.block(p.0, p.1, p.2).0;
            if before.get(&p).copied().unwrap_or(self.res.facts().air.0) != now {
                out.push(format!("set {},{},{} {now}", p.0, p.1, p.2));
            }
        }
        let mut containers: Vec<_> = self.world.block_entities.keys().copied().collect();
        containers.sort_unstable();
        for p in containers {
            let entity = &self.world.block_entities[&p];
            let single = match entity {
                BlockEntity::Jukebox(j) => &j.items,
                _ => entity.slots(),
            };
            for (i, slot) in single.iter().enumerate() {
                if let Some(stack) = slot {
                    out.push(format!(
                        "inv {},{},{} {i} {}",
                        p.0,
                        p.1,
                        p.2,
                        canon_stack(Some(stack))
                    ));
                }
            }
        }
        let mut crafters: Vec<_> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(p, e)| match e {
                BlockEntity::Crafter(c) => Some((*p, c)),
                _ => None,
            })
            .collect();
        crafters.sort_unstable_by_key(|c| c.0);
        for (p, c) in crafters {
            let disabled: Vec<String> = (0..9)
                .filter(|&i| c.disabled[i])
                .map(|i| i.to_string())
                .collect();
            out.push(format!(
                "crafter {},{},{} ticks={} triggered={} signal={} disabled={}",
                p.0,
                p.1,
                p.2,
                c.crafting_ticks,
                c.triggered,
                c.redstone_signal(),
                disabled.join(",")
            ));
        }
        // A spawner's next spawn data (its id and number of keys), potentials and delay.
        let mut spawners: Vec<_> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(p, e)| match e {
                BlockEntity::Spawner(s) => Some((*p, s)),
                _ => None,
            })
            .collect();
        spawners.sort_unstable_by_key(|s| s.0);
        for (p, s) in spawners {
            let next = s.next_data().map_or("-".to_owned(), |d| {
                format!(
                    "{}/{}",
                    d.id().unwrap_or_else(|| "?".into()),
                    d.entity.len()
                )
            });
            out.push(format!(
                "spawner {},{},{} next={next} potentials={} delay={}",
                p.0,
                p.1,
                p.2,
                s.potentials.len(),
                s.spawn_delay as i16
            ));
        }
        self.fixture_entities(&old_mobs, &old_stands, &mut out);
        for (i, t) in targets.iter().enumerate() {
            out.push(self.fixture_describe(i, t));
        }
        let log = LOG.with(|log| log.borrow_mut().take()).unwrap_or_default();
        out.extend(log);
        for (sound, at, _, _) in self.audio.capture.take().unwrap_or_default() {
            out.push(format!("S {} {sound}", floor_pos(at)));
        }
        out.extend(used);
        out.push(format!("rng {}", self.world.random.next_long()));
        Ok(out)
    }

    /// A player holding the case's item right-clicks the block at `pos`
    /// (`use_item_as_player`'s block steps): the statistics, the result
    /// and the held stack, as `VanillaDispenserOracle.use` prints them.
    fn fixture_use(&mut self, pos: BlockPos, op: &Value) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.creative = op["creative"].as_bool().unwrap_or(false);
        self.inventory[self.selected] = stack.clone();
        let used_before = stack
            .as_ref()
            .map_or(0, |s| self.stats.get("used", &s.item));
        let at = [pos.x, pos.y, pos.z];
        let mut consumed = self.use_composter(at, stack.as_ref());
        if !consumed {
            let player = horizon_world::level::PlayerContext {
                yaw: 0.0,
                pitch: 90.0,
                may_build: true,
                can_eat: self.creative,
                game_master: false,
            };
            let hit = horizon_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 0.5,
            );
            let result = self.with_level(|b, level, ticks| {
                b.use_without_item(
                    level,
                    ticks,
                    pos,
                    horizon_core::geometry::Direction::Up,
                    hit,
                    player,
                )
            });
            consumed = result.consumes_action();
        }
        self.process_level_events()?;
        let mut out = Vec::new();
        if let Some(s) = &stack {
            let used = self.stats.get("used", &s.item) - used_before;
            if used != 0 {
                out.push(format!("stat {} minecraft:used {used}", s.item));
            }
        }
        out.push(format!(
            "use {} held {}",
            if consumed { "consumed" } else { "pass" },
            canon_stack(self.inventory[self.selected].as_ref())
        ));
        Ok(out)
    }

    /// A player three blocks south of `pos`, turned to the case's `yaw`,
    /// right-clicks its top face (or the case's `face`) holding
    /// the case's item (`use_item_as_player` on that hit): the statistics,
    /// the held stack and the item it started using, as
    /// `VanillaDispenserOracle.useItem` prints them.
    fn fixture_use_item(&mut self, pos: BlockPos, op: &Value) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.mode = if op["creative"].as_bool().unwrap_or(false) {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.creative = self.mode == GameMode::Creative;
        let distance = op["distance"].as_f64().unwrap_or(3.0);
        let mut player = PlayerMotion::new(
            self.motion,
            horizon_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 0.5 + distance,
            ),
        )?;
        if let Some(r) = op["respawn"].as_array() {
            let p = |i: usize| r.get(i).and_then(Value::as_i64).unwrap_or(0) as i32;
            self.respawn_config = Some(super::respawn_anchor::RespawnConfig {
                dim: self.dim,
                pos: [p(0), p(1), p(2)],
                yaw: 0.0,
                forced: false,
            });
        }
        player.health.food.food = op["food"].as_i64().unwrap_or(20) as i32;
        self.player = Some(player);
        self.yaw = op["yaw"].as_f64().unwrap_or(0.0) as f32;
        self.pitch = 90.0;
        self.keys.clear();
        if op["sneaking"].as_bool().unwrap_or(false) {
            self.keys.insert(KeyCode::ShiftLeft);
        }
        self.using = None;
        self.inventory[self.selected] = stack.clone();
        self.offhand = parse_stack(op["offhand"].as_str().unwrap_or("-"))?;
        // The used statistic of each item held, once.
        let mut items: Vec<_> = [stack.as_ref(), self.offhand.as_ref()]
            .into_iter()
            .flatten()
            .map(|s| s.item.clone())
            .collect();
        items.dedup();
        let used_before: Vec<_> = items.iter().map(|i| self.stats.get("used", i)).collect();
        // The top face, or the case's `face`, clicked at its centre.
        let face = match op["face"].as_str() {
            Some("down") => [0, -1, 0],
            Some("north") => [0, 0, -1],
            Some("south") => [0, 0, 1],
            Some("west") => [-1, 0, 0],
            Some("east") => [1, 0, 0],
            _ => [0, 1, 0],
        };
        let centre = |c: i32, f: i32| f64::from(c) + 0.5 + f64::from(f) * 0.5;
        self.forced_hit = Some(super::Hit {
            pos: [pos.x, pos.y, pos.z],
            face,
            point: [
                centre(pos.x, face[0]),
                centre(pos.y, face[1]),
                centre(pos.z, face[2]),
            ],
        });
        self.chat_capture = Some(Vec::new());
        let used = self.use_item_as_player();
        self.forced_hit = None;
        let messages = self.chat_capture.take().unwrap_or_default();
        used?;
        self.process_level_events()?;
        let mut out = Vec::new();
        for m in messages {
            let key = m.component()["translate"]
                .as_str()
                .map_or_else(|| m.plain(), str::to_owned);
            // The oracle player's advancements are not tracked.
            if !key.starts_with("chat.type.advancement") {
                out.push(format!("msg {key}"));
            }
        }
        for (item, before) in items.iter().zip(used_before) {
            let used = self.stats.get("used", item) - before;
            if used != 0 {
                out.push(format!("stat {item} minecraft:used {used}"));
            }
        }
        let using = self
            .using
            .and_then(|(slot, _)| self.stack_in(slot))
            .map_or("-".to_owned(), |s| s.item.to_string());
        out.push(format!(
            "held {} offhand {} using {using}",
            canon_stack(self.inventory[self.selected].as_ref()),
            canon_stack(self.offhand.as_ref())
        ));
        out.push(match self.respawn_config {
            Some(c) => format!(
                "respawn {} {},{},{}",
                c.dim.id(),
                c.pos[0],
                c.pos[1],
                c.pos[2]
            ),
            None => "respawn -".to_owned(),
        });
        self.player = None;
        self.using = None;
        self.offhand = None;
        Ok(out)
    }

    /// The player's hit on the first target with the case's item, from the
    /// attack strength's scaled `base` damage on: the item's damage bonus and
    /// source, the hurt with the weapon's armor effects, then `weapon_hit`
    /// (the mace's smash, post-attack enchantments, the wear), as
    /// `VanillaDispenserOracle.attack` prints them.
    fn fixture_attack(&mut self, op: &Value, targets: &[Target]) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.mode = if op["creative"].as_bool().unwrap_or(false) {
            GameMode::Creative
        } else {
            GameMode::Survival
        };
        self.creative = self.mode == GameMode::Creative;
        let v3 = |v: &Value| {
            horizon_core::geometry::Vec3::new(
                v[0].as_f64().unwrap_or(0.0),
                v[1].as_f64().unwrap_or(0.0),
                v[2].as_f64().unwrap_or(0.0),
            )
        };
        let mut player = PlayerMotion::new(self.motion, v3(&op["player"]))?;
        player.velocity = v3(&op["motion"]);
        player.fall_distance = op["fall"].as_f64().unwrap_or(0.0);
        player.fall_flying = op["fall_flying"].as_bool().unwrap_or(false);
        player.flying = op["flying"].as_bool().unwrap_or(false);
        player.may_fly = self.creative;
        if !op["impact"].is_null() {
            player.impulse.impact_position = Some(v3(&op["impact"]));
        }
        let feet = [player.position.x, player.position.y, player.position.z];
        self.player = Some(player);
        self.inventory[self.selected] = stack.clone();
        let used_before = stack
            .as_ref()
            .map_or(0, |s| self.stats.get("used", &s.item));
        let base = op["base"].as_f64().unwrap_or(0.0) as f32;
        let source = self.attack_source(feet);
        let (bonus, hurt) = match targets.first() {
            Some(&Target::Mob(id)) => {
                let index = self
                    .mobs
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or("the target is gone")?;
                let kind = self.mobs[index].kind.id.clone();
                let bonus = self.smash_damage_bonus(&kind);
                let hurt = self.hurt_by_player(index, base + bonus, source);
                // The hurt sound the mob raised, where it stands.
                for event in std::mem::take(&mut self.mobs[index].outbox) {
                    if let horizon_client::mobs::MobEvent::Sound(s, at, volume, pitch) = event {
                        self.audio
                            .play(&format!("minecraft:{s}"), Some(at), volume, pitch);
                    }
                }
                if hurt {
                    self.weapon_hit(index);
                }
                (bonus, hurt)
            }
            Some(&Target::Stand(id)) => {
                let index = self
                    .armor_stands
                    .iter()
                    .position(|s| s.id == id)
                    .ok_or("the target is gone")?;
                let bonus = self.smash_damage_bonus("minecraft:armor_stand");
                (bonus, self.hit_armor_stand(index, |_, _, _| {}))
            }
            None => return Err("an attack case has a target".into()),
        };
        self.process_level_events()?;
        let mut out = Vec::new();
        if let Some(s) = &stack {
            let used = self.stats.get("used", &s.item) - used_before;
            if used != 0 {
                out.push(format!("stat {} minecraft:used {used}", s.item));
            }
        }
        let source = match source.kind {
            horizon_client::mobs::DamageKind::MaceSmash => "minecraft:mace_smash",
            horizon_client::mobs::DamageKind::Spear => "minecraft:spear",
            _ => "minecraft:player_attack",
        };
        out.push(format!(
            "attack bonus={} source={source} hurt={hurt}",
            num(f64::from(bonus))
        ));
        for (i, t) in targets.iter().enumerate() {
            let (health, v) = match *t {
                Target::Mob(id) => match self.mobs.iter().find(|m| m.id == id) {
                    Some(m) => (num(f64::from(m.health)), m.velocity),
                    None => continue,
                },
                Target::Stand(id) => match self.armor_stands.iter().find(|s| s.id == id) {
                    Some(s) => ("-".to_owned(), s.velocity),
                    None => {
                        out.push(format!("V {i} gone"));
                        continue;
                    }
                },
            };
            out.push(format!(
                "V {i} {health} {} {} {}",
                num(v.x),
                num(v.y),
                num(v.z)
            ));
        }
        if let Some(p) = self.player.take() {
            let impact = p.impulse.impact_position.map_or("-".to_owned(), |i| {
                format!("{},{},{}", num(i.x), num(i.y), num(i.z))
            });
            out.push(format!(
                "P {} {} {} fall={} impact={impact} grace={} extra={}",
                num(p.velocity.x),
                num(p.velocity.y),
                num(p.velocity.z),
                num(p.fall_distance),
                p.impulse.reset_grace_time,
                p.spawn_extra_particles_on_fall
            ));
        }
        out.push(format!(
            "held {}",
            canon_stack(self.inventory[self.selected].as_ref())
        ));
        Ok(out)
    }

    /// A stack in a block entity's slot: a container's, a jukebox's record, a
    /// lectern's book, or the items of a block entity Horizon keeps as saved.
    fn fixture_fill(&mut self, p: (i32, i32, i32), slot: usize, stack: Option<ItemStack>) {
        let jukebox =
            &*self.res.facts().get(self.world.block(p.0, p.1, p.2)).name == "minecraft:jukebox";
        match self.world.block_entities.get_mut(&p) {
            Some(BlockEntity::Jukebox(j)) => j.items[0] = stack,
            Some(BlockEntity::Lectern(l)) => l.book = stack,
            Some(BlockEntity::Campfire(c)) => {
                if let Some(cell) = c.items.get_mut(slot) {
                    *cell = stack;
                }
            }
            Some(e) => {
                if let Some(cell) = e.slots_mut().get_mut(slot) {
                    *cell = stack;
                }
            }
            None if jukebox => {
                // JukeboxBlockEntity holding its record (setSongItemWithoutPlaying).
                self.world.block_entities.insert(
                    p,
                    BlockEntity::Jukebox(horizon_client::containers::Jukebox {
                        items: [stack],
                        ticks: 0,
                        playing: false,
                    }),
                );
            }
            None => self.fixture_fill_saved(p, slot, stack),
        }
    }
    /// The saved block entity of a campfire, decorated pot, chiseled
    /// bookshelf or shelf, as a loaded world keeps it.
    fn fixture_fill_saved(&mut self, p: (i32, i32, i32), slot: usize, stack: Option<ItemStack>) {
        use horizon_world::nbt::Tag;
        let name = self
            .res
            .facts()
            .get(self.world.block(p.0, p.1, p.2))
            .name
            .clone();
        let id = match name.strip_prefix("minecraft:").unwrap_or(&name) {
            "campfire" | "soul_campfire" => "minecraft:campfire",
            "decorated_pot" => "minecraft:decorated_pot",
            "chiseled_bookshelf" => "minecraft:chiseled_bookshelf",
            n if n.ends_with("_shelf") => "minecraft:shelf",
            _ => return,
        };
        let Some(chunk) = self.world.chunks.get_mut(&(p.0 >> 4, p.2 >> 4)) else {
            return;
        };
        let chunk = Arc::make_mut(chunk);
        let saved = Arc::make_mut(chunk.saved.get_or_insert_with(Default::default));
        let at = |t: &Tag| {
            t.get("x") == Some(&Tag::Int(p.0))
                && t.get("y") == Some(&Tag::Int(p.1))
                && t.get("z") == Some(&Tag::Int(p.2))
        };
        let index = match saved.block_entities.iter().position(at) {
            Some(i) => i,
            None => {
                saved.block_entities.push(Tag::compound([
                    ("id", Tag::String(id.into())),
                    ("x", Tag::Int(p.0)),
                    ("y", Tag::Int(p.1)),
                    ("z", Tag::Int(p.2)),
                ]));
                saved.block_entities.len() - 1
            }
        };
        let Tag::Compound(map) = &mut saved.block_entities[index] else {
            return;
        };
        let Some(stack) = stack else { return };
        if id == "minecraft:decorated_pot" {
            map.insert("item".into(), horizon_client::save::item_to_nbt(&stack));
            return;
        }
        let mut item = horizon_client::save::item_to_nbt(&stack);
        if let Tag::Compound(c) = &mut item {
            c.insert("Slot".into(), Tag::Byte(slot as i8));
        }
        match map
            .entry("Items".into())
            .or_insert_with(|| Tag::List(Vec::new()))
        {
            Tag::List(list) => list.push(item),
            other => *other = Tag::List(vec![item]),
        }
    }
    /// `Level.setBlock`, `destroyBlock` or `removeBlock` on the case's block, or
    /// a player mining it (`fixture_mine`).
    fn fixture_remove(&mut self, origin: BlockPos, op: &Value) -> Result<Vec<String>> {
        match op["op"].as_str().unwrap_or("") {
            "set" => {
                let state = BlockStateId(op["state"].as_u64().unwrap_or(0) as u32);
                let flags = op["flags"].as_i64().unwrap_or(3) as i32;
                self.with_level(|b, level, ticks| b.set_block(level, ticks, origin, state, flags));
            }
            "destroy" => {
                let drop = op["drop"].as_bool().unwrap_or(true);
                self.with_level(|b, level, ticks| b.destroy_block(level, ticks, origin, drop));
            }
            "remove" => {
                self.with_level(|b, level, ticks| b.remove_block(level, ticks, origin, false));
            }
            "mine" => return self.fixture_mine(origin, op),
            other => return Err(format!("unknown removal {other}").into()),
        }
        self.process_level_events()?;
        Ok(Vec::new())
    }

    /// A survival player three blocks south of `pos` mines it holding the
    /// case's item (`break_block`, which every player's mining goes through):
    /// the statistics and the held stack, as `VanillaDispenserOracle.mine`
    /// prints them.
    fn fixture_mine(&mut self, pos: BlockPos, op: &Value) -> Result<Vec<String>> {
        let stack = parse_stack(op["item"].as_str().unwrap_or("-"))?;
        self.mode = GameMode::Survival;
        self.creative = false;
        self.player = Some(PlayerMotion::new(
            self.motion,
            horizon_core::geometry::Vec3::new(
                f64::from(pos.x) + 0.5,
                f64::from(pos.y) + 1.0,
                f64::from(pos.z) + 3.5,
            ),
        )?);
        self.yaw = 0.0;
        self.pitch = 90.0;
        self.keys.clear();
        self.inventory[self.selected] = stack.clone();
        self.offhand = None;
        let state = self.world.block(pos.x, pos.y, pos.z);
        let block = self.res.facts().get(state).name.clone();
        let item = stack.as_ref().map(|s| s.item.clone());
        let before = |game: &Self| {
            let used = item.as_ref().map_or(0, |i| game.stats.get("used", i));
            (used, game.stats.get("mined", &block))
        };
        let (used, mined) = before(self);
        self.break_block([pos.x, pos.y, pos.z])?;
        let (used_after, mined_after) = before(self);
        let mut out = Vec::new();
        if let Some(item) = &item
            && used_after != used
        {
            out.push(format!("stat {item} minecraft:used {}", used_after - used));
        }
        if mined_after != mined {
            out.push(format!(
                "stat {block} minecraft:mined {}",
                mined_after - mined
            ));
        }
        out.push(format!(
            "mine held {}",
            canon_stack(self.inventory[self.selected].as_ref())
        ));
        Ok(out)
    }

    /// `EntityType.create` and `snapTo` for one target: no spawn finalization.
    fn fixture_target(&mut self, e: &Value) -> Result<Target> {
        let kind = e["type"].as_str().unwrap_or("minecraft:pig");
        let p = &e["pos"];
        let at = horizon_core::geometry::Vec3::new(
            p[0].as_f64().unwrap_or(0.0),
            p[1].as_f64().unwrap_or(0.0),
            p[2].as_f64().unwrap_or(0.0),
        );
        let yaw = e["yaw"].as_f64().unwrap_or(0.0) as f32;
        self.next_entity += 1;
        let id = self.next_entity;
        if kind == "minecraft:armor_stand" {
            let mut stand = horizon_client::entities::ArmorStand::new(id, at, yaw);
            stand.marker = e["marker"].as_bool().unwrap_or(false);
            stand.on_ground = e["ground"].as_bool().unwrap_or(false);
            if let Some(last_hit) = e["last_hit"].as_i64() {
                stand.last_hit = last_hit;
            }
            self.armor_stands.push(stand);
            return Ok(Target::Stand(id));
        }
        let t = self
            .res
            .mob_types()
            .get(kind)
            .cloned()
            .ok_or_else(|| format!("no mob type {kind}"))?;
        let mut mob = Mob::new(id, t, at, yaw);
        // EntityType.create equips nothing (that is finalizeSpawn's job).
        for slot in [
            &mut mob.extra.main_hand,
            &mut mob.extra.off_hand,
            &mut mob.extra.head,
            &mut mob.extra.chest,
            &mut mob.extra.legs,
            &mut mob.extra.feet,
        ] {
            *slot = None;
        }
        let s = mob.species();
        let flag = |name: &str| e[name].as_bool().unwrap_or(false);
        let ageable = !matches!(
            s,
            Species::Zombie | Species::Skeleton | Species::Bogged | Species::SnowGolem
        );
        if flag("baby") && ageable {
            mob.extra.age = -24000;
        }
        let tamable =
            body_in_head(s) || matches!(s, Species::Cat | Species::Parrot | Species::Ocelot);
        if flag("tame") && tamable {
            mob.extra.tame = true;
        }
        if flag("sheared") && matches!(s, Species::Sheep | Species::Bogged | Species::SnowGolem) {
            mob.extra.sheared = true;
        }
        if flag("chest")
            && matches!(
                s,
                Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
            )
        {
            mob.extra.chested = true;
        }
        if flag("loot") {
            mob.extra.pick_up_loot = true;
        }
        mob.on_ground = flag("ground");
        if let Some(health) = e["health"].as_f64() {
            mob.attrs.max_health = health as f32;
            mob.health = health as f32;
        }
        if let Some(resistance) = e["knockback_resistance"].as_f64() {
            mob.attrs.knockback_resistance = resistance;
        }
        if flag("brown") && s == Species::Mooshroom {
            mob.extra.variant = 1;
        }
        for (slot, snbt) in e["equip"].as_object().into_iter().flatten() {
            if let (Some(slot), Some(stack)) = (
                Slot::from_name(slot),
                parse_stack(snbt.as_str().unwrap_or("-"))?,
            ) {
                mob_set_slot(&mut mob, slot, stack);
            }
        }
        self.mobs.push(mob);
        Ok(Target::Mob(id))
    }

    /// `T` lines: removal, equipment by `EquipmentSlot`, shearing readiness, chests.
    fn fixture_describe(&self, index: usize, target: &Target) -> String {
        let mut line = format!("T {index} ");
        match *target {
            Target::Stand(id) => {
                line.push_str("minecraft:armor_stand");
                let Some(stand) = self.armor_stands.iter().find(|s| s.id == id) else {
                    line.push_str(" removed");
                    return line;
                };
                for (slot, name) in [
                    (0, "mainhand"),
                    (1, "offhand"),
                    (2, "feet"),
                    (3, "legs"),
                    (4, "chest"),
                    (5, "head"),
                ] {
                    if let Some(s) = &stand.equipment[slot] {
                        let _ = write!(line, " {name}={}x{}", s.item, s.count);
                    }
                }
            }
            Target::Mob(id) => {
                let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
                    return line + "removed";
                };
                line.push_str(&m.kind.id);
                if m.removed || !m.alive() {
                    line.push_str(" removed");
                }
                let s = m.species();
                let x = &m.extra;
                for (slot, name) in [
                    (Slot::MainHand, "mainhand"),
                    (Slot::OffHand, "offhand"),
                    (Slot::Feet, "feet"),
                    (Slot::Legs, "legs"),
                    (Slot::Chest, "chest"),
                    (Slot::Head, "head"),
                    (Slot::Body, "body"),
                    (Slot::Saddle, "saddle"),
                ] {
                    if let Some(item) = mob_slot_item(m, slot) {
                        let _ = write!(line, " {name}={item}x1");
                    }
                }
                let ready =
                    match s {
                        Species::Sheep => Some(!x.sheared && !m.is_baby()),
                        Species::Mooshroom => Some(!m.is_baby()),
                        Species::Bogged => Some(!x.sheared),
                        Species::SnowGolem => Some(!x.sheared),
                        Species::CopperGolem => Some(self.golem_shearable(
                            self.mobs.iter().position(|o| o.id == id).unwrap_or(0),
                        )),
                        _ => None,
                    };
                if let Some(r) = ready {
                    line.push_str(if r { " ready" } else { " unready" });
                }
                if x.chested
                    && matches!(
                        s,
                        Species::Donkey | Species::Mule | Species::Llama | Species::TraderLlama
                    )
                {
                    line.push_str(" chest");
                }
                if x.leash.is_some() {
                    line.push_str(" leashed");
                }
            }
        }
        line
    }

    /// `N` lines for every entity the dispenser added.
    fn fixture_entities(&self, old_mobs: &[u32], old_stands: &[u32], out: &mut Vec<String>) {
        let line = |kind: &str, p: [f64; 3], v: [f64; 3], extra: &str| {
            let mut l = format!(
                "N {kind} {} {} {} {} {} {}",
                num(p[0]),
                num(p[1]),
                num(p[2]),
                num(v[0]),
                num(v[1]),
                num(v[2])
            );
            if !extra.is_empty() {
                l.push(' ');
                l.push_str(extra);
            }
            l
        };
        for m in self.mobs.iter().filter(|m| !old_mobs.contains(&m.id)) {
            let (p, v) = (m.position, m.velocity);
            out.push(line(&m.kind.id, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for s in self
            .armor_stands
            .iter()
            .filter(|s| !old_stands.contains(&s.id))
        {
            let (p, v) = (s.position, s.velocity);
            out.push(line(
                "minecraft:armor_stand",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                "",
            ));
        }
        for d in &self.dropped {
            let (p, v) = (d.position, d.velocity);
            out.push(line(
                "minecraft:item",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                &canon_stack(Some(&d.stack)),
            ));
        }
        for shot in &self.creatures.projectiles {
            let kind = match &shot.kind {
                ProjectileKind::Arrow {
                    effect: Some(("minecraft:glowing", 200)),
                    ..
                } => "minecraft:spectral_arrow",
                ProjectileKind::Arrow { .. } => "minecraft:arrow",
                ProjectileKind::Snowball => "minecraft:snowball",
                ProjectileKind::Egg => "minecraft:egg",
                ProjectileKind::SmallFireball => "minecraft:small_fireball",
                ProjectileKind::WindCharge => "minecraft:wind_charge",
                _ => "minecraft:unknown_projectile",
            };
            let (p, v) = (shot.position, shot.velocity);
            out.push(line(kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for (kind, p, v) in self.potions.thrown_entities() {
            out.push(line(kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        for (p, v) in self.rockets.iter().map(|r| r.motion()) {
            out.push(line(
                "minecraft:firework_rocket",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                "",
            ));
        }
        for t in &self.tnts {
            let (p, v) = (t.position, t.velocity);
            out.push(line(
                "minecraft:tnt",
                [p.x, p.y, p.z],
                [v.x, v.y, v.z],
                &format!("fuse={}", t.fuse),
            ));
        }
        for b in &self.boats {
            let kind = b.entity_type();
            let (p, v) = (b.position, b.velocity);
            out.push(line(&kind, [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
        // A merged orb prints once per count, as the oracle does: whether orbs
        // merge depends on entity ids, which differ from vanilla's.
        for o in &self.orbs {
            let (p, v) = (o.position, o.velocity);
            for _ in 0..o.count.max(1) {
                out.push(line(
                    "minecraft:experience_orb",
                    [p.x, p.y, p.z],
                    [v.x, v.y, v.z],
                    "",
                ));
            }
        }
        for c in &self.carts {
            let (p, v) = (c.position, c.velocity);
            out.push(line(c.kind.id(), [p.x, p.y, p.z], [v.x, v.y, v.z], ""));
        }
    }
}
