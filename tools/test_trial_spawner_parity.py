#!/usr/bin/env python3
"""Compare trial spawners (TrialSpawner.tickServer and TrialSpawnerState) with vanilla.

Java (VanillaNaturalSpawnOracle, mode=trial) loads a TrialSpawnerBlockEntity
from each case's NBT on the oracle's stand-in ServerLevel, on a trial spawner
block in the case's state, and runs `TrialSpawner.tickServer` tick by tick
with the game time advancing. Rust (natural_spawn_fixture, mode=trial) loads
`mobs::trial_spawner::TrialSpawner` from the same NBT and ticks it on the same
world. Both log, tick by tick, the block's state changes, the player
detection events, every spawn (type, exact position and yaw bits) with its
level and game events, the next spawn data updates, the shutter sounds, each
ejected item (type, count, position and velocity bits), and at the end the
saved state data and the next draw of the level random, which pins every draw
in between.

Rooms are tuff halls around the spawner with pillars, glass and bars between
it and the players (line of sight), players inside and outside the required
range in every game mode, configs from the trial chamber registry and inline
ones from the edge values up, saved state data (registered players, tracked
mobs that are gone, cooldowns, spawn data, a loot table being ejected), every
state of the block, every difficulty and both game rules. Every so many ticks
the case kills the mobs the spawner added, so waves finish, rewards are
ejected and cooldowns run out.

Some players carry Trial Omen or Bad Omen, which turn a spawner that sees them
ominous (Bad Omen becoming Trial Omen, the spawner's mobs discarded, the
ominous config starting over), and some spawners start ominous. Ominous
spawners drop ominous item spawners above a player or a mob (item, exact
position and delay compared); the item spawners themselves are not ticked.

finalizeSpawn runs with a fixed random in the oracle; its own outcomes are not
compared. Spawned mobs get UUIDs in spawn order on both sides, so the
spawner's HashSets iterate them alike.
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
REGISTRY = ['breeze', 'melee/husk', 'melee/spider', 'melee/zombie', 'ranged/poison_skeleton', 'ranged/skeleton',
            'ranged/stray', 'slow_ranged/poison_skeleton', 'slow_ranged/skeleton', 'slow_ranged/stray',
            'small_melee/baby_zombie', 'small_melee/cave_spider', 'small_melee/silverfish', 'small_melee/slime']
MOBS = ['zombie', 'husk', 'spider', 'cave_spider', 'skeleton', 'stray', 'bogged', 'breeze', 'slime', 'silverfish',
        'creeper', 'witch', 'pig', 'blaze']
STATES = ['inactive', 'waiting_for_players', 'active', 'waiting_for_reward_ejection', 'ejecting_reward', 'cooldown']
TABLES = ['minecraft:spawners/trial_chamber/consumables', 'minecraft:spawners/trial_chamber/key',
          'minecraft:chests/trial_chambers/reward', 'minecraft:spawners/ominous/trial_chamber/key']


def uuid(rng):
    return '[I;' + ','.join(str(rng.randrange(-2**31, 2**31)) for _ in range(4)) + ']'


def inline_config(rng):
    fields = []
    if rng.random() < 0.6:
        fields.append(f'spawn_range:{rng.choice([1, 2, 4, 4, 6, 10])}')
    for key, values in [('total_mobs', [0.0, 1.0, 2.0, 3.5, 6.0]), ('simultaneous_mobs', [0.0, 1.0, 2.0, 3.0]),
                        ('total_mobs_added_per_player', [0.0, 0.5, 1.0, 2.0]),
                        ('simultaneous_mobs_added_per_player', [0.0, 0.5, 1.0])]:
        if rng.random() < 0.6:
            fields.append(f'{key}:{rng.choice(values)}f')
    if rng.random() < 0.6:
        fields.append(f'ticks_between_spawn:{rng.choice([0, 1, 5, 20, 40, 160])}')
    if rng.random() < 0.9:
        entries = []
        for _ in range(rng.choice([1, 1, 2, 3])):
            mob = rng.choice(MOBS)
            entity = rng.choice([f'{{id:"minecraft:{mob}"}}', f'{{id:"{mob}"}}', f'{{id:"minecraft:{mob}",PersistenceRequired:1b}}',
                                 '{id:"minecraft:not_an_entity"}', '{}'] if rng.random() < 0.2 else [f'{{id:"minecraft:{mob}"}}'])
            entries.append(f'{{data:{{entity:{entity}}},weight:{rng.choice([1, 1, 2, 5, 0])}}}')
        fields.append('spawn_potentials:[' + ','.join(entries) + ']')
    if rng.random() < 0.4:
        entries = [f'{{data:"{rng.choice(TABLES)}",weight:{rng.choice([1, 1, 3, 0])}}}' for _ in range(rng.choice([1, 2, 3]))]
        fields.append('loot_tables_to_eject:[' + ','.join(entries) + ']')
    return '{' + ','.join(fields) + '}'


def trial_snbt(rng, now):
    fields = []
    r = rng.random()
    if r < 0.6:
        name = rng.choice(REGISTRY)
        fields.append(f'normal_config:"minecraft:trial_chamber/{name}/normal"')
        fields.append(f'ominous_config:"minecraft:trial_chamber/{name}/ominous"')
    elif r < 0.9:
        fields.append(f'normal_config:{inline_config(rng)}')
    elif r < 0.95:
        fields.append('normal_config:"minecraft:trial_chamber/not_a_config"')
    if rng.random() < 0.5:
        fields.append(f'target_cooldown_length:{rng.choice([0, 1, 20, 45, 100, 200, 36000])}')
    if rng.random() < 0.4:
        fields.append(f'required_player_range:{rng.choice([1, 4, 8, 14, 20, 0, 200])}')
    if rng.random() < 0.15:
        fields.append('registered_players:[' + ','.join(rng.choice(['[I;0,0,0,1]', uuid(rng)]) for _ in range(rng.randrange(1, 3))) + ']')
    if rng.random() < 0.15:
        fields.append('current_mobs:[' + ','.join(uuid(rng) for _ in range(rng.randrange(1, 3))) + ']')
    if rng.random() < 0.2:
        fields.append(f'cooldown_ends_at:{now + rng.choice([-50, 0, 30, 100, 300])}L')
    if rng.random() < 0.2:
        fields.append(f'next_mob_spawns_at:{now + rng.choice([-5, 0, 10, 60])}L')
    if rng.random() < 0.15:
        fields.append(f'total_mobs_spawned:{rng.choice([0, 1, 3, 6, 20])}')
    if rng.random() < 0.15:
        fields.append(f'spawn_data:{{entity:{{id:"minecraft:{rng.choice(MOBS)}"}}}}')
    if rng.random() < 0.08:
        fields.append(f'ejecting_loot_table:"{rng.choice(TABLES)}"')
    return '{' + ','.join(fields) + '}'


def room(rng):
    """A trial chamber hall around the spawner; returns (world, spawner position)."""
    w = natural.World(rng)
    w.dim = 'minecraft:overworld'
    w.kv['dim'] = w.dim
    w.kv['biome'] = rng.choice(['minecraft:plains', 'minecraft:dripstone_caves', 'minecraft:deep_dark', 'minecraft:lush_caves'])
    sx, sy, sz = rng.randrange(-8, 9), rng.randrange(-40, 20), rng.randrange(-8, 9)
    w.surface = sy
    half, height = rng.choice([4, 5, 6, 8]), rng.choice([4, 5, 7])
    w.block(box(sx - 24, sy - 6, sz - 24, sx + 24, sy + 12, sz + 24), rng.choice(['minecraft:tuff_bricks', 'minecraft:stone']))
    w.block(box(sx - half, sy - 1, sz - half, sx + half, sy - 1, sz + half), rng.choice(['minecraft:chiseled_tuff', 'minecraft:waxed_copper_block']))
    w.block(box(sx - half, sy, sz - half, sx + half, sy + height - 1, sz + half), 'minecraft:air')
    if rng.random() < 0.3:
        # A corridor out of the hall, past the range.
        w.block(box(sx + half, sy, sz - 1, sx + 22, sy + 2, sz + 1), 'minecraft:air')
    for _ in range(rng.choice([0, 0, 1, 3, 6])):
        px, pz = sx + rng.randrange(-half, half + 1), sz + rng.randrange(-half, half + 1)
        if (px, pz) != (sx, sz):
            w.block(box(px, sy, pz, px, sy + rng.randrange(0, height), pz),
                    rng.choice(['minecraft:tuff_bricks', 'minecraft:glass', 'minecraft:iron_bars', 'minecraft:oak_slab', 'minecraft:copper_grate']))
    w.light(box(sx - 24, sy - 2, sz - 24, sx + 24, sy + 12, sz + 24), 0, rng.choice([0, 0, 4, 8, 15]))
    return w, (sx, sy, sz), half, height


def players(w, rng, spos, half, height):
    for _ in range(rng.choice([0, 1, 1, 1, 2, 3])):
        r = rng.random()
        if r < 0.75:
            x, y, z = spos[0] + rng.uniform(-half, half + 1), spos[1] + rng.choice([0, 0, 0, 1, height - 2]), spos[2] + rng.uniform(-half, half + 1)
        else:
            x, y, z = spos[0] + rng.uniform(-20, 21), spos[1] + rng.choice([0, 0, -8, 10]), spos[2] + rng.uniform(-20, 21)
        mode = rng.choice(['survival'] * 7 + ['creative', 'adventure', 'spectator'])
        omen = ''
        if rng.random() < 0.3:
            omen = f'/{rng.choice(["bad_omen", "bad_omen", "trial_omen"])}:{rng.choice([0, 0, 1, 4])}'
        w.players.append(f'{x:.3f},{y:.3f},{z:.3f}/{mode}{omen}')


def corpus():
    rng = random.Random(0x54524941)
    cases = []
    for k in range(2500):
        w, spos, half, height = room(rng)
        players(w, rng, spos, half, height)
        natural.common(w, rng, rng.randrange(-2**47, 2**47))
        w.kv.pop('unloaded', None)
        w.kv.pop('respawn', None)
        if rng.random() < 0.05:
            w.kv['spawners'] = 0
        if rng.random() < 0.05:
            w.kv['rules'] = 0
        t0 = rng.choice([0, 1, 19, 1000, rng.randrange(0, 10**6), rng.randrange(10**9, 10**12)])
        w.kv['spos'] = f'{spos[0]},{spos[1]},{spos[2]}'
        w.kv['state'] = rng.choices(STATES, [2, 6, 4, 1, 1, 1])[0]
        if rng.random() < 0.15:
            w.kv['ominous'] = 1
        w.kv['snbt'] = base64.b64encode(trial_snbt(rng, t0).encode()).decode()
        w.kv['t0'] = t0
        w.kv['ticks'] = rng.choice([1, 2, 20, 41, 100, 200, 400, 600])
        w.kv['kill'] = rng.choice([0, 1, 7, 20, 60, 60])
        cases.append(f'trial/{k} mode=trial {w.tokens()}')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-trial-spawner-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNaturalSpawnOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/natural_spawn_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/mobs/trial_spawner.rs',
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
    count = lambda text: sum(line.count(text) for line in vanilla)
    stats = {'spawns': count(' spawn minecraft:'), 'items': count(' item minecraft:'), 'states': count(' state '),
             'detections': count(' levent 3013 '), 'rewards': count(' levent 3014 '), 'kills': count(' kill '),
             'became_ominous': count(' levent 3020 1 '), 'trial_omens': count(' effect add minecraft:trial_omen'),
             'item_spawners': count(' ispawner ')}
    for state in STATES:
        stats[f'reached_{state}'] = sum(f' state {state} ' in line for line in vanilla)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        **stats, 'sources_sha256': tested,
        'scope': 'TrialSpawner: load (state data, FullConfig with registry and inline configs), tickServer (mob '
                 'tracking), TrialSpawnerState.tickAndGetNext for every state (display entity, canSpawnInLevel, '
                 'tryDetectPlayers with line of sight, waves and their targets per player, spawnMob with its '
                 'collision, line of sight, spawn rules, yaw and obstruction checks, next spawn data, reward '
                 'ejection with the loot tables and spawnItem, shutter sounds, cooldown, becoming ominous with Trial '
                 'and Bad Omen, ominous item spawners with their items, positions and delays) with every level '
                 'random draw. finalizeSpawn runs with a fixed random in the oracle; its own outcomes are not compared.'})
    print(f'{len(cases):,} trial spawner cases match vanilla (' + ', '.join(f'{v:,} {k}' for k, v in stats.items()) + ')')


if __name__ == '__main__':
    main()
