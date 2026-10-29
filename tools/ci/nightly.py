#!/usr/bin/env python3
"""The Nightly, the only build Horizon ships (.github/workflows/nightly-build.yml, docs/release-flow.md "Nightly").

    nightly.py pick [--force]      which commit tonight's nightly builds: GitHub outputs and nightly-notes.md
    nightly.py rotate              renames the current `nightly` pre-release to nightly-<its date>, frees the tag
    nightly.py prune [--days 7]    deletes the nightly-<date> pre-releases and tags older than that, nothing else
    nightly.py prune-builds [--days 7]   deletes the nightly/build-<date>-<n> branches older than that, but the
                                   newest and any a published nightly points to (Ludovic, 2026-10-27)
    nightly.py keep-source <sha> <date>  tags the nightly's commit nightly-source/<date>-<short>, for good

A nightly builds main plus the pull requests the trusted reviewers picked (the set: tools/ci/nightly_set.py), merged
in their order and pushed as nightly/build-<date>-<n>; with no set, main alone. Main is its newest commit where
`checks` and `parity-main` passed, as for a release (release_train.green_main_head), or NIGHTLY_BASE when the caller
gives that commit. It is published as the `nightly` pre-release; the previous one stays a week as nightly-<date>.
Its notes list what reached main since the previous nightly, then the picked pull requests. Runs from main's
checkout with `gh`; it reads and merges the branches with git and never runs anything from them.

Each nightly ships its exact source: horizon-source.tar.gz (`git archive` of the commit, signed with the builds) on
the release, and the tag nightly-source/<date>-<short>, which no prune deletes. Native mods pin a nightly's commit
(docs/native-mods.md), so that commit must stay downloadable after its release and build branch are gone.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

if __package__:  # imported as tools.ci.nightly (the tests)
    from . import nightly_set as ns
    from .. import release_train as train
else:  # run as tools/ci/nightly.py: its folder is on the path
    import nightly_set as ns
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    import release_train as train

DATED = re.compile(r"^nightly-(\d{4}-\d{2}-\d{2})$")
TITLE_DATE = re.compile(r"Nightly (\d{4}-\d{2}-\d{2})")
SOURCE_TAG = "nightly-source/"


@dataclass
class Ref:
    name: str
    sha: str
    date: dt.datetime  # committer date


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def gh(*args: str, check: bool = True) -> str:
    out = subprocess.run(["gh", *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        sys.exit(f"gh {' '.join(args)}: {out.stderr.strip()}")
    return out.stdout.strip() if out.returncode == 0 else ""


def is_ancestor(a: str, b: str) -> bool:
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b]).returncode == 0


def output(**values: str) -> None:
    with open(os.environ.get("GITHUB_OUTPUT", "/dev/stdout"), "a") as f:
        for k, v in values.items():
            f.write(f"{k}={v}\n")


PLAYERS = ("The launcher installs it by itself; your worlds stay as they are. A Nightly can include pull requests "
           "the Team picked before they are merged (below). Report bugs with the version the game shows "
           "(\"Nightly <date> · <commit>\").")


def source_tag(today: str, sha: str) -> str:
    """The permanent tag of a nightly's commit: neither `prune` (nightly-<date> only) nor `prune-builds` touches it."""
    return f"{SOURCE_TAG}{today}-{sha[:7]}"


def source_line(today: str, sha: str) -> str:
    return (f"Source: commit {sha}, kept for good as the tag `{source_tag(today, sha)}`; horizon-source.tar.gz is "
            "its tree. Native mods built for this Nightly pin that commit.")


def notes(today: str, chosen: Ref, main: Ref, picked: list[dict], left_out: list[str], merged: list[dict],
          issues: list[str]) -> str:
    """A nightly's notes: what reached main since the previous nightly (New, Fixed, as a release's notes), then the
    reviewers' picks, the known issues and the thanks."""
    shown = [pr for pr in merged if pr["paths"] and "skip-release" not in pr["labels"]]
    new = [pr for pr in shown if "feature" in pr["labels"]]
    fixed = [pr for pr in shown if "feature" not in pr["labels"]]
    built = f"main ({main.sha[:7]})" + (f" with {len(picked)} pull requests merged in" if picked else "")
    parts = [f"Nightly {today} · {chosen.sha[:7]}: {built}, built from `{chosen.name}` ({chosen.sha}).",
             source_line(today, chosen.sha), PLAYERS]
    if not shown and not picked:
        parts.append("Nothing new for players on main since the previous Nightly.")
    if new:
        parts.append("### New\n" + "\n".join(map(train.bullet, new)))
    if fixed:
        parts.append("### Fixed\n" + "\n".join(map(train.bullet, fixed)))
    if picked:
        parts.append(f"### Not merged yet ({len(picked)})\n" + "\n".join(
            f"- {ns.plain(e['title'])} (#{e['number']}) by {e['author']}, at {e['head'][:7]}, picked by {e['asked_by']}"
            for e in picked))
    if left_out:
        parts.append(f"### Left out ({len(left_out)})\n" + "\n".join(f"- {ns.plain(why)}" for why in left_out))
    if issues:
        parts.append("### Known issues\n" + "\n".join(issues))
    contributors = sorted({pr["author"] for pr in shown if pr["author"] and pr["author"] != train.MAINTAINER})
    if contributors:
        parts.append("Thanks to " + ", ".join(f"@{name}" for name in contributors) + ".")
    parts.append(train.DISCLAIMER)
    return "\n\n".join(parts) + "\n"


def merged_since(previous: str | None, main: str) -> list[dict]:
    """The pull requests merged into main since the previous nightly (its commit holds the main it was built on)."""
    if not previous or subprocess.run(["git", "cat-file", "-e", f"{previous}^{{commit}}"]).returncode != 0:
        return []
    return train.merged_between("game", previous, main)


def refresh(entries: list[dict], get, patterns: list[str], team: set[str]) -> tuple[list[dict], list[str]]:
    """The scheduled nightly's set: each pull request at its newest approved head (the Team's or Ludovic's agents'
    approval, ns.approvers); a newer head nobody approved yet waits, merged, closed and refused ones are out."""
    kept, left_out = [], []
    for e in entries:
        pr = get(e["number"])
        if pr is not None and pr.state == "merged":
            left_out.append(f"#{e['number']} {e['title']}: merged, so it's in main")
        elif why := ns.check(pr, e["number"], patterns):
            left_out.append(f"{why}: left out")
        elif ns.approvers(pr, pr.head, team):
            kept.append({**e, "head": pr.head, "title": pr.title})
        elif ns.approvers(pr, e["head"], team):
            kept.append(e)
            left_out.append(f"#{e['number']} {e['title']}: its new head {pr.head[:7]} isn't approved yet, so the "
                            f"approved {e['head'][:7]} stays")
        else:
            left_out.append(f"#{e['number']} {e['title']}: no approving review from the Team or Ludovic's agents at "
                            "its head: left out")
    return kept, left_out


def requested(spec: str, get) -> list[dict]:
    """A dry run's set from its input: `2640@<head> 2642@<head>`."""
    prs = []
    for word in spec.split():
        number, _, head = word.partition("@")
        pr = get(int(number))
        prs.append({"number": int(number), "head": head or (pr.head if pr else ""), "title": pr.title if pr else "?",
                    "author": pr.author if pr else "?", "asked_by": "dry run"})
    return prs


def stop(repo: str, issue: str, text: str) -> None:
    """Says why the build stopped, where it was asked for, and fails the job."""
    print(text)
    if issue and repo:
        ns.answer(repo, issue, text)
    sys.exit(1)


def build_set(main: Ref, prs: list[dict], left_out: list[str], today: str, force: bool) -> None:
    """main + the set, merged with git only (nightly_set.assemble), pushed as nightly/build-<date>-<n>."""
    repo, issue = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("NIGHTLY_ISSUE", "")
    token, event = os.environ.get("GH_TOKEN", ""), os.environ.get("GITHUB_EVENT_NAME", "")
    dry_run = os.environ.get("DRY_RUN") == "true"
    patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
    gone = set(ns.fetch_heads([(e["number"], e["head"]) for e in prs]))
    kept = []
    for e in prs:
        if e["number"] in gone:
            left_out.append(f"#{e['number']} {e['title']}: its head {e['head'][:7]} is gone; ask for it again")
        elif bad := [f for f in ns.changed_files(main.sha, e["head"]) if ns.guarded(f, patterns)]:
            left_out.append(f"#{e['number']} {e['title']}: changes `{bad[0]}`, which only Ludovic merges")
        else:
            kept.append(e)
    try:
        sha = ns.assemble(main.sha, [(e["number"], e["head"]) for e in kept])
    except ns.Conflict as c:
        state, _, _ = ns.load_state()
        asked_in = issue or next((x["issue"] for x in reversed(state.get("log", [])) if x.get("issue")), "")
        stop(repo, asked_in, f"the build stopped: {c.text()}")
    previous = git("ls-remote", "--tags", "origin", "refs/tags/nightly").split("\t")[0] or None
    build = force or previous != sha
    name = "(unchanged)"
    if build:
        done = ns.builds_today(today)
        if len(done) >= ns.MAX_BUILDS_PER_DAY and event != "schedule" and os.environ.get("NIGHTLY_URGENT") != "true":
            stop(repo, issue, f"{len(done)} Nightly builds today already, the most a day takes: the next scheduled "
                              "one (20:17 UTC) builds the set.")
        name = ns.next_build(today, done)
        if not ns.push(sha, name, token):
            stop(repo, issue, f"could not push {name}")
        record = {"ref": name, "sha": sha, "main": main.sha, "at": dt.datetime.now(dt.timezone.utc).isoformat(
            timespec="seconds"), "event": event, "dry_run": dry_run, "run": os.environ.get("GITHUB_RUN_ID", ""),
            "prs": [{k: e[k] for k in ("number", "head", "title", "author", "asked_by")} for e in kept],
            "left_out": left_out}
        ns.update_state(lambda st, builds: (st, builds + json.dumps(record) + "\n", f"build {name}"), token)
    chosen = Ref(name, sha, dt.datetime.now(dt.timezone.utc))
    with open("nightly-notes.md", "w") as f:
        f.write(notes(today, chosen, main, kept, left_out, merged_since(previous, main.sha), train.known_issues()))
    output(build=str(build).lower(), sha=sha, short=sha[:7], source=name, date=today, version=f"nightly-{today}",
           nightly=f"{today} {sha[:7]}", source_tag=source_tag(today, sha))
    print(f"{'build' if build else 'skip (unchanged since the last nightly)'}: {name} {sha}, main {main.sha[:7]}"
          + "".join(f" + #{e['number']}" for e in kept))


def green_base(repo: str, issue: str) -> Ref:
    """The main commit a nightly builds on: NIGHTLY_BASE, else main's newest commit where checks and parity-main
    passed (what releases ship), so a merge that breaks main holds nightlies too."""
    sha = os.environ.get("NIGHTLY_BASE", "").strip() or train.green_main_head(parity=True)
    if not sha:
        stop(repo, issue, "the build stopped: main's newest checks or parity-main run did not pass, so no commit of "
                          "main is green to build on. The next build after they pass takes it.")
    if not is_ancestor(sha, "refs/remotes/origin/main"):
        stop(repo, issue, f"the build stopped: {sha[:9]} is not on main.")
    sha = git("rev-parse", sha)
    return Ref("main", sha, dt.datetime.fromisoformat(git("show", "-s", "--format=%cI", sha)))


def pick(force: bool) -> None:
    git("fetch", "--quiet", "origin", "+refs/heads/main:refs/remotes/origin/main", "+refs/tags/nightly:refs/tags/nightly")
    today = dt.datetime.now(dt.timezone.utc).date().isoformat()
    repo, token = os.environ.get("GITHUB_REPOSITORY", ""), os.environ.get("GH_TOKEN", "")
    main = green_base(repo, os.environ.get("NIGHTLY_ISSUE", ""))
    if spec := os.environ.get("NIGHTLY_PRS", "").strip():  # a dry run's own set; the saved one stays
        return build_set(main, requested(spec, ns.lookup(repo)), [], today, force)
    state, _, _ = ns.load_state()
    prs, left_out = state.get("prs", []), []
    if prs and os.environ.get("GITHUB_EVENT_NAME") == "schedule":
        patterns = ns.guarded_patterns(Path(ns.CODEOWNERS).read_text() if Path(ns.CODEOWNERS).exists() else "")
        get = ns.lookup(repo)

        def fresh(st: dict, builds: str):
            team = ns.read_reviewers(Path(ns.REVIEWERS).read_text() if Path(ns.REVIEWERS).exists() else "").logins
            kept, out = refresh(st.get("prs", []), get, patterns, team)
            left_out[:] = out
            if kept == st.get("prs", []):
                return None
            return {**st, "prs": kept}, builds, "the scheduled nightly takes the newest heads"
        prs = ns.update_state(fresh, token).get("prs", [])
    build_set(main, prs, left_out, today, force)


def rotate() -> None:
    """The current `nightly` becomes nightly-<its date>, so tonight's can take the tag."""
    current = gh("release", "view", "nightly", "--json", "name", check=False)
    if not current:
        return
    m = TITLE_DATE.search(json.loads(current)["name"])
    dated = f"nightly-{m.group(1)}" if m else f"nightly-{dt.date.today() - dt.timedelta(days=1)}"
    if gh("release", "view", dated, "--json", "tagName", check=False):
        gh("release", "delete", dated, "--cleanup-tag", "--yes")  # a second nightly the same day replaces the first
    gh("release", "edit", "nightly", "--tag", dated)
    gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/tags/nightly", check=False)
    print(f"nightly -> {dated}")


def stale(tags: list[dict], today: dt.date, days: int) -> list[str]:
    """The dated nightly pre-releases older than `days`: never a release, never `nightly` itself."""
    cutoff = today - dt.timedelta(days=days)
    return [r["tagName"] for r in tags if r["isPrerelease"] and (m := DATED.match(r["tagName"]))
            and dt.date.fromisoformat(m.group(1)) < cutoff]


def prune(days: int) -> None:
    tags = json.loads(gh("release", "list", "--limit", "200", "--json", "tagName,isPrerelease"))
    for tag in stale(tags, dt.datetime.now(dt.timezone.utc).date(), days):
        gh("release", "delete", tag, "--cleanup-tag", "--yes")
        print(f"deleted {tag}")


BUILD = re.compile(r"^nightly/build-(\d{4}-\d{2}-\d{2})-(\d+)$")


def stale_builds(branches: dict[str, str], published: set[str], today: dt.date, days: int) -> list[str]:
    """The nightly/build-<date>-<n> branches (name -> commit) to delete: older than `days`, never the newest build,
    never one whose commit a published nightly release points to, never a branch not named like a build."""
    builds = {name: sha for name, sha in branches.items() if BUILD.match(name)}
    if not builds:
        return []
    order = lambda name: (BUILD.match(name).group(1), int(BUILD.match(name).group(2)))  # noqa: E731
    newest = max(builds, key=order)
    cutoff = today - dt.timedelta(days=days)
    return sorted(name for name, sha in builds.items() if name != newest and sha not in published
                  and dt.date.fromisoformat(BUILD.match(name).group(1)) < cutoff)


def remote_refs(pattern: str) -> dict[str, str]:
    """origin's refs matching `pattern`: short name -> commit (annotated tags peeled)."""
    refs: dict[str, str] = {}
    for line in git("ls-remote", "origin", pattern).splitlines():
        sha, ref = line.split("\t", 1)
        name = ref.removeprefix("refs/heads/").removeprefix("refs/tags/")
        if name.endswith("^{}"):
            refs[name[:-3]] = sha  # the peeled commit wins over the tag object
        else:
            refs.setdefault(name, sha)
    return refs


def prune_builds(days: int) -> None:
    """Deletes old nightly/build-* branches on origin through the API, one line per deletion (the run summary)."""
    branches = remote_refs("refs/heads/nightly/build-*")
    tags = {name: sha for name, sha in remote_refs("refs/tags/nightly*").items()
            if name == "nightly" or DATED.match(name)}
    today = dt.datetime.now(dt.timezone.utc).date()
    doomed = stale_builds(branches, set(tags.values()), today, days)
    for name in doomed:
        gh("api", "-X", "DELETE", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs/heads/{name}")
        print(f"- deleted `{name}` ({branches[name][:7]})")
    print(f"{len(doomed)} of {len(branches)} nightly/build-* branches deleted (kept: the newest, the published "
          f"nightlies' and the last {days} days')")


def keep_source(sha: str, today: str) -> None:
    """Tags the published nightly's commit for good (source_tag); a rerun of the same build finds it there."""
    tag = source_tag(today, sha)
    there = remote_refs(f"refs/tags/{tag}").get(tag)
    if there == sha:
        print(f"`{tag}` already names {sha[:7]}")
        return
    if there:
        raise SystemExit(f"`{tag}` names {there[:7]}, not {sha[:7]}")
    gh("api", f"repos/{os.environ['GITHUB_REPOSITORY']}/git/refs", "-f", f"ref=refs/tags/{tag}", "-f", f"sha={sha}")
    print(f"tagged {sha[:7]} `{tag}`")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pick").add_argument("--force", action="store_true")
    sub.add_parser("rotate")
    sub.add_parser("prune").add_argument("--days", type=int, default=7)
    sub.add_parser("prune-builds").add_argument("--days", type=int, default=7)
    keep = sub.add_parser("keep-source")
    keep.add_argument("sha")
    keep.add_argument("date")
    args = ap.parse_args()
    if args.cmd == "pick":
        pick(args.force)
    elif args.cmd == "rotate":
        rotate()
    elif args.cmd == "prune-builds":
        prune_builds(args.days)
    elif args.cmd == "keep-source":
        keep_source(args.sha, args.date)
    else:
        prune(args.days)


if __name__ == "__main__":
    main()
