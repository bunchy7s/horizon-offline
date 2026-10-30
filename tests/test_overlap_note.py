import unittest
from pathlib import Path

from tools.ci import overlap_note as ov

ROOT = Path(__file__).resolve().parents[1]

DIFF = """\
diff --git a/crates/horizon-client/src/game/dragon.rs b/crates/horizon-client/src/game/dragon.rs
index 1..2 100644
--- a/crates/horizon-client/src/game/dragon.rs
+++ b/crates/horizon-client/src/game/dragon.rs
@@ -440 +440,2 @@ impl Dragon {
-        let a = 1;
+        let a = 2;
+        let b = 3;
@@ -2092,0 +2093,3 @@     pub fn tick_dragon(&mut self) {
+        if x {
+            y();
+        }
@@ -2200,0 +2205,4 @@     fn last_existing(&self) -> u32 {
+
+    /// A new helper.
+    pub(crate) fn brand_new(&self) {
+    }
@@ -2300 +2310 @@ fn helper() {
-    a
+    b
@@ -10 +10 @@ use std::fmt;
-use a;
+use b;
diff --git a/tools/lan_e2e.py b/tools/lan_e2e.py
--- a/tools/lan_e2e.py
+++ b/tools/lan_e2e.py
@@ -5 +5 @@ def check_chest(args):
-    x
+    y
diff --git a/crates/horizon-client/examples/probe/mob_ai_probe.rs b/crates/horizon-client/examples/probe/mob_ai_probe.rs
--- a/crates/horizon-client/examples/probe/mob_ai_probe.rs
+++ b/crates/horizon-client/examples/probe/mob_ai_probe.rs
@@ -214,0 +215,2 @@ fn goal_name(m: &Mob, name: &str) -> String {
+        "a" => 1,
+        "b" => 2,
diff --git a/old.rs b/old.rs
deleted file mode 100644
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@ fn gone() {
-x
"""


def pull(n, funcs=(), closes=(), files=(), changed=None, author="a"):
    p = ov.Pull(n, f"PR {n}", author, False, f"{n:040x}", list(files), changed or len(files), set(closes))
    p.funcs = set(funcs)
    return p


def fx(*names, path="a.rs"):
    return {(path, n) for n in names}


class FunctionTests(unittest.TestCase):
    def test_functions_from_a_diff(self):
        self.assertEqual(ov.functions(DIFF), {
            ("crates/horizon-client/src/game/dragon.rs", "tick_dragon"),
            ("crates/horizon-client/src/game/dragon.rs", "helper"),
            ("tools/lan_e2e.py", "check_chest"),
        })
        # `impl Dragon` and `use` headers name no function; a hunk that only adds a new function after
        # `last_existing` doesn't count; probes, tests and deleted files don't either.

    def test_empty(self):
        self.assertEqual(ov.functions(""), set())


class OverlapTests(unittest.TestCase):
    def test_same_issue(self):
        me, o = pull(1, closes={9}), pull(2, closes={9, 10})
        self.assertEqual(ov.overlaps(me, [o]), [(o, {9}, set())])

    def test_two_shared_functions_and_a_tenth(self):
        me = pull(1, fx("a", "b", "c"))
        o = pull(2, fx("a", "b", *[f"x{i}" for i in range(10)]))
        self.assertEqual([t[0].number for t in ov.overlaps(me, [o])], [2])
        big = pull(3, fx("a", "b", *[f"x{i}" for i in range(30)]))
        self.assertEqual(ov.overlaps(me, [big]), [])  # 2 of 32: chance
        self.assertEqual(ov.overlaps(pull(1, fx("a")), [pull(2, fx("a"))]), [])  # one function: chance

    def test_hot_functions_dont_count(self):
        me = pull(1, fx("new", "draw_frame", "mine"))
        others = [pull(n, fx("new", "draw_frame")) for n in range(2, 6)] + [pull(9, fx("new", "draw_frame", "mine"))]
        self.assertEqual(ov.overlaps(me, others), [])

    def test_stacks_and_integration_branches_left_out(self):
        me, o = pull(1, fx("a", "b")), pull(2, fx("a", "b"))
        self.assertEqual(ov.overlaps(me, [o], stacked=lambda a, b: True), [])
        self.assertEqual(ov.overlaps(me, [pull(3, fx("a", "b"), changed=150)]), [])

    def test_most_shared_first_and_five_at_most(self):
        shared = {n: fx(*(f"f{n}_{i}" for i in range(n))) for n in range(2, 9)}  # 35 functions in all
        me = pull(1, set().union(*shared.values()))
        others = [pull(n, shared[n]) for n in shared] + [pull(20)]
        found = ov.overlaps(me, others)  # 4 or more of 35 is a tenth
        self.assertEqual([t[0].number for t in found], [8, 7, 6, 5, 4])
        me.closes = {5}
        others.append(pull(30, closes={5}))
        self.assertEqual(ov.overlaps(me, others)[0][0].number, 30)

    def test_candidates_share_a_file_or_an_issue(self):
        me = pull(1, files=["src/a.rs", "Cargo.lock", "src/lib.rs"], closes={4})
        cands = [pull(2, files=["src/a.rs"]), pull(3, files=["Cargo.lock", "src/lib.rs"]), pull(4, closes={4}),
                 pull(5, files=["src/a.rs"], changed=500)]
        self.assertEqual([p.number for p in ov.candidates(me, cands)], [4, 2])


class NoteTests(unittest.TestCase):
    def test_text(self):
        o = pull(7, author="dev")
        o.draft = True
        text = ov.note([(o, {3}, {("crates/x/src/a.rs", f) for f in "abcdef"})])
        self.assertTrue(text.startswith(ov.MARKER + "\n🤖 Overlap:"))
        self.assertIn("- #7 (draft) by dev: also closes #3; changes `a` in `a.rs`, `b` in `a.rs`, `c` in `a.rs`, "
                      "`d` in `a.rs` and 2 more", text)
        self.assertNotIn("@", text)  # names, never mentions
        self.assertIsNone(ov.note([]))
        self.assertTrue(ov.CLEARED.startswith(ov.MARKER))


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/overlap-note.yml").read_text()
        run_lines = [l.strip() for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["run: python3 tools/ci/overlap_note.py event"])
        for needed in ("pull_request_target", "ref: main", "persist-credentials: false", "filter: blob:none",
                       "sparse-checkout: |\n            tools/ci/overlap_note.py\n          sparse-checkout-cone-mode"):
            self.assertIn(needed, text)
        for bad in ("contents: write", "issues: write", "head.sha", "head.ref", "github.event.pull_request.body"):
            self.assertNotIn(bad, text)

    def test_git_never_runs_their_code(self):
        for flag in ("core.hooksPath=/dev/null", "core.fsmonitor=false", "diff.external="):
            self.assertIn(flag, ov.SAFE_GIT)


if __name__ == "__main__":
    unittest.main()
