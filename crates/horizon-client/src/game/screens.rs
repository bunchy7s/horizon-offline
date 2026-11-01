//! The pause menu (PauseScreen) and death screen (DeathScreen) layouts.
use super::{Game, Screen};
use horizon_client::{
    Result,
    command::args::GameMode,
    confirm_screen::Confirm,
    dialog::Dialog,
    hud::{HudBatch, rgba},
    text::{self, Text},
    ui,
    widgets::{self, ScreenInfo},
};
use horizon_world::nbt::Tag;
use winit::keyboard::KeyCode;

impl Screen {
    /// Screen.extractBackground for non-isInGameUi screens, and
    /// PauseScreen.extractBackground only when showPauseMenu is true.
    pub(super) fn blurs_background(self, pause_menu: bool) -> bool {
        match self {
            Self::Paused => pause_menu,
            Self::Options
            | Self::Stats
            | Self::Advancements
            | Self::WorldOptions
            | Self::Friends
            | Self::ServerLinks
            | Self::ConfirmLink
            | Self::DialogWarning
            | Self::DebugOptions => true,
            Self::Playing
            | Self::GameModeSwitcher
            | Self::Chat
            | Self::Creative
            | Self::Inventory
            | Self::Dead
            | Self::CommandBlock
            | Self::BlockEditor => false,
            // WinScreen.extractBackground: the poem fills the screen with the End portal, the
            // credits alone blur. Game::menu_blur knows which one rolls.
            Self::Win => false,
        }
    }
}

/// The pause menu grid's origin: GridLayout of two 98px columns (padding 4),
/// aligned at (0.5, 0.25). `row3`: the full-width row at y + 122 that holds
/// the `pause_screen_additions` button, or the `minecraft:server_links` one
/// when the tag is empty (`PauseScreen.getCustomAdditions`).
fn pause_grid(w: f32, h: f32, row3: bool) -> (f32, f32) {
    let (grid_w, grid_h) = (212.0, 166.0 + if row3 { 24.0 } else { 0.0 });
    (((w - grid_w) * 0.5).floor(), ((h - grid_h) * 0.25).floor())
}

/// PauseScreen buttons: (x, y, width, label key, action id, active).
/// `links`: the `minecraft:server_links` dialog's button, which
/// `PauseScreen.getCustomAdditions` falls back to when `pause_screen_additions`
/// is empty and the server sent links — a full-width row that pushes the last
/// two rows down.
/// `additions`: the `pause_screen_additions` button instead of the links one
/// (vanilla shows one or the other, never both); its label is the dialog's
/// external title, so the caller draws it (id 14).
/// https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/PauseScreen
fn pause_buttons(
    w: f32,
    h: f32,
    remote: bool,
    links: bool,
    additions: bool,
) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    let row3 = links || additions;
    let (x0, y0) = pause_grid(w, h, row3);
    let down = if row3 { 24.0 } else { 0.0 };
    let mut v = vec![
        (x0 + 4.0, y0 + 50.0, 204.0, "menu.returnToGame", 0, true),
        (x0 + 4.0, y0 + 74.0, 98.0, "gui.advancements", 4, true),
        (x0 + 110.0, y0 + 74.0, 98.0, "gui.stats", 3, true),
    ];
    if additions {
        // addCustomDialogButtons: one full-width button, labelled with the
        // dialog's CommonDialogData.computeExternalTitle.
        v.push((x0 + 4.0, y0 + 122.0, 204.0, "", 14, true));
    } else if links {
        // CommonDialogData.computeExternalTitle of Dialogs.SERVER_LINKS.
        v.push((x0 + 4.0, y0 + 122.0, 204.0, "menu.server_links", 11, true));
    }
    v.extend([
        (x0 + 4.0, y0 + 122.0 + down, 98.0, "menu.options", 1, true),
        (
            x0 + 110.0,
            y0 + 122.0 + down,
            98.0,
            "options.worldOptions.button",
            9,
            true,
        ),
        (
            x0 + 4.0,
            y0 + 146.0 + down,
            204.0,
            if remote {
                "menu.disconnect"
            } else {
                "menu.returnToMenu"
            },
            2,
            true,
        ),
    ]);
    v
}

/// The icon row: report bugs, feedback, friends, player reporting.
/// `FRIENDS_ICON` (vanilla's `CommonButtons.friends`) opens Horizon's
/// Play with Friends; the first two open their link through
/// `ConfirmLinkScreen`; player reporting (`SocialInteractionsScreen`, not
/// ported) stays inactive.
const FRIENDS_ICON: usize = 2;
/// `CommonLinks.SNAPSHOT_BUGS_FEEDBACK`, behind `PauseScreen.REPORT_BUGS`.
const BUGS_LINK: &str = "https://aka.ms/snapshotbugs?ref=game";
/// `CommonLinks.RELEASE_FEEDBACK` / `SNAPSHOT_FEEDBACK`, by `WorldVersion.stable()`.
fn feedback_link() -> &'static str {
    if horizon_core::version::STABLE {
        "https://aka.ms/javafeedback?ref=game"
    } else {
        "https://aka.ms/snapshotfeedback?ref=game"
    }
}
/// `PauseScreen.createPauseMenu`'s icon tooltips (`SpriteIconButton.withTootip`).
const ICON_LABELS: [&str; 4] = [
    "menu.reportBugs",
    "menu.sendFeedback",
    "gui.friends.open",
    "menu.playerReporting",
];
/// Whether an icon button is active: `reportBugsButton.active =
/// !dataVersion().isSideSeries()` (26.3 is the `main` series), the feedback
/// button always, friends when the friends list is shown.
fn icon_active(icon: usize, friends: bool) -> bool {
    match icon {
        0 | 1 => true,
        FRIENDS_ICON => friends,
        _ => false,
    }
}
fn pause_icon_focus_ids(friends: bool) -> Vec<u8> {
    (0..4)
        .filter(|&i| icon_active(i, friends))
        .map(|i| if i == FRIENDS_ICON { 10 } else { 12 + i as u8 })
        .collect()
}
fn pause_icons(w: f32, h: f32, links: bool, additions: bool) -> [(f32, f32, &'static str); 4] {
    let (x0, y0) = pause_grid(w, h, links || additions);
    let (x0, y) = (x0 + 60.0, y0 + 98.0);
    [
        (x0, y, "gui/sprites/pause_menu/bug"),
        (x0 + 24.0, y, "gui/sprites/pause_menu/social_interactions"),
        (x0 + 48.0, y, "gui/sprites/friends/friends"),
        (x0 + 72.0, y, "gui/sprites/pause_menu/player_reporting"),
    ]
}
/// DeathScreen.init: a hardcore player can only spectate.
/// `DeathScreen.TitleConfirmScreen`: its title and the yes and no labels.
fn death_confirm_texts() -> (Text, [Text; 2]) {
    let t = |key: &str| Text::translate(key, &[]);
    (
        t("deathScreen.quit.confirm"),
        [t("deathScreen.titleScreen"), t("deathScreen.respawn")],
    )
}
fn death_buttons(w: f32, h: f32, hardcore: bool) -> [(f32, f32, &'static str, u8); 2] {
    let respawn = if hardcore {
        "deathScreen.spectate"
    } else {
        "deathScreen.respawn"
    };
    // DeathScreen.init: `width / 2 - 100`, `height / 4 + 72` and `+ 96`, in
    // int GUI pixels.
    let (x, y) = ((w.ceil() / 2.0).floor() - 100.0, (h.ceil() / 4.0).floor());
    [
        (x, y + 72.0, respawn, 0),
        (x, y + 96.0, "deathScreen.titleScreen", 1),
    ]
}
impl Game<'_> {
    /// `PauseScreen.getCustomAdditions`'s additions: the
    /// `minecraft:pause_screen_additions` tag's one dialog, or the
    /// `minecraft:custom_options` `dialog_list` when it names several. `None`
    /// when the tag is empty: the caller falls back to the server links
    /// button, as vanilla does.
    fn pause_screen_additions(&self) -> Option<(Text, Tag)> {
        let registries = &self.remote.as_ref()?.registries;
        let dialogs = registries.registries.get("minecraft:dialog")?;
        let tag = registries
            .dialog_tags
            .get("minecraft:pause_screen_additions")?;
        // addCustomDialogButtons' label: computeExternalTitle falls back to the title.
        let button = |dialog_tag: Tag| -> Option<(Text, Tag)> {
            let dialog = Dialog::parse(&dialog_tag)?;
            let label = dialog.external_title.unwrap_or(dialog.title);
            Some((label, dialog_tag))
        };
        match tag.len() {
            0 => None,
            1 => {
                let i = usize::try_from(tag[0]).ok()?;
                button(dialogs.get(i)?.1.clone()?)
            }
            // `Dialogs.CUSTOM_OPTIONS`: the dialog list over the same tag.
            _ => button(
                dialogs
                    .iter()
                    .find(|(name, _)| name == "minecraft:custom_options")?
                    .1
                    .clone()?,
            ),
        }
    }

    /// Singleplayer also has a connection, backed by the integrated server.
    pub(super) fn external_server(&self) -> bool {
        self.remote.is_some() && self.integrated.is_none()
    }
    pub(super) fn draw_pause_menu(&mut self, batch: &mut HudBatch) {
        let friends = self.friends_view().shown;
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let title = Text::translate("menu.game", &[]);
        widgets::begin_screen(|| ScreenInfo::new("pause", title.plain()));
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            40.0,
            u32::MAX,
            true,
        );
        let links = !self.server_links().is_empty();
        let additions = self.pause_screen_additions();
        let mut tooltip = None;
        for (x, y, bw, key, id, active) in
            pause_buttons(w, h, self.external_server(), links, additions.is_some())
        {
            let is_focused = self.pause_focus == Some(id);
            let hover = active && (ui::hovered(self.cursor, x, y, bw, 20.0) || is_focused);
            let label = match (id, &additions) {
                (14, Some((label, _))) => label.clone(),
                _ => Text::translate(key, &[]),
            };
            ui::button(batch, atlas, x, y, bw, 20.0, &label, active, hover);
            // PauseScreen's customAdditions button: CUSTOM_OPTIONS_TOOLTIP.
            if (id == 11 || id == 14) && hover {
                tooltip = Some(Text::translate("menu.custom_options.tooltip", &[]));
            }
        }
        for (i, (x, y, sprite)) in pause_icons(w, h, links, additions.is_some())
            .into_iter()
            .enumerate()
        {
            let active = icon_active(i, friends);
            let is_focused = (self.pause_focus == Some(10) && i == FRIENDS_ICON)
                || (self.pause_focus == Some(12 + i as u8));
            let hover = active && (ui::hovered(self.cursor, x, y, 20.0, 20.0) || is_focused);
            // The icon has no label: name it for controller focus and the
            // control socket's widget tree.
            if i == FRIENDS_ICON {
                horizon_client::widgets::record(
                    horizon_client::friends_screen::TITLE,
                    [x, y, 20.0, 20.0],
                    active,
                );
            }
            ui::button(
                batch,
                atlas,
                x,
                y,
                20.0,
                20.0,
                &Text::empty(),
                active,
                hover,
            );
            // PauseScreen adds the icon row after Statistics: the icons'
            // messages (their tooltips too), the friends button's own.
            widgets::last_narratable_at(3 + i, |n| {
                let t = |key: &str| Text::translate(key, &[]).plain();
                let key = [
                    "menu.reportBugs",
                    "menu.sendFeedback",
                    "",
                    "menu.playerReporting",
                ][i];
                *n = if i == FRIENDS_ICON {
                    widgets::Narratable::named(t("gui.friends.open.narration"), n.rect, active)
                        .hint(t("gui.friends.open.tooltip"))
                } else {
                    widgets::Narratable::button(t(key), n.rect, active).hint(t(key))
                }
                .hovered(hover);
            });
            // SpriteIconButton.CenteredIcon: the 15x15 sprite at
            // x + width / 2 - 15 / 2 in integer pixels, untinted even
            // when the button is inactive.
            batch.sprite_scaled(atlas, sprite, x + 3.0, y + 3.0, 15.0, 15.0, u32::MAX);
            if hover {
                tooltip = Some(if i == FRIENDS_ICON {
                    Text::literal(horizon_client::friends_screen::TITLE)
                } else {
                    Text::translate(ICON_LABELS[i], &[])
                });
            }
        }
        // SpriteIconButton.withTootip: the label as a tooltip.
        if let Some(tip) = tooltip {
            let lines: Vec<Text> = atlas
                .font
                .split_lines(&tip, batch.width)
                .into_iter()
                .map(|(line, _)| line)
                .collect();
            ui::tooltip(batch, atlas, &lines, self.cursor);
        }
    }
    pub(super) fn execute_pause_action(&mut self, hit: u8) -> Result<()> {
        match hit {
            0 => {
                self.pause_focus = None;
                self.screen = Screen::Playing;
                self.capture(true);
            }
            1 => self.open_options(false),
            3 => self.open_stats(),
            4 => self.open_advancements(true),
            9 => self.open_world_options(),
            10 => self.screen = Screen::Friends,
            // DialogConnectionAccess.openDialog(Dialogs.SERVER_LINKS).
            11 => {
                self.links_ui.scroll = 0.0;
                self.screen = Screen::ServerLinks;
            }
            // ConfirmLinkScreen.confirmLink(this, CommonLinks...) on the pause screen.
            12 => self.confirm_trusted_link(BUGS_LINK, Screen::Paused),
            13 => self.confirm_trusted_link(feedback_link(), Screen::Paused),
            // addCustomDialogButtons: DialogConnectionAccess.openDialog of the
            // additions dialog, from the pause menu.
            14 => {
                if let Some((_, dialog)) = self.pause_screen_additions() {
                    self.pause_focus = None;
                    self.screen = Screen::Playing;
                    self.capture(true);
                    self.show_dialog(&dialog);
                }
            }
            2 => self.close_world()?,
            _ => {}
        }
        Ok(())
    }
    pub(super) fn pause_click(&mut self) -> Result<()> {
        let (w, h) = {
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size);
            (b.width, b.height)
        };
        let links = !self.server_links().is_empty();
        let additions = self.pause_screen_additions();
        let icons = pause_icons(w, h, links, additions.is_some());
        let friends = icons[FRIENDS_ICON];
        let hit = pause_buttons(w, h, self.external_server(), links, additions.is_some())
            .into_iter()
            .find(|b| b.5 && ui::hovered(self.cursor, b.0, b.1, b.2, 20.0))
            .map(|b| b.4)
            .or_else(|| {
                // The report bugs and feedback icons (ids 12 and 13).
                (0..2)
                    .find(|&i| ui::hovered(self.cursor, icons[i].0, icons[i].1, 20.0, 20.0))
                    .map(|i| 12 + i as u8)
            })
            .or_else(|| {
                (self.friends_view().shown
                    && ui::hovered(self.cursor, friends.0, friends.1, 20.0, 20.0))
                .then_some(10)
            });
        if let Some(id) = hit {
            self.pause_focus = Some(id);
            self.audio.play_button_click();
            self.execute_pause_action(id)?;
        }
        Ok(())
    }
    pub(super) fn pause_key(&mut self, key: KeyCode) -> Result<()> {
        let (w, h) = {
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size);
            (b.width, b.height)
        };
        let links = !self.server_links().is_empty();
        let buttons = pause_buttons(
            w,
            h,
            self.external_server(),
            links,
            self.pause_screen_additions().is_some(),
        );
        let mut active_ids: Vec<u8> = buttons.into_iter().filter(|b| b.5).map(|b| b.4).collect();
        let friends = self.friends_view().shown;
        active_ids.extend(pause_icon_focus_ids(friends));
        if active_ids.is_empty() {
            return Ok(());
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                let next = match self.pause_focus {
                    Some(cur) => {
                        if let Some(pos) = active_ids.iter().position(|&id| id == cur) {
                            if reverse {
                                if pos == 0 {
                                    active_ids.last().copied()
                                } else {
                                    Some(active_ids[pos - 1])
                                }
                            } else if pos + 1 >= active_ids.len() {
                                active_ids.first().copied()
                            } else {
                                Some(active_ids[pos + 1])
                            }
                        } else if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                    None => {
                        if reverse {
                            active_ids.last().copied()
                        } else {
                            active_ids.first().copied()
                        }
                    }
                };
                self.pause_focus = next;
            }
            KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let buttons = pause_buttons(
                    w,
                    h,
                    self.external_server(),
                    links,
                    self.pause_screen_additions().is_some(),
                );
                let mut rects: Vec<(u8, f32, f32, f32)> = buttons
                    .into_iter()
                    .filter(|b| b.5)
                    .map(|b| (b.4, b.0, b.1, b.2))
                    .collect();
                let friends = self.friends_view().shown;
                for (i, (x, y, _)) in
                    pause_icons(w, h, links, self.pause_screen_additions().is_some())
                        .into_iter()
                        .enumerate()
                {
                    if icon_active(i, friends) {
                        let id = if i == FRIENDS_ICON { 10 } else { 12 + i as u8 };
                        rects.push((id, x, y, 20.0));
                    }
                }
                let cur_id = self.pause_focus.unwrap_or(0);
                if let Some(cur) = rects.iter().find(|r| r.0 == cur_id) {
                    let (cx, cy) = (cur.1 + cur.3 / 2.0, cur.2 + 10.0);
                    let mut best: Option<(u8, f32)> = None;
                    for r in &rects {
                        if r.0 == cur_id {
                            continue;
                        }
                        let (rx, ry) = (r.1 + r.3 / 2.0, r.2 + 10.0);
                        let (dx, dy) = (rx - cx, ry - cy);
                        let valid = match key {
                            KeyCode::ArrowUp => dy < -4.0,
                            KeyCode::ArrowDown => dy > 4.0,
                            KeyCode::ArrowLeft => dx < -4.0,
                            KeyCode::ArrowRight => dx > 4.0,
                            _ => false,
                        };
                        if !valid {
                            continue;
                        }
                        let dist = match key {
                            KeyCode::ArrowUp | KeyCode::ArrowDown => dy.abs() + dx.abs() * 2.0,
                            KeyCode::ArrowLeft | KeyCode::ArrowRight => dx.abs() + dy.abs() * 2.0,
                            _ => f32::MAX,
                        };
                        if best.as_ref().is_none_or(|(_, d)| dist < *d) {
                            best = Some((r.0, dist));
                        }
                    }
                    if let Some((next_id, _)) = best {
                        self.pause_focus = Some(next_id);
                    }
                } else {
                    self.pause_focus = Some(active_ids[0]);
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                if let Some(hit) = self.pause_focus {
                    self.audio.play_button_click();
                    self.execute_pause_action(hit)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// DeathScreen: gradient, the title at double size, the cause and score.
    pub(super) fn draw_death_screen(&mut self, batch: &mut HudBatch) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // fillGradient(0x60500000 -> 0xA0803030).
        let rows = h.ceil() as i32;
        for y in 0..rows {
            let t = y as f32 / h.max(1.0);
            let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
            batch.fill(
                atlas,
                0.0,
                y as f32,
                w,
                1.0,
                rgba(
                    lerp(80.0, 128.0),
                    lerp(0.0, 48.0),
                    lerp(0.0, 48.0),
                    lerp(96.0, 160.0),
                ),
            );
        }
        // DeathScreen.handleExitToTitleScreen: the TitleConfirmScreen takes the
        // screen over, on the same background.
        if let Some(opened) = self.ui.death_confirm {
            let (title, labels) = death_confirm_texts();
            // ConfirmScreen.getNarrationMessage: its title and its (empty)
            // message.
            widgets::begin_screen(|| {
                ScreenInfo::new(
                    "death/confirm",
                    widgets::join_for_narration(&[&title.plain(), ""]),
                )
            });
            let size = self.gui_size();
            let mut confirm = Confirm::new(&atlas.font, size, &title, &Text::literal(""));
            confirm.focus = self.death_focus.map(|f| f as usize);
            confirm.draw_active(
                batch,
                atlas,
                self.cursor,
                [&labels[0], &labels[1]],
                opened.elapsed().as_millis() >= 1000,
            );
            return;
        }
        let title = if self.level.hardcore {
            "deathScreen.title.hardcore"
        } else {
            "deathScreen.title"
        };
        let title = Text::translate(title, &[]);
        widgets::begin_screen(|| ScreenInfo::new("death", title.plain()));
        let tw = atlas.font.width_text(&title);
        ui::text_scaled(
            batch,
            atlas,
            &title,
            (w / 2.0 - tw).floor(),
            60.0,
            2.0,
            u32::MAX,
        );
        if let Some(cause) = &self.ui.death_text {
            let cw = atlas.font.width_text(cause);
            batch.text_styled(atlas, cause, ((w - cw) / 2.0).floor(), 85.0, u32::MAX, true);
        }
        let score = Text::translate(
            "deathScreen.score.value",
            &[Text::colored(
                self.experience.total.to_string(),
                text::YELLOW,
            )],
        );
        let sw = atlas.font.width_text(&score);
        batch.text_styled(
            atlas,
            &score,
            ((w - sw) / 2.0).floor(),
            100.0,
            u32::MAX,
            true,
        );
        let active = self
            .ui
            .died_at
            .is_none_or(|t| t.elapsed().as_millis() >= 1000);
        for (x, y, key, id) in death_buttons(w, h, self.level.hardcore) {
            let is_focused = self.death_focus == Some(id);
            let hover = active && (ui::hovered(self.cursor, x, y, 200.0, 20.0) || is_focused);
            ui::button(
                batch,
                atlas,
                x,
                y,
                200.0,
                20.0,
                &Text::translate(key, &[]),
                active,
                hover,
            );
        }
    }
    /// LocalPlayer.respawn: the player comes back (the death screen's
    /// button, or at once with `immediate_respawn`).
    pub(super) fn respawn_player(&mut self) -> Result<()> {
        self.death_focus = None;
        if self.remote.is_none() {
            // PlayerList.respawn removes the dead player, and Player.remove's
            // inventoryMenu.removed drops what the cursor held, whatever
            // keep_inventory says (the 20-tick branch handles a later respawn).
            if let Some(stack) = self.carried.take()
                && let Some(eye) = self.eye_position()
            {
                self.throw_from_eyes(stack, eye, false);
            }
            let keep = self.ui.rules.bool("keep_inventory") || self.spectator();
            self.experience.respawn(keep);
            // The guests are sent the respawned host as a new entity, not the dead body.
            if let Some(host) = self.lan.as_mut() {
                let id = host.host_entity;
                host.hub.respawned(id);
            }
            // Autosaves keep a login position; death must resolve the respawn point instead.
            self.level.position = None;
            // The dead player's Entity.setRemoved stopped its riding: the
            // new one is not put back into the vehicle.
            self.riding = None;
        }
        if self.remote.is_some() {
            self.ui.death_text = None;
            self.toggles.respawn();
            self.remote_respawn();
        } else if self.respawn_config.is_some() {
            // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
            self.toggles.respawn();
            // PlayerList.respawn at the respawn anchor, once it is loaded.
            self.begin_anchor_respawn()?;
            self.screen = Screen::Playing;
            self.ui.death_text = None;
            self.spawn_player()?;
            self.capture(true);
        } else {
            // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
            self.toggles.respawn();
            self.respawn_out_of_end()?;
            self.player = None;
            self.screen = Screen::Playing;
            self.ui.death_text = None;
            self.spawn_player()?;
            self.capture(true);
        }
        // ServerGamePacketListenerImpl.handleClientCommand: a hardcore world's
        // respawned player only watches.
        if self.remote.is_none() && self.level.hardcore {
            self.set_mode(GameMode::Spectator);
        }
        Ok(())
    }
    pub(super) fn death_click(&mut self) -> Result<()> {
        if self
            .ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(());
        }
        if let Some(opened) = self.ui.death_confirm {
            // ConfirmScreen.setDelay: both buttons wait 20 ticks.
            if opened.elapsed().as_millis() < 1000 {
                return Ok(());
            }
            let (title, _) = death_confirm_texts();
            let confirm = Confirm::new(
                &self.gui.atlas.font,
                self.gui_size(),
                &title,
                &Text::literal(""),
            );
            if let Some(yes) = confirm.answer_at(self.cursor) {
                self.death_focus = Some(if yes { 0 } else { 1 });
                self.death_confirm_answer(yes)?;
            }
            return Ok(());
        }
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let hit = death_buttons(b.width, b.height, self.level.hardcore)
            .into_iter()
            .find(|d| ui::hovered(self.cursor, d.0, d.1, 200.0, 20.0))
            .map(|d| d.3);
        if let Some(id) = hit {
            self.death_focus = Some(id);
        }
        // handleExitToTitleScreen: outside hardcore, Title Screen asks first.
        if hit == Some(1) && !self.level.hardcore {
            self.ui.death_confirm = Some(std::time::Instant::now());
            self.audio.play_button_click();
            return Ok(());
        }
        self.death_action(hit)
    }
    pub(super) fn death_key(&mut self, key: KeyCode) -> Result<()> {
        if self
            .ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(());
        }
        if let Some(opened) = self.ui.death_confirm {
            if opened.elapsed().as_millis() < 1000 {
                return Ok(());
            }
            match key {
                KeyCode::Escape => {
                    self.audio.play_button_click();
                    self.death_confirm_answer(false)?;
                }
                KeyCode::Tab => {
                    let reverse = self.has_shift_down();
                    self.death_focus = match self.death_focus {
                        Some(0) => Some(1),
                        Some(1) => Some(0),
                        _ => {
                            if reverse {
                                Some(1)
                            } else {
                                Some(0)
                            }
                        }
                    };
                }
                KeyCode::ArrowLeft => self.death_focus = Some(0),
                KeyCode::ArrowRight => self.death_focus = Some(1),
                KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                    let yes = self.death_focus != Some(1);
                    self.audio.play_button_click();
                    self.death_confirm_answer(yes)?;
                }
                _ => {}
            }
            return Ok(());
        }
        match key {
            KeyCode::Tab => {
                let reverse = self.has_shift_down();
                self.death_focus = match self.death_focus {
                    Some(0) => Some(1),
                    Some(1) => Some(0),
                    _ => {
                        if reverse {
                            Some(1)
                        } else {
                            Some(0)
                        }
                    }
                };
            }
            KeyCode::ArrowUp => self.death_focus = Some(0),
            KeyCode::ArrowDown => self.death_focus = Some(1),
            KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
                let Some(hit) = self.death_focus else {
                    return Ok(());
                };
                self.audio.play_button_click();
                if hit == 1 && !self.level.hardcore {
                    self.ui.death_confirm = Some(std::time::Instant::now());
                    self.death_focus = Some(0);
                    return Ok(());
                }
                self.death_action(Some(hit))?;
            }
            _ => {}
        }
        Ok(())
    }
    /// The confirm screen's answer: yes leaves the world, no respawns
    /// (`DeathScreen.handleExitToTitleScreen`'s callback).
    pub(super) fn death_confirm_answer(&mut self, yes: bool) -> Result<()> {
        self.ui.death_confirm = None;
        if yes {
            self.audio.play_button_click();
            self.close_world()
        } else {
            self.death_action(Some(0))
        }
    }
    /// A death screen button: 0 respawns (or spectates in hardcore), 1 leaves.
    fn death_action(&mut self, hit: Option<u8>) -> Result<()> {
        match hit {
            Some(0) => self.respawn_player()?,
            Some(1) => self.close_world()?,
            _ => {}
        }
        if hit.is_some() {
            self.audio.play_button_click();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Screen, death_buttons, pause_buttons, pause_icon_focus_ids, pause_icons};

    #[test]
    fn death_buttons_sit_on_whole_gui_pixels() {
        // DeathScreen.init in ints: a 427x253 GUI (1709x1009 at scale 4) puts
        // Respawn at (113, 63 + 72), not (113.5, 63.25 + 72).
        let at = |w, h| death_buttons(w, h, false).map(|b| (b.0, b.1));
        assert_eq!(at(427.0, 253.0), [(113.0, 135.0), (113.0, 159.0)]);
        assert_eq!(at(320.0, 334.0), [(60.0, 155.0), (60.0, 179.0)]);
    }

    #[test]
    fn menu_blur_matches_vanilla_screen_backgrounds() {
        for screen in [
            Screen::Options,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
        ] {
            assert!(screen.blurs_background(false));
            assert!(screen.blurs_background(true));
        }
        assert!(Screen::Paused.blurs_background(true));
        assert!(!Screen::Paused.blurs_background(false));
        for screen in [
            Screen::Playing,
            Screen::GameModeSwitcher,
            Screen::Chat,
            Screen::Creative,
            Screen::Inventory,
            Screen::Dead,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::Win,
        ] {
            assert!(!screen.blurs_background(true));
            assert!(!screen.blurs_background(false));
        }
    }

    #[test]
    fn pause_menu_uses_disconnect_for_servers_and_keeps_world_options() {
        for (remote, label) in [(false, "menu.returnToMenu"), (true, "menu.disconnect")] {
            let buttons = pause_buttons(427.0, 240.0, remote, false, false);
            let quit = buttons.iter().find(|b| b.4 == 2).unwrap();
            assert_eq!(quit.3, label);
            assert!(quit.5);
            assert!(buttons.iter().any(|b| b.4 == 9 && b.5));
        }
    }

    #[test]
    fn pause_icons_report_bugs_and_feedback_are_active_like_vanilla() {
        use super::{FRIENDS_ICON, icon_active};
        // PauseScreen.createPauseMenu: the bug and feedback buttons are live,
        // friends follows its list, player reporting waits for its screen.
        assert!(icon_active(0, false) && icon_active(1, false));
        assert!(icon_active(FRIENDS_ICON, true) && !icon_active(FRIENDS_ICON, false));
        assert!(!icon_active(3, true));
    }

    #[test]
    fn pause_keyboard_focus_includes_the_active_icon_buttons() {
        assert_eq!(pause_icon_focus_ids(false), [12, 13]);
        assert_eq!(pause_icon_focus_ids(true), [12, 13, 10]);
    }

    #[test]
    fn server_links_adds_a_full_width_row_above_options() {
        let with = pause_buttons(427.0, 240.0, true, true, false);
        let without = pause_buttons(427.0, 240.0, true, false, false);
        let links = with.iter().find(|b| b.4 == 11).unwrap();
        assert_eq!(links.2, 204.0);
        assert_eq!(links.3, "menu.server_links");
        assert!(links.5);
        // The icon row sits above it; Options and Disconnect move down 24.
        let icons = pause_icons(427.0, 240.0, true, false);
        let y0_icons = ((240.0f32 - 190.0) * 0.25).floor() + 98.0;
        assert_eq!(icons[0].1, y0_icons);
        assert!(links.1 > icons[0].1);
        for b in &with {
            if b.4 == 1 || b.4 == 9 || b.4 == 2 {
                let same = without.iter().find(|o| o.4 == b.4).unwrap();
                assert_eq!(b.1, same.1 + 24.0 - 6.0, "{}", b.3);
            }
        }
    }

    #[test]
    fn pause_screen_additions_takes_the_server_links_row() {
        // getCustomAdditions: one additions button in row 3, never next to the
        // links button, so the grid keeps one extra row either way.
        let with = pause_buttons(427.0, 240.0, true, true, true);
        let links = pause_buttons(427.0, 240.0, true, true, false);
        assert!(with.iter().all(|b| b.4 != 11));
        let button = with.iter().find(|b| b.4 == 14).unwrap();
        assert_eq!(
            (button.0, button.1, button.2, button.5),
            (links[3].0, links[3].1, 204.0, true)
        );
        for id in [1, 9, 2] {
            let add = with.iter().find(|b| b.4 == id).unwrap();
            let link = links.iter().find(|b| b.4 == id).unwrap();
            assert_eq!(add.1, link.1);
        }
        assert_eq!(
            pause_icons(427.0, 240.0, true, true)[0].1,
            pause_icons(427.0, 240.0, true, false)[0].1
        );
    }
}
