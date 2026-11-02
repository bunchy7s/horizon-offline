//! Setup's monitor map and draft window choices. Apply is consumed by the game adapter.
use crate::{
    gamepad::Button as PadButton,
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
    /// Identify pressed: every view says which window it is in.
    pub identify: bool,
    /// Names: the names in the views' corners are shown (the game sets it), and pressed.
    pub names_shown: bool,
    pub toggle_names: bool,
    /// P1 has a controller (the game sets it), and Give Up Pad pressed: it's offered to join.
    pub p1_pad: bool,
    pub give_up_pad: bool,
    edit: Option<Draft>,
    selected: Option<usize>,
    /// A tile held by the mouse: whose, and where the pointer is (`None` until it moves).
    drag: Option<(usize, Option<(f32, f32)>)>,
    /// A player picked up with the controller (A), moved with the bumpers.
    held: Option<Held>,
    /// Dropped or put back: the player whose tile the controller's focus goes to.
    refocus: Option<usize>,
    /// The screen's size: at the last draw (the options screen sets it) or press.
    pub(crate) size: (f32, f32),
}

/// A player picked up with the controller: where it could go, the one picked so far, and the
/// draft with it there (the map shows it).
struct Held {
    player: usize,
    spots: Vec<(String, Drop)>,
    at: Option<usize>,
    preview: Option<Draft>,
}

/// Where a dragged player goes: a place (`Spot`), or a place among a window's players.
#[derive(Clone, Debug, PartialEq)]
enum Drop {
    At(Spot),
    /// Place `k` among the players of a window (before its `k`th; past the last: after them).
    Place(u8, usize),
}

#[derive(Clone, Debug, PartialEq)]
enum Target {
    Player(usize),
    Move(Spot),
    Two,
    Three,
    Identify,
    /// The players' names in their views' corners, on or off.
    Names,
    /// P1's controller freed and offered to join (handed to a friend).
    GiveUpPad,
    Revert,
    Apply,
    Done,
    Switch,
}
type Button = ([f32; 4], String, Target, bool);

/// How a layout's square button is drawn.
enum Glyph {
    Auto,
    /// The panes in a 14 x 10 window, 1 px apart.
    Panes(&'static [[f32; 4]]),
}

/// The glyph for a layout: 2 sharing (`splitscreen_two`: Auto, side by side, top and bottom) or
/// 3 (`splitscreen_three`: one wide on top, one tall on the left, side by side).
fn layout_glyph(three: bool, layout: u8) -> Glyph {
    match (three, layout) {
        (false, 0) => Glyph::Auto,
        (false, 1) => Glyph::Panes(&[[0.0, 0.0, 6.5, 10.0], [7.5, 0.0, 6.5, 10.0]]),
        (false, _) => Glyph::Panes(&[[0.0, 0.0, 14.0, 4.5], [0.0, 5.5, 14.0, 4.5]]),
        (true, 0) => Glyph::Panes(&[
            [0.0, 0.0, 14.0, 4.5],
            [0.0, 5.5, 6.5, 4.5],
            [7.5, 5.5, 6.5, 4.5],
        ]),
        (true, 1) => Glyph::Panes(&[
            [0.0, 0.0, 6.5, 10.0],
            [7.5, 0.0, 6.5, 4.5],
            [7.5, 5.5, 6.5, 4.5],
        ]),
        (true, _) => Glyph::Panes(&[
            [0.0, 0.0, 4.0, 10.0],
            [5.0, 0.0, 4.0, 10.0],
            [10.0, 0.0, 4.0, 10.0],
        ]),
    }
}
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
        self.drag = None;
        self.held = None;
    }
    /// The draft shown: with a held player where the controller put it, else the changes so
    /// far, else the arrangement now.
    fn draft(&self) -> Option<&Draft> {
        let preview = self.held.as_ref().and_then(|h| h.preview.as_ref());
        preview.or(self.edit.as_ref()).or(self.live.as_ref())
    }

    /// The controller (`pad_menu`), A at `at`: on a player's tile, picks it up. True when it did
    /// (the press is taken).
    pub fn pad_pick_up(&mut self, at: (f32, f32)) -> bool {
        let (w, h) = self.size;
        let hit = self
            .buttons(w, h)
            .into_iter()
            .rev()
            .find(|([x, y, bw, bh], _, _, active)| *active && ui::hovered(at, *x, *y, *bw, *bh));
        let Some((_, _, Target::Player(i), _)) = hit else {
            return false;
        };
        self.selected = Some(i);
        let spots = self.stops(i);
        self.held = Some(Held {
            player: i,
            spots,
            at: None,
            preview: None,
        });
        true
    }

    /// The controller while a player is held: the bumpers or left and right cycle where it goes,
    /// as when joining (the map shows it there), A drops it there (into the draft), B puts it
    /// back. True when the button was taken: every button, while one is held.
    pub fn pad_button(&mut self, button: PadButton) -> bool {
        let Some(held) = &self.held else {
            return false;
        };
        let player = held.player;
        match button {
            PadButton::LeftBumper
            | PadButton::RightBumper
            | PadButton::DPadLeft
            | PadButton::DPadRight => {
                let n = held.spots.len();
                if n == 0 {
                    return true;
                }
                let back = matches!(button, PadButton::LeftBumper | PadButton::DPadLeft);
                let next = match held.at {
                    None if back => n - 1,
                    None => 0,
                    Some(k) if back => (k + n - 1) % n,
                    Some(k) => (k + 1) % n,
                };
                let (player, drop) = (held.player, held.spots[next].1.clone());
                let open = self.open_now();
                let preview = self.edit.clone().or(self.live.clone()).map(|mut draft| {
                    match drop {
                        Drop::At(spot) => draft.place_player(player, spot),
                        Drop::Place(n, k) => {
                            draft.place_player(player, Spot::Window(n));
                            draft.put_at(player, k);
                        }
                    }
                    draft.drop_empty(&open);
                    draft
                });
                if let Some(held) = &mut self.held {
                    held.at = Some(next);
                    held.preview = preview;
                }
            }
            PadButton::South => {
                let held = self.held.take().expect("held");
                // One highlight after: the focus on the player where it landed, not selected too.
                (self.selected, self.refocus) = (None, Some(held.player));
                match held.at.map(|k| held.spots[k].1.clone()) {
                    Some(Drop::At(spot)) => self.place(held.player, spot),
                    Some(Drop::Place(n, k)) => self.place_at(held.player, n, k),
                    None => {}
                }
            }
            PadButton::East => {
                (self.selected, self.refocus) = (None, Some(player));
                self.held = None;
            }
            // While one is held, the focus stays put: A drops, it presses nothing else.
            _ => {}
        }
        true
    }

    /// Whether a player is held (the controller's left and right cycle it).
    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// The tile of the player just dropped or put back, for the controller's focus. Taken once.
    pub fn take_refocus(&mut self) -> Option<[f32; 4]> {
        let i = self.refocus.take()?;
        let (w, h) = self.size;
        self.buttons(w, h)
            .into_iter()
            .find(|b| b.2 == Target::Player(i))
            .map(|b| b.0)
    }
    fn map(&self, w: f32, h: f32) -> split_map::Map {
        if self.idle {
            return Default::default();
        }
        self.draft()
            .map_or_else(Default::default, |draft| self.map_of(draft, w, h))
    }
    /// `draft` on the map (the one shown, or one a drag would make).
    fn map_of(&self, draft: &Draft, w: f32, h: f32) -> split_map::Map {
        split_map::project(draft, &self.monitors, &self.main, w, h)
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

    /// Where player `i` can be dropped: each place among the players of each window ("Window 1
    /// (2 of 3)"), a window of its own, a new window, a monitor no window fills. Never into
    /// another player's place: it joins them.
    fn stops(&self, i: usize) -> Vec<(String, Drop)> {
        let Some(draft) = self.draft() else {
            return Vec::new();
        };
        let mut stops = Vec::new();
        for w in &draft.windows {
            let others = draft.sharing(i, w.number).len();
            if others == 0 {
                stops.push((tr("window", &[&w.number]), Drop::At(Spot::Window(w.number))));
            }
            for k in (0..=others).filter(|_| others > 0) {
                let label = tr("window_place", &[&w.number, &(k + 1), &(others + 1)]);
                stops.push((label, Drop::Place(w.number, k)));
            }
        }
        stops.push((tr("new_window", &[]), Drop::At(Spot::New)));
        for m in &self.monitors {
            let filled = draft
                .windows
                .iter()
                .any(|w| w.placement == Placement::Fill(m.name.clone()));
            if m.name != self.main && !filled {
                let label = tr("fill_monitor", &[&self.monitor_name(&m.name)]);
                stops.push((label, Drop::At(Spot::Fill(m.name.clone()))));
            }
        }
        stops
    }

    /// Where a tile dropped at `(x, y)` goes: the window on top there, else the monitor's (a new
    /// window on Window 1's monitor, the one filling another or a new one filling it).
    fn drop_spot(&self, (x, y): (f32, f32), w: f32, h: f32) -> Option<Spot> {
        let draft = self.draft()?;
        let (monitors, windows, _) = self.map(w, h);
        let inside = |[rx, ry, rw, rh]: [f32; 4]| x >= rx && x < rx + rw && y >= ry && y < ry + rh;
        if let Some((n, _)) = windows.iter().rev().find(|(_, r)| inside(*r)) {
            return Some(Spot::Window(*n));
        }
        let m = &self.monitors[monitors.iter().position(|r| inside(*r))?];
        if m.name == self.main {
            return Some(Spot::New);
        }
        Some(
            match draft
                .windows
                .iter()
                .find(|w| w.placement == Placement::Fill(m.name.clone()))
            {
                Some(win) => Spot::Window(win.number),
                None => Spot::Fill(m.name.clone()),
            },
        )
    }

    /// Every button: the players' tiles (window by window, as they stack), the move row, the
    /// switch, Names and the layouts, and Identify, Revert, Apply and Done. Splitscreen off or
    /// idle: the switch and Done.
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
        // P1's controller handed to a friend: it's offered to join.
        let give_up = (
            [90.0, 6.0, 80.0, 20.0],
            tr("give_up_pad", &[]),
            Target::GiveUpPad,
            true,
        );
        if !self.on || self.idle {
            let mut out = vec![switch];
            if self.on && self.p1_pad {
                out.push(give_up);
            }
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
        // Names, top left: each player's name in the corner of their view.
        let names = (
            [6.0, 6.0, 80.0, 20.0],
            tr(
                "switch",
                &[
                    &tr("names", &[]),
                    &tr(if self.names_shown { "on" } else { "off" }, &[]),
                ],
            ),
            Target::Names,
            true,
        );
        out.extend([switch, names]);
        if self.p1_pad {
            out.push(give_up);
        }
        // A square for how 2 (or 3) players share a window, while a window has that many (drawn
        // as the layout; the label is for the control socket and the tests).
        let mut footer = Vec::new();
        if let Some(draft) = self.draft() {
            let sharing = |k: usize| {
                draft
                    .windows
                    .iter()
                    .any(|win| draft.players.iter().filter(|&&n| n == win.number).count() == k)
            };
            if sharing(2) {
                let two = tr(&format!("layout.two.{}", draft.two.min(2)), &[]);
                footer.push((20.0, tr("layout.two", &[&two]), Target::Two, true));
            }
            if sharing(3) {
                let three = tr(&format!("layout.three.{}", draft.three.min(2)), &[]);
                footer.push((20.0, tr("layout.three", &[&three]), Target::Three, true));
            }
        }
        // Narrow enough for two squares in the narrowest view (320 GUI pixels).
        let editing = self.changed();
        footer.extend([
            (64.0, tr("identify", &[]), Target::Identify, true),
            (64.0, tr("revert", &[]), Target::Revert, editing),
            (64.0, tr("apply", &[]), Target::Apply, editing),
            done,
        ]);
        row(&mut out, h - 27.0, footer);
        out
    }

    /// What a button does, for the tooltip while the pointer is on it; for one that can't be
    /// pressed, why not.
    fn help(&self, (_, label, target, active): &Button) -> Option<String> {
        let auto = self.draft().is_some_and(|d| d.two == 0);
        Some(match target {
            Target::Player(_) => tr("help.player", &[label]),
            Target::Move(_) | Target::Done => return None,
            Target::Two if auto => tr("help.layout_auto", &[label]),
            Target::Two | Target::Three => tr("help.layout", &[label]),
            Target::Switch if !*active => tr("switch_disabled", &[]),
            Target::Switch => tr("help.switch", &[]),
            Target::Identify => tr("help.identify", &[]),
            Target::Names => tr("help.names", &[]),
            Target::GiveUpPad => tr("help.give_up_pad", &[]),
            Target::Revert if *active => tr("help.revert", &[]),
            Target::Apply if *active => tr("help.apply", &[]),
            Target::Revert | Target::Apply => tr("help.unchanged", &[]),
        })
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
        // The tile being dragged, under the pointer.
        if let Some((i, Some((x, y)))) = self.drag
            && let Some(name) = self.names.get(i)
        {
            let label = Text::colored(name.as_str(), crate::text::YELLOW);
            ui::button(
                batch,
                atlas,
                x - 30.0,
                y - 10.0,
                60.0,
                20.0,
                &label,
                true,
                true,
            );
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
        // While a player is held, only its tile is lit: the controller's pointer stays where
        // the pick-up was, over whichever tile is there now.
        let hover = *active && self.held.is_none() && ui::hovered(cursor, x, y, bw, bh);
        let selected = matches!(target, Target::Player(i) if Some(*i) == self.selected);
        if matches!(target, Target::Player(_)) {
            (x, y, bw, bh) = (x + 1.0, y + 1.0, (bw - 2.0).max(4.0), (bh - 2.0).max(4.0));
        }
        let glyph = self.draft().and_then(|d| match target {
            Target::Two => Some(layout_glyph(false, d.two)),
            Target::Three => Some(layout_glyph(true, d.three)),
            _ => None,
        });
        // The selected player's name in yellow: picked, not just under the pointer.
        let text = if glyph.is_some() {
            Text::literal("")
        } else if selected {
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
        // The held player: "< Name >", the arrows lit, for left and right.
        if matches!(target, Target::Player(i) if self.held.as_ref().is_some_and(|h| h.player == *i))
        {
            let (half, cx) = (HudBatch::text_width(atlas, label) / 2.0, x + bw / 2.0);
            let ty = (y + (bh - 8.0) / 2.0).floor();
            let lit = rgba(255, 255, 85, 255);
            let lw = HudBatch::text_width(atlas, "<");
            batch.text(atlas, "<", (cx - half - 4.0 - lw).floor(), ty, lit, true);
            batch.text(atlas, ">", (cx + half + 4.0).ceil(), ty, lit, true);
        }
        // A layout's square: its panes, in a 14 x 10 window; Auto a letter. Its name for the
        // control socket, which lists widgets by what they say.
        if glyph.is_some() {
            crate::widgets::record(label, [x, y, bw, bh], *active);
        }
        let (gx, gy) = (
            (x + (bw - 14.0) / 2.0).floor(),
            (y + (bh - 10.0) / 2.0).floor(),
        );
        match glyph {
            Some(Glyph::Auto) => {
                let a = Text::literal("A");
                ui::centered_text(batch, atlas, &a, x, y + 6.0, bw, rgba(255, 255, 255, 255));
            }
            Some(Glyph::Panes(panes)) => {
                for &[px, py, pw, ph] in panes {
                    batch.fill(atlas, gx + px, gy + py, pw, ph, rgba(230, 230, 230, 255));
                }
            }
            None => {}
        }
    }
    fn draw_notes(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        buttons: &[Button],
    ) {
        let (w, h) = (batch.width, batch.height);
        let held = self.held.as_ref().map(|h| {
            let who = self
                .names
                .get(h.player)
                .cloned()
                .unwrap_or_else(|| tr("this_player", &[]));
            match h.at {
                Some(k) => tr("note.held_at", &[&who, &h.spots[k].0]),
                None => tr("note.held", &[&who]),
            }
        });
        let note = match self.selected {
            _ if held.is_some() => held.unwrap_or_default(),
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
        // What the pointer's button does (disabled ones too, to say why), as the options screens
        // show theirs. Not the selected player's tile: the pointer is still on it after the click.
        let help = buttons
            .iter()
            .filter(|b| ui::hovered(cursor, b.0[0], b.0[1], b.0[2], b.0[3]))
            .filter(|b| !matches!(b.2, Target::Player(i) if Some(i) == self.selected))
            .find_map(|b| self.help(b));
        if let Some(help) = help {
            ui::tooltip(
                batch,
                atlas,
                &atlas.font.split(&Text::literal(help), 170.0),
                cursor,
            );
        }
    }
    /// A press at `(x, y)` on a screen `w` x `h`, or the mouse moving with it held (`drag`).
    pub fn click(&mut self, (x, y): (f32, f32), (w, h): (f32, f32), drag: bool) -> Click {
        self.size = (w, h);
        // The mouse takes over from a player held with the controller.
        if !drag {
            self.held = None;
        }
        if drag {
            if let Some((_, at)) = &mut self.drag {
                *at = Some((x, y));
            }
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
                self.drag = Some((i, None));
            }
            Target::Move(spot) => {
                if let Some(i) = self.selected {
                    self.place(i, spot);
                }
            }
            Target::Two => {
                if let Some(draft) = self.editing() {
                    draft.two = (draft.two + 1) % 3;
                }
            }
            Target::Three => {
                if let Some(draft) = self.editing() {
                    draft.three = (draft.three + 1) % 3;
                }
            }
            Target::Identify => self.identify = true,
            Target::Names => self.toggle_names = true,
            Target::GiveUpPad => self.give_up_pad = true,
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

    /// The mouse let go: a tile dragged somewhere goes there (a draft). True when it moved.
    pub fn release(&mut self) -> bool {
        let (w, h) = self.size;
        let Some((i, Some(at))) = self.drag.take() else {
            return false;
        };
        let before = self.draft().cloned();
        match self.drag_target(i, at, w, h) {
            Some(Drop::At(spot)) => self.place(i, spot),
            Some(Drop::Place(n, k)) => self.place_at(i, n, k),
            None => return false,
        }
        self.draft().cloned() != before
    }

    /// Where player `i` dropped at `at` goes, as the map shows it: the window drawn on top under
    /// the pointer, and in it the place (among its players) that puts the player's view under the
    /// pointer, else the nearest. Dropped back where its view was, it goes back there: a move and
    /// its reverse cancel out. Outside every window: `drop_spot` (a new window, a monitor).
    fn drag_target(&self, i: usize, at: (f32, f32), w: f32, h: f32) -> Option<Drop> {
        let base = self.edit.as_ref().or(self.live.as_ref())?;
        let open = self.open_now();
        let inside = |[x, y, rw, rh]: [f32; 4]| ui::hovered(at, x, y, rw, rh);
        let (_, windows, tiles) = self.map_of(base, w, h);
        // Where it is now: no move.
        if tiles.get(i).is_some_and(|t| inside(t.1)) {
            return None;
        }
        let Some(&(n, _)) = windows.iter().rev().find(|(_, r)| inside(*r)) else {
            let spot = self.drop_spot(at, w, h)?;
            let here = base.players.get(i).copied();
            return (!matches!(spot, Spot::Window(n) if Some(n) == here)).then_some(Drop::At(spot));
        };
        // Each place in that window, by how far its view would be from the pointer (0 inside).
        let distance = |[x, y, rw, rh]: [f32; 4]| {
            let dx = (x - at.0).max(at.0 - x - rw).max(0.0);
            let dy = (y - at.1).max(at.1 - y - rh).max(0.0);
            dx * dx + dy * dy
        };
        self.stops(i)
            .into_iter()
            .filter(|(_, drop)| matches!(drop, Drop::Place(m, _) | Drop::At(Spot::Window(m)) if *m == n))
            .filter_map(|(_, drop)| {
                let mut draft = base.clone();
                match &drop {
                    Drop::At(spot) => draft.place_player(i, spot.clone()),
                    Drop::Place(m, k) => {
                        draft.place_player(i, Spot::Window(*m));
                        draft.put_at(i, *k);
                    }
                }
                draft.drop_empty(&open);
                let view = self.map_of(&draft, w, h).2.get(i)?.1;
                Some((distance(view), drop))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, drop)| drop)
    }

    /// Player `i` to place `k` among the players of window `n` in the draft.
    fn place_at(&mut self, i: usize, n: u8, k: usize) {
        self.place(i, Spot::Window(n));
        if let Some(draft) = self.editing() {
            draft.put_at(i, k);
        }
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
    fn tiles_split_the_window_on_the_map() {
        let ui = screen(2);
        let (monitors, windows, tiles) = ui.map(SIZE.0, SIZE.1);
        assert_eq!(monitors.len(), 2);
        assert_eq!(windows.len(), 1);
        // Two halves of Window 1, side by side.
        let (a, b) = (tiles[0].1, tiles[1].1);
        assert!((a[2] - b[2]).abs() < 0.01);
        assert!((a[0] + a[2] - b[0]).abs() < 0.01);
    }

    #[test]
    fn a_window_over_another_takes_the_click() {
        let mut ui = screen(2);
        // P2 in a new window, opened over Window 1's corner.
        press(&mut ui, "P2");
        press(&mut ui, "New window");
        press(&mut ui, "Apply");
        ui.live = ui.apply.take();
        press(&mut ui, "P2");
        assert_eq!(ui.selected, Some(1));
    }

    #[test]
    fn one_window_button_cycles_through_the_windows() {
        let mut ui = screen(4);
        // P2, P3, P4 each in a new window: Windows 2, 3, 4.
        for p in ["P2", "P3", "P4"] {
            press(&mut ui, p);
            press(&mut ui, "New window");
        }
        press(&mut ui, "P2");
        let windows = |ui: &SplitUi| -> Vec<String> {
            labels(ui)
                .into_iter()
                .filter(|l| l.starts_with("Window "))
                .collect()
        };
        assert_eq!(windows(&ui), ["Window 3"]);
        press(&mut ui, "Window 3");
        assert_eq!(windows(&ui), ["Window 4"]);
        press(&mut ui, "Window 4");
        // Past the last: back to Window 1, then on.
        assert_eq!(windows(&ui), ["Window 1"]);
        press(&mut ui, "Window 1");
        assert_eq!(ui.edit.as_ref().unwrap().players[1], 1);
        assert!(labels(&ui).contains(&"New window".to_owned()));
    }

    #[test]
    fn windows_hold_still_until_apply() {
        let mut ui = screen(3);
        // Playing now: P2 in Window 2, P3 in Window 3.
        let live = ui.live.as_mut().unwrap();
        live.place_player(1, Spot::New);
        live.place_player(2, Spot::New);
        let numbers = |ui: &SplitUi| -> Vec<u8> {
            ui.draft()
                .unwrap()
                .windows
                .iter()
                .map(|w| w.number)
                .collect()
        };
        let next = |ui: &SplitUi| {
            labels(ui)
                .into_iter()
                .find(|l| l.starts_with("Window "))
                .unwrap()
        };
        // P2 into Window 1: Window 2 is open now, so it stays (empty) until Apply, and the
        // window button's list holds still: from Window 1 on to 2, then 3.
        press(&mut ui, "P2");
        let to = next(&ui);
        press(&mut ui, &to);
        assert_eq!(ui.edit.as_ref().unwrap().players[1], 3);
        press(&mut ui, "Window 1");
        assert_eq!(
            (numbers(&ui), next(&ui)),
            (vec![1, 2, 3], "Window 2".to_owned())
        );
        // A window the draft adds goes as soon as nobody is in it: never open, nothing to close.
        press(&mut ui, "New window");
        assert_eq!(numbers(&ui), [1, 2, 3, 4]);
        let to = next(&ui); // Window 1
        press(&mut ui, &to);
        assert_eq!(numbers(&ui), [1, 2, 3]);
        // Apply closes the emptied one.
        press(&mut ui, "Apply");
        let applied = ui.apply.as_ref().unwrap();
        assert_eq!(
            applied.windows.iter().map(|w| w.number).collect::<Vec<_>>(),
            [1, 3]
        );
    }

    #[test]
    fn names_says_what_the_game_shows_and_asks_it_to_change() {
        let mut ui = screen(2);
        ui.names_shown = true;
        assert!(labels(&ui).contains(&"Names: On".to_owned()));
        press(&mut ui, "Names: On");
        assert!(ui.toggle_names);
        // Nothing else changes: no draft.
        assert_eq!(ui.edit, None);
    }

    #[test]
    fn dropped_on_a_player_in_another_window_it_joins_that_window() {
        // Adam (P1) in Window 1, Sam (P2) in Window 2: Sam dropped on Adam joins Window 1;
        // Adam stays there too (they used to swap).
        let mut ui = screen(2);
        ui.live.as_mut().unwrap().place_player(1, Spot::New);
        let (p2, p1) = (rect(&ui, "P2"), rect(&ui, "P1"));
        ui.click((p2[0] + 2.0, p2[1] + 2.0), SIZE, false);
        ui.click((p1[0] + 2.0, p1[1] + 2.0), SIZE, true);
        assert!(ui.release());
        let draft = ui.edit.as_ref().unwrap();
        assert_eq!(draft.players, [1, 1]);
        // Its first half: before Adam.
        assert_eq!(ordered(&draft.order, 2), [1, 0]);
    }

    #[test]
    fn any_move_and_its_reverse_leave_nothing_to_apply() {
        // A fixed pseudo-random walk: layouts of 2 to 5 players over up to 3 windows, then one
        // player moved anywhere (another window, a place among its players, a new window) and
        // moved back to its window and its place there.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for case in 0..2000 {
            let players = 2 + next(4);
            let mut ui = screen(players);
            let live = ui.live.as_mut().unwrap();
            for i in 1..players {
                match next(3) {
                    0 => live.place_player(i, Spot::New),
                    1 => {
                        let n = live.windows[next(live.windows.len())].number;
                        live.place_player(i, Spot::Window(n));
                    }
                    _ => {}
                }
                let n = live.players[i];
                let k = next(live.sharing(i, n).len() + 1);
                live.put_at(i, k);
            }
            let i = next(players);
            let live = ui.live.clone().unwrap();
            let home = live.players[i];
            // Its place: how many of its window's players come before it.
            let place = ordered(&live.order, players)
                .into_iter()
                .filter(|&p| live.players[p] == home)
                .position(|p| p == i)
                .unwrap();
            let to = match next(3) {
                0 => Drop::At(Spot::New),
                1 => {
                    let n = live.windows[next(live.windows.len())].number;
                    let k = next(live.sharing(i, n).len() + 1);
                    Drop::Place(n, k)
                }
                _ => Drop::At(Spot::Window(live.windows[next(live.windows.len())].number)),
            };
            match to.clone() {
                Drop::At(spot) => ui.place(i, spot),
                Drop::Place(n, k) => ui.place_at(i, n, k),
            }
            ui.place_at(i, home, place);
            assert!(
                !ui.changed(),
                "case {case}: {players} players, P{} to {to:?} and back\nlive {live:?}\nedit {:?}",
                i + 1,
                ui.edit
            );
        }
    }

    #[test]
    fn the_switch_turns_off_only_once_p1_plays_alone() {
        let mut ui = screen(2);
        ui.on = true;
        let switch = |ui: &SplitUi| {
            ui.buttons(SIZE.0, SIZE.1)
                .into_iter()
                .find(|b| b.2 == Target::Switch)
                .unwrap()
                .3
        };
        assert!(!switch(&ui), "two players: off is greyed");
        ui.names.truncate(1);
        assert!(switch(&ui));
    }

    #[test]
    fn three_in_a_window_dragged_and_back_nothing_changed() {
        let mut ui = screen(3);
        let centre = |r: [f32; 4]| (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        // Pressed, P3 is selected and the map makes room for the move row: measure after.
        ui.click(centre(rect(&ui, "P3")), SIZE, false);
        let (p3, p1) = (rect(&ui, "P3"), rect(&ui, "P1"));
        // P3 to where P1's view is: it takes that place.
        ui.click(centre(p1), SIZE, true);
        assert!(ui.release());
        assert_eq!(ordered(&ui.edit.as_ref().unwrap().order, 3)[0], 2);
        assert!(rect(&ui, "P3") != p3, "it moved on the map");
        // Back to where its view was: as it was.
        let now = rect(&ui, "P3");
        ui.click(centre(now), SIZE, false);
        ui.click(centre(p3), SIZE, true);
        assert!(ui.release());
        assert!(!ui.changed(), "{:?}", ui.edit);
    }

    #[test]
    fn dragged_back_where_it_was_nothing_changed() {
        let mut ui = screen(2);
        ui.live.as_mut().unwrap().place_player(1, Spot::New);
        let window2 = ui
            .map(SIZE.0, SIZE.1)
            .1
            .iter()
            .find(|(n, _)| *n == 2)
            .unwrap()
            .1;
        let (p2, p1) = (rect(&ui, "P2"), rect(&ui, "P1"));
        ui.click((p2[0] + 2.0, p2[1] + 2.0), SIZE, false);
        ui.click((p1[0] + 2.0, p1[1] + 2.0), SIZE, true);
        assert!(ui.release());
        // Back into Window 2, emptied in the draft: where it was, so nothing to apply.
        let p2 = rect(&ui, "P2");
        ui.click((p2[0] + 2.0, p2[1] + 2.0), SIZE, false);
        ui.click(
            (window2[0] + window2[2] / 2.0, window2[1] + window2[3] / 2.0),
            SIZE,
            true,
        );
        assert!(ui.release(), "dropped into Window 2");
        assert_eq!(ui.draft().unwrap().players, [1, 2]);
        assert!(!ui.changed());
    }

    #[test]
    fn a_tile_under_a_window_is_pressed_where_it_shows() {
        // The left half of a tile hidden: its right half is left.
        let r = uncovered([0.0, 0.0, 10.0, 4.0], [[-5.0, -1.0, 10.0, 9.0]].into_iter());
        assert_eq!(r, [5.0, 0.0, 5.0, 4.0]);
        // All of it hidden: nothing to press.
        let r = uncovered([0.0, 0.0, 10.0, 4.0], [[-1.0, -1.0, 20.0, 9.0]].into_iter());
        assert_eq!(r[2] * r[3], 0.0);
        // P3 in a new window over the middle of Window 1: P2's tile, partly under it, still
        // answers at the middle of what shows of it.
        let mut ui = screen(3);
        press(&mut ui, "P3");
        press(&mut ui, "New window");
        press(&mut ui, "Apply");
        ui.live = ui.apply.take();
        let [x, y, w, h] = rect(&ui, "P2");
        ui.click((x + w / 2.0, y + h / 2.0), SIZE, false);
        assert_eq!(ui.selected, Some(1));
    }

    #[test]
    fn drag_a_player_onto_a_monitor() {
        let mut ui = screen(3);
        let (monitors, _, _) = ui.map(SIZE.0, SIZE.1);
        let tv = monitors[1];
        let p3 = rect(&ui, "P3");
        ui.click((p3[0] + 2.0, p3[1] + 2.0), SIZE, false);
        ui.click((tv[0] + 5.0, tv[1] + 5.0), SIZE, true);
        assert!(ui.release());
        assert_eq!(ui.edit.as_ref().unwrap().players, [1, 1, 2]);
        // P2 dropped on P3 (its tile fills that window): it joins P3 there, nobody swaps.
        let p2 = rect(&ui, "P2");
        ui.click((p2[0] + 2.0, p2[1] + 2.0), SIZE, false);
        ui.click((tv[0] + 5.0, tv[1] + 5.0), SIZE, true);
        assert!(ui.release());
        assert_eq!(ui.edit.as_ref().unwrap().players, [1, 2, 2]);
        // A click without moving only selects.
        let p1 = rect(&ui, "P1");
        ui.click((p1[0] + 2.0, p1[1] + 2.0), SIZE, false);
        assert!(!ui.release());
    }

    #[test]
    fn layout_squares_show_while_a_window_has_that_many() {
        let labels = |ui: &SplitUi| labels(ui).into_iter().filter(|l| l.contains("sharing"));
        // Three in Window 1: only the 3's square, drawn as its panes.
        let mut ui = screen(3);
        assert_eq!(
            labels(&ui).collect::<Vec<_>>(),
            ["3 sharing: One wide on top"]
        );
        press(&mut ui, "3 sharing: One wide on top");
        assert_eq!(ui.edit.as_ref().unwrap().three, 1);
        assert!(matches!(layout_glyph(true, 1), Glyph::Panes(p) if p.len() == 3));
        // Two: only the 2's, Auto a letter, explained under the pointer.
        let ui = screen(2);
        assert_eq!(labels(&ui).collect::<Vec<_>>(), ["2 sharing: Auto"]);
        assert!(matches!(layout_glyph(false, 0), Glyph::Auto));
        let buttons = ui.buttons(SIZE.0, SIZE.1);
        let two = buttons.iter().find(|b| b.2 == Target::Two).unwrap();
        assert_eq!(
            ui.help(two).as_deref(),
            Some("2 sharing: Auto (side by side when the window is wide). Press to change.")
        );
        // Alone: neither.
        assert_eq!(labels(&screen(1)).count(), 0);
    }

    fn centre(ui: &SplitUi, label: &str) -> (f32, f32) {
        let [x, y, w, h] = rect(ui, label);
        (x + w / 2.0, y + h / 2.0)
    }

    #[test]
    fn a_controller_picks_a_player_up_moves_it_and_drops_it() {
        let mut ui = screen(2);
        ui.size = SIZE;
        assert!(ui.pad_pick_up(centre(&ui, "P2")));
        // Nothing moves until the bumpers do; then the map shows it there: the places in Window 1
        // first, then a new window.
        assert_eq!(ui.draft().unwrap().players, [1, 1]);
        for _ in 0..3 {
            assert!(ui.pad_button(PadButton::RightBumper));
        }
        assert_eq!(ui.draft().unwrap().players, [1, 2]);
        assert_eq!(ui.edit, None);
        // Right again: filling the TV; A drops it there, into the draft.
        assert!(ui.pad_button(PadButton::RightBumper));
        assert!(ui.pad_button(PadButton::South));
        let draft = ui.edit.as_ref().unwrap();
        assert_eq!(draft.players, [1, 2]);
        assert_eq!(draft.windows[1].placement, Placement::Fill("TV".into()));
        // Not held any more: the buttons go back to the menu.
        assert!(!ui.pad_button(PadButton::RightBumper));
    }

    #[test]
    fn a_controller_moves_a_player_to_a_place_among_the_others() {
        let mut ui = screen(2);
        ui.size = SIZE;
        assert!(ui.pad_pick_up(centre(&ui, "P2")));
        let stops: Vec<String> = ui
            .held
            .as_ref()
            .unwrap()
            .spots
            .iter()
            .map(|s| s.0.clone())
            .collect();
        assert_eq!(
            &stops[..3],
            ["Window 1 (1 of 2)", "Window 1 (2 of 2)", "New window"]
        );
        assert!(
            !stops.iter().any(|s| s.starts_with("Swap")),
            "no swaps: it joins"
        );
        // RB to the first place: before P1.
        assert!(ui.pad_button(PadButton::RightBumper));
        assert!(ui.pad_button(PadButton::South));
        assert_eq!(ui.edit.as_ref().unwrap().order, [1, 0]);
    }

    #[test]
    fn idle_replaces_preview_but_keeps_the_switch_and_pad_handoff() {
        let mut ui = screen(1);
        ui.on = true;
        ui.idle = true;
        ui.p1_pad = true;
        assert_eq!(ui.status_lines().unwrap()[0], "Splitscreen idle");
        assert!(ui.map(SIZE.0, SIZE.1).2.is_empty());
        let buttons = ui.buttons(SIZE.0, SIZE.1);
        assert!(buttons.iter().any(|b| b.2 == Target::GiveUpPad));
        assert!(
            !buttons
                .iter()
                .any(|b| matches!(b.2, Target::Player(_) | Target::Apply | Target::Revert))
        );
        press(&mut ui, "Give Up Pad");
        assert!(ui.give_up_pad);
        ui.idle = false;
        assert!(ui.status_lines().is_none());
        assert_eq!(ui.map(SIZE.0, SIZE.1).2.len(), 1);
        ui.on = false;
        assert_eq!(ui.status_lines().unwrap()[0], "Splitscreen is off.");
    }

    #[test]
    fn give_up_pad_shows_only_while_p1_has_one() {
        let mut ui = screen(1);
        ui.on = true;
        let shown = |ui: &SplitUi| {
            ui.buttons(SIZE.0, SIZE.1)
                .iter()
                .any(|b| b.2 == Target::GiveUpPad)
        };
        assert!(!shown(&ui));
        ui.p1_pad = true;
        assert!(shown(&ui));
        press(&mut ui, "Give Up Pad");
        assert!(ui.give_up_pad);
    }

    #[test]
    fn b_puts_a_held_player_back() {
        let mut ui = screen(2);
        ui.size = SIZE;
        assert!(ui.pad_pick_up(centre(&ui, "P2")));
        ui.pad_button(PadButton::RightBumper);
        assert!(ui.pad_button(PadButton::East));
        assert_eq!(ui.edit, None);
        assert_eq!(ui.draft().unwrap().players, [1, 1]);
    }

    #[test]
    fn dropped_or_put_back_one_highlight_and_the_focus_follows() {
        let mut ui = screen(2);
        ui.size = SIZE;
        assert!(ui.pad_pick_up(centre(&ui, "P2")));
        assert!(ui.pad_button(PadButton::RightBumper)); // to a new window
        assert!(ui.pad_button(PadButton::South));
        // Not selected (the focus is its highlight), and the focus goes to its new tile.
        assert_eq!(ui.selected, None);
        assert_eq!(ui.take_refocus(), Some(rect(&ui, "P2")));
        assert_eq!(ui.take_refocus(), None);
        // Put back: the same.
        assert!(ui.pad_pick_up(centre(&ui, "P2")));
        assert!(ui.pad_button(PadButton::East));
        assert_eq!(
            (ui.selected, ui.take_refocus()),
            (None, Some(rect(&ui, "P2")))
        );
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
        assert!(labels(&ui).contains(&"Noms : Désactivé".to_owned()));
        assert!(labels(&ui).contains(&"2 par fenêtre : Auto".to_owned()));
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
