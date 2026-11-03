"""Run actual drag tracking, dispatch and cursor update with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ScreenMouseDraggedHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_drag_lifecycle_delta_and_cancelled_cursor(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text(encoding="utf-8")
        hooks = (ROOT / "crates/horizon-client/src/game/native_hooks.rs").read_text(encoding="utf-8")
        hooks = hooks.replace("pub(super) fn", "fn").replace("super::glfw_", "glfw_")
        track = method(hooks, "track_drag_button")
        dispatch = method(hooks, "screen_mouse_dragged")
        start = source.index("                        let previous = self.cursor;", source.index("WindowEvent::CursorMoved { position"))
        cursor = source[start:source.index("                        if self.dialog_open()", start)]
        start = source.index("        if let WindowEvent::Focused(false)")
        focus = source[start:source.index("        if let WindowEvent::ModifiersChanged", start)]
        fixture = r"""
use std::{cell::RefCell, collections::HashSet};
type MouseButton = i32;
fn glfw_button(button: i32) -> i32 { button }
fn glfw_modifiers(bits: u32) -> u32 { bits }
struct View { mouse: (f32, f32) }
struct Point { x: f64, y: f64 }
struct Batch { scale: f32 }
enum WindowEvent { Focused(bool) }
struct Game { drag_button: Option<i32>, physical_buttons: HashSet<i32>, physical_keys: HashSet<i32>,
    cursor: (f32, f32), modifiers: u32, synthetic: bool, open: bool, cancel: bool, handled: usize,
    calls: RefCell<Vec<(i32, [f32; 2], (f32, f32))>> }
impl Game {
    fn pad_synthetic_input(&self) -> bool { self.synthetic }
    fn screen_view_now(&self) -> Option<View> { self.open.then_some(View { mouse: self.cursor }) }
    fn native_screen_mouse_dragged_pre(&self, view: &View, button: i32, delta: [f32; 2], modifiers: u32, canceled: &mut bool) {
        assert_eq!(modifiers, 5);
        self.calls.borrow_mut().push((button, delta, view.mouse)); *canceled = self.cancel;
    }
    TRACK
    DISPATCH
    fn pointer(&mut self, position: Point) -> Result<(), ()> {
        let b = Batch { scale: 2.0 };
        CURSOR
        self.handled += 1; Ok(())
    }
    fn focus(&mut self, event: WindowEvent) { FOCUS }
}
#[test]
fn lifecycle_and_cancellation() {
    for cancel in [false, true] {
        let mut game = Game { drag_button: None, physical_buttons: HashSet::from([0, 1]),
            physical_keys: HashSet::from([10]), cursor: (10.0, 10.0), modifiers: 5, synthetic: false,
            open: true, cancel, handled: 0, calls: RefCell::new(vec![]) };
        game.track_drag_button(0, true);
        game.track_drag_button(1, true);
        game.track_drag_button(0, false);
        // Any release ends the drag (vanilla clears activeButton): right is still held, no drag.
        assert_eq!(game.drag_button, None);
        game.track_drag_button(1, true);
        game.pointer(Point { x: 50.0, y: 30.0 }).unwrap();
        game.pointer(Point { x: 54.0, y: 34.0 }).unwrap();
        assert_eq!(game.cursor, (27.0, 17.0));
        assert_eq!(game.handled, if cancel { 0 } else { 2 });
        assert_eq!(*game.calls.borrow(), [(1, [15.0, 5.0], (25.0, 15.0)), (1, [2.0, 2.0], (27.0, 17.0))]);
        game.calls.borrow_mut().clear();
        game.pointer(Point { x: 54.0, y: 34.0 }).unwrap();
        game.synthetic = true;
        game.pointer(Point { x: 56.0, y: 36.0 }).unwrap();
        game.synthetic = false; game.open = false;
        game.pointer(Point { x: 58.0, y: 38.0 }).unwrap();
        game.open = true; game.physical_buttons.clear();
        game.pointer(Point { x: 60.0, y: 40.0 }).unwrap();
        assert!(game.calls.borrow().is_empty());
        game.track_drag_button(1, false);
        assert_eq!(game.drag_button, None);
        game.track_drag_button(0, true);
        game.physical_buttons.insert(0);
        game.focus(WindowEvent::Focused(false));
        assert_eq!(game.drag_button, None);
        assert!(game.physical_buttons.is_empty() && game.physical_keys.is_empty());
        game.pointer(Point { x: 62.0, y: 42.0 }).unwrap();
        assert!(game.calls.borrow().is_empty());
    }
}
""".replace("TRACK", track).replace("DISPATCH", dispatch).replace("CURSOR", cursor).replace("FOCUS", focus)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "drag.rs", Path(folder) / "drag.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
