//! Shared GUI widgets drawn as HUD quads: nine-slice sprites (gui.scaling
//! metadata), buttons, sliders, tiled menu backgrounds, separators and the
//! EditBox text field with vanilla cursor, selection and scrolling.
use crate::{
    hud::{HudAtlas, HudBatch, Rect, rgba},
    text::Text,
};
use std::{collections::HashMap, path::Path, sync::RwLock, time::Instant};

/// `GuiSpriteScaling.NineSlice`: the sprite's size in GUI pixels, whatever
/// its texture's resolution, its borders (left, top, right, bottom) in the
/// same units, and whether the edges and centre stretch rather than tile.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NineSlice {
    width: u32,
    height: u32,
    border: [u32; 4],
    stretch_inner: bool,
}
/// Nine-slice scaling from `.png.mcmeta` files.
static NINE_SLICE: RwLock<Option<HashMap<String, NineSlice>>> = RwLock::new(None);

/// A `gui.scaling` section of type `nine_slice`.
fn parse_nine_slice(v: &serde_json::Value) -> Option<NineSlice> {
    let s = &v["gui"]["scaling"];
    if s["type"] != "nine_slice" {
        return None;
    }
    let size = |k: &str| s[k].as_u64().map(|n| n as u32);
    let border = &s["border"];
    let border = if let Some(n) = border.as_u64() {
        [n as u32; 4]
    } else {
        let g = |k: &str| border[k].as_u64().unwrap_or(0) as u32;
        [g("left"), g("top"), g("right"), g("bottom")]
    };
    let (width, height) = (size("width")?, size("height")?);
    // `NineSlice.validate`: borders that meet leave no centre slice, so the
    // sprite has no nine-slice metadata (vanilla draws it as a stretch).
    let [l, t, r, b] = border;
    if l.saturating_add(r) >= width || t.saturating_add(b) >= height {
        return None;
    }
    Some(NineSlice {
        width,
        height,
        border,
        stretch_inner: s["stretch_inner"].as_bool().unwrap_or(false),
    })
}
/// Read `gui.scaling` metadata for every GUI sprite under `root`.
pub fn load_metadata(root: &Path) {
    let mut map = HashMap::new();
    let base = root.join("assets/minecraft/textures");
    let mut stack = vec![base.join("gui/sprites")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.to_string_lossy().into_owned();
            let Some(stripped) = name.strip_suffix(".png.mcmeta") else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Some(nine_slice) = serde_json::from_str(&text)
                .ok()
                .as_ref()
                .and_then(parse_nine_slice)
            else {
                continue;
            };
            let key = Path::new(stripped)
                .strip_prefix(&base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            map.insert(key, nine_slice);
        }
    }
    *NINE_SLICE.write().unwrap_or_else(|e| e.into_inner()) = Some(map);
}
/// The sprite's nine-slice metadata, when its `.png.mcmeta` has any.
fn nine_slice_metadata(sprite: &str) -> Option<NineSlice> {
    NINE_SLICE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(sprite).copied())
}
/// The sprite's nine-slice metadata; without any (no resources loaded), its
/// GUI size and the widgets' usual borders.
fn scaling(sprite: &str, r: &Rect) -> NineSlice {
    nine_slice_metadata(sprite).unwrap_or(NineSlice {
        width: r.w,
        height: r.h,
        border: match sprite {
            "gui/sprites/widget/button"
            | "gui/sprites/widget/button_highlighted"
            | "gui/sprites/widget/button_disabled" => [3; 4],
            "gui/sprites/widget/slider_handle" | "gui/sprites/widget/slider_handle_highlighted" => {
                [2, 2, 2, 3]
            }
            _ => [1; 4],
        },
        stretch_inner: false,
    })
}
/// The region `[x, y, w, h]` of a sprite `size` GUI pixels large, in its
/// own units (`TextureAtlasSprite.getU(x / width)`).
fn sub(r: Rect, size: [f32; 2], [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / size[0], (r.v1 - r.v0) / size[1]);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
/// `blitTiledSprite`: `src` (a region of a sprite `size` large, in its
/// units) repeated over `dst` from its top left, the last row and column cut.
fn tile(batch: &mut HudBatch, r: Rect, size: [f32; 2], src: [f32; 4], dst: [f32; 4], color: u32) {
    let [sx, sy, sw, sh] = src;
    let [dx, dy, dw, dh] = dst;
    if sw <= 0.0 || sh <= 0.0 || dw <= 0.0 || dh <= 0.0 {
        return;
    }
    let mut y = 0.0;
    while y < dh {
        let h = sh.min(dh - y);
        let mut x = 0.0;
        while x < dw {
            let w = sw.min(dw - x);
            batch.rect_uv(dx + x, dy + y, w, h, sub(r, size, [sx, sy, w, h]), color);
            x += sw;
        }
        y += sh;
    }
}
/// `GuiGraphicsExtractor.blitNineSlicedSprite`: the corners at their size,
/// the edges and centre tiled (or stretched with `stretch_inner`), all in
/// the sprite's `gui.scaling` units, so a high-resolution pack's sprite draws
/// as the default one does.
pub fn nine_slice(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if let Some(r) = atlas.sprites.get(sprite) {
        draw_nine_slice(batch, *r, scaling(sprite, r), [x, y, w, h], color);
    }
}
/// `GuiGraphicsExtractor.blitSprite`: a sprite over `w` × `h` as its
/// `gui.scaling` says, nine-sliced, or stretched when it has no metadata.
/// (`tile`, which no vanilla sprite uses, isn't read yet: it stretches.)
pub fn blit_sprite(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
) {
    if let Some(r) = atlas.sprites.get(sprite) {
        draw_sprite(batch, *r, nine_slice_metadata(sprite), [x, y, w, h], color);
    }
}
fn draw_sprite(
    batch: &mut HudBatch,
    r: Rect,
    nine_slice: Option<NineSlice>,
    [x, y, w, h]: [f32; 4],
    color: u32,
) {
    match nine_slice {
        Some(n) => draw_nine_slice(batch, r, n, [x, y, w, h], color),
        None => batch.rect_uv(x, y, w, h, r, color),
    }
}
fn draw_nine_slice(
    batch: &mut HudBatch,
    r: Rect,
    n: NineSlice,
    [x, y, w, h]: [f32; 4],
    color: u32,
) {
    let size = [n.width as f32, n.height as f32];
    let [sw, sh] = size;
    let [l, t, rr, b] = n.border.map(|v| v as f32);
    let (half_w, half_h) = ((w / 2.0).floor(), (h / 2.0).floor());
    let (l, rr, t, b) = (l.min(half_w), rr.min(half_w), t.min(half_h), b.min(half_h));
    // blitSprite(sprite, width, height, textureX, textureY, x, y, w, h).
    let part = |batch: &mut HudBatch, tx: f32, ty: f32, dst: [f32; 4]| {
        if dst[2] != 0.0 && dst[3] != 0.0 {
            batch.rect_uv(
                dst[0],
                dst[1],
                dst[2],
                dst[3],
                sub(r, size, [tx, ty, dst[2], dst[3]]),
                color,
            );
        }
    };
    // blitNineSliceInnerSegment.
    let inner = |batch: &mut HudBatch, dst: [f32; 4], src: [f32; 4]| {
        if dst[2] <= 0.0 || dst[3] <= 0.0 {
            return;
        }
        if n.stretch_inner {
            batch.rect_uv(dst[0], dst[1], dst[2], dst[3], sub(r, size, src), color);
        } else {
            tile(batch, r, size, src, dst, color);
        }
    };
    if w == sw && h == sh {
        part(batch, 0.0, 0.0, [x, y, w, h]);
        return;
    }
    if h == sh {
        part(batch, 0.0, 0.0, [x, y, l, h]);
        inner(batch, [x + l, y, w - rr - l, h], [l, 0.0, sw - rr - l, sh]);
        part(batch, sw - rr, 0.0, [x + w - rr, y, rr, h]);
        return;
    }
    if w == sw {
        part(batch, 0.0, 0.0, [x, y, w, t]);
        inner(batch, [x, y + t, w, h - b - t], [0.0, t, sw, sh - b - t]);
        part(batch, 0.0, sh - b, [x, y + h - b, w, b]);
        return;
    }
    let (cw, ch) = (w - rr - l, h - b - t);
    let (scw, sch) = (sw - rr - l, sh - b - t);
    part(batch, 0.0, 0.0, [x, y, l, t]);
    inner(batch, [x + l, y, cw, t], [l, 0.0, scw, t]);
    part(batch, sw - rr, 0.0, [x + w - rr, y, rr, t]);
    part(batch, 0.0, sh - b, [x, y + h - b, l, b]);
    inner(batch, [x + l, y + h - b, cw, b], [l, sh - b, scw, b]);
    part(batch, sw - rr, sh - b, [x + w - rr, y + h - b, rr, b]);
    inner(batch, [x, y + t, l, ch], [0.0, t, l, sch]);
    inner(batch, [x + l, y + t, cw, ch], [l, t, scw, sch]);
    inner(batch, [x + w - rr, y + t, rr, ch], [sw - rr, t, rr, sch]);
}
/// Repeat a whole texture with each copy drawn `size` GUI pixels square.
pub fn tiled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: f32,
    color: u32,
) {
    tiled_sized(batch, atlas, sprite, x, y, w, h, [size, size], color);
}
/// Repeat a texture using its width and height in GUI pixels.
#[allow(clippy::too_many_arguments)]
pub fn tiled_sized(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    sprite: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    size: [f32; 2],
    color: u32,
) {
    let Some(r) = atlas.sprites.get(sprite).copied() else {
        return;
    };
    tile(
        batch,
        r,
        size,
        [0.0, 0.0, size[0], size[1]],
        [x, y, w, h],
        color,
    );
}
/// Text drawn at `scale` times its size (a scaled pose).
pub fn text_scaled(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    scale: f32,
    color: u32,
) {
    let start = batch.vertices.len();
    batch.text_styled(atlas, text, 0.0, 0.0, color, true);
    for v in &mut batch.vertices[start..] {
        v.pos = [x + v.pos[0] * scale, y + v.pos[1] * scale];
    }
}
pub fn hovered(cursor: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    cursor.0 >= x && cursor.0 < x + w && cursor.1 >= y && cursor.1 < y + h
}
/// `TextAlignment.CENTER.calculateLeft` at the box's integer centre:
/// `x + w / 2 - width / 2`. Flooring `(w - width) / 2` sits odd-width labels
/// one pixel left of vanilla when `w` is even.
fn centered_left(x: f32, w: f32, width: f32) -> f32 {
    x + (w / 2.0).floor() - (width / 2.0).floor()
}

/// Centered text with a shadow, scrolled when wider than `w` (AbstractWidget.renderScrollingString).
pub fn centered_text(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    text: &Text,
    x: f32,
    y: f32,
    w: f32,
    color: u32,
) {
    let text = text.resolved();
    let width = atlas.font.width_text(&text);
    if width <= w {
        batch.text_styled(atlas, &text, centered_left(x, w, width), y, color, true);
    } else {
        // Clip by characters (vanilla scrolls; we show the part that fits).
        let mut shown = Text::empty();
        let mut used = 0.0;
        for (i, (c, s)) in text.chars().enumerate() {
            used += atlas.font.advance(c, s);
            if used > w - 2.0 {
                break;
            }
            shown.push(text.slice(i, i + 1));
        }
        batch.text_styled(atlas, &shown, x + 1.0, y, color, true);
    }
}
/// Button (AbstractButton): nine-slice sprite plus the centred label.
pub fn button(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
) {
    button_as(batch, atlas, x, y, w, h, label, active, hover, |rect| {
        crate::widgets::Narratable::button(label.plain(), rect, active)
    });
}
/// A [`button`] that vanilla narrates as another widget (a `CycleButton`, a
/// `LockIconButton`): `narration` builds what it records from its rectangle,
/// and only while the narrator records.
#[allow(clippy::too_many_arguments)]
pub fn button_as(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &Text,
    active: bool,
    hover: bool,
    narration: impl FnOnce([f32; 4]) -> crate::widgets::Narratable,
) {
    crate::widgets::record(&label.plain(), [x, y, w, h], active);
    crate::widgets::narratable(|| narration([x, y, w, h]).hovered(hover));
    crate::widgets::widget_cursor(hover, active);
    let sprite = if !active {
        "gui/sprites/widget/button_disabled"
    } else if hover {
        "gui/sprites/widget/button_highlighted"
    } else {
        "gui/sprites/widget/button"
    };
    nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
    let color = if active {
        rgba(255, 255, 255, 255)
    } else {
        rgba(160, 160, 160, 255)
    };
    centered_text(
        batch,
        atlas,
        label,
        x + 2.0,
        y + ((h - 8.0) / 2.0).floor(),
        w - 4.0,
        color,
    );
}
/// AbstractSliderButton: track, 8px handle at `value` (0..1) and the label.
pub fn slider(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    label: &Text,
    value: f32,
    hover: bool,
    active: bool,
    dragging: bool,
) {
    slider_ext(
        batch, atlas, x, y, w, label, value, hover, false, false, active, dragging,
    );
}

/// `AbstractSliderButton.extractWidgetRenderState`: track, 8px handle at `value` (0..1),
/// with support for keyboard focus and `canChangeValue`.
pub fn slider_ext(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    label: &Text,
    value: f32,
    hover: bool,
    focused: bool,
    can_change_value: bool,
    active: bool,
    dragging: bool,
) {
    let kind = crate::widgets::Kind::Slider {
        value,
        track: [x + 4.0, w - 8.0],
    };
    // The control socket's widget tree (`ui`): its label and value.
    crate::widgets::record_as(&label.plain(), [x, y, w, 20.0], active, kind);
    crate::widgets::narratable(|| {
        crate::widgets::Narratable::slider(label.plain(), [x, y, w, 20.0], active).hovered(hover)
    });
    if active {
        // A controller's D-pad can focus it (Horizon's controller support).
        crate::widgets::target_as([x, y, w, 20.0], kind);
    }
    let (track, handle) = slider_sprites_ext(hover, active, focused, can_change_value, dragging);
    nine_slice(batch, atlas, track, x, y, w, 20.0, u32::MAX);
    nine_slice(
        batch,
        atlas,
        handle,
        x + (value.clamp(0.0, 1.0) * (w - 8.0)).floor(),
        y,
        8.0,
        20.0,
        u32::MAX,
    );
    let color = if active {
        u32::MAX
    } else {
        rgba(160, 160, 160, 255)
    };
    centered_text(batch, atlas, label, x + 2.0, y + 6.0, w - 4.0, color);
}
#[cfg(test)]
fn slider_sprites(hover: bool, active: bool, dragging: bool) -> (&'static str, &'static str) {
    slider_sprites_ext(hover, active, false, false, dragging)
}

/// Track and handle sprites: the track outline shows while dragged or focused with
/// keyboard navigation (!can_change_value), hovering or editing highlights the handle.
fn slider_sprites_ext(
    hover: bool,
    active: bool,
    focused: bool,
    can_change_value: bool,
    dragging: bool,
) -> (&'static str, &'static str) {
    let track = if dragging || (active && focused && !can_change_value) {
        "gui/sprites/widget/slider_highlighted"
    } else {
        "gui/sprites/widget/slider"
    };
    let handle = if active && (hover || can_change_value || dragging) {
        "gui/sprites/widget/slider_handle_highlighted"
    } else {
        "gui/sprites/widget/slider_handle"
    };
    (track, handle)
}
/// Slider value for a click at `cx` (AbstractSliderButton.setValueFromMouse).
pub fn slider_value(x: f32, w: f32, cx: f32) -> f32 {
    ((cx - (x + 4.0)) / (w - 8.0)).clamp(0.0, 1.0)
}
/// Screen.renderMenuBackground: in a world the translucent dark texture,
/// otherwise the dirt-like menu texture, tiled at 32 pixels.
pub fn menu_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_background"
    } else {
        "gui/menu_background"
    };
    if atlas.sprites.contains_key(sprite) {
        tiled(batch, atlas, sprite, x, y, w, h, 32.0, u32::MAX);
    } else {
        batch.fill(atlas, x, y, w, h, rgba(0, 0, 0, 160));
    }
}
/// Header and footer separators of list screens.
pub fn separators(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    header_bottom: f32,
    footer_top: f32,
    in_world: bool,
) {
    let (header, footer) = if in_world {
        (
            "gui/inworld_header_separator",
            "gui/inworld_footer_separator",
        )
    } else {
        ("gui/header_separator", "gui/footer_separator")
    };
    let w = batch.width;
    tiled_sized(
        batch,
        atlas,
        header,
        0.0,
        header_bottom - 2.0,
        w,
        2.0,
        [32.0, 2.0],
        u32::MAX,
    );
    tiled_sized(
        batch,
        atlas,
        footer,
        0.0,
        footer_top,
        w,
        2.0,
        [32.0, 2.0],
        u32::MAX,
    );
}
/// List background between the separators.
pub fn list_background(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    top: f32,
    bottom: f32,
    in_world: bool,
) {
    let sprite = if in_world {
        "gui/inworld_menu_list_background"
    } else {
        "gui/menu_list_background"
    };
    tiled(
        batch,
        atlas,
        sprite,
        0.0,
        top,
        batch.width,
        bottom - top,
        32.0,
        u32::MAX,
    );
}
/// The visible scrollbar thumb, shared by painting and pointer input.
pub(crate) fn scrollbar_thumb(
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) -> Option<(f32, f32)> {
    let height = bottom - top;
    if content <= height {
        return None;
    }
    let bar = (height * height / content)
        .clamp(32.0, height - 8.0)
        .floor();
    let y = top + ((scroll / (content - height)) * (height - bar)).floor();
    Some((y, bar))
}

/// `AbstractScrollArea`'s scrolling of a list `top..bottom` holding `content`
/// pixels: the amount, the mouse wheel and the held scrollbar.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scroll {
    pub amount: f32,
    /// The pointer's last y while the scrollbar is held (`scrolling`).
    held: Option<f32>,
}
impl Scroll {
    fn max(top: f32, bottom: f32, content: f32) -> f32 {
        (content - (bottom - top)).max(0.0)
    }
    /// `setScrollAmount`: kept within the content.
    pub fn clamp(&mut self, top: f32, bottom: f32, content: f32) {
        self.amount = self.amount.clamp(0.0, Self::max(top, bottom, content));
    }
    /// `mouseScrolled`: `rate` pixels per wheel step.
    pub fn wheel(&mut self, delta: f32, rate: f32, top: f32, bottom: f32, content: f32) {
        self.amount -= delta * rate;
        self.clamp(top, bottom, content);
    }
    /// `updateScrolling`: a press on the scrollbar column at `bar_x` (when
    /// there is something to scroll) holds it.
    pub fn press(
        &mut self,
        (x, y): (f32, f32),
        bar_x: f32,
        top: f32,
        bottom: f32,
        content: f32,
    ) -> bool {
        let over = x >= bar_x && x <= bar_x + 6.0 && y >= top && y < bottom;
        self.held = (over && Self::max(top, bottom, content) > 0.0).then_some(y);
        self.held.is_some()
    }
    pub fn held(&self) -> bool {
        self.held.is_some()
    }
    /// `mouseDragged` while held: above or below the list jumps to an end,
    /// otherwise the thumb follows the pointer.
    pub fn drag(&mut self, y: f32, top: f32, bottom: f32, content: f32) {
        let Some(last) = self.held else { return };
        let max = Self::max(top, bottom, content);
        if y < top {
            self.amount = 0.0;
        } else if y > bottom {
            self.amount = max;
        } else {
            let height = bottom - top;
            let bar = ((height * height / content).trunc()).clamp(32.0, height - 8.0);
            self.amount += (y - last) * (max.max(1.0) / (height - bar)).max(1.0);
        }
        self.clamp(top, bottom, content);
        self.held = Some(y);
    }
    /// `onRelease`.
    pub fn release(&mut self) {
        self.held = None;
    }
}

/// Vertical scroll bar (AbstractScrollArea): background and scroller sprites.
pub fn scrollbar(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    x: f32,
    top: f32,
    bottom: f32,
    content: f32,
    scroll: f32,
) {
    let Some((y, bar)) = scrollbar_thumb(top, bottom, content, scroll) else {
        return;
    };
    let height = bottom - top;
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller_background",
        x,
        top,
        6.0,
        height,
        u32::MAX,
    );
    nine_slice(
        batch,
        atlas,
        "gui/sprites/widget/scroller",
        x,
        y,
        6.0,
        bar,
        u32::MAX,
    );
}

/// Screen tooltip (DefaultTooltipPositioner + TooltipRenderUtil): the
/// background and frame sprites around the lines, beside the cursor.
pub fn tooltip(batch: &mut HudBatch, atlas: &HudAtlas, lines: &[Text], cursor: (f32, f32)) {
    if lines.is_empty() {
        return;
    }
    crate::widgets::frame_tooltip(lines);
    let w = lines
        .iter()
        .map(|l| atlas.font.width_text(l))
        .fold(0.0, f32::max);
    let h = if lines.len() == 1 {
        8.0
    } else {
        10.0 * lines.len() as f32
    };
    let mut x = cursor.0 + 12.0;
    let mut y = cursor.1 - 12.0;
    if x + w > batch.width {
        x = (cursor.0 - 24.0 - w).max(4.0);
    }
    if y + h + 3.0 > batch.height {
        y = batch.height - h - 3.0;
    }
    nine_slice(
        batch,
        atlas,
        "gui/sprites/tooltip/background",
        x - 12.0,
        y - 12.0,
        w + 24.0,
        h + 24.0,
        u32::MAX,
    );
    nine_slice(
        batch,
        atlas,
        "gui/sprites/tooltip/frame",
        x - 12.0,
        y - 12.0,
        w + 24.0,
        h + 24.0,
        u32::MAX,
    );
    let mut ly = y;
    for (i, line) in lines.iter().enumerate() {
        batch.text_styled(atlas, line, x, ly, u32::MAX, true);
        ly += if i == 0 { 12.0 } else { 10.0 };
    }
}

/// The EditBox text field.
#[derive(Clone, Debug)]
pub struct EditBox {
    pub value: String,
    /// Cursor and selection anchor, in characters.
    pub cursor: usize,
    pub highlight: usize,
    display: usize,
    pub max_length: usize,
    pub bordered: bool,
    /// `MultiLineEditBox`: a book page keeps line breaks. A single-line box strips them.
    pub multiline: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub focused: bool,
    focused_at: Instant,
    /// Grey completion shown after the text (EditBox.setSuggestion).
    pub suggestion: Option<String>,
    pub text_color: u32,
    /// `getMessage`: what the narrator calls the box ("<message> edit box:
    /// <value>").
    pub message: String,
}
/// The focused text field of a screen that keeps its values as `String`s
/// (the world list's search, world names, seeds, and the server screens): the `EditBox` state that
/// outlives a frame, its cursor, selection and scroll.
#[derive(Clone, Debug)]
pub struct Caret {
    field: Option<crate::clipboard::FieldId>,
    edit: EditBox,
    /// The last click's time and field, for double clicks.
    last_click: Option<(Instant, crate::clipboard::FieldId)>,
    /// The button is held since a press on the field (`onDrag`).
    held: bool,
}
impl Default for Caret {
    fn default() -> Self {
        Self {
            field: None,
            edit: EditBox::new(0.0, 0.0, 200.0, 20.0),
            last_click: None,
            held: false,
        }
    }
}
impl Caret {
    /// The box for field `id` holding `value`. A field newly focused, or
    /// whose value changed elsewhere, starts with its cursor at the end.
    fn sync(&mut self, id: crate::clipboard::FieldId, value: &str, max: usize) -> &mut EditBox {
        self.edit.max_length = max;
        if self.field != Some(id) || self.edit.value != value {
            self.edit.value = value.to_owned();
            self.edit.move_cursor_to_end(false);
            if self.field != Some(id) {
                self.edit.display = 0;
                self.field = Some(id);
            }
        }
        &mut self.edit
    }
    /// Edit field `id`'s `value` through its box; the value is written back.
    pub fn edit<R>(
        &mut self,
        id: crate::clipboard::FieldId,
        value: &mut String,
        max: usize,
        f: impl FnOnce(&mut EditBox) -> R,
    ) -> R {
        let edit = self.sync(id, value, max);
        let result = f(edit);
        value.clone_from(&edit.value);
        result
    }
    /// A press on field `id` at GUI `x` (`EditBox.onClick`): a second press
    /// within 250 ms is a double click (`MouseHandler`).
    pub fn press(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        rect: [f32; 4],
        x: f32,
        shift: bool,
    ) {
        let now = Instant::now();
        let double = self
            .last_click
            .is_some_and(|(at, field)| field == id && now.duration_since(at).as_millis() < 250);
        self.last_click = Some((now, id));
        self.held = true;
        self.edit(id, value, max, |edit| {
            [edit.x, edit.y, edit.width, edit.height] = rect;
            edit.click(atlas, x, shift, double);
        });
    }
    /// The pointer moved with the button held after a press on field `id`.
    pub fn drag(
        &mut self,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &mut String, usize),
        x: f32,
    ) {
        if self.held && self.last_click.is_some_and(|(_, field)| field == id) {
            self.edit(id, value, max, |edit| edit.drag_to(atlas, x));
        }
    }
    /// The button was released: drags no longer select.
    pub fn release(&mut self) {
        self.held = false;
    }
    /// Draw field `id` at `rect`: the focused one with its caret, the others
    /// showing the end of their text.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        (id, value, max): (crate::clipboard::FieldId, &str, usize),
        rect: [f32; 4],
        focused: bool,
        focused_at: Instant,
    ) {
        let mut idle;
        let edit = if focused {
            self.sync(id, value, max)
        } else {
            idle = EditBox::new(0.0, 0.0, 0.0, 0.0);
            idle.max_length = max;
            idle.set_value(value);
            &mut idle
        };
        [edit.x, edit.y, edit.width, edit.height] = rect;
        edit.focused = focused;
        if edit.focused_at < focused_at {
            edit.focused_at = focused_at;
        }
        edit.draw(batch, atlas, None);
    }
}

/// `EditBox.extractWidgetRenderState` for Horizon's append-only fields,
/// unbordered and with the cursor at the end: the text from (x, y), the
/// highlight when Select All covers the value (`invert_text` as
/// `setInvertHighlightedTextColor`), and the cursor, blinking from when the
/// field took the focus (`TextCursorUtils.isCursorVisible`): an underscore,
/// or a bar once the value is `max_chars` long. `shadow` is `setTextShadow`.
#[allow(clippy::too_many_arguments)]
pub fn append_field(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    value: &str,
    (x, y, width): (f32, f32, f32),
    (color, shadow): (u32, bool),
    max_chars: usize,
    selected: bool,
    invert_text: bool,
    focused_ms: u128,
) {
    let mut cursor_x = x;
    if !value.is_empty() {
        batch.text(atlas, value, x, y, color, shadow);
        cursor_x += HudBatch::text_width(atlas, value) + 1.0;
    }
    let insert = char_len(value) >= max_chars;
    if insert {
        cursor_x -= 1.0;
    }
    if selected && !value.is_empty() {
        let right = x + width;
        let (x0, x1) = (cursor_x.min(right), (x - 1.0).min(right));
        batch.text_highlight(atlas, (x1, y - 1.0, x0 - x1, 11.0), invert_text);
    }
    if (focused_ms / 300).is_multiple_of(2) {
        if insert {
            batch.fill(atlas, cursor_x, y - 1.0, 1.0, 11.0, color);
        } else {
            batch.text(atlas, "_", cursor_x, y, color, shadow);
        }
    }
}
fn byte_index(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}
/// `EditBox.getWordPosition(dir, from)`: the character index `dir` words away
/// from `from`. Forward, past the rest of the word and its spaces; backward,
/// over any spaces to the start of the word.
pub fn word_position(value: &str, dir: isize, from: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let mut pos = from.min(chars.len());
    if dir < 0 {
        while pos > 0 && chars[pos - 1] == ' ' {
            pos -= 1;
        }
        while pos > 0 && chars[pos - 1] != ' ' {
            pos -= 1;
        }
    } else {
        while pos < chars.len() && chars[pos] != ' ' {
            pos += 1;
        }
        while pos < chars.len() && chars[pos] == ' ' {
            pos += 1;
        }
    }
    pos
}
/// `MultilineTextField.getPreviousWord` / `getNextWord`'s `beginIndex` (a book page): any
/// whitespace, line breaks included, separates words, and the search starts from the cursor
/// clamped to the last character, as vanilla's does.
pub fn multiline_word_position(value: &str, dir: isize, from: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    if chars.is_empty() {
        return 0;
    }
    let mut pos = from.min(chars.len() - 1);
    if dir < 0 {
        while pos > 0 && java_whitespace(chars[pos - 1]) {
            pos -= 1;
        }
        while pos > 0 && !java_whitespace(chars[pos - 1]) {
            pos -= 1;
        }
    } else {
        while pos < chars.len() && !java_whitespace(chars[pos]) {
            pos += 1;
        }
        while pos < chars.len() && java_whitespace(chars[pos]) {
            pos += 1;
        }
    }
    pos
}
/// `Character.isWhitespace`: Unicode spaces and line breaks but not the no-break spaces, plus
/// the ASCII separators U+001C to U+001F.
pub(crate) fn java_whitespace(c: char) -> bool {
    (c.is_whitespace() && !matches!(c, '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}'))
        || ('\u{1c}'..='\u{1f}').contains(&c)
}
/// `EditBox.deleteWords(dir)` for an append-only field, whose caret is at the
/// end: removes the word in that direction. True when the value changed.
pub fn delete_words(value: &mut String, dir: isize) -> bool {
    let len = char_len(value);
    let to = word_position(value, dir, len);
    if to == len {
        return false;
    }
    value.truncate(byte_index(value, to));
    true
}
impl crate::clipboard::Editable for EditBox {
    fn select_all(&mut self) {
        self.move_cursor_to_end(false);
        self.highlight = 0;
    }
    fn selected_text(&self) -> String {
        EditBox::selected_text(self)
    }
    fn insert(&mut self, text: &str) -> bool {
        let before = self.value.clone();
        self.insert_text(text);
        self.value != before
    }
}
impl EditBox {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            value: String::new(),
            cursor: 0,
            highlight: 0,
            display: 0,
            max_length: 32,
            bordered: true,
            multiline: false,
            x,
            y,
            width,
            height,
            focused: true,
            focused_at: Instant::now(),
            suggestion: None,
            text_color: rgba(224, 224, 224, 255),
            message: String::new(),
        }
    }
    pub fn inner_width(&self) -> f32 {
        if self.bordered {
            self.width - 8.0
        } else {
            self.width
        }
    }
    pub fn text_x(&self) -> f32 {
        if self.bordered { self.x + 4.0 } else { self.x }
    }
    pub fn text_y(&self) -> f32 {
        if self.bordered {
            self.y + ((self.height - 8.0) / 2.0).floor()
        } else {
            self.y
        }
    }
    pub fn reset_blink(&mut self) {
        self.focused_at = Instant::now();
    }
    /// The caret is showing (`EditBox`'s blink, on for 300 ms of every 600).
    pub fn cursor_visible(&self) -> bool {
        self.focused && (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2)
    }
    pub fn set_value(&mut self, value: &str) {
        self.value = value.chars().take(self.max_length).collect();
        self.move_cursor_to_end(false);
    }
    fn selected(&self) -> (usize, usize) {
        (
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        )
    }
    pub fn selected_text(&self) -> String {
        let (a, b) = self.selected();
        self.value.chars().skip(a).take(b - a).collect()
    }
    /// EditBox.insertText: replace the selection, respecting the maximum length.
    pub fn insert_text(&mut self, text: &str) {
        let (a, b) = self.selected();
        let filtered = crate::clipboard::filter_text(text, self.multiline);
        let room = self
            .max_length
            .saturating_sub(char_len(&self.value) - (b - a));
        let insert: String = filtered.chars().take(room).collect();
        let (ba, bb) = (byte_index(&self.value, a), byte_index(&self.value, b));
        self.value.replace_range(ba..bb, &insert);
        let pos = a + char_len(&insert);
        self.set_cursor(pos, false);
    }
    /// Delete `n` characters (negative: backwards) or the selection.
    pub fn delete_chars(&mut self, n: isize) {
        if self.cursor != self.highlight {
            self.insert_text("");
            return;
        }
        let target = (self.cursor as isize + n).clamp(0, char_len(&self.value) as isize) as usize;
        let (a, b) = (self.cursor.min(target), self.cursor.max(target));
        let (ba, bb) = (byte_index(&self.value, a), byte_index(&self.value, b));
        self.value.replace_range(ba..bb, "");
        self.set_cursor(a, false);
    }
    /// EditBox.getWordPosition from the cursor.
    pub fn word_position(&self, dir: isize) -> usize {
        self.word_position_from(dir, self.cursor)
    }
    /// EditBox.getWordPosition(dir, from), or MultilineTextField's word for a book page.
    fn word_position_from(&self, dir: isize, from: usize) -> usize {
        if self.multiline {
            multiline_word_position(&self.value, dir, from)
        } else {
            word_position(&self.value, dir, from)
        }
    }
    pub fn set_cursor(&mut self, pos: usize, select: bool) {
        self.cursor = pos.min(char_len(&self.value));
        if !select {
            self.highlight = self.cursor;
        }
        self.scroll_to(self.cursor);
        self.reset_blink();
    }
    pub fn move_cursor_to_end(&mut self, select: bool) {
        self.set_cursor(char_len(&self.value), select);
    }
    /// EditBox.findClickedPositionInText: the character boundary under GUI `x`.
    pub fn position_at(&mut self, atlas: &HudAtlas, x: f32) -> usize {
        self.layout(atlas);
        let room = (x.floor() - self.text_x()).min(self.inner_width());
        let rest: String = self.value.chars().skip(self.display).collect();
        self.display + atlas.font.fit(&rest, room)
    }
    /// EditBox.onClick: the cursor to the pointer, extending the selection
    /// with Shift; a double click selects the word under it (`selectWord`).
    pub fn click(&mut self, atlas: &HudAtlas, x: f32, shift: bool, double: bool) {
        let pos = self.position_at(atlas, x);
        if double {
            let (start, end) = (
                self.word_position_from(-1, pos),
                self.word_position_from(1, pos),
            );
            self.set_cursor(start, false);
            self.set_cursor(end, true);
        } else {
            self.set_cursor(pos, shift);
        }
    }
    /// `MultilineTextField.selectWordAtCursor` on a book page: the word before
    /// the cursor is selected, its end found past non-whitespace only
    /// (`getWordEndPosition`).
    pub fn select_word_at_cursor(&mut self) {
        let begin = multiline_word_position(&self.value, -1, self.cursor);
        let end = begin
            + self
                .value
                .chars()
                .skip(begin)
                .take_while(|c| !java_whitespace(*c))
                .count();
        self.set_cursor(begin, false);
        self.set_cursor(end, true);
    }
    /// EditBox.onDrag: the selection follows the pointer.
    pub fn drag_to(&mut self, atlas: &HudAtlas, x: f32) {
        let pos = self.position_at(atlas, x);
        self.set_cursor(pos, true);
    }
    fn scroll_to(&mut self, pos: usize) {
        // EditBox.scrollTo without font access is completed in `layout`.
        self.display = self.display.min(char_len(&self.value));
        if pos < self.display {
            self.display = pos;
        }
    }
    fn layout(&mut self, atlas: &HudAtlas) {
        let len = char_len(&self.value);
        self.display = self.display.min(len);
        let (value, inner) = (self.value.clone(), self.inner_width());
        let visible = |d: usize| -> usize {
            let rest: String = value.chars().skip(d).collect();
            atlas.font.fit(&rest, inner)
        };
        while self.cursor > self.display + visible(self.display) && self.display < len {
            self.display += 1;
        }
        if self.cursor < self.display {
            self.display = self.cursor;
        }
    }
    /// Keyboard editing shared by every text field. Returns true if handled.
    pub fn key(&mut self, key: winit::keyboard::KeyCode, ctrl: bool, shift: bool) -> bool {
        use winit::keyboard::KeyCode as K;
        match key {
            K::Backspace => {
                let n = if ctrl {
                    self.word_position(-1) as isize - self.cursor as isize
                } else {
                    -1
                };
                self.delete_chars(n);
            }
            K::Delete => {
                let n = if ctrl {
                    self.word_position(1) as isize - self.cursor as isize
                } else {
                    1
                };
                self.delete_chars(n);
            }
            K::ArrowLeft => {
                let p = if ctrl {
                    self.word_position(-1)
                } else {
                    self.cursor.saturating_sub(1)
                };
                self.set_cursor(p, shift);
            }
            K::ArrowRight => {
                let p = if ctrl {
                    self.word_position(1)
                } else {
                    self.cursor + 1
                };
                self.set_cursor(p, shift);
            }
            K::Home => self.set_cursor(0, shift),
            K::End => self.move_cursor_to_end(shift),
            K::KeyA if ctrl => {
                self.move_cursor_to_end(false);
                self.highlight = 0;
            }
            _ => return false,
        }
        true
    }
    /// Ctrl+A/C/V/X (`EditBox.keyPressed`). True when the value changed.
    pub fn shortcut(
        &mut self,
        shortcut: crate::clipboard::EditShortcut,
        clipboard: &mut dyn crate::clipboard::Clipboard,
    ) -> bool {
        let before = self.value.clone();
        crate::clipboard::apply(self, shortcut, clipboard);
        self.reset_blink();
        self.value != before
    }
    /// The first shown character after scrolling to the cursor, and the
    /// milliseconds since the field was focused (the cursor's blink).
    pub fn view_state(&mut self, atlas: &HudAtlas) -> (usize, u64) {
        self.layout(atlas);
        (self.display, self.focused_at.elapsed().as_millis() as u64)
    }
    /// Draw the field; `format` styles the visible text (command colouring).
    pub fn draw(&mut self, batch: &mut HudBatch, atlas: &HudAtlas, format: Option<&Text>) {
        self.layout(atlas);
        crate::widgets::target_as(
            [self.x, self.y, self.width, self.height],
            crate::widgets::Kind::Field,
        );
        crate::widgets::narratable(|| {
            crate::widgets::Narratable::edit_box(
                self.message.clone(),
                self.value.clone(),
                [self.x, self.y, self.width, self.height],
            )
            .focused(self.focused)
        });
        if self.bordered {
            let sprite = if self.focused {
                "gui/sprites/widget/text_field_highlighted"
            } else {
                "gui/sprites/widget/text_field"
            };
            nine_slice(
                batch,
                atlas,
                sprite,
                self.x,
                self.y,
                self.width,
                self.height,
                u32::MAX,
            );
        }
        let color = self.text_color;
        let rest: String = self.value.chars().skip(self.display).collect();
        let shown_len = atlas.font.fit(&rest, self.inner_width());
        let displayed: String = rest.chars().take(shown_len).collect();
        let rel_cursor = self.cursor as isize - self.display as isize;
        let on_screen = rel_cursor >= 0 && rel_cursor as usize <= shown_len;
        let show_cursor = self.focused
            && (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2)
            && on_screen;
        let styled = |from: usize, to: usize| -> Text {
            match format {
                Some(t) => t.slice(self.display + from, self.display + to),
                None => Text::literal(
                    displayed
                        .chars()
                        .skip(from)
                        .take(to - from)
                        .collect::<String>(),
                ),
            }
        };
        let (tx, ty) = (self.text_x(), self.text_y());
        let mut draw_x = tx;
        if !displayed.is_empty() {
            let half = if on_screen {
                rel_cursor as usize
            } else {
                shown_len
            };
            let t = styled(0, half);
            batch.text_styled(atlas, &t, draw_x, ty, color, true);
            draw_x += atlas.font.width_text(&t) + 1.0;
        }
        let insert =
            self.cursor < char_len(&self.value) || char_len(&self.value) >= self.max_length;
        let mut cursor_x = draw_x;
        if !on_screen {
            cursor_x = if rel_cursor > 0 { tx + self.width } else { tx };
        } else if insert {
            cursor_x -= 1.0;
            draw_x -= 1.0;
        }
        if !displayed.is_empty() && on_screen && (rel_cursor as usize) < shown_len {
            batch.text_styled(
                atlas,
                &styled(rel_cursor as usize, shown_len),
                draw_x,
                ty,
                color,
                true,
            );
        }
        if !insert && let Some(s) = &self.suggestion {
            batch.text(atlas, s, cursor_x - 1.0, ty, rgba(128, 128, 128, 255), true);
        }
        let rel_highlight =
            (self.highlight as isize - self.display as isize).clamp(0, shown_len as isize) as usize;
        if rel_highlight as isize != rel_cursor {
            let hx = tx
                + atlas
                    .font
                    .width(&displayed.chars().take(rel_highlight).collect::<String>());
            // textHighlight between the cursor and the anchor, clipped at the right edge.
            let right = self.x + self.width;
            let (x0, x1) = (cursor_x.min(right), (hx - 1.0).min(right));
            let (a, b) = (x0.min(x1), x0.max(x1));
            batch.text_highlight(atlas, (a, ty - 1.0, b - a, 11.0), true);
        }
        if show_cursor {
            if insert {
                batch.fill(atlas, cursor_x, ty - 1.0, 1.0, 11.0, color);
            } else {
                batch.text(atlas, "_", cursor_x, ty, color, true);
            }
        }
        if self.focused {
            crate::preedit::report_caret(cursor_x, ty, 10.0);
        }
    }
    /// Character index under GUI x (mouse click).
    pub fn index_at(&self, atlas: &HudAtlas, x: f32) -> usize {
        let rest: String = self.value.chars().skip(self.display).collect();
        self.display + atlas.font.fit(&rest, (x - self.text_x()).max(0.0))
    }
    /// GUI x of a character index (EditBox.getScreenX).
    pub fn screen_x(&self, atlas: &HudAtlas, index: usize) -> f32 {
        if index > char_len(&self.value) {
            return self.x;
        }
        let shown: String = self
            .value
            .chars()
            .skip(self.display)
            .take(index.saturating_sub(self.display))
            .collect();
        self.text_x() + atlas.font.width(&shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quads' destination rectangles and their UVs.
    fn quads(batch: &HudBatch) -> Vec<([f32; 4], [f32; 4])> {
        batch
            .vertices
            .chunks(6)
            .map(|q| {
                let (a, c) = (q[0], q[2]);
                (
                    [a.pos[0], a.pos[1], c.pos[0], c.pos[1]],
                    [a.uv[0], a.uv[1], c.uv[0], c.uv[1]],
                )
            })
            .collect()
    }
    /// Destination and UV rectangles equal to within f32 rounding.
    fn assert_quad(actual: ([f32; 4], [f32; 4]), expected: ([f32; 4], [f32; 4])) {
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(
            close(actual.0, expected.0) && close(actual.1, expected.1),
            "{actual:?} != {expected:?}"
        );
    }
    /// A sprite filling the atlas, `w x h` texels as a 32x pack draws it.
    fn sprite(w: u32, h: u32) -> Rect {
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w,
            h,
        }
    }
    fn faithful(json: &str) -> NineSlice {
        parse_nine_slice(&serde_json::from_str(json).unwrap()).unwrap()
    }
    #[test]
    fn centered_label_matches_vanilla_integer_center() {
        // Even box, odd text: (w - width) / 2 floors one pixel left of w/2 - width/2.
        assert_eq!(centered_left(0.0, 200.0, 9.0), 96.0);
        assert_eq!(centered_left(0.0, 200.0, 8.0), 96.0);
        assert_eq!(centered_left(10.0, 196.0, 9.0), 104.0);
        assert_eq!(centered_left(2.0, 196.0, 7.0), 97.0);
    }
    #[test]
    fn slider_track_is_outlined_only_while_dragged() {
        let (track, handle) = slider_sprites(true, true, false);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle_highlighted");
        let (track, handle) = slider_sprites(true, true, true);
        assert_eq!(track, "gui/sprites/widget/slider_highlighted");
        assert_eq!(handle, "gui/sprites/widget/slider_handle_highlighted");
        let (track, handle) = slider_sprites(false, true, false);
        assert_eq!(track, "gui/sprites/widget/slider");
        assert_eq!(handle, "gui/sprites/widget/slider_handle");
    }
    #[test]
    fn separator_tiles_sample_both_rows_and_clip_the_last_column() {
        let mut batch = HudBatch::new([640, 480]);
        tile(
            &mut batch,
            sprite(32, 2),
            [32.0, 2.0],
            [0.0, 0.0, 32.0, 2.0],
            [0.0, 5.0, 40.0, 2.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 2);
        assert_quad(q[0], ([0.0, 5.0, 32.0, 7.0], [0.0, 0.0, 1.0, 1.0]));
        assert_quad(q[1], ([32.0, 5.0, 40.0, 7.0], [0.0, 0.0, 0.25, 1.0]));
    }
    #[test]
    fn a_border_that_leaves_no_centre_slice_is_no_metadata() {
        // NineSlice.validate rejects 12 + 12 >= 20, so the sprite has none.
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "nine_slice", "width": 20, "height": 20, "border": 12}}});
        assert!(parse_nine_slice(&json).is_none());
        // 9 + 9 < 20 still leaves a centre slice.
        let json = serde_json::json!({"gui": {"scaling": {
            "type": "nine_slice", "width": 20, "height": 20, "border": 9}}});
        assert_eq!(parse_nine_slice(&json).unwrap().border, [9; 4]);
    }
    #[test]
    fn a_high_resolution_button_slices_in_its_gui_units() {
        // Faithful 32x's widget/button: 400x40 texels, gui.scaling 200x20.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":200,"height":20,
                "border":{"left":20,"top":4,"right":20,"bottom":4}}}}"#,
        );
        let mut batch = HudBatch::new([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(400, 40),
            n,
            [10.0, 5.0, 150.0, 20.0],
            u32::MAX,
        );
        let q = quads(&batch);
        // Full height: left border, the centre tiled, right border
        // (blitNineSlicedSprite's height == nineSlice.height case).
        assert_quad(q[0], ([10.0, 5.0, 30.0, 25.0], [0.0, 0.0, 0.1, 1.0]));
        assert_quad(q[1], ([30.0, 5.0, 140.0, 25.0], [0.1, 0.0, 0.65, 1.0]));
        assert_quad(q[2], ([140.0, 5.0, 160.0, 25.0], [0.9, 0.0, 1.0, 1.0]));
        assert_eq!(q.len(), 3);
    }
    #[test]
    fn nine_slice_corners_and_a_stretched_centre() {
        // Faithful 32x's tooltip/frame: 200x200 texels, 100x100, border 12, stretched.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":100,"height":100,"border":12,"stretch_inner":true}}}"#,
        );
        let mut batch = HudBatch::new([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(200, 200),
            n,
            [0.0, 0.0, 300.0, 40.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 9);
        // Top left corner: 12 GUI pixels, 12/100 of the sprite.
        assert_quad(q[0], ([0.0, 0.0, 12.0, 12.0], [0.0, 0.0, 0.12, 0.12]));
        // The centre is one stretched quad (blitNineSliceInnerSegment, stretch_inner).
        assert_quad(q[7], ([12.0, 12.0, 288.0, 28.0], [0.12, 0.12, 0.88, 0.88]));
        // Bottom right corner.
        assert_quad(q[5], ([288.0, 28.0, 300.0, 40.0], [0.88, 0.88, 1.0, 1.0]));
    }
    #[test]
    fn a_sprite_drawn_at_its_own_size_is_one_quad() {
        let n = NineSlice {
            width: 50,
            height: 50,
            border: [10; 4],
            stretch_inner: false,
        };
        let mut batch = HudBatch::new([640, 480]);
        draw_nine_slice(
            &mut batch,
            sprite(100, 100),
            n,
            [4.0, 4.0, 50.0, 50.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 1);
        assert_quad(q[0], ([4.0, 4.0, 54.0, 54.0], [0.0, 0.0, 1.0, 1.0]));
    }
    #[test]
    fn blit_sprite_nine_slices_a_sprite_whose_metadata_says_so() {
        // Vanilla's tooltip/frame: 100x100, border 10, stretched centre.
        let n = faithful(
            r#"{"gui":{"scaling":{"type":"nine_slice","width":100,"height":100,"border":10,"stretch_inner":true}}}"#,
        );
        let mut batch = HudBatch::new([640, 480]);
        draw_sprite(
            &mut batch,
            sprite(100, 100),
            Some(n),
            [10.0, 10.0, 160.0, 26.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 9);
        // The corners keep their 10 pixels instead of stretching with the box.
        assert_quad(q[0], ([10.0, 10.0, 20.0, 20.0], [0.0, 0.0, 0.1, 0.1]));
        assert_quad(q[5], ([160.0, 26.0, 170.0, 36.0], [0.9, 0.9, 1.0, 1.0]));
    }
    #[test]
    fn blit_sprite_stretches_a_sprite_without_metadata() {
        let mut batch = HudBatch::new([640, 480]);
        draw_sprite(
            &mut batch,
            sprite(9, 9),
            None,
            [0.0, 0.0, 27.0, 18.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_eq!(q.len(), 1);
        assert_quad(q[0], ([0.0, 0.0, 27.0, 18.0], [0.0, 0.0, 1.0, 1.0]));
    }
    #[test]
    fn borders_shrink_to_half_the_drawn_size() {
        let n = NineSlice {
            width: 20,
            height: 20,
            border: [8; 4],
            stretch_inner: true,
        };
        let mut batch = HudBatch::new([640, 480]);
        // 9 wide: borders of floor(9 / 2) = 4, as vanilla's integer division.
        draw_nine_slice(
            &mut batch,
            sprite(20, 20),
            n,
            [0.0, 0.0, 9.0, 30.0],
            u32::MAX,
        );
        let q = quads(&batch);
        assert_quad(q[0], ([0.0, 0.0, 4.0, 8.0], [0.0, 0.0, 0.2, 0.4]));
        assert_quad(q[2], ([5.0, 0.0, 9.0, 8.0], [0.8, 0.0, 1.0, 0.4]));
    }
    #[test]
    fn append_fields_highlight_the_whole_value_and_blink_their_cursor() {
        use crate::hud::Blend;
        let atlas = HudAtlas::build(Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let end = 10.0 + HudBatch::text_width(&atlas, "abc") + 1.0;
        let span = |b: &HudBatch, r: &std::ops::Range<u32>| {
            let v = &b.vertices[r.start as usize..r.end as usize];
            let xs = v.iter().map(|v| v.pos[0]);
            let ys = v.iter().map(|v| v.pos[1]);
            (
                xs.clone().fold(f32::MAX, f32::min),
                xs.fold(f32::MIN, f32::max),
                ys.clone().fold(f32::MAX, f32::min),
                ys.fold(f32::MIN, f32::max),
            )
        };
        // Select All in the creative search: inverted white, then blue added,
        // from one pixel before the text to the cursor, 11 high.
        let mut b = HudBatch::new([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 80.0),
            (u32::MAX, true),
            50,
            true,
            true,
            0,
        );
        let blends: Vec<_> = b.blends.iter().map(|(_, blend)| *blend).collect();
        assert_eq!(blends, vec![Blend::Invert, Blend::Additive]);
        for (r, _) in &b.blends {
            assert_eq!(span(&b, r), (9.0, end, 19.0, 30.0));
        }
        // The anvil keeps its text colour; 300 ms later the cursor is hidden.
        let mut b = HudBatch::new([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 103.0),
            (u32::MAX, true),
            50,
            true,
            false,
            300,
        );
        let blends: Vec<_> = b.blends.iter().map(|(_, blend)| *blend).collect();
        assert_eq!(blends, vec![Blend::Additive]);
        let quads = |ms| {
            let mut b = HudBatch::new([854, 480]);
            append_field(
                &mut b,
                &atlas,
                "abc",
                (10.0, 20.0, 103.0),
                (0, true),
                50,
                false,
                false,
                ms,
            );
            b.vertices.len()
        };
        assert!(quads(300) < quads(600));
        // A full field shows a bar on its last pixel instead of an underscore.
        let mut b = HudBatch::new([854, 480]);
        append_field(
            &mut b,
            &atlas,
            "abc",
            (10.0, 20.0, 80.0),
            (u32::MAX, true),
            3,
            false,
            true,
            0,
        );
        let bar = b.vertices.len() as u32 - 6..b.vertices.len() as u32;
        assert_eq!(span(&b, &bar), (end - 1.0, end, 19.0, 30.0));
    }
    #[test]
    fn held_scrollbars_follow_the_pointer_and_stop_at_the_ends() {
        // A 100-pixel list holding 400: the thumb is 25 -> 32 high, so a
        // pixel of drag scrolls 300 / 68 pixels (AbstractScrollArea.mouseDragged).
        let (top, bottom, content) = (0.0, 100.0, 400.0);
        let mut s = Scroll::default();
        assert!(!s.press((50.0, 10.0), 100.0, top, bottom, content));
        assert!(s.press((103.0, 10.0), 100.0, top, bottom, content));
        s.drag(20.0, top, bottom, content);
        assert!((s.amount - 10.0 * 300.0 / 68.0).abs() < 1e-3);
        s.drag(150.0, top, bottom, content);
        assert_eq!(s.amount, 300.0);
        s.drag(-5.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        s.release();
        s.drag(50.0, top, bottom, content);
        assert_eq!(s.amount, 0.0);
        // Nothing to scroll: the column doesn't take the press.
        assert!(!s.press((103.0, 10.0), 100.0, top, bottom, 90.0));
        s.wheel(-2.0, 18.0, top, bottom, content);
        assert_eq!(s.amount, 36.0);
    }
    #[test]
    fn edit_box_clicks_place_the_cursor_and_select() {
        let atlas = HudAtlas::build(Path::new("/nonexistent/horizon-test-assets")).unwrap();
        let mut e = EditBox::new(10.0, 0.0, 200.0, 20.0);
        e.set_value("hello world");
        let at = |e: &EditBox, s: &str| e.text_x() + atlas.font.width(s);
        // findClickedPositionInText: the characters left of the pointer.
        e.click(&atlas, at(&e, "hello "), false, false);
        assert_eq!((e.cursor, e.highlight), (6, 6));
        // Shift extends the selection from the anchor.
        e.click(&atlas, at(&e, "hello wor"), true, false);
        assert_eq!(e.selected_text(), "wor");
        // selectWord: the word and the spaces after it.
        e.click(&atlas, at(&e, "he"), false, true);
        assert_eq!(e.selected_text(), "hello ");
        // onDrag from a click: left of the text is its start.
        e.click(&atlas, at(&e, "hello "), false, false);
        e.drag_to(&atlas, e.text_x() - 5.0);
        assert_eq!((e.cursor, e.highlight), (0, 6));
    }
    #[test]
    fn edit_box_editing() {
        let mut e = EditBox::new(0.0, 0.0, 100.0, 20.0);
        e.max_length = 10;
        e.insert_text("hello world");
        assert_eq!(e.value, "hello worl");
        e.set_cursor(5, false);
        assert_eq!(e.word_position(-1), 0);
        assert_eq!(e.word_position(1), 6);
        e.delete_chars(-2);
        assert_eq!(e.value, "hel worl");
        e.set_cursor(0, false);
        e.set_cursor(3, true);
        assert_eq!(e.selected_text(), "hel");
        e.insert_text("é§x");
        assert_eq!(e.value, "éx worl");
        assert_eq!(e.cursor, 2);
    }
    #[test]
    fn word_position_steps_over_a_word_and_its_spaces() {
        assert_eq!(word_position("hello world", -1, 11), 6);
        assert_eq!(word_position("hello world", -1, 6), 0);
        assert_eq!(word_position("hello   world", -1, 13), 8);
        assert_eq!(word_position("hello   world", 1, 5), 8);
        // From inside a word: back to its start, forward past it and its spaces.
        assert_eq!(word_position("hello world", -1, 8), 6);
        assert_eq!(word_position("hello world", 1, 2), 6);
        // The ends, and a position past the end.
        assert_eq!(word_position("hello", -1, 0), 0);
        assert_eq!(word_position("hello", 1, 5), 5);
        assert_eq!(word_position("hi", -1, 99), 0);
        assert_eq!(word_position("hi", 1, 99), 2);
    }
    #[test]
    fn multiline_words_split_at_any_whitespace() {
        // Ctrl+Backspace at the end of `one\ntwo` deletes `two` only.
        assert_eq!(multiline_word_position("one\ntwo", -1, 7), 4);
        // From the end the search starts at the last character, as vanilla's does.
        assert_eq!(multiline_word_position("one\n", -1, 4), 0);
        assert_eq!(multiline_word_position("a b", -1, 3), 0);
        assert_eq!(multiline_word_position("one\n\ntwo", 1, 0), 5);
        assert_eq!(multiline_word_position("one\u{a0}two", 1, 0), 7);
        assert_eq!(multiline_word_position("", -1, 0), 0);
    }
    #[test]
    fn delete_words_removes_the_word_before_the_end() {
        let mut v = "stone bricks".to_string();
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "stone ");
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "");
        assert!(!delete_words(&mut v, -1));
        // Characters, not bytes.
        let mut v = "a é§x".to_string();
        assert!(delete_words(&mut v, -1));
        assert_eq!(v, "a ");
        // Nothing after the end to delete.
        let mut v = "stone".to_string();
        assert!(!delete_words(&mut v, 1));
        assert_eq!(v, "stone");
    }
}
