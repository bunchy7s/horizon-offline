//! The loom screen (`LoomScreen`): the scrolling pattern list, the banner
//! preview and the six-layer error. The menu itself (`LoomMenu`: slots,
//! selectable patterns, the pattern buttons, quick moves, the result) runs on
//! the verified engine, `horizon_client::menu`, through `Game::menu_session`.
use super::{Game, stations::Kind};
use horizon_client::{
    banner::{self, BannerLayer},
    hud::{HudBatch, Rect, rgba},
    items::ItemStack,
    screen_layout::{self, LoomView, ScreenDraw},
};
use std::sync::Arc;

/// Per-menu loom state: `selectablePatterns`, the selected index and the
/// screen's scroller.
#[derive(Default)]
pub(super) struct LoomState {
    pub(super) patterns: Vec<Arc<str>>,
    pub(super) selected: Option<usize>,
    /// `displayPatterns`: the pattern buttons are shown.
    pub(super) display: bool,
    pub(super) has_max: bool,
    pub(super) scroll: f32,
    pub(super) start_row: usize,
    pub(super) scrolling: bool,
    /// The inputs the screen last saw (`bannerStack`, `dyeStack`, `patternStack`).
    last_inputs: [Option<ItemStack>; 3],
}
impl LoomState {
    fn total_rows(&self) -> usize {
        self.patterns.len().div_ceil(4)
    }
    fn offscreen_rows(&self) -> i32 {
        self.total_rows() as i32 - 4
    }
    /// `startRow` for a scroller position.
    fn row_for(&self, offs: f32) -> usize {
        ((offs * self.offscreen_rows() as f32 + 0.5) as i32).max(0) as usize
    }
}

const BUTTON: f32 = 14.0;

fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
impl Game<'_> {
    /// `LoomScreen.containerChanged`: the error flag, whether the pattern
    /// buttons show (only when an input changed), and the scroller reset.
    pub(super) fn loom_screen_changed(&mut self) {
        let Some(s) = self.station.as_mut().filter(|s| s.kind == Kind::Loom) else {
            return;
        };
        let inputs = [
            s.inputs[0].clone(),
            s.inputs[1].clone(),
            s.inputs[2].clone(),
        ];
        let l = &mut s.loom;
        l.has_max = inputs[0].as_ref().is_some_and(banner::has_max_layers);
        if inputs != l.last_inputs {
            l.display =
                inputs[0].is_some() && inputs[1].is_some() && !l.has_max && !l.patterns.is_empty();
        }
        if l.start_row >= l.total_rows() {
            l.start_row = 0;
            l.scroll = 0.0;
        }
        l.last_inputs = inputs;
    }
    /// A sound the loom or stonecutter menu played: the result's once per
    /// tick (`lastSoundTime`).
    pub(super) fn station_sound(&mut self, sound: &str) {
        let ticks = self.ticks;
        let Some(s) = self.station.as_mut() else {
            return;
        };
        if s.last_sound == ticks {
            return;
        }
        s.last_sound = ticks;
        let pos = s.pos;
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio.play(sound, Some(at), 1.0, 1.0);
    }
    /// `LoomScreen.mouseClicked` on a pattern button: `clickMenuButton`, and
    /// the select sound when the menu took it.
    fn loom_button(&mut self, index: usize) -> bool {
        let mut taken = false;
        let flag = &mut taken;
        self.menu_session(|menu, env, _| {
            *flag = menu.button(env, index as i32);
            Some(*flag)
        });
        if taken {
            self.audio
                .play("minecraft:ui.loom.select_pattern", None, 1.0, 1.0);
            self.remote_station_button(index as i32);
        }
        taken
    }
    /// A left click inside the loom's pattern grid or scroller.
    pub(super) fn loom_click(&mut self, local: (f32, f32)) -> bool {
        let Some(start_row) = self
            .station
            .as_ref()
            .filter(|s| s.loom.display)
            .map(|s| s.loom.start_row)
        else {
            return false;
        };
        for row in 0..4 {
            for column in 0..4 {
                let index = (row + start_row) * 4 + column;
                let (x, y) = (60.0 + column as f32 * BUTTON, 13.0 + row as f32 * BUTTON);
                if inside(local, x, y, BUTTON, BUTTON) && self.loom_button(index) {
                    return true;
                }
            }
        }
        if inside(local, 119.0, 9.0, 12.0, 56.0)
            && let Some(s) = self.station.as_mut()
        {
            s.loom.scrolling = true;
            return true;
        }
        false
    }
    /// The pattern buttons a controller can focus (GUI origin relative).
    pub(super) fn loom_targets(&self) -> Vec<[f32; 4]> {
        let Some(loom) = self.station.as_ref().map(|s| &s.loom).filter(|l| l.display) else {
            return Vec::new();
        };
        (0..16)
            .filter(|i| (i / 4 + loom.start_row) * 4 + i % 4 < loom.patterns.len())
            .map(|i| {
                let (row, column) = (i / 4, i % 4);
                [
                    60.0 + column as f32 * BUTTON,
                    13.0 + row as f32 * BUTTON,
                    BUTTON,
                    BUTTON,
                ]
            })
            .collect()
    }
    /// `LoomScreen.mouseDragged` on the scroller.
    pub(super) fn loom_drag(&mut self) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let (_, oy) = self.inventory_origin(&HudBatch::new(size));
        let cursor_y = self.cursor.1;
        let Some(l) = self.station.as_mut().map(|s| &mut s.loom) else {
            return;
        };
        if l.scrolling && l.display && l.offscreen_rows() > 0 {
            let top = oy + 13.0;
            l.scroll = ((cursor_y - top - 7.5) / (56.0 - 15.0)).clamp(0.0, 1.0);
            l.start_row = l.row_for(l.scroll);
        }
    }
    pub(super) fn loom_release(&mut self) {
        if let Some(s) = self.station.as_mut() {
            s.loom.scrolling = false;
        }
    }
    /// `LoomScreen.mouseScrolled`.
    pub(super) fn loom_scroll(&mut self, d: f32) {
        let Some(l) = self.station.as_mut().map(|s| &mut s.loom) else {
            return;
        };
        let offscreen = l.offscreen_rows();
        if l.display && offscreen > 0 {
            l.scroll = (l.scroll - d / offscreen as f32).clamp(0.0, 1.0);
            l.start_row = l.row_for(l.scroll);
        }
    }
    /// `GuiGraphicsExtractor.bannerPattern`: the banner flag model with its
    /// layers in a 20x40 picture in picture at `(x, y)`.
    fn draw_banner_result(
        &self,
        batch: &mut HudBatch,
        x: f32,
        y: f32,
        base: &str,
        layers: &[BannerLayer],
    ) {
        let banners = &self.stations().banners;
        let layers: Vec<(String, String)> = layers
            .iter()
            .map(|l| (banners.banner_texture(&l.pattern), l.color.to_string()))
            .collect();
        horizon_client::item_render::gui::draw_banner_result(
            &self.pack.item_renderer,
            &self.gui.atlas.sprites,
            batch,
            x,
            y,
            base,
            &layers,
        );
    }
    /// Paints `screen_layout` calls at the menu origin (ox, oy): the container
    /// background and the dimmed world are drawn by the caller.
    pub(super) fn paint_screen(
        &self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        draws: &[ScreenDraw],
        background: bool,
    ) -> Option<Vec<(horizon_client::text::Text, u32)>> {
        let atlas = &self.gui.atlas;
        let mut tooltip = None;
        let mut sheet: Option<String> = None;
        for d in draws {
            match d {
                ScreenDraw::Gradient { .. } => {}
                ScreenDraw::Blit {
                    texture,
                    x,
                    y,
                    w,
                    h,
                    tw,
                    th,
                } => {
                    if background {
                        let name = texture
                            .trim_start_matches("textures/")
                            .trim_end_matches(".png");
                        batch.sprite_region(
                            atlas,
                            name,
                            [ox + *x as f32, oy + *y as f32, *w as f32, *h as f32],
                            *tw as f32,
                            *th as f32,
                        );
                    }
                }
                ScreenDraw::Sprite { id, x, y, .. } => batch.sprite(
                    atlas,
                    &format!("gui/sprites/{id}"),
                    ox + *x as f32,
                    oy + *y as f32,
                ),
                ScreenDraw::Banner {
                    base,
                    layers,
                    x0,
                    y0,
                    ..
                } => self.draw_banner_result(batch, ox + *x0 as f32, oy + *y0 as f32, base, layers),
                ScreenDraw::Tooltip { key } => {
                    tooltip = Some(vec![(
                        horizon_client::text::Text::literal(self.translate(key, key)),
                        u32::MAX,
                    )]);
                }
                ScreenDraw::SpriteOf { id } => sheet = Some(id.clone()),
                ScreenDraw::Fill {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                } => {
                    let c = *color as u32;
                    batch.fill(
                        atlas,
                        ox + *x0 as f32,
                        oy + *y0 as f32,
                        (x1 - x0) as f32,
                        (y1 - y0) as f32,
                        rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8),
                    );
                }
                ScreenDraw::Atlas { x0, y0, x1, y1, uv } => {
                    if let Some(r) = sheet.as_ref().and_then(|n| atlas.sprites.get(n)).copied() {
                        let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                        let part = Rect {
                            u0: r.u0 + uv[0] * du,
                            u1: r.u0 + uv[1] * du,
                            v0: r.v0 + uv[2] * dv,
                            v1: r.v0 + uv[3] * dv,
                            w: (x1 - x0) as u32,
                            h: (y1 - y0) as u32,
                        };
                        batch.rect_uv(
                            ox + *x0 as f32,
                            oy + *y0 as f32,
                            (x1 - x0) as f32,
                            (y1 - y0) as f32,
                            part,
                            u32::MAX,
                        );
                    }
                }
            }
        }
        tooltip
    }
    /// `LoomScreen.extractBackground` after the background (the verified
    /// `screen_layout::loom`); returns the hovered pattern's tooltip.
    pub(super) fn draw_loom(
        &mut self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        local: (f32, f32),
    ) -> Option<Vec<(horizon_client::text::Text, u32)>> {
        let s = self.station.as_ref()?;
        let l = &s.loom;
        let dye = s.inputs[1]
            .as_ref()
            .and_then(|d| self.items().get(&d.item))
            .and_then(|i| i.dye.clone())
            .unwrap_or_else(|| "white".to_owned());
        let result = s
            .result
            .as_ref()
            .and_then(|r| Some((banner::banner_color(&r.item)?, banner::layers(r))));
        let view = LoomView {
            width: 176,
            height: 166,
            left: 0,
            top: 0,
            has_banner: s.inputs[0].is_some(),
            has_dye: s.inputs[1].is_some(),
            has_pattern: s.inputs[2].is_some(),
            patterns: &l.patterns,
            selected: l.selected.map_or(-1, |i| i as i32),
            display: l.display,
            has_max: l.has_max,
            scroll: l.scroll,
            start_row: l.start_row,
            result,
            dye: &dye,
            mouse: local,
        };
        let draws = screen_layout::loom(&view, &self.stations().banners);
        self.paint_screen(batch, ox, oy, &draws, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroller_rows_follow_the_offset() {
        let l = LoomState {
            patterns: (0..40)
                .map(|i| Arc::from(format!("p{i}").as_str()))
                .collect(),
            display: true,
            ..Default::default()
        };
        // 40 patterns = 10 rows, 6 off screen.
        assert_eq!(l.offscreen_rows(), 6);
        assert_eq!(l.row_for(0.0), 0);
        assert_eq!(l.row_for(1.0), 6);
        assert_eq!(l.row_for(0.5), 3);
    }
    #[test]
    fn few_patterns_do_not_scroll() {
        let l = LoomState {
            patterns: (0..16)
                .map(|i| Arc::from(format!("p{i}").as_str()))
                .collect(),
            display: true,
            ..Default::default()
        };
        assert_eq!(l.offscreen_rows(), 0);
    }
}
