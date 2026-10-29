#!/usr/bin/env python3
"""Horizon's roadmap: roadmap/roadmap.toml is the source, ROADMAP.md is generated from it.

    python3 tools/roadmap.py build              write ROADMAP.md and roadmap/roadmap.json
    python3 tools/roadmap.py build --check      fail when either is out of date (CI)
    python3 tools/roadmap.py check --offline    the file's own rules (CI)
    python3 tools/roadmap.py check              the same, then GitHub: links resolve, no stale Now item
    python3 tools/roadmap.py sync [--dry-run]   items whose pull requests all merged become shipped,
                                                with the release they went out in
    python3 tools/roadmap.py labels [--dry-run] a `roadmap:<id>` label per item on GitHub, and on the open
                                                pull requests the item links

The online commands read GitHub with GITHUB_TOKEN, GH_TOKEN or `gh auth token`. Maintenance: roadmap/README.md.
"""

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import tomllib
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "roadmap" / "roadmap.toml"
OUTPUT = ROOT / "ROADMAP.md"
# What the website's hub reads from main at build time (horizon-rust/horizon-site), and serves at /data/roadmap.json
# for Kira and others.
SITE_DATA = ROOT / "roadmap" / "roadmap.json"
# The journal's weekly summaries, one file per ISO week: roadmap/journal/2026-43.md (roadmap/README.md).
JOURNAL = ROOT / "roadmap" / "journal"
WEEK = re.compile(r"(\d{4})-(\d{2})\.md$")
REPO = "horizon-rust/horizon"
SITE = "https://playhorizon.dev/hub"

STATUSES = {
    "now": "Now",
    "next": "Next",
    "later": "Later",
    "help-wanted": "Help wanted",
    "shipped": "Recently shipped",
}
PLAIN_MAX = 240  # one post's worth, so a shipped item can be announced as written
TITLE_MAX = 70
FOCUS_RANGE = (3, 5)
STALE_DAYS = 14  # a Now item needs an open pull request, or one merged this recently
LABEL_PREFIX = "roadmap:"
# GitHub label colours by status, from the design system's palette.
LABEL_COLOURS = {"now": "5b3fd6", "next": "9a86ff", "later": "c5c7d1", "help-wanted": "1d7a48", "shipped": "646875"}
ITEM_KEYS = {"id", "title", "status", "area", "plain", "dev", "links", "focus", "ongoing", "shipped_in", "post",
             "announce"}

ISSUE = re.compile(r"#(\d+)$")
ID = re.compile(r"[a-z0-9]+(-[a-z0-9]+)*$")
# An item is the anchor playhorizon.dev/hub#<id>: it can't take the name of a tab, a section or a heading.
RESERVED = {"main", "overview", "roadmap", "board", "journal", "contribute", "report", "suggest", "contributors",
            "hub-data", *STATUSES}
RESERVED_PREFIXES = ("h-", "tab-", "col-", "week-", "journal-")
RELEASE = re.compile(r"(v\d[\w.]*|launcher@\d[\w.]*|launcher-v\d[\w.]*|website)$")
MONTHS = ("January|February|March|April|June|July|August|September|October|November|December")
# Kira's rule: no dates and no promises of when. "Coming soon" is the one allowed phrase.
DATE = re.compile(
    rf"\b(?:{MONTHS})\b|\bMay \d|(?<![#\w.])20\d\d(?!\w|\.\d)|\bQ[1-4]\b|\btomorrow\b|\btonight\b"
    r"|\b(?:next|this) (?:week|month|year)\b|\bin \d+ (?:days?|weeks?|months?)\b|\bby the end of\b",
    re.IGNORECASE)
MONTH_CASE = re.compile(rf"\b(?:{MONTHS})\b")


def load(path=SOURCE):
    with open(path, "rb") as source:
        return tomllib.load(source)


def link_url(link):
    """Where a link points: "#12" is an issue or a pull request (GitHub redirects one to the other)."""
    match = ISSUE.match(link)
    if match:
        return f"https://github.com/{REPO}/issues/{match[1]}"
    return link


def dated(text):
    """The first date or deadline in the text, or None. A month counts only capitalised ("march" is a verb)."""
    for match in DATE.finditer(text):
        if match[0].lower() not in MONTHS.lower().split("|") or MONTH_CASE.fullmatch(match[0]):
            return match[0]
    return None


def validate(data, root=ROOT):
    """The file's own rules, without the network. Returns a list of problems."""
    problems = []
    overview = data.get("overview", {})
    for key in ("today_plain", "today_dev", "help_plain", "help_dev"):
        if not str(overview.get(key, "")).strip():
            problems.append(f"overview: {key} is missing")
    areas = [area.get("id") for area in data.get("area", [])]
    if len(set(areas)) != len(areas):
        problems.append("areas: an id appears twice")
    seen = set()
    items = data.get("item", [])
    for index, item in enumerate(items):
        where = f"item {item.get('id') or index + 1}"
        for key in sorted(set(item) - ITEM_KEYS):
            problems.append(f"{where}: unknown key {key!r}")
        for key in ("id", "title", "status", "area", "plain", "dev"):
            if not str(item.get(key, "")).strip():
                problems.append(f"{where}: {key} is missing")
        ident = item.get("id", "")
        if ident and not ID.match(ident):
            problems.append(f"{where}: ids are lowercase words joined by dashes")
        if ident in RESERVED or ident.startswith(RESERVED_PREFIXES):
            problems.append(f"{where}: the id is taken by the hub page (hub#{ident} is a tab, a section or a heading)")
        if ident in seen:
            problems.append(f"{where}: the id appears twice")
        seen.add(ident)
        status = item.get("status")
        if status not in STATUSES:
            problems.append(f"{where}: status {status!r} is not one of {', '.join(STATUSES)}")
        if item.get("area") not in areas:
            problems.append(f"{where}: area {item.get('area')!r} is not declared in an [[area]]")
        if len(item.get("title", "")) > TITLE_MAX:
            problems.append(f"{where}: the title is longer than {TITLE_MAX} characters")
        for key in ("plain", "post"):
            if len(item.get(key, "")) > PLAIN_MAX:
                problems.append(f"{where}: {key} is {len(item[key])} characters, at most {PLAIN_MAX}")
        for key in ("title", "plain", "dev", "post"):
            found = dated(item.get(key, ""))
            if found:
                problems.append(f"{where}: {key} names a date or a deadline ({found!r}); the roadmap has none")
        shipped_in = item.get("shipped_in")
        if status == "shipped":
            if not shipped_in:
                problems.append(f"{where}: a shipped item needs shipped_in (a release tag, or \"website\")")
            elif not RELEASE.match(shipped_in):
                problems.append(f"{where}: shipped_in {shipped_in!r} is not a release tag or \"website\"")
        elif shipped_in:
            problems.append(f"{where}: only shipped items have shipped_in")
        for key in ("focus", "ongoing", "announce"):
            if key in item and not isinstance(item[key], bool):
                problems.append(f"{where}: {key} is true or false")
        if item.get("ongoing") and status not in ("now", "next", "later"):
            problems.append(f"{where}: only Now, Next and Later items are ongoing")
        if item.get("focus") and status != "now":
            problems.append(f"{where}: only Now items can be in focus")
        links = item.get("links", [])
        if not isinstance(links, list):
            problems.append(f"{where}: links is a list")
            links = []
        if not links and status != "later":
            problems.append(f"{where}: needs at least one link (an issue, a pull request or a doc)")
        for link in links:
            if ISSUE.match(link) or link.startswith("https://"):
                continue
            if link.startswith(("/", "http:")) or ".." in link or not (root / link).exists():
                problems.append(f"{where}: link {link!r} is neither #N, https:// nor a path in the repository")
    problems += weeks()[1] if root == ROOT else []
    focus = sum(1 for item in items if item.get("focus"))
    low, high = FOCUS_RANGE
    if not low <= focus <= high:
        problems.append(f"focus: {focus} items are in focus, the overview wants {low} to {high}")
    return problems


def areas_in_order(data):
    return [(area["id"], area["name"]) for area in data.get("area", [])]


def by_area(data, status):
    """The items of a status, grouped by area in the areas' order, each group in the file's order."""
    items = [item for item in data.get("item", []) if item.get("status") == status]
    return [(name, [item for item in items if item["area"] == key])
            for key, name in areas_in_order(data) if any(item["area"] == key for item in items)]


def linkify(text, links=()):
    """Markdown links for the text's #N, and for the item's own paths and URLs where the text names them."""
    text = re.sub(r"(?<![\w\[/])#(\d+)\b", lambda m: f"[#{m[1]}]({link_url('#' + m[1])})", text)
    for link in links:
        if not ISSUE.match(link):
            text = re.sub(rf"(?<![\w/\[])(?<!\]\(){re.escape(link)}(?![\w/])", f"[{link}]({link})", text, count=1)
    return text


def link_list(links):
    return " · ".join(f"[{link}]({link_url(link)})" for link in links)


def render(data):
    """ROADMAP.md, from the parsed TOML."""
    overview = data["overview"]
    items = data.get("item", [])
    out = [
        "# Horizon roadmap",
        "",
        "<!-- Generated by tools/roadmap.py from roadmap/roadmap.toml: edit that file, then run"
        " `python3 tools/roadmap.py build`. -->",
        "",
        "What is being done on Horizon, what comes next and where you can help. There are no dates: things are"
        " Now, Next or Later. Each item is said in plain words, with the technical picture folded under it."
        f" The same roadmap, with a Simple / Technical switch, the board, the journal and how to help, is on"
        f" [playhorizon.dev/hub]({SITE}). Pull requests carry the `roadmap:<id>` label of their item.",
        "",
        "## At a glance",
        "",
        overview["today_plain"].strip(),
        "",
        "**Being done now**",
        "",
    ]
    out += [f"- **{item['title']}**: {item['plain'].strip()}" for item in items if item.get("focus")]
    nexts = [item["title"] for item in items if item.get("status") == "next"]
    if nexts:
        out += ["", "**Next**: " + "; ".join(nexts) + "."]
    out += ["", f"**How to help**: {overview['help_plain'].strip()}", "",
            f"**Developers**: {overview['help_dev'].strip()}"]
    for status, heading in STATUSES.items():
        groups = by_area(data, status)
        if not groups:
            continue
        out += ["", f"## {heading}"]
        if status == "shipped":
            # Newest first: the file's order, where `sync` puts each newly shipped item on top.
            out += [""] + [f"- **{item['title']}** ({item['shipped_in']}): {item['plain'].strip()} "
                           f"{link_list(item['links'])}" for item in items if item.get("status") == status]
            continue
        for area, group in groups:
            out += ["", f"### {area}"]
            for item in group:
                out += ["", f"#### {item['title']}", "", item["plain"].strip(), "",
                        "<details><summary>Technical</summary>", "", linkify(item["dev"].strip(), item.get("links", []))]
                more = [link for link in item.get("links", []) if not re.search(rf"{re.escape(link)}\b", item["dev"])]
                if more:
                    out += ["", "More: " + link_list(more)]
                out += ["", "</details>"]
    return "\n".join(out) + "\n"


def weeks(directory=JOURNAL):
    """The weekly summaries, newest first: {"week", "date" (its Monday), "title", "plain", "dev"}, and problems."""
    entries, problems = [], []
    for path in sorted(directory.glob("*.md"), reverse=True) if directory.exists() else []:
        name = WEEK.match(path.name)
        text = path.read_text(encoding="utf-8")
        parts = re.match(r"# (.+)\n\n(.+?)\n\n## Technical\n\n(.+)", text.strip() + "\n", re.S)
        if not name or not 1 <= int(name[2]) <= 53:
            problems.append(f"journal/{path.name}: name it by its ISO week, like 2026-43.md")
            continue
        if not parts:
            problems.append(f"journal/{path.name}: needs '# Title', the plain text, then '## Technical' and its text")
            continue
        monday = datetime.date.fromisocalendar(int(name[1]), int(name[2]), 1)
        entries.append({"week": path.stem, "date": monday.isoformat(), "title": parts[1].strip(),
                        "plain": " ".join(parts[2].split()), "dev": " ".join(parts[3].split())})
    return entries, problems


def site_data(data):
    """roadmap/roadmap.json: the items as the hub and Kira read them, in the file's order."""
    keys = ("id", "title", "status", "area", "plain", "dev", "links", "focus", "ongoing", "shipped_in", "post",
            "announce")
    out = {
        "format": "horizon-roadmap",
        "version": 1,
        "source": "roadmap/roadmap.toml",
        "overview": {key: " ".join(value.split()) for key, value in data["overview"].items()},
        "statuses": [{"id": key, "name": name} for key, name in STATUSES.items()],
        "areas": [{"id": key, "name": name} for key, name in areas_in_order(data)],
        "items": [{key: (" ".join(item[key].split()) if isinstance(item[key], str) else item[key])
                   for key in keys if key in item} for item in data.get("item", [])],
        "journal": weeks()[0],
    }
    return json.dumps(out, indent=1, ensure_ascii=False) + "\n"


def outputs(data):
    """Every generated file and its content."""
    return {OUTPUT: render(data), SITE_DATA: site_data(data)}


# --- GitHub ---------------------------------------------------------------------------------------------------

def token():
    for name in ("GITHUB_TOKEN", "GH_TOKEN"):
        if os.environ.get(name):
            return os.environ[name]
    try:
        return subprocess.run(["gh", "auth", "token"], capture_output=True, text=True, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        sys.exit("roadmap: reading GitHub needs GITHUB_TOKEN, GH_TOKEN or `gh auth login` (or use --offline)")


def numbers_in(data):
    return sorted({int(ISSUE.match(link)[1]) for item in data.get("item", [])
                   for link in item.get("links", []) if ISSUE.match(link)})


def github_states(numbers, auth):
    """{number: {"kind": "Issue" | "PullRequest", "state", "merged_at", "merge_commit", "paths"}} for those
    that exist, in one GraphQL request per hundred numbers."""
    owner, name = REPO.split("/")
    states = {}
    for start in range(0, len(numbers), 100):
        fields = "\n".join(
            f"n{n}: issueOrPullRequest(number: {n}) {{ __typename "
            "... on Issue { state } "
            "... on PullRequest { state mergedAt mergeCommit { oid } files(first: 100) { nodes { path } } "
            "labels(first: 50) { nodes { name } } } }"
            for n in numbers[start:start + 100])
        query = f'query {{ repository(owner: "{owner}", name: "{name}") {{ {fields} }} }}'
        request = urllib.request.Request(
            "https://api.github.com/graphql", data=json.dumps({"query": query}).encode(),
            headers={"Authorization": f"bearer {auth}", "User-Agent": "horizon-roadmap"})
        with urllib.request.urlopen(request, timeout=30) as response:
            answer = json.load(response)
        for key, node in (answer.get("data", {}).get("repository") or {}).items():
            if node:
                states[int(key[1:])] = {
                    "kind": node["__typename"],
                    "state": node["state"],
                    "merged_at": node.get("mergedAt"),
                    "merge_commit": (node.get("mergeCommit") or {}).get("oid"),
                    "paths": [file["path"] for file in ((node.get("files") or {}).get("nodes") or [])],
                    "labels": [label["name"] for label in ((node.get("labels") or {}).get("nodes") or [])],
                }
    return states


def pull_requests(item, states):
    return [states[n] for n in (int(ISSUE.match(link)[1]) for link in item.get("links", []) if ISSUE.match(link))
            if n in states and states[n]["kind"] == "PullRequest"]


def open_issues(item, states):
    return [n for n in (int(ISSUE.match(link)[1]) for link in item.get("links", []) if ISSUE.match(link))
            if n in states and states[n]["kind"] == "Issue" and states[n]["state"] == "OPEN"]


def online_problems(data, states, now):
    """What only GitHub can tell: links that don't exist, Now items nobody works on, Help wanted items already
    done, items `sync` would ship."""
    problems, notes = [], []
    recent = now - datetime.timedelta(days=STALE_DAYS)
    for item in data.get("item", []):
        where = f"item {item['id']}"
        for link in item.get("links", []):
            match = ISSUE.match(link)
            if match and int(match[1]) not in states:
                problems.append(f"{where}: {link} is neither an issue nor a pull request of {REPO}")
        pulls = pull_requests(item, states)
        status = item.get("status")
        if status == "now":
            alive = [p for p in pulls if p["state"] == "OPEN" or
                     (p["merged_at"] and datetime.datetime.fromisoformat(p["merged_at"]) >= recent)]
            if not alive:
                problems.append(f"{where}: stale: Now, but no open pull request and none merged in the last "
                                f"{STALE_DAYS} days (link the work, or move it to Next or Later)")
        if status == "help-wanted":
            issues = [states[int(ISSUE.match(link)[1])] for link in item.get("links", [])
                      if ISSUE.match(link) and int(ISSUE.match(link)[1]) in states]
            if issues and all(i["state"] != "OPEN" for i in issues):
                problems.append(f"{where}: every linked issue and pull request is closed: ship or drop it")
        if status != "shipped" and pulls and all(p["state"] == "MERGED" for p in pulls):
            if item.get("ongoing"):
                notes.append(f"{where}: ongoing, and all its pull requests merged: link the new ones")
            elif not open_issues(item, states):
                notes.append(f"{where}: all its pull requests merged: `python3 tools/roadmap.py sync` ships it "
                             "once a release has them")
    return problems, notes


# --- sync -----------------------------------------------------------------------------------------------------

def family(paths):
    """Which releases carry a change: the launcher's, the website (deployed when merged) or the game's. The site
    left this repository on 2026-10-26 (horizon-rust/horizon-site), so only older pull requests are website-only;
    an item the site alone ships takes shipped_in = "website" by hand."""
    if paths and all(path.startswith("launcher/") for path in paths):
        return "launcher"
    if paths and all(path.startswith("website/") for path in paths):
        return "website"
    return "game"


def first_release(commit, kind):
    """The oldest release tag of this kind that contains the commit, or None while none does."""
    if kind == "website":
        return "website"
    tags = subprocess.run(["git", "tag", "--contains", commit, "--sort=creatordate"], cwd=ROOT,
                          capture_output=True, text=True).stdout.split()
    wanted = (lambda tag: tag.startswith(("launcher@", "launcher-v"))) if kind == "launcher" else \
        (lambda tag: re.match(r"v\d", tag))
    return next((tag for tag in tags if wanted(tag)), None)


def release_of(pulls):
    """The release that shipped the last of these merged pull requests, or None."""
    kinds = {family(p["paths"]) for p in pulls}
    kind = "game" if "game" in kinds or len(kinds) > 1 else kinds.pop()
    latest = max(pulls, key=lambda p: p["merged_at"])
    return first_release(latest["merge_commit"], kind) if latest["merge_commit"] else None


def item_span(lines, ident):
    """[start, end) of the item's lines: its [[item]] header to its last key, without the comments that follow."""
    starts = [i for i, line in enumerate(lines) if line.strip() == "[[item]]"]
    for position, start in enumerate(starts):
        end = starts[position + 1] if position + 1 < len(starts) else len(lines)
        if not any(re.match(rf'id\s*=\s*"{re.escape(ident)}"\s*$', line) for line in lines[start:end]):
            continue
        while end > start and (not lines[end - 1].strip() or re.match(r"#(\s|-|$)", lines[end - 1])):
            end -= 1
        return start, end
    raise KeyError(ident)


def mark_shipped(text, ident, release):
    """The TOML with this item shipped in `release`, moved to the top of the shipped items (newest first)."""
    lines = text.split("\n")
    start, end = item_span(lines, ident)
    block = [line for line in lines[start:end] if not line.startswith("shipped_in")]
    block = [f'status = "shipped"\nshipped_in = "{release}"' if re.match(r"status\s*=", line) else line
             for line in block]
    block = [line for line in block if not re.match(r"focus\s*=", line)]
    rest = lines[:start] + lines[end:]
    # Drop the blank line the block leaves behind.
    if start < len(rest) and start > 0 and not rest[start - 1].strip() and not rest[start].strip():
        del rest[start]
    shipped = [i for i, line in enumerate(rest) if re.match(r'status\s*=\s*"shipped"', line)]
    if shipped:
        header = max(i for i in range(shipped[0] + 1) if rest[i].strip() == "[[item]]")
        rest[header:header] = block + [""]
    else:
        rest += [""] + block
    return "\n".join(rest)


def sync(data, text, states):
    """Ship every item whose linked pull requests all went out in a release and whose linked issues are all
    closed, unless it is ongoing. Returns the new text and what happened."""
    log = []
    for item in data.get("item", []):
        if item.get("status") == "shipped":
            continue
        pulls = pull_requests(item, states)
        # An ongoing item, or one with an open issue, still has work left whatever its pull requests say.
        if item.get("ongoing") or open_issues(item, states):
            continue
        if not pulls or any(p["state"] != "MERGED" for p in pulls):
            continue
        release = release_of(pulls)
        if release is None:
            log.append(f"{item['id']}: merged, waiting for a release")
            continue
        text = mark_shipped(text, item["id"], release)
        log.append(f"{item['id']}: shipped in {release}")
    return text, log


# --- labels ---------------------------------------------------------------------------------------------------

def label_of(item):
    """The GitHub label of an item: its name, colour and description (GitHub allows 100 characters)."""
    description = f"Roadmap, {STATUSES[item['status']].lower()}: {item['title']}"
    return {"name": LABEL_PREFIX + item["id"], "color": LABEL_COLOURS[item["status"]], "description": description[:100]}


def label_changes(data, existing, states):
    """What `labels` does: [(action, label or number, payload)] for labels to create or update, and open pull
    requests linked from an item that lack its label. existing: {name: {"color", "description"}}; states as
    github_states, plus each pull request's "labels"."""
    changes = []
    for item in data.get("item", []):
        label = label_of(item)
        have = existing.get(label["name"])
        if have is None:
            changes.append(("create", label["name"], label))
        elif (have.get("color", "").lower(), have.get("description") or "") != (label["color"], label["description"]):
            changes.append(("update", label["name"], label))
        for link in item.get("links", []):
            match = ISSUE.match(link)
            state = states.get(int(match[1])) if match else None
            if state and state["kind"] == "PullRequest" and state["state"] == "OPEN" and \
                    label["name"] not in state.get("labels", []):
                changes.append(("add", int(match[1]), label["name"]))
    stale = sorted(set(existing) - {LABEL_PREFIX + item["id"] for item in data.get("item", [])})
    return changes, [name for name in stale if name.startswith(LABEL_PREFIX)]


def github_rest(method, path, auth, body=None):
    request = urllib.request.Request(
        f"https://api.github.com{path}", method=method, data=None if body is None else json.dumps(body).encode(),
        headers={"Authorization": f"bearer {auth}", "Accept": "application/vnd.github+json",
                 "User-Agent": "horizon-roadmap"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response) if response.status != 204 else None


def existing_labels(auth):
    labels, page = {}, 1
    while True:
        batch = github_rest("GET", f"/repos/{REPO}/labels?per_page=100&page={page}", auth)
        labels.update({label["name"]: label for label in batch})
        if len(batch) < 100:
            return labels
        page += 1


# --- commands -------------------------------------------------------------------------------------------------

def report(problems):
    for problem in problems:
        print(f"roadmap: {problem}", file=sys.stderr)
    return 1 if problems else 0


def command_build(args):
    data = load()
    problems = validate(data)
    if problems:
        return report(problems)
    files = outputs(data)
    if args.check:
        stale = [path for path, text in files.items() if not path.exists() or path.read_text(encoding="utf-8") != text]
        return report([f"{path.relative_to(ROOT)} is out of date: run `python3 tools/roadmap.py build`"
                       for path in stale])
    for path, text in files.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        print(f"roadmap: wrote {path.relative_to(ROOT)}")
    return 0


def command_check(args):
    data = load()
    problems = validate(data)
    if not problems and not args.offline:
        states = github_states(numbers_in(data), token())
        online, notes = online_problems(data, states, datetime.datetime.now(datetime.timezone.utc))
        problems += online
        for note in notes:
            print(f"roadmap: {note}")
    if not problems:
        print(f"roadmap: {len(data.get('item', []))} items, all good")
    return report(problems)


def command_sync(args):
    data = load()
    problems = validate(data)
    if problems:
        return report(problems)
    subprocess.run(["git", "fetch", "--quiet", "--tags", "origin"], cwd=ROOT, check=False)
    text, log = sync(data, SOURCE.read_text(encoding="utf-8"), github_states(numbers_in(data), token()))
    for line in log:
        print(f"roadmap: {line}")
    if args.dry_run or text == SOURCE.read_text(encoding="utf-8"):
        return 0
    new = tomllib.loads(text)
    if len(new.get("item", [])) != len(data.get("item", [])) or validate(new):
        return report(["sync produced an invalid file; nothing written"] + validate(new))
    SOURCE.write_text(text, encoding="utf-8")
    for path, content in outputs(new).items():
        path.write_text(content, encoding="utf-8")
    print("roadmap: wrote roadmap/roadmap.toml, ROADMAP.md and roadmap/roadmap.json")
    return 0


def command_labels(args):
    data = load()
    problems = validate(data)
    if problems:
        return report(problems)
    auth = token()
    changes, stale = label_changes(data, existing_labels(auth), github_states(numbers_in(data), auth))
    for action, target, payload in changes:
        print(f"roadmap: {action} {target}" + (f" ({payload})" if action == "add" else ""))
        if args.dry_run:
            continue
        if action == "create":
            github_rest("POST", f"/repos/{REPO}/labels", auth, payload)
        elif action == "update":
            github_rest("PATCH", f"/repos/{REPO}/labels/{urllib.parse.quote(target, safe='')}", auth,
                        {"color": payload["color"], "description": payload["description"]})
        else:
            github_rest("POST", f"/repos/{REPO}/issues/{target}/labels", auth, {"labels": [payload]})
    for name in stale:
        print(f"roadmap: {name} names no item any more: rename or delete it by hand (it stays on its pull requests)")
    if not changes:
        print("roadmap: labels up to date")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("build", help="write ROADMAP.md")
    build.add_argument("--check", action="store_true", help="fail when ROADMAP.md is out of date")
    check = commands.add_parser("check", help="the roadmap's rules, and GitHub's state of its links")
    check.add_argument("--offline", action="store_true", help="skip GitHub")
    sync_command = commands.add_parser("sync", help="ship the items whose pull requests all merged")
    sync_command.add_argument("--dry-run", action="store_true", help="say what would change, write nothing")
    labels = commands.add_parser("labels", help="a roadmap:<id> label per item, on its linked open pull requests")
    labels.add_argument("--dry-run", action="store_true", help="say what would change, change nothing")
    args = parser.parse_args(argv)
    return {"build": command_build, "check": command_check, "sync": command_sync,
            "labels": command_labels}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
