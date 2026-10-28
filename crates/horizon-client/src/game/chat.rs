//! Chat, the command registries and the per-world state behind commands
//! (game rules, the world clock, difficulty), bound to the game.
use super::{Game, Screen};
use horizon_client::{
    chat::{
        Chat, ChatAbilities, ChatAction, ChatScreen, ChatSource, ChatVisibility, Commands,
        SYSTEM_INDICATOR,
    },
    command::Registries,
    gamerules::GameRules,
    hud::HudBatch,
    screen_draw::Measure,
    text::{ClickEvent, Style, Text},
};
use std::collections::BTreeMap;

/// The tree the chat parses and completes with: a server's, once it has
/// sent one (ClientPacketListener.commands), else the game's own.
fn chat_tree<'a>(
    remote: &'a Option<super::remote::Remote>,
    mods: &'a super::mods::ModsState,
) -> &'a horizon_client::command::Dispatcher {
    match remote.as_ref().and_then(|r| r.commands.as_ref()) {
        Some(tree) => mods.remote_dispatcher().unwrap_or(tree),
        None => mods.dispatcher(),
    }
}
use winit::{
    event::KeyEvent,
    keyboard::{KeyCode, PhysicalKey},
};

/// The local player's name (offline single player).
pub const PLAYER_NAME: &str = "Player";

/// ServerClockManager.ServerClockInstance for the overworld clock (its
/// total ticks are the world's day time).
#[derive(Clone, Copy, Debug)]
pub struct WorldClock {
    pub partial: f32,
    pub rate: f32,
    pub paused: bool,
}
impl Default for WorldClock {
    fn default() -> Self {
        Self {
            partial: 0.0,
            rate: 1.0,
            paused: false,
        }
    }
}
/// A clock time marker from the timelines (ClockTimeMarker).
#[derive(Clone, Copy, Debug)]
pub struct TimeMarker {
    pub ticks: i64,
    pub period: Option<i64>,
    pub show: bool,
}
impl TimeMarker {
    pub fn occurs_at(&self, total: i64) -> bool {
        match self.period {
            Some(p) => self.ticks == total.rem_euclid(p),
            None => self.ticks == total,
        }
    }
    pub fn resolve(&self, total: i64) -> i64 {
        match self.period {
            Some(p) => {
                let d = self.ticks - total.rem_euclid(p);
                total + if d > 0 { d } else { p + d }
            }
            None => self.ticks,
        }
    }
}
/// UI and command state that lives beside the game.
#[derive(Default)]
pub struct UiState {
    pub chat: Chat,
    pub chat_screen: Option<ChatScreen>,
    /// When the chat screen opened (its input took the focus: the cursor blink).
    pub chat_opened: Option<std::time::Instant>,
    /// The chat screen is `InBedChatScreen`: the sleeper's, with a Leave Bed button.
    pub in_bed_chat: bool,
    pub registries: Registries,
    pub rules: GameRules,
    pub clock: WorldClock,
    /// Time markers and timelines of the overworld clock.
    pub markers: BTreeMap<String, TimeMarker>,
    pub timelines: BTreeMap<String, Option<i64>>,
    /// 0 peaceful .. 3 hard.
    pub difficulty: u8,
    /// F1: hide the HUD.
    pub hide_gui: bool,
    /// Death message key set by the cause of death (e.g. /kill).
    pub death_cause: Option<String>,
    /// Path of an F2 screenshot waiting for its frame.
    pub screenshot_message: Option<std::path::PathBuf>,
    /// The options screens.
    pub options: horizon_client::options_ui::OptionsUi,
    /// The prepared resources before resource packs are layered on.
    pub vanilla_root: std::path::PathBuf,
    /// Death screen cause and when the player died (buttons wait 20 ticks).
    pub death_text: Option<Text>,
    pub died_at: Option<std::time::Instant>,
    /// `DeathScreen.TitleConfirmScreen` and when it opened (its buttons wait
    /// 20 ticks, `setDelay`).
    pub death_confirm: Option<std::time::Instant>,
    /// `LivingEntity.deathTime` of the window's player: ticks since it died.
    pub death_time: u32,
}

impl UiState {
    /// The state a window starts with: the chat remembers the commands kept
    /// in the game folder (`ChatComponent`'s constructor).
    pub fn new() -> Self {
        let mut ui = Self::default();
        if let Ok(dir) = std::env::current_dir() {
            ui.chat.load_history(&dir);
        }
        ui
    }
}

impl Game<'_> {
    /// `PlayerList.isOp` for the single-player owner: the world's Allow Commands.
    pub(super) fn allow_commands(&self) -> bool {
        self.level.allow_commands == Some(true)
    }
    /// `IntegratedServer.setWorldAllowCommands`, then `updatePermissions`.
    pub(super) fn set_allow_commands(&mut self, allow: bool) {
        self.level.allow_commands = Some(allow);
        self.ui.registries.permission = owner_permission(allow);
    }
    /// Commands the game issues itself (the server console, harnesses, the
    /// control socket) run at the owner's level whatever Allow Commands says.
    pub(super) fn as_console<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let level = std::mem::replace(&mut self.ui.registries.permission, 4);
        let out = f(self);
        self.ui.registries.permission = level;
        out
    }
    /// Registry contents for argument parsing (built when a world opens).
    pub(super) fn build_registries(&mut self) {
        let permission = owner_permission(self.allow_commands());
        let reg = &mut self.ui.registries;
        reg.permission = permission;
        {
            let dir = |sub: &str| -> Vec<String> {
                let root = self.options.resources.join("data/minecraft").join(sub);
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(d) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&d) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().and_then(|x| x.to_str()) == Some("json")
                            && let Ok(rel) = p.strip_prefix(&root)
                        {
                            out.push(format!(
                                "minecraft:{}",
                                rel.with_extension("").to_string_lossy().replace('\\', "/")
                            ));
                        }
                    }
                }
                out.sort();
                out
            };
            self.ui.registries.recipes = dir("recipe");
            self.ui.registries.damage_types = dir("damage_type");
            self.ui.registries.loot_tables = dir("loot_table");
            self.ui.registries.templates = {
                let root = self.options.resources.join("worldgen/structure/minecraft");
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(d) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&d) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().and_then(|x| x.to_str()) == Some("nbt")
                            && let Ok(rel) = p.strip_prefix(&root)
                        {
                            out.push(format!(
                                "minecraft:{}",
                                rel.with_extension("").to_string_lossy().replace('\\', "/")
                            ));
                        }
                    }
                }
                out.sort();
                out
            };
        }
        self.ui.registries.sounds = self.audio.event_names();
        self.ui.registries.attributes = [
            "armor",
            "armor_toughness",
            "attack_damage",
            "attack_knockback",
            "attack_speed",
            "block_break_speed",
            "block_interaction_range",
            "burning_time",
            "camera_distance",
            "explosion_knockback_resistance",
            "entity_interaction_range",
            "fall_damage_multiplier",
            "flying_speed",
            "follow_range",
            "gravity",
            "jump_strength",
            "knockback_resistance",
            "luck",
            "max_absorption",
            "max_health",
            "mining_efficiency",
            "movement_efficiency",
            "movement_speed",
            "oxygen_bonus",
            "safe_fall_distance",
            "scale",
            "sneaking_speed",
            "spawn_reinforcements",
            "step_height",
            "submerged_mining_speed",
            "sweeping_damage_ratio",
            "tempt_range",
            "water_movement_efficiency",
        ]
        .iter()
        .map(|a| format!("minecraft:{a}"))
        .collect();
        let reg = &mut self.ui.registries;
        let mut items: Vec<String> = self
            .res
            .items()
            .order
            .iter()
            .map(|s| s.to_string())
            .collect();
        items.sort();
        reg.items = items;
        reg.item_tags = self
            .res
            .items()
            .item_tags
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        reg.item_tags.sort();
        reg.block_tags = self
            .res
            .items()
            .block_tags
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        reg.block_tags.sort();
        let registry = self.pack.assets.registry;
        let mut blocks: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        for id in 0..registry.len() {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(id as u32))
            else {
                continue;
            };
            let props = blocks.entry(state.name.clone()).or_default();
            for (k, v) in &state.properties {
                let values = props.entry(k.clone()).or_default();
                if !values.contains(v) {
                    values.push(v.clone());
                }
            }
        }
        reg.blocks = blocks
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        let mut summonable: Vec<String> = self
            .res
            .mob_types()
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        summonable.extend(
            [
                "minecraft:item",
                "minecraft:experience_orb",
                "minecraft:falling_block",
                "minecraft:arrow",
                "minecraft:lightning_bolt",
                "minecraft:minecart",
                "minecraft:chest_minecart",
                "minecraft:hopper_minecart",
                "minecraft:furnace_minecart",
                "minecraft:tnt_minecart",
            ]
            .map(String::from),
        );
        summonable.sort();
        summonable.dedup();
        reg.summonable = summonable;
        let root = self.options.resources.clone();
        let mut types: Vec<String> = serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(root.join("entity-types.json")).unwrap_or_default(),
        )
        .ok()
        .and_then(|v| {
            v.as_object().map(|o| {
                o.keys()
                    .map(|k| horizon_client::command::args::normalize(k))
                    .collect()
            })
        })
        .unwrap_or_default();
        if types.is_empty() {
            types = reg.summonable.clone();
            types.push("minecraft:player".into());
        }
        types.sort();
        reg.entity_types = types;
        let list = |dir: &str| -> Vec<String> {
            let mut out: Vec<String> = std::fs::read_dir(root.join(dir))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".json")
                        .map(|n| format!("minecraft:{n}"))
                })
                .collect();
            out.sort();
            out
        };
        // The mob effect registry: every effect `/effect` and the
        // `mob_effect` resource arguments accept.
        reg.effects = horizon_client::brewing::EFFECTS
            .iter()
            .map(|e| format!("minecraft:{}", e.id))
            .collect();
        reg.effects.sort();
        reg.enchantments = list("data/minecraft/enchantment");
        reg.structures = list("data/minecraft/worldgen/structure");
        reg.biomes = list("data/minecraft/worldgen/biome");
        reg.clocks = list("data/minecraft/world_clock");
        reg.dimensions = vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ];
        reg.players = vec![PLAYER_NAME.into()];
        // Timelines of the overworld clock and their time markers.
        self.ui.markers.clear();
        self.ui.timelines.clear();
        for name in list("data/minecraft/timeline") {
            let path = root.join(format!(
                "data/minecraft/timeline/{}.json",
                name.trim_start_matches("minecraft:")
            ));
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(path).unwrap_or_default(),
            ) else {
                continue;
            };
            if v["clock"].as_str() != Some("minecraft:overworld") {
                continue;
            }
            let period = v["period_ticks"].as_i64();
            self.ui.timelines.insert(name.clone(), period);
            for (marker, def) in v["time_markers"].as_object().into_iter().flatten() {
                let (ticks, show) = match def {
                    serde_json::Value::Number(n) => (n.as_i64().unwrap_or(0), false),
                    other => (
                        other["ticks"].as_i64().unwrap_or(0),
                        other["show_in_commands"].as_bool().unwrap_or(false),
                    ),
                };
                self.ui.markers.insert(
                    marker.clone(),
                    TimeMarker {
                        ticks,
                        period,
                        show,
                    },
                );
            }
        }
        // Vanilla's built-in registries (mob effects, components, dialogs...).
        for dir in [&self.options.resources, &self.ui.vanilla_root] {
            if let Ok(bytes) = std::fs::read(dir.join("command-registries.json"))
                && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
            {
                self.ui.registries.fill_from_export(&v);
                break;
            }
        }
        let reg = &mut self.ui.registries;
        reg.timelines = self.ui.timelines.keys().cloned().collect();
        reg.time_markers = self
            .ui
            .markers
            .iter()
            .filter(|(_, m)| m.show)
            .map(|(k, _)| k.clone())
            .collect();
    }
    /// The block the player is looking at, for coordinate suggestions.
    fn refresh_dynamic_registries(&mut self) {
        self.ui.registries.target_block = self.pick().map(|h| h.pos);
        // ClientPacketListener.getOnlinePlayerNames on a server.
        if let Some(names) = self.remote_online_names() {
            self.ui.registries.players = names;
        }
    }
    pub(super) fn open_chat(&mut self, initial: &str) {
        if self.player.is_none() {
            return;
        }
        self.refresh_dynamic_registries();
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        let mut screen = ChatScreen::new(
            &m,
            &c,
            initial,
            self.ui.chat.recent.len(),
            b.width as i32,
            b.height as i32,
            self.chat_abilities(),
        );
        screen.auto_suggestions = self.settings.auto_suggestions;
        self.ui.chat_screen = Some(screen);
        self.ui.chat_opened = Some(std::time::Instant::now());
        self.screen = Screen::Chat;
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.capture(false);
    }
    pub(super) fn close_chat(&mut self) {
        // InBedChatScreen.onClose: leaving the screen means getting up; the
        // screen goes once the player has woken (`tick_in_bed_chat`).
        if self.ui.in_bed_chat && self.sleeping.is_some() {
            self.wake_up();
            return;
        }
        self.ui.chat_screen = None;
        self.ui.chat.reset_scroll();
        if self.screen == Screen::Chat {
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// `Gui.tick`: a sleeper gets `InBedChatScreen`; when the player has
    /// woken (`onPlayerWokeUp`) it closes, or turns into the plain chat
    /// screen if something was typed.
    pub(super) fn tick_in_bed_chat(&mut self) {
        if self.screen != Screen::Chat {
            self.ui.in_bed_chat = false;
        }
        if self.ui.in_bed_chat {
            if self.sleeping.is_none() {
                self.ui.in_bed_chat = false;
                let text = self
                    .ui
                    .chat_screen
                    .as_ref()
                    .map_or_else(String::new, |s| s.value().to_owned());
                self.close_chat();
                if !text.is_empty() {
                    self.open_chat(&text);
                }
            }
        } else if self.sleeping.is_some() && self.screen == Screen::Playing {
            self.open_chat("");
            self.ui.in_bed_chat = self.screen == Screen::Chat;
        }
    }
    /// The Leave Bed button of `InBedChatScreen`: its bounds.
    fn leave_bed_button(width: f32, height: f32) -> (i32, i32, i32, i32) {
        (width as i32 / 2 - 100, height as i32 - 40, 200, 20)
    }
    /// `Minecraft.computeChatAbilities` from the Chat option.
    pub(super) fn chat_abilities(&self) -> ChatAbilities {
        ChatAbilities::new(ChatVisibility::from_option(self.settings.chat_visibility))
    }
    /// A chat message from the game (ChatComponent.addServerSystemMessage).
    pub(super) fn system_message(&mut self, text: Text) {
        if let Some(capture) = self.chat_capture.as_mut() {
            capture.push(text);
            return;
        }
        // Connected to a server (single player's own included), the chat on
        // screen is the connection's: the game's own messages join it.
        if self.remote.is_some() {
            self.chat_line(text, ChatSource::Server);
            return;
        }
        // The Chat option's Hidden level takes the server's messages too.
        self.local_chat_line(text, Some(SYSTEM_INDICATOR), ChatSource::Server);
    }
    /// A player's chat line on the window's own chat: a LAN guest's message
    /// while the host runs in-frame (`HostEvent::Chat`). `ChatListener`
    /// checks `canReceivePlayerMessages` for these, so Commands Only and
    /// Hidden drop them; command feedback (`/list`, `/tp`) is a system
    /// message and takes the system-message permission instead.
    pub(super) fn player_message(&mut self, text: Text) {
        if let Some(capture) = self.chat_capture.as_mut() {
            capture.push(text);
            return;
        }
        // A server's client receives the line as a packet (PlayerChat or
        // DisguisedChat) and filters it there.
        if self.remote.is_none() {
            self.local_chat_line(text, None, ChatSource::Player);
        }
    }
    /// A line on the window's own chat when the Chat option lets `source`
    /// through; running mods hear it.
    fn local_chat_line(&mut self, text: Text, indicator: Option<u32>, source: ChatSource) {
        if !self.ui.chat.shows(source) {
            return;
        }
        let focused = self.screen == Screen::Chat;
        let heard = self.mods.running().then(|| text.plain());
        self.ui.chat.add(
            &self.gui.atlas,
            text,
            indicator,
            source,
            self.ticks,
            focused,
        );
        if let Some(line) = heard {
            self.mods_chat(&line);
        }
    }
    /// Keys while the chat screen is open.
    pub(super) fn chat_key(&mut self, event: &KeyEvent) {
        if let Some(shortcut) = self.edit_shortcut(event) {
            let m = Measure(&self.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&self.remote, &self.mods),
                registries: &self.ui.registries,
            };
            if let Some(screen) = self.ui.chat_screen.as_mut() {
                screen.shortcut(&m, &c, shortcut, &mut self.clipboard);
            }
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        self.chat_key_code(key, event.text.as_deref());
    }
    /// A key (and the text it typed) while the chat screen is open.
    pub(super) fn chat_key_code(&mut self, key: KeyCode, typed_text: Option<&str>) {
        // InputQuirks.EDIT_SHORTCUT_KEY_MODIFIER: Cmd on macOS, Ctrl elsewhere.
        let (left, right) = if cfg!(target_os = "macos") {
            (KeyCode::SuperLeft, KeyCode::SuperRight)
        } else {
            (KeyCode::ControlLeft, KeyCode::ControlRight)
        };
        let ctrl = self.keys.contains(&left) || self.keys.contains(&right);
        let shift =
            self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        match key {
            // ChatScreen.keyPressed: the chat scrolls a page.
            KeyCode::PageUp | KeyCode::PageDown
                if !self
                    .ui
                    .chat_screen
                    .as_ref()
                    .is_some_and(|s| s.suggestions_visible()) =>
            {
                let n = self.ui.chat.lines_per_page(true) as i32 - 1;
                self.ui
                    .chat
                    .scroll_by(if key == KeyCode::PageUp { n } else { -n }, true);
                return;
            }
            _ => {}
        }
        let Some(mut screen) = self.ui.chat_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        match screen.key(&m, &c, &self.ui.chat.recent, key, shift, ctrl) {
            ChatAction::Close => self.close_chat(),
            ChatAction::Submit(message) => {
                // InBedChatScreen sends without closing (`closeOnSubmit` is
                // false): the box empties and the screen stays.
                if self.ui.in_bed_chat && self.sleeping.is_some() {
                    screen.suggest_command(&m, &c, "");
                    self.ui.chat.reset_scroll();
                    self.ui.chat_screen = Some(screen);
                } else {
                    self.close_chat();
                }
                if !message.is_empty() {
                    self.handle_chat_input(&message);
                }
            }
            ChatAction::None => {
                // charTyped: the text the key typed.
                if let Some(text) = typed_text
                    && !ctrl
                {
                    screen.type_text(&m, &c, text);
                }
                self.ui.chat_screen = Some(screen);
            }
        }
    }
    /// The server's completions for an `ask_server` request
    /// (ClientPacketListener.handleCommandSuggestions).
    pub(super) fn chat_server_suggestions(
        &mut self,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.server_suggestions(&m, &c, id, range, list);
        }
    }
    /// Text typed into the chat box (control socket `chat`).
    pub(super) fn chat_type(&mut self, text: &str) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.type_text(&m, &c, text);
        }
    }
    /// ChatScreen.handleChatInput: history, then a command or a chat line.
    /// The Chat option's restrictions stop the message here too, so a
    /// control-socket `chat` cannot send what the player's setting hides.
    pub(super) fn handle_chat_input(&mut self, message: &str) {
        if !self.chat_abilities().allows(message) {
            return;
        }
        self.ui.chat.add_recent(message);
        // On a server (single player's own included) the server runs it,
        // unless a client mod owns the command.
        if self.remote.is_some() {
            if let Some(command) = message.strip_prefix('/')
                && self.client_mod_command(command)
            {
                return;
            }
            self.send_chat(message);
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            // A client mod's command first, then a server mod's, as on a server.
            if !self.client_mod_command(command) && !self.local_server_mod_command(command) {
                self.run_command(command);
            }
        } else {
            let line = Text::translate(
                "chat.type.text",
                &[self.player_display_name(), Text::literal(message)],
            );
            let focused = self.screen == Screen::Chat;
            let heard = self.mods.running().then(|| line.plain());
            self.ui.chat.add(
                &self.gui.atlas,
                line,
                None,
                ChatSource::Player,
                self.ticks,
                focused,
            );
            if let Some(line) = heard {
                self.mods_chat(&line);
            }
            self.lan_host_chat(message);
            self.mod_player_chat(super::server_mods::Who::Local, message);
        }
    }
    pub(super) fn chat_scroll(&mut self, amount: f32) {
        let cursor = self.cursor;
        let height = self.renderer.as_ref().map_or(720.0, |r| {
            HudBatch::new([r.config.width, r.config.height]).height
        });
        let _ = height;
        let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
        if let Some(screen) = self.ui.chat_screen.as_mut()
            && screen.scroll(mouse, amount as f64)
        {
            return;
        }
        // ChatScreen.mouseScrolled: `hasShiftDown`, either Shift key.
        let shift = self.has_shift_down();
        let amount = amount.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
        self.ui.chat.scroll_by(amount as i32, true);
    }
    pub(super) fn chat_click(&mut self) {
        let cursor = self.cursor;
        let height = self.renderer.as_ref().map_or(720.0, |r| {
            HudBatch::new([r.config.width, r.config.height]).height
        });
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if self.ui.in_bed_chat && self.sleeping.is_some() {
            let width = self.renderer.as_ref().map_or(1280.0, |r| {
                HudBatch::new([r.config.width, r.config.height]).width
            });
            let (x, y, w, h) = Self::leave_bed_button(width, height);
            if horizon_client::screen_draw::inside((cursor.0 as i32, cursor.1 as i32), x, y, w, h) {
                self.wake_up();
                return;
            }
        }
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            if screen.click(&m, &c, cursor.0 as f64, cursor.1 as f64) {
                return;
            }
        } else {
            return;
        }
        let Some(style) = self.ui.chat.style_at(
            &self.gui.atlas.font,
            height as i32,
            cursor.0 as f64,
            cursor.1 as f64,
        ) else {
            return;
        };
        // ChatScreen.handleComponentClicked: with Shift held only an
        // insertion acts, else only the click event.
        if self.has_shift_down() {
            if let Some(insertion) = style.insertion
                && let Some(screen) = self.ui.chat_screen.as_mut()
            {
                screen.type_text(&m, &c, &insertion);
            }
            return;
        }
        match style.click {
            Some(ClickEvent::CopyToClipboard(value)) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&value);
            }
            Some(ClickEvent::OpenFile(path)) => {
                super::packs::open_folder(std::path::Path::new(&path));
            }
            Some(ClickEvent::OpenUrl(url)) => self.click_url(url),
            Some(ClickEvent::SuggestCommand(command)) => {
                if let Some(screen) = self.ui.chat_screen.as_mut() {
                    screen.suggest_command(&m, &c, &command);
                }
            }
            Some(ClickEvent::RunCommand(command)) => self.click_command(&command),
            // Screen.defaultHandleGameClickEvent: the dialog opens over the
            // screen, and closes back to it.
            Some(ClickEvent::ShowDialog(dialog)) => self.show_dialog_ref(&dialog),
            // Screen.defaultHandleClickEvent: a page link means nothing in chat.
            Some(ClickEvent::ChangePage(_)) | None => {}
        }
    }
    /// `Screen.clickUrlAction`: nothing with Web Links off, else the link
    /// opens, after the confirmation unless Prompt on Links is off.
    pub(super) fn click_url(&mut self, url: String) {
        if !self.settings.chat_links {
            return;
        }
        if self.settings.file.bool("chatLinksPrompt") {
            self.link_prompt = Some(url);
        } else {
            super::packs::open_uri(&url);
        }
    }
    /// `ConfirmLinkScreen` (untrusted): the link and its warning over the
    /// chat, answered yes to open it.
    fn link_prompt_layout(
        &self,
        url: &str,
        size: (f32, f32),
    ) -> horizon_client::confirm_screen::Choice {
        let warning = Text::translate("chat.link.warning", &[]).with_color(0xFF_CCCC);
        let message = Text::literal(url)
            .append(Text::literal("\n"))
            .append(warning);
        // ConfirmLinkScreen.addButtons: yes, copy and no, 100 wide each.
        horizon_client::confirm_screen::Choice::new(
            &self.gui.atlas.font,
            size,
            &Text::translate("chat.link.confirm", &[]),
            &message,
            3,
            100.0,
        )
    }
    pub(super) fn draw_link_prompt(&self, batch: &mut HudBatch) {
        let Some(url) = &self.link_prompt else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let labels = [
            Text::translate("gui.yes", &[]),
            Text::translate("chat.copy", &[]),
            Text::translate("gui.no", &[]),
        ];
        self.link_prompt_layout(url, (w, h))
            .draw(batch, atlas, self.cursor, &labels);
    }
    /// The prompt's answer: the link opens on yes; either way the chat is back.
    pub(super) fn answer_link_prompt(&mut self, yes: bool) {
        if let Some(url) = self.link_prompt.take()
            && yes
        {
            super::packs::open_uri(&url);
        }
    }
    pub(super) fn link_prompt_click(&mut self) {
        let Some(url) = self.link_prompt.clone() else {
            return;
        };
        let pressed = self
            .link_prompt_layout(&url, self.gui_size())
            .pressed_at(self.cursor);
        if let Some(button) = pressed {
            self.audio.play_button_click();
            if button == LINK_COPY {
                // ConfirmLinkScreen.copyToClipboard, then the callback's no.
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&url);
            }
            self.answer_link_prompt(button == LINK_YES);
        }
    }
    /// `Screen.clickCommandAction`: run a clicked command as the player
    /// without adding it to the chat history. A client mod's command goes
    /// first, as `ClientPacketListener.sendUnattendedCommand` lets client
    /// commands take it. (Vanilla asks first when the command fails to
    /// parse; here the server answers it.)
    pub(super) fn click_command(&mut self, command: &str) {
        let command = command.strip_prefix('/').unwrap_or(command);
        if self.client_mod_command(command) {
            return;
        }
        if self.remote.is_some() {
            self.send_chat(&format!("/{command}"));
        } else if !self.local_server_mod_command(command) {
            self.run_command(command);
        }
    }
    /// Chat lines over the HUD (focused while the chat screen is open).
    pub(super) fn draw_chat(&mut self, batch: &mut HudBatch) {
        let focused = self.screen == Screen::Chat;
        if self.ui.hide_gui && !focused {
            return;
        }
        self.ui
            .chat
            .draw(batch, &self.gui.atlas, self.ticks, focused);
    }
    pub(super) fn draw_chat_screen(&mut self, batch: &mut HudBatch) {
        let Some(mut screen) = self.ui.chat_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        screen.resize(&m, &c, batch.width as i32, batch.height as i32);
        // Options.getBackgroundColor(Integer.MIN_VALUE).
        let background = if self.settings.file.bool("backgroundForChatOnly") {
            i32::MIN
        } else {
            self.settings.text_background(0.0) as i32
        };
        let mouse = (self.cursor.0.floor() as i32, self.cursor.1.floor() as i32);
        let blink = self
            .ui
            .chat_opened
            .map_or(0, |t| t.elapsed().as_millis() as u64);
        let draws = screen.draw(&m, mouse, blink, background);
        horizon_client::screen_draw::paint(batch, &self.gui.atlas, &draws);
        self.ui.chat_screen = Some(screen);
        if self.ui.in_bed_chat && self.sleeping.is_some() {
            let (x, y, w, h) = Self::leave_bed_button(batch.width, batch.height);
            let label = Text::literal(horizon_client::lang::get_or(
                "multiplayer.stopSleeping",
                "Leave Bed",
            ));
            let mut button = Vec::new();
            horizon_client::screen_draw::button(
                &mut button,
                &m,
                &label,
                x,
                y,
                w,
                h,
                true,
                horizon_client::screen_draw::inside(mouse, x, y, w, h),
            );
            horizon_client::screen_draw::paint(batch, &self.gui.atlas, &button);
        }
        // The hovered message part's tooltip (`renderComponentHoverEffect`),
        // or the text of the message tag under the cursor.
        let height = batch.height as i32;
        let (cx, cy) = (f64::from(self.cursor.0), f64::from(self.cursor.1));
        let wrap = (batch.width as i32 / 2).max(200);
        if let Some(style) = self.ui.chat.style_at(&self.gui.atlas.font, height, cx, cy) {
            if let Some(hover) = style.hover {
                let lines = self.wrapped_rows(&hover, wrap);
                self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
            } else if let Some(stack) = style
                .hover_item
                .as_deref()
                .and_then(horizon_client::item_data::stack_from_hover)
                .filter(|s| self.res.items().get(&s.item).is_some())
            {
                self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
            }
        } else if self.ui.chat.indicator_at(height, cx, cy) {
            // GuiMessageTag.systemSinglePlayer's text, split at 210.
            let tag = Text::translate("chat.tag.system_single_player", &[]);
            let lines = self.wrapped_rows(&tag, 210);
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// `Font.split`: a tooltip text wrapped at `width`, one white row per line.
    fn wrapped_rows(&self, text: &Text, width: i32) -> Vec<(Text, u32)> {
        let split = self
            .gui
            .atlas
            .font
            .split(&text.resolve_legacy(), width as f32);
        split.into_iter().map(|row| (row, u32::MAX)).collect()
    }
    /// Advance the overworld clock (ServerClockManager.tick) when
    /// `advance_time` allows it.
    pub(super) fn advance_clock(&mut self) {
        if !self.ui.rules.bool("advance_time") || self.ui.clock.paused {
            return;
        }
        let c = &mut self.ui.clock;
        c.partial += c.rate;
        let whole = c.partial.floor();
        c.partial -= whole;
        self.world.day_time += whole as i64;
    }
    /// `ServerClockInstance.packNetworkState`: the rate players' clients
    /// advance the overworld clock at, 0 while it is paused (`/time pause`)
    /// or `advance_time` is off.
    pub(super) fn network_clock_rate(&self) -> f32 {
        if self.ui.rules.bool("advance_time") && !self.ui.clock.paused {
            self.ui.clock.rate
        } else {
            0.0
        }
    }
    /// The engine-side rule values (block behaviour and health).
    pub(super) fn level_rules(&self) -> horizon_world::level::GameRules {
        let r = &self.ui.rules;
        horizon_world::level::GameRules {
            water_source_conversion: r.bool("water_source_conversion"),
            lava_source_conversion: r.bool("lava_source_conversion"),
            random_tick_speed: r.int("random_tick_speed"),
            block_drops: r.bool("block_drops"),
            mob_griefing: r.bool("mob_griefing"),
            max_snow_accumulation_height: r.int("max_snow_accumulation_height"),
            spawn_mobs: r.bool("spawn_mobs"),
            spawn_monsters: r.bool("spawn_monsters"),
            fire_spread_radius_around_player: r.int("fire_spread_radius_around_player"),
            spread_vines: r.bool("spread_vines"),
        }
    }
    /// Apply rules mirrored in other state (natural regeneration, difficulty).
    pub(super) fn sync_rules(&mut self) {
        self.world.health_rules.natural_regeneration =
            self.ui.rules.bool("natural_health_regeneration");
        // Player.isInvulnerableTo: the damage-type rules.
        self.world.health_rules.drowning_damage = self.ui.rules.bool("drowning_damage");
        self.world.health_rules.fall_damage = self.ui.rules.bool("fall_damage");
        self.world.health_rules.fire_damage = self.ui.rules.bool("fire_damage");
        self.world.health_rules.freeze_damage = self.ui.rules.bool("freeze_damage");
        use horizon_world::food::Difficulty;
        self.world.health_rules.difficulty = match self.ui.difficulty {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            3 => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
    }
    /// Load per-world command state when a world opens.
    pub(super) fn load_world_state(&mut self) {
        self.ui.rules = GameRules::load(&self.world_root);
        self.ui.clock = WorldClock::default();
        // PrimaryLevelData: the difficulty comes back from level.dat's difficulty_settings.
        self.ui.difficulty = horizon_client::title::DIFFICULTIES
            .iter()
            .position(|d| *d == self.level.difficulty)
            .map_or(2, |i| i as u8);
        if let Ok(bytes) = std::fs::read(self.world_root.join("data/minecraft/world_clocks.dat"))
            && let Ok(raw) = horizon_world::compression::Compression::Gzip.decode(&bytes, 1 << 20)
            && let Ok((root, _)) =
                horizon_world::nbt::read_named(&raw, horizon_world::nbt::Limits::default())
            && let Some(c) = root
                .tag
                .get("data")
                .and_then(|d| d.get("minecraft:overworld"))
        {
            use horizon_world::nbt::Tag;
            if let Some(Tag::Float(r)) = c.get("rate") {
                self.ui.clock.rate = f32::from_bits(*r);
            }
            if let Some(Tag::Float(p)) = c.get("partial_tick") {
                self.ui.clock.partial = f32::from_bits(*p);
            }
            if let Some(Tag::Byte(p)) = c.get("paused") {
                self.ui.clock.paused = *p != 0;
            }
        }
        self.ui.chat.clear();
        self.build_registries();
        self.sync_rules();
    }
    /// Save the clock beside level.dat (Level.write saves the game rules: a
    /// second writer of game_rules.dat could leave it with the rules loaded
    /// when the world opened).
    pub(super) fn save_world_state(&mut self) {
        if self.world_root.as_os_str().is_empty() {
            return;
        }
        use horizon_world::nbt::Tag;
        let c = self.ui.clock;
        // The other clocks stay as saved; the overworld one keeps the fields Horizon
        // does not model, and (like vanilla's codec) omits rate, partial tick and
        // pause while they have their defaults.
        let path = self.world_root.join("data/minecraft/world_clocks.dat");
        let existing = horizon_client::save::read_dat(&path);
        let mut data = match existing.as_ref().and_then(|r| r.get("data")) {
            Some(d @ Tag::Compound(_)) => d.clone(),
            _ => Tag::Compound(Default::default()),
        };
        let mut clock = match data.get("minecraft:overworld") {
            Some(c @ Tag::Compound(_)) => c.clone(),
            _ => Tag::Compound(Default::default()),
        };
        if let Tag::Compound(map) = &mut clock {
            map.insert("total_ticks".into(), Tag::Long(self.world.day_time));
            for (key, value, default) in [
                (
                    "partial_tick",
                    Tag::Float(c.partial.to_bits()),
                    c.partial == 0.0,
                ),
                ("rate", Tag::Float(c.rate.to_bits()), c.rate == 1.0),
                ("paused", Tag::Byte(i8::from(c.paused)), !c.paused),
            ] {
                if default {
                    map.remove(&key.into());
                } else {
                    map.insert(key.into(), value);
                }
            }
        }
        if let Tag::Compound(map) = &mut data {
            map.insert("minecraft:overworld".into(), clock);
        }
        let root = Tag::compound([
            ("data", data),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ]);
        if let Err(e) = horizon_client::save::write_dat(&path, root) {
            log::warn!("the world clocks were not written: {e}");
        }
    }
    /// Death message in chat (CombatTracker.getDeathMessage, as far as the
    /// cause is known to the client).
    pub(super) fn death_message(&mut self) {
        let cause = self.ui.death_cause.take();
        self.ui.died_at = Some(std::time::Instant::now());
        self.ui.death_text = None;
        self.ui.death_confirm = None;
        if !self.ui.rules.bool("show_death_messages") {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let below_world = p.position.y < f64::from(horizon_client::chunk::MIN_Y - 64);
        let name = self.player_display_text();
        let message = match cause {
            Some(key) => Text::translate(&key, &[name]),
            None if below_world => Text::translate("death.attack.outOfWorld", &[name]),
            None => self.combat_death_message(),
        };
        self.ui.death_text = Some(message.clone());
        // ServerPlayer.die: a team that hides death messages keeps it out of
        // the dying player's own chat (the death screen still shows it).
        if horizon_client::scoreboard::own_death_message_shown(self.player_team()) {
            self.system_message(message);
        }
    }
    /// F1 hides the HUD (see `hud`); the screenshot key still works.
    pub(super) fn toggle_hide_gui(&mut self) {
        self.ui.hide_gui = !self.ui.hide_gui;
    }
    /// F2: ScreenShot.grab into `screenshots/` with vanilla's file name.
    pub(super) fn take_screenshot(&mut self) {
        let dir = std::env::current_dir()
            .unwrap_or_default()
            .join("screenshots");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let name = screenshot_name(&dir);
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let path = dir.join(name);
        renderer.screenshot = Some(path.clone());
        self.ui.screenshot_message = Some(path);
    }
    /// After a frame: report a finished F2 screenshot in chat.
    pub(super) fn finish_screenshot(&mut self, result: &std::result::Result<[u32; 2], String>) {
        let Some(path) = self.ui.screenshot_message.take() else {
            return;
        };
        let message = match result {
            Ok(_) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let link = Text::styled(
                    name,
                    Style {
                        underlined: Some(true),
                        click: Some(ClickEvent::OpenFile(path.to_string_lossy().into_owned())),
                        ..Style::default()
                    },
                );
                Text::translate("screenshot.success", &[link])
            }
            Err(e) => Text::translate("screenshot.failure", &[Text::literal(e.clone())]),
        };
        let focused = self.screen == Screen::Chat;
        self.ui.chat.add(
            &self.gui.atlas,
            message,
            None,
            ChatSource::Client,
            self.ticks,
            focused,
        );
    }
}
/// `yyyy-MM-dd_HH.mm.ss[_n].png` (Util.getFilenameFormattedDateTime, local time
/// approximated as UTC).
pub fn screenshot_name(dir: &std::path::Path) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86400) as i64;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    let base = format!("{y}-{mo:02}-{d:02}_{h:02}.{m:02}.{s:02}");
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{base}.png")
        } else {
            format!("{base}_{n}.png")
        };
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}

/// `IntegratedServer.getProfilePermissions` for the owner: `OWNER` (4) when
/// commands are allowed, else `ALL` (0).
fn owner_permission(allow_commands: bool) -> u8 {
    if allow_commands { 4 } else { 0 }
}

/// `ConfirmLinkScreen`'s buttons, left to right: yes, copy, no.
const LINK_YES: usize = 0;
const LINK_COPY: usize = 1;
