#!/usr/bin/env python3
"""Checks `horizon-port-pack upgrade-datapack` on vanilla's own world generation.

Vanilla 26.2's worldgen (from the 26.2 server jar the player downloaded
themselves, `--jar-26-2`, default `.local/vanilla-26.2/server-game.jar`) is
written out as a data pack, upgraded, and compared with vanilla 26.3's
worldgen (`.local/minecraft/26.3/jars/server-game.jar`) file by file: the
registries whose format changed and whose content did not must come out
identical. Per registry it prints how many files match and the paths where
the rest differ, most common first. Density functions and noise settings
differ where Mojang rewrote expressions (`lerp` for the blend formulas) or
changed content; those are judged by generating terrain, not here.

Nothing is written outside a temporary folder; the jars' files stay there.
"""
import argparse
import collections
import json
import pathlib
import subprocess
import sys
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import lab  # noqa: E402

PREFIXES = ("data/minecraft/worldgen/", "data/minecraft/dimension_type/")
RENAMES = {"configured_carver": "carver", "configured_feature": "feature"}


def extract(jar, out):
    with zipfile.ZipFile(jar) as z:
        for name in z.namelist():
            if name.startswith(PREFIXES) and name.endswith(".json"):
                target = out / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(z.read(name))


def files(root):
    return {p.relative_to(root).as_posix(): p for p in root.rglob("*.json")}


def diff(x, y, path, out):
    if isinstance(x, dict) and isinstance(y, dict):
        for k in set(x) | set(y):
            if k not in y:
                out.add(f"-{path}.{k}")
            elif k not in x:
                out.add(f"+{path}.{k}")
            else:
                diff(x[k], y[k], f"{path}.{k}", out)
    elif isinstance(x, list) and isinstance(y, list) and len(x) == len(y):
        for a, b in zip(x, y):
            diff(a, b, f"{path}[]", out)
    elif x != y:
        out.add(f"~{path}")


def normalise(v, providers):
    """26.3 names some block state providers (`worldgen/block_state_provider`)
    where 26.2 wrote them inline, and writes a default state as its bare id:
    both forms are read the same."""
    if isinstance(v, dict):
        if set(v) == {"id"}:
            return v["id"]
        if "spawns_by_category" in v:
            # The upgrade keeps empty categories so that none comes from
            # the dimension's defaults; vanilla leaves them out.
            v = dict(v, spawns_by_category={c: s for c, s in v["spawns_by_category"].items() if s})
        out = {}
        for k, x in v.items():
            if k.endswith("provider") and isinstance(x, str) and x in providers:
                x = providers[x]
            out[k] = normalise(x, providers)
        return out
    if isinstance(v, list):
        return [normalise(x, providers) for x in v]
    return v


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--jar-26-2", type=pathlib.Path,
                        default=ROOT / ".local/vanilla-26.2/server-game.jar")
    parser.add_argument("--registry", action="append", help="only these registries (e.g. worldgen/noise)")
    parser.add_argument("--show", type=int, default=6, help="diff paths shown per registry")
    parser.add_argument("--keep", type=pathlib.Path, help="copy both trees here to inspect them")
    args = parser.parse_args()
    new_jar = ROOT / ".local/minecraft/26.3/jars/server-game.jar"
    for jar in (args.jar_26_2, new_jar):
        if not jar.exists():
            sys.exit(f"missing {jar}")
    target = lab.cargo_target()
    subprocess.run(["cargo", "build", "-q", "-p", "horizon-port-pack", "--target-dir", str(target)],
                   cwd=ROOT, check=True)
    exe = target / "debug" / "horizon-port-pack"
    with tempfile.TemporaryDirectory(prefix="upgrade-rosetta-") as tmp:
        tmp = pathlib.Path(tmp)
        old, new, out = tmp / "26.2", tmp / "26.3", tmp / "upgraded"
        extract(args.jar_26_2, old)
        extract(new_jar, new)
        (old / "pack.mcmeta").write_text(json.dumps({"pack": {"description": "vanilla 26.2", "pack_format": 107}}))
        run = subprocess.run([str(exe), "upgrade-datapack", str(old), str(out)], capture_output=True, text=True)
        print(run.stdout.strip())
        if run.returncode:
            sys.exit(run.stderr)
        got, want = files(out), files(new)
        providers = {}
        for path, p in want.items():
            prefix = "data/minecraft/worldgen/block_state_provider/"
            if path.startswith(prefix):
                providers["minecraft:" + path[len(prefix):-5]] = json.loads(p.read_text())
        if args.keep:
            import shutil
            shutil.copytree(out, args.keep / "upgraded", dirs_exist_ok=True)
            shutil.copytree(new, args.keep / "26.3", dirs_exist_ok=True)
        per =collections.defaultdict(lambda: [0, 0, 0, collections.Counter()])
        for path, p in sorted(got.items()):
            parts = path.split("/")
            if len(parts) < 4 or path.startswith("data/minecraft/worldgen/density_function/") and "/upgraded/" in path:
                continue
            registry = "/".join(parts[2:4]) if parts[2] == "worldgen" else parts[2]
            if args.registry and registry not in args.registry:
                continue
            row = per[registry]
            if path not in want:
                row[2] += 1
                continue
            a = normalise(json.loads(p.read_text()), providers)
            b = normalise(json.loads(want[path].read_text()), providers)
            if a == b:
                row[0] += 1
            else:
                row[1] += 1
                paths = set()
                diff(a, b, "", paths)
                row[3].update(paths)
        print(f"{'registry':40} {'same':>5} {'differ':>6} {'not in 26.3':>11}")
        for registry, (same, differ, gone, paths) in sorted(per.items()):
            print(f"{registry:40} {same:5} {differ:6} {gone:11}")
            for p, n in paths.most_common(args.show):
                print(f"    {n:4}  {p}")


if __name__ == "__main__":
    main()
