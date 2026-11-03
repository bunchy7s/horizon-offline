#!/usr/bin/env python3
"""Hourly check: did Mojang republish the pinned version JSON (.github/workflows/mojang-republish.yml)?

    python3 tools/ci/mojang_republish.py [--dry-run]

Fetches Mojang's public version manifest and looks at the pinned version's entry. If its sha1 is the lock's
`manifest_sha1` or one of `accepted_manifest_sha1` (the rule of tools/lab.py `manifest_accepted`), it prints one
line and stops. Otherwise it downloads that version JSON and compares the client and server jar sha1s and the Java
major version with the lock, then opens an issue "Mojang republished 26.3: <sha8>" with the sha1s, a snippet to
paste into research-lock.json, and whether the jars are unchanged. An issue with that title, open or closed, is not opened again or commented on (no hourly noise).

The lock holds no library hashes, so libraries cannot be verified from it: "jars unchanged" covers the two jars
and the Java version only. Nothing from Mojang is stored; only hashes go in the issue. Needs the gh CLI;
--dry-run prints what it would do.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
LOCK = ROOT / "research-lock.json"
MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
TITLE = "Mojang republished {version}: "


def fetch_json(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        return json.load(response)


def accepted(game, checksum):
    """Same rule as tools/lab.py manifest_accepted."""
    return checksum == game["manifest_sha1"] or checksum in game.get("accepted_manifest_sha1", [])


def plan(lock, manifest, fetch, open_issues):
    """The action to take: None when the manifest is known, else {kind: open|skip, number, title, body, jars_unchanged}.

    open_issues: {title: number} of the issues, open or closed (a known one is not reported again). fetch(url) -> the JSON at url."""
    game = lock["minecraft"]
    version = game["version"]
    entry = next((v for v in manifest.get("versions", []) if v.get("id") == version), None)
    if entry is None:
        raise ValueError(f"{version} is not in Mojang's version manifest")
    checksum = entry["sha1"]
    if accepted(game, checksum):
        return None
    detail = fetch(entry["url"])
    downloads = detail.get("downloads", {})
    found = {
        "client_sha1": downloads.get("client", {}).get("sha1"),
        "server_sha1": downloads.get("server", {}).get("sha1"),
        "java_major": detail.get("javaVersion", {}).get("majorVersion"),
    }
    changed = sorted(key for key, value in found.items() if value != game[key])
    asset_index = detail.get("assetIndex", {}).get("sha1")
    verdict = ("Jars unchanged (client, server, Java major match the lock; libraries are not in the lock and were NOT "
               "checked): the version JSON differs only elsewhere, likely the asset index."
               if not changed else f"JARS CHANGED ({', '.join(changed)}): do not accept.")
    lock_snippet = {"accepted_manifest_sha1": game.get("accepted_manifest_sha1", []) + [checksum]}
    if asset_index and asset_index not in game.get("accepted_asset_index_sha1", []):
        lock_snippet["accepted_asset_index_sha1"] = game.get("accepted_asset_index_sha1", []) + [asset_index]
    body = (f"Mojang's manifest now lists a {version} version JSON the lock does not know.\n\n"
            f"- manifest sha1: `{checksum}`\n- url: {entry['url']}\n- assetIndex sha1: `{asset_index}`\n"
            f"- client sha1: `{found['client_sha1']}`, server sha1: `{found['server_sha1']}`, "
            f"Java major: `{found['java_major']}`\n\n**{verdict}**\n\n")
    if not changed:
        body += ("To accept it, update `minecraft` in `research-lock.json` (append these to the existing lists):\n\n"
                 f"```json\n{json.dumps(lock_snippet, indent=2)}\n```\n")
    title = TITLE.format(version=version) + checksum[:8]
    number = open_issues.get(title)
    return {"kind": "skip" if number else "open", "number": number, "title": title, "body": body,
            "jars_unchanged": not changed}


def gh(*arguments, dry_run=False):
    if dry_run:
        print("would run: gh " + " ".join(arguments[:4]) + " ...")
        return ""
    return subprocess.run(["gh", *arguments], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def open_issues_titled(version, dry_run=False, runner=gh):
    """{title: number} of issues, open or closed, whose title starts with the republish prefix (no label needed)."""
    prefix = TITLE.format(version=version)
    listed = json.loads(runner("issue", "list", "--state", "all", "--limit", "200", "--search",
                               f'"{prefix.strip()}" in:title', "--json", "number,title") or "[]")
    return {issue["title"]: issue["number"] for issue in listed if issue["title"].startswith(prefix)}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    lock = json.loads(LOCK.read_text())
    manifest = fetch_json(MANIFEST_URL)
    version = lock["minecraft"]["version"]
    try:
        listed = open_issues_titled(version)
    except (subprocess.CalledProcessError, FileNotFoundError):
        if not args.dry_run:
            raise
        listed = {}
    action = plan(lock, manifest, fetch_json, listed)
    if action is None:
        print(f"{version}: Mojang's version JSON is known to the lock, nothing to do.")
        return 0
    print(f"{action['kind']}: {action['title']} (jars unchanged: {action['jars_unchanged']})")
    if action["kind"] == "open":
        gh("issue", "create", "--title", action["title"], "--body", action["body"], dry_run=args.dry_run)
    if args.dry_run:
        print(action["body"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
