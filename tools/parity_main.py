#!/usr/bin/env python3
"""parity-main (.github/workflows/parity-main.yml): the vanilla harnesses run on main, a batch of merges at a time.

    python3 tools/parity_main.py base               # the commit this run checks from: main's last green parity-main
    python3 tools/parity_main.py bisect BASE        # after a failure: the first merge since BASE that broke a harness

`base` prints the newest commit on main whose parity-main run passed and that HEAD contains, else HEAD's first parent,
so a run tests every merge since the last green one (`tools/parity.py --changed <base>`), however many landed while
the previous run went on. `bisect` reads the harnesses that failed (.local/parity-runner/last-run.json), checks out
the first-parent merges between BASE and HEAD by halves and runs only those harnesses, then names, for each of them that
fails again at HEAD and passes at BASE, the first merge it fails on (written to $GITHUB_STEP_SUMMARY and, with --comment, on its pull request). HEAD is checked out again at the end.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
REPO = os.environ.get("GITHUB_REPOSITORY", "horizon-rust/horizon")
LAST_RUN = ROOT / ".local" / "parity-runner" / "last-run.json"


def git(*arguments, check=True):
    result = subprocess.run(["git", *arguments], cwd=ROOT, capture_output=True, text=True)
    if check and result.returncode:
        sys.exit(f"git {' '.join(arguments)}: {result.stderr.strip()}")
    return result


def green_runs():
    """Head commits of main's passing parity-main runs, newest first."""
    result = subprocess.run(["gh", "run", "list", "--repo", REPO, "--workflow", "parity-main", "--branch", "main",
                             "--status", "success", "--limit", "30", "--json", "headSha"], capture_output=True, text=True)
    if result.returncode:
        print(f"note: no parity-main history ({result.stderr.strip()})", file=sys.stderr)
        return []
    return [run["headSha"] for run in json.loads(result.stdout or "[]")]


def base(head="HEAD", runs=None):
    for sha in green_runs() if runs is None else runs:
        if git("merge-base", "--is-ancestor", sha, head, check=False).returncode == 0:
            return git("rev-parse", sha).stdout.strip()
    return git("rev-parse", f"{head}^1").stdout.strip()


def first_bad(commits, fails):
    """The first of commits (oldest first, the last known to fail; the one before them known to pass) where fails()."""
    good, bad = -1, len(commits) - 1
    while bad - good > 1:
        middle = (good + bad) // 2
        if fails(commits[middle]):
            bad = middle
        else:
            good = middle
    return commits[bad]


def harness_exists(name):
    return (ROOT / "tools" / f"test_{name.split('#')[0]}_parity.py").exists()


def run_harnesses(names, timeout):
    """The harnesses that fail at the checked-out commit (those it doesn't have yet can't), as parity.py records them."""
    present = [name for name in names if harness_exists(name)]
    if not present:
        return []
    if subprocess.run([sys.executable, "tools/parity.py", *present, "--timeout", str(timeout)], cwd=ROOT).returncode == 0:
        return []
    try:
        failed = json.loads(LAST_RUN.read_text(encoding="utf-8")).get("failed") or []
    except (OSError, ValueError):
        failed = []
    # A run that failed before recording its harnesses (a probe that doesn't build) fails them all.
    return [name for name in present if name in failed] or present


def bisect(since, timeout, comment):
    failed = []
    if LAST_RUN.exists():
        failed = json.loads(LAST_RUN.read_text(encoding="utf-8")).get("failed") or []
    if not failed:
        print("No failed harness recorded: nothing to bisect.")
        return 1
    head = git("rev-parse", "HEAD").stdout.strip()
    commits = git("rev-list", "--first-parent", "--reverse", f"{since}..{head}").stdout.split()
    if not commits:
        print("No merge since the base.")
        return 1

    def fails(commit, names):
        print(f"--- {commit[:9]} {git('log', '-1', '--format=%s', commit).stdout.strip()}", flush=True)
        git("checkout", "-q", "--detach", commit)
        return run_harnesses(names, timeout)

    def report(line):
        print(line)
        if os.environ.get("GITHUB_STEP_SUMMARY"):
            with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
                summary.write(f"### {line}\n")

    # Each harness is judged on its own. One that passes when run again at HEAD is a flake (counted as a pass); one
    # that fails at the base too is the runner's or a flake (no merge to blame: the search would end on the oldest
    # merge); the others fail at HEAD and pass at the base, so a merge broke them and the halving search names it.
    # Harnesses that break at different merges are searched again after the first one. The exit code is the run's
    # verdict: 0 only when every failed harness passes when run again at HEAD (the commit is good, so releases may
    # follow it), 1 otherwise.
    blames = []
    try:
        at_head = fails(head, failed)
        flakes = [name for name in failed if name not in at_head]
        if flakes:
            report(f"parity-main: {', '.join(flakes)} pass when run again at {head[:9]}: a flake, counted as a pass.")
        if not at_head:
            return 0
        at_base = fails(since, at_head)
        if at_base:
            report(f"parity-main: {', '.join(at_base)} fail at the base {since[:9]} too: the runner or a flake: "
                   "no merge is named.")
        remaining = [name for name in at_head if name not in at_base]
        # What failed where; a run of more harnesses also answers for fewer of them.
        tried = {head: set(at_head)}

        def failing(commit):
            if commit not in tried:
                tried[commit] = set(fails(commit, remaining))
            return tried[commit] & set(remaining)

        low = 0
        while remaining:
            span = commits[low:]
            culprit = first_bad(span, failing) if len(span) > 1 else span[0]
            hit = [name for name in remaining if name in failing(culprit)]
            blames.append((culprit, hit))
            remaining = [name for name in remaining if name not in hit]
            low = commits.index(culprit) + 1
    finally:
        git("checkout", "-q", "--detach", head)
    for culprit, hit in blames:
        subject = git("log", "-1", "--format=%s", culprit).stdout.strip()
        number = re.search(r"Merge pull request #(\d+)", subject)
        report(f"parity-main: {', '.join(hit)} first fail at {culprit[:9]} ({subject}), "
               f"of {len(commits)} merges since {since[:9]}.")
        if comment and number:
            url = f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{REPO}/actions/runs/{os.environ.get('GITHUB_RUN_ID', '')}"
            body = (f"🤖 Agent: parity-main found that this merge breaks {', '.join(f'`{name}`' for name in hit)} on main "
                    f"(bisected over {len(commits)} merges: {url}). Releases wait until parity-main passes again.")
            subprocess.run(["gh", "pr", "comment", number.group(1), "--repo", REPO, "--body", body])
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("base")
    bisecting = commands.add_parser("bisect")
    bisecting.add_argument("since", help="the last commit known to pass (the run's base)")
    bisecting.add_argument("--timeout", type=int, default=2400, help="seconds per harness")
    bisecting.add_argument("--comment", action="store_true", help="comment on the breaking merge's pull request")
    args = parser.parse_args()
    if args.command == "base":
        print(base())
        return 0
    return bisect(args.since, args.timeout, args.comment)


if __name__ == "__main__":
    sys.exit(main())
