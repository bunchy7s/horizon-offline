#!/usr/bin/env python3
"""Submerged boats eject their player on real vanilla and Horizon servers.

A vanilla-codec client deliberately sends no vehicle movement: the server must
count immersion even for a boat whose player normally moves it. Surface water
and a brief immersion followed by surfacing must not eject the player; a long
continuous immersion must. Includes a chest boat, a raft and flowing water.
"""
try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

SCRIPT = [('setup', ['cmd Alice gamerule spawn_mobs false'])]
EXPECTED = []
PROBES = []


def probe(label, mounted, player='Alice', actions=()):
    """A step running `actions`, at whose end `player` must be (or no longer be) a passenger."""
    PROBES.append((label, player))
    EXPECTED.append(f'{label} mounted={str(mounted).lower()}')
    SCRIPT.append((label, list(actions)))


for kind, water in [('oak_boat', 'water'), ('birch_chest_boat', 'water'),
                    ('bamboo_raft', 'water'), ('oak_boat', 'water[level=8]')]:
    case = kind + ('-flowing' if '[' in water else '-source')
    SCRIPT.append((case + '-setup', [
        'cmd Alice kill @e[type=!player]',
        'cmd Alice fill -4 79 -4 4 84 4 minecraft:air',
        'cmd Alice fill -4 79 -4 4 85 4 minecraft:stone hollow',
        'cmd Alice fill -4 85 -4 4 85 4 minecraft:air',
        'cmd Alice fill -3 80 -3 3 80 3 minecraft:water',
        'cmd Alice tp @s 0.5 82 0.5',
        f'cmd Alice summon minecraft:{kind} 0.5 80.6 0.5',
        f'cmd Alice ride @s mount @e[type=minecraft:{kind},limit=1]',
    ]))
    for n in range(3):
        SCRIPT.append((f'{case}-surface-wait-{n}', []))
    probe(case + '-surface', True)
    # Each dip is one step (20 ticks, plus the ticks the client's syncs take) observed at its own end, far from
    # the 60 that eject: observed a step later, a loaded machine's syncs pushed a short dip past 60 (2026-11-02).
    probe(case + '-short', True, actions=[f'cmd Alice fill -3 81 -3 3 84 3 minecraft:{water}'])
    SCRIPT.append((case + '-surface-reset', ['cmd Alice fill -3 81 -3 3 84 3 minecraft:air']))
    probe(case + '-reset', True, actions=[f'cmd Alice fill -3 81 -3 3 84 3 minecraft:{water}'])
    # 20 + 40 + 20 ticks under water by the last observation: past 60 however fast the steps go.
    for n in range(2):
        SCRIPT.append((f'{case}-underwater-wait-{n}', []))
    probe(case + '-ejected', False)


# `ejectPassengers` ejects both controlling and non-controlling players.
SCRIPT.append(('two-players-setup', [
    'join Bob',
    'cmd Alice kill @e[type=!player]',
    'cmd Alice fill -3 80 -3 3 84 3 minecraft:air',
    'cmd Alice fill -3 80 -3 3 80 3 minecraft:water',
    'cmd Alice summon minecraft:oak_boat 0.5 80.6 0.5',
    'cmd Alice ride @s mount @e[type=minecraft:oak_boat,limit=1]',
    'cmd Bob ride @s mount @e[type=minecraft:oak_boat,limit=1]',
]))
probe('two-players-first-mounted', True)
probe('two-players-second-mounted', True, 'Bob')
SCRIPT.append(('two-players-submerge', ['cmd Alice fill -3 81 -3 3 84 3 minecraft:water']))
for n in range(4):
    SCRIPT.append((f'two-players-underwater-wait-{n}', []))
probe('two-players-first-ejected', False)
probe('two-players-second-ejected', False, 'Bob')


def shown(lines):
    # Each client's transcript is grouped under its name, so replay that
    # player's real passenger packets up to each observation step separately.
    order = {label: i for i, (label, _) in enumerate(SCRIPT)}
    observations = []
    for label, player in PROBES:
        mounted = None
        for line in lines:
            fields = line.split()
            if (len(fields) == 4 and fields[0] == player and fields[2] == 'passenger'
                    and order.get(fields[1].removeprefix('step='), -1) <= order[label]):
                mounted = fields[3] == 'true'
        value = str(mounted).lower() if mounted is not None else 'missing mount'
        observations.append(f'{label} mounted={value}')
    return observations


def check(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the expected immersion outcomes: {native!r}'
    return None


def main():
    lines = net.one_player('boat-submerged-parity', SCRIPT, shown, check, __doc__, step_seconds=1.0)
    print(f'boat submersion: {len(lines)} passenger observations match vanilla')


if __name__ == '__main__':
    main()
