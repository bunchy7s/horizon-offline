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
    dialog_screen::{BOX, Layout, Part, WARNING_SPRITES, waiting_button},
    hud::{HudBatch, rgba},
    text::Text,
    ui,
};
use horizon_world::nbt::Tag;
use std::time::Instant;
use winit::keyboard::KeyCode;

/// An open `DialogScreen`.
struct Open {
    dialog: Dialog,
    values: Vec<InputValue>,
    /// The box of each text input.
    fields: Vec<Option<ui::EditBox>>,
    focus: Option<usize>,
    scroll: ui::Scroll,
    /// The number range input whose slider is held.
    dragging: Option<usize>,
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
        let values = dialog.inputs.iter().map(|i| i.control.initial()).collect();
        let fields = dialog
            .inputs
            .iter()
            .map(|i| match &i.control {
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
                    Some(edit)
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
        });
        self.dialogs.waiting = None;
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
                *value = InputValue::Text(f.value.clone());
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
            && !(self.settings.chat_links && self.settings.file.bool("chatLinksPrompt"))
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
                    let command = command.strip_prefix('/').unwrap_or(&command);
                    self.send_chat(&format!("/{command}"));
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
            // Screen.insertText: the screen after the dialog takes no text.
            Some(Click::SuggestCommand(_)) => {}
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
                    if let Some(Some(edit)) = open.fields.get_mut(input) {
                        [edit.x, edit.y, edit.width, edit.height] = rect;
                        edit.focused = open.focus == Some(input);
                        edit.draw(batch, atlas, None);
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
        open.focus = None;
        for (part, clipped) in layout.placed(open.scroll.amount) {
            if clipped && !(cursor.1 >= ay && cursor.1 < ay + ah) {
                continue;
            }
            match part {
                Part::Field {
                    input,
                    rect: [x, y, w, h],
                } if ui::hovered(cursor, x, y, w, h) => {
                    open.focus = Some(input);
                    if let Some(Some(edit)) = open.fields.get_mut(input) {
                        edit.focused = true;
                        edit.click(&self.gui.atlas, cursor.0, false, false);
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
                _ => {}
            }
        }
        let Some(action) = action else {
            return Ok(());
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
                let hovered = layout
                    .placed(open.scroll.amount)
                    .find_map(|(part, _)| match part {
                        Part::Cycle {
                            input,
                            rect: [x, y, w, h],
                        } if ui::hovered(cursor, x, y, w, h) => Some(input),
                        _ => None,
                    });
                if let Some(input) = hovered {
                    // CycleButton.mouseScrolled: up goes back.
                    cycle(open, input, if delta > 0.0 { -1 } else { 1 });
                    return;
                }
            }
            open.scroll.wheel(delta, 10.0, ay, ay + ah, layout.content);
        }
    }

    /// The pointer moved with a slider held (`AbstractSliderButton.onDrag`).
    pub(super) fn dialog_drag(&mut self) {
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
        if let Some(open) = self.dialogs.open.as_mut()
            && open.dragging.take().is_some()
        {
            self.audio.play_button_click();
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
        let Some(Some(edit)) = open.focus.and_then(|f| open.fields.get_mut(f)) else {
            return Ok(());
        };
        if let Some(shortcut) = shortcut {
            edit.shortcut(shortcut, &mut self.clipboard);
            return Ok(());
        }
        if edit.key(key, ctrl, shift) {
            return Ok(());
        }
        let typed: String = text
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if !typed.is_empty() {
            edit.insert_text(&typed);
        }
        Ok(())
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
        edit.insert_text(text);
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
                    *value = InputValue::Text(f.value.clone());
                }
            }
            open.dialog.json(&values)
        });
        serde_json::json!({ "open": open, "screen": screen })
    }
}
