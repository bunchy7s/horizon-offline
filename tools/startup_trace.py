#!/usr/bin/env python3
"""The game's start, step by step: launches it the way a player does (no
control socket, so the title shows while the world tables load), with
`HORIZON_STARTUP_TRACE` set, and prints the timeline the game wrote: every
named step (resource tables, the renderer's device and pipelines, the world's
open on the server thread) on the thread that ran it, and the marks (title
drawn, first frame in a world, the view filled).

    python3 tools/startup_trace.py                         # to the title screen
    python3 tools/startup_trace.py --world T --seed 1234   # into a world, until the view is full
    python3 tools/startup_trace.py --server 127.0.0.1:25565  # a server's join
    python3 tools/startup_trace.py --runs 3 --warm         # 3 later starts (caches built by a first one)

Every run gets fresh scratch folders (HORIZON_HOME, launcher and XDG dirs, the
working directory where saves go); `--warm` keeps one scratch for all runs and
starts once first, so the runs measured are later starts. `--keep FILE` writes
the trace wrapped as a JSON array: Perfetto (ui.perfetto.dev) opens it.
"""
import argparse
import json
import pathlib
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time

try:
    from . import lab
except ImportError:
    import lab

ROOT = pathlib.Path(__file__).resolve().parent.parent
# What each `--until` waits for.
UNTIL = {"title": "title.shown", "world": "world.view_full", "frame": "world.first_frame"}


def read_trace(path):
    """The trace's events: one JSON object per line, a torn last line skipped."""
    events = []
    try:
        text = pathlib.Path(path).read_text()
    except FileNotFoundError:
        return events
    for line in text.splitlines():
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return events


def summarize(events):
    """(steps, marks): steps as (start ms, end ms, name, thread) sorted by
    start, marks as {name: first ms}."""
    threads = {e["tid"]: e["args"]["name"] for e in events if e.get("ph") == "M"}
    steps, marks = [], {}
    for e in events:
        if e.get("ph") == "X":
            start = e["ts"] / 1000
            steps.append((start, start + e["dur"] / 1000, e["name"], threads.get(e["tid"], "?")))
        elif e.get("ph") == "i":
            marks.setdefault(e["name"], e["ts"] / 1000)
    steps.sort()
    return steps, marks


def scratch_env(home):
    """lab.game_env with every folder in `home`, so a later start (`--warm`) finds what the first one built."""
    for d in ("home", "launcher", "data", "config", "trash"):
        (home / d).mkdir(parents=True, exist_ok=True)
    return lab.game_env(
        HORIZON_HOME=home / "home",
        HORIZON_LAUNCHER_HOME=home / "launcher",
        XDG_DATA_HOME=home / "data",
        XDG_CONFIG_HOME=home / "config",
        HORIZON_LAUNCHER_TRASH=home / "trash",
        HORIZON_TELEMETRY_URL="off",
    )


def run_once(args, home, label):
    trace = home / f"trace-{label}.jsonl"
    env = scratch_env(home)
    env["HORIZON_STARTUP_TRACE"] = str(trace)
    cmd = [args.bin, "--resources", args.resources]
    if args.world:
        cmd += ["--world", args.world, "--seed", str(args.seed), "--creative",
                "--render-distance", str(args.render_distance)]
    elif args.server:
        cmd += ["--server", args.server, "--render-distance", str(args.render_distance)]
    cmd += args.extra
    want = UNTIL[args.until]
    log = open(home / f"game-{label}.log", "w")
    proc = subprocess.Popen(cmd, cwd=home, env=env, stdout=log, stderr=subprocess.STDOUT)
    deadline = time.monotonic() + args.timeout
    marks = {}
    while time.monotonic() < deadline and proc.poll() is None:
        _, marks = summarize(read_trace(trace))
        if want in marks:
            time.sleep(args.linger)
            break
        time.sleep(0.05)
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
    log.close()
    steps, marks = summarize(read_trace(trace))
    if want not in marks:
        print(f"{label}: no {want} within {args.timeout} s (log: {home / f'game-{label}.log'})",
              file=sys.stderr)
    return steps, marks, trace


def print_timeline(steps, marks, floor_ms):
    rows = [(s, e, f"{name}  [{thread}]") for s, e, name, thread in steps if e - s >= floor_ms]
    rows += [(t, None, f"* {name}") for name, t in marks.items()]
    rows.sort(key=lambda r: r[0])
    for start, end, what in rows:
        if end is None:
            print(f"{start:8.0f}            {what}")
        else:
            print(f"{start:8.0f} {end - start:8.0f} ms  {what}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", default=str(ROOT / "target/release/horizon-client"))
    ap.add_argument("--resources", default=str(ROOT / ".local/horizon-client/26.3"))
    ap.add_argument("--world", help="open (or create) this world from the command line")
    ap.add_argument("--seed", type=int, default=1234)
    ap.add_argument("--server", help="join this server from the command line")
    ap.add_argument("--render-distance", type=int, default=12)
    ap.add_argument("--until", choices=sorted(UNTIL), help="default: world with --world/--server, else title")
    ap.add_argument("--runs", type=int, default=1)
    ap.add_argument("--warm", action="store_true", help="start once first in the same scratch: later starts")
    ap.add_argument("--timeout", type=float, default=180)
    ap.add_argument("--linger", type=float, default=0.3, help="seconds to keep running after the mark")
    ap.add_argument("--floor", type=float, default=5, help="hide steps shorter than this (ms)")
    ap.add_argument("--keep", help="write the last run's trace here as a JSON array (Perfetto)")
    ap.add_argument("--json", action="store_true", help="print the marks of every run as JSON")
    ap.add_argument("extra", nargs="*", help="more game arguments (after --)")
    args = ap.parse_args()
    args.until = args.until or ("world" if args.world or args.server else "title")
    # The game runs in its scratch folder.
    args.bin, args.resources = (str(pathlib.Path(p).resolve()) for p in (args.bin, args.resources))

    shared = pathlib.Path(tempfile.mkdtemp(prefix="horizon-startup-")) if args.warm else None
    if shared:
        run_once(args, shared, "warmup")
    results = []
    for i in range(args.runs):
        home = shared or pathlib.Path(tempfile.mkdtemp(prefix="horizon-startup-"))
        steps, marks, trace = run_once(args, home, f"run{i + 1}")
        results.append(marks)
        print(f"== run {i + 1} ({'later start' if shared else 'first start'}, {home})")
        print_timeline(steps, marks, args.floor)
        if args.keep:
            pathlib.Path(args.keep).write_text(json.dumps(read_trace(trace)))
        if not shared:
            shutil.rmtree(home, ignore_errors=True)
    if shared:
        shutil.rmtree(shared, ignore_errors=True)
    names = sorted({n for m in results for n in m}, key=lambda n: statistics.median(
        [m[n] for m in results if n in m]))
    print("== medians (ms since start)")
    for name in names:
        values = [m[name] for m in results if name in m]
        print(f"{statistics.median(values):8.0f}  {name}  (n={len(values)})")
    if args.json:
        print(json.dumps(results))


if __name__ == "__main__":
    main()
