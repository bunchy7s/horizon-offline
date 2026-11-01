import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "stamp_native_hooks", Path(__file__).resolve().parents[1] / "tools/stamp_native_hooks.py"
)
stamp = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stamp)


class StampTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.original = stamp.ROOT
        stamp.ROOT = self.root
        self.addCleanup(setattr, stamp, "ROOT", self.original)
        subprocess.run(["git", "init", "-q", "-b", "master", str(self.root)], check=True)
        for path, content in {
            stamp.SOURCE: "//! API 7 adds old.\n//!\npub(crate) const NATIVE_HOOKS_API: u32 = 7;\n",
            stamp.DOCS: "API 7 adds old.\n\n## What the launcher does\n",
            stamp.CATALOG: '{"hooks": []}\n',
        }.items():
            (self.root / path).parent.mkdir(parents=True, exist_ok=True)
            (self.root / path).write_text(content)
        (self.root / stamp.FRAGMENTS).mkdir(parents=True)

    def git(self, *args):
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Test",
                        "-c", "user.email=test@example.invalid", *args],
                       check=True, capture_output=True)

    def fragment(self, name):
        path = self.root / stamp.FRAGMENTS / f"{name}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({
            "hooks": [{"name": name, "target": f"target::{name}"}],
            "rust_summary": f"{name}.",
            "doc_summary": f'{name}; requires @API:{name}@.',
        }))
        return path

    def test_merge_order_and_idempotence(self):
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.git("checkout", "-qb", "first")
        self.fragment("z")
        self.git("add", ".")
        self.git("commit", "-qm", "first")
        self.git("checkout", "-q", "master")
        self.git("checkout", "-qb", "second")
        self.fragment("a")
        self.git("add", ".")
        self.git("commit", "-qm", "second")
        self.git("checkout", "-q", "master")
        self.git("merge", "--no-ff", "-qm", "merge first", "first")
        self.git("merge", "--no-ff", "-qm", "merge second", "second")
        self.assertTrue(stamp.stamp(True))
        self.assertFalse(stamp.stamp(False))
        source = (self.root / stamp.SOURCE).read_text()
        docs = (self.root / stamp.DOCS).read_text()
        self.assertIn("API 8 adds z.", source)
        self.assertIn("API 9 adds a.", source)
        self.assertIn("API 8 adds z; requires 8.", docs)
        self.assertIn("API 9 adds a; requires 9.", docs)
        self.assertIn("NATIVE_HOOKS_API: u32 = 9;", source)
        hooks = json.loads((self.root / stamp.CATALOG).read_text())["hooks"]
        self.assertEqual([(h["name"], h["version"]) for h in hooks], [("z", 1), ("a", 1)])
        self.assertFalse(stamp.stamp(False))

    def test_helper_only_gets_a_number(self):
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        path = self.fragment("helper")
        fragment = json.loads(path.read_text())
        fragment["hooks"] = []
        path.write_text(json.dumps(fragment))
        self.assertFalse(stamp.stamp(False))
        self.assertIn("NATIVE_HOOKS_API: u32 = 8;", (self.root / stamp.SOURCE).read_text())
        self.assertEqual(json.loads((self.root / stamp.CATALOG).read_text())["hooks"], [])

    def test_catalog_facts_travel_with_the_hook(self):
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        path = self.fragment("served")
        fragment = json.loads(path.read_text())
        fragment["hooks"][0].update({"side": "server", "cancellable": True, "other": 1})
        path.write_text(json.dumps(fragment))
        self.assertFalse(stamp.stamp(False))
        hooks = json.loads((self.root / stamp.CATALOG).read_text())["hooks"]
        self.assertEqual(hooks, [{"name": "served", "version": 1, "target": "target::served",
                                  "side": "server", "cancellable": True}])

    def test_duplicate_fails_before_writes(self):
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.fragment("same")
        (self.root / stamp.CATALOG).write_text('{"hooks": [{"name": "same"}]}')
        before = (self.root / stamp.SOURCE).read_text()
        with self.assertRaisesRegex(ValueError, "duplicate"):
            stamp.stamp(False)
        self.assertEqual((self.root / stamp.SOURCE).read_text(), before)


if __name__ == "__main__":
    unittest.main()
