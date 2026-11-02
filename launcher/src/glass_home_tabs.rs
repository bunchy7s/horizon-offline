//! Home's tabbed panel in the glass look (the mockup's `TabbedPanel` and `Overview`,
//! `launcher-views.tsx`): one glass plate whose top-left corner is the tab strip, and under it
//! the open tab. Overview shows the instance's last screenshots beside the newest patch log,
//! then what Play still needs (the launcher's rows). Dark appearance only.
use super::*;
use crate::glass_ui::{self as ui, white};
use home_tabs::HomeTab;
use horizon_theme::glass::{ink, radius};

/// The tabbed panel's own body is at least this tall (`min-h-[340px]`), and so is the patch log.
const BODY: f32 = 340.;

impl Launcher {
    /// The tabbed panel: the strip, then the open tab. `rows`: what Overview shows under its
    /// screenshots and patch log (what Play needs, the open export or launch panel).
    pub(super) fn glass_home_tabs(&mut self, rows: Div, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let tabs = self.home_tab_list(cx);
        let open = tabs
            .iter()
            .map(|(tab, _)| *tab)
            .find(|tab| *tab == self.home_tab)
            .unwrap_or(HomeTab::Overview);
        let strip = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .pt(px(8.))
            .pb(px(4.))
            .child(crate::glass::measure(self.home_strip.clone()))
            .children(tabs.into_iter().map(|(tab, count)| {
                let (name, icon) = match tab {
                    HomeTab::Overview => (tr("Overview"), Some("dashboard-square-01")),
                    HomeTab::Mods => (tr("Mods"), Some("puzzle")),
                    HomeTab::Worlds => (tr("Worlds"), Some("earth")),
                    HomeTab::Multiplayer => (tr("Multiplayer"), Some("user-multiple-02")),
                    HomeTab::Packs => (tr("Packs"), None),
                    HomeTab::Screenshots => (tr("Screenshots"), None),
                    HomeTab::Console => (tr("Console"), Some("computer-terminal-01")),
                };
                ui::count_tab(
                    format!("home-tab-{tab:?}"),
                    icon,
                    name,
                    count.filter(|n| *n > 0),
                    tab == open,
                    &t,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.home_tab = tab;
                    cx.notify();
                }))
            }));
        // The open tab's own buttons head its body, at the right.
        let (actions, body): (Option<Div>, Option<AnyElement>) = match open {
            HomeTab::Overview => (None, Some(self.glass_overview(rows, cx).into_any_element())),
            HomeTab::Mods => {
                let (actions, body) = self.instance_mods_tab(cx);
                (Some(actions), Some(body.into_any_element()))
            }
            HomeTab::Worlds => (
                None,
                self.glass_worlds(cx).map(IntoElement::into_any_element),
            ),
            HomeTab::Multiplayer => (
                None,
                self.glass_multiplayer(cx)
                    .map(IntoElement::into_any_element),
            ),
            HomeTab::Packs => split(self.packs_section(cx)),
            HomeTab::Screenshots => split(self.screenshots_section(cx)),
            HomeTab::Console => split(self.console_section(cx)),
        };
        div()
            .relative()
            // `mt-12` under the head, of which Home's own gap gives 28.
            .mt(px(20.))
            .child(crate::glass::tabbed(self.home_strip.clone()))
            .child(div().relative().flex().child(strip))
            .child(
                div().relative().p(px(8.)).child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .min_h(px(BODY))
                        .px(px(4.))
                        .pt(px(8.))
                        .pb(px(4.))
                        .children(actions.map(|a| div().flex().justify_end().px(px(12.)).child(a)))
                        .children(body),
                ),
            )
    }

    /// Overview: the last screenshots and the patch log side by side, then `rows`.
    fn glass_overview(&mut self, rows: Div, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(
                div()
                    .flex()
                    .gap(px(20.))
                    .child(self.overview_shots(cx))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(px(380.))
                            .min_h(px(BODY))
                            .child(self.patch_log(cx)),
                    ),
            )
            .child(rows)
    }

    /// The instance's three newest screenshots (the first twice as big when there are three),
    /// "See all" opening the Screenshots tab; a line about F2 while there are none.
    fn overview_shots(&mut self, cx: &mut Context<Self>) -> Div {
        let count = self.screenshots_count();
        let shots: Vec<_> = self.screenshots.list.iter().take(3).cloned().collect();
        let see_all = || {
            cx.listener(
                |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                    this.home_tab = HomeTab::Screenshots;
                    cx.notify();
                },
            )
        };
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .h(px(32.))
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(card_title(tr("Screenshots")))
                    .children((count > 0).then(|| {
                        ui::text(16., FontWeight::NORMAL, ink::DIM).child(count.to_string())
                    })),
            )
            .children((count > 0).then(|| {
                ui::text(13., FontWeight::NORMAL, ink::NOTE)
                    .id("shots-see-all")
                    .cursor_pointer()
                    .hover(|s| s.text_color(white(ink::FULL)))
                    .child(tr("See all"))
                    .on_click(see_all())
            }));
        let shot = |i: usize, path: std::path::PathBuf| {
            div()
                .id(SharedString::from(format!("overview-shot-{i}")))
                .overflow_hidden()
                .rounded(px(radius::ROW))
                .cursor_pointer()
                .child(
                    gpui::img(path)
                        .size_full()
                        .rounded(px(radius::ROW))
                        .object_fit(gpui::ObjectFit::Cover),
                )
        };
        let body = match shots.len() {
            0 => ui::text(14., FontWeight::NORMAL, ink::NOTE)
                .mt(px(8.))
                .child(tr("No screenshots yet. Press F2 in the game."))
                .into_any_element(),
            n if n >= 3 => {
                let mut it = shots.into_iter().enumerate();
                let (i0, first) = it.next().unwrap();
                div()
                    .mt(px(12.))
                    .flex()
                    .gap(px(8.))
                    .w_full()
                    .aspect_ratio(8. / 3.)
                    .child(shot(i0, first.path).flex_1().h_full().on_click(see_all()))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .w(gpui::relative(1. / 3.))
                            .flex_none()
                            .h_full()
                            .children(it.map(|(i, s)| {
                                shot(i, s.path).flex_1().w_full().on_click(see_all())
                            })),
                    )
                    .into_any_element()
            }
            _ => div()
                .mt(px(12.))
                .flex()
                .gap(px(8.))
                .children(shots.into_iter().enumerate().map(|(i, s)| {
                    shot(i, s.path)
                        .flex_1()
                        .aspect_ratio(16. / 9.)
                        .on_click(see_all())
                }))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .px(px(12.))
            .child(head)
            .child(body)
    }

    /// The patch log (`PatchLog`): the newest launcher release's changes by section, "All notes"
    /// opening Patch notes. It fills its column and scrolls inside.
    fn patch_log(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let release = match &self.releases {
            Load::Ready(list) => list
                .iter()
                .find(|r| r.nightly.is_none() && !r.prerelease)
                .cloned(),
            _ => None,
        };
        let note = release.as_ref().map_or_else(
            || tr("No notes yet").to_string(),
            |r| {
                let version = r
                    .tag
                    .rsplit('@')
                    .next()
                    .unwrap_or(&r.tag)
                    .trim_start_matches('v');
                format!(
                    "{} · {}",
                    trf("Launcher {version}", &[("version", version)]),
                    r.date
                )
            },
        );
        let sections = release.map(|r| patch_sections(&r.body)).unwrap_or_default();
        let many = sections.len() > 1;
        let list =
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .mt(px(16.))
                .children(sections.into_iter().map(|(section, items)| {
                    div()
                        .flex()
                        .flex_col()
                        .children(
                            many.then(|| {
                                ui::text(13., FontWeight::MEDIUM, ink::QUIET).child(section)
                            }),
                        )
                        .child(div().mt(px(6.)).flex().flex_col().gap(px(6.)).children(
                            items.into_iter().map(|item| {
                                div()
                                    .flex()
                                    .gap(px(10.))
                                    .child(
                                        div()
                                            .flex_none()
                                            .mt(px(9.))
                                            .size(px(5.))
                                            .rounded_full()
                                            .bg(white(0.4)),
                                    )
                                    .child(
                                        ui::text(14., FontWeight::NORMAL, ink::STRONG)
                                            .line_height(px(21.))
                                            .min_w_0()
                                            .child(item),
                                    )
                            }),
                        ))
                }));
        div()
            .id("patch-log")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .p(px(16.))
            .rounded(px(radius::LARGE))
            .bg(white(0.04))
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(px(12.))
                    .child(card_title(tr("Patch log")))
                    .child(
                        ui::text(13., FontWeight::NORMAL, ink::NOTE)
                            .id("patch-log-all")
                            .cursor_pointer()
                            .hover(|s| s.text_color(white(ink::FULL)))
                            .child(tr("All notes"))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.go(Page::Notes);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                ui::text(13., FontWeight::NORMAL, ink::NOTE)
                    .mt(px(2.))
                    .child(note),
            )
            .child(list)
    }
}

/// A card's title (`CARD_TITLE`): 16 px semibold.
fn card_title(label: impl Into<SharedString>) -> Div {
    ui::text(16., FontWeight::SEMIBOLD, ink::FULL).child(label.into())
}

/// A tab's buttons and body, as the panel takes them.
fn split(tab: Option<(Div, impl IntoElement)>) -> (Option<Div>, Option<AnyElement>) {
    match tab {
        Some((actions, body)) => (Some(actions), Some(body.into_any_element())),
        None => (None, None),
    }
}

/// A release's changes by section ("### New", "### Fixed"), each line without its pull request
/// numbers; the contributors' thanks and what follows the notes are left out.
fn patch_sections(body: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("### ") {
            out.push((name.trim().to_string(), Vec::new()));
        } else if line == "---" || line.starts_with("Your launcher updates itself") {
            break;
        } else if let (Some(item), Some((section, items))) =
            (line.strip_prefix("- "), out.last_mut())
        {
            if section.as_str() != "Contributors" {
                items.push(strip_refs(item));
            }
        }
    }
    out.retain(|(name, items)| name != "Contributors" && !items.is_empty());
    out
}

/// "Fix the picker (#6389, fixes #6342)" → "Fix the picker".
fn strip_refs(item: &str) -> String {
    let mut s = item.trim();
    while let Some(open) = s.rfind(" (") {
        let tail = &s[open..];
        if tail.ends_with(')') && tail.contains('#') {
            s = s[..open].trim_end();
        } else {
            break;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_body_becomes_its_sections_without_refs_or_thanks() {
        let body = "### New\n- Native mods: export a headless engine (#6462)\n\n### Fixed\n- Play with friends: any port (#6363, fixes #6342)\n- Keep (this) aside\n\n### Contributors\nThanks to @a.\n\nYour launcher updates itself. New here?\n\n---\nNot an official Minecraft product.";
        assert_eq!(
            patch_sections(body),
            vec![
                (
                    "New".to_string(),
                    vec!["Native mods: export a headless engine".to_string()]
                ),
                (
                    "Fixed".to_string(),
                    vec![
                        "Play with friends: any port".to_string(),
                        "Keep (this) aside".to_string()
                    ]
                ),
            ]
        );
    }
}
