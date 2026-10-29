from pathlib import Path
import json
import os
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import parity


class ParityRunnerTests(unittest.TestCase):
    def test_dep_info_keeps_escaped_spaces(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "probe"
            Path(temp, "probe.d").write_text(f"{binary}: /a/x.rs /b/with\\ space.rs\n\n/a/x.rs:\n")
            self.assertEqual(parity.dep_info(binary), ["/a/x.rs", "/b/with space.rs"])

    def test_artifacts_follow_profile_and_stop_at_program_arguments(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "--example", "a", "--bin", "b"]),
                         [["example", "a", "release", [], True], ["bin", "b", "release", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "run", "--profile", "fast", "--example", "c", "--", "--example", "x"]),
                         [["example", "c", "fast", [], True]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--example", "d"]), [["example", "d", "dev", [], True]])

    def test_artifacts_keep_the_package_and_feature_selection(self):
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--locked", "--profile=fast", "-j", "4", "--package",
                                              "horizon-client", "--bins", "-F", "x"]),
                         [["bins", "", "fast", ["-p", "horizon-client", "--features", "x"], False]])
        self.assertEqual(parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--bin=horizon-world"]),
                         [["bin", "horizon-world", "release", ["-p", "horizon-world"], False]])

    def test_prebuild_repeats_each_harness_selection(self):
        artifacts = [
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "a"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "b"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--bin", "horizon-world"]),
            *parity.artifacts_of(["cargo", "build", "--release", "--bin", "horizon-world"]),
            *parity.artifacts_of(["cargo", "run", "--profile", "fast", "-p", "horizon-client", "--example", "c"]),
            *parity.artifacts_of(["cargo", "build", "--release", "-p", "horizon-world", "--example", "renamed"]),
            ["example", "old", "release"],  # recorded before selections were kept: its own crate's -p
            ["example", "unknown", "release"],  # no longer in the workspace: left to its harness
        ]
        packages = {("example", "a"): "horizon-world", ("example", "b"): "horizon-world", ("example", "c"): "horizon-client",
                    ("bin", "horizon-world"): "horizon-world", ("example", "old"): "horizon-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "fast", "-p", "horizon-client", "--example", "c"],
            ["cargo", "build", "--locked", "--profile", "release", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--example", "a", "--example", "b",
             "--example", "old"],
        ])

    def test_a_probe_binary_depends_only_on_the_probes_it_ran(self):
        commands = [["/t/release/examples/world_probe", "a_fixture", "/dir"],
                    ["cargo", "run", "-p", "horizon-client", "--example", "client_probe", "--", "b_fixture"]]
        self.assertEqual(parity.probes_run(commands), {"a_fixture", "b_fixture"})
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            probe = Path(temp, "crates/w/examples/probe")
            (probe / "support").mkdir(parents=True)
            (probe / "a_fixture.rs").write_text('mod integration { include!("d_fixture.rs"); }')
            (probe / "d_fixture.rs").write_text("use crate::support::shared as s;")
            (probe / "b_fixture.rs").write_text("use crate::e_fixture;")
            for name in ("c_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs", "support/shared.rs", "support/other.rs"):
                (probe / name).write_text("")
            paths = [f"crates/w/examples/probe/{name}" for name in ("a_fixture.rs", "b_fixture.rs", "c_fixture.rs",
                                                                    "d_fixture.rs", "dispatch.rs", "e_fixture.rs", "main.rs",
                                                                    "support/other.rs", "support/shared.rs")]
            self.assertEqual(parity.only_probes_run(["crates/w/src/lib.rs", *paths], {"a_fixture", "b_fixture"}),
                             ["crates/w/src/lib.rs", *paths[:2], *paths[3:6], paths[8]])
            self.assertEqual(parity.only_probes_run(paths, set()), paths)

    def test_probe_names_are_each_crates_own_target(self):
        self.assertEqual(parity.probes_run([
            ["C:/t/debug/examples/client_probe.exe", "a_fixture"],
            ["cargo", "run", "--release", "--example=world_probe", "--", "b_fixture", "x"],
            ["/t/release/examples/probe", "old_fixture"],  # the shared name before 2026-10-24
            ["cargo", "run", "--example", "probe", "--", "old_fixture"],
            ["/t/release/examples/mod_port_probe", "not_a_fixture"],
            ["cargo", "run", "--example", "mod_port_probe", "--", "not_a_fixture"],
            ["cargo", "build", "--example", "world_probe"],
        ]), {"a_fixture", "b_fixture"})

    def test_records_of_the_shared_probe_name_build_their_crates_probe(self):
        record = {"dependencies": ["crates/horizon-world/examples/probe/a_fixture.rs", "crates/horizon-world/src/lib.rs"],
                  "artifacts": [["example", "probe", "release", ["-p", "horizon-client"], True],
                                ["example", "probe", "release"],  # no selection: the crate of its probe sources
                                ["example", "probe", "release", ["-p", "horizon-core", "-p", "horizon-world"], True],
                                ["bin", "horizon-world", "release", ["-p", "horizon-world"], False]]}
        artifacts = parity.recorded_artifacts(record)
        self.assertEqual(artifacts, [["example", "client_probe", "release", ["-p", "horizon-client"], True],
                                     ["example", "world_probe", "release"],
                                     ["example", "world_probe", "release", ["-p", "horizon-core", "-p", "horizon-world"], True],
                                     ["bin", "horizon-world", "release", ["-p", "horizon-world"], False]])
        packages = {("example", "client_probe"): "horizon-client", ("example", "world_probe"): "horizon-world",
                    ("bin", "horizon-world"): "horizon-world"}
        self.assertEqual(parity.prebuild_commands(artifacts, packages), [
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-client", "--example", "client_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-core", "-p", "horizon-world",
             "--example", "world_probe"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--bin", "horizon-world"],
            ["cargo", "build", "--locked", "--profile", "release", "-p", "horizon-world", "--example", "world_probe"],
        ])
        self.assertEqual(parity.recorded_artifacts({"artifacts": [["example", "probe", "release"]]}),
                         [["example", "probe", "release"]])  # no crate to tell: left to its harness

    def test_java_sources_include_classes_pulled_from_the_sourcepath(self):
        with tempfile.TemporaryDirectory() as temp:
            sources, classes = Path(temp, "src"), Path(temp, "classes")
            (sources / "p").mkdir(parents=True)
            (classes / "p").mkdir(parents=True)
            for name in ("Oracle", "Helper"):
                (sources / "p" / f"{name}.java").write_text("")
            for name in ("Oracle", "Helper$Inner"):
                (classes / "p" / f"{name}.class").write_bytes(b"")
            found = parity.java_sources(["javac", "-sourcepath", str(sources), "-d", str(classes), str(sources / "p/Oracle.java")])
            self.assertEqual(sorted(Path(path).name for path in found), ["Helper.java", "Oracle.java", "Oracle.java"])

    def test_a_pass_goes_stale_when_a_dependency_changes(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            source = Path(temp, "a.rs")
            source.write_text("one")
            state = {"harnesses": {"x": {"status": "pass", "dependencies": ["a.rs"]}}, "hashes": {}}
            hasher = parity.Hasher(state["hashes"])
            state["harnesses"]["x"]["key"] = parity.key(state["harnesses"]["x"], hasher, "env")
            self.assertFalse(parity.stale("x", state, hasher, "env"))
            self.assertTrue(parity.stale("x", state, hasher, "other toolchain"))
            source.write_text("two!")
            self.assertTrue(parity.stale("x", state, hasher, "env"))

    def test_a_change_affects_the_harnesses_that_depend_on_it(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            found = {name: Path(temp, "tools", f"test_{name}_parity.py") for name in ("food", "light", "new")}
            records = {"food": {"dependencies": ["crates/a.rs", "tools/lab.py"]},
                       "light": {"dependencies": ["crates/lib.rs#keep:light"]}}
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/a.rs"}), {"food", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/lib.rs"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"tools/test_light_parity.py"}), {"light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"Cargo.lock"}), {"food", "light", "new"})
            self.assertEqual(parity.affected_harnesses(found, records, {"README.md"}), {"new"})

    def test_platform_specific_harnesses_are_skipped_elsewhere(self):
        config = {"linux_only": {"options": "libm"}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config), {})
        with mock.patch.object(parity.sys, "platform", "darwin"):
            self.assertEqual(parity.platform_skips(config), {"options": "libm"})

    def test_termination_signals_are_caught_where_they_exist(self):
        for numbers in ({"SIGTERM": 15, "SIGHUP": 1}, {"SIGTERM": 15}):  # Windows has no SIGHUP
            handled = []
            fake = SimpleNamespace(**numbers, signal=lambda number, _handler: handled.append(number))
            with self.subTest(numbers=numbers), mock.patch.object(parity, "signal", fake):
                parity.exit_on_termination()
                self.assertEqual(handled, list(numbers.values()))

    def test_a_hosts_excludes_are_skipped_when_it_runs_parity_itself(self):
        config = {"hosts": {"playhorizon": {"exclude": {"remote_fireworks": "starved"}}, "macmini": {"exclude": {}}}}
        with mock.patch.object(parity.sys, "platform", "linux"):
            self.assertEqual(parity.platform_skips(config, "playhorizon"), {"remote_fireworks": "starved"})
            self.assertEqual(parity.platform_skips(config, "macmini"), {})
            self.assertEqual(parity.platform_skips(config, "elsewhere"), {})

    def test_named_only_harnesses_are_listed_from_the_host_config(self):
        self.assertEqual(parity.named_only({"named_only": {"lan_integration": "slow"}}), {"lan_integration": "slow"})
        self.assertEqual(parity.named_only({}), {})
        self.assertIn("lan_integration", parity.named_only(parity.host_config()))

    def test_a_pass_from_another_worktree_counts_only_for_the_same_content(self):
        with tempfile.TemporaryDirectory() as temp:
            elsewhere, here, shared = Path(temp, "a"), Path(temp, "b"), Path(temp, "shared")
            for root in (elsewhere, here):
                root.mkdir()
                (root / "x.rs").write_text("same")
            with mock.patch.object(parity, "SHARED", shared):
                with mock.patch.object(parity, "ROOT", elsewhere):
                    record = {"status": "pass", "dependencies": ["x.rs"], "artifacts": []}
                    record["key"] = parity.key(record, parity.Hasher({}), "env")
                    parity.publish("x", record)
                with mock.patch.object(parity, "ROOT", here):
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertFalse(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))
                    self.assertTrue(state["harnesses"]["x"]["shared"])
                    (here / "x.rs").write_text("changed")
                    state = {"harnesses": {}, "hashes": {}}
                    self.assertTrue(parity.stale("x", state, parity.Hasher(state["hashes"]), "env"))


class DependencyPathTests(unittest.TestCase):
    def test_traced_dependencies_use_repository_posix_paths(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp).resolve()):
            source = Path(temp, "crates/w/src/lib.rs")
            source.parent.mkdir(parents=True)
            source.write_text("")
            trace = {"modules": [str(source)], "read": [], "commands": []}
            self.assertEqual(parity.dependencies(trace), (["crates/w/src/lib.rs"], []))

    def test_legacy_local_and_shared_paths_select_without_rewriting_records(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")):
            root = Path(temp)
            found = {name: root / "tools" / f"test_{name}_parity.py" for name in ("local", "shared", "empty")}
            record = {"dependencies": [r"crates\w\src\lib.rs#keep:used"], "key": "recorded-content-key"}
            local = root / "state.json"
            local.write_text(json.dumps({"local": record, "empty": {"dependencies": []}}), encoding="utf-8")
            shared = parity.SHARED / "dependencies/shared.json"
            shared.parent.mkdir(parents=True)
            shared.write_text(json.dumps(record), encoding="utf-8")
            records = json.loads(local.read_text(encoding="utf-8"))
            before = local.read_bytes(), shared.read_bytes()
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/lib.rs"}, reach=False),
                             {"local", "shared"})
            self.assertEqual(parity.affected_harnesses(found, records, {"crates/w/src/other.rs"}, reach=False), set())
            self.assertEqual(records, {"local": record, "empty": {"dependencies": []}})
            self.assertEqual((local.read_bytes(), shared.read_bytes()), before)

    def test_changed_probes_and_oracles_accept_legacy_separators(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            found = {"probe": Path(temp, "tools/test_probe_parity.py")}
            for path in ("crates/w/examples/probe.rs", "crates/w/src/bin/probe.rs",
                         "mods/horizon/src/test/java/Oracle.java"):
                with self.subTest(path=path), mock.patch.object(parity, "changed_files", return_value={path}):
                    records = {"probe": {"dependencies": [path.replace("/", "\\")]}}
                    self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})
            with mock.patch.object(parity, "changed_files", return_value={"crates/w/examples/probe.rs"}):
                records = {"probe": {"dependencies": [r"crates\w\examples\probe.rs#keep:used"]}}
                self.assertEqual(parity.changed_harnesses(found, records, "main"), {"probe"})

    def test_legacy_probe_records_still_narrow_to_reachable_sources(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "SHARED", Path(temp, "shared")), \
                mock.patch.object(parity, "STATE_DIR", Path(temp, "state")):
            root = Path(temp)
            files = {
                "crates/horizon-world/Cargo.toml": '[package]\nname = "horizon-world"\n'
                                                   '\n[[example]]\nname = "world_probe"\npath = "examples/probe.rs"\n',
                "crates/horizon-world/src/lib.rs": "pub mod used;\npub mod unused;\n",
                "crates/horizon-world/src/used.rs": "pub fn f() {}\n",
                "crates/horizon-world/src/unused.rs": "pub fn f() {}\n",
                "crates/horizon-world/examples/probe.rs": "fn main() { horizon_world::used::f(); }\n",
            }
            for path, text in files.items():
                target = root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            dependencies = [path.replace("/", "\\") for path in files]
            dependencies[1] += "#keep:used"
            record = {"dependencies": dependencies, "artifacts": [["example", "probe", "release"]]}
            found = {"probe": root / "tools/test_probe_parity.py"}
            for path, expected in (("crates/horizon-world/src/used.rs", {"probe"}),
                                   ("crates/horizon-world/src/unused.rs", set())):
                with self.subTest(path=path):
                    self.assertEqual(parity.affected_harnesses(found, {"probe": record}, {path}), expected)
            self.assertEqual(record["dependencies"], dependencies)


class RustScopeTests(unittest.TestCase):
    """PARITY_RUST_SCOPE drops Rust sources only when nothing in the scope can reach them."""

    def crate(self, temp, files):
        root = Path(temp)
        (root / "crates/w/src").mkdir(parents=True)
        (root / "crates/w/examples").mkdir(parents=True)
        (root / "crates/w/Cargo.toml").write_text("")
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        return root

    FILES = {
        "crates/w/src/lib.rs": "pub mod a;\npub mod b;\n/// The other one.\npub mod c;\npub const V: i32 = 1;\n",
        "crates/w/src/a.rs": 'pub struct Tag;\npub fn f() -> i32 { let s = "crate::c::x"; crate::V }\n'
                             "mod tests { fn g() { super::super::b::h(); } }\n",
        "crates/w/src/b.rs": "pub fn h() {}\n",
        "crates/w/src/c.rs": "pub struct Other;\nimpl std::fmt::Display for Other { }\nimpl<R: Copy> Foo for R {}\n",
        "crates/w/examples/probe.rs": "use w::a::f;\nfn main() { f(); }\n",
    }

    def narrow(self, files, scope):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", self.crate(temp, files)):
            return parity.narrow_rust_sources(sorted(files), scope)

    def test_a_closed_scope_drops_the_rest(self):
        kept, note = self.narrow(self.FILES, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIsNone(note)
        self.assertEqual(kept, ["crates/w/examples/probe.rs", "crates/w/src/a.rs", "crates/w/src/b.rs",
                                "crates/w/src/lib.rs#keep:a,b"])

    def test_references_out_of_the_scope_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/b.rs": "pub fn h() { crate::c::Other; }\n"})
        kept, note = self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])
        self.assertIn("crate::c::Other", note)
        self.assertEqual(kept, sorted(files))

    def test_impls_elsewhere_for_scope_types_keep_everything(self):
        files = dict(self.FILES, **{"crates/w/src/c.rs": "impl From<i32> for crate::a::Tag { }\n"})
        self.assertIn("Tag", self.narrow(files, ["crates/w/src/a.rs", "crates/w/src/b.rs"])[1])

    def test_new_modules_in_the_crate_root_do_not_count(self):
        keep = {"a", "b"}
        root = self.FILES["crates/w/src/lib.rs"]
        self.assertEqual(parity.crate_root_without_modules(root, keep),
                         parity.crate_root_without_modules(root + "#[cfg(test)]\nmod d;\n", keep))
        self.assertNotEqual(parity.crate_root_without_modules(root, keep),
                            parity.crate_root_without_modules(root + "pub const W: i32 = 2;\n", keep))

    def test_rust_code_blanks_literals_and_comments(self):
        code = parity.rust_code('let a = "{ crate::x }"; // crate::y\nlet c = \'{\'; fn f<\'a>() { r#"}"# }')
        self.assertNotIn("crate", code)
        self.assertEqual(code.count("{"), code.count("}"))
        self.assertIn("<'a>", code)


class ShardTests(unittest.TestCase):
    def test_a_sharded_harness_runs_as_numbered_entries(self):
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            (Path(temp) / "tools").mkdir()
            (Path(temp) / "tools/test_x_parity.py").write_text("PARITY_SHARDS = 3\n")
            (Path(temp) / "tools/test_y_parity.py").write_text("")
            self.assertEqual(sorted(parity.harnesses()), ["x#1", "x#2", "x#3", "y"])
            self.assertEqual(parity.SHARDS["x#2"], "2/3")
            self.assertEqual(parity.base_name("x#2"), "x")


@unittest.skipIf(os.name == "nt", "oracle PATH shimming uses Unix executable symlink semantics")
class OracleReuseTests(unittest.TestCase):
    """tools/parity_shim.py replays a run only when its inputs and outputs are as recorded."""

    def setUp(self):
        import os, subprocess, sys
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.bin, self.shim = root / "bin", root / "shim"
        self.bin.mkdir(); self.shim.mkdir()
        # A fake oracle: copies its input to its output and counts its runs.
        fake = self.bin / "java"
        fake.write_text(f"#!{sys.executable}\nimport sys, pathlib\n"
                        "runs = pathlib.Path(sys.argv[1]).parent / 'runs'\n"
                        "runs.write_text(str(int(runs.read_text() or 0) + 1) if runs.exists() else '1')\n"
                        "pathlib.Path(sys.argv[2]).write_text(pathlib.Path(sys.argv[1]).read_text())\nprint('done')\n")
        fake.chmod(0o755)
        (self.shim / "java").symlink_to(Path(parity.__file__).with_name("parity_shim.py"))
        self.input, self.output = root / "work" / "in.txt", root / "work" / "out.txt"
        self.input.parent.mkdir()
        self.input.write_text("a")
        # The shim starts with `#!/usr/bin/env python3`, and a Python's own folder may name it only
        # python3.13 (Homebrew's): a python3 of its own keeps the PATH below enough.
        python = root / "python"
        python.mkdir()
        (python / "python3").symlink_to(sys.executable)
        self.env = dict(os.environ, PATH=f"{self.shim}:{self.bin}:{python}", PARITY_SHIM_REAL_PATH=str(self.bin),
                        PARITY_SHIM_CACHE=str(root / "cache"))
        self.subprocess = subprocess

    def tearDown(self):
        self.temp.cleanup()

    def oracle(self):
        result = self.subprocess.run(["java", str(self.input), str(self.output)], env=self.env, capture_output=True, text=True,
                                     stdin=self.subprocess.DEVNULL, check=True)
        self.assertEqual(result.stdout, "done\n")
        return int((self.input.parent / "runs").read_text())

    def test_reuses_until_an_input_or_output_changes(self):
        self.assertEqual(self.oracle(), 1)
        self.assertEqual(self.oracle(), 1)  # replayed, stdout included
        self.input.write_text("a")  # same content, new mtime: still reusable
        self.assertEqual(self.oracle(), 1)
        self.input.write_text("b")
        self.assertEqual(self.oracle(), 2)
        self.assertEqual(self.output.read_text(), "b")
        self.output.write_text("tampered")
        self.assertEqual(self.oracle(), 3)
        self.assertEqual(self.output.read_text(), "b")



class LargeOutputTests(unittest.TestCase):
    def test_a_pass_clears_only_its_own_directory(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own, shared = local / "decorated-region-parity", local / "chunk-generation-parity"
            for directory in (own, shared):
                directory.mkdir(parents=True)
                (directory / "dump.txt").write_bytes(b"x" * 16)
            trace = {"commands": [["java", "-d", str(shared / "classes")], ["probe", str(own / "dump.txt")]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8):
                parity.outputs_listing("decorated_region", fresh=True)
                parity.clear_large_outputs("decorated_region", trace)
            self.assertFalse((own / "dump.txt").exists())
            # chunk_generation may still be writing there.
            self.assertTrue((shared / "dump.txt").exists())

    def test_rust_named_directories_and_big_oracle_outputs_go_too(self):
        with tempfile.TemporaryDirectory() as tmp:
            local, state = Path(tmp) / ".local", Path(tmp) / "state"
            own = local / "rust-state-provider-parity"
            own.mkdir(parents=True)
            for name, size in (("rust.txt", 16), ("server.txt", 16), ("client.txt", 64), ("report.json", 4)):
                (own / name).write_bytes(b"x" * size)
            trace = {"commands": [["java", "Oracle", str(own)], ["probe", "fixture", str(own)]]}
            with mock.patch.multiple(parity, LOCAL=local, STATE_DIR=state, LARGE_OUTPUT=8, KEPT_ORACLE_OUTPUT=32):
                listing = parity.outputs_listing("state_provider#2", fresh=True)
                listing.write_text(f"{own / 'server.txt'}\n{own / 'client.txt'}\n")
                parity.clear_large_outputs("state_provider#2", trace)
            self.assertEqual(sorted(path.name for path in own.iterdir()), ["report.json", "server.txt"])


class FakeExecutor:
    """Runs nothing: each start of a harness takes the next status from its script, a list."""
    name, dead, max_peak = "fake", False, None

    def __init__(self):
        self.started, self.finished = [], []

    def free(self):
        return 1

    def accepts(self, name):
        return True

    def fits(self, expected, only_here=True):
        return True

    def start(self, label, script, reuse, expected, shard=None):
        self.started.append((label, reuse))
        self.finished.append((label, {"status": script.pop(0), "seconds": 1.0, "dependencies": []}))
        return True

    def poll(self):
        finished, self.finished = self.finished, []
        return finished


class KnownFlakyTests(unittest.TestCase):
    def run_harnesses(self, scripts, flaky, rounds=None):
        executor = FakeExecutor()
        state = {"harnesses": {name: {"status": "pass", "oracle_reuse": True} for name in scripts}}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.multiple(parity, STATE_DIR=Path(tmp), SHARED=Path(tmp)), \
                mock.patch.object(parity, "save_state"), mock.patch.object(parity, "time") as clock:
            clock.monotonic.return_value = 0.0
            results, _ = parity.run(scripts, state, None, "env", [executor], True, rounds, flaky)
        return results, executor.started

    def test_a_known_flaky_failure_runs_once_more_from_scratch_and_says_so(self):
        results, started = self.run_harnesses({"dedicated_server": ["fail", "pass"]}, {"dedicated_server": "timer"})
        self.assertEqual(results["dedicated_server"]["status"], "pass")
        self.assertEqual(results["dedicated_server"]["retried"], "known flaky")
        # The first run reused oracle results; a known flaky harness doesn't lose that by flaking.
        self.assertTrue(results["dedicated_server"]["oracle_reuse"])
        self.assertEqual(started, [("dedicated_server", True), ("dedicated_server", False)])

    def test_only_once(self):
        results, started = self.run_harnesses({"dedicated_server": ["fail", "fail"]}, {"dedicated_server": "timer"})
        self.assertEqual(results["dedicated_server"]["status"], "fail")
        self.assertEqual(len(started), 2)

    def test_other_harnesses_keep_only_the_reused_oracle_retry(self):
        results, started = self.run_harnesses({"food": ["fail", "pass"]}, {"dedicated_server": "timer"})
        self.assertEqual((results["food"]["status"], results["food"]["retried"]), ("pass", "reused oracle results"))
        self.assertEqual(started, [("food", True), ("food", False)])
        self.assertFalse(results["food"]["oracle_reuse"])

    def test_repeated_runs_get_no_retry(self):
        results, started = self.run_harnesses({"dedicated_server": ["pass", "fail"]}, {"dedicated_server": "timer"},
                                              rounds={"dedicated_server": 2})
        self.assertEqual((results["dedicated_server"]["status"], len(started)), ("fail", 2))

    def test_retries_reach_the_job_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            summary = Path(tmp) / "summary.md"
            with mock.patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": str(summary)}), \
                    mock.patch("builtins.print"):
                parity.report_retries({"dedicated_server": {"status": "pass", "retried": "known flaky"},
                                       "food": {"status": "pass"}}, {"dedicated_server": "timer"})
            text = summary.read_text()
        self.assertIn("RETRIED dedicated_server: failed, then passed on its one retry (known flaky: timer", text)
        self.assertNotIn("food", text)

    def test_the_known_flaky_list_has_a_reason_per_harness(self):
        flaky = parity.known_flaky(parity.host_config())
        self.assertTrue(all(isinstance(reason, str) and len(reason) > 20 for reason in flaky.values()))
        self.assertTrue(set(flaky) <= {parity.base_name(name) for name in parity.harnesses()})



class SoloTests(unittest.TestCase):
    """Harnesses driving game windows run alone on a small machine (playhorizon's 4-CPU runner)."""

    def executor(self, running):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(parity, "STATE_DIR", Path(tmp)):
            local = parity.LocalExecutor(4, 1 << 40, 60)
        local.solo = {"remote_fireworks"}
        local.running = {label: {} for label in running}
        return local

    def test_a_solo_harness_waits_for_an_empty_machine_and_then_holds_it(self):
        busy = self.executor(["food"])
        self.assertFalse(busy.accepts("remote_fireworks"))
        self.assertTrue(busy.accepts("light"))
        self.assertEqual(busy.free(), 3)
        self.assertTrue(self.executor([]).accepts("remote_fireworks"))
        alone = self.executor(["remote_fireworks~2"])
        self.assertEqual(alone.free(), 0)

    def test_only_small_machines_and_listed_harnesses(self):
        config = {"solo": {"remote_fireworks": "two game windows"}}
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "1"}):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        with mock.patch.dict(os.environ, {"PARITY_SOLO": "0"}):
            self.assertEqual(parity.solo_harnesses(config), set())
        with mock.patch.dict(os.environ, {"PARITY_SOLO": ""}), mock.patch.object(parity, "SOLO_CPUS", 10 ** 6):
            self.assertEqual(parity.solo_harnesses(config), {"remote_fireworks"})
        solo = parity.host_config().get("solo", {})
        self.assertTrue(solo and set(solo) <= {parity.base_name(name) for name in parity.harnesses()})


if __name__ == "__main__":
    unittest.main()


class WorkspaceManifestTests(unittest.TestCase):
    """A crate split's Cargo.toml and Cargo.lock changes don't select every harness; a dependency change still does."""
    CLIENT = '[package]\nname = "horizon-client"\n\n[dependencies]\nserde = "1"\nwgpu = { version = "27", features = ["x"] }\n'
    LOCK = ('version = 4\n\n[[package]]\nname = "horizon-client"\nversion = "0.1.0"\ndependencies = [\n "serde",\n]\n\n'
            '[[package]]\nname = "serde"\nversion = "1.0.1"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n')

    def verdict(self, base, head):
        """base, head: {path: text or None}; the manifests workspace_only_manifests leaves out."""
        with tempfile.TemporaryDirectory() as temp, \
                mock.patch.object(parity, "ROOT", Path(temp)), \
                mock.patch.object(parity, "git_show", lambda _base, path: base.get(path)):
            for path, text in head.items():
                if text is not None:
                    Path(temp, path).parent.mkdir(parents=True, exist_ok=True)
                    Path(temp, path).write_text(text)
            return parity.workspace_only_manifests(set(head) | set(base), "base")

    def test_a_crate_split_is_left_out(self):
        command = '[package]\nname = "horizon-command"\n\n[dependencies]\nserde = "1"\nhorizon-core = { path = "../horizon-core" }\n'
        client = self.CLIENT + 'horizon-command = { path = "../horizon-command" }\n'
        lock = self.LOCK.replace('[[package]]\nname = "serde"', '[[package]]\nname = "horizon-command"\nversion = "0.1.0"\n\n'
                                 '[[package]]\nname = "serde"')
        base = {"Cargo.toml": '[workspace]\nmembers = ["crates/horizon-client"]\nresolver = "2"\n',
                "crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        head = {"Cargo.toml": '[workspace]\nmembers = ["crates/horizon-client", "crates/horizon-command"]\nresolver = "2"\n',
                "crates/horizon-client/Cargo.toml": client, "crates/horizon-command/Cargo.toml": command, "Cargo.lock": lock}
        self.assertEqual(self.verdict(base, head), set(head))

    def test_a_dependency_change_still_counts(self):
        base = {"crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK}
        bumped = {"crates/horizon-client/Cargo.toml": self.CLIENT, "Cargo.lock": self.LOCK.replace("1.0.1", "1.0.2")}
        self.assertEqual(self.verdict(base, bumped), set())
        feature = {"crates/horizon-client/Cargo.toml": self.CLIENT.replace('["x"]', '["x", "y"]')}
        self.assertEqual(self.verdict({"crates/horizon-client/Cargo.toml": self.CLIENT}, feature), set())
        resolver = {"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "3"\n'}
        self.assertEqual(self.verdict({"Cargo.toml": '[workspace]\nmembers = ["a"]\nresolver = "2"\n'}, resolver), set())

    def test_a_feature_of_a_workspace_crate_counts_through_its_sources(self):
        items = '[package]\nname = "horizon-items"\n\n[features]\ntest-support = []\n'
        client = self.CLIENT + '\n[dev-dependencies]\nhorizon-items = { path = "../horizon-items", features = ["test-support"] }\n'
        head = {"crates/horizon-items/Cargo.toml": items, "crates/horizon-client/Cargo.toml": client,
                "crates/horizon-items/src/lib.rs": '#[cfg(feature = "test-support")]\npub fn for_tests() {}\n'}
        base = {"crates/horizon-client/Cargo.toml": self.CLIENT}
        # The new crate's files are all changed, the code the feature gates with them.
        self.assertEqual(self.verdict(base, head), {"crates/horizon-items/Cargo.toml", "crates/horizon-client/Cargo.toml"})
        # Turning on a feature of a crate whose gated code didn't change selects as before.
        world = '[package]\nname = "horizon-world"\n\n[features]\nextra = ["more"]\nmore = []\n'
        with tempfile.TemporaryDirectory() as temp, mock.patch.object(parity, "ROOT", Path(temp)):
            Path(temp, "crates/horizon-world/src").mkdir(parents=True)
            Path(temp, "crates/horizon-world/Cargo.toml").write_text(world)
            Path(temp, "crates/horizon-world/src/lib.rs").write_text('#[cfg(feature = "more")]\nfn x() {}\n')
            self.assertFalse(parity.feature_sources_changed("horizon-world", "extra", {"crates/horizon-client/Cargo.toml"}))
            self.assertTrue(parity.feature_sources_changed("horizon-world", "extra", {"crates/horizon-world/src/lib.rs"}))
            self.assertFalse(parity.feature_sources_changed("horizon-nothing", "extra", set()))
        # A feature that reaches an external package counts.
        optional = {"crates/horizon-client/Cargo.toml": self.CLIENT + '\n[features]\nfast = ["wgpu/x"]\n'}
        self.assertEqual(self.verdict({"crates/horizon-client/Cargo.toml": self.CLIENT}, optional), set())

    def test_without_a_base_everything_counts(self):
        self.assertEqual(parity.workspace_only_manifests({"Cargo.lock"}, None), set())
