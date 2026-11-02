#!/usr/bin/env python3
"""Boat paddle metadata transitions through actual vanilla/Horizon servers.

Drive both paddles, stop one, stop both, resume one, then dismount without
sending a final paddle packet. Clients must receive the false transitions.
A boat with two players is driven by its first rider only: the second rider's
paddle packets leave the metadata alone.
"""
try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

SCRIPT = [('setup', ['boats on', 'cmd Alice gamerule spawn_mobs false'])]
EXPECTED = []
for kind in ['oak_boat', 'birch_chest_boat', 'bamboo_raft']:
    SCRIPT.append((kind + '-setup', [
        'cmd Alice kill @e[type=!player]',
        'cmd Alice fill -5 79 -5 5 79 5 minecraft:stone',
        'cmd Alice fill -5 80 -5 5 80 5 minecraft:water',
        'cmd Alice tp @s 0.5 81 0.5',
        f'cmd Alice summon minecraft:{kind} 0.5 81 0.5',
        f'cmd Alice ride @s mount @e[type=minecraft:{kind},limit=1]',
    ]))
    for step, action, changes in [
        ('both', 'paddle Alice true true', [(11, True), (12, True)]),
        ('left', 'paddle Alice true false', [(12, False)]),
        ('stop', 'paddle Alice false false', [(11, False)]),
        ('resume', 'paddle Alice false true', [(12, True)]),
        ('dismount', 'cmd Alice ride @s dismount', [(12, False)]),
    ]:
        label = f'{kind}-{step}'
        SCRIPT.append((label, [action]))
        EXPECTED += [f'{label} {index} {str(value).lower()}' for index, value in changes]


# Two players in one boat: `ServerGamePacketListenerImpl.handlePaddleBoat`
# applies a packet only when the sender is the boat's controlling passenger
# (`player.getControlledVehicle()`), and `Entity.addPassenger` seats the first
# player in front, so the second rider's paddle packet changes no metadata.
SCRIPT.append(('two-riders-setup', [
    'join Bob',
    'cmd Alice kill @e[type=!player]',
    'cmd Alice fill -5 79 -5 5 79 5 minecraft:stone',
    'cmd Alice fill -5 80 -5 5 80 5 minecraft:water',
    'cmd Alice tp @s 0.5 81 0.5',
    'cmd Alice summon minecraft:oak_boat 0.5 81 0.5',
    'cmd Alice ride @s mount @e[type=minecraft:oak_boat,limit=1]',
    'cmd Bob ride @s mount @e[type=minecraft:oak_boat,limit=1]',
]))
SCRIPT.append(('two-riders-both', ['paddle Alice true true']))
SCRIPT.append(('two-riders-second-left', ['paddle Bob true false']))
SCRIPT.append(('two-riders-second-stop', ['paddle Bob false false']))
SCRIPT.append(('two-riders-first-right', ['paddle Alice true false']))
# The first rider gets out, leaving the second alone in the boat: the seat it
# took over is the one the boat's paddles follow now.
SCRIPT.append(('two-riders-first-out', ['cmd Alice ride @s dismount']))
SCRIPT.append(('two-riders-second-alone', ['paddle Bob true true']))
# Each client that tracks the boat records the same metadata transition, and
# the transcript groups a player's lines together (Alice joined first).
EXPECTED += [f'{label} {index} {value}' for _ in ('Alice', 'Bob')
             for label, index, value in [('two-riders-both', 11, 'true'), ('two-riders-both', 12, 'true'),
                                         ('two-riders-first-right', 12, 'false'),
                                         ('two-riders-second-alone', 12, 'true')]]


def shown(lines):
    return [f'{p[1].removeprefix("step=")} {p[3]} {p[4]}'
            for line in lines if len(p := line.split()) == 5 and p[2] == 'paddle'
            and not p[1].endswith('-setup')]


def check(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the expected transitions: {native!r}'
    return None


def main():
    lines = net.one_player('boat-paddle-reset-parity', SCRIPT, shown, check, __doc__, step_seconds=0.75)
    print(f'boat paddle resets: {len(lines)} metadata transitions match vanilla')


if __name__ == '__main__':
    main()
