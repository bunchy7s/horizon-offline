//! Windows and monitors: which window each player plays in, opening and placing them (a
//! monitor's over its menu bar), closing them, the views' layout and the windows' titles.
use super::*;
use horizon_client::split_layout::Placement;
use winit::window::Window;

/// Where a monitor is and its size, as windows are placed: macOS places them in points, each
/// monitor at its own scale (pixels would be read at the window's scale: a window for a 1x TV
/// beside a 2x screen would open on the screen, half size); Windows and X11 in desktop pixels.
pub(super) fn cover(m: &MonitorHandle) -> (winit::dpi::Position, winit::dpi::Size) {
    #[cfg(target_os = "macos")]
    {
        let scale = m.scale_factor();
        (
            m.position().to_logical::<f64>(scale).into(),
            m.size().to_logical::<f64>(scale).into(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        (m.position().into(), m.size().into())
    }
}

/// Where new window `number` (854 x 480) opens on monitor `m`, in `cover`'s units: centred, and
/// cascaded from the others (`layout::cascade`).
pub(super) fn centred(m: &MonitorHandle, number: u8) -> winit::dpi::Position {
    let (at, size) = cover(m);
    // The numbers as they are, logical or physical.
    let (at, size) = (at.to_physical::<f64>(1.0), size.to_physical::<f64>(1.0));
    let u = if cfg!(target_os = "macos") {
        1.0
    } else {
        m.scale_factor()
    };
    let step = layout::cascade(number) * u;
    let x = at.x + (size.width - 854.0 * u) / 2.0 + step;
    let y = at.y + (size.height - 480.0 * u) / 2.0 + step;
    if cfg!(target_os = "macos") {
        winit::dpi::LogicalPosition::new(x, y).into()
    } else {
        winit::dpi::PhysicalPosition::new(x, y).into()
    }
}

/// The monitor's window titled `title` over that monitor's menu bar, all of it: one level above
/// the menu bar, at the screen's whole frame. Hiding the menu bar (the presentation options) works
/// only for the window with the keyboard, and a monitor's window opens without it. AppKit's own
/// list of the app's windows, by title: winit's handle to it would need `unsafe`.
#[cfg(target_os = "macos")]
pub(super) fn above_menu_bar(title: &str) {
    let Some(main_thread) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(main_thread);
    for window in app.windows() {
        if window.title().to_string() != title {
            continue;
        }
        // NSMainMenuWindowLevel (24) + 1.
        window.setLevel(25);
        if let Some(screen) = window.screen() {
            window.setFrame_display(screen.frame(), true);
        }
    }
}

/// Move a player into a native window. Changing only its viewport keeps the grab;
/// moving to another window releases it until a click there.
pub(super) fn into_window(game: &mut Game<'_>, window: Arc<Window>) {
    let moved = game
        .window
        .as_ref()
        .is_none_or(|old| old.id() != window.id());
    if moved && game.captured {
        game.capture(false);
    }
    game.window = Some(window);
}

/// A window showing players' views, and the compositor that copies their frames into it.
pub(super) struct Surface {
    /// Which window this is, by number (1: the game's), and where it was opened: a number
    /// given again to a window placed elsewhere opens again there.
    pub(super) key: u8,
    pub(super) placement: Placement,
    pub(super) window: Arc<Window>,
    pub(super) compositor: Compositor,
    pub(super) size: PhysicalSize<u32>,
    pub(super) focused: bool,
    /// The title it was last given.
    pub(super) title: String,
}

impl<'a> Split<'a> {
    /// The window player `i` chose, by number.
    pub(super) fn window_number(&self, i: usize) -> u8 {
        match i {
            0 => self.p1_window,
            _ => self.guests[i - 1].window,
        }
    }

    pub(super) fn set_window(&mut self, i: usize, number: u8) {
        match i {
            0 => self.p1_window = number,
            _ => self.guests[i - 1].window = number,
        }
    }

    pub(super) fn placement(&self, number: u8) -> Option<&Placement> {
        self.windows
            .iter()
            .find(|w| w.number == number)
            .map(|w| &w.placement)
    }

    /// The monitors now, with their names, left to right, and the one Window 1 is on.
    pub(super) fn refresh_monitors(&mut self) {
        let Some(window) = self
            .surfaces
            .first()
            .map(|s| s.window.clone())
            .or_else(|| self.host.window.clone())
        else {
            return;
        };
        self.screens = window
            .available_monitors()
            .filter_map(|m| Some((m.name()?, m)))
            .collect();
        self.screens
            .sort_by_key(|(_, m)| (m.position().x, m.position().y));
        // Wayland can list one monitor twice, and not say which one a window is on.
        self.screens.dedup_by(|a, b| a.0 == b.0);
        self.main_screen = window.current_monitor().and_then(|m| m.name()).or_else(|| {
            match self.screens.as_slice() {
                [(only, _)] => Some(only.clone()),
                _ => None,
            }
        });
    }

    /// The monitors, by name, left to right (as of this frame).
    pub(super) fn monitors(&self) -> &[(String, MonitorHandle)] {
        &self.screens
    }

    /// The monitor the game's window is on: Window 1 already shows there.
    pub(super) fn main_monitor(&self) -> Option<&String> {
        self.main_screen.as_ref()
    }

    /// How a monitor is called: numbered left to right, with its size (the system's names are
    /// ids on some).
    pub(super) fn monitor_label(&self, name: &str) -> Option<String> {
        let (n, (_, m)) = self
            .monitors()
            .iter()
            .enumerate()
            .find(|(_, (n, _))| n == name)?;
        let size = m.size();
        Some(format!(
            "Monitor {} ({}x{})",
            n + 1,
            size.width,
            size.height
        ))
    }

    /// The window player `i` plays in now: Window 1 for a window that fills a monitor that's
    /// gone, or the one Window 1 is on.
    pub(super) fn window_of(&self, i: usize) -> u8 {
        let number = self.window_number(i);
        let shown = match self.placement(number) {
            Some(Placement::Fill(name)) => {
                Some(name) != self.main_monitor() && self.monitors().iter().any(|(n, _)| n == name)
            }
            Some(Placement::Free(_)) => true,
            None => false,
        };
        if shown { number } else { 1 }
    }

    /// Where player `i` can play, as the screens say it: Window 1 (the game's), the other windows
    /// open, a new window, and each monitor no window fills but Window 1's. The page's button
    /// steps forward through them (buttons take the left click only), a joining player's bumpers
    /// both ways. A player alone in its window is at "New window" or its monitor's choice (its
    /// window is that), so the list stays the same as it steps.
    pub(super) fn choices(&self, i: usize) -> Vec<(String, Spot)> {
        let alone = self.alone(i);
        let mine = self.window_of(i);
        let mut open: Vec<u8> = (0..self.players())
            .map(|j| self.window_of(j))
            .filter(|&n| !(alone && n == mine))
            .collect();
        open.sort_unstable();
        open.dedup();
        let filled: Vec<&String> = open
            .iter()
            .filter_map(|&n| match self.placement(n) {
                Some(Placement::Fill(name)) => Some(name),
                _ => None,
            })
            .collect();
        let main = self.main_monitor();
        let monitors = self.monitors().iter().filter_map(|(name, _)| {
            if Some(name) == main || filled.contains(&name) {
                return None;
            }
            Some((self.monitor_label(name)?, Spot::Fill(name.clone())))
        });
        std::iter::once((WINDOW_1.to_owned(), Spot::Window(1)))
            .chain(
                open.into_iter()
                    .filter(|&n| n != 1)
                    .map(|n| (self.window_label(n), Spot::Window(n))),
            )
            .chain(std::iter::once((NEW_WINDOW.to_owned(), Spot::New)))
            .chain(monitors)
            .collect()
    }

    /// Player `i` plays alone in a window of its own (not Window 1).
    pub(super) fn alone(&self, i: usize) -> bool {
        let mine = self.window_of(i);
        mine != 1 && (0..self.players()).all(|j| j == i || self.window_of(j) != mine)
    }

    /// Player `i`'s place among its choices: its window, or alone, what its window is.
    pub(super) fn current(&self, i: usize) -> Spot {
        let mine = self.window_of(i);
        match self.placement(mine) {
            Some(Placement::Free(_)) if self.alone(i) => Spot::New,
            Some(Placement::Fill(name)) if self.alone(i) => Spot::Fill(name.clone()),
            _ => Spot::Window(mine),
        }
    }

    /// How window `number` is shown: "Window 2", and the monitor it fills.
    pub(super) fn window_label(&self, number: u8) -> String {
        match self.placement(number) {
            Some(Placement::Fill(name)) if number != 1 => match self.monitor_label(name) {
                Some(monitor) => format!("Window {number}, {monitor}"),
                None => format!("Window {number}"),
            },
            _ => format!("Window {number}"),
        }
    }

    /// Where player `i` plays, as the screens say it.
    pub(super) fn label(&self, i: usize) -> String {
        self.window_label(self.window_of(i))
    }

    /// Player `i` into a window: one there is, or a new one with the first number free.
    pub(super) fn choose(&mut self, i: usize, choice: Spot) {
        let placement = match choice {
            Spot::Window(n) => return self.set_window(i, n),
            Spot::New => Placement::Free(None),
            Spot::Fill(name) => Placement::Fill(name),
        };
        // Out of its window first: alone there, its number is free again.
        self.set_window(i, 1);
        self.forget_empty_windows();
        let number = (2..=u8::MAX)
            .find(|n| self.windows.iter().all(|w| w.number != *n))
            .unwrap_or(u8::MAX);
        self.windows.push(layout::Window { number, placement });
        self.set_window(i, number);
    }

    /// Windows nobody plays in close (Window 1 stays).
    pub(super) fn forget_empty_windows(&mut self) {
        let used: Vec<u8> = (0..self.players()).map(|i| self.window_number(i)).collect();
        self.windows
            .retain(|w| w.number == 1 || used.contains(&w.number));
    }

    /// Every player's view, in the windows there are now.
    pub(super) fn views(&self) -> Vec<View> {
        let windows: Vec<usize> = (0..self.players())
            .map(|i| {
                let number = self.window_of(i);
                self.surfaces
                    .iter()
                    .position(|s| s.key == number)
                    .unwrap_or(0)
            })
            .collect();
        let sizes: Vec<_> = self.surfaces.iter().map(|s| s.size).collect();
        layout::place_ordered(&windows, &self.player_order(), &sizes, 0, 0)
    }

    /// The players (indices) in the order they fill a window they share (`order`, by seat;
    /// players it doesn't name last).
    pub(super) fn player_order(&self) -> Vec<usize> {
        let seats: Vec<usize> = std::iter::once(1)
            .chain(self.guests.iter().map(|g| g.seat))
            .collect();
        let named: Vec<usize> = self
            .order
            .iter()
            .filter_map(|seat| seats.iter().position(|s| s == seat))
            .collect();
        layout::ordered(&named, seats.len())
    }

    /// Every player into its view: its window, then its size. Keyboard and mouse follow their
    /// player into another window; a player whose view moved or grew shows where on its badge.
    pub(super) fn relayout(&mut self, event_loop: &ActiveEventLoop) {
        let views = self.views();
        let before = std::mem::replace(&mut self.applied, views);
        for (i, &(surface, at, size)) in self.applied.clone().iter().enumerate() {
            let Some(window) = self.surfaces.get(surface).map(|s| s.window.clone()) else {
                continue;
            };
            let moved = self
                .game(i)
                .and_then(|g| g.window.as_ref())
                .is_none_or(|old| old.id() != window.id());
            if let Some(game) = self.game_mut(i) {
                into_window(game, window.clone());
                game.options.split_in_main_window = surface == 0;
                game.view_at = at;
            }
            if moved && i == self.focus && before.get(i).is_some() {
                window.focus_window();
            }
            if moved && before.get(i).is_some() {
                let label = format!("Window {}", self.window_of(i));
                self.tag(i, &label);
            }
            if let Some((player, id)) = self.player(i) {
                player.window_event(event_loop, id, WindowEvent::Resized(size));
            }
        }
        self.turn(0);
    }

    /// The places a joining player steps through with its bumpers: each window it could play in,
    /// at each place among the players there ("Window 1 (1 of 2)": first is left or on top),
    /// then a new window and each monitor (`choices`).
    pub(super) fn joining_stops(&self, i: usize) -> Vec<(String, Spot, usize)> {
        let mut stops = Vec::new();
        for (label, spot) in self.choices(i) {
            let others = match spot {
                Spot::Window(n) => self.sharing(i, n).len(),
                _ => 0,
            };
            if others == 0 {
                stops.push((label, spot, 0));
                continue;
            }
            for k in 0..=others {
                let label = format!("{label} ({} of {})", k + 1, others + 1);
                stops.push((label, spot.clone(), k));
            }
        }
        stops
    }

    /// The players but `i` in window `number`, in their order.
    fn sharing(&self, i: usize, number: u8) -> Vec<usize> {
        self.player_order()
            .into_iter()
            .filter(|&j| j != i && self.window_of(j) == number)
            .collect()
    }

    pub(super) fn seat_of(&self, i: usize) -> usize {
        if i == 0 { 1 } else { self.guests[i - 1].seat }
    }

    /// Player `i` to place `k` among the others in its window (0: before them all).
    pub(super) fn put_at(&mut self, i: usize, k: usize) {
        let others: Vec<usize> = self
            .sharing(i, self.window_of(i))
            .into_iter()
            .map(|j| self.seat_of(j))
            .collect();
        let me = self.seat_of(i);
        let mut order: Vec<usize> = self
            .player_order()
            .into_iter()
            .map(|j| self.seat_of(j))
            .filter(|&s| s != me)
            .collect();
        let at = match others.get(k) {
            Some(seat) => order.iter().position(|s| s == seat).unwrap_or(order.len()),
            // After the last of them.
            None => others
                .last()
                .and_then(|seat| order.iter().position(|s| s == seat))
                .map_or(order.len(), |p| p + 1),
        };
        order.insert(at, me);
        self.order = order;
    }

    /// A bumper on the profile page (joining or switching): player `i`'s view moves to the next
    /// place it could play (`back`: the one before), at once, so it sees where it will be.
    pub(super) fn move_with_bumpers(&mut self, i: usize, back: bool) {
        let stops = self.joining_stops(i);
        let n = stops.len();
        if n == 0 {
            return;
        }
        let (spot, before) = (self.current(i), self.sharing_before(i));
        let at = stops
            .iter()
            .position(|(_, s, k)| *s == spot && *k == before)
            .unwrap_or(0);
        let (shown, next, k) = stops[if back { (at + n - 1) % n } else { (at + 1) % n }].clone();
        self.choose(i, next);
        self.put_at(i, k);
        if let Some(game) = self.game_mut(i) {
            game.split_hud.place = Some((
                format!("LB < {shown} > RB"),
                Instant::now() + Duration::from_secs(3),
            ));
        }
    }

    /// Where player `i` plays now, as its bumpers' stops name it ("Window 1 (2 of 2)"); its own
    /// window by its name ("Window 2").
    pub(super) fn stop_label(&self, i: usize) -> Option<String> {
        let (spot, before) = (self.current(i), self.sharing_before(i));
        if spot != Spot::Window(self.window_of(i)) {
            return Some(self.label(i));
        }
        self.joining_stops(i)
            .into_iter()
            .find(|(_, s, k)| *s == spot && *k == before)
            .map(|(label, ..)| label)
    }

    /// How many players come before `i` in its window.
    fn sharing_before(&self, i: usize) -> usize {
        let mine = self.window_of(i);
        self.player_order()
            .into_iter()
            .take_while(|&j| j != i)
            .filter(|&j| self.window_of(j) == mine)
            .count()
    }

    /// What a window's title says: its name and who plays in it.
    pub(super) fn title(&self, k: usize) -> String {
        let views = &self.applied;
        let who: Vec<String> = (0..self.players())
            .filter(|&i| views.get(i).is_some_and(|v| v.0 == k))
            .map(|i| self.name(i))
            .collect();
        let name = match self.surfaces[k].key {
            1 if self.surfaces.len() == 1 => String::new(),
            n => format!(" - Window {n}"),
        };
        if self.players() > 1 && !who.is_empty() {
            format!("Horizon{name} · {}", who.join(", "))
        } else {
            format!("Horizon{name}")
        }
    }

    /// The windows the players' screens need: the game's (hidden while nobody plays on it), one
    /// on each monitor picked, one for each player who wants its own. True when they changed.
    pub(super) fn sync_surfaces(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if self.surfaces.is_empty() {
            return false;
        }
        self.forget_empty_windows();
        let monitors = self.monitors().to_vec();
        let wanted: Vec<u8> = (0..self.players()).map(|i| self.window_of(i)).collect();
        let placements: std::collections::HashMap<u8, Placement> = self
            .windows
            .iter()
            .map(|w| (w.number, w.placement.clone()))
            .collect();
        let before = self.surfaces.len();
        for s in &self.surfaces {
            // A free window's place only: a monitor's would open a later free one monitor-sized.
            if s.key != 1
                && !wanted.contains(&s.key)
                && matches!(s.placement, Placement::Free(_))
                && let Ok(at) = s.window.outer_position()
            {
                // In points: a window from a 1x TV opens there again beside a 2x screen.
                let scale = s.window.scale_factor();
                let size = s.window.inner_size().to_logical(scale);
                self.window_rects
                    .insert(s.key, (at.to_logical(scale), size));
            }
        }
        self.surfaces.retain(|s| {
            s.key == 1 || (wanted.contains(&s.key) && placements.get(&s.key) == Some(&s.placement))
        });
        let mut changed = self.surfaces.len() != before;
        // macOS would join new windows to the game's as tabs (when the system prefers tabs).
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::ActiveEventLoopExtMacOS;
            event_loop.set_allows_automatic_window_tabbing(false);
        }
        let mut failed = Vec::new();
        for key in &wanted {
            if failed.contains(key) || self.surfaces.iter().any(|s| &s.key == key) {
                continue;
            }
            // Opened without the focus: whoever types in the game's window keeps the keyboard.
            let attributes = Window::default_attributes()
                .with_window_icon(horizon_client::window::icon())
                .with_active(false);
            // A monitor's window covers it, borderless. On macOS not the system's fullscreen, a
            // Space of its own that hides the other windows as it slides in.
            let placement = self.placement(*key).cloned();
            let fill = match &placement {
                Some(Placement::Fill(name)) => monitors
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, m)| m.clone()),
                _ => None,
            };
            let cover = fill.as_ref().map(cover);
            let attributes = match cover {
                // Its own title for a moment: `above_menu_bar` finds it by it.
                Some((at, size)) => {
                    let attributes = attributes
                        .with_title(format!("Horizon {key}"))
                        .with_decorations(false)
                        .with_position(at)
                        .with_inner_size(size);
                    // Elsewhere the system's borderless fullscreen (F11's): over the taskbar too.
                    #[cfg(not(target_os = "macos"))]
                    let attributes = attributes.with_fullscreen(
                        fill.clone()
                            .map(|m| winit::window::Fullscreen::Borderless(Some(m))),
                    );
                    attributes
                }
                // A numbered window where it was last, else 854 x 480.
                None => {
                    let last = self.window_rects.get(key).copied();
                    match last {
                        Some((at, size)) => attributes.with_position(at).with_inner_size(size),
                        None => {
                            let main = self.main_monitor();
                            let home = monitors.iter().find(|(n, _)| Some(n) == main);
                            let attributes =
                                attributes.with_inner_size(LogicalSize::new(854.0, 480.0));
                            match home {
                                Some((_, m)) => attributes.with_position(centred(m, *key)),
                                None => attributes,
                            }
                        }
                    }
                    .with_title("Horizon")
                }
            };
            let made = event_loop
                .create_window(attributes)
                .map_err(Into::into)
                .and_then(|w| {
                    let w = Arc::new(w);
                    let renderer = self.host.renderer.as_ref().ok_or("no renderer")?;
                    Ok::<_, Box<dyn std::error::Error>>((w.clone(), renderer.compositor(w)?))
                });
            if let (Ok((window, _)), Some((at, size))) = (&made, cover) {
                // macOS moves a new window under the menu bar: place it again, size first (it
                // keeps a window's bottom-left corner when it resizes).
                let _ = window.request_inner_size(size);
                window.set_outer_position(at);
                #[cfg(target_os = "macos")]
                above_menu_bar(&format!("Horizon {key}"));
            }
            match made {
                Ok((window, compositor)) => {
                    self.surfaces.push(Surface {
                        key: *key,
                        placement: placement.clone().unwrap_or(Placement::Free(None)),
                        size: window.inner_size(),
                        window,
                        compositor,
                        focused: false,
                        title: String::new(),
                    });
                    changed = true;
                }
                // Not tried again each frame: its players play in Window 1 (where they were
                // drawn meanwhile) and the window is forgotten.
                Err(e) => {
                    log::warn!("Horizon: splitscreen: no Window {key}: {e}");
                    failed.push(*key);
                }
            }
        }
        for key in failed {
            for i in 0..self.players() {
                if self.window_number(i) == key {
                    self.set_window(i, 1);
                }
            }
            self.windows.retain(|w| w.number != key);
        }
        // The game's window only while someone plays on it.
        let main_used = (0..self.players()).any(|i| self.window_of(i) == 1);
        if self.window1_shown != main_used {
            self.surfaces[0].window.set_visible(main_used);
            self.window1_shown = main_used;
        }
        for k in 0..self.surfaces.len() {
            let title = self.title(k);
            if self.surfaces[k].title != title {
                self.surfaces[k].window.set_title(&title);
                self.surfaces[k].title = title;
            }
        }
        changed
    }

    /// A window other than Window 1 was closed: its players leave, P1 goes back to Window 1.
    pub(super) fn close_window(&mut self, surface: usize) -> Result<()> {
        let views = self.views();
        for i in (0..self.players()).rev() {
            if views.get(i).is_none_or(|v| v.0 != surface) {
                continue;
            }
            if i == 0 {
                self.p1_window = 1;
                continue;
            }
            self.turn(i);
            let guest = &mut self.guests[i - 1];
            if guest.game.remote.is_some() {
                // One failing to leave doesn't keep the others in.
                if let Err(e) = guest.game.leave_server(None) {
                    log::warn!("Horizon: splitscreen: P{} could not leave: {e}", guest.seat);
                }
            } else {
                // No connection yet: remove the picker and its view immediately.
                self.remove_guest(i - 1);
            }
        }
        self.turn(0);
        Ok(())
    }
}

/// The surface whose presenting waits for the display (vsync), and whose monitor's refresh rate
/// paces the frames: Window 1, or the next while Window 1 is hidden (a hidden window's present
/// can block or be throttled).
pub(super) fn paced_surface(window1_shown: bool, surfaces: usize) -> usize {
    if window1_shown || surfaces < 2 { 0 } else { 1 }
}

#[cfg(test)]
mod paced_tests {
    use super::paced_surface;

    #[test]
    fn vsync_follows_the_first_shown_window() {
        assert_eq!(paced_surface(true, 3), 0);
        assert_eq!(
            paced_surface(false, 3),
            1,
            "Window 1 hidden: the next window waits"
        );
        assert_eq!(paced_surface(false, 1), 0);
    }
}
