import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.ci import nightly_set as ns

ROOT = Path(__file__).resolve().parents[1]
NOW = "2026-10-28T12:00:00+00:00"
TEAM = {"kacpherek", "rdbtcvs", "online5277", "levzzz5154"}
PATTERNS = ns.guarded_patterns((ROOT / ".github/CODEOWNERS").read_text())


def pr(n, head="h", state="open", files=("crates/horizon-world/src/lib.rs",), base="main", approved_by=("rdbtcvs",),
       approved_at=None):
    sha = f"{head}{n}"
    return ns.PullRequest(n, state, sha, f"Fix {n}", f"dev{n}", base, list(files),
                          {who: approved_at or sha for who in approved_by})


def lookup(*prs):
    known = {p.number: p for p in prs}
    return lambda n: known.get(n)


def run(state, text, login="kacpherek", *prs):
    return ns.apply(state, ns.parse(text), login, NOW, lookup(*prs), PATTERNS, TEAM)


class ApprovalTests(unittest.TestCase):
    """A Nightly reaches every player: only pull requests the Team or Ludovic's agents approved at that head."""

    def test_an_unreviewed_pull_request_is_refused(self):
        with self.assertRaisesRegex(ns.Refused, "no approving review"):
            run({}, "/nightly add 1", "kacpherek", pr(1, approved_by=()))

    def test_the_authors_own_approval_and_outsiders_dont_count(self):
        with self.assertRaisesRegex(ns.Refused, "no approving review"):
            run({}, "/nightly add 1", "kacpherek", pr(1, approved_by=("dev1", "someone")))

    def test_ludovics_agents_and_the_team_count(self):
        for who in ("ludovic111", "levzzz5154"):
            self.assertEqual(run({}, "/nightly add 1", "kacpherek", pr(1, approved_by=(who,))).state["prs"][0]["number"], 1)

    def test_an_approval_on_an_older_head_doesnt_count(self):
        with self.assertRaisesRegex(ns.Refused, "no approving review"):
            run({}, "/nightly add 1", "kacpherek", pr(1, approved_at="old1"))

    def test_a_dry_run_needs_no_approval(self):
        self.assertTrue(run({}, "/nightly dry-run 1", "kacpherek", pr(1, approved_by=())).dry_run)


class ParseTests(unittest.TestCase):
    def test_commands(self):
        self.assertEqual(ns.parse("/nightly 2640 #2642, 2640"), ns.Command("set", [2640, 2642]))
        self.assertEqual(ns.parse("/nightly add 2650"), ns.Command("add", [2650]))
        self.assertEqual(ns.parse("/Nightly REMOVE 2640"), ns.Command("remove", [2640]))
        self.assertEqual(ns.parse("/nightly clear"), ns.Command("clear", []))
        self.assertEqual(ns.parse("/nightly status"), ns.Command("status", []))
        self.assertEqual(ns.parse("/nightly"), ns.Command("status", []))
        self.assertEqual(ns.parse("/nightly dry-run 1 2"), ns.Command("dry-run", [1, 2]))
        self.assertEqual(ns.parse("\n  /nightly 7\nthanks!"), ns.Command("set", [7]))

    def test_other_comments_are_not_commands(self):
        for body in ("", "LGTM", "please /nightly 2640", "/nightlyx 1", "> /nightly 1"):
            self.assertIsNone(ns.parse(body), body)

    def test_wrong_commands_are_answered(self):
        for body in ("/nightly please", "/nightly add", "/nightly clear 5", "/nightly 1 `rm -rf`",
                     "/nightly " + " ".join(str(n) for n in range(1, 32))):
            with self.assertRaises(ns.Refused, msg=body):
                ns.parse(body)

    def test_answers_never_quote_markdown_or_mentions(self):
        with self.assertRaises(ns.Refused) as e:
            ns.parse("/nightly @ludovic111`x`")
        self.assertNotIn("@", str(e.exception).split("isn't")[0])
        self.assertNotIn("`x`", str(e.exception))


class AllowTests(unittest.TestCase):
    LIST = "# trusted\nkacpherek   # added by Ludovic\nrdbtCVS\n\nnot a login!\n"

    def test_listed_reviewers_and_ludovic(self):
        reviewers = ns.read_reviewers(self.LIST)
        ns.authorize("kacpherek", reviewers)
        ns.authorize("RDBTcvs", reviewers)
        ns.authorize("ludovic111", reviewers)
        for login in ("levzzz5154", "someone", "not"):
            with self.assertRaises(ns.Refused):
                ns.authorize(login, reviewers)

    def test_paused_stops_reviewers_not_ludovic(self):
        reviewers = ns.read_reviewers(self.LIST + "paused  # Ludovic, 2026-10-29\n")
        self.assertTrue(reviewers.paused)
        with self.assertRaises(ns.Refused):
            ns.authorize("kacpherek", reviewers)
        ns.authorize("ludovic111", reviewers)

    def test_the_repos_list_names_the_three_reviewers(self):
        reviewers = ns.read_reviewers((ROOT / ns.REVIEWERS).read_text())
        self.assertEqual(reviewers.logins, {"kacpherek", "rdbtcvs", "online5277", "levzzz5154"})
        self.assertFalse(reviewers.paused)

class RefusalTests(unittest.TestCase):
    def test_what_codeowners_guards_is_refused(self):
        for path in (".github/workflows/x.yml", "tools/ci/nightly.py", "crates/horizon-release-sign/src/lib.rs",
                     "crates/horizon-server/src/update.rs", "launcher/src/backend.rs", "launcher/links.json",
                     "launcher/src/signed.rs", "tools/release_train.py"):
            self.assertTrue(ns.guarded(path, PATTERNS), path)
        for path in ("crates/horizon-client/src/game.rs", "launcher/src/main.rs", "tools/ci.py", "docs/x.md",
                     "crates/horizon-server/src/main.rs"):
            self.assertFalse(ns.guarded(path, PATTERNS), path)
        self.assertTrue(ns.guarded("tools/ci/x.py", ns.guarded_patterns("")))  # even without CODEOWNERS

    def test_a_set_with_a_refused_pull_request_changes_nothing(self):
        state = {"prs": []}
        bad = [pr(2, files=["tools/ci/nightly.py"]), pr(3, state="closed"), pr(4, state="merged"), pr(5, base="x")]
        for p in bad:
            with self.assertRaises(ns.Refused):
                run(state, f"/nightly 1 {p.number}", "kacpherek", pr(1), p)
        with self.assertRaises(ns.Refused):
            run(state, "/nightly 1 9", "kacpherek", pr(1))  # 9 doesn't exist

    def test_at_most_thirty(self):
        state = {"prs": [ns.entry(pr(n), "a", NOW) for n in range(1, 31)]}
        with self.assertRaises(ns.Refused):
            run(state, "/nightly add 31", "kacpherek", pr(31))


class SetTests(unittest.TestCase):
    def test_set_add_remove_clear(self):
        r = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2))
        self.assertTrue(r.build)
        self.assertEqual([e["number"] for e in r.state["prs"]], [1, 2])
        self.assertEqual(r.state["prs"][0]["asked_by"], "kacpherek")
        # add takes the newest head again and keeps its place
        r = run(r.state, "/nightly add 1 3", "rdbtCVS", pr(1, head="new"), pr(3))
        self.assertEqual([(e["number"], e["head"]) for e in r.state["prs"]], [(1, "new1"), (2, "h2"), (3, "h3")])
        r = run(r.state, "/nightly remove 2", "rdbtCVS")
        self.assertEqual([e["number"] for e in r.state["prs"]], [1, 3])
        r = run(r.state, "/nightly clear", "kacpherek")
        self.assertEqual(r.state["prs"], [])
        self.assertTrue(r.build)

    def test_status_and_dry_run_change_nothing(self):
        state = run({}, "/nightly 1", "kacpherek", pr(1)).state
        status = run(state, "/nightly status", "levzzz5154")
        self.assertIs(status.state, state)
        self.assertFalse(status.build)
        self.assertIn("#1 Fix 1", status.answer)
        dry = run(state, "/nightly dry-run 2 3", "kacpherek", pr(2), pr(3))
        self.assertIs(dry.state, state)
        self.assertEqual([e["number"] for e in dry.dry_run], [2, 3])

    def test_ludovics_remove_wins(self):
        state = run({}, "/nightly 1 2", "kacpherek", pr(1), pr(2)).state
        state = run(state, "/nightly remove 2", "ludovic111").state
        self.assertEqual(state["blocked"], [2])
        for text in ("/nightly add 2", "/nightly 1 2", "/nightly dry-run 2"):
            with self.assertRaises(ns.Refused):
                run(state, text, "kacpherek", pr(1), pr(2))
        state = run(state, "/nightly add 2", "ludovic111", pr(2)).state
        self.assertEqual(state["blocked"], [])
        self.assertEqual([e["number"] for e in run(state, "/nightly add 2", "kacpherek", pr(2)).state["prs"]], [1, 2])

    def test_titles_never_mention_anyone(self):
        p = pr(1)
        p.title = "Fix @everyone\nsecond line"
        answer = run({}, "/nightly 1", "kacpherek", p).answer
        self.assertNotIn("@everyone", answer)
        self.assertNotIn("second line\n", answer)

    def test_builds_are_numbered_per_day(self):
        self.assertEqual(ns.next_build("2026-10-28", []), "nightly/build-2026-10-28-1")
        self.assertEqual(ns.next_build("2026-10-28", ["nightly/build-2026-10-28-1", "nightly/build-2026-10-28-4"]),
                         "nightly/build-2026-10-28-5")


def sh(repo, *args, env=None):
    return subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True,
                          env={**os.environ, **(env or {})}).stdout.strip()


class AssemblyTests(unittest.TestCase):
    """A fixture repository: main and three pull requests, two of them touching the same line."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = self.tmp.name
        ident = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
                 "GIT_COMMITTER_EMAIL": "t@t", "GIT_AUTHOR_DATE": "2026-10-28T10:00:00Z",
                 "GIT_COMMITTER_DATE": "2026-10-28T10:00:00Z"}
        self.env = ident
        sh(self.repo, "init", "-q", "-b", "main")
        self.write({"a.txt": "a\n", "b.txt": "b\n", "c.txt": "c\n"})
        self.main = self.commit("main")
        self.p1 = self.branch("p1", {"a.txt": "a from p1\n"})
        self.p2 = self.branch("p2", {"b.txt": "b from p2\n", ".gitattributes": "* filter=evil\n"})
        self.p3 = self.branch("p3", {"a.txt": "a from p3\n"})
        sh(self.repo, "checkout", "-q", "main")

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, files):
        for name, text in files.items():
            Path(self.repo, name).write_text(text)

    def commit(self, message):
        sh(self.repo, "add", "-A")
        sh(self.repo, "commit", "-q", "-m", message, env=self.env)
        return sh(self.repo, "rev-parse", "HEAD")

    def branch(self, name, files):
        sh(self.repo, "checkout", "-q", "-b", name, self.main)
        self.write(files)
        return self.commit(name)

    def test_clean_merges_in_order_and_the_same_inputs_give_the_same_commit(self):
        built = ns.assemble(self.main, [(1, self.p1), (2, self.p2)], cwd=self.repo)
        self.assertEqual(sh(self.repo, "show", f"{built}:a.txt"), "a from p1")
        self.assertEqual(sh(self.repo, "show", f"{built}:b.txt"), "b from p2")
        self.assertEqual(sh(self.repo, "rev-list", "--count", f"{self.main}..{built}", "--first-parent"), "2")
        self.assertEqual(ns.assemble(self.main, [(1, self.p1), (2, self.p2)], cwd=self.repo), built)
        # nothing was checked out: the working tree is still main's
        self.assertEqual(Path(self.repo, "a.txt").read_text(), "a\n")
        self.assertFalse(Path(self.repo, ".gitattributes").exists())

    def test_two_pull_requests_that_conflict_are_named(self):
        with self.assertRaises(ns.Conflict) as e:
            ns.assemble(self.main, [(1, self.p1), (2, self.p2), (3, self.p3)], cwd=self.repo)
        self.assertEqual((e.exception.number, e.exception.other, e.exception.files), (3, 1, ["a.txt"]))
        self.assertIn("#1 and #3 conflict in `a.txt`", e.exception.text())

    def test_a_conflict_with_main_says_so(self):
        sh(self.repo, "checkout", "-q", "main")
        self.write({"a.txt": "a from main\n"})
        main = self.commit("main moves")
        with self.assertRaises(ns.Conflict) as e:
            ns.assemble(main, [(1, self.p1)], cwd=self.repo)
        self.assertIsNone(e.exception.other)
        self.assertIn("conflicts with main", e.exception.text())

    def test_changed_files_finds_guarded_paths(self):
        self.assertEqual(sorted(ns.changed_files(self.main, self.p2, cwd=self.repo)), [".gitattributes", "b.txt"])


class StateTests(unittest.TestCase):
    """The nightly-state branch on a local bare repository standing in for origin."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.origin, self.repo = Path(self.tmp.name, "origin.git"), Path(self.tmp.name, "work")
        subprocess.run(["git", "init", "-q", "--bare", str(self.origin)], check=True)
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        sh(self.repo, "remote", "add", "origin", str(self.origin))

    def tearDown(self):
        self.tmp.cleanup()

    def test_first_save_then_load(self):
        self.assertEqual(ns.load_state(str(self.repo)), ({}, "", None))
        ns.update_state(lambda st, b: ({"prs": [1]}, b + "{}\n", "first"), "t", cwd=str(self.repo))
        state, builds, head = ns.load_state(str(self.repo))
        self.assertEqual((state, builds), ({"prs": [1]}, "{}\n"))
        self.assertIn("first", sh(self.repo, "log", "-1", "--format=%s", head))

    def test_a_run_that_pushed_first_is_kept(self):
        ns.update_state(lambda st, b: ({"prs": [1]}, b, "one"), "t", cwd=str(self.repo))
        other = Path(self.tmp.name, "other")
        subprocess.run(["git", "init", "-q", str(other)], check=True)
        sh(other, "remote", "add", "origin", str(self.origin))
        seen = []

        def add_two(st, b):
            seen.append(list(st.get("prs", [])))
            if len(seen) == 1:  # meanwhile another run saves first
                ns.update_state(lambda s2, b2: ({"prs": s2["prs"] + [3]}, b2, "other"), "t", cwd=str(other))
            return {"prs": st["prs"] + [2]}, b, "two"
        final = ns.update_state(add_two, "t", cwd=str(self.repo))
        self.assertEqual(seen, [[1], [1, 3]])
        self.assertEqual(final["prs"], [1, 3, 2])
        self.assertEqual(ns.load_state(str(self.repo))[0]["prs"], [1, 3, 2])

    def test_builds_today_lists_the_days_branches(self):
        sh(self.repo, "commit", "-q", "--allow-empty", "-m", "x", env={
            "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"})
        head = sh(self.repo, "rev-parse", "HEAD")
        for name in ("nightly/build-2026-10-28-1", "nightly/build-2026-10-28-2", "nightly/build-2026-10-27-1"):
            self.assertTrue(ns.push(head, name, "t", cwd=str(self.repo)))
        self.assertEqual(sorted(ns.builds_today("2026-10-28", cwd=str(self.repo))),
                         ["nightly/build-2026-10-28-1", "nightly/build-2026-10-28-2"])


if __name__ == "__main__":
    unittest.main()
