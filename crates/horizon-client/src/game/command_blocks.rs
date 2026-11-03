//! Command blocks: `CommandBlock`'s placement (`setPlacedBy`), its scheduled
//! tick (`execute`, `executeChain`), `BaseCommandBlock.performCommand`
//! through the command system with the block's output, and
//! `CommandBlockEditScreen` with what `handleSetCommandBlock` does with it.
use super::{Game, Screen, commands::execute::Source, lan::LanHost};
use horizon_client::{
    chat::{ChatScreen, Commands},
    command_block::{self, CommandBlock, Mode},
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    screen_draw::Measure,
    text::Text,
    ui::{self, EditBox},
    world::World,
};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use horizon_world::{
    chunk::BlockState,
    level::flags,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

type Pos = (i32, i32, i32);

/// The open `CommandBlockEditScreen`.
pub(super) struct CommandBlockScreen {
    pos: Pos,
    /// The command block minecart being edited (`MinecartCommandBlockEditScreen`);
    /// `pos` is unused then.
    cart: Option<u32>,
    /// The command box with its `CommandSuggestions`.
    pub(super) command: ChatScreen,
    /// When the command box was focused, for its cursor's blink.
    opened: std::time::Instant,
    previous: EditBox,
    pub(super) track_output: bool,
    mode: Mode,
    conditional: bool,
    auto: bool,
}

/// What `ServerboundSetCommandBlockPacket` carries.
struct CommandBlockEdit {
    pos: Pos,
    command: String,
    track_output: bool,
    mode: Mode,
    conditional: bool,
    auto: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Output,
    Mode,
    Conditional,
    Auto,
    Done,
    Cancel,
}

/// `AbstractCommandBlockEditScreen.getPreviousY`.
fn previous_y(cart: bool) -> f32 {
    if cart { 150.0 } else { 135.0 }
}

/// `EditBox.textColorUneditable`.
const UNEDITABLE: u32 = 0xFF70_7070;

impl CommandBlockScreen {
    /// `AbstractCommandBlockEditScreen.init` and `CommandBlockEditScreen`'s
    /// buttons, in GUI pixels of a `w`×`h` screen.
    /// A minecart's screen has no mode buttons (`getPreviousY` 150).
    fn buttons(w: f32, h: f32, cart: bool) -> Vec<(Button, f32, f32, f32)> {
        let mid = (w / 2.0).floor();
        let low = (h / 4.0).floor() + 132.0;
        let mut out = vec![(Button::Output, mid + 130.0, previous_y(cart), 20.0)];
        if !cart {
            out.extend([
                (Button::Mode, mid - 154.0, 165.0, 100.0),
                (Button::Conditional, mid - 50.0, 165.0, 100.0),
                (Button::Auto, mid + 54.0, 165.0, 100.0),
            ]);
        }
        out.extend([
            (Button::Done, mid - 154.0, low, 150.0),
            (Button::Cancel, mid + 4.0, low, 150.0),
        ]);
        out
    }
    /// What button `b` narrates: the switches are `CycleButton`s that show
    /// only their value (`displayOnlyValue`), "<name>: <value> button" and
    /// the value a click moves to.
    fn narration(
        &self,
        b: Button,
        rect: [f32; 4],
        tr: super::screen_info::Tr<'_>,
    ) -> horizon_client::widgets::Narratable {
        let state = (self.track_output, self.mode, self.conditional, self.auto);
        button_narration(b, state, rect, tr)
    }
    fn label(&self, b: Button) -> Text {
        let tr = |key: &str| Text::translate(key, &[]);
        match b {
            Button::Output => Text::literal(if self.track_output { "O" } else { "X" }),
            Button::Mode => tr(self.mode.label()),
            Button::Conditional => tr(if self.conditional {
                "advMode.mode.conditional"
            } else {
                "advMode.mode.unconditional"
            }),
            Button::Auto => tr(if self.auto {
                "advMode.mode.autoexec.bat"
            } else {
                "advMode.mode.redstoneTriggered"
            }),
            Button::Done => tr("gui.done"),
            Button::Cancel => tr("gui.cancel"),
        }
    }
    pub(super) fn pos(&self) -> Pos {
        self.pos
    }
    /// `updatePreviousOutput`.
    fn show_previous(&mut self, block: &CommandBlock) {
        let shown = if self.track_output {
            block.last_output_string()
        } else {
            "-".to_owned()
        };
        self.previous.set_value(&shown);
    }
}

/// [`CommandBlockScreen::narration`] of the switches' state (track output,
/// mode, conditional, automatic).
fn button_narration(
    b: Button,
    (track_output, mode, conditional_on, auto_on): (bool, Mode, bool, bool),
    rect: [f32; 4],
    tr: super::screen_info::Tr<'_>,
) -> horizon_client::widgets::Narratable {
    use horizon_client::widgets::Narratable;
    let full_tr = tr;
    let tr = |key: &str| full_tr(key, &[]);
    let o_x = |on: bool| if on { "O" } else { "X" }.to_owned();
    let conditional = |on: bool| {
        tr(if on {
            "advMode.mode.conditional"
        } else {
            "advMode.mode.unconditional"
        })
    };
    let auto = |on: bool| {
        tr(if on {
            "advMode.mode.autoexec.bat"
        } else {
            "advMode.mode.redstoneTriggered"
        })
    };
    let (name, value, next) = match b {
        Button::Output => ("advMode.trackOutput", o_x(track_output), o_x(!track_output)),
        Button::Mode => ("advMode.mode", tr(mode.label()), tr(mode.next().label())),
        Button::Conditional => (
            "advMode.type",
            conditional(conditional_on),
            conditional(!conditional_on),
        ),
        Button::Auto => ("advMode.triggering", auto(auto_on), auto(!auto_on)),
        Button::Done => return Narratable::button(tr("gui.done"), rect, true),
        Button::Cancel => return Narratable::button(tr("gui.cancel"), rect, true),
    };
    let full = full_tr("options.generic_value", &[&tr(name), &value]);
    Narratable::cycle(full, next, rect, true)
}

fn state_of(registry: &BlockRegistry, state: BlockStateId) -> Option<&BlockState> {
    registry.state(state)
}
fn facing_step(state: &BlockState) -> [i32; 3] {
    match state.properties.get("facing").map(String::as_str) {
        Some("down") => [0, -1, 0],
        Some("up") => [0, 1, 0],
        Some("south") => [0, 0, 1],
        Some("west") => [-1, 0, 0],
        Some("east") => [1, 0, 0],
        _ => [0, 0, -1],
    }
}
/// `Direction.toYRot`: south 0, west 90, north 180, east (and up/down) 270.
fn facing_yaw(state: &BlockState) -> f32 {
    match state.properties.get("facing").map(String::as_str) {
        Some("south") => 0.0,
        Some("west") => 90.0,
        Some("north") => 180.0,
        _ => 270.0,
    }
}
fn conditional(state: &BlockState) -> bool {
    state
        .properties
        .get("conditional")
        .is_some_and(|c| c == "true")
}
fn offset(pos: Pos, step: [i32; 3]) -> Pos {
    (pos.0 + step[0], pos.1 + step[1], pos.2 + step[2])
}
fn command_block_mut(world: &mut World, pos: Pos) -> Option<&mut CommandBlock> {
    match world.block_entities.get_mut(&pos)? {
        BlockEntity::CommandBlock(c) => Some(c),
        _ => None,
    }
}

/// `CommandBlockEntity.markConditionMet`: a conditional block needs the
/// command block behind it to have succeeded.
pub(super) fn mark_condition_met(world: &mut World, registry: &BlockRegistry, pos: Pos) -> bool {
    let Some(state) = state_of(registry, world.block(pos.0, pos.1, pos.2)) else {
        return false;
    };
    let mut met = true;
    if conditional(state) {
        let back = facing_step(state).map(|v| -v);
        let behind = offset(pos, back);
        let behind_block = state_of(registry, world.block(behind.0, behind.1, behind.2))
            .is_some_and(|s| Mode::of_block(&s.name).is_some());
        met = behind_block
            && matches!(world.block_entities.get(&behind),
                Some(BlockEntity::CommandBlock(c)) if c.success_count > 0);
    }
    if let Some(c) = command_block_mut(world, pos) {
        c.condition_met = met;
    }
    met
}

/// `CommandBlockEntity.setPowered` from `CommandBlock.setPoweredAndUpdate`
/// (the level's neighbour hook): the previous flag and `isAutomatic`; a
/// rising signal that will tick the block checks its condition.
pub(super) fn set_powered(
    world: &mut World,
    registry: &BlockRegistry,
    pos: Pos,
    powered: bool,
) -> Option<(bool, bool)> {
    let mode = state_of(registry, world.block(pos.0, pos.1, pos.2))
        .and_then(|s| Mode::of_block(&s.name))?;
    let c = command_block_mut(world, pos)?;
    let (was, auto) = (c.powered, c.auto);
    c.powered = powered;
    if command_block::power_schedules(mode, was, powered, auto) {
        mark_condition_met(world, registry, pos);
    }
    Some((was, auto))
}

/// `getAnalogOutputSignal`: the success count.
pub(super) fn analog_signal(world: &World, pos: Pos) -> Option<i32> {
    match world.block_entities.get(&pos)? {
        BlockEntity::CommandBlock(c) => Some(c.success_count),
        _ => None,
    }
}

impl Game<'_> {
    /// `Player.canUseGameMasterBlocks`: creative with game master commands.
    pub(super) fn can_use_game_master_blocks(&self) -> bool {
        self.creative && self.ui.registries.permission >= 2
    }
    /// `CommandBlock.useWithoutItem` on a server's client, which opens the
    /// screen itself (`Player.openCommandBlock`) while the server only
    /// answers the click: the block at `pos`, when it is a command block the
    /// player may edit.
    pub(super) fn open_command_block_if_editable(&mut self, pos: Pos) {
        let is_command_block = self
            .block_state_at(pos)
            .is_some_and(|state| Mode::of_block(&state.name).is_some());
        if is_command_block && self.can_use_game_master_blocks() {
            self.open_command_block(pos);
        }
    }
    fn command_block(&self, pos: Pos) -> Option<&CommandBlock> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::CommandBlock(c) => Some(c),
            _ => None,
        }
    }
    /// The `BaseCommandBlock` an edit screen shows: the block entity's or the
    /// minecart's.
    fn edit_base(&self, s: &CommandBlockScreen) -> Option<&CommandBlock> {
        match s.cart {
            Some(id) => self.carts.iter().find(|c| c.id == id).map(|c| &c.command),
            None => self.command_block(s.pos),
        }
    }
    fn block_state_at(&self, pos: Pos) -> Option<&'static BlockState> {
        let registry = self.pack.assets.registry;
        state_of(registry, self.world.block(pos.0, pos.1, pos.2))
    }
    fn schedule_command_block(&mut self, pos: Pos) {
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.schedule_tick(level, ticks, at, 1));
    }
    /// `CommandBlockEntity.scheduleTick`.
    fn schedule_command_block_tick(&mut self, pos: Pos) {
        mark_condition_met(&mut self.world, self.pack.assets.registry, pos);
        self.schedule_command_block(pos);
    }
    fn update_output_signal(&mut self, pos: Pos) {
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        self.with_level(|b, level, ticks| b.update_neighbour_for_output_signal(level, ticks, at));
    }
    /// `ClientPacketListener.handleBlockEntityData` then
    /// `CommandBlockEditScreen.updateGui`: new data for the command block the
    /// edit screen shows refreshes its fields. The mode and conditional live in
    /// the block state here (vanilla's `CommandBlockEntity.Mode`).
    pub(super) fn command_block_data_arrived(&mut self, pos: Pos) {
        if self
            .command_block_screen
            .as_ref()
            .is_none_or(|s| s.cart.is_some() || s.pos != pos)
        {
            return;
        }
        let state = self.block_state_at(pos).map(|s| {
            (
                Mode::of_block(&s.name).unwrap_or(Mode::Redstone),
                conditional(s),
            )
        });
        let block = self.command_block(pos).cloned();
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        let Some(screen) = self.command_block_screen.as_mut() else {
            return;
        };
        if let Some((mode, conditional)) = state {
            screen.mode = mode;
            screen.conditional = conditional;
        }
        if let Some(block) = block {
            screen.command.suggest_command(&m, &c, &block.command);
            screen.track_output = block.track_output;
            screen.auto = block.auto;
            screen.show_previous(&block);
        }
    }

    /// `CommandBlock.setPlacedBy`: unless the item carried block entity data
    /// (`defaults` false), output tracking follows `send_command_feedback` and
    /// the block's own automatic flag; then the block takes the signal around it.
    pub(super) fn command_block_placed(&mut self, pos: Pos, defaults: bool) {
        let feedback = self.ui.rules.bool("send_command_feedback");
        let Some(name) = self.block_state_at(pos).map(|s| s.name.as_str()) else {
            return;
        };
        let auto = command_block::automatic_block(name);
        if defaults && let Some(c) = command_block_mut(&mut self.world, pos) {
            c.track_output = feedback;
            c.auto = auto;
        }
        let at = BlockPos::new(pos.0, pos.1, pos.2);
        let signal = self.with_level(|b, level, ticks| b.has_neighbor_signal(level, ticks, at));
        let registry = self.pack.assets.registry;
        if let Some((was, auto)) = set_powered(&mut self.world, registry, pos, signal)
            && signal
            && !was
            && !auto
            && !command_block::automatic_block(name)
        {
            self.schedule_command_block(pos);
        }
    }

    /// `CommandBlock.tick`.
    pub(super) fn command_block_tick(&mut self, pos: BlockPos) {
        let key = (pos.x, pos.y, pos.z);
        let Some(state) = self.block_state_at(key) else {
            return;
        };
        let Some(mode) = Mode::of_block(&state.name) else {
            return;
        };
        let Some(was_met) = self.command_block(key).map(|c| c.condition_met) else {
            return;
        };
        if mode == Mode::Auto {
            mark_condition_met(&mut self.world, self.pack.assets.registry, key);
        }
        match command_block::tick_action(mode, was_met, conditional(state)) {
            Some(true) => self.command_block_execute(key, state),
            Some(false) => {
                if let Some(c) = command_block_mut(&mut self.world, key) {
                    c.success_count = 0;
                }
            }
            None => {}
        }
        if let Some(c) = self.command_block(key)
            && command_block::repeats(mode, c.powered, c.auto)
        {
            self.schedule_command_block(key);
        }
        self.update_output_signal(key);
    }
    /// `CommandBlock.execute`: the command (or no successes without one),
    /// then the chain in front.
    fn command_block_execute(&mut self, pos: Pos, state: &BlockState) {
        let has_command = self
            .command_block(pos)
            .is_some_and(|c| !c.command.is_empty());
        if has_command {
            self.perform_command_block(pos);
        } else if let Some(c) = command_block_mut(&mut self.world, pos) {
            c.success_count = 0;
        }
        self.execute_chain(pos, facing_step(state));
    }
    /// `CommandBlock.executeChain`: the chain command blocks in front, each
    /// facing on to the next, up to `max_command_sequence_length`.
    fn execute_chain(&mut self, start: Pos, mut step: [i32; 3]) {
        let registry = self.pack.assets.registry;
        let limit = self.ui.rules.int("max_command_sequence_length");
        let mut remaining = limit;
        let mut pos = start;
        while remaining > 0 {
            remaining -= 1;
            pos = offset(pos, step);
            let Some(state) = self.block_state_at(pos) else {
                break;
            };
            if Mode::of_block(&state.name) != Some(Mode::Sequence) {
                break;
            }
            let Some(c) = self.command_block(pos) else {
                break;
            };
            if c.powered || c.auto {
                if mark_condition_met(&mut self.world, registry, pos) {
                    if !self.perform_command_block(pos) {
                        break;
                    }
                    self.update_output_signal(pos);
                } else if conditional(state)
                    && let Some(c) = command_block_mut(&mut self.world, pos)
                {
                    c.success_count = 0;
                }
            }
            step = facing_step(state);
        }
        if remaining <= 0 {
            log::warn!(
                "Command Block chain tried to execute more than {} steps!",
                limit.max(0)
            );
        }
    }
    /// `BaseCommandBlock.performCommand`: runs the command as the block, at
    /// game master level, counting the successful results. False when it
    /// already ran this game tick.
    fn perform_command_block(&mut self, pos: Pos) -> bool {
        let game_time = self.world.game_time;
        let Some(c) = command_block_mut(&mut self.world, pos) else {
            return false;
        };
        if c.last_execution == game_time {
            return false;
        }
        if c.command.eq_ignore_ascii_case("Searge") {
            c.last_output = Some(Text::literal("#itzlipofutzli"));
            c.success_count = 1;
            return true;
        }
        c.success_count = 0;
        let command = c.command.clone();
        if self.ui.rules.bool("command_blocks_work") && !command.is_empty() {
            if let Some(c) = command_block_mut(&mut self.world, pos) {
                c.last_output = None;
            }
            let Some(state) = self.block_state_at(pos) else {
                return false;
            };
            let source = Source {
                entity: None,
                pos: [
                    f64::from(pos.0) + 0.5,
                    f64::from(pos.1) + 0.5,
                    f64::from(pos.2) + 0.5,
                ],
                rot: [facing_yaw(state), 0.0],
                eyes: false,
                silent: false,
                store: None,
                dimension: self.dim,
                level: None,
                block: Some(pos),
                cart: None,
            };
            let successes = self.run_as_game_master(&command, source);
            if let Some(c) = command_block_mut(&mut self.world, pos) {
                c.success_count = successes;
            }
        }
        if let Some(c) = command_block_mut(&mut self.world, pos) {
            c.last_execution = if c.update_last_execution {
                game_time
            } else {
                -1
            };
        }
        true
    }
    /// `Commands.performPrefixedCommand` for a `BaseCommandBlock`: `command`
    /// from `source` at game master level, and how many of its results
    /// succeeded.
    pub(super) fn run_as_game_master(&mut self, command: &str, source: Source) -> i32 {
        // performPrefixedCommand: a leading slash is allowed.
        let line = command.strip_prefix('/').unwrap_or(command).to_owned();
        let outer = self.cmds.results.replace(Vec::new());
        let level = std::mem::replace(&mut self.ui.registries.permission, 2);
        self.with_source(source, |g| g.run_command(&line));
        self.ui.registries.permission = level;
        let results = std::mem::replace(&mut self.cmds.results, outer).unwrap_or_default();
        results.iter().filter(|(success, _)| *success).count() as i32
    }
    /// `CloseableCommandBlockSource.sendSystemMessage`: the block's last
    /// output, stamped with the time. Nothing when it doesn't track output
    /// (`CommandSource.NULL`).
    pub(super) fn command_block_output(&mut self, pos: Pos, message: Text) {
        if let Some(c) = command_block_mut(&mut self.world, pos)
            && c.track_output
        {
            let time = chrono::Local::now().format("[%H:%M:%S] ").to_string();
            c.last_output = Some(Text::literal(time).append(message));
        }
    }
    /// `CommandSourceStack.sendSuccess` from a command block: its output when
    /// `send_command_feedback`, and `broadcastToAdmins` when `broadcast` and
    /// `command_block_output`.
    pub(super) fn command_block_success(&mut self, pos: Pos, message: Text, broadcast: bool) {
        let Some(name) = self
            .command_block(pos)
            .filter(|c| c.track_output)
            .map(CommandBlock::name)
        else {
            return;
        };
        let feedback = self.ui.rules.bool("send_command_feedback");
        if feedback {
            self.command_block_output(pos, message.clone());
        }
        if broadcast && feedback && self.ui.rules.bool("command_block_output") {
            self.broadcast_source_to_admins(name, message);
        }
    }

    /// `LocalPlayer.openCommandBlock`.
    pub(super) fn open_command_block(&mut self, pos: Pos) {
        let Some(block) = self.command_block(pos).cloned() else {
            return;
        };
        let Some(state) = self.block_state_at(pos) else {
            return;
        };
        let mode = Mode::of_block(&state.name).unwrap_or(Mode::Redstone);
        self.open_edit_screen(pos, None, block, mode, conditional(state));
    }
    /// The screen over `block`: a command block's at `pos`, or a minecart's.
    pub(super) fn open_edit_screen(
        &mut self,
        pos: Pos,
        cart: Option<u32>,
        block: CommandBlock,
        mode: Mode,
        conditional: bool,
    ) {
        let (w, h) = self.gui_size();
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        let mut command = ChatScreen::command_block(&m, &c, &block.command, w as i32, h as i32);
        command.auto_suggestions = self.settings.auto_suggestions;
        command.input.message = super::screen_info::tr("advMode.command", &[]);
        let mut previous = EditBox::new(0.0, previous_y(cart.is_some()), 276.0, 20.0);
        previous.message = super::screen_info::tr("advMode.previousOutput", &[]);
        previous.max_length = 32500;
        previous.text_color = UNEDITABLE;
        previous.focused = false;
        let mut screen = CommandBlockScreen {
            pos,
            cart,
            command,
            opened: std::time::Instant::now(),
            previous,
            track_output: block.track_output,
            mode,
            conditional,
            auto: block.auto,
        };
        screen.show_previous(&block);
        self.command_block_screen = Some(screen);
        self.screen = Screen::CommandBlock;
        self.capture(false);
    }
    pub(super) fn close_command_block(&mut self) {
        self.command_block_screen = None;
        self.screen = Screen::Playing;
        self.capture(true);
    }
    pub(super) fn draw_command_block(&mut self, batch: &mut HudBatch) {
        let Some(mut s) = self.command_block_screen.take() else {
            return;
        };
        // AbstractCommandBlockEditScreen.tick: the block (or the cart) is gone.
        if self.edit_base(&s).is_none() {
            self.close_command_block();
            return;
        }
        // AbstractCommandBlockEditScreen: `GameNarrator.NO_TITLE`; its usage
        // is the suggestions' while they show (`getUsageNarration`).
        let usage = s.command.suggestions_usage();
        horizon_client::widgets::begin_screen(|| {
            let info = horizon_client::widgets::ScreenInfo::new("command_block", "");
            match usage {
                Some(key) => info.usage(super::screen_info::tr(key, &[])),
                None => info,
            }
        });
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let mid = (w / 2.0).floor();
        // Screen.extractBackground of an isInGameUi screen: the dimmed world,
        // with no blur and no menu background.
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, w, h),
            rgba(0x10, 0x10, 0x10, 0xC0),
            rgba(0x10, 0x10, 0x10, 0xD0),
        );
        let title = Text::translate("advMode.setCommand", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            (mid - tw / 2.0).floor(),
            20.0,
            u32::MAX,
            true,
        );
        let grey = rgba(160, 160, 160, 255);
        let label = Text::translate("advMode.command", &[]);
        batch.text_styled(atlas, &label, mid - 149.0, 40.0, grey, true);
        if !s.previous.value.is_empty() {
            let label = Text::translate("advMode.previousOutput", &[]);
            // 75 + 5 * 9 + 1 + getPreviousY() - 135, then the label's 4.
            let y = 125.0 + previous_y(s.cart.is_some()) - 135.0;
            batch.text_styled(atlas, &label, mid - 149.0, y, grey, true);
            s.previous.x = mid - 150.0;
            s.previous.draw(batch, atlas, None);
        }
        for (b, x, y, bw) in CommandBlockScreen::buttons(w, h, s.cart.is_some()) {
            let hover = ui::hovered(self.cursor, x, y, bw, 20.0);
            ui::button_as(
                batch,
                atlas,
                x,
                y,
                bw,
                20.0,
                &s.label(b),
                true,
                hover,
                |rect| s.narration(b, rect, &super::screen_info::tr),
            );
        }
        // The command box, then its suggestions over the widgets.
        let m = Measure(&atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        s.command.resize(&m, &c, w as i32, h as i32);
        let mouse = (self.cursor.0.floor() as i32, self.cursor.1.floor() as i32);
        let blink = s.opened.elapsed().as_millis() as u64;
        // The command box is added first.
        let mark = horizon_client::widgets::narratables_mark();
        let draws = s.command.draw(&m, mouse, blink, 0);
        horizon_client::widgets::move_narratables(mark, 0);
        horizon_client::screen_draw::paint(batch, atlas, &draws);
        self.command_block_screen = Some(s);
    }
    /// The wheel over the suggestion list (`CommandSuggestions.mouseScrolled`).
    pub(super) fn command_block_scroll(&mut self, amount: f32) {
        let mouse = (self.cursor.0.floor() as i32, self.cursor.1.floor() as i32);
        if let Some(s) = self.command_block_screen.as_mut() {
            s.command.scroll(mouse, f64::from(amount));
        }
    }
    /// The command box's completion request for an `ask_server` node.
    pub(super) fn command_block_suggestion_request(&mut self) -> Option<(i32, String)> {
        self.command_block_screen.as_mut()?.command.take_request()
    }
    /// The server's completions for the command box's `ask_server` request.
    pub(super) fn command_block_server_suggestions(
        &mut self,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let Some(mut s) = self.command_block_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        s.command.server_suggestions(&m, &c, id, range, list);
        self.command_block_screen = Some(s);
    }
    pub(super) fn command_block_click(&mut self) {
        let Some(mut s) = self.command_block_screen.take() else {
            return;
        };
        // CommandSuggestions.mouseClicked first, then the command box.
        {
            let m = Measure(&self.gui.atlas.font);
            let c = Commands {
                dispatcher: super::chat::chat_tree(&self.remote),
                registries: &self.ui.registries,
            };
            let (x, y) = (f64::from(self.cursor.0), f64::from(self.cursor.1));
            if s.command.click(&m, &c, x, y) {
                self.command_block_screen = Some(s);
                return;
            }
        }
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let hit = CommandBlockScreen::buttons(b.width, b.height, s.cart.is_some())
            .into_iter()
            .find(|(_, x, y, bw)| ui::hovered(self.cursor, *x, *y, *bw, 20.0))
            .map(|b| b.0);
        if hit.is_some() {
            self.audio.play_button_click();
        }
        match hit {
            Some(Button::Output) => {
                s.track_output = !s.track_output;
                if let Some(block) = self.edit_base(&s) {
                    s.show_previous(block);
                }
            }
            Some(Button::Mode) => s.mode = s.mode.next(),
            Some(Button::Conditional) => s.conditional = !s.conditional,
            Some(Button::Auto) => s.auto = !s.auto,
            Some(Button::Done) => {
                self.set_command_block(&s);
                self.close_command_block();
                return;
            }
            Some(Button::Cancel) => {
                self.close_command_block();
                return;
            }
            None => {}
        }
        self.command_block_screen = Some(s);
    }
    /// `AbstractCommandBlockEditScreen.keyPressed`: the suggestions first,
    /// then Enter is Done, Escape cancels, the rest edits the command.
    pub(super) fn command_block_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        let shortcut = self.edit_shortcut(event);
        self.command_block_key_code(key, shortcut, event.text.as_deref());
    }
    /// A key pressed on the command block screen, with the edit shortcut it
    /// makes and the text it types.
    pub(super) fn command_block_key_code(
        &mut self,
        key: KeyCode,
        shortcut: Option<horizon_client::clipboard::EditShortcut>,
        text: Option<&str>,
    ) {
        let ctrl = self.has_control_down_with_quirk();
        let shift = self.has_shift_down();
        // Windows' AltGr is Ctrl+Alt, and its characters still type.
        let types_text = self.types_text();
        let Some(mut s) = self.command_block_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        if s.command.suggestions_key(&m, &c, key, shift) {
            self.command_block_screen = Some(s);
            return;
        }
        let typed = match key {
            KeyCode::Escape | KeyCode::Enter | KeyCode::NumpadEnter => false,
            _ => {
                if let Some(shortcut) = shortcut {
                    s.command.shortcut(&m, &c, shortcut, &mut self.clipboard);
                    false
                } else {
                    !s.command.field_key(&m, &c, key, shift, ctrl) && types_text
                }
            }
        };
        self.command_block_screen = Some(s);
        if typed {
            self.command_block_type(text.unwrap_or(""));
        }
        match key {
            KeyCode::Escape => self.close_command_block(),
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(s) = self.command_block_screen.take() {
                    self.set_command_block(&s);
                }
                self.close_command_block();
            }
            _ => {}
        }
    }
    /// Characters typed into the command box (`StringUtil.isAllowedChatCharacter`).
    pub(super) fn command_block_type(&mut self, typed: &str) {
        let allowed: String = typed
            .chars()
            .filter(|&c| c != '\u{a7}' && c >= ' ' && c != '\u{7f}')
            .collect();
        if allowed.is_empty() {
            return;
        }
        let Some(mut s) = self.command_block_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: super::chat::chat_tree(&self.remote),
            registries: &self.ui.registries,
        };
        s.command.type_text(&m, &c, &allowed);
        self.command_block_screen = Some(s);
    }
    /// The edit screen's Done: a server's client sends the packet, the
    /// window's own world applies it (`handleSetCommandBlock`).
    fn set_command_block(&mut self, s: &CommandBlockScreen) {
        if !self.can_use_game_master_blocks() {
            self.system_message(Text::translate("advMode.notAllowed", &[]));
            return;
        }
        if let Some(id) = s.cart {
            self.set_command_minecart(id, s);
            return;
        }
        let edit = CommandBlockEdit {
            pos: s.pos,
            command: s.command.value().to_owned(),
            track_output: s.track_output,
            mode: s.mode,
            conditional: s.conditional,
            auto: s.auto,
        };
        if self.remote.is_some() {
            self.send(S::SetCommandBlock(play::SetCommandBlock {
                pos: [edit.pos.0, edit.pos.1, edit.pos.2],
                command: edit.command,
                mode: edit.mode.ordinal(),
                flags: i8::from(edit.track_output)
                    | (i8::from(edit.conditional) << 1)
                    | (i8::from(edit.auto) << 2),
            }));
        } else {
            self.apply_command_block(&edit);
        }
    }
    /// `ServerGamePacketListenerImpl.handleSetCommandBlock` for a guest: the
    /// block is edited only by a creative player with game master commands.
    pub(super) fn lan_set_command_block(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: play::SetCommandBlock,
    ) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let allowed = g.creative() && g.permission >= 2;
        let edit = CommandBlockEdit {
            pos: (p.pos[0], p.pos[1], p.pos[2]),
            command: p.command,
            track_output: p.flags & 1 != 0,
            mode: Mode::from_ordinal(p.mode),
            conditional: p.flags & 2 != 0,
            auto: p.flags & 4 != 0,
        };
        // The confirmation (`advMode.setCommand.success`) is the editing player's alone.
        self.as_guest_messaged(host, conn, |game| {
            if allowed {
                game.apply_command_block(&edit);
                // BaseCommandBlock.onUpdated: the clients get the new command.
                game.mark_block_entity_updated(edit.pos);
            } else {
                game.system_message(Text::translate("advMode.notAllowed", &[]));
            }
        });
    }
    /// `ServerPlayer.openCommandBlock`: the command block's data
    /// (`saveCustomOnly`) goes to the guest whose use opened it, as the
    /// chunk's update tag leaves it out and the edit screen shows it.
    pub(super) fn lan_open_command_block(&mut self, host: &mut LanHost, conn: u32, pos: Pos) {
        if !host
            .hub
            .guest(conn)
            .is_some_and(|g| g.creative() && g.permission >= 2)
        {
            return;
        }
        let Some(block) = self.command_block(pos) else {
            return;
        };
        let tag = Tag::compound(block.save());
        let state = self.world.block(pos.0, pos.1, pos.2).0;
        let Some(kind) = host.hub.encoder.entity_type(state) else {
            return;
        };
        host.hub.send_to(
            conn,
            &C::BlockEntityData(play::BlockEntityData {
                pos: [pos.0, pos.1, pos.2],
                kind,
                tag,
            }),
        );
    }
    /// What `handleSetCommandBlock` does once the player may edit the block.
    fn apply_command_block(&mut self, s: &CommandBlockEdit) {
        let pos = s.pos;
        let registry = self.pack.assets.registry;
        let Some(current) = self.block_state_at(pos) else {
            return;
        };
        let Some(old_mode) = Mode::of_block(&current.name) else {
            return;
        };
        if self.command_block(pos).is_none() {
            return;
        }
        // The mode is the block, the conditional flag its state; the block
        // entity stays.
        let mut wanted = current.clone();
        wanted.name = s.mode.block().to_owned();
        wanted
            .properties
            .insert("conditional".into(), s.conditional.to_string());
        if let Ok(id) = registry.resolve(&wanted)
            && wanted != *current
        {
            let entity = self.world.block_entities.take(&pos);
            let at = BlockPos::new(pos.0, pos.1, pos.2);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, at, id, flags::UPDATE_CLIENTS)
            });
            if let Some(entity) = entity {
                self.world.block_entities.put(pos, entity);
            }
            let _ = self.process_level_events();
        }
        let command = s.command.clone();
        let Some(c) = command_block_mut(&mut self.world, pos) else {
            return;
        };
        c.set_command(&command);
        c.track_output = s.track_output;
        if !s.track_output {
            c.last_output = None;
        }
        // CommandBlockEntity.setAutomatic, then onModeSwitch.
        let (was_auto, powered) = (c.auto, c.powered);
        c.auto = s.auto;
        if !was_auto && s.auto && !powered && s.mode != Mode::Sequence {
            self.schedule_command_block_tick(pos);
        }
        if old_mode != s.mode && s.mode == Mode::Auto && (powered || s.auto) {
            self.schedule_command_block_tick(pos);
        }
        if !command.is_empty() {
            let key = if self.ui.rules.bool("command_blocks_work") {
                "advMode.setCommand.success"
            } else {
                "advMode.setCommand.disabled"
            };
            self.system_message(Text::translate(key, &[Text::literal(command)]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minecart_screen_has_no_mode_buttons_and_a_lower_output_row() {
        let cart = CommandBlockScreen::buttons(400.0, 240.0, true);
        let kinds: Vec<_> = cart.iter().map(|b| b.0).collect();
        assert!(kinds == [Button::Output, Button::Done, Button::Cancel]);
        // MinecartCommandBlockEditScreen.getPreviousY.
        assert_eq!(cart[0].2, 150.0);
        let block = CommandBlockScreen::buttons(400.0, 240.0, false);
        assert_eq!(block.len(), 6);
        assert_eq!(block[0].2, 135.0);
        // Done and Cancel sit at the same height on both.
        assert_eq!(cart[1].2, block[4].2);
    }
}

#[cfg(test)]
mod narration_tests {
    use super::*;
    use horizon_client::widgets::What;

    fn keys(key: &str, args: &[&str]) -> String {
        if args.is_empty() {
            key.to_owned()
        } else {
            format!("{key}({})", args.join(", "))
        }
    }

    #[test]
    fn the_switches_narrate_as_cycle_buttons_showing_their_value() {
        let state = (true, Mode::Redstone, false, true);
        let cycle = |b| button_narration(b, state, [0.0; 4], &keys).what;
        let expect = |message: &str, next: &str| What::Cycle {
            message: message.into(),
            next: next.into(),
            active: true,
        };
        assert_eq!(
            cycle(Button::Output),
            expect("options.generic_value(advMode.trackOutput, O)", "X")
        );
        assert_eq!(
            cycle(Button::Mode),
            expect(
                "options.generic_value(advMode.mode, advMode.mode.redstone)",
                "advMode.mode.sequence"
            )
        );
        assert_eq!(
            cycle(Button::Conditional),
            expect(
                "options.generic_value(advMode.type, advMode.mode.unconditional)",
                "advMode.mode.conditional"
            )
        );
        assert_eq!(
            cycle(Button::Auto),
            expect(
                "options.generic_value(advMode.triggering, advMode.mode.autoexec.bat)",
                "advMode.mode.redstoneTriggered"
            )
        );
        assert_eq!(
            cycle(Button::Done),
            What::Button {
                message: "gui.done".into(),
                active: true
            }
        );
    }
}
