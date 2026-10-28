"""tools/parity_reach.py: which workspace sources a harness's probe can reach."""
from pathlib import Path
import tempfile
import textwrap
import unittest

from tools import parity, parity_reach

PROBE = [["example", "world_probe", "release", ["-p", "a"], True]]


def workspace(root, files):
    for path, text in files.items():
        (root / path).parent.mkdir(parents=True, exist_ok=True)
        (root / path).write_text(textwrap.dedent(text), encoding="utf-8")


class ReachTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        workspace(self.root, {
            "crates/a/Cargo.toml": '[package]\nname = "a"\n\n[[example]]\nname = "world_probe"\npath = "examples/probe/main.rs"\n',
            "crates/a/src/lib.rs": """
                pub mod used;
                pub mod unused;
                mod methods;
                mod display;
                mod table;
                #[cfg(test)]
                mod tests;
            """,
            "crates/a/src/used.rs": """
                pub struct Thing;
                pub fn helper() -> u32 { 1 }
            """,
            # Same names as used.rs, but nothing names this module.
            "crates/a/src/unused.rs": "pub struct Thing;\npub fn helper() -> u32 { 2 }\n",
            "crates/a/src/methods.rs": """
                use crate::used::Thing;
                impl Thing {
                    pub fn method(&self) -> u32 { 3 }
                    pub fn never_called(&self) -> u32 { 4 }
                }
            """,
            "crates/a/src/display.rs": """
                impl std::fmt::Display for crate::used::Thing {
                    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "thing") }
                }
            """,
            "crates/a/src/table.rs": """
                macro_rules! table { () => { pub struct Table; }; }
                table!();
            """,
            "crates/a/src/tests.rs": "use crate::used::Thing;\nimpl Thing { pub fn method_in_tests(&self) {} }\n",
            "crates/a/examples/probe/main.rs": "mod dispatch;\nmod p;\n",
            "crates/a/examples/probe/dispatch.rs": "pub fn args() {}\n",
            "crates/a/examples/probe/p.rs": """
                use a::used::{helper, Thing};
                pub fn main() { println!("{} {} {}", helper(), Thing.method(), Thing); }
            """,
        })
        self.dependencies = sorted(path.relative_to(self.root).as_posix() for path in self.root.rglob("*") if path.is_file())
        self.dependencies.remove("crates/a/examples/probe/main.rs")  # parity.only_probes_run leaves it out

    def tearDown(self):
        self.directory.cleanup()

    def narrowed(self, changed=(), artifacts=PROBE):
        graph = parity_reach.SourceGraph(self.root, parity.rust_code)
        return parity_reach.narrow(self.dependencies, artifacts, graph, set(changed))

    def test_keeps_what_the_probe_reaches(self):
        kept = {path for path, (whole, _) in self.narrowed().items() if whole}
        self.assertIn("crates/a/src/used.rs", kept)
        self.assertIn("crates/a/src/methods.rs", kept)  # Thing.method()
        self.assertIn("crates/a/src/display.rs", kept)  # std's Display for a reached type
        self.assertIn("crates/a/Cargo.toml", kept)
        self.assertNotIn("crates/a/src/unused.rs", kept)  # its names match, but nothing names its module
        self.assertNotIn("crates/a/src/table.rs", kept)
        self.assertNotIn("crates/a/src/tests.rs", kept)  # compiled into tests only

    def test_module_parents_count_through_their_declarations(self):
        self.assertEqual(self.narrowed()["crates/a/src/lib.rs"], (False, {"used", "methods", "display"}))
        text = (self.root / "crates/a/src/lib.rs").read_text()
        self.assertEqual(parity_reach.declarations(text, {"used"}, False), "pub mod used;")
        self.assertEqual(parity_reach.declarations(text, {"tests"}, False), "#[cfg(test)]\nmod tests;")
        # Adding or dropping a module beside the reached ones changes neither view.
        other = text.replace("mod table;", "mod table2;\nmod more;")
        self.assertEqual(parity_reach.declarations(other, {"used"}, False), "pub mod used;")
        self.assertEqual(parity_reach.declarations(other, {"used"}, True), parity_reach.declarations(text, {"used"}, True))
        self.assertNotEqual(parity_reach.declarations(text + "pub fn f() {}\n", {"used"}, True),
                            parity_reach.declarations(text, {"used"}, True))

    def test_a_new_file_counts_once_it_is_reached(self):
        (self.root / "crates/a/src/lib.rs").write_text("pub mod used;\nmod extra;\n")
        workspace(self.root, {"crates/a/src/extra.rs": "impl crate::used::Thing { pub fn method(&self) -> u32 { 5 } }\n"})
        self.assertIn("crates/a/src/extra.rs", self.narrowed(changed={"crates/a/src/extra.rs"}))

    def test_a_deleted_dependency_counts(self):
        (self.root / "crates/a/src/unused.rs").unlink()
        self.assertIn("crates/a/src/unused.rs", self.narrowed())

    def test_a_search_told_to_stop_still_keeps_the_file_it_stopped_at(self):
        graph = parity_reach.SourceGraph(self.root, parity.rust_code)
        kept = parity_reach.narrow(self.dependencies, PROBE, graph, {"crates/a/src/used.rs"}, {"crates/a/src/used.rs"})
        self.assertEqual(kept["crates/a/src/used.rs"], (True, set()))
        self.assertFalse(graph.results, "a partial search is not remembered")
        # A stop file outside the harness's sources doesn't end its search.
        full = parity_reach.narrow(self.dependencies, PROBE, graph, (), {"crates/b/src/lib.rs"})
        self.assertIn("crates/a/src/display.rs", full)
        self.assertTrue(graph.results)

    def test_a_module_moved_to_a_new_crate_counts_through_its_reexport(self):
        # used.rs moves to crate b after the harness was recorded; a re-exports it under its old path. Crate c is
        # a dependency nothing reached names: its items stay out even where their names match.
        (self.root / "crates/a/src/used.rs").unlink()
        workspace(self.root, {
            "crates/a/Cargo.toml": (self.root / "crates/a/Cargo.toml").read_text()
            + '\n[dependencies]\nb = { path = "../b" }\nc = { path = "../c" }\n',
            "crates/a/src/lib.rs": "pub use b::used;\npub mod unused;\nmod methods;\nmod display;\n",
            "crates/b/Cargo.toml": '[package]\nname = "b"\n',
            "crates/b/src/lib.rs": "pub mod used;\n",
            "crates/b/src/used.rs": "pub struct Thing;\npub fn helper() -> u32 { 1 }\n",
            "crates/c/Cargo.toml": '[package]\nname = "c"\n',
            "crates/c/src/lib.rs": "pub fn helper() -> u32 { 3 }\n",
        })
        changed = {"crates/b/src/used.rs", "crates/c/src/lib.rs"}
        self.assertEqual(parity_reach.unrecorded(self.root, self.dependencies, changed), changed)
        kept = self.narrowed(changed=changed)
        self.assertIn("crates/b/src/used.rs", kept)
        self.assertNotIn("crates/c/src/lib.rs", kept)

    def test_unknown_probes_keep_everything(self):
        self.assertEqual(set(self.narrowed(artifacts=[])), set(self.dependencies))
        self.assertEqual(set(self.narrowed(artifacts=[["example", "missing", "release", ["-p", "a"], True]])),
                         set(self.dependencies))


if __name__ == "__main__":
    unittest.main()
