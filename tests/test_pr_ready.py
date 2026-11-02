import argparse
import datetime
import json
import unittest
from unittest import mock

from tools import pr_ready

PACKAGES = {"horizon-core": "crates/horizon-core", "horizon-client": "crates/horizon-client",
            "horizon-theme": "crates/horizon-theme", "xtask": "xtask"}
LIBRARIES = set(PACKAGES) - {"xtask"}
DEPENDENCIES = {"horizon-client": {"horizon-core"}}
MODS = ""
CLIPPY = pr_ready.CI_CLIPPY + " && " + pr_ready.CI_CLIPPY_SERVER


def args(host="local", quick=False):
    return argparse.Namespace(host=host, quick=quick)


class PrReadyTests(unittest.TestCase):
    def test_paths_map_to_the_crates_that_own_them(self):
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/src/jmath.rs", "crates/horizon-client/build.rs",
                                                  "xtask/src/main.rs", "tools/parity.py"}, PACKAGES),
                         ["horizon-client", "horizon-core", "xtask"])
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core/README.md"}, PACKAGES), [])
        # The generated stylesheet is checked by horizon-theme's tests.
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-theme/tokens.css"}, PACKAGES), ["horizon-theme"])
        # A crate whose name prefixes another's directory doesn't own it.
        self.assertEqual(pr_ready.touched_crates({"crates/horizon-core-extra/x.rs"}, PACKAGES), [])
        # The workspace manifest and lock touch every crate.
        self.assertIsNone(pr_ready.touched_crates({"Cargo.lock", "crates/horizon-core/src/lib.rs"}, PACKAGES))

    def test_shared_client_sources_select_the_library_without_gameplay(self):
        packages = {**PACKAGES, "horizon-client-lib": "crates/horizon-client-lib"}
        for path in ["src/client_lib.rs", "src/health.rs", "src/lan/hub.rs", "src/bin/horizon-ctl/main.rs",
                     "examples/probe/main.rs", "tests/lan_bot.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/horizon-client/{path}"}, packages),
                             ["horizon-client-lib"])
        for path in ["src/lib.rs", "src/main.rs", "src/entry.rs", "src/game.rs", "src/game/combat.rs",
                     "tests/launch_api.rs", "Cargo.toml", "build.rs"]:
            self.assertEqual(pr_ready.touched_crates({f"crates/horizon-client/{path}"}, packages),
                             ["horizon-client"])

    def test_which_workspace_builds_a_path(self):
        self.assertTrue(pr_ready.workspace_rust("crates/horizon-theme/fonts/x.ttf"))
        self.assertTrue(pr_ready.workspace_rust("Cargo.toml"))
        self.assertFalse(pr_ready.workspace_rust("crates/horizon-client/README.md"))
        self.assertFalse(pr_ready.workspace_rust("launcher/src/main.rs"))
        self.assertTrue(pr_ready.launcher_rust("launcher/src/main.rs"))
        # CI's launcher job also runs for the root workspace's manifests and for its own check.
        self.assertTrue(pr_ready.launcher_rust("Cargo.lock"))
        self.assertTrue(pr_ready.launcher_rust("tools/ci/rust_changes.py"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-prepare/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-theme/src/lib.rs"))
        self.assertTrue(pr_ready.launcher_rust("crates/horizon-telemetry/src/lib.rs"))
        self.assertFalse(pr_ready.launcher_rust("launcher/links.json"))
        self.assertFalse(pr_ready.launcher_rust("launcher/README.md"))
        self.assertFalse(pr_ready.launcher_rust("crates/horizon-client/src/game.rs"))

    def test_crate_checks_do_not_link_unrelated_examples(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-client/src/game.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        self.assertEqual(by_name["tests: horizon-client"].command,
                         "cargo test --locked -p horizon-client --tests")
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p horizon-client --doc")
        self.assertIsNone(by_name["doc tests"].status)

    def test_content_root_tests_run_only_when_what_they_load_changes(self):
        def content_step(path):
            with mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
            return {step.name: step for step in steps}["content root tests"]
        self.assertEqual(content_step("crates/horizon-client/src/game.rs").skip, "content root untouched")
        for path in ["crates/horizon-client/src/mod_content.rs", "crates/horizon-client/src/mod_content_tests.rs",
                     "crates/horizon-mods/src/content.rs", "crates/horizon-assets/src/lib.rs", "Cargo.lock"]:
            with self.subTest(path=path):
                step = content_step(path)
                self.assertIsNone(step.skip)
                self.assertEqual(step.command, pr_ready.CONTENT_ROOT_TESTS)

    def test_shared_rust_configuration_keeps_workspace_coverage(self):
        for path in sorted(pr_ready.ROOT_RUST | {".cargo/config.toml", "crates/unknown/src/lib.rs"}):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                self.assertEqual(by_name["clippy"].command, CLIPPY)
                self.assertEqual(by_name["tests: workspace"].command,
                                 "cargo test --locked --workspace" + MODS + " --tests")
                self.assertEqual(by_name["doc tests"].command, "cargo test --locked --workspace --doc")

    def test_tests_get_the_features_ci_s_workspace_run_gives_them(self):
        self.assertEqual(pr_ready.mod_features(["horizon-world"]), "")
        self.assertEqual(pr_ready.mod_features(["horizon-client"]), "")
        self.assertEqual(pr_ready.mod_features(["horizon-mods"]),
                         " --features horizon-mods/index")
        self.assertEqual(pr_ready.mod_features(None), MODS)

    def test_mod_api_check_tracks_hook_inputs(self):
        for path in ("xtask/src/mod_api.rs", "native-hooks.json",
                     "crates/horizon-client/src/game/native_hooks.rs", "README.md"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                step = {step.name: step for step in steps}["mod API up to date"]
                self.assertEqual(step.command, "cargo test --locked -p xtask mod_api")
                self.assertEqual(step.skip, "native hooks untouched" if path == "README.md" else None)

    def test_python_tests_always_run(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-core/src/lib.rs"},
                                  PACKAGES, LIBRARIES, DEPENDENCIES)
        self.assertIsNone({step.name: step for step in steps}["Python tests"].status)

    def test_non_rust_changes_skip_workspace_checks(self):
        for path in ("README.md", "tools/pr_ready.py", "launcher/src/main.rs"):
            with self.subTest(path=path), mock.patch.object(pr_ready, "affected_here", return_value=[]):
                steps = pr_ready.plan(args(quick=True), "origin/main", {path}, PACKAGES, LIBRARIES, DEPENDENCIES)
                by_name = {step.name: step for step in steps}
                for name in ("clippy", "tests", "doc tests"):
                    self.assertEqual(by_name[name].status, "skip")

    def test_multiple_touched_crates_share_the_same_check_scope(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"crates/horizon-core/src/lib.rs", "crates/horizon-client/examples/probe.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        for step in steps:
            if step.name in ("tests: horizon-client, horizon-core", "doc tests"):
                self.assertIn("-p horizon-client -p horizon-core", step.command)
                self.assertIsNone(step.status)

    def test_binary_only_crates_skip_doc_tests(self):
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"xtask/src/main.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["tests: xtask"].command, "cargo test --locked -p xtask --tests")
        self.assertIsNone(by_name["tests: xtask"].status)
        self.assertEqual(by_name["doc tests"].status, "skip")
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main",
                                  {"xtask/src/main.rs", "crates/horizon-core/src/lib.rs"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p horizon-core --doc")

    def test_clippy_is_ci_s_and_the_server_build_when_the_game_is_affected(self):
        packages = {name: f"crates/{name}" for name in ("core", "world", "horizon-client", "unrelated")}
        dependencies = {"world": {"core"}, "horizon-client": {"world"}, "unrelated": set()}
        with mock.patch.object(pr_ready, "affected_here", return_value=[]):
            steps = pr_ready.plan(args(), "origin/main", {"crates/core/src/lib.rs"},
                                  packages, set(packages), dependencies)
            unrelated = pr_ready.plan(args(), "origin/main", {"crates/unrelated/src/lib.rs"},
                                      packages, set(packages), dependencies)
        self.assertEqual({step.name: step for step in unrelated}["clippy"].command, pr_ready.CI_CLIPPY)
        by_name = {step.name: step for step in steps}
        self.assertEqual(by_name["clippy"].command, CLIPPY)
        self.assertEqual(by_name["tests: core"].command, "cargo test --locked -p core --tests")
        self.assertEqual(by_name["doc tests"].command, "cargo test --locked -p core --doc")

    def test_dependent_scope_handles_cycles_and_multiple_changed_crates(self):
        dependencies = {"a": {"b"}, "b": {"a"}, "c": {"b"}, "d": set()}
        self.assertEqual(pr_ready.dependent_crates(["a"], dependencies), ["a", "b", "c"])
        self.assertEqual(pr_ready.dependent_crates(["a", "d"], dependencies), ["a", "b", "c", "d"])
        self.assertEqual(pr_ready.dependent_crates([], dependencies), [])
        self.assertIsNone(pr_ready.dependent_crates(None, dependencies))

    def test_metadata_includes_renamed_optional_platform_build_and_dev_consumers(self):
        core = str(pr_ready.ROOT / "crates/core")
        metadata = {"workspace_members": ["core", "optional", "build", "dev"], "packages": [
            {"id": "core", "name": "core", "manifest_path": core + "/Cargo.toml",
             "targets": [{"kind": ["lib"]}], "dependencies": []},
        ]}
        for name, kind in (("optional", None), ("build", "build"), ("dev", "dev")):
            metadata["packages"].append({
                "id": name, "name": name, "manifest_path": str(pr_ready.ROOT / f"crates/{name}/Cargo.toml"),
                "targets": [{"kind": ["bin"]}], "dependencies": [
                    {"name": "core", "rename": "alias", "path": core, "kind": kind,
                     "optional": name == "optional", "target": "cfg(windows)"},
                    {"name": "external", "path": None},
                ],
            })
        # A path dependency outside this workspace must not become a selectable package.
        metadata["packages"].append({"id": "outside", "name": "outside", "manifest_path": "/outside/Cargo.toml"})
        with mock.patch.object(pr_ready.subprocess, "run", return_value=mock.Mock(stdout=json.dumps(metadata))):
            directories, libraries, dependencies = pr_ready.packages()
        self.assertEqual(set(directories), {"core", "optional", "build", "dev"})
        self.assertEqual(libraries, {"core"})
        self.assertEqual(pr_ready.dependent_crates(["core"], dependencies), ["build", "core", "dev", "optional"])

    def test_scope_warns_on_size_and_mixed_areas(self):
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-client/src/game.rs", "tools/test_food_parity.py",
                                                  "mods/horizon/src/test/java/dev/horizonlab/parity/X.java",
                                                  "docs/native-client.md"}, 120), [])
        self.assertEqual(pr_ready.scope_warnings({"crates/horizon-theme/src/lib.rs", "launcher/src/theme.rs"}, 10), [])
        warnings = pr_ready.scope_warnings({"crates/horizon-client/src/game.rs", "launcher/src/main.rs",
                                            ".github/workflows/ci.yml"}, 900)
        self.assertEqual(len(warnings), 2)
        self.assertIn("900 lines of code: over 500", warnings[0])
        self.assertEqual(warnings[1], "mixes CI, game, launcher")

    def test_documentation_and_tests_are_not_counted_in_the_size(self):
        self.assertTrue(pr_ready.reviewed("tools/pr_ready.py"))
        self.assertTrue(pr_ready.reviewed("crates/horizon-client/src/game.rs"))
        self.assertFalse(pr_ready.reviewed("CONTRIBUTING.md"))
        self.assertFalse(pr_ready.reviewed("tests/test_pr_ready.py"))
        self.assertFalse(pr_ready.reviewed("tools/test_food_parity.py"))
        self.assertFalse(pr_ready.reviewed("crates/horizon-core/tests/jmath.rs"))

    def test_cargo_lock_changes_only_with_a_manifest(self):
        self.assertIsNone(pr_ready.lock_problem({"crates/horizon-core/src/lib.rs"}))
        self.assertIsNone(pr_ready.lock_problem({"Cargo.lock", "crates/horizon-client/Cargo.toml"}))
        self.assertIsNone(pr_ready.lock_problem({"launcher/Cargo.lock", "crates/horizon-theme/Cargo.toml"}))
        self.assertIn("Cargo.lock", pr_ready.lock_problem({"Cargo.lock", "crates/horizon-core/src/lib.rs"}))

    def test_label_follows_what_players_get(self):
        self.assertEqual(pr_ready.suggested_label({"tools/pr_ready.py", "CONTRIBUTING.md"}), "`skip-release`")
        self.assertEqual(pr_ready.suggested_label({"crates/horizon-world/examples/probe.rs"}), "`skip-release`")
        self.assertIn("feature", pr_ready.suggested_label({"crates/horizon-client/src/game.rs"}))
        self.assertIn("feature", pr_ready.suggested_label({"launcher/src/main.rs"}))

    def test_affected_harnesses_come_from_the_parity_runner(self):
        found = {"food": "tools/test_food_parity.py", "datafix#1": "tools/test_datafix_parity.py",
                 "datafix#2": "tools/test_datafix_parity.py"}
        with mock.patch.object(pr_ready.parity, "harnesses", return_value=found), \
                mock.patch.object(pr_ready.parity, "load_state", return_value={"harnesses": {
                    "food": {"dependencies": ["crates/horizon-world/src/food.rs"]},
                    "datafix#1": {"dependencies": ["crates/horizon-world/src/datafix.rs"]},
                    "datafix#2": {"dependencies": ["crates/horizon-world/src/datafix.rs"]}}}), \
                mock.patch.object(pr_ready.parity, "ROOT", pr_ready.parity.Path(".")):
            self.assertEqual(pr_ready.affected_here({"crates/horizon-world/src/datafix.rs"}), ["datafix"])
            self.assertEqual(pr_ready.affected_here({"README.md"}), [])
            self.assertEqual(pr_ready.affected_here({"Cargo.lock"}), ["food", "datafix"])

    def test_harnesses_are_listed_when_they_cannot_run(self):
        changed = {"crates/horizon-world/src/food.rs"}
        with mock.patch.object(pr_ready, "affected_here", return_value=["food", "light"]):
            steps = pr_ready.plan(args(quick=True), "origin/main", changed, {"horizon-world": "crates/horizon-world"}, {"horizon-world"}, {})
        harness = steps[-1]
        self.assertEqual(harness.status, "list")
        self.assertIn("2 affected, to run in CI (--quick): food, light", harness.detail)
        by_name = {step.name: step for step in steps}
        self.assertIn("tests: horizon-world", by_name)
        self.assertEqual(by_name["launcher clippy"].status, "skip")
        self.assertIsNone(by_name["clippy"].status)
        with mock.patch.object(pr_ready, "affected_here", return_value=["food"]):
            steps = pr_ready.plan(args(host="macmini"), "origin/main", changed, {"horizon-world": "crates/horizon-world"}, {"horizon-world"}, {})
        self.assertIn("--changed-files", steps[-1].command)
        self.assertIn("crates/horizon-world/src/food.rs", steps[-1].command)

    def test_ci_s_rust_follows_the_six_week_train(self):
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 2, 20)), 85)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2025, 6, 26)), 88)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 10, 31)), 99)
        self.assertEqual(pr_ready.ci_rust(datetime.date(2026, 11, 12)), 100)

    def test_an_older_rust_warns_without_failing(self):
        today = datetime.date(2026, 10, 31)
        self.assertIsNone(pr_ready.old_rust((1, 99, 1), today))
        self.assertIsNone(pr_ready.old_rust(None, today))
        warning = pr_ready.old_rust((1, 97, 0), today)
        self.assertIn("1.97.0 is older than CI's stable (1.99)", warning)
        self.assertIn("`rustup update`", warning)
        steps = pr_ready.plan(args(quick=True), "origin/main", {"crates/horizon-core/src/jmath.rs"}, PACKAGES, LIBRARIES,
                              DEPENDENCIES)
        by_name = {step.name: step for step in steps}
        by_name["clippy"].status, by_name["clippy"].detail = "fail", "failed"
        pr_ready.mark_old_rust(steps, warning)
        names = [step.name for step in steps]
        self.assertEqual(names.index("Rust") + 1, names.index("clippy"))
        self.assertEqual(steps[names.index("Rust")].status, "warn")
        self.assertEqual(by_name["clippy"].detail, "failed; likely the old Rust above")
        steps = pr_ready.plan(args(quick=True), "origin/main", {"README.md"}, PACKAGES, LIBRARIES, DEPENDENCIES)
        pr_ready.mark_old_rust(steps, warning)
        self.assertNotIn("Rust", [step.name for step in steps])

    def test_summaries_of_check_output(self):
        output = "test result: ok. 3 passed; 0 failed;\ntest result: FAILED. 4 passed; 1 failed;\n"
        self.assertEqual(pr_ready.tests_passed(output), "7 passed, 1 failed")
        self.assertEqual(pr_ready.unittest_passed("....\nRan 12 tests in 1.0s\n\nOK\n"), "12 passed")
        self.assertEqual(pr_ready.unittest_passed("Ran 12 tests in 1.0s\n\nFAILED (failures=2)\n"),
                         "12 run, FAILED (failures=2)")
        parity_output = ("3 files changed: 2 harnesses depend on them, 2 without a pass for their current inputs.\n"
                         "\n1 passed, 1 failed in 2.0 min.\n  light: /x/.local/parity-runner/logs/light.log\n")
        self.assertEqual(pr_ready.parity_summary(parity_output), "2 affected, 1 passed, failed: light")
        self.assertEqual(pr_ready.parity_summary("2 files changed: 0 harnesses depend on them, 0 without a pass for "
                                                 "their current inputs.\nNothing to run: every harness passed."),
                         "no harness affected")
        self.assertEqual(pr_ready.duration(125), "2 min 5 s")


if __name__ == "__main__":
    unittest.main()
