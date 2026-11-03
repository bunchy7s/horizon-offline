#!/usr/bin/env python3
"""Armor-stand Frost Walker through real vanilla and Horizon servers.

Count frozen water after equipment changes, movement, landing and piston
pushes. Negative controls keep equipment in the wrong slot, change an
unrelated slot, and leave an equipped stand idle after thawing its disk.
"""
import json

try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

STAND = '@e[type=minecraft:armor_stand,limit=1]'
# Suppress neighbor updates while counting, so removing one edge does not
# melt other frosted blocks before the fill visits them.
THAW = 'fill -5 80 -5 7 80 5 minecraft:water replace minecraft:frosted_ice strict'


def cmd(command):
    return 'cmd Alice ' + command


def boots(level):
    return f'minecraft:leather_boots[minecraft:enchantments={{"minecraft:frost_walker":{level}}}]'


SCRIPT = [('setup', ['chat on', cmd('gamerule spawn_mobs false'),
                     cmd('gamerule random_tick_speed 0'), cmd('tp @s 4.5 83 -3.5')])]
EXPECTED = []


def setup(name, nbt='', height=81):
    SCRIPT.append((name + '-setup', [
        cmd('kill @e[type=minecraft:armor_stand]'),
        cmd('fill -6 79 -6 8 79 6 minecraft:stone'),
        cmd('fill -6 81 -6 8 87 6 minecraft:air'),
        cmd('fill -5 80 -5 7 80 5 minecraft:water'),
        cmd('setblock 0 80 0 minecraft:stone'),
        cmd(f'summon minecraft:armor_stand 0.5 {height} 0.5 {nbt}'),
    ]))


def check_step(name, count):
    SCRIPT.append((name + '-check', [cmd(THAW)]))
    EXPECTED.append(f'{name}-check ice {count}')


for level, count in [(1, 24), (2, 44)]:
    name = f'feet-{level}'
    setup(name)
    SCRIPT.append((name + '-equip', [cmd(f'item replace entity {STAND} armor.feet with {boots(level)}')]))
    check_step(name, count)
    check_step(name + '-idle', 0)
    SCRIPT.append((name + '-helmet', [cmd(f'item replace entity {STAND} armor.head with minecraft:iron_helmet')]))
    check_step(name + '-helmet', 0)
    SCRIPT.append((name + '-move', [cmd(f'tp {STAND} 1.5 81 0.5')]))
    check_step(name + '-move', count)

setup('hand')
SCRIPT.append(('hand-equip', [cmd(f'item replace entity {STAND} weapon.mainhand with {boots(2)}')]))
check_step('hand', 0)

setup('plain')
SCRIPT.append(('plain-equip', [cmd(f'item replace entity {STAND} armor.feet with minecraft:leather_boots')]))
check_step('plain', 0)

WORN = '{equipment:{feet:{id:"minecraft:leather_boots",count:1,components:{"minecraft:enchantments":{"minecraft:frost_walker":2}}}}}'
setup('spawn-equipped', WORN)
check_step('spawn-equipped', 0)
setup('landing', WORN, height=85)
check_step('landing', 44)

for name, components, count in [
    ('broken', 'minecraft:damage=65', 0),
    ('unbreakable', 'minecraft:damage=65,minecraft:unbreakable={}', 44),
    ('custom-durability', 'minecraft:max_damage=10,minecraft:damage=10', 0),
]:
    setup(name)
    SCRIPT.append((name + '-equip', [cmd(f'item replace entity {STAND} armor.feet with {boots(2)[:-1]},{components}]')]))
    check_step(name, count)

setup('piston')
SCRIPT.append(('piston-equip', [cmd(f'item replace entity {STAND} armor.feet with {boots(2)}')]))
check_step('piston-equipped', 44)
SCRIPT.append(('piston-place', [cmd('setblock -1 81 0 minecraft:piston[facing=east]')]))
SCRIPT.append(('piston-push', [cmd('setblock -2 81 0 minecraft:redstone_block')]))
check_step('piston-push', 0)


def shown(lines):
    out = []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) < 4 or not parts[1].endswith('-check') or parts[2] != 'chat':
            continue
        value = json.loads(parts[3])
        # Failed fills are wrapped in the command error's text component.
        if isinstance(value, dict) and 'extra' in value:
            value = next((v for v in value['extra'] if isinstance(v, dict)
                          and v.get('translate', '').startswith('commands.fill.')), value)
        key = value.get('translate', '') if isinstance(value, dict) else ''
        if key == 'commands.fill.success':
            out.append(f'{parts[1].removeprefix("step=")} ice {value["with"][0]}')
        elif key == 'commands.fill.failed':
            out.append(f'{parts[1].removeprefix("step=")} ice 0')
    return out


def check(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the intended cases: {native!r}'
    return None


def main():
    lines = net.one_player('armor-stand-frost-parity', SCRIPT, shown, check, __doc__, step_seconds=1.5)
    print(f'armor-stand Frost Walker: {len(lines)} results match vanilla')


if __name__ == '__main__':
    main()
