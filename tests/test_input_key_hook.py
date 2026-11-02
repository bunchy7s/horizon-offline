"""Run the actual window-key callback with fake normal/early-return handlers."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class InputKeyHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_original_event_after_handling_and_source_gates(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        actual = method(source, "window_event")
        fixture = r"""
type WindowId = i32;
struct ActiveEventLoop;
struct Window;
impl Window { fn id(&self) -> i32 { 7 } }
mod input_trace {
    #[derive(Clone, Copy)]
    pub enum InputOrigin { User, Agent, Game }
}
#[derive(Clone, Debug, PartialEq)]
struct KeyEvent { physical_key: i32, logical_key: &'static str, location: i32,
                  text: &'static str, pressed: bool, repeat: bool }
enum WindowEvent { KeyboardInput { event: KeyEvent, is_synthetic: bool }, Other }
fn glfw_modifiers(bits: u32) -> u32 { bits }
struct Game { window: Option<Window>, modifiers: u32, early: bool,
              calls: Vec<&'static str>, observed: Vec<(KeyEvent, u32)> }
impl Game {
    fn handle_window_event(&mut self, _: &ActiveEventLoop, _: WindowId, mut event: WindowEvent,
                           _: input_trace::InputOrigin) {
        self.calls.push("handle"); self.modifiers = 0;
        if let WindowEvent::KeyboardInput { event, .. } = &mut event { event.physical_key = 99; event.text = ""; }
        if self.early { return; }
        self.calls.push("finish");
    }
    fn native_input_key(&mut self, event: &KeyEvent, modifiers: u32) {
        self.calls.push("hook"); self.observed.push((event.clone(), modifiers));
    }
    METHOD
}
#[test]
fn all_actions_consumption_and_window_gates() {
    for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
        for early in [false, true] {
            let original = KeyEvent { physical_key: -1, logical_key: "logical", location: 2,
                text: "z", pressed, repeat };
            for (window, id, synthetic, fires) in [(true,7,false,true), (true,8,false,false),
                (true,7,true,false), (false,7,false,false)] {
                let mut game = Game { window: window.then_some(Window), modifiers: 5, early,
                    calls: vec![], observed: vec![] };
                game.window_event(&ActiveEventLoop, id, WindowEvent::KeyboardInput { event: original.clone(), is_synthetic: synthetic });
                let mut expected = vec!["handle"];
                if !early { expected.push("finish"); }
                if fires { expected.push("hook"); }
                assert_eq!(game.calls, expected);
                assert_eq!(game.observed, if fires { vec![(original.clone(),5)] } else { vec![] });
            }
        }
    }
    let mut game = Game { window: Some(Window), modifiers: 5, early: false, calls: vec![], observed: vec![] };
    game.window_event(&ActiveEventLoop, 7, WindowEvent::Other);
    assert!(game.observed.is_empty());
}
""".replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "key.rs", Path(folder) / "key.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
