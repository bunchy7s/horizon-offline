//! P1's Setup screen's side in the game: the arrangement as a draft, and Apply.
use super::*;

impl<'a> Split<'a> {
    /// The arrangement now, for the Setup screen: free windows where they are.
    pub(super) fn draft(&self) -> layout::Draft {
        // Wayland never says where a window is: those are shown at their real size, side by side
        // from the first monitor's corner, so the map never stacks them.
        let mut shelf = self.monitors().first().map_or(0, |(_, m)| m.position().x);
        let windows = self
            .windows
            .iter()
            .map(|w| {
                let here = self.surfaces.iter().find(|s| s.key == w.number);
                let placement = match (&w.placement, here) {
                    (Placement::Free(_), Some(s)) => {
                        // On the desktop as windows are placed: macOS in points.
                        let k = if cfg!(target_os = "macos") {
                            s.window.scale_factor()
                        } else {
                            1.0
                        };
                        let at = match s.window.outer_position() {
                            Ok(at) => [(f64::from(at.x) / k) as i32, (f64::from(at.y) / k) as i32],
                            Err(_) => {
                                let y = self.monitors().first().map_or(0, |(_, m)| m.position().y);
                                let at = [shelf, y];
                                shelf += (f64::from(s.size.width) / k) as i32 + 16;
                                at
                            }
                        };
                        Placement::Free(Some((at, [s.size.width, s.size.height])))
                    }
                    (p, _) => p.clone(),
                };
                layout::Window {
                    number: w.number,
                    placement,
                }
            })
            .collect();
        layout::Draft {
            windows,
            players: (0..self.players()).map(|i| self.window_number(i)).collect(),
            two: 0,
            three: 0,
            order: self.player_order(),
        }
    }

    /// The monitors for the Setup screen, left to right (as `monitor_label` numbers them), and
    /// the one Window 1 is on.
    pub(super) fn setup_monitors(&self) -> (Vec<layout::Monitor>, String) {
        let monitors = self
            .monitors()
            .iter()
            .map(|(name, m)| layout::Monitor {
                name: name.clone(),
                at: [m.position().x, m.position().y],
                size: [m.size().width, m.size().height],
                scale: m.scale_factor(),
            })
            .collect();
        (monitors, self.main_monitor().cloned().unwrap_or_default())
    }

    /// The Setup screen applied: its windows, and every player in its window. Windows nobody
    /// plays in close (`forget_empty_windows`).
    pub(super) fn apply(&mut self, draft: layout::Draft) {
        let mut windows: Vec<layout::Window> = draft
            .windows
            .into_iter()
            .map(|w| {
                // A free window stays where it is; a new one opens where the system puts it.
                let placement = match (&w.placement, self.placement(w.number)) {
                    (Placement::Free(_), Some(Placement::Free(p))) => Placement::Free(*p),
                    (Placement::Free(_), _) => Placement::Free(None),
                    (p, _) => p.clone(),
                };
                layout::Window {
                    number: w.number,
                    placement,
                }
            })
            .collect();
        if !windows.iter().any(|w| w.number == 1) {
            windows.insert(
                0,
                layout::Window {
                    number: 1,
                    placement: Placement::Free(None),
                },
            );
        }
        self.windows = windows;
        // Players who joined while it was open stay where they are.
        for (i, &number) in draft.players.iter().enumerate().take(self.players()) {
            let number = if self.placement(number).is_some() {
                number
            } else {
                1
            };
            self.set_window(i, number);
        }
        // Who is first in a shared window, by seat (players are by index in the draft).
        let n = self.players();
        self.order = layout::ordered(&draft.order, n)
            .into_iter()
            .map(|i| self.seat_of(i))
            .collect();
    }

    /// P1's Setup screen: the arrangement now, the monitors and the players' names; Apply
    /// arranges them. True when it did.
    pub(super) fn sync_setup(&mut self) -> bool {
        use horizon_client::options_ui::Page;
        if self.host.ui.options.page() != Page::SplitSetup {
            return false;
        }
        // Window/fullscreen changes during P1's turn can invalidate Windows monitor handles.
        self.refresh_monitors();
        let live = self.draft();
        let (monitors, main) = self.setup_monitors();
        let names = (0..self.players()).map(|i| self.name(i)).collect();
        let seats: Vec<usize> = (0..self.players()).map(|i| self.seat_of(i)).collect();
        let moved = std::mem::replace(&mut self.setup_seats, seats) != self.setup_seats;
        let setup = &mut self.host.ui.options.split_setup;
        // Someone joined or left: the draft would put the players in the wrong places.
        if moved {
            setup.reset();
        }
        let applied = setup.apply.take();
        let setup = &mut self.host.ui.options.split_setup;
        setup.idle = self.surfaces.is_empty();
        setup.live = Some(live);
        setup.monitors = monitors;
        setup.main = main;
        setup.names = names;
        let changed = applied.is_some();
        if let Some(draft) = applied {
            self.apply(draft);
        }
        // Every view says where it is: after a change, or asked.
        if changed {
            for i in 0..self.players() {
                let label = horizon_client::lang::tr(
                    "horizon.splitscreen.setup.window",
                    &[&self.window_of(i)],
                );
                self.tag(i, &label);
            }
        }
        changed
    }
}
