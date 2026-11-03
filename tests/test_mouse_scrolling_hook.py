"""Run the actual gameplay wheel branch and real scroll accumulator with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class MouseScrollingHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_axes_accumulation_context_gates_and_cancellation(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        start = source.index("                    let (raw_x, raw_y) = match delta")
        normalize = source[start:source.index("                    // ScreenEvent.MouseScrolled.Pre", start)]
        start = source.index("                    if self.screen == Screen::Playing", start)
        dispatch = source[start:source.index("                    if self.screen == Screen::Chat", start)]
        fixture = r"""
use std::collections::HashSet;
#[allow(dead_code)]
#[path = "MOUSE_PATH"] mod mouse;
mod horizon_client { pub mod mouse { pub use crate::mouse::*; } }
#[derive(Clone, Copy, PartialEq, Eq, Hash)] enum MouseButton { Left, Middle, Right }
#[derive(Clone, Copy, PartialEq)] enum Screen { Playing, Other }
struct Point { x: f64, y: f64 }
enum MouseScrollDelta { LineDelta(f32, f32), PixelDelta(Point) }
struct Settings { discrete_scroll: bool, mouse_wheel_sensitivity: f32 }
struct Player { flying_speed: f32 }
struct SpectatorGui { active: bool }
impl SpectatorGui { fn is_menu_active(&self) -> bool { self.active } }
fn glfw_modifiers(bits: u32) -> u32 { bits }
struct Game { screen: Screen, overlay: bool, sleeping: Option<()>, player: Option<Player>,
    legacy_claimed: bool, synthetic: bool, cancel: bool, mouse: mouse::MouseLook, selected: usize,
    settings: Settings, physical_buttons: HashSet<MouseButton>, cursor: (f32, f32), modifiers: u32,
    spectator: bool, spectator_gui: SpectatorGui, menu_scrolls: Vec<i32>, legacy_calls: usize,
    calls: Vec<([f64; 2], [i32; 2], [bool; 3], [f32; 2], u32)> }
impl Game {
    fn new() -> Self { Self { screen: Screen::Playing, overlay: false, sleeping: None,
        player: Some(Player { flying_speed: 0.1 }), legacy_claimed: false, synthetic: false,
        cancel: false, mouse: mouse::MouseLook::default(), selected: 4,
        settings: Settings { discrete_scroll: false, mouse_wheel_sensitivity: 1.0 },
        physical_buttons: HashSet::from([MouseButton::Left, MouseButton::Right]),
        cursor: (10.0, 20.0), modifiers: 5, spectator: false, spectator_gui: SpectatorGui { active: false },
        menu_scrolls: vec![], legacy_calls: 0, calls: vec![] } }
    fn screen_over_playing(&self) -> bool { self.overlay }
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    fn spectator(&self) -> bool { self.spectator }
    fn spectator_scroll(&mut self, steps: i32) { self.menu_scrolls.push(steps); }
    fn native_wheel_event(&mut self, _: f32) -> bool { self.legacy_calls += 1; self.legacy_claimed }
    fn native_mouse_scrolling(&mut self, scroll: [f64;2], accumulated: [i32;2], buttons: [bool;3],
        cursor: [f32;2], modifiers: u32, canceled: &mut bool) {
        self.calls.push((scroll, accumulated, buttons, cursor, modifiers)); *canceled = self.cancel;
    }
    fn wheel(&mut self, delta: MouseScrollDelta) -> Result<(), ()> {
        NORMALIZE
        DISPATCH
        Ok(())
    }
}
#[test]
fn horizontal_fractional_steps_and_context() {
    let mut game = Game::new();
    game.wheel(MouseScrollDelta::LineDelta(0.25, 0.0)).unwrap();
    assert_eq!(game.selected, 4); assert!(game.calls.is_empty());
    game.wheel(MouseScrollDelta::LineDelta(0.75, 0.0)).unwrap();
    assert_eq!(game.selected, 5); assert_eq!(game.legacy_calls, 2);
    assert_eq!(game.calls, [([0.75,0.0], [1,0], [true,false,true], [10.0,20.0],5)]);
    let mut game = Game::new(); game.settings.mouse_wheel_sensitivity = 2.0;
    game.wheel(MouseScrollDelta::PixelDelta(Point { x: 60.0, y: -30.0 })).unwrap();
    assert_eq!(game.calls[0].0, [4.0,-2.0]); assert_eq!(game.calls[0].1, [4,-2]);
    let mut game = Game::new(); game.settings.mouse_wheel_sensitivity = 2.0; game.settings.discrete_scroll = true;
    game.wheel(MouseScrollDelta::PixelDelta(Point { x: 60.0, y: -30.0 })).unwrap();
    assert_eq!(game.calls[0].0, [2.0,-2.0]); assert_eq!(game.calls[0].1, [2,-2]);
}
#[test]
fn cancellation_consumes_steps_and_preserves_gameplay() {
    for spectator in [false, true] {
        for menu in [false, true] {
            let mut game = Game::new(); game.cancel = true; game.spectator = spectator; game.spectator_gui.active = menu;
            game.wheel(MouseScrollDelta::LineDelta(0.0,1.0)).unwrap();
            assert_eq!(game.selected,4); assert!(game.menu_scrolls.is_empty());
            assert_eq!(game.player.as_ref().unwrap().flying_speed,0.1);
            game.cancel = false;
            game.wheel(MouseScrollDelta::LineDelta(0.0,0.25)).unwrap();
            assert_eq!(game.calls.len(),1);
            game.wheel(MouseScrollDelta::LineDelta(0.0,0.75)).unwrap();
            assert_eq!(game.calls.len(),2);
            if !spectator { assert_eq!(game.selected,3); }
            else if menu { assert_eq!(game.menu_scrolls,[-1]); }
            else { assert!(game.player.as_ref().unwrap().flying_speed > 0.1); }
        }
    }
}
#[test]
fn source_and_legacy_gates() {
    for gate in 0..6 {
        let mut game = Game::new();
        match gate { 0 => game.screen = Screen::Other, 1 => game.overlay = true,
            2 => game.sleeping = Some(()), 3 => game.player = None,
            4 => game.legacy_claimed = true, _ => game.synthetic = true }
        game.wheel(MouseScrollDelta::LineDelta(0.0,1.0)).unwrap();
        assert!(game.calls.is_empty());
        assert_eq!(game.selected, if gate==5 {3} else {4});
    }
}
""".replace("MOUSE_PATH", (ROOT / "crates/horizon-client-input/src/mouse.rs").as_posix()).replace("NORMALIZE", normalize).replace("DISPATCH", dispatch)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "scroll.rs", Path(folder) / "scroll.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
