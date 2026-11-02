#!/usr/bin/env python3
"""A fishing bobber against armor stands and boats on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival holding a fishing rod. LocatorBarClient (a player made of
vanilla's protocol code) casts the rod ahead and down at an armor stand, a marker armor stand and a boat three blocks
away (ServerboundUseItemPacket) and reels it in. `Projectile.canHitEntity` lets the bobber hook what
`Entity.canBeHitByProjectile` accepts (`isAlive() && isPickable()`; `ArmorStand.isPickable` is false for a marker), and
`FishingHook.retrieve` pulls the hooked entity 0.1 of the way to its owner (`pullEntity`). The entity each bobber hooks
(`DATA_HOOKED_ENTITY`) and the position the entity ends at are compared exactly.
"""
import re
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'fishing-hook-entities-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')
# Alice stands at the world spawn (0.5 -60 0.5) looking along +z; the target is three blocks ahead.
TARGETS = [
    ('stand', 'minecraft:armor_stand', 'minecraft:armor_stand 0.5 -60 3.0'),
    ('marker', 'minecraft:armor_stand', 'minecraft:armor_stand 0.5 -60 3.0 {Marker:1b}'),
    ('boat', 'minecraft:oak_boat', 'minecraft:oak_boat 0.5 -60 3.0'),
]
SCRIPT = [('setup', ['hooks on', 'chat on', 'cmd Alice give @s minecraft:fishing_rod'])]
for name, kind, summon in TARGETS:
    SCRIPT += [
        (f'{name}-summon', [f'cmd Alice kill @e[type={kind}]', f'cmd Alice summon {summon}']),
        (f'{name}-cast', ['use Alice 30']),
        (f'{name}-reel', ['use Alice 30']),
        (f'{name}-position', [f'cmd Alice data get entity @e[type={kind},limit=1] Pos']),
    ]


def shown(lines):
    """What each bobber hooked, and the position each target ended at (a boat's to a hundredth: its own
    ground friction is not under test)."""
    out = []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) < 3 or parts[1] == 'step=setup':
            continue
        if parts[2] == 'hooked':
            out.append(line)
        elif parts[2] == 'chat' and 'commands.data.entity.query' in line:
            position = [float(v) for v in re.findall(r'"text":"(-?[0-9.]+)","color":"gold"', line)]
            digits = 2 if parts[1].startswith('step=boat') else None
            out.append(f'{parts[0]} {parts[1]} position ' + ' '.join(str(round(v, digits) if digits else v) for v in position))
    return out


def check(vanilla, vanilla_all):
    hooked = {line.split()[3] for line in vanilla if line.split()[2] == 'hooked'}
    if not {'minecraft:armor_stand', 'minecraft:oak_boat'} <= hooked:
        return f'vanilla hooked only {sorted(hooked)}'
    if any(line.startswith('Alice step=marker-cast hooked minecraft:armor_stand') for line in vanilla):
        return 'vanilla hooked a marker armor stand'
    positions = {line.split()[1]: line.split()[4:] for line in vanilla if line.split()[2] == 'position'}
    if positions.get('step=stand-position', [''])[-1] == '3.0' or positions.get('step=boat-position', [''])[-1] == '3.0':
        return 'vanilla did not pull the hooked entity'
    return None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, 'A fishing bobber cast at an armor stand, a marker armor '
                               'stand and a boat on a dedicated server: the entity it hooks and where reeling in '
                               'leaves it.', step_seconds=3.0)
    print(f'fishing hook entities: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
