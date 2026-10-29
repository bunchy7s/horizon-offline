#!/usr/bin/env python3
"""Compare which sections of a chunk random-tick, and the positions drawn in them, with the pinned client.

Vanilla random-ticks a section while its count of randomly ticking blocks and fluids is above zero
(LevelChunkSection.isRandomlyTicking, counted by setBlockState and recalcBlockCounts), whatever its palette still lists.
Each case loads sections (every block state fills one, alone, so each state's own answer is compared; others mix a
ticking and a non-ticking state), ticks, sets and removes randomly ticking blocks (leaving them in the palette), and
ticks again. Vanilla runs ServerLevel.tickChunk's section loop on real sections with the real getBlockRandomPos
(VanillaRandomTickSectionsOracle); Horizon runs Behaviours::tick_chunk_random with the game's RandomTickSections
(examples/probe/random_tick_sections_probe). Both print every drawn position, whether its block and its fluid tick
randomly, and the final randValue.
"""
import itertools
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness

COUNT = 3000
SECTIONS = 24


def make_cases(build):
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    states = json.loads((resources / 'gameplay-facts.json').read_text())['states']
    ticking = [s['id'] for s in states if s['random_ticks']]
    still = [s['id'] for s in states if not s['random_ticks']]
    sweep = itertools.cycle(range(len(states)))
    rng = random.Random(0x5449434B)
    lines = [f'#resources {resources}']
    for _ in range(COUNT):
        ops = []
        loaded = rng.sample(range(SECTIONS), 14)
        for section in loaded[:12]:
            state = next(sweep)
            ops.append(f'f:{section}:{state}:{state}:1')
        mixed = loaded[12:]
        for section in mixed:
            ops.append(f'f:{section}:{rng.choice(still)}:{rng.choice(ticking)}:{rng.choice([1, 2, 7, 64, 4096])}')
        ops.append('t')
        placed = []
        for _ in range(rng.randrange(1, 6)):
            section, index = rng.randrange(SECTIONS), rng.randrange(4096)
            ops.append(f's:{section}:{index}:{rng.choice(ticking)}')
            placed.append((section, index))
        ops.append('t')
        # Take them away again, and the lone ticking block of the mixed sections:
        # their palettes keep the states.
        for section, index in placed + [(section, 4095) for section in mixed]:
            ops.append(f's:{section}:{index}:{rng.choice([0, 0, rng.choice(still)])}')
        ops.append('t')
        ops.append('t')
        lines.append(f'{rng.randrange(-2**31, 2**31)} {rng.choice([0, 1, 2, 3, 3, 3, 4])} {" ".join(ops)}')
    path = build / 'cases.txt'
    path.write_text('\n'.join(lines) + '\n')
    return path, COUNT


if __name__ == '__main__':
    harness.differential(
        'random-tick-sections-parity', 'VanillaRandomTickSectionsOracle', 'random_tick_sections_probe', make_cases,
        'Which sections of a chunk random-tick (block and fluid counts over loads, sets and removals, not the palette) '
        'and the positions tickChunk draws in them, with each position\'s block and fluid random-tick flags. The '
        'blocks\' random-tick behaviour itself is test_random_tick_parity\'s.',
        sources=[lab.ROOT / 'crates/horizon-client/src/random_tick_sections.rs',
                 lab.ROOT / 'crates/horizon-client-render/src/chunk.rs',
                 lab.ROOT / 'crates/horizon-core/src/paletted.rs',
                 lab.ROOT / 'crates/horizon-world/src/level/tick.rs',
                 lab.ROOT / 'crates/horizon-world/src/level/ticks.rs'],
        expected_lines=COUNT)
