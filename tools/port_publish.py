#!/usr/bin/env python3
"""File a port report's work items as GitHub issues (docs/version-ports.md, "The port report").

Used through `version_port.py publish OLD NEW [--yes]`. A dry run by default: it prints what it would create or
update; --yes does it (labels, issues, the Kira card). The maintainer approves the first --yes run and the
`version-port` label.

- One issue per work item of rank 1, of rank 2 when a harness newly fails on NEW (or its oracle no longer
  compiles), and of rank 3 when Horizon's comments name a changed method, titled "[NEW] <item>" and labelled `version-port` and `port-NEW`. An open issue with
  the same title is updated: only the generated part of its body (between the markers) is replaced, so what people
  wrote around it stays.
- One tracking issue, "[NEW] Port to Minecraft NEW", listing those issues and every other item (rank-2 leads no
  harness confirmed, new content, unported changes).
- Kira shows every open issue as a card in To do, labelled, so contributors and agents can claim them there; publish
  adds one Kira card for the port itself, linking the tracking issue.

Bodies hold names and signatures only: vanilla classes, methods with their descriptors, fields, registry entries,
packets, harness names, javac's one-line errors (about Horizon's own oracles), Horizon file paths and the local
path of the diff. Never decompiled code: `assert_no_code` refuses a body with a line that reads like Java source.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

try:
    from . import port_report
except ImportError:
    import port_report

LABEL = "version-port"
START, END = "<!-- version-port: generated -->", "<!-- /version-port -->"
KIRA = Path(os.environ.get("HORIZON_KIRA", Path.home() / "code/horizon-parity/tools/kira.mjs"))
RANKS = {1: "blocks the switch", 2: "proved by a harness", 3: "ported, no harness", 4: "new content",
         5: "not ported"}
# A line of Java source: a declaration, a statement, an import (what decompiled code looks like).
CODE = re.compile(r"^\s*(?:(?:public|private|protected|static|final|abstract)\s+[\w<>\[\], ]+[\s(]"
                  r"|(?:return|throw|import|package)\s+\S|(?:if|for|while|switch)\s*\(|[}{]\s*$)|;\s*$")
# Inside backticks: names, JVM descriptors (`m(Lx/Y;I)V`, no spaces) and javac's one-line errors, never braces,
# statements or assignments.
QUOTED_CODE = re.compile(r"[{}]|;\s+\S|\s=\s|\breturn\s|\bnew\s+[A-Z]")


def published(item):
    """Whether an item gets its own issue: rank 1; rank 2 when a harness newly fails; rank 3 when Horizon's
    comments name one of the changed methods (the others are leads, listed in the tracking issue)."""
    if item["rank"] == 3:
        return any(info.get("named") for info in (item.get("classes") or {}).values())
    return item["rank"] == 1 or (item["rank"] == 2 and bool(item.get("failed")))


def title(new, item):
    return f"[{new}] {item['title']}"[:240]


def signatures(name, change, limit=20):
    """A changed class's members by name and descriptor (from the class index: no code)."""
    lines = []
    for kind in ("methods", "fields"):
        for label in ("added", "removed", "changed"):
            members = change.get(kind, {}).get(label, [])
            if members:
                shown = ", ".join(f"`{m}`" for m in members[:limit])
                more = f" (+{len(members) - limit})" if len(members) > limit else ""
                lines.append(f"  - {kind} {label}: {shown}{more}")
    return lines


def body(item, report, diff, store_path, limit=25):
    """The generated part of an item's issue."""
    old, new = report["old"], report["new"]
    lines = [START, f"Minecraft {old} → {new}, rank {item['rank']} ({RANKS[item['rank']]}), from "
             f"`tools/version_port.py report {old} {new}` (docs/version-ports.md).", ""]
    if item.get("command"):
        lines.append(f"Run: `{item['command']}`")
    if item.get("compile_errors"):
        lines += ["javac, against " + new + ":", *(f"- `{e}`" for e in item["compile_errors"])]
    if item.get("failed"):
        lines.append(f"Harnesses failing on {new}: " + ", ".join(f"`{h}`" for h in item["failed"])
                     + (f" (oracle no longer compiles: {', '.join(item['oracle'])})" if item.get("oracle") else ""))
    if item.get("harnesses"):
        lines.append("Harnesses that prove it: " + ", ".join(f"`{h}`" for h in item["harnesses"][:limit]))
    if item.get("files"):
        lines.append("Horizon: " + ", ".join(f"`{f}`" for f in item["files"][:limit])
                     + (f" (+{len(item['files']) - limit})" if len(item["files"]) > limit else ""))
    classes = item.get("classes")
    if isinstance(classes, dict):
        lines += ["", "Vanilla classes (names and signatures):"]
        changed = (diff.get("classes") or {}).get("changed", {})
        for name, info in list(classes.items())[:limit]:
            what = "removed" if info["removed"] else f"{info['methods']} methods changed"
            named = f"; Horizon's comments name {', '.join(info['named'])}" if info["named"] else ""
            lines.append(f"- `{name.replace('/', '.')}`: {what}{named}")
            lines += signatures(name, changed.get(name, {}))
    elif classes:
        lines.append("New classes: " + ", ".join(f"`{c.replace('/', '.')}`" for c in classes[:limit]))
    if item.get("entries"):
        lines.append("Entries: " + ", ".join(f"`{e}`" for e in item["entries"][:limit]))
    details = item.get("details", {})
    if item["kind"] == "protocol":
        for flow, change in details["packets"].items():
            lines.append(f"- {flow}: added {', '.join(change['added']) or 'none'}; removed "
                         f"{', '.join(change['removed']) or 'none'}; {len(change['renumbered'])} renumbered")
    for key in ("schemas", "fixes"):
        if details.get(key):
            lines.append(f"{key.capitalize()}: " + ", ".join(f"`{port_report.short(n)}`" for n in details[key]))
    if item["kind"] == "components":
        types = details["types"]
        lines += [f"Types {label}: " + ", ".join(f"`{t}`" for t in types.get(label, []))
                  for label in ("added", "removed") if types.get(label)]
        lines += [f"- `{c}`: " + ", ".join(f"{k} on {n} items" for k, n in counts.items() if n)
                  for c, counts in sorted(details["items"].items(), key=lambda kv: -sum(kv[1].values()))[:limit]]
    if item["kind"] == "facts":
        lines += [f"- {key}: {a} → {b}" for key, (a, b) in details.items()]
    # A body is the same on every machine: Posix separators, `~` for the home.
    where = Path(store_path).as_posix().replace(Path.home().as_posix(), "~")
    lines += ["", f"The full diff stays on the maintainer's machine (`{where}`), with the vanilla files it was built "
              "from: they are never published.", END]
    text = "\n".join(lines) + "\n"
    assert_no_code(text)
    return text


def assert_no_code(text):
    """Every name and signature goes in `backticks`, one line at most; what is left must not read like Java."""
    if "```" in text:
        raise ValueError("Refusing to publish a code block")
    for line in text.splitlines():
        if any(QUOTED_CODE.search(span) for span in re.findall(r"`([^`]*)`", line)):
            raise ValueError(f"Refusing to publish a quoted span that reads like source code: {line[:120]!r}")
        prose = re.sub(r"`[^`]*`", "", line).strip(" -*,;:()")
        if line.startswith("<!--") or not prose:
            continue
        if CODE.search(prose):
            raise ValueError(f"Refusing to publish a line that reads like source code: {line[:120]!r}")


def merged(existing, generated):
    """An issue's body with its generated part replaced, the rest kept."""
    if existing and START in existing and END in existing:
        head, rest = existing.split(START, 1)
        return head + generated.strip() + rest.split(END, 1)[1]
    return generated


def tracking_body(report, issues, others, limit=200):
    lines = [START, f"Port of Horizon to Minecraft {report['new']} (from {report['old']}): `tools/version_port.py "
             f"report {report['old']} {report['new']}` (docs/version-ports.md).", ""]
    summary = report["summary"]
    lines += [f"- {key}: {a} → {b}" for key, (a, b) in summary["facts"].items()]
    if report.get("harnesses"):
        h = report["harnesses"]
        lines.append(f"- harnesses on {report['new']}: {h['pass']} pass, {h['oracle']} oracle no longer compiles, "
                     f"{h['fail']} fail; {h['new']} newly failing")
    lines += ["", "Work items with their own issue:"]
    lines += [f"- [ ] {ref} {title}" for title, ref in issues] or ["- none"]
    lines += ["", "Other items (leads, new content, unported changes):"]
    lines += [f"- rank {i['rank']}: {i['title']}" for i in others[:limit]]
    if len(others) > limit:
        lines.append(f"- … and {len(others) - limit} more in report.md")
    lines.append(END)
    text = "\n".join(lines) + "\n"
    assert_no_code(text)
    return text


def plan(report, diff, store_path, open_issues):
    """[(action, title, body, number)]: "create" or "update" for each published item, then the tracking issue.
    open_issues: {title: (number, body)} of the open version-port issues."""
    new = report["new"]
    actions, others = [], []
    for item in report["items"]:
        if not published(item):
            others.append(item)
            continue
        name = title(new, item)
        number, existing = open_issues.get(name, (None, None))
        actions.append(("update" if number else "create", name, merged(existing, body(item, report, diff, store_path)),
                        number))
    tracking = f"[{new}] Port to Minecraft {new}"
    refs = [(name, f"#{number}" if number else "(new)") for _, name, _, number in actions]
    number, existing = open_issues.get(tracking, (None, None))
    actions.append(("update" if number else "create", tracking,
                    merged(existing, tracking_body(report, refs, others)), number))
    return actions


# --- GitHub and Kira (only with --yes) ---------------------------------------------------------------------

def gh(*args):
    return subprocess.run(["gh", *args], capture_output=True, text=True, check=True).stdout


def open_version_port_issues():
    found = json.loads(gh("issue", "list", "--label", LABEL, "--state", "open", "--limit", "1000",
                          "--json", "number,title,body"))
    return {issue["title"]: (issue["number"], issue["body"]) for issue in found}


def missing_labels(new):
    existing = set(gh("label", "list", "--limit", "500", "--json", "name", "--jq", ".[].name").split("\n"))
    return [label for label in (LABEL, f"port-{new}") if label not in existing]


def apply(actions, report, open_issues):
    """Create the labels, then each item's issue, then the tracking issue listing them; returns {title: url}."""
    new = report["new"]
    for label in missing_labels(new):
        gh("label", "create", label, "--color", "5319e7",
           "--description", "Work to port Horizon to a new Minecraft release (tools/version_port.py publish)")
    *items, tracking = actions
    urls = {name: issue(action, name, text, number, new) for action, name, text, number in items}
    refs = [(name, "#" + urls[name].rsplit("/", 1)[-1]) for _, name, _, _ in items]
    others = [i for i in report["items"] if not published(i)]
    action, name, _, number = tracking
    text = merged(open_issues.get(name, (None, None))[1], tracking_body(report, refs, others))
    urls[name] = issue(action, name, text, number, new)
    return urls


def issue(action, name, text, number, new):
    """Create or update one issue; its URL."""
    # gh reads the body as UTF-8, whatever the system's code page.
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".md", delete=False) as stream:
        stream.write(text)
    try:
        if action == "create":
            url = gh("issue", "create", "--title", name, "--body-file", stream.name, "--label", LABEL,
                     "--label", f"port-{new}").strip()
        else:
            gh("issue", "edit", str(number), "--body-file", stream.name)
            url = gh("issue", "view", str(number), "--json", "url", "--jq", ".url").strip()
    finally:
        os.unlink(stream.name)
    print(f"{action}d {url} {name}", flush=True)
    return url


def kira(tool, arguments):
    return json.loads(subprocess.run(["node", str(KIRA), tool, json.dumps(arguments)], capture_output=True,
                                     text=True, check=True).stdout or "null")


def kira_card(new, url):
    """One Kira card for the port, linking the tracking issue, unless the board has it already."""
    board = kira("board_list", {"label": LABEL, "kind": "card", "limit": 100}) or {}
    if any(card.get("url") == url or url in json.dumps(card.get("url")) for card in board.get("cards", [])):
        return None
    return kira("card_add", {"title": f"Port Horizon to Minecraft {new}", "url": url, "column": "todo",
                             "labels": [LABEL]})
