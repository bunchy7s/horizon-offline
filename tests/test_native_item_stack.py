"""Exercise the screen stack helper without a game window."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class NativeItemStackTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_stack_and_decorations_are_forwarded_and_scaled(self):
        source = (ROOT / "crates/horizon-client/src/game/native_hooks.rs").read_text(encoding="utf-8")
        actual = method(source.replace("pub(super) fn", "fn"), "native_item_stack")
        fixture = r"""
struct ItemStack { item: String, count: u32, damage: u32 }
struct Items;
impl Items { fn get(&self, id: &str) -> Option<()> { (id == "known").then_some(()) } }
struct Resources;
impl Resources { fn items(&self) -> Items { Items } }
struct HudBatch { vertices: Vec<u32>, scaled: Option<(usize, f32, f32, f32, f32)> }
impl HudBatch {
    fn scale_since(&mut self, start: usize, x: f32, y: f32, sx: f32, sy: f32) {
        self.scaled = Some((start, x, y, sx, sy));
    }
}
struct Game { res: Resources }
impl Game {
    fn draw_item_model(&self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        assert_eq!((stack.count, stack.damage, x, y), (42, 13, 2.0, 3.0));
        batch.vertices.push(1);
    }
    fn draw_item_decorations(&self, batch: &mut HudBatch, stack: &ItemStack,
                             x: f32, y: f32, count: Option<String>) {
        assert_eq!((stack.count, stack.damage, x, y, count.as_deref()), (42, 13, 2.0, 3.0, Some("")));
        batch.vertices.push(2);
    }
    METHOD
}
#[test]
fn stack_and_unknown_item() {
    let game = Game { res: Resources };
    let mut batch = HudBatch { vertices: vec![0], scaled: None };
    let mut stack = ItemStack { item: "unknown".into(), count: 42, damage: 13 };
    assert!(!game.native_item_stack(&mut batch, &stack, [2.0, 3.0], 32.0));
    assert_eq!(batch.vertices, vec![0]);
    assert_eq!(batch.scaled, None);
    stack.item = "known".into();
    assert!(game.native_item_stack(&mut batch, &stack, [2.0, 3.0], 32.0));
    assert_eq!(batch.vertices, vec![0, 1, 2]);
    assert_eq!(batch.scaled, Some((1, 2.0, 3.0, 2.0, 2.0)));
}
""".replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "item.rs", Path(folder) / "item.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
