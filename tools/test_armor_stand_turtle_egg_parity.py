#!/usr/bin/env python3
"""Armor stands standing on turtle eggs through real vanilla and Horizon servers.

An armor stand is a LivingEntity, so `TurtleEggBlock.stepOn` cracks an egg
under it one tick in 100 while mob griefing is on (`canDestroyEgg`). After
1500 ticks a stack of four is gone with near certainty (fewer than four
cracks: about 2 in 10000). Controls: a marker stand never lands, so it never
steps; an egg with no stand stays; a stand whose centre is past the egg's
edge still stands on it (`mainSupportingBlockPos`); with mob griefing off
the eggs stay.
"""
import json

try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

STEP_SECONDS = 5.0
WAIT_STEPS = 15  # 15 x 100 ticks


def cmd(command):
    return 'cmd Alice ' + command


SCRIPT = [('setup', ['chat on', cmd('gamerule spawn_mobs false'),
                     cmd('gamerule random_tick_speed 0'), cmd('tp @s 4.5 83 -4.5'),
                     cmd('fill -2 79 -2 15 79 2 minecraft:stone'),
                     cmd('fill -2 80 -2 15 84 2 minecraft:air')])]
EXPECTED = []


def eggs(x):
    return cmd(f'setblock {x} 80 0 minecraft:turtle_egg[eggs=4]')


def wait(name):
    for i in range(WAIT_STEPS):
        SCRIPT.append((f'{name}-wait-{i}', []))


def check(name, x, block, result):
    SCRIPT.append((f'{name}-check', [cmd(f'execute if block {x} 80 0 {block}')]))
    EXPECTED.append(f'{name}-check {result}')


SCRIPT.append(('griefing-setup', [
    cmd('gamerule mob_griefing true'),
    eggs(0), eggs(3), eggs(6), eggs(9), eggs(12),
    cmd('summon minecraft:armor_stand 0.5 80.5 0.5'),
    cmd('summon minecraft:armor_stand 3.5 80.5 0.5 {Small:1b}'),
    cmd('summon minecraft:armor_stand 6.5 80.5 0.5 {Marker:1b}'),
    # Centre past the egg's edge, box still on the eggs (getOnPos takes the
    # supporting block's column, not the air under the centre).
    cmd('summon minecraft:armor_stand 13.05 80.5 0.5'),
]))
wait('griefing')
check('stand', 0, 'minecraft:turtle_egg', 'fail')
check('small-stand', 3, 'minecraft:turtle_egg', 'fail')
check('marker-stand', 6, 'minecraft:turtle_egg[eggs=4]', 'pass')
# Three cracks, then the single egg's narrower shape no longer reaches under
# the stand: it drops to the stone and the last egg stays.
check('edge-stand', 12, 'minecraft:turtle_egg[eggs=1]', 'pass')
check('no-stand', 9, 'minecraft:turtle_egg[eggs=4]', 'pass')

SCRIPT.append(('no-griefing-setup', [
    cmd('kill @e[type=minecraft:armor_stand]'),
    cmd('gamerule mob_griefing false'),
    eggs(0),
    cmd('summon minecraft:armor_stand 0.5 80.5 0.5'),
]))
wait('no-griefing')
check('no-griefing-stand', 0, 'minecraft:turtle_egg[eggs=4]', 'pass')


def shown(lines):
    out = []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) < 4 or not parts[1].endswith('-check') or parts[2] != 'chat':
            continue
        value = json.loads(parts[3])
        # A failed test comes wrapped in the command error's text component.
        if isinstance(value, dict) and 'extra' in value:
            value = next((v for v in value['extra'] if isinstance(v, dict)
                          and v.get('translate', '').startswith('commands.execute.conditional.')), value)
        key = value.get('translate', '') if isinstance(value, dict) else ''
        if key.startswith('commands.execute.conditional.'):
            out.append(f'{parts[1].removeprefix("step=")} {key.rsplit(".", 1)[1]}')
    return out


def check_vanilla(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the intended cases: {native!r}'
    return None


def main():
    lines = net.one_player('armor-stand-turtle-egg-parity', SCRIPT, shown, check_vanilla, __doc__,
                           step_seconds=STEP_SECONDS)
    print(f'armor-stand turtle eggs: {len(lines)} results match vanilla')


if __name__ == '__main__':
    main()
