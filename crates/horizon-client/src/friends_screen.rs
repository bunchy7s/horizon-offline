//! Play with Friends: Horizon's own sharing of the open world behind an
//! invite code (`game/friends.rs`). The pause menu's Friends button, the icon
//! vanilla's PauseScreen already has, opens it. It is drawn the way a mod adds
//! a screen to vanilla: the menu background, a centred title and text,
//! vanilla's buttons and Done in the footer.
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer::wrap,
    text::Text,
    ui,
};

/// The English title: the fallback of [`title`], and what the screen's widgets are named by.
pub const TITLE: &str = "Play with Friends";
/// A string of this screen: the language table's, or the English text.
fn tr(key: &str, english: &str) -> String {
    crate::lang::get_or(key, english)
}
/// The title in the current language.
pub fn title() -> String {
    tr("horizon.friends.title", TITLE)
}
/// What the screen shows (game/friends.rs fills it).
#[derive(Clone, Debug, Default)]
pub struct FriendsView {
    /// A singleplayer world (not a server): sharing is offered.
    pub shown: bool,
    pub sharing: bool,
    pub code: Option<String>,
    /// Who can join and what to do when friends on the internet can't (or,
    /// before the code, what the game is doing).
    pub status: String,
    /// Friends on the internet can reach the host.
    pub reachable: bool,
    /// Where the troubleshooting is, when they can't.
    pub help: Option<String>,
    /// The players in the world besides the host.
    pub guests: Vec<String>,
    pub notice: Option<String>,
    /// The code was just copied.
    pub copied: bool,
}

/// A button's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Share,
    Copy,
    Stop,
    /// Open the troubleshooting page.
    Help,
    Done,
}

fn grey() -> u32 {
    rgba(160, 160, 160, 255)
}
const WHITE: u32 = u32::MAX;
fn hint() -> String {
    tr(
        "horizon.friends.hint",
        "Friends paste the code after the title screen's Friends button, in Direct Connection, or on the launcher's Play page.",
    )
}

struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    action: Action,
    active: bool,
}

/// The centred lines (text, colour, y) and the buttons.
fn layout(
    atlas: &HudAtlas,
    (w, h): (f32, f32),
    view: &FriendsView,
) -> (Vec<(String, u32, f32)>, Vec<Button>) {
    let width = (w - 40.0).min(310.0);
    let cx = (w / 2.0).floor();
    let mut lines = Vec::new();
    let mut y = 40.0;
    let add = |lines: &mut Vec<(String, u32, f32)>, text: &str, color: u32, y: &mut f32| {
        for line in wrap(text, width, atlas) {
            lines.push((line, color, *y));
            *y += 10.0;
        }
    };
    let mut buttons = Vec::new();
    if view.sharing {
        lines.push((
            tr("horizon.multiplayer.invite_code", "Invite Code"),
            grey(),
            y,
        ));
        y += 12.0;
        match &view.code {
            Some(code) => {
                for line in code_lines(atlas, code, width) {
                    lines.push((line, WHITE, y));
                    y += 10.0;
                }
            }
            None => add(&mut lines, &view.status, WHITE, &mut y),
        }
        y += 6.0;
        buttons.push(Button {
            x: cx - 154.0,
            y,
            w: 150.0,
            label: if view.copied {
                tr("horizon.friends.copied", "Copied")
            } else {
                tr("horizon.friends.copy_code", "Copy Code")
            },
            action: Action::Copy,
            active: view.code.is_some(),
        });
        buttons.push(Button {
            x: cx + 4.0,
            y,
            w: 150.0,
            label: tr("horizon.friends.stop_sharing", "Stop Sharing"),
            action: Action::Stop,
            active: true,
        });
        y += 30.0;
        if view.code.is_some() {
            // Who can join: grey when friends anywhere can, white (it asks
            // for something) when only the LAN and VPNs can.
            let tint = if view.reachable { grey() } else { WHITE };
            add(&mut lines, &view.status, tint, &mut y);
            y += 4.0;
        }
        let here = if view.guests.is_empty() {
            tr("horizon.friends.none_yet", "No friends here yet.")
        } else {
            tr("horizon.friends.here_now", "Here now: %s").replacen(
                "%s",
                &view.guests.join(", "),
                1,
            )
        };
        add(&mut lines, &here, grey(), &mut y);
        if let Some(notice) = &view.notice {
            add(&mut lines, notice, WHITE, &mut y);
        }
        // Where friends paste the code, while it fits above the footer.
        let hint = hint();
        if y + 10.0 * wrap(&hint, width, atlas).len() as f32 <= h - 33.0 {
            add(&mut lines, &hint, grey(), &mut y);
        }
    } else {
        add(
            &mut lines,
            &tr(
                "horizon.friends.share_intro",
                "Share this world behind an invite code: it opens to LAN, and friends on the internet join with the code. No server of ours is involved.",
            ),
            WHITE,
            &mut y,
        );
        y += 4.0;
        add(&mut lines, &hint(), grey(), &mut y);
        buttons.push(Button {
            x: cx - 100.0,
            y: y + 8.0,
            w: 200.0,
            label: tr("horizon.friends.start_sharing", "Start Sharing"),
            action: Action::Share,
            active: view.shown,
        });
    }
    // HeaderAndFooterLayout's footer: Done, beside Troubleshooting when
    // friends on the internet can't reach the host.
    if view.sharing && view.help.is_some() {
        buttons.push(Button {
            x: cx - 154.0,
            y: h - 27.0,
            w: 150.0,
            label: tr("horizon.friends.troubleshooting", "Troubleshooting..."),
            action: Action::Help,
            active: true,
        });
        buttons.push(Button {
            x: cx + 4.0,
            y: h - 27.0,
            w: 150.0,
            label: "gui.done".to_owned(),
            action: Action::Done,
            active: true,
        });
    } else {
        buttons.push(Button {
            x: cx - 100.0,
            y: h - 27.0,
            w: 200.0,
            label: "gui.done".to_owned(),
            action: Action::Done,
            active: true,
        });
    }
    (lines, buttons)
}
/// The code's groups packed into lines of at most `width`.
fn code_lines(atlas: &HudAtlas, code: &str, width: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for group in code.split('-') {
        match lines.last_mut() {
            Some(l) if HudBatch::text_width(atlas, &format!("{l}-{group}")) <= width => {
                l.push('-');
                l.push_str(group);
            }
            _ => lines.push(group.to_owned()),
        }
    }
    lines
}
fn label(key: &str) -> Text {
    if key.starts_with("gui.") {
        Text::translate(key, &[])
    } else {
        Text::literal(key)
    }
}
pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    cursor: (f32, f32),
    view: &FriendsView,
    in_world: bool,
) {
    let (w, h) = (batch.width, batch.height);
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
    let title = Text::literal(title());
    let tw = atlas.font.width_text(&title);
    batch.text_styled(atlas, &title, ((w - tw) / 2.0).floor(), 15.0, WHITE, true);
    let (lines, buttons) = layout(atlas, (w, h), view);
    for (line, color, y) in lines {
        let lw = HudBatch::text_width(atlas, &line);
        batch.text(atlas, &line, ((w - lw) / 2.0).floor(), y, color, true);
    }
    for b in buttons {
        let hover = b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0);
        ui::button(
            batch,
            atlas,
            b.x,
            b.y,
            b.w,
            20.0,
            &label(&b.label),
            b.active,
            hover,
        );
    }
}
/// The button under a click, if it is active.
pub fn click(
    atlas: &HudAtlas,
    size: (f32, f32),
    cursor: (f32, f32),
    view: &FriendsView,
) -> Option<Action> {
    layout(atlas, size, view)
        .1
        .into_iter()
        .find(|b| b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0))
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    #[test]
    fn share_then_copy_and_stop() {
        let atlas = atlas();
        let size = (427.0, 240.0);
        let idle = FriendsView {
            shown: true,
            ..FriendsView::default()
        };
        let (_, buttons) = layout(&atlas, size, &idle);
        let share = buttons.iter().find(|b| b.action == Action::Share).unwrap();
        assert_eq!(
            click(&atlas, size, (share.x + 1.0, share.y + 1.0), &idle),
            Some(Action::Share)
        );
        let sharing = FriendsView {
            shown: true,
            sharing: true,
            status: "Finding how friends can reach you…".into(),
            ..FriendsView::default()
        };
        let (_, buttons) = layout(&atlas, size, &sharing);
        let copy = buttons.iter().find(|b| b.action == Action::Copy).unwrap();
        assert!(!copy.active, "nothing to copy before the code");
        let with_code = FriendsView {
            code: Some("0B33K-Q405Q-1Z5CV-X6NHY-R12N0-VK8T1-60N00-RA134-952PE-1HA08-910P0-K5M01-ST1NB-JDTB6-K4W1P-0".into()),
            status: "Friends on your network or VPN can join. For friends on the internet: turn on UPnP on your router, or forward TCP port 25580 to 10.0.0.2, or use a relay, or a VPN such as Tailscale or ZeroTier (the code already holds your VPN address).".into(),
            help: Some("https://example.invalid/help".into()),
            ..sharing
        };
        let (lines, buttons) = layout(&atlas, size, &with_code);
        assert!(
            lines.iter().all(|(_, _, y)| y + 9.0 <= size.1 - 27.0),
            "the text stays above the footer"
        );
        for action in [Action::Copy, Action::Stop, Action::Help, Action::Done] {
            let b = buttons.iter().find(|b| b.action == action).unwrap();
            assert_eq!(
                click(&atlas, size, (b.x + 1.0, b.y + 1.0), &with_code),
                Some(action)
            );
        }
    }

    #[test]
    fn a_server_offers_no_sharing() {
        let atlas = atlas();
        let view = FriendsView::default();
        let (_, buttons) = layout(&atlas, (427.0, 240.0), &view);
        let share = buttons.iter().find(|b| b.action == Action::Share).unwrap();
        assert!(!share.active);
    }
}
