#!/usr/bin/env python3
"""One comment on a pull request that names the other open pull requests doing the same work (issue #3859).

    overlap_note.py event               one pull request event (overlap-note.yml): writes, edits or clears the note
    overlap_note.py show <N>...         prints the note each pull request would get; changes nothing
    overlap_note.py survey              the notes of every open pull request, for measuring; changes nothing

Two pull requests overlap when they close the same issue, or change the same function of the same file: the function
is the one git names in the hunk header with its built-in `rust`, `python` and `java` drivers, which find indented
methods too (GitHub's own headers stop at `impl Game<'_>`). A hunk that only adds new functions doesn't count, and
neither do functions 5 of the compared pull requests change (constructors, the frame loop), harness probes and
tests, stacks (one head contains the other) or pull requests of more than 100 files (integration branches). Without a
shared issue, two pull requests must share 2 functions or more, and a tenth of the bigger one's. At most 5 listed.

The note is one comment, edited on every push and never repeated; with no overlap left it says so, and none is
written when there never was one. Nothing of a pull request runs: their heads are fetched as git objects and only
diffed (never checked out), with hooks and fsmonitor off; the rest comes from the API.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field

MARKER = "<!-- overlap-note -->"
MAX_LISTED = 5
MAX_FILES = 100
MAX_CANDIDATES = 40
HOT = 5
# On the 2026-10-29 survey (205 open pull requests) one shared function alone, or a few out of a big pull request's
# hundred, were mostly chance: 58 pairs, half unrelated. With both rules, 21 pairs (34 notes): 16 related
# (4 duplicates), 4 doubtful, 1 unrelated.
MIN_SHARED = 2
MIN_SHARE = 0.1  # of the bigger pull request's changed functions
SAFE_GIT = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "diff.external=",
            "-c", "core.quotePath=false"]
ATTRIBUTES = "*.rs diff=rust\n*.py diff=python\n*.java diff=java\n"
# Harness probes, tests and examples: everyone adds to the same lists there.
NOT_WORK = re.compile(r"/examples/|^tests/|/tests/|^tools/test_|_tests?\.(?:rs|py)$")
IGNORED = re.compile(r"(?:^|/)(?:Cargo\.lock|Cargo\.toml|lib\.rs|mod\.rs|main\.rs)$|\.(?:json|tsv|snap|png)$")
HUNK = re.compile(r"^@@ -\d+(?:,(\d+))? \+\d+(?:,\d+)? @@ ?(.*)$")
# A hunk that only adds lines whose first real one (past blanks, comments and attributes) declares an item: new code.
SKIP_LINE = re.compile(r"^\+\s*(?:$|//|#\[|#(?!\[)|@)")
NEW_ITEM = re.compile(r"^\+\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe|extern)\s+)*"
                      r"(?:fn|def|class|impl|struct|enum|trait|mod|macro_rules!)\b")
# Functions only: an `impl`, `class` or `mod` header would make every change in a big block the same "function".
NAME = re.compile(r"\b(?:fn|def)\s+([^\s(<{:]+)")


@dataclass
class Pull:
    number: int
    title: str
    author: str
    draft: bool
    head: str
    files: list[str]
    changed_files: int
    closes: set[int] = field(default_factory=set)
    funcs: set[tuple[str, str]] = field(default_factory=set)  # (path, function); filled for the ones compared


def functions(diff: str) -> set[tuple[str, str]]:
    """(path, function) for every hunk of a `git diff -U0` output that changes existing code inside a function."""
    out: set[tuple[str, str]] = set()
    path, header, removed, added = None, "", 0, []

    def close_hunk():
        first = next((line for line in added if not SKIP_LINE.match(line)), "")
        if path and header and (removed or not NEW_ITEM.match(first)):
            m = NAME.search(header)
            if m and not NOT_WORK.search(path):
                out.add((path, m.group(1)))

    for line in diff.splitlines():
        if line.startswith("diff --git "):
            close_hunk()
            path, header, removed, added = None, "", 0, []
        elif line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif (m := HUNK.match(line)):
            close_hunk()
            removed = 1 if m.group(1) is None else int(m.group(1))
            header, added = m.group(2), []
        elif line.startswith("+") and header:
            added.append(line)
    close_hunk()
    return out


def hot(pulls: list[Pull]) -> set[tuple[str, str]]:
    """Functions that HOT or more of these pull requests change (a constructor, the frame loop): sharing one says
    nothing about the work."""
    count: dict[tuple[str, str], int] = {}
    for p in pulls:
        for f in p.funcs:
            count[f] = count.get(f, 0) + 1
    return {f for f, n in count.items() if n >= HOT}


def overlaps(me: Pull, others: list[Pull], stacked=lambda a, b: False):
    """The other pull requests that overlap `me`, with the shared issues and functions, the most shared first.
    `others` are the ones compared (candidates()); `stacked(a, b)` says one head contains the other (a stack)."""
    common = hot([me, *others])
    found = []
    for o in others:
        if o.number == me.number or o.changed_files > MAX_FILES:
            continue
        issues, funcs = me.closes & o.closes, (me.funcs & o.funcs) - common
        if not issues and (len(funcs) < MIN_SHARED or len(funcs) < MIN_SHARE * max(len(me.funcs), len(o.funcs))):
            continue  # chance: a big pull request shares a little with everything
        if (issues or funcs) and not stacked(me, o):
            found.append((o, issues, funcs))
    found.sort(key=lambda t: (-len(t[1]) * 100 - len(t[2]), -t[0].number))
    return found[:MAX_LISTED]


def note(found) -> str | None:
    if not found:
        return None
    lines = [MARKER, "🤖 Overlap: these open pull requests touch the same work. Have a look, and talk to each other "
             "in the threads if it's the same change; nothing to do if it isn't."]
    for o, issues, funcs in found:
        what = []
        if issues:
            what.append("also closes " + ", ".join(f"#{i}" for i in sorted(issues)))
        if funcs:
            shown = sorted(funcs)[:4]
            more = f" and {len(funcs) - len(shown)} more" if len(funcs) > len(shown) else ""
            what.append("changes " + ", ".join(f"`{f}` in `{p.rsplit('/', 1)[-1]}`" for p, f in shown) + more)
        draft = " (draft)" if o.draft else ""
        lines.append(f"- #{o.number}{draft} by {o.author}: " + "; ".join(what))
    lines.append("\n<sub>Edited on every push; it lists at most 5. tools/ci/overlap_note.py</sub>")
    return "\n".join(lines)


CLEARED = MARKER + "\n🤖 Overlap: none any more."


# --- GitHub and git -----------------------------------------------------------------------------------------------


def gh(*args: str) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {r.stderr.strip()[:300]}")
    return r.stdout.strip()


def git(*args: str, check: bool = True) -> str:
    r = subprocess.run(["git", *SAFE_GIT, *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"git {args[0]}: {r.stderr.strip()[:300]}")
    return r.stdout


def has_commit(sha: str) -> bool:
    return bool(sha) and subprocess.run(["git", *SAFE_GIT, "cat-file", "-e", f"{sha}^{{commit}}"],
                                        capture_output=True).returncode == 0


OPEN_PULLS = """
query($owner: String!, $name: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(first: 50, states: OPEN, baseRefName: "main", after: $cursor) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title isDraft headRefOid changedFiles author { login }
        closingIssuesReferences(first: 10) { nodes { number } }
        files(first: 100) { nodes { path } }
      }
    }
  }
}"""


def open_pulls(repo: str) -> list[Pull]:
    owner, name = repo.split("/")
    out, cursor = [], None
    while True:
        args = ["api", "graphql", "-F", f"owner={owner}", "-F", f"name={name}", "-f", f"query={OPEN_PULLS}"]
        if cursor:
            args += ["-F", f"cursor={cursor}"]
        page = json.loads(gh(*args))["data"]["repository"]["pullRequests"]
        for n in page["nodes"]:
            out.append(Pull(n["number"], n["title"], (n["author"] or {}).get("login", "ghost"), n["isDraft"],
                            n["headRefOid"], [f["path"] for f in (n["files"] or {}).get("nodes", [])],
                            n["changedFiles"], {i["number"] for i in n["closingIssuesReferences"]["nodes"]}))
        if not page["pageInfo"]["hasNextPage"]:
            return out
        cursor = page["pageInfo"]["endCursor"]


def candidates(me: Pull, pulls: list[Pull]) -> list[Pull]:
    """The pull requests worth diffing against `me`: a shared file (lists and manifests left out) or issue."""
    mine = {p for p in me.files if not IGNORED.search(p)}
    scored = []
    for o in pulls:
        if o.number == me.number or o.changed_files > MAX_FILES:
            continue
        shared = len(mine & set(o.files))
        if shared or me.closes & o.closes:
            scored.append((shared + 100 * bool(me.closes & o.closes), o))
    scored.sort(key=lambda t: (-t[0], -t[1].number))
    return [o for _, o in scored[:MAX_CANDIDATES]]


def fill_functions(pulls: list[Pull], base: str = "origin/main") -> None:
    """Fetches the heads and fills each pull request's (path, function) set from its diff against main."""
    with tempfile.NamedTemporaryFile("w", suffix=".attributes", delete=False) as f:
        f.write(ATTRIBUTES)
    attrs = ["-c", f"core.attributesFile={f.name}"]
    try:
        # The workflow's clone is blobless: the fetch brings commits and trees, the diff only the blobs it reads.
        missing = [p.head for p in pulls if not has_commit(p.head)]
        if missing:  # a bare `git fetch origin` would fetch every branch
            git("fetch", "--quiet", "--no-tags", "origin", *missing, check=False)
        for p in pulls:
            if not has_commit(p.head):
                continue  # force-pushed away
            files = [x for x in p.files if not IGNORED.search(x)]
            if not files:
                continue
            diff = git(*attrs, "diff", "-U0", "--no-color", "--no-ext-diff", "--no-textconv", f"{base}...{p.head}",
                       "--", *files, check=False)
            p.funcs = functions(diff)
    finally:
        os.unlink(f.name)


def stacked(a: Pull, b: Pull) -> bool:
    """One head contains the other: a stack its author made on purpose."""
    return any(subprocess.run(["git", *SAFE_GIT, "merge-base", "--is-ancestor", x.head, y.head],
                              capture_output=True).returncode == 0 for x, y in ((a, b), (b, a)))


def compare(me: Pull, pulls: list[Pull]):
    others = candidates(me, pulls)
    fill_functions([me, *others])
    return overlaps(me, others, stacked)


class Api:
    def __init__(self, repo: str):
        self.repo = repo

    def own_note(self, n: int) -> int | None:
        raw = gh("api", "--paginate", f"repos/{self.repo}/issues/{n}/comments?per_page=100", "--jq",
                 f'.[] | select(.user.login == "github-actions[bot]" and (.body | startswith("{MARKER}"))) | .id')
        ids = raw.split()
        return int(ids[0]) if ids else None

    def write(self, n: int, comment: int | None, text: str) -> None:
        if comment:
            gh("api", "-X", "PATCH", f"repos/{self.repo}/issues/comments/{comment}", "-f", f"body={text}")
        else:
            gh("api", f"repos/{self.repo}/issues/{n}/comments", "-f", f"body={text}")


def event() -> int:
    with open(os.environ["GITHUB_EVENT_PATH"]) as f:
        number = json.load(f)["pull_request"]["number"]
    repo = os.environ["REPO"]
    pulls = open_pulls(repo)
    me = next((p for p in pulls if p.number == number), None)
    if me is None:
        return 0  # closed, or not against main
    text = note(compare(me, pulls))
    api = Api(repo)
    existing = api.own_note(number)
    if text is None and existing is None:
        return 0
    api.write(number, existing, text or CLEARED)
    return 0


if __name__ == "__main__":
    verb, rest = (sys.argv[1:2] or [""])[0], sys.argv[2:]
    if verb == "event" and not rest:
        sys.exit(event())
    if verb in ("show", "survey"):
        pulls = open_pulls(os.environ.get("REPO", "horizon-rust/horizon"))
        wanted = {int(x) for x in rest} if verb == "show" else {p.number for p in pulls}
        fill_functions([p for p in pulls if p.changed_files <= MAX_FILES])
        result = {}
        for me in pulls:
            if me.number in wanted:
                found = overlaps(me, candidates(me, pulls), stacked)
                result[me.number] = [(o.number, sorted(i), sorted(f), len(me.funcs), len(o.funcs))
                                     for o, i, f in found]
                if verb == "show":
                    print(f"#{me.number}:\n{note(found) or '(no note)'}\n")
        if verb == "survey":
            print(json.dumps(result))
        sys.exit(0)
    sys.exit(__doc__)
