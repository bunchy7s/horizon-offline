#!/usr/bin/env python3
"""Write an HDR test gallery into a world, as a datapack: every block that gives light,
in its brightest state, alone in dark tunnels, to judge and tune how light sources shine
in HDR output (#1441).

  hdr_gallery.py --world SAVES/MyWorld [--resources .local/horizon-client/26.3]

Then, in the world (creative, on a flat open spot, high up is easiest):

  /reload
  /function hdr:gallery        one or two of each family (a verdict goes for the family)
  /function hdr:gallery_full   every block that gives light

Both first run hdr:setup: always midnight, no weather, no mobs, no random ticks (lit
redstone ore stays lit), effects cleared. Each block has 6 empty blocks on each side,
on grass (stone around fire and lava), against the far wall of a stone tunnel; the
tunnels sit side by side around where the function runs. index.txt in the datapack
lists every cell.

HDR's light sources are tuned with Brightness and Smooth Lighting at Horizon's defaults, HDR10 at
Paper White 200 and Peak 500 nits, and HORIZON_HDR_DEBUG=1 to see which pixels lift: judge
under the same. The block list and light levels come from the prepared resources
(gameplay-facts.json, facts.json), so the gallery follows the pinned version.
"""
import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Preferred properties where several states share the top light level.
PREFER = {
    "waterlogged": "false",
    "hanging": "false",
    "facing": "north",
    "lit": "true",
    "signal_fire": "false",
    "charges": "4",
    "candles": "4",
    "pickles": "4",
    "berries": "true",
    "eye": "true",
    "sculk_sensor_phase": "inactive",
    "power": "0",
    "powered": "false",
    "ominous": "false",
    "has_bottle_0": "false",
    "has_bottle_1": "false",
    "has_bottle_2": "false",
    "bloom": "false",
    "age": "0",
    "level": "0",
    "axis": "x",
}
FORCE = {
    "sea_pickle": {"waterlogged": "true"},
    "amethyst_cluster": {"facing": "up"},
    "large_amethyst_bud": {"facing": "up"},
    "medium_amethyst_bud": {"facing": "up"},
    "small_amethyst_bud": {"facing": "up"},
    "end_rod": {"facing": "up"},
    "trial_spawner": {"trial_spawner_state": "active"},
    "vault": {"vault_state": "active"},
}
FACES = ("north", "south", "east", "west", "up", "down")

# The order of the tunnels: families side by side. Candles, candle cakes and any block
# not named here follow.
GROUPS = [
    ["torch", "wall_torch", "soul_torch", "soul_wall_torch", "copper_torch",
     "copper_wall_torch", "redstone_torch", "redstone_wall_torch", "campfire",
     "soul_campfire", "fire", "soul_fire", "lava", "lava_cauldron", "magma_block"],
    ["lantern", "soul_lantern", "jack_o_lantern", "sea_lantern", "copper_lantern",
     "exposed_copper_lantern", "weathered_copper_lantern", "oxidized_copper_lantern",
     "waxed_copper_lantern", "waxed_exposed_copper_lantern",
     "waxed_weathered_copper_lantern", "waxed_oxidized_copper_lantern"],
    ["glowstone", "shroomlight", "ochre_froglight", "verdant_froglight",
     "pearlescent_froglight", "crying_obsidian", "redstone_lamp", "redstone_ore",
     "deepslate_redstone_ore", "beacon", "conduit", "end_rod", "respawn_anchor"],
    ["copper_bulb", "exposed_copper_bulb", "weathered_copper_bulb", "oxidized_copper_bulb",
     "waxed_copper_bulb", "waxed_exposed_copper_bulb", "waxed_weathered_copper_bulb",
     "waxed_oxidized_copper_bulb"],
    ["furnace", "smoker", "blast_furnace", "brewing_stand", "enchanting_table",
     "ender_chest", "vault", "trial_spawner"],
    ["glow_lichen", "cave_vines", "cave_vines_plant", "sea_pickle", "brown_mushroom",
     "firefly_bush", "dragon_egg", "sculk_sensor", "calibrated_sculk_sensor",
     "sculk_catalyst", "amethyst_cluster", "large_amethyst_bud", "medium_amethyst_bud",
     "small_amethyst_bud"],
    ["end_portal_frame", "end_portal", "end_gateway"],
]
# hdr:gallery: one or two of each family, whose verdict goes for the whole family
# (wall torches with their torch, every oxidation stage, waxed or not, every candle
# colour), and none of those that stay as in SDR (mesher::hdr_tier's 0).
SHORT = {
    "torch", "soul_torch", "copper_torch", "redstone_torch", "campfire", "soul_campfire",
    "fire", "soul_fire", "lava", "lava_cauldron", "magma_block",
    "lantern", "soul_lantern", "jack_o_lantern", "sea_lantern", "copper_lantern",
    "oxidized_copper_lantern",
    "glowstone", "shroomlight", "ochre_froglight", "verdant_froglight",
    "pearlescent_froglight", "crying_obsidian", "redstone_lamp", "redstone_ore", "beacon",
    "conduit", "end_rod", "respawn_anchor",
    "copper_bulb", "oxidized_copper_bulb",
    "furnace", "smoker", "blast_furnace", "brewing_stand", "enchanting_table",
    "ender_chest", "vault", "trial_spawner",
    "cave_vines", "firefly_bush", "sculk_sensor", "calibrated_sculk_sensor",
    "sculk_catalyst", "amethyst_cluster", "small_amethyst_bud",
    "end_portal", "end_gateway",
    "candle", "white_candle", "red_candle", "candle_cake", "red_candle_cake",
}
# Not shown: invisible, or built separately (the nether portal needs its frame).
SKIP = {"light", "nether_portal"}
# Blocks that burn or melt: stone around them instead of grass.
FIERY = {"fire", "soul_fire", "campfire", "soul_campfire", "lava", "lava_cauldron",
         "magma_block"}

PER_ROW = 19
STEP = 7
LENGTH = PER_ROW * STEP + 10
X0 = -LENGTH // 2

SETUP = """\
# HDR gallery: the same light every visit. Game rules are saved with the world.
# Always midnight, never rain or thunder.
time set midnight
weather clear
gamerule advance_time false
gamerule advance_weather false
# No mobs wandering in front of the light sources (or their glowing eyes).
gamerule spawn_mobs false
gamerule spawn_monsters false
gamerule spawn_phantoms false
gamerule spawn_patrols false
gamerule spawn_wandering_traders false
gamerule spawn_wardens false
# No random ticks: lit redstone ore stays lit, nothing grows, decays or spreads.
gamerule random_tick_speed 0
gamerule fire_spread_radius_around_player 0
# Night vision and other effects change how bright the world is drawn.
effect clear @s
tellraw @s {"text":"HDR setup: always midnight, clear weather, no mobs, no random ticks, \
effects cleared. Keep Options > Video Brightness and Smooth Lighting at their defaults: \
the tiers are tuned there."}
"""


def light_states(resources):
    """Each block that gives light: its states as (light level, properties)."""
    gameplay = json.loads((resources / "gameplay-facts.json").read_text())["states"]
    facts = json.loads((resources / "facts.json").read_text())["states"]
    states = {}
    for st in gameplay:
        if st["emission"] > 0:
            entry = facts[st["id"]]
            name = entry[0].removeprefix("minecraft:")
            states.setdefault(name, []).append((st["emission"], entry[4].get("properties", {})))
    return states


def brightest(name, states):
    """The block's brightest state as a command argument, and its light level."""
    top = max(e for e, _ in states)
    cands = [p for e, p in states if e == top]
    if name == "glow_lichen":
        cands = [p for p in cands if p.get("down") == "true"
                 and sum(p.get(f) == "true" for f in FACES) == 1] or cands
    if name == "fire":
        cands = [p for p in cands if not any(
            p.get(f) == "true" for f in FACES if f != "down")] or cands
    want = {**PREFER, **FORCE.get(name, {})}
    best = max(cands, key=lambda p: sum(p.get(k) == v for k, v in want.items() if k in p))
    props = ",".join(f"{k}={v}" for k, v in sorted(best.items()))
    return f"minecraft:{name}" + (f"[{props}]" if props else ""), top


def gallery(names, states):
    """The function's commands and the index of its cells."""
    tunnels = (len(names) - 1) // PER_ROW + 1
    lines = [
        "# HDR gallery (tools/hdr_gallery.py): every block in its brightest state, 6 empty",
        "# blocks between neighbours, on grass, in dark tunnels. index.txt lists the cells.",
        "function hdr:setup",
    ]
    index = []
    for r in range(tunnels):
        z0 = -30 + r * 10  # The tunnel spans z0 .. z0+6: cells at z0+4, walk at z0+1..3.
        lines += [
            f"fill ~{X0} ~-1 ~{z0} ~{X0 + LENGTH} ~5 ~{z0 + 6} minecraft:smooth_stone hollow",
            f"fill ~{X0 + 1} ~-1 ~{z0 + 1} ~{X0 + LENGTH - 1} ~-1 ~{z0 + 5} minecraft:grass_block",
            f"fill ~{X0} ~ ~{z0 + 1} ~{X0} ~3 ~{z0 + 3} minecraft:air",
            f"fill ~{X0 + LENGTH} ~ ~{z0 + 1} ~{X0 + LENGTH} ~3 ~{z0 + 3} minecraft:air",
        ]
    for i, name in enumerate(names):
        r, c = divmod(i, PER_ROW)
        z = -30 + r * 10 + 4
        x = X0 + 5 + c * STEP
        state, level = brightest(name, states[name])
        at = f"~{x} ~ ~{z}"
        if name in FIERY:
            lines.append(f"fill ~{x - 1} ~-1 ~{z - 1} ~{x + 1} ~-1 ~{z + 1} minecraft:smooth_stone")
        # What each block needs to stay.
        if "wall_torch" in name:
            lines.append(f"setblock ~{x} ~ ~{z + 1} minecraft:smooth_stone")
        elif name in ("cave_vines", "cave_vines_plant"):
            lines.append(f"setblock ~{x} ~2 ~{z} minecraft:smooth_stone")
            if name == "cave_vines_plant":
                lines.append(f"setblock ~{x} ~1 ~{z} {state}")
                state = "minecraft:cave_vines[age=0,berries=false]"
            else:
                lines.append(f"setblock ~{x} ~1 ~{z} minecraft:cave_vines_plant[berries=false]")
        elif name == "fire":
            lines.append(f"setblock ~{x} ~-1 ~{z} minecraft:netherrack")
        elif name == "soul_fire":
            lines.append(f"setblock ~{x} ~-1 ~{z} minecraft:soul_soil")
        elif name == "redstone_lamp":
            lines.append(f"setblock ~{x} ~-1 ~{z} minecraft:redstone_block")
        elif name in ("lava", "sea_pickle"):
            # In the floor, so it (or the pickles' water) can't spread.
            lines.append(f"setblock ~{x} ~-2 ~{z} minecraft:smooth_stone")
            at = f"~{x} ~-1 ~{z}"
        lines.append(f"setblock {at} {state}")
        index.append(f"tunnel {r + 1}, #{c + 1:2} (x ~{x}, z ~{z}): {name}, light {level}  {state}")
    # The nether portal, in its frame, after the last block.
    r = (len(names) - 1) // PER_ROW
    z = -30 + r * 10 + 4
    x = X0 + 5 + (len(names) - r * PER_ROW) * STEP + 2
    lines += [
        f"fill ~{x} ~-1 ~{z} ~{x + 3} ~3 ~{z} minecraft:obsidian",
        f"fill ~{x + 1} ~ ~{z} ~{x + 2} ~2 ~{z} minecraft:nether_portal[axis=x]",
        'tellraw @s {"text":"HDR gallery built: ' + str(len(names) + 1)
        + f' light sources in {tunnels} tunnels (index.txt in the datapack)."}}',
    ]
    index.append(f"tunnel {r + 1}, end (x ~{x + 1}, z ~{z}): nether_portal, light 11, in its frame")
    return lines, index


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--world", type=Path, required=True, help="the world's save folder")
    parser.add_argument("--resources", type=Path, default=ROOT / ".local/horizon-client/26.3")
    args = parser.parse_args()
    if not (args.world / "level.dat").exists():
        sys.exit(f"{args.world}: no level.dat, not a world folder")
    states = light_states(args.resources)
    named = [n for g in GROUPS for n in g]
    order = named + sorted(n for n in states if n.endswith("candle")) \
        + sorted(n for n in states if n.endswith("candle_cake"))
    order += sorted(set(states) - set(order) - SKIP)
    full = [n for n in order if n in states and n not in SKIP]
    short = [n for n in full if n in SHORT]
    if (len(full) - 1) // PER_ROW >= 8:
        sys.exit(f"{len(full)} blocks: too many for the tunnels")

    pack = args.world / "datapacks" / "hdr-gallery"
    functions = pack / "data" / "hdr" / "function"
    functions.mkdir(parents=True, exist_ok=True)
    (pack / "pack.mcmeta").write_text(
        json.dumps({"pack": {"description": "Horizon HDR test gallery", "pack_format": 94}}))
    (functions / "setup.mcfunction").write_text(SETUP)
    index = []
    for fn, names in (("gallery", short), ("gallery_full", full)):
        lines, cells = gallery(names, states)
        (functions / f"{fn}.mcfunction").write_text("\n".join(lines) + "\n")
        index += [f"hdr:{fn}"] + cells + [""]
    (pack / "index.txt").write_text("\n".join(index))
    print(f"{pack}: hdr:gallery ({len(short) + 1} blocks), hdr:gallery_full ({len(full) + 1})")


if __name__ == "__main__":
    main()
