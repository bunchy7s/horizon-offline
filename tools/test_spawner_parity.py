#!/usr/bin/env python3
"""Compare monster spawners (BaseSpawner.serverTick) with vanilla.

Java (VanillaNaturalSpawnOracle, mode=spawner) loads a SpawnerBlockEntity from
each case's NBT on the oracle's stand-in ServerLevel and runs
`SpawnerBlockEntity.serverTick` for a number of ticks. Rust
(natural_spawn_fixture, mode=spawner) loads `mobs::spawner::Spawner` from the
same NBT and ticks it on the same world. Both log, tick by tick, every spawn
(type, exact position and yaw bits), `sendBlockUpdated`, the block event, the
2004 level event, the entity_place game event and the spawn animation, then
the saved delay, next spawn data and potentials, and the next draw of the
level random, which pins every draw in between.

Worlds are rooms around the spawner (dungeons, caves, open fields, flooded
rooms, nether halls) with dark, dim and lit light, players inside and outside
the required range (spectators, a negative range), mobs of the spawner's type
already around it, every difficulty and the spawner_blocks_work rule. Spawn
data covers generated spawners (no potentials), loaded ones (potentials
missing, so shared with the next data), weighted potentials (zero weights
included), custom light rules, fixed `Pos`, configured entities (no
finalizeSpawn), ids without a namespace, unknown, invalid and missing ids, and
delays, counts, caps and ranges from the edge values up.

finalizeSpawn runs with a fixed random in the oracle (nextInt(100) answers 99
so spiders do not grow skeleton jockeys); its own outcomes are not compared.
Entity types that are not mobs (Horizon creates only mobs from a spawner) are
not in the corpus.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base, test_natural_spawn_parity as natural
except ImportError:
    import lab
    import test_rust_parity as base
    import test_natural_spawn_parity as natural

box = natural.box
MONSTERS = ['zombie', 'skeleton', 'spider', 'cave_spider', 'blaze', 'silverfish', 'magma_cube', 'creeper', 'witch',
            'enderman', 'zombified_piglin', 'husk', 'stray', 'bogged', 'slime', 'drowned', 'piglin', 'wither_skeleton',
            'endermite', 'parched', 'zombie_villager', 'breeze']
CREATURES = ['pig', 'cow', 'chicken', 'sheep', 'rabbit', 'wolf', 'bat', 'squid', 'cod', 'strider', 'fox', 'horse']
FLOORS = ['minecraft:cobblestone', 'minecraft:mossy_cobblestone', 'minecraft:stone', 'minecraft:deepslate',
          'minecraft:grass_block', 'minecraft:netherrack', 'minecraft:sand', 'minecraft:infested_stone']


def entity_snbt(rng, mob, spos):
    r = rng.random()
    if r < 0.55:
        return f'{{id:"minecraft:{mob}"}}'
    if r < 0.65:
        return f'{{id:"{mob}"}}'
    if r < 0.75:
        return f'{{id:"minecraft:{mob}",PersistenceRequired:1b}}'
    if r < 0.83:
        x, y, z = (spos[0] + rng.uniform(-3, 3), spos[1] + rng.choice([0, 0, 1, -1, 3]), spos[2] + rng.uniform(-3, 3))
        return f'{{id:"minecraft:{mob}",Pos:[{x:.4f}d,{y}.0d,{z:.4f}d]}}'
    if r < 0.88:
        return '{id:"minecraft:not_an_entity"}'
    if r < 0.93:
        return f'{{id:"{mob.capitalize()}"}}'
    return '{}'


def spawn_data(rng, mob, spos, rules=True):
    out = f'entity:{entity_snbt(rng, mob, spos)}'
    if rules and rng.random() < 0.2:
        def limit():
            lo = rng.randrange(0, 16)
            hi = rng.randrange(lo, 16)
            return rng.choice([str(lo), f'[I;{lo},{hi}]', f'{{min_inclusive:{lo},max_inclusive:{hi}}}'])
        parts = []
        if rng.random() < 0.7:
            parts.append(f'block_light_limit:{limit()}')
        if rng.random() < 0.7:
            parts.append(f'sky_light_limit:{limit()}')
        out += ',custom_spawn_rules:{' + ','.join(parts) + '}'
    return '{' + out + '}'


def spawner_snbt(rng, mob, spos, pool):
    fields = []
    shape = rng.random()
    if shape < 0.35:
        # Generated (setEntityId): the next data and no potentials.
        fields.append(f'SpawnData:{{entity:{{id:"minecraft:{mob}"}}}}')
        fields.append('SpawnPotentials:[]')
    elif shape < 0.55:
        # Potentials missing: the next data becomes the one potential.
        fields.append(f'SpawnData:{spawn_data(rng, mob, spos)}')
    elif shape < 0.85:
        n = rng.randrange(1, 5)
        entries = []
        for _ in range(n):
            other = mob if rng.random() < 0.5 else rng.choice(pool)
            weight = rng.choice([1, 1, 2, 5, 0, 20])
            entries.append(f'{{data:{spawn_data(rng, other, spos)},weight:{weight}}}')
        fields.append('SpawnPotentials:[' + ','.join(entries) + ']')
        if rng.random() < 0.5:
            fields.append(f'SpawnData:{spawn_data(rng, mob, spos)}')
    elif shape < 0.95:
        fields.append(f'SpawnData:{spawn_data(rng, mob, spos)}')
        fields.append(f'SpawnPotentials:[{{data:{spawn_data(rng, mob, spos)},weight:1}}]')
    # else: an empty spawner (placed from the item).
    fields.append(f'Delay:{rng.choice([0, 0, 1, 3, 20, -1, 5])}s')
    if rng.random() < 0.8:
        lo = rng.choice([0, 1, 5, 10, 20, 200])
        hi = rng.choice([lo, lo + 1, lo + 10, lo + 40, max(lo - 5, 0)])
        fields.append(f'MinSpawnDelay:{lo}s')
        fields.append(f'MaxSpawnDelay:{hi}s')
    if rng.random() < 0.6:
        fields.append(f'SpawnCount:{rng.choice([1, 2, 4, 4, 6, 8, 0])}s')
    if rng.random() < 0.5:
        fields.append(f'MaxNearbyEntities:{rng.choice([0, 1, 3, 6, 6, 10])}s')
    if rng.random() < 0.4:
        fields.append(f'RequiredPlayerRange:{rng.choice([16, 16, 4, 32, -1, 0])}s')
    if rng.random() < 0.5:
        fields.append(f'SpawnRange:{rng.choice([1, 2, 4, 4, 6, 8])}s')
    return '{' + ','.join(fields) + '}'


def room(rng):
    """A world around the spawner; returns (world, spawner position, the mob)."""
    w = natural.World(rng)
    dim = rng.choices(['overworld', 'nether', 'end'], [80, 17, 3])[0]
    w.dim = f'minecraft:{"the_" if dim != "overworld" else ""}{dim}'
    w.kv['dim'] = w.dim
    w.min_y = -64 if dim == 'overworld' else 0
    sx, sy, sz = rng.randrange(-8, 9), rng.randrange(-20, 60) if dim == 'overworld' else rng.randrange(30, 90), rng.randrange(-8, 9)
    w.surface = sy
    if dim == 'nether':
        w.kv['biome'] = rng.choice(list(natural.NETHER))
        pool = ['blaze', 'magma_cube', 'zombified_piglin', 'wither_skeleton', 'piglin', 'skeleton', 'strider', 'ghast']
    elif dim == 'end':
        w.kv['biome'] = 'minecraft:the_end'
        pool = ['enderman', 'endermite', 'zombie', 'shulker']
    else:
        w.kv['biome'] = rng.choice(['minecraft:plains', 'minecraft:desert', 'minecraft:dripstone_caves',
                                    'minecraft:deep_dark', 'minecraft:swamp', 'minecraft:forest'])
        pool = MONSTERS + CREATURES
    mob = rng.choice(pool)
    half, height = rng.choice([3, 4, 4, 5, 6]), rng.choice([3, 4, 4, 6])
    kind = rng.random()
    floor = rng.choice(FLOORS if dim == 'overworld' else ['minecraft:netherrack', 'minecraft:nether_bricks', 'minecraft:end_stone'])
    if kind < 0.65:
        # A dungeon or hall: solid all around, an air room.
        w.block(box(sx - 12, sy - 6, sz - 12, sx + 12, sy + 10, sz + 12), rng.choice(['minecraft:stone', 'minecraft:cobblestone', 'minecraft:netherrack']))
        w.block(box(sx - half, sy - 1, sz - half, sx + half, sy - 1, sz + half), floor)
        w.block(box(sx - half, sy, sz - half, sx + half, sy + height - 1, sz + half), rng.choice(['minecraft:cave_air', 'minecraft:air']))
    elif kind < 0.85:
        # Open ground.
        w.block(box(sx - 16, sy - 6, sz - 16, sx + 16, sy - 2, sz + 16), 'minecraft:dirt')
        w.block(box(sx - 16, sy - 1, sz - 16, sx + 16, sy - 1, sz + 16), floor)
    else:
        # A flooded room.
        w.block(box(sx - 12, sy - 6, sz - 12, sx + 12, sy + 10, sz + 12), 'minecraft:stone')
        w.block(box(sx - half, sy - 1, sz - half, sx + half, sy - 1, sz + half), floor)
        w.block(box(sx - half, sy, sz - half, sx + half, sy + height - 1, sz + half), 'minecraft:water')
        if rng.random() < 0.5:
            mob = rng.choice(['drowned', 'squid', 'cod', 'guardian', 'zombie'])
    if rng.random() < 0.3:
        # Pillars and slabs in the way.
        for _ in range(rng.randrange(1, 6)):
            px, pz = sx + rng.randrange(-half, half + 1), sz + rng.randrange(-half, half + 1)
            w.block(box(px, sy, pz, px, sy + rng.randrange(0, 3), pz), rng.choice(['minecraft:cobblestone', 'minecraft:oak_slab', 'minecraft:torch', 'minecraft:glass']))
    w.block(box(sx, sy, sz, sx, sy, sz), 'minecraft:spawner')
    sky, light = rng.choice([(0, 0), (0, 0), (0, 0), (0, 3), (0, 7), (0, 12), (15, 0), (15, 15), (8, 8)])
    w.light(box(sx - 16, sy - 2, sz - 16, sx + 16, sy + 12, sz + 16), sky, light)
    if rng.random() < 0.25:
        # A torch's glow on one side.
        w.light(box(sx, sy - 1, sz - half, sx + half, sy + height, sz + half), sky, rng.choice([7, 11, 14]))
    return w, (sx, sy, sz), mob, pool


def players(w, rng, spos):
    for _ in range(rng.choice([1, 1, 1, 2, 0])):
        r = rng.random()
        if r < 0.7:
            d = rng.uniform(0, 15)
        elif r < 0.85:
            d = rng.uniform(14, 18)
        else:
            d = rng.uniform(18, 40)
        import math
        a, b = rng.uniform(0, 6.283), rng.uniform(-0.6, 0.6)
        x = spos[0] + 0.5 + d * math.cos(a) * math.cos(b)
        y = spos[1] + 0.5 + d * math.sin(b)
        z = spos[2] + 0.5 + d * math.sin(a) * math.cos(b)
        mode = rng.choice(['survival'] * 7 + ['creative', 'adventure', 'spectator'])
        w.players.append(f'{x:.3f},{y:.3f},{z:.3f}/{mode}')


def corpus():
    rng = random.Random(0x5350574E)
    cases = []
    for k in range(6000):
        w, spos, mob, pool = room(rng)
        players(w, rng, spos)
        if rng.random() < 0.5:
            for _ in range(rng.choice([1, 2, 3, 5, 6, 8])):
                t = mob if rng.random() < 0.8 else rng.choice(pool)
                x, z = spos[0] + rng.uniform(-6, 7), spos[2] + rng.uniform(-6, 7)
                w.entities.append(f'minecraft:{t}/{x:.3f},{spos[1] + rng.choice([0, 0, -1, 2])},{z:.3f}')
        natural.common(w, rng, rng.randrange(-2**47, 2**47))
        w.kv.pop('unloaded', None)
        if rng.random() < 0.05:
            w.kv['spawners'] = 0
        snbt = spawner_snbt(rng, mob, spos, pool)
        w.kv['spos'] = f'{spos[0]},{spos[1]},{spos[2]}'
        w.kv['snbt'] = base64.b64encode(snbt.encode()).decode()
        w.kv['ticks'] = rng.choice([1, 2, 5, 21, 40, 80, 120])
        cases.append(f'spawner/{k} mode=spawner {w.tokens()}')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-spawner-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNaturalSpawnOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/natural_spawn_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/spawner.rs',
               lab.ROOT / 'crates/horizon-client/src/chunk/block_entity.rs',
               lab.ROOT / 'crates/horizon-client-core/src/identifier.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/spawn_selection.rs',
               lab.ROOT / 'crates/horizon-client/src/biome.rs',
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
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--example', 'client_probe'], 'compile-rust')
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
        report = '\n'.join(f'  vanilla {a[:2000]}\n  rust    {b[:2000]}' for a, b in bad[:12])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} cases differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged(sources, tested)
    spawns = sum(line.count(' spawn minecraft:') for line in vanilla)
    delays = sum(line.count(' bevent 1 ') for line in vanilla)
    updates = sum(line.count(' update') for line in vanilla)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        'spawns': spawns, 'delays': delays, 'block_updates': updates, 'sources_sha256': tested,
        'scope': 'BaseSpawner: load (SpawnData and potentials codecs, shared next data), serverTick (player range, '
                 'spawner_blocks_work, delay, getOrCreateNextSpawnData, Pos, getSpawnAABB collision, custom spawn '
                 'rules and peaceful, SpawnPlacements, the nearby cap, yaw, checkSpawnRules and checkSpawnObstruction, '
                 'events) and delay (weighted potentials), with every level random draw. finalizeSpawn runs with a '
                 'fixed random in the oracle; its own outcomes are not compared.'})
    print(f'{len(cases):,} spawner cases match vanilla ({spawns:,} spawns, {delays:,} delays, {updates:,} block updates)')


if __name__ == '__main__':
    main()
