"""Exercise the client's capture methods without opening or focusing an OS window.

The fixture substitutes only winit and mouse state. Method bodies come from the
client, so the assertions observe grab attempts, visibility and stored capture
state. Native Windows delivery remains a manual check. Vanilla 26.3 reference:
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
        tick = remote[remote.index("    pub(super) fn tick_remote("):]
        ready = tick[tick.index("if self.remote.as_ref().is_some_and(|r| !r.loaded_sent)"):]
        self.assertLess(ready.index("r.loaded_sent = true"),
                        ready.index("self.capture_loaded_world()"))
        self.assertLess(ready.index("self.capture_loaded_world()"),
                        ready.index("let stick = self.pad_move()"))

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
struct ServerPacks { prompt: Option<()> }
struct Game {
    focused: bool, agent_pointer: bool, window: Option<Window>, mouse: Mouse,
    keyboard_mouse: bool, view_at: [u32; 2],
    captured: bool, title: Option<()>, screen: Screen, remote: Option<Remote>,
    mining: bool, lan_acting: Option<u32>, renderer: Option<Renderer>, cursor: (f32, f32),
    server_packs: ServerPacks,
}
impl Default for Game {
    fn default() -> Self {
        Self { focused: true, agent_pointer: false, window: Some(Window::default()),
            keyboard_mouse: true, view_at: [0, 0],
            mouse: Mouse::default(), captured: false, title: None,
            screen: Screen::Playing, remote: None, mining: false, lan_acting: None,
            renderer: None, cursor: (0.0, 0.0), server_packs: ServerPacks::default() }
    }
}
HELPER
impl Game {
    fn stop_mining(&mut self) { self.mining = false; }
    fn reset_inventory_clicks(&mut self) {}
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
        assert!(!game.captured); // Focus alone does not resume gameplay.
        game.capture(true); // Click or resume action.
        assert!(game.captured);
        assert_eq!(game.cursor, (213.5, 120.0)); // The grab puts the mouse at the centre.
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
    game.renderer = Some(Renderer { config: Config { width: 1000, height: 600 } });
    game.capture(true);
    assert!(game.captured);
    let window = game.window.as_ref().unwrap();
    assert_eq!((window.warps.get(), window.at.get()), (1, (500.0, 300.0)));
    game.capture(false);
    let window = game.window.as_ref().unwrap();
    assert_eq!(window.warps.get(), 3);
    assert_eq!(game.cursor, (250.0, 150.0));
    game.cursor = (1.0, 1.0);
    game.capture(false);
    assert_eq!(game.window.as_ref().unwrap().warps.get(), 3);
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
                                  method(source, "capture_loaded_world"))
        with tempfile.TemporaryDirectory(prefix="horizon-input-") as folder:
            folder = Path(folder)
            rust = folder / "capture.rs"
            binary = folder / ("capture.exe" if os.name == "nt" else "capture")
            rust.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", str(rust), "-o", str(binary)],
                           check=True, capture_output=True, text=True, timeout=60)
            subprocess.run([str(binary)], check=True, capture_output=True, text=True, timeout=10)


if __name__ == "__main__":
    unittest.main()
