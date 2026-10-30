#!/usr/bin/env python3
"""Kind and area labels on pull requests (CONTRIBUTING.md "Pull requests", issue #3859).

    pr_labels.py event                     one pull request event (pr-labels.yml): its kind and area labels
    pr_labels.py create [--apply]          creates the kind and area labels the repository lacks
    pr_labels.py backfill [--apply]        labels the open pull requests once; prints what it would do without --apply

Kind: the box ticked in the template's "Kind" list. One label, added only when the pull request has no kind label yet;
unticking the box that set it and ticking another swaps them. A label someone set by hand is never removed.
`feature` keeps its release meaning (listed under New, docs/release-flow.md).
Areas: from the paths the pull request changes (docs/architecture.md), at most 3, each with a tenth of the changed
lines or more. Added, never removed.

The workflow runs on `pull_request_target` from main: this script reads the event file and the API's list of changed
files, never a pull request's code.
"""
from __future__ import annotations

import json
import os
import re
import subprocess
import sys

# Kind: (the word ticked in the template, the label, its description).
KINDS = [
    ("parity", "parity", "Kind: matches vanilla 26.3 behaviour"),
    ("bug", "bug", "Something isn't working"),
    ("perf", "perf", "Kind: faster, or less memory"),
    ("feature", "feature", "A new feature: listed under New in the release notes (Kira pings for it)"),
    ("extra", "extra", "Kind: Horizon-only, off by default"),
    ("tooling", "tooling", "Kind: tools, CI, tests, harnesses"),
    ("docs", "documentation", "Improvements or additions to documentation"),
]
KIND_LABELS = {label for _, label, _ in KINDS}

_G = r"crates/horizon-client(?:-render|-core|-input)?/src/"  # the client's crates
_GAME = _G + r"(?:game/)?"
# Area: (label, description, the paths). First match wins for each file.
AREAS = [
    ("launcher", "Changes the launcher", r"^launcher/"),
    ("mods", "Area: mods, mod SDK, modpack porting",
     r"^crates/(?:horizon-mod|cargo-horizon-mod|horizon-port-pack)|" + _GAME
     + r"(?:mods|mod_content|mod_list|native_\w+)\.rs$|^docs/(?:mods|native-mods|modpacks)"),
    ("worldgen", "Area: world generation", r"^crates/horizon-world/src/worldgen/|/vanillagen\.rs$"),
    ("mobs", "Area: mobs, their AI, spawning and models",
     r"/mobs/|/mobs\.rs$|" + _GAME + r"(?:creatures|natural_spawning|spawners?|dragon|wither|villagers|piglins|golems"
     r"|raids|llamas|wardens?)\.rs$"),
    ("rendering", "Area: the renderer, shaders, models, particles",
     r"\.wgsl$|/gfx(?:\.rs$|/)|/mesher(?:\.rs$|/)|/lod(?:\.rs$|/)|^crates/(?:horizon-shader|horizon-assets|horizon-hdr)|"
     + _GAME + r"(?:particles|block_entity_render|item_render|fog|clouds|camera|first_person|render_tick|item_draw"
     r"|sprites|atlas_sources|mipmap|model_library)\.rs$"),
    ("server", "Area: dedicated and integrated server, hosting",
     r"^crates/horizon-server|/dedicated(?:\.rs$|/)|" + _GAME + r"(?:integrated|levels|server_travel)\.rs$"
     r"|^docs/hosting"),
    ("multiplayer", "Area: protocol, LAN, multiplayer client, friends",
     r"^crates/(?:horizon-protocol|horizon-friends|horizon-relay)/|/lan/|" + _GAME + r"(?:lan\w*|remote\w*|net)\.rs$"),
    ("saves", "Area: saving and loading worlds",
     r"^crates/horizon-world/src/(?:region|nbt)\.rs$|/datafix/|" + _GAME
     + r"(?:save|saving|persist|entity_nbt|block_entity_nbt)\.rs$"),
    ("commands", "Area: commands and functions",
     r"^crates/horizon-command/|/commands(?:\.rs$|/)|" + _GAME + r"(?:scoreboard_cmd|data_cmd|functions"
     r"|command_blocks)\.rs$"),
    ("audio", "Area: sound and music",
     _GAME + r"(?:audio|sound_\w+|music|ambience|subtitle_hud)\.rs$"),
    ("chunks", "Area: chunk streaming, light, sections, tickets",
     _GAME + r"(?:streaming|chunk|chunk_map|light|world|world_loading|ticket_levels|section\w*|\w+_sections"
     r"|tick_chunk_membership)\.rs$|^crates/horizon-core/src/paletted"),
    ("blocks", "Area: blocks, items, redstone, block entities",
     r"^crates/horizon-world/src/level/|^crates/horizon-items/|" + _GAME + r"(?:dispenser|machines|moving_pistons"
     r"|block_entity_ticks|hand|consume|potions|crossbow|trident|fishing|shield|stations|crafting|menu)\.rs$"),
    ("ui", "Area: screens, menus, HUD, chat",
     r"^crates/horizon-theme/|/title(?:\.rs$|/)|" + _GAME + r"(?:\w+_ui|\w+_screen|screens|screen_draw|ui|hud\w*"
     r"|chat|tab_list|startup_menu|options|sign_edit|books|containers)\.rs$"),
    ("website", "Area: what playhorizon.dev reads from this repository",
     r"^docs/api/|^roadmap/|^README\.md$|^launcher/links\.json$"),
]
AREA_RES = [(label, re.compile(paths)) for label, _, paths in AREAS]
MAX_AREAS = 3
KIND_LINE = re.compile(r"^\s*[-*]\s*\[([ xX])\]\s*\**`?(\w+)`?", re.M)


def ticked(body: str | None) -> list[str]:
    """The kind labels ticked in a description's "Kind" section (any order, any number)."""
    if not body:
        return []
    m = re.search(r"^#+\s*Kind\b.*?$(.*?)(?=^#|\Z)", body, re.M | re.S | re.I)
    if not m:
        return []
    by_word = {word: label for word, label, _ in KINDS}
    return [by_word[w.lower()] for box, w in KIND_LINE.findall(m.group(1)) if box != " " and w.lower() in by_word]


def area_of(path: str) -> str | None:
    return next((label for label, rx in AREA_RES if rx.search(path)), None)


def areas(files: list[tuple[str, int]]) -> list[str]:
    """Areas of a change, from (path, changed lines): the biggest first, each a tenth of the lines or more."""
    total = sum(max(n, 1) for _, n in files) or 1
    lines: dict[str, int] = {}
    for path, n in files:
        if (a := area_of(path)):
            lines[a] = lines.get(a, 0) + max(n, 1)
    big = [a for a, n in sorted(lines.items(), key=lambda kv: -kv[1]) if n * 10 >= total]
    return big[:MAX_AREAS]


def kind_change(body: str | None, old_body: str | None, have: set[str]) -> tuple[str | None, str | None]:
    """(label to add, label to remove) for the description's kind. One ticked box only; else nothing."""
    now = ticked(body)
    if len(now) != 1 or now[0] in have:
        return None, None
    before = ticked(old_body)
    kinds_on = have & KIND_LABELS
    if not kinds_on:
        return now[0], None
    if len(before) == 1 and before[0] in kinds_on and len(kinds_on) == 1:
        return now[0], before[0]  # the author changed their tick: follow it
    return None, None  # a kind someone set stays


# --- GitHub ------------------------------------------------------------------------------------------------------


def gh(*args: str) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"gh {args[0]} {args[1] if len(args) > 1 else ''}: {r.stderr.strip()[:300]}")
    return r.stdout.strip()


class Api:
    def __init__(self, repo: str):
        self.repo = repo

    def files(self, n: int) -> list[tuple[str, int]]:
        raw = gh("api", "--paginate", f"repos/{self.repo}/pulls/{n}/files?per_page=100", "--jq",
                 ".[] | [.filename, .changes]")
        return [tuple(json.loads(line)) for line in raw.splitlines() if line.strip()]

    def add(self, n: int, names: list[str]) -> None:
        args = ["api", f"repos/{self.repo}/issues/{n}/labels"]
        for name in names:
            args += ["-f", f"labels[]={name}"]
        gh(*args)

    def remove(self, n: int, name: str) -> None:
        gh("api", "-X", "DELETE", f"repos/{self.repo}/issues/{n}/labels/{name}")

    def open_pulls(self) -> list[dict]:
        raw = gh("pr", "list", "-R", self.repo, "--state", "open", "--limit", "1000", "--json",
                 "number,title,body,labels")
        return json.loads(raw)

    def existing_labels(self) -> set[str]:
        return set(gh("label", "list", "-R", self.repo, "--limit", "1000", "--json", "name", "--jq", ".[].name")
                   .splitlines())

    def create_label(self, name: str, color: str, description: str) -> None:
        gh("label", "create", name, "-R", self.repo, "--color", color, "--description", description)


def plan_event(event: dict, files: list[tuple[str, int]] | None) -> tuple[list[str], str | None]:
    """(labels to add, label to remove) for one pull_request_target event. `files` is None when they didn't change."""
    pr = event["pull_request"]
    have = {l["name"] for l in pr.get("labels", [])}
    old_body = ((event.get("changes") or {}).get("body") or {}).get("from")
    if event.get("action") == "edited" and old_body is None:
        old_body = pr.get("body")  # a title edit: the tick didn't move
    add, remove = kind_change(pr.get("body"), old_body, have)
    new = [add] if add else []
    if files is not None:
        new += [a for a in areas(files) if a not in have and a not in new]
    return new, remove


def event(api) -> int:
    with open(os.environ["GITHUB_EVENT_PATH"]) as f:
        ev = json.load(f)
    n = ev["pull_request"]["number"]
    files = None if ev.get("action") == "edited" else api.files(n)
    add, remove = plan_event(ev, files)
    if remove:
        api.remove(n, remove)
    if add:
        api.add(n, add)
    print(f"#{n}: +{add} -{remove or []}")
    return 0


def guess_kind(title: str, body: str | None, files: list[tuple[str, int]]) -> str | None:
    """The kind of an open pull request without a ticked box, only where it is clear; None when unsure."""
    if (t := ticked(body)) and len(t) == 1:
        return t[0]
    paths = [p for p, _ in files]
    if paths and all(p.endswith(".md") or p.startswith("docs/") for p in paths):
        return "documentation"
    if paths and all(re.match(r"^(?:tools/|tests/|\.github/|\.githooks/|xtask/|mods/horizon/src/test/)", p)
                     for p in paths):
        return "tooling"
    if re.search(r"^perf\b|\b(?:faster|stutters?|allocations?|fps|mspt)\b", title, re.I):
        return "perf"
    if re.search(r"\bcrash", title, re.I):
        return "bug"
    return None


def backfill(api, apply: bool) -> list[tuple[int, list[str]]]:
    out = []
    for pr in api.open_pulls():
        have = {l["name"] for l in pr["labels"]}
        files = api.files(pr["number"])
        add = []
        if not have & KIND_LABELS and (k := guess_kind(pr["title"], pr["body"], files)):
            add.append(k)
        add += [a for a in areas(files) if a not in have]
        if add:
            out.append((pr["number"], add))
            if apply:
                api.add(pr["number"], add)
    return out


def create(api, apply: bool) -> list[str]:
    have = {n.lower() for n in api.existing_labels()}
    todo = [(label, "5319e7", desc) for _, label, desc in KINDS if label not in have]
    todo += [(label, "c5def5", desc) for label, desc, _ in AREAS if label not in have]
    for label, color, desc in todo if apply else []:
        api.create_label(label, color, desc)
    return [t[0] for t in todo]


if __name__ == "__main__":
    verb, rest = (sys.argv[1:2] or [""])[0], sys.argv[2:]
    if verb == "event" and not rest:
        sys.exit(event(Api(os.environ["REPO"])))
    if verb in ("create", "backfill") and rest in ([], ["--apply"]):
        api, apply = Api(os.environ.get("REPO", "horizon-rust/horizon")), rest == ["--apply"]
        for line in (create(api, apply) if verb == "create" else backfill(api, apply)):
            print(line)
        sys.exit(0)
    sys.exit(__doc__)
