//! `horizon-bots`: load-test bots for any Minecraft Java Edition 26.3 server, vanilla, Paper or Horizon
//! (docs/server.md, section 9). Each bot logs in offline, goes through configuration as vanilla's client
//! does, answers keep-alives and pings, accepts teleports, acknowledges chunk batches, and moves at 20 Hz.
//!
//! Bots fly a little above the ground, so the server must give them creative mode (`gamemode=creative`
//! and `force-gamemode=true`): they never touch blocks, so they need no world of their own, and no server
//! can refuse their moves for walking into terrain. Every behaviour is deterministic from `--seed`.
//!
//! `--behaviour fight` is the exception: the bots play in survival (`gamemode=survival`, `force-gamemode=true`,
//! and `allow-flight=true` because they hold their spawn height instead of standing on terrain they cannot
//! see). They track the entities the server sends, walk to the nearest living one and attack it at vanilla's
//! bare-hand cooldown (every 5 ticks). With `--pvp true` (the default) the other bots are targets too, which
//! hurts only when the server has `pvp=true`.
//!
//!   horizon-bots --server 127.0.0.1:25565 --bots 25 --behaviour mixed --seconds 360 --report bots.json
//!
//! It prints one JSON line a second (bots online, chunks received) and writes a summary with `--report`.
//! A bot moves only while it holds the chunk it stands in: vanilla's client stops ticking its player in a chunk
//! it hasn't received, so a player can't outrun the server's chunk sending, on any server.
//! Each bot also keeps the chunks it holds, as a client's chunk cache does, so the report tells chunks that
//! entered the view apart from chunks sent again while the bot still held them (`resent`), and counts the
//! times the server pulled a bot back (`pulled_back`: a teleport after the first, as for "moved too quickly").

//! The report also has wire bytes in and out (`bytes_in`, `bytes_out`, per bot and in total, after
//! compression) and join-time percentiles (`join_time`: p50, p99 and max in milliseconds).

use horizon_protocol::{
    PROTOCOL_VERSION,
    buf::{Reader, Writer},
    connection::{ConnWriter, Connection, Counters},
    crypto::offline_uuid,
    entity_data, ids,
    item::Stack,
    packets::{
        Empty, configuration,
        handshake::{self, Intention},
        login::{self, ServerHello},
        play, recipes,
    },
};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const HELP: &str = "\
horizon-bots: load-test bots for a Minecraft Java Edition 26.3 server (offline mode, creative)

  --server HOST:PORT     default 127.0.0.1:25565
  --bots N               default 10
  --behaviour B          idle, walk, explore, fight or mixed (60 % walk, 20 % explore, 20 % idle); default mixed.
                         fight needs gamemode=survival, force-gamemode=true and allow-flight=true.
                         stand: stays on the ground where the server puts it (/tp), for a survival test.
                         One abusive client, to test a server's limits (run it beside a normal crowd):
                         flood-move (--flood moves a tick, each 64 to 256 blocks away, ignoring chunks),
                         flood-acks (--flood chunk batch acks asking for 64 chunks a tick, while exploring),
                         flood-packets (--flood rotations a tick),
                         flood-chunks (one move a tick, 9.9 blocks straight ahead: as fast as the
                         moved-too-quickly check allows, never waiting for chunks),
                         and --flood a tick of packets that cost the server work:
                         flood-use (place and break stone around itself), flood-click (inventory
                         clicks with a stale state), flood-suggest (command suggestions, every
                         other one 30 000 characters long), flood-book (100 pages of 1024 characters
                         into a writable book), flood-sign (four 384-character lines on the block
                         below), flood-recipe (recipe book placements)
  --flood N              packets a tick for the flood behaviours; default 100
  --attack NAME          attack the player NAME twice a second, from wherever the bot is (a reach test),
                         swinging after each attack as vanilla's client does. NAME,NAME...: each of them
                         in the same tick (a multi-target kill aura, for the anticheat's tests);
                         the report gives each bot's last health
  --attack-after S       start attacking S seconds after joining; default 5
  --no-swing             attack without swinging (a kill aura hiding its swing, for the anticheat's tests)
  --no-loaded            never say the level is loaded (ServerboundPlayerLoadedPacket): a vanilla server
                         ignores the player's moves and actions for its first 3 seconds
  --no-read              stop reading from the server once spawned (a client that lets the
                         server's replies pile up)
  --join-every SECONDS   default 1
  --seconds N            how long to run after the first bot joins; default 60
  --view-distance N      what the bots ask for; default 10
  --seed N               default 12345
  --explore-speed B      blocks a second an explore bot flies; default 22 (as fast as chunks come)
  --pvp B                fight bots also attack each other (true or false); default true
  --name PREFIX          bot names: PREFIX0, PREFIX1...; default Bot
  --brand B              the client brand sent in configuration (minecraft:brand); default none
  --height H             blocks above spawn the bots fly at (walk, explore, idle); default 24
  --ground               claim the ground in every move (with --height 0 on a flat world: walking
                         on it; higher, or over water: a no-fall or jesus cheat, to test an anticheat)
  --give SLOT:ID:COUNT,...  creative-slot packets sent once joined (inventory menu slot, item
                         registry id, count), to test what a server takes
  --click X,Y,Z:SLOT:BUTTON:MODE  3 s after joining, open the block at X,Y,Z (a chest), then a second later
                         click SLOT of its window (ServerboundContainerClickPacket), to test what a server takes
  --raw TICK:ID:HEX,...  raw serverbound play packets (id, body in hex) sent TICK ticks after joining,
                         to test what a server takes
  --timer F              cheat, to test an anticheat: tick F times as fast as vanilla's client (1.5: 30 Hz)
  --report FILE          a JSON summary at the end (- prints it after the per-second lines)
";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Behaviour {
    Idle,
    Stand,
    Walk,
    Explore,
    Fight,
    FloodMove,
    FloodAcks,
    FloodPackets,
    FloodChunks,
    FloodUse,
    FloodClick,
    FloodSuggest,
    FloodBook,
    FloodSign,
    FloodRecipe,
}

impl Behaviour {
    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Stand => "stand",
            Self::Walk => "walk",
            Self::Explore => "explore",
            Self::Fight => "fight",
            Self::FloodMove => "flood-move",
            Self::FloodAcks => "flood-acks",
            Self::FloodPackets => "flood-packets",
            Self::FloodChunks => "flood-chunks",
            Self::FloodUse => "flood-use",
            Self::FloodClick => "flood-click",
            Self::FloodSuggest => "flood-suggest",
            Self::FloodBook => "flood-book",
            Self::FloodSign => "flood-sign",
            Self::FloodRecipe => "flood-recipe",
        }
    }
}

struct Options {
    host: String,
    port: u16,
    bots: usize,
    behaviour: String,
    join_every: f64,
    seconds: u64,
    view_distance: i8,
    seed: u64,
    explore_speed: f64,
    pvp: bool,
    /// The player to attack, and how long after joining to start.
    attack: Option<String>,
    attack_after: f64,
    no_swing: bool,
    flood: usize,
    no_read: bool,
    no_loaded: bool,
    height: f64,
    ground: bool,
    /// --raw: (tick after joining, packet id, body).
    raw: Vec<(u64, i32, Vec<u8>)>,
    /// ServerboundSetCreativeModeSlotPackets to send once joined: (slot, item id, count).
    give: Vec<(i16, i32, i32)>,
    /// --click: the block to open, then the slot, button and click type in its window.
    click: Option<([i32; 3], i16, i8, i32)>,
    name: String,
    /// The `minecraft:brand` to send, if any.
    brand: Option<String>,
    report: Option<String>,
    /// How many times as fast as vanilla's 20 Hz the bots tick (a timer cheat above 1).
    timer: f64,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        host: "127.0.0.1".into(),
        port: 25565,
        bots: 10,
        behaviour: "mixed".into(),
        join_every: 1.0,
        seconds: 60,
        view_distance: 10,
        seed: 12345,
        explore_speed: 22.0,
        pvp: true,
        attack: None,
        attack_after: 5.0,
        no_swing: false,
        flood: 100,
        no_read: false,
        no_loaded: false,
        height: 24.0,
        ground: false,
        raw: Vec::new(),
        give: Vec::new(),
        click: None,
        name: "Bot".into(),
        report: None,
        brand: None,
        timer: 1.0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-h" || arg == "--help" {
            print!("{HELP}");
            std::process::exit(0);
        }
        if arg == "--no-read" {
            o.no_read = true;
            continue;
        }
        if arg == "--no-swing" {
            o.no_swing = true;
            continue;
        }
        if arg == "--no-loaded" {
            o.no_loaded = true;
            continue;
        }
        if arg == "--ground" {
            o.ground = true;
            continue;
        }
        let value = args.next().ok_or(format!("{arg} needs a value"))?;
        let number = |what: &str| format!("{arg} needs {what}, not {value:?}");
        match arg.as_str() {
            "--server" => {
                let (host, port) = value.rsplit_once(':').unwrap_or((&value, "25565"));
                o.host = host.into();
                o.port = port.parse().map_err(|_| number("a port"))?;
            }
            "--bots" => o.bots = value.parse().map_err(|_| number("a number"))?,
            "--behaviour" => o.behaviour = value,
            "--join-every" => o.join_every = value.parse().map_err(|_| number("seconds"))?,
            "--seconds" => o.seconds = value.parse().map_err(|_| number("seconds"))?,
            "--view-distance" => o.view_distance = value.parse().map_err(|_| number("2 to 32"))?,
            "--seed" => o.seed = value.parse().map_err(|_| number("a number"))?,
            "--explore-speed" => {
                o.explore_speed = value.parse().map_err(|_| number("blocks a second"))?;
            }
            "--pvp" => o.pvp = value.parse().map_err(|_| number("true or false"))?,
            "--flood" => o.flood = value.parse().map_err(|_| number("packets a tick"))?,
            "--attack" => o.attack = Some(value),
            "--attack-after" => o.attack_after = value.parse().map_err(|_| number("seconds"))?,
            "--name" => o.name = value,
            "--report" => o.report = Some(value),
            "--brand" => o.brand = Some(value),
            "--give" => {
                for part in value.split(',') {
                    let n: Vec<&str> = part.split(':').collect();
                    let parsed = match n[..] {
                        [s, i, c] => s.parse().ok().zip(i.parse().ok()).zip(c.parse().ok()),
                        _ => None,
                    };
                    let ((slot, item), count) = parsed.ok_or_else(|| number("SLOT:ID:COUNT"))?;
                    o.give.push((slot, item, count));
                }
            }
            "--click" => {
                let bad = || number("X,Y,Z:SLOT:BUTTON:MODE");
                let n: Vec<&str> = value.split(':').collect();
                let [at, slot, button, mode] = n[..] else {
                    return Err(bad());
                };
                let at: Vec<i32> = at
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| bad())?;
                let at = <[i32; 3]>::try_from(at).map_err(|_| bad())?;
                let parse = |s: &str| s.parse::<i32>().map_err(|_| bad());
                o.click = Some((at, parse(slot)? as i16, parse(button)? as i8, parse(mode)?));
            }
            "--raw" => {
                for part in value.split(',') {
                    let bad = || number("TICK:ID:HEX");
                    let [tick, id, hex] = part.split(':').collect::<Vec<_>>()[..] else {
                        return Err(bad());
                    };
                    let body = (0..hex.len())
                        .step_by(2)
                        .map(|i| {
                            hex.get(i..i + 2)
                                .and_then(|b| u8::from_str_radix(b, 16).ok())
                        })
                        .collect::<Option<Vec<u8>>>()
                        .ok_or_else(bad)?;
                    o.raw.push((
                        tick.parse().map_err(|_| bad())?,
                        id.parse().map_err(|_| bad())?,
                        body,
                    ));
                }
            }
            "--height" => o.height = value.parse().map_err(|_| number("blocks"))?,
            "--timer" => {
                o.timer = value.parse().map_err(|_| number("a speed, like 1.5"))?;
                if !(o.timer >= 0.1 && o.timer <= 20.0) {
                    return Err(format!("--timer {value}: use 0.1 to 20"));
                }
            }
            _ => return Err(format!("unknown option {arg}; --help lists them")),
        }
    }
    const BEHAVIOURS: [&str; 16] = [
        "idle",
        "walk",
        "explore",
        "fight",
        "mixed",
        "stand",
        "flood-move",
        "flood-acks",
        "flood-packets",
        "flood-chunks",
        "flood-use",
        "flood-click",
        "flood-suggest",
        "flood-book",
        "flood-sign",
        "flood-recipe",
    ];
    if !BEHAVIOURS.contains(&o.behaviour.as_str()) {
        return Err(format!(
            "--behaviour {}: use idle, walk, explore, fight, mixed or a flood behaviour (--help lists them)",
            o.behaviour
        ));
    }
    Ok(o)
}

/// What one bot is doing, shared between its reader and its ticks.
#[derive(Default)]
struct Bot {
    online: AtomicBool,
    chunks: AtomicU64,
    /// Chunks received while the bot already held them (no ForgetLevelChunk in between).
    resent: AtomicU64,
    /// Different chunks received over the run.
    unique: AtomicU64,
    forgotten: AtomicU64,
    /// Teleports after the spawn one: the server refused a move.
    pulled_back: AtomicU64,
    /// Ticks the bot couldn't move because it didn't hold its chunk yet.
    waited: AtomicU64,
    /// Attack packets sent (fight bots and --attack).
    attacks: AtomicU64,
    /// The entities the server tells the bot about (fight bots only).
    entities: Mutex<Entities>,
    /// The chunks the bot holds (ClientChunkCache).
    held: Mutex<HashSet<(i32, i32)>>,
    /// Where the server last put the bot (a teleport), to start from.
    teleport: Mutex<Option<[f64; 3]>>,
    error: Mutex<Option<String>>,
    joined_after_ms: AtomicU64,
    /// The connection's byte counters (wire bytes, after compression and encryption), set once it is open.
    counters: OnceLock<Arc<Counters>>,
    /// The bot stopped reading (--no-read).
    deaf: AtomicBool,
    /// `--attack`: each target's UUID and, once AddEntity showed it, its entity id.
    target: Mutex<Vec<(u128, Option<i32>)>>,
    /// The last SetHealth.
    health: Mutex<Option<f32>>,
}

impl Bot {
    fn bytes_in(&self) -> u64 {
        self.counters
            .get()
            .map_or(0, |c| c.bytes_in.load(Ordering::Relaxed).max(0) as u64)
    }
    fn bytes_out(&self) -> u64 {
        self.counters
            .get()
            .map_or(0, |c| c.bytes_out.load(Ordering::Relaxed).max(0) as u64)
    }
}

/// Nearest-rank percentile (`p` in 0..=100) of an ascending-sorted slice; 0 when it is empty.
fn percentile(sorted: &[u64], p: u64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * sorted.len() as u64).div_ceil(100).max(1) as usize;
    sorted[rank - 1]
}

/// Join-time summary in milliseconds over the bots that got in: count, p50, p99 and max.
fn join_summary(mut times: Vec<u64>) -> serde_json::Value {
    times.sort_unstable();
    json!({
        "count": times.len(),
        "p50_ms": percentile(&times, 50),
        "p99_ms": percentile(&times, 99),
        "max_ms": times.last().copied().unwrap_or(0),
    })
}

/// A small deterministic generator (SplitMix64), so every run moves the bots the same way.
struct Random(u64);
impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Vanilla's bare-hand attack speed is 4 a second: a full-strength swing every 20 / 4 ticks.
const ATTACK_COOLDOWN_TICKS: u32 = 5;
/// How far, in blocks, a fight bot strikes (the survival entity interaction range is 3).
const ATTACK_REACH: f64 = 2.5;
/// A fight bot walks up to this horizontal distance, so it does not stand inside its target.
const CLOSE_IN: f64 = 1.5;
/// Walking speed in blocks a tick.
const FIGHT_STEP: f64 = 0.215;
/// `LivingEntity.DATA_HEALTH_ID`: only living entities carry it, and it is sent once it differs from 1.
const HEALTH_DATA: u8 = 9;

/// What a fight bot knows of one entity.
#[derive(Clone, Debug, PartialEq)]
struct Tracked {
    uuid: u128,
    pos: [f64; 3],
    /// Set once the server sends the health of the entity, which only living entities have.
    health: Option<f32>,
}

/// The entities around a bot, from AddEntity, the move packets, SetEntityData and RemoveEntities.
/// Ordered by id, so ties between equally near targets always go to the same entity.
#[derive(Default)]
struct Entities(BTreeMap<i32, Tracked>);

impl Entities {
    fn update(&mut self, id: i32, body: &[u8]) {
        let Ok(packet) = play::Clientbound::decode(id, body) else {
            return;
        };
        match packet {
            play::Clientbound::AddEntity(p) => {
                self.0.insert(
                    p.id,
                    Tracked {
                        uuid: p.uuid,
                        pos: p.position,
                        health: None,
                    },
                );
            }
            play::Clientbound::MoveEntityPos(play::MoveEntityPos(m))
            | play::Clientbound::MoveEntityPosRot(play::MoveEntityPosRot(m)) => {
                if let (Some(e), Some(delta)) = (self.0.get_mut(&m.id), &m.delta) {
                    e.pos = delta.end(e.pos);
                }
            }
            play::Clientbound::EntityPositionSync(p) => {
                if let Some(e) = self.0.get_mut(&p.id) {
                    e.pos = p.path.end();
                }
            }
            play::Clientbound::SetEntityData(p) => {
                if let Some(e) = self.0.get_mut(&p.id) {
                    for (index, _, value) in &p.values {
                        if *index == HEALTH_DATA
                            && let entity_data::Value::Float(health) = value
                        {
                            e.health = Some(*health);
                        }
                    }
                }
            }
            play::Clientbound::RemoveEntities(p) => {
                for id in p.ids {
                    self.0.remove(&id);
                }
            }
            _ => {}
        }
    }

    /// The nearest living entity that is not dead; other bots only when `pvp`.
    fn nearest(&self, from: [f64; 3], pvp: bool, bots: &HashSet<u128>) -> Option<(i32, [f64; 3])> {
        let mut best: Option<(f64, i32, [f64; 3])> = None;
        for (id, e) in &self.0 {
            if !e.health.is_some_and(|h| h > 0.0) || (!pvp && bots.contains(&e.uuid)) {
                continue;
            }
            let d = distance(from, e.pos);
            if best.is_none_or(|(b, ..)| d < b) {
                best = Some((d, *id, e.pos));
            }
        }
        best.map(|(_, id, pos)| (id, pos))
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// One tick of a fighting bot with a target: walk up to it, and strike once the cooldown is over.
/// Returns the entity to attack. `since_attack` counts ticks since the last swing.
fn fight_tick(pos: &mut [f64; 3], target: (i32, [f64; 3]), since_attack: &mut u32) -> Option<i32> {
    *since_attack = since_attack.saturating_add(1);
    let (id, at) = target;
    let (dx, dz) = (at[0] - pos[0], at[2] - pos[2]);
    let horizontal = dx.hypot(dz);
    if horizontal > CLOSE_IN {
        let step = FIGHT_STEP.min(horizontal - CLOSE_IN);
        pos[0] += dx / horizontal * step;
        pos[2] += dz / horizontal * step;
    }
    if *since_attack >= ATTACK_COOLDOWN_TICKS && distance(*pos, at) <= ATTACK_REACH {
        *since_attack = 0;
        return Some(id);
    }
    None
}

/// One tick of walking to random points around the spawn, at sprint speed (5.6 blocks a second).
fn walk_step(
    pos: &mut [f64; 3],
    target: &mut [f64; 3],
    spawn: [f64; 3],
    cruise: f64,
    random: &mut Random,
) {
    let (dx, dz) = (target[0] - pos[0], target[2] - pos[2]);
    let distance = dx.hypot(dz);
    if distance < 1.0 {
        let angle = random.unit() * std::f64::consts::TAU;
        let reach = 8.0 + random.unit() * 40.0;
        *target = [
            spawn[0] + angle.cos() * reach,
            cruise,
            spawn[2] + angle.sin() * reach,
        ];
    } else {
        let step = 0.28_f64.min(distance);
        pos[0] += dx / distance * step;
        pos[2] += dz / distance * step;
    }
}

/// One bot thread that panics poisons the locks it held; the others and the report go on.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn raw(id: i32, write: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(id);
    write(&mut w);
    w.data
}

/// Item ids of the pinned version (26.3's item registry).
const STONE: i32 = 1;
const WRITABLE_BOOK: i32 = 1371;

/// The `i`th packet of a costly flood this tick, the bot at `pos`; none for other behaviours.
fn costly(
    behaviour: Behaviour,
    i: usize,
    sequence: i32,
    pos: [f64; 3],
    random: &mut Random,
) -> Option<Vec<u8>> {
    use ids::play::serverbound as sb;
    static LONG_COMMAND: LazyLock<String> =
        LazyLock::new(|| format!("/msg {}", "x".repeat(30_000)));
    static BOOK: LazyLock<Vec<u8>> = LazyLock::new(|| {
        play::Serverbound::EditBook(play::EditBook {
            slot: 0,
            pages: vec!["a".repeat(1024); 100],
            title: None,
        })
        .encode()
    });
    let block = pos.map(|v| v.floor() as i32);
    let packet = match behaviour {
        Behaviour::FloodUse => {
            // A block within reach: place stone there, or break what is there.
            let at = [0, 1, 2].map(|k| block[k] + (random.unit() * 7.0) as i32 - 3);
            if i % 2 == 0 {
                play::Serverbound::UseItemOn(play::UseItemOn {
                    hand: 0,
                    pos: at,
                    face: 1,
                    cursor: [0.5, 1.0, 0.5],
                    inside: false,
                    world_border: false,
                    sequence,
                })
            } else {
                play::Serverbound::PlayerAction(play::PlayerAction {
                    action: 0,
                    pos: at,
                    direction: 1,
                    sequence,
                })
            }
        }
        Behaviour::FloodClick => play::Serverbound::ContainerClick(play::ContainerClick {
            container: 0,
            state: 0,
            slot: 36 + (i % 9) as i16,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        }),
        Behaviour::FloodSuggest => {
            let text = if i % 2 == 0 {
                "/"
            } else {
                LONG_COMMAND.as_str()
            };
            return Some(raw(sb::COMMAND_SUGGESTION, |w| {
                w.varint(sequence);
                w.string(text);
            }));
        }
        Behaviour::FloodBook => return Some(BOOK.clone()),
        Behaviour::FloodSign => play::Serverbound::SignUpdate(play::SignUpdate {
            pos: [block[0], block[1] - 1, block[2]],
            lines: std::array::from_fn(|_| "a".repeat(384)),
            front: true,
        }),
        Behaviour::FloodRecipe => play::Serverbound::PlaceRecipe(recipes::PlaceRecipe {
            container: 0,
            recipe: (i % 2042) as i32,
            use_max: true,
        }),
        _ => return None,
    };
    Some(packet.encode())
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Handshake, offline login and configuration; returns the connection in the play state.
fn join(o: &Options, name: &str) -> Result<Connection, String> {
    let mut c = Connection::connect(&o.host, o.port, Duration::from_secs(10)).map_err(err)?;
    c.send(
        &handshake::Serverbound::Intention(Intention {
            protocol: PROTOCOL_VERSION,
            host: o.host.clone(),
            port: o.port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(err)?;
    c.send(
        &login::Serverbound::Hello(ServerHello {
            name: name.into(),
            uuid: offline_uuid(name),
        })
        .encode(),
    )
    .map_err(err)?;
    loop {
        let (id, body) = c.recv().map_err(err)?;
        match login::Clientbound::decode(id, &body).map_err(err)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(err)?;
                break;
            }
            login::Clientbound::Disconnect(p) => return Err(format!("refused: {}", p.reason_json)),
            login::Clientbound::Hello(_) => {
                return Err("the server is in online mode; bots need online-mode=false".into());
            }
            login::Clientbound::CustomQuery(p) => c
                .send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: p.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::CookieRequest(p) => c
                .send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: p.key,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(err)?,
            login::Clientbound::Unknown(..) => {}
        }
    }
    use ids::configuration::clientbound as cb;
    c.send(
        &configuration::Serverbound::ClientInformation(configuration::ClientInformation::new(
            o.view_distance,
        ))
        .encode(),
    )
    .map_err(err)?;
    // ClientBrandRetriever: vanilla's client says "vanilla", Horizon's "horizon".
    if let Some(brand) = &o.brand {
        let mut w = Writer::new();
        w.string(brand);
        c.send(
            &configuration::Serverbound::CustomPayload(configuration::CustomPayload {
                channel: "minecraft:brand".into(),
                data: w.data,
            })
            .encode(),
        )
        .map_err(err)?;
    }
    loop {
        let (id, body) = c.recv().map_err(err)?;
        let reply = match id {
            cb::SELECT_KNOWN_PACKS
            | cb::KEEP_ALIVE
            | cb::PING
            | cb::CODE_OF_CONDUCT
            | cb::COOKIE_REQUEST
            | cb::DISCONNECT
            | cb::FINISH_CONFIGURATION => {
                match configuration::Clientbound::decode(id, &body).map_err(err)? {
                    configuration::Clientbound::SelectKnownPacks(p) => {
                        configuration::Serverbound::SelectKnownPacks(p)
                    }
                    configuration::Clientbound::KeepAlive(p) => {
                        configuration::Serverbound::KeepAlive(p)
                    }
                    configuration::Clientbound::Ping(p) => configuration::Serverbound::Pong(p),
                    configuration::Clientbound::CodeOfConduct(_) => {
                        configuration::Serverbound::AcceptCodeOfConduct(Empty)
                    }
                    configuration::Clientbound::CookieRequest(p) => {
                        configuration::Serverbound::CookieResponse(configuration::CookieResponse {
                            key: p.key,
                            payload: None,
                        })
                    }
                    configuration::Clientbound::Disconnect(p) => {
                        let reason = horizon_protocol::text::plain(&p.reason, &Default::default());
                        return Err(format!("disconnected during configuration: {reason}"));
                    }
                    configuration::Clientbound::FinishConfiguration(_) => {
                        c.send(&configuration::Serverbound::FinishConfiguration(Empty).encode())
                            .map_err(err)?;
                        return Ok(c);
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };
        c.send(&reply.encode()).map_err(err)?;
    }
}

/// ChunkBatchSizeCalculator: the chunks per tick a vanilla client asks for, from how long its batches take
/// to arrive.
struct BatchSize {
    nanos_per_chunk: f64,
    old_samples_weight: u32,
    started: Instant,
}

impl BatchSize {
    fn new() -> Self {
        Self {
            nanos_per_chunk: 2_000_000.0,
            old_samples_weight: 1,
            started: Instant::now(),
        }
    }
    fn finished(&mut self, size: i32) {
        if size > 0 {
            let per_chunk = self.started.elapsed().as_nanos() as f64 / f64::from(size);
            let clamped = per_chunk.clamp(self.nanos_per_chunk / 3.0, self.nanos_per_chunk * 3.0);
            let weight = f64::from(self.old_samples_weight);
            self.nanos_per_chunk = (self.nanos_per_chunk * weight + clamped) / (weight + 1.0);
            self.old_samples_weight = (self.old_samples_weight + 1).min(49);
        }
    }
    fn desired_chunks_per_tick(&self) -> f32 {
        (7_000_000.0 / self.nanos_per_chunk) as f32
    }
}

/// Reads the play state: answers what a client must answer, counts chunks, records teleports.
fn read(
    mut reader: horizon_protocol::connection::ConnReader,
    writer: Arc<Mutex<ConnWriter>>,
    bot: Arc<Bot>,
    track: bool,
) {
    use ids::play::clientbound as cb;
    use ids::play::serverbound as sb;
    let send = |body: Vec<u8>| lock(&writer).send(&body);
    let mut seen = HashSet::new();
    let mut batch = BatchSize::new();
    let mut teleports = 0u64;
    loop {
        if bot.deaf.load(Ordering::Relaxed) {
            while bot.online.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(100));
            }
            return;
        }
        let (id, body) = match reader.recv() {
            Ok(packet) => packet,
            Err(e) => {
                if bot.online.swap(false, Ordering::SeqCst) {
                    *lock(&bot.error) = Some(e.to_string());
                }
                return;
            }
        };
        let mut r = Reader::new(&body);
        let sent = match id {
            cb::KEEP_ALIVE => match r.i64() {
                Ok(v) => send(raw(sb::KEEP_ALIVE, |w| w.i64(v))),
                Err(_) => Ok(()),
            },
            cb::PING => match r.i32() {
                Ok(v) => send(raw(sb::PONG, |w| w.i32(v))),
                Err(_) => Ok(()),
            },
            cb::LEVEL_CHUNK_WITH_LIGHT => {
                bot.chunks.fetch_add(1, Ordering::Relaxed);
                if let (Ok(x), Ok(z)) = (r.i32(), r.i32()) {
                    if !lock(&bot.held).insert((x, z)) {
                        bot.resent.fetch_add(1, Ordering::Relaxed);
                    }
                    if seen.insert((x, z)) {
                        bot.unique.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Ok(())
            }
            cb::FORGET_LEVEL_CHUNK => {
                if let Ok(play::Clientbound::ForgetLevelChunk(p)) =
                    play::Clientbound::decode(id, &body)
                    && lock(&bot.held).remove(&(p.pos[0], p.pos[1]))
                {
                    bot.forgotten.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            }
            cb::ADD_ENTITY => {
                if let Ok(play::Clientbound::AddEntity(p)) = play::Clientbound::decode(id, &body) {
                    for target in lock(&bot.target).iter_mut() {
                        if target.0 == p.uuid {
                            target.1 = Some(p.id);
                        }
                    }
                }
                if track {
                    lock(&bot.entities).update(id, &body);
                }
                Ok(())
            }
            cb::SET_HEALTH => {
                if let Ok(play::Clientbound::SetHealth(p)) = play::Clientbound::decode(id, &body) {
                    *lock(&bot.health) = Some(p.health);
                }
                Ok(())
            }
            cb::CHUNK_BATCH_START => {
                batch.started = Instant::now();
                Ok(())
            }
            cb::CHUNK_BATCH_FINISHED => {
                batch.finished(r.varint().unwrap_or(0));
                send(
                    play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                        chunks_per_tick: batch.desired_chunks_per_tick(),
                    })
                    .encode(),
                )
            }
            cb::PLAYER_POSITION => match play::Clientbound::decode(id, &body) {
                Ok(play::Clientbound::PlayerPosition(p)) => {
                    let mut at = lock(&bot.teleport);
                    let previous = at.unwrap_or([0.0; 3]);
                    // Relative flags: X 1, Y 2, Z 4.
                    let pos: [f64; 3] = std::array::from_fn(|i| {
                        if p.relatives & (1 << i) != 0 {
                            previous[i] + p.position[i]
                        } else {
                            p.position[i]
                        }
                    });
                    *at = Some(pos);
                    teleports += 1;
                    if teleports > 1 {
                        bot.pulled_back.fetch_add(1, Ordering::Relaxed);
                    }
                    send(
                        play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                            id: p.teleport,
                            x: pos[0],
                            y: pos[1],
                            z: pos[2],
                            yaw: p.yaw,
                            pitch: p.pitch,
                        })
                        .encode(),
                    )
                }
                _ => Ok(()),
            },
            cb::MOVE_ENTITY_POS
            | cb::MOVE_ENTITY_POS_ROT
            | cb::ENTITY_POSITION_SYNC
            | cb::SET_ENTITY_DATA
            | cb::REMOVE_ENTITIES
                if track =>
            {
                lock(&bot.entities).update(id, &body);
                Ok(())
            }
            cb::DISCONNECT | cb::START_CONFIGURATION => {
                bot.online.store(false, Ordering::SeqCst);
                *lock(&bot.error) = Some(if id == cb::DISCONNECT {
                    format!(
                        "disconnected: {}",
                        String::from_utf8_lossy(&body)
                            .chars()
                            .filter(|c| !c.is_control())
                            .collect::<String>()
                    )
                } else {
                    "sent back to configuration".into()
                });
                return;
            }
            _ => Ok(()),
        };
        if sent.is_err() {
            bot.online.store(false, Ordering::SeqCst);
            return;
        }
    }
}

/// One bot: join, then move every 50 ms until `until`.
fn run_bot(
    o: Arc<Options>,
    index: usize,
    behaviour: Behaviour,
    bots: Arc<HashSet<u128>>,
    bot: Arc<Bot>,
    until: Arc<Mutex<Option<Instant>>>,
) {
    let name = format!("{}{index}", o.name);
    let asked = Instant::now();
    let connection = match join(&o, &name) {
        Ok(c) => c,
        Err(e) => {
            *lock(&bot.error) = Some(e);
            return;
        }
    };
    let (reader, writer) = connection.split();
    let _ = bot.counters.set(reader.counters.clone());
    let writer = Arc::new(Mutex::new(writer));
    *lock(&bot.target) = o
        .attack
        .iter()
        .flat_map(|names| names.split(','))
        .map(|name| (offline_uuid(name), None))
        .collect();
    bot.online.store(true, Ordering::SeqCst);
    {
        let (writer, bot) = (writer.clone(), bot.clone());
        let track = behaviour == Behaviour::Fight;
        thread::spawn(move || read(reader, writer, bot, track));
    }
    use ids::play::serverbound as sb;
    // Wait for the spawn position.
    let spawn = loop {
        if let Some(at) = *lock(&bot.teleport) {
            break at;
        }
        if !bot.online.load(Ordering::SeqCst) || asked.elapsed() > Duration::from_secs(60) {
            if lock(&bot.error).is_none() {
                *lock(&bot.error) = Some("no spawn position within 60 s".into());
            }
            return;
        }
        thread::sleep(Duration::from_millis(20));
    };
    bot.joined_after_ms
        .store(asked.elapsed().as_millis() as u64, Ordering::SeqCst);
    let send = |body: Vec<u8>| lock(&writer).send(&body).is_ok();
    if !o.no_loaded {
        send(raw(sb::PLAYER_LOADED, |_| {}));
    }
    bot.deaf.store(o.no_read, Ordering::Relaxed);
    // What the costly floods need in hand (creative: the server takes the bot's word for it).
    let tool = match behaviour {
        Behaviour::FloodUse => Some(Stack::new(STONE, 64)),
        Behaviour::FloodBook => Some(Stack::new(WRITABLE_BOOK, 1)),
        _ => None,
    };
    for &(slot, item, count) in &o.give {
        send(
            play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot,
                item: Some(Stack::new(item, count)),
            })
            .encode(),
        );
    }
    if tool.is_some() {
        send(
            play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
                slot: 36,
                item: tool,
            })
            .encode(),
        );
        send(play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot: 0 }).encode());
    }
    let mut sequence = 0;
    let mut random = Random(o.seed ^ (index as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    let mut pos = spawn;
    // Fight bots stay at their spawn height; the others climb to fly above the terrain.
    let cruise = if behaviour == Behaviour::Fight {
        spawn[1]
    } else {
        spawn[1] + o.height
    };
    let mut since_attack = ATTACK_COOLDOWN_TICKS;
    let heading = random.unit() * std::f64::consts::TAU;
    let mut target = pos;
    let mut next = Instant::now();
    let (joined, mut tick) = (Instant::now(), 0u64);
    loop {
        if lock(&until).is_some_and(|u| Instant::now() >= u) || !bot.online.load(Ordering::SeqCst) {
            break;
        }
        // A teleport from the server wins over where the bot thought it was.
        if let Some(at) = lock(&bot.teleport).take() {
            pos = at;
        }
        let here = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
        // The abusive client doesn't wait for its chunk, as a real client must.
        if behaviour == Behaviour::Stand {
            // Where the server put it, on the ground.
        } else if behaviour == Behaviour::FloodMove {
            // The last of these moves goes out below, with the tick's end.
            pos[1] = cruise;
            for _ in 1..o.flood.max(1) {
                let angle = random.unit() * std::f64::consts::TAU;
                let reach = 64.0 + random.unit() * 192.0;
                pos[0] += angle.cos() * reach;
                pos[2] += angle.sin() * reach;
                send(
                    play::Serverbound::MovePlayerPos(play::MovePlayerPos {
                        x: pos[0],
                        y: pos[1],
                        z: pos[2],
                        flags: 0,
                    })
                    .encode(),
                );
            }
            let angle = random.unit() * std::f64::consts::TAU;
            pos[0] += angle.cos() * 128.0;
            pos[2] += angle.sin() * 128.0;
        } else if behaviour == Behaviour::FloodChunks {
            // Up to cruising height first, then just under the server's 10 blocks a tick:
            // every tick asks for new chunks.
            if pos[1] < cruise {
                pos[1] = (pos[1] + 0.5).min(cruise);
            } else {
                pos[0] += heading.cos() * 9.9;
                pos[2] += heading.sin() * 9.9;
            }
        } else if !lock(&bot.held).contains(&here) {
            bot.waited.fetch_add(1, Ordering::Relaxed);
        } else if pos[1] < cruise {
            pos[1] = (pos[1] + 0.5).min(cruise);
        } else {
            match behaviour {
                Behaviour::Idle | Behaviour::Stand => {}
                Behaviour::Walk => walk_step(&mut pos, &mut target, spawn, cruise, &mut random),
                Behaviour::Fight => {
                    let near = lock(&bot.entities).nearest(pos, o.pvp, &bots);
                    if let Some(near) = near {
                        if let Some(victim) = fight_tick(&mut pos, near, &mut since_attack) {
                            // The client sends the attack, then swings the arm.
                            let sent = send(
                                play::Serverbound::Attack(play::Attack { id: victim }).encode(),
                            ) && send(raw(sb::PUNCH, |w| w.varint(0)));
                            if sent {
                                bot.attacks.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    } else {
                        since_attack = since_attack.saturating_add(1);
                        walk_step(&mut pos, &mut target, spawn, cruise, &mut random);
                    }
                }
                Behaviour::FloodMove
                | Behaviour::FloodChunks
                | Behaviour::FloodUse
                | Behaviour::FloodClick
                | Behaviour::FloodSuggest
                | Behaviour::FloodBook
                | Behaviour::FloodSign
                | Behaviour::FloodRecipe => {}
                Behaviour::FloodPackets => {
                    for i in 0..o.flood {
                        send(
                            play::Serverbound::MovePlayerRot(play::MovePlayerRot {
                                yaw: (i % 360) as f32,
                                pitch: 0.0,
                                flags: 0,
                            })
                            .encode(),
                        );
                    }
                }
                Behaviour::FloodAcks | Behaviour::Explore => {
                    if behaviour == Behaviour::FloodAcks {
                        for _ in 0..o.flood {
                            send(
                                play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                                    chunks_per_tick: 64.0,
                                })
                                .encode(),
                            );
                        }
                    }
                    // Flying in creative, by default about 22 blocks a second, which keeps new
                    // chunks coming as fast as the server sends them.
                    let step = o.explore_speed / 20.0;
                    pos[0] += heading.cos() * step;
                    pos[2] += heading.sin() * step;
                }
            }
        }
        if let Some((at, slot, button, input)) = o.click {
            // Ticks of 50 ms: the block 3 s after joining, the click a second later.
            if tick == 60 {
                sequence += 1;
                send(
                    play::Serverbound::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: at,
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence,
                    })
                    .encode(),
                );
            } else if tick == 80 {
                // The first window a player opens is 1.
                send(
                    play::Serverbound::ContainerClick(play::ContainerClick {
                        container: 1,
                        state: 0,
                        slot,
                        button,
                        input,
                        changed: Vec::new(),
                        carried: None,
                    })
                    .encode(),
                );
            }
        }
        for (at, id, body) in &o.raw {
            if *at == tick {
                send(raw(*id, |w| w.bytes(body)));
            }
        }
        // The costly floods go out wherever the bot is, chunks or not (--no-read gets none).
        for i in 0..o.flood {
            sequence += 1;
            if let Some(body) = costly(behaviour, i, sequence, pos, &mut random) {
                send(body);
            }
        }
        if joined.elapsed().as_secs_f64() >= o.attack_after && tick % 10 == 0 {
            let targets: Vec<i32> = lock(&bot.target).iter().filter_map(|t| t.1).collect();
            for id in targets {
                send(play::Serverbound::Attack(play::Attack { id }).encode());
                // Minecraft.startAttack swings right after the attack.
                if !o.no_swing {
                    send(play::Serverbound::Punch(Empty).encode());
                }
                bot.attacks.fetch_add(1, Ordering::Relaxed);
            }
        }
        tick += 1;
        let ok = send(
            play::Serverbound::MovePlayerPos(play::MovePlayerPos {
                x: pos[0],
                y: pos[1],
                z: pos[2],
                // Fight bots stand on the ground; the others fly.
                flags: u8::from(
                    matches!(behaviour, Behaviour::Fight | Behaviour::Stand) || o.ground,
                ),
            })
            .encode(),
        ) && send(raw(sb::CLIENT_TICK_END, |_| {}));
        if !ok {
            bot.online.store(false, Ordering::SeqCst);
            break;
        }
        next += Duration::from_secs_f64(0.05 / o.timer);
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        } else if now - next > Duration::from_secs(1) {
            next = now;
        }
    }
    // Offline first, so the reader doesn't count the closed connection as a failure.
    bot.online.store(false, Ordering::SeqCst);
    lock(&writer).shutdown();
}

fn behaviour_of(mode: &str, index: usize) -> Behaviour {
    match mode {
        "idle" => Behaviour::Idle,
        "stand" => Behaviour::Stand,
        "walk" => Behaviour::Walk,
        "explore" => Behaviour::Explore,
        "fight" => Behaviour::Fight,
        "flood-move" => Behaviour::FloodMove,
        "flood-acks" => Behaviour::FloodAcks,
        "flood-packets" => Behaviour::FloodPackets,
        "flood-chunks" => Behaviour::FloodChunks,
        "flood-use" => Behaviour::FloodUse,
        "flood-click" => Behaviour::FloodClick,
        "flood-suggest" => Behaviour::FloodSuggest,
        "flood-book" => Behaviour::FloodBook,
        "flood-sign" => Behaviour::FloodSign,
        "flood-recipe" => Behaviour::FloodRecipe,
        // mixed: of every five bots, three walk, one explores, one stands.
        _ => match index % 5 {
            0 | 2 | 4 => Behaviour::Walk,
            1 => Behaviour::Explore,
            _ => Behaviour::Idle,
        },
    }
}

fn main() {
    let o = match options() {
        Ok(o) => Arc::new(o),
        Err(e) => {
            eprintln!("horizon-bots: {e}");
            std::process::exit(2);
        }
    };
    let start = Instant::now();
    let until: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let bots: Vec<Arc<Bot>> = (0..o.bots).map(|_| Arc::new(Bot::default())).collect();
    // The uuids of all bots, so that `--pvp false` leaves them alone.
    let bot_uuids: Arc<HashSet<u128>> = Arc::new(
        (0..o.bots)
            .map(|i| offline_uuid(&format!("{}{i}", o.name)))
            .collect(),
    );
    let launcher = {
        let (o, bots, until) = (o.clone(), bots.clone(), until.clone());
        let bot_uuids = bot_uuids.clone();
        thread::spawn(move || {
            let mut handles = Vec::new();
            for (i, bot) in bots.iter().enumerate() {
                let (o2, bot, until) = (o.clone(), bot.clone(), until.clone());
                let (behaviour, uuids) = (behaviour_of(&o.behaviour, i), bot_uuids.clone());
                handles.push(thread::spawn(move || {
                    run_bot(o2, i, behaviour, uuids, bot, until);
                }));
                thread::sleep(Duration::from_secs_f64(o.join_every));
            }
            handles
        })
    };
    // The run lasts --seconds after the first bot is in.
    let mut previous_chunks = 0;
    let mut first = None;
    loop {
        thread::sleep(Duration::from_secs(1));
        let online = bots
            .iter()
            .filter(|b| b.online.load(Ordering::SeqCst))
            .count();
        let chunks: u64 = bots.iter().map(|b| b.chunks.load(Ordering::Relaxed)).sum();
        let total = |f: fn(&Bot) -> &AtomicU64| -> u64 {
            bots.iter().map(|b| f(b).load(Ordering::Relaxed)).sum()
        };
        let failed = bots.iter().filter(|b| lock(&b.error).is_some()).count();
        if first.is_none() && online > 0 {
            first = Some(Instant::now());
            *lock(&until) = Some(Instant::now() + Duration::from_secs(o.seconds));
        }
        println!(
            "{}",
            json!({
                "t": start.elapsed().as_secs(),
                "online": online,
                "failed": failed,
                "chunks": chunks,
                "chunks_per_second": chunks - previous_chunks,
                "unique": total(|b| &b.unique),
                "forgotten": total(|b| &b.forgotten),
                "resent": total(|b| &b.resent),
                "pulled_back": total(|b| &b.pulled_back),
                "waited_ticks": total(|b| &b.waited),
                "attacks": total(|b| &b.attacks),
            })
        );
        previous_chunks = chunks;
        let over = lock(&until).is_some_and(|u| Instant::now() >= u);
        let all_failed = failed == o.bots;
        if over || all_failed || (first.is_none() && start.elapsed() > Duration::from_secs(120)) {
            *lock(&until) = Some(Instant::now());
            break;
        }
    }
    for handle in launcher.join().unwrap_or_default() {
        let _ = handle.join();
    }
    let summary = json!({
        "server": format!("{}:{}", o.host, o.port),
        "bots": o.bots,
        "behaviour": o.behaviour,
        "seed": o.seed,
        "explore_speed": o.explore_speed,
        "pvp": o.pvp,
        "view_distance": o.view_distance,
        "seconds": o.seconds,
        "joined": bots.iter().filter(|b| b.joined_after_ms.load(Ordering::SeqCst) > 0).count(),
        "chunks": bots.iter().map(|b| b.chunks.load(Ordering::Relaxed)).sum::<u64>(),
        "resent": bots.iter().map(|b| b.resent.load(Ordering::Relaxed)).sum::<u64>(),
        "pulled_back": bots.iter().map(|b| b.pulled_back.load(Ordering::Relaxed)).sum::<u64>(),
        "waited_ticks": bots.iter().map(|b| b.waited.load(Ordering::Relaxed)).sum::<u64>(),
        "attacks": bots.iter().map(|b| b.attacks.load(Ordering::Relaxed)).sum::<u64>(),
        "bytes_in": bots.iter().map(|b| b.bytes_in()).sum::<u64>(),
        "bytes_out": bots.iter().map(|b| b.bytes_out()).sum::<u64>(),
        "join_time": join_summary(
            bots.iter()
                .map(|b| b.joined_after_ms.load(Ordering::SeqCst))
                .filter(|&ms| ms > 0)
                .collect(),
        ),
        "per_bot": bots.iter().enumerate().map(|(i, b)| json!({
            "name": format!("{}{i}", o.name),
            "behaviour": behaviour_of(&o.behaviour, i).name(),
            "joined_after_ms": b.joined_after_ms.load(Ordering::SeqCst),
            "bytes_in": b.bytes_in(),
            "bytes_out": b.bytes_out(),
            "chunks": b.chunks.load(Ordering::Relaxed),
            "unique_chunks": b.unique.load(Ordering::Relaxed),
            "resent": b.resent.load(Ordering::Relaxed),
            "forgotten": b.forgotten.load(Ordering::Relaxed),
            "pulled_back": b.pulled_back.load(Ordering::Relaxed),
            "waited_ticks": b.waited.load(Ordering::Relaxed),
            "error": *lock(&b.error),
            "attacks": b.attacks.load(Ordering::Relaxed),
            "health": *lock(&b.health),
        })).collect::<Vec<_>>(),
    });
    if o.report.as_deref() == Some("-") {
        println!("{summary:#}");
    } else if let Some(path) = &o.report {
        if let Err(e) = std::fs::write(path, format!("{summary:#}\n")) {
            eprintln!("horizon-bots: {path}: {e}");
        }
    }
    let errors: Vec<String> = bots
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            b.error
                .lock()
                .unwrap()
                .clone()
                .map(|e| format!("{}{i}: {e}", o.name))
        })
        .collect();
    for e in errors.iter().take(10) {
        eprintln!("horizon-bots: {e}");
    }
    std::process::exit(if errors.is_empty() { 0 } else { 1 });
}

#[cfg(test)]
mod tests {
    use super::*;
    use play::{
        AddEntity, Clientbound, MoveEntity, MoveEntityPos, RemoveEntities, SetEntityData, VecDelta,
    };

    fn feed(entities: &mut Entities, packet: &Clientbound) {
        let bytes = packet.encode();
        let mut r = Reader::new(&bytes);
        let id = r.varint().unwrap();
        entities.update(id, &bytes[bytes.len() - r.remaining()..]);
    }

    fn add(id: i32, uuid: u128, pos: [f64; 3]) -> Clientbound {
        Clientbound::AddEntity(AddEntity {
            id,
            uuid,
            kind: 1,
            position: pos,
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        })
    }

    fn health(id: i32, health: f32) -> Clientbound {
        Clientbound::SetEntityData(SetEntityData {
            id,
            values: vec![(HEALTH_DATA, 3, entity_data::Value::Float(health))],
        })
    }

    #[test]
    fn tracks_living_entities_only() {
        let mut e = Entities::default();
        feed(&mut e, &add(1, 10, [3.0, 64.0, 0.0]));
        feed(&mut e, &add(2, 11, [5.0, 64.0, 0.0]));
        feed(&mut e, &health(2, 20.0));
        // Entity 1 never sent a health (an item, an arrow), so it is no target.
        assert_eq!(
            e.nearest([0.0, 64.0, 0.0], true, &HashSet::new()),
            Some((2, [5.0, 64.0, 0.0]))
        );
        feed(&mut e, &health(2, 0.0));
        assert_eq!(e.nearest([0.0, 64.0, 0.0], true, &HashSet::new()), None);
        feed(&mut e, &health(2, 4.0));
        feed(
            &mut e,
            &Clientbound::RemoveEntities(RemoveEntities { ids: vec![2] }),
        );
        assert_eq!(e.nearest([0.0, 64.0, 0.0], true, &HashSet::new()), None);
    }

    #[test]
    fn moves_follow_the_deltas() {
        let mut e = Entities::default();
        feed(&mut e, &add(7, 1, [0.0, 64.0, 0.0]));
        feed(&mut e, &health(7, 20.0));
        feed(
            &mut e,
            &Clientbound::MoveEntityPos(MoveEntityPos(MoveEntity {
                id: 7,
                delta: Some(VecDelta::Linear([4096, 0, -8192])),
                rotation: None,
                on_ground: true,
            })),
        );
        assert_eq!(e.0[&7].pos, [1.0, 64.0, -2.0]);
    }

    #[test]
    fn pvp_false_spares_the_bots_and_ties_go_to_the_lowest_id() {
        let mut e = Entities::default();
        feed(&mut e, &add(9, 100, [2.0, 64.0, 0.0]));
        feed(&mut e, &add(4, 101, [-2.0, 64.0, 0.0]));
        feed(&mut e, &add(5, 102, [0.0, 64.0, 6.0]));
        for id in [9, 4, 5] {
            feed(&mut e, &health(id, 20.0));
        }
        let bots = HashSet::from([100, 101]);
        let from = [0.0, 64.0, 0.0];
        assert_eq!(e.nearest(from, true, &bots).map(|t| t.0), Some(4));
        assert_eq!(e.nearest(from, false, &bots).map(|t| t.0), Some(5));
    }

    #[test]
    fn attacks_at_the_vanilla_cooldown() {
        let mut pos = [0.0, 64.0, 0.0];
        let target = (3, [1.0, 64.0, 0.0]);
        let mut since = ATTACK_COOLDOWN_TICKS;
        let hits: Vec<u32> = (0..40)
            .filter_map(|tick| fight_tick(&mut pos, target, &mut since).map(|_| tick))
            .collect();
        assert_eq!(hits[0], 0, "a bot in reach strikes at once");
        assert!(
            hits.windows(2)
                .all(|w| w[1] - w[0] == ATTACK_COOLDOWN_TICKS)
        );
        assert_eq!(hits.len(), 8);
    }

    #[test]
    fn walks_into_reach_before_striking() {
        let mut pos = [0.0, 64.0, 0.0];
        let target = (3, [10.0, 64.0, 0.0]);
        let mut since = ATTACK_COOLDOWN_TICKS;
        let mut first = None;
        for tick in 0..100 {
            if fight_tick(&mut pos, target, &mut since).is_some() {
                first = Some(tick);
                break;
            }
        }
        assert!(first.is_some_and(|t| t > 30));
        assert!((target.1[0] - pos[0]).abs() <= ATTACK_REACH);
        assert!(pos[0] < 10.0 - CLOSE_IN + 1e-9);
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[7], 99), 7);
        let v: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&v, 50), 50);
        assert_eq!(percentile(&v, 99), 99);
        assert_eq!(percentile(&[10, 20, 30, 40], 50), 20);
    }

    #[test]
    fn join_summary_sorts_and_reports_max() {
        let s = join_summary(vec![300, 100, 200]);
        assert_eq!(s["count"], 3);
        assert_eq!(s["p50_ms"], 200);
        assert_eq!(s["p99_ms"], 300);
        assert_eq!(s["max_ms"], 300);
        assert_eq!(join_summary(vec![])["count"], 0);
    }

    #[test]
    fn bot_bytes_come_from_the_connection_counters() {
        let bot = Bot::default();
        assert_eq!((bot.bytes_in(), bot.bytes_out()), (0, 0));
        let counters = Arc::new(Counters::default());
        counters.bytes_in.store(1234, Ordering::Relaxed);
        counters.bytes_out.store(56, Ordering::Relaxed);
        let _ = bot.counters.set(counters);
        assert_eq!((bot.bytes_in(), bot.bytes_out()), (1234, 56));
    }
}
