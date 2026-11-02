#!/usr/bin/env python3
"""End-to-end harness for Java mods from the launcher's mod browser (docs/mod-browser.md).

    mod_browser_harness.py [--mod SLUG] [--query TEXT] [--keep]

In a scratch game folder, as a player would through the Mods page (its CLI):

1. `horizon-launcher mods search --kind mod TEXT` finds the mod on Modrinth;
2. `horizon-launcher mods check SLUG` downloads its jar (checksum checked) and says Works or Partly;
3. `horizon-launcher mods install SLUG` ports it into horizon-mods/ and turns it on;
4. the game opens a survival world with it (horizon-ctl, offscreen), and
5. a ported block item is given, held, and placed: the world then holds the mod's block;
6. a ported recipe with vanilla ingredients is crafted on a crafting table: the result slot shows
   the mod's item and taking it puts it in the inventory.

It also times the install and the world's load with the mod against the same world with none
(a ported mod must not slow a game that doesn't use it; this measures what one that does costs).
Prints one JSON line per step and a summary; exits 1 when a step fails.

Needs the launcher (launcher/target/debug/horizon-launcher, built with
`--features gpui_platform/runtime_shaders` on macOS), the game and horizon-ctl
(target/fast/horizon-client, target/fast/horizon-ctl, or --bin DIR) and the prepared resources in
.local/horizon-client/26.3. Downloads from Modrinth.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]


class Failed(Exception):
    pass


def run(cmd, env, cwd=None, timeout=600):
    p = subprocess.run(cmd, env=env, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        raise Failed(f"{' '.join(map(str, cmd))}: exit {p.returncode}: {p.stderr.strip()[-800:] or p.stdout.strip()[-800:]}")
    return p.stdout


def report(step, **fields):
    print(json.dumps({"step": step, **fields}), flush=True)


class Game:
    def __init__(self, ctl, client, resources, home, env):
        self.ctl, self.client, self.resources, self.home, self.env = ctl, client, resources, home, env
        self.socket = str(home / "ctl.sock")
        self.pid = None

    def __call__(self, *args, timeout=120):
        out = run([self.ctl, "--socket", self.socket, "--timeout", str(timeout), *map(str, args)], self.env,
                  timeout=timeout + 30)
        return json.loads(out.strip().splitlines()[-1])

    def launch(self, world):
        start = time.monotonic()
        state = self("launch", "--game", self.client, "--world", world, "--seed", "1234", "--wait", "--offscreen",
                     "--resources", self.resources, timeout=300)
        self.pid = state.get("launch", {}).get("pid")
        return time.monotonic() - start, state

    def quit(self):
        try:
            self("quit", timeout=30)
        except Exception:
            pass
        if self.pid:
            for _ in range(50):
                try:
                    os.kill(self.pid, 0)
                except OSError:
                    break
                time.sleep(0.2)
            else:
                os.kill(self.pid, 9)
        self.pid = None


def resources_loaded(log):
    """Seconds the game's log says its resources took."""
    m = re.findall(r"resources loaded in ([0-9.]+)s", Path(log).read_text(errors="replace"))
    return float(m[-1]) if m else None


def pick_block(mod):
    """A block the carry declares with its item."""
    for f in sorted((mod / "data").glob("*/horizon/block/*.json")):
        decl = json.loads(f.read_text())
        if decl.get("item", True):
            return f"{f.parts[-4]}:{f.stem}"
    raise Failed("the carry declares no block with an item")


def pick_recipe(mod, ns):
    """A crafting recipe of the mod's whose result is its own and whose ingredients are plain vanilla
    item ids, shaped first."""
    found = []
    for f in sorted((mod / "data").glob("*/recipe/**/*.json")):
        try:
            r = json.loads(f.read_text())
        except ValueError:
            continue
        result = (r.get("result") or {}).get("id", "")
        if not result.startswith(ns + ":"):
            continue
        if r.get("type") == "minecraft:crafting_shaped":
            keys = r.get("key", {})
            if all(isinstance(v, str) and v.startswith("minecraft:") for v in keys.values()) and \
                    len(r["pattern"]) <= 3 and all(len(row) <= 3 for row in r["pattern"]):
                found.append((0, f.name, r))
        elif r.get("type") == "minecraft:crafting_shapeless":
            ing = r.get("ingredients", [])
            if ing and len(ing) <= 9 and all(isinstance(v, str) and v.startswith("minecraft:") for v in ing):
                found.append((1, f.name, r))
    if not found:
        raise Failed("no crafting recipe of the mod's with vanilla ingredients")
    return sorted(found, key=lambda x: (x[0], x[1]))[0][2]


def grid_cells(recipe):
    """(slot, item) for each grid cell of a 3x3 crafting table (slots 100-108)."""
    if recipe["type"] == "minecraft:crafting_shaped":
        return [(100 + 3 * y + x, recipe["key"][c]) for y, row in enumerate(recipe["pattern"])
                for x, c in enumerate(row) if c != " "]
    return [(100 + i, item) for i, item in enumerate(recipe["ingredients"])]


def inventory_slot(game, item):
    inv = game("inventory")
    for s in inv["hotbar"] + inv["main"]:
        if s.get("item") == item:
            return s["slot"]
    raise Failed(f"{item} is not in the inventory: {inv.get('counts')}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mod", default="macaws-fences-and-walls", help="Modrinth slug of a Works or Partly mod")
    ap.add_argument("--query", default="macaw", help="the search text that finds it")
    ap.add_argument("--launcher", default=str(ROOT / "launcher/target/debug/horizon-launcher"))
    ap.add_argument("--bin", default=str(ROOT / "target/fast"), help="folder with horizon-client and horizon-ctl")
    ap.add_argument("--resources", default=str(ROOT / ".local/horizon-client/26.3"))
    ap.add_argument("--keep", action="store_true", help="keep the scratch game folder")
    args = ap.parse_args()

    home = Path(tempfile.mkdtemp(prefix="horizon-mod-browser-"))
    env = lab.game_env(HORIZON_HOME=home, HORIZON_NO_HEARTBEAT="1", HORIZON_CONTROL=home / "ctl.sock")
    launcher = [args.launcher]
    game = Game(str(Path(args.bin) / "horizon-ctl"), str(Path(args.bin) / "horizon-client"),
                args.resources, home, env)
    summary = {"mod": args.mod}
    ok = False
    try:
        # A baseline world first, with no mod: the same world's load time with the mod is compared.
        base_home = home / "vanilla"
        base_home.mkdir()
        base = Game(game.ctl, game.client, args.resources, base_home,
                    dict(env, HORIZON_HOME=str(base_home), HORIZON_CONTROL=str(base_home / "ctl.sock")))
        base.socket = str(base_home / "ctl.sock")
        os.chdir(base_home)
        took, _ = base.launch("browser")
        summary["world_load_without_mods_s"] = round(took, 2)
        summary["resources_without_mods_s"] = resources_loaded(base_home / "ctl.log")
        base.quit()

        found = json.loads(run(launcher + ["--json", "mods", "search", "--kind", "mod", *args.query.split()], env))
        slugs = [p["slug"] for p in found]
        report("search", query=args.query, results=len(found), found=args.mod in slugs)
        if args.mod not in slugs:
            raise Failed(f"{args.mod} not in the results: {slugs[:10]}")

        start = time.monotonic()
        check = json.loads(run(launcher + ["--json", "mods", "check", args.mod], env).strip().splitlines()[-1])
        summary["check_s"] = round(time.monotonic() - start, 2)
        report("check", verdict=check["label"], summary=check["summary"], seconds=summary["check_s"])
        if check["verdict"] not in ("works", "partly"):
            raise Failed(f"{args.mod} doesn't run: {check['summary']}")

        start = time.monotonic()
        run(launcher + ["mods", "install", args.mod], env)
        summary["install_s"] = round(time.monotonic() - start, 2)
        mod_id = check["mod_id"]
        mod = home / "horizon-mods" / mod_id
        on = (home / "horizon-mods.txt").read_text()
        report("install", folder=str(mod.relative_to(home)), turned_on=f"mod.{mod_id}:true" in on,
               seconds=summary["install_s"])
        if not (mod / "horizon-mod.toml").is_file() or f"mod.{mod_id}:true" not in on:
            raise Failed("not installed or not turned on")

        os.chdir(home)
        took, state = game.launch("browser")
        summary["world_load_with_mod_s"] = round(took, 2)
        summary["resources_with_mod_s"] = resources_loaded(home / "ctl.log")
        log = (home / "ctl.log").read_text(errors="replace")
        loaded = "content from 1 mod(s)" in log
        report("world", seconds=round(took, 2), content_loaded=loaded, screen=state.get("screen"))
        if not loaded:
            raise Failed("the game didn't load the mod's content")

        x, y, z = (int(v // 1) for v in state["player"]["position"])
        block = pick_block(mod)
        game("command", "gamemode survival")
        game("command", f"fill {x + 1} {y - 1} {z - 2} {x + 4} {y - 1} {z + 2} minecraft:stone")
        game("command", f"fill {x + 1} {y} {z - 2} {x + 4} {y + 2} {z + 2} minecraft:air")
        game("give", block, 1)
        game("select", block)
        held = game("state")["held"]
        placed = game("place", x + 2, y, z, "--item", block)
        at = game("block", x + 2, y, z)
        report("place", block=block, held=held, placed=at.get("block"))
        if at.get("block") != block:
            raise Failed(f"placing {block} left {at.get('block')}")

        recipe = pick_recipe(mod, mod_id)
        result = recipe["result"]["id"]
        # A server command: horizon-ctl's own setblock changes the client's world only.
        game("command", f"setblock {x + 2} {y} {z + 2} minecraft:crafting_table")
        cells = grid_cells(recipe)
        game("select", 9)
        game("interact", x + 2, y, z + 2)
        if game("screen").get("craft_size") != 3:
            raise Failed("the crafting table didn't open")
        # One item at a time, picked up and put down with left clicks: quick right clicks can
        # cross the server's answers and be dropped.
        for slot, item in cells:
            game("give", item, 1)
            time.sleep(0.2)
            game("click-slot", inventory_slot(game, item))
            time.sleep(0.2)
            game("click-slot", slot)
            time.sleep(0.2)
        screen = game("screen")
        out = next((s for s in screen.get("slots", []) if s.get("slot") == 200), {})
        report("craft", recipe=result, result_slot=out.get("item"), count=out.get("count"))
        if out.get("item") != result:
            raise Failed(f"the table shows {out.get('item')}, not {result}")
        game("click-slot", 200, "shift")
        time.sleep(0.3)
        counts = game("inventory")["counts"]
        report("crafted", inventory=counts.get(result, 0))
        if counts.get(result, 0) < 1:
            raise Failed(f"{result} didn't reach the inventory")
        game("close")
        ok = True
    except Failed as e:
        report("failed", why=str(e))
    finally:
        game.quit()
        summary["ok"] = ok
        print(json.dumps({"summary": summary}), flush=True)
        if args.keep:
            print(f"kept {home}", file=sys.stderr)
        else:
            shutil.rmtree(home, ignore_errors=True)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
