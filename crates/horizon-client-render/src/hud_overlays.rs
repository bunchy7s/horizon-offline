//! The `Hud` overlays laid out as [`GuiDraw`] calls like
//! [`crate::hud_layout`]: status effect icons (`extractEffects`), titles and
//! the action bar (`extractTitle`, `extractOverlayMessage` and the title
//! timers of `Hud.tick`), the selected item name (`extractSelectedItemName`
//! and its `toolHighlightTimer`), boss bars (`BossHealthOverlay`), the jump
//! bar (`JumpableVehicleBar`), the scoreboard sidebar, item decorations
//! (`GuiGraphicsExtractor.itemDecorations`), the camera overlays, the
//! sleep overlay and the subtitles (`SubtitleOverlay`).
use crate::{
    font::Font,
    hud_layout::{
        Blend, GuiDraw, PoseOp, color_from_float, fill, hsv_to_argb, multiply, white, width,
    },
    text::{Style, Text},
};
use std::{borrow::Cow, cmp::Ordering};

/// `graphics.text`: dropped when fully transparent.
fn text(out: &mut Vec<GuiDraw>, text: Text, x: i32, y: i32, color: u32, shadow: bool) {
    if color >> 24 != 0 {
        out.push(GuiDraw::Text {
            text,
            x,
            y,
            color,
            shadow,
        });
    }
}

/// `graphics.textWithBackdrop`; `background` is `Options.getBackgroundColor(0.0F)`.
pub fn text_with_backdrop(
    out: &mut Vec<GuiDraw>,
    t: Text,
    x: i32,
    y: i32,
    w: i32,
    color: u32,
    background: u32,
) {
    if background != 0 {
        out.push(fill(
            x - 2,
            y - 2,
            x + w + 2,
            y + 9 + 2,
            multiply(background, color),
        ));
    }
    text(out, t, x, y, color, true);
}

/// `Arrays.sort(T[], Comparator)` for the short arrays the HUD sorts: Java's
/// TimSort below `MIN_MERGE` (a natural run, then binary insertion), which
/// matters when the comparator is not a total order. Longer arrays fall back
/// to a stable sort.
pub fn java_sort<T>(a: &mut [T], cmp: impl Fn(&T, &T) -> Ordering) {
    let n = a.len();
    if n < 2 {
        return;
    }
    if n >= 32 {
        a.sort_by(cmp);
        return;
    }
    // countRunAndMakeAscending.
    let mut run = 2;
    if cmp(&a[1], &a[0]) == Ordering::Less {
        while run < n && cmp(&a[run], &a[run - 1]) == Ordering::Less {
            run += 1;
        }
        a[..run].reverse();
    } else {
        while run < n && cmp(&a[run], &a[run - 1]) != Ordering::Less {
            run += 1;
        }
    }
    // binarySort.
    for start in run..n {
        let (mut left, mut right) = (0, start);
        while left < right {
            let mid = (left + right) / 2;
            if cmp(&a[start], &a[mid]) == Ordering::Less {
                right = mid;
            } else {
                left = mid + 1;
            }
        }
        a[left..=start].rotate_right(1);
    }
}

/// A `MobEffectInstance` as `extractEffects` sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectIcon {
    /// The effect id without namespace (`speed`): the sprite is `mob_effect/<id>`.
    pub id: String,
    /// `MobEffect.isBeneficial`.
    pub beneficial: bool,
    /// `MobEffect.getColor`.
    pub color: i32,
    pub ambient: bool,
    /// Remaining ticks, -1 for infinite.
    pub duration: i32,
    pub show_icon: bool,
}

impl From<&crate::effects::EffectInstance> for EffectIcon {
    fn from(effect: &crate::effects::EffectInstance) -> Self {
        let def = horizon_client_core::potions::effect(effect.id);
        Self {
            id: effect.id.to_owned(),
            beneficial: def
                .is_some_and(|d| d.category == horizon_client_core::potions::Category::Beneficial),
            color: def.map_or(0, |d| d.color as i32),
            ambient: effect.ambient,
            duration: effect.duration,
            // Horizon keeps no showIcon flag: every effect shows its icon.
            show_icon: true,
        }
    }
}

/// `MobEffectInstance.compareTo`.
pub(crate) fn compare_effects(a: &EffectIcon, o: &EffectIcon) -> Ordering {
    if (a.duration <= 32147 || o.duration <= 32147) && !(a.ambient && o.ambient) {
        a.ambient
            .cmp(&o.ambient)
            .then((a.duration == -1).cmp(&(o.duration == -1)))
            .then(a.duration.cmp(&o.duration))
            .then(a.color.cmp(&o.color))
    } else {
        a.ambient.cmp(&o.ambient).then(a.color.cmp(&o.color))
    }
}

/// `Hud.extractEffects`: `effects` in `getActiveEffects` order; `demo` moves
/// the icons below the demo timer.
pub fn effects(w: i32, demo: bool, effects: &[EffectIcon]) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let mut sorted: Vec<&EffectIcon> = effects.iter().collect();
    // Ordering.natural().reverse().sortedCopy.
    java_sort(&mut sorted, |a, b| compare_effects(b, a));
    let (mut beneficial, mut harmful) = (0, 0);
    for e in sorted.into_iter().filter(|e| e.show_icon) {
        let mut x = w;
        let mut y = 1 + if demo { 15 } else { 0 };
        if e.beneficial {
            beneficial += 1;
            x -= 25 * beneficial;
        } else {
            harmful += 1;
            x -= 25 * harmful;
            y += 26;
        }
        let mut alpha = 1.0f32;
        let background = if e.ambient {
            "hud/effect_background_ambient"
        } else {
            if e.duration != -1 && e.duration <= 200 {
                let remaining = e.duration;
                let used = 10 - remaining / 20;
                alpha = (remaining as f32 / 10.0 / 5.0 * 0.5).clamp(0.0, 0.5)
                    + horizon_core::geometry::trig::cos(f64::from(
                        remaining as f32 * std::f32::consts::PI / 5.0,
                    )) * (used as f32 / 10.0 * 0.25).clamp(0.0, 0.25);
                alpha = alpha.clamp(0.0, 1.0);
            }
            "hud/effect_background"
        };
        out.push(GuiDraw::Sprite {
            sprite: Cow::Borrowed(background),
            x,
            y,
            w: 24,
            h: 24,
            region: None,
            color: crate::hud_layout::WHITE,
            blend: crate::hud_layout::Blend::Textured,
        });
        out.push(GuiDraw::Sprite {
            sprite: Cow::Owned(format!("mob_effect/{}", e.id)),
            x: x + 3,
            y: y + 3,
            w: 18,
            h: 18,
            region: None,
            color: white(alpha),
            blend: crate::hud_layout::Blend::Textured,
        });
    }
    out
}

/// The saving indicator: `Hud.tickAutosaveIndicator` fades a value towards 1
/// while the server is saving and `extractSavingIndicator` draws the text.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SavingIndicator {
    last: f32,
    value: f32,
}
impl SavingIndicator {
    /// `Hud.tickAutosaveIndicator`: `Mth.lerp(0.2F, value, saving ? 1 : 0)`.
    pub fn tick(&mut self, saving: bool) {
        self.last = self.value;
        self.value += (if saving { 1.0 } else { 0.0 } - self.value) * 0.2;
    }
    /// `Hud.extractSavingIndicator` without the option check: the text in the
    /// bottom right corner.
    pub fn draw(
        &self,
        w: i32,
        h: i32,
        partial: f32,
        font: &Font,
        label: Text,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        if self.value <= 0.0 && self.last <= 0.0 {
            return out;
        }
        let fade = (self.last + (self.value - self.last) * partial).clamp(0.0, 1.0);
        let alpha = (255.0 * fade).floor() as u32;
        if alpha > 0 {
            let label_width = width(font, &label);
            let color = crate::hud_layout::argb(alpha, 255, 255, 255);
            text_with_backdrop(
                &mut out,
                label,
                w - label_width - 5,
                h - 9 - 5,
                label_width,
                color,
                background,
            );
        }
        out
    }
}

/// Titles and the action bar: `Hud.title`, `subtitle`, `titleTime`, the
/// fade times and `overlayMessageString` / `overlayMessageTime`.
#[derive(Clone, Debug, PartialEq)]
pub struct Titles {
    pub title: Option<Text>,
    pub subtitle: Option<Text>,
    pub time: i32,
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
    pub overlay: Option<Text>,
    pub overlay_time: i32,
    /// `animateOverlayMessageColor`: the "Now Playing" rainbow.
    pub overlay_animated: bool,
}
impl Default for Titles {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            time: 0,
            fade_in: 10,
            stay: 70,
            fade_out: 20,
            overlay: None,
            overlay_time: 0,
            overlay_animated: false,
        }
    }
}
impl Titles {
    /// `setTitle`.
    pub fn set_title(&mut self, title: Text) {
        self.title = Some(title);
        self.time = self.fade_in + self.stay + self.fade_out;
    }
    /// `setSubtitle`.
    pub fn set_subtitle(&mut self, subtitle: Text) {
        self.subtitle = Some(subtitle);
    }
    /// `setTimes`: negative values keep the current time.
    pub fn set_times(&mut self, fade_in: i32, stay: i32, fade_out: i32) {
        if fade_in >= 0 {
            self.fade_in = fade_in;
        }
        if stay >= 0 {
            self.stay = stay;
        }
        if fade_out >= 0 {
            self.fade_out = fade_out;
        }
        if self.time > 0 {
            self.time = self.fade_in + self.stay + self.fade_out;
        }
    }
    /// `clearTitles`.
    pub fn clear(&mut self) {
        self.title = None;
        self.subtitle = None;
        self.time = 0;
    }
    /// `resetTitleTimes`.
    pub fn reset_times(&mut self) {
        (self.fade_in, self.stay, self.fade_out) = (10, 70, 20);
    }
    /// `setOverlayMessage`.
    pub fn set_overlay(&mut self, text: Text, animate: bool) {
        self.overlay = Some(text);
        self.overlay_time = 60;
        self.overlay_animated = animate;
    }
    /// The title and action bar part of `Hud.tick`.
    pub fn tick(&mut self) {
        if self.overlay_time > 0 {
            self.overlay_time -= 1;
        }
        if self.time > 0 {
            self.time -= 1;
            if self.time <= 0 {
                self.title = None;
                self.subtitle = None;
            }
        }
    }
    /// `Hud.extractOverlayMessage`; `bar_height` is
    /// `max(leftHeight, rightHeight)`.
    pub fn overlay_message(
        &self,
        w: i32,
        h: i32,
        bar_height: i32,
        partial: f32,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(message) = &self.overlay else {
            return out;
        };
        if self.overlay_time <= 0 {
            return out;
        }
        let t = self.overlay_time as f32 - partial;
        let alpha = ((t * 255.0 / 20.0) as i32).min(255);
        if alpha > 0 {
            let message = message.resolve_legacy();
            // The bars' height plus the gap between the item name (59) and the
            // overlay (68), at least the default level.
            let y_shift = bar_height + (68 - 59);
            out.push(GuiDraw::Push(vec![PoseOp::Translate(
                (w / 2) as f32,
                (h - y_shift.max(68)) as f32,
            )]));
            let color = if self.overlay_animated {
                hsv_to_argb(t / 50.0, 0.7, 0.6, alpha as u32)
            } else {
                crate::hud_layout::argb(alpha as u32, 255, 255, 255)
            };
            let width = width(font, &message);
            text_with_backdrop(&mut out, message, -width / 2, -4, width, color, background);
            out.push(GuiDraw::Pop);
        }
        out
    }
    /// `Hud.extractTitle`.
    pub fn title(
        &self,
        w: i32,
        h: i32,
        partial: f32,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(title) = &self.title else {
            return out;
        };
        if self.time <= 0 {
            return out;
        }
        let t = self.time as f32 - partial;
        let mut alpha = 255;
        if self.time > self.fade_out + self.stay {
            let time = (self.fade_in + self.stay + self.fade_out) as f32 - t;
            alpha = (time * 255.0 / self.fade_in as f32) as i32;
        }
        if self.time <= self.fade_out {
            alpha = (t * 255.0 / self.fade_out as f32) as i32;
        }
        let alpha = alpha.clamp(0, 255);
        if alpha > 0 {
            let title = title.resolve_legacy();
            out.push(GuiDraw::Push(vec![PoseOp::Translate(
                (w / 2) as f32,
                (h / 2) as f32,
            )]));
            out.push(GuiDraw::Push(vec![PoseOp::Scale(4.0, 4.0)]));
            let title_width = width(font, &title);
            let color = crate::hud_layout::argb(alpha as u32, 255, 255, 255);
            text_with_backdrop(
                &mut out,
                title,
                -title_width / 2,
                -10,
                title_width,
                color,
                background,
            );
            out.push(GuiDraw::Pop);
            if let Some(subtitle) = &self.subtitle {
                let subtitle = subtitle.resolve_legacy();
                out.push(GuiDraw::Push(vec![PoseOp::Scale(2.0, 2.0)]));
                let subtitle_width = width(font, &subtitle);
                text_with_backdrop(
                    &mut out,
                    subtitle,
                    -subtitle_width / 2,
                    5,
                    subtitle_width,
                    color,
                    background,
                );
                out.push(GuiDraw::Pop);
            }
            out.push(GuiDraw::Pop);
        }
        out
    }
}

/// The selected item as `Hud.tick` compares it: its item id and hover name.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldName {
    pub item: String,
    /// `ItemStack.getHoverName`.
    pub hover_name: Text,
    /// `getRarity().color()`.
    pub rarity_color: u32,
    /// The stack has `CUSTOM_NAME`.
    pub custom_name: bool,
}

/// `Hud.toolHighlightTimer` and `lastToolHighlight`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolHighlight {
    pub timer: i32,
    pub last: Option<HeldName>,
}
impl ToolHighlight {
    /// The selected-item part of `Hud.tick`; `display_time` is the
    /// `notificationDisplayTime` option.
    pub fn tick(&mut self, selected: Option<HeldName>, display_time: f64) {
        match &selected {
            None => self.timer = 0,
            Some(s) => {
                let changed = self
                    .last
                    .as_ref()
                    .is_none_or(|l| l.item != s.item || l.hover_name != s.hover_name);
                if changed {
                    self.timer = (40.0 * display_time) as i32;
                } else if self.timer > 0 {
                    self.timer -= 1;
                }
            }
        }
        self.last = selected;
    }
    /// `Hud.extractSelectedItemName`; `can_hurt` is
    /// `gameMode.canHurtPlayer()` (survival and adventure) and `bar_height`
    /// is `max(leftHeight, rightHeight)`.
    pub fn draw(
        &self,
        w: i32,
        h: i32,
        bar_height: i32,
        can_hurt: bool,
        font: &Font,
        background: u32,
    ) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        let Some(last) = &self.last else {
            return out;
        };
        if self.timer <= 0 {
            return out;
        }
        let mut name = last.hover_name.clone().with_color(last.rarity_color);
        if last.custom_name {
            name = name.with_style(&Style {
                italic: Some(true),
                ..Style::default()
            });
        }
        let str_width = width(font, &name);
        let x = (w - str_width) / 2;
        let mut y = h - bar_height.max(59);
        if !can_hurt {
            y += 14;
        }
        let alpha = ((self.timer as f32 * 256.0 / 10.0) as i32).min(255);
        if alpha > 0 {
            let color = crate::hud_layout::argb(alpha as u32, 255, 255, 255);
            text_with_backdrop(&mut out, name, x, y, str_width, color, background);
        }
        out
    }
}

/// One `LerpingBossEvent` as `BossHealthOverlay` draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct BossBar {
    pub name: Text,
    /// `getProgress()` (the lerped value).
    pub progress: f32,
    /// `BossBarColor` ordinal: pink, blue, red, green, yellow, purple, white.
    pub color: usize,
    /// `BossBarOverlay` ordinal: progress, notched_6, _10, _12, _20.
    pub overlay: usize,
}
const BOSS_COLORS: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];
const BOSS_OVERLAYS: [&str; 5] = [
    "progress",
    "notched_6",
    "notched_10",
    "notched_12",
    "notched_20",
];

/// `BossHealthOverlay.extractRenderState`: the bars in event order, 19 pixels
/// apart from y = 12 until a third of the screen height.
pub fn boss_overlay(w: i32, h: i32, bars: &[BossBar], font: &Font) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    let mut y_offset = 12;
    for bar in bars {
        let x_left = w / 2 - 91;
        let part = |out: &mut Vec<GuiDraw>, width: i32, kind: &str| {
            let sprite = |name: String| GuiDraw::Sprite {
                sprite: Cow::Owned(format!("boss_bar/{name}_{kind}")),
                x: x_left,
                y: y_offset,
                w: width,
                h: 5,
                region: Some([182, 5, 0, 0]),
                color: crate::hud_layout::WHITE,
                blend: crate::hud_layout::Blend::Textured,
            };
            out.push(sprite(BOSS_COLORS[bar.color].to_owned()));
            if bar.overlay != 0 {
                out.push(sprite(BOSS_OVERLAYS[bar.overlay].to_owned()));
            }
        };
        part(&mut out, 182, "background");
        let progress = crate::hud_layout::lerp_discrete(bar.progress, 0, 182);
        if progress > 0 {
            part(&mut out, progress, "progress");
        }
        let name = bar.name.resolve_legacy();
        let width = width(font, &name);
        text(
            &mut out,
            name,
            w / 2 - width / 2,
            y_offset - 9,
            crate::hud_layout::WHITE,
            true,
        );
        y_offset += 10 + 9;
        if y_offset >= h / 3 {
            break;
        }
    }
    out
}

/// `JumpableVehicleBar.extractBackground`: `cooldown` is the mount's jump
/// cooldown and `scale` the player's `getJumpRidingScale()`.
pub fn jump_bar(w: i32, h: i32, cooldown: i32, scale: f32) -> Vec<GuiDraw> {
    let (left, top) = ((w - 182) / 2, h - 24 - 5);
    let sprite = |name: &'static str, width: i32, region: Option<[i32; 4]>| GuiDraw::Sprite {
        sprite: Cow::Borrowed(name),
        x: left,
        y: top,
        w: width,
        h: 5,
        region,
        color: crate::hud_layout::WHITE,
        blend: crate::hud_layout::Blend::Textured,
    };
    let mut out = vec![sprite("hud/jump_bar_background", 182, None)];
    if cooldown > 0 {
        out.push(sprite("hud/jump_bar_cooldown", 182, None));
    } else {
        let progress = crate::hud_layout::lerp_discrete(scale, 0, 182);
        if progress > 0 {
            out.push(sprite(
                "hud/jump_bar_progress",
                progress,
                Some([182, 5, 0, 0]),
            ));
        }
    }
    out
}

/// `String.CASE_INSENSITIVE_ORDER` over UTF-16 code units.
pub fn compare_ignore_case(a: &str, b: &str) -> Ordering {
    fn single(mut it: impl Iterator<Item = char>, c: char) -> char {
        match (it.next(), it.next()) {
            (Some(m), None) => m,
            _ => c,
        }
    }
    let unit = |u: u16| char::from_u32(u32::from(u));
    let (x, y): (Vec<u16>, Vec<u16>) = (a.encode_utf16().collect(), b.encode_utf16().collect());
    for (&p, &q) in x.iter().zip(&y) {
        if p == q {
            continue;
        }
        let (Some(c1), Some(c2)) = (unit(p), unit(q)) else {
            return p.cmp(&q);
        };
        let (u1, u2) = (single(c1.to_uppercase(), c1), single(c2.to_uppercase(), c2));
        if u1 != u2 {
            let (l1, l2) = (single(u1.to_lowercase(), u1), single(u2.to_lowercase(), u2));
            if l1 != l2 {
                return (l1 as u32).cmp(&(l2 as u32));
            }
        }
    }
    x.len().cmp(&y.len())
}

/// `Hud.displayScoreboardSidebar` for `objective`; `background` and
/// `header` are `Options.getBackgroundColor(0.3F)` and `(0.4F)`.
pub fn sidebar(
    w: i32,
    h: i32,
    board: &crate::scoreboard::Scoreboard,
    objective: &crate::scoreboard::Objective,
    font: &Font,
    background: u32,
    header: u32,
) -> Vec<GuiDraw> {
    use crate::scoreboard::NumberFormat;
    let format = objective.format_or_default(NumberFormat::sidebar_default());
    let mut entries: Vec<_> = board
        .list_scores(&objective.name)
        .into_iter()
        .filter(|e| !e.is_hidden())
        .collect();
    // SCORE_DISPLAY_ORDER: highest value first, then owners case-insensitively.
    java_sort(&mut entries, |a, b| {
        b.value
            .cmp(&a.value)
            .then_with(|| compare_ignore_case(&a.owner, &b.owner))
    });
    entries.truncate(15);
    // Font draws each literal through StringDecomposer.iterateFormatted:
    // legacy `§` codes style what follows and are not drawn.
    let rows: Vec<(Text, Text, i32)> = entries
        .iter()
        .map(|e| {
            let name = match board.players_team(&e.owner) {
                Some(team) => team.formatted_name(e.owner_name()),
                None => e.owner_name(),
            }
            .resolve_legacy();
            let score = e.format_value(&format).resolve_legacy();
            let score_width = width(font, &score);
            (name, score, score_width)
        })
        .collect();
    let title = objective.display_text().resolve_legacy();
    let title_width = width(font, &title);
    let spacer = font.width(": ") as i32;
    let mut biggest = title_width;
    for (name, _, score_width) in &rows {
        let extra = if *score_width > 0 {
            spacer + score_width
        } else {
            0
        };
        biggest = biggest.max(width(font, name) + extra);
    }
    let count = rows.len() as i32;
    let height = count * 9;
    let bottom = h / 2 + height / 3;
    let left = w - biggest - 3;
    let right = w - 3 + 2;
    let header_y = bottom - count * 9;
    let mut out = vec![
        fill(left - 2, header_y - 9 - 1, right, header_y - 1, header),
        fill(left - 2, header_y - 1, right, bottom, background),
    ];
    let white = crate::hud_layout::WHITE;
    text(
        &mut out,
        title,
        left + biggest / 2 - title_width / 2,
        header_y - 9,
        white,
        false,
    );
    for (i, (name, score, score_width)) in rows.into_iter().enumerate() {
        let y = bottom - (count - i as i32) * 9;
        text(&mut out, name, left, y, white, false);
        text(&mut out, score, right - score_width, y, white, false);
    }
    out
}

/// What `GuiGraphicsExtractor.itemDecorations` draws over a stack.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemDecorations {
    pub count: u32,
    /// The text shown instead of the count, if any.
    pub count_text: Option<String>,
    /// `isBarVisible`: `getBarWidth` and `getBarColor` (RGB).
    pub bar: Option<(i32, u32)>,
    /// `ItemCooldowns.getCooldownPercent` of the stack.
    pub cooldown: f32,
}

/// `Item.getBarWidth` and `getBarColor` for a damaged stack (`isDamaged`).
pub fn damage_bar(damage: i32, max_damage: i32) -> Option<(i32, u32)> {
    if max_damage <= 0 || damage <= 0 {
        return None;
    }
    let width = (13.0 - damage as f32 * 13.0 / max_damage as f32)
        .round()
        .clamp(0.0, 13.0) as i32;
    let health = ((max_damage as f32 - damage as f32) / max_damage as f32).max(0.0);
    Some((width, hsv_to_argb(health / 3.0, 1.0, 1.0, 0)))
}

/// `GuiGraphicsExtractor.itemDecorations` for a non-empty stack at (x, y):
/// the durability bar, the cooldown shade and the count.
pub fn item_decorations(x: i32, y: i32, d: &ItemDecorations, font: &Font) -> Vec<GuiDraw> {
    let mut out = vec![GuiDraw::Push(Vec::new())];
    if let Some((width, color)) = d.bar {
        let (left, top) = (x + 2, y + 13);
        out.push(fill(left, top, left + 13, top + 2, 0xFF00_0000));
        out.push(fill(left, top, left + width, top + 1, color | 0xFF00_0000));
    }
    if d.cooldown > 0.0 {
        let top = y + crate::hud_layout::floor(16.0 * (1.0 - d.cooldown));
        let bottom = top + crate::hud_layout::ceil(16.0 * d.cooldown);
        out.push(fill(x, top, x + 16, bottom, 0x7FFF_FFFF));
    }
    if d.count != 1 || d.count_text.is_some() {
        let amount = d.count_text.clone().unwrap_or_else(|| d.count.to_string());
        let w = font.width(&amount) as i32;
        text(
            &mut out,
            Text::literal(amount),
            x + 19 - 2 - w,
            y + 6 + 3,
            crate::hud_layout::WHITE,
            true,
        );
    }
    out.push(GuiDraw::Pop);
    out
}

/// A full-screen `graphics.blit` of `textures/<texture>.png`.
fn screen_blit(
    w: i32,
    h: i32,
    texture: impl Into<Cow<'static, str>>,
    blend: Blend,
    color: u32,
) -> GuiDraw {
    GuiDraw::Blit {
        texture: texture.into(),
        blend,
        x: 0,
        y: 0,
        w,
        h,
        uv: [0.0, 0.0],
        src: [w, h],
        tex: [w, h],
        color,
    }
}

/// `Hud.extractVignette`: `brightness` is `vignetteBrightness` and
/// `border_warning` the world border warning strength (0 when far).
pub fn vignette(w: i32, h: i32, brightness: f32, border_warning: f32) -> GuiDraw {
    let brightness = brightness.clamp(0.0, 1.0);
    let color = if border_warning > 0.0 {
        let warning = border_warning.clamp(0.0, 1.0);
        let red = brightness * (1.0 - warning);
        let green_blue = brightness + (1.0 - brightness) * warning;
        color_from_float(1.0, red, green_blue, green_blue)
    } else {
        color_from_float(1.0, brightness, brightness, brightness)
    };
    screen_blit(w, h, "misc/vignette", Blend::Vignette, color)
}

/// The border warning strength of `extractVignette` for a camera
/// `distance` from a border with `warning_blocks`, `warning_time`, its
/// `lerp_speed` and how far its size still has to move.
pub fn border_warning(
    distance: f32,
    warning_blocks: i32,
    warning_time: i32,
    lerp_speed: f64,
    size_left: f64,
) -> f32 {
    let moving = (lerp_speed * f64::from(warning_time)).min(size_left.abs());
    let warning_distance = f64::from(warning_blocks).max(moving);
    if f64::from(distance) < warning_distance {
        1.0 - (f64::from(distance) / warning_distance) as f32
    } else {
        0.0
    }
}

/// `Hud.extractSpyglassOverlay`.
pub fn spyglass(w: i32, h: i32, scale: f32) -> Vec<GuiDraw> {
    let src = w.min(h) as f32;
    let ratio = (w as f32 / src).min(h as f32 / src) * scale;
    let width = crate::hud_layout::floor(src * ratio);
    let height = crate::hud_layout::floor(src * ratio);
    let left = (w - width) / 2;
    let top = (h - height) / 2;
    let (right, bottom) = (left + width, top + height);
    vec![
        GuiDraw::Blit {
            texture: Cow::Borrowed("misc/spyglass_scope"),
            blend: Blend::Textured,
            x: left,
            y: top,
            w: width,
            h: height,
            uv: [0.0, 0.0],
            src: [width, height],
            tex: [width, height],
            color: crate::hud_layout::WHITE,
        },
        fill(0, bottom, w, h, 0xFF00_0000),
        fill(0, 0, w, top, 0xFF00_0000),
        fill(0, top, left, bottom, 0xFF00_0000),
        fill(right, top, w, bottom, 0xFF00_0000),
    ]
}

/// `Hud.extractPortalOverlay`: the nether portal block sprite over the screen.
pub fn portal_overlay(w: i32, h: i32, mut alpha: f32) -> GuiDraw {
    if alpha < 1.0 {
        alpha *= alpha;
        alpha *= alpha;
        alpha = alpha * 0.8 + 0.2;
    }
    GuiDraw::AtlasSprite {
        sprite: Cow::Borrowed("block/nether_portal"),
        x: 0,
        y: 0,
        w,
        h,
        color: white(alpha),
    }
}

/// `Hud.extractConfusionOverlay`: the nausea swirl scaled about the centre.
pub fn confusion_overlay(w: i32, h: i32, strength: f32) -> Vec<GuiDraw> {
    let size = 2.0 + strength * (1.0 - 2.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    vec![
        GuiDraw::Push(vec![
            PoseOp::Translate(cx, cy),
            PoseOp::Scale(size, size),
            PoseOp::Translate(-cx, -cy),
        ]),
        screen_blit(
            w,
            h,
            "misc/nausea",
            Blend::Nausea,
            color_from_float(1.0, 0.2 * strength, 0.4 * strength, 0.2 * strength),
        ),
        GuiDraw::Pop,
    ]
}

/// What `Hud.extractCameraOverlays` reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraOverlays {
    /// `Options.vignette`: `vignetteBrightness` and the border warning strength.
    pub vignette: Option<(f32, f32)>,
    pub first_person: bool,
    /// `LocalPlayer.isScoping`.
    pub scoping: bool,
    /// `DeltaTracker.getGameTimeDeltaTicks`.
    pub game_time_delta: f32,
    /// `Equippable.cameraOverlay` textures (`misc/pumpkinblur`) of the worn
    /// items in `EquipmentSlot` order.
    pub equipment: Vec<String>,
    /// `getTicksFrozen` and `getPercentFrozen`.
    pub ticks_frozen: i32,
    pub percent_frozen: f32,
    /// The partial tick and `oPortalEffectIntensity` / `portalEffectIntensity`.
    pub partial: f32,
    pub portal: [f32; 2],
    /// `getEffectBlendFactor(NAUSEA, partial)` and `Options.screenEffectScale`.
    pub nausea: f32,
    pub screen_effect_scale: f32,
}

/// `Hud.extractCameraOverlays`, easing `scope_scale` (`Hud.scopeScale`).
pub fn camera_overlays(w: i32, h: i32, i: &CameraOverlays, scope_scale: &mut f32) -> Vec<GuiDraw> {
    let mut out = Vec::new();
    if let Some((brightness, warning)) = i.vignette {
        out.push(vignette(w, h, brightness, warning));
    }
    *scope_scale += 0.5 * i.game_time_delta * (1.125 - *scope_scale);
    if i.first_person {
        if i.scoping {
            out.extend(spyglass(w, h, *scope_scale));
        } else {
            *scope_scale = 0.5;
            for texture in &i.equipment {
                out.push(screen_blit(
                    w,
                    h,
                    texture.clone(),
                    Blend::Textured,
                    crate::hud_layout::WHITE,
                ));
            }
        }
    }
    if i.ticks_frozen > 0 {
        out.push(screen_blit(
            w,
            h,
            "misc/powder_snow_outline",
            Blend::Textured,
            white(i.percent_frozen),
        ));
    }
    let portal = i.portal[0] + i.partial * (i.portal[1] - i.portal[0]);
    if portal > 0.0 {
        out.push(portal_overlay(w, h, portal));
    } else if i.nausea > 0.0 && i.screen_effect_scale < 1.0 {
        out.extend(confusion_overlay(
            w,
            h,
            i.nausea * (1.0 - i.screen_effect_scale),
        ));
    }
    out
}

/// `Hud.extractSleepOverlay`.
pub fn sleep_overlay(w: i32, h: i32, sleep_timer: i32) -> Vec<GuiDraw> {
    if sleep_timer <= 0 {
        return Vec::new();
    }
    let timer = sleep_timer as f32;
    let mut amount = timer / 100.0;
    if amount > 1.0 {
        amount = 1.0 - (timer - 100.0) / 10.0;
    }
    let color = (((220.0 * amount) as i32) << 24) as u32 | 0x10_1020;
    vec![fill(0, 0, w, h, color)]
}

// ---- subtitles (`SubtitleOverlay`) ------------------------------------------
//
// Closed captions for the sounds the player could hear, with direction arrows
// and a fade over the notification display time. Like vanilla, the overlay
// listens to the sound engine only from the frame that renders it with the
// "Show Subtitles" option on until a frame renders with it off; turning it
// off keeps the captions it has (they show again if the option comes back
// before they expire).

/// `SubtitleOverlay.DISPLAY_TIME` (milliseconds).
const DISPLAY_TIME: i64 = 3000;
/// `LOOPING_DISPLAY_FULL_BRIGHTNESS_TIME`: looping sounds stay at full brightness this long.
const LOOPING_FULL_BRIGHTNESS: i64 = 500;

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayedAt {
    location: [f64; 3],
    time: i64,
}
#[derive(Clone, Debug)]
struct Subtitle {
    /// The translation key of the sound event's subtitle (`Component.translatable`).
    key: String,
    range: f32,
    played: Vec<PlayedAt>,
    looping: bool,
}
fn distance_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}
/// `Vec3.closerThan(pos, distance)`.
fn closer_than(a: [f64; 3], b: [f64; 3], range: f32) -> bool {
    let r = f64::from(range);
    distance_sqr(a, b) < r * r
}
/// `Vec3.equals`: `Double.compare` on each axis.
fn same_location(a: [f64; 3], b: [f64; 3]) -> bool {
    let bits = |v: f64| {
        if v.is_nan() {
            f64::NAN.to_bits()
        } else {
            v.to_bits()
        }
    };
    (0..3).all(|i| bits(a[i]) == bits(b[i]))
}
impl Subtitle {
    /// `getBestSubtitleCandidate`: the only play, else the latest in range
    /// (the farthest of equally recent ones; the first among full ties).
    fn best_candidate(&self, position: [f64; 3]) -> Option<PlayedAt> {
        match self.played.as_slice() {
            [] => None,
            [only] => Some(*only),
            many => many
                .iter()
                .filter(|p| closer_than(p.location, position, self.range))
                .fold(None, |best: Option<PlayedAt>, p| match best {
                    Some(b)
                        if b.time > p.time
                            || b.time == p.time
                                && distance_sqr(b.location, position).sqrt()
                                    >= distance_sqr(p.location, position).sqrt() =>
                    {
                        Some(b)
                    }
                    _ => Some(*p),
                }),
        }
    }
    fn refresh(&mut self, location: [f64; 3], now: i64) {
        self.played.retain(|p| !same_location(location, p.location));
        self.played.push(PlayedAt {
            location,
            time: now,
        });
    }
    fn audible_from(&self, camera: [f64; 3]) -> bool {
        if self.range.is_infinite() {
            return true;
        }
        self.best_candidate(camera)
            .is_some_and(|p| closer_than(camera, p.location, self.range))
    }
    fn purge_old_instances(&mut self, max_age: f64, now: i64) {
        self.played.retain(|p| ((now - p.time) as f64) <= max_age);
    }
    /// `getBrightness`: fades from 255 to 75 over the display time.
    fn brightness(&self, at: PlayedAt, multiplier: f64, now: i64) -> i32 {
        let full = if self.looping {
            LOOPING_FULL_BRIGHTNESS
        } else {
            0
        };
        let age = (now - at.time - full).max(0) as f32;
        let t = age / (((DISPLAY_TIME - full) as f64) * multiplier) as f32;
        let v = if t < 0.0 {
            255.0
        } else if t > 1.0 {
            75.0
        } else {
            255.0 + t * (75.0 - 255.0_f32)
        };
        v.floor() as i32
    }
}

/// What a frame needs to render the overlay: the sound listener, the
/// accessibility options and the GUI size.
#[derive(Clone, Copy, Debug)]
pub struct SubtitleFrame {
    /// `options.showSubtitles`.
    pub show: bool,
    /// `SoundManager.getListenerTransform`: position, forward and up.
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    /// `options.notificationDisplayTime`.
    pub multiplier: f64,
    /// `options.getBackgroundColor(0.8F)` (ARGB, `Settings::text_background`).
    pub background: u32,
    pub width: i32,
    pub height: i32,
}
/// Milliseconds on a monotonic clock (`Util.getMillis`).
pub fn now_ms() -> i64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as i64
}
#[derive(Default)]
pub struct SubtitleOverlay {
    subtitles: Vec<Subtitle>,
    listening: bool,
}
impl SubtitleOverlay {
    /// Whether the overlay is one of the sound engine's listeners.
    pub fn listening(&self) -> bool {
        self.listening
    }
    /// `onPlaySound` for a sound the engine played while the overlay listens:
    /// a subtitle already listed (by its text) refreshes its location.
    pub fn on_play(
        &mut self,
        subtitle: Option<&str>,
        location: [f64; 3],
        range: f32,
        looping: bool,
        now: i64,
    ) {
        let Some(key) = subtitle else {
            return;
        };
        if let Some(s) = self.subtitles.iter_mut().find(|s| s.key == key) {
            s.refresh(location, now);
            return;
        }
        self.subtitles.push(Subtitle {
            key: key.to_owned(),
            range,
            played: vec![PlayedAt {
                location,
                time: now,
            }],
            looping,
        });
    }
    /// `extractRenderState`: starts or stops listening as the option says,
    /// forgets old plays and lays out the boxes at the bottom right.
    pub fn extract(&mut self, frame: &SubtitleFrame, font: &Font, now: i64) -> Vec<GuiDraw> {
        let mut out = Vec::new();
        if self.listening != frame.show {
            self.listening = frame.show;
        }
        if !self.listening {
            return out;
        }
        let position = frame.position;
        let (f, u) = (frame.forward, frame.up);
        // ListenerTransform.right: forward x up.
        let right = [
            f[1] * u[2] - f[2] * u[1],
            f[2] * u[0] - f[0] * u[2],
            f[0] * u[1] - f[1] * u[0],
        ];
        let width_of = |t: &Text| width(font, t);
        let mut audible = Vec::new();
        let mut width = 0;
        for (i, s) in self.subtitles.iter_mut().enumerate() {
            if !s.played.is_empty() {
                s.purge_old_instances(3000.0 * frame.multiplier, now);
            }
            if s.audible_from(position) && !s.played.is_empty() {
                let text_width = width_of(&Text::translate(&s.key, &[]));
                width = width.max(text_width);
                audible.push((i, text_width));
            }
        }
        if audible.is_empty() {
            return out;
        }
        let w = |s: &str| font.width(s) as i32;
        width += w("<") + w(" ") + w(">") + w(" ");
        let mut row = 0;
        for (i, text_width) in audible {
            let s = &self.subtitles[i];
            let text_value = Text::translate(&s.key, &[]);
            let Some(closest) = s.best_candidate(position) else {
                continue;
            };
            let mut delta = [
                closest.location[0] - position[0],
                closest.location[1] - position[1],
                closest.location[2] - position[2],
            ];
            // Vec3.normalize.
            let length = distance_sqr(delta, [0.0; 3]).sqrt();
            delta = if length < f64::from(1.0e-5_f32) {
                [0.0; 3]
            } else {
                delta.map(|v| v / length)
            };
            let dot = |a: [f64; 3]| a[0] * delta[0] + a[1] * delta[1] + a[2] * delta[2];
            let (rightness, forwardness) = (dot(right), dot(f));
            let in_view = forwardness > 0.5;
            let half_width = width / 2;
            let height = 9;
            let half_height = height / 2;
            let brightness = s.brightness(closest, frame.multiplier, now);
            out.push(GuiDraw::Push(vec![
                PoseOp::Translate(
                    frame.width as f32 - half_width as f32 * 1.0 - 2.0,
                    (frame.height - 35) as f32 - (row * (height + 1)) as f32 * 1.0,
                ),
                PoseOp::Scale(1.0, 1.0),
            ]));
            out.push(fill(
                -half_width - 1,
                -half_height - 1,
                half_width + 1,
                half_height + 1,
                frame.background,
            ));
            let b = brightness as u32 & 0xFF;
            let color = 0xFF00_0000 | b << 16 | b << 8 | b;
            if !in_view {
                if rightness > 0.0 {
                    text(
                        &mut out,
                        Text::literal(">"),
                        half_width - w(">"),
                        -half_height,
                        color,
                        true,
                    );
                } else if rightness < 0.0 {
                    text(
                        &mut out,
                        Text::literal("<"),
                        -half_width,
                        -half_height,
                        color,
                        true,
                    );
                }
            }
            text(
                &mut out,
                text_value,
                -text_width / 2,
                -half_height,
                color,
                true,
            );
            out.push(GuiDraw::Pop);
            row += 1;
        }
        out
    }
}

#[cfg(test)]
mod subtitle_tests {
    use super::*;

    #[test]
    fn candidates_and_brightness() {
        let mut s = Subtitle {
            key: "a".into(),
            range: 16.0,
            played: vec![PlayedAt {
                location: [0.0, 0.0, 3.0],
                time: 1000,
            }],
            looping: false,
        };
        assert_eq!(s.brightness(s.played[0], 1.0, 1000), 255);
        assert_eq!(s.brightness(s.played[0], 1.0, 2500), 165);
        assert_eq!(s.brightness(s.played[0], 2.0, 2500), 210);
        assert_eq!(s.brightness(s.played[0], 1.0, 9000), 75);
        // Equally recent plays: the farthest in range wins.
        s.refresh([0.0, 0.0, 5.0], 1000);
        s.refresh([0.0, 0.0, 40.0], 1000);
        let best = s.best_candidate([0.0; 3]).unwrap();
        assert_eq!(best.location, [0.0, 0.0, 5.0]);
        s.purge_old_instances(3000.0, 4001);
        assert!(s.played.is_empty());
        assert!(!s.audible_from([0.0; 3]));
    }

    #[test]
    fn listening_follows_the_option() {
        let mut o = SubtitleOverlay::default();
        o.on_play(Some("a"), [0.0; 3], 16.0, false, 0);
        assert_eq!(o.subtitles.len(), 1);
        o.on_play(None, [0.0; 3], 16.0, false, 0);
        o.on_play(Some("a"), [1.0; 3], 16.0, false, 5);
        assert_eq!(o.subtitles.len(), 1);
        assert_eq!(o.subtitles[0].played.len(), 2);
        assert!(!o.listening());
    }
}
