#!/usr/bin/env python3
"""Compare Mob.finalizeSpawn (spawned mob gear, enchantments and state) with vanilla.

Java (VanillaFinalizeSpawnOracle) creates each case's mobs on a stand-in
ServerLevel and runs vanilla's finalizeSpawn with a seeded level random (what
finalizeSpawn reads through level.getRandom()) and seeded entity randoms; Rust
(finalize_spawn_fixture) runs horizon_client::mobs::finalize on the same case.
Cases cover every mob type, every spawn reason, peaceful to hard, game times,
inhabited times and moon phases (the local difficulty and its special
multiplier), packs sharing group data, biomes (variants, villager types,
sheep colours, rabbit and fox types), structures (black cats), roofs (camel
husk room) and chickens standing by (zombie jockeys).

Each entity created in a case, the mobs and the jockeys finalizeSpawn makes,
is compared: equipment with its enchantments, drop chances, CanPickUpLoot,
LeftHanded, persistence, age, health, attribute values, effects, variants,
the memories finalizeSpawn sets and the vehicle it rides; then the next draw
of every entity random and of the level random, which pins every draw in
between.
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

REASONS = ['NATURAL', 'CHUNK_GENERATION', 'SPAWNER', 'STRUCTURE', 'BREEDING', 'MOB_SUMMONED', 'JOCKEY', 'EVENT',
           'CONVERSION', 'REINFORCEMENT', 'TRIGGERED', 'BUCKET', 'SPAWN_ITEM_USE', 'COMMAND', 'DISPENSER', 'PATROL',
           'TRIAL_SPAWNER', 'LOAD', 'DIMENSION_TRAVEL']
# Mobs whose finalizeSpawn draws, by how often the corpus picks them.
GEARED = ['zombie', 'husk', 'drowned', 'zombie_villager', 'zombified_piglin', 'skeleton', 'stray', 'bogged', 'parched',
          'wither_skeleton', 'pillager', 'vindicator', 'piglin', 'piglin_brute', 'vex', 'fox', 'spider', 'illusioner',
          'evoker', 'ravager', 'witch']
OTHERS = ['pig', 'cow', 'mooshroom', 'sheep', 'chicken', 'rabbit', 'horse', 'donkey', 'mule', 'llama', 'trader_llama',
          'goat', 'panda', 'polar_bear', 'wolf', 'cat', 'ocelot', 'parrot', 'bee', 'turtle', 'armadillo', 'frog',
          'axolotl', 'squid', 'glow_squid', 'cod', 'salmon', 'tropical_fish', 'pufferfish', 'dolphin', 'bat', 'villager',
          'wandering_trader', 'iron_golem', 'snow_golem', 'creeper', 'cave_spider', 'enderman', 'endermite',
          'silverfish', 'slime', 'magma_cube', 'phantom', 'hoglin', 'zoglin', 'ghast', 'blaze', 'strider', 'guardian',
          'elder_guardian', 'shulker', 'camel', 'allay', 'breeze', 'sniffer', 'happy_ghast', 'warden', 'creaking',
          'camel_husk', 'nautilus', 'zombie_nautilus', 'sulfur_cube', 'zombie_horse', 'skeleton_horse', 'tadpole',
          'giant', 'wither']
BIOMES = ['minecraft:plains', 'minecraft:desert', 'minecraft:snowy_plains', 'minecraft:snowy_taiga', 'minecraft:taiga',
          'minecraft:jungle', 'minecraft:bamboo_jungle', 'minecraft:swamp', 'minecraft:mangrove_swamp',
          'minecraft:savanna', 'minecraft:badlands', 'minecraft:forest', 'minecraft:grove', 'minecraft:old_growth_pine_taiga',
          'minecraft:old_growth_spruce_taiga', 'minecraft:sparse_jungle', 'minecraft:windswept_savanna',
          'minecraft:mushroom_fields', 'minecraft:river', 'minecraft:frozen_river', 'minecraft:ocean',
          'minecraft:warm_ocean', 'minecraft:lukewarm_ocean', 'minecraft:cold_ocean', 'minecraft:frozen_ocean',
          'minecraft:ice_spikes', 'minecraft:cherry_grove', 'minecraft:pale_garden', 'minecraft:dripstone_caves',
          'minecraft:lush_caves', 'minecraft:deep_dark', 'minecraft:nether_wastes', 'minecraft:crimson_forest',
          'minecraft:soul_sand_valley', 'minecraft:the_end', 'minecraft:snowy_slopes', 'minecraft:frozen_peaks',
          'minecraft:wooded_badlands', 'minecraft:beach', 'minecraft:snowy_beach', 'minecraft:meadow',
          'minecraft:sulfur_caves']
STRUCTURES = ['minecraft:swamp_hut', 'minecraft:village_plains', 'minecraft:pillager_outpost', 'minecraft:fortress',
              'minecraft:trial_chambers']
FLOORS = ['minecraft:grass_block', 'minecraft:sand', 'minecraft:stone', 'minecraft:snow_block', 'minecraft:netherrack']


def dimension(biome):
    if biome in ('minecraft:nether_wastes', 'minecraft:crimson_forest', 'minecraft:soul_sand_valley'):
        return 'minecraft:the_nether'
    if biome == 'minecraft:the_end':
        return 'minecraft:the_end'
    return 'minecraft:overworld'


def corpus():
    rng = random.Random(0x46494E414C)
    cases = []
    for k in range(int(os.environ.get('FINALIZE_SPAWN_CASES', '24000'))):
        kind = rng.choice(GEARED) if rng.random() < 0.55 else rng.choice(OTHERS)
        reason = rng.choice(['NATURAL'] * 10 + ['CHUNK_GENERATION', 'SPAWNER', 'STRUCTURE', 'COMMAND', 'SPAWN_ITEM_USE',
                                                'TRIAL_SPAWNER', 'PATROL', 'EVENT', 'JOCKEY'] * 2 + REASONS)
        biome = rng.choice(BIOMES)
        kv = {
            'type': f'minecraft:{kind}',
            'reason': reason,
            'difficulty': rng.choice(['hard', 'hard', 'normal', 'normal', 'easy', 'peaceful']),
            'time': rng.choice([0, 0, 50000, 72000, 400000, 800000, 1512000, 3000000, rng.randrange(0, 4000000)]),
            'inhabited': rng.choice([0, 0, 0, 100000, 1000000, 3600000, 5000000, rng.randrange(0, 4000000)]),
            'phase': rng.randrange(8),
            'seed': rng.randrange(-2**63, 2**63),
            'eseed': rng.randrange(-2**47, 2**47),
            'pack': rng.choice([1, 1, 1, 2, 3, 4, 6]),
            'pos': f'{rng.randrange(-200, 200) + rng.choice([0.5, 0.25, 0.0]):.2f},{rng.randrange(-30, 120)},'
                   f'{rng.randrange(-200, 200) + rng.choice([0.5, 0.75, 0.0]):.2f}',
            'biome': biome,
            'dim': dimension(biome),
            'floor': rng.choice(FLOORS),
        }
        if rng.random() < 0.15:
            kv['structure'] = rng.choice(STRUCTURES)
        if rng.random() < 0.2:
            kv['roof'] = rng.choice([1, 2, 3])
        if rng.random() < 0.15:
            kv['chickens'] = rng.choice([1, 2])
        if rng.random() < 0.05:
            kv['monsters'] = 0
        cases.append(f'case/{k} ' + ' '.join(f'{a}={b}' for a, b in kv.items()))
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--java-only', action='store_true', help='run the oracle and stop')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-finalize-spawn-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaFinalizeSpawnOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/finalize_spawn_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/finalize.rs']
    tested = base.source_hashes([s for s in sources if s.exists()])
    inputs = build / 'inputs.txt'
    cases = corpus()
    inputs.write_text(''.join(line + '\n' for line in cases), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        path = build / (name + '.log')
        with path.open('w', encoding='utf-8') as log:
            try:
                base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
            except subprocess.CalledProcessError:
                log.flush()
                raise SystemExit(f'{name} failed:\n' + path.read_text(errors='replace')[-4000:])
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx3G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.horizonlab.parity.VanillaFinalizeSpawnOracle', inputs, build / 'server.txt'], 'server')
    vanilla = (build / 'server.txt').read_text().splitlines()
    errors = [l for l in vanilla if ' error ' in l]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    if args.java_only:
        print(f'{len(vanilla)} oracle lines')
        return
    halloween = vanilla[0].split()[1]
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples/client_probe'
    run([binary, 'finalize_spawn_fixture', resources, inputs, build / 'rust.txt', halloween], 'rust')
    rust = (build / 'rust.txt').read_text().splitlines()
    bad = [(a, b) for a, b in zip(vanilla, rust) if a != b]
    if bad or len(vanilla) != len(rust):
        report = '\n'.join(f'  vanilla {a[:2000]}\n  rust    {b[:2000]}' for a, b in bad[:12])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} lines differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged([s for s in sources if s.exists()], tested)
    entities = sum(line.count(' | ') for line in vanilla) // 2
    enchanted = sum(line.count('[minecraft:') for line in vanilla)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        'entities': entities, 'enchanted_items': enchanted, 'sources_sha256': tested,
        'scope': 'Mob.finalizeSpawn for every mob type and spawn reason: populateDefaultEquipmentSlots, '
                 'populateDefaultEquipmentEnchantments through the mob_spawn_equipment and pillager_spawn_crossbow '
                 'providers, drop chances, CanPickUpLoot, LeftHanded, the follow range spawn bonus, babies and '
                 'group data, jockeys (chicken, spider, strider, zombie horse, camel husk, zombie nautilus), '
                 'variants and sound variants, zombie attributes, spider effects, patrol leaders, villager types, '
                 'cube sizes, horse and llama attributes and the memories finalizeSpawn sets, with every draw of the '
                 'level random and of the entities\' own randoms.'})
    print(f'{len(cases):,} finalizeSpawn cases match vanilla ({entities:,} entities, {enchanted:,} enchanted items)')


if __name__ == '__main__':
    main()
