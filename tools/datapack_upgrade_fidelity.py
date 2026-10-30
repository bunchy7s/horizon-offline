#!/usr/bin/env python3
"""Does an upgraded data pack generate on 26.3 what the original did on 26.2?

Starts vanilla's dedicated server of each version in a new folder, with the
pack in its world's `datapacks/` (26.2 gets `--old-pack`, 26.3 gets
`--new-pack`, the output of `horizon-port-pack upgrade-datapack`; without
packs both run vanilla, which calibrates what 26.3 itself changed), force
loads the same chunks, saves, and compares the saved chunks block by block.
Prints how many blocks differ, the most common (26.2 -> 26.3) pairs and
which heights they are at. The server logs are checked for data pack and
registry errors: a pack 26.3 refuses fails the run.

Jars: `.local/vanilla-26.2/server.jar` and `.local/minecraft/26.3/jars/server.jar`
(the bundler jars the player downloaded). Needs Java 25 on PATH or JAVA_HOME.

usage: datapack_upgrade_fidelity.py [--old-pack P --new-pack Q] [--seed N]
           [--site X,Z]... [--radius R] [--status full|carvers] [--keep DIR]
"""
import argparse
import collections
import os
import pathlib
import re
import shutil
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import anvil_dump  # noqa: E402
from test_dedicated_server_parity import Server, free_port  # noqa: E402

JARS = {"26.2": ROOT / ".local/vanilla-26.2/server.jar", "26.3": ROOT / ".local/minecraft/26.3/jars/server.jar"}
ERRORS = re.compile(r"Failed to (load|parse|get element)|Errors in registry|Unknown registry key|"
                    r"No key|Missing|Couldn't load|Failed to load datapacks|Exception", re.I)


def java():
    home = os.environ.get("JAVA_HOME")
    return str(pathlib.Path(home) / "bin" / "java") if home else "java"


def run_server(version, folder, seed, pack, chunks):
    folder.mkdir(parents=True)
    (folder / "eula.txt").write_text("eula=true\n")
    (folder / "server.properties").write_text(
        f"level-seed={seed}\nonline-mode=false\nserver-port={free_port()}\nspawn-protection=0\n"
        "max-tick-time=-1\nsync-chunk-writes=true\nview-distance=2\nsimulation-distance=2\n")
    if pack:
        target = folder / "world/datapacks" / pack.name
        target.parent.mkdir(parents=True)
        (shutil.copytree if pack.is_dir() else shutil.copy)(pack, target)
    server = Server([java(), "-Xmx2G", "-jar", JARS[version], "nogui"], folder, f"server-{version}")
    try:
        server.wait_for(r"Done \(", timeout=1800)
        problems = [l for l in server.lines if ERRORS.search(l)]
        if pack and not any(pack.name in l for l in server.lines):
            problems.append(f"{version}: the pack {pack.name} was not mentioned in the log (not loaded?)")
        # One forceload per chunk: a rectangle over several sites would pass
        # the command's 256-chunk limit and load nothing.
        for x, z in chunks:
            server.console(f"forceload add {x * 16} {z * 16}")
        deadline = time.time() + 3600
        for poll in range(100000):
            if time.time() > deadline:
                raise AssertionError(f"{version}: chunks still not loaded after an hour")
            start = len(server.lines)
            for x, z in chunks:
                server.console(f"execute unless loaded {x * 16} 0 {z * 16} run say unloaded")
            server.console(f"say poll {poll}")
            server.wait_for(rf"\[Server\] poll {poll}$", timeout=1800)
            if not any(l.endswith("[Server] unloaded") for l in server.lines[start:]):
                break
            # Polls are cheap, but each adds lines to the kept log.
            del server.lines[start:]
            time.sleep(5)
        server.console("save-all flush")
        server.wait_for(r"Saved the game", timeout=600)
    finally:
        server.stop()
    return problems


def blocks(world, chunks):
    """{(cx, cz): {y: [state per x,z]}} of the saved world."""
    regions = {}
    out = {}
    for cx, cz in chunks:
        rx, rz = cx >> 5, cz >> 5
        if (rx, rz) not in regions:
            # 26.x keeps each dimension under dimensions/<ns>/<name>/; older worlds used region/.
            path = world / "dimensions" / "minecraft" / "overworld" / "region" / f"r.{rx}.{rz}.mca"
            if not path.exists():
                path = world / "region" / f"r.{rx}.{rz}.mca"
            regions[(rx, rz)] = anvil_dump.region_chunks(path) if path.exists() else {}
        chunk = regions[(rx, rz)].get((cx & 31, cz & 31))
        if chunk is None:
            continue
        status = chunk.get("Status", "")
        column = {}
        for section in chunk.get("sections", []):
            sy = section["Y"][1] if isinstance(section["Y"], tuple) else section["Y"]
            states = section.get("block_states")
            if not states:
                continue
            palette = [state_name(p) for p in states["palette"]]
            data = states.get("data", ("L", []))[1]
            bits = max(4, (len(palette) - 1).bit_length())
            per = 64 // bits
            mask = (1 << bits) - 1
            for i in range(4096):
                if data:
                    word = data[i // per] & ((1 << 64) - 1)
                    index = (word >> ((i % per) * bits)) & mask
                else:
                    index = 0
                y = sy * 16 + (i >> 8)
                column.setdefault(y, [None] * 256)[i & 255] = palette[index]
        out[(cx, cz)] = (status, column)
    return out


def state_name(entry):
    # 26.2 saves {Name, Properties}; 26.3 saves the bare id, or {id, properties}. In a list
    # mixing both, NBT wraps each bare id in a compound with an empty key.
    if isinstance(entry, str):
        return entry
    if "" in entry:
        return entry[""]
    name = entry.get("Name") or entry["id"]
    props = entry.get("Properties") or entry.get("properties")
    if props:
        name += "[" + ",".join(f"{k}={v}" for k, v in sorted(props.items())) + "]"
    return name


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--old-pack", type=pathlib.Path)
    parser.add_argument("--new-pack", type=pathlib.Path)
    parser.add_argument("--seed", default="12345")
    parser.add_argument("--site", action="append", help="chunk X,Z of a site's centre (default 0,0 and 40,-25)")
    parser.add_argument("--radius", type=int, default=1, help="chunks around each site")
    parser.add_argument("--keep", type=pathlib.Path, help="keep the server folders here")
    parser.add_argument("--top", type=int, default=20)
    args = parser.parse_args()
    if bool(args.old_pack) != bool(args.new_pack):
        sys.exit("pass both packs or neither")
    sites = [tuple(map(int, s.split(","))) for s in (args.site or ["0,0", "40,-25"])]
    chunks = sorted({(x + dx, z + dz) for x, z in sites
                     for dx in range(-args.radius, args.radius + 1)
                     for dz in range(-args.radius, args.radius + 1)})
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="upgrade-fidelity-"))
    try:
        worlds, problems = {}, []
        for version, pack in (("26.2", args.old_pack), ("26.3", args.new_pack)):
            folder = tmp / version
            print(f"{version}: generating {len(chunks)} chunks" + (f" with {pack.name}" if pack else ""), flush=True)
            problems += run_server(version, folder, args.seed, pack, chunks)
            worlds[version] = blocks(folder / "world", chunks)
        for p in problems:
            print("LOG:", p)
        old, new = worlds["26.2"], worlds["26.3"]
        total = differ = 0
        pairs = collections.Counter()
        heights = collections.Counter()
        missing = 0
        for c in chunks:
            if c not in old or c not in new:
                missing += 1
                continue
            (s1, a), (s2, b) = old[c], new[c]
            for y in sorted(set(a) | set(b)):
                ra, rb = a.get(y, ["minecraft:air"] * 256), b.get(y, ["minecraft:air"] * 256)
                for i in range(256):
                    total += 1
                    if ra[i] != rb[i]:
                        differ += 1
                        pairs[(ra[i], rb[i])] += 1
                        heights[(y // 64) * 64] += 1
        print(f"compared {total} blocks in {len(chunks) - missing} chunks ({missing} missing): "
              f"{differ} differ ({100 * differ / max(total, 1):.3f}%)")
        for (x, y), n in pairs.most_common(args.top):
            print(f"  {n:8}  {x} -> {y}")
        print("  by height:", ", ".join(f"y{h}..{h + 63}: {n}" for h, n in sorted(heights.items())))
        if problems or missing:
            sys.exit(1)
    finally:
        if args.keep:
            shutil.copytree(tmp, args.keep, dirs_exist_ok=True)
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
