#!/usr/bin/env python3
"""Compare creative marker selection, ambient sampling and the campfire's own
particles with the pinned ClientLevel."""
import itertools
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab
    import oracle_harness as harness

CAMPFIRES = ['campfire', 'soul_campfire']
FACINGS = ['north', 'south', 'west', 'east']
# Which of the four cooking slots hold an item: the smoke above them and the
# draw each occupied slot makes.
SLOTS = [[0, 0, 0, 0], [1, 0, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1],
         [1, 1, 0, 0], [1, 0, 0, 1], [0, 1, 1, 0], [1, 1, 1, 1]]
SEEDS = [0, 1057, 7]
MARKER_ITEMS = ['air', 'barrier', 'light', 'stone', 'structure_void', 'apple']


def state(name, lit, signal, facing):
    return 'minecraft:%s[lit=%s,signal_fire=%s,facing=%s]' % (
        name, str(lit).lower(), str(signal).lower(), facing)


def marker_cases():
    return [dict(mode=mode, main=main, off=off, count=count)
            for mode, main, off, count in itertools.product(
                ['survival', 'creative', 'adventure', 'spectator'], MARKER_ITEMS, MARKER_ITEMS,
                [0, 1, 64])]


def campfire_cases(rng):
    """CampfireBlockEntity.particleTick: the smoke column over the fire and the
    puff above each occupied cooking slot."""
    for name, lit, signal, facing, slots, seed in itertools.product(
            CAMPFIRES, (True, False), (False, True), FACINGS, SLOTS, SEEDS):
        yield dict(kind='campfire', state=state(name, lit, signal, facing), slots=slots,
                   seed=seed, ticks=6)
    for _ in range(240):
        yield dict(kind='campfire',
                   state=state(rng.choice(CAMPFIRES), rng.random() < 0.8, rng.random() < 0.5,
                               rng.choice(FACINGS)),
                   slots=[rng.choice([0, 0, 1]) for _ in range(4)],
                   seed=rng.randrange(-2 ** 63, 2 ** 63), ticks=rng.randrange(1, 9))


def puff_cases(rng):
    """CampfireBlock.makeParticles: the cosy or signal smoke over the fire, and
    the plain smoke a dousing adds with it."""
    for signal, smoking, seed, puffs in itertools.product(
            (False, True), (False, True), SEEDS, (1, 20)):
        yield dict(kind='make', signal=signal, smoking=smoking, seed=seed, puffs=puffs)
    for _ in range(120):
        yield dict(kind='make', signal=rng.random() < 0.5, smoking=rng.random() < 0.5,
                   seed=rng.randrange(-2 ** 63, 2 ** 63), puffs=rng.randrange(1, 24))


def make_input(build):
    rng = random.Random(0x43414d50)
    cases = marker_cases() + list(campfire_cases(rng)) + list(puff_cases(rng))
    path = build / 'cases.jsonl'
    path.write_text(''.join(json.dumps(case) + '\n' for case in cases))
    return path, len(cases)


def main():
    harness.differential(
        'marker-particles-parity', 'VanillaMarkerParticlesOracle', 'marker_particles_probe',
        make_input, 'Creative main-hand marker selection and barrier marker positions, velocities and random draws, '
        'and the campfire: the real CampfireBlockEntity.particleTick for both campfire blocks, lit and unlit, signal '
        'fire on and off, all four facings and every one of the four cooking slots held or empty, ticked six times '
        'from a seeded level random, and the real CampfireBlock.makeParticles for the cosy and the signal smoke with '
        'and without the plain smoke puff a douse adds. Every particle the level is asked for is compared by its '
        'type and exact position bits, in the order it was asked for, and the level random continues to be compared '
        'through the draw after the last one. The lit filter the client tick loop applies before it ticks a campfire '
        'is supplied to the oracle; the always-visible flag, the particle-set limiter, the velocities (each particle '
        'draws its own random for them) and the particles past their creation are outside this comparison.',
        sources=[lab.ROOT / 'tools/test_marker_particles_parity.py',
                 lab.ROOT / 'crates/horizon-client/src/animate_tick.rs'], expected_lines=25578)


if __name__ == '__main__':
    main()
