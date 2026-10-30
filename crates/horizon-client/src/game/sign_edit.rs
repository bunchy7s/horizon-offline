//! The sign editing screen (`SignEditScreen` / `HangingSignEditScreen`):
//! opened when a sign is placed, four lines limited to the sign's line
//! width, Up/Down/Enter move between lines, Escape or Done closes.
use super::{
    Game,
    commands::{Target, execute::Source},
    lan::LanHost,
};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    items::ItemStack,
    signs::{ClickAction, DYE_TEXT, click_action, wood_type},
    ui::EditBox,
};
use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
use winit::{
    event::{ElementState, KeyEvent},
    keyboard::{KeyCode, PhysicalKey},
};

pub(super) struct SignEditor {
    pub pos: (i32, i32, i32),
    pub front: bool,
    pub line: usize,
    /// The line being edited (`AbstractSignEditScreen`'s text field).
    edit: EditBox,
    /// `AbstractSignEditScreen.messages`: the side's lines as edited, sent
    /// when the editor closes, even if the sign is gone by then.
    lines: [String; 4],
}

fn sign_box() -> EditBox {
    let mut edit = EditBox::new(0.0, 0.0, 90.0, 9.0);
    edit.bordered = false;
    edit.max_length = 384;
    edit.focused = true;
    edit
}

/// AbstractSignEditScreen.init's Done button: `width / 2 - 100`,
/// `height / 4 + 144`, int GUI pixels.
fn done_button(w: f32, h: f32) -> (f32, f32) {
    (
        (w.ceil() / 2.0).floor() - 100.0,
        (h.ceil() / 4.0).floor() + 144.0,
    )
}

impl Game<'_> {
    /// `SignBlockEntity.getSlotPlayerIsFacing`: whether the player stands in
    /// front of the sign's text (within 90 degrees of its facing).
    fn facing_sign_front(&self, pos: (i32, i32, i32)) -> bool {
        let Some(player) = self.player.as_ref() else {
            return true;
        };
        let state = self.world.block(pos.0, pos.1, pos.2);
        let Some(state) = self.pack.assets.registry.state(state) else {
            return true;
        };
        let prop = |key: &str| state.properties.get(key).map(String::as_str);
        // getYRotationDegrees: a wall sign's `Direction.toYRot`, a standing or
        // ceiling sign's rotation segment.
        let y_rot = if state.name.contains("_wall_") {
            match prop("facing") {
                Some("west") => 90.0,
                Some("north") => 180.0,
                Some("east") => 270.0,
                _ => 0.0,
            }
        } else {
            prop("rotation")
                .and_then(|r| r.parse::<f32>().ok())
                .unwrap_or(0.0)
                * 22.5
        };
        // getSignHitboxCenterPosition: only a wall sign's board is off-centre,
        // against the wall it hangs on.
        let centre = if state.name.ends_with("_wall_sign") {
            match y_rot as i32 {
                90 => (0.9375, 0.5),
                180 => (0.5, 0.9375),
                270 => (0.0625, 0.5),
                _ => (0.5, 0.0625),
            }
        } else {
            (0.5, 0.5)
        };
        let xd = player.position.x - (f64::from(pos.0) + centre.0);
        let zd = player.position.z - (f64::from(pos.2) + centre.1);
        let player_y_rot = (zd.atan2(xd).to_degrees() - 90.0) as f32;
        // Mth.degreesDifferenceAbs.
        let diff = (player_y_rot - y_rot).rem_euclid(360.0);
        let diff = if diff >= 180.0 { 360.0 - diff } else { diff };
        diff <= 90.0
    }
    /// `SignBlockEntity.canExecuteClickCommands` for the side the acting
    /// player faces: a waxed sign with a `run_command` click event on it.
    pub(super) fn sign_can_execute_click_commands(&self, pos: (i32, i32, i32)) -> bool {
        // Only a waxed sign can; skip the facing maths for everything else.
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        sign.waxed && sign.can_execute_click_commands(self.facing_sign_front(pos))
    }
    /// `SignBlockEntity.createCommandSourceStack`: the acting player runs the
    /// command from the middle of the sign's block, rotation zero, at the
    /// game master level, with `CommandSource.NULL` (no feedback, no failure
    /// messages).
    fn sign_command_source(&self, pos: [i32; 3]) -> Source {
        Source {
            entity: self.player.is_some().then_some(Target::Player),
            pos: pos.map(|v| f64::from(v) + 0.5),
            rot: [0.0, 0.0],
            eyes: false,
            silent: true,
            store: None,
            dimension: self.dim,
            level: None,
            block: None,
        }
    }
    /// Run a sign's `run_command` line as the prefixed command vanilla runs
    /// it as: a leading slash is allowed, errors are not reported (the NULL
    /// source).
    fn run_sign_command(&mut self, pos: [i32; 3], command: &str) {
        let source = self.sign_command_source(pos);
        let line = command.strip_prefix('/').unwrap_or(command);
        self.exec = Default::default();
        let permission = std::mem::replace(&mut self.ui.registries.permission, 2);
        let _ = self.execute_line(line, source);
        self.ui.registries.permission = permission;
        // InventoryChangeTrigger fires as slots change, not on the next tick.
        if self.player.is_some() {
            self.check_inventory_trigger();
        }
    }
    /// `SignBlockEntity.executeClickCommandsIfPresent`: every line of the
    /// side whose style has a `run_command`, `show_dialog` or `custom` click
    /// event runs it as the acting player when the sign allows op features;
    /// otherwise the player is told click actions are disabled. Whether any
    /// line had one.
    fn execute_sign_click_commands(&mut self, pos: [i32; 3], front: bool) -> bool {
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&key) else {
            return false;
        };
        let allow_op_features = sign.allow_op_features;
        let side = if front { &sign.front } else { &sign.back };
        // The side as it was when the loop began: a command may change it.
        let events: Vec<_> = side.click.iter().flatten().cloned().collect();
        let mut any_click_command = false;
        for event in &events {
            match click_action(event) {
                Some(ClickAction::RunCommand(command)) => {
                    if allow_op_features {
                        self.run_sign_command(pos, &command);
                    }
                    any_click_command = true;
                }
                Some(ClickAction::ShowDialog(dialog)) => {
                    if allow_op_features {
                        self.show_click_dialog(dialog);
                    }
                    any_click_command = true;
                }
                // MinecraftServer.handleCustomClickAction only logs it.
                Some(ClickAction::Custom { id, payload }) => {
                    if allow_op_features {
                        log::debug!("Received custom click action {id} with payload {payload:?}");
                    }
                    any_click_command = true;
                }
                None => {}
            }
        }
        if !allow_op_features && any_click_command {
            self.sign_click_actions_disabled();
        }
        any_click_command
    }
    /// `Player.sendOverlayMessage(CLICK_ACTIONS_DISABLED)`: red text on the
    /// action bar. A guest gets a packet, so its own client translates it.
    fn sign_click_actions_disabled(&mut self) {
        const KEY: &str = "sign.click_actions_disabled";
        if self.lan_acting.is_some() {
            let message = horizon_client::text::Text::translate(KEY, &[])
                .with_color(horizon_client::text::RED);
            self.guest_packets.push(C::SystemChat(play::SystemChat {
                content: horizon_client::lan::data::component_nbt(&message.component()),
                overlay: true,
            }));
        } else {
            self.message = Some((
                self.translate(KEY, "Click actions are disabled for this sign"),
                std::time::Instant::now(),
            ));
        }
    }
    /// `SignBlock.useWithoutItem` (the block's `OpenMenu` effect): the side
    /// the player faces runs its click commands first; then a waxed sign
    /// plays the failed-interaction sound, a sign that ran one does nothing
    /// more, and otherwise a player who may build edits the side they face.
    pub(super) fn use_sign_empty_hand(&mut self, pos: [i32; 3]) {
        let key = (pos[0], pos[1], pos[2]);
        if !matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sign(_))
        ) {
            return;
        }
        let front = self.facing_sign_front(key);
        let executed = self.execute_sign_click_commands(pos, front);
        // The commands may have changed the sign, or removed it.
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&key) else {
            return;
        };
        if sign.waxed {
            // `getSignInteractionFailedSoundEvent`: a hanging sign has its own.
            let sound = if sign.hanging {
                "minecraft:block.hanging_sign.waxed_interact_fail"
            } else {
                "minecraft:block.sign.waxed_interact_fail"
            };
            self.audio
                .play(sound, Some(pos.map(|v| f64::from(v) + 0.5)), 1.0, 1.0);
        } else if !executed && self.may_build() {
            self.open_sign_editor(key, front);
        }
    }
    /// `SignBlock.useItemOn` with a `SignApplicator` in hand (dyes, ink sacs,
    /// honeycomb): the item changes the side the player faces and is used up.
    /// Whether it did; a sign it does not apply to goes on to the empty hand.
    pub(super) fn use_sign_applicator(&mut self, pos: [i32; 3], held: Option<&ItemStack>) -> bool {
        let Some(stack) = held else {
            return false;
        };
        let key = (pos[0], pos[1], pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&key) else {
            return false;
        };
        if sign.waxed || !self.may_build() {
            return false;
        }
        let item = &*stack.item;
        let dye = if item.ends_with("_dye") {
            self.res
                .items()
                .get(&stack.item)
                .and_then(|i| i.dye.clone())
        } else {
            None
        };
        let colour = dye.and_then(|d| DYE_TEXT.iter().position(|(n, _)| *n == d));
        if !matches!(
            item,
            "minecraft:honeycomb" | "minecraft:glow_ink_sac" | "minecraft:ink_sac"
        ) && colour.is_none()
        {
            return false;
        }
        let front = self.facing_sign_front(key);
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&key) else {
            return false;
        };
        let centre = pos.map(|v| f64::from(v) + 0.5);
        let sound = if item == "minecraft:honeycomb" {
            // HoneycombItem.tryApplyToSign: waxes any sign, empty or not.
            sign.waxed = true;
            "minecraft:item.honeycomb.wax_on"
        } else {
            let side = if front {
                &mut sign.front
            } else {
                &mut sign.back
            };
            // SignApplicator.canApplyToSign: the side must have text.
            if side.is_empty() {
                return false;
            }
            match colour {
                Some(c) if side.color != c as u8 => {
                    side.color = c as u8;
                    "minecraft:item.dye.use"
                }
                None if item == "minecraft:glow_ink_sac" && !side.glowing => {
                    side.glowing = true;
                    "minecraft:item.glow_ink_sac.use"
                }
                None if item == "minecraft:ink_sac" && side.glowing => {
                    side.glowing = false;
                    "minecraft:item.ink_sac.use"
                }
                _ => return false,
            }
        };
        if item == "minecraft:honeycomb" {
            self.level_event_at(3003, pos, 0);
        }
        self.audio.play(sound, Some(centre), 1.0, 1.0);
        self.mark_block_entity_updated(key);
        self.execute_sign_click_commands(pos, front);
        let state = self.world.block(pos[0], pos[1], pos[2]);
        self.post_block_game_event(
            "minecraft:block_change",
            pos,
            self.sculk_actor(),
            Some(state),
        );
        self.stats.add("used", &stack.item, 1);
        self.consume_held();
        self.trigger_block("minecraft:item_used_on_block", pos, stack);
        true
    }
    /// `SignBlock.openTextEdit` for the front side after placement.
    pub(super) fn open_sign_editor(&mut self, pos: (i32, i32, i32), front: bool) {
        // Minecraft.setScreen: the editor already open is removed first.
        self.close_sign_editor();
        let lines = match self.world.block_entities.get(&pos) {
            Some(BlockEntity::Sign(sign)) if front => sign.front.lines.clone(),
            Some(BlockEntity::Sign(sign)) => sign.back.lines.clone(),
            _ => Default::default(),
        };
        self.sign_edit = Some(SignEditor {
            pos,
            front,
            line: 0,
            edit: sign_box(),
            lines,
        });
        self.sign_load();
        self.keys.clear();
        self.right = false;
        self.capture(false);
    }
    /// `AbstractSignEditScreen.removed`: a server's client sends what it
    /// wrote (ServerboundSignUpdatePacket); the window's own world keeps it
    /// and sends it to its guests (`sendBlockUpdated`).
    pub(super) fn close_sign_editor(&mut self) {
        if let Some(edit) = self.sign_edit.take() {
            if self.remote.is_some() {
                self.send(S::SignUpdate(play::SignUpdate {
                    pos: [edit.pos.0, edit.pos.1, edit.pos.2],
                    lines: edit.lines,
                    front: edit.front,
                }));
            } else {
                self.mark_block_entity_updated(edit.pos);
            }
        }
        self.capture(true);
    }
    /// `ServerPlayer.openTextEdit` for the guest whose action opened a sign:
    /// the block, then the editor, on its client, and the guest may write on it.
    pub(super) fn lan_open_sign_editor(&mut self, host: &mut LanHost, conn: u32, edit: SignEditor) {
        let pos = [edit.pos.0, edit.pos.1, edit.pos.2];
        let state = self.world.block(pos[0], pos[1], pos[2]).0 as i32;
        host.hub
            .send_to(conn, &C::BlockUpdate(play::BlockUpdate { pos, state }));
        host.hub.send_to(
            conn,
            &C::OpenSignEditor(play::OpenSignEditor {
                pos,
                front: edit.front,
            }),
        );
        host.play.entry(conn).or_default().sign_editing = Some(pos);
    }
    /// `ServerGamePacketListenerImpl.handleSignUpdate`: the lines, formatting
    /// codes stripped, go on the sign the guest may write on unless it is
    /// waxed (`SignBlockEntity.updateSignText`); a guest gone out of reach
    /// may no longer write (`playerIsTooFarAwayToEdit`).
    pub(super) fn lan_sign_update(&mut self, host: &mut LanHost, conn: u32, p: play::SignUpdate) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let near = g.within_block_range(p.pos, 4.0);
        let Some(play) = host.play.get_mut(&conn) else {
            return;
        };
        if !near {
            play.sign_editing = None;
        }
        if play.sign_editing != Some(p.pos) {
            log::warn!("{} just tried to change non-editable sign", g.name);
            return;
        }
        let key = (p.pos[0], p.pos[1], p.pos[2]);
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&key) else {
            return;
        };
        if sign.waxed {
            return;
        }
        let side = if p.front {
            &mut sign.front
        } else {
            &mut sign.back
        };
        for (line, new) in side.lines.iter_mut().zip(&p.lines) {
            *line = horizon_client::clipboard::strip_formatting(new);
        }
        play.sign_editing = None;
        self.mark_block_entity_updated(key);
    }
    /// `ClientPacketListener.handleOpenSignEditor`: the editor on a sign this
    /// client has.
    pub(super) fn remote_open_sign_editor(&mut self, p: play::OpenSignEditor) {
        let key = (p.pos[0], p.pos[1], p.pos[2]);
        if matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Sign(_))
        ) {
            self.open_sign_editor(key, p.front);
        } else {
            log::warn!(
                "ignoring openTextEdit on a block that is no sign at {:?}",
                p.pos
            );
        }
    }
    pub(super) fn sign_edit_key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if let Some(shortcut) = self.edit_shortcut(event) {
            self.sign_shortcut(shortcut);
            return;
        }
        let key = match event.physical_key {
            PhysicalKey::Code(k) => Some(k),
            _ => None,
        };
        self.sign_input(key, event.text.as_deref());
    }
    /// A key and the text it typed on the sign being edited (the keyboard,
    /// or a controller's on-screen keyboard).
    pub(super) fn sign_input(&mut self, key: Option<KeyCode>, typed: Option<&str>) {
        if self.sign_edit.is_none() {
            return;
        }
        match key {
            Some(KeyCode::Escape) => self.close_sign_editor(),
            Some(KeyCode::ArrowUp) => self.sign_move_line(3),
            Some(KeyCode::ArrowDown | KeyCode::Enter | KeyCode::NumpadEnter) => {
                self.sign_move_line(1)
            }
            Some(
                key @ (KeyCode::ArrowLeft
                | KeyCode::ArrowRight
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Backspace
                | KeyCode::Delete),
            ) => {
                let (ctrl, shift) = (self.has_control_down(), self.has_shift_down());
                self.sign_change(|edit| {
                    edit.key(key, ctrl, shift);
                });
            }
            _ => {
                let Some(typed) = typed else {
                    return;
                };
                self.sign_change(|edit| edit.insert_text(typed));
            }
        }
    }
    /// Up or down a line: the box writes the line it leaves and loads the next.
    fn sign_move_line(&mut self, by: usize) {
        self.sign_write();
        if let Some(edit) = self.sign_edit.as_mut() {
            edit.line = (edit.line + by) & 3;
        }
        self.sign_load();
    }
    /// The edited line's text.
    fn sign_current(&self) -> Option<String> {
        let edit = self.sign_edit.as_ref()?;
        let BlockEntity::Sign(sign) = self.world.block_entities.get(&edit.pos)? else {
            return None;
        };
        let side = if edit.front { &sign.front } else { &sign.back };
        Some(side.lines[edit.line].clone())
    }
    /// `EditBox.setValue` from the line now being edited.
    fn sign_load(&mut self) {
        let Some(text) = self.sign_current() else {
            return;
        };
        if let Some(edit) = self.sign_edit.as_mut() {
            edit.edit.set_value(&text);
        }
    }
    /// The box's text onto the editor's lines and the sign
    /// (`AbstractSignEditScreen.setMessage`), when it still fits the line.
    fn sign_write(&mut self) {
        let Some(edit) = self.sign_edit.as_ref() else {
            return;
        };
        let (pos, front, line, value) = (edit.pos, edit.front, edit.line, edit.edit.value.clone());
        if !self.sign_fits(&value) {
            return;
        }
        if let Some(edit) = self.sign_edit.as_mut() {
            edit.lines[line].clone_from(&value);
        }
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get_mut(&pos) else {
            return;
        };
        let side = if front {
            &mut sign.front
        } else {
            &mut sign.back
        };
        side.lines[line] = value;
    }
    /// TextFieldHelper: `font.width(s) <= maxTextLineWidth`.
    fn sign_fits(&self, value: &str) -> bool {
        let Some(edit) = self.sign_edit.as_ref() else {
            return false;
        };
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&edit.pos) else {
            return false;
        };
        HudBatch::text_width(&self.gui.atlas, value) <= sign.max_line_width()
    }
    /// Edit the line, then keep the result only when it still fits.
    fn sign_change(&mut self, f: impl FnOnce(&mut EditBox)) {
        let before = self
            .sign_edit
            .as_ref()
            .map(|e| (e.edit.value.clone(), e.edit.cursor, e.edit.highlight));
        if let Some(edit) = self.sign_edit.as_mut() {
            f(&mut edit.edit);
            let clean = horizon_client::clipboard::strip_formatting(&edit.edit.value);
            if clean != edit.edit.value {
                let n = horizon_client::ui::char_len(&clean);
                edit.edit.value = clean;
                edit.edit.cursor = edit.edit.cursor.min(n);
                edit.edit.highlight = edit.edit.highlight.min(n);
            }
        }
        self.sign_keep(before);
    }
    /// Write the box when the line still fits; otherwise put the old text back.
    fn sign_keep(&mut self, before: Option<(String, usize, usize)>) {
        let value = self
            .sign_edit
            .as_ref()
            .map(|e| e.edit.value.clone())
            .unwrap_or_default();
        if self.sign_fits(&value) {
            self.sign_write();
        } else if let (Some(edit), Some((value, cursor, highlight))) =
            (self.sign_edit.as_mut(), before)
        {
            edit.edit.value = value;
            edit.edit.cursor = cursor;
            edit.edit.highlight = highlight;
        }
    }
    /// `TextFieldHelper.keyPressed`'s shortcuts on the edited line. Paste
    /// reads `getClipboardContents` before `insertText`. A paste that would
    /// pass the sign's width is refused whole.
    fn sign_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        if shortcut == horizon_client::clipboard::EditShortcut::Paste {
            let raw = horizon_client::clipboard::Clipboard::get(&mut self.clipboard);
            let text = horizon_client::clipboard::helper_clipboard_contents(&raw);
            self.sign_change(|edit| edit.insert_text(&text));
            return;
        }
        let before = self
            .sign_edit
            .as_ref()
            .map(|e| (e.edit.value.clone(), e.edit.cursor, e.edit.highlight));
        if let Some(edit) = self.sign_edit.as_mut() {
            edit.edit.shortcut(shortcut, &mut self.clipboard);
            let clean = horizon_client::clipboard::strip_formatting(&edit.edit.value);
            if clean != edit.edit.value {
                let n = horizon_client::ui::char_len(&clean);
                edit.edit.value = clean;
                edit.edit.cursor = edit.edit.cursor.min(n);
                edit.edit.highlight = edit.edit.highlight.min(n);
            }
        }
        self.sign_keep(before);
    }
    /// Done button (`this.width / 2 - 100, this.height / 4 + 144`, 200x20),
    /// or a click on a line (`EditBox.onClick`).
    pub(super) fn sign_edit_click(&mut self, size: [u32; 2]) {
        let b = HudBatch::new(size);
        let (x, y) = done_button(b.width, b.height);
        let (mx, my) = self.cursor;
        if mx >= x && mx < x + 200.0 && my >= y && my < y + 20.0 {
            self.audio.play_button_click();
            self.close_sign_editor();
            return;
        }
        let Some((cx, top, line_height, max_width, lines)) = self.sign_lines(b.width) else {
            return;
        };
        let shift = self.has_shift_down();
        for i in 0..lines.len() {
            let ly = top + i as f32 * line_height;
            let left = cx - max_width / 2.0;
            if mx < left || mx >= left + max_width || my < ly || my >= ly + line_height {
                continue;
            }
            let current = self.sign_edit.as_ref().map(|e| e.line);
            if current != Some(i) {
                self.sign_move_line((i + 4 - current.unwrap_or(0)) & 3);
            }
            let value = self
                .sign_edit
                .as_ref()
                .map(|e| e.edit.value.clone())
                .unwrap_or_default();
            let lw = HudBatch::text_width(&self.gui.atlas, &value);
            let text_x = cx - (lw as i32 / 2) as f32;
            if let Some(edit) = self.sign_edit.as_mut() {
                edit.edit.x = text_x;
                edit.edit.y = ly;
                edit.edit.width = max_width;
                edit.edit.click(&self.gui.atlas, mx, shift, false);
            }
            self.sign_write();
            return;
        }
    }
    /// Centre x, the first line's y, the line height, the width limit and the four lines.
    fn sign_lines(&self, width: f32) -> Option<(f32, f32, f32, f32, Vec<String>)> {
        let edit = self.sign_edit.as_ref()?;
        let BlockEntity::Sign(sign) = self.world.block_entities.get(&edit.pos)? else {
            return None;
        };
        let text = if edit.front { &sign.front } else { &sign.back };
        let line_height = sign.line_height();
        let y = if sign.hanging { 125.0 } else { 90.0 };
        let top = y - (4.0 * line_height / 2.0).floor();
        Some((
            width / 2.0,
            top,
            line_height,
            sign.max_line_width(),
            text.lines.to_vec(),
        ))
    }
    pub(super) fn draw_sign_editor(&self, batch: &mut HudBatch) {
        let Some(edit) = &self.sign_edit else { return };
        let Some(BlockEntity::Sign(sign)) = self.world.block_entities.get(&edit.pos) else {
            return;
        };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(16, 16, 16, 160));
        let (key, fallback) = if sign.hanging {
            ("hanging_sign.edit", "Edit Hanging Sign Message")
        } else {
            ("sign.edit", "Edit Sign Message")
        };
        let title = self.translate(key, fallback);
        let tw = HudBatch::text_width(atlas, &title);
        batch.text(atlas, &title, (w - tw) / 2.0, 40.0, u32::MAX, true);
        let name = self
            .res
            .facts()
            .get(self.world.block(edit.pos.0, edit.pos.1, edit.pos.2))
            .name
            .clone();
        let wood = wood_type(&name);
        let cx = w / 2.0;
        let y = if sign.hanging {
            // HangingSignEditScreen: 16x16 texture scaled 4.5 at (0, -13).
            let s = 4.5;
            batch.sprite_scaled(
                atlas,
                &format!("gui/hanging_signs/{wood}"),
                cx - 8.0 * s,
                125.0 - 13.0 - 8.0 * s,
                16.0 * s,
                16.0 * s,
                u32::MAX,
            );
            125.0
        } else {
            // SignEditScreen: 24x26 texture scaled 3.9 at (0, 27); wall signs show 12 rows.
            let s = 3.9;
            let wall = name.contains("_wall_");
            let rows = if wall { 12.0 } else { 26.0 };
            if let Some(mut r) = atlas.sprites.get(&format!("gui/signs/{wood}")).copied() {
                r.v1 = r.v0 + (r.v1 - r.v0) * rows / 26.0;
                batch.rect_uv(
                    cx - 12.0 * s,
                    90.0 + 27.0 - 13.0 * s,
                    24.0 * s,
                    rows * s,
                    r,
                    u32::MAX,
                );
            }
            90.0
        };
        let text = if edit.front { &sign.front } else { &sign.back };
        let color = if text.glowing {
            text.text_color()
        } else {
            text.dark_color()
        };
        let color = rgba((color >> 16) as u8, (color >> 8) as u8, color as u8, 255);
        let line_height = sign.line_height();
        let midpoint = (4.0 * line_height / 2.0).floor();
        for (i, line) in text.lines.iter().enumerate() {
            let shown = if i == edit.line {
                edit.edit.value.as_str()
            } else {
                line.as_str()
            };
            let lw = HudBatch::text_width(atlas, shown);
            let x = cx - (lw as i32 / 2) as f32;
            let ly = y + i as f32 * line_height - midpoint;
            batch.text(atlas, shown, x, ly, color, false);
            if i != edit.line {
                continue;
            }
            let value = &edit.edit.value;
            if edit.edit.cursor != edit.edit.highlight {
                let (a, b) = (
                    edit.edit.cursor.min(edit.edit.highlight),
                    edit.edit.cursor.max(edit.edit.highlight),
                );
                let pre: String = value.chars().take(a).collect();
                let sel: String = value.chars().skip(a).take(b - a).collect();
                batch.text_highlight(
                    atlas,
                    (
                        x + HudBatch::text_width(atlas, &pre),
                        ly,
                        HudBatch::text_width(atlas, &sel),
                        line_height,
                    ),
                    true,
                );
            }
            if edit.edit.cursor_visible() {
                let pre: String = value.chars().take(edit.edit.cursor).collect();
                batch.fill(
                    atlas,
                    x + HudBatch::text_width(atlas, &pre),
                    ly,
                    1.0,
                    line_height,
                    color,
                );
            }
        }
        let (bx, by) = done_button(w, h);
        batch.sprite_scaled(
            atlas,
            "gui/sprites/widget/button",
            bx,
            by,
            200.0,
            20.0,
            u32::MAX,
        );
        let tw = HudBatch::text_width(atlas, "Done");
        batch.text(
            atlas,
            "Done",
            bx + 100.0 - (tw / 2.0).floor(),
            by + 6.0,
            u32::MAX,
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn done_button_sits_on_whole_gui_pixels() {
        // `width / 2 - 100, height / 4 + 144` in ints: 334 high is 83 + 144.
        assert_eq!(super::done_button(427.0, 334.0), (113.0, 227.0));
        assert_eq!(super::done_button(320.0, 240.0), (60.0, 204.0));
    }
}
