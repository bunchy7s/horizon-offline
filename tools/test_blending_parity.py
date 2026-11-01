#!/usr/bin/env python3
"""Compare the terrain next to chunks an older version generated with vanilla.

Vanilla side: VanillaChunkGenerationOracle with HORIZON_OLD_CHUNKS: the chunks
of a second seed count as saved by an older version (blending data, no
worldgen heightmap), and the target chunks around them are generated with
`Blender.of(region)`: biomes, noise terrain, surface and carvers. Horizon side:
the chunk_dump probe does the same with `worldgen::blending`. Every block and
quart biome of the target chunks is compared.

usage: test_blending_parity.py [--filter REGEX] [--limit N]
"""
import argparse
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor

try:
    from . import lab, test_chunk_generation_parity as base
except ImportError:
    import lab
    import test_chunk_generation_parity as base

# (target seed, old seed, old chunks x0,z0,x1,z1, target chunks x0,z0,x1,z1, label)
CASES = [
    (12345, 777, '3,-1,4,1', '0,-2,2,2', 'a wall of old chunks to the east'),
    (12345, 777, '0,0,0,0', '-3,-3,3,3', 'one old chunk in the middle'),
    (-7046029254386353131, 4242, '-2,-2,-1,-1', '-5,-5,1,1', 'a block of four, another seed'),
    (987654321987, -1, '8,8,9,9', '4,4,12,12', 'far from the origin'),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', help='regex over the case labels')
    parser.add_argument('--limit', type=int, default=6, help='block differences shown per chunk')
    parser.add_argument('--phase', choices=['terrain', 'full'], default='terrain',
                        help='full: with the features, trees and the rest spilling into the old chunks')
    args = parser.parse_args()
    import re
    os.environ['HORIZON_PHASE'] = args.phase
    oracle = base.Oracle()
    binary = base.rust_binary()
    names = base.state_names()
    totals = [0, 0, 0, 0]
    compared = 0
    started = time.time()
    for seed, old_seed, old, rect, label in CASES:
        if args.filter and not re.search(args.filter, label):
            continue
        os.environ['HORIZON_OLD_CHUNKS'] = f'{old_seed}:{old}'
        tag = f'blending-{args.phase}-{seed}-{old_seed}-{old.replace(",", "_")}'
        vanilla_path, rust_path = base.build_dir() / f'vanilla-{tag}.txt', base.build_dir() / f'rust-{tag}.txt'
        with ThreadPoolExecutor(2) as pool:
            rust_run = pool.submit(base.run_rust, binary, rust_path, seed, 'overworld', [rect])
            oracle.run(vanilla_path, ['dump', seed, 'overworld', rect])
            stderr = rust_run.result()
        for line in stderr.splitlines()[:5]:
            print('  horizon:', line)
        vanilla, rust = base.parse(vanilla_path), base.parse(rust_path)
        ox0, oz0, ox1, oz1 = map(int, old.split(','))
        # The old chunks themselves are another seed's plain terrain: only what blends with them is compared.
        vanilla = {k: v for k, v in vanilla.items() if not (ox0 <= k[0] <= ox1 and oz0 <= k[1] <= oz1)}
        one = base.compare(vanilla, rust, names, f'{label}', limit=args.limit)
        totals = [a + b for a, b in zip(totals, one)]
        compared += len(vanilla)
        print(f'{label}: {len(vanilla)} chunks, differing chunks {one[0]} (blocks {one[1]}, biome cells {one[2]})', flush=True)
    print(f'Compared {compared} chunks in {time.time() - started:.0f}s; differences: {totals[0]} chunks, {totals[1]} blocks, {totals[2]} biome cells')
    if totals[0]:
        sys.exit(1)


if __name__ == '__main__':
    main()
