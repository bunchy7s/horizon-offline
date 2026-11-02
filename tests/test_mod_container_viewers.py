"""Run the real viewer counting of mod containers through a fake mod."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ModContainerViewersTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_opens_second_viewer_close_and_leaving_without_closing(self):
        source = (ROOT / "crates/horizon-client/src/game/native_hooks.rs").read_text(encoding="utf-8")
        start = source.index("pub(super) fn viewer_changes(")
        end = source.index("\n}\n", start) + 3
        function = source[start:end].replace("pub(super) fn", "fn")
        fixture = r"""
use std::collections::HashMap;
type Known = HashMap<(&'static str, (i32, i32, i32)), u32>;
FUNCTION
fn now(counts: &[((i32, i32, i32), u32)]) -> HashMap<(i32, i32, i32), u32> {
    counts.iter().copied().collect()
}
#[test]
fn heard_once_per_change_in_each_level() {
    let (a, b) = ((1, 64, 1), (2, 64, 2));
    let mut known = Known::new();
    // The first viewer opens it: `onOpen`.
    assert_eq!(viewer_changes(&mut known, "overworld", now(&[(a, 1)])), [(a, 0, 1)]);
    // Nothing changed: silence.
    assert!(viewer_changes(&mut known, "overworld", now(&[(a, 1)])).is_empty());
    // A second viewer, and another container.
    assert_eq!(viewer_changes(&mut known, "overworld", now(&[(a, 2), (b, 1)])), [(a, 1, 2), (b, 0, 1)]);
    // Another level's recount does not touch this level's counts.
    assert!(viewer_changes(&mut known, "nether", now(&[])).is_empty());
    // One of a's viewers left without closing; b's only one closed: `onClose`.
    assert_eq!(viewer_changes(&mut known, "overworld", now(&[(a, 1)])), [(a, 2, 1), (b, 1, 0)]);
    // The last viewer disconnected: no player counts it any more.
    assert_eq!(viewer_changes(&mut known, "overworld", now(&[])), [(a, 1, 0)]);
    assert!(known.is_empty());
    assert!(viewer_changes(&mut known, "overworld", now(&[])).is_empty());
}
""".replace("FUNCTION", function)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "viewers.rs", Path(folder) / "viewers.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
