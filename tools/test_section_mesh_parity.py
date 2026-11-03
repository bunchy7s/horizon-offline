#!/usr/bin/env python3
"""Compare Horizon's section mesher with the client's own ModelBlockRenderer and FluidRenderer.

Both sides mesh the same scenes (block states by registry ID, per-cell sky/block
light, constant biome colours, overworld or Nether cardinal lighting). The Java
oracle runs vanilla smooth lighting (AO on, cutout leaves) block by block the way
SectionCompiler visits a section; the Rust fixture runs ``Mesher::mesh`` on real
client chunks. Quads are compared as multisets keyed by sprite, render layer class
and rounded geometry; matched quads then compare UVs, ARGB colours and light
coordinates per vertex. A second gate compares the per-state facts the Rust mesher
derives (shade brightness, emissive rendering, legacy solidity, offsets, ...)
with the client's own state accessors for every registry state.
"""
import argparse
from collections import Counter, defaultdict
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

BUILD = lab.ROOT / 'target/section-mesh-parity'
RESOURCES = lab.RESOURCES
SECTION_Y = 0
COLORS = {'grass': 0x91BD59, 'foliage': 0x77AB2F, 'dry_foliage': 0xA88B5E, 'water': 0x3F76E4}
POSITION_TOLERANCE = 1e-4
UV_TOLERANCE = 1e-4


class States:
    """Registry state IDs by name and properties, from the prepared registry facts."""
    def __init__(self, resources):
        facts = json.loads((resources / 'facts.json').read_text())
        self.defaults = facts['defaults']
        self.properties = [entry[4].get('properties', {}) for entry in facts['states']]
        self.names = [entry[0] for entry in facts['states']]
        self.by_name = defaultdict(list)
        for index, name in enumerate(self.names):
            self.by_name[name].append(index)
        gameplay = json.loads((resources / 'gameplay-facts.json').read_text())['states']
        self.dampening = [s['dampening'] for s in gameplay]
        self.emission = [s['emission'] for s in gameplay]
        self.opaque = [s['solid_render'] for s in gameplay]
        self.render_shape = [s['render_shape'] for s in gameplay]

    def __call__(self, name, **properties):
        name = name if ':' in name else 'minecraft:' + name
        wanted = dict(self.properties[self.defaults[name]])
        for key, value in properties.items():
            key = key.rstrip('_')
            if key not in wanted:
                raise KeyError(f'{name} has no property {key}')
            wanted[key] = str(value).lower()
        for index in self.by_name[name]:
            if self.properties[index] == wanted:
                return index
        raise KeyError(f'no state {name} {wanted}')


class Scene:
    def __init__(self, states, name, nether=False):
        self.states, self.name, self.nether = states, name, nether
        self.blocks = {}
        self.light_override = {}
        self.noise = None
        self.biomes = None
        self.options = {}

    def put(self, x, y, z, name, **properties):
        self.blocks[(x, y, z)] = name if isinstance(name, int) else self.states(name, **properties)

    def fill(self, x0, y0, z0, x1, y1, z1, name, **properties):
        state = self.states(name, **properties)
        for x in range(x0, x1 + 1):
            for y in range(y0, y1 + 1):
                for z in range(z0, z1 + 1):
                    self.blocks[(x, y, z)] = state

    def floor(self, y=3, name='stone'):
        self.fill(-1, y, -1, 16, y, 16, name)

    def lights(self):
        """Simple deterministic propagation: straight-down skylight, -1 per step
        sideways, dampening per cell; emitters spread block light the same way."""
        lo, hi = -2, 17
        base = SECTION_Y * 16
        cells = [(x, y, z) for x in range(lo, hi + 1) for y in range(base + lo, base + hi + 1) for z in range(lo, hi + 1)]
        damp = {c: self.states.dampening[self.blocks.get(c, 0)] for c in cells}
        opaque = {c: self.states.opaque[self.blocks.get(c, 0)] and damp[c] > 0 for c in cells}
        def spread(seeds, straight_down):
            level = {c: 0 for c in cells}
            queue = []
            for c, v in seeds:
                if v > level[c]:
                    level[c] = v
                    queue.append(c)
            while queue:
                nxt = []
                for c in queue:
                    v = level[c]
                    for d in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]:
                        n = (c[0] + d[0], c[1] + d[1], c[2] + d[2])
                        if n not in level or opaque[n]:
                            continue
                        loss = 0 if straight_down and d == (0, -1, 0) and v == 15 and damp[n] == 0 else 1
                        w = v - loss - damp[n]
                        if w > level[n]:
                            level[n] = w
                            nxt.append(n)
                queue = nxt
            return level
        top = base + hi
        sky = spread([((x, top, z), 15) for x in range(lo, hi + 1) for z in range(lo, hi + 1) if not opaque[(x, top, z)]], True)
        block = spread([(c, self.states.emission[self.blocks.get(c, 0)]) for c in cells], False)
        rng = random.Random(self.noise) if self.noise is not None else None
        result = []
        for c in cells:
            s, b = sky[c], block[c]
            if rng is not None:
                s, b = rng.randrange(16), rng.randrange(16)
            s, b = self.light_override.get(c, (s, b))
            if (s, b) != (15, 0):
                result.append([*c, s, b])
        return result

    def row(self):
        row = {'name': self.name, 'nether': self.nether, 'section_y': SECTION_Y, 'colors': COLORS, 'light': [15, 0],
               'blocks': [[*p, s] for p, s in sorted(self.blocks.items())], 'lights': self.lights()}
        if self.biomes is not None:
            row['biomes'] = self.biomes
        row.update(self.options)
        return row

    def tinted_columns(self):
        """Every tint source at several heights over the section's 16x16 footprint."""
        names = [('grass_block', {}), ('short_grass', {}), ('oak_leaves', {}), ('water', {}), ('leaf_litter', {'segment_amount': 2}),
                 ('sugar_cane', {}), ('fern', {}), ('vine', {'north': True}), ('mangrove_leaves', {}), ('jungle_leaves', {})]
        for x in range(16):
            for z in range(16):
                for layer, y in enumerate((1, 5, 9, 13)):
                    name, properties = names[(x * 5 + z * 3 + layer) % len(names)]
                    self.put(x, y, z, 'stone')
                    self.put(x, y + 1, z, name, **properties)
                    if name == 'water':
                        self.put(x, y + 2, z, 'water')

    def biome_field(self, seed, radius, names, default='plains'):
        """Noise-biome quart cells: a Voronoi patchwork that shifts with height."""
        rng = random.Random(seed)
        sites = [[(rng.uniform(-4, 8), rng.uniform(-4, 8), rng.choice(names)) for _ in range(9)] for _ in range(10)]
        cells = []
        for qy in range(-3, 8):
            layer = sites[(qy + 3) // 2 % len(sites)]
            for qx in range(-4, 8):
                for qz in range(-4, 8):
                    best = min(layer, key=lambda site: (site[0] - qx) ** 2 + (site[1] - qz) ** 2)
                    cells.append([qx, qy, qz, 'minecraft:' + best[2]])
        self.biomes = {'seed': rng.randrange(-2 ** 63, 2 ** 63), 'radius': radius, 'default': 'minecraft:' + default, 'cells': cells}


def corpus(states, full=True):
    scenes = []
    def scene(name, **kwargs):
        s = Scene(states, name, **kwargs)
        scenes.append(s)
        return s
    # Full cubes: open air, corners, cavities, overhangs and steps for AO.
    s = scene('cubes_open')
    s.put(8, 8, 8, 'stone')
    s.put(3, 3, 3, 'grass_block')
    s = scene('cubes_corners')
    s.floor(3)
    s.fill(0, 4, 0, 15, 7, 0, 'stone')
    s.fill(0, 4, 0, 0, 7, 15, 'cobblestone')
    s.fill(4, 4, 4, 6, 4, 6, 'oak_planks')
    s.put(5, 5, 5, 'stone')
    s.fill(9, 4, 9, 12, 6, 12, 'stone')
    s.put(10, 6, 10, 'air')
    s.put(11, 5, 10, 'air')
    s.put(10, 5, 10, 'air')
    s.fill(2, 8, 2, 6, 8, 6, 'stone')
    s.put(13, 4, 2, 'glass')
    s.put(13, 4, 3, 'stone')
    s.put(14, 4, 2, 'oak_leaves')
    s.put(14, 5, 2, 'grass_block')
    s.put(12, 4, 14, 'soul_sand')
    s.put(13, 4, 14, 'mud')
    s.put(14, 4, 14, 'barrier')
    s.put(12, 5, 14, 'grass_block')
    s.put(13, 5, 14, 'grass_block')
    s.put(14, 5, 14, 'snow', layers=8)
    s.put(15, 5, 14, 'snow', layers=4)
    s = scene('cave')
    s.fill(-1, 0, -1, 16, 15, 16, 'stone')
    s.fill(2, 3, 2, 13, 9, 13, 'air')
    s.fill(5, 3, 5, 9, 3, 9, 'water')
    s.put(4, 4, 4, 'torch')
    s.put(12, 3, 12, 'glowstone')
    s.put(3, 3, 12, 'magma_block')
    s.put(12, 3, 3, 'sea_lantern')
    s.put(7, 9, 7, 'shroomlight')
    s.put(10, 4, 4, 'lantern')
    s.put(2, 5, 7, 'wall_torch', facing='east')
    s.put(6, 4, 11, 'redstone_wire', power=15)
    s.put(7, 4, 11, 'redstone_wire', power=7)
    s.put(8, 4, 11, 'redstone_wire', power=0)
    s.put(6, 3, 11, 'stone')
    s.put(9, 4, 12, 'sculk_sensor', sculk_sensor_phase='active')
    s.put(10, 4, 12, 'sculk_sensor')
    s.put(11, 4, 12, 'glow_lichen', down=True)
    # Shapes on a floor, against walls and in the open.
    s = scene('shapes')
    s.floor(3)
    s.fill(-1, 4, 15, 16, 7, 16, 'stone')
    x = 0
    for name, props in [('stone_slab', {'type': 'bottom'}), ('stone_slab', {'type': 'top'}), ('stone_slab', {'type': 'double'}),
                        ('oak_stairs', {}), ('oak_stairs', {'facing': 'east', 'half': 'top'}), ('oak_stairs', {'shape': 'inner_left'}),
                        ('oak_stairs', {'facing': 'south', 'shape': 'outer_right'}), ('cobblestone_wall', {'north': 'low', 'up': True}),
                        ('oak_fence', {'north': True, 'east': True}), ('glass_pane', {'north': True, 'south': True}),
                        ('iron_bars', {'east': True, 'west': True}), ('torch', {}), ('poppy', {}), ('dandelion', {}),
                        ('short_grass', {}), ('fern', {})]:
        s.put(x, 4, 7, name, **props)
        s.put(x, 4, 14, name, **props)
        x += 1
    x = 0
    for name, props in [('tall_grass', {'half': 'lower'}), ('large_fern', {'half': 'lower'}), ('sunflower', {'half': 'lower'}),
                        ('grass_block', {}), ('oak_leaves', {}), ('spruce_leaves', {}), ('birch_leaves', {}), ('snow', {'layers': 1}),
                        ('snow', {'layers': 7}), ('white_carpet', {}), ('farmland', {}), ('dirt_path', {}), ('cauldron', {}),
                        ('water_cauldron', {'level': 3}), ('hopper', {}), ('anvil', {})]:
        s.put(x, 4, 4, name, **props)
        if name in ('tall_grass', 'large_fern', 'sunflower'):
            s.put(x, 5, 4, name, half='upper')
        x += 1
    x = 0
    for name, props in [('iron_chain', {}), ('iron_chain', {'axis': 'x'}), ('lantern', {}), ('lantern', {'hanging': True}),
                        ('soul_lantern', {}), ('lily_pad', {}), ('vine', {'north': True}), ('sugar_cane', {}), ('pink_petals', {'flower_amount': 4}),
                        ('melon_stem', {'age': 5}), ('attached_pumpkin_stem', {}), ('leaf_litter', {'segment_amount': 3}),
                        ('small_dripleaf', {}), ('pointed_dripstone', {}), ('bamboo', {}), ('brewing_stand', {})]:
        s.put(x, 4, 10, name, **props)
        x += 1
    s.put(15, 8, 10, 'stone')
    s.put(3, 7, 10, 'stone')
    s.put(2, 5, 10, 'stone')
    s.put(15, 4, 1, 'scaffolding')
    # Fluids: sources, flowing levels, falling columns, mixed neighbours, waterlogging.
    s = scene('fluids')
    s.floor(2)
    s.fill(0, 3, 0, 4, 3, 4, 'water')
    for i, level in enumerate([1, 2, 3, 4, 5, 6, 7]):
        s.put(5 + i, 3, 2, 'water', level=level)
        s.put(5 + i, 3, 3, 'water', level=8 - level)
        s.put(5 + i, 3, 6, 'lava', level=level)
    s.put(2, 4, 2, 'water', level=8)
    s.put(2, 5, 2, 'water', level=8)
    s.put(13, 3, 3, 'glass')
    s.put(12, 4, 3, 'oak_leaves')
    s.put(4, 4, 4, 'stone_slab', type='bottom', waterlogged=True)
    s.put(4, 4, 3, 'oak_stairs', waterlogged=True)
    s.put(3, 4, 4, 'oak_fence', waterlogged=True)
    s.put(1, 4, 1, 'ice')
    s.fill(0, 3, 8, 3, 3, 11, 'lava')
    s.put(1, 4, 9, 'lava', level=8)
    s.put(2, 4, 10, 'stone')
    s.fill(8, 3, 9, 12, 5, 13, 'water')
    s.put(10, 6, 11, 'water', level=3)
    s.put(9, 6, 11, 'oak_slab', type='top')
    s.put(11, 6, 11, 'glass')
    s.put(10, 6, 12, 'stone')
    s.put(13, 4, 11, 'kelp')
    s.put(14, 3, 11, 'seagrass')
    s.put(15, 3, 13, 'water', level=2)
    s.put(15, 4, 14, 'water', level=9)
    s.put(15, 3, 15, 'stone_slab', type='top')
    # Section edges with neighbours in other chunks and sections.
    s = scene('edges')
    for x in [0, 15]:
        for y in [0, 15]:
            for z in [0, 15]:
                s.put(x, y, z, 'stone')
                s.put(x + (-1 if x == 0 else 1), y, z, 'stone')
                s.put(x, y + (-1 if y == 0 else 1), z, 'dirt')
    s.put(7, 15, 7, 'water')
    s.put(7, 16, 7, 'water')
    s.put(8, 0, 8, 'water', level=3)
    s.put(8, -1, 8, 'stone')
    s.put(-1, 7, 7, 'water')
    s.put(0, 7, 7, 'water', level=2)
    s.put(16, 7, 7, 'oak_leaves')
    s.put(15, 7, 7, 'oak_leaves')
    # Every block with a model, default state, on a sparse grid.
    model_blocks = sorted(name for name, default in states.defaults.items() if states.render_shape[default] == 'MODEL'
                          and name not in ('minecraft:air', 'minecraft:cave_air', 'minecraft:void_air'))
    per_scene = 8 * 8 * 8
    for chunk in range(0, len(model_blocks), per_scene):
        s = scene(f'catalog_{chunk // per_scene}')
        for i, name in enumerate(model_blocks[chunk:chunk + per_scene]):
            s.put(i % 8 * 2, i // 64 * 2, i // 8 % 8 * 2, states.defaults[name])
    # Every model state (rotations, multipart, UV lock, waterlogging) on a sparse grid.
    air = {'minecraft:air', 'minecraft:cave_air', 'minecraft:void_air'}
    model_states = [i for i, name in enumerate(states.names) if states.render_shape[i] == 'MODEL' and name not in air]
    if full:
        for chunk in range(0, len(model_states), per_scene):
            s = scene(f'states_{chunk // per_scene}')
            for i, state in enumerate(model_states[chunk:chunk + per_scene]):
                s.put(i % 8 * 2, i // 64 * 2, i // 8 % 8 * 2, state)
    # Dense random states from the whole registry packed against each other and fluids.
    fluids = [states('water'), states('lava'), states('water', level=3), states('water', level=9), states('lava', level=4)]
    for seed in range(6 if full else 2):
        rng = random.Random(0x44454E53 + seed)
        s = scene(f'dense_{seed}', nether=seed == 5)
        for x in range(-1, 17):
            for y in range(-1, 17):
                for z in range(-1, 17):
                    roll = rng.random()
                    if roll < 0.45:
                        s.put(x, y, z, rng.choice(model_states))
                    elif roll < 0.6:
                        s.put(x, y, z, rng.choice(fluids))
        if seed % 2:
            s.noise = seed
    # Random dense scenes over a palette, with structured and noisy light.
    palette = ['stone', 'glass', 'oak_leaves', 'grass_block', 'water', 'lava', 'oak_slab', 'oak_stairs', 'torch', 'poppy',
               'short_grass', 'glowstone', 'oak_fence', 'snow', 'white_carpet', 'magma_block', 'soul_sand', 'ice', 'farmland', 'dirt_path']
    for seed in range(8):
        rng = random.Random(0x4D455348 + seed)
        s = scene(f'random_{seed}', nether=seed % 4 == 3)
        for _ in range(900):
            name = rng.choice(palette)
            ids = states.by_name['minecraft:' + name]
            s.put(rng.randrange(-1, 17), rng.randrange(-1, 17), rng.randrange(-1, 17), rng.choice(ids))
        if seed % 2:
            s.noise = seed
    # Biome tint blending: BiomeManager's zoomed biome at each block's own height, averaged
    # over the blend radius, for real biome colours (colormaps, overrides, modifiers).
    biome_names = sorted(path.stem for path in (RESOURCES / 'data/minecraft/worldgen/biome').glob('*.json'))
    common = ['plains', 'swamp', 'dark_forest', 'desert', 'jungle', 'badlands', 'mangrove_swamp', 'warm_ocean', 'snowy_plains',
              'birch_forest', 'cherry_grove', 'pale_garden', 'savanna', 'taiga', 'old_growth_pine_taiga', 'lush_caves']
    for i, radius in enumerate((2, 0, 1, 3, 5, 7)):
        s = scene(f'biome_blend_r{radius}')
        s.floor(0)
        s.tinted_columns()
        s.biome_field(0x42494F4D + i, radius, common)
    for i in range(0, len(biome_names), 8):
        s = scene(f'biome_each_{i // 8}')
        s.tinted_columns()
        s.biome_field(0x45414348 + i, 2, biome_names[i:i + 8], biome_names[i])
    # Options: flat lighting (ao off) and opaque leaves (cutoutLeaves off) alone and together.
    leaves = ['oak_leaves', 'spruce_leaves', 'birch_leaves', 'jungle_leaves', 'acacia_leaves', 'dark_oak_leaves',
              'azalea_leaves', 'flowering_azalea_leaves', 'mangrove_leaves', 'cherry_leaves', 'pale_oak_leaves']
    option_palette = ['stone', 'glass', 'grass_block', 'water', 'lava', 'oak_slab', 'oak_stairs', 'torch', 'poppy', 'short_grass',
                      'glowstone', 'oak_fence', 'ice', 'oak_log', 'tinted_glass', 'slime_block', 'honey_block'] + leaves * 2
    for i, options in enumerate(({'smooth': False}, {'cutout_leaves': False}, {'smooth': False, 'cutout_leaves': False})):
        rng = random.Random(0x4F505453 + i)
        s = scene(f'options_{i}', nether=i == 2)
        s.options = options
        s.floor(0)
        for x in range(-1, 17):
            for y in range(1, 17):
                for z in range(-1, 17):
                    if rng.random() < 0.5:
                        name = rng.choice(option_palette)
                        s.put(x, y, z, rng.choice(states.by_name['minecraft:' + name]))
        if i:
            s.noise = i
    s = scene('cubes_nether', nether=True)
    s.floor(3, 'netherrack')
    s.fill(4, 4, 4, 6, 6, 6, 'nether_bricks')
    s.fill(8, 4, 8, 10, 4, 10, 'lava')
    s.put(12, 4, 12, 'glowstone')
    return scenes


def f32(value):
    """Both sides emit binary32 values; Java prints them shortest-form, so parse back to the same float."""
    return struct.unpack('<f', struct.pack('<f', value))[0]


def quad_key(quad):
    vertices = quad['vertices']
    rounded = [tuple(round(f32(v[i]) * 1000) for i in range(3)) for v in vertices]
    rotation = min(range(4), key=lambda r: rounded[r:] + rounded[:r])
    layer = 'translucent' if quad['layer'] == 'translucent' else 'opaque'
    return (quad['sprite'], layer, tuple(rounded[rotation:] + rounded[:rotation])), rotation


def channels(color):
    color &= 0xFFFFFFFF
    return [color >> 16 & 255, color >> 8 & 255, color & 255, color >> 24]


def light(coords):
    return [coords >> 16 & 0xFFFF, coords & 0xFFFF]


def compare(scene, client, rust, states, report, limit):
    blend_zero = scene.get('biomes', {}).get('radius') == 0
    blocks = {tuple(b[:3]): b[3] for b in scene['blocks']}
    def describe(quad, rotation):
        v = quad['vertices']
        centre = [sum(p[i] for p in v) / 4 for i in range(3)]
        block = quad.get('block') or [math.floor(c + 1e-4 if i != 1 else c - 1e-3) for i, c in enumerate(centre)]
        state = blocks.get(tuple(block), 0)
        return {'block': block, 'state': f"{states.names[state]}{states.properties[state] or ''}",
                'sprite': quad['sprite'], 'kind': quad.get('kind', 'rust'),
                'vertices': [list(x) for x in (v[rotation:] + v[:rotation])]}
    groups = defaultdict(lambda: ([], []))
    for side, quads in [(0, client), (1, rust)]:
        for quad in quads:
            key, rotation = quad_key(quad)
            groups[key][side].append((quad, rotation))
    counts = Counter()
    def add(category, *examples):
        counts[category] += 1
        report['categories'][category] = report['categories'].get(category, 0) + 1
        bucket = report['examples'].setdefault(category, [])
        if len(bucket) < limit:
            bucket.append({'scene': scene['name'], 'quads': list(examples)})
    def attributes(entry):
        quad, rotation = entry
        v = quad['vertices']
        return str([(round(x[3], 3), round(x[4], 3), channels(x[5]), light(x[6])) for x in v[rotation:] + v[:rotation]])
    for key, (left, right) in groups.items():
        # Coincident quads (back faces, overlapping elements) pair up by their attributes.
        left.sort(key=attributes)
        right.sort(key=attributes)
        for a, b in zip(left, right):
            report['matched_quads'] += 1
            ca = a[0]['vertices'][a[1]:] + a[0]['vertices'][:a[1]]
            cb = b[0]['vertices'][b[1]:] + b[0]['vertices'][:b[1]]
            problems = set()
            for p, q in zip(ca, cb):
                if any(abs(p[i] - q[i]) > POSITION_TOLERANCE for i in range(3)):
                    problems.add('position')
                if any(abs(p[i] - q[i]) > UV_TOLERANCE for i in (3, 4)):
                    problems.add('uv')
                cp, cq = channels(p[5]), channels(q[5])
                if blend_zero and cp[3] == 0 and cq[3] == 255:
                    # With biomeBlendRadius 0 the client uses the resolver's colour as is,
                    # and biome colours given as plain RGB have alpha 0: tinted vertices come
                    # out fully transparent. Horizon keeps them opaque; RGB is compared.
                    cp[3] = 255
                if cp != cq:
                    problems.add('color' if max(abs(x - y) for x, y in zip(cp, cq)) > 1 else 'color_rounding')
                if light(p[6]) != light(q[6]):
                    lp, lq = light(p[6]), light(q[6])
                    problems.add('light_sky' if lp[0] != lq[0] else 'light_block')
            if not problems:
                report['identical_quads'] += 1
            for problem in sorted(problems):
                add(f"{problem}:{a[0].get('kind', 'model')}", describe(*a), describe(*b))
        for entry in left[len(right):]:
            add(f"missing:{entry[0]['kind']}", describe(*entry))
        for entry in right[len(left):]:
            add('extra', describe(*entry))
    report['client_quads'] += len(client)
    report['rust_quads'] += len(rust)
    return counts


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--strict', action='store_true', help='fail when any quad or fact differs')
    parser.add_argument('--examples', type=int, default=12, help='examples kept per mismatch category')
    parser.add_argument('--only', help='only scenes whose name contains this text')
    parser.add_argument('--quick', action='store_true', help='skip the all-states catalogue and most dense scenes')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    BUILD.mkdir(parents=True, exist_ok=True)
    report_path = BUILD / 'report.json'
    report_path.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSectionMeshOracle.java'
    helpers = [oracle.with_name(n + '.java') for n in ['VanillaCuboidModelOracle', 'VanillaHeightmapOracle']]
    sources = [Path(__file__), lab.ROOT / 'tools/lab.py', lab.ROOT / 'tools/test_rust_parity.py', oracle, *helpers,
               lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/horizon-client/examples/probe/section_mesh_fixture.rs']
    for crate in ['horizon-core', 'horizon-world', 'horizon-assets', 'horizon-client']:
        sources += [lab.ROOT / 'crates' / crate / 'Cargo.toml', *sorted((lab.ROOT / 'crates' / crate / 'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    states = States(resources)
    scenes = [s for s in corpus(states, not args.quick) if not args.only or args.only in s.name]
    rows = [s.row() for s in scenes]
    inputs = BUILD / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    def run(command, name):
        path = BUILD / (name + '.log')
        with path.open('w') as log:
            code = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT).returncode
        if code:
            tail = [line for line in path.read_text(errors='replace').splitlines() if not line.startswith('WARNING')][-15:]
            raise SystemExit(f'{name} failed ({code}); see {path}\n' + '\n'.join(tail))
    classes = BUILD / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', dependencies, '-d', classes, oracle, *helpers], 'compile-java')
    env_jobs = os.environ.get('CARGO_BUILD_JOBS', '3')
    run(['cargo', 'build', '--locked', '--profile', 'fast', '-j', env_jobs, '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'fast/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    java = ['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaSectionMeshOracle']
    run([binary, 'section_mesh_fixture',resources, inputs, BUILD / 'rust.jsonl'], 'rust')
    run([*java, inputs, BUILD / 'client.jsonl', resources], 'client')
    run([*java, 'facts', BUILD / 'client-facts.jsonl', resources], 'client-facts')
    run([binary, 'section_mesh_fixture',resources, 'facts', BUILD / 'rust-facts.jsonl'], 'rust-facts')
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(),
              'scenes': len(rows), 'client_quads': 0, 'rust_quads': 0, 'matched_quads': 0, 'identical_quads': 0,
              'categories': {}, 'examples': {}, 'per_scene': {}, 'facts': {}}
    client = [json.loads(line) for line in (BUILD / 'client.jsonl').read_text().splitlines()]
    rust = [json.loads(line) for line in (BUILD / 'rust.jsonl').read_text().splitlines()]
    for row, a, b in zip(rows, client, rust):
        assert row['name'] == a['name'] == b['name']
        counts = compare(row, a['quads'], b['quads'], states, report, args.examples)
        report['per_scene'][row['name']] = {'client': len(a['quads']), 'rust': len(b['quads']), **counts}
    client_facts = [json.loads(line) for line in (BUILD / 'client-facts.jsonl').read_text().splitlines()]
    rust_facts = [json.loads(line) for line in (BUILD / 'rust-facts.jsonl').read_text().splitlines()]
    fact_mismatches = Counter()
    for a, b in zip(client_facts, rust_facts):
        for key in a:
            if key == 'offsets':
                same = len(a[key]) == len(b.get(key, [])) and all(abs(x - y) < 1e-6 for p, q in zip(a[key], b[key]) for x, y in zip(p, q))
            elif isinstance(a[key], float):
                same = isinstance(b.get(key), (int, float)) and abs(f32(a[key]) - f32(b[key])) == 0
            else:
                same = a[key] == b.get(key)
            if not same:
                fact_mismatches[key] += 1
                examples = report['facts'].setdefault(key, [])
                if len(examples) < args.examples:
                    examples.append({'state': f"{states.names[a['id']]}{states.properties[a['id']]}", 'client': a[key], 'rust': b.get(key)})
    timing = [line for line in (BUILD / 'rust.log').read_text().splitlines() if line.startswith('meshed ')]
    report['rust_mesh_timing'] = timing[-1] if timing else None
    report['fact_mismatches'] = dict(fact_mismatches)
    report['fact_states'] = len(client_facts)
    base.ensure_sources_unchanged(sources, tested)
    report['sources_sha256'] = tested
    lab.write_json(report_path, report)
    total = report['client_quads']
    print(f"Section mesh parity: {len(rows)} scenes, client {total:,} quads, Rust {report['rust_quads']:,} quads, "
          f"{report['matched_quads']:,} geometry matches, {report['identical_quads']:,} identical "
          f"({100.0 * report['identical_quads'] / max(total, 1):.2f}% of client quads)")
    for category, count in sorted(report['categories'].items(), key=lambda kv: -kv[1]):
        print(f'  {category:<28} {count:,}')
    print(f"Mesh facts: {len(client_facts):,} states, mismatches: {dict(fact_mismatches) or 'none'}")
    print(f"Rust fixture (fast profile): {report['rust_mesh_timing']}")
    print(f'Report: {report_path}')
    if args.strict and (report['categories'] or fact_mismatches):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
