#!/usr/bin/env python3
"""Render regression for monster spawner mob previews.

Requires a built client/ctl, prepared resources and Pillow. Uses a fresh world
on the normal integrated-server path. A stone backdrop and a zero activation
range keep the cage static; changing only its spawn type must change the pixels
inside it. This fails when the display cache exists but is never rendered.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from PIL import Image, ImageChops

import lab

ROOT = Path(__file__).resolve().parents[1]


def changed_pixels(first, second):
    with Image.open(first) as left, Image.open(second) as right:
        assert left.size == right.size, "The test window changed size"
        width, height = left.size
        # Camera points at the cage's centre; ignore chat, hands and tutorial UI.
        box = (int(width * .3), int(height * .35), int(width * .7), int(height * .65))
        difference = ImageChops.difference(left.convert("RGB").crop(box),
                                          right.convert("RGB").crop(box))
        return sum(max(difference.getpixel((x, y))) > 20
                   for y in range(difference.height) for x in range(difference.width))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, default=ROOT / "target/fast/horizon-client")
    parser.add_argument("--ctl", type=Path, default=ROOT / "target/fast/horizon-ctl")
    parser.add_argument("--resources", type=Path, default=ROOT / ".local/horizon-client/26.3")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    output = (args.output or Path(tempfile.mkdtemp(prefix="spawner-display-"))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="game-", dir=output))
    homes = {}
    for key in ("HORIZON_HOME", "HORIZON_LAUNCHER_HOME", "XDG_DATA_HOME", "XDG_CONFIG_HOME"):
        homes[key] = work / key
        homes[key].mkdir()
    env = lab.game_env(**homes, HORIZON_TELEMETRY_URL="off")
    socket = work / "ctl.sock"

    def ctl(*words, timeout=60):
        result = subprocess.run([str(args.ctl.resolve()), "--socket", str(socket), *map(str, words)],
                                env=env, cwd=work, capture_output=True, text=True, timeout=timeout)
        reply = json.loads(result.stdout)
        if result.returncode or "error" in reply:
            raise RuntimeError(f"{words}: {reply}; {result.stderr}")
        return reply

    def command(text):
        return ctl("command", text)

    results = []
    try:
        ctl("launch", "--game", args.game.resolve(), "--world", "Preview", "--creative", "--wait",
            "--offscreen", "--no-pause", "--render-distance", 2,
            "--resources", args.resources.resolve(), timeout=120)
        state = ctl("state")
        x, _, z = state["player"]["block_pos"]
        x, z = x // 16 * 16 + 8, z // 16 * 16 + 8
        y = 200
        command(f"fill {x-4} {y} {z-5} {x+4} {y+8} {z+4} minecraft:air")
        command(f"fill {x-4} {y-1} {z-5} {x+4} {y-1} {z+4} minecraft:stone")
        command(f"fill {x-4} {y} {z+2} {x+4} {y+6} {z+2} minecraft:stone")
        ctl("tp", x + .5, y, z - 3.5)
        ctl("look", 0, 15)
        ctl("options-set", "guiScale", 2)
        ctl("key", "f1", "tap")  # Hide command chat, the hand and all other HUD changes.
        for block in ("spawner",):
            command(f"setblock {x} {y} {z} minecraft:{block}")
            shots = []
            for kind in ("zombie", "pig", "zombie"):
                tag = '{SpawnData:{entity:{id:"minecraft:' + kind + '"}},RequiredPlayerRange:0s}'
                command(f"data merge block {x} {y} {z} {tag}")
                command("time set noon")
                ctl("wait", 5)
                shot = output / f"{block}-{len(shots)}-{kind}.png"
                ctl("screenshot", shot)
                shots.append(shot)
            forward = changed_pixels(shots[0], shots[1])
            backward = changed_pixels(shots[1], shots[2])
            result = {"block": block, "zombie_to_pig_pixels": forward,
                      "pig_to_zombie_pixels": backward, "pass": min(forward, backward) >= 50}
            results.append(result)
            print(json.dumps(result), flush=True)
    finally:
        try:
            ctl("quit")
        except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError):
            pass
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    assert len(results) == 1 and all(row["pass"] for row in results), "Spawner mob preview is not rendered"


if __name__ == "__main__":
    main()
