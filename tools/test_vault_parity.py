#!/usr/bin/env python3
"""Compare vaults (VaultBlockEntity.Server, VaultState and VaultBlock.useItemOn) with vanilla.

Java (VanillaNaturalSpawnOracle, mode=vault) loads a VaultBlockEntity from
each case's NBT on the oracle's stand-in ServerLevel, on a vault block in the
case's state, and runs `VaultBlockEntity.Server.tick` tick by tick with the
game time advancing; before a tick, players move and use stacks on the vault
through `VaultBlock.useItemOn`. Rust (natural_spawn_fixture, mode=vault) loads
`vault::Vault` from the same NBT and does the same. Both log, tick by tick,
the block's state changes, the level events, the sounds (with their pitch),
each ejected item (type, count, position and velocity bits), every shared data
update (display item and connected players), each use (accepted or not and
the stack left), and at the end the saved server and shared data, the saved
config's fields and the next draw of the level random, which pins every draw
in between (the display item rolls, the ejection velocities).

Configs are the default, the ominous vaults' and inline ones from the edge
values up (activation past deactivation, keys of two, no key); saved data has
rewarded players, a paused state, items left to eject and display items.
Players stand in and out of the activation and deactivation ranges in every
game mode, walk in and out, and use trial keys, ominous keys, too few keys and
other items, more than once and after being rewarded.
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
STATES = ['inactive', 'active', 'unlocking', 'ejecting']
TABLES = ['minecraft:chests/trial_chambers/reward', 'minecraft:chests/trial_chambers/reward_ominous',
          'minecraft:chests/trial_chambers/reward_rare', 'minecraft:chests/trial_chambers/reward_common',
          'minecraft:spawners/trial_chamber/consumables', 'minecraft:chests/simple_dungeon', 'minecraft:empty']
ITEMS = ['minecraft:trial_key', 'minecraft:ominous_trial_key', 'minecraft:diamond', 'minecraft:emerald',
         'minecraft:golden_apple', 'minecraft:arrow', 'minecraft:heavy_core']


def uuid(rng):
    return '[I;' + ','.join(str(rng.randrange(-2**31, 2**31)) for _ in range(4)) + ']'


def player_uuid(i):
    return f'[I;0,0,0,{i + 1}]'


def stack(rng):
    item = rng.choice(ITEMS)
    return f'{{id:"{item}",count:{rng.choice([1, 1, 1, 2, 3, 64])}}}'


def config(rng):
    r = rng.random()
    if r < 0.35:
        return None
    if r < 0.6:
        # The ominous vaults of the trial chambers.
        return '{key_item:{count:1,id:"minecraft:ominous_trial_key"},loot_table:"minecraft:chests/trial_chambers/reward_ominous"}'
    fields = []
    if rng.random() < 0.5:
        fields.append(f'loot_table:"{rng.choice(TABLES)}"')
    if rng.random() < 0.4:
        fields.append(f'activation_range:{rng.choice([0.0, 1.0, 2.5, 4.0, 6.0])}d')
    if rng.random() < 0.4:
        fields.append(f'deactivation_range:{rng.choice([0.5, 3.0, 4.5, 8.0])}d')
    if rng.random() < 0.85:
        key = rng.choice(['minecraft:trial_key', 'minecraft:trial_key', 'minecraft:ominous_trial_key', 'minecraft:diamond'])
        fields.append(f'key_item:{{count:{rng.choice([1, 1, 1, 2])},id:"{key}"}}')
    if rng.random() < 0.3:
        fields.append(f'override_loot_table_to_display:"{rng.choice(TABLES)}"')
    return '{' + ','.join(fields) + '}'


def vault_snbt(rng, now, count):
    fields = []
    c = config(rng)
    if c is not None:
        fields.append(f'config:{c}')
    server = []
    if count and rng.random() < 0.25:
        rewarded = rng.sample(range(count), rng.randrange(1, count + 1))
        server.append('rewarded_players:[' + ','.join(player_uuid(i) for i in rewarded) + ']')
    elif rng.random() < 0.1:
        server.append(f'rewarded_players:[{uuid(rng)}]')
    if rng.random() < 0.3:
        server.append(f'state_updating_resumes_at:{now + rng.choice([-20, 0, 5, 19, 40])}L')
    if rng.random() < 0.15:
        server.append('items_to_eject:[' + ','.join(stack(rng) for _ in range(rng.randrange(1, 4))) + ']')
    if rng.random() < 0.1:
        server.append(f'total_ejections_needed:{rng.choice([0, 1, 3])}')
    if server or rng.random() < 0.3:
        fields.append('server_data:{' + ','.join(server) + '}')
    shared = []
    if rng.random() < 0.25:
        shared.append(f'display_item:{stack(rng)}')
    if count and rng.random() < 0.2:
        shared.append('connected_players:[' + ','.join(player_uuid(i) for i in range(rng.randrange(1, count + 1))) + ']')
    if rng.random() < 0.1:
        shared.append(f'connected_particles_range:{rng.choice([2.0, 4.5, 9.0])}d')
    if shared:
        fields.append('shared_data:{' + ','.join(shared) + '}')
    return '{' + ','.join(fields) + '}'


def spot(rng, vpos):
    """A player position near the vault, around its activation (4) and deactivation (4.5) ranges."""
    d = rng.choice([0.5, 1.5, 2.5, 3.5, 4.2, 4.6, 5.5, 8.0])
    dx = rng.uniform(-d, d)
    dz = rng.choice([-1, 1]) * max(0.0, d * d - dx * dx) ** 0.5
    return vpos[0] + 0.5 + dx, vpos[1] + rng.choice([0, 0, 0, 1, -1, 3]), vpos[2] + 0.5 + dz


def corpus():
    rng = random.Random(0x5641554c)
    cases = []
    for k in range(3000):
        w = natural.World(rng)
        w.kv['dim'] = w.dim = 'minecraft:overworld'
        vpos = (rng.randrange(-8, 9), rng.randrange(-40, 20), rng.randrange(-8, 9))
        w.block(box(vpos[0] - 10, vpos[1] - 1, vpos[2] - 10, vpos[0] + 10, vpos[1] - 1, vpos[2] + 10), 'minecraft:tuff_bricks')
        count = rng.choice([0, 1, 1, 1, 2, 2, 3])
        modes = []
        for _ in range(count):
            mode = rng.choice(['survival'] * 6 + ['creative', 'adventure', 'spectator'])
            modes.append(mode)
            x, y, z = spot(rng, vpos)
            w.players.append(f'{x:.3f},{y:.3f},{z:.3f}/{mode}')
        natural.common(w, rng, rng.randrange(-2**47, 2**47))
        for key in ('unloaded', 'respawn', 'spawners', 'rules'):
            w.kv.pop(key, None)
        t0 = rng.choice([0, 1, 15, 19, 20, 1000, rng.randrange(0, 10**6), rng.randrange(10**9, 10**12)])
        ticks = rng.choice([1, 2, 20, 21, 60, 100, 200, 400])
        w.kv['spos'] = f'{vpos[0]},{vpos[1]},{vpos[2]}'
        w.kv['state'] = rng.choices(STATES, [5, 5, 1, 1])[0]
        w.kv['ominous'] = int(rng.random() < 0.3)
        w.kv['snbt'] = base64.b64encode(vault_snbt(rng, t0, count).encode()).decode()
        w.kv['t0'] = t0
        w.kv['ticks'] = ticks
        if count:
            uses = []
            for _ in range(rng.choice([0, 1, 1, 2, 3, 6])):
                item = rng.choice(['minecraft:trial_key'] * 4 + ['minecraft:ominous_trial_key'] * 2 + ITEMS)
                uses.append(f'{rng.randrange(ticks)}/{rng.randrange(count)}/{item}/{rng.choice([1, 1, 1, 2, 5])}')
            if uses:
                w.kv['use'] = ';'.join(uses)
            moves = []
            for _ in range(rng.choice([0, 0, 1, 2, 4])):
                x, y, z = spot(rng, vpos)
                moves.append(f'{rng.randrange(ticks)}/{rng.randrange(count)}/{x:.3f},{y:.3f},{z:.3f}')
            if moves:
                w.kv['move'] = ';'.join(moves)
        cases.append(f'vault/{k} mode=vault {w.tokens()}')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-vault-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client/26.3'
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaNaturalSpawnOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/natural_spawn_fixture.rs',
               lab.ROOT / 'crates/horizon-client-render/src/vault.rs',
               lab.ROOT / 'crates/horizon-client-render/src/chunk/block_entity.rs',
               lab.ROOT / 'crates/horizon-client-core/src/identifier.rs',
               lab.ROOT / 'crates/horizon-client-core/src/nbt_value.rs']
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
    stats = {'uses_on_active': count(' use ok '), 'refused': count('reject_rewarded_player'), 'wrong_keys': count('insert_item_fail'),
             'items': count(' item minecraft:'), 'states': count(' state '), 'updates': count(' update ')}
    for state in STATES:
        stats[f'reached_{state}'] = sum(f' state {state} ' in line for line in vanilla)
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': len(cases),
        **stats, 'sources_sha256': tested,
        'scope': 'VaultBlockEntity: load and save (VaultConfig with its validation, VaultServerData, VaultSharedData), '
                 'Server.tick (display item cycling from the loot table, VaultState.tickAndGetNext for every state with '
                 'connected players in range, ejection with spawnItem and its pitch, onTransition sounds and level '
                 'events, shared data updates), VaultBlock.useItemOn and Server.tryInsertKey (key checks, rewarded '
                 'players, fail sounds and their buffer, reward roll with the random sequence, unlocking, creative '
                 'players keep the key) with every level random draw.'})
    print(f'{len(cases):,} vault cases match vanilla (' + ', '.join(f'{v:,} {k}' for k, v in stats.items()) + ')')


if __name__ == '__main__':
    main()
