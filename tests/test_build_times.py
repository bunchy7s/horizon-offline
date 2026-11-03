import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

from tools import build_times as bt

# The UNIT_DATA cargo 1.99.1 wrote for a workspace of a (lib) <- b (lib) <- c (bin with a build script).
# `c` bin waited for the build script's run, which waited for the build script: that chain decided the end.
REAL_UNITS = [
    {"i": 0, "name": "a", "version": "0.1.0", "mode": "todo", "target": "", "features": [], "start": 0.15,
     "duration": 0.08, "unblocked_units": [], "unblocked_rmeta_units": [1],
     "sections": [["frontend", {"start": 0.0, "end": 0.05}], ["codegen", {"start": 0.05, "end": 0.08}]]},
    {"i": 3, "name": "c", "version": "0.1.0", "mode": "todo", "target": " build-script", "features": [],
     "start": 0.15, "duration": 0.23, "unblocked_units": [4], "unblocked_rmeta_units": [], "sections": None},
    {"i": 1, "name": "b", "version": "0.1.0", "mode": "todo", "target": "", "features": [], "start": 0.2,
     "duration": 0.08, "unblocked_units": [], "unblocked_rmeta_units": [],
     "sections": [["frontend", {"start": 0.0, "end": 0.04}], ["codegen", {"start": 0.04, "end": 0.08}]]},
    {"i": 4, "name": "c", "version": "0.1.0", "mode": "run-custom-build", "target": " build-script (run)",
     "features": [], "start": 0.39, "duration": 0.02, "unblocked_units": [2], "unblocked_rmeta_units": [],
     "sections": None},
    {"i": 2, "name": "c", "version": "0.1.0", "mode": "todo", "target": ' c "bin"', "features": [],
     "start": 0.4, "duration": 0.27, "unblocked_units": [], "unblocked_rmeta_units": [], "sections": None},
]


def page(units):
    """A timings page shaped like cargo's: the array pretty-printed inside a script."""
    return ("<html><script>\nDURATION = 2;\nconst UNIT_DATA = " + json.dumps(units, indent=2)
            + ";\nconst CONCURRENCY_DATA = [];\n</script></html>")


def unit(i, name, start, duration, unblocks=(), rmeta=(), frontend=None, mode="todo", target=""):
    sections = None if frontend is None else [["frontend", {"start": 0.0, "end": frontend}]]
    return {"i": i, "name": name, "version": "0.1.0", "mode": mode, "target": target, "features": [],
            "start": start, "duration": duration, "unblocked_units": list(unblocks),
            "unblocked_rmeta_units": list(rmeta), "sections": sections}


class TimingsTests(unittest.TestCase):
    def test_reads_the_units_of_a_real_timings_page(self):
        units = bt.parse_timings(page(REAL_UNITS))
        self.assertEqual([u["i"] for u in units], [0, 3, 1, 4, 2])
        self.assertEqual(units[4]["target"], ' c "bin"')

    def test_page_without_units_or_with_broken_ones(self):
        self.assertIsNone(bt.parse_timings("<html>no data</html>"))
        self.assertIsNone(bt.parse_timings("const UNIT_DATA = [ {oops"))
        self.assertEqual(bt.parse_timings(page([])), [])

    def test_critical_path_follows_what_each_unit_waited_for(self):
        path = bt.critical_path(REAL_UNITS)
        self.assertEqual([p["unit"] for p in path],
                         ["c (build-script)", "c (run build script)", "c (bin)"])
        self.assertEqual(path[-1]["end_s"], 0.67)
        self.assertEqual(path[0]["start_s"], 0.15)

    def test_critical_path_picks_the_slowest_dependency_of_a_diamond(self):
        #   core(0..2) -> left(2..3) -> top(6..7)
        #              -> right(2..6) -> top
        units = [unit(0, "core", 0, 2, unblocks=[1, 2]), unit(1, "left", 2, 1, unblocks=[3]),
                 unit(2, "right", 2, 4, unblocks=[3]), unit(3, "top", 6, 1)]
        self.assertEqual([p["unit"] for p in bt.critical_path(units)], ["core", "right", "top"])

    def test_dependent_that_needs_only_metadata_waits_for_the_frontend_not_the_end(self):
        # fast_meta ends its frontend at 1.0 (but runs until 5.5); slow ends at 5.0. user, which needs the
        # metadata of the first and the whole of the second, started at 5.0: slow decided it.
        units = [unit(0, "fast_meta", 0, 5.5, rmeta=[2], frontend=1.0), unit(1, "slow", 0, 5, unblocks=[2]),
                 unit(2, "user", 5, 2)]
        self.assertEqual([p["unit"] for p in bt.critical_path(units)], ["slow", "user"])
        # With the frontend ending at 4.5 the metadata is ready later than slow's end 4.0.
        units = [unit(0, "fast_meta", 0, 5.5, rmeta=[2], frontend=4.5), unit(1, "slow", 0, 4, unblocks=[2]),
                 unit(2, "user", 5, 2)]
        self.assertEqual([p["unit"] for p in bt.critical_path(units)], ["fast_meta", "user"])

    def test_a_binary_named_like_its_package_is_labelled_by_its_kind(self):
        self.assertEqual(bt.unit_label(unit(0, "horizon-client", 0, 1, target=' horizon-client "bin"')), "horizon-client (bin)")
        self.assertEqual(bt.unit_label(unit(0, "horizon-client", 0, 1, target=' lan_bot "test"')), "horizon-client (test lan_bot)")
        self.assertEqual(bt.unit_label(unit(0, "horizon-client", 0, 1)), "horizon-client")
        # cargo before 1.99 puts the kind first
        self.assertEqual(bt.unit_label(unit(0, "horizon-client", 0, 1, target=' bin "horizon-client"')), "horizon-client (bin)")
        self.assertEqual(bt.unit_label(unit(0, "horizon-client", 0, 1, target=' bin "horizon-ctl"')), "horizon-client (bin horizon-ctl)")

    def test_pages_of_older_cargo_give_the_same_critical_path(self):
        # Up to 1.92 cargo writes unlocked_units / unlocked_rmeta_units, and rmeta_time instead of sections.
        def old(u):
            u = dict(u)
            u["unlocked_units"], u["unlocked_rmeta_units"] = u.pop("unblocked_units"), u.pop("unblocked_rmeta_units")
            sections = dict(u.pop("sections") or [])
            u["rmeta_time"] = sections["frontend"]["end"] if "frontend" in sections else None
            return u
        self.assertEqual(bt.critical_path([old(u) for u in REAL_UNITS]), bt.critical_path(REAL_UNITS))
        # what the metadata-only dependent waits for: rmeta_time, as the frontend section in newer pages
        units = [unit(0, "fast_meta", 0, 5.5, rmeta=[2], frontend=4.5), unit(1, "slow", 0, 4, unblocks=[2]), unit(2, "user", 5, 2)]
        self.assertEqual([p["unit"] for p in bt.critical_path([old(u) for u in units])], ["fast_meta", "user"])

    def test_units_without_the_fields_read_are_no_timings_rather_than_a_crash(self):
        self.assertIsNone(bt.parse_timings(page([{"i": 0, "name": "a", "begin": 0.0, "length": 1.0}])))
        self.assertIsNone(bt.parse_timings(page(["a"])))

    def test_summary_adds_a_crates_units_and_ranks_them(self):
        summary = bt.summarize_timings(REAL_UNITS, top=2)
        self.assertEqual(summary["units"], 5)
        self.assertEqual(summary["total_s"], 0.67)
        # c: build script 0.23 + its run 0.02 + bin 0.27 = 0.52
        self.assertEqual(summary["top"], [{"crate": "c", "seconds": 0.52, "units": 3},
                                          {"crate": "a", "seconds": 0.08, "units": 1}])
        self.assertEqual(len(summary["crates"]), 3)
        self.assertIsNone(bt.summarize_timings([]))


class EditTests(unittest.TestCase):
    SOURCE = (b"use std::fmt;\n\npub struct A;\n\nimpl A {\n    pub fn f(&self) {}\n}\n\npub fn g() {}\n\n"
              b"pub fn h() {}\n\nfn tail() {}\n")

    def test_every_mode_adds_exactly_one_comment_line(self):
        for mode in ["middle", "top", "append"]:
            with self.subTest(mode=mode):
                edited = bt.apply_edit(self.SOURCE, mode, "t1")
                added = [l for l in edited.splitlines(keepends=True) if l not in self.SOURCE.splitlines(keepends=True)]
                self.assertEqual(added, [b"// build-times probe t1\n"])
                removed = edited.replace(b"// build-times probe t1\n", b"")
                self.assertEqual(removed, self.SOURCE)

    def test_modes_place_the_line_where_they_say(self):
        self.assertTrue(bt.apply_edit(self.SOURCE, "top", "t").startswith(b"// build-times probe"))
        self.assertTrue(bt.apply_edit(self.SOURCE, "append", "t").endswith(b"// build-times probe t\n"))
        lines = bt.apply_edit(self.SOURCE, "middle", "t").splitlines()
        at = lines.index(b"// build-times probe t")
        self.assertEqual(lines[at - 1], b"")  # between items, never inside one
        self.assertTrue(lines[at + 1].startswith((b"pub fn", b"impl", b"fn")))
        self.assertTrue(0 < at < len(lines) - 2)

    def test_files_without_a_final_newline_and_with_windows_line_ends(self):
        self.assertEqual(bt.apply_edit(b"fn a() {}", "append", "t"), b"fn a() {}\n// build-times probe t\n")
        edited = bt.apply_edit(b"fn a() {}\r\nfn b() {}\r\n", "append", "t")
        self.assertEqual(edited, b"fn a() {}\r\nfn b() {}\r\n// build-times probe t\r\n")

    def test_touch_leaves_the_bytes_alone_but_moves_the_modification_time(self):
        self.assertEqual(bt.apply_edit(self.SOURCE, "touch", "t"), self.SOURCE)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "a.rs")
            path.write_bytes(self.SOURCE)
            os.utime(path, ns=(1_500_000_000_000_000_000,) * 2)
            probe = bt.Probe(path, Path(tmp))
            probe.apply("touch", "t")
            self.assertEqual(path.read_bytes(), self.SOURCE)
            self.assertGreater(path.stat().st_mtime_ns, 1_500_000_000_000_000_000)
            probe.close()

    def test_unknown_mode_is_an_error(self):
        with self.assertRaises(bt.BuildTimesError):
            bt.apply_edit(self.SOURCE, "sideways", "t")


FAKE_CARGO = textwrap.dedent('''
    import json, os, pathlib, sys, time
    args = sys.argv[1:]
    state = json.loads(pathlib.Path(os.environ["FAKE_CARGO_STATE"]).read_text())
    calls = pathlib.Path(state["calls"])
    with calls.open("a") as out:
        out.write(json.dumps({"args": args, "CARGO_TARGET_DIR": os.environ.get("CARGO_TARGET_DIR"),
                              "seen_probe": [("build-times probe" in pathlib.Path(f).read_text()) for f in state["watch"]],
                              "tokens": [l for f in state["watch"] for l in pathlib.Path(f).read_text().splitlines() if "build-times probe" in l]}) + "\\n")
    if args[0] == "--version":
        print("cargo 1.99.1 (fake)")
    elif args[0] == "metadata":
        print(json.dumps(state["metadata"]))
    elif args[0] == "build":
        target = pathlib.Path(args[args.index("--target-dir") + 1])
        if state.get("fail_on_probe") and any(("build-times probe" in pathlib.Path(f).read_text()) for f in state["watch"]):
            print("error[E0000]: the fake compiler refuses the edit")
            sys.exit(101)
        (target / "cargo-timings").mkdir(parents=True, exist_ok=True)
        probe = any(("build-times probe" in pathlib.Path(f).read_text()) for f in state["watch"])
        built = target / "built"
        cold = not built.exists()
        built.write_text("x")
        units = state["cold_units"] if cold else (state["edit_units"] if probe else [])
        if not state.get("no_timings"):
            html = "<script>\\nconst UNIT_DATA = " + json.dumps(units, indent=2) + ";\\n</script>"
            (target / "cargo-timings" / ("cargo-timing-%d.html" % time.time_ns())).write_text(html)
''')


def git(root, *args):
    subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args], cwd=root, check=True,
                   capture_output=True)


class RunTests(unittest.TestCase):
    """The whole run, with a fake cargo and a throwaway git repository."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name, "repo").resolve()
        self.files = {"horizon-items": "crates/horizon-items/src/items.rs",
                      "horizon-world": "crates/horizon-world/src/level/mod.rs"}
        for relative in self.files.values():
            path = self.root / relative
            path.parent.mkdir(parents=True)
            path.write_bytes(b"use a;\n\npub fn one() {}\n\npub fn two() {}\n\npub fn three() {}\n")
        (self.root / "crates/horizon-items/src/lib.rs").write_text("pub mod items;\n")
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "first")
        (self.root / ".gitignore").write_text(".local/\n")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "ignore .local")
        self.fake = Path(self.tmp.name, "fake_cargo.py")
        self.fake.write_text(FAKE_CARGO)
        self.state_path = Path(self.tmp.name, "state.json")
        self.calls = Path(self.tmp.name, "calls.jsonl")
        self.state = {
            "calls": str(self.calls), "watch": [str(self.root / r) for r in self.files.values()],
            "metadata": {"packages": [
                {"name": "horizon-items", "targets": [{"kind": ["lib"], "src_path": str(self.root / "crates/horizon-items/src/lib.rs")}]},
                {"name": "horizon-world", "targets": [{"kind": ["lib"], "src_path": str(self.root / "crates/horizon-world/src/lib.rs")}]}]},
            "cold_units": REAL_UNITS,
            "edit_units": [unit(0, "horizon-items", 0, 1.5, unblocks=[1]), unit(1, "horizon-client", 1.5, 2.0, target=' x "bin"')],
        }
        self.write_state()
        patches = [mock.patch.dict(os.environ, {"FAKE_CARGO_STATE": str(self.state_path)}),
                   mock.patch.object(bt, "other_build_processes", return_value=[]),
                   mock.patch.object(bt, "cpu_busy_percent", return_value=1.0),
                   mock.patch.object(bt, "machine_info", return_value={"os": "linux", "cpu": "Test CPU", "cpu_threads": 8,
                                                                       "ram_mb": 16384, "os_version": "x", "arch": "x86_64"})]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        for key in ("CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "RUSTC_WRAPPER", "CARGO_INCREMENTAL"):
            os.environ.pop(key, None)

    def write_state(self):
        self.state_path.write_text(json.dumps(self.state))

    def call(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = bt.main(list(argv), root=self.root, cargo=[sys.executable, str(self.fake)])
        return code, out.getvalue(), err.getvalue()

    def report(self, *argv):
        output = Path(self.tmp.name, "out", "report.json")
        code, out, err = self.call("run", "--output", str(output), *argv)
        return code, json.loads(output.read_text()), out, err

    def builds(self):
        return [json.loads(l) for l in self.calls.read_text().splitlines() if json.loads(l)["args"][0] == "build"]

    def test_report_has_what_two_machines_need_to_be_compared(self):
        code, report, out, _ = self.report("--profiles", "dev", "--crates", "horizon-items,horizon-world", "--jobs", "6")
        self.assertEqual(code, 0)
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.root, capture_output=True, text=True).stdout.strip()
        self.assertEqual(report["format"], "horizon-build-times")
        self.assertEqual(report["commit"]["sha"], head)
        self.assertFalse(report["commit"]["dirty"])
        self.assertEqual(report["machine"]["cpu"], "Test CPU")
        self.assertEqual(report["machine"]["ram_mb"], 16384)
        self.assertEqual(report["toolchain"]["cargo"], "cargo 1.99.1 (fake)")
        self.assertEqual(report["jobs"], {"jobs": 6, "source": "--jobs"})
        cold = report["cold"]["dev"]
        self.assertTrue(cold["ok"])
        self.assertGreater(cold["seconds"], 0)
        self.assertEqual(cold["timings"]["units"], 5)
        self.assertEqual([p["unit"] for p in cold["timings"]["critical_path"]][-1], "c (bin)")
        self.assertTrue(report["noop"]["dev"]["ok"])
        inc = report["incremental"]["dev"]
        self.assertEqual(set(inc), {"horizon-items", "horizon-world"})
        self.assertEqual(inc["horizon-items"]["file"], "crates/horizon-items/src/items.rs")
        self.assertEqual(inc["horizon-items"]["rebuilt_crates"], ["horizon-client", "horizon-items"])
        self.assertEqual(inc["horizon-items"]["rebuilt_units"], 2)
        self.assertIn("| dev | horizon-items |", out)
        self.assertTrue(Path(self.tmp.name, "out", "report.md").is_file())

    def test_cargo_is_told_the_same_command_in_every_build(self):
        self.report("--profiles", "dev,fast", "--crates", "horizon-items", "--jobs", "3")
        for call in self.builds():
            args = call["args"]
            self.assertEqual(args[:5], ["build", "--locked", "-p", "horizon-client", "--profile"])
            self.assertIn(args[5], ["dev", "fast"])
            self.assertIn("--timings", args)
            self.assertEqual(args[args.index("--jobs") + 1], "3")
        profiles = {c["args"][5] for c in self.builds()}
        self.assertEqual(profiles, {"dev", "fast"})
        self.assertTrue(any(c["args"][0] == "fetch" for c in map(json.loads, self.calls.read_text().splitlines())))

    def test_jobs_come_from_the_environment_when_not_given(self):
        with mock.patch.dict(os.environ, {"CARGO_BUILD_JOBS": "4"}):
            _, report, _, _ = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(report["jobs"], {"jobs": "4", "source": "CARGO_BUILD_JOBS"})
        self.assertEqual(report["environment"]["CARGO_BUILD_JOBS"], "4")
        self.assertNotIn("--jobs", self.builds()[0]["args"])

    def test_the_edited_file_is_what_cargo_sees_and_is_put_back_exactly(self):
        path = self.root / self.files["horizon-items"]
        before = path.read_bytes()
        os.utime(path, ns=(1_500_000_000_000_000_000, 1_500_000_000_000_000_000))
        mtime = path.stat().st_mtime_ns
        self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(path.stat().st_mtime_ns, mtime)
        seen = [c["seen_probe"][0] for c in self.builds()]
        # warm build, no-op, the edited build, the build of the unedited file
        self.assertEqual(seen, [False, False, True, False])
        self.assertEqual(subprocess.run(["git", "status", "--porcelain"], cwd=self.root, capture_output=True,
                                        text=True).stdout, "")

    def test_a_failing_build_leaves_the_file_as_found(self):
        path = self.root / self.files["horizon-items"]
        before = path.read_bytes()
        os.utime(path, ns=(1_500_000_000_000_000_000,) * 2)
        self.state["fail_on_probe"] = True
        self.write_state()
        code, report, out, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertEqual(code, 1)
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(path.stat().st_mtime_ns, 1_500_000_000_000_000_000)
        entry = report["incremental"]["dev"]["horizon-items"]
        self.assertFalse(entry["ok"])
        self.assertIn("refuses the edit", entry["runs"][0]["error"])
        self.assertIn("FAILED", out)
        self.assertEqual(list((self.root / ".local/build-times").glob("pending-*.json")), [])

    def test_a_failed_edit_is_built_back_before_the_next_crate_is_measured(self):
        # else the next measurement also pays for recompiling the crate that failed
        self.state["fail_on_probe"] = True
        self.write_state()
        self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items,horizon-world")
        seen = [c["seen_probe"] for c in self.builds()]
        # warm, no-op, items edited (fails), items built back, world edited (fails), world built back
        self.assertEqual(seen, [[False, False], [False, False], [True, False], [False, False], [False, True],
                                [False, False]])

    def test_an_interrupt_during_the_build_leaves_the_file_as_found(self):
        path = self.root / self.files["horizon-world"]
        before = path.read_bytes()
        os.utime(path, ns=(1_500_000_000_000_000_000,) * 2)
        real = bt.Builder.build

        def interrupted(builder, profile, label):
            if label.startswith("edit-"):
                self.assertNotEqual(path.read_bytes(), before)  # the edit is in place when the interrupt comes
                raise KeyboardInterrupt
            return real(builder, profile, label)

        with mock.patch.object(bt.Builder, "build", interrupted), self.assertRaises(KeyboardInterrupt):
            self.call("run", "--profiles", "dev", "--only", "incremental", "--crates", "horizon-world",
                      "--output", str(Path(self.tmp.name, "out", "r.json")))
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(path.stat().st_mtime_ns, 1_500_000_000_000_000_000)
        self.assertEqual(list((self.root / ".local/build-times").glob("pending-*.json")), [])

    def test_a_file_someone_else_writes_during_the_run_is_not_put_back(self):
        path = self.root / self.files["horizon-items"]
        os.utime(path, ns=(1_500_000_000_000_000_000,) * 2)
        mine = b"use a;\n\npub fn my_new_work() {}\n"
        real = bt.Builder.build

        def editor_saves(builder, profile, label):
            result = real(builder, profile, label)
            if label.startswith("edit-"):
                path.write_bytes(mine)  # the developer saves the file while it builds
            return result

        with mock.patch.object(bt.Builder, "build", editor_saves):
            code, _, err = self.call("run", "--profiles", "dev", "--only", "incremental", "--crates", "horizon-items",
                                     "--output", str(Path(self.tmp.name, "out", "r.json")))
        self.assertEqual(code, 2)
        self.assertIn("changed by something else", err)
        self.assertEqual(path.read_bytes(), mine)
        self.assertGreater(path.stat().st_mtime_ns, 1_500_000_000_000_000_000)  # not set back: cargo must see the change
        self.assertEqual(list((self.root / ".local/build-times").glob("pending-*.json")), [])

    def test_a_killed_run_is_undone_by_the_next_one(self):
        path = self.root / self.files["horizon-items"]
        before = path.read_bytes()
        os.utime(path, ns=(1_500_000_000_000_000_000,) * 2)
        probe = bt.Probe(path, self.root)
        probe.apply("middle", "left-behind")  # as a run killed from outside would leave it
        self.assertNotEqual(path.read_bytes(), before)
        code, report, _, err = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(code, 0)
        self.assertEqual(path.read_bytes(), before)
        self.assertEqual(path.stat().st_mtime_ns, 1_500_000_000_000_000_000)
        self.assertIn("restored", err)
        self.assertFalse(report["commit"]["dirty"])

    def test_a_run_killed_while_writing_the_file_is_undone_too(self):
        path = self.root / self.files["horizon-items"]
        before = path.read_bytes()
        bt.Probe(path, self.root).apply("top", "cut")
        path.write_bytes(path.read_bytes()[:10])  # the write was cut short
        code, _, _, _ = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(code, 0)
        self.assertEqual(path.read_bytes(), before)

    def test_work_done_after_a_killed_run_is_never_overwritten(self):
        path = self.root / self.files["horizon-items"]
        bt.Probe(path, self.root).apply("middle", "left-behind")
        mine = b"use a;\n\npub fn my_new_work() {}\n"  # the developer took the line out and went on working
        path.write_bytes(mine)
        code, _, err = self.call("run", "--profiles", "dev", "--only", "cold", "--allow-dirty")
        self.assertEqual(code, 2)
        self.assertIn("was changed since a run was interrupted", err)
        self.assertEqual(path.read_bytes(), mine)
        self.assertFalse(self.calls.exists())  # not one cargo call

    def test_refuses_a_dirty_tree_unless_allowed(self):
        path = self.root / self.files["horizon-items"]
        path.write_bytes(path.read_bytes() + b"// mine\n")
        code, out, err = self.call("run", "--profiles", "dev", "--only", "cold")
        self.assertEqual(code, 2)
        self.assertIn("uncommitted changes", err)
        self.assertIn("items.rs", err)
        self.assertFalse(self.calls.exists())  # not one cargo call
        self.assertEqual(path.read_bytes().endswith(b"// mine\n"), True)
        code, report, _, _ = self.report("--profiles", "dev", "--only", "cold", "--allow-dirty")
        self.assertEqual(code, 0)
        self.assertTrue(report["commit"]["dirty"])

    def test_an_edit_in_a_dirty_file_still_restores_the_dirty_content(self):
        path = self.root / self.files["horizon-items"]
        path.write_bytes(path.read_bytes() + b"// mine\n")
        mine = path.read_bytes()
        self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items", "--allow-dirty")
        self.assertEqual(path.read_bytes(), mine)

    def test_a_crate_that_does_not_exist_is_refused_with_the_list(self):
        code, _, err = self.call("run", "--crates", "horizon-nope", "--profiles", "dev")
        self.assertEqual(code, 2)
        self.assertIn("no crate named horizon-nope", err)
        self.assertIn("horizon-items", err)
        self.assertNotIn("Traceback", err)

    def test_a_file_that_does_not_exist_is_refused(self):
        code, _, err = self.call("run", "--crates", "horizon-items", "--file", "horizon-items=crates/none.rs")
        self.assertEqual(code, 2)
        self.assertIn("does not exist", err)

    def test_a_file_outside_the_repository_is_refused(self):
        outside = Path(self.tmp.name, "elsewhere.rs")
        outside.write_bytes(b"fn a() {}\n")
        for given in (str(outside), "../elsewhere.rs"):
            with self.subTest(given=given):
                code, _, err = self.call("run", "--crates", "horizon-items", "--file", f"horizon-items={given}")
                self.assertEqual(code, 2)
                self.assertIn("outside the repository", err)
                self.assertNotIn("Traceback", err)
        self.assertEqual(outside.read_bytes(), b"fn a() {}\n")

    def test_a_crate_without_a_listed_file_gets_its_lib_rs(self):
        self.state["metadata"]["packages"].append(
            {"name": "horizon-extra", "targets": [{"kind": ["lib"], "src_path": str(self.root / "crates/horizon-items/src/lib.rs")}]})
        self.write_state()
        _, report, _, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-extra")
        self.assertEqual(report["incremental"]["dev"]["horizon-extra"]["file"], "crates/horizon-items/src/lib.rs")

    def test_the_users_cargo_target_dir_is_not_used_or_touched(self):
        mine = Path(self.tmp.name, "my-target")
        mine.mkdir()
        (mine / "keep.txt").write_text("precious")
        with mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": str(mine)}):
            _, report, _, _ = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(sorted(p.name for p in mine.iterdir()), ["keep.txt"])
        build = self.builds()[0]
        ours = self.root / ".local/build-times/target-dev"
        self.assertEqual(Path(build["args"][build["args"].index("--target-dir") + 1]), ours)
        self.assertEqual(Path(build["CARGO_TARGET_DIR"]), ours)
        self.assertTrue(any("CARGO_TARGET_DIR is set" in w for w in report["warnings"]))

    def test_cold_build_empties_only_its_own_target_folder(self):
        ours = self.root / ".local/build-times/target-dev"
        self.report("--profiles", "dev", "--only", "cold")
        (ours / "stale.rlib").write_text("old")
        self.report("--profiles", "dev", "--only", "cold")
        self.assertFalse((ours / "stale.rlib").exists())
        self.assertTrue((ours / bt.MARKER).exists())

    def test_a_folder_the_tool_did_not_make_is_never_emptied(self):
        foreign = Path(self.tmp.name, "somebody-elses")
        (foreign / "target-dev").mkdir(parents=True)
        (foreign / "target-dev" / "work.txt").write_text("hands off")
        code, _, err = self.call("run", "--profiles", "dev", "--only", "cold", "--target-dir", str(foreign))
        self.assertEqual(code, 2)
        self.assertIn("was not made by build_times.py", err)
        self.assertEqual((foreign / "target-dev" / "work.txt").read_text(), "hands off")

    def test_the_repository_target_folder_is_never_emptied(self):
        (self.root / "target").mkdir()
        (self.root / "target" / "keep").write_text("x")
        with self.assertRaises(bt.BuildTimesError):
            bt.prepare_target(self.root, self.root / "target", wipe=True)
        self.assertTrue((self.root / "target" / "keep").exists())

    def test_missing_timings_still_give_the_seconds_and_a_warning(self):
        self.state["no_timings"] = True
        self.write_state()
        code, report, _, _ = self.report("--profiles", "dev", "--crates", "horizon-items")
        self.assertEqual(code, 0)
        self.assertIsNone(report["cold"]["dev"]["timings"])
        self.assertGreater(report["cold"]["dev"]["seconds"], 0)
        self.assertIsNone(report["incremental"]["dev"]["horizon-items"]["rebuilt_crates"])
        self.assertTrue(any(w.startswith("cold dev: cargo wrote no timings page") for w in report["warnings"]))
        self.assertIn("Cold builds", bt.render_markdown(report))

    def test_an_edit_that_recompiles_nothing_is_called_out(self):
        self.state["edit_units"] = []
        self.write_state()
        _, report, _, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertTrue(any("recompiled nothing" in w for w in report["warnings"]))

    def test_a_middle_edit_says_where_it_went_and_when_it_had_to_go_to_the_top(self):
        _, report, out, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        # 7 lines, items after a blank line on lines 3, 5 and 7; the middle (line 4) is as near 3 as 5: the first wins
        self.assertEqual(report["incremental"]["dev"]["horizon-items"]["line"], 3)
        self.assertIn("`crates/horizon-items/src/items.rs:3`", out)
        self.assertEqual([w for w in report["warnings"] if "line 1" in w], [])
        path = self.root / self.files["horizon-items"]
        path.write_bytes(b"/// Docs.\npub fn one() {}\n/// More.\npub fn two() {}\n")  # no item after a blank line
        git(self.root, "commit", "-qam", "no spot")
        _, report, _, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertEqual(report["incremental"]["dev"]["horizon-items"]["line"], 1)
        self.assertTrue(any("items.rs has no top-level item after a blank line, so the comment went on line 1" in w
                            for w in report["warnings"]))

    def test_an_edit_that_misses_its_own_crate_is_called_out(self):
        self.state["edit_units"] = [unit(0, "horizon-client", 0, 1)]
        self.write_state()
        _, report, _, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertTrue(any("did not recompile horizon-items itself" in w for w in report["warnings"]))

    def test_other_cargo_processes_are_recorded_as_skew(self):
        with mock.patch.object(bt, "other_build_processes", return_value=["cargo", "rustc"]):
            _, report, out, _ = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(report["cold"]["dev"]["other_processes"], ["cargo", "rustc"])
        self.assertTrue(any("other build programs were running (cargo, rustc)" in w for w in report["warnings"]))
        self.assertIn("warning: cold dev: other build programs", out)

    def test_a_busy_machine_is_warned_about_and_the_figure_kept_with_the_number(self):
        _, report, out, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertEqual(report["noop"]["dev"]["cpu_busy_before_pct"], 1.0)
        self.assertEqual([w for w in report["warnings"] if "busy" in w], [])
        with mock.patch.object(bt, "cpu_busy_percent", return_value=62.5), mock.patch.object(bt.time, "sleep"):
            _, report, out, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items")
        self.assertEqual(report["incremental"]["dev"]["horizon-items"]["runs"][0]["cpu_busy_before_pct"], 62.5)
        self.assertTrue(any("incremental dev horizon-items: the machine was 62.5% busy" in w for w in report["warnings"]))
        self.assertIn("warning: no-op dev: the machine was 62.5% busy", out)

    def test_a_busy_cpu_that_calms_down_is_waited_for_not_warned_about(self):
        readings = iter([40.0, 30.0, 3.0] + [3.0] * 50)
        with mock.patch.object(bt, "cpu_busy_percent", side_effect=lambda *_: next(readings)), \
                mock.patch.object(bt.time, "sleep") as slept:
            _, report, _, _ = self.report("--profiles", "dev", "--only", "cold")
        self.assertEqual(report["cold"]["dev"]["cpu_busy_before_pct"], 3.0)
        self.assertEqual([w for w in report["warnings"] if "busy" in w], [])
        self.assertEqual(slept.call_count, 2)

    def test_repeats_run_the_edit_again_and_report_the_median(self):
        _, report, _, _ = self.report("--profiles", "dev", "--only", "incremental", "--crates", "horizon-items",
                                      "--repeats", "3")
        entry = report["incremental"]["dev"]["horizon-items"]
        self.assertEqual(len(entry["runs"]), 3)
        self.assertEqual(entry["seconds"], sorted(r["seconds"] for r in entry["runs"])[1])
        # each repeat edits with its own text, so rustc never sees the state of an earlier edit
        tokens = [t for c in self.builds() for t in c["tokens"]]
        self.assertEqual(len(tokens), 3)
        self.assertEqual(len(set(tokens)), 3)

    def test_a_second_run_of_the_day_keeps_the_first_report(self):
        self.call("run", "--profiles", "dev", "--only", "cold")
        self.call("run", "--profiles", "dev", "--only", "cold")
        reports = {p.stem for p in (self.root / ".local/build-times/reports").glob("*.json")}
        self.assertEqual(len(reports), 2)
        self.assertTrue({r + "-2" for r in reports} & reports, reports)

    def test_a_failed_cold_build_is_reported_and_stops_the_profile(self):
        with mock.patch.object(bt.Builder, "build", return_value={"ok": False, "seconds": 3.0, "log": str(self.root / "x.log"),
                                                                   "code": 101, "units": None, "timings": None}):
            code, report, out, _ = self.report("--profiles", "dev", "--crates", "horizon-items")
        self.assertEqual(code, 1)
        self.assertFalse(report["cold"]["dev"]["ok"])
        self.assertEqual(report["incremental"], {})
        self.assertIn("FAILED", out)


def sample(seconds, cpu="Test CPU", jobs=8, commit="aaaaaaaaa"):
    return {"format": "horizon-build-times", "version": 1, "created": "2026-11-01T10:00:00Z", "label": None,
            "commit": {"short": commit, "sha": commit, "branch": "main", "subject": "s", "dirty": False},
            "machine": {"os": "linux", "cpu": cpu, "cpu_threads": 8, "ram_mb": 16384},
            "toolchain": {"rustc": "rustc 1.99.1", "cargo": "cargo 1.99.1", "host": "x"}, "jobs": {"jobs": jobs, "source": "default"},
            "options": {"edit": "middle", "repeats": 1, "files": {"horizon-items": "crates/horizon-items/src/items.rs"}},
            "warnings": [], "environment": {}, "command": "cargo build",
            "cold": {"dev": {"ok": True, "seconds": seconds["cold"], "timings": {
                "units": 3, "total_s": seconds["cold"],
                "top": [{"crate": "horizon-world", "seconds": seconds["world"], "units": 1}],
                "critical_path": [{"unit": "horizon-world", "start_s": 0, "end_s": seconds["world"], "seconds": seconds["world"]}]}}},
            "noop": {"dev": {"ok": True, "seconds": seconds["noop"]}},
            "incremental": {"dev": {"horizon-items": {"ok": True, "seconds": seconds["items"], "file": "x"}}}}


class CompareTests(unittest.TestCase):
    def test_shows_before_after_and_the_change(self):
        a = sample({"cold": 200.0, "noop": 1.0, "items": 20.0, "world": 80.0})
        b = sample({"cold": 150.0, "noop": 1.0, "items": 25.0, "world": 40.0}, commit="bbbbbbbbb")
        text = bt.compare(a, b)
        self.assertIn("| dev | 200.0 | 150.0 | -50.0 | -25% |", text)
        self.assertIn("| dev | horizon-items | 20.0 | 25.0 | +5.0 | +25% |", text)
        self.assertIn("| dev | 1.0 | 1.0 | +0.0 | +0% |", text)
        self.assertIn("| horizon-world | 80.0 | 40.0 | -40.0 | -50% |", text)
        self.assertIn("`aaaaaaaaa`", text)
        self.assertIn("`bbbbbbbbb`", text)
        self.assertNotIn("Read with care", text)

    def test_warns_when_the_machines_differ(self):
        a = sample({"cold": 200.0, "noop": 1.0, "items": 20.0, "world": 80.0})
        b = sample({"cold": 150.0, "noop": 1.0, "items": 25.0, "world": 40.0}, cpu="Other CPU", jobs=4)
        text = bt.compare(a, b)
        self.assertIn("Read with care", text)
        self.assertIn("CPU differs: Test CPU / Other CPU", text)
        self.assertIn("jobs differs: 8 / 4", text)

    def test_warns_when_the_build_environment_differs(self):
        a = sample({"cold": 200.0, "noop": 1.0, "items": 20.0, "world": 80.0})
        b = sample({"cold": 150.0, "noop": 1.0, "items": 25.0, "world": 40.0})
        a["environment"] = {"CARGO_TARGET_DIR": "/a"}
        b["environment"] = {"CARGO_TARGET_DIR": "/b"}
        self.assertNotIn("Read with care", bt.compare(a, b))  # the tool never builds there
        b["environment"]["RUSTFLAGS"] = "-C target-cpu=native"
        self.assertIn('environment differs: {} / {"RUSTFLAGS": "-C target-cpu=native"}', bt.compare(a, b))

    def test_a_report_whose_first_incremental_build_failed_can_be_compared(self):
        a = sample({"cold": 200.0, "noop": 1.0, "items": 20.0, "world": 80.0})
        b = sample({"cold": 150.0, "noop": 1.0, "items": 25.0, "world": 40.0})
        b["incremental"]["dev"] = {"error": "the first build failed:\nerror[E0000]"}
        self.assertIn("| dev | horizon-items | 20.0 | - | - | - |", bt.compare(a, b))
        self.assertIn("| dev | horizon-items | - | 20.0 | - | - |", bt.compare(b, a))

    def test_crates_and_profiles_on_one_side_only_get_a_dash(self):
        a = sample({"cold": 200.0, "noop": 1.0, "items": 20.0, "world": 80.0})
        b = sample({"cold": 150.0, "noop": 1.0, "items": 25.0, "world": 40.0})
        b["cold"]["fast"] = dict(b["cold"]["dev"])
        b["incremental"]["dev"]["horizon-new"] = {"ok": True, "seconds": 3.0, "file": "y"}
        text = bt.compare(a, b)
        self.assertIn("| fast | - | 150.0 | - | - |", text)
        self.assertIn("| dev | horizon-new | - | 3.0 | - | - |", text)

    def test_command_line_compare_reads_files_and_rejects_other_json(self):
        with tempfile.TemporaryDirectory() as tmp:
            a, b = Path(tmp, "a.json"), Path(tmp, "b.json")
            a.write_text(json.dumps(sample({"cold": 10.0, "noop": 1.0, "items": 2.0, "world": 3.0})))
            b.write_text(json.dumps(sample({"cold": 5.0, "noop": 1.0, "items": 2.0, "world": 3.0})))
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                self.assertEqual(bt.main(["compare", str(a), str(b), "--output", str(Path(tmp, "c.md"))]), 0)
            self.assertIn("| dev | 10.0 | 5.0 | -5.0 | -50% |", out.getvalue())
            self.assertEqual(Path(tmp, "c.md").read_text().strip(), out.getvalue().strip())
            b.write_text('{"hello": 1}')
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                self.assertEqual(bt.main(["compare", str(a), str(b)]), 2)
            self.assertIn("not a build_times report", err.getvalue())


if __name__ == "__main__":
    unittest.main()
