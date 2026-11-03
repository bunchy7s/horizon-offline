#!/usr/bin/env python3
"""Compare spawn placement checks and spawn predicates with vanilla.

Java (VanillaSpawnRulesOracle) runs SpawnPlacements.isSpawnPositionOk and
checkSpawnRules on a stand-in ServerLevel holding one column of blocks with
uniform light; Rust (spawn_rules_fixture) runs horizon_client::mobs::spawn_rules
on the same column. Every mob type is tried on surfaces, caves, water, lava,
the nether and the end, dangerous blocks, light levels, thunder, difficulty,
biomes, slime chunks, nearby players and several spawn reasons; each case
also prints the next draw of its random, pinning the number of draws.
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

# Living types that are not Horizon mob types or not mobs at all.
SKIPPED = {'player', 'mannequin', 'armor_stand', 'ender_dragon', 'giant', 'copper_golem'}

SCENARIOS = {
    'grass-day': dict(b=dict(m1='minecraft:grass_block'), sky=15, surface=64),
    'grass-night': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=11, surface=64),
    'grass-dusk': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=7, surface=64),
    'grass-thunder': dict(b=dict(m1='minecraft:grass_block'), sky=15, thunder=1, surface=64),
    'grass-peaceful': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=11, difficulty='peaceful', surface=64),
    'grass-torch': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=11, light=9, surface=64),
    'cave-dark': dict(b=dict(m1='minecraft:stone'), sky=0, y=20, surface=80),
    'cave-lit1': dict(b=dict(m1='minecraft:stone'), sky=0, light=1, y=20, surface=80),
    'cave-lit8': dict(b=dict(m1='minecraft:deepslate'), sky=0, light=8, y=-20, surface=80),
    'cave-lit12': dict(b=dict(m1='minecraft:stone'), sky=0, light=12, y=20, surface=80),
    'cave-dim': dict(b=dict(m1='minecraft:stone'), sky=3, y=40, surface=80),
    'water-surface': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=15, y=60, surface=64, biome='minecraft:ocean'),
    'water-warm': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=15, y=58, surface=64, biome='minecraft:warm_ocean'),
    'water-deep': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=0, y=25, surface=64, biome='minecraft:deep_ocean'),
    'water-deep-lit': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=0, light=3, y=25, surface=64, biome='minecraft:lukewarm_ocean'),
    'water-mid': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=15, y=45, surface=64, biome='minecraft:cold_ocean'),
    'water-roofed': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water', p3='minecraft:stone'), sky=0, y=50, surface=64, biome='minecraft:ocean'),
    'water-river': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:water'), sky=0, y=50, surface=64, biome='minecraft:river', darken=11),
    'water-conductor': dict(b=dict(m1='minecraft:water', p0='minecraft:water', p1='minecraft:stone'), sky=15, y=60, surface=64),
    'water-seagrass': dict(b=dict(m1='minecraft:sand', p0='minecraft:water', p1='minecraft:water'), sky=15, y=55, surface=64, biome='minecraft:ocean'),
    'water-clay': dict(b=dict(m1='minecraft:clay', p0='minecraft:water', p1='minecraft:water'), sky=0, y=10, surface=64, biome='minecraft:lush_caves'),
    'lava-nether': dict(b=dict(m1='minecraft:lava', p0='minecraft:lava', p1='minecraft:lava'), sky=0, light=15, y=31, sea=32, dim='minecraft:the_nether', biome='minecraft:basalt_deltas'),
    'lava-open': dict(b=dict(m1='minecraft:lava', p0='minecraft:lava'), sky=0, light=15, y=31, sea=32, dim='minecraft:the_nether', biome='minecraft:nether_wastes'),
    'nether-dark': dict(b=dict(m1='minecraft:netherrack'), sky=0, light=0, y=70, sea=32, dim='minecraft:the_nether', biome='minecraft:nether_wastes'),
    'nether-lit7': dict(b=dict(m1='minecraft:netherrack'), sky=0, light=7, y=70, sea=32, dim='minecraft:the_nether', biome='minecraft:crimson_forest'),
    'nether-lit8': dict(b=dict(m1='minecraft:crimson_nylium'), sky=0, light=8, y=70, sea=32, dim='minecraft:the_nether', biome='minecraft:crimson_forest'),
    'nether-wart': dict(b=dict(m1='minecraft:nether_wart_block'), sky=0, light=0, y=70, sea=32, dim='minecraft:the_nether', biome='minecraft:crimson_forest'),
    'nether-soul': dict(b=dict(m1='minecraft:soul_sand'), sky=0, light=3, y=40, sea=32, dim='minecraft:the_nether', biome='minecraft:soul_sand_valley'),
    'nether-magma': dict(b=dict(m1='minecraft:magma_block'), sky=0, light=3, y=40, sea=32, dim='minecraft:the_nether', biome='minecraft:basalt_deltas'),
    'end': dict(b=dict(m1='minecraft:end_stone'), sky=0, light=0, y=60, dim='minecraft:the_end', biome='minecraft:the_end'),
    'end-lit': dict(b=dict(m1='minecraft:end_stone'), sky=0, light=14, y=60, dim='minecraft:the_end', biome='minecraft:end_highlands'),
    'ice-frozen': dict(b=dict(m1='minecraft:ice'), sky=15, y=63, surface=64, biome='minecraft:frozen_ocean'),
    'ice-plains': dict(b=dict(m1='minecraft:packed_ice'), sky=15, y=64, surface=64, biome='minecraft:snowy_plains'),
    'snow-block': dict(b=dict(m1='minecraft:snow_block'), sky=15, darken=11, y=70, surface=70, biome='minecraft:snowy_plains'),
    'snow-layer': dict(b=dict(m1='minecraft:grass_block[snowy=true]', p0='minecraft:snow'), sky=15, y=70, surface=71, biome='minecraft:snowy_taiga'),
    'powder-snow-stray': dict(b=dict(m1='minecraft:snow_block', p1='minecraft:powder_snow', p2='minecraft:powder_snow'), sky=15, darken=11, y=90, surface=93, biome='minecraft:frozen_peaks'),
    'sand-beach': dict(b=dict(m1='minecraft:sand'), sky=15, y=63, surface=64, biome='minecraft:beach'),
    'sand-desert': dict(b=dict(m1='minecraft:sand'), sky=15, darken=11, y=70, surface=70, biome='minecraft:desert'),
    'red-sand': dict(b=dict(m1='minecraft:red_sand'), sky=15, y=80, surface=80, biome='minecraft:badlands'),
    'leaves-jungle': dict(b=dict(m1='minecraft:jungle_leaves'), sky=15, y=90, surface=90, biome='minecraft:jungle'),
    'grass-jungle': dict(b=dict(m1='minecraft:grass_block'), sky=15, y=70, surface=70, biome='minecraft:jungle'),
    'mycelium': dict(b=dict(m1='minecraft:mycelium'), sky=15, y=70, surface=70, biome='minecraft:mushroom_fields'),
    'podzol': dict(b=dict(m1='minecraft:podzol'), sky=15, y=70, surface=70, biome='minecraft:old_growth_pine_taiga'),
    'mud-swamp': dict(b=dict(m1='minecraft:mud'), sky=15, y=62, surface=62, biome='minecraft:mangrove_swamp'),
    'stone-peaks': dict(b=dict(m1='minecraft:stone'), sky=15, y=160, surface=160, biome='minecraft:jagged_peaks'),
    'coarse-savanna': dict(b=dict(m1='minecraft:coarse_dirt'), sky=15, y=70, surface=70, biome='minecraft:savanna'),
    'slab-bottom': dict(b=dict(m1='minecraft:oak_slab[type=bottom]'), sky=0, y=40, surface=80),
    'slab-top': dict(b=dict(m1='minecraft:oak_slab[type=top]'), sky=0, y=40, surface=80),
    'glass': dict(b=dict(m1='minecraft:glass'), sky=0, y=40, surface=80),
    'glowstone': dict(b=dict(m1='minecraft:glowstone'), sky=0, light=15, y=40, surface=80),
    'fire': dict(b=dict(m1='minecraft:netherrack', p0='minecraft:fire'), sky=0, light=15, y=40, surface=80),
    'soul-fire': dict(b=dict(m1='minecraft:soul_soil', p0='minecraft:soul_fire'), sky=0, light=10, y=40, sea=32, dim='minecraft:the_nether', biome='minecraft:soul_sand_valley'),
    'wither-rose': dict(b=dict(m1='minecraft:grass_block', p0='minecraft:wither_rose'), sky=0, y=40, surface=80),
    'berry-bush': dict(b=dict(m1='minecraft:grass_block', p0='minecraft:sweet_berry_bush'), sky=15, y=70, surface=70, biome='minecraft:taiga'),
    'cactus': dict(b=dict(m1='minecraft:sand', p0='minecraft:cactus'), sky=0, y=70, surface=72, biome='minecraft:desert'),
    'powder-snow-feet': dict(b=dict(m1='minecraft:snow_block', p0='minecraft:powder_snow'), sky=15, darken=11, y=90, surface=91, biome='minecraft:snowy_slopes'),
    'campfire': dict(b=dict(m1='minecraft:stone', p0='minecraft:campfire[lit=true]'), sky=0, light=15, y=40, surface=80),
    'campfire-out': dict(b=dict(m1='minecraft:stone', p0='minecraft:campfire[lit=false]'), sky=0, y=40, surface=80),
    'rail': dict(b=dict(m1='minecraft:stone', p0='minecraft:rail'), sky=0, y=40, surface=80),
    'redstone-torch': dict(b=dict(m1='minecraft:stone', p0='minecraft:redstone_torch'), sky=0, light=7, y=40, surface=80),
    'tall-grass': dict(b=dict(m1='minecraft:grass_block', p0='minecraft:short_grass', p1='minecraft:air'), sky=15, darken=11, y=70, surface=70),
    'low-ceiling': dict(b=dict(m1='minecraft:stone', p1='minecraft:stone'), sky=0, y=40, surface=80),
    'swamp-surface': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=11, y=62, surface=63, biome='minecraft:swamp', slime=0.5),
    'swamp-full-moon': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=4, y=64, surface=65, biome='minecraft:swamp', slime=0.5),
    'swamp-new-moon': dict(b=dict(m1='minecraft:grass_block'), sky=15, darken=11, y=63, surface=64, biome='minecraft:swamp', slime=0.0),
    'bat-cave': dict(b=dict(m1='minecraft:stone'), sky=0, y=10, surface=80),
    'bat-surface': dict(b=dict(m1='minecraft:stone'), sky=0, y=79, surface=79),
}
REASONS = ['natural', 'natural', 'natural', 'chunk_generation', 'spawner', 'trial_spawner', 'reinforcement', 'patrol']


def corpus(types):
    rng = random.Random(0x5350574E)
    cases = []
    for t in types:
        n = t.split(':')[1]
        for name, s in SCENARIOS.items():
            for k in range(3):
                kv = {k2: v for k2, v in s.items() if k2 != 'b'}
                kv.setdefault('y', 64)
                for key, block in s['b'].items():
                    kv['at' + ('-' if key[0] == 'm' else '') + key[1:]] = block
                kv['reason'] = REASONS[k] if k == 0 else rng.choice(REASONS)
                kv['seed'] = rng.randrange(-2**47, 2**47)
                cases.append(f'{n}/{name}/{k} type={t} ' + ' '.join(f'{a}={b}' for a, b in kv.items()))
        # Slime chunks: deep caves at many chunk positions and world seeds.
        if n in ('slime', 'zombie'):
            for k in range(600 if n == 'slime' else 60):
                cases.append(f'{n}/slime-chunk/{k} type={t} at-1=minecraft:stone sky=0 y={rng.randrange(-60, 45)} '
                             f'x={rng.randrange(-4000, 4000)} z={rng.randrange(-4000, 4000)} wseed={rng.randrange(-2**63, 2**63)} '
                             f'seed={rng.randrange(-2**47, 2**47)} reason=natural surface=80')
        # Nearby players (silverfish and endermite refuse to spawn next to a targetable one).
        for k, player in enumerate(['1,0,1,survival', '1,0,1,creative', '4,0,4,survival', '3,1,3,adventure', '0,4.4,0,survival',
                                    '2,2,2,spectator']):
            cases.append(f'{n}/player/{k} type={t} at-1=minecraft:stone sky=0 y=20 surface=80 player={player} '
                         f'seed={rng.randrange(-2**47, 2**47)} reason={rng.choice(REASONS)}')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-spawn-rules-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    entity_types = json.loads((resources / 'entity-types.json').read_text())
    types = sorted(t for t, v in entity_types.items() if 'attributes' in v and t.split(':')[1] not in SKIPPED)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSpawnRulesOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/spawn_rules_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/spawn_rules.rs']
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    cases = corpus(types)
    inputs.write_text(''.join(line + '\n' for line in cases), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx2G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.horizonlab.parity.VanillaSpawnRulesOracle', inputs, build / 'server.txt'], 'server')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples/client_probe'
    run([binary, 'spawn_rules_fixture', resources, inputs, build / 'rust.txt'], 'rust')
    vanilla = (build / 'server.txt').read_text().splitlines()
    errors = [l for l in vanilla if ' error ' in l]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    rust = (build / 'rust.txt').read_text().splitlines()
    bad = [(a, b) for a, b in zip(vanilla, rust) if a != b]
    if bad or len(vanilla) != len(rust):
        report = '\n'.join(f'  vanilla {a}\n  rust    {b}' for a, b in bad[:40])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} cases differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        'types': len(types), 'scenarios': sorted(SCENARIOS), 'sources_sha256': tested,
        'scope': 'SpawnPlacements.isSpawnPositionOk (placement types, isValidSpawn, isValidEmptySpawnBlock with per-type '
                 'dangerous blocks) and checkSpawnRules (every registered predicate, peaceful, spawn reasons) with the '
                 'random draw count.'})
    print(f'{len(cases):,} spawn rule cases over {len(types)} entity types match vanilla')


if __name__ == '__main__':
    main()
