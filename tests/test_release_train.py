import datetime as dt
from pathlib import Path
import tempfile
import unittest
from unittest import mock

from tools import release_train as train

NOW = dt.datetime(2026, 10, 15, 12, 0, tzinfo=dt.timezone.utc)


def pr(number, title="Fix a thing", labels=(), paths=("crates/horizon-client/src/game.rs",), issues=(), author="ludovic111",
       merged=dt.timedelta(hours=9)):
    return {"number": number, "title": title, "labels": list(labels), "paths": list(paths), "issues": list(issues),
            "author": author, "url": f"https://github.com/horizon-rust/horizon/pull/{number}", "merged_at": NOW - merged}


class ReleaseTrainTests(unittest.TestCase):
    def test_player_facing_paths(self):
        exporters = {"mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobDataExporter.java"}
        self.assertTrue(train.player_facing("crates/horizon-client/src/game.rs", "game"))
        self.assertTrue(train.player_facing("Cargo.lock", "game"))
        self.assertTrue(train.player_facing("xtask/src/main.rs", "game"))
        self.assertTrue(train.player_facing(next(iter(exporters)), "game", exporters))
        self.assertFalse(train.player_facing("mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFoodOracle.java", "game", exporters))
        self.assertFalse(train.player_facing("crates/horizon-world/examples/probe.rs", "game"))
        self.assertFalse(train.player_facing("crates/horizon-client/README.md", "game"))
        self.assertFalse(train.player_facing("tools/test_food_parity.py", "game"))
        self.assertFalse(train.player_facing("launcher/src/main.rs", "game"))
        self.assertTrue(train.player_facing("launcher/src/main.rs", "launcher"))
        self.assertTrue(train.player_facing("crates/horizon-prepare/src/lib.rs", "launcher"))
        self.assertFalse(train.player_facing("launcher/links.json", "launcher"))
        self.assertFalse(train.player_facing("crates/horizon-client/src/game.rs", "launcher"))

    def test_the_exporters_are_those_horizon_prepare_names(self):
        found = train.exporters()
        self.assertIn(train.EXPORTERS + "VanillaMobDataExporter.java", found)
        self.assertTrue(all(path.endswith(".java") for path in found))

    def test_game_tags_count_up_within_a_day(self):
        today = dt.date(2026, 10, 15)
        self.assertEqual(train.next_game_tag(["v2026.10.14", "launcher-v0.1.4"], today), "v2026.10.15")
        self.assertEqual(train.next_game_tag(["v2026.10.15"], today), "v2026.10.15.01")
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.01", "v2026.10.15.02"], today), "v2026.10.15.03")
        # Unpadded tags from before the change still count, and the next one sorts after them as text too.
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.9", "v2026.10.15.20"], today), "v2026.10.15.21")
        self.assertEqual(train.next_game_tag(["v2026.10.15", "v2026.10.15.08"], today), "v2026.10.15.09")
        tags = ["v2026.10.15.09", "v2026.10.15.10", "v2026.10.15"]
        self.assertEqual(sorted(tags), sorted(tags, key=train.tag_key))

    def test_launcher_tags_bump_the_patch_or_the_minor(self):
        tags = ["launcher-v0.1.3", "launcher-v0.1.10", "v2026.10.15", "launcher-v0.1.4"]
        self.assertEqual(train.next_launcher_tag(tags, minor=False), "launcher-v0.1.11")
        self.assertEqual(train.next_launcher_tag(tags, minor=True), "launcher-v0.2.0")

    def test_0_2_tags_follow_every_tag_from_before(self):
        order = ["launcher-v0.1.9", "launcher-v0.16.0", "v2026.10.19.15", "v2026.10.21", "v0.2.0-beta.1", "v0.2.0",
                 "v0.2.1-beta", "v0.2.1-beta.1", "v0.2.1-beta.2", "v0.2.1-beta.11", "v0.2.1-rc.1", "v0.2.1", "v0.2.9",
                 "v0.2.10", "v0.3.0-1", "v0.3.0-alpha", "v0.3.0", "v1.0.0"]
        self.assertEqual(sorted(reversed(order), key=train.tag_key), order)
        self.assertGreater(train.tag_key("launcher@0.2.0"), train.tag_key("launcher-v0.16.0"))
        self.assertEqual(train.tag_key("v2026.10.19"), train.tag_key("v2026.10.19.0"))
        for tag in ["v0.2.0", "v0.3.0-rc.1", "v2026.10.21", "v2026.10.19.15"]:
            self.assertTrue(train.is_game_tag(tag), tag)
        for tag in ["launcher@0.2.0", "launcher-v0.2.0", "v0.2", "vnext", "v2026.10.21-x", "v0.2.0-"]:
            self.assertFalse(train.is_game_tag(tag), tag)
        self.assertTrue(train.is_launcher_tag("launcher@0.2.0") and train.is_launcher_tag("launcher-v0.16.0"))
        self.assertFalse(train.is_launcher_tag("v0.2.0"))

    def test_tags_on_a_release_line(self):
        line = (0, 2)
        before = ["v2026.10.19.15", "v2026.10.21", "launcher-v0.16.0", "launcher-v0.2.0", "launcher-v0.2.1"]
        today = dt.date(2026, 10, 23)
        self.assertEqual(train.next_game_tag(before, today, line), "v0.2.0")
        self.assertEqual(train.next_launcher_tag(before, minor=True, line=line), "launcher@0.2.0")
        self.assertTrue(train.first_on_line(before, "v", line))
        after = before + ["v0.2.0", "v0.2.1", "launcher@0.2.0", "v0.3.0-rc.1"]
        self.assertEqual(train.next_game_tag(after, today, line), "v0.2.2")
        # A feature doesn't move the line; only .github/release-line.txt does.
        self.assertEqual(train.next_launcher_tag(after, minor=True, line=line), "launcher@0.2.1")
        self.assertFalse(train.first_on_line(after, "v", line))
        self.assertTrue(train.first_on_line(after, "v", (0, 3)), "a pre-release isn't the line's first release")
        self.assertEqual(train.next_game_tag(after, today, (0, 3)), "v0.3.0")
        with self.assertRaises(SystemExit):
            train.next_game_tag(after + ["v0.3.0"], today, line)
        # Without a line, the tags from before 0.2, whatever 0.2 tags exist.
        self.assertEqual(train.next_game_tag(before, today), "v2026.10.23")
        self.assertEqual(train.next_launcher_tag(after, minor=False), "launcher-v0.16.1")

    def test_the_release_line_file(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "release-line.txt"
            self.assertIsNone(train.release_line(path))
            path.write_text("0.2\n")
            self.assertEqual(train.release_line(path), (0, 2))
            path.write_text("0.2.0\n")
            with self.assertRaises(SystemExit):
                train.release_line(path)
        # Not set on main until the switch to 0.2 (docs/versioning.md).
        self.assertIsNone(train.release_line()) if not train.LINE_FILE.exists() else self.assertIsNotNone(train.release_line())

    def test_a_new_line_releases_at_once_unless_paused(self):
        self.assertEqual(train.decision("game", [], NOW, new_line="0.2"), (True, "the first release of 0.2"))
        self.assertFalse(train.decision("game", [], NOW, paused=True, new_line="0.2")[0])
        self.assertFalse(train.decision("game", [], NOW)[0])

    def test_the_first_release_of_0_2_says_what_changed(self):
        title, body = train.notes("game", "v0.2.0", [pr(1)], [], previous="v2026.10.21")
        self.assertTrue(title.startswith("Horizon 0.2.0: "), title)
        body = body.removeprefix(train.PLAYERS + "\n\n")
        self.assertTrue(body.startswith("Horizon is numbered from now on: this is Horizon 0.2.0, the release after 2026.10.21."))
        self.assertIn("`horizon-server update --version v0.2.0`", body)
        title, body = train.notes("launcher", "launcher@0.2.0", [], [], previous="launcher-v0.17.0")
        self.assertEqual(title, "Horizon launcher 0.2.0")
        self.assertIn("the release after launcher 0.17.0", body)
        self.assertIn("Rebuilt from the latest main.", body)
        body = train.notes("game", "v0.3.0", [pr(1)], [], previous="v0.2.14")[1]
        self.assertTrue(body.startswith(train.PLAYERS + "\n\nThe first release of Horizon 0.3."))
        for version, previous in [("v0.2.1", "v0.2.0"), ("v0.3.0-rc.1", "v0.2.9"), ("v2026.10.23", "v2026.10.21")]:
            body = train.notes("game", version, [pr(1)], [], previous=previous)[1]
            self.assertTrue(body.startswith(train.PLAYERS + "\n\n### Fixed"), (version, body))

    def test_decision(self):
        minute = dt.timedelta(minutes=1)
        decide = lambda prs, **kw: train.decision("game", prs, NOW, **kw)[0]
        hour = 60 * minute
        five = lambda newest: [pr(n, merged=newest + n * minute) for n in range(5)]
        self.assertTrue(decide(five(6 * minute)), "a batch, settled for the debounce")
        self.assertFalse(decide(five(2 * minute)), "a batch still merging")
        self.assertFalse(decide(five(6 * minute)[:4]), "not yet a batch")
        self.assertFalse(decide([pr(1, merged=5 * hour)]), "a lone fix waits for others")
        self.assertTrue(decide([pr(1, merged=6 * hour)]), "until the oldest has waited 6 hours")
        self.assertTrue(decide([pr(1, merged=7 * hour), pr(2, merged=minute)]), "merges that never pause still ship")
        self.assertFalse(decide([pr(n, merged=6 * minute) for n in range(4)] + [pr(9, labels=["skip-release"])]),
                         "work players don't see doesn't fill a batch")
        self.assertTrue(decide([pr(1, labels=["release-now"], merged=minute)]))
        self.assertFalse(decide([pr(1, paths=[])]), "nothing players run")
        self.assertFalse(decide([pr(1, labels=["skip-release"])]))
        self.assertFalse(decide([pr(1, labels=["release-now", "skip-release"])]))
        self.assertFalse(decide([pr(1)], paused=True))
        self.assertTrue(decide([], paused=True, force=True))

    def test_an_open_release_blocker_holds_every_release_even_forced(self):
        release, why = train.decision("game", [pr(1, labels=["release-now"])], NOW, blockers=[7, 9])
        self.assertFalse(release)
        self.assertEqual(why, "held by release-blocker #7, release-blocker #9")
        self.assertFalse(train.decision("game", [], NOW, force=True, blockers=[7])[0])
        self.assertFalse(train.decision("game", [], NOW, new_line="0.3", blockers=[7])[0])
        self.assertTrue(train.decision("game", [pr(1)], NOW, blockers=[])[0])

    def test_game_releases_ship_only_what_parity_main_passed(self):
        def runs(parity, checks):
            def gh_json(*arguments):
                table = parity if "parity-main" in arguments else checks
                if "--commit" in arguments:
                    sha = arguments[arguments.index("--commit") + 1]
                    return [{"status": s, "conclusion": c} for h, s, c in table if h == sha]
                return [{"headSha": h, "status": s, "conclusion": c} for h, s, c in table]
            return mock.patch.object(train, "gh_json", gh_json)
        checks = [("c", "in_progress", ""), ("b", "completed", "success"), ("a", "completed", "success")]
        with runs([("c", "queued", ""), ("b", "completed", "cancelled"), ("a", "completed", "success")], checks):
            self.assertEqual(train.green_main_head(), "b", "checks alone: the newest green checks")
            self.assertEqual(train.green_main_head(parity=True), "a", "a superseded parity-main run counts for nothing")
            self.assertTrue(train.passed_on("parity-main", "a"))
            self.assertFalse(train.passed_on("parity-main", "b"))
        with runs([("b", "completed", "failure"), ("a", "completed", "success")], checks):
            self.assertIsNone(train.green_main_head(parity=True), "a failed batch holds releases until fixed")
        with runs([("c", "completed", "success")], checks):
            self.assertIsNone(train.green_main_head(parity=True), "its checks must pass too")

    def test_a_release_is_published_only_after_its_smoke_test_and_with_no_blocker(self):
        drew = {"error": None, "frames": 12710, "chunks": 81, "meshed_sections": 550, "player": [160.5, 71.0, 156.5]}
        self.assertIsNone(train.gate(drew, []))
        self.assertEqual(train.gate(drew, [4]), "release-blocker #4 is open")
        self.assertEqual(train.gate(None, []), "no smoke-test.json: the release smoke test did not run")
        self.assertEqual(train.gate(None, [4]),
                         "no smoke-test.json: the release smoke test did not run; release-blocker #4 is open")
        self.assertEqual(train.smoke_problem({**drew, "error": "device lost"}), "the smoke run failed: device lost")
        for broken in ({**drew, "frames": 1499}, {**drew, "chunks": 0}, {**drew, "meshed_sections": 0},
                       {**drew, "player": None}, {}):
            self.assertEqual(train.smoke_problem(broken), "the smoke run did not draw a world", broken)

    def test_notes_never_call_a_release_alpha_or_beta(self):
        _, body = train.notes("game", "v0.2.12", [pr(1)], [])
        self.assertNotIn("alpha", body.lower())
        self.assertNotIn("beta", body.lower())
        self.assertNotIn("Stable", body)

    def test_notes(self):
        prs = [pr(5, "Nether portals: vanilla timing and linking (fixes #94)", issues=[94]),
               pr(6, "Controllers: gilrs input layer and gameplay", labels=["feature"], author="cosmic"),
               pr(7, "Release folder in Rust", labels=["skip-release"]),
               pr(8, "Website footer", paths=[])]
        title, body = train.notes("game", "v2026.10.15.1", prs, ["- Sound cuts out after an hour."])
        self.assertEqual(title, "Horizon 2026.10.15.1: Controllers, nether portals")
        self.assertIn("### New\n- Controllers: gilrs input layer and gameplay (#6)", body)
        self.assertIn("### Fixed\n- Nether portals: vanilla timing and linking (#5, fixes #94)", body)
        self.assertIn("### Known issues\n- Sound cuts out after an hour.", body)
        self.assertNotIn("#7", body)
        self.assertNotIn("#8", body)
        self.assertIn("Thanks to @cosmic.", body)
        self.assertLess(body.index("### New"), body.index("### Fixed"))
        title, _ = train.notes("game", "v2026.10.15.2", [pr(9, "Horizon's mark", labels=["feature"]),
                                                         pr(10, "Game menus", labels=["feature"])], [])
        self.assertEqual(title, "Horizon 2026.10.15.2: Game menus, Horizon's mark", "proper nouns keep their capital")

    def test_a_long_release_is_named_after_its_first_titles(self):
        prs = [pr(n, f"Change number {n} with a fairly long title") for n in range(1, 10)]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertTrue(title.startswith("Horizon 2026.10.15: Change number 9"), title)
        self.assertTrue(title.endswith("more fixes"), title)
        self.assertLess(len(title), 100)

    def test_a_feature_split_over_several_pull_requests_is_named_once(self):
        prs = [pr(1, "Controllers: rumble", labels=["feature"]), pr(2, "Controllers: menus", labels=["feature"]),
               pr(3, "Maces: smash attacks"), pr(4, "Fix stuck keys when opening Options")]
        title, _ = train.notes("game", "v2026.10.15", prs, [])
        self.assertEqual(title, "Horizon 2026.10.15: Controllers, stuck keys when opening Options, maces")
        self.assertEqual(train.summary([], [pr(n, f"Change number {n} with a fairly long title") for n in range(4)]),
                         "Change number 3 with a fairly long title and 3 more fixes")

    def test_launcher_notes(self):
        title, body = train.notes("launcher", "launcher-v0.2.0", [pr(3, "Launcher: say the self-update succeeded")], [])
        self.assertEqual(title, "Horizon launcher 0.2.0")
        self.assertIn("### Fixed\n- Launcher: say the self-update succeeded (#3)", body)
        self.assertNotIn("Known issues", body)
        self.assertNotIn(train.PLAYERS, body)
        self.assertIn("Your launcher updates itself.", body)

    def test_a_game_release_opens_with_the_way_to_the_launcher(self):
        # GitHub's latest is a game release: whoever lands on it is sent to the launcher first (#1647).
        _, body = train.notes("game", "v0.2.50", [pr(1)], ["- Sound cuts out after an hour."])
        self.assertTrue(body.startswith('<h3>Want to play? Download the <a href="https://playhorizon.dev">Horizon launcher'))
        self.assertIn(f'href="https://github.com/{train.REPO}/releases?q=launcher&amp;expanded=true"', body)
        self.assertNotIn("\n", train.PLAYERS, "one line: one HTML block, which ends at the blank line after it")
        self.assertEqual(body.count("playhorizon.dev"), 1, "said once, at the top")
        _, body = train.notes("game", "v0.2.51", [], [])
        self.assertTrue(body.startswith(train.PLAYERS + "\n\nRebuilt from the latest main."))

    def test_known_issues_are_the_bullets_of_the_file(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            path.write_text("# Known issues\n\nSome text.\n\n- One.\n- Two, with **bold**.  \n"
                            "  continued with `code`\n\tand a [link](https://example.com).  \n- Three.\n")
            self.assertEqual(train.known_issues(path),
                             ["- One.", "- Two, with **bold**. continued with `code` and a [link](https://example.com).",
                              "- Three."])
            self.assertEqual(train.known_issues(Path(temp, "missing.md")), [])

    def test_known_issues_do_not_include_unrelated_text(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            for separator in ["\n", "  \n", "# Heading\n", "Some text.\n"]:
                with self.subTest(separator=separator):
                    path.write_text("  Intro.\n- One.\n  continued.\n" + separator + "  Not an issue.\n- Two.\n")
                    self.assertEqual(train.known_issues(path), ["- One. continued.", "- Two."])
            path.write_text("# Known issues\n\nNo issues.\n")
            self.assertEqual(train.known_issues(path), [])
            path.write_text("")
            self.assertEqual(train.known_issues(path), [])

    def test_notes_keep_the_wrapped_shaderpacks_issue(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp, "known-issues.md")
            path.write_text("- Shader packs have been removed for now. Packs in your\n"
                            "  `shaderpacks` folder are left where they are.\n")
            _, body = train.notes("game", "v0.2.70", [pr(1)], train.known_issues(path))
            self.assertIn("### Known issues\n- Shader packs have been removed for now. Packs in your "
                          "`shaderpacks` folder are left where they are.\n", body)


if __name__ == "__main__":
    unittest.main()
