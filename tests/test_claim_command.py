import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from tools.ci import claim_command as cc
from tools.ci import nightly_set as ns

ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 10, 29, 12, tzinfo=timezone.utc)
LIST = ns.read_reviewers("kacpherek\nrdbtCVS\n")


def thread(state="open", holders=(), pr=False, locked=False):
    t = {"state": state, "assignees": [{"login": h} for h in holders], "locked": locked}
    if pr:
        t["pull_request"] = {"merged_at": None}
    return t


class Fake:
    def __init__(self, threads=None, claims=0, assignable=True):
        self.threads = {5: thread()} if threads is None else threads
        self.claims, self.assignable, self.calls = claims, assignable, []

    def issue(self, n):
        return self.threads.get(n)

    def assign(self, n, login):
        self.calls.append(("assign", n, login))
        return cc.assignees(self.threads[n]) + ([login] if self.assignable else [])

    def unassign(self, n, login):
        self.calls.append(("unassign", n, login))

    def open_claims(self, login):
        return self.claims

    def comment(self, n, text):
        self.calls.append(("comment", n, text))


def do(text, api=None, login="newcomer", reviewers=LIST, n=5):
    api = api or Fake()
    return cc.run(api, cc.parse(text), login, n, reviewers), api


class ParseTests(unittest.TestCase):
    def test_commands(self):
        self.assertEqual(cc.parse("/claim"), cc.Command("claim"))
        self.assertEqual(cc.parse("  /CLAIM  \nI'll take it"), cc.Command("claim"))
        self.assertEqual(cc.parse("/unclaim"), cc.Command("unclaim"))
        self.assertEqual(cc.parse("/unclaim @Some-One"), cc.Command("unclaim", "Some-One"))
        self.assertEqual(cc.parse("/unclaim some-one"), cc.Command("unclaim", "some-one"))

    def test_not_commands(self):
        for text in ("", "claim", "/claimed", "/claims", "thanks\n/claim", "/close", "/unclaimed"):
            self.assertIsNone(cc.parse(text), text)

    def test_malformed(self):
        for text in ("/claim this one", "/claim @other", "/unclaim @a b", "/unclaim @-bad", "/unclaim <x>"):
            with self.assertRaises(cc.Refused, msg=text):
                cc.parse(text)


class ClaimTests(unittest.TestCase):
    def test_claim_a_free_issue(self):
        text, api = do("/claim")
        self.assertIsNone(text)  # the 👍 says it
        self.assertEqual(api.calls, [("assign", 5, "newcomer")])

    def test_taken(self):
        with self.assertRaisesRegex(cc.Refused, "@other already claimed"):
            do("/claim", Fake({5: thread(holders=("other",))}))

    def test_claimed_on_discord(self):
        t = thread()
        t["labels"] = [{"name": "In progress"}]
        api = Fake({5: t})
        with self.assertRaisesRegex(cc.Refused, "on Discord"):
            do("/claim", api)
        self.assertEqual(api.calls, [])

    def test_already_yours(self):
        with self.assertRaisesRegex(cc.Refused, "already hold"):
            do("/claim", Fake({5: thread(holders=("NewComer",))}))

    def test_not_claimable(self):
        for t in (thread(pr=True), thread(state="closed"), thread(locked=True)):
            api = Fake({5: t})
            with self.assertRaises(cc.Refused):
                do("/claim", api)
            self.assertEqual(api.calls, [])
        with self.assertRaisesRegex(cc.Refused, "no such"):
            do("/claim", Fake({}))

    def test_five_claims_at_most(self):
        api = Fake(claims=cc.MAX_CLAIMS)
        with self.assertRaisesRegex(cc.Refused, "5 open issues"):
            do("/claim", api)
        self.assertEqual(api.calls, [])
        do("/claim", Fake(claims=cc.MAX_CLAIMS - 1))

    def test_team_has_no_limit_unless_paused(self):
        do("/claim", Fake(claims=50), login="ludovic111")
        do("/claim", Fake(claims=50), login="RdbtCVS")
        with self.assertRaises(cc.Refused):
            do("/claim", Fake(claims=50), login="rdbtCVS", reviewers=ns.read_reviewers("rdbtCVS\npaused\n"))

    def test_github_dropped_the_assignee(self):
        with self.assertRaisesRegex(cc.Refused, "didn't assign"):
            do("/claim", Fake(assignable=False))


class UnclaimTests(unittest.TestCase):
    def test_own(self):
        text, api = do("/unclaim", Fake({5: thread(holders=("NewComer",))}))
        self.assertIsNone(text)
        self.assertEqual(api.calls, [("unassign", 5, "NewComer")])

    def test_not_held(self):
        with self.assertRaisesRegex(cc.Refused, "don't hold"):
            do("/unclaim", Fake({5: thread(holders=("other",))}))

    def test_someone_elses_needs_the_team(self):
        api = Fake({5: thread(holders=("Other",))})
        with self.assertRaisesRegex(cc.Refused, "trusted reviewers"):
            do("/unclaim @other", api)
        self.assertEqual(api.calls, [])
        text, api = do("/unclaim @other", Fake({5: thread(holders=("Other",))}), login="kacpherek")
        self.assertIn("released @Other's claim, on behalf of @kacpherek", text)
        self.assertEqual(api.calls, [("unassign", 5, "Other")])

    def test_someone_who_holds_nothing(self):
        with self.assertRaisesRegex(cc.Refused, "`ghost` doesn't hold"):
            do("/unclaim @ghost", login="ludovic111")


def node(number=7, holders=("dev",), events=(), closing=()):
    return {"number": number, "assignees": {"nodes": [{"login": h} for h in holders]},
            "closedByPullRequestsReferences": {"nodes": [{"number": c} for c in closing]},
            "timelineItems": {"nodes": list(events)}}


def assigned(login, days_ago, actor="github-actions"):
    when = (NOW - timedelta(days=days_ago)).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {"__typename": "AssignedEvent", "createdAt": when, "actor": {"login": actor}, "assignee": {"login": login}}


def mentioned(state):
    return {"__typename": "CrossReferencedEvent", "source": {"number": 9, "state": state}}


class ReleaseTests(unittest.TestCase):
    def test_stale_claim_released(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 6)]), NOW), ["dev"])

    def test_fresh_claim_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 4)]), NOW), [])

    def test_newest_assignment_counts(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), assigned("dev", 1)]), NOW), [])

    def test_hand_assignments_kept(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 30, actor="kacpherek")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[]), NOW), [])

    def test_an_open_pull_request_keeps_it(self):
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("OPEN")]), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9)], closing=(9,)), NOW), [])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), mentioned("CLOSED")]), NOW), ["dev"])
        self.assertEqual(cc.stale_claims(node(events=[assigned("dev", 9), {"__typename": "CrossReferencedEvent",
                                                                          "source": {}}]), NOW), ["dev"])

    def test_release_unassigns_and_says_why(self):
        class Api(Fake):
            def assigned_issues(self):
                return [node(7, ("a", "b"), [assigned("a", 6), assigned("b", 2)]), node(8, ("c",), [assigned("c", 8)])]

        api = Api()
        self.assertEqual(cc.release(api, NOW, dry_run=True), [(7, "a"), (8, "c")])
        self.assertEqual(api.calls, [])
        cc.release(api, NOW, dry_run=False)
        self.assertEqual([c[:3] for c in api.calls if c[0] == "unassign"], [("unassign", 7, "a"), ("unassign", 8, "c")])
        comments = [c[2] for c in api.calls if c[0] == "comment"]
        self.assertEqual(len(comments), 2)
        self.assertTrue(comments[0].startswith(cc.PREFIX + "released @a's claim: 5 days"))


class WorkflowTests(unittest.TestCase):
    def test_workflow_is_safe(self):
        text = (ROOT / ".github/workflows/claim-command.yml").read_text()
        run_lines = [l.strip() for l in text.splitlines() if l.strip().startswith("run:")]
        self.assertEqual(run_lines, ["run: python3 tools/ci/claim_command.py command",
                                     "run: python3 tools/ci/claim_command.py release"])
        self.assertEqual(text.count("persist-credentials: false"), 2)
        self.assertEqual(text.count("ref: main"), 2)
        self.assertNotIn("pull_request_target", text)
        self.assertNotIn("contents: write", text)
        self.assertNotIn("pull-requests: write", text)
        for line in text.splitlines():
            if "github.event.comment" in line and "run:" in line:
                self.fail(f"comment data in a shell line: {line}")


if __name__ == "__main__":
    unittest.main()
