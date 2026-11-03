//! Sculk, a server host that sponsors Horizon (Ludovic, 2026-10-26): its wide banner on the Servers
//! page, and a card made for the sidebar on the other pages, where the banner's baked-in text was too
//! small to read. The pictures are in the binary, never fetched; a click opens Sculk's page in the
//! browser. A "Partner" label above each says it is an ad. After `SEEN_BEFORE_HIDE` seconds on screen
//! a "Hide" appears on either, and hiding one hides both for good.
use crate::theme::Theme;
use gpui::{
    div, img, prelude::*, px, rgb, ClickEvent, Div, FontWeight, HighlightStyle, Image, ImageFormat,
    Rgba, SharedString, Stateful, StyledText,
};
use std::sync::{Arc, LazyLock};

pub const URL: &str = "https://my.sculkhosting.com/ref/horizon";

/// How long the sidebar's banner stays on screen, counted over every start, before a "Hide" shows
/// beside its label: long enough to be seen, then the player may put it away for good.
pub const SEEN_BEFORE_HIDE: u64 = 180;

/// The small "Hide" beside a banner's or card's label, top right of whatever holds it (which must be
/// `relative`). The caller adds the click: it sets `Settings::partner_hidden`, which hides every
/// partner banner for good.
pub fn hide_button(id: &'static str, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let hide = div()
        .id(id)
        .absolute()
        .top(px(0.))
        .right(px(0.))
        .cursor_pointer()
        .child(
            crate::text(crate::theme::MICRO)
                .text_color(t.muted)
                .hover(move |s| s.text_color(t.ink))
                .child(horizon_launcher::i18n::tr("HIDE")),
        );
    crate::keyboard(hide, &t)
}

/// 1200×441: twice the widest it is shown at, for Retina screens. Its transparent edges let the
/// characters stand out of the card.
const WIDTH: f32 = 1200.;
const HEIGHT: f32 = 441.;
static BANNER: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Webp,
        include_bytes!("../assets/sculk-banner.webp").to_vec(),
    ))
});

/// The banner `width` logical pixels wide, under its label. Hovering lifts it a little and turns the
/// label to ink. The narrowest window (960) leaves the page 648 pixels, so the Servers one always fits.
pub fn banner(id: &'static str, width: f32, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let el = div()
        .id(id)
        .group(id)
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(width))
        .cursor_pointer()
        .child(
            crate::text(crate::theme::MICRO)
                .text_color(t.muted)
                .group_hover(id, move |s| s.text_color(t.ink))
                .child(horizon_launcher::i18n::tr("PARTNER")),
        )
        .child(
            div().relative().group_hover(id, |s| s.top(px(-2.))).child(
                img(BANNER.clone())
                    .w(px(width))
                    .h(px(width * HEIGHT / WIDTH)),
            ),
        )
        .on_click(|_: &ClickEvent, _, cx| cx.open_url(URL));
    crate::keyboard(el, &t)
}

/// Sculk's characters (Steve, Alex and the wolf) cropped from its banner at 2x the card's width.
/// Above `ART_TOP` (in the card's 228 logical pixels) the picture is transparent but for their heads
/// and the pickaxe, which stand out of the card; its foot and right edge fade into `CARD`.
const ART_WIDTH: f32 = 456.;
const ART_HEIGHT: f32 = 364.;
const ART_TOP: f32 = 11.;
static ART: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Webp,
        include_bytes!("../assets/sculk-card-art.webp").to_vec(),
    ))
});

/// Sculk's mark and name, cropped from its banner, 2x for a 18 pixel height.
const LOGO_WIDTH: f32 = 191.;
const LOGO_HEIGHT: f32 = 36.;
static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Webp,
        include_bytes!("../assets/sculk-logo.webp").to_vec(),
    ))
});

/// Sculk's colours, taken from its banner: the card is Sculk's in both themes, as an ad is.
const CARD: u32 = 0x0b0d12;
const CYAN: u32 = 0x5dd4fb;
const CYAN_HOVER: u32 = 0x8fe3fd;
const WHITE: u32 = 0xf1f5f9;
const GREY: u32 = 0x9aa6b2;
const ON_CYAN: u32 = 0x062330;

/// The promo code: Sculk's, never translated.
const CODE: &str = "HORIZON15";

/// What the tall card needs of the sidebar's height, label included, with room for a third line of
/// headline: below it the sidebar shows the compact card.
pub const TALL_HEIGHT: f32 = 410.;

/// "Your Minecraft server, 24/7 online." with "24/7 online" in Sculk's cyan, in the launcher's
/// language.
fn headline() -> StyledText {
    let online = horizon_launcher::i18n::tr("24/7 online");
    let line = horizon_launcher::i18n::tr("Your Minecraft server, {online}.");
    let (before, after) = line.split_once("{online}").unwrap_or((line, ""));
    let text = format!("{before}{online}{after}");
    let lit = before.len()..before.len() + online.len();
    StyledText::new(SharedString::from(text)).with_highlights([(
        lit,
        HighlightStyle {
            color: Some(rgb(CYAN).into()),
            ..Default::default()
        },
    )])
}

/// "HORIZON15 · 15% off" in a dashed cyan frame, as on Sculk's banner.
fn code_chip(size: f32, height: f32) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(height))
        .px(px(10.))
        .border_1()
        .border_dashed()
        .border_color(rgba(CYAN, 0.7))
        .rounded(px(6.))
        .font_family(crate::theme::MONO)
        .text_size(px(size))
        .whitespace_nowrap()
        .child(
            div()
                .text_color(rgb(WHITE))
                .font_weight(FontWeight::SEMIBOLD)
                .child(CODE),
        )
        .child(div().text_color(rgb(GREY)).child("·"))
        .child(
            div()
                .text_color(rgb(CYAN))
                .child(horizon_launcher::i18n::tr("15% off")),
        )
}

fn rgba(hex: u32, alpha: f32) -> Rgba {
    Rgba {
        a: alpha,
        ..rgb(hex)
    }
}

/// Sculk's card for the sidebar, `width` logical pixels wide: its characters standing out of the top,
/// then its name, the headline, a line, a "Get my server" button and the promo code, all drawn by
/// the launcher so they stay sharp and follow its language. `compact` puts the characters beside a
/// shorter text, for windows too short for the tall card. Hovering lifts it a little.
pub fn card(id: &'static str, width: f32, compact: bool, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let label = crate::text(crate::theme::MICRO)
        .text_color(t.muted)
        .group_hover(id, move |s| s.text_color(t.ink))
        .child(horizon_launcher::i18n::tr("PARTNER"));
    let face = div()
        .relative()
        .flex()
        .flex_none()
        .w(px(width))
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgba(CYAN, 0.22))
        .rounded(px(10.))
        .group_hover(id, |s| s.top(px(-2.)).border_color(rgba(CYAN, 0.5)));
    let logo = |height: f32| {
        img(LOGO.clone())
            .flex_none()
            .w(px(height * LOGO_WIDTH / LOGO_HEIGHT))
            .h(px(height))
    };
    let face = if compact {
        // The characters beside the text, their faded foot melting into the card, the code under both.
        let art_w = 104.;
        face.flex_col()
            .child(
                div()
                    .flex()
                    .items_end()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap(px(8.))
                            .pl(px(12.))
                            .pt(px(12.))
                            .child(logo(13.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .line_height(px(19.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(WHITE))
                                    .child(headline()),
                            ),
                    )
                    .child(
                        img(ART.clone())
                            .flex_none()
                            .w(px(art_w))
                            .h(px(art_w * ART_HEIGHT / ART_WIDTH)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .px(px(12.))
                    .pt(px(6.))
                    .pb(px(12.))
                    .child(code_chip(11., 26.).flex_1()),
            )
    } else {
        let art_h = width * ART_HEIGHT / ART_WIDTH;
        let button = div()
            .flex()
            .items_center()
            .justify_center()
            .h(px(36.))
            .rounded(px(8.))
            .bg(rgb(CYAN))
            .group_hover(id, |s| s.bg(rgb(CYAN_HOVER)))
            .text_color(rgb(ON_CYAN))
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(horizon_launcher::i18n::tr("Get my server"));
        // The art stands out of the card's top; the text starts over its faded foot.
        face.mt(px(ART_TOP))
            .flex_col()
            .child(
                img(ART.clone())
                    .absolute()
                    .top(px(-ART_TOP))
                    .left_0()
                    .w(px(width))
                    .h(px(art_h)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(art_h - ART_TOP - 14.))
                    .px(px(16.))
                    .pb(px(16.))
                    .child(logo(16.))
                    .child(
                        div()
                            .mt(px(12.))
                            .text_size(px(19.))
                            .line_height(px(23.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(WHITE))
                            .child(headline()),
                    )
                    .child(
                        div()
                            .mt(px(6.))
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .text_color(rgb(GREY))
                            .child(horizon_launcher::i18n::tr("Ready in under a minute")),
                    )
                    .child(div().mt(px(16.)).child(button))
                    .child(div().mt(px(8.)).flex().child(code_chip(12., 30.).flex_1())),
            )
    };
    let el = div()
        .id(id)
        .group(id)
        .flex()
        .flex_col()
        .gap(px(8.))
        .w(px(width))
        .cursor_pointer()
        .child(label)
        .child(face)
        .on_click(|_: &ClickEvent, _, cx| cx.open_url(URL));
    crate::keyboard(el, &t)
}

#[cfg(test)]
mod tests {
    use super::{ART_HEIGHT, ART_WIDTH, HEIGHT, LOGO_HEIGHT, LOGO_WIDTH, WIDTH};

    /// Each bundled picture decodes as a WebP of the size the code lays it out at.
    #[test]
    fn the_card_pictures_decode() {
        for (bytes, size) in [
            (
                &include_bytes!("../assets/sculk-card-art.webp")[..],
                (ART_WIDTH, ART_HEIGHT),
            ),
            (
                &include_bytes!("../assets/sculk-logo.webp")[..],
                (LOGO_WIDTH, LOGO_HEIGHT),
            ),
        ] {
            let pic = image::load_from_memory_with_format(bytes, image::ImageFormat::WebP)
                .expect("a WebP");
            assert_eq!((pic.width(), pic.height()), (size.0 as u32, size.1 as u32));
        }
    }

    #[test]
    fn the_bundled_banner_decodes() {
        let bytes = include_bytes!("../assets/sculk-banner.webp");
        let banner = image::load_from_memory_with_format(bytes, image::ImageFormat::WebP)
            .expect("the banner is a WebP");
        assert_eq!(
            (banner.width(), banner.height()),
            (WIDTH as u32, HEIGHT as u32)
        );
    }
}
