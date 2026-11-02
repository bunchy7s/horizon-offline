#!/usr/bin/env python3
"""Compare both fire starters' use sound with the pinned client: the real
FireChargeItem.useOn and FlintAndSteelItem.useOn on a campfire, a candle and a
stone block, on level randoms from many seeds, comparing the sound, its volume
and pitch bits, and the level random's next value (how many values it drew)."""
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


ITEMS = ['fire_charge', 'flint_and_steel']
TARGETS = ['campfire', 'candle', 'stone']


def make_cases(build):
    path = build / 'cases.txt'
    rng = random.Random(0x46495245)
    seeds = [0, 1, -1, 42, 2**63 - 1, -2**63] + [rng.randrange(-2**63, 2**63) for _ in range(20000)]
    cases = [f'{item} {target} {seed}' for seed in seeds for item in ITEMS for target in TARGETS]
    path.write_text(''.join(f'{c}\n' for c in cases))
    return path, len(cases)


def main():
    scope = ('Real FireChargeItem.useOn and FlintAndSteelItem.useOn on a ServerLevel stand-in holding a campfire, a '
             'candle or stone, with a seeded LegacyRandomSource: the sound event, volume, pitch ((nextFloat - nextFloat) '
             '* 0.2 + 1 for the charge, nextFloat * 0.4 + 0.8 for the steel) and the random afterwards. Both items light '
             'a campfire and a candle and place fire on stone; the block states they write, their game events and the '
             'item damage are not compared.')
    harness.differential(
        'rust-fire-starter-sound-parity', 'VanillaFireStarterSoundOracle', 'fire_starter_sound_probe', make_cases,
        scope, sources=[lab.ROOT / 'crates/horizon-items/src/items.rs'])


if __name__ == '__main__':
    main()
