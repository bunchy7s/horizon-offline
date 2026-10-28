#!/usr/bin/env python3
"""Trusted reviewers pick the pull requests a Nightly holds (docs/release-flow.md "Nightly channel").

    nightly_set.py command     one `/nightly …` comment (nightly-command.yml): checks who asked, updates the set,
                               starts the build and answers in the thread

A comment by Ludovic or by a login in .github/trusted-reviewers.txt, on any issue or pull request:

    /nightly 2640 2642     the set becomes these pull requests, merged onto main in this order; builds now
    /nightly add 2650      adds one, or takes its newest head again; builds now
    /nightly remove 2640   drops one; builds now. Ludovic's remove also keeps reviewers from adding it back
    /nightly clear         empties the set: the Nightly is the proven union or main again; builds now
    /nightly status        what the set holds
    /nightly dry-run 2640  builds main + these and signs with a throwaway key; publishes nothing, the set stays

The set (set.json on the nightly-state branch) keeps each pull request at the head it had when someone asked for
it; the scheduled nightly takes their newest heads and drops the merged and closed ones (nightly.py pick).
builds.jsonl there records every build's set, for the stats. Nothing here runs a pull request's code: its commits
are fetched and merged with `git merge-tree --write-tree`, which writes git objects and never a working tree, so no
file of theirs reaches the disk and no hook, filter or script of theirs runs.
"""
from __future__ import annotations

import copy
import datetime as dt
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

OWNER = "ludovic111"
REVIEWERS = ".github/trusted-reviewers.txt"
CODEOWNERS = ".github/CODEOWNERS"
# What CODEOWNERS guards today, refused even if the file moved: what builds, signs and checks releases.
ALWAYS_GUARDED = ["/.github/", "/tools/ci/", "/crates/horizon-release-sign/", "/crates/horizon-server/src/update.rs",
                  "/launcher/src/backend.rs", "/launcher/links.json"]
STATE_BRANCH = "nightly-state"
BUILD_PREFIX = "nightly/build-"
MAX_PRS = 30
MAX_BUILDS_PER_DAY = 12
BOT = ("github-actions[bot]", "41898283+github-actions[bot]@users.noreply.github.com")
# Git never runs anything of the merged commits; these make sure nothing of the runner's either.
SAFE_GIT = ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", "-c", "merge.renormalize=false"]
NUMBER = re.compile(r"^#?(\d{1,6}),?$")
LOGIN = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$")
VERBS = ("add", "remove", "clear", "status", "dry-run")
USAGE = ("Use `/nightly 2640 2642`, `/nightly add 2650`, `/nightly remove 2640`, `/nightly clear`, "
         "`/nightly status` or `/nightly dry-run 2640`.")


class Refused(Exception):
    """A command that changes nothing; its text is the answer."""


@dataclass
class Command:
    verb: str  # set, add, remove, clear, status, dry-run
    prs: list[int] = field(default_factory=list)

    def text(self) -> str:
        words = ["/nightly"] + ([] if self.verb == "set" else [self.verb]) + [str(n) for n in self.prs]
        return " ".join(words)


def safe(word: str) -> str:
    """A word of a comment, fit to quote back: no markdown, no mentions."""
    return re.sub(r"[^\w#:.-]", "", word)[:40] or "?"


def parse(body: str) -> Command | None:
    """The command on a comment's first line; None when it isn't one. Raises Refused when it is one, but wrong."""
    line = next((line.strip() for line in body.splitlines() if line.strip()), "")
    words = line.split()
    if not words or words[0].lower() != "/nightly":
        return None
    rest = words[1:]
    verb = "status" if not rest else "set"
    if rest and rest[0].lower() in VERBS:
        verb = rest.pop(0).lower()
    prs: list[int] = []
    for word in rest:
        m = NUMBER.match(word)
        if not m:
            raise Refused(f"`{safe(word)}` isn't a pull request number. {USAGE}")
        if (n := int(m.group(1))) not in prs:
            prs.append(n)
    if verb in ("clear", "status") and prs:
        raise Refused(f"`/nightly {verb}` takes no numbers. {USAGE}")
    if verb not in ("clear", "status") and not prs:
        raise Refused(f"`/nightly {'' if verb == 'set' else verb + ' '}` needs pull request numbers. {USAGE}")
    if len(prs) > MAX_PRS:
        raise Refused(f"A Nightly holds at most {MAX_PRS} pull requests; that's {len(prs)}.")
    return Command(verb, prs)


@dataclass
class Reviewers:
    logins: set[str]  # lowercase
    paused: bool


def read_reviewers(text: str) -> Reviewers:
    """The list file: one login per line, `#` comments, and a `paused` line that stops every reviewer."""
    logins, paused = set(), False
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        words = line.split()
        if len(words) != 1:  # not a login: ignored rather than half-read
            continue
        word = words[0]
        if word.lower() == "paused":
            paused = True
        elif LOGIN.match(word):
            logins.add(word.lower())
    return Reviewers(logins, paused)


def is_owner(login: str) -> bool:
    return login.lower() == OWNER


def authorize(login: str, reviewers: Reviewers) -> None:
    """Ludovic always; a listed reviewer unless the list is paused; nobody else."""
    if is_owner(login):
        return
    if login.lower() not in reviewers.logins:
        raise Refused(f"Only Ludovic and the trusted reviewers listed in `{REVIEWERS}` can pick the Nightly.")
    if reviewers.paused:
        raise Refused(f"`/nightly` is paused: Ludovic stopped it in `{REVIEWERS}`.")


def guarded_patterns(codeowners: str) -> list[str]:
    """The paths CODEOWNERS gives to Ludovic, plus ALWAYS_GUARDED."""
    found = [line.split()[0] for line in codeowners.splitlines() if line.strip().startswith("/")]
    return sorted(set(found + ALWAYS_GUARDED))


def guarded(path: str, patterns: list[str]) -> bool:
    p = "/" + path.lstrip("/")
    return any(p.startswith(pat) if pat.endswith("/") else p == pat for pat in patterns)


@dataclass
class PullRequest:
    number: int
    state: str  # open, closed, merged
    head: str
    title: str
    author: str
    base: str
    files: list[str]


def check(pr: PullRequest | None, number: int, patterns: list[str]) -> str | None:
    """Why `pr` can't go in a Nightly, or None."""
    if pr is None:
        return f"#{number} isn't a pull request here"
    if pr.state != "open":
        return f"#{number} is {pr.state}"
    if pr.base != "main":
        return f"#{number} targets `{safe(pr.base)}`, not main"
    bad = [f for f in pr.files if guarded(f, patterns)]
    if bad:
        more = f" and {len(bad) - 1} more" if len(bad) > 1 else ""
        return f"#{number} changes `{bad[0]}`{more}, which only Ludovic merges (CODEOWNERS)"
    return None


def entry(pr: PullRequest, asked_by: str, now: str) -> dict:
    return {"number": pr.number, "head": pr.head, "title": pr.title, "author": pr.author, "asked_by": asked_by,
            "asked_at": now}


@dataclass
class Result:
    state: dict
    answer: str
    build: bool
    dry_run: list[dict] | None = None  # the set a dry run builds; the saved set stays


def plain(text: str) -> str:
    """A title fit for a comment or the notes: no mention, no markdown that spans lines."""
    return text.replace("@", "@\u200b").replace("\n", " ").strip()[:200]


def describe(prs: list[dict], heading: str = "The set") -> str:
    if not prs:
        return "The set is empty: the Nightly is the newest proven union, or main."
    lines = [f"{heading} ({len(prs)}), merged onto main in this order:"]
    lines += [f"- #{e['number']} {plain(e['title'])} (by {e['author']}, at {e['head'][:7]}, asked by "
              f"{e['asked_by']})" for e in prs]
    return "\n".join(lines)


def apply(state: dict, cmd: Command, login: str, now: str, lookup: Callable[[int], PullRequest | None],
          patterns: list[str]) -> Result:
    """What `cmd` by `login` makes of `state` (set.json). Pure but for `lookup`; raises Refused."""
    st = copy.deepcopy(state)
    prs: list[dict] = st.setdefault("prs", [])
    blocked: list[int] = st.setdefault("blocked", [])
    owner = is_owner(login)
    if cmd.verb == "status":
        extra = f"\nOnly Ludovic can add these back: {', '.join(f'#{n}' for n in blocked)}." if blocked else ""
        return Result(state, describe(prs) + extra, False)
    if cmd.verb == "clear":
        st["prs"] = []
    elif cmd.verb == "remove":
        absent = [n for n in cmd.prs if n not in [e["number"] for e in prs]]
        if absent and len(absent) == len(cmd.prs) and not owner:
            raise Refused(f"{', '.join(f'#{n}' for n in absent)} isn't in the set. " + describe(prs))
        st["prs"] = [e for e in prs if e["number"] not in cmd.prs]
        if owner:
            st["blocked"] = blocked + [n for n in cmd.prs if n not in blocked]
    else:  # set, add, dry-run
        if not owner and (held := [n for n in cmd.prs if n in blocked]):
            raise Refused(f"Ludovic took {', '.join(f'#{n}' for n in held)} out of the Nightly; only he can put "
                          "it back.")
        problems, asked = [], []
        for n in cmd.prs:
            pr = lookup(n)
            if why := check(pr, n, patterns):
                problems.append(why)
            else:
                asked.append(entry(pr, login, now))
        if problems:
            raise Refused("Nothing changed: " + "; ".join(problems) + ".")
        if cmd.verb == "add":
            fresh = {e["number"]: e for e in asked}
            new = [fresh.pop(e["number"], e) for e in prs] + list(fresh.values())  # keeps its place, newest head
        else:
            new = asked
        if len(new) > MAX_PRS:
            raise Refused(f"A Nightly holds at most {MAX_PRS} pull requests; that would be {len(new)}.")
        if cmd.verb == "dry-run":
            return Result(state, describe(new, "Dry run, the saved set stays as it is. It builds"), True,
                          dry_run=new)
        if owner:
            st["blocked"] = [n for n in blocked if n not in cmd.prs]
        st["prs"] = new
    return Result(st, describe(st["prs"]), True)


def log_command(state: dict, cmd: Command, login: str, now: str, issue: str, url: str) -> dict:
    st = copy.deepcopy(state)
    st["log"] = (st.get("log", []) + [{"at": now, "by": login, "command": cmd.text(), "issue": issue,
                                       "url": url}])[-50:]
    return st


# --- git -------------------------------------------------------------------------------------------------------


def git(*args: str, cwd: str | None = None, env: dict | None = None, check: bool = True,
        input: str | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *SAFE_GIT, *args], cwd=cwd, env={**os.environ, **(env or {})}, check=check,
                          capture_output=True, text=True, input=input)


def out(*args: str, cwd: str | None = None, env: dict | None = None) -> str:
    return git(*args, cwd=cwd, env=env).stdout.strip()


def identity(date: str) -> dict:
    """The bot's name, and a fixed date so the same inputs give the same commit."""
    name, email = BOT
    return {"GIT_AUTHOR_NAME": name, "GIT_AUTHOR_EMAIL": email, "GIT_COMMITTER_NAME": name,
            "GIT_COMMITTER_EMAIL": email, "GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date}


def merge_tree(a: str, b: str, cwd: str | None = None) -> tuple[str | None, list[str]]:
    """The merged tree of commits a and b, or None and the conflicted paths. Objects only, no working tree."""
    r = git("merge-tree", "--write-tree", "--name-only", "--no-messages", a, b, cwd=cwd, check=False)
    if r.returncode == 0:
        return r.stdout.split()[0], []
    if r.returncode == 1:
        return None, [line for line in r.stdout.splitlines()[1:] if line.strip()]
    raise RuntimeError(f"git merge-tree {a[:9]} {b[:9]}: {r.stderr.strip()}")


def commit_tree(tree: str, parents: list[str], message: str, date: str, cwd: str | None = None) -> str:
    args = ["commit-tree", tree]
    for p in parents:
        args += ["-p", p]
    return out(*args, "-m", message, cwd=cwd, env=identity(date))


@dataclass
class Conflict(Exception):
    number: int
    other: int | None  # the earlier pull request it conflicts with; None: main; 0: only with several together
    files: list[str]

    def text(self) -> str:
        where = ", ".join(f"`{f}`" for f in self.files[:8]) + (" and more" if len(self.files) > 8 else "")
        if self.other is None:
            return (f"#{self.number} conflicts with main in {where}: it needs main merged into it first.")
        if self.other == 0:
            return (f"#{self.number} conflicts with the pull requests before it, taken together, in {where}.")
        return (f"#{self.other} and #{self.number} conflict in {where}. Make an integration pull request that "
                "combines them (in your fork) and pick that one instead.")


def changed_files(base: str, head: str, cwd: str | None = None) -> list[str]:
    """What `head` changes since it left `base`, renames' old paths included."""
    since = out("merge-base", base, head, cwd=cwd)
    names = out("diff", "--name-status", "--no-renames", since, head, cwd=cwd)
    return [line.split("\t", 1)[1] for line in names.splitlines() if "\t" in line]


def culprit(main: str, earlier: list[tuple[int, str]], head: str, date: str, cwd: str | None) -> int | None:
    if merge_tree(main, head, cwd)[0] is None:
        return None
    for n, h in earlier:
        tree, _ = merge_tree(main, h, cwd)
        if tree is None:
            continue
        if merge_tree(commit_tree(tree, [main, h], "probe", date, cwd), head, cwd)[0] is None:
            return n
    return 0


def assemble(main: str, heads: list[tuple[int, str]], cwd: str | None = None) -> str:
    """main, then each pull request's head merged in order: the commit a Nightly builds. Raises Conflict."""
    date = out("show", "-s", "--format=%cI", main, cwd=cwd)
    current = main
    for i, (n, head) in enumerate(heads):
        tree, files = merge_tree(current, head, cwd)
        if tree is None:
            raise Conflict(n, culprit(main, heads[:i], head, date, cwd), files)
        current = commit_tree(tree, [current, head], f"Nightly: merge #{n} at {head[:12]}", date, cwd)
    return current


def fetch_heads(heads: list[tuple[int, str]], cwd: str | None = None) -> list[int]:
    """Fetches each head from origin; returns the numbers whose head is gone (force-pushed away and collected)."""
    shas = [h for _, h in heads]
    if shas and git("fetch", "--quiet", "--no-tags", "origin", *shas, cwd=cwd, check=False).returncode == 0:
        return []
    gone = []
    for n, h in heads:
        if git("cat-file", "-e", f"{h}^{{commit}}", cwd=cwd, check=False).returncode == 0:
            continue
        git("fetch", "--quiet", "--no-tags", "origin", h, cwd=cwd, check=False)
        if git("cat-file", "-e", f"{h}^{{commit}}", cwd=cwd, check=False).returncode != 0:
            gone.append(n)
    return gone


def auth(token: str) -> list[str]:
    """A `git -c` for pushing with the job's token, never written to a file or a log."""
    import base64
    basic = base64.b64encode(f"x-access-token:{token}".encode()).decode()
    os.environ.setdefault("GIT_TERMINAL_PROMPT", "0")
    return ["-c", f"http.https://github.com/.extraheader=AUTHORIZATION: basic {basic}"]


def push(sha: str, ref: str, token: str, cwd: str | None = None, force: bool = False) -> bool:
    spec = f"{'+' if force else ''}{sha}:refs/heads/{ref}"
    return git(*auth(token), "push", "--quiet", "--no-verify", "origin", spec, cwd=cwd, check=False).returncode == 0


def builds_today(today: str, cwd: str | None = None) -> list[str]:
    listed = out("ls-remote", "origin", f"refs/heads/{BUILD_PREFIX}{today}-*", cwd=cwd)
    return [line.split("refs/heads/", 1)[1] for line in listed.splitlines() if "refs/heads/" in line]


def next_build(today: str, existing: list[str]) -> str:
    taken = [int(m.group(1)) for b in existing if (m := re.search(r"-(\d+)$", b))]
    return f"{BUILD_PREFIX}{today}-{max(taken, default=0) + 1}"


# --- the state branch ------------------------------------------------------------------------------------------


def load_state(cwd: str | None = None) -> tuple[dict, str, str | None]:
    """set.json, builds.jsonl and the branch's head (None before the first command)."""
    if git("fetch", "--quiet", "--no-tags", "origin", f"+refs/heads/{STATE_BRANCH}:refs/nightly/state", cwd=cwd,
           check=False).returncode != 0:
        return {}, "", None
    head = out("rev-parse", "refs/nightly/state", cwd=cwd)
    state = json.loads(git("show", f"{head}:set.json", cwd=cwd, check=False).stdout or "{}")
    builds = git("show", f"{head}:builds.jsonl", cwd=cwd, check=False).stdout
    return state, builds, head


def write_state(state: dict, builds: str, parent: str | None, message: str, cwd: str | None = None) -> str:
    """A commit of the state branch, made in a private index: the checkout isn't touched."""
    with tempfile.TemporaryDirectory() as tmp:
        env = {"GIT_INDEX_FILE": str(Path(tmp) / "index")}
        for name, text in (("set.json", json.dumps(state, indent=2) + "\n"), ("builds.jsonl", builds),
                           ("README.md", STATE_README)):
            blob = git("hash-object", "-w", "--stdin", cwd=cwd, input=text).stdout.strip()
            git("update-index", "--add", "--cacheinfo", f"100644,{blob},{name}", cwd=cwd, env=env)
        tree = out("write-tree", cwd=cwd, env=env)
    now = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    return commit_tree(tree, [parent] if parent else [], message, now, cwd)


STATE_README = """# nightly-state

Written by tools/ci/nightly_set.py and tools/ci/nightly.py, never by hand. `set.json`: the pull requests the trusted
reviewers picked for the Nightly (docs/release-flow.md "Nightly channel"), each at the head it was asked at.
`builds.jsonl`: one line per Nightly build with its set.
"""


def update_state(change: Callable[[dict, str], tuple[dict, str, str] | None], token: str,
                 cwd: str | None = None, tries: int = 5):
    """Applies change(state, builds) -> (state, builds, message), or None for no change, and pushes it; when another
    run pushed first, reads the branch again and applies it again. Returns the last state."""
    for _ in range(tries):
        state, builds, head = load_state(cwd)
        changed = change(state, builds)
        if changed is None:
            return state
        new, new_builds, message = changed
        if push(write_state(new, new_builds, head, message, cwd), STATE_BRANCH, token, cwd):
            return new
        time.sleep(2)
    raise RuntimeError(f"could not update {STATE_BRANCH} after {tries} tries")


# --- GitHub ----------------------------------------------------------------------------------------------------


def gh(*args: str, check: bool = True) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {r.stderr.strip()[:300]}")
    return r.stdout.strip() if r.returncode == 0 else ""


def lookup(repo: str) -> Callable[[int], PullRequest | None]:
    def get(n: int) -> PullRequest | None:
        raw = gh("api", f"repos/{repo}/pulls/{n}", check=False)
        if not raw:
            return None
        p = json.loads(raw)
        files = gh("api", "--paginate", f"repos/{repo}/pulls/{n}/files", "--jq",
                   ".[] | .filename, (.previous_filename // empty)").splitlines()
        state = "merged" if p.get("merged_at") else p["state"]
        return PullRequest(n, state, p["head"]["sha"], p["title"], p["user"]["login"], p["base"]["ref"], files)
    return get


def answer(repo: str, issue: str, text: str) -> None:
    gh("api", f"repos/{repo}/issues/{issue}/comments", "-f", f"body=🤖 Nightly: {text}")


def dispatch(repo: str, issue: str, request: str, dry_run: list[dict] | None) -> str:
    """Starts `nightly build` for this request; returns its run's URL once GitHub lists it."""
    args = ["workflow", "run", "nightly-build.yml", "--repo", repo, "--ref", "main", "-f", f"issue={issue}",
            "-f", f"request={request}", "-f", "force=true"]
    if dry_run is not None:
        args += ["-f", "dry_run=true", "-f", "prs=" + " ".join(f"{e['number']}@{e['head']}" for e in dry_run)]
    gh(*args)
    for _ in range(15):
        time.sleep(4)
        runs = json.loads(gh("run", "list", "--repo", repo, "--workflow", "nightly-build.yml", "--event",
                             "workflow_dispatch", "--limit", "20", "--json", "displayTitle,url", check=False) or "[]")
        for r in runs:
            if request in r["displayTitle"]:
                return r["url"]
    return f"https://github.com/{repo}/actions/workflows/nightly-build.yml"


def command() -> None:
    """nightly-command.yml: the comment arrives in env vars, never in a shell line."""
    repo, issue = os.environ["GITHUB_REPOSITORY"], os.environ["ISSUE"]
    login, body, url = os.environ["AUTHOR"], os.environ.get("BODY", ""), os.environ.get("COMMENT_URL", "")
    token = os.environ["GH_TOKEN"]
    try:
        cmd = parse(body)
    except Refused as e:
        answer(repo, issue, str(e))
        return
    if cmd is None:
        return
    now = dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds")
    try:
        reviewers = Path(REVIEWERS).read_text() if Path(REVIEWERS).exists() else ""
        authorize(login, read_reviewers(reviewers))
        patterns = guarded_patterns(Path(CODEOWNERS).read_text() if Path(CODEOWNERS).exists() else "")
        if cmd.verb != "status":
            today = now[:10]
            done = builds_today(today)
            if len(done) >= MAX_BUILDS_PER_DAY:
                raise Refused(f"{len(done)} Nightly builds today already, the most a day takes: the next scheduled "
                              "one (20:17 UTC) builds the set.")
        results: list[Result] = []

        def change(state: dict, builds: str):
            result = apply(state, cmd, login, now, lookup(repo), patterns)
            results.append(result)
            if result.state is state:  # status, dry run: nothing to save
                return None
            return log_command(result.state, cmd, login, now, issue, url), builds, f"{cmd.text()} by {login}"

        update_state(change, token)
        result = results[-1]
    except Refused as e:
        answer(repo, issue, str(e))
        return
    text = result.answer
    if result.build:
        run = dispatch(repo, issue, f"{issue}-{int(time.time())}", result.dry_run)
        text += f"\n\nBuilding: {run}. I'll answer here when it's {'done' if result.dry_run else 'published'}."
    answer(repo, issue, text)


def main() -> None:
    if sys.argv[1:] == ["command"]:
        command()
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
