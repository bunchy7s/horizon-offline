"""Exercise screen release cancellation and its actual mapping dispatch."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ScreenKeyReleasedHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_cancelled_release_still_releases_every_binding(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        hooks = (ROOT / "crates/horizon-client/src/game/native_hooks.rs").read_text(encoding="utf-8")
        gate = method(hooks.replace("pub(super) fn", "fn"), "screen_key_released")
        mapping = method(source, "mapped_key_event")
        button = method(source.replace("fn keyboard_button(", "    fn keyboard_button("), "keyboard_button")
        start = source.index("            if let WindowEvent::KeyboardInput", source.index("let result: Result<()> = (|| {"))
        opening = source.index("{", source.index("&& self.screen_key_released(key)", start))
        end, depth = opening + 1, 1
        while depth:
            depth += (source[end] == "{") - (source[end] == "}")
            end += 1
        dispatch = source[start:end]
        fixture = r"""
use std::{cell::Cell, collections::HashSet};
type KeyCode = i32;
type Result<T> = std::result::Result<T, ()>;
#[derive(Clone, Copy, PartialEq)] enum ElementState { Pressed, Released }
#[derive(Clone, Copy)] enum PhysicalKey { Code(i32), Unidentified }
enum WindowEvent { KeyboardInput { event: KeyEvent }, Other }
struct KeyEvent { state: ElementState, physical_key: PhysicalKey }
#[derive(Clone, Copy, PartialEq)] enum KeyAction { Press, Release, Repeat }
fn glfw_modifiers(bits: u32) -> u32 { bits }
fn gameplay_has_keyboard(screen: bool, title: bool, dialog: bool) -> bool { screen && !title && !dialog }
BUTTON
fn container_key_clones(_: i32, _: KeyAction) -> bool { false }
mod keybinds { pub fn key_name(key: i32) -> Option<&'static str> { (key == 1).then_some("key.keyboard.a") } }
struct Bindings;
impl Bindings { fn translate_all(&self, _: i32) -> Vec<std::result::Result<i32, i32>> { vec![Ok(10), Ok(11), Err(2)] } }
struct Settings { keys: Bindings }
struct Game { keys: HashSet<i32>, settings: Settings, screen: bool, title: Option<()>, modifiers: u32,
    open: bool, cancel: bool, calls: Cell<usize>, handled: usize, released: Vec<(bool, i32)> }
impl Game {
    fn dialog_open(&self) -> bool { false }
    fn screen_view_now(&self) -> Option<&'static str> { self.open.then_some("options") }
    fn native_screen_key_released_pre(&self, screen: &&str, name: &str, modifiers: u32, canceled: &mut bool) {
        assert_eq!((*screen, name, modifiers), ("options", "key.keyboard.a", 5));
        self.calls.set(self.calls.get() + 1); *canceled = self.cancel;
    }
    fn mapped_key(&mut self, key: i32, pressed: bool, repeat: bool, raw: i32) -> Result<()> {
        assert!(!pressed && !repeat && raw == 1); self.released.push((false, key)); Ok(())
    }
    fn play_mouse(&mut self, button: i32, pressed: bool) -> Result<()> {
        assert!(!pressed); self.released.push((true, button)); Ok(())
    }
    fn inventory_pick_key(&mut self) { panic!("release cannot clone") }
    GATE
    MAPPING
    fn dispatch(&mut self, event: WindowEvent) -> Result<()> {
        DISPATCH
        self.handled += 1; Ok(())
    }
}
#[test]
fn release_and_screen_gates() {
    for (open, cancel, state, key, expected_calls, handled) in [
        (true, true, ElementState::Released, PhysicalKey::Code(1), 1, 0),
        (true, false, ElementState::Released, PhysicalKey::Code(1), 1, 1),
        (false, true, ElementState::Released, PhysicalKey::Code(1), 0, 1),
        (true, true, ElementState::Pressed, PhysicalKey::Code(1), 0, 1),
        (true, true, ElementState::Released, PhysicalKey::Code(2), 0, 1),
        (true, true, ElementState::Released, PhysicalKey::Unidentified, 0, 1)] {
        let mut game = Game { keys: HashSet::from([1]), settings: Settings { keys: Bindings }, screen: false,
            title: None, modifiers: 5, open, cancel, calls: Cell::new(0), handled: 0, released: vec![] };
        game.dispatch(WindowEvent::KeyboardInput { event: KeyEvent { state, physical_key: key } }).unwrap();
        assert_eq!((game.calls.get(), game.handled), (expected_calls, handled));
        if handled == 0 { assert!(!game.keys.contains(&1)); assert_eq!(game.released, [(false,10), (false,11)]); }
        game.dispatch(WindowEvent::Other).unwrap();
        game.released.clear();
        game.screen = true;
        game.mapped_key_event(1, false, false).unwrap();
        assert_eq!(game.released, [(false,10), (false,11), (true,2)]);
        game.released.clear();
        game.title = Some(());
        game.mapped_key_event(1, false, false).unwrap();
        assert_eq!(game.released, [(false,10), (false,11)]);
    }
}
""".replace("GATE", gate.replace("super::glfw_modifiers", "glfw_modifiers")).replace("MAPPING", mapping).replace("DISPATCH", dispatch).replace("BUTTON", button)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "release.rs", Path(folder) / "release.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
