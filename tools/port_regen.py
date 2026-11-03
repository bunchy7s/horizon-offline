#!/usr/bin/env python3
"""Regenerate what Horizon derives from vanilla, for a new version, in a candidate worktree
(docs/version-ports.md, "Regenerate").

Used through `version_port.py regen NEW`:

1. the candidate: a worktree of `port/NEW` (from --base, default origin/main), its research-lock.json pinned to
   NEW (version, version JSON, jars, Java);
2. its .local: NEW's jars, libraries and manifest prepared by tools/lab.py prepare from the version store (a
   launcher-shaped copy of the download), NEW's data generator reports, the shared tools;
3. the two vanilla classes the generators parse (EntityTypes, BlockStateData) decompiled alone, with the full
   jar and its libraries for types, into the version store;
4. every generator run there, each recorded as unchanged, regenerated (with its diff stat) or failed (with the
   end of its output); the regenerated tree is committed on port/NEW, never pushed.

Nothing here writes into the shared .local (~/code/horizon-parity/shared-local): the candidate links only what
it reads.
"""

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

try:
    from . import lab
except ImportError:
    import lab

# Classes whose decompiled source a generator parses: (side, class).
SOURCES = [("client", "net/minecraft/world/entity/EntityTypes"),
           ("server", "net/minecraft/util/datafix/fixes/BlockStateData")]
PARITY = "mods/horizon/src/test/java/dev/horizonlab/parity"


# --- the candidate -----------------------------------------------------------------------------------------

def git(*args, cwd=lab.ROOT, check=True):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=check).stdout


def candidate(version, base, path):
    """The port/VERSION worktree at path, made from base unless it exists."""
    if (path / ".git").exists():
        return path
    path.parent.mkdir(parents=True, exist_ok=True)
    branch = f"port/{version}"
    exists = git("rev-parse", "--verify", "--quiet", f"refs/heads/{branch}", check=False).strip()
    git("worktree", "add", *([str(path), branch] if exists else ["-b", branch, str(path), base]))
    return path


def pinned_lock(lock, manifest, manifest_sha1):
    """research-lock.json pinned to the version this manifest describes. The same version keeps its block (and
    the republished version JSONs it accepts); another version gets a new one."""
    game = lock["minecraft"]
    same = (game["version"] == manifest["id"]
            and manifest_sha1 in [game["manifest_sha1"], *game.get("accepted_manifest_sha1", [])]
            and game["client_sha1"] == manifest["downloads"]["client"]["sha1"]
            and game["server_sha1"] == manifest["downloads"]["server"]["sha1"])
    if same:
        return lock
    return {**lock, "minecraft": {
        "version": manifest["id"], "manifest_sha1": manifest_sha1,
        "client_sha1": manifest["downloads"]["client"]["sha1"],
        "server_sha1": manifest["downloads"]["server"]["sha1"],
        "java_major": manifest["javaVersion"]["majorVersion"],
    }}


def pin(root, version_json):
    lock_path = root / "research-lock.json"
    lock = json.loads(lock_path.read_text())
    pinned = pinned_lock(lock, json.loads(version_json.read_text()), lab.digest(version_json, "sha1"))
    if pinned is not lock:
        lab.write_json(lock_path, pinned)


def link(target, link_path):
    link_path.parent.mkdir(parents=True, exist_ok=True)
    if link_path.is_symlink() or link_path.exists():
        if link_path.is_symlink() and Path(os.readlink(link_path)) == target:
            return
        raise SystemExit(f"{link_path} exists and is not a link to {target}; remove it")
    link_path.symlink_to(target)


def launcher_tree(store, manifest):
    """A launcher-shaped copy of a downloaded version (versions/ID/ID.json and .jar, libraries/), so that
    tools/lab.py prepare imports it exactly as it imports a player's install."""
    tree, version = store / "launcher", manifest["id"]
    versions = tree / "versions" / version
    versions.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(store / "version.json", versions / f"{version}.json")
    jar = versions / f"{version}.jar"
    if not jar.exists():
        os.link(store / "jars" / "client.jar", jar)
    for artifact in lab.client_library_artifacts(manifest):
        lab.download(artifact["url"], tree / "libraries" / artifact["path"], artifact["sha1"])
    return tree


def prepare_local(root, version, found, store):
    """The candidate's .local: tools from the shared one, the version's prepared jars and its reports."""
    shared = Path(os.environ.get("HORIZON_SHARED_LOCAL", Path.home() / "code/horizon-parity/shared-local"))
    local = root / ".local"
    if local.is_symlink():
        raise SystemExit(f"{local} links to a shared directory; a candidate needs its own")
    link(shared / "tools", local / "tools")
    link(found["reports"], local / "datagen" / "out" / "reports")
    jars = found["client"].parent
    if jars.parent.name == version and (jars.parent / "prepared.json").exists() and store not in jars.parents:
        link(jars.parent, local / "minecraft" / version)  # The pinned version, prepared by lab.py already.
        return
    game = store / "game"
    game.mkdir(exist_ok=True)
    link(game, local / "minecraft" / version)
    if not (game / "prepared.json").exists():
        tree = launcher_tree(store, json.loads(found["version_json"].read_text()))
        run([python(), "tools/lab.py", "prepare", "--minecraft-dir", tree, "--server-jar", found["server"]], root)


_PYTHON = None


def python():
    """The interpreter to run the other tools with. Git for Windows puts a
    `python3` on PATH that is only the Store stub: it prints "Python" and exits
    49, so probe the names and keep the first one that runs Python 3."""
    global _PYTHON
    if _PYTHON is None:
        _PYTHON = "python3"
        for name in ("python3", "python", "py"):
            found = shutil.which(name)
            probe = [found, "-c", "import sys; sys.exit(sys.version_info < (3,))"] if found else None
            if probe and subprocess.run(probe, capture_output=True).returncode == 0:
                _PYTHON = found
                break
    return _PYTHON


class Failed(RuntimeError):
    """A command that exited non-zero, with all it printed."""

    def __init__(self, command, code, output):
        super().__init__(f"{' '.join(map(str, command))[:300]} exited {code}:\n{output[-2500:]}")
        self.output = output


def run(command, cwd, log=None, env=None):
    """Run a command; on failure raise Failed."""
    result = subprocess.run([str(c) for c in command], cwd=cwd, capture_output=True, text=True, env=env)
    if log:
        log.write_text(result.stdout + result.stderr)
    if result.returncode:
        raise Failed(command, result.returncode, result.stdout + result.stderr)
    return result.stdout


def compile_errors(output, limit=12):
    """javac's errors, one line each: "ParityData.java:45: cannot find symbol: variable WORLD (class
    RegistryLayer)" names the vanilla API an oracle used that the new version changed."""
    lines = output.splitlines()
    found = []
    for index, line in enumerate(lines):
        match = re.match(r".*?([\w$]+\.java):(\d+): error: (.*)", line)
        if not match:
            continue
        entry = f"{match[1]}:{match[2]}: {match[3]}"
        after = [text.strip() for text in lines[index + 1:index + 6]]
        symbol = next((text.split(":", 1)[1].strip() for text in after if text.startswith("symbol:")), None)
        where = next((text.split(":", 1)[1].strip() for text in after if text.startswith("location:")), None)
        found.append(entry + (f": {symbol}" if symbol else "") + (f" ({where})" if where else ""))
    return found[:limit]


# --- decompiling the classes the generators parse ------------------------------------------------------------

def decompile_classes(root, version, store):
    """Decompile only SOURCES' classes (with their inner classes) into store/sources/SIDE, with the full jar and
    the side's libraries on the decompiler's path so types resolve as in a full run."""
    lock = json.loads((root / "research-lock.json").read_text())
    vineflower = root / ".local" / "tools" / f"vineflower-{lock['vineflower']['version']}.jar"
    prepared = json.loads((root / ".local" / "minecraft" / version / "prepared.json").read_text())
    for side, name in SOURCES:
        output = store / "sources" / side
        if (output / f"{name}.java").exists():
            continue
        jar = root / prepared["jars"][side]
        with tempfile.TemporaryDirectory(prefix=".decompile-", dir=store) as scratch:
            scratch = Path(scratch)
            small = scratch / "classes.jar"
            with zipfile.ZipFile(jar) as full, zipfile.ZipFile(small, "w") as out:
                for entry in full.namelist():
                    if entry == f"{name}.class" or (entry.startswith(f"{name}$") and entry.endswith(".class")):
                        out.writestr(entry, full.read(entry))
            command = ["java", "-Xmx2G", "-jar", vineflower, "--folder", "--skip-extra-files=1", "--log-level=warn",
                       f"--add-external={jar}", *(f"--add-external={root / p}" for p in prepared["libraries"][side]),
                       small, scratch / "out"]
            run(command, root)
            target = output / f"{name}.java"
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(scratch / "out" / f"{name}.java", target)


# --- the generators ------------------------------------------------------------------------------------------

def compile_oracle(root, version, classpath, sources, build):
    lock = json.loads((root / "research-lock.json").read_text())
    classes = build / "classes"
    classes.mkdir(parents=True, exist_ok=True)
    run(["javac", "--release", lock["minecraft"]["java_major"], "-proc:none", "-sourcepath",
         root / "mods/horizon/src/test/java", "-cp", os.pathsep.join(map(str, classpath)), "-d", classes,
         *sources], root)
    return classes


def classpaths(root, version):
    prepared = json.loads((root / ".local" / "minecraft" / version / "prepared.json").read_text())
    return {side: [root / prepared["jars"][side], *(root / p for p in prepared["libraries"][side])]
            for side in ("client", "server")}


def gen_suggestions(root, version, store, build):
    client = classpaths(root, version)["client"]
    classes = compile_oracle(root, version, client, [root / PARITY / "VanillaCommandSuggestionsExporter.java"],
                             build / "suggestions")
    tsv = build / "suggestions.tsv"
    run(["java", "-Xmx1G", "-cp", os.pathsep.join(map(str, [classes, *client])),
         "dev.horizonlab.parity.VanillaCommandSuggestionsExporter", tsv], build)
    return [python(), "tools/generate_command_suggestions.py", tsv]


def gen_vanilla_order(root, version, store, build):
    """vanilla's tree (the command-parse oracle's export) against Horizon's (command_parse_fixture tree)."""
    server = classpaths(root, version)["server"]
    classes = compile_oracle(root, version, server, [root / PARITY / "VanillaCommandParseOracle.java"],
                             build / "parse")
    run(["java", "-Xmx1G", "-cp", os.pathsep.join(map(str, [classes, *server])),
         "dev.horizonlab.parity.VanillaCommandParseOracle", "tree", build], build)
    env = dict(os.environ, CARGO_TARGET_DIR=str(root / ".local" / "port-target"))
    jobs = os.environ.get("FLEET_LOAD_JOBS")
    run(["cargo", "build", "-p", 'horizon-client-lib', "--example", "client_probe",
         *(["-j", jobs] if jobs else [])], root, env=env)
    fixture = root / ".local/port-target/debug/examples/client_probe"
    run([fixture, "command_parse_fixture", "tree", build / "rust_tree.json"], root)
    return [python(), "tools/generate_command_order.py", build / "tree.json", build / "rust_tree.json"]


# (name, outputs, how to get the generator's command: a list, or a function that prepares its input first).
GENERATORS = [
    ("version_consts", ["crates/horizon-core/src/version.rs"],
     lambda root, version, store, build: [python(), "tools/gen_version_consts.py"]),
    ("packet_ids", ["crates/horizon-protocol/src/ids.rs"],
     lambda root, version, store, build: [python(), "tools/generate_packet_ids.py"]),
    ("entity_tracking", ["crates/horizon-client/src/lan/tracking.rs"],
     lambda root, version, store, build: [python(), "tools/generate_entity_tracking.py", "--sources",
                                          store / "sources" / "client"]),
    ("block_state_data", ["crates/horizon-world/src/datafix/block_state_data.txt"],
     lambda root, version, store, build: [python(), "tools/gen_block_state_data.py",
                                          store / "sources/server" / f"{SOURCES[1][1]}.java"]),
    ("lan_command_order", ["crates/horizon-client/src/lan/command_order.rs"],
     lambda root, version, store, build: [python(), "tools/generate_lan_command_order.py"]),
    ("command_suggestions", ["crates/horizon-client/src/lan/command_suggestions.rs"], gen_suggestions),
    ("vanilla_command_order", ["crates/horizon-command/src/vanilla_order.rs"], gen_vanilla_order),
]


def status(root, outputs):
    """unchanged, or regenerated with {path: [added, removed]} against the candidate's last commit."""
    stat = {}
    for line in git("diff", "--numstat", "HEAD", "--", *outputs, cwd=root).splitlines():
        added, removed, path = line.split("\t")
        stat[path] = [int(added), int(removed)]
    return ("regenerated", stat) if stat else ("unchanged", {})


def run_generators(root, version, store, names=None):
    """Every generator in the candidate: {name: {status, outputs, stat | error}}."""
    build = root / ".local" / "port-regen"
    build.mkdir(parents=True, exist_ok=True)
    results = {}
    for name, outputs, command in GENERATORS:
        if names and name not in names:
            continue
        print(f"  {name}", flush=True)
        try:
            run(command(root, version, store, build), root, log=build / f"{name}.log")
            rust = [root / o for o in outputs if o.endswith(".rs")]
            if rust:
                run(["rustfmt", "--edition", "2024", *rust], root)
            state, stat = status(root, outputs)
            results[name] = {"status": state, "outputs": outputs, "stat": stat}
        except (RuntimeError, OSError, subprocess.CalledProcessError, KeyError, ValueError) as error:
            git("checkout", "HEAD", "--", *outputs, cwd=root, check=False)
            results[name] = {"status": "failed", "outputs": outputs, "error": str(error)[-2500:],
                             "compile_errors": compile_errors(getattr(error, "output", str(error)))}
        print(f"  {name}: {results[name]['status']}", flush=True)
    return results


def commit(root, version, results):
    """Commit the pin and what regenerated cleanly on port/VERSION (local only)."""
    paths = ["research-lock.json"] + [o for r in results.values() if r["status"] == "regenerated" for o in r["outputs"]]
    git("add", "--", *paths, cwd=root)
    if git("diff", "--cached", "--name-only", cwd=root).strip():
        git("commit", "-q", "-m", f"Port to Minecraft {version}: pin it and regenerate (version_port.py regen)",
            cwd=root)
    return git("rev-parse", "HEAD", cwd=root).strip()


def regen(version, found, store, base, path, names=None):
    """The whole stage: {old, new, candidate, commit, generators}."""
    old = json.loads(git("show", f"{base}:research-lock.json"))["minecraft"]["version"]
    root = candidate(version, base, path)
    pin(root, found["version_json"])
    print(f"Candidate {root} (port/{version}, was {old})", flush=True)
    prepare_local(root, version, found, store)
    decompile_classes(root, version, store)
    results = run_generators(root, version, store, names)
    return {"old": old, "new": version, "candidate": str(root), "commit": commit(root, version, results),
            "generators": results}
