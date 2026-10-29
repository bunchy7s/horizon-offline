//! WorldOptionsScreen: the in-world options with the multiplayer section
//! that opens the world to the local network (the 26.3 replacement of the
//! old "Open to LAN" screen): default and personal game mode, cheats,
//! difficulty, LAN on/off with its port, command access for guests and
//! forcing the game mode. Its difficulty lock asks first (PopupScreen), and
//! Restrictions opens RestrictionsScreen.
//! Parity reference: https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/WorldOptionsScreen
use super::{Game, Screen, integrated::ServerOptions, lan::LanSettings};
use horizon_client::{
    font::Font,
    game_rules_screen::{Exit, RulesScreen},
    hud::{HudBatch, rgba},
    text::{Text, named_color},
    ui::{self, EditBox},
};
use horizon_protocol::packets::play::{self, Serverbound as S};
use winit::{event::KeyEvent, keyboard::PhysicalKey};

pub(super) struct WorldOptions {
    remote: bool,
    gamemaster: bool,
    default_mode: i32,
    personal_mode: i32,
    cheats: bool,
    difficulty: u8,
    lan: bool,
    port: EditBox,
    /// The free port suggested while the box is empty.
    hint: u16,
    guest_commands: bool,
    force: bool,
    hardcore: bool,
    /// The level's own lock (LevelData.isDifficultyLocked).
    difficulty_locked: bool,
    /// wantedDifficultyLocked: the lock icon, from the lock or hardcore.
    locked: bool,
    initial: Snapshot,
    /// ScrollableLayout's scroll amount.
    scroll: f32,
    view: View,
    /// InWorldGameRulesScreen, while it is open.
    rules: Option<RulesScreen>,
}
/// The screen on show: the options, the lock's question over them, or the
/// RestrictionsScreen or InWorldGameRulesScreen they opened (all return to
/// the same options).
#[derive(Clone, Copy, Debug, PartialEq)]
enum View {
    Options,
    LockQuestion,
    Restrictions,
    GameRules,
}
#[derive(Clone, Copy, PartialEq)]
struct Snapshot {
    default_mode: i32,
    personal_mode: i32,
    cheats: bool,
    difficulty: u8,
    locked: bool,
    lan: bool,
    port: u16,
    guest_commands: bool,
    force: bool,
}

const MODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];
const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

/// HeaderAndFooterLayout's header and footer heights.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
/// The scrolled content: both grids, each padded by 8 and 8 apart.
const CONTENT: f32 = 218.0;
/// The category headings' tops (FocusableTextWidget, padding 4).
const GENERAL_TOP: f32 = 8.0;
const MULTIPLAYER_TOP: f32 = 145.0;
/// The port box's top beside the LAN button.
const PORT_TOP: f32 = 166.0;
/// PopupScreen: 250 wide less its 18 pixel border each side, two buttons
/// `min((214 - 6) / 2, 150)` wide and 6 apart, rows 12 apart.
const POPUP_CONTENT: f32 = 214.0;
const POPUP_BUTTON: f32 = 104.0;
/// RestrictionsScreen's text widgets: 250 wide, padded by 4.
const RESTRICTION_WIDTH: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Button {
    DefaultMode,
    PersonalMode,
    Cheats,
    Difficulty,
    /// DifficultyButtons.lockButton (LockIconButton).
    Lock,
    GameRules,
    Restrictions,
    Lan,
    CommandAccess,
    Force,
    Apply,
    Cancel,
}

fn on_off(on: bool) -> Text {
    Text::translate(if on { "options.on" } else { "options.off" }, &[])
}
fn labelled(key: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[Text::translate(key, &[]), value])
}

impl WorldOptions {
    /// Vanilla 26.3 WorldOptionsScreen.generalOptions: guests see difficulty,
    /// game rules and restrictions; game modes, commands and LAN are host-only.
    fn visible_buttons(&self, w: f32) -> Vec<(Button, f32, f32, f32)> {
        if !self.remote {
            return Self::buttons(w, self.top());
        }
        let x = Self::left(w);
        let top = self.top();
        vec![
            (Button::Difficulty, x, top + 29.0, 130.0),
            (Button::Lock, x + 130.0, top + 29.0, 20.0),
            (Button::GameRules, x + 158.0, top + 29.0, 150.0),
            (Button::Restrictions, x, top + 53.0, 150.0),
        ]
    }
    fn content_height(&self) -> f32 {
        if self.remote { 81.0 } else { CONTENT }
    }
    fn scroll_limit(&self, h: f32) -> f32 {
        (self.content_height() - (h - HEADER - FOOTER)).max(0.0)
    }
    fn server_difficulty(&mut self, difficulty: u8, locked: bool) {
        // A server update is the new baseline, not a local edit. In particular,
        // the reply to Apply may arrive after the screen has already reopened.
        self.difficulty = difficulty;
        self.difficulty_locked = locked;
        self.locked = locked || self.hardcore;
        self.initial.difficulty = self.difficulty;
        self.initial.locked = self.locked;
    }
    fn parsed_port(&self) -> Option<u16> {
        if self.port.value.is_empty() {
            return Some(self.hint);
        }
        self.port.value.parse::<u16>().ok().filter(|p| *p >= 1024)
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            default_mode: self.default_mode,
            personal_mode: self.personal_mode,
            cheats: self.cheats,
            difficulty: self.difficulty,
            locked: self.locked,
            lan: self.lan,
            port: if self.lan {
                self.parsed_port().unwrap_or(0)
            } else {
                0
            },
            guest_commands: self.guest_commands && self.lan && self.cheats,
            force: self.force,
        }
    }
    fn changed(&self) -> bool {
        let valid = !self.lan || self.parsed_port().is_some();
        valid && self.snapshot() != self.initial
    }
    /// The scrolled content's top: under the header, less the scroll.
    fn top(&self) -> f32 {
        HEADER - self.scroll.trunc()
    }
    /// The grids' left edge: the content (308 wide, padded by 8) centred
    /// in a ScrollableLayout reserving 10 pixels for the scrollbar each side.
    fn left(w: f32) -> f32 {
        (w / 2.0).floor() - 154.0
    }
    /// The scrolled widgets: each grid's rows 4 apart, columns 8 apart.
    fn buttons(w: f32, top: f32) -> Vec<(Button, f32, f32, f32)> {
        let x = Self::left(w);
        let right = x + 158.0;
        vec![
            (Button::DefaultMode, x, top + 29.0, 308.0),
            (Button::PersonalMode, x, top + 53.0, 308.0),
            (Button::Cheats, x, top + 77.0, 150.0),
            // DifficultyButtons: the cycle button gives the lock its 20 pixels.
            (Button::Difficulty, right, top + 77.0, 130.0),
            (Button::Lock, right + 130.0, top + 77.0, 20.0),
            (Button::GameRules, x, top + 101.0, 150.0),
            (Button::Restrictions, right, top + 101.0, 150.0),
            (Button::Lan, x, top + 166.0, 150.0),
            (Button::CommandAccess, x, top + 190.0, 150.0),
            (Button::Force, right, top + 190.0, 150.0),
        ]
    }
    /// The footer's two buttons, centred in its 33 pixels (FrameLayout
    /// truncates the 6.5 pixel offset).
    fn footer(w: f32, h: f32) -> [(Button, f32, f32, f32); 2] {
        let x = Self::left(w);
        [
            (Button::Apply, x, h - FOOTER + 6.0, 150.0),
            (Button::Cancel, x + 158.0, h - FOOTER + 6.0, 150.0),
        ]
    }
    /// The widgets under the pointer's row: scrolled ones only inside the
    /// scroll area.
    fn hit(&self, (x, y): (f32, f32), (w, h): (f32, f32)) -> Option<Button> {
        let inside = y >= HEADER && y < h - FOOTER;
        self.visible_buttons(w)
            .into_iter()
            .filter(|_| inside)
            .chain(Self::footer(w, h))
            .find(|(b, bx, by, bw)| self.active(*b) && ui::hovered((x, y), *bx, *by, *bw, 20.0))
            .map(|b| b.0)
    }
    fn label(&self, b: Button) -> Text {
        let mode = |m: i32| {
            Text::translate(
                &format!("selectWorld.gameMode.{}", MODES[m as usize & 3]),
                &[],
            )
        };
        match b {
            Button::DefaultMode => {
                labelled("options.worldOptions.game_mode", mode(self.default_mode))
            }
            Button::PersonalMode => labelled(
                "options.worldOptions.personal_game_mode",
                mode(self.personal_mode),
            ),
            Button::Cheats => labelled("selectWorld.allowCommands", on_off(self.cheats)),
            Button::Lock => Text::translate("narrator.button.difficulty_lock", &[]),
            Button::GameRules => Text::translate("editGamerule.inGame.button", &[]),
            Button::Restrictions => Text::translate("restrictions_screen.button", &[]),
            Button::Difficulty => labelled(
                "options.difficulty",
                Text::translate(
                    &format!(
                        "options.difficulty.{}",
                        DIFFICULTIES[usize::from(self.difficulty) & 3]
                    ),
                    &[],
                ),
            ),
            Button::Lan => labelled("menu.multiplayerOptions.lan", on_off(self.lan)),
            Button::CommandAccess => labelled(
                "options.worldOptions.guest.command_access",
                on_off(self.guest_commands && self.lan && self.cheats),
            ),
            Button::Force => labelled(
                "options.worldOptions.guest.force_game_mode",
                on_off(self.force || !self.lan || self.hardcore),
            ),
            Button::Apply => Text::translate("menu.multiplayerOptions.applyChanges", &[]),
            Button::Cancel => Text::translate("gui.cancel", &[]),
        }
    }
    /// WorldOptionsScreen.updateButton and the update*Button methods.
    fn active(&self, b: Button) -> bool {
        if self.remote {
            return match b {
                Button::Difficulty | Button::Lock => {
                    self.gamemaster && !self.hardcore && !self.difficulty_locked
                }
                Button::GameRules => self.gamemaster,
                Button::Restrictions | Button::Cancel => true,
                Button::Apply => self.gamemaster && self.changed(),
                _ => false,
            };
        }
        match b {
            Button::DefaultMode | Button::PersonalMode => self.cheats && !self.hardcore,
            // An owner's commands come with Allow Commands.
            Button::Cheats => !self.hardcore || self.initial.cheats,
            // DifficultyButtons.updateDifficultyButtonsState: the level's
            // lock or hardcore, not the wanted lock.
            Button::Difficulty | Button::Lock => !self.hardcore && !self.difficulty_locked,
            // updateButton from the player's permission: the Allow Commands
            // switch leaves it (updatePermissionDependentButtons(.., false)).
            Button::GameRules => !self.hardcore && self.initial.cheats,
            Button::Restrictions => true,
            Button::CommandAccess => self.lan && self.cheats,
            Button::Force => self.lan && !(self.guest_commands && self.cheats) && !self.hardcore,
            Button::Apply => self.changed(),
            Button::Lan | Button::Cancel => true,
        }
    }
}

/// MultiLineLabel: the lines of `t` wrapped to `width`.
fn wrap(font: &Font, t: &Text, width: f32) -> Vec<Text> {
    font.split_lines(t, width)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}
fn widest(font: &Font, lines: &[Text]) -> f32 {
    lines.iter().map(|l| font.width_text(l)).fold(0.0, f32::max)
}
/// Centred MultiLineTextWidget lines: each centred on `mid` from `top`, 9 apart.
fn centred_lines(font: &Font, lines: Vec<Text>, mid: f32, top: f32) -> Vec<(Text, f32, f32)> {
    lines
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            (line, x, top + 9.0 * i as f32)
        })
        .collect()
}

/// PopupScreen centred on the screen: its title (bold) and message lines,
/// the background sprite's rectangle and its two buttons.
struct Popup {
    lines: Vec<(Text, f32, f32)>,
    frame: (f32, f32, f32, f32),
    buttons: [(f32, f32); 2],
}
impl Popup {
    fn new(font: &Font, title: &Text, message: &Text, (w, h): (f32, f32)) -> Self {
        let blocks = [
            wrap(font, title, POPUP_CONTENT),
            wrap(font, message, POPUP_CONTENT),
        ];
        let row = 2.0 * POPUP_BUTTON + 6.0;
        let lw = blocks.iter().map(|b| widest(font, b)).fold(row, f32::max);
        let lh = blocks
            .iter()
            .map(|b| 9.0 * b.len() as f32 + 12.0)
            .sum::<f32>()
            + 20.0;
        let (x, y) = (((w - lw) / 2.0).trunc(), ((h - lh) / 2.0).trunc());
        let mut lines = Vec::new();
        let mut top = y;
        for block in blocks {
            let bw = widest(font, &block);
            let mid = x + ((lw - bw) / 2.0).trunc() + (bw / 2.0).trunc();
            let n = block.len() as f32;
            lines.extend(centred_lines(font, block, mid, top));
            top += 9.0 * n + 12.0;
        }
        let bx = x + ((lw - row) / 2.0).trunc();
        Self {
            lines,
            frame: (x - 18.0, y - 18.0, lw + 36.0, lh + 36.0),
            buttons: [(bx, top), (bx + POPUP_BUTTON + 6.0, top)],
        }
    }
    /// The lock's question: WorldOptionsScreen's Apply when the lock changed.
    fn lock_question(font: &Font, difficulty: u8, size: (f32, f32)) -> Self {
        let name = Text::translate(
            &format!(
                "options.difficulty.{}",
                DIFFICULTIES[usize::from(difficulty) & 3]
            ),
            &[],
        );
        Self::new(
            font,
            &Text::translate("difficulty.lock.title", &[]).bold(),
            &Text::translate("difficulty.lock.question", &[name]),
            size,
        )
    }
}

/// RestrictionsScreen for the player of a single-player world, whose chat
/// abilities come from the Chat option alone (Minecraft.computeChatAbilities).
struct Restrictions {
    lines: Vec<(Text, f32, f32)>,
    /// The restriction's action, 200 wide: Chat Settings.
    action: Option<(f32, f32)>,
    /// The ScrollableLayout's top and bottom.
    area: (f32, f32),
}
impl Restrictions {
    fn new(font: &Font, chat_visibility: u8, (w, h): (f32, f32)) -> Self {
        let red = named_color("red").unwrap_or(0xFF5555);
        let green = named_color("green").unwrap_or(0x55FF55);
        let inner = RESTRICTION_WIDTH - 8.0;
        // ChatVisiblity HIDDEN takes chat and commands, SYSTEM the chat.
        let restriction = match chat_visibility {
            2 => Some("chat_restriction.chat_and_commands_disabled_by_options"),
            1 => Some("chat_restriction.chat_disabled_by_options"),
            _ => None,
        };
        let abilities = horizon_client::chat::ChatAbilities::new(
            horizon_client::chat::ChatVisibility::from_option(chat_visibility),
        );
        let (chat, commands) = (abilities.send_messages, abilities.send_commands);
        // The body's widgets: text blocks (lines, top) and the action's top.
        let mut blocks = Vec::new();
        let mut action = None;
        let mut y = 0.0;
        if let Some(key) = restriction {
            let lines = wrap(
                font,
                &Text::translate(key, &[]).bold().with_color(red),
                inner,
            );
            let n = lines.len() as f32;
            blocks.push((lines, y));
            y += 9.0 * n + 8.0;
            action = Some(y);
            // The button, then a 10 pixel SpacerElement.
            y += 20.0 + 10.0;
        }
        let permissions = [
            ("send_messages", chat),
            ("send_commands", commands),
            ("receive_system_messages", commands),
            ("receive_player_messages", chat),
        ];
        let lines: Vec<Text> = permissions
            .iter()
            .flat_map(|&(id, allowed)| {
                let state = if allowed { "allowed" } else { "denied" };
                let t =
                    Text::translate(&format!("restrictions_screen.permission.{id}.{state}"), &[])
                        .with_color(if allowed { green } else { red });
                wrap(font, &t, inner)
            })
            .collect();
        let n = lines.len() as f32;
        blocks.push((lines, y));
        y += 9.0 * n + 8.0;
        // HeaderAndFooterLayout: 30 under the header if it fits; the
        // ScrollableLayout keeps 10 pixels each side for its scrollbar.
        let height = y.min(h - HEADER - FOOTER);
        let top = (HEADER + 30.0).min(h - FOOTER - height);
        let x = ((w - (RESTRICTION_WIDTH + 20.0)) / 2.0).trunc() + 10.0;
        let mid = x + RESTRICTION_WIDTH / 2.0;
        Self {
            lines: blocks
                .into_iter()
                .flat_map(|(lines, by)| centred_lines(font, lines, mid, top + by + 4.0))
                .collect(),
            action: action.map(|ay| (x + 25.0, top + ay)),
            area: (top, top + height),
        }
    }
    /// The footer's Done, 200 wide.
    fn done(w: f32, h: f32) -> (f32, f32) {
        (((w - 200.0) / 2.0).trunc(), h - FOOTER + 6.0)
    }
}

impl Game<'_> {
    pub(super) fn remote_world_options_permission(&mut self, permission: bool) {
        if let Some(o) = self.world_options.as_mut() {
            let lost = o.gamemaster && !permission;
            o.gamemaster = permission;
            if lost && o.remote {
                self.close_world_options();
            }
        }
    }
    pub(super) fn remote_world_options_difficulty(&mut self, difficulty: u8, locked: bool) {
        if let Some(o) = self.world_options.as_mut() {
            o.server_difficulty(difficulty, locked);
        }
    }
    pub(super) fn remote_world_options_rules(&mut self, entries: Vec<play::GameRuleEntry>) {
        let Some(o) = self
            .world_options
            .as_mut()
            .filter(|o| o.view == View::GameRules)
        else {
            return;
        };
        let mut rules = horizon_client::gamerules::GameRules::default();
        for entry in entries {
            rules.set_string(&entry.name, &entry.value);
        }
        o.rules = Some(RulesScreen::new(&rules));
    }
    pub(super) fn open_world_options(&mut self) {
        let hint = horizon_client::lan::available_port();
        let mut port = EditBox::new(0.0, 0.0, 150.0, 20.0);
        port.max_length = 5;
        // The world is the server thread's when there is one.
        let state = match &self.integrated {
            Some(server) => match server.call(|g| g.server_options()) {
                Some(state) => state,
                None => return,
            },
            None => self.local_server_options(),
        };
        if let Some(p) = state.port {
            port.set_value(&p.to_string());
        }
        let mut options = WorldOptions {
            remote: self.external_server(),
            gamemaster: self.ui.registries.permission >= 2,
            default_mode: state.default_mode,
            personal_mode: state.personal_mode,
            cheats: state.cheats,
            difficulty: state.difficulty,
            lan: state.port.is_some(),
            port,
            hint,
            guest_commands: state.guest_commands,
            force: state.force,
            hardcore: state.hardcore,
            difficulty_locked: state.difficulty_locked,
            locked: state.difficulty_locked || state.hardcore,
            scroll: 0.0,
            view: View::Options,
            rules: None,
            initial: Snapshot {
                default_mode: 0,
                personal_mode: 0,
                cheats: false,
                difficulty: 0,
                locked: false,
                lan: false,
                port: 0,
                guest_commands: false,
                force: false,
            },
        };
        options.initial = options.snapshot();
        self.world_options = Some(options);
        self.screen = Screen::WorldOptions;
    }

    pub(super) fn close_world_options(&mut self) {
        self.world_options = None;
        self.screen = Screen::Paused;
    }

    /// Escape: the question and RestrictionsScreen return to the options
    /// (their onClose), which close; the game rules ask first.
    pub(super) fn world_options_escape(&mut self) {
        match self.world_options.as_mut() {
            Some(o) if o.view == View::GameRules => {
                if o.rules.is_none() {
                    // A server may be slow to answer REQUEST_GAMERULE_VALUES.
                    o.view = View::Options;
                } else if let Some(exit) = o.rules.as_mut().and_then(RulesScreen::escape) {
                    self.close_game_rules(exit);
                }
            }
            Some(o) if o.view != View::Options => o.view = View::Options,
            _ => self.close_world_options(),
        }
    }

    /// InWorldGameRulesScreen's exit, back to the options. Done sends the
    /// changed rules (ServerboundSetGameRulePacket), which the server's
    /// handleSetGameRule sets for a gamemaster and tells its operators.
    fn close_game_rules(&mut self, exit: Exit) {
        if let Some(o) = self.world_options.as_mut() {
            o.view = View::Options;
            o.rules = None;
        }
        let Exit::Apply(changes) = exit else {
            return;
        };
        if self.remote.is_some() {
            if self.ui.registries.permission >= 2 && !changes.is_empty() {
                self.send(S::SetGameRule(play::SetGameRule {
                    entries: changes
                        .into_iter()
                        .map(|(name, value)| play::GameRuleEntry {
                            name: format!("minecraft:{name}"),
                            value: value.to_string(),
                        })
                        .collect(),
                }));
            }
            return;
        }
        if !self.allow_commands() || changes.is_empty() {
            return;
        }
        for (name, value) in changes {
            self.ui.rules.set(name, value);
            self.system_message(Text::tr("commands.gamerule.set", &[&name, &value]));
        }
        self.sync_rules();
    }

    pub(super) fn draw_world_options(&mut self, batch: &mut HudBatch) {
        let Some(mut o) = self.world_options.take() else {
            return;
        };
        match o.view {
            View::Options => self.draw_options_view(&mut o, batch, self.cursor),
            View::LockQuestion => {
                // PopupScreen: the screen behind without the pointer, then the popup.
                self.draw_options_view(&mut o, batch, (-1.0, -1.0));
                self.draw_lock_question(&o, batch);
            }
            View::Restrictions => self.draw_restrictions(batch),
            View::GameRules => {
                if let Some(rules) = o.rules.as_mut() {
                    rules.draw(batch, &self.gui.atlas, self.cursor);
                } else {
                    ui::menu_background(
                        batch,
                        &self.gui.atlas,
                        0.0,
                        0.0,
                        batch.width,
                        batch.height,
                        true,
                    );
                }
            }
        }
        self.world_options = Some(o);
    }

    fn draw_lock_question(&self, o: &WorldOptions, batch: &mut HudBatch) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // Screen.extractTransparentBackground.
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, w, h),
            rgba(0x10, 0x10, 0x10, 0xC0),
            rgba(0x10, 0x10, 0x10, 0xD0),
        );
        let p = Popup::lock_question(&atlas.font, o.difficulty, (w, h));
        let (fx, fy, fw, fh) = p.frame;
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/popup/background",
            fx,
            fy,
            fw,
            fh,
            u32::MAX,
        );
        for (line, x, y) in &p.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for ((bx, by), key) in p.buttons.into_iter().zip(["gui.yes", "gui.no"]) {
            let hover = ui::hovered(self.cursor, bx, by, POPUP_BUTTON, 20.0);
            let label = Text::translate(key, &[]);
            ui::button(
                batch,
                atlas,
                bx,
                by,
                POPUP_BUTTON,
                20.0,
                &label,
                true,
                hover,
            );
        }
    }

    fn draw_restrictions(&self, batch: &mut HudBatch) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let title = Text::translate("restrictions_screen.title", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        let r = Restrictions::new(&atlas.font, self.settings.chat_visibility, (w, h));
        let first = batch.vertices.len();
        for (line, x, y) in &r.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        if let Some((ax, ay)) = r.action {
            let hover = ui::hovered(self.cursor, ax, ay, 200.0, 20.0);
            let label = Text::translate("chat_restriction.chat_disabled_by_options.action", &[]);
            ui::button(batch, atlas, ax, ay, 200.0, 20.0, &label, true, hover);
        }
        batch.clip_since(first, 0.0, r.area.0, w, r.area.1);
        let (dx, dy) = Restrictions::done(w, h);
        let hover = ui::hovered(self.cursor, dx, dy, 200.0, 20.0);
        let label = Text::translate("gui.done", &[]);
        ui::button(batch, atlas, dx, dy, 200.0, 20.0, &label, true, hover);
    }

    fn draw_options_view(&self, o: &mut WorldOptions, batch: &mut HudBatch, cursor: (f32, f32)) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        o.scroll = o.scroll.clamp(0.0, o.scroll_limit(h));
        // extractMenuBackground: the in-world background, the list
        // background under the content, then the separators over it.
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        ui::list_background(batch, atlas, HEADER, h - FOOTER, true);
        let title = Text::translate("options.worldOptions.title", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        let top = o.top();
        let first = batch.vertices.len();
        let x = WorldOptions::left(w);
        for (key, y) in [
            ("options.worldOptions.general.title", GENERAL_TOP),
            ("options.worldOptions.multiplayer.title", MULTIPLAYER_TOP),
        ] {
            if o.remote && key == "options.worldOptions.multiplayer.title" {
                continue;
            }
            // A FocusableTextWidget padded by 4, centred over both columns.
            let t = Text::translate(key, &[]).underlined().bold();
            let tw = atlas.font.width_text(&t);
            let tx = x + ((308.0 - (tw + 8.0)) / 2.0).trunc() + 4.0;
            batch.text_styled(atlas, &t, tx, top + y + 4.0, u32::MAX, true);
        }
        let inside = cursor.1 >= HEADER && cursor.1 < h - FOOTER;
        for (b, bx, by, bw) in o.visible_buttons(w) {
            let active = o.active(b);
            let hover = active && inside && ui::hovered(cursor, bx, by, bw, 20.0);
            if b == Button::Lock {
                let lock = if o.locked { "locked" } else { "unlocked" };
                let state = match (active, hover) {
                    (false, _) => "_disabled",
                    (true, true) => "_highlighted",
                    (true, false) => "",
                };
                let sprite = format!("gui/sprites/widget/{lock}_button{state}");
                batch.sprite_scaled(atlas, &sprite, bx, by, 20.0, 20.0, u32::MAX);
                continue;
            }
            ui::button(batch, atlas, bx, by, bw, 20.0, &o.label(b), active, hover);
        }
        // The port box beside the LAN button: editable only with LAN on,
        // its hint the port to use (setPortError colours the text).
        if !o.remote {
            o.port.x = x + 158.0;
            o.port.y = top + PORT_TOP;
            o.port.focused = o.lan && o.port.focused;
            o.port.text_color = if !o.lan {
                rgba(0x70, 0x70, 0x70, 255)
            } else if o.parsed_port().is_some() {
                rgba(0xE0, 0xE0, 0xE0, 255)
            } else {
                rgba(0xDF, 0x50, 0x50, 255)
            };
            o.port.draw(batch, atlas, None);
            if o.port.value.is_empty() && !o.port.focused {
                let hint = if o.lan {
                    Text::literal(o.hint.to_string())
                } else {
                    Text::translate("lanServer.port", &[])
                };
                // EditBox.DEFAULT_HINT_STYLE: dark gray.
                batch.text_styled(
                    atlas,
                    &hint,
                    o.port.text_x(),
                    o.port.text_y(),
                    rgba(0x55, 0x55, 0x55, 255),
                    true,
                );
            }
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::scrollbar(
            batch,
            atlas,
            (w / 2.0).floor() + 166.0,
            HEADER,
            h - FOOTER,
            o.content_height(),
            o.scroll.trunc(),
        );
        ui::separators(batch, atlas, HEADER, h - FOOTER, true);
        for (b, bx, by, bw) in WorldOptions::footer(w, h) {
            let active = o.active(b);
            let hover = active && ui::hovered(cursor, bx, by, bw, 20.0);
            ui::button(batch, atlas, bx, by, bw, 20.0, &o.label(b), active, hover);
        }
    }

    /// The mouse wheel over the screen (ScrollableLayout, 10 pixels a step).
    pub(super) fn world_options_scroll(&mut self, delta: f32) {
        let h = self.gui_size().1;
        match self.world_options.as_mut() {
            Some(o) if o.view == View::Options => {
                o.scroll = (o.scroll - delta * 10.0).clamp(0.0, o.scroll_limit(h));
            }
            Some(o) => {
                if let Some(rules) = o.rules.as_mut() {
                    rules.wheel(delta, h);
                }
            }
            None => {}
        }
    }

    /// The pointer moved while a list's scrollbar is held: the game rules'
    /// list follows it (AbstractSelectionList.mouseDragged).
    pub(super) fn world_options_drag(&mut self) {
        let h = self.gui_size().1;
        if let Some(o) = self.world_options.as_mut()
            && o.view == View::GameRules
            && let Some(rules) = o.rules.as_mut()
        {
            rules.drag(self.cursor.1, h);
        }
    }

    /// The button came up: a held scrollbar lets go (onRelease).
    pub(super) fn world_options_release(&mut self) {
        if let Some(o) = self.world_options.as_mut()
            && let Some(rules) = o.rules.as_mut()
        {
            rules.release();
        }
    }

    pub(super) fn world_options_click(&mut self) {
        let Some(mut o) = self.world_options.take() else {
            return;
        };
        let (w, h) = self.gui_size();
        match o.view {
            View::Options => {}
            View::GameRules => {
                let atlas = &self.gui.atlas;
                let exit = o.rules.as_mut().and_then(|rules| {
                    if rules.clicks_button(atlas, self.cursor, (w, h)) {
                        self.audio.play_button_click();
                    }
                    rules.click(atlas, self.cursor, (w, h))
                });
                self.world_options = Some(o);
                if let Some(exit) = exit {
                    self.close_game_rules(exit);
                }
                return;
            }
            View::LockQuestion => {
                let p = Popup::lock_question(&self.gui.atlas.font, o.difficulty, (w, h));
                let hit = p
                    .buttons
                    .iter()
                    .position(|&(bx, by)| ui::hovered(self.cursor, bx, by, POPUP_BUTTON, 20.0));
                if hit.is_some() {
                    self.audio.play_button_click();
                }
                match hit {
                    // Yes: apply, then back to the pause menu; No: back to the options.
                    Some(0) => return self.apply_and_close_world_options(o),
                    Some(_) => o.view = View::Options,
                    None => {}
                }
                self.world_options = Some(o);
                return;
            }
            View::Restrictions => {
                let r =
                    Restrictions::new(&self.gui.atlas.font, self.settings.chat_visibility, (w, h));
                let in_body = self.cursor.1 >= r.area.0 && self.cursor.1 < r.area.1;
                let action = r.action.is_some_and(|(ax, ay)| {
                    in_body && ui::hovered(self.cursor, ax, ay, 200.0, 20.0)
                });
                let (dx, dy) = Restrictions::done(w, h);
                let done = ui::hovered(self.cursor, dx, dy, 200.0, 20.0);
                if action || done {
                    self.audio.play_button_click();
                    o.view = View::Options;
                }
                self.world_options = Some(o);
                if action {
                    // ChatRestriction.Action.GO_TO_CHAT_SETTINGS: Chat Settings,
                    // which returns to the screen behind RestrictionsScreen.
                    self.open_options_at(false, horizon_client::options_ui::Page::Chat);
                    self.options_to_world_options = true;
                }
                return;
            }
        }
        let inside = self.cursor.1 >= HEADER && self.cursor.1 < h - FOOTER;
        o.port.focused = !o.remote
            && o.lan
            && inside
            && ui::hovered(
                self.cursor,
                WorldOptions::left(w) + 158.0,
                o.top() + PORT_TOP,
                150.0,
                20.0,
            );
        let hit = o.hit(self.cursor, (w, h));
        if hit.is_some() {
            self.audio.play_button_click();
        }
        match hit {
            Some(Button::DefaultMode) => o.default_mode = (o.default_mode + 1) % 4,
            Some(Button::PersonalMode) => o.personal_mode = (o.personal_mode + 1) % 4,
            Some(Button::Cheats) => {
                o.cheats = !o.cheats;
                // updatePermissionDependentButtons: without commands both
                // game modes return to the world's default.
                if !o.cheats {
                    o.default_mode = o.initial.default_mode;
                    o.personal_mode = o.default_mode;
                }
            }
            Some(Button::Difficulty) => o.difficulty = (o.difficulty + 1) % 4,
            Some(Button::Lock) => o.locked = !o.locked,
            Some(Button::Restrictions) => o.view = View::Restrictions,
            Some(Button::GameRules) => {
                if self.remote.is_some() {
                    o.rules = None;
                    self.send(S::ClientCommand(play::ClientCommand { action: 2 }));
                } else {
                    o.rules = Some(RulesScreen::new(&self.ui.rules));
                }
                o.view = View::GameRules;
            }
            Some(Button::Lan) => {
                o.lan = !o.lan;
                o.port.focused = false;
            }
            Some(Button::CommandAccess) => o.guest_commands = !o.guest_commands,
            Some(Button::Force) => o.force = !o.force,
            Some(Button::Cancel) => {
                self.world_options = Some(o);
                self.close_world_options();
                return;
            }
            // A new lock asks first.
            Some(Button::Apply) if o.locked != o.initial.locked => o.view = View::LockQuestion,
            Some(Button::Apply) => return self.apply_and_close_world_options(o),
            None => {}
        }
        self.world_options = Some(o);
    }

    fn apply_and_close_world_options(&mut self, o: WorldOptions) {
        let wanted = o.snapshot();
        let before = o.initial;
        self.world_options = Some(o);
        self.apply_world_options(&before, &wanted);
        self.close_world_options();
    }

    pub(super) fn world_options_key(&mut self, event: &KeyEvent) {
        if event.state != winit::event::ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        let ctrl = self.keys.contains(&winit::keyboard::KeyCode::ControlLeft);
        let shortcut = self.edit_shortcut(event);
        let types_text = self.types_text();
        let Some(o) = self.world_options.as_mut() else {
            return;
        };
        if let Some(rules) = o.rules.as_mut().filter(|r| r.typing()) {
            match shortcut {
                Some(shortcut) => rules.edit(|b| {
                    b.shortcut(shortcut, &mut self.clipboard);
                }),
                None => {
                    let mut handled = false;
                    rules.edit(|b| handled = b.key(key, ctrl, false));
                    if !handled && types_text {
                        self.world_options_text(event.text.as_deref());
                    }
                }
            }
            return;
        }
        if !o.port.focused || o.view != View::Options {
            return;
        }
        if let Some(shortcut) = shortcut {
            o.port.shortcut(shortcut, &mut self.clipboard);
            return;
        }
        if !o.port.key(key, ctrl, false) && types_text {
            self.world_options_text(event.text.as_deref());
        }
    }
    /// Digits typed into the port field (the keyboard, or a controller's
    /// on-screen keyboard), and its backspace.
    pub(super) fn world_options_text(&mut self, text: Option<&str>) {
        if let Some(rules) = self
            .world_options
            .as_mut()
            .and_then(|o| o.rules.as_mut())
            .filter(|r| r.typing())
        {
            match text {
                Some(text) => {
                    let typed: String = text.chars().filter(|c| !c.is_control()).collect();
                    if !typed.is_empty() {
                        rules.edit(|b| b.insert_text(&typed));
                    }
                }
                None => rules.edit(|b| {
                    b.key(winit::keyboard::KeyCode::Backspace, false, false);
                }),
            }
            return;
        }
        let Some(o) = self.world_options.as_mut().filter(|o| o.port.focused) else {
            return;
        };
        match text {
            Some(text) => {
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                if !digits.is_empty() {
                    o.port.insert_text(&digits);
                }
            }
            None => {
                o.port
                    .key(winit::keyboard::KeyCode::Backspace, false, false);
            }
        }
    }

    /// The world options of a world run in the frame.
    fn local_server_options(&self) -> ServerOptions {
        let (guest_commands, force) = self.lan.as_ref().map_or((false, true), |h| {
            (h.settings.guest_commands, h.settings.force_game_mode)
        });
        ServerOptions {
            default_mode: self.level.mode(),
            personal_mode: self.mode_id(),
            cheats: self.allow_commands(),
            difficulty: self.ui.difficulty,
            hardcore: self.level.hardcore,
            difficulty_locked: self.level.difficulty_locked,
            port: self.lan_port(),
            guest_commands,
            force,
        }
    }

    fn apply_world_options(&mut self, before: &Snapshot, wanted: &Snapshot) {
        if self.external_server() {
            if self.ui.registries.permission >= 2 {
                if wanted.difficulty != before.difficulty {
                    self.send(S::ChangeDifficulty(play::ChangeDifficultyRequest {
                        difficulty: i32::from(wanted.difficulty),
                    }));
                }
                if wanted.locked && !before.locked {
                    self.send(S::LockDifficulty(play::LockDifficulty { locked: true }));
                }
            }
            return;
        }
        if let Some(server) = &self.integrated {
            // The server applies them; the window tells what came of
            // Open to LAN (ShareToLanScreen's messages).
            let (b, w) = (*before, *wanted);
            server.call(move |g| g.apply_world_options(&b, &w));
            if w.lan != b.lan || (w.lan && w.port != b.port) {
                match (w.lan, self.lan_port()) {
                    (true, Some(port)) => self.system_message(Text::translate(
                        "menu.multiplayerOptions.publish.started.lan",
                        &[Text::literal(port.to_string())],
                    )),
                    (true, None) => {
                        self.system_message(Text::translate("commands.publish.failed", &[]));
                    }
                    (false, _) => self.system_message(Text::translate(
                        "menu.multiplayerOptions.publish.stopped",
                        &[],
                    )),
                }
            }
            return;
        }
        if wanted.difficulty != before.difficulty {
            let name = DIFFICULTIES[usize::from(wanted.difficulty) & 3];
            self.as_console(|g| g.run_command(&format!("difficulty {name}")));
        }
        // ServerboundLockDifficultyPacket(true): MinecraftServer.setDifficultyLocked.
        if wanted.locked && !before.locked {
            self.level.difficulty_locked = true;
        }
        if wanted.default_mode != before.default_mode {
            self.set_default_game_mode(wanted.default_mode);
        }
        if wanted.personal_mode != before.personal_mode {
            if self.serving() {
                self.set_owner_mode(wanted.personal_mode);
            } else {
                self.set_mode_id(wanted.personal_mode);
            }
        }
        if wanted.cheats != before.cheats {
            self.set_allow_commands(wanted.cheats);
            self.owner_allow_commands(wanted.cheats);
        }
        let settings_changed =
            wanted.guest_commands != before.guest_commands || wanted.force != before.force;
        if wanted.lan != before.lan || (wanted.lan && wanted.port != before.port) {
            self.lan_close(before.lan);
            if wanted.lan {
                let opened = self.lan_open(LanSettings {
                    port: wanted.port,
                    guest_commands: wanted.guest_commands,
                    force_game_mode: wanted.force,
                });
                if opened.is_err() {
                    self.system_message(Text::translate("commands.publish.failed", &[]));
                }
            }
        } else if settings_changed {
            self.lan_apply_settings(wanted.guest_commands, wanted.force);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(cheats: bool, hardcore: bool) -> WorldOptions {
        let mut o = WorldOptions {
            remote: false,
            gamemaster: cheats,
            default_mode: 1,
            personal_mode: 1,
            cheats,
            difficulty: 2,
            lan: false,
            port: EditBox::new(0.0, 0.0, 150.0, 20.0),
            hint: 25565,
            guest_commands: false,
            force: true,
            hardcore,
            difficulty_locked: false,
            locked: hardcore,
            scroll: 0.0,
            view: View::Options,
            rules: None,
            initial: Snapshot {
                default_mode: 0,
                personal_mode: 0,
                cheats: false,
                difficulty: 0,
                locked: false,
                lan: false,
                port: 0,
                guest_commands: false,
                force: false,
            },
        };
        o.initial = o.snapshot();
        o
    }

    #[test]
    fn layout_follows_the_header_footer_and_grids() {
        let at = |b: Button| {
            let o = options(true, false);
            WorldOptions::buttons(427.0, o.top())
                .into_iter()
                .chain(WorldOptions::footer(427.0, 240.0))
                .find(|e| e.0 == b)
                .map(|(_, x, y, w)| (x, y, w))
                .unwrap()
        };
        // Header 33, content padding 8, heading 17, rows 4 apart.
        assert_eq!(at(Button::DefaultMode), (59.0, 62.0, 308.0));
        assert_eq!(at(Button::Cheats), (59.0, 110.0, 150.0));
        assert_eq!(at(Button::Difficulty), (217.0, 110.0, 130.0));
        assert_eq!(at(Button::Lock), (347.0, 110.0, 20.0));
        assert_eq!(at(Button::Restrictions), (217.0, 134.0, 150.0));
        // The Multiplayer grid 24 below: padding, spacing, padding.
        assert_eq!(at(Button::Lan), (59.0, 199.0, 150.0));
        assert_eq!(at(Button::Force), (217.0, 223.0, 150.0));
        assert_eq!(at(Button::Apply), (59.0, 213.0, 150.0));
        // 218 pixels of content over 174 between the separators.
        assert_eq!(options(true, false).scroll_limit(240.0), 44.0);
        assert_eq!(options(true, false).scroll_limit(400.0), 0.0);
    }

    #[test]
    fn buttons_follow_commands_hardcore_and_lan() {
        let o = options(true, false);
        assert!(o.active(Button::DefaultMode) && o.active(Button::Difficulty));
        assert!(!o.active(Button::CommandAccess) && !o.active(Button::Force));
        assert!(!o.active(Button::Apply));
        assert!(o.active(Button::GameRules));
        let mut o = options(false, false);
        assert!(!o.active(Button::DefaultMode) && !o.active(Button::PersonalMode));
        // Game Rules follows the commands the player has, not the switch.
        assert!(!o.active(Button::GameRules));
        o.cheats = true;
        assert!(!o.active(Button::GameRules));
        let o = options(true, true);
        assert!(!o.active(Button::GameRules));
        let o = options(false, true);
        assert!(!o.active(Button::Cheats) && !o.active(Button::Difficulty));
        assert!(!o.active(Button::Force));
        // Scrolled widgets only take clicks between the separators.
        let mut o = options(true, false);
        o.scroll = 44.0;
        assert_eq!(o.hit((100.0, 30.0), (427.0, 240.0)), None);
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), Some(Button::Cheats));
    }

    #[test]
    fn the_lock_follows_the_level_and_counts_as_a_change() {
        let mut o = options(true, false);
        assert!(o.active(Button::Lock) && o.active(Button::Restrictions));
        assert!(!o.locked && !o.changed());
        // The wanted lock alone enables Apply (hasSettingsChanges).
        o.locked = true;
        assert!(o.changed() && o.active(Button::Difficulty));
        // A locked level: neither the difficulty nor the lock.
        let mut o = options(true, false);
        o.difficulty_locked = true;
        assert!(!o.active(Button::Lock) && !o.active(Button::Difficulty));
        // Hardcore shows the lock closed, and inactive.
        let o = options(true, true);
        assert!(o.locked && !o.active(Button::Lock));
    }

    #[test]
    fn remote_options_follow_vanilla_guest_layout_and_permissions() {
        let mut o = options(false, false);
        o.remote = true;
        // Vanilla omits host-only rows instead of disabling the whole screen.
        let rows = o.visible_buttons(427.0);
        assert_eq!(
            rows.iter().map(|b| b.0).collect::<Vec<_>>(),
            vec![
                Button::Difficulty,
                Button::Lock,
                Button::GameRules,
                Button::Restrictions,
            ]
        );
        assert_eq!(rows[0], (Button::Difficulty, 59.0, 62.0, 130.0));
        assert_eq!(rows[2], (Button::GameRules, 217.0, 62.0, 150.0));
        assert_eq!(rows[3], (Button::Restrictions, 59.0, 86.0, 150.0));
        assert_eq!(o.scroll_limit(240.0), 0.0);
        // A non-operator cannot activate the difficulty or rule controls.
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), None);
        assert_eq!(o.hit((240.0, 70.0), (427.0, 240.0)), None);
        assert_eq!(
            o.hit((100.0, 90.0), (427.0, 240.0)),
            Some(Button::Restrictions)
        );
        o.gamemaster = true;
        assert_eq!(
            o.hit((100.0, 70.0), (427.0, 240.0)),
            Some(Button::Difficulty)
        );
        assert_eq!(
            o.hit((240.0, 70.0), (427.0, 240.0)),
            Some(Button::GameRules)
        );
        o.difficulty_locked = true;
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), None);
        // Remote hardcore does not hide an operator's Game Rules screen.
        o.hardcore = true;
        assert!(o.active(Button::GameRules));
        assert!(!o.active(Button::Lock));
        for b in [
            Button::DefaultMode,
            Button::PersonalMode,
            Button::Cheats,
            Button::Lan,
            Button::CommandAccess,
            Button::Force,
        ] {
            assert!(!o.active(b));
        }
    }

    #[test]
    fn a_delayed_server_reply_does_not_mark_reopened_options_as_edited() {
        let mut o = options(false, false);
        o.remote = true;
        o.gamemaster = true;
        // Reopened while the previous Apply's reply was still in flight.
        o.server_difficulty(3, false);
        assert_eq!(o.difficulty, 3);
        assert!(!o.active(Button::Apply));
        o.difficulty = 0;
        assert!(o.active(Button::Apply));
        o.server_difficulty(3, true);
        assert!(!o.active(Button::Apply));
        assert!(o.locked);
        assert!(!o.active(Button::Difficulty));
    }
}
