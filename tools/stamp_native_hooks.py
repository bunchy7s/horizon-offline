#!/usr/bin/env python3
"""Assign API numbers to pending native hook fragments."""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRAGMENTS = Path("tools/native-hook-fragments")
SOURCE = Path("crates/horizon-client/src/game/native_hooks.rs")
DOCS = Path("docs/native-mods.md")
CATALOG = Path("native-hooks.json")
# The mod API's types and the files `cargo run -p xtask -- mod-api` generates: they copy hook and
# helper comments, markers included, so every marker is stamped in them too and `--check` holds.
MOD_API = (
    Path("crates/horizon-client/src/mod_api.rs"),
    Path("crates/horizon-client/src/mod_api/generated.rs"),
    Path("crates/horizon-client/src/mod_api/generated_host.rs"),
    Path("crates/horizon-client/src/game/mod_api_host_generated.rs"),
    Path("crates/horizon-client-render/src/gfx/native_render/generated.rs"),
)


def stamp(check):
    paths = sorted((ROOT / FRAGMENTS).glob("*.json"))
    if not paths:
        return False
    if check:
        print(f"{len(paths)} pending native hook fragment(s)")
        return True

    # First-parent additions give merge order, not author timestamps or filenames.
    result = subprocess.run(
        ["git", "log", "--first-parent", "--reverse", "--diff-merges=first-parent",
         "--format=", "--name-only", "--diff-filter=A", "--", str(FRAGMENTS)],
        cwd=ROOT, check=True, text=True, capture_output=True,
    )
    pending = {p.relative_to(ROOT).as_posix(): p for p in paths}
    ordered = [pending.pop(line) for line in result.stdout.splitlines() if line in pending]
    # A developer can stamp uncommitted fragments for a local test.
    ordered.extend(pending[name] for name in sorted(pending))

    source = (ROOT / SOURCE).read_text()
    docs = (ROOT / DOCS).read_text()
    catalog = json.loads((ROOT / CATALOG).read_text())
    match = re.search(r"^pub\(crate\) const NATIVE_HOOKS_API: u32 = (\d+);$", source, re.M)
    if not match:
        raise ValueError("missing numeric NATIVE_HOOKS_API")
    version = int(match.group(1))
    names = {hook["name"] for hook in catalog["hooks"]}
    extra = {}
    for path in ordered:
        fragment = json.loads(path.read_text())
        version += 1
        marker = f"@API:{path.stem}@"
        for hook in fragment["hooks"]:
            if hook["name"] in names:
                raise ValueError(f"duplicate hook: {hook['name']}")
            names.add(hook["name"])
            entry = {"name": hook["name"], "version": 1, "target": hook["target"]}
            # The catalog's optional facts about a hook travel with it.
            entry.update({k: hook[k] for k in ("neoforge", "side", "cancellable", "mod_api") if k in hook})
            catalog["hooks"].append(entry)
        rust_summary = fragment["rust_summary"].replace(marker, str(version))
        doc_summary = fragment["doc_summary"].replace(marker, str(version))
        if not rust_summary or not doc_summary:
            raise ValueError(f"{path}: expected summaries")
        rust_entry = f"//! API {version} adds " + rust_summary.replace("\n", "\n//! ") + "\n//!\n"
        if "//! API " not in source:
            raise ValueError("missing source history insertion point")
        source = source.replace("//! API ", rust_entry + "//! API ", 1)
        docs_marker = "\n## What the launcher does"
        if docs_marker not in docs:
            raise ValueError("missing documentation insertion point")
        docs = docs.replace(docs_marker, f"\nAPI {version} adds {doc_summary}\n" + docs_marker, 1)
        for filename in fragment.get("comments", []):
            file = Path(filename)
            if file.is_absolute() or ".." in file.parts or file in (SOURCE, DOCS, CATALOG, *MOD_API):
                raise ValueError(f"invalid comment path: {filename}")
            text = extra.get(file, (ROOT / file).read_text())
            if marker not in text:
                raise ValueError(f"missing {marker} in {filename}")
            extra[file] = text.replace(marker, str(version))
        # Most hook comments live in the source itself.
        source = source.replace(marker, str(version))
        for file in MOD_API:
            if (ROOT / file).exists():
                text = extra.get(file, (ROOT / file).read_text())
                extra[file] = text.replace(marker, str(version))

    source = re.sub(
        r"^(pub\(crate\) const NATIVE_HOOKS_API: u32 = )\d+;$",
        lambda m: f"{m.group(1)}{version};", source, count=1, flags=re.M)
    (ROOT / SOURCE).write_text(source)
    (ROOT / DOCS).write_text(docs)
    (ROOT / CATALOG).write_text(json.dumps(catalog, indent=2, ensure_ascii=False) + "\n")
    for file, content in extra.items():
        (ROOT / file).write_text(content)
    for path in ordered:
        path.unlink()
    print(f"Stamped {len(ordered)} fragment(s), through API {version}")
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="fail if fragments are pending")
    args = parser.parse_args()
    try:
        return int(stamp(args.check))
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"stamp_native_hooks: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
