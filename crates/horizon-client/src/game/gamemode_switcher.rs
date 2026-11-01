//! The non-pausing F3+F4 screen (`GameModeSwitcherScreen`).
use super::{Game, ItemStack, Screen};
use horizon_client::{
    game_mode_switcher::{GameModeSwitcher, ICONS, MODES, slot_origin},
    hud::HudBatch,
    keybinds,
    text::Text,
};

impl Game<'_> {
    pub(super) fn open_gamemode_switcher(&mut self) {
        self.gamemode_switcher = Some(GameModeSwitcher::new(self.mode, self.previous_mode));
        self.screen = Screen::GameModeSwitcher;
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.mining = None;
        self.capture(false);
    }

    pub(super) fn close_gamemode_switcher(&mut self, apply: bool) {
        let selected = self.gamemode_switcher.take().map(|s| s.selected);
        self.screen = Screen::Playing;
        self.capture(true);
        if apply
            && self.player.is_some()
            && self.gamemaster()
            && let Some(mode) = selected
            && mode != self.mode
        {
            self.change_game_mode(mode);
        }
    }

    pub(super) fn draw_gamemode_switcher(&mut self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let Some(switcher) = &mut self.gamemode_switcher else {
            return;
        };
        // GameModeSwitcherScreen: `GameNarrator.NO_TITLE`; its slots are no
        // widgets, so it says how to navigate.
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("gamemode_switcher", "")
        });
        switcher.hover(w, h, (self.cursor.0 as i32, self.cursor.1 as i32));
        let selected = switcher.selected;
        let atlas = &self.gui.atlas;
        batch.sprite_region(
            atlas,
            "gui/container/gamemode_switcher",
            [(w / 2 - 62) as f32, (h / 2 - 58) as f32, 125.0, 75.0],
            128.0,
            128.0,
        );
        let title = Text::translate(&format!("gameMode.{}", selected.name()), &[]);
        let help = Text::translate(
            "debug.gamemodes.select_next",
            &[
                keybinds::display_name(self.settings.keys.get("key.debug.switchGameMode"))
                    .with_color(0x55FFFF),
            ],
        );
        for (text, y) in [(title, h / 2 - 51), (help, h / 2 + 5)] {
            let x = w / 2 - (HudBatch::text_width(atlas, &text.plain()) / 2.0) as i32;
            batch.text_styled(atlas, &text, x as f32, y as f32, u32::MAX, true);
        }
        for (index, mode) in MODES.into_iter().enumerate() {
            let (x, y) = slot_origin(w, h, index);
            batch.sprite(
                &self.gui.atlas,
                "gui/sprites/gamemode_switcher/slot",
                x as f32,
                y as f32,
            );
            if mode == selected {
                batch.sprite(
                    &self.gui.atlas,
                    "gui/sprites/gamemode_switcher/selection",
                    x as f32,
                    y as f32,
                );
            }
            self.draw_item_model(
                batch,
                &ItemStack::new(ICONS[index], 1),
                (x + 5) as f32,
                (y + 5) as f32,
            );
        }
    }
}
