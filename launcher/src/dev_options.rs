//! Developer options: a Settings section nothing in the window points to. Seven clicks on the
//! sidebar's "Launcher x.y.z" reveal it for this run; it stays shown while one of its options is
//! on. Its one option lets offline names play without an account that owns the game
//! (`backend::Settings::dev_offline`). The game still comes from the player's own install: the
//! launcher downloads Mojang's files only for an owner (`backend::download_for_owner`).
use super::*;

/// Clicks on the launcher's version that reveal the section.
const CLICKS: u8 = 7;

impl Launcher {
    /// A click on the sidebar's launcher version; the last one opens Settings on the section.
    pub(super) fn version_clicked(&mut self) {
        if self.dev_options {
            return;
        }
        self.dev_clicks += 1;
        if self.dev_clicks >= CLICKS {
            self.dev_options = true;
            self.go(Page::Settings);
        }
    }

    /// The section, when revealed or when one of its options is on.
    pub(super) fn dev_options_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.dev_options && !self.settings.dev_offline {
            return None;
        }
        let t = self.theme;
        let toggle = switch("dev-offline", self.settings.dev_offline, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.dev_offline = !this.settings.dev_offline;
                this.settings.save();
                cx.notify();
            },
        ));
        Some(
            div()
                .flex()
                .flex_col()
                .child(
                    section_title("Developer options", "terminal", &t)
                        .mt(px(20.))
                        .mb(px(12.)),
                )
                .child(index_row(
                    0,
                    tr("Offline names without an account"),
                    Some(
                        div()
                            .child(tr(
                                "Offline names play without a Microsoft account that owns the game. \
                                 Minecraft still comes from your own install.",
                            ))
                            .into_any_element(),
                    ),
                    Some(toggle.into_any_element()),
                    &t,
                )),
        )
    }
}
