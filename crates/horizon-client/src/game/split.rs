//! Splitscreen (in parts; #3541): more players on this computer in P1's world, each its own `Game`
//! in this process that joins P1's server thread through memory. Every player renders offscreen
//! on one device and the window's compositor copies their frames into their views (halves for
//! two, `split_layout`). Keyboard and mouse drive the player whose view was clicked last (Esc
//! frees the mouse first). A player who leaves its world (pause menu, Disconnect) drops out;
//! when P1 leaves, all do. Alone, P1 plays exactly as without splitscreen: no compositor.
//!
//! Each player plays in a window: Window 1 (the game's), Window 2, 3... (opened for the players
//! put there, closed with their last player), or one filling a monitor, borderless over it.
//! Players in one window share it. Closing a player's own window drops its players; the game's
//! window hides while nobody plays in it (`windows`). `HORIZON_SPLIT_WINDOWS` (testing, until
//! the Setup screen): each seat's window in order, `1,1,2` (P1 and P2 in Window 1, P3 in Window
//! 2), `m2` for a window filling monitor 2 (left to right).
//!
//! Design: one process, one `Game` per player, one wgpu device. The players share the game's
//! data (`Resources`, the packs), one audio output and P1's server. Not one game process per
//! player: that needs GPU textures, input and audio shared across processes.
//!
//! Controllers: P1's is the first used in a menu. In P1's world a controller it isn't using
//! pressing a button is offered to join, on every view (A joins, Y gives it to P1, B says no); a
//! controller that disconnects leaves its player waiting, its view dimmed, until it comes back
//! or another takes over (`pads`).
//!
//! Horizon Settings > Splitscreen... enables joining (off by default).
//! `HORIZON_SPLIT=N` (testing) enables it too, and
//! N-1 players without one join once P1 is in its world, as P2, P3... (`HORIZON_SPLIT=1`: only
//! the controllers). The player in seat N takes `horizon-ctl` on P1's socket path + "N".

use super::{
    Game, UiResources, build_renderer,
    pad::{PadId, PadRouter},
    start_game,
};
use horizon_client::gamepad::Button;
use horizon_client::split_layout::{self as layout, MAX_PLAYERS, Placement, View};
use horizon_client::{Result, gfx::compositor::Compositor, widgets};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    monitor::MonitorHandle,
    window::WindowId,
};

mod hud;
mod pads;
mod players;
mod windows;

pub(super) use hud::Hud;
use hud::*;
use windows::*;

thread_local! {
    /// The options file of the player whose turn it is (none for P1, whose is the game's own):
    /// every player runs on this thread, one after another.
    static OPTIONS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The `options.txt` of the player whose turn it is; `None` for P1.
pub(super) fn options_path() -> Option<PathBuf> {
    OPTIONS.with_borrow(Clone::clone)
}

impl Game<'_> {
    /// An error stops the event loop, as ever, but not in another splitscreen player's game:
    /// that player drops out (`Split::drop_out`), the others play on.
    pub(in crate::game) fn exit_on_error(&self, event_loop: &dyn super::event_loop::EventLoopCtl) {
        if !self.options.split_guest {
            event_loop.exit();
        }
    }
}

const WINDOW_1: &str = "Window 1";
const NEW_WINDOW: &str = "New window";

/// A place offered while picking a profile.
#[derive(Clone, PartialEq)]
enum Spot {
    Window(u8),
    New,
    Fill(String),
}

struct Guest<'a> {
    game: Box<Game<'a>>,
    /// The profile it plays as; `None` while it picks one.
    name: Option<String>,
    /// Its seat (2 to `MAX_PLAYERS`): its number (P2...), its control socket, its sounds.
    seat: usize,
    options: PathBuf,
    /// The controller it plays with.
    pad: Option<PadId>,
    /// The window it plays in, by number.
    window: u8,
    /// Its buttons and focus targets while another player's game runs.
    widgets: widgets::Saved,
    /// Its profile got into the world: turned away after that (a kick), it leaves; before, it
    /// picks again.
    entered: bool,
    bumpers: [bool; 2],
}

pub(super) struct Split<'a> {
    pub(super) host: Game<'a>,
    guests: Vec<Guest<'a>>,
    /// `HORIZON_SPLIT=N`: players still to add once P1 is in a world.
    test_players: usize,
    /// `HORIZON_SPLIT` is set: splitscreen is on for this run, without touching the saved switch.
    forced: bool,
    /// While another player is in, the windows players play in, each with its compositor: the
    /// game's first, then the others (else P1 draws on its window).
    surfaces: Vec<Surface>,
    /// The windows players play in (Window 1, the game's, always first), each free or filling a
    /// monitor, and P1's.
    windows: Vec<layout::Window>,
    p1_window: u8,
    /// Seats in the order their views fill a shared window.
    order: Vec<usize>,
    /// Whether Window 1 is shown (hidden while nobody plays in it): set once per change, since
    /// Wayland can't say whether a window is visible.
    window1_shown: bool,
    /// Where each numbered window was when it closed, to open there again (this session).
    window_rects: std::collections::HashMap<u8, (LogicalPosition<f64>, LogicalSize<f64>)>,
    /// The monitors with their names, left to right, and the one Window 1 is on: read while
    /// splitscreen is on, once a second (`refresh_monitors`), and when it last read them.
    screens: Vec<(String, MonitorHandle)>,
    main_screen: Option<String>,
    screens_read: Option<Instant>,
    /// `HORIZON_SPLIT_WINDOWS`: each seat's window, `1`, `2`... or `m<monitor>`.
    window_plan: Vec<String>,
    /// The views the players were last fitted to.
    applied: Vec<View>,
    /// The player keyboard and mouse drive (0 = P1), and the pointer in the window.
    focus: usize,
    cursor: PhysicalPosition<f64>,
    limiter: horizon_client::window::FramerateLimiter,
    clock: horizon_client::window::SystemClock,
    /// P1's resources, shared by the other players' games for the process's lifetime.
    res: Option<&'static Arc<super::Resources>>,
    /// Whose game runs now (0 = P1), and P1's buttons while another's does.
    turn: usize,
    host_widgets: widgets::Saved,
    /// The controllers while several play (`pads`), and P1's.
    pads: PadRouter,
    p1_pad: Option<PadId>,
    /// Pads whose buttons still count as the press that gave them: released first.
    fresh: Vec<PadId>,
    /// Controllers offered to join, each since when: several may join at once.
    offers: Vec<(PadId, Instant)>,
    /// Players whose controller disconnected, longest waiting first.
    lost: Vec<pads::Lost<PadId>>,
}

impl<'a> Split<'a> {
    pub(super) fn new(host: Game<'a>) -> Self {
        let split = std::env::var("HORIZON_SPLIT").ok();
        let test_players = split
            .as_ref()
            .and_then(|n| n.parse::<usize>().ok())
            .map_or(0, |n| n.clamp(1, MAX_PLAYERS) - 1);
        Self {
            host,
            guests: Vec::new(),
            test_players,
            forced: split.is_some(),
            surfaces: Vec::new(),
            windows: vec![layout::Window {
                number: 1,
                placement: Placement::Free(None),
            }],
            p1_window: 1,
            order: Vec::new(),
            window1_shown: true,
            window_rects: Default::default(),
            screens: Vec::new(),
            main_screen: None,
            screens_read: None,
            window_plan: std::env::var("HORIZON_SPLIT_WINDOWS")
                .map(|w| w.split(',').map(|s| s.trim().to_owned()).collect())
                .unwrap_or_default(),
            applied: Vec::new(),
            focus: 0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            limiter: horizon_client::window::FramerateLimiter::new(0),
            clock: Default::default(),
            res: None,
            turn: 0,
            host_widgets: Default::default(),
            pads: Default::default(),
            p1_pad: None,
            fresh: Vec::new(),
            offers: Vec::new(),
            lost: Vec::new(),
        }
    }

    fn players(&self) -> usize {
        1 + self.guests.len()
    }

    /// Player `i`'s turn (0 = P1): its options file, and its own buttons and focus targets, which
    /// screens record while drawing and the control socket and the controller's cursor read.
    fn turn(&mut self, i: usize) {
        if let Some(game) = self.game(i) {
            horizon_client::hud::GUI_SCALE.store(
                game.settings.gui_scale,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        if i == self.turn {
            return;
        }
        let current = widgets::take();
        match self.turn {
            0 => self.host_widgets = current,
            t => {
                if let Some(g) = self.guests.get_mut(t - 1) {
                    g.widgets = current;
                }
            }
        }
        let next = match i {
            0 => std::mem::take(&mut self.host_widgets),
            _ => self
                .guests
                .get_mut(i - 1)
                .map(|g| std::mem::take(&mut g.widgets))
                .unwrap_or_default(),
        };
        widgets::put(next);
        OPTIONS.set(
            self.guests
                .get(i.wrapping_sub(1))
                .map(|g| g.options.clone()),
        );
        self.turn = i;
    }

    /// Player `i`'s game (0 = P1).
    fn game(&self, i: usize) -> Option<&Game<'a>> {
        match i {
            0 => Some(&self.host),
            _ => self.guests.get(i - 1).map(|g| &*g.game),
        }
    }

    fn game_mut(&mut self, i: usize) -> Option<&mut Game<'a>> {
        match i {
            0 => Some(&mut self.host),
            _ => self.guests.get_mut(i - 1).map(|g| &mut *g.game),
        }
    }

    /// Player `i`'s game, in its turn, and its window's id.
    fn player(&mut self, i: usize) -> Option<(&mut dyn ApplicationHandler, WindowId)> {
        self.turn(i);
        let focus = self.focus;
        let game = self.game_mut(i)?;
        game.keyboard_mouse = i == focus;
        let id = game.window.as_ref()?.id();
        Some((game, id))
    }

    /// Give keyboard and mouse to one player, releasing the previous owner's held controls.
    fn handoff(&mut self, next: usize) {
        if next != self.focus {
            self.turn(self.focus);
            if let Some(game) = self.game_mut(self.focus) {
                for key in std::mem::take(&mut game.physical_keys) {
                    game.native_key_event(key, false, false);
                }
                for key in std::mem::take(&mut game.keys) {
                    if let Err(e) = game.mapped_key(key, false, false) {
                        log::warn!("Horizon: splitscreen: releasing input: {e}");
                    }
                }
                for button in std::mem::take(&mut game.physical_buttons) {
                    game.extras_mouse(button, false);
                }
                game.drag_button = None;
                game.modifiers = Default::default();
                game.left = false;
                game.right = false;
                game.inventory_mouse_release(false);
                game.inventory_mouse_release(true);
                game.inventory_pick_release();
                game.loom_release();
                game.creative_screen.release();
                game.ui.options.release();
                game.dragging = false;
                game.stats_release();
                game.world_options_release();
                game.advancements_mouse(false, [0, 0]);
                if let Some(title) = &mut game.title {
                    title.release();
                    title.release_field();
                }
                game.capture(false);
                game.keyboard_mouse = false;
            }
        }
        self.focus = next;
        for i in 0..self.players() {
            if let Some(game) = self.game_mut(i) {
                game.keyboard_mouse = i == next;
            }
        }
    }

    /// The pointer as the focused player sees it: from its view's corner.
    fn cursor_in_focus(&self) -> PhysicalPosition<f64> {
        let [x, y] = self.views().get(self.focus).map_or([0, 0], |v| v.1);
        let at = self.cursor;
        PhysicalPosition::new(at.x - f64::from(x), at.y - f64::from(y))
    }

    /// Player `i`'s name (0 = P1).
    fn name(&self, i: usize) -> String {
        match i {
            0 => "P1".into(),
            _ => self.guests[i - 1]
                .name
                .clone()
                .unwrap_or_else(|| "New player".into()),
        }
    }

    /// Where `HORIZON_SPLIT_WINDOWS` puts the player in `seat`: a window by number, or a new
    /// one filling a monitor.
    fn planned_window(&mut self, i: usize, seat: usize) {
        let Some(plan) = self.window_plan.get(seat - 1).cloned() else {
            return;
        };
        if let Some(k) = plan.strip_prefix('m') {
            let monitor = k
                .parse::<usize>()
                .ok()
                .and_then(|k| self.screens.get(k.checked_sub(1)?));
            let Some(name) = monitor.map(|(name, _)| name.clone()) else {
                log::warn!("Horizon: splitscreen: no monitor {k}");
                return;
            };
            let number = (2..=u8::MAX)
                .find(|n| self.windows.iter().all(|w| w.number != *n))
                .unwrap_or(u8::MAX);
            self.windows.push(layout::Window {
                number,
                placement: Placement::Fill(name),
            });
            return self.set_window(i, number);
        }
        let Ok(number) = plan.parse::<u8>() else {
            return;
        };
        if !self.windows.iter().any(|w| w.number == number) {
            self.windows.push(layout::Window {
                number,
                placement: Placement::Free(None),
            });
        }
        self.set_window(i, number);
    }

    /// Another player joins: P1 renders offscreen on the same device and a compositor presents
    /// on its window; the world is meshed again for the new renderer.
    #[inline(never)]
    fn enable(&mut self) -> Result<()> {
        let window = self.host.window.clone().ok_or("no window")?;
        let mut old = self.host.renderer.take().ok_or("no renderer")?;
        old.release_surface();
        let api = self.host.settings.horizon.api;
        let batching = self.host.settings.horizon.translucent_batching;
        let shader_pack = super::shader_packs::picked_pack_draws_flat(&self.host.settings);
        let built = build_renderer(
            window.clone(),
            true,
            &self.host.pack,
            &self.host.gui,
            Some(&old),
            None,
            api,
            batching,
            shader_pack,
        );
        let renderer = match built {
            Ok(r) => r,
            Err(e) => {
                old.recreate_surface()?;
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        // A window it can't present on: P1 keeps drawing on it as before.
        let compositor = match old.compositor(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                drop(renderer);
                old.recreate_surface()?;
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        self.host.renderer = Some(renderer);
        self.host.options.offscreen = true;
        self.host.options.paced_by_split = true;
        // The new renderer takes the old one's present mode, settings and all: P1's again.
        self.host.apply_settings();
        self.host.clear_lod();
        self.host.all_sections_changed();
        self.surfaces = vec![Surface {
            key: 1,
            placement: Placement::Free(None),
            size: window.inner_size(),
            window,
            compositor,
            focused: true,
            title: String::new(),
        }];
        self.planned_window(0, 1);
        Ok(())
    }

    /// Splitscreen off or P1 alone again: it draws on its window again.
    #[inline(never)]
    fn disable(&mut self, event_loop: &ActiveEventLoop, watching: bool) -> Result<()> {
        for k in (0..self.guests.len()).rev() {
            self.remove_guest(k);
        }
        self.applied.clear();
        self.order.clear();
        if !watching {
            self.stop_routing();
        }
        self.p1_window = 1;
        self.windows.truncate(1);
        self.surfaces.truncate(1);
        let Some(main) = self.surfaces.pop() else {
            return Ok(());
        };
        let window = main.window.clone();
        window.set_visible(true);
        self.window1_shown = true;
        self.host.window = Some(window.clone());
        let old = self.host.renderer.take().ok_or("no renderer")?;
        old.take_back(main.compositor);
        let api = self.host.settings.horizon.api;
        let batching = self.host.settings.horizon.translucent_batching;
        let shader_pack = super::shader_packs::picked_pack_draws_flat(&self.host.settings);
        let built = build_renderer(
            window.clone(),
            false,
            &self.host.pack,
            &self.host.gui,
            Some(&old),
            None,
            api,
            batching,
            shader_pack,
        );
        let renderer = match built {
            Ok(r) => r,
            Err(e) => {
                // P1 stays offscreen, presented as before (alone now).
                let compositor = old.compositor(window)?;
                self.surfaces.push(Surface { compositor, ..main });
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        drop(old);
        self.host.renderer = Some(renderer);
        self.host.options.offscreen = false;
        self.host.options.paced_by_split = false;
        // Alone again: the pointer is P1's, its view the whole window.
        self.focus = 0;
        self.host.keyboard_mouse = true;
        self.host.view_at = [0, 0];
        // From the offscreen renderer it took no vsync: P1's settings again.
        self.host.apply_settings();
        self.host.clear_lod();
        self.host.all_sections_changed();
        let id = window.id();
        self.host
            .window_event(event_loop, id, WindowEvent::Resized(window.inner_size()));
        Ok(())
    }

    /// P1 is playing in its own world, which others can join.
    fn in_world(&self) -> bool {
        self.host.title.is_none() && self.host.player.is_some() && self.host.integrated.is_some()
    }

    /// The pads' players who accepted, and the testing players (`HORIZON_SPLIT`, one a frame),
    /// join: whether anyone did.
    fn admit(&mut self, _event_loop: &ActiveEventLoop, pad_joins: Vec<PadId>) -> bool {
        let mut changed = false;
        for pad in pad_joins {
            match self.join(Some(pad), None) {
                Ok(()) => changed = true,
                Err(e) => log::warn!("Horizon: splitscreen: a player could not join: {e}"),
            }
        }
        if self.in_world() && self.test_players > 0 {
            // HORIZON_SPLIT_PICK: they pick a profile like a controller's player, else P2...
            let name = std::env::var_os("HORIZON_SPLIT_PICK")
                .is_none()
                .then(|| format!("P{}", self.free_seat().unwrap_or(0)));
            match self.join(None, name) {
                Ok(()) => {
                    self.test_players -= 1;
                    changed = true;
                }
                Err(e) => {
                    // One player failing doesn't stop the others: P1 keeps playing.
                    log::warn!("Horizon: splitscreen player couldn't join: {e}");
                    self.test_players = 0;
                }
            }
        }
        changed
    }

    /// Each window shows its players' frames, then the GPU wait, once, for all of them. Only
    /// the game's window waits for the display (vsync): windows each waiting would divide the rate.
    fn present(&mut self) {
        let views = self.views();
        let renderers: Vec<_> = std::iter::once(&self.host.renderer)
            .chain(self.guests.iter().map(|g| &g.game.renderer))
            .collect();
        let vsync = self.host.settings.vsync;
        let paced = windows::paced_surface(self.window1_shown, self.surfaces.len());
        for (k, surface) in self.surfaces.iter_mut().enumerate() {
            let frames: Vec<_> = renderers
                .iter()
                .zip(&views)
                .filter(|(_, v)| v.0 == k)
                .filter_map(|(r, v)| Some((r.as_ref()?.offscreen_frame()?, v.1)))
                .collect();
            surface.compositor.set_vsync(vsync && k == paced);
            surface.compositor.present(&frames);
        }
        // One player's CPU work overlapped the GPU drawing another's.
        let renderers = std::iter::once(&mut self.host.renderer)
            .chain(self.guests.iter_mut().map(|g| &mut g.game.renderer));
        for renderer in renderers.flatten() {
            renderer.paced_elsewhere = true;
            renderer.wait_in_flight();
        }
    }
}

impl ApplicationHandler for Split<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.host.resumed(event_loop);
        if !self.surfaces.is_empty() {
            self.relayout(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.surfaces.is_empty() {
            return self.host.window_event(event_loop, id, event);
        }
        let Some(surface) = self.surfaces.iter().position(|s| s.window.id() == id) else {
            return;
        };
        match event {
            WindowEvent::Resized(size) => {
                let s = &mut self.surfaces[surface];
                s.size = size;
                s.compositor.resize(size.width, size.height);
                self.relayout(event_loop);
            }
            // The views draw themselves offscreen; the compositors present in about_to_wait.
            WindowEvent::RedrawRequested => {}
            // The game's window: P1's to handle (it saves the world, then quits). Another
            // window: its players leave.
            WindowEvent::CloseRequested if surface == 0 => {
                if let Some((host, id)) = self.player(0) {
                    host.window_event(event_loop, id, WindowEvent::CloseRequested);
                }
            }
            WindowEvent::CloseRequested => {
                if let Err(e) = self.close_window(surface) {
                    log::warn!("Horizon: splitscreen: {e}");
                }
            }
            // Players' pads count while any of the windows has focus.
            WindowEvent::Focused(focused) => {
                self.surfaces[surface].focused = focused;
                let any = self.surfaces.iter().any(|s| s.focused);
                for i in 0..self.players() {
                    if let Some((player, id)) = self.player(i) {
                        player.window_event(event_loop, id, WindowEvent::Focused(any));
                    }
                }
            }
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                self.cursor = position;
                let position = self.cursor_in_focus();
                let here = self.views().get(self.focus).is_some_and(|v| v.0 == surface);
                if here && let Some((player, id)) = self.player(self.focus) {
                    player.window_event(
                        event_loop,
                        id,
                        WindowEvent::CursorMoved {
                            device_id,
                            position,
                        },
                    );
                }
            }
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                ..
            } => {
                // A click on another player's view gives it keyboard and mouse, unless the
                // focused player holds the mouse (playing): Esc frees it first, as in vanilla.
                let (x, y) = (self.cursor.x as u32, self.cursor.y as u32);
                let clicked = self.views().iter().position(|(s, [vx, vy], size)| {
                    *s == surface
                        && (*vx..vx + size.width).contains(&x)
                        && (*vy..vy + size.height).contains(&y)
                });
                let holding = self.game(self.focus).is_some_and(|g| g.captured);
                if let Some(clicked) = clicked
                    && clicked != self.focus
                    && !holding
                {
                    self.handoff(clicked);
                    self.tag(clicked, "Keyboard and mouse");
                    let position = self.cursor_in_focus();
                    if let Some((player, id)) = self.player(clicked) {
                        player.window_event(
                            event_loop,
                            id,
                            WindowEvent::CursorMoved {
                                device_id,
                                position,
                            },
                        );
                    }
                }
                // Only into the focused player's own window: a click in another window
                // is not that player's attack or use.
                let here = self.views().get(self.focus).is_some_and(|v| v.0 == surface);
                if here && let Some((player, id)) = self.player(self.focus) {
                    player.window_event(event_loop, id, event);
                }
            }
            _ => {
                // Presses (keys, wheel) from a window the focused player is not in stay there;
                // releases still pass, so a key held across a window change can't stick.
                let here = self.views().get(self.focus).is_some_and(|v| v.0 == surface);
                let press = match &event {
                    WindowEvent::KeyboardInput { event: key, .. } => {
                        key.state == ElementState::Pressed
                    }
                    WindowEvent::MouseWheel { .. } => true,
                    _ => false,
                };
                if (here || !press)
                    && let Some((player, id)) = self.player(self.focus)
                {
                    player.window_event(event_loop, id, event);
                }
            }
        }
        self.turn(0);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        // Alone: exactly P1's path, as in window_event (no GUI-scale store per mouse motion).
        if self.surfaces.is_empty() {
            return self.host.device_event(event_loop, id, event);
        }
        if let Some((player, _)) = self.player(self.focus) {
            player.device_event(event_loop, id, event);
        }
        self.turn(0);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let on = self.host.settings.extras.splitscreen || self.forced;
        // Only splitscreen places windows on monitors. Listing them is a round trip to the
        // window server (macOS: 0.1-0.3 ms, more on a busy machine), so not every frame.
        if (on || !self.surfaces.is_empty())
            && self
                .screens_read
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
        {
            self.screens_read = Some(Instant::now());
            self.refresh_monitors();
        }
        self.host.ui.options.split_joined = self.guests.iter().any(|g| g.name.is_some());
        // Off, or the last other player gone: P1 draws on its window again.
        if !self.surfaces.is_empty()
            && (!on || self.guests.is_empty())
            && let Err(e) = self.disable(event_loop, on)
        {
            log::warn!("Horizon: splitscreen could not switch back: {e}");
        }
        if self.surfaces.is_empty() {
            if !on {
                self.stop_routing();
                return self.host.about_to_wait(event_loop);
            }
            // Alone, the controllers watched for someone joining; P1's own drives P1, its
            // output (rumble, triggers) through the split's reader for P1's turn.
            let pad_joins = self.route_pads(true);
            self.turn(0);
            if let Some(id) = self.p1_pad {
                self.pads.lend_output(&mut self.host.pad, id);
            }
            self.host.about_to_wait(event_loop);
            if self.p1_pad.is_some() {
                self.pads.reclaim_output(&mut self.host.pad);
            }
            if self.admit(event_loop, pad_joins) {
                self.relayout(event_loop);
            }
            return;
        }
        let pad_joins = self.route_pads(false);
        self.turn(0);
        self.host.about_to_wait(event_loop);
        let mut changed = self.drop_out() | self.admit(event_loop, pad_joins);
        self.sync_profiles();
        changed |= self.pick_profiles();
        changed |= self.sync_surfaces(event_loop);
        if changed || self.views() != self.applied {
            self.relayout(event_loop);
        }
        // Each player's badge: who it is, shown a moment when something changes for it.
        self.host.split_hud.badge = Some("P1".into());
        for g in &mut self.guests {
            g.game.split_hud.badge = Some(g.name.clone().unwrap_or_else(|| "New player".into()));
        }
        let master = self.host.audio.master;
        for i in 0..self.guests.len() {
            self.turn(i + 1);
            // One set of speakers: P1's volume.
            self.guests[i].game.audio.master = master;
            self.guests[i].game.about_to_wait(event_loop);
            // A player's game stops the event loop on an error, as P1's does: say why.
            if let Some(e) = &self.guests[i].game.error {
                log::error!("Horizon: splitscreen: P{}: {e}", self.guests[i].seat);
            }
            // Choosing its profile, the bumpers move its view (its pad as its game read it: the
            // controller, or `horizon-ctl pad`).
            let place = self.stop_label(i + 1);
            let game = &mut self.guests[i].game;
            if game.ui.options.page() == horizon_client::options_ui::Page::SplitProfiles
                && game.screen == super::Screen::Options
            {
                game.ui.options.split_profiles.place = place;
                let state = game.pad.state;
                let held = [
                    state.held(Button::LeftBumper),
                    state.held(Button::RightBumper),
                ];
                let before = std::mem::replace(&mut self.guests[i].bumpers, held);
                if held[0] && !before[0] {
                    self.move_with_bumpers(i + 1, true);
                } else if held[1] && !before[1] {
                    self.move_with_bumpers(i + 1, false);
                }
            }
        }
        self.turn(0);
        self.present();
        // One pace for all: the fastest any player wants, so an idle one (vanilla's AFK limit)
        // can't slow the others. With vsync the display paces anything at or above its refresh
        // rate; the limiter on top would miss refreshes.
        let limit = std::iter::once(self.host.framerate_limit())
            .chain(self.guests.iter().map(|g| g.game.framerate_limit()))
            .max()
            .unwrap_or(60);
        let refresh_hz = self
            .surfaces
            .get(windows::paced_surface(
                self.window1_shown,
                self.surfaces.len(),
            ))
            .and_then(|s| s.window.current_monitor()?.refresh_rate_millihertz())
            .map_or(60, |mhz| mhz.div_ceil(1000) as i32);
        if !(self.host.settings.vsync && limit >= refresh_hz) {
            horizon_client::window::end_frame(&mut self.limiter, limit, &mut self.clock);
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        for i in (0..self.players()).rev() {
            if let Some((player, _)) = self.player(i) {
                player.exiting(event_loop);
            }
        }
        self.turn(0);
    }
}
