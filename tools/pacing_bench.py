#!/usr/bin/env python3
"""Frame pacing benchmark: every frame of long stutter-prone sessions, and what the slow ones did.

perf_bench.py measures throughput on short flights; this measures smoothness (1% and 0.1% lows,
p99/p99.9, the worst frame, frames over 33 ms) in the situations where players see hitches:

    fly16      the village flight of perf_bench, 16 blocks/s, 12 s
    ocean      perf_bench's ocean flight (16 blocks/s, 12 s): Horizon's worst lows on Windows at RD 32
    fly64      a 1,920-block straight line at 64 blocks/s (30 s), new terrain all the way
    teleport   8 teleports of 1,500-3,000 blocks, 5 s recorded after each
    elytra     a 990-block straight line at 33 blocks/s (30 s, an elytra's cruise), new terrain all the way
    nether     the overworld -> Nether -> overworld (/execute in ... run tp), 8 s recorded after each
    portal     through a Nether portal twice (an obsidian frame built at two far places), 8 s recorded after each
    join       joining a dedicated server (seed 1234, the same view distance; its spawn chunks, entities and
               tab list arrive at once), recorded from `connect` until 5 s after the view is loaded: a game
               of its own, started on the title screen
    inventory  the inventory and the pause menu opened and closed 20 times each
    mobs       68 mob types summoned one by one in view (NoAI), 0.6 s recorded after each: first-sight costs
    items      50 item entities of different models, the same way
    block_entities  34 block entities set one by one in view, the same way
    screens    the creative inventory and its 12 tabs, then the survival inventory and its recipe book,
               opened for the first time and once more
    reload     F3+T, 15 s recorded
    tnt        50 TNT set off on a stone block, 10 s recorded (world changes: block updates, light and
               remeshing in one frame; each built in the air in front of the camera)
    fill       /fill of a 64x64x16 stone volume (two fills, the default block limit), then cleared
    fluids     water, then lava, poured down a 16-block slope, 15 s each
    fire       fire lit at 5 places on a 25x25 grove's canopy of leaves over logs, 25 s
    pistons    an observer clock driving 20 pistons, 15 s

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
SCENARIOS = ["fly16", "ocean", "fly64", "elytra", "teleport", "nether", "portal", "inventory", "mobs", "items",
             "block_entities", "screens", "reload", "tnt", "fill", "fluids", "fire", "pistons", "join"]
WORLD_CHANGES = ["tnt", "fill", "fluids", "fire", "pistons"]

# First sight: what each kind of thing costs the first frame it is on screen.
MOBS = ["zombie", "skeleton", "creeper", "spider", "enderman", "cow", "pig", "sheep", "chicken", "horse", "donkey",
        "villager", "iron_golem", "wolf", "cat", "fox", "bee", "axolotl", "frog", "allay", "warden", "wither",
        "ghast", "blaze", "piglin", "hoglin", "strider", "shulker", "guardian", "elder_guardian", "squid",
        "glow_squid", "dolphin", "turtle", "panda", "polar_bear", "llama", "camel", "sniffer", "armadillo", "breeze",
        "bogged", "creaking", "slime", "magma_cube", "phantom", "vex", "evoker", "ravager", "witch", "drowned",
        "husk", "stray", "zombie_villager", "parrot", "rabbit", "goat", "mooshroom", "ocelot", "salmon", "cod",
        "tropical_fish", "pufferfish", "bat", "snow_golem", "wandering_trader", "armor_stand", "ender_dragon"]
ITEMS = ["diamond_sword", "bow", "crossbow", "trident", "shield", "compass", "clock", "recovery_compass", "spyglass",
         "elytra", "totem_of_undying", "player_head", "creeper_head", "white_banner", "red_bed", "chest",
         "ender_chest", "shulker_box", "conduit", "decorated_pot", "filled_map", "potion", "splash_potion",
         "tipped_arrow", "firework_rocket", "written_book", "leather_chestplate", "diamond_helmet", "turtle_helmet",
         "carved_pumpkin", "oak_sign", "glass_pane", "iron_bars", "oak_fence", "torch", "lantern", "bell",
         "enchanting_table", "anvil", "brewing_stand", "cauldron", "hopper", "campfire", "lectern", "mace",
         "wolf_armor", "goat_horn", "bundle", "honey_bottle", "dragon_egg"]
BLOCK_ENTITIES = ["chest", "trapped_chest", "ender_chest", "oak_sign", "oak_hanging_sign", "white_banner",
                  "red_bed", "shulker_box", "player_head", "skeleton_skull", "dragon_head", "enchanting_table",
                  "lectern", "bell", "conduit", "decorated_pot", "campfire", "soul_campfire", "beacon",
                  "end_portal", "end_gateway", "brewing_stand", "spawner", "trial_spawner", "vault",
                  "chiseled_bookshelf", "suspicious_sand", "creaking_heart",
                  "sculk_shrieker", "sculk_catalyst", "jukebox", "piston", "copper_golem_statue",
                  "oak_shelf"]

SPIKE_RE = re.compile(r"slow frame ([\d.]+) ms")
GAP_RE = re.compile(r", ([\d.]+) ms before it")


def spike_ms(line):
    """A spike line's frame time plus the time before the frame (between frames)."""
    gap = GAP_RE.search(line)
    return float(SPIKE_RE.search(line).group(1)) + (float(gap.group(1)) if gap else 0.0)


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

    def add(result, step=None):
        frames.extend(result[0])
        cpu.extend(result[1])
        if step:
            print(f"  {name} {step}: worst frame {max(result[0], default=0):.1f} ms", flush=True)

    if name in ("fly16", "ocean"):
        _, start, end, pitch = next(s for s in pb.SCENES if s[0] == {"fly16": "village"}.get(name, name))
        control("tp {} {} {}".format(*start))
        control("fly {0} {1} {2} {0} {1} {2} 0.1 {3}".format(*start, pitch))
        pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
        time.sleep(2)
        reply = control("fly {} {} {} {} {} {} 12 {} raw".format(*start, *end, pitch))
        add((reply.get("frame_ms") or [], reply.get("cpu_ms") or []))
    elif name in ("fly64", "elytra"):
        start, end = {
            "fly64": ((3000.0, 140.0, -3000.0), (3000.0, 140.0, -1080.0)),
            "elytra": ((-3000.0, 160.0, 3000.0), (-2010.0, 160.0, 3000.0)),
        }[name]
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
            # Through chat, as a player types it: `command` waits on the server thread in the frame.
            add(record(control, lambda: control(f"chat /execute in minecraft:{dim} run tp @s {x} {y} {z}"), 8))
    elif name == "portal":
        # A creative player crosses a portal after one tick in it (Player.getPortalWaitTime).
        for x, y, z in [(1200, 120, -1200), (-1800, 120, 600)]:
            control(f"command execute in minecraft:overworld run tp @s {x + 3} {y} {z}")
            control(f"fly {x + 3} {y} {z} {x + 3} {y} {z} 0.1 10")
            pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
            control(f"command fill {x} {y} {z - 1} {x} {y + 4} {z + 2} minecraft:obsidian")
            control(f"command fill {x} {y + 1} {z} {x} {y + 3} {z + 1} minecraft:nether_portal[axis=z]")
            time.sleep(1)
            add(record(control, lambda: control(f"tp {x + 0.5} {y + 1} {z + 1.0}"), 8))
            dimension = str(control("state").get("dimension"))
            if not dimension.endswith("the_nether"):
                raise RuntimeError(f"the portal at {x} {y} {z} led to {dimension}, not the Nether")
    elif name in ("mobs", "items", "block_entities"):
        # Hovering in the air facing south; each thing appears 6-12 blocks ahead, in a new spot.
        x, y, z = 6000, 200, 6000
        control(f"command execute in minecraft:overworld run tp @s {x} {y} {z} 0 10")
        control(f"fly {x} {y} {z} {x} {y} {z} 0.1 10")
        pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
        time.sleep(2)
        things = {"mobs": MOBS, "items": ITEMS, "block_entities": BLOCK_ENTITIES}[name]
        for i, thing in enumerate(things):
            at = (x - 12 + (i % 8) * 3, y - 1 + (i // 8) % 3 * 2, z + 6 + i // 24 * 4)
            if name == "mobs":
                command = f"summon minecraft:{thing} {at[0]} {at[1]} {at[2]} {{NoAI:1b,NoGravity:1b,Silent:1b}}"
            elif name == "items":
                command = (f"summon minecraft:item {at[0]} {at[1]} {at[2]} "
                           f"{{NoGravity:1b,PickupDelay:32767,Age:-32768,Item:{{id:\"minecraft:{thing}\",count:1}}}}")
            else:
                command = f"setblock {at[0]} {at[1]} {at[2]} minecraft:{thing}"
            # Through chat, as a player types it: `command` waits on the server thread in the frame.
            add(record(control, lambda: control(f"chat /{command}"), 0.6))
    elif name == "screens":
        # First opens: the creative inventory and each of its tabs, then (in survival) the inventory
        # and its recipe book; each opened a second time to compare. Tabs and the recipe book button
        # are clicked where vanilla puts them at GUI scale 3 of 1280x720 (gui 427x240).
        left, top = (427 - 195) // 2, (240 - 136) // 2
        tabs = [(left + 27 * i + 13, top - 12) for i in range(6)] + \
               [(left + 27 * i + 13, top + 136 + 12) for i in range(6)]
        for n in (1, 2):
            add(record(control, lambda: control("key e tap"), 1), f"creative inventory, open {n}")
            for i, (tx, ty) in enumerate(tabs):
                add(record(control, lambda: control(f"click-at {tx} {ty}"), 0.6), f"tab {i}, open {n}")
            control("key esc tap")
            time.sleep(0.5)
        control("chat /gamemode survival")
        time.sleep(1)
        for n in (1, 2):
            add(record(control, lambda: control("key e tap"), 1), f"inventory, open {n}")
            add(record(control, lambda: control("click-at 239 107"), 1), f"recipe book shown, open {n}")
            add(record(control, lambda: control("click-at 239 107"), 0.6), f"recipe book hidden, open {n}")
            control("key esc tap")
            time.sleep(0.5)
        control("chat /gamemode creative")
    elif name == "reload":
        # F3+T: every resource reloaded, as a player presses it.
        def reload():
            control("key f3 down")
            control("key t tap")
            control("key f3 up")
        add(record(control, reload, 15), "F3+T")
    elif name in WORLD_CHANGES:
        for step, result in world_change(name, control, distance, lambda action, seconds: record(control, action, seconds)):
            add(result, step)
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


def world_change(name, control, distance, record):
    """Builds `name`'s setup in the air (y 200, away from terrain) in front of the camera, then
    records it running: a list of (step, (frame times, cpu times))."""
    cx, y, cz = 9000 + 300 * WORLD_CHANGES.index(name), 200, 9000
    control(f"command execute in minecraft:overworld run tp @s {cx} {y + 22} {cz - 34} 0 30")
    control(f"fly {cx} {y + 22} {cz - 34} {cx} {y + 22} {cz - 34} 0.1 30")
    pb.wait_loaded(control, lambda p: True, 300, pb.view_chunks(distance))
    # Typed in chat, as a player does: `command` blocks the frame on the server tick.
    def chat(command):
        control(f"chat /{command}")
        time.sleep(0.06)
    steps = []
    if name == "tnt":
        chat(f"fill {cx - 10} {y - 10} {cz - 10} {cx + 10} {y - 1} {cz + 10} minecraft:stone")
        chat(f"fill {cx - 2} {y} {cz - 2} {cx + 2} {y + 1} {cz + 2} minecraft:tnt")
        time.sleep(2)
        steps.append(("50 TNT", record(lambda: chat(f"setblock {cx + 3} {y} {cz} minecraft:redstone_block"), 10)))
    elif name == "fill":
        fills = [(y, y + 7), (y + 8, y + 15)]
        steps.append(("fill 64x64x16 stone", record(lambda: [chat(
            f"fill {cx - 32} {lo} {cz - 32} {cx + 31} {hi} {cz + 31} minecraft:stone") for lo, hi in fills], 5)))
        steps.append(("clear it", record(lambda: [chat(
            f"fill {cx - 32} {lo} {cz - 32} {cx + 31} {hi} {cz + 31} minecraft:air") for lo, hi in fills], 5)))
    elif name == "fluids":
        # Two 9-wide slopes falling 16 blocks over 16, side by side: water on one, lava on the other.
        for i in range(16):
            chat(f"fill {cx - 14} {y - 20} {cz + i} {cx + 14} {y - i} {cz + i} minecraft:stone")
        chat(f"fill {cx - 14} {y - 20} {cz + 16} {cx + 14} {y - 15} {cz + 30} minecraft:stone")
        time.sleep(2)
        steps.append(("water", record(lambda: chat(f"fill {cx - 13} {y + 1} {cz} {cx - 5} {y + 1} {cz} minecraft:water"), 15)))
        steps.append(("lava", record(lambda: chat(f"fill {cx + 5} {y + 1} {cz} {cx + 13} {y + 1} {cz} minecraft:lava"), 15)))
    elif name == "fire":
        # A 25x25 grove: grass, a log every 4 blocks, persistent leaves around them.
        chat(f"fill {cx - 12} {y - 1} {cz - 12} {cx + 12} {y - 1} {cz + 12} minecraft:grass_block")
        chat(f"fill {cx - 12} {y + 3} {cz - 12} {cx + 12} {y + 6} {cz + 12} minecraft:oak_leaves[persistent=true]")
        for x in range(cx - 12, cx + 13, 4):
            for z in range(cz - 12, cz + 13, 4):
                chat(f"fill {x} {y} {z} {x} {y + 5} {z} minecraft:oak_log")
        time.sleep(2)
        spots = [(cx - 8, cz - 8), (cx + 8, cz - 8), (cx, cz), (cx - 8, cz + 8), (cx + 8, cz + 8)]
        steps.append(("fire spreading", record(lambda: [chat(f"setblock {x} {y + 7} {z} minecraft:fire")
                                                         for x, z in spots], 25)))
    elif name == "pistons":
        # Two observers facing each other pulse every other tick into a dust line each way; each
        # line powers the stone it lies on, and 10 pistons beside that stone push a block.
        chat(f"fill {cx - 12} {y - 1} {cz - 1} {cx + 12} {y - 1} {cz + 3} minecraft:stone")
        for xs in (range(cx + 1, cx + 11), range(cx - 11, cx - 1)):
            for x in xs:
                chat(f"setblock {x} {y} {cz} minecraft:stone")
                chat(f"setblock {x} {y + 1} {cz} minecraft:redstone_wire")
                chat(f"setblock {x} {y} {cz + 1} minecraft:piston[facing=south]")
                chat(f"setblock {x} {y} {cz + 2} minecraft:oak_planks")
        chat(f"setblock {cx} {y} {cz} minecraft:stone")
        time.sleep(2)
        def start():
            chat(f"setblock {cx} {y + 1} {cz} minecraft:observer[facing=west]")
            chat(f"setblock {cx - 1} {y + 1} {cz} minecraft:observer[facing=east]")
        steps.append(("20-piston clock", record(start, 15)))
    return steps


def launch(args, distance, tag):
    """A fresh game on its title screen: (process, control, log path, log file, game dir, socket)."""
    import shutil
    game_dir = WORK / f"game-{args.label}-rd{distance}{tag}"
    shutil.rmtree(game_dir, ignore_errors=True)
    pb.write_options(game_dir, distance)
    sock = Path("/tmp") / f"horizon-pacing-{os.getpid()}{tag}.sock"
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
    log_path = WORK / f"{args.label}-rd{distance}{tag}.log"
    log = log_path.open("w")
    proc = subprocess.Popen(command, cwd=game_dir, env=env, stdout=log, stderr=subprocess.STDOUT)
    control = pb.Control(sock)
    pb.poll(control, lambda s: s.get("screen") == "title" and s.get("renderer"), 300)
    return proc, control, log_path, log, game_dir, sock


def dedicated_server(args, distance, directory):
    """A dedicated server of the same build (seed 1234, offline, the client's view distance), once it is up:
    (server, port)."""
    from test_dedicated_server_parity import Server
    directory.mkdir(parents=True, exist_ok=True)
    port = lab.free_port()
    properties = {
        "online-mode": "false", "white-list": "false", "level-seed": str(pb.SEED), "gamemode": "creative",
        "spawn-protection": "0", "view-distance": str(distance), "simulation-distance": "12",
        "pause-when-empty-seconds": "-1", "server-port": str(port),
    }
    (directory / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in properties.items()),
                                                 encoding="iso-8859-1")
    (directory / "eula.txt").write_text("eula=true\n")
    binary = Path(args.build).resolve() / "horizon-client"
    server = Server([binary, "--resources", Path(args.resources).resolve(), "--dedicated", "--nogui"],
                    directory, "server")
    server.wait_for(r"Done \(", 900)
    return server, port


def play(args, distance, names, result):
    """One game through `names`: in a fresh world, or (`join` alone) joining a dedicated server."""
    import shutil
    joining = names == ["join"]
    proc, control, log_path, log, game_dir, sock = launch(args, distance, "-join" if joining else "")
    server = None
    try:
        if joining:
            server, port = dedicated_server(args, distance, WORK / f"server-{args.label}-rd{distance}")
        else:
            control(f"open {pb.WORLD} {pb.SEED} creative")
            pb.wait_loaded(control, lambda p: True, args.timeout, pb.view_chunks(distance))
        for name in names:
            mark = log_path.stat().st_size
            if name == "join":
                control("stats reset")
                control(f"connect 127.0.0.1:{port}")
                pb.wait_loaded(control, lambda p: True, args.timeout, pb.view_chunks(distance))
                time.sleep(5)
                reply = control("stats raw reset")
                frames, cpu = reply.get("frame_ms") or [], reply.get("cpu_ms") or []
            else:
                frames, cpu = scenario(name, control, distance)
            log.flush()
            with log_path.open(errors="replace") as f:
                f.seek(mark)
                spikes = [line.strip() for line in f if "slow frame" in line]
            spikes.sort(key=lambda line: -spike_ms(line))
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
        log.close()
        sock.unlink(missing_ok=True)
        shutil.rmtree(game_dir, ignore_errors=True)
        if server:
            server.stop()
            shutil.rmtree(WORK / f"server-{args.label}-rd{distance}", ignore_errors=True)


def run(args, distance):
    names = args.scenarios.split(",")
    result = {"label": args.label, "distance": distance, "load_before": pb.load_average(), "scenarios": {}}
    in_world = [n for n in names if n != "join"]
    if in_world:
        play(args, distance, in_world, result)
    if "join" in names:
        play(args, distance, ["join"], result)
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
