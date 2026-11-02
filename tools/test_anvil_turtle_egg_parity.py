#!/usr/bin/env python3
"""Anvils landing on turtle eggs through real vanilla and Horizon servers.

An anvil cannot replace turtle eggs, so `FallingBlockEntity.tick` discards it
where it came to rest, on top of the eggs: `Fallable.onBrokenAfterFall`
(AnvilBlock: level event 1029) and `spawnAtLocation`, an item at the entity's
own position with `new ItemEntity`'s motion (at most 0.1 sideways, 0.2 up).
The item is added on top of the eggs, right under the anvil's centre, and
about half the time falls off them. Horizon dropped it like a mined block,
somewhere inside the eggs' cell, where it was pushed out sideways tick after
tick and ended up to 2.5 blocks away (median 1.5, vanilla's 0.6). Each
item's position when it is added is compared; its motion is the item's own
random. Horizon tells clients about an entity added after `tick_lan` (falling
blocks tick later) a tick late, after its first move: 0.2 up less 0.04 of
gravity, and at most 0.1 sideways. An item added at the anvil's centre on
top of the eggs, either then or one tick on, shows as `at-anvil`.
The eggs stay: a falling block is no LivingEntity
(`TurtleEggBlock.canDestroyEgg`). Controls: an anvil that drops no item
breaks silently; one landing on stone is placed (level event 1031).
"""
import json

try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

STEP_SECONDS = 2.0
EGGS = [(0, 1), (4, 2), (8, 3), (12, 4), (16, 1), (20, 2), (24, 3), (28, 4)]
NO_DROP = 32
STONE = 36


def cmd(command):
    return 'cmd Alice ' + command


def anvil(x, extra=''):
    return cmd(f'summon minecraft:falling_block {x}.5 86 0.5 {{BlockState:"minecraft:anvil",Time:1{extra}}}')


SCRIPT = [('setup', ['chat on', cmd('gamerule spawn_mobs false'),
                     cmd('gamerule random_tick_speed 0'), cmd('gamerule mob_griefing true'),
                     cmd('fill -2 79 -4 42 79 2 minecraft:stone'),
                     cmd('fill -2 80 -4 42 90 2 minecraft:air'),
                     cmd('tp @s 18.5 80 -3.5')])]
SCRIPT.append(('eggs', [cmd(f'setblock {x} 80 0 minecraft:turtle_egg[eggs={n}]')
                        for x, n in EGGS + [(NO_DROP, 4)]]))
SCRIPT.append(('drop', ['events on', 'spawns on'] + [anvil(x) for x, _ in EGGS]
               + [anvil(NO_DROP, ',DropItem:0b'), anvil(STONE)]))
SCRIPT.append(('fall', []))
SCRIPT.append(('settle', ['events off', 'spawns off']))
SCRIPT.append(('settle-2', []))
EXPECTED = []


def check(name, condition, result):
    SCRIPT.append((f'{name}-check', [cmd(f'execute {condition}')]))
    EXPECTED.append(f'{name}-check {result}')


for x, n in EGGS:
    check(f'eggs-{x}', f'if block {x} 80 0 minecraft:turtle_egg[eggs={n}]', 'pass')
check('no-drop-eggs', f'if block {NO_DROP} 80 0 minecraft:turtle_egg[eggs=4]', 'pass')
check('no-drop-item', f'if entity @e[type=minecraft:item,x={NO_DROP}.5,y=80.2,z=0.5,distance=..1.5]', 'fail')
check('stone-anvil', f'if block {STONE} 80 0 minecraft:anvil', 'pass')


EGG_TOP = 80 + 7 / 16


def at_anvil(spawn):
    """`minecraft:item X Y Z` as `minecraft:item CELL at-anvil` when the item
    was added at its cell's centre on the eggs, now or a tick ago."""
    kind, *xyz = spawn.split()
    if kind != 'minecraft:item':
        return spawn
    x, y, z = map(float, xyz)
    cell = int(x // 1)
    moved = max(abs(x - cell - 0.5), abs(z - 0.5))
    if (moved < 1e-4 and abs(y - EGG_TOP) < 1e-4) or (moved <= 0.1 + 1e-4 and abs(y - EGG_TOP - 0.16) < 1e-4):
        return f'{kind} {cell} at-anvil'
    return spawn


def shown(lines):
    out, events = [], []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) >= 3 and parts[2] in ('event', 'spawn'):
            # One tick's events and entities may come in any order.
            what = at_anvil(parts[3]) if parts[2] == 'spawn' else parts[3]
            events.append(f'{parts[1].removeprefix("step=")} {parts[2]} {what}')
            continue
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
    return sorted(events) + out


def check_vanilla(native, all_lines):
    wanted = sorted([f'drop event 1029 {x} 80 0 0' for x, _ in EGGS] + [f'drop event 1031 {STONE} 80 0 0']
                    + [f'drop spawn minecraft:item {x} at-anvil' for x, _ in EGGS]
                    + [f'drop spawn minecraft:falling_block {x}.5000 86.0000 0.5000'
                       for x in [x for x, _ in EGGS] + [NO_DROP, STONE]])
    if native != wanted + EXPECTED:
        return f'vanilla did not exercise the intended cases: {native!r}'
    # Vanilla's own items are added before their first tick, exactly there.
    exact = {f'spawn minecraft:item {x}.5000 80.4375 0.5000' for x, _ in EGGS}
    if not exact <= {line.split(' ', 2)[2] for line in all_lines if ' spawn ' in line}:
        return 'vanilla added an item away from its anvil'
    return None


def main():
    lines = net.one_player('anvil-turtle-egg-parity', SCRIPT, shown, check_vanilla, __doc__,
                           step_seconds=STEP_SECONDS)
    print(f'anvil turtle eggs: {len(lines)} results match vanilla')


if __name__ == '__main__':
    main()
