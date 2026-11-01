//! The Rust side of tools/test_sculk_parity.py: with
//! `HORIZON_SCULK_FIXTURE=SCENARIOS,OUTPUT` the game loads its resources and
//! runs every scenario (sculk blocks, the player with a warning level, scripted
//! game events and block changes) through its own sculk code on a fresh
//! headless world, ticking in `ServerLevel.tick`'s order, and prints what
//! changed after every tick in the format of `VanillaSculkOracle`.
use super::{Game, dispense_fixture};
use horizon_client::{
    Result,
    chunk::{Chunk, MIN_Y},
    command::args::GameMode,
    containers::BlockEntity,
    mobs::{Mob, Who},
    motion::PlayerMotion,
    sculk::{Info, SculkKind, WardenSpawnTracker},
    world::World,
};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
    random::LegacyRandom,
};
use horizon_world::{chunk::BlockState, level::Ticks, registry::BlockStateId};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    io::Write as _,
    sync::Arc,
};
use winit::keyboard::KeyCode;

struct MobSpec {
    kind: String,
    pos: [f64; 3],
    yaw: f32,
    seed: i64,
    flags: Vec<String>,
}
#[derive(Default)]
struct Scenario {
    name: String,
    ticks: i32,
    origin: [i32; 2],
    level_seed: i64,
    difficulty: String,
    wardens: bool,
    blocks: Vec<Vec<String>>,
    player: Option<[f64; 3]>,
    mode: String,
    sneak: bool,
    warning: Option<i32>,
    mobs: Vec<MobSpec>,
    watch: Vec<[i32; 3]>,
    regions: Vec<[i32; 6]>,
    actions: Vec<(i32, Vec<String>)>,
}

fn parse(text: &str) -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut cur: Option<Scenario> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() || f[0].starts_with('#') {
            continue;
        }
        let s = cur.get_or_insert_with(Scenario::default);
        match f[0] {
            "scenario" => {
                s.name = f[1].to_owned();
                s.difficulty = "normal".into();
                s.wardens = true;
                s.mode = "survival".into();
                for kv in &f[2..] {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    match k {
                        "ticks" => s.ticks = v.parse().unwrap_or(0),
                        "levelseed" => s.level_seed = v.parse().unwrap_or(0),
                        "difficulty" => s.difficulty = v.to_owned(),
                        "nowardens" => s.wardens = false,
                        _ => {}
                    }
                }
            }
            "origin" => s.origin = [f[1].parse().unwrap_or(0), f[2].parse().unwrap_or(0)],
            "fill" | "block" => s.blocks.push(f.iter().map(|x| (*x).to_owned()).collect()),
            "player" => {
                s.player = Some([
                    f[1].parse().unwrap_or(0.0),
                    f[2].parse().unwrap_or(0.0),
                    f[3].parse().unwrap_or(0.0),
                ]);
                for w in &f[4..] {
                    if *w == "sneak" {
                        s.sneak = true;
                    } else if let Some(level) = w.strip_prefix("warn=") {
                        s.warning = level.parse().ok();
                    } else {
                        s.mode = (*w).to_owned();
                    }
                }
            }
            "mob" => s.mobs.push(MobSpec {
                kind: f[1].to_owned(),
                pos: [
                    f[2].parse().unwrap_or(0.0),
                    f[3].parse().unwrap_or(0.0),
                    f[4].parse().unwrap_or(0.0),
                ],
                yaw: f[5].parse().unwrap_or(0.0),
                seed: f[6].parse().unwrap_or(0),
                flags: f[7..].iter().map(|x| (*x).to_owned()).collect(),
            }),
            "watch" => s.watch.push([
                f[1].parse().unwrap_or(0),
                f[2].parse().unwrap_or(0),
                f[3].parse().unwrap_or(0),
            ]),
            "region" => {
                let n = |i: usize| f[i].parse::<i32>().unwrap_or(0);
                s.regions.push([n(1), n(2), n(3), n(4), n(5), n(6)]);
            }
            "at" => s.actions.push((
                f[1].parse().unwrap_or(0),
                f[2..].iter().map(|x| (*x).to_owned()).collect(),
            )),
            "end" => out.extend(cur.take()),
            _ => {}
        }
    }
    out
}

/// An entity's own random is unseeded in vanilla, and so are its sensors'
/// first delays and its brain's `RunOne` shuffles: the oracle seeds what
/// appeared, and so does this.
fn seed_fresh(m: &mut Mob, seed: i64) {
    m.random = LegacyRandom::new(seed);
    horizon_client::mobs::warden::init_brain(m, seed);
}
fn d(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn f(v: f32) -> String {
    format!("{:x}", v.to_bits())
}

/// Sounds the client plays for level events (3006 sculk charge, 3007
/// shriek) and for itself (the warden's heartbeat): vanilla's server never
/// sends them.
const CLIENT_SOUNDS: &[&str] = &[
    "minecraft:block.sculk_shrieker.shriek",
    "minecraft:block.sculk.charge",
    "minecraft:entity.warden.heartbeat",
];

impl Game<'_> {
    /// Run every scenario of `spec` ("SCENARIOS,OUTPUT") and write the results.
    pub(super) fn sculk_fixture(&mut self, spec: &str) -> Result<()> {
        assert!(
            !self.options.execution_role.is_client(),
            "fixture requires a server Game"
        );
        let (input, output) = spec
            .split_once(',')
            .ok_or("HORIZON_SCULK_FIXTURE=SCENARIOS,OUTPUT")?;
        let text = std::fs::read_to_string(input)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for s in parse(&text) {
            writeln!(out, "== {}", s.name)?;
            match self.sculk_scenario(&s) {
                Ok(lines) => out.write_all(lines.as_bytes())?,
                Err(e) => writeln!(out, "0 failed {e}")?,
            }
        }
        Ok(())
    }

    /// `minecraft:name[key=value,...]` to a state.
    fn fixture_state(&self, text: &str) -> Result<BlockStateId> {
        let (name, props) = match text.split_once('[') {
            Some((n, rest)) => (n, rest.trim_end_matches(']')),
            None => (text, ""),
        };
        let registry = self.pack.assets.registry;
        let base = registry
            .default_state(name)
            .ok_or_else(|| format!("unknown block {name}"))?;
        let mut properties: BTreeMap<String, String> = registry
            .state(base)
            .map(|s| s.properties.clone().into_iter().collect())
            .unwrap_or_default();
        for kv in props.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or("property")?;
            properties.insert(k.to_owned(), v.to_owned());
        }
        Ok(registry.resolve(&BlockState {
            name: name.to_owned(),
            properties,
        })?)
    }

    /// `BlockStateParser.serialize`.
    fn fixture_state_text(&self, state: BlockStateId) -> String {
        let Some(s) = self.pack.assets.registry.state(state) else {
            return "?".into();
        };
        if s.properties.is_empty() {
            return s.name.clone();
        }
        let mut sorted: Vec<(&String, &String)> = s.properties.iter().collect();
        sorted.sort();
        let props: Vec<String> = sorted.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{}[{}]", s.name, props.join(","))
    }

    fn fixture_who(&self, who: Option<Who>, names: &HashMap<u32, String>) -> String {
        match who {
            None => "none".into(),
            Some(Who::Player) => "player".into(),
            Some(Who::Mob(id)) => names.get(&id).cloned().unwrap_or_else(|| format!("e{id}")),
        }
    }

    fn fixture_info(&self, info: Option<&Info>, names: &HashMap<u32, String>) -> String {
        let Some(i) = info else {
            return "-".into();
        };
        // A source entity that is gone reads as none (VibrationInfo.getEntity).
        let alive = |w: Option<Who>| match w {
            Some(Who::Mob(id)) if !self.mobs.iter().any(|m| m.id == id && !m.removed) => None,
            other => other,
        };
        format!(
            "{}/{}/{},{},{}/{}/{}",
            i.event.trim_start_matches("minecraft:"),
            f(i.distance),
            d(i.pos[0]),
            d(i.pos[1]),
            d(i.pos[2]),
            self.fixture_who(alive(i.source), names),
            self.fixture_who(alive(i.owner), names)
        )
    }

    fn sculk_describe(&self, p: [i32; 3], names: &HashMap<u32, String>) -> String {
        let state = self.world.block(p[0], p[1], p[2]);
        let mut line = self.fixture_state_text(state);
        if let Some(BlockEntity::Sculk(e)) = self.world.block_entities.get(&(p[0], p[1], p[2])) {
            let vibration = |line: &mut String| {
                let v = &e.vibration;
                let _ = write!(
                    line,
                    " cur={} travel={}",
                    self.fixture_info(v.current.as_ref(), names),
                    v.travel
                );
                match v.selector.current() {
                    None => line.push_str(" sel=-"),
                    Some((info, tick)) => {
                        let _ = write!(
                            line,
                            " sel={}@{tick}",
                            self.fixture_info(Some(&info), names)
                        );
                    }
                }
            };
            match e.kind {
                SculkKind::Sensor | SculkKind::Calibrated => {
                    let _ = write!(line, " freq={}", e.last_frequency);
                    vibration(&mut line);
                }
                SculkKind::Shrieker => {
                    let _ = write!(line, " warn={}", e.warning_level);
                    vibration(&mut line);
                }
                SculkKind::Catalyst => {
                    let _ = write!(line, " cursors={}", e.spreader.cursors.len());
                    for c in &e.spreader.cursors {
                        let _ = write!(
                            line,
                            " {},{},{}:{}:{}:{}:",
                            c.pos.x, c.pos.y, c.pos.z, c.charge, c.update_delay, c.decay_delay
                        );
                        match &c.facings {
                            None => line.push('-'),
                            Some(faces) => {
                                let mut faces = faces.clone();
                                faces.sort_by_key(|d| *d as u8);
                                for d in faces {
                                    line.push(match d {
                                        Direction::Down => 'd',
                                        Direction::Up => 'u',
                                        Direction::North => 'n',
                                        Direction::South => 's',
                                        Direction::West => 'w',
                                        Direction::East => 'e',
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        line
    }

    fn sculk_fixture_reset(&mut self, s: &Scenario) -> Result<()> {
        self.world = World::new(self.res.facts().air, 0);
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        let layers = [
            self.fixture_state("minecraft:bedrock")?,
            self.fixture_state("minecraft:dirt")?,
            self.fixture_state("minecraft:dirt")?,
            self.fixture_state("minecraft:grass_block")?,
        ];
        let (ocx, ocz) = (s.origin[0] >> 4, s.origin[1] >> 4);
        for cx in ocx - 4..=ocx + 4 {
            for cz in ocz - 4..=ocz + 4 {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    Arc::make_mut(section).biomes = horizon_core::paletted::PalettedContainer::new(
                        horizon_client::chunk::BIOME_STRATEGY,
                        plains,
                    );
                }
                chunk.lit = true;
                self.world.chunks.insert((cx, cz), Arc::new(chunk));
                for x in 0..16 {
                    for z in 0..16 {
                        for (y, layer) in layers.iter().enumerate() {
                            self.world
                                .set(cx * 16 + x, MIN_Y + y as i32, cz * 16 + z, *layer)?;
                        }
                    }
                }
            }
        }
        self.block_ticks = Ticks::new(0);
        self.tick_chunks.clear();
        self.sync_tick_chunks();
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.level_events.clear();
        self.ui.rules = Default::default();
        for (rule, value) in [
            ("spawn_mobs", false),
            ("spawn_monsters", false),
            ("spawn_patrols", false),
            ("spawn_phantoms", false),
            ("spawn_wandering_traders", false),
            ("advance_time", false),
            ("advance_weather", false),
            ("raids", false),
            ("spawn_wardens", s.wardens),
        ] {
            self.ui
                .rules
                .set(rule, horizon_client::gamerules::Value::Bool(value));
        }
        self.ui.rules.set(
            "random_tick_speed",
            horizon_client::gamerules::Value::Int(0),
        );
        use horizon_world::food::Difficulty;
        let difficulty = match s.difficulty.as_str() {
            "peaceful" => Difficulty::Peaceful,
            "easy" => Difficulty::Easy,
            "hard" => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
        self.world.health_rules.difficulty = difficulty;
        self.ui.difficulty = match difficulty {
            Difficulty::Peaceful => 0,
            Difficulty::Easy => 1,
            Difficulty::Normal => 2,
            Difficulty::Hard => 3,
        };
        self.level.seed = 0;
        self.potions.effects = Default::default();
        self.sculk = Default::default();
        self.sculk.tracker_loaded = true;
        self.sculk_prepare();
        Ok(())
    }

    fn sculk_scenario(&mut self, s: &Scenario) -> Result<String> {
        self.sculk_fixture_reset(s)?;
        let at = s.player.unwrap_or([
            f64::from(s.origin[0]) + 0.5,
            100.0,
            f64::from(s.origin[1]) + 0.5,
        ]);
        self.mode = match s.mode.as_str() {
            "creative" => GameMode::Creative,
            "spectator" => GameMode::Spectator,
            "adventure" => GameMode::Adventure,
            _ => GameMode::Survival,
        };
        self.creative = self.mode == GameMode::Creative;
        let mut player = PlayerMotion::new(self.motion, Vec3::new(at[0], at[1], at[2]))?;
        player.spectator = self.mode == GameMode::Spectator;
        self.player = Some(player);
        self.keys.clear();
        if s.sneak {
            self.keys.insert(KeyCode::ShiftLeft);
        }
        self.sculk.tracker = WardenSpawnTracker::default();
        if let Some(level) = s.warning {
            self.sculk.tracker.set_warning_level(level);
        }
        // The blocks, in order (sculk block entities tick in the order they
        // were placed), with `Level.setBlock(pos, state, 2 | 16)` at game time 1000.
        self.world.game_time = 1000;
        let mut cells = Vec::new();
        for b in &s.blocks {
            let n = |i: usize| b[i].parse::<i32>().unwrap_or(0);
            if b[0] == "block" {
                cells.push((BlockPos::new(n(1), n(2), n(3)), self.fixture_state(&b[4])?));
            } else {
                let state = self.fixture_state(&b[7])?;
                for x in n(1).min(n(4))..=n(1).max(n(4)) {
                    for y in n(2).min(n(5))..=n(2).max(n(5)) {
                        for z in n(3).min(n(6))..=n(3).max(n(6)) {
                            cells.push((BlockPos::new(x, y, z), state));
                        }
                    }
                }
            }
        }
        self.with_level(|b, level, ticks| {
            for (pos, state) in cells {
                b.set_block(level, ticks, pos, state, 2 | 16);
            }
        });
        self.level_events.clear();
        self.sculk.known = self.world.chunks.keys().copied().collect();
        self.sculk.known_revision = Some(self.world.chunks.membership_revision());
        self.sculk_sync_index();
        let mut names: HashMap<u32, String> = HashMap::new();
        let mut sources: Vec<u32> = Vec::new();
        for spec in &s.mobs {
            let kind = self
                .res
                .mob_types()
                .get(&spec.kind)
                .cloned()
                .ok_or_else(|| format!("unknown mob {}", spec.kind))?;
            let id = 100_000 + sources.len() as u32;
            let mut m = Mob::new(
                id,
                kind,
                Vec3::new(spec.pos[0], spec.pos[1], spec.pos[2]),
                spec.yaw,
            );
            for flag in &spec.flags {
                match flag.as_str() {
                    "persistent" => m.persistent = true,
                    "noai" => m.no_ai = true,
                    _ => {}
                }
            }
            m.random = LegacyRandom::new(spec.seed);
            // EntityType.create equips nothing (that is finalizeSpawn's job).
            for slot in [
                &mut m.extra.main_hand,
                &mut m.extra.off_hand,
                &mut m.extra.head,
                &mut m.extra.chest,
                &mut m.extra.legs,
                &mut m.extra.feet,
            ] {
                *slot = None;
            }
            names.insert(id, format!("m{}", sources.len()));
            sources.push(id);
            self.mobs.push(m);
        }
        self.next_entity = self.next_entity.max(200_000);
        self.world.day_time = 6000;
        self.world.random = LegacyRandom::new(s.level_seed);
        self.audio.capture = Some(Vec::new());
        let mut known: HashSet<u32> = self.mobs.iter().map(|m| m.id).collect();
        let mut seeded: HashSet<u32> = HashSet::new();
        let mut known_items = self.dropped.len();
        let mut known_orbs = self.orbs.len();
        let mut last: HashMap<[i32; 3], String> = HashMap::new();
        let mut region_last: HashMap<[i32; 3], String> = HashMap::new();
        let (mut last_player, mut last_rng) = (String::new(), String::new());
        let mut out = String::new();
        for t in 0..=s.ticks {
            dispense_fixture::start_recording();
            if t > 0 {
                self.sculk_fixture_tick(t, s, &sources, &known, names.len(), &mut seeded)?;
            }
            let mut lines: Vec<String> = Vec::new();
            for p in &s.watch {
                let line = self.sculk_describe(*p, &names);
                if last.get(p) != Some(&line) {
                    last.insert(*p, line.clone());
                    lines.push(format!("blk {},{},{} {line}", p[0], p[1], p[2]));
                }
            }
            // Regions: every block whose state changed.
            for r in &s.regions {
                for x in r[0]..=r[3] {
                    for y in r[1]..=r[4] {
                        for z in r[2]..=r[5] {
                            let state = self.fixture_state_text(self.world.block(x, y, z));
                            let before = region_last.insert([x, y, z], state.clone());
                            if t > 0 && before.as_ref() != Some(&state) {
                                lines.push(format!("set {x},{y},{z} {state}"));
                            }
                        }
                    }
                }
            }
            let tr = self.sculk.tracker;
            let dark = self
                .potions
                .effects
                .get("darkness")
                .map_or("-".to_owned(), |e| e.duration.to_string());
            let now = format!(
                "warn {} {} dark {dark}",
                tr.warning_level, tr.cooldown_ticks
            );
            if now != last_player {
                last_player = now.clone();
                lines.push(now);
            }
            let mut fresh: Vec<&Mob> = self
                .mobs
                .iter()
                .filter(|m| !known.contains(&m.id))
                .collect();
            fresh.sort_by_key(|m| m.id);
            let fresh: Vec<(u32, String)> = fresh
                .iter()
                .map(|m| {
                    (
                        m.id,
                        format!(
                            "new {} {},{},{}",
                            m.kind.id,
                            d(m.position.x),
                            d(m.position.y),
                            d(m.position.z)
                        ),
                    )
                })
                .collect();
            for (id, line) in fresh {
                known.insert(id);
                names.insert(id, format!("n{}", names.len()));
                let seed = 0x5eed + names.len() as i64;
                if seeded.insert(id)
                    && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
                {
                    seed_fresh(m, seed);
                }
                lines.push(line);
            }
            for i in known_items..self.dropped.len() {
                let d = &self.dropped[i];
                names.insert(u32::MAX - i as u32, format!("n{}", names.len()));
                lines.push(format!(
                    "new minecraft:item {},{},{}",
                    self::d(d.position.x),
                    self::d(d.position.y),
                    self::d(d.position.z)
                ));
            }
            known_items = self.dropped.len();
            // Where an orb flies comes from its own random: its value only.
            for orb in &self.orbs[known_orbs.min(self.orbs.len())..] {
                lines.push(format!("new minecraft:experience_orb value={}", orb.value));
            }
            known_orbs = self.orbs.len();
            for line in dispense_fixture::stop_recording() {
                if let Some(rest) = line.strip_prefix("L ") {
                    lines.push(format!("level {rest}"));
                }
            }
            for (sound, at, volume, pitch) in
                self.audio.capture.replace(Vec::new()).unwrap_or_default()
            {
                if CLIENT_SOUNDS.contains(&sound.as_str()) {
                    continue;
                }
                lines.push(format!(
                    "sound {sound} {},{},{} {} {}",
                    at[0].floor() as i32,
                    at[1].floor() as i32,
                    at[2].floor() as i32,
                    f(volume),
                    f(pitch)
                ));
            }
            let rng = format!("rng {:x}", self.world.random.state());
            if rng != last_rng {
                last_rng = rng.clone();
                lines.push(rng);
            }
            for line in lines {
                let _ = writeln!(out, "{t} {line}");
            }
        }
        self.audio.capture = None;
        Ok(out)
    }

    /// One `ServerLevel.tick` of a scenario.
    fn sculk_fixture_tick(
        &mut self,
        t: i32,
        s: &Scenario,
        sources: &[u32],
        known: &HashSet<u32>,
        named: usize,
        seeded: &mut HashSet<u32>,
    ) -> Result<()> {
        self.world.game_time += 1;
        // Scheduled block and fluid ticks, then block events.
        self.tick_scheduled()?;
        self.with_level(|b, level, ticks| b.run_block_events(level, ticks));
        self.process_level_events()?;
        // The scenario's actions happen among the entities.
        for (tick, words) in &s.actions {
            if *tick != t {
                continue;
            }
            match words[0].as_str() {
                "event" => {
                    let event = horizon_client::sculk::intern(&words[1])
                        .ok_or_else(|| format!("unknown game event {}", words[1]))?;
                    let num = |i: usize| words[i].parse::<f64>().unwrap_or(0.0);
                    let pos = [num(2), num(3), num(4)];
                    let who = match words[5].as_str() {
                        "none" => None,
                        "player" => Some(Who::Player),
                        m => sources
                            .get(m[1..].parse::<usize>().unwrap_or(usize::MAX))
                            .map(|id| Who::Mob(*id)),
                    };
                    let state = match words.get(6) {
                        Some(text) => Some(self.fixture_state(text)?),
                        None => None,
                    };
                    let ctx = self.who_context(who, state);
                    self.post_game_event(event, pos, ctx);
                }
                "kill" => {
                    // LivingEntity.hurtServer with a player attack.
                    let id = sources
                        .get(words[1][1..].parse::<usize>().unwrap_or(usize::MAX))
                        .copied()
                        .ok_or("kill: no such mob")?;
                    let amount = words
                        .get(2)
                        .and_then(|a| a.parse::<f32>().ok())
                        .unwrap_or(1000.0);
                    let at = self
                        .player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z])
                        .unwrap_or_default();
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        m.damage(amount, horizon_client::mobs::Damage::player(at));
                    }
                    self.last_hurt_by_player.insert(id, self.lan_acting);
                }
                "set" => {
                    let n = |i: usize| words[i].parse::<i32>().unwrap_or(0);
                    let pos = BlockPos::new(n(1), n(2), n(3));
                    let state = self.fixture_state(&words[4])?;
                    self.with_level(|b, level, ticks| {
                        b.set_block(level, ticks, pos, state, 3);
                    });
                    self.process_level_events()?;
                }
                other => return Err(format!("unknown action {other}").into()),
            }
        }
        // Mobs that appeared during the block ticks (a summoned warden) tick
        // in this tick's entity phase: seed them now, as they are seeded
        // after the tick.
        let mut born: Vec<u32> = self
            .mobs
            .iter()
            .map(|m| m.id)
            .filter(|id| !known.contains(id))
            .collect();
        born.sort_unstable();
        for (i, id) in born.into_iter().enumerate() {
            if seeded.insert(id)
                && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                seed_fresh(m, 0x5eed + (named + i + 1) as i64);
            }
        }
        // The player's tickets, as `tick_world` takes them.
        self.update_chunk_tickets();
        // Entities: the player's warden spawn tracker, then the mobs.
        self.tick_sculk_entities();
        // (ServerChunkCache.tick, natural spawning and its chunk shuffle, is not run.)
        self.tick_creature_entities()?;
        self.tick_items();
        // Block entities.
        self.tick_sculk_blocks()?;
        Ok(())
    }
}
