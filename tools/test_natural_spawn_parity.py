#!/usr/bin/env python3
"""Compare natural spawning (NaturalSpawner) with vanilla.

Java (VanillaNaturalSpawnOracle) runs vanilla's NaturalSpawner on a stand-in
ServerLevel built from each case: boxes of blocks, light and biomes, structure
starts with their pieces, existing mobs, players, the world spawn point and
unloaded chunks. Rust (natural_spawn_fixture) runs
horizon_client::mobs::spawn_selection on the same world. Three modes:

- chunk: createState over the case's mobs (category counts, spawn potential
  charges, local caps), getFilteredSpawningCategories under the global caps,
  then spawnForChunk (local caps, getRandomPosWithin, spawnCategoryForPosition);
- pos: spawnCategoryForPosition from a chosen start (surfaces, caves, water,
  lava, structures);
- gen: spawnMobsForChunkGeneration.

Each case's log lists every attempt (the nearest player lookup), each type that
passed isValidSpawnPostitionForType with the spawn potential's answer, each
spawn with its exact position and yaw bits, the heightmap columns of
chunk-generation attempts, and the next draw of the spawner's random, which
pins every draw in between. Worlds cover the overworld, nether and end, their
biomes, caves, water and lava, block and sky light, day, night and thunder,
peaceful to hard, spawn costs (soul sand valley, warped forest), structure
spawn overrides (fortress, swamp hut, monument, outpost, trial chambers,
ancient city), mob caps, players near and far, spectators and the world spawn.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

R = 48  # world boxes span [-R, 16 + R) around chunk (0, 0)

OVERWORLD = {
    # biome: (top block, filler)
    'minecraft:plains': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:forest': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:birch_forest': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:dark_forest': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:flower_forest': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:sunflower_plains': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:meadow': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:cherry_grove': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:taiga': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:old_growth_pine_taiga': ('minecraft:podzol', 'minecraft:dirt'),
    'minecraft:snowy_taiga': ('minecraft:grass_block[snowy=true]', 'minecraft:dirt'),
    'minecraft:snowy_plains': ('minecraft:snow_block', 'minecraft:dirt'),
    'minecraft:ice_spikes': ('minecraft:snow_block', 'minecraft:packed_ice'),
    'minecraft:frozen_peaks': ('minecraft:snow_block', 'minecraft:stone'),
    'minecraft:jagged_peaks': ('minecraft:snow_block', 'minecraft:stone'),
    'minecraft:stony_peaks': ('minecraft:stone', 'minecraft:stone'),
    'minecraft:windswept_hills': ('minecraft:grass_block', 'minecraft:stone'),
    'minecraft:desert': ('minecraft:sand', 'minecraft:sandstone'),
    'minecraft:badlands': ('minecraft:red_sand', 'minecraft:terracotta'),
    'minecraft:wooded_badlands': ('minecraft:coarse_dirt', 'minecraft:terracotta'),
    'minecraft:savanna': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:jungle': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:bamboo_jungle': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:sparse_jungle': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:swamp': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:mangrove_swamp': ('minecraft:mud', 'minecraft:mud'),
    'minecraft:mushroom_fields': ('minecraft:mycelium', 'minecraft:dirt'),
    'minecraft:beach': ('minecraft:sand', 'minecraft:sand'),
    'minecraft:snowy_beach': ('minecraft:sand', 'minecraft:sand'),
    'minecraft:stony_shore': ('minecraft:stone', 'minecraft:stone'),
    'minecraft:river': ('minecraft:sand', 'minecraft:dirt'),
    'minecraft:pale_garden': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:grove': ('minecraft:snow_block', 'minecraft:dirt'),
    'minecraft:snowy_slopes': ('minecraft:snow_block', 'minecraft:stone'),
    'minecraft:savanna_plateau': ('minecraft:grass_block', 'minecraft:dirt'),
    'minecraft:eroded_badlands': ('minecraft:red_sand', 'minecraft:terracotta'),
}
OCEANS = ['minecraft:ocean', 'minecraft:deep_ocean', 'minecraft:warm_ocean', 'minecraft:lukewarm_ocean',
          'minecraft:deep_lukewarm_ocean', 'minecraft:cold_ocean', 'minecraft:deep_cold_ocean',
          'minecraft:frozen_ocean', 'minecraft:deep_frozen_ocean', 'minecraft:river', 'minecraft:frozen_river']
CAVES = ['minecraft:lush_caves', 'minecraft:dripstone_caves', 'minecraft:deep_dark', 'minecraft:sulfur_caves']
NETHER = {
    'minecraft:nether_wastes': 'minecraft:netherrack',
    'minecraft:crimson_forest': 'minecraft:crimson_nylium',
    'minecraft:warped_forest': 'minecraft:warped_nylium',
    'minecraft:soul_sand_valley': 'minecraft:soul_sand',
    'minecraft:basalt_deltas': 'minecraft:basalt',
}
END = ['minecraft:the_end', 'minecraft:end_highlands', 'minecraft:end_midlands', 'minecraft:small_end_islands',
       'minecraft:end_barrens']
MOBS = {
    'monster': ['zombie', 'skeleton', 'creeper', 'spider', 'enderman', 'witch', 'slime', 'drowned', 'husk', 'stray',
                'zombie_villager', 'cave_spider', 'bogged', 'parched'],
    'creature': ['cow', 'pig', 'sheep', 'chicken', 'horse', 'rabbit', 'wolf', 'fox', 'llama', 'polar_bear', 'goat',
                 'strider'],
    'ambient': ['bat'],
    'water_creature': ['squid', 'dolphin'],
    'water_ambient': ['cod', 'salmon', 'tropical_fish', 'pufferfish'],
    'axolotls': ['axolotl'],
    'underground_water_creature': ['glow_squid'],
    'nether': ['zombified_piglin', 'piglin', 'hoglin', 'ghast', 'magma_cube', 'skeleton', 'enderman', 'strider',
               'blaze', 'wither_skeleton'],
}
CATEGORIES = ['monster', 'creature', 'ambient', 'axolotls', 'underground_water_creature', 'water_creature',
              'water_ambient']
TORCH_LIGHT = [0, 0, 0, 1, 4, 7, 8, 9, 12, 14]


def java_next_int(seed, bound):
    """java.util.Random(seed).nextInt(bound)."""
    mask = (1 << 48) - 1
    state = (seed ^ 0x5DEECE66D) & mask
    while True:
        state = (state * 0x5DEECE66D + 0xB) & mask
        bits = state >> 17
        value = bits % bound
        if bits - value + bound - 1 < 2**31:
            return value


def slime_chunk(world_seed, cx, cz):
    """WorldgenRandom.seedSlimeChunk(cx, cz, seed, 987234911).nextInt(10) == 0."""
    def i32(v):
        return (v + 2**31) % 2**32 - 2**31
    seed = (world_seed + i32(cx * cx * 4987142) + i32(cx * 5947611) + i32(cz * cz) * 4392871 + i32(cz * 389711)) ^ 987234911
    return java_next_int(seed & (2**64 - 1), 10) == 0


def box(x0, y0, z0, x1, y1, z1):
    return f'{x0},{y0},{z0},{x1},{y1},{z1}'


class World:
    """One case's world, written as the oracle's key=value tokens."""

    def __init__(self, rng):
        self.rng = rng
        self.kv = {}
        self.blocks, self.lights, self.biomes, self.starts, self.entities, self.players = [], [], [], [], [], []
        self.sites = []  # (x, y, z, kind): open positions above a floor, in water or in lava
        self.dim = 'minecraft:overworld'
        self.surface = 64
        self.min_y = -64

    def block(self, b, state):
        self.blocks.append(f'{b}/{state}')

    def light(self, b, sky, block):
        self.lights.append(f'{b}/{sky},{block}')

    def tokens(self):
        out = dict(self.kv)
        for prefix, items in (('k', self.blocks), ('l', self.lights), ('b', self.biomes), ('s', self.starts),
                              ('e', self.entities), ('p', self.players)):
            for i, v in enumerate(items):
                out[f'{prefix}{i}'] = v
        return ' '.join(f'{k}={v}' for k, v in out.items())


def overworld(w, rng, biome=None, ocean=None):
    lo, hi = -R, 16 + R - 1
    w.kv['dim'] = 'minecraft:overworld'
    w.kv['sea'] = 63
    ocean = rng.random() < 0.2 if ocean is None else ocean
    if ocean:
        biome = biome or rng.choice(OCEANS)
        floor = rng.randrange(28, 58)
        w.kv['biome'] = biome
        w.block(box(lo, -64, lo, hi, floor - 1, hi), 'minecraft:stone')
        w.block(box(lo, floor - 1, lo, hi, floor - 1, hi), rng.choice(['minecraft:sand', 'minecraft:gravel', 'minecraft:clay']))
        top = 62
        water = 'minecraft:ice' if 'frozen' in biome and rng.random() < 0.5 else 'minecraft:water'
        w.block(box(lo, floor, lo, hi, 62, hi), 'minecraft:water')
        if water == 'minecraft:ice':
            w.block(box(lo, 62, lo, hi, 62, hi), 'minecraft:ice')
        for _ in range(6):
            w.sites.append((rng.randrange(-4, 20), rng.randrange(floor, 62), rng.randrange(-4, 20), 'water'))
        w.surface = 63
        depth = 62 - floor
        w.light(box(lo, -64, lo, hi, 62, hi), max(0, 15 - depth // 2) if rng.random() < 0.5 else 15, 0)
        w.light(box(lo, -64, lo, hi, floor - 1, hi), 0, 0)
        # An island.
        if rng.random() < 0.3:
            x, z = rng.randrange(-8, 20), rng.randrange(-8, 20)
            w.block(box(x, floor, z, x + rng.randrange(3, 12), 64, z + rng.randrange(3, 12)), 'minecraft:sand')
        return
    biome = biome or rng.choice(list(OVERWORLD))
    top, filler = OVERWORLD[biome]
    surface = rng.randrange(58, 90)
    w.surface = surface + 1
    w.kv['biome'] = biome
    w.block(box(lo, -64, lo, hi, surface - 4, hi), rng.choice(['minecraft:stone', 'minecraft:stone', 'minecraft:deepslate']))
    w.block(box(lo, surface - 3, lo, hi, surface - 1, hi), filler)
    w.block(box(lo, surface, lo, hi, surface, hi), top)
    w.light(box(lo, -64, lo, hi, surface, hi), 0, 0)
    for _ in range(4):
        w.sites.append((rng.randrange(-2, 18), surface + 1, rng.randrange(-2, 18), 'surface'))
    # Terraces and hills at other heights.
    for _ in range(rng.randrange(0, 4)):
        x, z, dy = rng.randrange(lo, hi), rng.randrange(lo, hi), rng.choice([-3, -2, -1, 1, 2, 3, 5])
        sx, sz = rng.randrange(3, 20), rng.randrange(3, 20)
        if dy > 0:
            w.block(box(x, surface, z, x + sx, surface + dy - 1, z + sz), filler)
            w.block(box(x, surface + dy, z, x + sx, surface + dy, z + sz), top)
        else:
            w.block(box(x, surface + dy + 1, z, x + sx, surface, z + sz), 'minecraft:air')
            w.block(box(x, surface + dy, z, x + sx, surface + dy, z + sz), top)
            w.light(box(x, surface + dy + 1, z, x + sx, surface, z + sz), 15, 0)
    # Ponds, lava, trees and features on the surface.
    for _ in range(rng.randrange(0, 3)):
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        fluid = 'minecraft:water' if rng.random() < 0.8 else 'minecraft:lava'
        d = rng.randrange(1, 4)
        w.block(box(x, surface - d + 1, z, x + rng.randrange(2, 9), surface, z + rng.randrange(2, 9)), fluid)
        w.sites.append((x, surface, z, 'water' if fluid == 'minecraft:water' else 'lava'))
    for _ in range(rng.randrange(0, 4)):
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        h = rng.randrange(4, 7)
        leaves = rng.choice(['minecraft:oak_leaves', 'minecraft:spruce_leaves', 'minecraft:jungle_leaves', 'minecraft:birch_leaves'])
        w.block(box(x - 2, surface + h - 2, z - 2, x + 2, surface + h + 1, z + 2), leaves)
        w.block(box(x, surface + 1, z, x, surface + h, z), 'minecraft:oak_log')
        w.light(box(x - 2, surface + 1, z - 2, x + 2, surface + h - 3, z + 2), 14, 0)
    for _ in range(rng.randrange(0, 6)):
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        thing = rng.choice(['minecraft:short_grass', 'minecraft:tall_grass[half=lower]', 'minecraft:snow[layers=1]',
                            'minecraft:oak_slab[type=bottom]', 'minecraft:glass', 'minecraft:rail', 'minecraft:torch',
                            'minecraft:sweet_berry_bush', 'minecraft:powder_snow', 'minecraft:cobweb',
                            'minecraft:white_carpet', 'minecraft:oak_fence', 'minecraft:redstone_block', 'minecraft:magma_block'])
        w.block(box(x, surface + 1, z, x + rng.randrange(0, 5), surface + 1, z + rng.randrange(0, 5)), thing)
    # Caves: lit or dark rooms underground, some with water, a cave biome.
    for _ in range(rng.choice([0, 1, 2, 2, 3, 4, 5])):
        x, z = rng.randrange(-20, 14), rng.randrange(-20, 14)
        y = rng.randrange(-60, surface - 6)
        sx, sy, sz = rng.randrange(4, 24), rng.randrange(2, 8), rng.randrange(4, 24)
        y1 = min(y + sy, surface - 2)
        w.block(box(x, y, z, x + sx, y1, z + sz), 'minecraft:cave_air' if rng.random() < 0.5 else 'minecraft:air')
        w.light(box(x, y, z, x + sx, y1, z + sz), 0, rng.choice(TORCH_LIGHT))
        wet = rng.random() < 0.35
        if wet:
            w.block(box(x, y, z, x + sx, y + rng.randrange(1, 4), z + sz), 'minecraft:water')
        if rng.random() < 0.3:
            w.block(box(x, y - 1, z, x + sx, y - 1, z + sz), rng.choice(['minecraft:clay', 'minecraft:moss_block', 'minecraft:sculk', 'minecraft:dripstone_block', 'minecraft:clay']))
        if rng.random() < 0.45:
            w.biomes.append(f'{box(x - 2, y - 2, z - 2, x + sx + 2, y1 + 2, z + sz + 2)}/{rng.choice(CAVES)}')
        for _ in range(3):
            w.sites.append((rng.randrange(x, x + sx + 1), y + (rng.randrange(0, 2) if wet else 0), rng.randrange(z, z + sz + 1), 'water' if wet else 'cave'))
        if rng.random() < 0.2:
            # An opening to the sky.
            w.block(box(x + 1, y1, z + 1, x + 3, surface, z + 3), 'minecraft:air')
            w.light(box(x + 1, y, z + 1, x + 3, surface, z + 3), 15, 0)


def nether(w, rng, biome=None):
    lo, hi = -R, 16 + R - 1
    biome = biome or rng.choice(list(NETHER))
    w.kv['dim'] = 'minecraft:the_nether'
    w.kv['sea'] = 32
    w.kv['biome'] = biome
    w.kv['sky'] = 0
    floor = rng.randrange(28, 80)
    w.surface = 128
    w.block(box(lo, 0, lo, hi, 0, hi), 'minecraft:bedrock')
    w.block(box(lo, 1, lo, hi, floor - 1, hi), 'minecraft:netherrack')
    w.block(box(lo, floor, lo, hi, floor, hi), NETHER[biome])
    w.block(box(lo, rng.randrange(100, 120), lo, hi, 126, hi), 'minecraft:netherrack')
    w.block(box(lo, 127, lo, hi, 127, hi), 'minecraft:bedrock')
    w.light(box(lo, 0, lo, hi, 127, hi), 0, rng.choice([0, 0, 3, 7, 11]))
    for _ in range(4):
        w.sites.append((rng.randrange(-2, 18), floor + 1, rng.randrange(-2, 18), 'surface'))
    if floor <= 34 or rng.random() < 0.4:
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        w.block(box(x, min(floor, 20), z, x + rng.randrange(6, 30), 31, z + rng.randrange(6, 30)), 'minecraft:lava')
        w.sites.append((x + 2, 31, z + 2, 'lava'))
        w.light(box(x, 20, z, x + 30, 40, z + 30), 0, 15)
    for _ in range(rng.randrange(0, 4)):
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        y = rng.randrange(floor - 10, floor + 20)
        w.block(box(x, y, z, x + rng.randrange(3, 14), y, z + rng.randrange(3, 14)),
                rng.choice(['minecraft:netherrack', 'minecraft:nether_wart_block', 'minecraft:soul_soil', 'minecraft:magma_block',
                            'minecraft:glowstone', 'minecraft:blackstone', 'minecraft:nether_bricks', 'minecraft:fire']))
    if rng.random() < 0.3:
        x, z = rng.randrange(lo, hi), rng.randrange(lo, hi)
        w.biomes.append(f'{box(x, 0, z, x + rng.randrange(8, 40), 127, z + rng.randrange(8, 40))}/{rng.choice(list(NETHER))}')
    return floor


def end(w, rng):
    lo, hi = -R, 16 + R - 1
    w.kv['dim'] = 'minecraft:the_end'
    w.kv['sea'] = 0
    w.kv['sky'] = 0
    w.kv['biome'] = rng.choice(END)
    top = rng.randrange(40, 70)
    w.surface = top + 1
    w.block(box(lo, top - rng.randrange(5, 30), lo, hi, top, hi), 'minecraft:end_stone')
    w.light(box(lo, 0, lo, hi, 255, hi), 0, rng.choice([0, 0, 5, 14]))
    for _ in range(4):
        w.sites.append((rng.randrange(-2, 18), top + 1, rng.randrange(-2, 18), 'surface'))


def structures(w, rng):
    dim = w.kv['dim']
    s = w.surface
    if dim == 'minecraft:the_nether':
        if rng.random() < 0.6:
            # Fortress corridors on nether bricks (FORTRESS_ENEMIES and the piece override).
            y = rng.randrange(40, 80)
            pieces = []
            for _ in range(rng.randrange(1, 4)):
                x, z = rng.randrange(-30, 30), rng.randrange(-30, 30)
                along = rng.random() < 0.5
                x1, z1 = (x + rng.randrange(10, 40), z + 4) if along else (x + 4, z + rng.randrange(10, 40))
                pieces.append(box(x, y, z, x1, y + 6, z1))
                w.block(box(x, y, z, x1, y, z1), 'minecraft:nether_bricks')
                w.block(box(x, y + 1, z, x1, y + 5, z1), 'minecraft:air')
                w.light(box(x, y + 1, z, x1, y + 5, z1), 0, rng.choice([0, 7, 11]))
                w.block(box(x, y + 6, z, x1, y + 6, z1), 'minecraft:nether_bricks')
                w.sites.append((rng.randrange(x, x1 + 1), y + 1, rng.randrange(z, z1 + 1), 'structure'))
            w.starts.append('minecraft:fortress/' + ';'.join(pieces))
        if rng.random() < 0.2:
            x, z = rng.randrange(-20, 20), rng.randrange(-20, 20)
            w.starts.append(f'minecraft:bastion_remnant/{box(x, 30, z, x + 30, 80, z + 30)}')
        return
    if dim != 'minecraft:overworld':
        return
    biome = w.kv['biome']
    roll = rng.random()
    x, z = rng.randrange(-24, 24), rng.randrange(-24, 24)
    if biome in ('minecraft:swamp', 'minecraft:mangrove_swamp') and roll < 0.7:
        # Swamp hut: witches and cats only in its piece.
        y = s + 1
        w.block(box(x, y, z, x + 6, y, z + 8), 'minecraft:spruce_planks')
        w.block(box(x + 1, y + 1, z + 1, x + 5, y + 4, z + 7), 'minecraft:air')
        w.light(box(x + 1, y + 1, z + 1, x + 5, y + 4, z + 7), rng.choice([0, 3, 9]), 0)
        w.block(box(x, y + 5, z, x + 6, y + 5, z + 8), 'minecraft:spruce_planks')
        w.starts.append(f'minecraft:swamp_hut/{box(x, y, z, x + 6, y + 6, z + 8)}')
        w.sites.append((x + 3, y + 1, z + 4, 'structure'))
    elif 'ocean' in biome and roll < 0.6:
        w.starts.append(f'minecraft:monument/{box(x, 39, z, x + 57, 61, z + 57)}')
        w.sites.append((x + 8, rng.randrange(40, 61), z + 8, 'structure'))
    elif roll < 0.15:
        w.starts.append(f'minecraft:pillager_outpost/{box(x, s, z, x + 8, s + 20, z + 8)};{box(x + 10, s, z + 2, x + 14, s + 5, z + 6)}')
        w.sites.append((x + 4, s, z + 4, 'structure'))
    elif roll < 0.25:
        y = rng.randrange(-40, -10)
        w.block(box(x, y, z, x + 20, y + 8, z + 20), 'minecraft:air')
        w.block(box(x, y - 1, z, x + 20, y - 1, z + 20), 'minecraft:tuff_bricks')
        w.light(box(x, y, z, x + 20, y + 8, z + 20), 0, 0)
        w.starts.append(f'minecraft:trial_chambers/{box(x, y - 1, z, x + 20, y + 9, z + 20)}')
        w.sites.append((x + 5, y, z + 5, 'structure'))
    elif roll < 0.32:
        y = rng.randrange(-52, -30)
        w.block(box(x, y, z, x + 30, y + 10, z + 30), 'minecraft:air')
        w.block(box(x, y - 1, z, x + 30, y - 1, z + 30), 'minecraft:deepslate_tiles')
        w.light(box(x, y, z, x + 30, y + 10, z + 30), 0, 0)
        w.biomes.append(f'{box(x - 4, y - 4, z - 4, x + 34, y + 14, z + 34)}/minecraft:deep_dark')
        w.starts.append(f'minecraft:ancient_city/{box(x, y - 1, z, x + 30, y + 11, z + 30)}')
        w.sites.append((x + 5, y, z + 5, 'structure'))
    elif roll < 0.4:
        w.starts.append(f'minecraft:village_plains/{box(x, s - 2, z, x + 12, s + 8, z + 12)}')


def mob_entities(w, rng):
    dim = w.kv['dim']
    pool = MOBS['nether'] if dim == 'minecraft:the_nether' else sum((MOBS[c] for c in CATEGORIES if c in MOBS), [])
    n = rng.choice([0, 0, 1, 3, 6, 12, 25, 80])
    for _ in range(n):
        t = rng.choice(pool)
        x, z = rng.uniform(-R + 2, 16 + R - 2), rng.uniform(-R + 2, 16 + R - 2)
        y = rng.choice([w.surface, w.surface, rng.randrange(w.min_y + 2, w.surface + 1)])
        persistent = '/p' if rng.random() < 0.2 else ''
        w.entities.append(f'minecraft:{t}/{x:.3f},{y},{z:.3f}{persistent}')
    if w.kv.get('biome') in ('minecraft:soul_sand_valley', 'minecraft:warped_forest') or any('soul_sand_valley' in b or 'warped' in b for b in w.biomes):
        for _ in range(rng.randrange(0, 10)):
            t = rng.choice(['skeleton', 'ghast', 'enderman', 'strider'])
            w.entities.append(f'minecraft:{t}/{rng.uniform(-10, 26):.3f},{w.sites[0][1]},{rng.uniform(-10, 26):.3f}')
    # Clusters right where spawns happen (obstruction and spawn potential).
    if rng.random() < 0.3:
        t = rng.choice(pool)
        for _ in range(rng.randrange(1, 8)):
            w.entities.append(f'minecraft:{t}/{rng.randrange(0, 16) + 0.5},{w.surface},{rng.randrange(0, 16) + 0.5}')


def place_players(w, rng):
    count = rng.choice([1, 1, 1, 1, 2, 3])
    for _ in range(count):
        kind = rng.random()
        if kind < 0.75:
            # 24 to ~100 blocks from the chunk: most attempts are allowed.
            ang, dist = rng.uniform(0, 6.283), rng.uniform(26, 100)
        elif kind < 0.87:
            ang, dist = rng.uniform(0, 6.283), rng.uniform(0, 30)
        else:
            ang, dist = rng.uniform(0, 6.283), rng.uniform(100, 200)
        import math
        x, z = 8 + dist * math.cos(ang), 8 + dist * math.sin(ang)
        y = w.surface + rng.choice([0, 0, 5, 30, -20])
        mode = rng.choice(['survival'] * 8 + ['creative', 'adventure', 'spectator'])
        w.players.append(f'{x:.3f},{y},{z:.3f}/{mode}')


def common(w, rng, seed):
    w.kv['seed'] = seed
    w.kv['wseed'] = rng.randrange(-2**63, 2**63)
    if w.kv['dim'] == 'minecraft:overworld':
        w.kv['darken'] = rng.choice([0, 0, 2, 4, 7, 11, 11, 11, 11, 11])
        if rng.random() < 0.08:
            w.kv['thunder'] = 1
        if rng.random() < 0.3:
            w.kv['slime'] = rng.choice([0.0, 0.125, 0.25, 0.5])
    w.kv['difficulty'] = rng.choice(['normal', 'normal', 'hard', 'easy', 'peaceful'])
    if rng.random() < 0.15:
        w.kv['respawn'] = f'{rng.randrange(-20, 36)},{w.surface},{rng.randrange(-20, 36)}/{w.kv["dim"]}'
    if rng.random() < 0.2:
        w.kv['unloaded'] = ';'.join(f'{rng.choice([-1, 0, 1])},{rng.choice([-1, 0, 1])}' for _ in range(rng.randrange(1, 5)))


def make_world(rng, seed, dim=None, biome=None, ocean=None):
    w = World(rng)
    dim = dim or rng.choices(['overworld', 'nether', 'end'], [70, 22, 8])[0]
    if dim == 'overworld':
        overworld(w, rng, biome=biome, ocean=ocean)
    elif dim == 'nether':
        w.min_y = 0
        nether(w, rng, biome=biome)
    else:
        w.min_y = 0
        end(w, rng)
    structures(w, rng)
    mob_entities(w, rng)
    place_players(w, rng)
    common(w, rng, seed)
    return w


def corpus():
    rng = random.Random(0x4E5350)
    cases = []
    # Whole chunk ticks.
    for k in range(3000):
        w = make_world(rng, rng.randrange(-2**47, 2**47))
        if rng.random() < 0.2:
            w.kv['chunkcount'] = rng.choice([0, 1, 4, 17, 40, 100, 200, 289, 578])
        w.kv['enemies'] = 1 if rng.random() < 0.9 else 0
        w.kv['persistent'] = 1 if rng.random() < 0.5 else 0
        cases.append(f'chunk/{k} mode=chunk {w.tokens()}')
    # Chosen starts at sites: surfaces, cave floors, water, lava, structure floors.
    for k in range(7000):
        w = make_world(rng, rng.randrange(-2**47, 2**47))
        dim = w.kv['dim']
        x, y, z, kind = rng.choice(w.sites)
        if kind == 'structure' and rng.random() < 0.5:
            x, z = x + rng.randrange(-3, 4), z + rng.randrange(-3, 4)
        if rng.random() < 0.1:
            y = rng.randrange(w.min_y + 1, w.surface + 2)
        if dim == 'minecraft:the_nether':
            category = 'creature' if kind == 'lava' or rng.random() < 0.2 else 'monster'
        elif dim == 'minecraft:the_end':
            category = 'monster'
        elif kind == 'water':
            category = rng.choice(['water_creature', 'water_ambient', 'axolotls', 'underground_water_creature', 'monster'])
        elif kind == 'cave':
            category = rng.choice(['monster', 'monster', 'ambient', 'ambient', 'creature'])
        elif kind == 'structure':
            category = rng.choice(['monster', 'monster', 'creature', 'ambient', 'water_ambient'])
        else:
            category = rng.choice(['monster', 'creature', 'creature', 'ambient'])
        if rng.random() < 0.05:
            category = rng.choice(CATEGORIES)
        cases.append(f'pos/{k} mode=pos cat={category} start={x},{y},{z} {w.tokens()}')
    # Targeted nights and places for the rarer types.
    targets = [
        ('minecraft:desert', False, 'surface', 'monster'), ('minecraft:desert', False, 'surface', 'creature'),
        ('minecraft:snowy_plains', False, 'surface', 'monster'), ('minecraft:ice_spikes', False, 'surface', 'creature'),
        ('minecraft:swamp', False, 'surface', 'monster'), ('minecraft:mangrove_swamp', False, 'surface', 'monster'),
        ('minecraft:jungle', False, 'surface', 'monster'), ('minecraft:bamboo_jungle', False, 'surface', 'monster'),
        ('minecraft:plains', False, 'surface', 'monster'), ('minecraft:savanna', False, 'surface', 'creature'),
        ('minecraft:badlands', False, 'surface', 'creature'), ('minecraft:warm_ocean', True, 'water', 'water_ambient'),
        ('minecraft:lukewarm_ocean', True, 'water', 'water_creature'), ('minecraft:deep_ocean', True, 'water', 'monster'),
        ('minecraft:frozen_ocean', True, 'water', 'creature'), ('minecraft:lush_caves', None, 'cave', 'axolotls'),
        ('minecraft:sulfur_caves', None, 'cave', 'monster'), ('minecraft:dripstone_caves', None, 'cave', 'monster'),
        ('minecraft:crimson_forest', 'nether', 'surface', 'monster'), ('slime', None, 'cave', 'monster'),
        ('minecraft:river', True, 'water', 'monster'),
    ]
    for k in range(4000):
        biome, ocean, kind, category = rng.choice(targets)
        if ocean == 'nether':
            w = make_world(rng, rng.randrange(-2**47, 2**47), dim='nether', biome=biome)
        elif kind == 'cave':
            w = make_world(rng, rng.randrange(-2**47, 2**47), dim='overworld', ocean=False)
        else:
            w = make_world(rng, rng.randrange(-2**47, 2**47), dim='overworld', biome=biome, ocean=ocean)
        w.kv['darken'] = 11 if category == 'monster' else rng.choice([0, 0, 4, 11])
        w.kv.pop('thunder', None)
        sites = [t for t in w.sites if t[3] == kind] or w.sites
        x, y, z, _ = rng.choice(sites)
        if kind == 'cave':
            # A dark room under the start, in the cave biome (or a slime chunk).
            w.block(box(x - 6, y, z - 6, x + 6, y + 4, z + 6), 'minecraft:cave_air')
            floor = 'minecraft:clay' if category == 'axolotls' else rng.choice(['minecraft:stone', 'minecraft:deepslate'])
            w.block(box(x - 6, y - 1, z - 6, x + 6, y - 1, z + 6), floor)
            if category == 'axolotls' or biome == 'minecraft:dripstone_caves' and rng.random() < 0.5:
                w.block(box(x - 6, y, z - 6, x + 6, y + 2, z + 6), 'minecraft:water')
            w.light(box(x - 6, y, z - 6, x + 6, y + 4, z + 6), 0, rng.choice([0, 0, 0, 3]))
            if biome == 'slime':
                y = min(y, rng.randrange(-50, 38))
                w.block(box(x - 6, y, z - 6, x + 6, y + 4, z + 6), 'minecraft:cave_air')
                w.block(box(x - 6, y - 1, z - 6, x + 6, y - 1, z + 6), 'minecraft:stone')
                w.light(box(x - 6, y, z - 6, x + 6, y + 4, z + 6), 0, 0)
                while not slime_chunk(w.kv['wseed'], 0, 0):
                    w.kv['wseed'] = rng.randrange(-2**63, 2**63)
            else:
                w.biomes.append(f'{box(x - 8, y - 3, z - 8, x + 8, y + 6, z + 8)}/{biome}')
        if biome in ('minecraft:swamp', 'minecraft:mangrove_swamp'):
            w.kv['slime'] = rng.choice([0.0, 0.25, 0.5])
        cases.append(f'target/{k} mode=pos cat={category} start={x},{y},{z} {w.tokens()}')
    # Chunk generation: creature packs placed on the surface.
    for k in range(1500):
        w = World(rng)
        dim = rng.choices(['overworld', 'nether'], [85, 15])[0]
        if dim == 'overworld':
            biome = rng.choice(list(OVERWORLD) + ['minecraft:frozen_ocean', 'minecraft:beach'] * 2)
            if biome in OVERWORLD:
                overworld(w, rng, biome=biome, ocean=False)
            else:
                overworld(w, rng, biome=biome, ocean=True)
        else:
            w.min_y = 0
            nether(w, rng)
        common(w, rng, rng.randrange(-2**47, 2**47))
        w.kv['gseed'] = rng.randrange(-2**47, 2**47)
        if rng.random() < 0.05:
            w.kv['rules'] = 0
        w.kv['chunk'] = f'{rng.randrange(-3, 3)},{rng.randrange(-3, 3)}'
        cases.append(f'gen/{k} mode=gen {w.tokens()}')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-natural-spawn-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNaturalSpawnOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/natural_spawn_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/spawn_selection.rs',
               lab.ROOT / 'crates/horizon-client-render/src/biome.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/spawn_rules.rs']
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    cases = corpus()
    inputs.write_text(''.join(line + '\n' for line in cases), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx3G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.horizonlab.parity.VanillaNaturalSpawnOracle', inputs, build / 'server.txt'], 'server')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples/client_probe'
    run([binary, 'natural_spawn_fixture', resources, inputs, build / 'rust.txt'], 'rust')
    vanilla = (build / 'server.txt').read_text().splitlines()
    errors = [l for l in vanilla if ' error ' in l]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    rust = (build / 'rust.txt').read_text().splitlines()
    bad = [(a, b) for a, b in zip(vanilla, rust) if a != b]
    if bad or len(vanilla) != len(rust):
        report = '\n'.join(f'  vanilla {a[:1500]}\n  rust    {b[:1500]}' for a, b in bad[:20])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} cases differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged(sources, tested)
    spawns = sum(line.count(' ; spawn ') for line in vanilla)
    attempts = sum(line.count(' ; try ') + line.count(' ; col ') for line in vanilla)
    tests = sum(line.count(' ; test ') for line in vanilla)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        'attempts': attempts, 'type_checks_passed': tests, 'spawns': spawns, 'sources_sha256': tested,
        'scope': 'NaturalSpawner: createState (counts, spawn potential, local caps), getFilteredSpawningCategories, '
                 'spawnForChunk, spawnCategoryForPosition (distance and respawn checks, getRandomSpawnMobAt over mobsAt: '
                 'fortress bounds, structure spawn overrides, biome spawns; isValidSpawnPostitionForType with '
                 'SpawnPlacements; isValidPositionForMob with each class\'s checkSpawnRules, checkSpawnObstruction and '
                 'removeWhenFarAway; cluster and group sizes) and spawnMobsForChunkGeneration, with every random draw. '
                 'finalizeSpawn runs with a fixed random in the oracle; its own outcomes are not compared.'})
    print(f'{len(cases):,} natural spawn cases match vanilla ({attempts:,} attempts, {tests:,} type checks passed, '
          f'{spawns:,} spawns)')


if __name__ == '__main__':
    main()
