//! The Advancements screen (`AdvancementsScreen`, `AdvancementTab`,
//! `AdvancementWidget`): one tab per displayed root, the tree laid out by
//! `TreeNodePosition`, dragged or scrolled inside the 252x140 window, with
//! the progress tooltip of the hovered advancement.
use super::{Game, Screen, advancements::text_of};
use horizon_client::{
    advancements::{Frame, Tree},
    hud::HudBatch,
    items::ItemStack,
    text::{Style, Text},
    ui,
};
use std::{sync::Arc, time::Instant};

const INSIDE_W: i32 = 234;
const INSIDE_H: i32 = 113;
const FOOTER: f32 = 33.0;
/// The roots in the order the tabs appear.
const TAB_ORDER: [&str; 5] = [
    "minecraft:story/root",
    "minecraft:nether/root",
    "minecraft:end/root",
    "minecraft:adventure/root",
    "minecraft:husbandry/root",
];
/// `AdvancementTabType`: (kind, width, height, max).
const TAB_TYPES: [(Side, f32, f32, usize); 4] = [
    (Side::Above, 28.0, 32.0, 8),
    (Side::Below, 28.0, 32.0, 8),
    (Side::Left, 32.0, 28.0, 5),
    (Side::Right, 32.0, 28.0, 5),
];
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Above,
    Below,
    Left,
    Right,
}
fn tab_type(mut index: usize) -> Option<((Side, f32, f32, usize), usize)> {
    for t in TAB_TYPES {
        if index < t.3 {
            return Some((t, index));
        }
        index -= t.3;
    }
    None
}
/// Tab position relative to the window (`getX`/`getY`).
fn tab_pos((side, w, h, _): (Side, f32, f32, usize), i: usize) -> (f32, f32) {
    let i = i as f32;
    match side {
        Side::Above => ((w + 4.0) * i, -h + 4.0),
        Side::Below => ((w + 4.0) * i, 136.0),
        Side::Left => (-w + 4.0, h * i),
        Side::Right => (248.0, h * i),
    }
}
fn tab_sprite((side, ..): (Side, f32, f32, usize), i: usize, max: usize, selected: bool) -> String {
    let (first, middle, last) = match side {
        Side::Above => ("above_left", "above_middle", "above_right"),
        Side::Below => ("below_left", "below_middle", "below_right"),
        Side::Left => ("left_top", "left_middle", "left_bottom"),
        Side::Right => ("right_top", "right_middle", "right_bottom"),
    };
    let part = if i == 0 {
        first
    } else if i == max - 1 {
        last
    } else {
        middle
    };
    let sel = if selected { "_selected" } else { "" };
    format!("gui/sprites/advancements/tab_{part}{sel}")
}
fn icon_offset(side: Side) -> (f32, f32) {
    match side {
        Side::Above => (6.0, 9.0),
        Side::Below => (6.0, 6.0),
        Side::Left => (10.0, 5.0),
        Side::Right => (6.0, 5.0),
    }
}
fn frame_sprite(frame: Frame, obtained: bool) -> &'static str {
    match (frame, obtained) {
        (Frame::Task, true) => "gui/sprites/advancements/task_frame_obtained",
        (Frame::Task, false) => "gui/sprites/advancements/task_frame_unobtained",
        (Frame::Goal, true) => "gui/sprites/advancements/goal_frame_obtained",
        (Frame::Goal, false) => "gui/sprites/advancements/goal_frame_unobtained",
        (Frame::Challenge, true) => "gui/sprites/advancements/challenge_frame_obtained",
        (Frame::Challenge, false) => "gui/sprites/advancements/challenge_frame_unobtained",
    }
}
fn box_sprite(obtained: bool) -> &'static str {
    if obtained {
        "gui/sprites/advancements/box_obtained"
    } else {
        "gui/sprites/advancements/box_unobtained"
    }
}
/// The layout of the loaded tree, cached per tree.
struct Cache {
    tree: Arc<Tree>,
    kids: Vec<Vec<usize>>,
    /// Widget pixel position (`x * 28`, `y * 27`).
    pos: Vec<Option<(i32, i32)>>,
}
#[derive(Default)]
pub(super) struct AdvUi {
    cache: Option<Cache>,
    /// The selected tab's root advancement (kept between openings).
    tab: Option<usize>,
    /// Scroll of the selected tab; `None` until centred.
    scroll: Option<(f64, f64)>,
    fade: f32,
    hovered: Option<usize>,
    /// The cursor at the last drag step while the left button is down.
    drag: Option<(f32, f32)>,
    scrolling: bool,
    from_pause: bool,
    last_frame: Option<Instant>,
}
/// What the current frame shows: tabs, the visible widgets of the
/// selected one (tree order) and its bounds.
struct View {
    tabs: Vec<usize>,
    widgets: Vec<usize>,
    parent: Vec<Option<usize>>,
    bounds: (i32, i32, i32, i32),
}
fn window(w: f32, h: f32) -> (f32, f32) {
    (((w - 252.0) / 2.0).floor(), ((h - 140.0) / 2.0).floor())
}
impl Game<'_> {
    pub(super) fn open_advancements(&mut self, from_pause: bool) {
        self.load_advancements();
        let tab = self.adv_ui.tab;
        self.adv_ui = AdvUi {
            cache: self.adv_ui.cache.take(),
            tab,
            from_pause,
            ..Default::default()
        };
        self.screen = Screen::Advancements;
        self.capture(false);
    }
    pub(super) fn close_advancements(&mut self) {
        if self.adv_ui.from_pause {
            self.screen = Screen::Paused;
        } else {
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    fn adv_cache(&mut self) {
        if self
            .adv_ui
            .cache
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(&c.tree, &self.adv.tree))
        {
            return;
        }
        let tree = self.adv.tree.clone();
        let kids = tree.children();
        // A server's advancements come placed (`DisplayInfo` x, y).
        let pos = match &self.adv.positions {
            Some(server) => server
                .iter()
                .map(|&(x, y)| Some(((x * 28.0).floor() as i32, (y * 27.0).floor() as i32)))
                .collect(),
            None => tree
                .layout()
                .into_iter()
                .map(|p| p.map(|(x, y)| (x * 28, (y * 27.0).floor() as i32)))
                .collect(),
        };
        self.adv_ui.cache = Some(Cache { tree, kids, pos });
    }
    fn adv_done(&self, i: usize) -> bool {
        self.adv.progress.done(&self.adv.tree.list[i])
    }
    fn adv_view(&mut self) -> View {
        self.adv_cache();
        let Some(c) = self.adv_ui.cache.as_ref() else {
            return View {
                tabs: Vec::new(),
                widgets: Vec::new(),
                parent: Vec::new(),
                bounds: (0, 0, 0, 0),
            };
        };
        let tree = &c.tree;
        // A server sends only what the player may see.
        let visible = if self.adv.positions.is_some() {
            vec![true; tree.list.len()]
        } else {
            tree.visible(&c.kids, |i| self.adv.progress.done(&tree.list[i]))
        };
        // AdvancementTab.create: a displayed root with a background.
        let mut tabs: Vec<usize> = (0..tree.list.len())
            .filter(|&i| {
                let a = &tree.list[i];
                a.parent.is_none()
                    && visible[i]
                    && a.display.as_ref().is_some_and(|d| d.background.is_some())
            })
            .collect();
        tabs.sort_by_key(|&i| {
            let id = &tree.list[i].id;
            (
                TAB_ORDER
                    .iter()
                    .position(|r| r == id)
                    .unwrap_or(TAB_ORDER.len()),
                id.clone(),
            )
        });
        tabs.truncate(26);
        let tab = self
            .adv_ui
            .tab
            .filter(|t| tabs.contains(t))
            .or_else(|| tabs.first().copied());
        let mut parent = vec![None; tree.list.len()];
        let mut widgets = Vec::new();
        let mut bounds = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        if let Some(t) = tab {
            // Tree order from the root, nearest displayed ancestor as parent.
            let mut stack = vec![(t, None::<usize>)];
            while let Some((i, up)) = stack.pop() {
                let shown = visible[i] && tree.list[i].display.is_some() && c.pos[i].is_some();
                let next = if shown {
                    parent[i] = up;
                    widgets.push(i);
                    let (x, y) = c.pos[i].unwrap_or_default();
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x + 28);
                    bounds.3 = bounds.3.max(y + 27);
                    Some(i)
                } else {
                    up
                };
                for &k in c.kids[i].iter().rev() {
                    stack.push((k, next));
                }
            }
        }
        self.adv_ui.tab = tab;
        View {
            tabs,
            widgets,
            parent,
            bounds,
        }
    }
    fn adv_scroll_by(&mut self, view: &View, dx: f64, dy: f64) {
        let (min_x, min_y, max_x, max_y) = view.bounds;
        let Some((sx, sy)) = self.adv_ui.scroll.as_mut() else {
            return;
        };
        if max_x - min_x > INSIDE_W {
            *sx = (*sx + dx).clamp(f64::from(-(max_x - INSIDE_W)), 0.0);
        }
        if max_y - min_y > INSIDE_H {
            *sy = (*sy + dy).clamp(f64::from(-(max_y - INSIDE_H)), 0.0);
        }
    }
    /// `AdvancementWidget`'s title lines, description lines and width.
    fn adv_widget_text(&self, i: usize) -> (Vec<Text>, Vec<Text>, f32) {
        let font = &self.gui.atlas.font;
        let a = &self.adv.tree.list[i];
        let Some(d) = &a.display else {
            return (Vec::new(), Vec::new(), 0.0);
        };
        let title = font.split(&text_of(&d.title), 163.0);
        let title_w = title
            .iter()
            .map(|l| font.width_text(l))
            .fold(0.0f32, f32::max)
            .max(80.0);
        let n = a.requirements.len();
        let progress_w = if n <= 1 {
            0.0
        } else {
            let t = Text::translate(
                "advancements.progress",
                &[Text::literal(n.to_string()), Text::literal(n.to_string())],
            );
            font.width_text(&t) + 8.0
        };
        let mut longest = 29.0 + title_w + progress_w;
        let color = match d.frame {
            Frame::Challenge => 0xAA00AA,
            _ => 0x55FF55,
        };
        let desc = text_of(&d.description).with_style(&Style::color(color));
        // findOptimalLines.
        let mut best: Option<(f32, Vec<Text>)> = None;
        for margin in [0.0, 10.0, -10.0, 25.0, -25.0] {
            let lines = font.split(&desc, longest - margin);
            let widest = lines.iter().map(|l| font.width_text(l)).fold(0.0, f32::max);
            let distance = (widest - longest).abs();
            if distance <= 10.0 {
                best = Some((distance, lines));
                break;
            }
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, lines));
            }
        }
        let desc = best.map(|b| b.1).unwrap_or_default();
        for l in &desc {
            longest = longest.max(font.width_text(l));
        }
        (title, desc, longest + 3.0 + 5.0)
    }
    pub(super) fn draw_advancements(&mut self, batch: &mut HudBatch) {
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new(
                "advancements",
                super::screen_info::tr("gui.advancements", &[]),
            )
        });
        let (w, h) = (batch.width, batch.height);
        let view = self.adv_view();
        let (left, top) = window(w, h);
        let (ix, iy) = (left + 9.0, top + 18.0);
        // Centre a freshly opened tab.
        if self.adv_ui.scroll.is_none() && !view.widgets.is_empty() {
            let (min_x, min_y, max_x, max_y) = view.bounds;
            self.adv_ui.scroll = Some((
                f64::from(117 - (max_x + min_x) / 2),
                f64::from(56 - (max_y + min_y) / 2),
            ));
        }
        let (sx, sy) = self.adv_ui.scroll.unwrap_or_default();
        let (sx, sy) = (sx.floor() as i32, sy.floor() as i32);
        self.adv_tick_hover(&view, sx, sy, left, top);
        let atlas = &self.gui.atlas;
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let title = Text::translate("gui.advancements", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        // The inside.
        let tree = self.adv.tree.clone();
        let tab = self.adv_ui.tab;
        match tab {
            None => {
                batch.fill(atlas, ix, iy, 234.0, 113.0, 0xFF00_0000);
                let mid = ix + 117.0;
                for (key, y) in [
                    ("advancements.empty", iy + 56.0 - 4.0),
                    ("advancements.sad_label", iy + 113.0 - 9.0),
                ] {
                    let t = Text::translate(key, &[]);
                    let tw = atlas.font.width_text(&t);
                    batch.text_styled(atlas, &t, (mid - tw / 2.0).floor(), y, u32::MAX, true);
                }
            }
            Some(root) => {
                let start = batch.vertices.len();
                let bg = tree.list[root]
                    .display
                    .as_ref()
                    .and_then(|d| d.background.clone())
                    .unwrap_or_default();
                let bg = bg.strip_prefix("minecraft:").unwrap_or(&bg).to_owned();
                let (l, t) = ((sx % 16) as f32, (sy % 16) as f32);
                for x in -1..=15 {
                    for y in -1..=8 {
                        batch.sprite_scaled(
                            atlas,
                            &bg,
                            ix + l + 16.0 * x as f32,
                            iy + t + 16.0 * y as f32,
                            16.0,
                            16.0,
                            u32::MAX,
                        );
                    }
                }
                let all = self
                    .adv_ui
                    .cache
                    .as_ref()
                    .map(|c| c.pos.clone())
                    .unwrap_or_default();
                let pos = |i: usize| all.get(i).copied().flatten().unwrap_or_default();
                let (ox, oy) = (ix as i32 + sx, iy as i32 + sy);
                for background in [true, false] {
                    let col = if background { 0xFF00_0000 } else { u32::MAX };
                    let hline = |batch: &mut HudBatch, x0: i32, x1: i32, y: i32| {
                        let (a, b) = (x0.min(x1), x0.max(x1));
                        batch.fill(atlas, a as f32, y as f32, (b + 1 - a) as f32, 1.0, col);
                    };
                    let vline = |batch: &mut HudBatch, x: i32, y0: i32, y1: i32| {
                        let (a, b) = (y0.min(y1), y0.max(y1));
                        if b > a + 1 {
                            batch.fill(
                                atlas,
                                x as f32,
                                (a + 1) as f32,
                                1.0,
                                (b - a - 1) as f32,
                                col,
                            );
                        }
                    };
                    for &i in &view.widgets {
                        let Some(p) = view.parent[i] else {
                            continue;
                        };
                        let (px, py) = pos(p);
                        let (mx, my) = pos(i);
                        let dep_x = ox + px + 13;
                        let split_x = ox + px + 26 + 4;
                        let dep_y = oy + py + 13;
                        let (my_x, my_y) = (ox + mx + 13, oy + my + 13);
                        if background {
                            hline(batch, split_x, dep_x, dep_y - 1);
                            hline(batch, split_x + 1, dep_x, dep_y);
                            hline(batch, split_x, dep_x, dep_y + 1);
                            hline(batch, my_x, split_x - 1, my_y - 1);
                            hline(batch, my_x, split_x - 1, my_y);
                            hline(batch, my_x, split_x - 1, my_y + 1);
                            vline(batch, split_x - 1, my_y, dep_y);
                            vline(batch, split_x + 1, my_y, dep_y);
                        } else {
                            hline(batch, split_x, dep_x, dep_y);
                            hline(batch, my_x, split_x, my_y);
                            vline(batch, split_x, my_y, dep_y);
                        }
                    }
                }
                for &i in &view.widgets {
                    let a = &tree.list[i];
                    let Some(d) = &a.display else { continue };
                    let done = self.adv_done(i);
                    if d.hidden && !done {
                        continue;
                    }
                    let (x, y) = pos(i);
                    let (x, y) = ((ox + x) as f32, (oy + y) as f32);
                    let atlas = &self.gui.atlas;
                    batch.sprite_scaled(
                        atlas,
                        frame_sprite(d.frame, done),
                        x + 3.0,
                        y,
                        26.0,
                        26.0,
                        u32::MAX,
                    );
                    let icon = ItemStack::new(&d.icon, 1);
                    self.draw_item(batch, &icon, x + 8.0, y + 5.0);
                }
                batch.clip_since(start, ix, iy, ix + 234.0, iy + 113.0);
            }
        }
        // The window, tabs and title.
        let atlas = &self.gui.atlas;
        batch.sprite_region(
            atlas,
            "gui/advancements/window",
            [left, top, 252.0, 140.0],
            256.0,
            256.0,
        );
        if view.tabs.len() > 1 {
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let sprite = tab_sprite(ty, i, ty.3, Some(t) == tab);
                batch.sprite_scaled(
                    &self.gui.atlas,
                    &sprite,
                    left + x,
                    top + y,
                    ty.1,
                    ty.2,
                    u32::MAX,
                );
            }
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let (dx, dy) = icon_offset(ty.0);
                if let Some(d) = &tree.list[t].display {
                    let icon = ItemStack::new(&d.icon, 1);
                    self.draw_item(batch, &icon, left + x + dx, top + y + dy);
                }
            }
        }
        let atlas = &self.gui.atlas;
        let title = tab.and_then(|t| tree.list[t].display.as_ref()).map_or_else(
            || Text::translate("gui.advancements", &[]),
            |d| text_of(&d.title),
        );
        batch.text_styled(atlas, &title, left + 8.0, top + 6.0, 0xFF40_4040, false);
        // Tooltips.
        if tab.is_some() {
            let alpha = (self.adv_ui.fade * 255.0).floor() as u32;
            batch.fill(atlas, ix, iy, 234.0, 113.0, alpha << 24);
            if let Some(i) = self.adv_ui.hovered {
                self.adv_draw_hover(batch, i, ix, iy, sx, sy, left, w);
            }
        }
        // Done.
        let atlas = &self.gui.atlas;
        let (bx, by) = ((w / 2.0 - 100.0).floor(), h - FOOTER + 6.0);
        let hover = ui::hovered(self.cursor, bx, by, 200.0, 20.0);
        ui::button(
            batch,
            atlas,
            bx,
            by,
            200.0,
            20.0,
            &Text::translate("gui.done", &[]),
            true,
            hover,
        );
        // Tab tooltips: the tab title under the cursor.
        if view.tabs.len() > 1 {
            for (n, &t) in view.tabs.iter().enumerate() {
                let Some((ty, i)) = tab_type(n) else { continue };
                let (x, y) = tab_pos(ty, i);
                let (x, y) = (left + x, top + y);
                let (cx, cy) = self.cursor;
                if cx > x && cx < x + ty.1 && cy > y && cy < y + ty.2 {
                    if let Some(d) = &tree.list[t].display {
                        ui::tooltip(batch, &self.gui.atlas, &[text_of(&d.title)], self.cursor);
                    }
                    break;
                }
            }
        }
    }
    /// `AdvancementTab.tick`, run per frame at the tick rate's pace.
    fn adv_tick_hover(&mut self, view: &View, sx: i32, sy: i32, left: f32, top: f32) {
        let now = Instant::now();
        let ticks = self
            .adv_ui
            .last_frame
            .map_or(0.0, |t| (now - t).as_secs_f32() * 20.0)
            .min(5.0);
        self.adv_ui.last_frame = Some(now);
        let mx = (self.cursor.0 - left - 9.0).floor() as i32;
        let my = (self.cursor.1 - top - 18.0).floor() as i32;
        let mut hovered = None;
        if mx > 0 && mx < INSIDE_W && my > 0 && my < INSIDE_H {
            for &i in &view.widgets {
                let hidden = self.adv.tree.list[i]
                    .display
                    .as_ref()
                    .is_some_and(|d| d.hidden);
                if hidden && !self.adv_done(i) {
                    continue;
                }
                let Some((x, y)) = self.adv_ui.cache.as_ref().and_then(|c| c.pos[i]) else {
                    continue;
                };
                let (x0, y0) = (sx + x, sy + y);
                if mx >= x0 && mx <= x0 + 26 && my >= y0 && my <= y0 + 26 {
                    hovered = Some(i);
                    break;
                }
            }
        }
        if hovered.is_some() {
            self.adv_ui.hovered = hovered;
            self.adv_ui.fade = (self.adv_ui.fade + 0.06 * ticks).clamp(0.0, 0.3);
        } else {
            self.adv_ui.hovered = None;
            self.adv_ui.fade = (self.adv_ui.fade - 0.12 * ticks).clamp(0.0, 1.0);
        }
    }
    /// `AdvancementWidget.extractHover`, in GUI pixels.
    #[allow(clippy::too_many_arguments)]
    fn adv_draw_hover(
        &mut self,
        batch: &mut HudBatch,
        i: usize,
        ix: f32,
        iy: f32,
        sx: i32,
        sy: i32,
        left: f32,
        screen_w: f32,
    ) {
        let (title, desc, width) = self.adv_widget_text(i);
        let tree = self.adv.tree.clone();
        let a = &tree.list[i];
        let Some(d) = &a.display else { return };
        let Some((wx, wy)) = self.adv_ui.cache.as_ref().and_then(|c| c.pos[i]) else {
            return;
        };
        let width = width as i32;
        let (x, y) = (sx + wx, sy + wy);
        let title_bar_h = 9 * title.len() as i32 + 9 + 8;
        let title_top = y + (26 - title_bar_h) / 2;
        let title_bottom = title_top + title_bar_h;
        let desc_text_h = desc.len() as i32 * 9;
        let desc_h = 6 + desc_text_h;
        let left_side = left as i32 + x + width + 26 >= screen_w as i32;
        let (got, total) = self.adv.progress.counts(a);
        let progress = (total > 1 && !a.criteria.is_empty()).then(|| {
            Text::translate(
                "advancements.progress",
                &[
                    Text::literal(got.to_string()),
                    Text::literal(total.to_string()),
                ],
            )
        });
        let atlas = &self.gui.atlas;
        let progress_w = progress.as_ref().map_or(0.0, |t| atlas.font.width_text(t)) as i32;
        let top_side = title_bottom + desc_h >= INSIDE_H;
        let amount = if a.criteria.is_empty() || total == 0 {
            0.0
        } else {
            got as f32 / total as f32
        };
        let mut first_w = (amount * width as f32).floor() as i32;
        let (first, second, icon_frame) = if amount >= 1.0 {
            first_w = width / 2;
            (true, true, true)
        } else if first_w < 2 {
            first_w = width / 2;
            (false, false, false)
        } else if first_w > width - 2 {
            first_w = width / 2;
            (true, true, false)
        } else {
            (true, false, false)
        };
        let second_w = width - first_w;
        let title_left = if left_side { x - width + 26 + 6 } else { x };
        let bg_h = title_bar_h + desc_h;
        let f = |v: i32| v as f32;
        if !desc.is_empty() {
            let by = if top_side {
                title_bottom - bg_h
            } else {
                title_top
            };
            ui::nine_slice(
                batch,
                atlas,
                "gui/sprites/advancements/title_box",
                ix + f(title_left),
                iy + f(by),
                f(width),
                f(bg_h),
                u32::MAX,
            );
        }
        if first != second {
            for (sprite, src_x, at, w) in [
                (box_sprite(first), 0, title_left, first_w),
                (
                    box_sprite(second),
                    200 - second_w,
                    title_left + first_w,
                    second_w,
                ),
            ] {
                let start = batch.vertices.len();
                ui::nine_slice(
                    batch,
                    atlas,
                    sprite,
                    ix + f(at - src_x),
                    iy + f(title_top),
                    200.0,
                    f(title_bar_h),
                    u32::MAX,
                );
                batch.clip_since(
                    start,
                    ix + f(at),
                    iy + f(title_top),
                    ix + f(at + w),
                    iy + f(title_bottom),
                );
            }
        } else {
            ui::nine_slice(
                batch,
                atlas,
                box_sprite(first),
                ix + f(title_left),
                iy + f(title_top),
                f(width),
                f(title_bar_h),
                u32::MAX,
            );
        }
        batch.sprite_scaled(
            atlas,
            frame_sprite(d.frame, icon_frame),
            ix + f(x + 3),
            iy + f(y),
            26.0,
            26.0,
            u32::MAX,
        );
        let desc_left = title_left + 5;
        let lines = |batch: &mut HudBatch, lines: &[Text], x: i32, y: i32, color: u32| {
            for (n, l) in lines.iter().enumerate() {
                batch.text_styled(atlas, l, ix + f(x), iy + f(y + n as i32 * 9), color, true);
            }
        };
        let (title_x, progress_x) = if left_side {
            (desc_left, x - progress_w)
        } else {
            (x + 32, x + width - progress_w - 5)
        };
        lines(batch, &title, title_x, title_top + 9, u32::MAX);
        if let Some(p) = &progress {
            batch.text_styled(
                atlas,
                p,
                ix + f(progress_x),
                iy + f(title_top + 9),
                u32::MAX,
                true,
            );
        }
        let desc_y = if top_side {
            title_top - desc_text_h + 1
        } else {
            title_bottom
        };
        lines(batch, &desc, desc_left, desc_y, 0xFF00_FF00);
        let icon = ItemStack::new(&d.icon, 1);
        self.draw_item(batch, &icon, ix + f(x + 8), iy + f(y + 5));
    }
    /// A left button press or release.
    pub(super) fn advancements_mouse(&mut self, pressed: bool, size: [u32; 2]) {
        if !pressed {
            self.adv_ui.drag = None;
            self.adv_ui.scrolling = false;
            return;
        }
        let b = HudBatch::new(size);
        let (w, h) = (b.width, b.height);
        if ui::hovered(
            self.cursor,
            (w / 2.0 - 100.0).floor(),
            h - FOOTER + 6.0,
            200.0,
            20.0,
        ) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.close_advancements();
            return;
        }
        let view = self.adv_view();
        let (left, top) = window(w, h);
        for (n, &t) in view.tabs.iter().enumerate() {
            let Some((ty, i)) = tab_type(n) else { continue };
            let (x, y) = tab_pos(ty, i);
            let (x, y) = (left + x, top + y);
            let (cx, cy) = self.cursor;
            if cx > x && cx < x + ty.1 && cy > y && cy < y + ty.2 {
                if self.adv_ui.tab != Some(t) {
                    self.adv_ui.tab = Some(t);
                    self.adv_ui.scroll = None;
                }
                return;
            }
        }
        self.adv_ui.drag = Some(self.cursor);
    }
    /// `mouseDragged` with the left button: the first step starts
    /// scrolling, later ones move the tree.
    pub(super) fn advancements_drag(&mut self) {
        let Some(last) = self.adv_ui.drag else {
            return;
        };
        let (dx, dy) = (self.cursor.0 - last.0, self.cursor.1 - last.1);
        self.adv_ui.drag = Some(self.cursor);
        if !self.adv_ui.scrolling {
            self.adv_ui.scrolling = true;
            return;
        }
        let view = self.adv_view();
        self.adv_scroll_by(&view, f64::from(dx), f64::from(dy));
    }
    pub(super) fn advancements_scroll(&mut self, x: f32, y: f32) {
        let view = self.adv_view();
        self.adv_scroll_by(&view, f64::from(x) * 16.0, f64::from(y) * 16.0);
    }
}
