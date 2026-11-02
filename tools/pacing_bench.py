#!/usr/bin/env python3
"""Frame pacing benchmark: every frame of long stutter-prone sessions, and what the slow ones did.

perf_bench.py measures throughput on short flights; this measures smoothness (1% and 0.1% lows,
p99/p99.9, the worst frame, frames over 33 ms) in the situations where players see hitches:

    fly16      the village flight of perf_bench, 16 blocks/s, 12 s
    ocean      perf_bench's ocean flight (16 blocks/s, 12 s): Horizon's worst lows on Windows at RD 32
    fly64      a 1,920-block straight line at 64 blocks/s (30 s), new terrain all the way
    teleport   8 teleports of 1,500-3,000 blocks, 5 s recorded after each
    nether     the overworld -> Nether -> overworld (/execute in ... run tp), 8 s recorded after each
    inventory  the inventory and the pause menu opened and closed 20 times each

One game per render distance, from a fresh world (seed 1234, creative), driven over horizon-ctl.
The game runs with HORIZON_SPIKES so each slow frame logs its timed spans; the report keeps the
slowest frames' span lines next to each scenario's figures.

    python3 tools/pacing_bench.py --build target/fast --label main --distances 32
    python3 tools/pacing_bench.py --compare main mybranch

Results: .local/pacing-bench/<label>-rd<N>.json. Use alternated A/B runs on a loaded machine.
"""
import argparse
import json
import math
import os
import re
import subprocess
import time
from pathlib import Path

try:
    from . import lab
    from . import perf_bench as pb
except ImportError:
    import lab
    import perf_bench as pb

WORK = lab.ROOT / ".local/pacing-bench"
SCENARIOS = ["fly16", "ocean", "fly64", "teleport", "nether", "inventory"]
SPIKE_RE = re.compile(r"slow frame ([\d.]+) ms")


def stats(frame_ms):
    """p50/p99/p99.9/max, 1% and 0.1% lows (average of the slowest 1%/0.1%, as FPS), frames over 33 ms
    and over twice the median."""
    values = sorted(frame_ms)
    n = len(values)
    if not n:
        return {"frames": 0}
    pct = lambda p: values[min(n, max(1, math.ceil(n * p))) - 1]
    low = lambda per: 1000.0 / (sum(values[n - math.ceil(n / per):]) / math.ceil(n / per))
    median = pct(0.5)
    return {
        "frames": n,
        "seconds": round(sum(values) / 1000.0, 2),
        "avg_fps": round(n * 1000.0 / sum(values), 1),
        "p50_ms": round(median, 2),
        "p99_ms": round(pct(0.99), 2),
        "p999_ms": round(pct(0.999), 2),
        "max_ms": round(values[-1], 2),
        "low_1pct_fps": round(low(100), 1),
        "low_01pct_fps": round(low(1000), 1),
        "over_33ms": sum(v > 33.34 for v in values),
        "over_2x_median": sum(v > 2 * median for v in values),
    }


def record(control, action, seconds):
    """Resets the frame stats, runs action(), waits `seconds` and returns every frame time since."""
    control("stats reset")
    action()
    time.sleep(seconds)
    reply = control("stats raw reset")
    return reply.get("frame_ms") or [], reply.get("cpu_ms") or []


def scenario(name, control, distance):
    frames, cpu = [], []

    def add(result):
        frames.extend(result[0])
        cpu.extend(result[1])

    if name in ("fly16", "ocean"):
        _, start, end, pitch = next(s for s in pb.SCENES if s[0] == {"fly16": "village"}.get(name, name))
        control("tp {} {} {}".format(*start))
        control("fly {0} {1} {2} {0} {1} {2} 0.1 {3}".format(*start, pitch))
        pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
        time.sleep(2)
        reply = control("fly {} {} {} {} {} {} 12 {} raw".format(*start, *end, pitch))
        add((reply.get("frame_ms") or [], reply.get("cpu_ms") or []))
    elif name == "fly64":
        start, end = (3000.0, 140.0, -3000.0), (3000.0, 140.0, -1080.0)
        control("tp {} {} {}".format(*start))
        control("fly {0} {1} {2} {0} {1} {2} 0.1 20".format(*start))
        pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
        time.sleep(2)
        reply = control("fly {} {} {} {} {} {} 30 20 raw".format(*start, *end))
        add((reply.get("frame_ms") or [], reply.get("cpu_ms") or []))
    elif name == "teleport":
        targets = [(-3000, 150, 2000), (1500, 150, 2500), (4000, 150, -500), (0, 150, -3500),
                   (-2500, 150, -1500), (2500, 150, 1000), (-500, 150, 4000), (3500, 150, 3500)]
        for x, y, z in targets:
            add(record(control, lambda: (control(f"tp {x} {y} {z}"),
                                         control(f"fly {x} {y} {z} {x} {y} {z} 0.1 10")), 5))
    elif name == "nether":
        for dim, (x, y, z) in [("the_nether", (100, 100, 100)), ("overworld", (800, 150, 800)),
                               ("the_nether", (-300, 100, 200)), ("overworld", (-2400, 150, 1600))]:
            add(record(control, lambda: control(f"command execute in minecraft:{dim} run tp @s {x} {y} {z}"), 8))
    elif name == "inventory":
        def spam():
            for _ in range(20):
                control("open-inventory")
                time.sleep(0.15)
                control("key esc tap")
                time.sleep(0.15)
            for _ in range(20):
                control("pause")
                time.sleep(0.15)
                control("resume")
                time.sleep(0.15)
        add(record(control, spam, 0.5))
    return frames, cpu


def run(args, distance):
    game_dir = WORK / f"game-{args.label}-rd{distance}"
    import shutil
    shutil.rmtree(game_dir, ignore_errors=True)
    pb.write_options(game_dir, distance)
    sock = Path("/tmp") / f"horizon-pacing-{os.getpid()}.sock"
    sock.unlink(missing_ok=True)
    client = Path(args.build).resolve() / "horizon-client"
    command = [str(client), "--resources", str(Path(args.resources).resolve()), "--control-socket", str(sock),
               "--no-pause", "--weather", "clear", "--time", "6000"]
    if args.size:
        width, _, height = args.size.partition("x")
        command += ["--width", width, "--height", height]
    if args.offscreen:
        command.append("--offscreen")
    env = lab.game_env(HORIZON_TELEMETRY_URL="off", HORIZON_SPIKES=str(args.spike_ms))
    for kv in args.env:
        k, _, v = kv.partition("=")
        env[k] = v
    log_path = WORK / f"{args.label}-rd{distance}.log"
    result = {"label": args.label, "distance": distance, "load_before": pb.load_average(), "scenarios": {}}
    with log_path.open("w") as log:
        proc = subprocess.Popen(command, cwd=game_dir, env=env, stdout=log, stderr=subprocess.STDOUT)
        control = pb.Control(sock)
        try:
            pb.poll(control, lambda s: s.get("screen") == "title" and s.get("renderer"), 300)
            control(f"open {pb.WORLD} {pb.SEED} creative")
            pb.wait_loaded(control, lambda p: True, args.timeout, pb.view_chunks(distance))
            for name in args.scenarios.split(","):
                mark = log_path.stat().st_size
                frames, cpu = scenario(name, control, distance)
                log.flush()
                with log_path.open(errors="replace") as f:
                    f.seek(mark)
                    spikes = [line.strip() for line in f if "slow frame" in line]
                spikes.sort(key=lambda line: -float(SPIKE_RE.search(line).group(1)))
                entry = stats(frames)
                entry["cpu"] = stats(cpu)
                entry["slowest_spans"] = spikes[:12]
                entry["spike_lines"] = len(spikes)
                entry["frame_ms"] = [round(v, 2) for v in frames]
                result["scenarios"][name] = entry
                print(f"rd{distance} {name}: " + ", ".join(f"{k} {v}" for k, v in entry.items()
                                                         if k not in ("cpu", "slowest_spans", "frame_ms")), flush=True)
        finally:
            try:
                control("quit", timeout=60)
            except (OSError, ValueError, RuntimeError):
                pass
            pb.stop(proc)
            sock.unlink(missing_ok=True)
            shutil.rmtree(game_dir, ignore_errors=True)
    result["load_after"] = pb.load_average()
    return result


COLUMNS = [("p50_ms", "p50"), ("p99_ms", "p99"), ("p999_ms", "p99.9"), ("max_ms", "max"),
           ("low_1pct_fps", "1% low"), ("low_01pct_fps", "0.1% low"), ("over_33ms", ">33 ms")]


def compare(labels):
    """Markdown table per scenario: the median over a label's runs (files <label>-rd<N>*.json)."""
    runs = {}
    for label in labels:
        for path in sorted(WORK.glob(f"{label}-rd*.json")):
            data = json.loads(path.read_text())
            if data["label"] != label:
                continue
            runs.setdefault((data["distance"], label), []).append(data)
    for distance in sorted({d for d, _ in runs}):
        names = [n for n in SCENARIOS if any(n in r["scenarios"] for (d, _), rs in runs.items() if d == distance
                                                 for r in rs)]
        for name in names:
            print(f"\n**RD {distance}, {name}** (median of n runs)\n")
            print("| build | n | " + " | ".join(c for _, c in COLUMNS) + " |")
            print("|---|---|" + "---|" * len(COLUMNS))
            for label in labels:
                entries = [r["scenarios"][name] for r in runs.get((distance, label), []) if name in r["scenarios"]]
                if not entries:
                    continue
                cells = []
                for key, _ in COLUMNS:
                    values = sorted(e.get(key, 0) for e in entries)
                    cells.append(f"{values[len(values) // 2]:g}")
                print(f"| {label} | {len(entries)} | " + " | ".join(cells) + " |")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--label", default="horizon")
    parser.add_argument("--build", default=str(lab.ROOT / "target/fast"))
    parser.add_argument("--resources", default=str(lab.ROOT / ".local/horizon-client/26.3"))
    parser.add_argument("--distances", default="32")
    parser.add_argument("--scenarios", default=",".join(SCENARIOS))
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--spike-ms", type=float, default=20.0, help="HORIZON_SPIKES threshold (CPU ms)")
    parser.add_argument("--offscreen", action="store_true")
    parser.add_argument("--size", help="window size WxH (default the game's 1280x720); a small one keeps a software "
                        "renderer (lavapipe) from hiding the CPU side")
    parser.add_argument("--env", action="append", default=[], help="KEY=VALUE for the game")
    parser.add_argument("--run", type=int, default=0, help="run number, kept in the file name")
    parser.add_argument("--compare", nargs="+", metavar="LABEL")
    args = parser.parse_args()
    if args.compare:
        compare(args.compare)
        return
    WORK.mkdir(parents=True, exist_ok=True)
    for distance in map(int, args.distances.split(",")):
        result = run(args, distance)
        out = WORK / f"{args.label}-rd{distance}-{args.run}.json"
        out.write_text(json.dumps(result))
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
