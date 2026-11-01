//! The edit screens of the game master blocks (`StructureBlockEditScreen`,
//! `JigsawBlockEditScreen`, `TestBlockEditScreen`,
//! `TestInstanceBlockEditScreen`): one screen that runs an [`Editor`], which
//! lays out its widgets, takes their presses and builds the packet for the
//! server. The screens change the client's copy of the block entity as they
//! are used, as vanilla's do, so the box preview follows them.
use super::{Game, Screen};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    text::Text,
    ui::{self, EditBox},
};
use horizon_protocol::packets::play::Serverbound as S;
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

pub(super) type Pos = (i32, i32, i32);

/// `Screen.extractRenderState`'s grey for the labels: `-6250336`.
pub(super) const LABEL: u32 = 0xFFA0_A0A0;

/// A widget of an edit screen at GUI pixels.
pub(super) enum Item {
    Label {
        text: Text,
        x: f32,
        y: f32,
        /// Right-aligned at `x` (the labels beside the right-hand buttons).
        right: bool,
    },
    Title {
        text: Text,
        y: f32,
    },
    Edit {
        index: usize,
        x: f32,
        y: f32,
        w: f32,
    },
    Button {
        id: u8,
        x: f32,
        y: f32,
        w: f32,
        label: Text,
        active: bool,
    },
    /// A multi-line message (`FittingMultiLineTextWidget`), wrapped to `w`
    /// and cut to what fits `h`.
    Info {
        text: Text,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    /// An `AbstractSliderButton`: `value` is 0..1.
    Slider {
        id: u8,
        x: f32,
        y: f32,
        w: f32,
        label: Text,
        value: f32,
    },
}

/// What a press on a button does to the screen.
pub(super) enum Outcome {
    Stay,
    /// Send the packet and close.
    Send(S),
    /// Send the packets in order and close.
    SendAll(Vec<S>),
    /// Close, after the editor reverted what it changed.
    Cancel,
}

/// One edit screen's widgets and rules.
pub(super) trait Editor {
    /// The text boxes, in tab order.
    fn edits(&mut self) -> &mut [EditBox];
    /// The widgets for the block entity as it is now, in a `w`×`h` screen.
    fn items(&self, entity: &BlockEntity, w: f32, h: f32) -> Vec<Item>;
    /// A button pressed (`shift` held), possibly changing the entity.
    fn press(
        &mut self,
        entity: &mut BlockEntity,
        pos: Pos,
        id: u8,
        shift: bool,
        alt: bool,
    ) -> Outcome;
    /// The screen opened: packets it sends first (`init`).
    fn opened(&mut self, _entity: &BlockEntity, _pos: Pos) -> Vec<S> {
        Vec::new()
    }
    /// The text of box `edit` changed (`EditBox.setResponder`): packets to send.
    fn edited(&mut self, _edit: usize, _pos: Pos) -> Vec<S> {
        Vec::new()
    }
    /// A status from the server (`ClientboundTestInstanceBlockStatus`).
    fn status(&mut self, _entity: &BlockEntity, _status: Text, _size: Option<[i32; 3]>) {}
    /// A slider moved to `value` (0..1).
    fn slide(&mut self, _id: u8, _value: f32) {}
    /// Done: the packet that carries the screen's values.
    fn done(&mut self, entity: &mut BlockEntity, pos: Pos) -> Outcome;
    /// Cancel: put the block entity back.
    fn cancel(&mut self, entity: &mut BlockEntity);
    /// Whether the box takes the typed character (`EditBox.charTyped`).
    fn accepts(&self, _edit: usize, _current: &str, _cursor: usize, _c: char) -> bool {
        true
    }
    /// Whether Enter is Done (a screen with its own `keyPressed`).
    fn enter_confirms(&self) -> bool {
        true
    }
    /// The box that has the focus when the screen opens.
    fn initial_focus(&self) -> usize {
        0
    }
}

/// The open edit screen.
pub(super) struct BlockEditScreen {
    pub(super) pos: Pos,
    pub(super) editor: Box<dyn Editor>,
    /// The text box with the focus, and which of them show.
    focus: Option<usize>,
    /// The text of each box when the screen last looked, to see what changed.
    values: Vec<String>,
    /// The slider being dragged: its id and the x and width it is drawn at.
    dragging: Option<(u8, f32, f32)>,
}

impl BlockEditScreen {
    pub(super) fn new(pos: Pos, editor: Box<dyn Editor>) -> Self {
        let focus = Some(editor.initial_focus());
        Self {
            pos,
            editor,
            focus,
            values: Vec::new(),
            dragging: None,
        }
    }
}

impl Game<'_> {
    /// `LocalPlayer.open*Block`: the edit screen for the block entity at
    /// `pos`.
    pub(super) fn open_block_editor(&mut self, pos: Pos, editor: Box<dyn Editor>) {
        let mut screen = BlockEditScreen::new(pos, editor);
        let opening = self
            .world
            .block_entities
            .get(&pos)
            .map(|entity| screen.editor.opened(entity, pos))
            .unwrap_or_default();
        screen.values = screen
            .editor
            .edits()
            .iter()
            .map(|e| e.value.clone())
            .collect();
        self.block_editor = Some(screen);
        self.screen = Screen::BlockEditor;
        self.capture(false);
        for packet in opening {
            self.send(packet);
        }
    }

    /// A use of a game master block on a server's client, which opens the
    /// screen itself (`BlockEntity.usedBy`) while the server only answers the
    /// click. The block at `pos`, when the player may use it.
    pub(super) fn open_block_editor_if_editable(&mut self, pos: Pos) {
        if !self.can_use_game_master_blocks() {
            return;
        }
        match self.world.block_entities.get(&pos) {
            Some(BlockEntity::StructureBlock(_)) => self.open_structure_block(pos),
            Some(BlockEntity::JigsawBlock(_)) => self.open_jigsaw_block(pos),
            Some(BlockEntity::TestBlock(_)) => self.open_test_block(pos),
            Some(BlockEntity::TestInstanceBlock(_)) => self.open_test_instance_block(pos),
            _ => {}
        }
    }

    pub(super) fn close_block_editor(&mut self) {
        self.block_editor = None;
        self.screen = Screen::Playing;
        self.capture(true);
    }

    /// Run `f` on the open screen with its block entity; the screen closes
    /// when the block is gone (`BlockEntity` removed).
    fn with_block_editor<R>(
        &mut self,
        f: impl FnOnce(&mut BlockEditScreen, &mut BlockEntity) -> R,
    ) -> Option<R> {
        let mut screen = self.block_editor.take()?;
        let result = self
            .world
            .block_entities
            .get_mut(&screen.pos)
            .map(|entity| f(&mut screen, entity));
        if result.is_some() {
            self.block_editor = Some(screen);
        } else {
            self.screen = Screen::Playing;
            self.capture(true);
        }
        result
    }

    fn block_editor_outcome(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Stay => {}
            Outcome::Send(packet) => {
                self.send(packet);
                self.close_block_editor();
            }
            Outcome::SendAll(packets) => {
                for packet in packets {
                    self.send(packet);
                }
                self.close_block_editor();
            }
            Outcome::Cancel => self.close_block_editor(),
        }
    }

    pub(super) fn draw_block_editor(&mut self, batch: &mut HudBatch) {
        let cursor = self.cursor;
        let Some(items) = self.with_block_editor(|s, e| {
            let items = s.editor.items(e, batch.width, batch.height);
            for (i, edit) in s.editor.edits().iter_mut().enumerate() {
                edit.focused = s.focus == Some(i);
            }
            items
        }) else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        batch.fill_gradient(
            atlas,
            (0.0, 0.0, w, h),
            rgba(0x10, 0x10, 0x10, 0xC0),
            rgba(0x10, 0x10, 0x10, 0xD0),
        );
        let Some(mut screen) = self.block_editor.take() else {
            return;
        };
        // Buttons first, then the labels and boxes, as `Screen` renders its
        // widgets before the screen draws its own.
        for item in &items {
            if let Item::Button {
                x,
                y,
                w: bw,
                label,
                active,
                ..
            } = item
            {
                let hover = ui::hovered(cursor, *x, *y, *bw, 20.0);
                ui::button(batch, atlas, *x, *y, *bw, 20.0, label, *active, hover);
            }
        }
        for item in &items {
            match item {
                Item::Title { text, y } => {
                    let tw = atlas.font.width_text(text);
                    batch.text_styled(
                        atlas,
                        text,
                        (w / 2.0 - tw / 2.0).floor(),
                        *y,
                        u32::MAX,
                        true,
                    );
                }
                Item::Label { text, x, y, right } => {
                    let x = if *right {
                        x - atlas.font.width_text(text)
                    } else {
                        *x
                    };
                    batch.text_styled(atlas, text, x, *y, LABEL, true);
                }
                Item::Edit { index, x, y, w: ew } => {
                    if let Some(edit) = screen.editor.edits().get_mut(*index) {
                        (edit.x, edit.y, edit.width, edit.height) = (*x, *y, *ew, 20.0);
                        edit.draw(batch, atlas, None);
                    }
                }
                Item::Button { .. } => {}
                Item::Info {
                    text,
                    x,
                    y,
                    w: iw,
                    h: ih,
                } => {
                    let lines = atlas.font.wrap_components(text, *iw as i32);
                    for (i, line) in lines.iter().take((*ih / 9.0) as usize).enumerate() {
                        batch.text_styled(atlas, line, *x, *y + i as f32 * 9.0, u32::MAX, true);
                    }
                }
                Item::Slider {
                    id,
                    x,
                    y,
                    w: sw,
                    label,
                    value,
                } => {
                    let hover = ui::hovered(cursor, *x, *y, *sw, 20.0);
                    let dragging = screen.dragging.is_some_and(|(d, _, _)| d == *id);
                    ui::slider(
                        batch, atlas, *x, *y, *sw, label, *value, hover, true, dragging,
                    );
                }
            }
        }
        self.block_editor = Some(screen);
    }

    pub(super) fn block_editor_click(&mut self) {
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let cursor = self.cursor;
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let alt = self.keys.contains(&KeyCode::AltLeft);
        let atlas_click = |game: &Game<'_>, edit: &mut EditBox| {
            edit.click(&game.gui.atlas, cursor.0, false, false);
        };
        let Some(items) = self.with_block_editor(|s, e| s.editor.items(e, b.width, b.height))
        else {
            return;
        };
        // The widgets in the order they were added: the last on top.
        for item in items.iter().rev() {
            match item {
                Item::Button {
                    id,
                    x,
                    y,
                    w,
                    active,
                    ..
                } if *active && ui::hovered(cursor, *x, *y, *w, 20.0) => {
                    self.audio.play_button_click();
                    let id = *id;
                    if let Some(outcome) = self.with_block_editor(|s, e| {
                        let pos = s.pos;
                        s.editor.press(e, pos, id, shift, alt)
                    }) {
                        self.block_editor_outcome(outcome);
                    }
                    return;
                }
                Item::Slider { id, x, y, w, .. } if ui::hovered(cursor, *x, *y, *w, 20.0) => {
                    let (id, x, w) = (*id, *x, *w);
                    let value = ui::slider_value(x, w, cursor.0);
                    if let Some(mut screen) = self.block_editor.take() {
                        screen.dragging = Some((id, x, w));
                        screen.editor.slide(id, value);
                        self.block_editor = Some(screen);
                    }
                    return;
                }
                Item::Edit { index, x, y, w } if ui::hovered(cursor, *x, *y, *w, 20.0) => {
                    let index = *index;
                    if let Some(mut screen) = self.block_editor.take() {
                        screen.focus = Some(index);
                        if let Some(edit) = screen.editor.edits().get_mut(index) {
                            (edit.x, edit.width) = (*x, *w);
                            atlas_click(self, edit);
                        }
                        self.block_editor = Some(screen);
                    }
                    return;
                }
                _ => {}
            }
        }
    }

    /// The cursor moved with a slider held (`AbstractSliderButton.onDrag`).
    pub(super) fn block_editor_drag(&mut self) {
        let x = self.cursor.0;
        if let Some(screen) = self.block_editor.as_mut()
            && let Some((id, sx, sw)) = screen.dragging
        {
            screen.editor.slide(id, ui::slider_value(sx, sw, x));
        }
    }

    /// The left button released: a held slider lets go.
    pub(super) fn block_editor_release(&mut self) {
        if let Some(screen) = self.block_editor.as_mut() {
            screen.dragging = None;
        }
    }

    pub(super) fn block_editor_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        let shortcut = self.edit_shortcut(event);
        self.block_editor_key_code(key, shortcut, event.text.as_deref());
    }

    /// A key pressed on an edit screen: Enter is Done, Escape cancels, Tab
    /// moves between the boxes, the rest edits the focused one.
    pub(super) fn block_editor_key_code(
        &mut self,
        key: KeyCode,
        shortcut: Option<horizon_client::clipboard::EditShortcut>,
        text: Option<&str>,
    ) {
        let ctrl = self.keys.contains(&KeyCode::ControlLeft);
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let types_text = self.types_text();
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        match key {
            KeyCode::Escape => {
                let outcome = self
                    .with_block_editor(|s, e| {
                        s.editor.cancel(e);
                        Outcome::Cancel
                    })
                    .unwrap_or(Outcome::Cancel);
                self.block_editor_outcome(outcome);
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                let confirms = self
                    .block_editor
                    .as_ref()
                    .is_some_and(|s| s.editor.enter_confirms());
                if !confirms {
                    return;
                }
                if let Some(outcome) = self.with_block_editor(|s, e| {
                    let pos = s.pos;
                    s.editor.done(e, pos)
                }) {
                    self.block_editor_outcome(outcome);
                }
            }
            KeyCode::Tab => {
                self.with_block_editor(|s, e| {
                    // The boxes on screen, in the order they were added.
                    let shown: Vec<usize> = s
                        .editor
                        .items(e, b.width, b.height)
                        .iter()
                        .filter_map(|i| match i {
                            Item::Edit { index, .. } => Some(*index),
                            _ => None,
                        })
                        .collect();
                    if shown.is_empty() {
                        return;
                    }
                    let at = s.focus.and_then(|f| shown.iter().position(|&i| i == f));
                    let next = match at {
                        None => 0,
                        Some(p) if shift => (p + shown.len() - 1) % shown.len(),
                        Some(p) => (p + 1) % shown.len(),
                    };
                    s.focus = Some(shown[next]);
                });
            }
            _ => {
                let Some(mut screen) = self.block_editor.take() else {
                    return;
                };
                if let Some(focus) = screen.focus
                    && let Some(edit) = screen.editor.edits().get_mut(focus)
                {
                    if let Some(shortcut) = shortcut {
                        edit.shortcut(shortcut, &mut self.clipboard);
                    } else if !edit.key(key, ctrl, shift) && types_text {
                        for c in text.unwrap_or("").chars() {
                            self.block_editor_type_into(&mut screen, focus, c);
                        }
                    }
                }
                self.block_editor = Some(screen);
                self.block_editor_after_input();
            }
        }
    }

    /// What typing changed: the boxes whose text differs from before tell
    /// the editor, which may have packets to send.
    fn block_editor_after_input(&mut self) {
        let Some(mut screen) = self.block_editor.take() else {
            return;
        };
        let pos = screen.pos;
        let now: Vec<String> = screen
            .editor
            .edits()
            .iter()
            .map(|e| e.value.clone())
            .collect();
        let mut packets = Vec::new();
        for (i, value) in now.iter().enumerate() {
            if screen.values.get(i) != Some(value) {
                packets.extend(screen.editor.edited(i, pos));
            }
        }
        screen.values = now;
        self.block_editor = Some(screen);
        for packet in packets {
            self.send(packet);
        }
    }

    /// A status for the open screen.
    pub(super) fn block_editor_status(&mut self, status: Text, size: Option<[i32; 3]>) {
        let Some(mut screen) = self.block_editor.take() else {
            return;
        };
        if let Some(entity) = self.world.block_entities.get(&screen.pos) {
            screen.editor.status(entity, status, size);
        }
        screen.values = screen
            .editor
            .edits()
            .iter()
            .map(|e| e.value.clone())
            .collect();
        self.block_editor = Some(screen);
    }

    fn block_editor_type_into(&mut self, screen: &mut BlockEditScreen, focus: usize, c: char) {
        if c.is_control() {
            return;
        }
        let Some(edit) = screen.editor.edits().get(focus) else {
            return;
        };
        let (value, cursor) = (edit.value.clone(), edit.cursor);
        if !screen.editor.accepts(focus, &value, cursor, c) {
            return;
        }
        if let Some(edit) = screen.editor.edits().get_mut(focus) {
            edit.insert_text(&c.to_string());
        }
    }

    /// Characters typed (`type TEXT` from the agent control): into the box
    /// with the focus.
    pub(super) fn block_editor_type(&mut self, typed: &str) {
        let Some(mut screen) = self.block_editor.take() else {
            return;
        };
        if let Some(focus) = screen.focus {
            for c in typed.chars() {
                self.block_editor_type_into(&mut screen, focus, c);
            }
        }
        self.block_editor = Some(screen);
        self.block_editor_after_input();
    }
}

/// `Screen.isValidCharacterForName`: the characters an identifier takes,
/// with one `:` before any `/`, and `/` only after it.
pub(super) fn valid_character_for_name(current: &str, c: char, cursor: usize) -> bool {
    let colon = current.chars().position(|x| x == ':');
    let slash = current.chars().position(|x| x == '/');
    match c {
        ':' => slash.is_none_or(|s| cursor <= s) && colon.is_none(),
        '/' => colon.map_or(-1, |c| c as i64) < cursor as i64,
        '_' | '-' | '.' | 'a'..='z' | '0'..='9' => true,
        _ => false,
    }
}

/// `CycleButton.cycleValue`: the value `delta` steps on in `values`, wrapping;
/// a current value that is not in the list steps from -1.
pub(super) fn cycle<T: Copy + PartialEq>(values: &[T], current: T, delta: i32) -> T {
    let at = values
        .iter()
        .position(|v| *v == current)
        .map_or(-1, |i| i as i32);
    let len = values.len() as i32;
    values[(at + delta).rem_euclid(len) as usize]
}

/// `CommonComponents.OPTION_ON` / `OPTION_OFF`.
pub(super) fn on_off(on: bool) -> Text {
    Text::translate(if on { "options.on" } else { "options.off" }, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_characters_follow_screen_is_valid_character_for_name() {
        assert!(valid_character_for_name("", 'a', 0));
        assert!(valid_character_for_name("a", '_', 1));
        assert!(!valid_character_for_name("a", 'A', 1));
        assert!(!valid_character_for_name("a", ' ', 1));
        // One colon, not after a slash.
        assert!(valid_character_for_name("ns", ':', 2));
        assert!(!valid_character_for_name("ns:x", ':', 4));
        assert!(!valid_character_for_name("a/b", ':', 3));
        assert!(valid_character_for_name("a/b", ':', 1));
        // A slash only after the colon.
        assert!(valid_character_for_name("ns:a", '/', 4));
        assert!(!valid_character_for_name("ns:a", '/', 1));
        assert!(valid_character_for_name("a", '/', 1));
    }

    #[test]
    fn cycling_wraps_and_steps_from_minus_one_off_the_list() {
        assert_eq!(cycle(&[1, 2, 3], 3, 1), 1);
        assert_eq!(cycle(&[1, 2, 3], 1, -1), 3);
        assert_eq!(cycle(&[1, 2, 3], 9, 1), 1);
        assert_eq!(cycle(&[1, 2, 3], 9, -1), 2);
    }
}
