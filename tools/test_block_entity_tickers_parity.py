#!/usr/bin/env python3
"""Compare the order block entities tick in with vanilla's `Level.blockEntityTickers`.

Random scenarios load chunks with block entities in a save order (enough in
one chunk to make its block entity map grow), place, replace and remove block
entities, give jukeboxes a record and take it, and unload chunks, with ticks
in between. `VanillaBlockEntityTickerOracle` runs them on real `LevelChunk`s
of a server level and prints, after each tick, the positions of the live
tickers in list order; `examples/probe/block_entity_tickers_fixture` runs the same
on `horizon_client::block_entities::BlockEntities`. The block entities are
the kinds the game ticks in that list (hoppers, furnaces, beacons, spawners,
trial spawners, vaults, hives, crafters, brewing stands, jukebox), with chests and barrels,
which have no ticker, among them.
"""
import argparse
import os
import random
import subprocess
import sys
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
TICKING = ['hopper', 'furnace', 'blast_furnace', 'smoker', 'beacon', 'spawner', 'trial_spawner', 'vault',
           'beehive', 'bee_nest', 'crafter', 'brewing_stand', 'jukebox_record']
KINDS = TICKING + ['jukebox', 'chest', 'barrel']


def scenario(rng):
    """One scenario's lines: a few chunks near the origin and the ops on them."""
    lines = ['reset']
    loaded, blocks = set(), {}

    def free(cx, cz):
        while True:
            p = (cx * 16 + rng.randrange(16), rng.randrange(-64, 320), cz * 16 + rng.randrange(16))
            if p not in blocks:
                return p

    def load():
        options = [(x, z) for x in range(-2, 3) for z in range(-2, 3) if (x, z) not in loaded]
        if not options:
            return
        cx, cz = rng.choice(options)
        loaded.add((cx, cz))
        words = ['load', str(cx), str(cz)]
        for _ in range(rng.choice([0, 1, 3, 8, 20, 30, 60])):
            p, kind = free(cx, cz), rng.choice(KINDS)
            blocks[p] = kind
            words += [kind, *map(str, p)]
        lines.append(' '.join(words))
    for _ in range(rng.randrange(1, 4)):
        load()
    lines.append('tick')
    for _ in range(rng.randrange(10, 60)):
        op = rng.random()
        mine = sorted(blocks)
        if op < 0.25 and loaded:
            cx, cz = rng.choice(sorted(loaded))
            p, kind = free(cx, cz), rng.choice(KINDS)
            blocks[p] = kind
            lines.append(f'place {kind} {p[0]} {p[1]} {p[2]}')
        elif op < 0.4 and mine:
            p = rng.choice(mine)
            lines.append(f'replace {p[0]} {p[1]} {p[2]}')
        elif op < 0.6 and mine:
            p = rng.choice(mine)
            del blocks[p]
            lines.append(f'remove {p[0]} {p[1]} {p[2]}')
        elif op < 0.72:
            jukeboxes = [p for p in mine if blocks[p].startswith('jukebox')]
            if jukeboxes:
                p = rng.choice(jukeboxes)
                record = blocks[p] == 'jukebox'
                blocks[p] = 'jukebox_record' if record else 'jukebox'
                lines.append(f'record {p[0]} {p[1]} {p[2]} {int(record)}')
        elif op < 0.78 and loaded:
            cx, cz = rng.choice(sorted(loaded))
            loaded.discard((cx, cz))
            for p in [p for p in blocks if (p[0] >> 4, p[2] >> 4) == (cx, cz)]:
                del blocks[p]
            lines.append(f'unload {cx} {cz}')
        elif op < 0.85:
            load()
        else:
            lines.append('tick')
    lines.append('tick')
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--scenarios', type=int, default=3000)
    parser.add_argument('--seed', type=int, default=0x7469636b)
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-block-entity-tickers-parity'
    build.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)
    lines = [line for _ in range(args.scenarios) for line in scenario(rng)]
    inputs = build / 'inputs.txt'
    inputs.write_text('\n'.join(lines) + '\n')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies,
         '-d', classes, *(PARITY / f'{n}.java' for n in ('ParityData', 'VanillaBlockEntityTickerOracle'))],
        'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies,
         'dev.horizonlab.parity.VanillaBlockEntityTickerOracle', inputs, build / 'vanilla.txt'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example',
         'client_probe', '--', 'block_entity_tickers_fixture', inputs, build / 'rust.txt'], 'rust')
    matched = base.compare(build / 'vanilla.txt', build / 'rust.txt')
    ticks = sum(1 for line in lines if line == 'tick')
    listed = sum(len(line.split()) - 1 for line in (build / 'vanilla.txt').read_text().splitlines())
    print(f'block entity tickers: {args.scenarios} scenarios, {matched} of {ticks} ticks match vanilla '
          f'({listed} tickers listed)')


if __name__ == '__main__':
    main()
