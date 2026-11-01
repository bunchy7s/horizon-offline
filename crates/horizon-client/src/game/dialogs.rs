//! A server's dialogs bound to the game (`ClientCommonPacketListenerImpl.
//! showDialog` and `clearDialog`, `DialogScreen`, its `WarningScreen` and the
//! `WaitingForResponseScreen`): the screen over whatever is open (the
//! connecting screen in configuration), its inputs, and the actions its
//! buttons send back.
use super::Game;
use horizon_client::{
    Result,
    clipboard::EditShortcut,
    confirm_screen::Confirm,
    dialog::{AfterAction, Body, Click, Control, Dialog, InputValue, Kind},
    dialog_screen::{BOX, Layout, Part, WARNING_SPRITES, text_height, waiting_button},
    font::Font,
    hud::{HudBatch, rgba},
    multiline_edit::MultiLineEdit,
    text::{ClickEvent, Text},
    ui,
    widgets::{self, ScreenInfo},
};
use horizon_world::nbt::Tag;
use std::time::Instant;
use winit::keyboard::KeyCode;

/// An open `DialogScreen`.
struct Open {
    dialog: Dialog,
    values: Vec<InputValue>,
    /// The box of each text input.
    fields: Vec<Option<Field>>,
    focus: Option<usize>,
    scroll: ui::Scroll,
    /// The number range input whose slider is held.
    dragging: Option<usize>,
    /// The multiline input the pointer is selecting in.
    selecting: Option<usize>,
}

/// A text input's control: `EditBox`, or `MultiLineEditBox` for a multiline one.
enum Field {
    Line(ui::EditBox),
    Multi(Box<MultiLineEdit>),
}
impl Field {
    fn value(&self) -> String {
        match self {
            Self::Line(edit) => edit.value.clone(),
            Self::Multi(edit) => edit.value(),
        }
    }
    fn insert_text(&mut self, font: &Font, text: &str) {
        match self {
            Self::Line(edit) => edit.insert_text(text),
            Self::Multi(edit) => edit.insert_text(font, text),
        }
    }
    fn key(&mut self, font: &Font, key: KeyCode, ctrl: bool, shift: bool) -> bool {
        match self {
            Self::Line(edit) => edit.key(key, ctrl, shift),
            Self::Multi(edit) => edit.key(font, key, ctrl, shift),
        }
    }
    fn shortcut(
        &mut self,
        font: &Font,
        shortcut: EditShortcut,
        clipboard: &mut dyn horizon_client::clipboard::Clipboard,
    ) {
        match self {
            Self::Line(edit) => {
                edit.shortcut(shortcut, clipboard);
            }
            Self::Multi(edit) => {
                edit.shortcut(font, shortcut, clipboard);
            }
        }
    }
}

/// The dialog screens a server can open.
#[derive(Default)]
pub(super) struct DialogState {
    open: Option<Open>,
    /// `WaitingForResponseScreen`, since when.
    waiting: Option<Instant>,
    /// `DialogScreen.WarningScreen`, over the dialog.
    warning: bool,
}

/// `CycleButton.cycleValue`: the next option (`delta` 1) or the one before
/// it, wrapping.
fn cycle(open: &mut Open, input: usize, delta: i32) {
    let Control::SingleOption { options, .. } = &open.dialog.inputs[input].control else {
        return;
    };
    if let Some(InputValue::Option(i)) = open.values.get_mut(input) {
        *i = (*i as i32 + delta).rem_euclid(options.len().max(1) as i32) as usize;
    }
}

fn warning_texts() -> (Text, Text, [Text; 2]) {
    let t = |key: &str| Text::translate(key, &[]);
    (
        t("menu.custom_screen_info.title"),
        t("menu.custom_screen_info.contents"),
        [t("menu.disconnect"), t("gui.back")],
    )
}

impl Game<'_> {
    /// Whether a dialog screen takes the input.
    pub(super) fn dialog_open(&self) -> bool {
        let d = &self.dialogs;
        d.open.is_some() || d.waiting.is_some() || d.warning
    }
    /// Whether the dialog's focus is one of its text inputs, which takes the
    /// keys.
    pub(super) fn dialog_typing(&self) -> bool {
        let d = &self.dialogs;
        !d.warning
            && d.waiting.is_none()
            && d.open.as_ref().is_some_and(|o| {
                o.focus
                    .and_then(|f| o.fields.get(f))
                    .is_some_and(Option::is_some)
            })
    }

    /// `showDialog`: the dialog replaces an open one (and a waiting screen);
    /// under the warning screen, it is what Back returns to.
    pub(super) fn show_dialog(&mut self, tag: &Tag) {
        let Some(mut dialog) = Dialog::parse(tag) else {
            log::warn!("Horizon: failed to show dialog for data {tag:?}");
            return;
        };
        if let Kind::DialogList { dialogs, .. } = &dialog.kind {
            let entries = self.remote.as_ref().map(|r| {
                let list = r.registries.registries.get("minecraft:dialog");
                let by_id = |id: &str| {
                    let name = if id.contains(':') {
                        id.to_owned()
                    } else {
                        format!("minecraft:{id}")
                    };
                    list?.iter().find(|(n, _)| *n == name)?.1.clone()
                };
                let by_tag = |name: &str| {
                    let name = if name.contains(':') {
                        name.to_owned()
                    } else {
                        format!("minecraft:{name}")
                    };
                    let ids = r.registries.dialog_tags.get(&name).into_iter().flatten();
                    ids.filter_map(|&i| list?.get(usize::try_from(i).ok()?)?.1.clone())
                        .collect()
                };
                Dialog::list_entries(dialogs, &by_tag, &by_id)
            });
            dialog.set_list(&entries.unwrap_or_default());
        }
        self.open_dialog(dialog);
    }

    /// The parsed dialog's screen, its inputs at their initial values.
    fn open_dialog(&mut self, dialog: Dialog) {
        let values = dialog.inputs.iter().map(|i| i.control.initial()).collect();
        let font = &self.gui.atlas.font;
        let fields = dialog
            .inputs
            .iter()
            .map(|i| match &i.control {
                Control::Text {
                    width,
                    initial,
                    max_length,
                    multiline: Some((max_lines, _)),
                    ..
                } => {
                    // TextInputHandler: the `MultiLineEditBox` with its limits.
                    let height = text_height(Some((*max_lines, None)));
                    let height = match &i.control {
                        Control::Text {
                            multiline: Some((_, Some(h))),
                            ..
                        } => *h as f32,
                        _ => height,
                    };
                    let mut edit = MultiLineEdit::new(font, 0.0, 0.0, *width as f32, height);
                    edit.set_character_limit((*max_length).max(0) as usize);
                    if let Some(lines) = max_lines {
                        edit.set_line_limit((*lines).max(0) as usize);
                    }
                    edit.set_value(font, initial);
                    Some(Field::Multi(Box::new(edit)))
                }
                Control::Text {
                    width,
                    initial,
                    max_length,
                    ..
                } => {
                    let mut edit = ui::EditBox::new(0.0, 0.0, *width as f32, 20.0);
                    edit.max_length = (*max_length).max(1) as usize;
                    edit.set_value(initial);
                    edit.focused = false;
                    Some(Field::Line(edit))
                }
                _ => None,
            })
            .collect();
        self.dialogs.open = Some(Open {
            dialog,
            values,
            fields,
            focus: None,
            scroll: ui::Scroll::default(),
            dragging: None,
            selecting: None,
        });
        self.dialogs.waiting = None;
        self.keys.clear();
        self.physical_keys.clear();
        self.left = false;
        self.right = false;
        self.capture(false);
    }

    /// `clearDialog`: the dialog closes (under the warning screen, Back then
    /// returns to what was open before it).
    pub(super) fn clear_dialog(&mut self) {
        if self.dialogs.open.take().is_some() && !self.dialog_open() {
            self.dialog_closed();
        }
    }

    /// Every dialog screen goes (a disconnect, or the level loading screen).
    pub(super) fn close_dialogs(&mut self) {
        self.dialogs = DialogState::default();
    }

    fn dialog_closed(&mut self) {
        if self.player.is_some() && self.screen == super::Screen::Playing && self.title.is_none() {
            self.capture(true);
        }
    }

    /// The input values as the controls hold them now.
    fn dialog_values(open: &mut Open) {
        for (value, field) in open.values.iter_mut().zip(&open.fields) {
            if let Some(f) = field {
                *value = InputValue::Text(f.value());
            }
        }
    }

    /// `DialogScreen.runAction`: the click event, then the screen the dialog's
    /// `after_action` leaves.
    fn run_dialog_action(&mut self, click: Option<Click>, after: AfterAction) -> Result<()> {
        // defaultHandleClickEvent: an open_url never sets the screen itself, so
        // the dialog stays unless the confirmation takes over (Web Links off,
        // or Prompt on Links off, opens nothing or the browser behind it).
        if let Some(Click::OpenUrl(url)) = &click
            && !(self.settings.chat_links && self.settings.chat_links_prompt)
        {
            if self.settings.chat_links {
                super::packs::open_uri(url);
            }
            return Ok(());
        }
        match after {
            AfterAction::None => {}
            AfterAction::Close => self.dialogs.open = None,
            AfterAction::WaitForResponse => {
                self.dialogs.open = None;
                self.dialogs.waiting = Some(Instant::now());
            }
        }
        match click {
            Some(Click::Custom { id, payload }) => {
                if let Some(r) = &self.remote {
                    r.session.send_custom_click(&id, payload);
                }
            }
            Some(Click::RunCommand(command)) => {
                // In configuration there is nothing to run it (vanilla warns).
                if self.player.is_some() {
                    self.send_unattended_command(&command);
                } else {
                    log::warn!(
                        "Horizon: commands are not supported in configuration, trying to run '{command}'"
                    );
                }
            }
            Some(Click::ShowDialog(dialog)) => self.show_dialog_ref(&dialog),
            Some(Click::CopyToClipboard(value)) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&value);
            }
            // ConfirmLinkScreen over the screen the dialog leaves.
            Some(Click::OpenUrl(url)) => self.click_url(url),
            // defaultHandleClickEvent: `activeScreen.insertText(command, true)`.
            // Only the chat screen (opened under the dialog) takes text.
            Some(Click::SuggestCommand(command)) => {
                if !self.dialog_open() && self.ui.chat_screen.is_some() {
                    let m = horizon_client::screen_draw::Measure(&self.gui.atlas.font);
                    let c = horizon_client::chat::Commands {
                        dispatcher: super::chat::chat_tree(&self.remote),
                        registries: &self.ui.registries,
                    };
                    if let Some(screen) = self.ui.chat_screen.as_mut() {
                        screen.suggest_command(&m, &c, &command);
                    }
                }
            }
            Some(other) => log::error!("Don't know how to handle {other:?}"),
            None => {}
        }
        if !self.dialog_open() {
            self.dialog_closed();
        }
        Ok(())
    }

    /// A `show_dialog` click event's dialog: a registry id or the dialog itself.
    pub(super) fn show_dialog_ref(&mut self, dialog: &Tag) {
        match dialog {
            Tag::String(id) => {
                let id = id.to_string_lossy();
                let name = if id.contains(':') {
                    id.clone()
                } else {
                    format!("minecraft:{id}")
                };
                let found = self.remote.as_ref().and_then(|r| {
                    let entries = r.registries.registries.get("minecraft:dialog")?;
                    entries.iter().find(|(n, _)| *n == name)?.1.clone()
                });
                match found {
                    Some(tag) => self.show_dialog(&tag),
                    None => log::warn!("Horizon: unknown dialog {id}"),
                }
            }
            tag => self.show_dialog(tag),
        }
    }

    /// `ClientboundShowDialogPacket` in play: a registry entry or the dialog.
    pub(super) fn show_play_dialog(
        &mut self,
        dialog: &horizon_protocol::packets::play::DialogHolder,
    ) {
        use horizon_protocol::packets::play::DialogHolder;
        match dialog {
            DialogHolder::Direct(tag) => self.show_dialog(tag),
            DialogHolder::Registry(id) => {
                let tag = self
                    .remote
                    .as_ref()
                    .and_then(|r| r.registries.data("minecraft:dialog", *id).cloned());
                match tag {
                    Some(tag) => self.show_dialog(&tag),
                    None => log::warn!("Horizon: unknown dialog {id}"),
                }
            }
        }
    }

    fn dialog_layout(&self) -> Option<Layout> {
        let open = self.dialogs.open.as_ref()?;
        Some(Layout::new(
            &open.dialog,
            &self.gui.atlas.font,
            self.gui_size(),
        ))
    }

    pub(super) fn draw_dialog(&mut self, batch: &mut HudBatch) {
        if !self.dialog_open() {
            return;
        }
        let (w, h) = (batch.width, batch.height);
        let cursor = self.cursor;
        let atlas = &self.gui.atlas;
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        if self.dialogs.warning {
            let (title, message, [disconnect, back]) = warning_texts();
            // DialogScreen.WarningScreen, a ConfirmScreen: its title and its
            // message.
            widgets::begin_screen(|| {
                ScreenInfo::new(
                    "dialog/warning",
                    widgets::join_for_narration(&[&title.plain(), &message.plain()]),
                )
            });
            Confirm::new(&atlas.font, (w, h), &title, &message).draw(
                batch,
                atlas,
                cursor,
                [&disconnect, &back],
            );
            return;
        }
        if let Some(since) = self.dialogs.waiting {
            // HeaderAndFooterLayout(33, 0) centred: the title, the button 33 below.
            let ticks = (since.elapsed().as_millis() / 50) as u32;
            let (label, visible, active) = waiting_button(ticks);
            let title = Text::translate("gui.waitingForResponse.title", &[]);
            widgets::begin_screen(|| ScreenInfo::new("dialog/waiting", title.plain()));
            // WaitingForResponseScreen.tick: the close button turning active
            // after five seconds is narrated at once.
            if active && ticks < 102 {
                widgets::narrate_now();
            }
            let top = ((h - 53.0) / 2.0).trunc();
            let tw = atlas.font.width_text(&title);
            batch.text_styled(
                atlas,
                &title,
                ((w - tw) / 2.0).trunc(),
                top + 12.0,
                u32::MAX,
                true,
            );
            if visible {
                let (bx, by) = (((w - 200.0) / 2.0).trunc(), top + 33.0);
                let hover = ui::hovered(cursor, bx, by, 200.0, 20.0);
                ui::button(batch, atlas, bx, by, 200.0, 20.0, &label, active, hover);
            }
            return;
        }
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        // Out of `self` while it draws: an item body draws through `self`.
        let Some(mut open) = self.dialogs.open.take() else {
            return;
        };
        let atlas = &self.gui.atlas;
        let [ax, ay, aw, ah] = layout.area;
        open.scroll.clamp(ay, ay + ah, layout.content);
        // DialogScreen: titled by its dialog; a new dialog is a new screen.
        widgets::begin_screen(|| {
            let title = open.dialog.title.plain();
            ScreenInfo::new(format!("dialog:{title}"), title)
        });
        batch.text_styled(
            atlas,
            &open.dialog.title,
            layout.title.0,
            layout.title.1,
            u32::MAX,
            true,
        );
        let [wx, wy, ww, wh] = layout.warning;
        let warn_hover = ui::hovered(cursor, wx, wy, ww, wh);
        horizon_client::widgets::record("warning", layout.warning, true);
        // The warning ImageButton: its narration message and tooltip, first
        // in tab order (group -10).
        widgets::narratable(|| {
            let t = |key: &str| Text::translate(key, &[]).plain();
            widgets::Narratable::button(
                t("menu.custom_screen_info.button_narration"),
                layout.warning,
                true,
            )
            .hovered(warn_hover)
            .hint(t("menu.custom_screen_info.tooltip"))
            .order(-10)
        });
        batch.sprite_scaled(
            atlas,
            WARNING_SPRITES[if warn_hover { 2 } else { 0 }],
            wx,
            wy,
            ww,
            wh,
            u32::MAX,
        );
        let buttons = open.dialog.buttons();
        let in_area = |y: f32| y >= ay && y < ay + ah;
        let mut tooltip = None;
        let mut item_tip = None;
        let mut start = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && start.is_none() {
                start = Some(batch.vertices.len());
            } else if !clipped && let Some(s) = start.take() {
                batch.clip_since(s, ax, ay, ax + aw, ay + ah);
            }
            match part {
                Part::Line(text, x, y) => {
                    batch.text_styled(atlas, &text, x, y, u32::MAX, true);
                }
                Part::Field { input, rect } => {
                    let focused = open.focus == Some(input);
                    // The edit box is named after its input's label.
                    let label = match &open.dialog.inputs[input].control {
                        Control::Text { label, .. } if widgets::narrating() => label.plain(),
                        _ => String::new(),
                    };
                    match open.fields.get_mut(input) {
                        Some(Some(Field::Line(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = rect;
                            edit.focused = focused;
                            if widgets::narrating() {
                                edit.message = label;
                            }
                            edit.draw(batch, atlas, None);
                        }
                        Some(Some(Field::Multi(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = rect;
                            edit.set_focused(focused);
                            edit.draw(batch, atlas);
                            // MultiLineEditBox.updateWidgetNarration: as an EditBox.
                            widgets::narratable(|| {
                                widgets::Narratable::edit_box(label, edit.value(), rect)
                                    .focused(focused)
                            });
                        }
                        _ => {}
                    }
                }
                Part::Check { input, rect } => {
                    let [x, y, cw, ch] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, cw, ch);
                    let on = open.values.get(input) == Some(&InputValue::Bool(true));
                    let sprite = match (on, hover) {
                        (true, true) => "gui/sprites/widget/checkbox_selected_highlighted",
                        (true, false) => "gui/sprites/widget/checkbox_selected",
                        (false, true) => "gui/sprites/widget/checkbox_highlighted",
                        (false, false) => "gui/sprites/widget/checkbox",
                    };
                    let Control::Boolean { label, .. } = &open.dialog.inputs[input].control else {
                        continue;
                    };
                    horizon_client::widgets::record(&label.plain(), rect, true);
                    widgets::narratable(|| {
                        widgets::Narratable::checkbox(label.plain(), on, rect, true).hovered(hover)
                    });
                    batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
                    batch.text_styled(
                        atlas,
                        label,
                        x + BOX + 4.0,
                        y + 4.0,
                        rgba(224, 224, 224, 255),
                        true,
                    );
                }
                Part::Cycle { input, rect } => {
                    let [x, y, cw, ch] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, cw, ch);
                    let Some(label) = open
                        .values
                        .get(input)
                        .and_then(|v| open.dialog.inputs[input].control.message(v))
                    else {
                        continue;
                    };
                    ui::button(batch, atlas, x, y, cw, ch, &label, true, hover);
                    // A CycleButton: the option a click moves to.
                    widgets::last_narratable(|n| {
                        let control = &open.dialog.inputs[input].control;
                        let next = match (control, open.values.get(input)) {
                            (
                                Control::SingleOption { options, .. },
                                Some(InputValue::Option(i)),
                            ) if !options.is_empty() => {
                                let (id, display, _) = &options[(i + 1) % options.len()];
                                display.clone().unwrap_or_else(|| Text::literal(id)).plain()
                            }
                            _ => String::new(),
                        };
                        n.what = widgets::What::Cycle {
                            message: label.plain(),
                            next,
                            active: true,
                        };
                    });
                }
                Part::Slider { input, rect } => {
                    let [x, y, sw, sh] = rect;
                    let hover = in_area(cursor.1) && ui::hovered(cursor, x, y, sw, sh);
                    let Some((InputValue::Slider(position), Some(label))) = open
                        .values
                        .get(input)
                        .map(|v| (v.clone(), open.dialog.inputs[input].control.message(v)))
                    else {
                        continue;
                    };
                    let held = open.dragging == Some(input);
                    ui::slider(
                        batch,
                        atlas,
                        x,
                        y,
                        sw,
                        &label,
                        position,
                        hover || held,
                        true,
                        held,
                    );
                }
                Part::Item { body, rect } => {
                    let [x, y, iw, ih] = rect;
                    let Some(Body::Item {
                        item,
                        show_decorations,
                        show_tooltip,
                        ..
                    }) = open.dialog.body.get(body)
                    else {
                        continue;
                    };
                    let Some(stack) = horizon_items::item_nbt::item_from_nbt(item) else {
                        continue;
                    };
                    // ItemDisplayWidget: the item at its corner, then its decorations.
                    self.draw_item_model(batch, &stack, x, y);
                    if *show_decorations {
                        self.draw_item_decorations(batch, &stack, x, y, None);
                    }
                    if *show_tooltip && in_area(cursor.1) && ui::hovered(cursor, x, y, iw, ih) {
                        item_tip = Some(stack);
                    }
                }
                Part::Button { index, rect } => {
                    let [x, y, bw, bh] = rect;
                    let hover =
                        (!clipped || in_area(cursor.1)) && ui::hovered(cursor, x, y, bw, bh);
                    let b = buttons[index];
                    ui::button(batch, atlas, x, y, bw, bh, &b.label, true, hover);
                    if let Some(t) = &b.tooltip {
                        widgets::last_narratable(|n| n.hint = Some(t.plain()));
                    }
                    if hover && let Some(t) = &b.tooltip {
                        tooltip = Some(t.clone());
                    }
                }
            }
        }
        if let Some(s) = start {
            batch.clip_since(s, ax, ay, ax + aw, ay + ah);
        }
        if layout.content > ah {
            ui::scrollbar(
                batch,
                atlas,
                ax + aw - 6.0,
                ay,
                ay + ah,
                layout.content,
                open.scroll.amount,
            );
        }
        if warn_hover {
            tooltip = Some(Text::translate("menu.custom_screen_info.tooltip", &[]));
        }
        if let Some(t) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&t, 170.0)
                .into_iter()
                .map(|l| l.0)
                .collect();
            ui::tooltip(batch, atlas, &lines, cursor);
        }
        self.dialogs.open = Some(open);
        if let Some(stack) = item_tip {
            self.draw_stack_tooltip(batch, &stack, cursor.0, cursor.1);
        }
    }

    /// A left click on a dialog screen.
    pub(super) fn dialog_click(&mut self) -> Result<()> {
        let cursor = self.cursor;
        if self.dialogs.warning {
            let (title, message, _) = warning_texts();
            let answer = Confirm::new(&self.gui.atlas.font, self.gui_size(), &title, &message)
                .answer_at(cursor);
            match answer {
                Some(true) => {
                    self.audio.play_button_click();
                    let reason = Text::translate("menu.custom_screen_info.disconnect", &[]);
                    return self.leave_server(Some(reason.plain()));
                }
                Some(false) => {
                    self.audio.play_button_click();
                    self.dialogs.warning = false;
                    if !self.dialog_open() {
                        self.dialog_closed();
                    }
                }
                None => {}
            }
            return Ok(());
        }
        if let Some(since) = self.dialogs.waiting {
            let (w, h) = self.gui_size();
            let active = waiting_button((since.elapsed().as_millis() / 50) as u32).2;
            let (bx, by) = (
                ((w - 200.0) / 2.0).trunc(),
                ((h - 53.0) / 2.0).trunc() + 33.0,
            );
            if active && ui::hovered(cursor, bx, by, 200.0, 20.0) {
                self.audio.play_button_click();
                self.dialogs.waiting = None;
                self.dialog_closed();
            }
            return Ok(());
        }
        let Some(layout) = self.dialog_layout() else {
            return Ok(());
        };
        let [wx, wy, ww, wh] = layout.warning;
        if ui::hovered(cursor, wx, wy, ww, wh) {
            self.audio.play_button_click();
            self.dialogs.warning = true;
            return Ok(());
        }
        let shift = self.modifiers.shift_key();
        let Some(open) = self.dialogs.open.as_mut() else {
            return Ok(());
        };
        let [_, ay, _, ah] = layout.area;
        let mut action = None;
        let mut text_click: Option<ClickEvent> = None;
        open.focus = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && !(cursor.1 >= ay && cursor.1 < ay + ah) {
                continue;
            }
            match part {
                Part::Field {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(
                    cursor,
                    x,
                    y,
                    // A multiline box answers over its scroll bar too.
                    w + if matches!(open.fields.get(input), Some(Some(Field::Multi(_)))) {
                        6.0
                    } else {
                        0.0
                    },
                    h,
                ) =>
                {
                    open.focus = Some(input);
                    match open.fields.get_mut(input) {
                        Some(Some(Field::Line(edit))) => {
                            edit.focused = true;
                            edit.click(&self.gui.atlas, cursor.0, false, false);
                        }
                        Some(Some(Field::Multi(edit))) => {
                            [edit.x, edit.y, edit.width, edit.height] = [x, y, w, h];
                            edit.set_focused(true);
                            edit.click(&self.gui.atlas.font, cursor, shift, false);
                            open.selecting = Some(input);
                        }
                        _ => {}
                    }
                }
                Part::Check {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    if let Some(InputValue::Bool(b)) = open.values.get_mut(input) {
                        *b = !*b;
                    }
                    action = Some(None);
                }
                Part::Cycle {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    // CycleButton.onPress: Shift goes back.
                    cycle(open, input, if shift { -1 } else { 1 });
                    action = Some(None);
                }
                Part::Slider {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    open.focus = Some(input);
                    open.dragging = Some(input);
                    if let Some(v) = open.values.get_mut(input) {
                        *v = InputValue::Slider(ui::slider_value(x, w, cursor.0));
                    }
                }
                Part::Button {
                    index,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    action = Some(Some(index));
                }
                // AbstractStringWidget.setComponentClickHandler: the click event
                // of the styled text under a centred message line.
                Part::Line(line, x, y)
                    if text_click.is_none()
                        && cursor.0 >= x
                        && (0.0..9.0).contains(&(cursor.1 - y)) =>
                {
                    let content = line.resolve_legacy();
                    let mut left = cursor.0 - x;
                    for (c, style) in content.chars() {
                        left -= self.gui.atlas.font.advance(c, style);
                        if left < 0.0 {
                            text_click = style.click.clone();
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(action) = action else {
            // A text click runs its event (DialogScreen.runAction) with no button sound.
            let click = match text_click {
                Some(ClickEvent::OpenUrl(url)) => Click::OpenUrl(url),
                Some(ClickEvent::ChangePage(page)) => Click::ChangePage(page),
                Some(ClickEvent::SuggestCommand(command)) => Click::SuggestCommand(command),
                Some(ClickEvent::RunCommand(command)) => Click::RunCommand(command),
                Some(ClickEvent::CopyToClipboard(value)) => Click::CopyToClipboard(value),
                Some(ClickEvent::ShowDialog(dialog)) => Click::ShowDialog(dialog),
                Some(ClickEvent::OpenFile(_)) | None => return Ok(()),
            };
            let after = open.dialog.after_action;
            return self.run_dialog_action(Some(click), after);
        };
        self.audio.play_button_click();
        let Some(index) = action else {
            return Ok(());
        };
        let Some(open) = self.dialogs.open.as_mut() else {
            return Ok(());
        };
        Self::dialog_values(open);
        let d = &open.dialog;
        let click = d.buttons()[index]
            .action
            .as_ref()
            .and_then(|a| a.click(&d.inputs, &open.values));
        let after = d.after_action;
        self.run_dialog_action(click, after)
    }

    /// The mouse wheel over a dialog: a cycle button under it takes the
    /// step, else the body scrolls 10 a step.
    pub(super) fn dialog_scroll(&mut self, delta: f32) {
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        let cursor = self.cursor;
        if let Some(open) = self.dialogs.open.as_mut() {
            let [_, ay, _, ah] = layout.area;
            if delta != 0.0 && cursor.1 >= ay && cursor.1 < ay + ah {
                let hovered_cycle =
                    layout
                        .placed(open.scroll.amount)
                        .find_map(|(part, _)| match part {
                            Part::Cycle {
                                input,
                                rect: [x, y, w, h],
                            } if ui::hovered(cursor, x, y, w, h) => Some(input),
                            _ => None,
                        });
                if let Some(input) = hovered_cycle {
                    // CycleButton.mouseScrolled: up goes back.
                    cycle(open, input, if delta > 0.0 { -1 } else { 1 });
                    return;
                }
                // AbstractScrollArea.mouseScrolled: a multiline box takes the wheel.
                let text = layout
                    .placed(open.scroll.amount)
                    .find_map(|(part, _)| match part {
                        Part::Field { input, rect } => match open.fields.get(input) {
                            Some(Some(Field::Multi(_)))
                                if ui::hovered(
                                    cursor,
                                    rect[0],
                                    rect[1],
                                    rect[2] + 6.0,
                                    rect[3],
                                ) =>
                            {
                                Some(input)
                            }
                            _ => None,
                        },
                        _ => None,
                    });
                if let Some(Some(Field::Multi(edit))) = text.and_then(|i| open.fields.get_mut(i)) {
                    edit.wheel(delta);
                    return;
                }
            }
            open.scroll.wheel(delta, 10.0, ay, ay + ah, layout.content);
        }
    }

    /// The pointer moved with a slider held (`AbstractSliderButton.onDrag`).
    pub(super) fn dialog_drag(&mut self) {
        let (cursor, shift) = (self.cursor, self.modifiers.shift_key());
        if let Some(open) = self.dialogs.open.as_mut()
            && let Some(Some(Field::Multi(edit))) =
                open.selecting.and_then(|i| open.fields.get_mut(i))
        {
            // MultiLineEditBox.onDrag.
            edit.drag(&self.gui.atlas.font, cursor, shift);
        }
        let Some(input) = self.dialogs.open.as_ref().and_then(|o| o.dragging) else {
            return;
        };
        let Some(layout) = self.dialog_layout() else {
            return;
        };
        let x = self.cursor.0;
        let Some(open) = self.dialogs.open.as_mut() else {
            return;
        };
        let slider = layout.body.iter().find_map(|p| match p {
            Part::Slider { input: i, rect } if *i == input => Some(*rect),
            _ => None,
        });
        if let (Some([sx, _, sw, _]), Some(value)) = (slider, open.values.get_mut(input)) {
            *value = InputValue::Slider(ui::slider_value(sx, sw, x));
        }
    }

    /// The left button released: a held slider lets go and clicks.
    pub(super) fn dialog_release(&mut self) {
        if let Some(open) = self.dialogs.open.as_mut() {
            if let Some(Some(Field::Multi(edit))) =
                open.selecting.take().and_then(|i| open.fields.get_mut(i))
            {
                edit.release();
            }
            if open.dragging.take().is_some() {
                self.audio.play_button_click();
            }
        }
    }

    /// A key pressed on a dialog screen: Escape closes what allows it, Tab
    /// moves between the text inputs, the rest goes to the focused one
    /// (`shortcut`: the press as an edit shortcut; `text`: what it types).
    pub(super) fn dialog_key(
        &mut self,
        key: KeyCode,
        shortcut: Option<EditShortcut>,
        text: Option<&str>,
    ) -> Result<()> {
        let escape = key == KeyCode::Escape;
        if self.dialogs.warning {
            if escape {
                self.dialogs.warning = false;
                if !self.dialog_open() {
                    self.dialog_closed();
                }
            }
            return Ok(());
        }
        if let Some(since) = self.dialogs.waiting {
            // shouldCloseOnEsc: once Back works.
            if escape && waiting_button((since.elapsed().as_millis() / 50) as u32).2 {
                self.dialogs.waiting = None;
                self.dialog_closed();
            }
            return Ok(());
        }
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        let shift = self.modifiers.shift_key();
        let Some(open) = self.dialogs.open.as_mut() else {
            return Ok(());
        };
        if escape {
            if !open.dialog.can_close_with_escape {
                return Ok(());
            }
            // onClose: the cancel action, and the screen closes.
            Self::dialog_values(open);
            let d = &open.dialog;
            let click = d.on_cancel().and_then(|a| a.click(&d.inputs, &open.values));
            return self.run_dialog_action(click, AfterAction::Close);
        }
        if key == KeyCode::Tab {
            let texts: Vec<usize> = (0..open.fields.len())
                .filter(|&i| open.fields[i].is_some())
                .collect();
            if !texts.is_empty() {
                let at = open.focus.and_then(|f| texts.iter().position(|&i| i == f));
                let next = match at {
                    None if shift => texts.len() - 1,
                    None => 0,
                    Some(p) if shift => (p + texts.len() - 1) % texts.len(),
                    Some(p) => (p + 1) % texts.len(),
                };
                open.focus = Some(texts[next]);
            }
            return Ok(());
        }
        if let Some(focus) = open.focus
            && let Some(input) = open.dialog.inputs.get(focus)
            && let Control::NumberRange {
                start,
                end,
                step,
                width,
                ..
            } = &input.control
            && let Some(InputValue::Slider(val)) = open.values.get_mut(focus)
            && matches!(key, KeyCode::ArrowLeft | KeyCode::ArrowRight)
        {
            let delta = if let Some(step) = step {
                let range = (end - start).abs();
                if range > 0.0 { *step / range } else { 0.01 }
            } else {
                1.0 / (*width as f32 - 8.0).max(1.0)
            };
            let step_sign = if key == KeyCode::ArrowRight {
                delta
            } else {
                -delta
            };
            *val = (*val + step_sign).clamp(0.0, 1.0);
            return Ok(());
        }
        let Some(Some(edit)) = open.focus.and_then(|f| open.fields.get_mut(f)) else {
            return Ok(());
        };
        let font = &self.gui.atlas.font;
        if let Some(shortcut) = shortcut {
            edit.shortcut(font, shortcut, &mut self.clipboard);
            return Ok(());
        }
        if edit.key(font, key, ctrl, shift) {
            return Ok(());
        }
        let typed: String = text
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if !typed.is_empty() {
            edit.insert_text(font, &typed);
        }
        Ok(())
    }

    /// Whether the open dialog has a text input.
    pub(super) fn dialog_has_text_field(&self) -> bool {
        self.dialogs
            .open
            .as_ref()
            .is_some_and(|open| open.fields.iter().any(Option::is_some))
    }

    /// `type TEXT` with a dialog open: into the focused text input, or the
    /// first one.
    pub(super) fn dialog_type(&mut self, text: &str) -> bool {
        let Some(open) = self.dialogs.open.as_mut() else {
            return false;
        };
        let focus = open
            .focus
            .or_else(|| open.fields.iter().position(Option::is_some));
        let Some(Some(edit)) = focus.and_then(|f| open.fields.get_mut(f)) else {
            return false;
        };
        open.focus = focus;
        edit.insert_text(&self.gui.atlas.font, text);
        true
    }

    /// What `state` reports: the open dialog and its values, or the waiting
    /// and warning screens.
    pub(super) fn dialog_status(&self) -> serde_json::Value {
        let d = &self.dialogs;
        let screen = if d.warning {
            Some("warning")
        } else if d.waiting.is_some() {
            Some("waiting_for_response")
        } else {
            None
        };
        let open = d.open.as_ref().map(|open| {
            let mut values = open.values.clone();
            for (value, field) in values.iter_mut().zip(&open.fields) {
                if let Some(f) = field {
                    *value = InputValue::Text(f.value());
                }
            }
            open.dialog.json(&values)
        });
        serde_json::json!({ "open": open, "screen": screen })
    }
}
