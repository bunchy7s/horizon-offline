"""Run the actual outline filter with a fake mod, without a renderer."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class BlockHighlightHookTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_cancel_filters_only_the_target_outline(self):
        source = (ROOT / "crates/horizon-client/src/game.rs").read_text()
        call = source.index("self.native_block_highlight(")
        start = source.rindex(".filter(|h| {", 0, call)
        end = source.index(".map(|h| {", call)
        self.assertLess(source.index("let outline = if !self.ui.hide_gui"), start)
        self.assertLess(source.index("self.can_place_on_in_adventure(h.pos)"), start)
        fixture = r"""
use std::cell::Cell;
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit { pos: [i32; 3] }
#[derive(Clone, Copy, Debug, PartialEq)]
struct BlockStateId(u32);
struct World;
impl World { fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId { assert_eq!([x,y,z], [1,2,3]); BlockStateId(42) } }
struct Game { world: World, cancel: bool, calls: Cell<usize> }
impl Game {
    fn native_block_highlight(&self, pos: [i32; 3], state: BlockStateId, partial: f32, canceled: &mut bool) {
        assert_eq!(pos, [1,2,3]); assert_eq!(state, BlockStateId(42)); assert_eq!(partial, 0.5);
        self.calls.set(self.calls.get()+1); *canceled = self.cancel;
    }
    fn filter(&self, hit: Option<Hit>) -> Option<Hit> { let alpha = 0.5; hit FILTER }
}
#[test]
fn cancelled_default_and_missing_target() {
    for cancel in [false, true] {
        let game = Game { world: World, cancel, calls: Cell::new(0) };
        let hit = Hit { pos: [1,2,3] };
        assert_eq!(game.filter(Some(hit)), if cancel { None } else { Some(hit) });
        assert_eq!(game.filter(None), None);
        assert_eq!(game.calls.get(), 1);
    }
}
""".replace("FILTER", source[start:end])
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "outline.rs", Path(folder) / "outline.exe"
            src.write_text(fixture)
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
