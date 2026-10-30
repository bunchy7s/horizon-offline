//! Titles, the action bar and the selected item name: the state and layout
//! live in `hud_overlays` (`Titles`, `ToolHighlight`); this feeds and draws them.
use super::Game;
pub(super) use horizon_client::hud_overlays::Titles;
use horizon_client::{
    hud::HudBatch,
    hud_layout,
    hud_overlays::HeldName,
    item_tooltip,
    text::{self, Text},
};

impl Game<'_> {
    /// `Hud.extractOverlayMessage` and `extractTitle`.
    pub(super) fn draw_titles(&self, batch: &mut HudBatch, bar_height: i32) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.clock.fraction() as f32;
        let background = self.settings.text_background(0.0);
        let font = &self.gui.atlas.font;
        let mut draws = self
            .titles
            .overlay_message(w, h, bar_height, partial, font, background);
        draws.extend(self.titles.title(w, h, partial, font, background));
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// `Hud.extractSavingIndicator`, under the screens: the option, then the
    /// fading "Saving world" text.
    pub(super) fn draw_saving_indicator(&self, batch: &mut HudBatch) {
        if !self.settings.autosave_indicator {
            return;
        }
        let draws = self.saving_indicator.draw(
            batch.width as i32,
            batch.height as i32,
            self.clock.fraction() as f32,
            &self.gui.atlas.font,
            Text::literal(self.translate("menu.savingLevel", "Saving world")),
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    /// The selected stack as `Hud.tick` compares it for the name highlight.
    fn held_name(&self) -> Option<HeldName> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        let custom = stack.data().is_some_and(|data| data.custom_name.is_some());
        let default_rarity = self.item_rarity(&stack.item);
        let rarity = item_tooltip::rarity(stack, &default_rarity);
        Some(HeldName {
            item: stack.item.to_string(),
            hover_name: item_tooltip::styled_name(stack, || {
                item_tooltip::name_line(stack, &|key| self.pack.names.display_get(key))
            }),
            rarity_color: match rarity {
                "uncommon" => text::YELLOW,
                "rare" => text::AQUA,
                "epic" => text::LIGHT_PURPLE,
                _ => text::WHITE,
            },
            custom_name: custom,
        })
    }
    /// The title, action bar and selected-item parts of `Hud.tick`.
    pub(super) fn tick_hud(&mut self) {
        self.titles.tick();
        self.tick_in_bed_chat();
        // `MinecraftServer.isCurrentlySaving` of single player's server.
        let saving = self
            .integrated
            .as_ref()
            .map_or(self.autosaving, |server| server.is_saving());
        self.saving_indicator.tick(saving);
        let held = self.held_name();
        self.tool_highlight
            .tick(held, f64::from(self.settings.notification_display_time));
    }
    /// `Hud.extractSelectedItemName`.
    pub(super) fn draw_selected_name(&self, batch: &mut HudBatch, bar_height: i32, can_hurt: bool) {
        let draws = self.tool_highlight.draw(
            batch.width as i32,
            batch.height as i32,
            bar_height,
            can_hurt,
            &self.gui.atlas.font,
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}
