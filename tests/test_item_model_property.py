"""Exercise the real ItemEnv bridge without a game window or prepared assets."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from tests.test_world_input_capture import method

ROOT = Path(__file__).resolve().parents[1]


class ItemModelPropertyTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "requires Rust")
    def test_no_mod_fast_path_and_borrowed_stack_dispatch(self):
        source = (ROOT / "crates/horizon-client/src/game/item_draw.rs").read_text(encoding="utf-8")
        actual = method(source, "item_model_property")
        fixture = r'''
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
static LOADED: AtomicBool = AtomicBool::new(false);
mod horizon_client {
    pub mod mod_api {
        pub fn mods() -> &'static [()] {
            if crate::LOADED.load(crate::Ordering::Relaxed) { &[()] } else { &[] }
        }
    }
}
struct StackView { count: i32 }
struct Game { calls: Cell<usize>, expected: *const StackView }
impl Game {
    fn native_item_model_property(&self, property: &str, stack: &StackView, value: &mut bool) {
        assert_eq!(stack as *const _, self.expected);
        self.calls.set(self.calls.get() + 1);
        if property == "example:charged" { *value = stack.count > 1; }
    }
}
struct Env<'a> { game: &'a Game }
impl Env<'_> { METHOD }
#[test]
fn bridge() {
    let stack = StackView { count: 2 };
    let game = Game { calls: Cell::new(0), expected: &stack };
    let env = Env { game: &game };
    assert!(!env.item_model_property("example:charged", &stack));
    assert_eq!(game.calls.get(), 0);
    LOADED.store(true, Ordering::Relaxed);
    assert!(env.item_model_property("example:charged", &stack));
    assert!(!env.item_model_property("other:property", &stack));
    assert_eq!(game.calls.get(), 2);
}
'''.replace("METHOD", actual)
        with tempfile.TemporaryDirectory() as folder:
            src, exe = Path(folder) / "property.rs", Path(folder) / "property.exe"
            src.write_text(fixture, encoding="utf-8")
            subprocess.run(["rustc", "--edition=2024", "--test", str(src), "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)
