//! A server's Code of Conduct bound to the game (`ClientConfigurationPacketListenerImpl.
//! handleCodeOfConduct` and `CodeOfConductScreen`): the text in a scroll area,
//! the "do not notify again" box, Acknowledge and Disconnect, and the server
//! list entry that keeps the accepted text. The server waits in configuration
//! until the answer, so the connection stays open while the screen shows.
use super::{Game, Screen, server_packs::saves};
use horizon_client::{
    Result,
    dialog_screen::BOX,
    font::Font,
    hud::{HudBatch, rgba},
    multiplayer,
    text::Text,
    ui,
};
use horizon_core::random::java_string_hash;
use horizon_protocol::servers;

/// The text's line height.
const LINE: f32 = 9.0;
/// The footer's button: `Button.DEFAULT_WIDTH` and the row's spacing of 8.
const BUTTON: (f32, f32) = (150.0, 20.0);
const GAP: f32 = 8.0;
/// The text box's cell padding (`s.padding(12)`) and the text's padding inside it.
const MARGIN: f32 = 12.0;
const PAD: f32 = 4.0;
/// Narrowest column: the two buttons and their gap.
const COLUMN: f32 = 2.0 * BUTTON.0 + GAP;

/// An open `CodeOfConductScreen`.
pub(super) struct Prompt {
    text: String,
    /// `text.hashCode()`: what the server list keeps once it is accepted.
    hash: i32,
    /// `stopShowing`: the box is ticked.
    stop_showing: bool,
    scroll: ui::Scroll,
}

/// Where the screen's parts go (`WarningScreen`: title, the text box with 12 px
/// around it, the checkbox, then the buttons, 8 px apart): the title and the
/// wrapped text in its box (`area`: x, y, w, h), then the check and the
/// two buttons.
struct Layout {
    title: (Text, f32, f32),
    lines: Vec<Text>,
    area: [f32; 4],
    /// The text's full height with its padding: more than the area's scrolls.
    content: f32,
    check: [f32; 4],
    label: Text,
    buttons: [[f32; 4]; 2],
}

impl Prompt {
    fn layout(&self, font: &Font, (w, h): (f32, f32)) -> Layout {
        let title = Text::translate("multiplayer.codeOfConduct.title", &[]).bold();
        let label = Text::translate("multiplayer.codeOfConduct.check", &[]);
        // `FittingMultiLineTextWidget`: `width - 100` wide, `height - 100` at
        // most, the text 4 px inside.
        let col = (w - 100.0).max(COLUMN);
        let lines: Vec<Text> = font
            .split_lines(&Text::literal(self.text.as_str()), col - 2.0 * PAD)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        let content = LINE * lines.len() as f32 + 2.0 * PAD;
        let box_h = content.min((h - 100.0).max(LINE + 2.0 * PAD));
        let fixed = LINE + GAP + MARGIN + MARGIN + GAP + BOX + GAP + BUTTON.1;
        let total = fixed + box_h;
        let ly = ((h - total) / 2.0).trunc();
        let cell = |cw: f32| ((w - cw) / 2.0).trunc();
        let area_y = ly + LINE + GAP + MARGIN;
        let check_y = area_y + box_h + MARGIN + GAP;
        let check_w = BOX + 4.0 + font.width_text(&label);
        let button_y = check_y + BOX + GAP;
        let bx = cell(COLUMN);
        Layout {
            title: (title.clone(), cell(font.width_text(&title)), ly),
            lines,
            area: [cell(col), area_y, col, box_h],
            content,
            check: [cell(check_w), check_y, check_w, BOX],
            label,
            buttons: [
                [bx, button_y, BUTTON.0, BUTTON.1],
                [bx + BUTTON.0 + GAP, button_y, BUTTON.0, BUTTON.1],
            ],
        }
    }
}

impl Game<'_> {
    /// The server list entry of the joined address, if it has one.
    fn accepted_code_of_conduct(&self) -> Option<i32> {
        let address = self.server_packs.address.as_deref()?;
        servers::load(&multiplayer::servers_file(&saves()))
            .into_iter()
            .find(|s| s.ip == address)
            .map(|s| s.accepted_code_of_conduct)
    }

    /// `ServerList.saveSingleServer`: the joined address's entry keeps `accepted`.
    fn store_code_of_conduct(&self, accepted: i32) {
        let Some(address) = &self.server_packs.address else {
            return;
        };
        let file = multiplayer::servers_file(&saves());
        let mut list = servers::load(&file);
        let mut found = false;
        for entry in list.iter_mut().filter(|s| &s.ip == address) {
            entry.accepted_code_of_conduct = accepted;
            found = true;
        }
        if found && let Err(e) = servers::save(&file, &list) {
            log::warn!("Horizon: could not save {}: {e}", file.display());
        }
    }

    /// `handleCodeOfConduct`: text accepted before for this server is
    /// accepted at once; otherwise the screen opens over the connecting one.
    pub(super) fn handle_code_of_conduct(&mut self, text: String) {
        let Some(r) = &self.remote else {
            return;
        };
        let hash = java_string_hash(text.encode_utf16());
        if self.accepted_code_of_conduct() == Some(hash) {
            r.session.accept_code_of_conduct();
            return;
        }
        self.capture(false);
        self.code_of_conduct = Some(Prompt {
            text,
            hash,
            stop_showing: false,
            scroll: ui::Scroll::default(),
        });
    }

    /// `CodeOfConductScreen.onResult`: accepting tells the server, declining
    /// ends the connection. The entry keeps the text only when it was
    /// accepted with the box ticked, else the stored text is cleared.
    fn answer_code_of_conduct(&mut self, accepted: bool) -> Result<()> {
        let Some(prompt) = self.code_of_conduct.take() else {
            return Ok(());
        };
        if accepted {
            if let Some(r) = &self.remote {
                r.session.accept_code_of_conduct();
            }
            if self.player.is_some() && self.screen == Screen::Playing {
                self.capture(true);
            }
        }
        let kept = if accepted && prompt.stop_showing {
            prompt.hash
        } else {
            0
        };
        self.store_code_of_conduct(kept);
        if accepted {
            return Ok(());
        }
        let reason = Text::translate("multiplayer.disconnect.code_of_conduct", &[]);
        self.leave_server(Some(reason.plain()))
    }

    pub(super) fn draw_code_of_conduct(&self, batch: &mut HudBatch) {
        let Some(prompt) = &self.code_of_conduct else {
            return;
        };
        // CodeOfConductScreen: WarningScreen's narration, the title and the
        // text on the next line.
        horizon_client::widgets::begin_screen(|| {
            let title = super::screen_info::tr("multiplayer.codeOfConduct.title", &[]);
            horizon_client::widgets::ScreenInfo::new(
                "code_of_conduct",
                format!("{title}\n{}", prompt.text),
            )
        });
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        if self.player.is_none() {
            // In place of the connecting screen, on its background.
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
        }
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, self.player.is_some());
        let layout = prompt.layout(&atlas.font, (w, h));
        let cursor = self.cursor;
        let [ax, ay, aw, ah] = layout.area;
        let offset = prompt
            .scroll
            .amount
            .clamp(0.0, (layout.content - ah).max(0.0));

        let (title, tx, ty) = &layout.title;
        batch.text_styled(atlas, title, *tx, *ty, u32::MAX, true);
        ui::nine_slice(
            batch,
            atlas,
            "gui/sprites/widget/text_field",
            ax,
            ay,
            aw,
            ah,
            u32::MAX,
        );
        let start = batch.vertices.len();
        for (i, line) in layout.lines.iter().enumerate() {
            let y = ay + PAD + i as f32 * LINE - offset;
            batch.text_styled(atlas, line, ax + PAD, y, u32::MAX, true);
        }
        batch.clip_since(start, ax + 1.0, ay + 1.0, ax + aw - 1.0, ay + ah - 1.0);
        if layout.content > ah {
            // AbstractTextAreaWidget.scrollBarX: the box's right edge.
            ui::scrollbar(batch, atlas, ax + aw, ay, ay + ah, layout.content, offset);
        }

        let [x, y, cw, ch] = layout.check;
        let hover = ui::hovered(cursor, x, y, cw, ch);
        horizon_client::widgets::narratable(|| {
            horizon_client::widgets::Narratable::new(
                horizon_client::widgets::What::Checkbox {
                    message: layout.label.plain(),
                    selected: prompt.stop_showing,
                    active: true,
                },
                [x, y, cw, ch],
            )
            .hovered(hover)
        });
        let sprite = match (prompt.stop_showing, hover) {
            (true, true) => "gui/sprites/widget/checkbox_selected_highlighted",
            (true, false) => "gui/sprites/widget/checkbox_selected",
            (false, true) => "gui/sprites/widget/checkbox_highlighted",
            (false, false) => "gui/sprites/widget/checkbox",
        };
        batch.sprite_scaled(atlas, sprite, x, y, BOX, BOX, u32::MAX);
        batch.text_styled(
            atlas,
            &layout.label,
            x + BOX + 4.0,
            y + 4.0,
            rgba(224, 224, 224, 255),
            true,
        );

        let labels = [
            Text::translate("gui.acknowledge", &[]),
            Text::translate("menu.disconnect", &[]),
        ];
        for ([x, y, bw, bh], label) in layout.buttons.into_iter().zip(&labels) {
            let hover = ui::hovered(cursor, x, y, bw, bh);
            ui::button(batch, atlas, x, y, bw, bh, label, true, hover);
        }
    }

    /// A left click: the box toggles, a button answers.
    pub(super) fn code_of_conduct_click(&mut self) -> Result<()> {
        let size = self.gui_size();
        let cursor = self.cursor;
        let Some(prompt) = self.code_of_conduct.as_mut() else {
            return Ok(());
        };
        let layout = prompt.layout(&self.gui.atlas.font, size);
        let [x, y, w, h] = layout.check;
        let toggled = ui::hovered(cursor, x, y, w, h);
        if toggled {
            prompt.stop_showing = !prompt.stop_showing;
        }
        let answer = layout
            .buttons
            .iter()
            .position(|&[x, y, w, h]| ui::hovered(cursor, x, y, w, h))
            .map(|i| i == 0);
        if !toggled && answer.is_none() {
            return Ok(());
        }
        self.audio.play_button_click();
        match answer {
            Some(accepted) => self.answer_code_of_conduct(accepted),
            None => Ok(()),
        }
    }

    /// The mouse wheel over the screen: the text scrolls 10 a step.
    pub(super) fn code_of_conduct_scroll(&mut self, delta: f32) {
        let size = self.gui_size();
        let Some(prompt) = self.code_of_conduct.as_mut() else {
            return;
        };
        let layout = prompt.layout(&self.gui.atlas.font, size);
        let [_, ay, _, ah] = layout.area;
        prompt
            .scroll
            .wheel(delta, 10.0, ay, ay + ah, layout.content);
    }
}
