"""Run the actual raw mouse dispatch with a fake mod and window events."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class MouseButtonEventsTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_order_cancellation_release_and_synthetic_input(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        outer = method(source.replace("pub(super) fn", "fn"), "handle_window_event")
        dispatch = outer[outer.index("        if let WindowEvent::MouseInput { state, button, .. } = &event {"):]
        fixture = r"""
use std::collections::HashSet;
#[derive(Clone, Copy, PartialEq)] enum ElementState { Pressed, Released }
enum WindowEvent { MouseInput { state: ElementState, button: i32 }, Other }
fn glfw_button(button: i32) -> i32 { button }
fn glfw_modifiers(modifiers: u32) -> u32 { modifiers }
struct Game { physical_buttons: HashSet<i32>, modifiers: u32, synthetic: bool, cancel: bool,
              early: bool, calls: Vec<&'static str> }
impl Game {
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    fn native_mouse_button_pre(&mut self, button: i32, pressed: bool, modifiers: u32, canceled: &mut bool) {
        assert_eq!((button, modifiers), (2, 5));
        assert_eq!(self.physical_buttons.contains(&button), pressed);
        self.calls.push("pre"); *canceled = self.cancel;
    }
    fn native_mouse_button_post(&mut self, button: i32, pressed: bool, modifiers: u32) {
        assert_eq!((button, modifiers), (2, 5));
        assert_eq!(self.physical_buttons.contains(&button), pressed);
        self.calls.push("post");
    }
    fn handle_window_event_inner(&mut self, _: (), _: WindowEvent) {
        self.calls.push("handle");
        if self.early { return; }
        self.calls.push("finish");
    }
    fn dispatch(&mut self, event: WindowEvent) {
        let event_loop = ();
DISPATCH
}
#[test]
fn dispatch_order_and_bookkeeping() {
    for state in [ElementState::Pressed, ElementState::Released] {
        for (cancel, synthetic, early, expected) in [
            (false, false, false, vec!["pre", "handle", "finish", "post"]),
            (true, false, false, vec!["pre"]),
            (false, false, true, vec!["pre", "handle", "post"]),
            (true, true, false, vec!["handle", "finish"])] {
            let mut game = Game { physical_buttons: HashSet::from([2]), modifiers: 5,
                synthetic, cancel, early, calls: vec![] };
            game.dispatch(WindowEvent::MouseInput { state, button: 2 });
            assert_eq!(game.calls, expected);
            assert_eq!(game.physical_buttons.contains(&2), state == ElementState::Pressed);
        }
    }
    let mut game = Game { physical_buttons: HashSet::new(), modifiers: 5, synthetic: false,
        cancel: true, early: false, calls: vec![] };
    game.dispatch(WindowEvent::Other);
    assert_eq!(game.calls, ["handle", "finish"]);
}
""".replace("DISPATCH", dispatch)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "mouse.rs", Path(folder) / "mouse.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
