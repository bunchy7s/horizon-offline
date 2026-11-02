//! Control-socket commands for agents and scripts that play the game: read
//! the world, the player, the inventory and the open screen as JSON, and act
//! like a player (walk, look, break, place, use, click buttons). Actions that
//! take time run as a task advanced every frame and reply when they finish or
//! time out, so `horizon-ctl break 10 64 3` returns what the block dropped.
use super::event_loop::EventLoopCtl;
use super::{Game, Screen};
use horizon_client::{
    Result,
    block_entity_anim::Ticker,
    items::ItemStack,
    mobs::{
        Species,
        render::{PROFESSIONS, VILLAGER_TYPES},
    },
    widgets,
};
use horizon_core::geometry::Vec3;
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use winit::{
    dpi::PhysicalPosition,
    event::{DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent},
    keyboard::KeyCode,
};

pub(super) const HELP: &[&str] = &[
    "state                          player, world, screen and crosshair target in one reply",
    "looking-at                     the block or entity under the crosshair (distance, face)",
    "entities [RADIUS] [TYPE]       mobs, dropped items and orbs near the player, nearest first",
    "block X Y Z                    the block state at a position",
    "blocks X1 Y1 Z1 X2 Y2 Z2       the non-air blocks of a box (at most 32768 blocks)",
    "light X1 Y1 Z1 X2 Y2 Z2        the stored sky and block light of a box (at most 32768 blocks)",
    "find-block BLOCK [RADIUS]      the nearest loaded blocks of a kind (default radius 32)",
    "inventory                      hotbar, main inventory, armor, offhand and carried stack",
    "screen                         the open screen or menu, its slots and its buttons",
    "messages [N]                   recent chat and system messages, title and action bar",
    "move DIR SECONDS [--sprint] [--sneak] [--jump]  walk forward|back|left|right",
    "jump                           jump once",
    "goto X Y Z [--near R] [--sprint] [--direct] [--timeout S]  walk to a position along a path",
    "look-at X Y Z | look-at block X Y Z [FACE] | look-at entity ID  [--over S]",
    "turn DYAW DPITCH --over S      rotate the view with mouse motion spread over S seconds",
    "mouse-move DX DY               relative mouse motion, turned like a player's mouse",
    "attack [ID]                    swing at the target (after looking at entity ID)",
    "break X Y Z [--timeout S]      mine a block until it breaks; reports drops and pickups",
    "place X Y Z [--item ITEM]      place the held (or given) item at a position",
    "interact X Y Z [FACE]          right-click a block (open a door, chest, crafting table...)",
    "use-item [--hold S]            right-click with the held item (eat, drink, throw, draw a bow)",
    "select SLOT|ITEM               select a hotbar slot (1-9) or the hotbar slot holding ITEM",
    "drop [all]                     drop one of the held item (all: the stack)",
    "open-inventory                 open the inventory screen",
    "click-button LABEL             click a button of the open screen or menu by its label",
    "click-at X Y [right]           click at GUI pixel X Y of the window",
    "drag-at [right] X Y X Y...     press at the first GUI point, move through the others, release",
    "scroll-at X Y STEPS            turn the wheel at GUI pixel X Y (positive scrolls up)",
    "command TEXT                   run a /command",
    "pause                          open the pause menu",
    "wait TICKS                     wait for game ticks to pass (held keys stay held)",
    "stop                           cancel the running action and release every key",
];

/// Keys the agent holds for a task (the game's movement reads these).
const MOVE_KEYS: [KeyCode; 7] = [
    KeyCode::KeyW,
    KeyCode::KeyS,
    KeyCode::KeyA,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::ShiftLeft,
    KeyCode::ControlLeft,
];

/// Words of a command line: positional values and `--flag [value]` options.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Args<'a> {
    pub pos: Vec<&'a str>,
    pub flags: HashMap<&'a str, Option<&'a str>>,
}
/// Options that take a value (`--timeout 10` or `--timeout=10`).
const VALUE_FLAGS: &[&str] = &["timeout", "near", "item", "radius", "hold", "face", "over"];

pub(super) fn parse_args<'a>(words: &[&'a str]) -> Args<'a> {
    let mut args = Args::default();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if let Some(flag) = w.strip_prefix("--") {
            if let Some((k, v)) = flag.split_once('=') {
                args.flags.insert(k, Some(v));
            } else if VALUE_FLAGS.contains(&flag) && i + 1 < words.len() {
                args.flags.insert(flag, Some(words[i + 1]));
                i += 1;
            } else {
                args.flags.insert(flag, None);
            }
        } else {
            args.pos.push(w);
        }
        i += 1;
    }
    args
}
impl Args<'_> {
    fn has(&self, flag: &str) -> bool {
        self.flags.contains_key(flag)
    }
    fn num(&self, flag: &str, default: f64) -> Result<f64> {
        match self.flags.get(flag) {
            Some(Some(v)) => Ok(v.parse()?),
            Some(None) => Err(format!("--{flag} needs a value").into()),
            None => Ok(default),
        }
    }
    fn get(&self, i: usize, what: &str) -> Result<&str> {
        self.pos
            .get(i)
            .copied()
            .ok_or_else(|| format!("missing {what}").into())
    }
    fn f64(&self, i: usize, what: &str) -> Result<f64> {
        let v = self.get(i, what)?;
        v.parse()
            .map_err(|_| format!("{what} must be a number, not {v}").into())
    }
    fn i32(&self, i: usize, what: &str) -> Result<i32> {
        let v = self.get(i, what)?;
        v.parse()
            .map_err(|_| format!("{what} must be an integer, not {v}").into())
    }
    fn block_pos(&self, from: usize) -> Result<[i32; 3]> {
        Ok([
            self.i32(from, "X")?,
            self.i32(from + 1, "Y")?,
            self.i32(from + 2, "Z")?,
        ])
    }
}

fn namespaced(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("minecraft:{name}")
    }
}

const FACES: [(&str, [i32; 3]); 6] = [
    ("down", [0, -1, 0]),
    ("up", [0, 1, 0]),
    ("north", [0, 0, -1]),
    ("south", [0, 0, 1]),
    ("west", [-1, 0, 0]),
    ("east", [1, 0, 0]),
];
fn face_name(face: [i32; 3]) -> &'static str {
    FACES
        .iter()
        .find(|(_, d)| *d == face)
        .map_or("?", |(n, _)| n)
}
fn face_dir(name: &str) -> Result<[i32; 3]> {
    FACES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
        .ok_or_else(|| format!("unknown face {name} (down up north south west east)").into())
}
fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
/// Minecraft's yaw and pitch facing from `eye` towards `at` (yaw 0 faces +Z,
/// 90 faces -X; positive pitch looks down).
pub(super) fn yaw_pitch(eye: [f64; 3], at: [f64; 3]) -> (f32, f32) {
    let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
    let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
    (
        (-d[0]).atan2(d[2]).to_degrees() as f32,
        (-d[1]).atan2(horizontal).to_degrees() as f32,
    )
}

fn counts(items: impl Iterator<Item = ItemStack>) -> BTreeMap<String, i64> {
    let mut map = BTreeMap::new();
    for s in items {
        *map.entry(s.item.to_string()).or_insert(0) += i64::from(s.count);
    }
    map
}
/// Item counts that changed between two inventories (positive: gained).
fn delta(before: &BTreeMap<String, i64>, after: &BTreeMap<String, i64>) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for key in before.keys().chain(after.keys()) {
        let d = after.get(key).copied().unwrap_or(0) - before.get(key).copied().unwrap_or(0);
        if d != 0 {
            out.insert(key.clone(), d);
        }
    }
    out
}

/// A feet position the A* path walks through.
type Node = [i32; 3];

enum TaskKind {
    /// Hold keys until a deadline.
    Move {
        keys: Vec<KeyCode>,
        until: Instant,
        from: Vec3,
    },
    Wait {
        until_tick: u64,
    },
    Jump {
        release_tick: u64,
        from_y: f64,
    },
    Goto(Box<Goto>),
    Break(Box<Break>),
    /// A right click on a block or a placement: report after a few ticks.
    Click {
        pos: Option<[i32; 3]>,
        before: Option<BlockStateId>,
        inventory: BTreeMap<String, i64>,
        report_tick: u64,
        release: Option<Instant>,
        place: bool,
    },
    Attack {
        id: Option<u32>,
        health: Option<f32>,
        report_tick: u64,
    },
}
impl TaskKind {
    /// Whether the keys and buttons are released when the task ends. A wait
    /// holds nothing, so what `key w down` pressed stays down through it.
    fn releases_input(&self) -> bool {
        !matches!(self, TaskKind::Wait { .. })
    }
}
struct Goto {
    target: [f64; 3],
    near: f64,
    sprint: bool,
    path: Vec<Node>,
    next: usize,
    /// Tick of the last waypoint reached, and how many replans so far.
    progress_tick: u64,
    replans: u32,
    jump_until: u64,
    warning: Option<String>,
}
struct Break {
    pos: [i32; 3],
    before: BlockStateId,
    items_before: HashSet<u32>,
    inventory: BTreeMap<String, i64>,
    started_tick: u64,
    /// Set once the block changed: collect drops until this tick.
    collect_until: Option<u64>,
    broke_tick: u64,
    drops: BTreeMap<u32, ItemStack>,
}
struct Task {
    kind: TaskKind,
    reply: Sender<String>,
    deadline: Instant,
    name: &'static str,
}

/// Mouse motion an agent feeds over frames (`mouse-move`, `turn --over`,
/// `look-at --over`): eased, through `MouseHandler`'s accumulated movement.
struct Glide {
    /// The whole motion, in mouse units, and how much has been fed.
    total: (f64, f64),
    fed: (f64, f64),
    started: Instant,
    over: Duration,
    /// The frame count when motion was last fed: the reply waits for the
    /// frame that turns the view with it.
    fed_frame: u64,
    deadline: Instant,
    reply: Sender<String>,
    /// What `look-at` aimed at, reported once the turn is done.
    aim: Option<&'static str>,
}

#[derive(Default)]
pub(super) struct Agent {
    task: Option<Task>,
    glide: Option<Glide>,
    /// Control-fixture barrier: hold simulation and packet handling, not transport threads.
    pub hold_server: bool,
}

pub(super) enum Handled {
    Reply(Value),
    /// The reply comes later, from the task.
    Deferred,
    /// Not an agent command.
    No,
}

impl Game<'_> {
    pub(super) fn state_string(&self, state: BlockStateId) -> String {
        let Some(s) = self.pack.assets.registry.state(state) else {
            return "?".into();
        };
        if s.properties.is_empty() {
            return s.name.clone();
        }
        let props: Vec<String> = s
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!("{}[{}]", s.name, props.join(","))
    }
    fn block_json(&self, pos: [i32; 3]) -> Value {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let mut v = json!({
            "pos": pos,
            "block": &*self.res.facts().get(state).name,
            "state": self.state_string(state),
            "loaded": self.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4)),
        });
        // How far a chest's or a shulker box's lid is open, 0..1, as drawn.
        match self.be_anims.ticker(state.0) {
            Some(Ticker::Lid) => v["lid"] = json!(self.chest_lid(pos, 1.0)),
            Some(Ticker::Shulker { .. }) => v["lid"] = json!(self.block_anim(pos, 1.0).shulker.0),
            _ => {}
        }
        // The lines of a sign, as this client has them.
        if let Some(horizon_client::containers::BlockEntity::Sign(sign)) =
            self.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        {
            v["sign"] = json!({ "front": sign.front.lines, "back": sign.back.lines });
        }
        v
    }
    pub(super) fn stack_value(&self, s: &ItemStack) -> Value {
        let mut v = json!({ "item": &*s.item, "count": s.count });
        if s.damage > 0 {
            v["damage"] = json!(s.damage);
        }
        if let Some(c) = horizon_client::item_data::encode_components(s) {
            v["components"] = json!(horizon_client::snbt::to_snbt(&c));
        }
        v
    }
    fn slot_value(&self, slot: usize, s: &Option<ItemStack>) -> Value {
        match s {
            Some(s) => {
                let mut v = self.stack_value(s);
                v["slot"] = json!(slot);
                v
            }
            None => json!({ "slot": slot, "item": null }),
        }
    }
    fn need_world(&self) -> Result<()> {
        if self.title.is_some() {
            return Err("no world open (title screen); use: open WORLD [SEED] [creative]".into());
        }
        Ok(())
    }
    fn playing(&self) -> Result<()> {
        if self.title.is_some() {
            return Err("no world open (title screen); use: open WORLD [SEED] [creative]".into());
        }
        if self.player.is_none() {
            return Err("the player has not spawned yet; try wait-ready".into());
        }
        if self.screen != Screen::Playing || self.book.is_some() || self.sign_edit.is_some() {
            return Err(format!(
                "the {} screen is open; close it first (close, or click-button)",
                self.screen_label()
            )
            .into());
        }
        Ok(())
    }
    pub(super) fn screen_label(&self) -> &'static str {
        if self.dialog_open() {
            return "dialog";
        }
        if self.title.is_some() {
            return "title";
        }
        if self.book.is_some() {
            return "book";
        }
        if self.sign_edit.is_some() {
            return "sign";
        }
        match self.screen {
            Screen::Playing => "playing",
            Screen::GameModeSwitcher => "gamemode-switcher",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Inventory if self.station.is_some() => "station",
            Screen::Inventory if self.container.is_some() => "container",
            Screen::Inventory => "inventory",
            Screen::Creative => "creative",
            Screen::Dead => "dead",
            Screen::Chat => "chat",
            Screen::Stats => "stats",
            Screen::Advancements => "advancements",
            Screen::WorldOptions => "world-options",
            Screen::Friends => "friends",
            Screen::Social => "social",
            Screen::ServerLinks => "server-links",
            Screen::ConfirmLink => "confirm-link",
            Screen::DialogWarning => "dialog-warning",
            Screen::CommandBlock => "command-block",
            Screen::BlockEditor => "block-editor",
            Screen::Win => "win",
            Screen::DebugOptions => "debug-options",
            Screen::Mod => "mod",
        }
    }
    pub(super) fn inventory_counts(&self) -> BTreeMap<String, i64> {
        counts(
            self.inventory
                .iter()
                .chain(self.armor.iter())
                .chain(std::iter::once(&self.offhand))
                .chain(std::iter::once(&self.carried))
                .flatten()
                .cloned(),
        )
    }

    /// The block or entity under the crosshair.
    fn looking_at(&self) -> Value {
        let Some(eye) = self.eye_position() else {
            return Value::Null;
        };
        let block = self.pick().map(|hit| {
            let mut v = self.block_json(hit.pos);
            v["face"] = json!(face_name(hit.face));
            v["point"] = json!(hit.point);
            v["distance"] = json!(dist(eye, hit.point));
            v
        });
        let entity = self.mob_hit().map(|(i, d)| {
            let m = &self.mobs[i];
            json!({ "id": m.id, "type": m.kind.id, "health": m.health, "distance": d,
                    "pos": [m.position.x, m.position.y, m.position.z] })
        });
        let kind = match (&block, &entity) {
            (Some(b), Some(e)) if e["distance"].as_f64() <= b["distance"].as_f64() => "entity",
            (_, Some(_)) if block.is_none() => "entity",
            (Some(_), _) => "block",
            _ => "none",
        };
        json!({ "kind": kind, "block": block, "entity": entity })
    }

    fn state_value(&self) -> Value {
        let player = self.player.as_ref().map(|p| {
            json!({
                "position": [p.position.x, p.position.y, p.position.z],
                "block_pos": [p.position.x.floor() as i32, p.position.y.floor() as i32, p.position.z.floor() as i32],
                "eye": self.eye_position(),
                "yaw": self.yaw,
                "pitch": self.pitch,
                "velocity": [p.velocity.x, p.velocity.y, p.velocity.z],
                "on_ground": p.ground.on_ground,
                "flying": p.flying,
                "fall_flying": p.fall_flying,
                "sneaking": self.keys.contains(&KeyCode::ShiftLeft),
                "sprinting": p.sprinting,
                "fov_modifier": self.view.fov_modifier,
                "health": p.health.health(),
                "absorption": p.health.absorption(),
                // The hurt animation's ticks left (the camera's hurt tilt).
                "hurt_time": p.health.hurt_animation_time,
                // LivingEntity.deathTime while dead (the camera's death roll).
                "death_time": self.death_time(),
                "air": p.air,
                // LocalPlayer.portalEffectIntensity: the Nether portal overlay.
                "portal_effect": self.screen_fx.portal,
                "food": p.health.food.food,
                "saturation": p.health.food.saturation,
                "shield_cooldown": self.shield_cooldown,
                "xp": { "level": self.experience.level, "progress": self.experience.progress, "total": self.experience.total },
                "effects": self.potions.effects.list.iter().map(|e| json!({ "id": e.id, "duration_ticks": e.duration, "amplifier": e.amplifier })).collect::<Vec<_>>(),
            })
        });
        json!({
            "dialog": self.dialog_status(),
            "screen": self.screen_label(),
            "menu": (self.title.is_some() || self.split_profiles_open()).then(|| self.title_value()),
            "world": (self.title.is_none()).then(|| match &self.integrated {
                Some(server) => server.world_name.clone(),
                None => self.level.name.clone(),
            }),
            "dimension": self.dim.id(),
            "gamemode": format!("{:?}", self.mode).to_lowercase(),
            // The bed (its head) the player sleeps in.
            "sleeping": self.sleeping,
            // The item in use: its slot and for how many ticks.
            "using": self.using.map(|(slot, ticks)| json!({ "slot": slot, "ticks": ticks })),
            // The block being dug: how far along, after how many ticks.
            "mining": self.mining.as_ref().map(|m| json!({ "pos": m.pos, "progress": m.progress, "ticks": m.ticks })),
            // The arm swings started (LivingEntity.swing).
            "swings": self.swings_started,
            "time": { "day_time": self.world.day_time, "day": self.world.day_time.div_euclid(24000), "time_of_day": self.world.day_time.rem_euclid(24000) },
            "weather": { "raining": self.world.weather.raining, "thundering": self.world.weather.thundering },
            "paused": self.pause,
            // Mouse movement turns the camera (`apply_mouse_look`): the real
            // pointer is grabbed, or an agent's `mouse-move` stands in for it.
            "pointer_grabbed": self.captured || self.agent_pointer,
            "player": player,
            "selected": self.selected + 1,
            "held": self.held().map(|s| self.stack_value(s)),
            "looking_at": self.looking_at(),
            "busy": self.agent.task.as_ref().map(|t| t.name),
            "ticks": self.ticks,
        })
    }

    /// Splitscreen's profile page, a joining player's (`split_profiles`).
    fn split_profiles_open(&self) -> bool {
        self.screen == Screen::Options
            && self.ui.options.page() == horizon_client::options_ui::Page::SplitProfiles
    }
    fn title_value(&self) -> Value {
        use horizon_client::title::Mode;
        if self.split_profiles_open() {
            return self.ui.options.split_profiles.state();
        }
        let Some(title) = &self.title else {
            return Value::Null;
        };
        let worlds = || {
            title
                .recent_worlds()
                .iter()
                .map(
                    |w| json!({ "folder": w.folder, "name": w.name, "creative": w.game_mode == 1 }),
                )
                .collect::<Vec<_>>()
        };
        match &title.mode {
            Mode::Main => json!({ "mode": "main", "worlds": worlds() }),
            Mode::Select {
                worlds: list,
                selected,
            } => json!({
                "mode": "select-world",
                "worlds": list.iter().map(|w| json!({ "folder": w.folder, "name": w.name, "creative": w.game_mode == 1, "corrupted": w.corrupted })).collect::<Vec<_>>(),
                "selected": selected,
            }),
            Mode::Create {
                name,
                seed,
                creative,
                focus,
                options,
            } => json!({
                "mode": "create-world",
                "tab": (["game", "world", "more"][options.tab.min(2)]),
                "fields": { "name": name, "seed": seed },
                "focus": match focus { 0 => "name", 1 => "seed", _ => "none" },
                "creative": creative,
                "hardcore": options.hardcore,
                "difficulty": horizon_client::title::DIFFICULTIES[options.difficulty()],
                "allow_commands": options.commands(*creative),
                "world_type": options.world_type.id(),
                "generate_structures": options.structures,
                "bonus_chest": options.bonus_chest(),
            }),
            Mode::CreateFlat {
                name,
                seed,
                options,
                flat,
                ..
            } => json!({
                "mode": "customize-flat",
                "fields": { "name": name, "seed": seed },
                "superflat": options.world_type == horizon_client::save::WorldPreset::Flat,
                "biome": flat.settings().biome,
                "layers": flat.settings().layers.iter().map(|layer| json!({
                    "block": layer.block,
                    "height": layer.height,
                })).collect::<Vec<_>>(),
            }),
            Mode::CreateBuffet { buffet, .. } => json!({
                "mode": "customize-single-biome",
                "biome": buffet.biome(),
                "query": buffet.query(),
            }),
            Mode::Edit {
                world,
                name,
                focused,
            } => {
                json!({ "mode": "edit-world", "folder": world.folder, "fields": { "name": name }, "focus": if *focused { "name" } else { "none" } })
            }
            Mode::Backup { world, .. } => {
                json!({ "mode": "creating-backup", "folder": world.folder })
            }
            Mode::Delete { world } => {
                json!({ "mode": "delete-world", "folder": world.folder, "name": world.name })
            }
            Mode::BackupBeforeUpgrade { world, .. } => {
                json!({ "mode": "backup-before-upgrade", "folder": world.folder, "name": world.name })
            }
            Mode::OptimizeConfirm {
                world, erase_cache, ..
            } => {
                json!({ "mode": "optimize-world-confirm", "folder": world.folder, "name": world.name, "erase_cache": erase_cache })
            }
            Mode::Optimize { world } => {
                json!({ "mode": "optimize-world", "folder": world.folder, "name": world.name })
            }
            Mode::LowDiskSpace { world } => {
                json!({ "mode": "low-disk-space", "folder": world.folder, "name": world.name })
            }
            Mode::Recover { world, issues } => {
                let issue = |i: &Option<horizon_client::save::DatIssue>| match i {
                    None => json!(null),
                    Some(horizon_client::save::DatIssue::Missing) => json!("missing file"),
                    Some(horizon_client::save::DatIssue::Unreadable(why)) => json!(why),
                };
                json!({ "mode": "recover-world", "folder": world.folder, "level_dat": issue(&issues[0]), "level_dat_old": issue(&issues[1]) })
            }
            Mode::Recovered { world, success } => {
                json!({ "mode": "recovery-done", "folder": world.folder, "success": success })
            }
            Mode::Error { message, .. } => {
                json!({ "mode": "world-operation-error", "message": message })
            }
            Mode::Multiplayer(horizon_client::multiplayer::Screen::Disconnected {
                reason, ..
            }) => {
                json!({ "mode": "multiplayer", "disconnected": reason })
            }
            Mode::Multiplayer(_) => json!({ "mode": "multiplayer" }),
            Mode::MultiplayerWarning { check } => {
                json!({ "mode": "multiplayer-warning", "check": check })
            }
            Mode::Onboarding(_) => json!({ "mode": "accessibility-onboarding" }),
            Mode::BanNotice(_) => json!({ "mode": "ban-notice" }),
        }
    }

    fn entities_value(&self, radius: f64, kind: Option<&str>) -> Value {
        let Some(p) = &self.player else {
            return json!({ "entities": [] });
        };
        let me = [p.position.x, p.position.y, p.position.z];
        let want = kind.map(namespaced);
        let mut list: Vec<(f64, Value)> = Vec::new();
        let mut push = |pos: [f64; 3], ty: &str, v: Value| {
            let d = dist(me, pos);
            if d <= radius && want.as_deref().is_none_or(|w| w == ty) {
                let mut v = v;
                v["type"] = json!(ty);
                v["pos"] = json!(pos);
                v["distance"] = json!(d);
                list.push((d, v));
            }
        };
        // CAN_BE_PICKED's isPickable (`!isRemoved`): a dead-but-tracked mob
        // — a fake player the server keeps at health 0 — is still here.
        for m in self.mobs.iter().filter(|m| !m.removed) {
            let mut v = json!({ "id": m.id, "kind": "mob", "health": m.health, "baby": m.is_baby(), "hurt_time": m.hurt_time });
            // VillagerData as this game knows it: on a client, what the
            // server's synched data says.
            if matches!(m.species(), Species::Villager | Species::ZombieVillager) {
                v["villager"] = json!({
                    "type": VILLAGER_TYPES[m.extra.variant as usize % VILLAGER_TYPES.len()],
                    "profession": PROFESSIONS[m.extra.variant2 as usize % PROFESSIONS.len()],
                    "level": m.extra.variant3,
                });
            }
            push(
                [m.position.x, m.position.y, m.position.z],
                m.kind.id.as_ref(),
                v,
            );
        }
        for item in &self.dropped {
            push(
                [item.position.x, item.position.y, item.position.z],
                "minecraft:item",
                json!({ "id": item.id, "kind": "item", "item": self.stack_value(&item.stack) }),
            );
        }
        for orb in &self.orbs {
            push(
                [orb.position.x, orb.position.y, orb.position.z],
                "minecraft:experience_orb",
                json!({ "kind": "orb", "value": orb.value, "velocity": [orb.velocity.x, orb.velocity.y, orb.velocity.z] }),
            );
        }
        list.sort_by(|a, b| a.0.total_cmp(&b.0));
        json!({ "entities": list.into_iter().map(|(_, v)| v).collect::<Vec<_>>() })
    }

    fn inventory_value(&self) -> Value {
        let armor = ["head", "chest", "legs", "feet"];
        json!({
            "selected": self.selected + 1,
            "held": self.held().map(|s| self.stack_value(s)),
            "hotbar": (0..9).map(|i| self.slot_value(i, &self.inventory[i])).collect::<Vec<_>>(),
            "main": (9..36).map(|i| self.slot_value(i, &self.inventory[i])).collect::<Vec<_>>(),
            "armor": armor.iter().enumerate().map(|(i, name)| {
                let mut v = self.slot_value(500 + i, &self.armor[i]);
                v["part"] = json!(name);
                v
            }).collect::<Vec<_>>(),
            "offhand": self.slot_value(504, &self.offhand),
            "carried": self.carried.as_ref().map(|s| self.stack_value(s)),
            "counts": self.inventory_counts(),
        })
    }

    fn screen_value(&mut self) -> Value {
        let buttons: Vec<Value> = widgets::last()
            .iter()
            .map(|w| json!({ "label": w.label, "enabled": w.enabled, "x": w.x, "y": w.y, "w": w.w, "h": w.h }))
            .collect();
        let mut v = json!({
            "screen": self.screen_label(),
            "buttons": buttons,
        });
        if let Some(chat) = &self.ui.chat_screen {
            v["chat_input"] = json!(chat.input.value);
            v["chat_suggestions"] = json!(chat.suggestions_visible());
        }
        if self.title.is_some() || self.split_profiles_open() {
            v["menu"] = self.title_value();
        }
        if self.screen == Screen::Inventory && self.title.is_none() {
            let mut slots = Vec::new();
            for i in self.slots() {
                let stack = if i == 200 {
                    self.craft_result()
                } else {
                    self.slot_mut(i).clone()
                };
                slots.push(self.slot_value(i, &stack));
            }
            v["slots"] = json!(slots);
            v["carried"] = json!(self.carried.as_ref().map(|s| self.stack_value(s)));
            v["craft_size"] = json!(self.craft_size);
            v["station"] = json!(self.station.as_ref().map(|s| format!("{:?}", s.kind)));
            v["container"] = json!(self.container);
            // The title a server gave the menu, span by span as it is drawn.
            if let Some(title) = self.remote_menu_title() {
                let spans = title.spans.iter().map(|s| {
                    json!({
                        "text": s.text,
                        "color": s.style.color.map(|c| format!("#{c:06X}")),
                        "bold": s.style.is_bold(),
                        "italic": s.style.is_italic(),
                    })
                });
                v["title"] = json!(spans.collect::<Vec<_>>());
            }
            // The recipe book's buttons (GUI pixels): the book button, then
            // the recipe buttons of the page, page arrows, filter and tabs.
            let book = self.recipe_book_targets();
            if !book.is_empty() {
                let (open, filtering) = self.recipe_book_flags().unwrap_or_default();
                v["recipe_book"] = json!({
                    "targets": book.iter().map(|t| t.rect).collect::<Vec<_>>(),
                    "known": self.recipe_book_known(),
                    "open": open,
                    "filtering": filtering,
                });
            }
            v["slot_ids"] = json!(
                "0-8 hotbar, 9-35 inventory, 100+ crafting grid (row-major), 200 crafting result, 300+ container, 500-503 armor, 504 offhand, 600+ station"
            );
        }
        v
    }

    fn messages_value(&self, n: usize) -> Value {
        let titles = &self.titles;
        json!({
            "chat": self.remote_chat(n).unwrap_or_else(|| self.ui.chat.recent_lines(n)),
            "order": "newest first",
            // The chat lines as drawn: colours, click and hover events.
            "styled": self.ui.chat.recent_components(n),
            "title": (titles.time > 0).then(|| titles.title.as_ref().map(|t| t.plain())).flatten(),
            "subtitle": (titles.time > 0).then(|| titles.subtitle.as_ref().map(|t| t.plain())).flatten(),
            "actionbar": (titles.overlay_time > 0).then(|| titles.overlay.as_ref().map(|t| t.plain())).flatten(),
        })
    }

    /// Whether a block stops movement, for the path finder.
    fn blocks_motion(&self, pos: [i32; 3]) -> bool {
        let f = self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]));
        f.full_collision || f.legacy_solid
    }
    fn walkable(&self, n: Node) -> bool {
        let below = self.world.block(n[0], n[1] - 1, n[2]);
        let lava = |p: [i32; 3]| {
            &*self
                .res
                .facts()
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                == "minecraft:lava"
        };
        self.world.chunks.contains_key(&(n[0] >> 4, n[2] >> 4))
            && (self.blocks_motion([n[0], n[1] - 1, n[2]])
                || self.res.facts().get(below).fluid != horizon_client::blocks::Fluid::None)
            && !self.blocks_motion(n)
            && !self.blocks_motion([n[0], n[1] + 1, n[2]])
            && !lava(n)
            && !lava([n[0], n[1] - 1, n[2]])
    }
    /// A* over standable feet positions: steps up one block, drops up to three.
    fn find_path(&self, from: Node, goal: [f64; 3], near: f64) -> Option<Vec<Node>> {
        use std::{cmp::Reverse, collections::BinaryHeap};
        let reached = |n: Node| {
            let c = [
                f64::from(n[0]) + 0.5,
                f64::from(n[1]),
                f64::from(n[2]) + 0.5,
            ];
            ((c[0] - goal[0]).powi(2) + (c[2] - goal[2]).powi(2)).sqrt() <= near.max(0.5)
                && (c[1] - goal[1]).abs() <= near.max(1.0) + 0.5
        };
        let h = |n: Node| {
            ((f64::from(n[0]) + 0.5 - goal[0]).abs()
                + (f64::from(n[1]) - goal[1]).abs()
                + (f64::from(n[2]) + 0.5 - goal[2]).abs())
                * 10.0
        };
        let mut open = BinaryHeap::new();
        let mut cost: HashMap<Node, u32> = HashMap::new();
        let mut came: HashMap<Node, Node> = HashMap::new();
        cost.insert(from, 0);
        open.push(Reverse((h(from) as u32, from)));
        let mut expanded = 0;
        while let Some(Reverse((_, n))) = open.pop() {
            if reached(n) {
                let mut path = vec![n];
                let mut at = n;
                while let Some(&p) = came.get(&at) {
                    path.push(p);
                    at = p;
                }
                path.reverse();
                return Some(path);
            }
            expanded += 1;
            if expanded > 30_000 {
                return None;
            }
            let base = cost[&n];
            for d in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
                for dy in [0, 1, -1, -2, -3] {
                    let m = [n[0] + d[0], n[1] + dy, n[2] + d[1]];
                    // A step up needs head room above the start; a drop needs
                    // the column in front clear down to the landing.
                    let clear = match dy {
                        1 => !self.blocks_motion([n[0], n[1] + 2, n[2]]),
                        d if d < 0 => {
                            (m[1]..=n[1] + 1).all(|y| !self.blocks_motion([m[0], y, m[2]]))
                        }
                        _ => true,
                    };
                    if !clear || !self.walkable(m) {
                        continue;
                    }
                    let water = self
                        .res
                        .facts()
                        .get(self.world.block(m[0], m[1], m[2]))
                        .fluid
                        != horizon_client::blocks::Fluid::None;
                    let step = 10 + if dy != 0 { 5 } else { 0 } + if water { 30 } else { 0 };
                    let c = base + step;
                    if cost.get(&m).is_none_or(|&old| c < old) {
                        cost.insert(m, c);
                        came.insert(m, n);
                        open.push(Reverse((c + h(m) as u32, m)));
                    }
                    break;
                }
            }
        }
        None
    }

    /// Point the view at `at` from the eye.
    fn face_point(&mut self, at: [f64; 3]) -> Result<()> {
        let eye = self.eye_position().ok_or("no player")?;
        let (yaw, pitch) = yaw_pitch(eye, at);
        self.yaw = yaw;
        self.pitch = pitch.clamp(-90.0, 90.0);
        Ok(())
    }
    /// Aim at a face of `pos` the crosshair can reach; the face hit.
    fn aim_block(&mut self, pos: [i32; 3], face: Option<[i32; 3]>) -> Result<[i32; 3]> {
        let eye = self.eye_position().ok_or("no player")?;
        let centre = pos.map(|v| f64::from(v) + 0.5);
        let mut faces: Vec<[i32; 3]> = match face {
            Some(f) => vec![f],
            None => FACES.iter().map(|(_, d)| *d).collect(),
        };
        // Faces turned towards the eye first, nearest first.
        faces.sort_by(|a, b| {
            let point = |f: &[i32; 3]| [0, 1, 2].map(|i| centre[i] + f64::from(f[i]) * 0.45);
            dist(eye, point(a)).total_cmp(&dist(eye, point(b)))
        });
        for f in faces {
            for inset in [0.45, 0.3, 0.0] {
                let point = [0, 1, 2].map(|i| centre[i] + f64::from(f[i]) * inset);
                let (yaw, pitch) = yaw_pitch(eye, point);
                let dir = super::view_vector(yaw, pitch);
                if let Some(hit) = self.clip_ray(eye, dir, self.reach())
                    && hit.pos == pos
                    && (face.is_none() || hit.face == f)
                {
                    self.yaw = yaw;
                    self.pitch = pitch;
                    return Ok(hit.face);
                }
            }
        }
        let d = dist(eye, centre);
        if d > self.reach() + 0.9 {
            return Err(format!(
                "block {pos:?} is {d:.1} blocks away, out of reach ({:.1}); goto it first",
                self.reach()
            )
            .into());
        }
        Err(format!("no face of block {pos:?} is visible from here").into())
    }

    /// A left or right click, as the mouse would.
    fn agent_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        self.play_mouse(button, pressed)
    }
    fn release_agent_keys(&mut self) {
        for k in MOVE_KEYS {
            self.keys.remove(&k);
        }
        if self.left {
            let _ = self.agent_mouse(MouseButton::Left, false);
        }
        if self.right {
            let _ = self.agent_mouse(MouseButton::Right, false);
        }
    }

    /// Synthesize the pointer moving to GUI pixel (x, y) and a click there.
    pub(super) fn synth_click(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        x: f32,
        y: f32,
        button: MouseButton,
    ) -> Result<()> {
        self.synth_drag(event_loop, &[(x, y)], button)
    }
    /// The window events of a press at the first GUI point, moves through
    /// the others and a release at the last (one point is a click).
    fn synth_drag(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        points: &[(f32, f32)],
        button: MouseButton,
    ) -> Result<()> {
        let id = self.window.as_ref().ok_or("no window yet")?.id();
        let scale = self
            .renderer
            .as_ref()
            .map(|r| horizon_client::hud::HudBatch::new([r.config.width, r.config.height]).scale)
            .ok_or("no renderer yet")?;
        let device_id = DeviceId::dummy();
        for (i, &(x, y)) in points.iter().enumerate() {
            let position = PhysicalPosition::new(f64::from(x * scale), f64::from(y * scale));
            self.handle_window_event(
                event_loop,
                id,
                WindowEvent::CursorMoved {
                    device_id,
                    position,
                },
            );
            if i == 0 {
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::MouseInput {
                        device_id,
                        state: ElementState::Pressed,
                        button,
                    },
                );
            }
        }
        self.handle_window_event(
            event_loop,
            id,
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Released,
                button,
            },
        );
        Ok(())
    }

    fn start_task(
        &mut self,
        name: &'static str,
        kind: TaskKind,
        reply: &Sender<String>,
        timeout: f64,
    ) -> Result<Handled> {
        self.agent.task = Some(Task {
            kind,
            reply: reply.clone(),
            deadline: Instant::now() + Duration::from_secs_f64(timeout.clamp(0.05, 3600.0)),
            name,
        });
        Ok(Handled::Deferred)
    }

    /// Agent commands; `Handled::No` when `words[0]` is not one of them.
    pub(super) fn agent_command(
        &mut self,
        words: &[&str],
        line: &str,
        reply: &Sender<String>,
        event_loop: &dyn EventLoopCtl,
    ) -> Result<Handled> {
        let name = words[0];
        let a = parse_args(&words[1..]);
        let ok = |v: Value| Ok(Handled::Reply(v));
        let blocking = matches!(
            name,
            "move"
                | "jump"
                | "goto"
                | "break"
                | "place"
                | "interact"
                | "use-item"
                | "attack"
                | "wait"
        );
        if blocking && let Some(task) = &self.agent.task {
            return Err(format!("busy with {}; wait for it or send stop", task.name).into());
        }
        match name {
            "state" => ok(self.state_value()),
            "looking-at" | "target" => {
                self.need_world()?;
                ok(self.looking_at())
            }
            "entities" => {
                self.need_world()?;
                let radius = a.pos.first().map_or(Ok(32.0), |r| r.parse::<f64>())?;
                ok(self.entities_value(radius, a.pos.get(1).copied()))
            }
            "block" => {
                self.need_world()?;
                ok(self.block_json(a.block_pos(0)?))
            }
            "blocks" => {
                self.need_world()?;
                let (p, q) = (a.block_pos(0)?, a.block_pos(3)?);
                let lo = [0, 1, 2].map(|i| p[i].min(q[i]));
                let hi = [0, 1, 2].map(|i| p[i].max(q[i]));
                let volume: i64 = (0..3).map(|i| i64::from(hi[i] - lo[i] + 1)).product();
                if volume > 32768 {
                    return Err(format!("{volume} blocks; at most 32768 per request").into());
                }
                let mut list = Vec::new();
                let mut census: BTreeMap<String, u32> = BTreeMap::new();
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        for x in lo[0]..=hi[0] {
                            let state = self.world.block(x, y, z);
                            if self.res.facts().is_air(state) {
                                continue;
                            }
                            let name = self.res.facts().get(state).name.to_string();
                            *census.entry(name).or_insert(0) += 1;
                            list.push(json!([x, y, z, self.state_string(state)]));
                        }
                    }
                }
                ok(json!({ "from": lo, "to": hi, "counts": census, "blocks": list }))
            }
            "light" => {
                self.need_world()?;
                let (p, q) = (a.block_pos(0)?, a.block_pos(3)?);
                let lo = [0, 1, 2].map(|i| p[i].min(q[i]));
                let hi = [0, 1, 2].map(|i| p[i].max(q[i]));
                let volume: i64 = (0..3).map(|i| i64::from(hi[i] - lo[i] + 1)).product();
                if volume > 32768 {
                    return Err(format!("{volume} blocks; at most 32768 per request").into());
                }
                // One hex digit per block, x fastest, then z, then y; `.`
                // outside the loaded chunks.
                let (mut sky, mut block) = (String::new(), String::new());
                for y in lo[1]..=hi[1] {
                    for z in lo[2]..=hi[2] {
                        for x in lo[0]..=hi[0] {
                            let packed = self
                                .world
                                .chunks
                                .get(&(x >> 4, z >> 4))
                                .filter(|_| {
                                    (self.world.bottom_y()..self.world.top_y()).contains(&y)
                                })
                                .map(|c| c.light(x & 15, y, z & 15));
                            let digit = |v: u8| char::from_digit(u32::from(v), 16).unwrap_or('.');
                            sky.push(packed.map_or('.', |v| digit(v >> 4)));
                            block.push(packed.map_or('.', |v| digit(v & 15)));
                        }
                    }
                }
                ok(json!({ "from": lo, "to": hi, "sky": sky, "block": block }))
            }
            "find-block" => {
                self.need_world()?;
                let want = namespaced(a.get(0, "BLOCK")?);
                let radius = a
                    .pos
                    .get(1)
                    .map_or(Ok(32), |r| r.parse::<i32>())?
                    .clamp(1, 64);
                let eye = self.eye_position().ok_or("no player")?;
                let c = eye.map(|v| v.floor() as i32);
                let mut found = Vec::new();
                for y in (c[1] - radius).max(-64)..=(c[1] + radius).min(319) {
                    for z in c[2] - radius..=c[2] + radius {
                        for x in c[0] - radius..=c[0] + radius {
                            let state = self.world.block(x, y, z);
                            if *self.res.facts().get(state).name == *want {
                                let d = dist(eye, [x, y, z].map(|v| f64::from(v) + 0.5));
                                found.push((d, [x, y, z]));
                            }
                        }
                    }
                }
                found.sort_by(|a, b| a.0.total_cmp(&b.0));
                let total = found.len();
                ok(json!({
                    "block": want,
                    "count": total,
                    "nearest": found.iter().take(16).map(|(d, p)| json!({ "pos": p, "distance": d })).collect::<Vec<_>>(),
                }))
            }
            "inventory" => {
                self.need_world()?;
                ok(self.inventory_value())
            }
            "screen" => ok(self.screen_value()),
            "messages" => {
                let n = a.pos.first().map_or(Ok(20), |n| n.parse::<usize>())?;
                ok(self.messages_value(n))
            }
            "move" => {
                self.playing()?;
                let key = match a.get(0, "DIR")? {
                    "forward" | "w" => KeyCode::KeyW,
                    "back" | "backward" | "s" => KeyCode::KeyS,
                    "left" | "a" => KeyCode::KeyA,
                    "right" | "d" => KeyCode::KeyD,
                    other => {
                        return Err(
                            format!("DIR is forward, back, left or right, not {other}").into()
                        );
                    }
                };
                let seconds = a.f64(1, "SECONDS")?.clamp(0.0, 600.0);
                let mut keys = vec![key];
                if a.has("sprint") {
                    keys.push(KeyCode::ControlLeft);
                }
                if a.has("sneak") {
                    keys.push(KeyCode::ShiftLeft);
                }
                if a.has("jump") {
                    keys.push(KeyCode::Space);
                }
                for &k in &keys {
                    self.keys.insert(k);
                }
                let from = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
                let until = Instant::now() + Duration::from_secs_f64(seconds);
                self.start_task(
                    "move",
                    TaskKind::Move { keys, until, from },
                    reply,
                    seconds + 5.0,
                )
            }
            "jump" => {
                self.playing()?;
                self.keys.insert(KeyCode::Space);
                let from_y = self.player.as_ref().map_or(0.0, |p| p.position.y);
                let release_tick = self.ticks + 3;
                self.start_task(
                    "jump",
                    TaskKind::Jump {
                        release_tick,
                        from_y,
                    },
                    reply,
                    5.0,
                )
            }
            "goto" => {
                self.playing()?;
                let target = [a.f64(0, "X")?, a.f64(1, "Y")?, a.f64(2, "Z")?];
                let near = a.num("near", 0.6)?.max(0.3);
                let timeout = a.num("timeout", 60.0)?;
                let p = self.player.as_ref().ok_or("no player")?.position;
                let from = [p.x.floor() as i32, p.y.round() as i32, p.z.floor() as i32];
                let (path, warning) = match self.find_path(from, target, near) {
                    Some(path) => (path, None),
                    None if a.has("direct") => (
                        Vec::new(),
                        Some("no path found through loaded terrain; walking straight".to_owned()),
                    ),
                    None => {
                        return Err(format!(
                            "no walking path to {target:?} through loaded terrain (climbs of more than one block, drops of more than three, lava and unloaded chunks are avoided); pick a nearer or lower spot, raise --near, or use --direct to walk straight at it"
                        )
                        .into());
                    }
                };
                let goto = Goto {
                    target,
                    near,
                    sprint: a.has("sprint"),
                    path,
                    next: 1,
                    progress_tick: self.ticks,
                    replans: 0,
                    jump_until: 0,
                    warning,
                };
                self.start_task("goto", TaskKind::Goto(Box::new(goto)), reply, timeout)
            }
            "mouse-move" => {
                self.turning()?;
                let motion = (a.f64(0, "DX")?, a.f64(1, "DY")?);
                self.start_glide(motion, 0.0, None, reply)
            }
            "turn" if a.has("over") => {
                self.turning()?;
                let (dyaw, dpitch) = (a.f64(0, "DYAW")?, a.f64(1, "DPITCH")?);
                let motion =
                    horizon_client::mouse::motion_for_turn(dyaw, dpitch, self.look_options());
                self.start_glide(motion, a.num("over", 0.0)?, None, reply)
            }
            "look-at" => {
                self.need_world()?;
                let over = a.has("over").then(|| a.num("over", 0.0)).transpose()?;
                if over.is_some() {
                    self.turning()?;
                }
                let before = (self.yaw, self.pitch);
                let (aim, value) = match a.pos.first().copied() {
                    Some("block") => {
                        let pos = a.block_pos(1)?;
                        let face = a.pos.get(4).map(|f| face_dir(f)).transpose()?;
                        let hit = self.aim_block(pos, face)?;
                        (
                            "block",
                            json!({ "ok": true, "yaw": self.yaw, "pitch": self.pitch, "face": face_name(hit) }),
                        )
                    }
                    Some("entity") => {
                        let id: u32 = a.get(1, "ID")?.parse()?;
                        let m = self
                            .mobs
                            .iter()
                            .find(|m| m.id == id)
                            .ok_or_else(|| format!("no entity {id}"))?;
                        let c = m.center();
                        self.face_point(c)?;
                        (
                            "entity",
                            json!({ "ok": true, "yaw": self.yaw, "pitch": self.pitch, "target": self.looking_at() }),
                        )
                    }
                    _ => {
                        let at = [a.f64(0, "X")?, a.f64(1, "Y")?, a.f64(2, "Z")?];
                        self.face_point(at)?;
                        (
                            "point",
                            json!({ "ok": true, "yaw": self.yaw, "pitch": self.pitch }),
                        )
                    }
                };
                let Some(over) = over else {
                    return ok(value);
                };
                // Turn there with the mouse instead: the shorter way round.
                let target = (self.yaw, self.pitch);
                (self.yaw, self.pitch) = before;
                let dyaw = f64::from(target.0 - before.0 + 180.0).rem_euclid(360.0) - 180.0;
                let dpitch = f64::from(target.1 - before.1);
                let motion =
                    horizon_client::mouse::motion_for_turn(dyaw, dpitch, self.look_options());
                self.start_glide(motion, over, Some(aim), reply)
            }
            "attack" => {
                self.playing()?;
                let id = a.pos.first().map(|i| i.parse::<u32>()).transpose()?;
                if let Some(id) = id {
                    let m = self
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .ok_or_else(|| format!("no entity {id}"))?;
                    let c = m.center();
                    self.face_point(c)?;
                }
                let target = self
                    .mob_hit()
                    .map(|(i, _)| (self.mobs[i].id, self.mobs[i].health));
                if id.is_some() && target.map(|t| t.0) != id {
                    return Err("that entity is out of reach or hidden; get closer".into());
                }
                self.agent_mouse(MouseButton::Left, true)?;
                self.agent_mouse(MouseButton::Left, false)?;
                let report_tick = self.ticks + 4;
                self.start_task(
                    "attack",
                    TaskKind::Attack {
                        id: target.map(|t| t.0),
                        health: target.map(|t| t.1),
                        report_tick,
                    },
                    reply,
                    5.0,
                )
            }
            "break" => {
                self.playing()?;
                let pos = a.block_pos(0)?;
                let before = self.world.block(pos[0], pos[1], pos[2]);
                if self.res.facts().is_air(before) {
                    return Err(format!("there is no block at {pos:?}").into());
                }
                self.aim_block(pos, None)?;
                let timeout = a.num("timeout", 30.0)?;
                let task = Break {
                    pos,
                    before,
                    items_before: self.dropped.iter().map(|i| i.id).collect(),
                    inventory: self.inventory_counts(),
                    started_tick: self.ticks,
                    collect_until: None,
                    broke_tick: 0,
                    drops: BTreeMap::new(),
                };
                self.agent_mouse(MouseButton::Left, true)?;
                self.start_task("break", TaskKind::Break(Box::new(task)), reply, timeout)
            }
            "place" => {
                self.playing()?;
                let pos = a.block_pos(0)?;
                if let Some(Some(item)) = a.flags.get("item") {
                    self.select_item(&namespaced(item))?;
                }
                let held = self
                    .held()
                    .ok_or("nothing in hand to place; select an item")?;
                let held = held.item.clone();
                let before = self.world.block(pos[0], pos[1], pos[2]);
                if !self.res.facts().get(before).replaceable && !self.res.facts().is_air(before) {
                    return Err(format!(
                        "{pos:?} holds {}; break it first",
                        self.res.facts().get(before).name
                    )
                    .into());
                }
                // A replaceable block there (grass, snow) is clicked itself, as
                // a player would; otherwise the face of a neighbour touching it.
                let mut aimed = !self.res.facts().is_air(before)
                    && !self.res.facts().get(before).outline.is_empty()
                    && self.aim_block(pos, None).is_ok();
                for (_, d) in FACES.iter().filter(|_| !aimed) {
                    let d = *d;
                    let support = add(pos, d);
                    let state = self.world.block(support[0], support[1], support[2]);
                    let f = self.res.facts().get(state);
                    if self.res.facts().is_air(state)
                        || f.replaceable
                        || f.fluid != horizon_client::blocks::Fluid::None && !f.waterlogged
                    {
                        continue;
                    }
                    let face = d.map(|v| -v);
                    if self.aim_block(support, Some(face)).is_ok() {
                        aimed = true;
                        break;
                    }
                }
                if !aimed {
                    return Err(format!(
                        "no visible face to place {held} against at {pos:?}; it needs a solid neighbour in reach"
                    )
                    .into());
                }
                let inventory = self.inventory_counts();
                self.agent_mouse(MouseButton::Right, true)?;
                self.agent_mouse(MouseButton::Right, false)?;
                let report_tick = self.ticks + 2;
                self.start_task(
                    "place",
                    TaskKind::Click {
                        pos: Some(pos),
                        before: Some(before),
                        inventory,
                        report_tick,
                        release: None,
                        place: true,
                    },
                    reply,
                    5.0,
                )
            }
            "interact" | "use-item" => {
                self.playing()?;
                let hold = a.num("hold", 0.0)?;
                let pos = if name == "interact" {
                    let pos = a.block_pos(0)?;
                    let face = a.pos.get(3).map(|f| face_dir(f)).transpose()?;
                    self.aim_block(pos, face)?;
                    Some(pos)
                } else {
                    None
                };
                let before = pos.map(|p| self.world.block(p[0], p[1], p[2]));
                let inventory = self.inventory_counts();
                self.agent_mouse(MouseButton::Right, true)?;
                let release = if hold > 0.0 {
                    Some(Instant::now() + Duration::from_secs_f64(hold))
                } else {
                    self.agent_mouse(MouseButton::Right, false)?;
                    None
                };
                let report_tick = self.ticks + 2;
                self.start_task(
                    if name == "interact" {
                        "interact"
                    } else {
                        "use-item"
                    },
                    TaskKind::Click {
                        pos,
                        before,
                        inventory,
                        report_tick,
                        release,
                        place: false,
                    },
                    reply,
                    hold + 5.0,
                )
            }
            "select" => {
                self.need_world()?;
                let what = a.get(0, "SLOT or ITEM")?;
                match what.parse::<usize>() {
                    Ok(n) if (1..=9).contains(&n) => self.selected = n - 1,
                    Ok(_) => return Err("hotbar slots are 1..9".into()),
                    Err(_) => self.select_item(&namespaced(what))?,
                }
                ok(
                    json!({ "ok": true, "selected": self.selected + 1, "held": self.held().map(|s| self.stack_value(s)) }),
                )
            }
            "drop" => {
                self.playing()?;
                let all = a.pos.first() == Some(&"all");
                let before = self.held().map(|s| self.stack_value(s));
                if self.remote.is_some() {
                    // As Q does on a server (and on the single-player server
                    // thread): the server throws the stack, the client predicts it.
                    if !self.spectator() {
                        self.remote_drop(all);
                    }
                } else {
                    self.drop_held(all);
                }
                ok(
                    json!({ "ok": true, "dropped_from": before, "held": self.held().map(|s| self.stack_value(s)) }),
                )
            }
            "open-inventory" => {
                self.playing()?;
                self.key_pressed(KeyCode::KeyE)?;
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            "click-button" => {
                let label = line
                    .trim_start()
                    .strip_prefix("click-button")
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"');
                if label.is_empty() {
                    return Err("click-button LABEL".into());
                }
                let w = widgets::find(&widgets::last(), label)?;
                if !w.enabled {
                    return Err(format!("button {:?} is disabled", w.label).into());
                }
                self.synth_click(
                    event_loop,
                    w.x + w.w / 2.0,
                    w.y + w.h / 2.0,
                    MouseButton::Left,
                )?;
                ok(json!({ "ok": true, "clicked": w.label, "screen": self.screen_label() }))
            }
            "drag-at" => {
                let right = a.pos.first() == Some(&"right");
                let numbers = &a.pos[usize::from(right)..];
                let points = numbers
                    .chunks(2)
                    .map(|c| match c {
                        [x, y] => Ok((x.parse::<f32>()?, y.parse::<f32>()?)),
                        _ => Err("drag-at needs X Y pairs".into()),
                    })
                    .collect::<Result<Vec<_>>>()?;
                if points.is_empty() {
                    return Err("drag-at needs X Y pairs".into());
                }
                let button = if right {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                self.synth_drag(event_loop, &points, button)?;
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            "scroll-at" => {
                let (x, y) = (a.f64(0, "X")? as f32, a.f64(1, "Y")? as f32);
                let steps = a.f64(2, "STEPS")? as f32;
                let id = self.window.as_ref().ok_or("no window yet")?.id();
                let scale = self
                    .renderer
                    .as_ref()
                    .map(|r| {
                        horizon_client::hud::HudBatch::new([r.config.width, r.config.height]).scale
                    })
                    .ok_or("no renderer yet")?;
                let device_id = DeviceId::dummy();
                let position = PhysicalPosition::new(f64::from(x * scale), f64::from(y * scale));
                self.handle_window_event(
                    event_loop,
                    id,
                    WindowEvent::CursorMoved {
                        device_id,
                        position,
                    },
                );
                for _ in 0..steps.abs() as u32 {
                    self.handle_window_event(
                        event_loop,
                        id,
                        WindowEvent::MouseWheel {
                            device_id,
                            delta: MouseScrollDelta::LineDelta(0.0, steps.signum()),
                            phase: TouchPhase::Moved,
                        },
                    );
                }
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            "click-at" => {
                let (x, y) = (a.f64(0, "X")? as f32, a.f64(1, "Y")? as f32);
                let button = if a.pos.get(2) == Some(&"right") {
                    MouseButton::Right
                } else {
                    MouseButton::Left
                };
                self.synth_click(event_loop, x, y, button)?;
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            // Splitscreen's profile page, over a joining player's title screen.
            "type" if self.split_profiles_open() => {
                let text = line
                    .trim_start()
                    .strip_prefix("type")
                    .unwrap_or("")
                    .strip_prefix(' ')
                    .unwrap_or("");
                let page = &mut self.ui.options.split_profiles;
                if text == "\\b" {
                    page.key(winit::keyboard::KeyCode::Backspace, None);
                } else {
                    page.type_text(text);
                }
                ok(json!({ "ok": true, "menu": self.title_value() }))
            }
            "type" if self.title.is_some() => {
                // type [--field name|seed] [--replace] TEXT on the title screens.
                let mut text = line
                    .trim_start()
                    .strip_prefix("type")
                    .unwrap_or("")
                    .strip_prefix(' ')
                    .unwrap_or("");
                let mut field = None;
                let mut replace = false;
                loop {
                    if let Some(rest) = text.strip_prefix("--replace") {
                        replace = true;
                        text = rest.trim_start();
                    } else if let Some(rest) = text.strip_prefix("--field") {
                        let rest = rest.trim_start();
                        let (name, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                        field = Some(name.to_owned());
                        text = rest;
                    } else {
                        break;
                    }
                }
                let title = self.title.as_mut().ok_or("no title screen")?;
                if let Some(field) = &field {
                    match &mut title.mode {
                        horizon_client::title::Mode::Create { focus, options, .. } => {
                            // Each box is on its own tab.
                            *focus = match field.as_str() {
                                "name" => {
                                    options.tab = 0;
                                    0
                                }
                                "seed" => {
                                    options.tab = 1;
                                    1
                                }
                                other => {
                                    return Err(format!(
                                        "the create-world fields are name and seed, not {other}"
                                    )
                                    .into());
                                }
                            }
                        }
                        horizon_client::title::Mode::Edit { focused, .. } if field == "name" => {
                            *focused = true
                        }
                        _ => {
                            return Err(
                                "--field works on the create-world or edit-world screen".into()
                            );
                        }
                    }
                }
                if replace {
                    for _ in 0..64 {
                        title.backspace();
                    }
                }
                if text == "\\b" {
                    title.backspace();
                } else {
                    title.type_text(text);
                }
                ok(json!({ "ok": true, "menu": self.title_value() }))
            }
            "command" => {
                self.need_world()?;
                let text = line
                    .trim_start()
                    .strip_prefix("command")
                    .unwrap_or("")
                    .trim()
                    .trim_start_matches('/');
                if text.is_empty() {
                    return Err("command TEXT".into());
                }
                let command = format!("/{text}");
                // Single player's console is its server thread's.
                if let Some(server) = &self.integrated {
                    let chat = server
                        .call(move |g| g.owner_console_command(&command))
                        .ok_or("the integrated server stopped")?;
                    return ok(json!({ "ok": true, "chat": chat }));
                }
                if self.remote.is_some() {
                    self.send_chat(&command);
                    return ok(json!({ "ok": true }));
                }
                self.as_console(|g| g.handle_chat_input(&command));
                ok(json!({ "ok": true, "chat": self.ui.chat.recent_lines(5) }))
            }
            "respawn" if self.remote.is_none() => {
                if self.screen != Screen::Dead {
                    return Err("respawn needs the death screen".into());
                }
                let key = if self.level.hardcore {
                    "deathScreen.spectate"
                } else {
                    "deathScreen.respawn"
                };
                let label = horizon_client::text::Text::translate(key, &[]).plain();
                let w = widgets::find(&widgets::last(), &label)?;
                if !w.enabled {
                    // DeathScreen enables its buttons after 20 ticks.
                    return Err(
                        "the Respawn button is not active yet; try again in a second (wait 20)"
                            .into(),
                    );
                }
                self.synth_click(
                    event_loop,
                    w.x + w.w / 2.0,
                    w.y + w.h / 2.0,
                    MouseButton::Left,
                )?;
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            "pause" => {
                self.playing()?;
                self.key_pressed(KeyCode::Escape)?;
                ok(json!({ "ok": true, "screen": self.screen_label() }))
            }
            "wait" => {
                self.need_world()?;
                let ticks: u64 = a.get(0, "TICKS")?.parse()?;
                let until_tick = self.ticks + ticks;
                let timeout = ticks as f64 / 20.0 * 4.0 + 10.0;
                self.start_task("wait", TaskKind::Wait { until_tick }, reply, timeout)
            }
            "stop" => {
                let cancelled = self.agent.task.take().map(|t| {
                    let _ = t.reply.send(
                        json!({ "error": "cancelled by stop", "cancelled": true }).to_string(),
                    );
                    t.name
                });
                let turning = self.agent.glide.take().map(|g| {
                    let _ = g.reply.send(
                        json!({ "error": "cancelled by stop", "cancelled": true }).to_string(),
                    );
                    "turn"
                });
                let cancelled = cancelled.or(turning);
                self.release_agent_keys();
                ok(json!({ "ok": true, "cancelled": cancelled }))
            }
            _ => Ok(Handled::No),
        }
    }

    /// Select the hotbar slot holding `item`.
    fn select_item(&mut self, item: &str) -> Result<()> {
        if let Some(i) =
            (0..9).find(|&i| self.inventory[i].as_ref().is_some_and(|s| &*s.item == item))
        {
            self.selected = i;
            return Ok(());
        }
        if self.inventory.iter().flatten().any(|s| &*s.item == item) {
            return Err(format!(
                "{item} is in the main inventory, not the hotbar; move it with open-inventory and click-slot"
            )
            .into());
        }
        Err(format!("no {item} in the inventory").into())
    }

    /// Mouse motion turns the view only in the world with no screen open
    /// (`MouseHandler` turns the player while the mouse is grabbed).
    fn turning(&self) -> Result<()> {
        self.playing()?;
        if self.agent.glide.is_some() {
            return Err("already turning; wait for it or send stop".into());
        }
        Ok(())
    }

    fn start_glide(
        &mut self,
        total: (f64, f64),
        over: f64,
        aim: Option<&'static str>,
        reply: &Sender<String>,
    ) -> Result<Handled> {
        if !over.is_finite() || !(0.0..=60.0).contains(&over) {
            return Err("--over takes 0 to 60 seconds".into());
        }
        if !(total.0.is_finite() && total.1.is_finite()) {
            return Err("the motion must be finite".into());
        }
        let now = Instant::now();
        let over = Duration::from_secs_f64(over);
        self.agent.glide = Some(Glide {
            total,
            fed: (0.0, 0.0),
            started: now,
            over,
            fed_frame: self.frames,
            deadline: now + over + Duration::from_secs(10),
            reply: reply.clone(),
            aim,
        });
        self.advance_glide();
        Ok(Handled::Deferred)
    }

    /// Feed the motion due by now (eased over the glide's time); reply once
    /// all of it has been turned by a frame.
    fn advance_glide(&mut self) {
        let Some(mut g) = self.agent.glide.take() else {
            return;
        };
        let done = |game: &Self, extra: Value| {
            let mut v = json!({
                "ok": true,
                "yaw": game.yaw,
                "pitch": game.pitch,
                "motion": [g.total.0, g.total.1],
            });
            if let (Value::Object(m), Value::Object(e)) = (&mut v, extra) {
                m.extend(e);
            }
            v
        };
        if let Err(e) = self.playing() {
            let _ = g.reply.send(
                json!({ "error": format!("stopped turning: {e}"), "yaw": self.yaw, "pitch": self.pitch })
                    .to_string(),
            );
            return;
        }
        let now = Instant::now();
        let t = if g.over.is_zero() {
            1.0
        } else {
            now.duration_since(g.started).as_secs_f64() / g.over.as_secs_f64()
        };
        let f = horizon_client::mouse::ease_in_out(t);
        let due = (g.total.0 * f, g.total.1 * f);
        let step = (due.0 - g.fed.0, due.1 - g.fed.1);
        if step != (0.0, 0.0) {
            self.mouse.add_motion(step.0, step.1);
            self.agent_pointer = true;
            g.fed = due;
            g.fed_frame = self.frames;
        }
        if t >= 1.0 && self.frames > g.fed_frame {
            let extra = match g.aim {
                Some("block") | Some("entity") => json!({ "target": self.looking_at() }),
                _ => json!({}),
            };
            let _ = g.reply.send(done(self, extra).to_string());
            return;
        }
        if now >= g.deadline {
            let _ = g.reply.send(
                json!({ "error": "no frame was drawn to turn the view", "yaw": self.yaw, "pitch": self.pitch })
                    .to_string(),
            );
            return;
        }
        self.agent.glide = Some(g);
    }

    /// Advance the running agent task; reply when it ends.
    pub(super) fn advance_agent(&mut self) {
        self.advance_glide();
        let Some(mut task) = self.agent.task.take() else {
            return;
        };
        let timed_out = Instant::now() >= task.deadline;
        let result = self.step_task(&mut task.kind, timed_out);
        match result {
            Some(value) => {
                if task.kind.releases_input() {
                    self.release_agent_keys();
                }
                let _ = task.reply.send(value.to_string());
            }
            None => self.agent.task = Some(task),
        }
    }

    fn position(&self) -> [f64; 3] {
        self.player
            .as_ref()
            .map_or([0.0; 3], |p| [p.position.x, p.position.y, p.position.z])
    }

    /// One frame of a task: Some(reply) when it is over.
    fn step_task(&mut self, kind: &mut TaskKind, timed_out: bool) -> Option<Value> {
        // Leaving the world (death, a screen, the title) ends movement tasks.
        let interrupted = if self.title.is_some() || self.player.is_none() {
            Some("the world closed")
        } else if self.screen == Screen::Dead {
            Some("the player died")
        } else {
            None
        };
        match kind {
            TaskKind::Move { keys, until, from } => {
                if let Some(why) = interrupted {
                    return Some(json!({ "error": why, "position": self.position() }));
                }
                for &k in keys.iter() {
                    self.keys.insert(k);
                }
                if Instant::now() < *until && !timed_out {
                    return None;
                }
                let to = self.position();
                Some(
                    json!({ "ok": true, "from": [from.x, from.y, from.z], "position": to,
                             "moved": dist([from.x, from.y, from.z], to) }),
                )
            }
            TaskKind::Wait { until_tick } => {
                if self.ticks >= *until_tick {
                    Some(json!({ "ok": true, "ticks": self.ticks }))
                } else if timed_out {
                    Some(
                        json!({ "error": "game ticks are not advancing (paused?)", "ticks": self.ticks }),
                    )
                } else {
                    None
                }
            }
            TaskKind::Jump {
                release_tick,
                from_y,
            } => {
                if self.ticks >= *release_tick || timed_out {
                    self.keys.remove(&KeyCode::Space);
                    let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
                    if on_ground || timed_out {
                        return Some(
                            json!({ "ok": true, "from_y": from_y, "position": self.position() }),
                        );
                    }
                }
                None
            }
            TaskKind::Goto(goto) => {
                if let Some(why) = interrupted {
                    return Some(json!({ "error": why, "position": self.position() }));
                }
                if self.screen != Screen::Playing {
                    return Some(
                        json!({ "error": format!("the {} screen opened", self.screen_label()), "position": self.position() }),
                    );
                }
                self.step_goto(goto, timed_out)
            }
            TaskKind::Break(b) => self.step_break(b, timed_out, interrupted),
            TaskKind::Click {
                pos,
                before,
                inventory,
                report_tick,
                release,
                place,
            } => {
                if let Some(at) = release {
                    if Instant::now() < *at && !timed_out && interrupted.is_none() {
                        return None;
                    }
                    let _ = self.agent_mouse(MouseButton::Right, false);
                    *release = None;
                    *report_tick = self.ticks + 2;
                }
                if self.ticks < *report_tick && !timed_out {
                    return None;
                }
                let changed = match (*pos, *before) {
                    (Some(p), Some(b)) => self.world.block(p[0], p[1], p[2]) != b,
                    _ => false,
                };
                // A client of a server (single player's server thread, a LAN
                // guest) sees the block once the server's answer comes.
                if *place
                    && !changed
                    && self.remote.is_some()
                    && self.ticks < *report_tick + 20
                    && !timed_out
                {
                    return None;
                }
                let after = pos.map(|p| self.block_json(p));
                let used = delta(inventory, &self.inventory_counts());
                let mut v = json!({
                    "ok": !*place || changed,
                    "block_before": before.map(|b| self.state_string(b)),
                    "block": after,
                    "changed": changed,
                    "inventory_change": used,
                    "screen": self.screen_label(),
                });
                if *place && !changed {
                    v["error"] = json!(
                        "the block did not change (blocked by an entity, or the item cannot be placed)"
                    );
                }
                Some(v)
            }
            TaskKind::Attack {
                id,
                health,
                report_tick,
            } => {
                if self.ticks < *report_tick && !timed_out {
                    return None;
                }
                let now = id.and_then(|id| self.mobs.iter().find(|m| m.id == id));
                Some(json!({
                    "ok": true,
                    "target": id,
                    "health_before": health,
                    "health": now.map(|m| m.health),
                    "alive": now.is_some_and(|m| m.alive()),
                    "hit": id.is_some(),
                }))
            }
        }
    }

    fn step_goto(&mut self, g: &mut Goto, timed_out: bool) -> Option<Value> {
        let p = self.position();
        let on_ground = self.player.as_ref().is_some_and(|p| p.ground.on_ground);
        let horizontal =
            |a: [f64; 3], b: [f64; 3]| ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
        let arrived = horizontal(p, g.target) <= g.near
            && (p[1] - g.target[1]).abs() <= g.near.max(1.0) + 1.0;
        if arrived {
            return Some(
                json!({ "ok": true, "arrived": true, "position": p, "target": g.target,
                                "distance": dist(p, g.target), "warning": g.warning }),
            );
        }
        if timed_out {
            return Some(
                json!({ "error": "timed out before arriving", "arrived": false, "position": p,
                                "target": g.target, "distance": dist(p, g.target), "warning": g.warning }),
            );
        }
        // The next waypoint: path nodes, then the target itself.
        let mut aim = g.target;
        while g.next < g.path.len() {
            let n = g.path[g.next];
            let c = [
                f64::from(n[0]) + 0.5,
                f64::from(n[1]),
                f64::from(n[2]) + 0.5,
            ];
            let last = g.next + 1 == g.path.len();
            if horizontal(p, c) < if last { 0.3 } else { 0.45 } && (p[1] - c[1]).abs() < 1.1 {
                g.next += 1;
                g.progress_tick = self.ticks;
                continue;
            }
            aim = c;
            break;
        }
        // Stuck: jump, then replan from here.
        let stalled = self.ticks.saturating_sub(g.progress_tick);
        if stalled > 60 && on_ground {
            g.jump_until = self.ticks + 4;
        }
        if stalled > 120 {
            g.replans += 1;
            if g.replans > 3 {
                return Some(
                    json!({ "error": "stuck: cannot make progress", "arrived": false, "position": p,
                                    "target": g.target, "distance": dist(p, g.target) }),
                );
            }
            let from = [
                p[0].floor() as i32,
                p[1].round() as i32,
                p[2].floor() as i32,
            ];
            if let Some(path) = self.find_path(from, g.target, g.near) {
                g.path = path;
                g.next = 1;
            }
            g.progress_tick = self.ticks;
        }
        let d = [aim[0] - p[0], aim[2] - p[2]];
        if d[0].abs() + d[1].abs() > 1e-3 {
            self.yaw = (-d[0]).atan2(d[1]).to_degrees() as f32;
        }
        self.keys.insert(KeyCode::KeyW);
        if g.sprint {
            self.keys.insert(KeyCode::ControlLeft);
        }
        // Step up onto the next block, or out of water.
        let in_water = {
            let b = p.map(|v| v.floor() as i32);
            self.res
                .facts()
                .get(self.world.block(b[0], b[1], b[2]))
                .fluid
                != horizon_client::blocks::Fluid::None
        };
        if aim[1] > p[1] + 0.5 && horizontal(p, aim) < 1.6 && on_ground {
            g.jump_until = self.ticks + 4;
        }
        if self.ticks < g.jump_until || in_water {
            self.keys.insert(KeyCode::Space);
        } else {
            self.keys.remove(&KeyCode::Space);
        }
        None
    }

    fn step_break(
        &mut self,
        b: &mut Break,
        timed_out: bool,
        interrupted: Option<&str>,
    ) -> Option<Value> {
        let state = self.world.block(b.pos[0], b.pos[1], b.pos[2]);
        if b.collect_until.is_none() {
            if let Some(why) = interrupted {
                return Some(json!({ "error": why, "pos": b.pos }));
            }
            if state != b.before {
                // Broken: collect what drops for a moment.
                let _ = self.agent_mouse(MouseButton::Left, false);
                b.broke_tick = self.ticks;
                b.collect_until = Some(self.ticks + 20);
            } else if timed_out {
                let _ = self.agent_mouse(MouseButton::Left, false);
                return Some(
                    json!({ "error": "timed out before the block broke", "pos": b.pos,
                                    "block": self.state_string(state), "ticks": self.ticks - b.started_tick }),
                );
            } else if self.screen != Screen::Playing {
                return Some(
                    json!({ "error": format!("the {} screen opened", self.screen_label()), "pos": b.pos }),
                );
            } else {
                // Keep the crosshair on the block (the player may drift).
                if self.pick().is_none_or(|h| h.pos != b.pos)
                    && let Err(e) = self.aim_block(b.pos, None)
                {
                    let _ = self.agent_mouse(MouseButton::Left, false);
                    return Some(json!({ "error": e.to_string(), "pos": b.pos }));
                }
                if !self.left {
                    let _ = self.agent_mouse(MouseButton::Left, true);
                }
                return None;
            }
        }
        // New item entities near the block are its drops.
        let centre = b.pos.map(|v| f64::from(v) + 0.5);
        for item in &self.dropped {
            if !b.items_before.contains(&item.id)
                && dist(centre, [item.position.x, item.position.y, item.position.z]) < 3.0
            {
                b.drops.insert(item.id, item.stack.clone());
            }
        }
        let remaining = b
            .drops
            .keys()
            .filter(|id| self.dropped.iter().any(|i| i.id == **id))
            .count();
        let until = b.collect_until.unwrap_or(0);
        if (self.ticks < until && (remaining > 0 || self.ticks < b.broke_tick + 3)) && !timed_out {
            return None;
        }
        Some(json!({
            "ok": true,
            "pos": b.pos,
            "block_before": self.state_string(b.before),
            "block": self.state_string(state),
            "ticks": b.broke_tick - b.started_tick,
            "drops": counts(b.drops.values().cloned()),
            "picked_up": delta(&b.inventory, &self.inventory_counts()),
            "items_left_on_ground": remaining,
            "on_ground": self.dropped.iter().filter(|i| b.drops.contains_key(&i.id)).map(|i| json!({
                "id": i.id, "item": &*i.stack.item, "count": i.stack.count,
                "pos": [i.position.x, i.position.y, i.position.z],
            })).collect::<Vec<_>>(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_wait_keeps_held_input() {
        assert!(!TaskKind::Wait { until_tick: 40 }.releases_input());
        let jump = TaskKind::Jump {
            release_tick: 4,
            from_y: 64.0,
        };
        assert!(jump.releases_input());
    }

    #[test]
    fn parses_flags_and_values() {
        let a = parse_args(&["1", "2", "--near", "3", "--sprint", "--timeout=9", "-4"]);
        assert_eq!(a.pos, ["1", "2", "-4"]);
        assert_eq!(a.flags["near"], Some("3"));
        assert_eq!(a.flags["sprint"], None);
        assert_eq!(a.flags["timeout"], Some("9"));
        assert_eq!(a.num("near", 0.0).unwrap(), 3.0);
        assert_eq!(a.num("missing", 7.0).unwrap(), 7.0);
        assert!(a.f64(5, "X").is_err());
        assert_eq!(a.block_pos(0).ok(), Some([1, 2, -4]));
        assert!(a.block_pos(1).is_err());
        assert_eq!(
            parse_args(&["-1", "64", "2"]).block_pos(0).unwrap(),
            [-1, 64, 2]
        );
    }

    #[test]
    fn yaw_and_pitch_follow_minecraft() {
        let (yaw, pitch) = yaw_pitch([0.0; 3], [0.0, 0.0, 1.0]);
        assert!(yaw.abs() < 1e-4 && pitch.abs() < 1e-4);
        let (yaw, _) = yaw_pitch([0.0; 3], [-1.0, 0.0, 0.0]);
        assert!((yaw - 90.0).abs() < 1e-4);
        let (_, pitch) = yaw_pitch([0.0; 3], [0.0, -1.0, 0.0]);
        assert!((pitch - 90.0).abs() < 1e-4);
    }

    #[test]
    fn faces_round_trip() {
        for (name, dir) in FACES {
            assert_eq!(face_dir(name).unwrap(), dir);
            assert_eq!(face_name(dir), name);
        }
        assert!(face_dir("sideways").is_err());
    }

    #[test]
    fn inventory_delta() {
        let before = BTreeMap::from([("a".to_owned(), 2), ("b".to_owned(), 1)]);
        let after = BTreeMap::from([("a".to_owned(), 5), ("c".to_owned(), 1)]);
        let d = delta(&before, &after);
        assert_eq!(
            d,
            BTreeMap::from([
                ("a".to_owned(), 3),
                ("b".to_owned(), -1),
                ("c".to_owned(), 1)
            ])
        );
    }
}
