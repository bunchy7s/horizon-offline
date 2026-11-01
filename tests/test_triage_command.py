import unittest
from datetime import datetime, timezone
from pathlib import Path

from tools.ci import nightly_set as ns
from tools.ci import triage_command as tc

ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 10, 28, 12, tzinfo=timezone.utc)
URL = "https://github.com/horizon-rust/horizon/issues/5#issuecomment-1"
LIST = ns.read_reviewers("kacpherek\nrdbtCVS\n")


def thread(state="open", user="dev", labels=("bug",), pr=False, merged=False, locked=False):
    t = {"state": state, "user": {"login": user}, "labels": [{"name": n} for n in labels], "locked": locked}
    if pr:
        t["pull_request"] = {"merged_at": "2026-10-27T00:00:00Z" if merged else None}
    return t


class Fake:
    def __init__(self, threads=None, labels=("bug", "good first issue", "release-now", "skip-release"), past=()):
        self.threads = {5: thread()} if threads is None else threads
        self.labels, self.past, self.calls = list(labels), list(past), []

    def issue(self, n):
        return self.threads.get(n)

    def label(self, name):
        return next((l for l in self.labels if l.lower() == name.lower()), None)

    def add_labels(self, n, names):
        self.calls.append(("add", n, names))

    def remove_label(self, n, name):
        self.calls.append(("remove", n, name))

    def close(self, n, reason):
        self.calls.append(("close", n, reason))

    def reopen(self, n):
        self.calls.append(("reopen", n))

    def comment(self, n, text):
        self.calls.append(("comment", n, text))

    def recent_comments(self, since):
        return self.past


def do(text, api=None, login="kacpherek", reviewers=LIST, n=5):
    api = api or Fake()
    return tc.run(api, tc.parse(text), login, n, URL, reviewers, NOW), api


class ParseTests(unittest.TestCase):
    def test_commands_and_case(self):
        self.assertEqual(tc.parse("/close"), tc.Command("close", "completed"))
        self.assertEqual(tc.parse("  /CLOSE   Not   Planned "), tc.Command("close", "not_planned"))
        self.assertEqual(tc.parse("/close completed"), tc.Command("close", "completed"))
        self.assertEqual(tc.parse("/close duplicate of #123"), tc.Command("close", "duplicate", 123))
        self.assertEqual(tc.parse("/Close  duplicate   of 7"), tc.Command("close", "duplicate", 7))
        self.assertEqual(tc.parse("/reopen"), tc.Command("reopen"))
        self.assertEqual(tc.parse("/label bug,  good first issue ,bug"), tc.Command("label", labels=("bug", "good first issue")))
        self.assertEqual(tc.parse("/unlabel bug"), tc.Command("unlabel", labels=("bug",)))

    def test_first_line_only(self):
        self.assertEqual(tc.parse("/close\n/label release-now"), tc.Command("close", "completed"))
        self.assertIsNone(tc.parse("thanks\n/close"))
        self.assertEqual(tc.parse("\n\n/reopen\nwhy"), tc.Command("reopen"))

    def test_not_commands(self):
        for text in ("", "hello", "/nightly 5", "/closed", "/labels bug", "/closer", "close", "/unlabeled x"):
            self.assertIsNone(tc.parse(text), text)

    def test_malformed(self):
        for text in ("/close maybe", "/close duplicate of", "/reopen now", "/label", "/label ,", "/unlabel a, b",
                     "/label " + ",".join(str(i) for i in range(11))):
            with self.assertRaises(tc.Refused, msg=text):
                tc.parse(text)

    def test_quoting_is_safe(self):
        with self.assertRaises(tc.Refused) as e:
            tc.parse("/close @ludovic111 [x](http://evil)")
        self.assertNotIn("@", str(e.exception))
        self.assertNotIn("](", str(e.exception))


class AllowTests(unittest.TestCase):
    def test_listed_and_ludovic(self):
        do("/close", login="RDBTcvs")
        do("/close", login="ludovic111")

    def test_stranger_refused(self):
        with self.assertRaisesRegex(tc.Refused, "Only Ludovic"):
            do("/close", login="someone")

    def test_paused(self):
        paused = ns.read_reviewers("kacpherek\npaused\n")
        with self.assertRaisesRegex(tc.Refused, "paused"):
            do("/close", reviewers=paused)
        do("/close", reviewers=paused, login="ludovic111")

    def test_the_repos_list_is_the_nightlys(self):
        self.assertEqual(ns.REVIEWERS, ".github/trusted-reviewers.txt")
        listed = ns.read_reviewers((ROOT / ns.REVIEWERS).read_text())
        self.assertEqual(listed.logins, {"kacpherek", "rdbtcvs", "online5277", "levzzz5154", "qwerner88880"})
        self.assertFalse(listed.paused)


class ActionTests(unittest.TestCase):
    def test_close_issue(self):
        text, api = do("/close not planned")
        self.assertEqual(api.calls, [("close", 5, "not_planned")])
        self.assertIn("on behalf of @kacpherek", text)
        self.assertIn(URL, text)
        self.assertTrue(text.startswith("🤖 Triage: "))

    def test_close_pr_has_no_reason(self):
        api = Fake({5: thread(pr=True)})
        text, _ = do("/close", api)
        self.assertEqual(api.calls, [("close", 5, None)])
        self.assertIn("without merging", text)

    def test_duplicate(self):
        api = Fake({5: thread(), 9: thread()})
        text, _ = do("/close duplicate of #9", api)
        self.assertEqual(api.calls, [("comment", 5, "Duplicate of #9"), ("close", 5, "duplicate")])
        self.assertIn("#9", text)

    def test_duplicate_of_missing_or_itself(self):
        with self.assertRaisesRegex(tc.Refused, "no #9"):
            do("/close duplicate of #9")
        with self.assertRaisesRegex(tc.Refused, "itself"):
            do("/close duplicate of #5")

    def test_reopen(self):
        _, api = do("/reopen", Fake({5: thread("closed")}))
        self.assertEqual(api.calls, [("reopen", 5)])
        with self.assertRaisesRegex(tc.Refused, "already open"):
            do("/reopen")
        with self.assertRaisesRegex(tc.Refused, "already closed"):
            do("/close", Fake({5: thread("closed")}))

    def test_label_uses_the_real_name_and_skips_present(self):
        _, api = do("/label Good First Issue, BUG")
        self.assertEqual(api.calls, [("add", 5, ["good first issue"])])
        with self.assertRaisesRegex(tc.Refused, "already has"):
            do("/label bug")

    def test_label_must_exist(self):
        with self.assertRaisesRegex(tc.Refused, "no label"):
            do("/label nonsense")

    def test_unlabel(self):
        _, api = do("/unlabel BUG")
        self.assertEqual(api.calls, [("remove", 5, "bug")])
        with self.assertRaisesRegex(tc.Refused, "doesn't have"):
            do("/unlabel good first issue")


class RefusalTests(unittest.TestCase):
    def test_release_and_parity_labels(self):
        for name in ("release-now", "Release-Blocker", "skip-release", "full-parity", "prerelease"):
            for verb in ("/label", "/unlabel"):
                with self.assertRaisesRegex(tc.Refused, "release or parity", msg=name):
                    do(f"{verb} {name}", Fake(labels=(name,)))
        with self.assertRaises(tc.Refused):
            do("/label bug, release-now")

    def test_ludovics_threads(self):
        for text in ("/close", "/label bug", "/unlabel bug", "/reopen"):
            for t in (thread(user="ludovic111", state="closed" if text == "/reopen" else "open"),
                      thread(user="Ludovic111", pr=True)):
                with self.assertRaisesRegex(tc.Refused, "Ludovic's"):
                    do(text, Fake({5: t}))

    def test_merged_pr(self):
        for text in ("/close", "/reopen", "/label bug", "/unlabel bug"):
            with self.assertRaisesRegex(tc.Refused, "merged"):
                do(text, Fake({5: thread("closed", pr=True, merged=True)}))

    def test_locked(self):
        for text in ("/close", "/label good first issue", "/unlabel bug"):
            with self.assertRaisesRegex(tc.Refused, "locked"):
                do(text, Fake({5: thread(locked=True)}))

    def test_missing_thread(self):
        with self.assertRaisesRegex(tc.Refused, "no such"):
            do("/close", Fake({}))

    def test_nothing_written_on_refusal(self):
        api = Fake({5: thread(user="ludovic111")})
        with self.assertRaises(tc.Refused):
            do("/close", api)
        self.assertEqual(api.calls, [])


def past(login, n, who="github-actions[bot]"):
    return [{"user": who, "body": f"🤖 Triage: closed as completed, on behalf of @{login} ([their comment](u))."}
            for _ in range(n)]


class LimitTests(unittest.TestCase):
    def test_sixty_a_day(self):
        with self.assertRaisesRegex(tc.Refused, "60"):
            do("/close", Fake(past=past("kacpherek", 60)))
        do("/close", Fake(past=past("kacpherek", 59)))

    def test_other_reviewers_and_forged_comments_dont_count(self):
        do("/close", Fake(past=past("rdbtcvs", 80)))
        do("/close", Fake(past=past("kacpherek", 80, who="someone")))

    def test_refusals_dont_count(self):
        refusals = [{"user": "github-actions[bot]", "body": "🤖 Triage: refused, nope (asked by @kacpherek)"}] * 70
        do("/close", Fake(past=refusals))

    def test_counts_case_insensitively(self):
        with self.assertRaises(tc.Refused):
            do("/close", Fake(past=past("kacpherek", 60)), login="Kacpherek")

    def test_ludovic_is_unlimited(self):
        do("/close", Fake(past=past("ludovic111", 100)), login="ludovic111")


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/triage-command.yml").read_text()
        run_lines = [l for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["        run: python3 tools/ci/triage_command.py command"])
        self.assertIn("persist-credentials: false", text)
        self.assertNotIn("pull_request_target", text)
        for perm in ("issues: write", "pull-requests: write", "contents: read"):
            self.assertIn(perm, text)
        self.assertNotIn("contents: write", text)


if __name__ == "__main__":
    unittest.main()
