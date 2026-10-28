from pathlib import Path
import tempfile
import unittest

from tools import port_harness as harness

JAVAC = """/x/mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java:45: error: cannot find symbol
  symbol:   variable WORLD
  location: class RegistryLayer
1 error
"""


def state(**statuses):
    return {"harnesses": {name.replace("_1", "#1"): {"status": status} for name, status in statuses.items()}}


class ClassifyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.logs = Path(self.directory.name)
        (self.logs / "food.log").write_text(JAVAC)
        (self.logs / "food_extra.log").write_text("not food's log")
        (self.logs / "light.log").write_text("first difference at (3, 64, 2)")

    def tearDown(self):
        self.directory.cleanup()

    def test_pass_oracle_fail_and_new(self):
        now = state(food="fail", light="fail", mob_ai="fail", move="pass", move_1="fail")
        baseline = state(food="pass", light="pass", mob_ai="fail", move="pass")
        found = harness.classify(now, self.logs, baseline)
        self.assertEqual(found["food"], {"status": "oracle", "baseline": "pass", "new": True, "compile_errors": [
            "ParityData.java:45: cannot find symbol: variable WORLD (class RegistryLayer)"]})
        self.assertEqual(found["light"], {"status": "fail", "baseline": "pass", "new": True})
        self.assertEqual(found["mob_ai"], {"status": "fail", "baseline": "fail", "new": False})
        self.assertEqual(found["move"]["status"], "fail")  # one shard failed
        self.assertEqual(harness.summary(found), {"pass": 0, "oracle": 1, "fail": 3, "new": 3, "known": 1})

    def test_without_a_baseline_every_failure_is_new(self):
        found = harness.classify(state(light="fail", food="pass"), self.logs)
        self.assertTrue(found["light"]["new"])
        self.assertFalse(found["food"]["new"])

    def test_named_harnesses_only(self):
        self.assertEqual(list(harness.classify(state(light="fail", food="pass"), self.logs, names=["food"])), ["food"])

    def test_logs_of_one_harness(self):
        (self.logs / "food#2.log").write_text("")
        self.assertEqual([p.name for p in harness.harness_logs(self.logs, "food")], ["food#2.log", "food.log"])


class AssetTests(unittest.TestCase):
    def test_javac_errors_from_the_log_horizon_prepare_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "export.log"
            log.write_text("Oracle.java:33: error: cannot find symbol\n")
            output = f"error: javac failed (exit status: 1) while exporting block states; see {log}\n"
            self.assertIn("Oracle.java:33: error", harness.named_logs(output))
            self.assertEqual(harness.named_logs("error: see /nonexistent/x.log"), "")

    def test_the_reports_link_goes_before_horizon_prepare(self):
        with tempfile.TemporaryDirectory() as tmp:
            root, store = Path(tmp) / "candidate", Path(tmp) / "store-reports"
            (store / "minecraft").mkdir(parents=True)
            reports = root / ".local" / "datagen" / "out" / "reports"
            reports.parent.mkdir(parents=True)
            try:
                reports.symlink_to(store)
            except OSError as e:
                self.skipTest(f"this host cannot create symlinks: {e}")
            harness.own_reports(root)
            self.assertFalse(reports.exists() or reports.is_symlink())
            self.assertTrue((store / "minecraft").is_dir())
            reports.mkdir()  # its own reports, from an earlier horizon-prepare, stay
            harness.own_reports(root)
            self.assertTrue(reports.is_dir())

    def test_sounds_and_panorama_only(self):
        self.assertTrue(harness.wanted_asset("minecraft/sounds.json"))
        self.assertTrue(harness.wanted_asset("minecraft/sounds/mob/cow/say1.ogg"))
        self.assertTrue(harness.wanted_asset("minecraft/textures/gui/title/background/panorama_overlay.png"))
        self.assertFalse(harness.wanted_asset("minecraft/lang/fr_fr.json"))


if __name__ == "__main__":
    unittest.main()
