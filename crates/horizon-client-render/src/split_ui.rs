//! Setup's monitor map and draft window choices. Apply is consumed by the game adapter.
use crate::{
    graphics_style::STYLE,
    hud::{HudAtlas, HudBatch, rgba},
    lang,
    split_layout::{Draft, Monitor, Placement, Spot, Warning, ordered},
    split_map::{self, BOTTOM, uncovered},
    text::Text,
    ui,
};

/// Resolve against the selected language on every draw, with bundled English
/// defaults even in a resource-free UI test. Arguments use Minecraft's formatter.
fn tr(key: &str, args: &[&dyn std::fmt::Display]) -> String {
    static ENGLISH: std::sync::OnceLock<lang::Language> = std::sync::OnceLock::new();
    let key = if key.starts_with("horizon.") {
        key.to_owned()
    } else {
        format!("horizon.splitscreen.setup.{key}")
    };
    let template = lang::current()
        .and_then(|language| language.get(&key).map(str::to_owned))
        .unwrap_or_else(|| {
            ENGLISH
                .get_or_init(|| lang::Language::load(&[], "en_us"))
                .get(&key)
                .unwrap_or(&key)
                .to_owned()
        });
    let args: Vec<_> = args
        .iter()
        .map(|arg| Text::literal(arg.to_string()))
        .collect();
    crate::text::format_translation(&template, &args).plain()
}

#[derive(Default)]
pub struct SplitUi {
    pub on: bool,
    pub idle: bool,
    pub live: Option<Draft>,
    pub monitors: Vec<Monitor>,
    pub main: String,
    pub names: Vec<String>,
    pub apply: Option<Draft>,
    edit: Option<Draft>,
    selected: Option<usize>,
}

#[derive(Clone)]
enum Target {
    Player(usize),
    Move(Spot),
    Revert,
    Apply,
    Done,
    Switch,
}
type Button = ([f32; 4], String, Target, bool);
pub enum Click {
    Nothing,
    Pressed,
    Done,
    Switch,
}

impl SplitUi {
    pub fn reset(&mut self) {
        self.edit = None;
        self.selected = None;
        self.apply = None;
    }
    fn draft(&self) -> Option<&Draft> {
        self.edit.as_ref().or(self.live.as_ref())
    }
    fn map(&self, w: f32, h: f32) -> split_map::Map {
        if self.idle {
            return Default::default();
        }
        self.draft().map_or_else(Default::default, |draft| {
            split_map::project(draft, &self.monitors, &self.main, w, h)
        })
    }
    fn editing(&mut self) -> Option<&mut Draft> {
        if self.edit.is_none() {
            self.edit = self.live.clone();
        }
        self.edit.as_mut()
    }

    fn monitor_name(&self, name: &str) -> String {
        match self.monitors.iter().position(|m| m.name == name) {
            Some(n) => tr("monitor", &[&(n + 1)]),
            None => tr("monitor_unknown", &[]),
        }
    }

    /// Where the selected player can go: the other windows, a new one, a monitor no window fills.
    fn spots(&self) -> Vec<(String, Spot)> {
        let (Some(draft), Some(i)) = (self.draft(), self.selected) else {
            return Vec::new();
        };
        let mine = draft.players.get(i).copied();
        let mut spots: Vec<(String, Spot)> = draft
            .windows
            .iter()
            .filter(|w| Some(w.number) != mine)
            .map(|w| (tr("window", &[&w.number]), Spot::Window(w.number)))
            .collect();
        spots.push((tr("new_window", &[]), Spot::New));
        for m in &self.monitors {
            let filled = draft
                .windows
                .iter()
                .any(|w| w.placement == Placement::Fill(m.name.clone()));
            if m.name != self.main && !filled {
                spots.push((
                    tr("fill_monitor", &[&self.monitor_name(&m.name)]),
                    Spot::Fill(m.name.clone()),
                ));
            }
        }
        spots
    }

    fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let row = |out: &mut Vec<Button>, y: f32, items: Vec<(f32, String, Target, bool)>| {
            let total: f32 = items.iter().map(|i| i.0 + 4.0).sum::<f32>() - 4.0;
            let mut x = ((w - total) / 2.0).floor();
            for (bw, label, target, active) in items {
                out.push(([x, y, bw, 20.0], label, target, active));
                x += bw + 4.0;
            }
        };
        let done = (
            64.0,
            lang::get_or("gui.done", &tr("done", &[])),
            Target::Done,
            true,
        );
        // The switch, top right (P1's): off only once P1 plays alone (turning it off with others
        // in drops them all at once).
        let switch = (
            [w - 110.0, 6.0, 104.0, 20.0],
            tr(
                "switch",
                &[
                    &tr("horizon.extras.splitscreen", &[]),
                    &tr(if self.on { "on" } else { "off" }, &[]),
                ],
            ),
            Target::Switch,
            !self.on || self.names.len() <= 1,
        );
        if !self.on || self.idle {
            let mut out = vec![switch];
            row(&mut out, h - 27.0, vec![done]);
            return out;
        }
        let (_, windows, tiles) = self.map(w, h);
        let mut order: Vec<usize> = (0..tiles.len()).collect();
        order.sort_by_key(|&i| windows.iter().position(|(n, _)| *n == tiles[i].0));
        let mut out: Vec<Button> = order
            .into_iter()
            .map(|i| {
                let name = self
                    .names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| tr("player", &[&(i + 1)]));
                // The part no later window covers: what can be seen and pressed.
                let at = windows.iter().position(|(n, _)| *n == tiles[i].0);
                let covers = at.map_or(&[][..], |k| &windows[k + 1..]);
                let rect = uncovered(tiles[i].1, covers.iter().map(|(_, r)| *r));
                (rect, name, Target::Player(i), true)
            })
            .collect();
        // The move row: one button for the windows, which cycles (the next window after the
        // player's, by number, wrapping), then a new window and the monitors.
        let mut spots = self.spots();
        let mine = self
            .selected
            .and_then(|i| self.draft().and_then(|d| d.players.get(i).copied()))
            .unwrap_or(0);
        let next = spots
            .iter()
            .filter_map(|(_, s)| match s {
                Spot::Window(n) => Some(*n),
                _ => None,
            })
            .min_by_key(|&n| (n <= mine, n));
        spots.retain(|(_, s)| !matches!(s, Spot::Window(n) if Some(*n) != next));
        if !spots.is_empty() {
            let items = spots
                .into_iter()
                .map(|(label, spot)| (86.0, label, Target::Move(spot), true))
                .collect();
            row(&mut out, h - BOTTOM + 4.0, items);
        }
        out.push(switch);
        let mut footer = Vec::new();
        let editing = self.changed();
        footer.extend([
            (64.0, tr("revert", &[]), Target::Revert, editing),
            (64.0, tr("apply", &[]), Target::Apply, editing),
            done,
        ]);
        row(&mut out, h - 27.0, footer);
        out
    }

    fn warnings(&self) -> Vec<String> {
        let Some(draft) = self.draft() else {
            return Vec::new();
        };
        draft
            .warnings(&self.monitors, &self.main)
            .into_iter()
            .map(|w| match w {
                Warning::Empty(1) => tr("warning.empty_main", &[]),
                // One open now closes; one only in the draft never opens.
                Warning::Empty(n) if self.live.as_ref().is_some_and(|l| !l.empty(n)) => {
                    tr("warning.empty_close", &[&n])
                }
                Warning::Empty(n) => tr("warning.empty_new", &[&n]),
                Warning::OnWindow1(n) => tr("warning.on_main", &[&n]),
                Warning::Unplugged(n, _) => tr("warning.unplugged", &[&n]),
            })
            .collect()
    }

    /// The state represented by the preview area, also available to agent controls.
    pub fn status(&self) -> &'static str {
        if !self.on {
            "off"
        } else if self.idle {
            "idle"
        } else {
            "active"
        }
    }

    fn status_lines(&self) -> Option<[String; 3]> {
        if !self.on {
            Some([
                tr("status.off", &[]),
                tr("status.enable_hint", &[]),
                tr("status.join_hint", &[]),
            ])
        } else if self.idle {
            Some([
                tr("status.idle", &[]),
                tr("status.alone", &[]),
                tr("status.solo_controls", &[]),
            ])
        } else {
            None
        }
    }

    pub fn draw(&self, batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32), in_world: bool) {
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, in_world);
        STYLE.title(
            batch,
            atlas,
            &Text::literal(tr("horizon.pages.split_setup", &[])),
        );
        if let Some(lines) = self.status_lines() {
            let grey = rgba(170, 170, 170, 255);
            let lines: Vec<_> = lines
                .iter()
                .flat_map(|line| atlas.font.split(&Text::literal(line), (w - 40.0).max(10.0)))
                .collect();
            let top = h / 2.0 - 6.0 * lines.len() as f32;
            for (k, line) in lines.into_iter().enumerate() {
                let y = (top + 12.0 * k as f32).floor();
                ui::centered_text(batch, atlas, &line, 0.0, y, w, grey);
            }
            for b in &self.buttons(w, h) {
                self.draw_button(batch, atlas, cursor, b);
            }
            return;
        }
        let (monitors, windows, tiles) = self.map(w, h);
        let grey = rgba(160, 160, 160, 255);
        for &[x, y, mw, mh] in &monitors {
            batch.fill(atlas, x, y, mw, mh, rgba(255, 255, 255, 60));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                mw - 2.0,
                mh - 2.0,
                rgba(0, 0, 0, 170),
            );
        }
        let buttons = self.buttons(w, h);
        let yellow = rgba(255, 255, 85, 255);
        // Each window, then its players: a window over another covers it.
        let draft = self.draft();
        // A window nobody plays in any more stays until Apply: grey, under the others.
        let emptied = |n: u8| n != 1 && draft.is_some_and(|d| d.empty(n));
        for &(number, [x, y, ww, wh]) in &windows {
            let edge = match emptied(number) {
                true => rgba(160, 160, 160, 120),
                false => rgba(255, 255, 85, 200),
            };
            batch.fill(atlas, x, y, ww, wh, edge);
            if emptied(number) {
                let label = Text::literal(tr("window_empty", &[&number]));
                batch.text_styled(atlas, &label, x + 3.0, y - 10.0, grey, true);
            }
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                ww - 2.0,
                wh - 2.0,
                rgba(40, 40, 40, 230),
            );
            for b in buttons
                .iter()
                .filter(|b| matches!(b.2, Target::Player(i) if tiles[i].0 == number))
            {
                self.draw_button(batch, atlas, cursor, b);
            }
        }
        // The windows' names over everything, so another window's tiles never hide them.
        for (number, [x, y, _, _]) in windows.iter().copied().filter(|w| !emptied(w.0)) {
            let (label, color) = (tr("window", &[&number]), yellow);
            // A title above this window can land on another player's visible tile.
            let covered = windows.iter().any(|(n, r)| {
                *n != number && ui::hovered((x + 3.0, y - 10.0), r[0], r[1], r[2], r[3])
            });
            let title_y = if covered { y + 3.0 } else { y - 10.0 };
            batch.text_styled(atlas, &Text::literal(label), x + 3.0, title_y, color, true);
        }
        for b in buttons.iter().filter(|b| !matches!(b.2, Target::Player(_))) {
            self.draw_button(batch, atlas, cursor, b);
        }
        // The monitors' names under them, where no window covers them.
        for (n, [x, y, _, mh]) in monitors.into_iter().enumerate() {
            let label = Text::literal(tr("monitor", &[&(n + 1)]));
            batch.text_styled(atlas, &label, x + 1.0, y + mh + 2.0, grey, true);
        }
        self.draw_notes(batch, atlas, cursor, &buttons);
    }

    fn draw_button(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        ([x, y, bw, bh], label, target, active): &Button,
    ) {
        let (mut x, mut y, mut bw, mut bh) = (*x, *y, *bw, *bh);
        let hover = *active && ui::hovered(cursor, x, y, bw, bh);
        let selected = matches!(target, Target::Player(i) if Some(*i) == self.selected);
        if matches!(target, Target::Player(_)) {
            (x, y, bw, bh) = (x + 1.0, y + 1.0, (bw - 2.0).max(4.0), (bh - 2.0).max(4.0));
        }
        // The selected player's name in yellow: picked, not just under the pointer.
        let text = if selected {
            Text::colored(label.as_str(), crate::text::YELLOW)
        } else {
            Text::literal(label.as_str())
        };
        ui::button(
            batch,
            atlas,
            x,
            y,
            bw,
            bh,
            &text,
            *active,
            hover || selected,
        );
    }
    fn draw_notes(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        buttons: &[Button],
    ) {
        let (w, h) = (batch.width, batch.height);
        let note = match self.selected {
            None => tr("note.select", &[]),
            Some(_) if self.edit.is_some() => tr("note.apply", &[]),
            Some(i) => tr(
                "note.place",
                &[&self
                    .names
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| tr("this_player", &[]))],
            ),
        };
        for (k, (line, color)) in std::iter::once((note, rgba(170, 170, 170, 255)))
            .chain(
                self.warnings()
                    .into_iter()
                    .map(|w| (w, rgba(255, 170, 85, 255))),
            )
            .take(2)
            .enumerate()
        {
            ui::centered_text(
                batch,
                atlas,
                &Text::literal(line),
                0.0,
                h - BOTTOM + 28.0 + k as f32 * 11.0,
                w,
                color,
            );
        }
        if buttons.iter().any(|(r, _, t, active)| {
            !active && matches!(t, Target::Switch) && ui::hovered(cursor, r[0], r[1], r[2], r[3])
        }) {
            ui::tooltip(
                batch,
                atlas,
                &atlas
                    .font
                    .split(&Text::literal(tr("switch_disabled", &[])), 170.0),
                cursor,
            );
        }
    }
    /// A press at `(x, y)` on a screen `w` x `h`, or the mouse moving with it held (`drag`).
    pub fn click(&mut self, (x, y): (f32, f32), (w, h): (f32, f32), drag: bool) -> Click {
        if drag {
            return Click::Nothing;
        }
        // The topmost: the last drawn.
        let hit = self
            .buttons(w, h)
            .into_iter()
            .rev()
            .find(|([bx, by, bw, bh], _, _, active)| {
                *active && ui::hovered((x, y), *bx, *by, *bw, *bh)
            });
        let Some((_, _, target, _)) = hit else {
            return Click::Nothing;
        };
        match target {
            Target::Player(i) => {
                self.selected = Some(i);
            }
            Target::Move(spot) => {
                if let Some(i) = self.selected {
                    self.place(i, spot);
                }
            }
            Target::Revert => self.edit = None,
            Target::Apply => {
                // Windows emptied in the draft close now, not as players moved.
                self.apply = self.edit.take().map(|mut d| {
                    d.drop_empty(&[]);
                    d
                });
            }
            Target::Done => {
                self.reset();
                return Click::Done;
            }
            Target::Switch => {
                self.reset();
                return Click::Switch;
            }
        }
        Click::Pressed
    }

    fn changed(&self) -> bool {
        // A free window's spot moves under the draft as it's dragged around: not a change.
        let windows = |d: &Draft| -> Vec<(u8, Option<String>)> {
            let fill = |p: &Placement| match p {
                Placement::Fill(name) => Some(name.clone()),
                Placement::Free(_) => None,
            };
            d.windows
                .iter()
                .map(|w| (w.number, fill(&w.placement)))
                .collect()
        };
        // The order only counts among players sharing a window (who is first in it).
        let order = |d: &Draft| -> Vec<(u8, usize)> {
            let mut by_window: Vec<(u8, usize)> = ordered(&d.order, d.players.len())
                .into_iter()
                .filter_map(|i| d.players.get(i).map(|&n| (n, i)))
                .collect();
            by_window.sort_by_key(|&(n, _)| n);
            by_window
        };
        let same = |a: &Draft, b: &Draft| {
            windows(a) == windows(b)
                && a.players == b.players
                && (a.two, a.three) == (b.two, b.three)
                && order(a) == order(b)
        };
        match (&self.edit, &self.live) {
            (Some(edit), Some(live)) => !same(edit, live),
            (edit, _) => edit.is_some(),
        }
    }

    /// Player `i` to `spot` in the draft.
    fn place(&mut self, i: usize, spot: Spot) {
        let open = self.open_now();
        if let Some(draft) = self.editing() {
            draft.place_player(i, spot);
            draft.drop_empty(&open);
        }
    }

    /// The windows open now: emptied in a draft, they stay until Apply (a window the draft
    /// added goes as soon as nobody is in it).
    fn open_now(&self) -> Vec<u8> {
        self.live
            .as_ref()
            .map_or_else(Vec::new, |l| l.windows.iter().map(|w| w.number).collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f32, f32) = (427.0, 300.0);

    fn screen(players: usize) -> SplitUi {
        SplitUi {
            live: Some(Draft::everyone_here(
                players,
                Placement::Free(Some(([0, 0], [2000, 1200]))),
                0,
                0,
            )),
            monitors: vec![
                Monitor {
                    name: "Built-in".into(),
                    at: [0, 0],
                    size: [3024, 1964],
                    scale: 2.0,
                },
                Monitor {
                    name: "TV".into(),
                    at: [3024, 0],
                    size: [1920, 1080],
                    scale: 1.0,
                },
            ],
            main: "Built-in".into(),
            names: (1..=players).map(|n| format!("P{n}")).collect(),
            on: true,
            ..Default::default()
        }
    }

    fn labels(ui: &SplitUi) -> Vec<String> {
        ui.buttons(SIZE.0, SIZE.1)
            .into_iter()
            .map(|b| b.1)
            .collect()
    }

    #[test]
    fn off_only_the_switch_and_done() {
        let mut ui = screen(2);
        ui.on = false;
        assert_eq!(labels(&ui), ["Splitscreen: Off", "Done"]);
        let r = rect(&ui, "Splitscreen: Off");
        assert!(matches!(
            ui.click((r[0] + 1.0, r[1] + 1.0), SIZE, false),
            Click::Switch
        ));
    }

    fn rect(ui: &SplitUi, label: &str) -> [f32; 4] {
        ui.buttons(SIZE.0, SIZE.1)
            .into_iter()
            .find(|(_, l, _, _)| l == label)
            .unwrap_or_else(|| panic!("no {label}"))
            .0
    }

    fn press(ui: &mut SplitUi, label: &str) {
        let r = rect(ui, label);
        ui.click((r[0] + 1.0, r[1] + 1.0), SIZE, false);
    }

    #[test]
    fn move_a_player_then_apply() {
        let mut ui = screen(2);
        press(&mut ui, "P2");
        press(&mut ui, "Fill Monitor 2");
        // Nothing applied yet: the draft only.
        assert_eq!(ui.apply, None);
        assert_eq!(ui.live.as_ref().unwrap().players, [1, 1]);
        press(&mut ui, "Apply");
        let applied = ui.apply.take().unwrap();
        assert_eq!(applied.players, [1, 2]);
        assert_eq!(applied.windows[1].placement, Placement::Fill("TV".into()));
    }

    #[test]
    fn revert_drops_the_draft() {
        let mut ui = screen(2);
        press(&mut ui, "P2");
        press(&mut ui, "New window");
        assert!(ui.edit.is_some());
        press(&mut ui, "Revert");
        assert!(ui.edit.is_none());
        // Done closes and forgets the selection.
        press(&mut ui, "Done");
        assert_eq!(ui.selected, None);
    }
    #[test]
    fn locale_switch_and_pack_overrides_keep_the_same_draft() {
        // The language table is process-wide. Run this test alone in a child so
        // other UI tests cannot observe its temporary French language.
        const CHILD: &str = "HORIZON_TEST_SPLIT_SETUP_LOCALE";
        if std::env::var_os(CHILD).is_none() {
            let module = module_path!().split_once("::").unwrap().1;
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    &format!("{module}::locale_switch_and_pack_overrides_keep_the_same_draft"),
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed;"),
                "locale child test did not run: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        struct Pack;
        impl lang::ResourceSource for Pack {
            fn namespaces(&self) -> Vec<String> {
                vec!["horizon".into()]
            }
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                (path == "assets/horizon/lang/fr_fr.json").then(|| {
                    serde_json::to_vec(&serde_json::json!({
                        "horizon.splitscreen.setup.new_window": "Créer une fenêtre",
                        "horizon.splitscreen.setup.monitor": "Écran personnalisé %s",
                        "horizon.splitscreen.setup.fill_monitor": "Occuper %s",
                        "horizon.splitscreen.setup.warning.empty_main": "Fenêtre principale masquée"
                    }))
                    .unwrap()
                })
            }
        }
        let mut ui = screen(2);
        ui.names[1] = "Élodie %s".into();
        lang::set(lang::Language::load(&[], "en_us"));
        press(&mut ui, "Élodie %s");
        assert!(labels(&ui).contains(&"New window".to_owned()));
        lang::set(lang::Language::load(&[], "fr_fr"));
        assert!(labels(&ui).contains(&"Nouvelle fenêtre".to_owned()));
        assert!(labels(&ui).contains(&"Appliquer".to_owned()));
        assert!(
            labels(&ui).contains(&"Élodie %s".to_owned()),
            "player names remain literal"
        );
        press(&mut ui, "Nouvelle fenêtre");
        let draft = ui.edit.clone().unwrap();
        assert_eq!(draft.players, [1, 2]);
        lang::set(lang::Language::load(&[&Pack], "fr_fr"));
        let translated = labels(&ui);
        assert!(translated.contains(&"Créer une fenêtre".to_owned()));
        assert!(translated.contains(&"Occuper Écran personnalisé 2".to_owned()));
        assert_eq!(
            ui.edit,
            Some(draft),
            "a language change must not reset the arrangement"
        );
        ui.place(0, Spot::Window(2));
        assert!(
            ui.warnings()
                .contains(&"Fenêtre principale masquée".to_owned())
        );
        press(&mut ui, "Appliquer");
        assert_eq!(ui.apply.take().unwrap().players, [2, 2]);
        ui.on = false;
        assert_eq!(
            ui.status(),
            "off",
            "agent protocol states stay language independent"
        );
        assert_eq!(
            ui.status_lines().unwrap()[0],
            "L'écran partagé est désactivé."
        );
        assert!(labels(&ui).contains(&"Écran partagé : Désactivé".to_owned()));
        lang::set(lang::Language::load(&[], "de_de"));
        assert_eq!(
            ui.status_lines().unwrap()[0],
            "Splitscreen is off.",
            "untranslated new keys use bundled English"
        );
    }
}
