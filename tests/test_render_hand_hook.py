"""Exercise the hand renderer's actual cancellation dispatch with a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class RenderHandHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_each_hand_can_be_cancelled(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        start = source.index("            let mut canceled = false;", source.index("fn hand_geometry("))
        end = source.index("            // The main hand on its arm", start)
        dispatch = source[start:end]
        self.assertLess(source.index("if !shown[i]", source.index("fn hand_geometry(")), start)
        self.assertLess(end, source.index("let Some(stack) = item", end))
        self.assertLess(end, source.index("if let Some(id) = maps::map_id", end))
        fixture = r"""
struct Game { cancel: Option<usize>, calls: Vec<(usize, Option<i32>, f32, f32, f32)> }
impl Game {
    fn native_render_hand(&mut self, hand: usize, item: Option<&i32>, a: f32,
                          attack: f32, equip: f32, canceled: &mut bool) {
        self.calls.push((hand, item.copied(), a, attack, equip));
        *canceled = self.cancel == Some(hand);
    }
    fn draw(&mut self) -> Vec<usize> {
        let mut rendered = Vec::new();
        for (hand, item) in [Some(42), None].into_iter().enumerate() {
            let (a, attack, inverse_height) = (0.5, 0.25, 0.75);
            DISPATCH
            rendered.push(hand);
        }
        rendered
    }
}
#[test]
fn cancellation_is_per_hand_and_carries_displayed_state() {
    for (cancel, expected) in [(None, vec![0, 1]), (Some(0), vec![1]), (Some(1), vec![0])] {
        let mut game = Game { cancel, calls: Vec::new() };
        assert_eq!(game.draw(), expected);
        assert_eq!(game.calls, vec![(0, Some(42), 0.5, 0.25, 0.75), (1, None, 0.5, 0.25, 0.75)]);
    }
}
""".replace("DISPATCH", dispatch)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "hand.rs", Path(folder) / "hand.exe"
            src.write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
