#!/usr/bin/env python3
"""Anyone claims an issue before working on it (CONTRIBUTING.md "Claim before you start").

    claim_command.py command               one comment (claim-command.yml): assigns or unassigns, answers in the thread
    claim_command.py release [--dry-run]   the daily run: releases the claims older than 5 days with no open pull request

A comment on an issue, on its first line:

    /claim             assigns the commenter (GitHub lets an issue's commenters be assigned, no role needed)
    /unclaim           unassigns the commenter
    /unclaim @login    Ludovic and the trusted reviewers (.github/trusted-reviewers.txt) release someone else's claim

An assigned issue is taken, and so is one claimed on Discord (Kira's `in progress` label). Refused with a one-line
answer: a pull request (claim the issue it fixes), a closed or locked issue, an issue someone else holds, more than 5 open claims at once (Ludovic and the trusted reviewers have no
limit). A success answers with a 👍 on the comment; the assignment shows in the thread.
A claim made here is released after 5 days unless an open pull request links the issue (`Fixes #N`, a mention, or the
sidebar's link); assignments made by hand are never released. No code of any pull request is read or run: only the
comment's text, its author and the API's answers, which reach this script as env vars.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import nightly_set as ns  # noqa: E402  (the same trusted reviewers and owner)

PREFIX = "🤖 Claims: "
MAX_CLAIMS = 5
STALE = timedelta(days=5)
LOGIN = re.compile(r"^@?([A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))$")
USAGE = "Use `/claim` or `/unclaim` (the trusted reviewers: `/unclaim @login`)."
# The login GitHub shows for this workflow's token: `github-actions[bot]` in REST, `github-actions` in GraphQL.
BOT_LOGINS = ("github-actions", "github-actions[bot]")
# Kira's Discord `/claim` marks an issue with this label (horizon-rust/kira, WORK.label): it is taken too.
KIRA_LABEL = "in progress"


class Refused(Exception):
    """A command that changes nothing; its text is the answer."""


@dataclass
class Command:
    verb: str  # claim, unclaim
    who: str | None = None  # unclaim @login


def safe(text: str) -> str:
    """Text of a comment, fit to quote back: no markdown, no mentions."""
    return re.sub(r"[^\w#:. -]", "", text)[:60].strip() or "?"


def parse(body: str) -> Command | None:
    """The command on a comment's first line; None when it isn't one. Raises Refused when it is one, but wrong."""
    line = next((line.strip() for line in body.splitlines() if line.strip()), "")
    m = re.match(r"^/(\w+)(?:\s+(.*))?$", line)
    if not m or m.group(1).lower() not in ("claim", "unclaim"):
        return None
    verb, rest = m.group(1).lower(), (m.group(2) or "").strip()
    if not rest:
        return Command(verb)
    who = LOGIN.match(rest)
    if verb == "claim" or not who:
        raise Refused(f"`{safe(rest)}` isn't understood here. {USAGE}")
    return Command(verb, who.group(1))


def is_team(login: str, reviewers: ns.Reviewers) -> bool:
    """Ludovic, or a listed reviewer while the list isn't paused."""
    return ns.is_owner(login) or (login.lower() in reviewers.logins and not reviewers.paused)


def assignees(issue: dict) -> list[str]:
    return [a["login"] for a in issue.get("assignees") or []]


def target_refusal(issue: dict) -> str | None:
    """Why this thread can't be claimed or released (None when it can). `issue` is GitHub's issue object."""
    if "pull_request" in issue:
        return "Pull requests aren't claimed: claim the issue this one fixes, or open one first."
    if issue.get("locked"):
        return "This issue is locked."
    if issue.get("state") == "closed":
        return "This issue is closed."
    return None


# --- GitHub ------------------------------------------------------------------------------------------------------


def gh(*args: str, check: bool = True, stdin: str | None = None) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, input=stdin)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {r.stderr.strip()[:300]}")
    return r.stdout.strip() if r.returncode == 0 else ""


ASSIGNED_ISSUES = """
query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    issues(first: 50, states: OPEN, filterBy: {assignee: "*"}, after: $cursor) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number
        assignees(first: 10) { nodes { login } }
        closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { nodes { number } }
        timelineItems(last: 100, itemTypes: [ASSIGNED_EVENT, CROSS_REFERENCED_EVENT]) {
          nodes {
            __typename
            ... on AssignedEvent { createdAt actor { login } assignee { ... on User { login } } }
            ... on CrossReferencedEvent { source { ... on PullRequest { number state } } }
          }
        }
      }
    }
  }
}"""


class Api:
    """The few GitHub calls this needs; tests replace it."""

    def __init__(self, repo: str):
        self.repo = repo

    def issue(self, n: int) -> dict | None:
        raw = gh("api", f"repos/{self.repo}/issues/{n}", check=False)
        return json.loads(raw) if raw else None

    def assign(self, n: int, login: str) -> list[str]:
        """Assigns `login`; returns the assignees GitHub kept (it silently drops someone it can't assign)."""
        raw = gh("api", f"repos/{self.repo}/issues/{n}/assignees", "-f", f"assignees[]={login}")
        return assignees(json.loads(raw))

    def unassign(self, n: int, login: str) -> None:
        gh("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/assignees", "-f", f"assignees[]={login}")

    def open_claims(self, login: str) -> int:
        q = f"repo:{self.repo} is:issue is:open assignee:{login}"
        return int(gh("api", "-X", "GET", "search/issues", "-f", f"q={q}", "-f", "per_page=1", "--jq", ".total_count"))

    def react(self, comment_id: int) -> None:
        gh("api", f"repos/{self.repo}/issues/comments/{comment_id}/reactions", "-f", "content=+1")

    def comment(self, n: int, text: str) -> None:
        gh("api", f"repos/{self.repo}/issues/{n}/comments", "-f", f"body={text}")

    def assigned_issues(self) -> list[dict]:
        owner, name = self.repo.split("/")
        out, cursor = [], None
        while True:
            args = ["api", "graphql", "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={ASSIGNED_ISSUES}"]
            if cursor:
                args += ["-F", f"cursor={cursor}"]
            page = json.loads(gh(*args))["data"]["repository"]["issues"]
            out += page["nodes"]
            if not page["pageInfo"]["hasNextPage"]:
                return out
            cursor = page["pageInfo"]["endCursor"]


def run(api, cmd: Command, login: str, number: int, reviewers: ns.Reviewers) -> str | None:
    """Does the command; returns the comment to post, or None when a 👍 says it. Raises Refused for a refusal."""
    issue = api.issue(number)
    if issue is None:
        raise Refused("There is no such issue.")
    if (why := target_refusal(issue)):
        raise Refused(why)
    holders = assignees(issue)
    held = {h.lower() for h in holders}
    if cmd.verb == "claim":
        if login.lower() in held:
            raise Refused("You already hold this issue.")
        if holders:
            names = ", ".join(f"@{h}" for h in holders)
            raise Refused(f"{names} already claimed this issue: pick another, or ask them here.")
        if KIRA_LABEL in {str(l.get("name", "")).lower() for l in issue.get("labels") or []}:
            raise Refused(f"someone claimed this issue on Discord (Kira's `{KIRA_LABEL}` label): pick another, or ask "
                          "in the thread.")
        if not is_team(login, reviewers) and api.open_claims(login) >= MAX_CLAIMS:
            raise Refused(f"You hold {MAX_CLAIMS} open issues: finish or `/unclaim` one first.")
        if login.lower() not in {a.lower() for a in api.assign(number, login)}:
            raise Refused("GitHub didn't assign you; nothing changed.")
        return None
    who = cmd.who or login
    if who.lower() != login.lower() and not is_team(login, reviewers):
        raise Refused(f"Only Ludovic and the trusted reviewers in `{ns.REVIEWERS}` release someone else's claim.")
    if who.lower() not in held:
        raise Refused("You don't hold this issue." if who.lower() == login.lower() else f"`{safe(who)}` doesn't hold this issue.")
    real = next(h for h in holders if h.lower() == who.lower())
    api.unassign(number, real)
    if who.lower() == login.lower():
        return None
    return f"{PREFIX}released @{real}'s claim, on behalf of @{login}. `/claim` takes it."


def command() -> int:
    body, login = os.environ["BODY"], os.environ["AUTHOR"]
    number, repo, comment_id = int(os.environ["ISSUE"]), os.environ["REPO"], int(os.environ["COMMENT_ID"])
    api: Api = Api(repo)
    try:
        cmd = parse(body)
        if cmd is None:
            return 0
        reviewers = ns.read_reviewers(Path(ns.REVIEWERS).read_text())
        text = run(api, cmd, login, number, reviewers)
    except Refused as e:
        text = f"{PREFIX}refused, {e} (asked by @{safe(login)})"
    except RuntimeError as e:
        print(e, file=sys.stderr)
        text = f"{PREFIX}GitHub refused that, nothing more was done (asked by @{safe(login)})."
    if text is None:
        api.react(comment_id)
    else:
        api.comment(number, text)
    return 0


# --- The daily release --------------------------------------------------------------------------------------------


def stale_claims(issue: dict, now: datetime) -> list[str]:
    """The assignees of one issue (GraphQL node) whose claim this workflow made over 5 days ago, when no open pull
    request links the issue."""
    events = issue["timelineItems"]["nodes"]
    if issue["closedByPullRequestsReferences"]["nodes"] or any(
            e["__typename"] == "CrossReferencedEvent" and (e.get("source") or {}).get("state") == "OPEN"
            for e in events):
        return []
    last: dict[str, dict] = {}  # each assignee's newest assignment
    for e in events:
        if e["__typename"] == "AssignedEvent" and (e.get("assignee") or {}).get("login"):
            last[e["assignee"]["login"].lower()] = e
    out = []
    for node in issue["assignees"]["nodes"]:
        e = last.get(node["login"].lower())
        if not e or (e.get("actor") or {}).get("login") not in BOT_LOGINS:
            continue  # assigned by a person (or before the window GitHub returns): never released
        when = datetime.fromisoformat(e["createdAt"].replace("Z", "+00:00"))
        if now - when > STALE:
            out.append(node["login"])
    return out


def release(api, now: datetime, dry_run: bool) -> list[tuple[int, str]]:
    done = []
    for issue in api.assigned_issues():
        for login in stale_claims(issue, now):
            done.append((issue["number"], login))
            if dry_run:
                continue
            api.unassign(issue["number"], login)
            api.comment(issue["number"],
                        f"{PREFIX}released @{login}'s claim: {STALE.days} days without an open pull request that links "
                        f"this issue. `/claim` again when you pick it back up (a draft pull request with "
                        f"`Fixes #{issue['number']}` keeps the claim).")
    return done


if __name__ == "__main__":
    if sys.argv[1:] == ["command"]:
        sys.exit(command())
    if sys.argv[1:2] == ["release"] and sys.argv[2:] in ([], ["--dry-run"]):
        dry = sys.argv[2:] == ["--dry-run"]
        for n, who in release(Api(os.environ["REPO"]), datetime.now(timezone.utc), dry):
            print(f"{'would release' if dry else 'released'} #{n} @{who}")
        sys.exit(0)
    sys.exit(__doc__)
