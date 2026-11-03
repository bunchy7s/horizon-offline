#!/usr/bin/env python3
"""How long Horizon takes to build, measured the same way on any checkout (docs/build-times.md).

    python3 tools/build_times.py run                          # cold + incremental, dev and fast profiles
    python3 tools/build_times.py run --profiles dev --only incremental --crates horizon-items
    python3 tools/build_times.py compare before.json after.json

`run` builds the game (`cargo build --locked -p horizon-client --profile P --timings`) and writes a JSON
report and a Markdown table under .local/build-times/reports/:

- cold: a build from an empty target folder of its own (never target/), per profile. The timings page of
  that build gives the slowest crates and the critical path: the chain of units, each waiting on the one
  before it, that ends last.
- no-op: the same build again with nothing changed (the floor of every incremental build).
- incremental: after a one-line change in one file of each large crate (by default horizon-world,
  horizon-client-render, horizon-client's game.rs and horizon-items), the build that follows, and which
  crates it recompiled.

The change is a comment line (--edit): it cannot change behaviour, yet cargo and rustc see a changed file.
The file is always put back byte for byte, with its old modification time, even on Ctrl+C or a crash; a
run that is killed outright leaves a note in .local/build-times/ and the next run restores the file first.
A file that something else wrote in the meantime is never put back: that would undo someone's work.
`run` refuses a checkout with uncommitted changes (--allow-dirty), and warns when other builds run,
because they skew every number.

`compare` puts two reports side by side: seconds before, after, the difference.
"""
import argparse
import base64
import contextlib
import datetime
import json
import os
import platform
import re
import shutil
import signal
import stat
import statistics
import subprocess
import sys
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = lab.ROOT
REPORT_FORMAT, REPORT_VERSION = "horizon-build-times", 1
WORK = Path(".local") / "build-times"
MARKER = ".horizon-build-times"
PACKAGE = "horizon-client"
PROFILES = ["dev", "fast"]
# The files an edit goes to by default: large, often changed, and each in a different layer. horizon-client's
# game.rs belongs to the game binary, the others to libraries the binary links. Any other crate in the
# workspace gets its lib.rs; --file crate=path picks another file.
EDIT_FILES = {
    "horizon-world": "crates/horizon-world/src/level/mod.rs",
    "horizon-client-render": "crates/horizon-client-render/src/gfx.rs",
    "horizon-client": "crates/horizon-client/src/game.rs",
    "horizon-items": "crates/horizon-items/src/items.rs",
}
EDIT_MODES = ["middle", "top", "append", "touch"]
# Programs that build, link or check Rust; any of them running beside the measurement skews it.
BUILD_PROCESSES = {"cargo", "rustc", "rustdoc", "clippy-driver", "rust-lld", "link", "lld-link", "ld", "ld.lld",
                   "mold", "cc", "clang", "gcc", "cl"}
TOP_CRATES = 15


class BuildTimesError(Exception):
    """A problem with the request or the checkout; shown without a traceback."""


# Cargo's timings page ---------------------------------------------------------------------------------------

def parse_timings(html):
    """The units of cargo's `--timings` page (its embedded UNIT_DATA array; empty when nothing was compiled); None when
    the page has none, or units without the fields this tool reads."""
    marker = re.search(r"\bconst UNIT_DATA\s*=\s*", html)
    if not marker:
        return None
    try:
        units, _ = json.JSONDecoder().raw_decode(html, marker.end())
    except ValueError:
        return None
    if not isinstance(units, list) or not all(isinstance(u, dict) and isinstance(u.get("name"), str) and
                                              all(isinstance(u.get(k), (int, float)) for k in ("i", "start", "duration"))
                                              for u in units):
        return None
    return units


TARGET_KINDS = {"lib", "bin", "test", "bench", "example"}


def unit_label(unit):
    if unit.get("mode") == "run-custom-build":
        return f"{unit['name']} (run build script)"
    target = (unit.get("target") or "").strip()
    # A bin, test or example: older cargo writes `bin "name"`, cargo 1.99 `name "bin"`.
    named = re.match(r'(\S+) "([^"]+)"$', target)
    if named:
        kind, name = named.groups() if named.group(1) in TARGET_KINDS else reversed(named.groups())
        target = kind if name == unit["name"] else f"{kind} {name}"
    return f"{unit['name']} ({target})" if target else unit["name"]


def unit_end(unit):
    return unit["start"] + unit["duration"]


def unblocked(unit, what):
    """The units this one let start (what = "units", or "rmeta_units" for those that needed only its metadata);
    cargo up to 1.92 calls the field `unlocked_`."""
    return unit.get(f"unblocked_{what}") or unit.get(f"unlocked_{what}") or []


def metadata_ready(unit):
    """When a unit's dependents that need only its metadata (.rmeta) can start: `rmeta_time` (older cargo), else the
    end of its frontend section, else its end."""
    if isinstance(unit.get("rmeta_time"), (int, float)):
        return unit["start"] + unit["rmeta_time"]
    for name, span in unit.get("sections") or []:
        if str(name).lower() == "frontend":
            return unit["start"] + span["end"]
    return unit_end(unit)


def critical_path(units):
    """The chain of units that decided when the build ended, first unit first.

    Start at the unit that finished last. Its predecessor is the dependency that made it wait longest: for
    each unit, the one among those that unblock it that was ready last (finished, or for a dependent that
    only needs metadata, got through its frontend). Repeat until a unit that waited on nothing."""
    if not units:
        return []
    by_index = {unit["i"]: unit for unit in units}
    ready = {}
    for unit in units:
        for child in unblocked(unit, "units"):
            ready.setdefault(child, []).append((unit_end(unit), unit["i"]))
        for child in unblocked(unit, "rmeta_units"):
            ready.setdefault(child, []).append((metadata_ready(unit), unit["i"]))
    current = max(units, key=lambda u: (unit_end(u), u["i"]))
    path, seen = [current], {current["i"]}
    while True:
        waits = [(t, i) for t, i in ready.get(current["i"], []) if i in by_index and i not in seen]
        if not waits:
            break
        current = by_index[max(waits)[1]]
        seen.add(current["i"])
        path.append(current)
    path.reverse()
    return [{"unit": unit_label(u), "start_s": round(u["start"], 2), "end_s": round(unit_end(u), 2),
             "seconds": round(u["duration"], 2)} for u in path]


def summarize_timings(units, top=TOP_CRATES):
    """Per crate (a package: its lib, build script, bins added up) and the critical path of one build."""
    if not units:
        return None
    crates = {}
    for unit in units:
        entry = crates.setdefault(unit["name"], {"crate": unit["name"], "seconds": 0.0, "units": 0})
        entry["seconds"] += unit["duration"]
        entry["units"] += 1
    ranked = sorted(crates.values(), key=lambda c: (-c["seconds"], c["crate"]))
    for entry in ranked:
        entry["seconds"] = round(entry["seconds"], 2)
    path = critical_path(units)
    return {"units": len(units), "total_s": round(max(unit_end(u) for u in units), 2),
            "crates": ranked, "top": ranked[:top], "critical_path": path}


# The machine and the checkout -------------------------------------------------------------------------------

def machine_info():
    try:
        try:
            from . import perf_bench
        except ImportError:
            import perf_bench
        info = perf_bench.hardware()
    except Exception:
        info = {"os": sys.platform, "arch": platform.machine(), "cpu_threads": os.cpu_count(),
                "os_version": platform.platform(), "cpu": platform.processor(), "ram_mb": 0}
    info.setdefault("cpu_threads", os.cpu_count())
    return info


def run_text(command, cwd=None):
    """The stdout of a command, stripped; "" when it is missing or fails."""
    try:
        done = subprocess.run(command, cwd=cwd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    except OSError:
        return ""
    return done.stdout.strip() if done.returncode == 0 else ""


def toolchain_info(cargo, root):
    # In the checkout, as the builds run: a rustup override or toolchain file there picks another rustc.
    version = run_text([*cargo, "--version"], cwd=root)
    rustc = run_text(["rustc", "-vV"], cwd=root)
    fields = dict(line.split(": ", 1) for line in rustc.splitlines() if ": " in line)
    return {"cargo": version or "unknown", "rustc": (rustc.splitlines() or ["unknown"])[0],
            "host": fields.get("host", "unknown"), "llvm": fields.get("LLVM version", "unknown")}


def git(root, *args):
    return run_text(["git", *args], cwd=root)


def commit_info(root):
    return {"sha": git(root, "rev-parse", "HEAD"), "short": git(root, "rev-parse", "--short=9", "HEAD"),
            "branch": git(root, "rev-parse", "--abbrev-ref", "HEAD"), "subject": git(root, "log", "-1", "--format=%s"),
            "dirty": bool(git(root, "status", "--porcelain"))}


def require_clean(root, allow_dirty):
    """The checkout must have no uncommitted changes: a number is only about a commit if the tree is that commit."""
    done = subprocess.run(["git", "status", "--porcelain"], cwd=root, capture_output=True, text=True,
                          encoding="utf-8", errors="replace")
    if done.returncode != 0:
        raise BuildTimesError(f"{root} is not a git checkout (git status failed)")
    if done.stdout.strip() and not allow_dirty:
        lines = done.stdout.strip().splitlines()
        raise BuildTimesError("the working tree has uncommitted changes, so the numbers would not be about a "
                              "commit (commit or stash them, or pass --allow-dirty):\n  " + "\n  ".join(lines[:8])
                              + ("\n  ..." if len(lines) > 8 else ""))


def other_build_processes():
    """Names of other cargo, rustc or linker processes running now (this tool's own children are not running yet)."""
    if sys.platform == "win32":
        out = run_text(["tasklist", "/FO", "CSV", "/NH"])
        names = [line.split('","')[0].strip('"') for line in out.splitlines() if line]
        names = [n[:-4] if n.lower().endswith(".exe") else n for n in names]
    else:
        names = [Path(n).name for n in run_text(["ps", "-A", "-o", "comm="]).splitlines()]
    return sorted({n for n in names if n.lower() in BUILD_PROCESSES})


def cpu_busy_percent(seconds=1.0):
    """How busy the whole machine is, over a short sample; None where it cannot be told."""
    try:
        if sys.platform == "win32":
            import ctypes
            from ctypes import wintypes

            def times():
                idle, kernel, user = (wintypes.FILETIME() for _ in range(3))
                ctypes.windll.kernel32.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user))
                value = lambda f: (f.dwHighDateTime << 32) | f.dwLowDateTime
                return value(idle), value(kernel) + value(user)  # kernel time includes idle time
        elif Path("/proc/stat").exists():
            def times():
                fields = [int(x) for x in Path("/proc/stat").read_text().splitlines()[0].split()[1:]]
                return fields[3] + fields[4], sum(fields[:8])
        else:
            return round(min(100.0, 100 * os.getloadavg()[0] / (os.cpu_count() or 1)), 1)
        idle0, total0 = times()
        time.sleep(seconds)
        idle1, total1 = times()
        return round(100 * (1 - (idle1 - idle0) / max(1, total1 - total0)), 1)
    except Exception:
        return None


# Running cargo ----------------------------------------------------------------------------------------------

class Builder:
    """Runs `cargo build` for one profile in the tool's own target folder and reads back the timings page."""

    def __init__(self, root, cargo, target_dir, jobs, log_dir):
        self.root, self.cargo, self.target_dir, self.jobs, self.log_dir = root, cargo, Path(target_dir), jobs, log_dir
        self.count = 0

    def command(self, profile):
        command = [*self.cargo, "build", "--locked", "-p", PACKAGE, "--profile", profile, "--timings",
                   "--target-dir", str(self.target_dir), "--color", "never"]
        if self.jobs:
            command += ["--jobs", str(self.jobs)]
        return command

    def env(self):
        env = dict(os.environ)
        # An absolute --target-dir on the command line wins over this and over [build] target-dir in a config
        # file; the variable is set too so build scripts and tools that read it see the same folder.
        env["CARGO_TARGET_DIR"] = str(self.target_dir)
        env.pop("CARGO_BUILD_TARGET_DIR", None)
        env["CARGO_TERM_PROGRESS_WHEN"] = "never"
        return env

    def timing_pages(self):
        folder = self.target_dir / "cargo-timings"
        return {p.name for p in folder.glob("cargo-timing-*.html")} if folder.is_dir() else set()

    def build(self, profile, label):
        """One build. {"ok", "seconds", "log", "timings": summary or None, "units": [...] or None}."""
        self.count += 1
        self.log_dir.mkdir(parents=True, exist_ok=True)
        log = self.log_dir / f"{time.strftime('%Y%m%dT%H%M%S')}-{self.count:02d}-{label}.log"
        before = self.timing_pages()
        started = time.perf_counter()
        with open(log, "wb") as out:
            process = subprocess.Popen(self.command(profile), cwd=self.root, env=self.env(),
                                       stdout=out, stderr=subprocess.STDOUT)
            try:
                code = process.wait()
            except BaseException:
                process.kill()
                process.wait()
                raise
        seconds = time.perf_counter() - started
        units = None
        pages = sorted(self.timing_pages() - before)
        if pages:
            units = parse_timings((self.target_dir / "cargo-timings" / pages[-1]).read_text(encoding="utf-8",
                                                                                         errors="replace"))
        return {"ok": code == 0, "seconds": round(seconds, 2), "log": str(log), "code": code,
                "units": units, "timings": summarize_timings(units)}

    def fetch(self):
        """Download the crates beforehand, so a cold build is not charged for the network."""
        done = subprocess.run([*self.cargo, "fetch", "--locked"], cwd=self.root, capture_output=True, text=True,
                              encoding="utf-8", errors="replace")
        if done.returncode != 0:
            raise BuildTimesError("cargo fetch --locked failed:\n" + (done.stderr or done.stdout)[-2000:])


def tail(path, lines=25):
    try:
        return "\n".join(Path(path).read_text(encoding="utf-8", errors="replace").splitlines()[-lines:])
    except OSError:
        return ""


# The tool's own target folders ------------------------------------------------------------------------------

def prepare_target(root, target_dir, wipe):
    """Make the folder the tool builds in. Only a folder this tool made (it holds MARKER) is ever emptied."""
    target_dir = Path(target_dir).resolve()
    marker = target_dir / MARKER
    if target_dir.exists() and not marker.exists():
        if any(target_dir.iterdir()):
            raise BuildTimesError(f"{target_dir} exists and was not made by build_times.py (no {MARKER} in it), "
                                  "so it is not emptied; pass another --target-dir")
    elif wipe and target_dir.exists():
        def writable(function, path, _info):
            os.chmod(path, stat.S_IWRITE)
            function(path)
        shutil.rmtree(target_dir, onerror=writable)
    target_dir.mkdir(parents=True, exist_ok=True)
    marker.write_text("A cargo target folder owned by tools/build_times.py; it empties this folder for each cold build.\n")
    return target_dir


# Editing a file and putting it back -------------------------------------------------------------------------

def line_ending(data):
    return b"\r\n" if b"\r\n" in data else b"\n"


ITEM_START = re.compile(rb"^(pub(\([a-z]+\))? )?(async |unsafe |const )*(fn|struct|enum|impl|trait|mod|static) ")


def apply_edit(data, mode, token):
    """The file's bytes with one comment line added (a comment cannot change what the program does), or
    unchanged for "touch", which only gives the file a new modification time when it is written back."""
    if mode == "touch":
        return data
    end = line_ending(data)
    comment = b"// build-times probe " + token.encode() + end
    if mode == "append":
        return data + (b"" if data.endswith(b"\n") or not data else end) + comment
    if mode == "top":
        return comment + data
    if mode != "middle":
        raise BuildTimesError(f"unknown edit mode {mode!r} (one of {', '.join(EDIT_MODES)})")
    lines = data.splitlines(keepends=True)
    middle = len(lines) // 2
    # The line starting an item nearest the middle that follows a blank line: a place a developer's edit
    # could be and no string literal or macro body is likely to be.
    spots = [i for i in range(1, len(lines)) if lines[i - 1].strip() == b"" and ITEM_START.match(lines[i])]
    if not spots:
        return comment + data
    at = min(spots, key=lambda i: abs(i - middle))
    return b"".join(lines[:at]) + comment + b"".join(lines[at:])


def ours(current, *versions):
    """Whether a file holds one of the versions this tool wrote, or a write of one that was cut short."""
    return current is not None and any(v.startswith(current) for v in versions)


def changed_line(original, edited):
    """The line (from 1) the comment went to; None when the bytes are the same (touch)."""
    if original == edited:
        return None
    before, after = original.splitlines(), edited.splitlines()
    return next((n + 1 for n, (a, b) in enumerate(zip(before, after)) if a != b), len(before) + 1)


def pending_note(root):
    return root / WORK / "pending-edit.json"


class Probe:
    """A comment line put in a file and taken out again. close() leaves the original bytes and modification time;
    a file that something else wrote meanwhile is left as it is, and the run stops."""

    def __init__(self, path, root):
        self.path, self.note = Path(path), pending_note(root)
        self.original = self.edited = self.stat = self.line = None

    def apply(self, mode, token):
        self.original = self.path.read_bytes()
        self.stat = self.path.stat()
        edited = apply_edit(self.original, mode, token)
        self.line = changed_line(self.original, edited)
        self.note.parent.mkdir(parents=True, exist_ok=True)
        # Written before the file is touched: a process killed from outside leaves the way back.
        self.note.write_text(json.dumps({"path": str(self.path), "mtime_ns": self.stat.st_mtime_ns,
                                         "atime_ns": self.stat.st_atime_ns, "mode": mode, "token": token,
                                         "original": base64.b64encode(self.original).decode()}))
        self.edited = edited
        self.path.write_bytes(edited)

    def restore_content(self):
        """The original bytes back, modification time now, so that cargo sees the file change again."""
        if self.edited is not None:
            self._still_ours(ours(self.path.read_bytes(), self.edited, self.original))
            self.path.write_bytes(self.original)
            self.edited = None

    def restore_times(self):
        """The original modification time: the file is as it was found, and a target folder built from it before
        the run (yours) does not rebuild the crate."""
        if self.stat is not None:
            self._still_ours(self.path.read_bytes() == self.original)
            os.utime(self.path, ns=(self.stat.st_atime_ns, self.stat.st_mtime_ns))

    def close(self):
        if self.stat is not None:
            self.restore_content()
            self.restore_times()
            self.note.unlink(missing_ok=True)  # kept when putting back fails or is cut short: the next run finishes it

    def _still_ours(self, ours_now):
        """Stop when something else wrote the file meanwhile: putting the old bytes or time back would undo it."""
        if not ours_now:
            self.edited = self.stat = None
            self.note.unlink(missing_ok=True)
            raise BuildTimesError(f"{self.path} was changed by something else during the run, so it is left as it is "
                                  "now; remove its `// build-times probe` line if it has one")


def recover_pending(root):
    """Put back a file a killed run left edited, if it still holds what that run wrote. Returns the path, or None."""
    note = pending_note(root)
    if not note.exists():
        return None
    try:
        saved = json.loads(note.read_text())
        path = Path(saved["path"])
        original = base64.b64decode(saved["original"])
        edited = apply_edit(original, saved["mode"], saved["token"])
        current = path.read_bytes() if path.is_file() else None
    except (OSError, ValueError, KeyError, BuildTimesError):
        raise BuildTimesError(f"{note} is a note of an interrupted run that could not be read; check `git status`, "
                              "remove any `// build-times probe` line, and delete the note")
    restored = current != original
    if restored:
        if not ours(current, edited, original):
            raise BuildTimesError(f"{path} was changed since a run was interrupted with an edit in it, so it is left "
                                  f"as it is; remove its `// build-times probe` line if it has one, then delete {note}")
        path.write_bytes(original)
        os.utime(path, ns=(saved["atime_ns"], saved["mtime_ns"]))
        print(f"restored {path} from an interrupted run", file=sys.stderr)
    note.unlink()
    return path if restored else None


# Choosing what to measure -----------------------------------------------------------------------------------

def workspace_packages(root, cargo):
    done = subprocess.run([*cargo, "metadata", "--format-version", "1", "--no-deps", "--locked"], cwd=root,
                          capture_output=True, text=True, encoding="utf-8", errors="replace")
    if done.returncode != 0:
        raise BuildTimesError("cargo metadata failed:\n" + done.stderr[-2000:])
    return {p["name"]: p for p in json.loads(done.stdout)["packages"]}


def edit_targets(root, packages, crates, overrides):
    """[(crate, file)] for the crates asked for; a crate that is not in the workspace is an error."""
    unknown = [c for c in crates if c not in packages]
    if unknown:
        raise BuildTimesError(f"no crate named {', '.join(unknown)} in the workspace; crates: " + ", ".join(sorted(packages)))
    targets = []
    for crate in crates:
        relative = overrides.get(crate) or EDIT_FILES.get(crate)
        path = (root / relative) if relative else None
        if path is None:
            sources = [t["src_path"] for t in packages[crate]["targets"] if "lib" in t["kind"]] or \
                      [t["src_path"] for t in packages[crate]["targets"]]
            path = Path(sources[0])
        if not path.is_file():
            raise BuildTimesError(f"{crate}: the file to edit, {path}, does not exist; name another with --file {crate}=path")
        path = path.resolve()
        if not path.is_relative_to(root):  # the clean-tree check covers only the repository's files
            raise BuildTimesError(f"{crate}: the file to edit, {path}, is outside the repository {root}")
        targets.append((crate, path))
    return targets


def jobs_info(requested):
    if requested:
        return {"jobs": requested, "source": "--jobs"}
    if os.environ.get("CARGO_BUILD_JOBS"):
        return {"jobs": os.environ["CARGO_BUILD_JOBS"], "source": "CARGO_BUILD_JOBS"}
    return {"jobs": os.cpu_count(), "source": "default: cargo uses the logical CPUs"}


def environment_info():
    keys = ["CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "RUSTFLAGS",
            "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS", "CARGO_PROFILE_DEV_DEBUG"]
    return {k: os.environ[k] for k in keys if k in os.environ}


# The measurements -------------------------------------------------------------------------------------------

BUSY_WARNING_PERCENT = 15
QUIET_RETRIES = 8  # seconds to wait for the CPU to calm down before a build


def sample_load(report, label):
    """What else the machine is doing just before a build: other builds, and how busy the CPU is. Either makes
    the number that follows a worse one, so both are warned about and kept with it."""
    busy = cpu_busy_percent(0.5)
    for _ in range(QUIET_RETRIES):  # a build that just ended (and the virus scanner after it) leaves the CPU busy for a moment
        if busy is None or busy <= BUSY_WARNING_PERCENT:
            break
        time.sleep(1)
        busy = cpu_busy_percent(0.5)
    load = {"other_processes": other_build_processes(), "cpu_busy_before_pct": busy}
    if load["other_processes"]:
        message = f"{label}: other build programs were running ({', '.join(load['other_processes'])}); the number is skewed"
        report["warnings"].append(message)
        print("warning: " + message, flush=True)
    if busy is not None and busy > BUSY_WARNING_PERCENT:
        message = f"{label}: the machine was {busy}% busy before the build (something else is running); the number is skewed"
        report["warnings"].append(message)
        print("warning: " + message, flush=True)
    return load


def measurement(result, load):
    entry = {"ok": result["ok"], "seconds": result["seconds"], **load}
    if not result["ok"]:
        entry["error"] = tail(result["log"])
    return entry


def timings_block(result):
    t = result["timings"]
    return None if t is None else {k: t[k] for k in ("units", "total_s", "top", "critical_path")}


def cold_build(builder, profile, report, root):
    print(f"cold {profile}: emptying {builder.target_dir}, then building ...", flush=True)
    prepare_target(root, builder.target_dir, wipe=True)
    load = sample_load(report, f"cold {profile}")
    result = builder.build(profile, f"cold-{profile}")
    entry = measurement(result, load)
    entry["timings"] = timings_block(result)
    if result["ok"] and result["units"] is None:
        report["warnings"].append(f"cold {profile}: cargo wrote no timings page; no per-crate times")
    report["cold"][profile] = entry
    print(f"cold {profile}: {result['seconds']:.1f} s" + ("" if result["ok"] else "  FAILED"), flush=True)
    return result["ok"]


def incremental_builds(builder, profile, targets, args, report, root):
    prepare_target(root, builder.target_dir, wipe=False)
    warm = builder.build(profile, f"warm-{profile}")
    if not warm["ok"]:
        report["incremental"][profile] = {"error": "the first build failed:\n" + tail(warm["log"])}
        print(f"incremental {profile}: the first build failed, see {warm['log']}", flush=True)
        return False
    results = report["incremental"].setdefault(profile, {})
    load = sample_load(report, f"no-op {profile}")
    noop = builder.build(profile, f"noop-{profile}")
    report["noop"][profile] = measurement(noop, load)
    print(f"no-op {profile}: {noop['seconds']:.1f} s", flush=True)
    all_ok = True
    for crate, path in targets:
        runs = []
        for n in range(args.repeats):
            probe = Probe(path, root)
            try:
                probe.apply(args.edit, f"{os.getpid()}-{time.time_ns()}")
                load = sample_load(report, f"incremental {profile} {crate}")
                result = builder.build(profile, f"edit-{profile}-{crate}")
                probe.restore_content()
                # Back to the unedited crate, after a failed build too: the next measurement starts from it.
                settle = builder.build(profile, f"settle-{profile}-{crate}")
                probe.restore_times()
                if not settle["ok"]:
                    raise BuildTimesError(f"{crate}: the build of the unedited file failed, see {settle['log']}")
            finally:
                probe.close()
            run = measurement(result, load)
            units = result["units"]  # None: cargo left no timings page, so what it rebuilt is unknown
            run["rebuilt_crates"] = None if units is None else sorted({u["name"] for u in units})
            run["rebuilt_units"] = None if units is None else len(units)
            run["slowest"] = (result["timings"] or {}).get("top", [])[:5]
            runs.append(run)
            if not result["ok"]:
                break
        good = [r for r in runs if r["ok"]]
        entry = {"file": str(Path(path).relative_to(root)).replace("\\", "/"), "line": probe.line, "edit": args.edit,
                 "runs": runs, "ok": len(good) == len(runs)}
        if args.edit == "middle" and probe.line == 1:
            report["warnings"].append(f"incremental {profile} {crate}: {entry['file']} has no top-level item after a "
                                      "blank line, so the comment went on line 1, as with --edit top")
        if good:
            entry["seconds"] = round(statistics.median(r["seconds"] for r in good), 2)
            rebuilt = good[-1]["rebuilt_crates"]
            entry["rebuilt_units"] = good[-1]["rebuilt_units"]
            entry["rebuilt_crates"] = rebuilt
            entry["slowest"] = good[-1]["slowest"]
            if rebuilt is None:
                report["warnings"].append(f"incremental {profile} {crate}: cargo wrote no timings page; "
                                          "which crates it rebuilt is unknown")
            elif not rebuilt:
                report["warnings"].append(f"incremental {profile} {crate}: cargo recompiled nothing, "
                                          "so the edit did not reach the game build")
            elif crate not in rebuilt:
                report["warnings"].append(f"incremental {profile} {crate}: the edit did not recompile {crate} itself")
        results[crate] = entry
        all_ok = all_ok and entry["ok"]
        print(f"incremental {profile} {crate}: " + (f"{entry['seconds']:.1f} s, {entry.get('rebuilt_units', 0)} units"
                                                    if good else "FAILED"), flush=True)
    return all_ok


def new_report(root, args, cargo, targets):
    return {
        "format": REPORT_FORMAT, "version": REPORT_VERSION,
        "created": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "label": args.label,
        "commit": commit_info(root), "machine": machine_info(), "toolchain": toolchain_info(cargo, root),
        "jobs": jobs_info(args.jobs), "environment": environment_info(),
        "command": "cargo build --locked -p " + PACKAGE + " --profile <profile> --timings",
        "options": {"profiles": args.profiles, "only": args.only, "crates": [c for c, _ in targets],
                    "files": {c: str(p.relative_to(root)).replace("\\", "/") for c, p in targets},
                    "edit": args.edit, "repeats": args.repeats},
        "warnings": [], "cold": {}, "noop": {}, "incremental": {},
    }


def run(args, root=None, cargo=None):
    root = Path(root or ROOT).resolve()
    cargo = cargo or [args.cargo]
    recover_pending(root)
    require_clean(root, args.allow_dirty)
    profiles = [p for p in args.profiles.split(",") if p]
    if not profiles:
        raise BuildTimesError("--profiles is empty")
    if args.repeats < 1:
        raise BuildTimesError("--repeats must be at least 1")
    crates = [c for c in args.crates.split(",") if c]
    overrides = {}
    for item in args.file or []:
        crate, _, relative = item.partition("=")
        if not relative:
            raise BuildTimesError(f"--file {item!r}: write crate=path")
        overrides[crate] = relative
    packages = workspace_packages(root, cargo)
    targets = [] if args.only == "cold" else edit_targets(root, packages, crates, overrides)
    args.profiles = profiles
    report = new_report(root, args, cargo, targets)
    if os.environ.get("CARGO_TARGET_DIR") or os.environ.get("CARGO_BUILD_TARGET_DIR"):
        report["warnings"].append("CARGO_TARGET_DIR is set in the environment; this run builds in its own "
                                  "folder instead and leaves yours alone")
    if os.environ.get("RUSTC_WRAPPER") or os.environ.get("RUSTC_WORKSPACE_WRAPPER"):
        report["warnings"].append("RUSTC_WRAPPER is set; a compiler cache makes a cold build faster than it is")
    if os.environ.get("CARGO_INCREMENTAL"):
        report["warnings"].append("CARGO_INCREMENTAL is set, which overrides the profiles' incremental setting")
    out_dir = Path(args.out_dir) if args.out_dir else root / WORK / "reports"
    base = Path(args.target_dir) if args.target_dir else root / WORK
    log_dir = root / WORK / "logs"
    print(f"{report['commit']['short']} on {report['machine'].get('os')}, {report['jobs']['jobs']} jobs "
          f"({report['jobs']['source']})", flush=True)
    for warning in report["warnings"]:
        print("warning: " + warning, flush=True)
    failed = False
    try:
        for profile in profiles:
            target = base / f"target-{profile}"
            builder = Builder(root, cargo, target, args.jobs, log_dir)
            if args.only != "incremental" or not (target / MARKER).exists():
                builder.fetch()
            ok = True
            if args.only != "incremental":
                ok = cold_build(builder, profile, report, root)
            if ok and args.only != "cold":
                ok = incremental_builds(builder, profile, targets, args, report, root)
            failed = failed or not ok
    finally:
        paths = write_report(report, out_dir, args.output)
        print("wrote " + ", ".join(str(p) for p in paths), flush=True)
    print(render_markdown(report))
    return 1 if failed else 0


def write_report(report, out_dir, output=None):
    if output:
        json_path = Path(output)
    else:
        stem = f"{report['created'][:10]}-{report['commit']['short'] or 'nocommit'}-{report['machine'].get('os', 'os')}"
        if report.get("label"):
            stem += "-" + re.sub(r"[^A-Za-z0-9._-]+", "-", report["label"])
        json_path, n = Path(out_dir) / (stem + ".json"), 1
        while json_path.exists():  # an earlier run of the same day and commit: keep it
            n += 1
            json_path = Path(out_dir) / f"{stem}-{n}.json"
    json_path.parent.mkdir(parents=True, exist_ok=True)
    json_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    md_path = json_path.with_suffix(".md")
    md_path.write_text(render_markdown(report) + "\n", encoding="utf-8")
    return json_path, md_path


# Markdown ---------------------------------------------------------------------------------------------------

def seconds(value):
    return "-" if value is None else f"{value:.1f}"


def table(header, rows):
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    out += ["| " + " | ".join(str(c) for c in row) + " |" for row in rows]
    return out


def machine_line(m):
    ram = f", {round(m['ram_mb'] / 1024)} GB RAM" if m.get("ram_mb") else ""
    return f"{m.get('cpu', 'unknown CPU')} ({m.get('cpu_threads', '?')} threads){ram}"


def render_markdown(report):
    c, m, t, j = report["commit"], report["machine"], report["toolchain"], report["jobs"]
    lines = ["# Build times", ""]
    lines += table(["", ""], [
        ["Commit", f"`{c['short']}` {c['branch']}" + (" (uncommitted changes)" if c.get("dirty") else "") + f": {c['subject']}"],
        ["Date", report["created"]],
        ["OS", f"{m.get('os')} {m.get('os_version', '')} {m.get('arch', '')}".strip()],
        ["Machine", machine_line(m)],
        ["Rust", f"{t['rustc']}; {t['cargo']}; host {t['host']}"],
        ["Jobs", f"{j['jobs']} ({j['source']})"],
        ["Command", f"`{report['command']}`"],
        ["Edit", f"{report['options']['edit']} (a comment line), {report['options']['repeats']} run(s) each, median"],
    ])
    if report["warnings"]:
        lines += ["", "Warnings:", ""] + [f"- {w}" for w in report["warnings"]]
    if report["cold"]:
        lines += ["", "## Cold builds", "", "From an empty target folder.", ""]
        lines += table(["Profile", "Seconds", "Units"], [
            [p, seconds(v["seconds"]) if v["ok"] else "FAILED", (v.get("timings") or {}).get("units", "-")]
            for p, v in report["cold"].items()])
        for profile, v in report["cold"].items():
            timings = v.get("timings")
            if not timings:
                continue
            lines += ["", f"### Slowest crates, {profile}", "", "Seconds of compiler time added up over the crate's units (they overlap).", ""]
            lines += table(["Crate", "Seconds", "Units"], [[x["crate"], seconds(x["seconds"]), x["units"]] for x in timings["top"]])
            lines += ["", f"### Critical path, {profile}", "", "Each unit waited for the one above it; the last ended the build.", ""]
            lines += table(["Unit", "Start (s)", "End (s)", "Seconds"], [
                [x["unit"], seconds(x["start_s"]), seconds(x["end_s"]), seconds(x["seconds"])] for x in timings["critical_path"]])
    if report["noop"]:
        lines += ["", "## Nothing changed", "", "The same build again: the least any build costs.", ""]
        lines += table(["Profile", "Seconds"], [[p, seconds(v["seconds"]) if v["ok"] else "FAILED"] for p, v in report["noop"].items()])
    if report["incremental"]:
        lines += ["", "## Incremental builds", "", "After a one-line change in one file of the crate.", ""]
        rows = []
        for profile, crates in report["incremental"].items():
            if "error" in crates:
                rows.append([profile, "-", "-", "FAILED", "-", "-"])
                continue
            for crate, v in crates.items():
                slow = ", ".join(f"{x['crate']} {x['seconds']:.1f}" for x in v.get("slowest", [])[:3])
                where = f"{v['file']}:{v['line']}" if v.get("line") else v["file"]
                rows.append([profile, crate, f"`{where}`", seconds(v.get("seconds")) if v["ok"] else "FAILED",
                             v.get("rebuilt_units", "-"), slow or "-"])
        lines += table(["Profile", "Crate", "File", "Seconds", "Units rebuilt", "Slowest crates (s)"], rows)
    return "\n".join(lines)


# Compare ----------------------------------------------------------------------------------------------------

def delta(before, after):
    if before is None or after is None:
        return "-", "-"
    diff = after - before
    percent = f"{100 * diff / before:+.0f}%" if before else "-"
    return f"{diff:+.1f}", percent


def row(name, before, after):
    return [*name, seconds(before), seconds(after), *delta(before, after)]


# Recorded, but no difference to the seconds: the jobs are compared on their own, the target folder is the tool's.
UNTIMED_ENVIRONMENT = {"CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"}


def comparable_differences(a, b):
    """What differs between two reports that makes their seconds a poor comparison."""
    notes = []
    for label, get in [("CPU", lambda r: r["machine"].get("cpu")), ("OS", lambda r: r["machine"].get("os")),
                       ("RAM (MB)", lambda r: r["machine"].get("ram_mb")),
                       ("jobs", lambda r: str(r["jobs"]["jobs"])), ("rustc", lambda r: r["toolchain"]["rustc"]),
                       ("edit mode", lambda r: r["options"]["edit"]),
                       ("edited files", lambda r: json.dumps(r["options"].get("files"), sort_keys=True)),
                       ("environment", lambda r: json.dumps({k: v for k, v in r.get("environment", {}).items()
                                                             if k not in UNTIMED_ENVIRONMENT}, sort_keys=True))]:
        if get(a) != get(b):
            notes.append(f"{label} differs: {get(a)} / {get(b)}")
    return notes


def compare(a, b):
    """A Markdown comparison of report a (before) with report b (after)."""
    for r in (a, b):
        if r.get("format") != REPORT_FORMAT:
            raise BuildTimesError("not a build_times report (format field)")
    lines = ["# Build times compared", ""]
    lines += table(["", "Before", "After"], [
        ["Commit", f"`{a['commit']['short']}` {a['commit']['subject']}", f"`{b['commit']['short']}` {b['commit']['subject']}"],
        ["Date", a["created"], b["created"]],
        ["Machine", machine_line(a["machine"]), machine_line(b["machine"])],
        ["Rust", a["toolchain"]["rustc"], b["toolchain"]["rustc"]],
        ["Jobs", a["jobs"]["jobs"], b["jobs"]["jobs"]]])
    notes = comparable_differences(a, b)
    warnings = [f"before: {w}" for w in a["warnings"]] + [f"after: {w}" for w in b["warnings"]]
    if notes or warnings:
        lines += ["", "Read with care:", ""] + [f"- {n}" for n in notes + warnings]
    head = ["", "Before (s)", "After (s)", "Change (s)", "Change"]
    cold = [row([p], a["cold"].get(p, {}).get("seconds") if a["cold"].get(p, {}).get("ok") else None,
                b["cold"].get(p, {}).get("seconds") if b["cold"].get(p, {}).get("ok") else None)
            for p in _union(a["cold"], b["cold"])]
    if cold:
        lines += ["", "## Cold builds", ""] + table(["Profile", *head[1:]], cold)
    noop = [row([p], _sec(a["noop"].get(p)), _sec(b["noop"].get(p))) for p in _union(a["noop"], b["noop"])]
    if noop:
        lines += ["", "## Nothing changed", ""] + table(["Profile", *head[1:]], noop)
    inc = []
    for profile in _union(a["incremental"], b["incremental"]):
        # A profile whose first build failed holds {"error": text} instead of its crates.
        ca, cb = ({k: v for k, v in r["incremental"].get(profile, {}).items() if isinstance(v, dict)} for r in (a, b))
        for crate in _union(ca, cb):
            inc.append(row([profile, crate], _sec(ca.get(crate)), _sec(cb.get(crate))))
    if inc:
        lines += ["", "## Incremental builds", ""] + table(["Profile", "Crate", *head[1:]], inc)
    for profile in _union(a["cold"], b["cold"]):
        ta = {x["crate"]: x["seconds"] for x in ((a["cold"].get(profile) or {}).get("timings") or {}).get("top", [])}
        tb = {x["crate"]: x["seconds"] for x in ((b["cold"].get(profile) or {}).get("timings") or {}).get("top", [])}
        names = sorted(set(ta) | set(tb), key=lambda n: -max(ta.get(n, 0), tb.get(n, 0)))
        if names:
            lines += ["", f"## Slowest crates, cold {profile}", "", "A crate missing on one side is below its top list or gone."]
            lines += [""] + table(["Crate", *head[1:]], [row([n], ta.get(n), tb.get(n)) for n in names])
        pa = ((a["cold"].get(profile) or {}).get("timings") or {}).get("critical_path") or []
        pb = ((b["cold"].get(profile) or {}).get("timings") or {}).get("critical_path") or []
        if pa or pb:
            lines += ["", f"## Critical path, cold {profile}", "",
                      "Before: " + (" > ".join(x["unit"] for x in pa) or "-"), "",
                      "After: " + (" > ".join(x["unit"] for x in pb) or "-")]
    return "\n".join(lines)


def _union(*dicts):
    names = []
    for d in dicts:
        names += [k for k in d if k not in names]
    return names


def _sec(entry):
    return entry.get("seconds") if entry and entry.get("ok", True) else None


# Command line -----------------------------------------------------------------------------------------------

def parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="command", required=True)
    r = sub.add_parser("run", help="measure this checkout")
    r.add_argument("--profiles", default=",".join(PROFILES), help="cargo profiles, comma separated (default dev,fast)")
    r.add_argument("--only", choices=["cold", "incremental"], help="measure only cold builds, or only incremental ones "
                   "(these reuse the tool's target folder from an earlier run)")
    r.add_argument("--crates", default=",".join(EDIT_FILES), help="crates to edit for the incremental builds")
    r.add_argument("--file", action="append", metavar="CRATE=PATH", help="the file to edit in a crate (path from the repository root)")
    r.add_argument("--edit", choices=EDIT_MODES, default=EDIT_MODES[0],
                   help="where the comment line goes: in the middle of the file, at the top, or at the end; or touch: no change but the modification time")
    r.add_argument("--repeats", type=int, default=1, help="incremental runs per crate (the median is reported)")
    r.add_argument("--jobs", type=int, help="cargo --jobs (else CARGO_BUILD_JOBS, else the logical CPUs)")
    r.add_argument("--allow-dirty", action="store_true", help="run on a tree with uncommitted changes")
    r.add_argument("--target-dir", help=f"where the target folders go (default <repo>/{WORK.as_posix()}); emptied only if this tool made it")
    r.add_argument("--out-dir", help=f"where reports go (default <repo>/{WORK.as_posix()}/reports)")
    r.add_argument("--output", help="the JSON report's path (the Markdown goes beside it)")
    r.add_argument("--label", help="a name for this run, added to the file name")
    r.add_argument("--cargo", default="cargo", help="the cargo program")
    c = sub.add_parser("compare", help="compare two reports")
    c.add_argument("before")
    c.add_argument("after")
    c.add_argument("--output", help="write the table here as well")
    return p


def main(argv=None, root=None, cargo=None):
    args = parser().parse_args(argv)
    for name in ("SIGTERM", "SIGBREAK"):
        if hasattr(signal, name):
            with contextlib.suppress(ValueError):
                signal.signal(getattr(signal, name), lambda *_: sys.exit(1))  # runs the finally blocks that put files back
    try:
        if args.command == "compare":
            text = compare(*(json.loads(Path(p).read_text(encoding="utf-8")) for p in (args.before, args.after)))
            print(text)
            if args.output:
                Path(args.output).write_text(text + "\n", encoding="utf-8")
            return 0
        return run(args, root=root, cargo=cargo)
    except BuildTimesError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError) as error:
        if args.command == "compare":
            print(f"error: cannot compare: {error}", file=sys.stderr)
            return 2
        raise


if __name__ == "__main__":
    sys.exit(main())
