#!/usr/bin/env python3
"""Compare which chests join into a double chest against vanilla 26.3.

The Java oracle (VanillaChestPairOracle) puts a chest and another block next
to it, each with its block entity, and runs DoubleBlockCombiner's
combineWithNeigbour as ChestBlock.combine does: the double chest's halves
(the right one first) or a single chest. That pairing decides a chest's
menu (9x6 or 9x3), what a hopper fills and the lid both halves share. The
Rust example (chest_pair_fixture) answers with horizon_client::containers'
ChestHalf, which the game's menus and hoppers use. Compared for chests,
trapped chests and copper chests in every weathering state, waxed or not,
next to each of them and a barrel, with every chest type, two facings and
the block on either side.
"""
import argparse
import json
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

CHESTS = ['chest', 'trapped_chest', 'copper_chest', 'exposed_copper_chest', 'weathered_copper_chest',
          'oxidized_copper_chest', 'waxed_copper_chest', 'waxed_exposed_copper_chest',
          'waxed_weathered_copper_chest', 'waxed_oxidized_copper_chest']
TYPES = ['single', 'left', 'right']
# A chest facing north connects east (left) or west (right): ChestBlock.getConnectedDirection.
SIDES = [(1, 0), (-1, 0)]


def state(block, facing, kind):
    return f'minecraft:{block}[facing={facing},type={kind}]'


def cases():
    out = []
    for block in CHESTS:
        for kind in TYPES:
            own = state(block, 'north', kind)
            for at in SIDES:
                for other in CHESTS:
                    for facing in ('north', 'south'):
                        for other_kind in TYPES:
                            out.append({'state': own, 'other': state(other, facing, other_kind), 'at': at})
                out.append({'state': own, 'other': 'minecraft:barrel[facing=north]', 'at': at})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('chest-pair-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaChestPairOracle')
    all_cases = cases()
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in all_cases), encoding='utf-8')
    vanilla.run('VanillaChestPairOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'chest_pair_fixture', work / 'cases.jsonl', work / 'rust.txt')
    errors = sum(line == 'error' for line in (work / 'native.txt').read_text(encoding='utf-8').split('\n'))
    if errors:
        print(f'{errors} cases fail in vanilla')
    total = lib.compare(all_cases, work / 'native.txt', work / 'rust.txt',
                        lambda case, line: case['state'].split('[')[0] + ' next to ' + case['other'].split('[')[0],
                        json.dumps, args.limit)
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
