"""Many-entity stress scenes for tools/perf_bench.py (`--scenes stress` or by name).

Each scene is built by vanilla commands, the same lines for every game (Horizon through horizon-ctl `command`,
vanilla and vanilla-mods through the bench driver mod), on a stone floor at y 289 high above the world spawn of
seed 1234 (loaded already: no terrain to generate), then held still for the flight's seconds, looking along +Z at
the scene. Its cleanup commands then remove every entity and block it made, before the next scene.

Vanilla renders an entity only within 64 * viewScale * its box size (Entity.shouldRenderAtSqrDistance), with
viewScale = clamp(render distance / 8, 1, 2.5): a dropped item (0.25) only within 32 blocks at RD 16. The scenes
keep everything within ~30 blocks of the camera and inside its view, so run them at RD 16 or more.

Entities are still and never despawn (NoAI, NoGravity, PersistenceRequired; items lie on barriers): the same picture every
frame and every run. Particles come from repeating command blocks under the floor, so they keep coming at the
same rate. Chests stay closed: opening one needs a player, which neither driver has.
"""

FLOOR_Y = 289
# The player's feet (the eyes are 1.62 higher).
PLAYER_Y = 298.0
PITCH = 15.0
# The camera stands at the origin, above the spawn of seed 1234 (-519.5, 58.5); the scene lies in front of it, at +Z.
ORIGIN = (-520, 40)
# What a scene may touch: x and z around the origin, y from the command blocks to above the TNT.
BOX = (-24, 24, 2, 40, FLOOR_Y - 9, FLOOR_Y + 26)

SETTINGS = [
    # The setup's own commands and the command blocks would fill the chat.
    "gamerule send_command_feedback false",
    "gamerule command_block_output false",
    "gamerule advance_time false",
    "gamerule spawn_mobs false",
    "time set 6000",
]
MOBS = ["husk", "creeper", "cow", "pig", "sheep", "chicken", "villager", "spider"]
HOLDERS = ["husk", "wither_skeleton", "vindicator", "piglin"]
HELD = ["iron_sword", "stone_axe", "torch", "apple", "bow", "carrot", "diamond_pickaxe", "stone"]
ITEMS = ["stone", "oak_log", "diamond", "apple", "iron_sword", "bread", "redstone", "torch"]
FRAMED = ["diamond_sword", "compass", "clock", "stone", "oak_planks", "apple", "shield", "golden_apple"]
CONTAINERS = ["chest", "trapped_chest", "ender_chest", "white_shulker_box"]
BANNER = ('{patterns:[{pattern:"minecraft:stripe_top",color:"red"},{pattern:"minecraft:creeper",color:"black"},'
          '{pattern:"minecraft:border",color:"blue"}]}')
SIGN_TEXT = '{front_text:{messages:["Horizon","stress","sign","wall"]}}'


def num(v):
    """A coordinate as commands write it: no trailing zeros, never an exponent."""
    text = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if text == "-0" else text


def floor(ox, oz):
    return [f"fill {ox - 24} {FLOOR_Y} {oz + 2} {ox + 24} {FLOOR_Y} {oz + 40} smooth_stone"]


def wall(ox, oz):
    """A stone wall facing the camera, 17 blocks ahead: the backing of frames and wall signs."""
    return [f"fill {ox - 16} {FLOOR_Y - 1} {oz + 17} {ox + 17} {FLOOR_Y + 16} {oz + 17} stone"]


def empty(ox, oz):
    return []


def mobs(ox, oz):
    """1000 mobs of 8 common kinds: 20 x 10 x 5 layers, 1.5 / 2 / 2.5 blocks apart."""
    out, i = [], 0
    for layer in range(5):
        for row in range(10):
            for col in range(20):
                kind = MOBS[i % len(MOBS)]
                x, y, z = ox + 0.5 - 14.25 + 1.5 * col, FLOOR_Y + 1 + 2.5 * layer, oz + 8 + 2 * row
                out.append(f"summon {kind} {num(x)} {num(y)} {num(z)} "
                           "{NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b}")
                i += 1
    return out


def held(ox, oz):
    """1000 mobs holding an item in each hand: the mobs scene's grid, of kinds that neither burn nor convert."""
    out, i = [], 0
    for layer in range(5):
        for row in range(10):
            for col in range(20):
                kind = HOLDERS[i % len(HOLDERS)]
                main, off = HELD[i % len(HELD)], HELD[(i + 3) % len(HELD)]
                x, y, z = ox + 0.5 - 14.25 + 1.5 * col, FLOOR_Y + 1 + 2.5 * layer, oz + 8 + 2 * row
                out.append(f"summon {kind} {num(x)} {num(y)} {num(z)} "
                           "{NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Silent:1b,IsImmuneToZombification:1b,"
                           f'equipment:{{mainhand:{{id:"minecraft:{main}"}},offhand:{{id:"minecraft:{off}"}}}}}}')
                i += 1
    return out


def items(ox, oz):
    """2000 dropped items of 8 kinds: 25 x 16 on the floor and on four invisible barrier layers above it, 2 blocks
    apart, a block apart in a layer (vanilla merges equal stacks within 0.625). They fall the last 0.2 block onto
    their layer: no NoGravity, which not every game reads for items."""
    out = [f"fill {ox - 12} {FLOOR_Y + 2 * layer} {oz + 8} {ox + 12} {FLOOR_Y + 2 * layer} {oz + 23} barrier"
           for layer in range(1, 5)]
    i = 0
    for layer in range(5):
        for row in range(16):
            for col in range(25):
                item = ITEMS[i % len(ITEMS)]
                x, y, z = ox + 0.5 - 12 + col, FLOOR_Y + 1.2 + 2 * layer, oz + 8.5 + row
                out.append(f"summon item {num(x)} {num(y)} {num(z)} "
                           f'{{Item:{{id:"minecraft:{item}",count:1}},Age:-32768s,PickupDelay:32767s}}')
                i += 1
    return out


def frames(ox, oz):
    """512 item frames holding 8 kinds of item on a wall (32 x 16)."""
    out = wall(ox, oz)
    i = 0
    for y in range(FLOOR_Y, FLOOR_Y + 16):
        for x in range(ox - 15, ox + 17):
            item = FRAMED[i % len(FRAMED)]
            # block_pos too: summoning places the entity, not the block a frame hangs in.
            out.append(f"summon item_frame {x} {y} {oz + 16} {{block_pos:[I;{x},{y},{oz + 16}],"
                       f'Facing:2b,Fixed:1b,Item:{{id:"minecraft:{item}",count:1}}}}')
            i += 1
    return out


def armor_stands(ox, oz):
    """500 armor stands in full armour holding a sword (25 x 20)."""
    gear = ('{ShowArms:1b,NoGravity:1b,equipment:{head:{id:"minecraft:diamond_helmet"},'
            'chest:{id:"minecraft:iron_chestplate"},legs:{id:"minecraft:golden_leggings"},'
            'feet:{id:"minecraft:leather_boots"},mainhand:{id:"minecraft:diamond_sword"}}}')
    return [f"summon armor_stand {num(ox + 0.5 - 12 + col)} {FLOOR_Y + 1} {oz + 8 + row} {gear}"
            for row in range(20) for col in range(25)]


def chests(ox, oz):
    """800 containers with block-entity models: four walls 25 wide and 8 high, one kind each."""
    return [f"fill {ox - 12} {FLOOR_Y + 1} {oz + 10 + 4 * i} {ox + 12} {FLOOR_Y + 8} {oz + 10 + 4 * i} {kind}"
            for i, kind in enumerate(CONTAINERS)]


def signs(ox, oz):
    """A wall of 512 written wall signs (32 x 16) and a field of 175 standing ones in front of it."""
    return wall(ox, oz) + [
        f"fill {ox - 15} {FLOOR_Y} {oz + 16} {ox + 16} {FLOOR_Y + 15} {oz + 16} oak_wall_sign[facing=north]{SIGN_TEXT}",
        f"fill {ox - 12} {FLOOR_Y + 1} {oz + 6} {ox + 12} {FLOOR_Y + 1} {oz + 12} oak_sign[rotation=8]{SIGN_TEXT}",
    ]


def banners(ox, oz):
    """300 waving banners with three patterns: 12 rows of 25."""
    return [f"fill {ox - 12} {FLOOR_Y + 1} {oz + 8 + 2 * row} {ox + 12} {FLOOR_Y + 1} {oz + 8 + 2 * row} "
            f"white_banner[rotation=8]{BANNER}" for row in range(12)]


def particles(ox, oz):
    """Steady heavy particles: repeating command blocks under the floor, powered by a redstone block under each,
    every tick: flames, smoke, end rods, firework rockets that burst over the floor, and TNT that explodes in the
    air (nothing within its reach)."""
    def block(dx, dz, command):
        command = command.replace('"', '\\"')
        at = f"{ox + dx} {FLOOR_Y - 8} {oz + dz}"
        return [f'setblock {at} repeating_command_block{{Command:"{command}"}}',
                f"setblock {ox + dx} {FLOOR_Y - 9} {oz + dz} redstone_block"]

    x, z = ox + 0.5, oz + 20.5
    emitters = [
        ("flame", 80, 0.01), ("flame", 80, 0.01), ("flame", 80, 0.01), ("flame", 80, 0.01),
        ("campfire_cosy_smoke", 4, 0.0), ("campfire_cosy_smoke", 4, 0.0),
        ("end_rod", 40, 0.05), ("end_rod", 40, 0.05),
    ]
    out = [line for i, (kind, count, speed) in enumerate(emitters) for line in
           block(-8 + 2 * i, 4, f"particle minecraft:{kind} {num(x)} {FLOOR_Y + 5} {num(z)} 8 4 8 {speed} {count} force")]
    rocket = ('{LifeTime:18,FireworksItem:{id:"minecraft:firework_rocket",count:1,components:{"minecraft:fireworks":'
              '{explosions:[{shape:"large_ball",colors:[I;16711680,65280,255],has_trail:1b}]}}}}')
    out += [line for i in range(3) for line in
            block(-8 + 2 * i, 6, f"summon firework_rocket {num(x - 6 + 6 * i)} {FLOOR_Y + 2} {num(z)} {rocket}")]
    out += block(0, 8, f"summon tnt {num(x)} {FLOOR_Y + 24} {num(z + 6)} {{fuse:0}}")
    return out


# name, builder: in the order --scenes stress runs them.
STRESS = [
    ("stress_empty", empty),
    ("stress_mobs", mobs),
    ("stress_items", items),
    ("stress_frames", frames),
    ("stress_armor_stands", armor_stands),
    ("stress_chests", chests),
    ("stress_signs", signs),
    ("stress_banners", banners),
    ("stress_particles", particles),
    ("stress_held", held),
]
NAMES = [name for name, _ in STRESS]


def origin(name):
    return ORIGIN


def scene(name):
    """(name, from, to, pitch) as perf_bench's scenes: a hold at the camera, facing +Z."""
    ox, oz = origin(name)
    at = (ox + 0.5, PLAYER_Y, oz + 0.5)
    return name, at, (at[0], at[1], at[2] + 0.01), PITCH


def setup(name, first=True):
    """The commands that build the scene, after its chunks are loaded; the game rules only for the first scene of
    a run (setting a rule again says so in red in the chat, which would stay on screen)."""
    ox, oz = origin(name)
    return (SETTINGS if first else []) + floor(ox, oz) + dict(STRESS)[name](ox, oz)


def cleanup(name):
    """Removes the scene: its entities first (a frame would drop its item when its wall goes), then every block
    in its box, in slabs under vanilla's fill limit, then anything that still dropped. Nothing it runs fails, so
    no red line stays in the chat during the next scene: `execute as` kills nothing quietly, and a marker block in
    each slab gives every fill something to clear."""
    ox, oz = origin(name)
    x0, x1, z0, z1, y0, y1 = BOX
    kill = "execute as @e[type=!minecraft:player] run kill @s"
    out = [kill]
    for y in range(y0, y1 + 1, 12):
        out.append(f"setblock {ox + x0} {y} {oz + z0} barrier")
        out.append(f"fill {ox + x0} {y} {oz + z0} {ox + x1} {min(y + 11, y1)} {oz + z1} air")
    return out + [kill]
