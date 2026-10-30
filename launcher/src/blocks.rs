//! The blocky look's parts (`docs/design-system.md`, "Blocks"): pixel type, pixel icons, panels
//! with a dark outline, a light bevel on top and a darker lip below, sunk wells, badges, the
//! inventory tabs, and the slots that hold icons downloaded at run time.
use crate::theme::{self, Theme};
use gpui::{
    div, img, prelude::*, px, svg, AnyElement, App, ClickEvent, Div, FontWeight, Hsla, ImageSource,
    ObjectFit, SharedString, Stateful, Svg, Window,
};
use std::borrow::Cow;

/// The icons, served to `svg()` as `icons/<name>.svg`.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path
            .strip_prefix("icons/")
            .and_then(|p| p.strip_suffix(".svg"));
        Ok(name.and_then(horizon_theme::icons::get).map(Cow::Borrowed))
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(horizon_theme::icons::ALL
            .iter()
            .map(|(name, _)| format!("icons/{name}.svg").into())
            .collect())
    }
}

/// A pixel icon (`horizon_theme::icons`) in `color`.
pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(format!("icons/{name}.svg"))
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// Text in the pixel face: headings, navigation, buttons, badges.
pub fn pixel(size: f32, weight: FontWeight) -> Div {
    div()
        .font_family(theme::PIXEL)
        .font_weight(weight)
        .text_size(px(size))
        .line_height(px((size * 1.2).round()))
}

/// The light edge along the top of a raised block and the darker lip along its bottom; the
/// block must be `relative`.
pub fn edges(top: Hsla, bottom: Hsla, lip: f32) -> [AnyElement; 2] {
    [
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(theme::BEVEL))
            .bg(top)
            .into_any_element(),
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(lip))
            .bg(bottom)
            .into_any_element(),
    ]
}

/// A panel: rows, cards, notices, dialogs.
pub fn panel(t: &Theme) -> Div {
    div()
        .relative()
        .bg(t.panel)
        .border_2()
        .border_color(t.outline)
        .children(edges(t.bevel, t.lip, theme::BEVEL))
}

/// A small label in a block of its colour: "Runs", "Partly", a version.
pub fn badge(label: impl Into<SharedString>, color: Hsla, t: &Theme) -> Div {
    let tint = if t.dark { 0.14 } else { 0.12 };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(22.))
        .px(px(8.))
        .bg(color.opacity(tint))
        .border_2()
        .border_color(t.outline)
        .child(div().size(px(6.)).bg(color))
        .child(
            pixel(12., FontWeight::MEDIUM)
                .text_color(color)
                .child(label.into()),
        )
}

/// Inventory tabs: a row of tabs on top of a panel, the open one joined to it.
pub fn tabs<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Div {
    let t = *t;
    div()
        .flex()
        .items_end()
        .gap(px(4.))
        .children(options.iter().map(|&(value, label, icon_name)| {
            let open = current == value;
            let pick = on_pick.clone();
            let tab = div()
                .id(SharedString::from(format!("{id}-{label}")))
                .relative()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(if open { 42. } else { 38. }))
                .px(px(14.))
                .border_2()
                .border_b_0()
                .border_color(t.outline)
                .cursor_pointer()
                .bg(if open { t.panel } else { t.side })
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(theme::BEVEL))
                        .bg(if open { t.bevel } else { t.raised }),
                )
                .child(icon(icon_name, 18., if open { t.accent } else { t.muted }))
                .child(
                    pixel(15., FontWeight::MEDIUM)
                        .text_color(if open { t.ink } else { t.muted })
                        .child(label),
                )
                .when(!open, |d| d.hover(move |s| s.bg(t.raised)))
                // The open tab covers the panel's outline under it.
                .when(open, |d| d.mb(px(-2.)))
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx));
            crate::keyboard(tab, &t)
        }))
}

/// A square slot for an icon: the picture at `source` when there is one, else the first letter of
/// `name` on a raised block.
pub fn slot(source: Option<ImageSource>, name: &str, size: f32, t: &Theme) -> Div {
    let frame = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .overflow_hidden()
        .border_2()
        .border_color(t.outline)
        .bg(t.well);
    match source {
        Some(source) => frame.child(img(source).size(px(size - 4.)).object_fit(ObjectFit::Cover)),
        None => frame.bg(t.raised).child(
            pixel((size * 0.42).round(), FontWeight::BOLD)
                .text_color(t.muted)
                .child(
                    name.chars()
                        .find(|c| c.is_alphanumeric())
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                ),
        ),
    }
}

/// A clickable block in the sidebar: an instance or a server.
pub fn side_item(id: SharedString, selected: bool, t: &Theme) -> Stateful<Div> {
    let t = *t;
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(px(10.))
        .p(px(6.))
        .cursor_pointer()
        .when(selected, |d| {
            d.bg(t.raised)
                .children(edges(t.bevel, t.lip, theme::BEVEL))
                .child(
                    div()
                        .absolute()
                        .left(px(-10.))
                        .top(px(8.))
                        .bottom(px(8.))
                        .w(px(4.))
                        .bg(t.accent),
                )
        })
        .when(!selected, |d| d.hover(move |s| s.bg(t.raised.opacity(0.6))))
}
