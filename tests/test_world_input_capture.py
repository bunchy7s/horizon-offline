"""Exercise the client's capture methods without opening or focusing an OS window.

Capture fixtures substitute winit; the raw-motion fixture uses the real mouse
accumulator and rotation math. Method bodies come from the client, so assertions
observe capture and camera outcomes. Native OS routing is checked separately by
`tools/splitscreen_e2e.py --only native_pointer` on macOS. Vanilla 26.3 reference:
LevelLoadingScreen.tick/onClose, Gui.setScreen, MouseHandler.grabMouse/releaseMouse.
"""
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


def method(source, name):
    start = source.index(f"    fn {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


def function(source, name):
    start = source.index(f"pub fn {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


class WorldInputCaptureTests(unittest.TestCase):
    def test_loading_completion_wiring(self):
        game = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        remote = (ROOT / "crates/horizon-client/src/game/remote.rs").read_text(encoding="utf-8")
        spawn = method(game, "spawn_player")
        self.assertLess(spawn.index("self.player = Some(player)"),
                        spawn.index("self.capture_loaded_world()"))
        # ClientPacketListener.tick: the level load tracker ends the loading screen,
        # the server hears PlayerLoaded, the mouse is captured, then the player moves.
        ready = remote[remote.index("    fn tick_client_load("):]
        self.assertLess(ready.index("r.loaded_sent = true"),
                        ready.index("self.capture_loaded_world()"))
        level = remote[remote.index("    fn tick_remote_level("):]
        self.assertLess(level.index("self.tick_client_load()"),
                        level.index("let stick = self.pad_move()"))

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_capture_lifecycle(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        mouse = (ROOT / "crates/horizon-client-input/src/mouse.rs").read_text(encoding="utf-8")
        fixture = r'''
use std::cell::Cell;
#[derive(Clone, Copy, PartialEq)]
enum Screen { Playing, Paused, Inventory, Chat }
enum CursorGrabMode { Locked, Confined, None }
mod winit {
    pub mod dpi {
        #[derive(Clone, Copy)]
        pub struct PhysicalPosition<T> { pub x: T, pub y: T }
        impl<T> PhysicalPosition<T> { pub fn new(x: T, y: T) -> Self { Self { x, y } } }
        pub struct PhysicalSize<T> { pub width: T, pub height: T }
        impl From<PhysicalSize<u32>> for [u32; 2] {
            fn from(size: PhysicalSize<u32>) -> Self { [size.width, size.height] }
        }
    }
}
use winit::dpi::{PhysicalPosition, PhysicalSize};
struct HudBatch { scale: f32 }
impl HudBatch { fn new(_size: [u32; 2]) -> Self { Self { scale: 2.0 } } }
struct Config { width: u32, height: u32 }
struct Renderer { config: Config }
#[derive(Default)]
struct Window { grabs: Cell<u32>, visible: Cell<bool>, fail: bool, warps: Cell<u32>, at: Cell<(f64, f64)> }
impl Window {
    fn inner_size(&self) -> PhysicalSize<u32> { PhysicalSize { width: 854, height: 480 } }
    fn set_cursor_position(&self, position: PhysicalPosition<f64>) -> Result<(), ()> {
        self.warps.set(self.warps.get() + 1);
        self.at.set((position.x, position.y));
        Ok(())
    }
    fn set_cursor_grab(&self, mode: CursorGrabMode) -> Result<(), ()> {
        if !matches!(mode, CursorGrabMode::None) {
            self.grabs.set(self.grabs.get() + 1);
        }
        if self.fail { Err(()) } else { Ok(()) }
    }
    fn set_cursor_visible(&self, visible: bool) { self.visible.set(visible); }
}
#[derive(Default)]
struct Mouse { ignore_first: bool }
impl Mouse { fn set_ignore_first_move(&mut self) { self.ignore_first = true; } }
struct Remote;
#[derive(Default)]
struct Options { offscreen: bool }
#[derive(Default)]
struct ServerPacks { prompt: Option<()> }
struct Game {
    focused: bool, agent_pointer: bool, window: Option<Window>, mouse: Mouse,
    keyboard_mouse: bool, split_window: bool, view_at: [u32; 2],
    captured: bool, regrab_on_refocus: bool, title: Option<()>, screen: Screen,
    remote: Option<Remote>,
    mining: bool, lan_acting: Option<u32>, renderer: Option<Renderer>, cursor: (f32, f32),
    server_packs: ServerPacks, options: Options,
}
impl Default for Game {
    fn default() -> Self {
        Self { focused: true, agent_pointer: false, window: Some(Window::default()),
            keyboard_mouse: true, split_window: false, view_at: [0, 0],
            mouse: Mouse::default(), captured: false, regrab_on_refocus: false, title: None,
            screen: Screen::Playing, remote: None, mining: false, lan_acting: None,
            renderer: None, cursor: (0.0, 0.0), server_packs: ServerPacks::default(),
            options: Options::default() }
    }
}
HELPER
impl Game {
    fn stop_mining(&mut self) { self.mining = false; }
    fn reset_inventory_clicks(&mut self) {}
    fn restore_held_keys(&mut self) {}
    fn dialog_open(&self) -> bool { false }
METHODS
}
fn main() {
    // The server pack prompt keeps the pointer: nothing grabs it while it is open.
    {
        let mut game = Game::default();
        game.server_packs.prompt = Some(());
        game.capture(true);
        assert!(!game.captured);
        game.server_packs.prompt = None;
        game.capture(true);
        assert!(game.captured);
    }
    // New and existing worlds, returning to title, and reopening a world.
    for remote in [false, true] {
        let mut game = Game::default();
        if remote { game.remote = Some(Remote); }
        for _ in 0..3 {
            game.title = Some(());
            game.capture(false);
            game.capture_loaded_world();
            assert!(!game.captured);
            game.title = None;
            game.capture_loaded_world();
            assert!(game.captured);
            assert!(game.mouse.ignore_first);
            assert!(!game.window.as_ref().unwrap().visible.get());
            for screen in [Screen::Paused, Screen::Inventory] {
                game.screen = screen;
                game.mining = true;
                game.capture(false);
                assert!(!game.mining);
                game.capture_loaded_world();
                assert!(!game.captured);
                assert!(game.window.as_ref().unwrap().visible.get());
                game.screen = Screen::Playing;
                game.capture(true);
                assert!(game.captured);
            }
        }
        // Alt+Tab during loading: no native grab or agent-pointer reset.
        game.focused = false;
        game.mining = true;
        game.capture(false);
        assert!(!game.mining);
        assert!(!game.captured);
        assert!(game.window.as_ref().unwrap().visible.get());
        game.agent_pointer = true;
        let grabs = game.window.as_ref().unwrap().grabs.get();
        game.capture_loaded_world();
        game.capture(true);
        assert!(!game.captured);
        assert!(game.agent_pointer);
        assert_eq!(game.window.as_ref().unwrap().grabs.get(), grabs);
        game.focused = true;
        assert!(!game.captured); // Setting the flag is not a grab: `capture` is.
        game.capture(true); // `regrab_on_refocus`, or a click on the world.
        assert!(game.captured);
        assert_eq!(game.cursor, (213.5, 120.0)); // The grab puts the mouse at the centre.
    }
    // A window that held the pointer asks for it back when the focus returns:
    // the system drops a grabbed pointer with the focus and vanilla's grab
    // outlives it (`MouseHandler.mouseGrabbed` stays true while GLFW keeps its
    // cursor mode), so no click is needed to lock it again.
    {
        let mut game = Game::default();
        game.capture(true);
        assert!(game.captured);
        let grabs = game.window.as_ref().unwrap().grabs.get();
        game.focused = false; // Alt+Tab: the release `window_event` makes.
        game.regrab_on_refocus = game.captured;
        game.capture(false);
        assert!(!game.captured);
        game.focused = true;
        game.regrab_on_refocus();
        assert!(game.captured);
        assert!(game.window.as_ref().unwrap().grabs.get() > grabs);
        assert!(game.mouse.ignore_first);
        // Taken once: a focus gain that follows a screen's release takes nothing.
        game.capture(false);
        let grabs = game.window.as_ref().unwrap().grabs.get();
        game.regrab_on_refocus();
        assert!(!game.captured);
        assert_eq!(game.window.as_ref().unwrap().grabs.get(), grabs);
        // A screen open while the window was away keeps the pointer free.
        game.regrab_on_refocus = true;
        game.screen = Screen::Inventory;
        game.regrab_on_refocus();
        assert!(!game.captured);
        assert_eq!(game.window.as_ref().unwrap().grabs.get(), grabs);
        // A window that never held the pointer does not take it on focus.
        game.screen = Screen::Playing;
        game.regrab_on_refocus();
        assert!(!game.captured);
        assert_eq!(game.window.as_ref().unwrap().grabs.get(), grabs);
        // A prompt over the world keeps the pointer, as the grab itself checks.
        game.regrab_on_refocus = true;
        game.server_packs.prompt = Some(());
        game.regrab_on_refocus();
        assert!(!game.captured);
    }
    // A controller-only player's screens leave the shared native pointer alone.
    {
        let mut game = Game::default();
        game.keyboard_mouse = false;
        game.captured = true;
        game.mining = true;
        for capture in [false, true] {
            game.capture(capture);
            assert!(!game.captured);
            assert!(!game.mining);
            let window = game.window.as_ref().unwrap();
            assert_eq!(window.grabs.get(), 0);
            assert_eq!(window.warps.get(), 0);
            assert!(!window.visible.get()); // No change to native visibility.
        }
    }
    // The owning player's warp includes its view's offset in the shared window;
    // its menu cursor remains in local GUI coordinates.
    {
        let mut game = Game::default();
        game.view_at = [1200, 400];
        game.renderer = Some(Renderer { config: Config { width: 1000, height: 600 } });
        game.capture(true);
        assert!(game.captured);
        assert_eq!(game.window.as_ref().unwrap().at.get(), (1700.0, 700.0));
        assert_eq!(game.cursor, (250.0, 150.0));
        game.capture(false);
        assert!(!game.captured);
        assert_eq!(game.window.as_ref().unwrap().at.get(), (1700.0, 700.0));
        assert_eq!(game.cursor, (250.0, 150.0));
    }
    // A composited view still has a visible native window. Its owner must grab,
    // hide and release the real pointer even though its renderer is offscreen.
    {
        let mut game = Game::default();
        game.options.offscreen = true;
        game.split_window = true;
        game.window.as_ref().unwrap().visible.set(true);
        game.capture(true);
        assert!(game.captured);
        let window = game.window.as_ref().unwrap();
        assert_eq!(window.grabs.get(), 1, "split owner must grab the native pointer");
        assert!(!window.visible.get());
        game.capture(false);
        assert!(!game.captured);
        assert!(game.window.as_ref().unwrap().visible.get());
    }
    // `--offscreen` shows no window: the grab is the game's own, and the system
    // pointer is never grabbed, warped or hidden.
    {
        let mut game = Game::default();
        game.options.offscreen = true;
        game.window.as_ref().unwrap().visible.set(true);
        game.capture_loaded_world();
        assert!(game.captured);
        assert!(game.mouse.ignore_first);
        game.capture(false);
        assert!(!game.captured);
        game.capture(true);
        assert!(game.captured);
        let window = game.window.as_ref().unwrap();
        assert_eq!((window.grabs.get(), window.warps.get()), (0, 0));
        assert!(window.visible.get());
    }
    let mut game = Game::default();
    // On a server the chat is ChatScreen (#708): loading never grabs over it.
    game.remote = Some(Remote);
    game.screen = Screen::Chat;
    game.capture_loaded_world();
    assert!(!game.captured);
    assert_eq!(game.window.as_ref().unwrap().grabs.get(), 0);
    game.remote = None;
    game.screen = Screen::Playing;
    // Grabbing and releasing put the pointer at the window's centre
    // (MouseHandler.grabMouse / releaseMouse), in the surface's pixels when it
    // has one; releasing an already released pointer leaves it where it is.
    // Windows also warps before the Locked grab, which clips the pointer
    // where it is (#6076).
    let grab_warps = if cfg!(windows) { 2 } else { 1 };
    game.renderer = Some(Renderer { config: Config { width: 1000, height: 600 } });
    game.capture(true);
    assert!(game.captured);
    let window = game.window.as_ref().unwrap();
    assert_eq!((window.warps.get(), window.at.get()), (grab_warps, (500.0, 300.0)));
    game.capture(false);
    let window = game.window.as_ref().unwrap();
    assert_eq!(window.warps.get(), grab_warps + 2);
    assert_eq!(game.cursor, (250.0, 150.0));
    game.cursor = (1.0, 1.0);
    game.capture(false);
    assert_eq!(game.window.as_ref().unwrap().warps.get(), grab_warps + 2);
    assert_eq!(game.cursor, (1.0, 1.0));
    game.renderer = None;
    game.window.as_mut().unwrap().fail = true;
    game.capture_loaded_world();
    assert!(!game.captured);
    assert!(game.window.as_ref().unwrap().visible.get());
}
'''
        capture = method(source, "capture").replace(
            "horizon_client::mouse::mouse_position_after_grab_change",
            "mouse_position_after_grab_change")
        fixture = fixture.replace("HELPER", function(mouse, "mouse_position_after_grab_change"))
        fixture = fixture.replace("METHODS", capture + "\n" +
                                  method(source, "capture_loaded_world") + "\n" +
                                  method(source, "regrab_on_refocus"))
        with tempfile.TemporaryDirectory(prefix="horizon-input-") as folder:
            folder = Path(folder)
            rust = folder / "capture.rs"
            binary = folder / ("capture.exe" if os.name == "nt" else "capture")
            rust.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                           check=True, capture_output=True, text=True, timeout=60)
            subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=10)


    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_native_split_mouse_turns_only_its_owner(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        fixture = r'''
use std::time::Instant;
#[path = "MOUSE_PATH"] mod mouse;
use mouse::{MouseLook, LookOptions, Rotation};
struct ActiveEventLoop;
struct DeviceId;
enum DeviceEvent { MouseMotion { delta: (f64, f64) }, Other }
#[derive(PartialEq)] enum Screen { Playing, Paused }
struct Options { offscreen: bool }
struct Game {
    pending_save: Option<()>, captured: bool, focused: bool, keyboard_mouse: bool, split_window: bool,
    options: Options, screen: Screen, mouse: MouseLook, last_input: Instant,
}
impl Game {
METHOD
}
fn main() {
    // (offscreen renderer, compositor, keyboard/mouse owner, active, gameplay, saving).
    for (offscreen, split, owner, active, playing, saving, turns) in [
        (false, false, true, true, true, false, true),
        (true, true, true, true, true, false, true),
        (true, true, false, true, true, false, false),
        (true, false, true, true, true, false, false),
        (true, true, true, false, true, false, false),
        (true, true, true, true, false, false, false),
        (true, true, true, true, true, true, false),
    ] {
        let mut game = Game {
            pending_save: saving.then_some(()), captured: true, focused: active,
            keyboard_mouse: owner, split_window: split, options: Options { offscreen },
            screen: if playing { Screen::Playing } else { Screen::Paused },
            mouse: MouseLook::default(), last_input: Instant::now(),
        };
        game.device_event(&ActiveEventLoop, DeviceId, DeviceEvent::MouseMotion { delta: (0.0, 0.0) });
        game.device_event(&ActiveEventLoop, DeviceId, DeviceEvent::MouseMotion { delta: (20.0, -10.0) });
        let look = LookOptions { sensitivity: 0.5, smooth_camera: false, scoping: false, invert_x: false, invert_y: false };
        let (x, y) = game.mouse.handle_accumulated_movement(1.0, true, true, look).unwrap();
        let mut rotation = Rotation { x_rot: 0.0, y_rot: 0.0, x_rot_o: 0.0, y_rot_o: 0.0 };
        rotation.turn(x, y);
        let want = if turns { (3.0, -1.5) } else { (0.0, 0.0) };
        assert!((rotation.y_rot - want.0).abs() < 0.00001 && (rotation.x_rot - want.1).abs() < 0.00001,
            "rotation=({}, {}), offscreen={offscreen}, split={split}, owner={owner}, active={active}, playing={playing}, saving={saving}", rotation.y_rot, rotation.x_rot);
    }
}
'''
        fixture = fixture.replace("MOUSE_PATH", (ROOT / "crates/horizon-client-input/src/mouse.rs").as_posix())
        fixture = fixture.replace("METHOD", method(source, "device_event"))
        self.run_fixture(fixture, "native-mouse")

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_split_layout_keeps_capture_in_the_same_window(self):
        source = (ROOT / "crates/horizon-client/src/game/split/windows.rs").read_text(encoding="utf-8")
        body = function(source.replace("pub(super) fn", "pub fn"), "into_window").replace("Game<'_>", "Game")
        fixture = r'''
use std::sync::Arc;
struct Window(usize);
impl Window { fn id(&self) -> usize { self.0 } }
struct Game { captured: bool, window: Option<Arc<Window>> }
impl Game { fn capture(&mut self, captured: bool) { self.captured = captured; } }
FUNCTION
fn main() {
    let first = Arc::new(Window(1));
    let mut game = Game { captured: true, window: Some(first.clone()) };
    into_window(&mut game, first.clone());
    assert!(game.captured, "a player joining or a layout resize must not release P1's existing native window");
    into_window(&mut game, Arc::new(Window(2)));
    assert!(!game.captured, "moving to a different native window releases the previous grab");
    assert_eq!(game.window.as_ref().unwrap().id(), 2);
}
'''.replace("FUNCTION", body)
        self.run_fixture(fixture, "split-window")

    def run_fixture(self, fixture, name):
        with tempfile.TemporaryDirectory(prefix="horizon-input-") as folder:
            folder = Path(folder)
            rust = folder / f"{name}.rs"
            binary = folder / (name + (".exe" if os.name == "nt" else ""))
            rust.write_text(fixture, encoding="utf-8")
            built = subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                                   capture_output=True, text=True, timeout=60)
            self.assertEqual(built.returncode, 0, built.stderr)
            ran = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
            self.assertEqual(ran.returncode, 0, ran.stderr)

if __name__ == "__main__":
    unittest.main()
