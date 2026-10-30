import copy
import datetime
import json
import tomllib
import unittest
from unittest import mock

from tools import roadmap

SAMPLE = '''[overview]
today_plain = "a"
today_dev = "b"
help_plain = "c"
help_dev = "d"

[[area]]
id = "performance"
name = "Performance"

# ---- Now

[[item]]
id = "fast"
title = "Fast"
status = "now"
area = "performance"
owner = "some-dev"
issue = "#20"
focus = true
plain = "Faster."
dev = """Multiply, \\
not divide (#10)."""
links = ["#10"]

[[item]]
id = "faster"
title = "Faster"
status = "now"
area = "performance"
owner = "some-dev"
issue = "#21"
focus = true
plain = "Even faster."
dev = "More (#11)."
links = ["#11", "#12"]

[[item]]
id = "fastest"
title = "Fastest"
status = "now"
area = "performance"
owner = "some-dev"
issue = "#22"
focus = true
ongoing = true
plain = "The fastest."
dev = "Most (#13)."
links = ["#13"]

# ---- Shipped

[[item]]
id = "old"
title = "Old"
status = "shipped"
shipped_in = "v0.2.1"
area = "performance"
plain = "Done."
dev = "Done (#1)."
links = ["#1"]
'''


def pull(state, merged_at=None, paths=("crates/horizon-client/src/game.rs",)):
    return {"kind": "PullRequest", "state": state, "merged_at": merged_at, "merge_commit": "abc" if merged_at else None,
            "paths": list(paths)}


def issue(state):
    return {"kind": "Issue", "state": state, "merged_at": None, "merge_commit": None, "paths": []}


class RepositoryRoadmap(unittest.TestCase):
    def test_the_roadmap_keeps_its_rules(self):
        self.assertEqual(roadmap.validate(roadmap.load()), [])

    def test_generated_files_match_the_toml(self):
        for path, text in roadmap.outputs(roadmap.load()).items():
            self.assertEqual(path.read_text(encoding="utf-8"), text, f"{path.name} is out of date: run `python3 tools/roadmap.py build`")

    def test_site_data_has_every_item_in_one_line_text(self):
        data = json.loads(roadmap.site_data(tomllib.loads(SAMPLE)))
        self.assertEqual([item["id"] for item in data["items"]], ["fast", "faster", "fastest", "old"])
        self.assertEqual(data["items"][0]["dev"], "Multiply, not divide (#10).")
        self.assertEqual(data["areas"], [{"id": "performance", "name": "Performance"}])


class Rules(unittest.TestCase):
    def setUp(self):
        self.data = tomllib.loads(SAMPLE)

    def problems(self, **changes):
        data = copy.deepcopy(self.data)
        data["item"][0].update(changes)
        return roadmap.validate(data)

    def test_the_sample_is_valid(self):
        self.assertEqual(roadmap.validate(self.data), [])

    def test_unknown_status_area_and_key(self):
        self.assertTrue(any("status" in p for p in self.problems(status="soon")))
        self.assertTrue(any("area" in p for p in self.problems(area="nowhere")))
        self.assertTrue(any("unknown key" in p for p in self.problems(eta="soon")))

    def test_plain_fits_a_post(self):
        self.assertTrue(any("at most 240" in p for p in self.problems(plain="x" * 241)))
        self.assertTrue(any("post" in p for p in self.problems(post="x" * 241)))

    def test_shipped_needs_its_release(self):
        self.assertTrue(any("shipped_in" in p for p in self.problems(status="shipped", focus=False)))
        self.assertTrue(any("only shipped" in p for p in self.problems(shipped_in="v0.2.3")))
        self.assertTrue(any("not a release" in p for p in self.problems(status="shipped", focus=False,
                                                                           shipped_in="soon")))

    def test_links_are_issues_urls_or_repository_paths(self):
        self.assertEqual(self.problems(links=["#4", "https://playhorizon.dev", "docs/benchmark.md"]), [])
        self.assertTrue(any("link" in p for p in self.problems(links=["docs/nothing-here.md"])))
        self.assertTrue(any("link" in p for p in self.problems(links=["http://example.com"])))

    def test_ids_are_free_anchors_of_the_hub(self):
        for taken in ("roadmap", "now", "h-next", "tab-board"):
            self.assertTrue(any("taken by the hub page" in p for p in self.problems(id=taken)), taken)

    def test_flags_are_true_or_false(self):
        self.assertEqual(self.problems(announce=True), [])
        self.assertTrue(any("announce is true or false" in p for p in self.problems(announce="yes")))

    def test_now_items_have_an_owner_and_a_tracking_issue(self):
        data = tomllib.loads(SAMPLE)
        del data["item"][0]["owner"]
        data["item"][1]["issue"] = "3859"
        data["item"][2]["owner"] = "@not a login"
        problems = roadmap.validate(data)
        self.assertIn("item fast: a Now item needs an owner and a tracking issue", problems)
        self.assertTrue(any(p.startswith("item faster: issue is the tracking issue") for p in problems))
        self.assertTrue(any(p.startswith("item fastest: owner is one GitHub login") for p in problems))
        data = tomllib.loads(SAMPLE)
        data["item"][3]["owner"] = "some-dev"
        self.assertIn("item old: shipped items have no owner or tracking issue", roadmap.validate(data))

    def test_now_and_next_are_short(self):
        data = tomllib.loads(SAMPLE)
        extra = dict(data["item"][0], focus=False)
        data["item"] += [dict(extra, id=f"now-{i}") for i in range(6)]
        data["item"] += [dict(extra, id=f"next-{i}", status="next") for i in range(13)]
        problems = roadmap.validate(data)
        self.assertIn("Now: 9 items, at most 8: move the rest to Later", problems)
        self.assertIn("Next: 13 items, at most 12: move the rest to Later", problems)

    def test_owner_and_tracking_issue_are_shown(self):
        data = tomllib.loads(SAMPLE)
        self.assertIn("Owner: [some-dev](https://github.com/some-dev) · Tracking: "
                      "[#20](https://github.com/horizon-rust/horizon/issues/20)", roadmap.render(data))
        site = json.loads(roadmap.site_data(data))
        self.assertEqual((site["items"][0]["owner"], site["items"][0]["issue"]), ("some-dev", "#20"))

    def test_three_to_five_items_in_focus(self):
        self.assertTrue(any("focus" in p for p in self.problems(focus=False)))

    def test_no_dates(self):
        for text in ("Ready in March.", "Before 2027.", "In Q3.", "Next week.", "In 3 weeks.", "By the end of it."):
            self.assertTrue(roadmap.dated(text), text)
        for text in ("Coming soon.", "Pull request #2026.", "Java 26.3.", "Mobs march on.", "It may land."):
            self.assertIsNone(roadmap.dated(text), text)
        self.assertTrue(any("date" in p for p in self.problems(plain="Done in October.")))


class Render(unittest.TestCase):
    def test_sections_links_and_the_overview(self):
        text = roadmap.render(tomllib.loads(SAMPLE))
        self.assertIn("## Now", text)
        self.assertIn("## Recently shipped", text)
        self.assertNotIn("## Later", text)
        self.assertIn("- **Fast**: Faster.", text)
        self.assertIn("Multiply, not divide ([#10](https://github.com/horizon-rust/horizon/issues/10)).", text)
        # A link the technical text already gives isn't repeated; the others are listed after it.
        self.assertIn("More: [#12](https://github.com/horizon-rust/horizon/issues/12)", text)
        self.assertIn("- **Old** (v0.2.1): Done.", text)


class GitHubState(unittest.TestCase):
    now = datetime.datetime(2030, 1, 20, tzinfo=datetime.timezone.utc)

    def test_a_now_item_without_live_pull_requests_is_stale(self):
        data = tomllib.loads(SAMPLE)
        states = {1: pull("MERGED", "2029-12-01T00:00:00Z"), 10: pull("OPEN"),
                  11: pull("MERGED", "2029-11-01T00:00:00Z"), 12: issue("OPEN"),
                  13: pull("MERGED", "2030-01-19T00:00:00Z"), 20: issue("OPEN"), 21: issue("OPEN"), 22: issue("OPEN")}
        problems, notes = roadmap.online_problems(data, states, self.now)
        self.assertEqual(len(problems), 1)
        self.assertIn("faster: stale", problems[0])
        self.assertTrue(any("fastest: ongoing" in note for note in notes))

    def test_the_tracking_issue_is_an_open_issue(self):
        states = {1: pull("MERGED", "2029-12-01T00:00:00Z"), 10: pull("OPEN"), 11: pull("OPEN"), 12: issue("OPEN"),
                  13: pull("OPEN"), 20: issue("CLOSED"), 21: pull("OPEN"), 22: issue("OPEN")}
        problems, _ = roadmap.online_problems(tomllib.loads(SAMPLE), states, self.now)
        self.assertEqual(sorted(p.split(":")[0] for p in problems), ["item fast", "item faster"])
        self.assertTrue(all("tracking issue" in p for p in problems))

    def test_missing_numbers_are_reported(self):
        problems, _ = roadmap.online_problems(tomllib.loads(SAMPLE), {}, self.now)
        self.assertTrue(any("#10 is neither" in p for p in problems))


class Journal(unittest.TestCase):
    def test_weekly_summaries_newest_first_with_their_monday(self):
        import tempfile
        from pathlib import Path
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            (folder / "2026-43.md").write_text("# A week\n\nPlain\nwords.\n\n## Technical\n\nDev (#1).\n")
            (folder / "2026-42.md").write_text("# Older\n\nPlain.\n\n## Technical\n\nDev.\n")
            (folder / "week-44.md").write_text("# Bad name\n")
            (folder / "2026-45.md").write_text("# No technical part\n\nPlain.\n")
            entries, problems = roadmap.weeks(folder)
        self.assertEqual([e["week"] for e in entries], ["2026-43", "2026-42"])
        self.assertEqual(entries[0], {"week": "2026-43", "date": "2026-10-19", "title": "A week",
                                      "plain": "Plain words.", "dev": "Dev (#1)."})
        self.assertEqual(len(problems), 2)


class Labels(unittest.TestCase):
    def test_creates_updates_and_adds_labels(self):
        data = tomllib.loads(SAMPLE)
        existing = {"roadmap:fast": {"color": "5B3FD6", "description": "Roadmap, now: Fast"},
                    "roadmap:faster": {"color": "000000", "description": "old"},
                    "roadmap:gone": {"color": "000000", "description": ""}}
        states = {10: dict(pull("OPEN"), labels=["roadmap:fast"]), 11: dict(pull("OPEN"), labels=[]),
                  13: dict(pull("MERGED", "2030-01-01T00:00:00Z"), labels=[])}
        changes, stale = roadmap.label_changes(data, existing, states)
        self.assertEqual([(action, target) for action, target, _ in changes],
                         [("update", "roadmap:faster"), ("add", 11), ("create", "roadmap:fastest"),
                          ("create", "roadmap:old")])
        self.assertEqual(changes[0][2]["color"], "5b3fd6")
        self.assertEqual(stale, ["roadmap:gone"])


class Sync(unittest.TestCase):
    def test_family_of_a_change(self):
        self.assertEqual(roadmap.family(["launcher/src/main.rs"]), "launcher")
        self.assertEqual(roadmap.family(["website/src/main.js"]), "website")
        self.assertEqual(roadmap.family(["launcher/src/main.rs", "crates/x/src/lib.rs"]), "game")

    def test_merged_and_released_items_move_to_the_top_of_shipped(self):
        data = tomllib.loads(SAMPLE)
        states = {10: pull("MERGED", "2030-01-02T00:00:00Z"), 11: pull("MERGED", "2030-01-03T00:00:00Z"),
                  12: issue("OPEN"), 13: pull("MERGED", "2030-01-03T00:00:00Z")}
        with mock.patch.object(roadmap, "first_release", return_value="v0.2.9") as release:
            text, log = roadmap.sync(data, SAMPLE, states)
        release.assert_called_once_with("abc", "game")
        # "faster" keeps an open issue and "fastest" is ongoing: only "fast" ships.
        self.assertEqual(log, ["fast: shipped in v0.2.9"])
        new = tomllib.loads(text)
        self.assertEqual([item["id"] for item in new["item"]], ["faster", "fastest", "fast", "old"])
        fast = new["item"][2]
        self.assertEqual((fast["status"], fast["shipped_in"]), ("shipped", "v0.2.9"))
        for key in ("focus", "owner", "issue"):
            self.assertNotIn(key, fast)
        self.assertEqual(fast["dev"], "Multiply, not divide (#10).")
        self.assertIn("# ---- Shipped\n\n[[item]]\nid = \"fast\"", text)

    def test_merged_but_unreleased_items_wait(self):
        states = {10: pull("MERGED", "2030-01-02T00:00:00Z")}
        with mock.patch.object(roadmap, "first_release", return_value=None):
            text, log = roadmap.sync(tomllib.loads(SAMPLE), SAMPLE, states)
        self.assertEqual(text, SAMPLE)
        self.assertEqual(log, ["fast: merged, waiting for a release"])


if __name__ == "__main__":
    unittest.main()
