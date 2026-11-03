//! Chat: the message list (ChatComponent: wrapping, 100-line history,
//! 200-tick fade, scrolling, system message indicator) and the chat screen
//! (ChatScreen + EditBox + CommandSuggestions: history navigation, argument
//! colouring, usage hints, parse errors and tab completion).
use crate::{
    command::{
        Dispatcher, ParseResults, Registries, Suggestion, SuggestionsBuilder, merge_suggestions,
    },
    hud::{HudAtlas, HudBatch},
    hud_layout::{self, GuiDraw, PoseOp},
    screen_draw::{self as d, Draw, Measure, TextField},
    text::{self, ClickEvent, Style, Text},
};

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;

/// `CommandHistory.PERSISTED_COMMANDS_FILE_NAME` and its size cap.
const COMMAND_HISTORY_FILE: &str = "command_history.txt";
const MAX_PERSISTED_COMMAND_HISTORY: usize = 50;
/// GuiMessageTag.systemSinglePlayer's indicator colour.
pub const SYSTEM_INDICATOR: u32 = 0xD0D0D0;
/// `GuiMessageTag.CHAT_MODIFIED_INDICATOR_COLOR`.
const MODIFIED_INDICATOR: u32 = 0x60_6060;
/// A player message's signature (MessageSignature), which names it to a
/// server's deletion.
pub type Signature = horizon_protocol::secure_chat::Signature;
/// How long a message shows before a deletion replaces it
/// (`ChatComponent.deleteMessageOrDelay`: `addedTime + 60`).
const DELETION_DELAY: u64 = 60;
/// The smooth chat extra: the ticks a new line takes to slide in and fade in,
/// and how far left of its place it starts (GUI pixels).
const INTRO_TICKS: f32 = 5.0;
const INTRO_WIDTH: f32 = 12.0;

/// `ChatVisiblity`: the Chat option's three values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatVisibility {
    Full,
    System,
    Hidden,
}
impl ChatVisibility {
    /// Options.chatVisibility's saved value (0 full, 1 system, 2 hidden).
    pub fn from_option(value: u8) -> Self {
        match value {
            0 => Self::Full,
            1 => Self::System,
            _ => Self::Hidden,
        }
    }
}

/// `ChatAbilities`: what the Chat option allows, as `Minecraft.computeChatAbilities`
/// derives it from the visibility alone (`ChatRestriction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatAbilities {
    pub send_messages: bool,
    pub send_commands: bool,
    pub receive_player_messages: bool,
    pub receive_system_messages: bool,
}
impl ChatAbilities {
    /// `ChatRestriction.CHAT_AND_COMMANDS_DISABLED_BY_OPTIONS` (Hidden) takes
    /// chat, commands, player messages and system messages;
    /// `CHAT_DISABLED_BY_OPTIONS` (System) takes the chat and player messages.
    pub fn new(visibility: ChatVisibility) -> Self {
        match visibility {
            ChatVisibility::Full => Self {
                send_messages: true,
                send_commands: true,
                receive_player_messages: true,
                receive_system_messages: true,
            },
            ChatVisibility::System => Self {
                send_messages: false,
                send_commands: true,
                receive_player_messages: false,
                receive_system_messages: true,
            },
            ChatVisibility::Hidden => Self {
                send_messages: false,
                send_commands: false,
                receive_player_messages: false,
                receive_system_messages: false,
            },
        }
    }
    /// `visibleMessagesFilter`: the client's own lines always show.
    pub fn shows(&self, source: ChatSource) -> bool {
        match source {
            ChatSource::Player => self.receive_player_messages,
            ChatSource::Server => self.receive_system_messages,
            ChatSource::Client => true,
        }
    }
    /// `ChatAbilities.hasAnyRestrictions`: the Chat option takes something.
    pub fn any_restriction(&self) -> bool {
        *self != Self::new(ChatVisibility::Full)
    }
    /// Whether the chat screen may send `input` (`hasAllowedInput`).
    pub fn allows(&self, input: &str) -> bool {
        if input.starts_with('/') {
            self.send_commands
        } else {
            self.send_messages
        }
    }
}

impl Default for ChatAbilities {
    fn default() -> Self {
        Self::new(ChatVisibility::Full)
    }
}

/// Where a chat line came from (`GuiMessageSource`): the visibility filters
/// player messages and the server's, while the client's own always show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatSource {
    Player,
    Server,
    Client,
}

/// The chat options (doubles, as vanilla keeps them).
#[derive(Clone, Copy, Debug)]
pub struct ChatOptions {
    pub opacity: f64,
    pub background_opacity: f64,
    pub scale: f64,
    pub width: f64,
    pub height_focused: f64,
    pub height_unfocused: f64,
    pub line_spacing: f64,
    /// Chat Colors (`chatColors`): legacy `§` codes format the text, or are
    /// left out.
    pub colors: bool,
}
impl Default for ChatOptions {
    fn default() -> Self {
        Self {
            colors: true,
            opacity: 1.0,
            background_opacity: 0.5,
            scale: 1.0,
            width: 1.0,
            height_focused: 1.0,
            height_unfocused: 70.0 / 160.0,
            line_spacing: 0.0,
        }
    }
}
impl ChatOptions {
    /// ChatComponent.getWidth/getHeight.
    pub fn width_px(&self) -> i32 {
        (self.width * 280.0 + 40.0).floor() as i32
    }
    pub fn height_px(&self, focused: bool) -> i32 {
        ((if focused {
            self.height_focused
        } else {
            self.height_unfocused
        }) * 160.0
            + 20.0)
            .floor() as i32
    }
    /// `getLineHeight`.
    pub fn line_height(&self) -> i32 {
        (9.0 * (self.line_spacing + 1.0)) as i32
    }
}
#[derive(Clone, Debug)]
struct Message {
    added: u64,
    content: Text,
    tag: Option<std::sync::Arc<Tag>>,
    source: ChatSource,
    /// `GuiMessage.signature`: a signed player message's, for deletion.
    signature: Option<Box<Signature>>,
}
/// A wrapped line (`GuiMessage.Line`).
#[derive(Clone, Debug)]
struct Line {
    added: u64,
    content: Text,
    tag: Option<std::sync::Arc<Tag>>,
}

/// `GuiMessageTag`: the bar at the left of a message's lines, the tooltip
/// over it, and for a modified message the icon after each line.
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub color: u32,
    /// `GuiMessageTag.Icon.CHAT_MODIFIED`.
    pub icon: bool,
    pub text: Text,
}
impl Tag {
    /// `GuiMessageTag.systemSinglePlayer`, in `color`.
    pub fn system_single_player(color: u32) -> Self {
        Self {
            color,
            icon: false,
            text: Text::translate("chat.tag.system_single_player", &[]),
        }
    }
    /// `GuiMessageTag.chatNotSecure`.
    pub fn not_secure() -> Self {
        Self {
            color: SYSTEM_INDICATOR,
            icon: false,
            text: Text::translate("chat.tag.not_secure", &[]),
        }
    }
    /// `GuiMessageTag.chatError`: red (`CHAT_ERROR`, 16733525), the tooltip `chat.tag.error`.
    pub fn chat_error() -> Self {
        Self {
            color: 0xFF_5555,
            icon: false,
            text: Text::translate("chat.tag.error", &[]),
        }
    }
    /// `GuiMessageTag.chatModified`: the tooltip adds the original content in grey.
    pub fn modified(original_content: &str) -> Self {
        Self {
            color: MODIFIED_INDICATOR,
            icon: true,
            text: Text::empty()
                .append(Text::translate("chat.tag.modified", &[]))
                .append(Text::literal("\n"))
                .append(Text::literal(original_content).with_color(text::GRAY)),
        }
    }
}

/// `ChatComponent.AlphaCalculator.timeBased`: opaque for 180 ticks, then
/// fading out quadratically over 20.
fn time_alpha(ticks: f32, added: u64) -> f32 {
    let mut t = 1.0 - (ticks - added as f32) / 200.0;
    t = (t * 10.0).clamp(0.0, 1.0);
    t * t
}
#[derive(Default)]
pub struct Chat {
    messages: Vec<Message>,
    /// Wrapped lines, newest first.
    lines: Vec<Line>,
    /// Sent messages and commands (ChatComponent.recentChat).
    pub recent: Vec<String>,
    /// `CommandHistory`: the last commands, kept in `command_history.txt`
    /// in the game folder once `load_history` has set it.
    commands: Vec<String>,
    history_file: Option<std::path::PathBuf>,
    scroll: usize,
    new_since_scroll: bool,
    pub options: ChatOptions,
    /// The smooth chat extra: a new line slides in from the left while it
    /// fades in, instead of appearing whole.
    pub smooth: bool,
    /// The smooth chat extra: no indicator bar at the left of a line.
    pub hide_indicator: bool,
    /// Messages printed to stdout as `[CHAT]` (vanilla logs them).
    pub log: bool,
    /// Messages added since the start, for the control socket's events.
    pub received: u64,
    /// The Chat option's `visibleMessageFilter` (`set_abilities`).
    abilities: ChatAbilities,
    /// `ChatComponent.messageDeletionQueue`: deletions of messages still
    /// too new to replace, with the tick from which they may be.
    deletions: Vec<(Box<Signature>, u64)>,
}
impl Chat {
    pub fn lines_per_page(&self, focused: bool) -> usize {
        (self.options.height_px(focused) / self.options.line_height().max(1)) as usize
    }
    /// The wrapping width, `Mth.floor(getWidth() / getScale())`.
    fn max_width(&self) -> i32 {
        (f64::from(self.options.width_px()) / self.options.scale).floor() as i32
    }
    /// ChatComponent.addMessage: wrap, prepend and trim to 100 lines. A
    /// message the Chat option hides is dropped, unlogged.
    pub fn add(
        &mut self,
        atlas: &HudAtlas,
        content: Text,
        indicator: Option<u32>,
        source: ChatSource,
        ticks: u64,
        focused: bool,
    ) {
        let tag = indicator.map(|color| std::sync::Arc::new(Tag::system_single_player(color)));
        self.add_message(&atlas.font, content, tag, source, ticks, focused);
    }
    /// `add` of a player message with its signature, which a server's
    /// deletion names it by (`GuiMessage.signature`).
    pub fn add_signed(
        &mut self,
        atlas: &HudAtlas,
        content: Text,
        signature: Option<Signature>,
        tag: Option<Tag>,
        ticks: u64,
        focused: bool,
    ) {
        self.add_player_message(&atlas.font, content, signature, tag, ticks, focused);
    }
    #[cfg(test)]
    fn add_signed_message(
        &mut self,
        font: &crate::font::Font,
        content: Text,
        signature: Option<Signature>,
        ticks: u64,
        focused: bool,
    ) {
        self.add_player_message(font, content, signature, None, ticks, focused);
    }
    /// `ChatComponent.addPlayerMessage`: the tag the message's trust level made.
    fn add_player_message(
        &mut self,
        font: &crate::font::Font,
        content: Text,
        signature: Option<Signature>,
        tag: Option<Tag>,
        ticks: u64,
        focused: bool,
    ) {
        self.push_message(
            font,
            Message {
                added: ticks,
                content,
                tag: tag.map(std::sync::Arc::new),
                source: ChatSource::Player,
                signature: signature.map(Box::new),
            },
            focused,
        );
    }
    fn add_message(
        &mut self,
        font: &crate::font::Font,
        content: Text,
        tag: Option<std::sync::Arc<Tag>>,
        source: ChatSource,
        ticks: u64,
        focused: bool,
    ) {
        self.push_message(
            font,
            Message {
                added: ticks,
                content,
                tag,
                source,
                signature: None,
            },
            focused,
        );
    }
    fn push_message(&mut self, font: &crate::font::Font, message: Message, focused: bool) {
        if !self.abilities.shows(message.source) {
            return;
        }
        // ChatComponent.logChatMessage: every message shown is in the log.
        let plain = message.content.plain();
        crate::logging::write_only(log::Level::Info, &format!("[CHAT] {plain}"));
        if self.log {
            println!("[CHAT] {plain}");
        }
        self.add_lines(font, &message, focused);
        self.messages.insert(0, message);
        self.messages.truncate(100);
        self.received += 1;
    }
    fn add_lines(&mut self, font: &crate::font::Font, message: &Message, focused: bool) {
        // GuiMessage.splitLines: room for the tag's icon (its width, 4 before and 2 after).
        let icon = if message.tag.as_ref().is_some_and(|tag| tag.icon) {
            9 + 4 + 2
        } else {
            0
        };
        let width = self.max_width() - icon;
        let content = self.displayed(&message.content);
        for line in font.wrap_components(&content, width) {
            if focused && self.scroll > 0 {
                self.new_since_scroll = true;
                self.scroll_by(1, focused);
            }
            self.lines.insert(
                0,
                Line {
                    added: message.added,
                    content: line,
                    tag: message.tag.clone(),
                },
            );
        }
        self.lines.truncate(100);
    }
    /// What a message's lines wrap (ComponentRenderUtils.wrapComponents):
    /// without Chat Colors its `§` codes are stripped (stripColor); with
    /// them the font draws them as formatting.
    fn displayed(&self, content: &Text) -> Text {
        if self.options.colors {
            content.resolve_legacy()
        } else {
            content.strip_legacy()
        }
    }
    /// Re-wrap after a width or scale change (ChatComponent.rescaleChat),
    /// keeping only the messages the Chat option shows.
    pub fn rescale(&mut self, atlas: &HudAtlas) {
        self.rewrap(&atlas.font);
    }
    fn rewrap(&mut self, font: &crate::font::Font) {
        self.scroll = 0;
        self.new_since_scroll = false;
        self.lines.clear();
        let messages: Vec<Message> = self
            .messages
            .iter()
            .rev()
            .filter(|m| self.abilities.shows(m.source))
            .cloned()
            .collect();
        for m in &messages {
            self.add_lines(font, m, false);
        }
    }
    /// `ChatComponent.setVisibleMessageFilter`: the Chat option changed;
    /// the messages it now hides leave the chat, those it shows again return.
    pub fn set_abilities(&mut self, font: &crate::font::Font, abilities: ChatAbilities) {
        if self.abilities != abilities {
            self.abilities = abilities;
            self.rewrap(font);
        }
    }
    /// Whether a message from `source` would show (`visibleMessagesFilter`).
    pub fn shows(&self, source: ChatSource) -> bool {
        self.abilities.shows(source)
    }
    pub fn clear(&mut self) {
        self.messages.clear();
        self.lines.clear();
        self.deletions.clear();
        self.scroll = 0;
    }
    /// `ChatComponent.deleteMessage`: the server deleted a message. One
    /// shown for 60 ticks becomes the grey "deleted" marker now; a newer
    /// one waits until then (`messageDeletionQueue`).
    pub fn delete_message(&mut self, atlas: &HudAtlas, signature: &Signature, ticks: u64) {
        self.delete_message_with(&atlas.font, signature, ticks);
    }
    fn delete_message_with(&mut self, font: &crate::font::Font, signature: &Signature, ticks: u64) {
        if let Some(delayed) = self.delete_or_delay(font, signature, ticks) {
            self.deletions.push((Box::new(*signature), delayed));
        }
    }
    /// `ChatComponent.tick` (`processMessageDeletionQueue`): the delayed
    /// deletions whose message has shown long enough.
    pub fn tick(&mut self, atlas: &HudAtlas, ticks: u64) {
        self.process_deletions(&atlas.font, ticks);
    }
    fn process_deletions(&mut self, font: &crate::font::Font, ticks: u64) {
        let mut deletions = std::mem::take(&mut self.deletions);
        deletions.retain(|(signature, after)| {
            ticks < *after || self.delete_or_delay(font, signature, ticks).is_some()
        });
        self.deletions = deletions;
    }
    /// `ChatComponent.deleteMessageOrDelay`: the newest message with this
    /// signature, replaced by `createDeletedMarker` once 60 ticks have
    /// passed since it was added; otherwise the tick to retry from.
    fn delete_or_delay(
        &mut self,
        font: &crate::font::Font,
        signature: &Signature,
        ticks: u64,
    ) -> Option<u64> {
        let message = self
            .messages
            .iter_mut()
            .find(|m| m.signature.as_deref() == Some(signature))?;
        let deletable = message.added + DELETION_DELAY;
        if ticks < deletable {
            return Some(deletable);
        }
        *message = Message {
            added: message.added,
            content: Self::deleted_marker(),
            tag: Some(std::sync::Arc::new(Tag::system_single_player(
                SYSTEM_INDICATOR,
            ))),
            source: ChatSource::Server,
            signature: None,
        };
        // refreshTrimmedMessages.
        self.rewrap(font);
        None
    }
    /// `ChatComponent.DELETED_CHAT_MESSAGE`: grey, italic.
    fn deleted_marker() -> Text {
        Text::translate("chat.deleted_marker", &[])
            .with_color(text::GRAY)
            .italic()
    }
    /// `new CommandHistory(gameFolder)`: the commands kept from earlier runs
    /// start the chat's history (`ChatComponent`'s constructor).
    pub fn load_history(&mut self, game_folder: &std::path::Path) {
        let file = game_folder.join(COMMAND_HISTORY_FILE);
        if let Ok(text) = std::fs::read_to_string(&file) {
            self.commands = text.lines().map(str::to_owned).collect();
            self.recent.extend(self.commands.iter().cloned());
        }
        self.history_file = Some(file);
    }
    /// `ChatComponent.addRecentChat`: a command also goes to the saved history.
    pub fn add_recent(&mut self, message: &str) {
        if self.recent.last().map(String::as_str) != Some(message) {
            if self.recent.len() >= 100 {
                self.recent.remove(0);
            }
            self.recent.push(message.to_owned());
        }
        if message.starts_with('/') && self.commands.last().map(String::as_str) != Some(message) {
            if self.commands.len() >= MAX_PERSISTED_COMMAND_HISTORY {
                self.commands.remove(0);
            }
            self.commands.push(message.to_owned());
            if let Some(file) = &self.history_file {
                let mut text = self.commands.join("\n");
                text.push('\n');
                if let Err(e) = std::fs::write(file, text) {
                    log::error!(
                        "Failed to write {COMMAND_HISTORY_FILE}, command history will be missing: {e}"
                    );
                }
            }
        }
    }
    pub fn reset_scroll(&mut self) {
        self.scroll = 0;
        self.new_since_scroll = false;
    }
    pub fn scroll_by(&mut self, dir: i32, focused: bool) {
        let max = self
            .lines
            .len()
            .saturating_sub(self.lines_per_page(focused)) as i32;
        let pos = (self.scroll as i32 + dir).min(max);
        if pos <= 0 {
            self.scroll = 0;
            self.new_since_scroll = false;
        } else {
            self.scroll = pos as usize;
        }
    }
    /// `ChatComponent.extractRenderState` (background mode unless `focused`)
    /// on a screen `height` GUI pixels tall at `Hud.tickCount` `ticks`.
    pub fn layout(&self, height: i32, ticks: f32, focused: bool) -> Vec<GuiDraw> {
        let total = self.lines.len();
        if total == 0 {
            return Vec::new();
        }
        let o = &self.options;
        let scale = o.scale as f32;
        let max_width = hud_layout::ceil(o.width_px() as f32 / scale);
        let chat_bottom = hud_layout::floor((height - 40) as f32 / scale);
        let text_opacity = o.opacity as f32 * 0.9 + 0.1;
        let background_opacity = o.background_opacity as f32;
        let entry_height = o.line_height();
        let bottom_to_text =
            (8.0 * (o.line_spacing + 1.0) - 4.0 * o.line_spacing + 0.5).floor() as i32;
        // Vanilla's 200-tick fade reads the whole GUI ticks
        // (`Gui.getGuiTicks`); only the smooth chat extra eases it per frame.
        let fade_ticks = if self.smooth { ticks } else { ticks.floor() };
        let alpha_of = |line: &Line| {
            if focused {
                1.0
            } else {
                time_alpha(fade_ticks, line.added)
            }
        };
        // The smooth chat extra: how far into its intro a line is, 0 at the
        // tick it was added and 1 once it has arrived, as (alpha, offset x).
        let intro_of = |line: &Line| {
            if focused || !self.smooth {
                return (1.0, 0.0);
            }
            let t = ((ticks - line.added as f32) / INTRO_TICKS).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            (eased, (eased - 1.0) * INTRO_WIDTH)
        };
        let visible = total
            .saturating_sub(self.scroll)
            .min(self.lines_per_page(focused));
        let lines: Vec<(usize, &Line, f32, f32)> = (0..visible)
            .rev()
            .map(|i| (i, &self.lines[i + self.scroll]))
            .map(|(i, line)| {
                let (eased, slide) = intro_of(line);
                (i, line, alpha_of(line) * eased, slide)
            })
            .filter(|(_, _, alpha, _)| *alpha > 1.0e-5)
            .collect();
        let mut out = vec![GuiDraw::Push(vec![
            PoseOp::Scale(scale, scale),
            PoseOp::Translate(4.0, 0.0),
        ])];
        for &(i, _, alpha, slide) in &lines {
            let bottom = chat_bottom - i as i32 * entry_height;
            if slide != 0.0 {
                out.push(GuiDraw::Push(vec![PoseOp::Translate(slide, 0.0)]));
            }
            out.push(hud_layout::fill(
                -4,
                bottom - entry_height,
                max_width + 4 + 4,
                bottom,
                hud_layout::as8(alpha * background_opacity) << 24,
            ));
            if slide != 0.0 {
                out.push(GuiDraw::Pop);
            }
        }
        for &(i, line, alpha, slide) in &lines {
            let bottom = chat_bottom - i as i32 * entry_height;
            if slide != 0.0 {
                out.push(GuiDraw::Push(vec![PoseOp::Translate(slide, 0.0)]));
            }
            let opacity = alpha * text_opacity;
            let color = hud_layout::white(opacity);
            if color >> 24 != 0 {
                out.push(GuiDraw::Text {
                    text: line.content.clone(),
                    x: 0,
                    y: bottom - bottom_to_text,
                    color,
                    shadow: true,
                });
            }
            if let Some(tag) = line.tag.as_ref().filter(|_| !self.hide_indicator) {
                let color = hud_layout::as8(opacity) << 24 | tag.color & 0xFF_FFFF;
                out.push(hud_layout::fill(
                    -4,
                    bottom - entry_height,
                    -2,
                    bottom,
                    color,
                ));
            }
            if slide != 0.0 {
                out.push(GuiDraw::Pop);
            }
        }
        if focused {
            let count = lines.len() as i32;
            let chat_height = count * entry_height;
            let virtual_height = total as i32 * entry_height;
            let y = self.scroll as i32 * chat_height / total as i32 - chat_bottom;
            let height = chat_height * chat_height / virtual_height;
            if virtual_height != chat_height {
                let alpha: u32 = if y > 0 { 170 } else { 96 };
                let color = if self.new_since_scroll {
                    0xCC3333
                } else {
                    0x3333AA
                };
                let x = max_width + 4;
                out.push(hud_layout::fill(
                    x,
                    -y,
                    x + 2,
                    -y - height,
                    alpha << 24 | color,
                ));
                out.push(hud_layout::fill(
                    x + 2,
                    -y,
                    x + 1,
                    -y - height,
                    alpha << 24 | 0xCCCCCC,
                ));
            }
        }
        out.push(GuiDraw::Pop);
        out
    }
    /// Draws [`Chat::layout`] into the HUD batch, and with the chat screen
    /// open the restricted prompt above the lines.
    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, ticks: f32, focused: bool) {
        let draws = self.layout(batch.height as i32, ticks, focused);
        hud_layout::paint(batch, atlas, &draws);
        if focused && self.abilities.any_restriction() {
            let prompt = self.restricted_prompt(&atlas.font, batch.height as i32);
            hud_layout::paint(batch, atlas, &prompt);
        }
    }
    /// `ChatComponent.extractRenderState` for `FOREGROUND_RESTRICTED`: a
    /// background row and the red, underlined "Chat is restricted" line in
    /// the row above the messages, clipped to the chat's width.
    fn restricted_prompt(&self, font: &crate::font::Font, height: i32) -> Vec<GuiDraw> {
        let o = &self.options;
        let scale = o.scale as f32;
        let max_width = hud_layout::ceil(o.width_px() as f32 / scale);
        let chat_bottom = hud_layout::floor((height - 40) as f32 / scale);
        let entry_height = o.line_height();
        let bottom_to_text =
            (8.0 * (o.line_spacing + 1.0) - 4.0 * o.line_spacing + 0.5).floor() as i32;
        let shown = self
            .lines
            .len()
            .saturating_sub(self.scroll)
            .min(self.lines_per_page(true));
        let top = chat_bottom - (shown as i32 + 1) * entry_height;
        let mut prompt = Text::translate("chat_screen.restricted", &[]).with_style(&Style {
            color: Some(text::RED),
            underlined: Some(true),
            ..Style::default()
        });
        // ComponentRenderUtils.clipText.
        let mut width = 0.0;
        let fits = prompt
            .chars()
            .take_while(|(c, style)| {
                width += font.advance(*c, style);
                width <= max_width as f32
            })
            .count();
        if fits < prompt.chars().count() {
            prompt = prompt.slice(0, fits);
        }
        vec![
            GuiDraw::Push(vec![
                PoseOp::Scale(scale, scale),
                PoseOp::Translate(4.0, 0.0),
            ]),
            hud_layout::fill(
                -2,
                top,
                max_width + 4 + 4,
                top + entry_height,
                hud_layout::as8(o.background_opacity as f32) << 24,
            ),
            GuiDraw::Text {
                text: prompt,
                x: 0,
                y: top + entry_height - bottom_to_text - 1,
                color: hud_layout::white(o.opacity as f32 * 0.9 + 0.1),
                shadow: true,
            },
            GuiDraw::Pop,
        ]
    }
    /// The click action under a GUI-space cursor while chat is focused.
    pub fn click_at(
        &self,
        font: &crate::font::Font,
        height: i32,
        x: f64,
        y: f64,
    ) -> Option<ClickEvent> {
        self.style_at(font, height, x, y)?.click
    }
    /// The hover text under a GUI-space cursor while chat is focused
    /// (ChatScreen.render's `renderComponentHoverEffect`).
    pub fn hover_at(&self, font: &crate::font::Font, height: i32, x: f64, y: f64) -> Option<Text> {
        self.style_at(font, height, x, y)?.hover.map(|h| *h)
    }
    /// The wrapped line under a GUI-space cursor while chat is focused, and
    /// the cursor's x within it (the text starts at 0).
    fn line_at(&self, height: i32, x: f64, y: f64) -> Option<(&Line, f64)> {
        let scale = self.options.scale as f32;
        let x = x / f64::from(scale) - 4.0;
        let bottom = hud_layout::floor((height - 40) as f32 / scale);
        let from_bottom = f64::from(bottom) - y / f64::from(scale);
        if x >= f64::from(self.max_width()) || from_bottom <= 0.0 {
            return None;
        }
        let row = (from_bottom / f64::from(self.options.line_height())).ceil() as usize - 1;
        if row >= self.lines_per_page(true) {
            return None;
        }
        Some((self.lines.get(self.scroll + row)?, x))
    }
    /// ChatComponent.getClickedComponentStyleAt.
    pub fn style_at(
        &self,
        font: &crate::font::Font,
        height: i32,
        x: f64,
        y: f64,
    ) -> Option<crate::text::Style> {
        let (line, mut x) = self.line_at(height, x, y)?;
        if x < 0.0 {
            return None;
        }
        let content = line.content.resolve_legacy();
        for (c, style) in content.chars() {
            x -= f64::from(font.advance(c, style));
            if x < 0.0 {
                return Some(style.clone());
            }
        }
        None
    }
    /// `DrawingFocusedGraphicsAccess.handleTag` and `handleTagIcon` with the
    /// cursor at GUI position `(x, y)` while chat is focused: the tooltip of
    /// the tag under it (its bar, or its icon) and the icons to draw, those
    /// under the cursor and those of the message the cursor is over.
    pub fn focused_tags(
        &self,
        font: &crate::font::Font,
        height: i32,
        x: f64,
        y: f64,
    ) -> (Option<Text>, Vec<GuiDraw>) {
        let o = &self.options;
        let scale = o.scale as f32;
        let chat_bottom = hud_layout::floor((height - 40) as f32 / scale);
        let entry_height = o.line_height();
        let bottom_to_text =
            (8.0 * (o.line_spacing + 1.0) - 4.0 * o.line_spacing + 0.5).floor() as i32;
        let (mouse_x, mouse_y) = (x / f64::from(scale) - 4.0, y / f64::from(scale));
        let inside = |left: i32, top: i32, right: i32, bottom: i32| {
            mouse_x >= f64::from(left)
                && mouse_x < f64::from(right)
                && mouse_y >= f64::from(top)
                && mouse_y < f64::from(bottom)
        };
        let visible = self
            .lines
            .len()
            .saturating_sub(self.scroll)
            .min(self.lines_per_page(true));
        let mut tooltip = None;
        let mut icons = Vec::new();
        let mut hovered_message = false;
        for i in (0..visible).rev() {
            let index = i + self.scroll;
            let line = &self.lines[index];
            let bottom = chat_bottom - i as i32 * entry_height;
            let top = bottom - entry_height;
            let width = font.width_text(&line.content).ceil() as i32;
            hovered_message |= inside(0, top, width, bottom);
            // GuiMessage.Line.endOfEntry: the next newer line is another message's.
            let end_of_entry = index == 0
                || match (&line.tag, &self.lines[index - 1].tag) {
                    (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
                    (None, None) => false,
                    _ => true,
                };
            let force_icon = end_of_entry && hovered_message;
            if end_of_entry {
                hovered_message = false;
            }
            let Some(tag) = &line.tag else { continue };
            if inside(-4, top, -2, bottom) {
                tooltip = Some(tag.text.clone());
            }
            if tag.icon {
                let (left, text_bottom) = (width + 4, bottom - bottom_to_text + 9);
                let icon_top = text_bottom - 9 - 1;
                let over = inside(left, icon_top, left + 9, text_bottom);
                if over {
                    tooltip = Some(tag.text.clone());
                }
                if force_icon || over {
                    icons.push(GuiDraw::Sprite {
                        sprite: "icon/chat_modified".into(),
                        x: left,
                        y: icon_top,
                        w: 9,
                        h: 9,
                        region: None,
                        color: hud_layout::WHITE,
                        blend: hud_layout::Blend::Textured,
                    });
                }
            }
        }
        if icons.is_empty() {
            return (tooltip, icons);
        }
        let mut draws = vec![GuiDraw::Push(vec![
            PoseOp::Scale(scale, scale),
            PoseOp::Translate(4.0, 0.0),
        ])];
        draws.extend(icons);
        draws.push(GuiDraw::Pop);
        (tooltip, draws)
    }
    /// Plain text of the newest messages (for tests and the control socket).
    pub fn recent_lines(&self, n: usize) -> Vec<String> {
        self.messages
            .iter()
            .take(n)
            .map(|m| m.content.plain())
            .collect()
    }
    /// The newest messages as components, styles included (control socket).
    pub fn recent_components(&self, n: usize) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .take(n)
            .map(|m| m.content.component())
            .collect()
    }
}

/// `SuggestionsList.getNarrationMessage` for entry `index` of `count`.
fn suggestion_narration(index: usize, count: usize, text: &str, tooltip: Option<&str>) -> Text {
    let mut args = vec![
        Text::literal((index + 1).to_string()),
        Text::literal(count.to_string()),
        Text::literal(text),
    ];
    match tooltip {
        Some(tooltip) => {
            args.push(Text::literal(tooltip));
            Text::translate("narration.suggestion.tooltip", &args)
        }
        None => Text::translate("narration.suggestion", &args),
    }
}

/// `CommandSuggestions.SuggestionsList`.
#[derive(Clone, Debug)]
struct SuggestionList {
    /// `rect`: x, y, width, height.
    rect: [i32; 4],
    original: String,
    list: Vec<Suggestion>,
    offset: usize,
    current: usize,
    last_mouse: (i32, i32),
    tab_cycles: bool,
    /// `lastNarratedEntry`: the entry the narrator last said (-1: none yet).
    last_narrated: isize,
}
impl SuggestionList {
    /// `getNarrationMessage`: "Selected suggestion 2 out of 5: …", with its
    /// tooltip when it has one. It marks the entry said.
    fn narration_message(&mut self) -> Text {
        self.last_narrated = self.current as isize;
        let s = &self.list[self.current];
        suggestion_narration(self.current, self.list.len(), &s.text, s.tooltip.as_deref())
    }
    fn contains(&self, x: i32, y: i32) -> bool {
        let [rx, ry, w, h] = self.rect;
        x >= rx && x <= rx + w && y >= ry && y <= ry + h
    }
}

/// What a key did on the chat screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    None,
    /// Escape: the screen closes.
    Close,
    /// Enter: the message is sent and the screen closes.
    Submit(String),
}

/// The dispatcher and registries the suggestions come from.
pub struct Commands<'a> {
    pub dispatcher: &'a Dispatcher,
    pub registries: &'a Registries,
}

/// `CommandSuggestions`' constructor arguments: how a screen's suggestions
/// parse and where they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SuggestLayout {
    /// `commandsOnly`: the input is a command without a leading `/`.
    pub commands_only: bool,
    /// `onlyShowIfCursorPastError`: completions from the command's start,
    /// not only past the `/`.
    pub only_past_error: bool,
    /// `suggestionLineLimit`.
    pub line_limit: usize,
    /// `anchorToBottom`: the list and usage over a box at the bottom, else
    /// under one at the top (y 72).
    pub anchor_to_bottom: bool,
    /// `fillColor`: behind the list and the usage lines.
    pub fill: i32,
}
impl SuggestLayout {
    /// ChatScreen.init's `CommandSuggestions`.
    pub const CHAT: Self = Self {
        commands_only: false,
        only_past_error: false,
        line_limit: 10,
        anchor_to_bottom: true,
        fill: FILL,
    };
    /// AbstractCommandBlockEditScreen.init's `CommandSuggestions`.
    pub const COMMAND_BLOCK: Self = Self {
        commands_only: true,
        only_past_error: true,
        line_limit: 7,
        anchor_to_bottom: false,
        fill: i32::MIN,
    };
}

/// The open chat screen: vanilla's `ChatScreen` with its `EditBox` and
/// `CommandSuggestions` (argument colouring, usage lines, parse errors,
/// the suggestion list, Tab completion, history).
pub struct ChatScreen {
    pub input: TextField,
    width: i32,
    height: i32,
    history_buffer: String,
    history_pos: usize,
    parse: Option<ParseResults>,
    /// The completions at the cursor (`pendingSuggestions`): range start (bytes) and list.
    pending: Option<(usize, Vec<Suggestion>)>,
    /// Completions waiting for the server's (an `ask_server` node at the
    /// cursor): `pendingSuggestions` is not done until they come.
    awaiting: Option<Awaiting>,
    /// The request to send (`ServerboundCommandSuggestionPacket`): id and
    /// the command up to the cursor.
    request: Option<(i32, String)>,
    suggestions: Option<SuggestionList>,
    usage: Vec<Text>,
    usage_position: i32,
    usage_width: i32,
    allow_suggestions: bool,
    keep_suggestions: bool,
    pub auto_suggestions: bool,
    /// `CommandSuggestions.setRestrictions`: what the Chat option allows.
    pub abilities: ChatAbilities,
    pub layout: SuggestLayout,
}
/// Local completions waiting for the server's reply to request `id`.
struct Awaiting {
    id: i32,
    /// The command up to the cursor, as sent.
    truncated: String,
    local: Vec<Suggestion>,
}
/// `ClientSuggestionProvider.pendingSuggestionsId`: one counter for the
/// connection, so a reply to an older request never matches.
static SUGGESTION_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `CommandSuggestions`' fill colour.
const FILL: i32 = -805306368;
fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}
fn chars_to(s: &str, byte: usize) -> usize {
    s[..byte.min(s.len())].chars().count()
}
/// `Mth.clamp(int)`: the minimum wins over a smaller maximum.
fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo { lo } else { v.min(hi) }
}
/// `CommandSuggestions.getLastWordIndex`: after the last whitespace run.
fn last_word_index(s: &str) -> usize {
    let mut out = 0;
    let mut prev_space = false;
    for (i, c) in s.char_indices() {
        let space = matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r');
        if prev_space && !space {
            out = i;
        }
        prev_space = space;
    }
    if prev_space { s.len() } else { out }
}
/// `ChatScreen.normalizeChatMessage`: `String.trim`, then
/// `StringUtils.normalizeSpace` (one space for every run of whitespace), then
/// `StringUtil.trimChatMessage` (the first 256 UTF-16 code units, the length
/// `ServerboundChatPacket`'s `stringUtf8(256)` decodes). The chat screen sends
/// this, not what was typed: a line of spaces sends nothing, and a message the
/// packet cannot hold is cut on the client, as vanilla's is, instead of by the
/// server.
pub fn normalize_chat_message(message: &str) -> String {
    // String.trim: the code points up to U+0020, which is not `str::trim`'s
    // Unicode whitespace (U+00A0 is not Java whitespace and stays).
    let trimmed = message.trim_matches(|c: char| c <= ' ');
    truncate_utf16(&normalize_space(trimmed), 256)
}
/// `StringUtils.normalizeSpace`: one space for every run of
/// `Character.isWhitespace`, and none at either end.
fn normalize_space(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if crate::ui::java_whitespace(c) {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
}
/// `StringUtil.truncateStringIfNecessary(s, 256, false)`: Java counts UTF-16
/// code units (`String.length`), so 129 emoji are cut to 128 of them, where
/// `chars().take(256)` would keep all 129 (258 units) for the server to
/// reject. A cut between the halves of a surrogate pair keeps the character
/// below it: vanilla's `substring` leaves half a pair, which a Java `String`
/// may hold and a Rust `String` may not.
fn truncate_utf16(s: &str, max_units: usize) -> String {
    let mut units = 0;
    for (byte, c) in s.char_indices() {
        units += c.len_utf16();
        if units > max_units {
            return s[..byte].to_owned();
        }
    }
    s.to_owned()
}
impl ChatScreen {
    /// `ChatScreen.init`: the input at the bottom, focused, holding `initial`.
    pub fn new(
        m: &Measure,
        c: &Commands,
        initial: &str,
        history_len: usize,
        width: i32,
        height: i32,
        abilities: ChatAbilities,
    ) -> Self {
        let mut input = TextField::new(4, height - 12, width - 4, 12);
        input.max_length = 256;
        input.bordered = false;
        input.focused = true;
        input.set_value(m, initial);
        let mut s = Self {
            input,
            width,
            height,
            history_buffer: String::new(),
            history_pos: history_len,
            parse: None,
            pending: None,
            awaiting: None,
            request: None,
            suggestions: None,
            usage: Vec::new(),
            usage_position: 0,
            usage_width: 0,
            allow_suggestions: false,
            keep_suggestions: false,
            auto_suggestions: true,
            abilities,
            layout: SuggestLayout::CHAT,
        };
        s.update_command_info(m, c);
        s
    }
    /// `AbstractCommandBlockEditScreen.init`: the command box at the top,
    /// focused, holding `command`, with suggestions from the start.
    pub fn command_block(
        m: &Measure,
        c: &Commands,
        command: &str,
        width: i32,
        height: i32,
    ) -> Self {
        let mut input = TextField::new(width / 2 - 150, 50, 300, 20);
        input.max_length = 32500;
        input.focused = true;
        input.set_value(m, command);
        let mut s = Self {
            input,
            width,
            height,
            history_buffer: String::new(),
            history_pos: 0,
            parse: None,
            pending: None,
            awaiting: None,
            request: None,
            suggestions: None,
            usage: Vec::new(),
            usage_position: 0,
            usage_width: 0,
            allow_suggestions: true,
            keep_suggestions: false,
            auto_suggestions: true,
            abilities: ChatAbilities::new(ChatVisibility::Full),
            layout: SuggestLayout::COMMAND_BLOCK,
        };
        s.update_command_info(m, c);
        s
    }
    pub fn value(&self) -> &str {
        &self.input.value
    }
    /// Replace the input for a suggest-command link and refresh completions.
    pub fn suggest_command(&mut self, m: &Measure, c: &Commands, command: &str) {
        self.input.set_value(m, command);
        self.edited(m, c);
    }
    /// `ChatScreen.resize`: a new screen with the current text.
    pub fn resize(&mut self, m: &Measure, c: &Commands, width: i32, height: i32) {
        if (width, height) != (self.width, self.height) {
            let value = self.input.value.clone();
            let auto = self.auto_suggestions;
            let abilities = self.abilities;
            let message = std::mem::take(&mut self.input.message);
            *self = if self.layout == SuggestLayout::CHAT {
                Self::new(m, c, &value, self.history_pos, width, height, abilities)
            } else {
                Self::command_block(m, c, &value, width, height)
            };
            self.auto_suggestions = auto;
            self.input.message = message;
        }
    }
    /// `ChatScreen.onEdited`.
    fn edited(&mut self, m: &Measure, c: &Commands) {
        self.allow_suggestions = true;
        self.update_command_info(m, c);
    }
    /// Ctrl+A/C/V/X in the input box.
    pub fn shortcut(
        &mut self,
        m: &Measure,
        c: &Commands,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) {
        if self.input.shortcut(m, shortcut, clipboard) {
            self.edited(m, c);
        }
    }
    /// Typed text (`charTyped` per character).
    pub fn type_text(&mut self, m: &Measure, c: &Commands, text: &str) {
        for ch in text.chars() {
            if ch >= ' '
                && ch != '\u{7f}'
                && ch != '§'
                && self.input.insert_text(m, &ch.to_string())
            {
                self.edited(m, c);
            }
        }
    }
    /// `CommandSuggestions.updateCommandInfo`.
    fn update_command_info(&mut self, m: &Measure, c: &Commands) {
        let command = self.input.value.clone();
        if self.parse.as_ref().is_some_and(|p| p.input != command) {
            self.parse = None;
        }
        if !self.keep_suggestions {
            self.input.suggestion = None;
            self.suggestions = None;
        }
        self.usage.clear();
        let cursor = self.input.cursor;
        let cursor_b = byte_at(&command, cursor);
        let slash = command.starts_with('/');
        if self.layout.commands_only || slash {
            let start = usize::from(slash);
            if self.parse.is_none() {
                self.parse = Some(c.dispatcher.parse_at(&command, start, c.registries));
            }
            let from = if self.layout.only_past_error {
                start
            } else {
                1
            };
            if cursor >= from && (self.suggestions.is_none() || !self.keep_suggestions) {
                let parse = self.parse.as_ref().expect("parsed above");
                let (range, list, ask) =
                    c.dispatcher
                        .client_suggestions_asking(parse, cursor_b, c.registries);
                if ask {
                    // ClientSuggestionProvider.customSuggestion: the
                    // suggestions are done when the server answers.
                    let id = SUGGESTION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let truncated = command[..cursor_b].to_owned();
                    self.request = Some((id, truncated.clone()));
                    self.awaiting = Some(Awaiting {
                        id,
                        truncated,
                        local: list,
                    });
                    self.pending = None;
                } else {
                    self.awaiting = None;
                    self.pending = Some((range.0, list));
                    self.update_usage_info(m, c);
                }
            }
        } else if !command.chars().all(char::is_whitespace) {
            let partial = &command[..cursor_b];
            let mut builder = SuggestionsBuilder::new(partial, last_word_index(partial));
            // ClientSuggestionProvider.getCustomTabSuggestions: the players
            // and the server's custom chat completions.
            builder.suggest_matching(c.registries.custom_tab_suggestions());
            let (range, list) = merge_suggestions(partial, builder.out);
            self.awaiting = None;
            self.pending = Some((range.0, list));
            self.usage_width = 0;
            self.usage_position = 0;
            // CommandSuggestions.updateCommandInfo: the Chat option may take
            // the messages.
            if !self.abilities.send_messages {
                let line =
                    Text::translate("chat_screen.messages_not_allowed", &[]).with_color(text::RED);
                self.usage_width = m.width(&line);
                self.usage.push(line);
            }
        } else {
            self.awaiting = None;
            self.pending = None;
        }
    }
    /// The completion request to send to the server, if one is waiting.
    pub fn take_request(&mut self) -> Option<(i32, String)> {
        self.request.take()
    }
    /// `ClientSuggestionProvider.completeCustomSuggestions`: the server's
    /// completions for request `id`, with `range` in UTF-16 units of the
    /// command sent. They join the local ones (`Suggestions.merge`) and the
    /// pending suggestions are done.
    pub fn server_suggestions(
        &mut self,
        m: &Measure,
        c: &Commands,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let Some(awaiting) = self.awaiting.take_if(|a| a.id == id) else {
            return;
        };
        let input = &awaiting.truncated;
        let byte = |units: usize| {
            let mut n = 0;
            input
                .char_indices()
                .find(|(_, ch)| {
                    let past = n >= units;
                    n += ch.len_utf16();
                    past
                })
                .map_or(input.len(), |(i, _)| i)
        };
        let range = (byte(range.0), byte(range.0 + range.1));
        let mut all = awaiting.local;
        all.extend(list.into_iter().map(|(text, tooltip)| Suggestion {
            range,
            text,
            tooltip,
            int_value: None,
        }));
        let (range, list) = merge_suggestions(input, all);
        self.pending = Some((range.0, list));
        self.update_usage_info(m, c);
    }
    /// `CommandSuggestions.updateUsageInfo`.
    fn update_usage_info(&mut self, m: &Measure, c: &Commands) {
        let Some(parse) = &self.parse else { return };
        let empty = self.pending.as_ref().is_none_or(|p| p.1.is_empty());
        let mut trailing = false;
        if self.input.cursor == self.input.value.chars().count() {
            if empty && !parse.errors.is_empty() {
                let mut literals = 0;
                for (_, e) in &parse.errors {
                    if e.literal_incorrect {
                        literals += 1;
                    } else {
                        self.usage.push(e.with_context());
                    }
                }
                if literals > 0 {
                    let reader = crate::command::Reader {
                        string: &parse.input,
                        cursor: parse.cursor,
                    };
                    self.usage.push(
                        crate::command::CommandError::tr_at(
                            "command.unknown.argument",
                            &[],
                            &reader,
                        )
                        .with_context(),
                    );
                }
            } else if parse.can_read() {
                trailing = true;
            }
        }
        let cursor_b = byte_at(&self.input.value, self.input.cursor);
        let (node, start, _) = parse.context.suggestion_context(cursor_b);
        if self.usage.is_empty() {
            let entries: Vec<Text> = c
                .dispatcher
                .smart_usage(node, c.registries.permission)
                .into_iter()
                .filter(|(n, _)| {
                    !matches!(
                        c.dispatcher.nodes[*n].kind,
                        crate::command::NodeKind::Literal(_)
                    )
                })
                .map(|(_, u)| Text::colored(u, text::GRAY))
                .collect();
            if entries.is_empty()
                && trailing
                && let Some(e) = parse.error()
            {
                self.usage.push(e.with_context());
            }
            self.usage.extend(entries);
        }
        // CommandSuggestions.updateUsageInfo: the Chat option may take the
        // commands, under the usage. (Vanilla also counts a command with a
        // message argument, `/say hi`, as a message; Horizon's parse keeps no
        // argument kinds, so only a bare message does.)
        if !self.abilities.send_commands {
            self.usage.push(
                Text::translate("chat_screen.commands_not_allowed", &[]).with_color(text::RED),
            );
        }
        self.usage_width = self.usage.iter().map(|u| m.width(u)).max().unwrap_or(0);
        self.usage_position = if self.usage.is_empty() {
            0
        } else {
            let start = chars_to(&self.input.value, start);
            clamp(
                self.input.screen_x(m, start),
                0,
                self.input.screen_x(m, 0) + self.input.inner_width() - self.usage_width,
            )
        };
        self.suggestions = None;
        if self.allow_suggestions && self.auto_suggestions {
            self.show_suggestions(m, false);
        }
    }
    /// `CommandSuggestions.showSuggestions`: `immediate` (Tab asked for
    /// them) has the narrator say the first one at once.
    fn show_suggestions(&mut self, m: &Measure, immediate: bool) {
        let Some((start, list)) = self.pending.clone() else {
            return;
        };
        if list.is_empty() {
            return;
        }
        let width = list.iter().map(|s| m.width_str(&s.text)).max().unwrap_or(0);
        let value = self.input.value.clone();
        let x = clamp(
            self.input.screen_x(m, chars_to(&value, start)),
            0,
            self.input.screen_x(m, 0) + self.input.inner_width() - width,
        );
        let y = if self.layout.anchor_to_bottom {
            self.height - 12
        } else {
            72
        };
        // sortSuggestions: those starting with the word at the cursor first.
        let partial = &value[..byte_at(&value, self.input.cursor)];
        let word = partial[last_word_index(partial)..].to_lowercase();
        let (mut first, rest): (Vec<Suggestion>, Vec<Suggestion>) =
            list.into_iter().partition(|s| {
                s.text.starts_with(&word) || s.text.starts_with(&format!("minecraft:{word}"))
            });
        first.extend(rest);
        // SuggestionsList: beside a bordered box, or over an unbordered one.
        let n = first.len().min(self.layout.line_limit) as i32;
        let bordered = i32::from(self.input.bordered);
        let top = if self.layout.anchor_to_bottom {
            y - 3 - n * 12
        } else {
            y - bordered
        };
        let mut l = SuggestionList {
            rect: [x - (1 - bordered), top, width + 1, n * 12],
            original: value,
            list: first,
            offset: 0,
            current: 0,
            last_mouse: (0, 0),
            tab_cycles: false,
            last_narrated: if immediate { -1 } else { 0 },
        };
        self.select(&mut l, 0);
        self.suggestions = Some(l);
    }
    /// `SuggestionsList.select`: the grey completion after the text.
    fn select(&mut self, l: &mut SuggestionList, index: isize) {
        let n = l.list.len() as isize;
        let mut i = index;
        if i < 0 {
            i += n;
        }
        if i >= n {
            i -= n;
        }
        l.current = i as usize;
        let applied = l.list[l.current].apply(&l.original);
        self.input.suggestion = applied
            .strip_prefix(self.input.value.as_str())
            .map(str::to_owned);
        if l.last_narrated != l.current as isize {
            horizon_narrator::say_system_now(&l.narration_message().plain());
        }
    }
    fn cycle(&mut self, l: &mut SuggestionList, dir: isize) {
        self.select(l, l.current as isize + dir);
        let limit = self.layout.line_limit;
        let max = l.list.len().saturating_sub(limit);
        if l.current < l.offset {
            l.offset = l.current.min(max);
        } else if l.current >= l.offset + limit {
            l.offset = (l.current as isize + 1 - limit as isize).clamp(0, max as isize) as usize;
        }
    }
    /// `SuggestionsList.useSuggestion`.
    fn use_suggestion(&mut self, m: &Measure, c: &Commands, l: &mut SuggestionList) {
        let s = l.list[l.current].clone();
        self.keep_suggestions = true;
        let value = s.apply(&l.original);
        self.suggestions = Some(l.clone());
        self.input.set_value(m, &value);
        self.edited(m, c);
        let end = chars_to(&l.original, s.range.0) + s.text.chars().count();
        self.input.set_cursor_and_highlight(m, end);
        self.select(l, l.current as isize);
        self.keep_suggestions = false;
        l.tab_cycles = true;
    }
    /// `ChatScreen.keyPressed`.
    pub fn key(
        &mut self,
        m: &Measure,
        c: &Commands,
        history: &[String],
        key: winit::keyboard::KeyCode,
        shift: bool,
        ctrl: bool,
    ) -> ChatAction {
        use winit::keyboard::KeyCode as K;
        if self.suggestions_key(m, c, key, shift) {
            return ChatAction::None;
        }
        match key {
            K::Escape => return ChatAction::Close,
            K::Enter | K::NumpadEnter => {
                // ChatScreen.keyPressed: without `hasAllowedInput` the input
                // stays, unsent, and the screen stays open.
                if !self.abilities.allows(&self.input.value) {
                    return ChatAction::None;
                }
                return ChatAction::Submit(self.normalized());
            }
            K::ArrowUp => self.move_in_history(m, c, history, -1),
            K::ArrowDown => self.move_in_history(m, c, history, 1),
            _ => {
                self.field_key(m, c, key, shift, ctrl);
            }
        }
        ChatAction::None
    }
    /// The input box's own keys (`EditBox.keyPressed`), the suggestions
    /// following an edit: whether the box took the key.
    pub fn field_key(
        &mut self,
        m: &Measure,
        c: &Commands,
        key: winit::keyboard::KeyCode,
        shift: bool,
        ctrl: bool,
    ) -> bool {
        match self.input.key(m, key, shift, ctrl) {
            Some(changed) => {
                if changed {
                    self.edited(m, c);
                }
                true
            }
            None => false,
        }
    }
    /// `CommandSuggestions.keyPressed`: the list's arrows, Tab and Escape,
    /// and Tab to show it. True when the suggestions took the key.
    pub fn suggestions_key(
        &mut self,
        m: &Measure,
        c: &Commands,
        key: winit::keyboard::KeyCode,
        shift: bool,
    ) -> bool {
        use winit::keyboard::KeyCode as K;
        if let Some(mut l) = self.suggestions.take() {
            let handled = match key {
                K::ArrowUp | K::ArrowDown => {
                    self.cycle(&mut l, if key == K::ArrowUp { -1 } else { 1 });
                    l.tab_cycles = false;
                    true
                }
                K::Tab => {
                    if l.tab_cycles {
                        self.cycle(&mut l, if shift { -1 } else { 1 });
                    }
                    self.use_suggestion(m, c, &mut l);
                    true
                }
                K::Escape => {
                    self.input.suggestion = None;
                    return true;
                }
                _ => false,
            };
            self.suggestions = Some(l);
            handled
        } else if key == K::Tab {
            self.show_suggestions(m, true);
            true
        } else {
            false
        }
    }
    /// `ChatScreen.moveInHistory`.
    fn move_in_history(&mut self, m: &Measure, c: &Commands, history: &[String], dir: isize) {
        let max = history.len();
        let new = (self.history_pos as isize + dir).clamp(0, max as isize) as usize;
        if new == self.history_pos {
            return;
        }
        if new == max {
            self.history_pos = max;
            let b = self.history_buffer.clone();
            self.input.set_value(m, &b);
            self.edited(m, c);
        } else {
            if self.history_pos == max {
                self.history_buffer = self.input.value.clone();
            }
            self.input.set_value(m, &history[new]);
            self.edited(m, c);
            self.allow_suggestions = false;
            self.suggestions = None;
            self.history_pos = new;
        }
    }
    /// A left click (`ChatScreen.mouseClicked`): a suggestion, else the input.
    pub fn click(&mut self, m: &Measure, c: &Commands, x: f64, y: f64) -> bool {
        if let Some(mut l) = self.suggestions.take() {
            let (xi, yi) = (x as i32, y as i32);
            if l.contains(xi, yi) {
                let line = (yi - l.rect[1]) / 12 + l.offset as i32;
                if line >= 0 && (line as usize) < l.list.len() {
                    self.select(&mut l, line as isize);
                    self.use_suggestion(m, c, &mut l);
                }
                self.suggestions = Some(l);
                return true;
            }
            self.suggestions = Some(l);
        }
        if self.input.contains(x, y) {
            self.input.click(m, x, false);
            return true;
        }
        false
    }
    /// The mouse wheel over the suggestion list (`SuggestionsList.mouseScrolled`).
    pub fn scroll(&mut self, mouse: (i32, i32), amount: f64) -> bool {
        let Some(l) = &mut self.suggestions else {
            return false;
        };
        if !l.contains(mouse.0, mouse.1) {
            return false;
        }
        let max = l.list.len().saturating_sub(self.layout.line_limit) as i32;
        l.offset = ((l.offset as f64 - amount.clamp(-1.0, 1.0)) as i32).clamp(0, max) as usize;
        true
    }
    /// `CommandSuggestions.formatText`: argument colours over the input.
    fn format(&self, value: &str, offset: usize) -> Text {
        let gray = Style::color(text::GRAY);
        let Some(parse) = &self.parse else {
            return Text::literal(value);
        };
        let full = &self.input.value;
        let chars = |b: usize| chars_to(full, b) as isize;
        let len = value.chars().count() as isize;
        let piece = |from: isize, to: isize| -> String {
            value
                .chars()
                .skip(from as usize)
                .take((to - from).max(0) as usize)
                .collect()
        };
        let mut out = Text::empty();
        let mut unformatted = 0isize;
        let styles = [
            text::AQUA,
            text::YELLOW,
            text::GREEN,
            text::LIGHT_PURPLE,
            text::GOLD,
        ];
        for (i, arg) in parse.context.last_child().args.iter().enumerate() {
            let start = (chars(arg.range.0) - offset as isize).max(0);
            if start >= len {
                break;
            }
            let end = (chars(arg.range.1) - offset as isize).min(len);
            if end > 0 {
                out.push(Text::styled(piece(unformatted, start), gray.clone()));
                out.push(Text::styled(
                    piece(start, end),
                    Style::color(styles[i % styles.len()]),
                ));
                unformatted = end;
            }
        }
        if parse.can_read() {
            let start = (chars(parse.cursor) - offset as isize).max(0);
            if start < len {
                let remaining = (full.len() - parse.cursor.min(full.len())) as isize;
                let end = (start + full[parse.cursor.min(full.len())..].chars().count() as isize)
                    .min(len);
                let _ = remaining;
                out.push(Text::styled(piece(unformatted, start), gray.clone()));
                out.push(Text::styled(piece(start, end), Style::color(text::RED)));
                unformatted = end;
            }
        }
        out.push(Text::styled(piece(unformatted, len), gray));
        out
    }
    /// `ChatScreen.extractRenderState` without the chat lines: the input's
    /// background (`background`, `Options.getBackgroundColor`), the input,
    /// then the suggestion list or the usage lines.
    pub fn draw(
        &mut self,
        m: &Measure,
        mouse: (i32, i32),
        blink_ms: u64,
        background: i32,
    ) -> Vec<Draw> {
        let (w, h) = (self.width, self.height);
        let mut out = Vec::new();
        if self.layout.anchor_to_bottom {
            d::fill(&mut out, 2, h - 14, w - 2, h - 2, background);
        }
        let format = |s: &str, o: usize| self.format(s, o);
        self.input.draw(&mut out, m, blink_ms, Some(&format));
        if let Some(mut l) = self.suggestions.take() {
            self.draw_list(&mut out, m, &mut l, mouse);
            self.suggestions = Some(l);
        } else {
            for (i, line) in self.usage.iter().enumerate() {
                let y = if self.layout.anchor_to_bottom {
                    h - 27 - 12 * i as i32
                } else {
                    72 + 12 * i as i32
                };
                d::fill(
                    &mut out,
                    self.usage_position - 1,
                    y,
                    self.usage_position + self.usage_width + 1,
                    y + 12,
                    self.layout.fill,
                );
                d::text(&mut out, line.clone(), self.usage_position, y + 2, -1, true);
            }
        }
        out
    }
    fn draw_list(
        &mut self,
        out: &mut Vec<Draw>,
        m: &Measure,
        l: &mut SuggestionList,
        mouse: (i32, i32),
    ) {
        let limit = l.list.len().min(self.layout.line_limit);
        let [rx, ry, rw, rh] = l.rect;
        let has_prev = l.offset > 0;
        let has_next = l.list.len() > l.offset + limit;
        let moved = l.last_mouse != mouse;
        if moved {
            l.last_mouse = mouse;
        }
        if has_prev || has_next {
            d::fill(out, rx, ry - 1, rx + rw, ry, self.layout.fill);
            d::fill(out, rx, ry + rh, rx + rw, ry + rh + 1, self.layout.fill);
            if has_prev {
                for x in (0..rw).step_by(2) {
                    d::fill(out, rx + x, ry - 1, rx + x + 1, ry, -1);
                }
            }
            if has_next {
                for x in (0..rw).step_by(2) {
                    d::fill(out, rx + x, ry + rh, rx + x + 1, ry + rh + 1, -1);
                }
            }
        }
        let mut hovered = false;
        for i in 0..limit {
            let y = ry + 12 * i as i32;
            d::fill(out, rx, y, rx + rw, y + 12, self.layout.fill);
            if mouse.0 > rx && mouse.0 < rx + rw && mouse.1 > y && mouse.1 < y + 12 {
                if moved {
                    self.select(l, (i + l.offset) as isize);
                }
                hovered = true;
            }
            let s = &l.list[i + l.offset];
            let color = if i + l.offset == l.current {
                -256
            } else {
                -5592406
            };
            d::text(
                out,
                Text::literal(s.text.clone()),
                rx + 1,
                ry + 2 + 12 * i as i32,
                color,
                true,
            );
        }
        if hovered && let Some(tip) = &l.list[l.current].tooltip {
            out.push(Draw::Tooltip {
                lines: vec![Text::literal(tip.clone())],
                x: mouse.0,
                y: mouse.1,
            });
        }
        let _ = m;
    }
    /// `ChatScreen.normalizeChatMessage`.
    pub fn normalized(&self) -> String {
        let collapsed = self
            .input
            .value
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        collapsed.chars().take(256).collect()
    }
    pub fn suggestions_visible(&self) -> bool {
        self.suggestions.is_some()
    }
    /// `CommandSuggestions.getUsageNarration` while the list shows
    /// (`isVisible`): whether Tab cycles it or fills the input in, and
    /// whether Escape hides it (`allowHiding`, which only the chat's turns
    /// off).
    pub fn suggestions_usage(&self) -> Option<&'static str> {
        let list = self.suggestions.as_ref()?;
        let hidable = self.layout != SuggestLayout::CHAT;
        Some(match (list.tab_cycles, hidable) {
            (true, true) => "narration.suggestion.usage.cycle.hidable",
            (true, false) => "narration.suggestion.usage.cycle.fixed",
            (false, true) => "narration.suggestion.usage.fill.hidable",
            (false, false) => "narration.suggestion.usage.fill.fixed",
        })
    }
}
