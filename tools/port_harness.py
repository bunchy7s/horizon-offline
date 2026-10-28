#!/usr/bin/env python3
"""Run every parity harness against a new version, in its candidate worktree (docs/version-ports.md, "Run every
harness against the new version").

Used through `version_port.py harnesses NEW` after `regen NEW`:

1. horizon-prepare in the candidate (.local/horizon-client/NEW and its exporters' outputs), then its `sounds` and
   `panorama` steps from the version store's launcher-shaped copy, whose asset index and the objects those steps
   copy are downloaded once per version (sha1 checked);
2. tools/parity.py there: every harness, or the ones named;
3. each harness classified: pass, oracle (its Java oracle no longer compiles against NEW: javac names what
   changed), or fail; and, against a baseline run on the pinned version (a tools/parity.py state.json), whether it
   newly fails, so the known failures of main don't count.

The result goes to diff-OLD-NEW/harnesses.json, which `version_port.py report` reads.
"""

from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

try:
    from . import lab, port_regen
except ImportError:
    import lab
    import port_regen

RESOURCES_URL = "https://resources.download.minecraft.net"
# What horizon-prepare's sounds and panorama steps copy from the launcher's asset store
# (crates/horizon-prepare/src/sounds.rs is_sound, is_panorama).
PANORAMA = [f"minecraft/textures/gui/title/background/panorama_{i}.png" for i in range(6)] + [
    "minecraft/textures/gui/title/background/panorama_overlay.png"]


def wanted_asset(name):
    return name == "minecraft/sounds.json" or name.startswith("minecraft/sounds/") or name in PANORAMA


def launcher_assets(tree, manifest, jobs=8):
    """The asset index and the objects horizon-prepare's sounds and panorama steps read, in the launcher-shaped
    tree (assets/indexes, assets/objects/xx/HASH)."""
    info = manifest["assetIndex"]
    index = lab.download(info["url"], tree / "assets" / "indexes" / f"{info['id']}.json", info["sha1"])
    objects = json.loads(Path(index).read_text())["objects"]
    hashes = sorted({entry["hash"] for name, entry in objects.items() if wanted_asset(name)})

    def fetch(checksum):
        lab.download(f"{RESOURCES_URL}/{checksum[:2]}/{checksum}", tree / "assets" / "objects" / checksum[:2] / checksum,
                     checksum)

    with ThreadPoolExecutor(jobs) as pool:
        list(pool.map(fetch, hashes))
    return len(hashes)


def own_reports(root):
    """Replace the candidate's link to the store's data generator reports (regen reads them) by nothing:
    horizon-prepare runs the data generator itself, and it must write into the candidate, not through the link
    into the version store (the generator also deletes the link as stale)."""
    reports = root / ".local" / "datagen" / "out" / "reports"
    if reports.is_symlink():
        reports.unlink()


def prepare_resources(root, version, store):
    """horizon-prepare against the candidate's version: {status: ok | failed, error, compile_errors}."""
    own_reports(root)
    log = root / ".local" / "port-regen" / "horizon-prepare.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(root / ".local" / "port-target"))
    jobs = os.environ.get("FLEET_LOAD_JOBS")
    try:
        port_regen.run(["cargo", "build", "--release", "-p", "horizon-prepare", *(["-j", jobs] if jobs else [])],
                       root, env=env)
        binary = root / ".local/port-target/release/horizon-prepare"
        port_regen.run([binary, "--root", root], root, log=log)
        found = json.loads((store / "prepared.json").read_text())["paths"]
        manifest = json.loads(Path(found["version_json"]).read_text())
        tree = store / "launcher"
        if tree.exists():  # A downloaded version; the pinned one's sounds come from the player's launcher.
            launcher_assets(tree, manifest)
            for step in ("sounds", "panorama"):
                port_regen.run([binary, step, "--root", root, "--minecraft-dir", tree], root,
                               log=log.with_name(f"horizon-prepare-{step}.log"))
        return {"status": "ok"}
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        output = getattr(error, "output", str(error))
        return {"status": "failed", "error": str(error)[-2500:],
                "compile_errors": port_regen.compile_errors(output + named_logs(output))}


def named_logs(output):
    """The logs horizon-prepare's errors point to ("…; see /path/export.log"): its javac errors are there."""
    text = ""
    # A log path is absolute on every host: "/tmp/x.log" or "C:\tmp\x.log".
    for name in sorted(set(re.findall(r"see ((?:/|[A-Za-z]:[\\/])\S+\.log)", output))):
        path = Path(name)
        if path.is_file():
            text += path.read_text(errors="replace")[-(1 << 20):]
    return text


def run_harnesses(root, names=None, timeout=None):
    """tools/parity.py in the candidate, every harness from scratch (or the ones named); its state.json."""
    command = [port_regen.python(), "tools/parity.py", *(names or ["--all"])]
    if timeout:
        command += ["--timeout", str(timeout)]
    env = dict(os.environ, PARITY_HOSTS="")
    log = root / ".local" / "port-regen" / "parity.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as stream:
        subprocess.run(command, cwd=root, stdout=stream, stderr=subprocess.STDOUT, env=env)
    return root / ".local" / "parity-runner" / "state.json"


def harness_logs(logs, harness):
    """A harness's logs: its own, and its shards' (NAME#N.log)."""
    return sorted([*logs.glob(f"{harness}.log"), *logs.glob(f"{harness}#*.log")])


def statuses(state):
    """{harness: "pass" | "fail"} from a tools/parity.py state; a harness fails if one of its shards does."""
    found = {}
    for name, record in state.get("harnesses", {}).items():
        harness = name.split("#")[0]
        found[harness] = "fail" if record.get("status") != "pass" or found.get(harness) == "fail" else "pass"
    return found


def classify(state, logs, baseline=None, names=None):
    """{harness: {status: pass | oracle | fail, baseline, new, compile_errors}} for the harnesses that ran.
    oracle: the run's log has javac errors (the oracle no longer compiles against the new version). new: it does
    not pass now and passed in the baseline (or there is no baseline)."""
    now = statuses(state)
    before = statuses(baseline) if baseline else {}
    result = {}
    for harness, status in sorted(now.items()):
        if names and harness not in names:
            continue
        entry = {"status": status, "baseline": before.get(harness)}
        if status != "pass":
            text = "".join(path.read_text(errors="replace")[-(1 << 20):] for path in harness_logs(logs, harness))
            errors = port_regen.compile_errors(text)
            if errors:
                entry["status"] = "oracle"
                entry["compile_errors"] = errors
        entry["new"] = entry["status"] != "pass" and entry["baseline"] != "fail"
        result[harness] = entry
    return result


def summary(results):
    counts = {"pass": 0, "oracle": 0, "fail": 0, "new": 0, "known": 0}
    for entry in results.values():
        counts[entry["status"]] += 1
        if entry["status"] != "pass":
            counts["new" if entry["new"] else "known"] += 1
    return counts


def harnesses(version, store, root, baseline=None, names=None, timeout=None, previous=None):
    """The whole stage: {candidate, commit, prepare, harnesses, summary}. previous: an earlier result to classify
    again (against another baseline) without preparing or running anything."""
    root = Path(root)
    if not (root / ".git").exists():
        raise SystemExit(f"No candidate at {root}; run version_port.py regen {version} first")
    pinned = json.loads((root / "research-lock.json").read_text())["minecraft"]["version"]
    if pinned != version:
        raise SystemExit(f"{root} pins {pinned}, not {version}")
    if previous:
        prepared, names = dict(previous["prepare"]), names or list(previous["harnesses"])
        if prepared["status"] == "failed" and not prepared.get("compile_errors"):
            prepared["compile_errors"] = port_regen.compile_errors(named_logs(prepared.get("error", "")))
        state = json.loads((root / ".local" / "parity-runner" / "state.json").read_text())
    else:
        print(f"horizon-prepare in {root}", flush=True)
        prepared = prepare_resources(root, version, store)
        print(f"  {prepared['status']}", flush=True)
        print("tools/parity.py " + (" ".join(names) if names else "--all"), flush=True)
        state = json.loads(run_harnesses(root, names, timeout).read_text())
    base = json.loads(Path(baseline).read_text()) if baseline else None
    results = classify(state, root / ".local" / "parity-runner" / "logs", base, names)
    return {"candidate": str(root), "commit": port_regen.git("rev-parse", "HEAD", cwd=root).strip(),
            "baseline": str(baseline) if baseline else None, "prepare": prepared, "harnesses": results,
            "summary": summary(results)}


def keep_logs(result, destination):
    """Copy the logs of the harnesses that don't pass beside harnesses.json (the candidate may be removed)."""
    logs = Path(result["candidate"]) / ".local" / "parity-runner" / "logs"
    destination.mkdir(parents=True, exist_ok=True)
    for harness, entry in result["harnesses"].items():
        if entry["status"] != "pass":
            for path in harness_logs(logs, harness):
                shutil.copyfile(path, destination / path.name)
