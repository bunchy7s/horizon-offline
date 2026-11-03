#!/usr/bin/env python3
"""Compare entity death drops (loot tables, equipment drops, experience) with vanilla.

Java (VanillaEntityDeathLootOracle) builds the real victim, killer and damage
source on a stand-in ServerLevel and runs LivingEntity.die's drop step; Rust
(entity_death_loot_fixture) runs horizon_client::mobs::drops on the same cases.
Every living entity type with a loot table is killed by a player with and
without looting, by nothing, on fire, with a smelting weapon, as a baby, with
luck and with worn equipment, plus each table's special conditions (sheep
colours, cube sizes, frog kills, skeleton and charged creeper kills, fireballs,
lightning, raid captains, vehicles and an enderman's carried block). Loot draws come from the world's random
sequences; several victims are killed in turn per case.
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

# Living entity types with an entity loot table that the corpus skips: players
# and stands are not mobs, the dragon dies over 200 ticks, and the giant is
# not a Horizon mob type.
SKIPPED = {'player', 'mannequin', 'armor_stand', 'ender_dragon', 'giant'}
# Types Horizon keeps body armour for in the head slot: their corpus wears
# body items instead of helmets.
BODY = {'horse', 'donkey', 'mule', 'skeleton_horse', 'zombie_horse', 'camel', 'camel_husk', 'llama', 'trader_llama',
        'happy_ghast', 'nautilus', 'zombie_nautilus', 'sulfur_cube', 'wolf'}
WEAPONS = ['minecraft:iron_sword', 'minecraft:diamond_sword;looting:1', 'minecraft:diamond_sword;looting:2',
           'minecraft:netherite_sword;looting:3', 'minecraft:golden_sword;fire_aspect:2', 'minecraft:diamond_axe;looting:3;fire_aspect:1']
ARMOUR = {'head': 'minecraft:iron_helmet', 'chest': 'minecraft:chainmail_chestplate', 'legs': 'minecraft:golden_leggings',
          'feet': 'minecraft:diamond_boots', 'mainhand': 'minecraft:iron_sword', 'offhand': 'minecraft:shield'}
# enderman_holdable blocks (and a few whose plain drop differs from the block).
CARRIED = ['grass_block', 'dirt', 'podzol', 'mycelium', 'mud', 'moss_block', 'pale_moss_block', 'sand', 'red_sand', 'gravel',
           'dandelion', 'poppy', 'brown_mushroom', 'red_mushroom', 'tnt', 'cactus', 'clay', 'pumpkin', 'carved_pumpkin', 'melon',
           'crimson_fungus', 'crimson_nylium', 'crimson_roots', 'warped_fungus', 'warped_nylium', 'warped_roots', 'cactus_flower']
DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple',
        'blue', 'brown', 'green', 'red', 'black']


def corpus(types):
    rng = random.Random(0x4C4F4F54)
    cases = []

    def add(name, **kv):
        kv.setdefault('seed', rng.randrange(-2**63, 2**63))
        kv.setdefault('rand', rng.randrange(-2**47, 2**47))
        cases.append(name + ''.join(f' {k}={v}' for k, v in kv.items()))
    for t in types:
        n = t.split(':')[1]
        add(f'{n}-player', type=t, killer='player', weapon='minecraft:iron_sword', kills=4)
        add(f'{n}-none', type=t, source='generic', kills=3)
        add(f'{n}-mob', type=t, killer='minecraft:spider', kills=2)
        for i, weapon in enumerate(WEAPONS[1:]):
            add(f'{n}-weapon{i}', type=t, killer='player', weapon=weapon, kills=4)
        add(f'{n}-fire', type=t, killer='player', fire=1, kills=3)
        add(f'{n}-fire-nokill', type=t, fire=1, source='fire', kills=2)
        add(f'{n}-baby', type=t, killer='player', baby=1, weapon='minecraft:iron_sword;looting:3', kills=3)
        add(f'{n}-luck', type=t, killer='player', luck=rng.choice([1, 2, 5]), kills=3)
        add(f'{n}-arrow', type=t, killer='player', source='arrow', weapon='minecraft:bow;looting:3', kills=2)
        head = 'body' if n in BODY else 'head'
        armour = dict(ARMOUR)
        if n in BODY:
            armour['body'] = armour.pop('head').replace('helmet', 'horse_armor')
        add(f'{n}-gear', type=t, killer='player', weapon='minecraft:iron_sword;looting:3', kills=6, **armour)
        add(f'{n}-gear-plain', type=t, killer='player', kills=6, mainhand='minecraft:stone_axe', chance_mainhand=0.5,
            **{head: 'minecraft:leather_helmet', f'chance_{head}': 2.0})
        add(f'{n}-gear-nokill', type=t, source='generic', kills=4, feet='minecraft:iron_boots', chance_feet=1.0,
            **{head: 'minecraft:carved_pumpkin', f'chance_{head}': 2.0})
        add(f'{n}-gear-mob', type=t, killer='minecraft:skeleton', source='arrow', kills=4, feet='minecraft:iron_boots', chance_feet=1.0,
            weapon='minecraft:bow;looting:3')
    for color in DYES:
        add(f'sheep-{color}', type='minecraft:sheep', color=color, killer='player', kills=3)
        add(f'sheep-{color}-sheared', type='minecraft:sheep', color=color, sheared=1, killer='player', kills=2)
    for cube in ['minecraft:slime', 'minecraft:magma_cube', 'minecraft:sulfur_cube']:
        for size in [1, 2, 3, 4]:
            add(f'{cube[10:]}-size{size}', type=cube, size=size, killer='player', weapon='minecraft:iron_sword;looting:2', kills=4)
    for frog in ['temperate', 'warm', 'cold']:
        for size in [1, 2, 4]:
            add(f'magma_cube-frog-{frog}-{size}', type='minecraft:magma_cube', size=size, killer='minecraft:frog', frog='minecraft:' + frog,
                pkill=rng.choice([0, 1]), kills=3)
    add('creeper-skeleton', type='minecraft:creeper', killer='minecraft:skeleton', source='arrow', pkill=1, kills=3)
    add('creeper-stray', type='minecraft:creeper', killer='minecraft:stray', source='arrow', kills=2)
    for victim in ['zombie', 'skeleton', 'creeper', 'wither_skeleton', 'piglin', 'zombie_villager', 'cow']:
        add(f'charged-{victim}', type='minecraft:' + victim, killer='minecraft:creeper', charged=1, source='explosion', kills=2)
        add(f'uncharged-{victim}', type='minecraft:' + victim, killer='minecraft:creeper', source='explosion', kills=2)
    add('ghast-fireball', type='minecraft:ghast', killer='player', source='fireball', kills=4)
    add('ghast-fireball-ghast', type='minecraft:ghast', killer='minecraft:ghast', source='fireball', kills=2)
    add('turtle-lightning', type='minecraft:turtle', source='lightning', kills=3)
    add('turtle-lightning-player', type='minecraft:turtle', source='lightning', pkill=1, kills=3)
    add('pillager-captain', type='minecraft:pillager', captain=1, leader=1, killer='player', kills=3)
    add('pillager-captain-none', type='minecraft:pillager', captain=1, leader=1, source='generic', kills=2)
    add('pillager-captain-looting', type='minecraft:pillager', captain=1, leader=1, killer='player',
        weapon='minecraft:iron_sword;looting:3', kills=4, chance_head=2.0)
    add('pillager-banner-only', type='minecraft:pillager', captain=1, killer='player', kills=3)
    add('pillager-leader-only', type='minecraft:pillager', leader=1, killer='player', kills=2)
    add('vindicator-captain', type='minecraft:vindicator', captain=1, leader=1, killer='player', kills=2)
    for animal in ['pig', 'strider', 'horse', 'camel', 'donkey']:
        add(f'{animal}-saddle', type='minecraft:' + animal, saddle='minecraft:saddle', chance_saddle=2.0, killer='player', kills=2)
        add(f'{animal}-saddle-default', type='minecraft:' + animal, saddle='minecraft:saddle', killer='player', kills=6)
    add('zombie-horse', type='minecraft:zombie', vehicle='minecraft:zombie_horse', killer='player', kills=4)
    add('zombie-baby-chicken', type='minecraft:zombie', baby=1, vehicle='minecraft:chicken', killer='player', kills=3)
    add('zombie-baby-chicken-none', type='minecraft:zombie', baby=1, vehicle='minecraft:chicken', source='generic', kills=2)
    add('husk-camel', type='minecraft:husk', vehicle='minecraft:camel_husk', killer='player', kills=3)
    # Enderman.dropCustomDeathLoot: the carried block's drops with a silk touch diamond axe.
    for block in CARRIED:
        add(f'enderman-carried-{block}', type='minecraft:enderman', carried='minecraft:' + block, killer='player',
            weapon='minecraft:iron_sword;looting:3', kills=3)
        add(f'enderman-carried-{block}-none', type='minecraft:enderman', carried='minecraft:' + block, source='generic', kills=2)
    for weapon in WEAPONS:
        add('wither-' + weapon.split(';')[-1].replace(':', ''), type='minecraft:wither', killer='player', weapon=weapon, kills=2)
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-entity-death-loot-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    tables = resources / 'data/minecraft/loot_table/entities'
    types = sorted('minecraft:' + p.stem for p in tables.glob('*.json') if p.stem not in SKIPPED)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityDeathLootOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/entity_death_loot_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/drops.rs', lab.ROOT / 'crates/horizon-client/src/mobs/enderman.rs',
               lab.ROOT / 'crates/horizon-world/src/loot.rs']
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
         'dev.horizonlab.parity.VanillaEntityDeathLootOracle', inputs, build / 'server.txt'], 'server')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples/client_probe'
    run([binary, 'entity_death_loot_fixture', resources, inputs, build / 'rust.txt'], 'rust')
    errors = [l for l in (build / 'server.txt').read_text().splitlines() if l.startswith('error ')]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    try:
        matched = base.compare(build / 'server.txt', build / 'rust.txt')
    except AssertionError as error:
        raise SystemExit(str(error) + explain(build)) from None
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        'types': len(types), 'skipped': sorted(SKIPPED), 'matched_output_lines': matched, 'sources_sha256': tested,
        'scope': 'LivingEntity.dropAllDeathLoot after the killer\'s killedEntity hook: entity loot tables under the world random '
                 'sequence, Mob.dropCustomDeathLoot equipment drops and durability, species death loot, experience orbs.'})
    print(f'{len(cases):,} death-drop cases over {len(types)} entity types match vanilla ({matched:,} lines)')


def explain(build):
    def sections(path):
        out, name = {}, None
        for line in path.read_text(encoding='utf-8', errors='replace').splitlines():
            if line.startswith('== '):
                name = line[3:]
                out[name] = []
            elif name is not None:
                out[name].append(line)
        return out
    left, right = sections(build / 'server.txt'), sections(build / 'rust.txt')
    bad = [n for n in left if left[n] != right.get(n)]
    report = f'\n{len(bad)} differing cases'
    for name in bad[:30]:
        a, b = left[name], right.get(name, [])
        diffs = [(x, y) for x, y in zip(a, b) if x != y][:3]
        if len(a) != len(b):
            diffs.append((f'<{len(a)} lines>', f'<{len(b)} lines>'))
        report += f'\n  {name}: {diffs}'
    return report


if __name__ == '__main__':
    main()
