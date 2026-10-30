#!/usr/bin/env python3
"""Render regression: bundle component tooltips in anvil and brewing screens.

Requires prepared 26.3 resources, a built horizon-client/horizon-ctl and Pillow.
Runs an isolated offscreen game; never opens existing player data. A name-only
bundle tooltip is narrower than the component's 96 GUI-pixel minimum, so its
purple frame cannot pass the image assertion. Screenshots are kept as evidence.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

from PIL import Image
import lab

ROOT = Path(__file__).resolve().parents[1]


def longest_tooltip_border(image):
    """Longest horizontal purple tooltip-frame run (physical pixels)."""
    rgb = image.convert("RGB")
    longest = 0
    pixels = rgb.load()
    for y in range(rgb.height):
        run = 0
        for x in range(rgb.width):
            r, g, b = pixels[x, y]
            if g < 25 and r < 100 and b >= r + 15:
                run += 1
                longest = max(longest, run)
            else:
                run = 0
    return longest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, default=ROOT / "target/fast/horizon-client")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/fast/horizon-ctl")
    parser.add_argument("--resources", type=Path, default=ROOT / ".local/horizon-client/26.3")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--in-frame", action="store_true", help="DEPRECATED, never use: unsupported, will be removed soon")
    args = parser.parse_args()
    output = args.output or Path(tempfile.mkdtemp(prefix="bundle-tooltip-"))
    output.mkdir(parents=True, exist_ok=True)
    output = output.resolve()
    homes = {}
    for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
        directory = output / key
        directory.mkdir(exist_ok=True)
        homes[key] = directory
    env = lab.game_env(**homes)
    socket = output / "ctl.sock"

    def ctl(*commands, timeout=45):
        result = subprocess.run(
            [str(args.ctl.resolve()), "--socket", str(socket), *map(str, commands)],
            env=env, cwd=output, capture_output=True, text=True, timeout=timeout,
        )
        data = json.loads(result.stdout)
        if result.returncode or data.get("error"):
            raise RuntimeError(f"{commands}: {data}; {result.stderr}")
        return data

    results = []
    try:
        ctl("launch", "--game", args.game.resolve(), "--world", output / "world",
            "--creative", "--offscreen", "--wait", "--resources", args.resources.resolve(),
            *(["--in-frame"] if args.in_frame else []), timeout=120)
        state = ctl("state")
        px, _, pz = state["player"]["block_pos"]
        x, z = (px // 16) * 16 + 8, (pz // 16) * 16 + 8
        ctl("command", f"fill {x-3} 99 {z-3} {x+3} 99 {z+3} minecraft:stone")
        ctl("command", f"fill {x-3} 100 {z-3} {x+3} 105 {z+3} minecraft:air")
        ctl("tp", x + 0.5, 100, z + 0.5)
        ctl("command", 'item replace entity @s hotbar.0 with minecraft:bundle['
            'minecraft:bundle_contents=[{id:"minecraft:stone",count:32}]]')
        for station in ("anvil", "brewing_stand"):
            ctl("command", f"setblock {x} 100 {z+2} minecraft:{station}")
            ctl("wait", 5)
            ctl("look-at-block", x, 100, z+2, "north")
            ctl("interact", x, 100, z+2, "north")
            time.sleep(0.3)
            ui = ctl("ui")
            slot = next(w for w in ui["widgets"] if w["id"] == "slot-0")
            assert slot["stack"]["item"] == "minecraft:bundle", slot
            sx, sy, sw, sh = slot["bounds"]
            ctl("scroll-at", sx + sw / 2, sy + sh / 2, 0)
            screenshot = output / f"{station}.png"
            ctl("screenshot", screenshot)
            with Image.open(screenshot) as image:
                scale = image.width / ui["gui_size"][0]
                longest = longest_tooltip_border(image)
            # The component reserves 96 GUI pixels; allow its border/integer rounding.
            required = int(90 * scale)
            results.append({"station": station, "border_pixels": longest,
                            "required_pixels": required, "pass": longest >= required,
                            "screenshot": str(screenshot)})
            ctl("close")
    finally:
        try:
            ctl("quit")
        except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError):
            pass
    report = {"mode": "in-frame" if args.in_frame else "server-thread", "results": results}
    (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    assert len(results) == 2 and all(r["pass"] for r in results), "Missing bundle component tooltip"


if __name__ == "__main__":
    main()
