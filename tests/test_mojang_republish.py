import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools" / "ci"))
import mojang_republish as mr  # noqa: E402

LOCK = {"minecraft": {
    "version": "26.3", "manifest_sha1": "aaaa1111", "accepted_manifest_sha1": ["bbbb2222"],
    "accepted_asset_index_sha1": ["a1a1a1a1"], "client_sha1": "c1", "server_sha1": "s1", "java_major": 25}}


def manifest(sha1):
    return {"versions": [{"id": "25.0", "sha1": "zzzz", "url": "u0"},
                         {"id": "26.3", "sha1": sha1, "url": f"https://x/{sha1}.json"}]}


def detail(client="c1", server="s1", java=25, asset="new11111"):
    return {"downloads": {"client": {"sha1": client}, "server": {"sha1": server}},
            "javaVersion": {"majorVersion": java}, "assetIndex": {"sha1": asset}}


class Plan(unittest.TestCase):
    def run_plan(self, sha1, info=None, issues=None):
        def fetch(url):
            self.assertIsNotNone(info, "known manifests must not be downloaded")
            return info
        return mr.plan(copy.deepcopy(LOCK), manifest(sha1), fetch, issues or {})

    def test_pinned_and_accepted_are_quiet(self):
        self.assertIsNone(self.run_plan("aaaa1111"))
        self.assertIsNone(self.run_plan("bbbb2222"))

    def test_unknown_with_same_jars_opens_an_issue_with_a_snippet(self):
        action = self.run_plan("cccc3333", detail())
        self.assertEqual(action["kind"], "open")
        self.assertEqual(action["title"], "Mojang republished 26.3: cccc3333")
        self.assertTrue(action["jars_unchanged"])
        self.assertIn("Jars unchanged", action["body"])
        self.assertIn("NOT checked", action["body"])
        for text in ('"cccc3333"', '"bbbb2222"', '"new11111"', '"a1a1a1a1"', "accepted_asset_index_sha1"):
            self.assertIn(text, action["body"])

    def test_unknown_with_different_jars_says_do_not_accept(self):
        for info, key in ((detail(client="c2"), "client_sha1"), (detail(server="s2"), "server_sha1"),
                          (detail(java=26), "java_major")):
            action = self.run_plan("cccc3333", info)
            self.assertFalse(action["jars_unchanged"])
            self.assertIn("JARS CHANGED", action["body"])
            self.assertIn(key, action["body"])
            self.assertNotIn("```json", action["body"])

    def test_existing_issue_is_skipped(self):
        issues = {"Mojang republished 26.3: cccc3333": 7, "Mojang republished 26.3: dddd4444": 8}
        action = self.run_plan("cccc3333", detail(), issues)
        self.assertEqual((action["kind"], action["number"]), ("skip", 7))

    def test_missing_version_is_an_error(self):
        with self.assertRaises(ValueError):
            mr.plan(LOCK, {"versions": []}, None, {})

    def test_accepted_matches_lab(self):
        sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
        import lab
        for sha1 in ("aaaa1111", "bbbb2222", "cccc3333"):
            self.assertEqual(mr.accepted(LOCK["minecraft"], sha1), lab.manifest_accepted(LOCK, sha1))

    def test_open_issues_filters_by_title_prefix(self):
        listed = '[{"number":1,"title":"Mojang republished 26.3: cccc3333"},{"number":2,"title":"Other"}]'
        self.assertEqual(mr.open_issues_titled("26.3", runner=lambda *a, **k: listed),
                         {"Mojang republished 26.3: cccc3333": 1})


if __name__ == "__main__":
    unittest.main()
