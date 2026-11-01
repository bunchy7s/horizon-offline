"""Check cancelled native screen presses without opening an OS window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ScreenMouseFocusTests(unittest.TestCase):
    def test_cancel_precedes_widget_focus(self):
        game = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        pre = game.index("// ScreenEvent.MouseButtonPressed.Pre")
        focus = game.index("self.key_focus_mouse_press(&event)")
        self.assertLess(game.index("self.key_focus_mouse(event_loop, &event)"), pre)
        self.assertLess(game.index("if canceled {", pre), focus)
        self.assertIn("return Ok(());", game[game.index("if canceled {", pre):focus])
        self.assertLess(focus, game.index("match event {", pre))

    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_accepted_press_focus(self):
        source = (ROOT / "crates/horizon-client/src/game/pad_menu.rs").read_text()
        source = source.replace("pub(super) fn", "fn")
        fixture = r"""
#[derive(Clone, Copy, PartialEq)]
enum ElementState { Pressed, Released }
#[derive(Clone, Copy, PartialEq)]
enum MouseButton { Left, Right }
enum WindowEvent { MouseInput { state: ElementState, button: MouseButton }, MouseWheel }
#[derive(Clone, Copy)]
struct Target { rect: [f32; 4] }
struct Menu { synthetic: bool, origin: Option<[f32; 4]> }
struct Pad { menu: Menu }
struct Game { pad: Pad, cursor: (f32, f32), in_menu: bool }
impl Game {
    fn pad_in_menu(&self) -> bool { self.in_menu }
    fn key_targets(&self) -> Vec<Target> { vec![Target { rect: [0., 0., 10., 10.] }] }
    METHOD
}
#[test]
fn accepted_and_ignored_presses() {
    let old = Some([20., 20., 10., 10.]);
    let mut game = Game { pad: Pad { menu: Menu { synthetic: false, origin: old } },
                          cursor: (5., 5.), in_menu: true };
    for event in [WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left },
                  WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right },
                  WindowEvent::MouseWheel] {
        game.key_focus_mouse_press(&event);
        assert_eq!(game.pad.menu.origin, old);
    }
    let press = WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left };
    game.pad.menu.synthetic = true;
    game.key_focus_mouse_press(&press);
    assert_eq!(game.pad.menu.origin, old);
    game.pad.menu.synthetic = false;
    game.in_menu = false;
    game.key_focus_mouse_press(&press);
    assert_eq!(game.pad.menu.origin, old);
    game.in_menu = true;
    game.key_focus_mouse_press(&press);
    assert_eq!(game.pad.menu.origin, Some([0., 0., 10., 10.]));
    game.cursor = (10., 5.);
    game.key_focus_mouse_press(&press);
    assert_eq!(game.pad.menu.origin, None);
}
""".replace("METHOD", method(source, "key_focus_mouse_press"))
        with tempfile.TemporaryDirectory() as folder:
            src = Path(folder) / "focus.rs"
            exe = Path(folder) / "focus.exe"
            src.write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
