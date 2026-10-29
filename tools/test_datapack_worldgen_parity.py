#!/usr/bin/env python3
"""Compare chunks generated with a world's data packs block-for-block with vanilla.

The packs of tools/fixtures/datapacks (Horizon's own, MIT) override and add
noises, density functions, the overworld noise settings, a placed feature and
a biome tag. Vanilla side: VanillaChunkGenerationOracle with HORIZON_DATAPACKS,
which selects every pack of the folder after vanilla, as a world does, and
loads the registries through vanilla's own pack repository. Horizon side:
chunk_dump with the same variable, reading the packs through
worldgen::overrides into GeneratorData::load_with. The same terrain sites are
compared for every block state, quart biome and block entity, first after
TERRAIN, then with decoration. Horizon without the packs must differ from
vanilla with them, so the packs are known to take effect.

usage: test_datapack_worldgen_parity.py [--seed N] [--sites K] [--phase terrain|full]
"""
import argparse
import os
import random
import sys
from pathlib import Path

try:
    from . import lab, test_chunk_generation_parity as chunks
except ImportError:
    import lab
    import test_chunk_generation_parity as chunks

# HORIZON_PARITY_DATAPACKS: another folder of packs to compare.
DATAPACKS = Path(os.environ.get('HORIZON_PARITY_DATAPACKS', lab.ROOT / 'tools/fixtures/datapacks'))
SEEDS = [12345, 4242]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, action='append')
    parser.add_argument('--sites', type=int, default=3, help='terrain sites per seed besides 0,0')
    parser.add_argument('--phase', choices=['terrain', 'full'], action='append')
    args = parser.parse_args()
    oracle = chunks.Oracle()
    binary = chunks.rust_binary()
    names = chunks.state_names()
    build = chunks.build_dir() / 'datapacks'
    build.mkdir(exist_ok=True)
    differing = 0
    pack_effect = 0
    for phase in args.phase or ['terrain', 'full']:
        os.environ['HORIZON_PHASE'] = phase
        for seed in args.seed or SEEDS:
            rng = random.Random(seed)
            sites = [(0, 0)] + [(rng.randint(-300, 300), rng.randint(-300, 300)) for _ in range(args.sites)]
            rects = [f'{x},{z},{x},{z}' for x, z in sites]
            tag = f'{seed}-{phase}'
            vanilla_path, rust_path, plain_path = (build / f'{side}-{tag}.txt' for side in ('vanilla', 'rust', 'plain'))
            os.environ['HORIZON_DATAPACKS'] = str(DATAPACKS)
            oracle.run(vanilla_path, ['dump', seed, 'overworld', *rects])
            stderr = chunks.run_rust(binary, rust_path, seed, 'overworld', rects)
            del os.environ['HORIZON_DATAPACKS']
            for line in stderr.splitlines():
                if 'data pack' in line:
                    print('  horizon:', line)
            vanilla, rust = chunks.parse(vanilla_path), chunks.parse(rust_path)
            counts = chunks.compare(vanilla, rust, names, f'{tag} packs')
            differing += counts[0]
            print(f'{tag}: {len(vanilla)} chunks with the packs, differing chunks {counts[0]} '
                  f'(blocks {counts[1]}, biome cells {counts[2]}, block entities {counts[3]})', flush=True)
            chunks.run_rust(binary, plain_path, seed, 'overworld', rects)
            plain = chunks.compare(vanilla, chunks.parse(plain_path), names, f'{tag} without packs', quiet=True)
            pack_effect += plain[0]
            print(f'{tag}: without the packs Horizon differs from them in {plain[0]} of {len(vanilla)} chunks ({plain[1]} blocks)')
    if not pack_effect:
        sys.exit('the packs changed nothing: the harness would not see them')
    if differing:
        sys.exit(f'{differing} chunks differ from vanilla with the packs')
    print('Data pack worldgen parity passed')


if __name__ == '__main__':
    main()
