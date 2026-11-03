#!/usr/bin/env python3
"""SpawnEggItem.use on source fluids, through real vanilla and Horizon servers.

Fluid cases send UseItem into a deep pool; a UseItemOn control has a mob
under the last look direction. Compare spawn, egg count and used stat. Covers
lava/water sources, flowing lava, a solid obstruction, waterlogged blocks,
creative, adventure (with/without can_place_on) and peaceful monster eggs.
"""
import json
try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

CASES = [
    ('block-use', 'stone', 'cow', 'survival', '', True),
    ('lava', 'lava', 'strider', 'survival', '', True),
    ('water', 'water', 'cow', 'survival', '', True),
    ('flowing', 'lava[level=1]', 'strider', 'survival', '', False),
    ('solid', 'stone', 'strider', 'survival', '', False),
    ('waterlogged', 'oak_slab[type=top,waterlogged=true]', 'strider', 'survival', '', False),
    ('creative', 'lava', 'strider', 'creative', '', True),
    ('adventure', 'lava', 'strider', 'adventure', '', False),
    ('adventure-allowed', 'lava', 'strider', 'adventure',
     '[minecraft:can_place_on={blocks:"minecraft:lava"}]', True),
    ('peaceful', 'lava', 'zombie', 'survival', '', False),
]
SCRIPT = [("setup", ["chat on", "cmd Alice gamerule spawn_mobs false"])]
previous_mode = "creative"
EXPECTED = []
for name, block, mob, mode, component, spawn in CASES:
    egg = f'minecraft:{mob}_spawn_egg'
    setup = [
        *(['cmd Alice gamemode creative'] if previous_mode != 'creative' else []),
        'cmd Alice kill @e[type=!player]', 'cmd Alice clear @s',
        'cmd Alice difficulty ' + ('peaceful' if name == 'peaceful' else 'normal'),
        'cmd Alice fill -2 74 -2 6 80 6 minecraft:stone',
        'cmd Alice fill -2 81 -2 6 85 6 minecraft:air',
        f'cmd Alice fill 0 75 0 4 80 4 minecraft:{block}',
        f'cmd Alice tp @s 2.5 81 -0.5 0 {0 if name == "block-use" else 50}',
        *(['cmd Alice summon minecraft:strider 2.5 81 1.5 {NoAI:1b}'] if name == 'block-use' else []),
        f'cmd Alice give @s {egg}{component} 8',
        'cmd Alice scoreboard objectives remove eggs',
        f'cmd Alice scoreboard objectives add eggs minecraft.used:minecraft.{mob}_spawn_egg',
        *([f'cmd Alice gamemode {mode}'] if mode != 'creative' else []),
    ]
    previous_mode = mode
    SCRIPT += [(name + '-setup', setup), (name + '-use', ['use-on Alice 4 80 -1' if name == 'block-use' else 'use Alice 50']),
               (name + '-check', [
                   f'cmd Alice execute {"if" if spawn else "unless"} entity @e[type=minecraft:{mob}]',
                   f'cmd Alice clear @s {egg} 0',
                   f'cmd Alice execute {"if" if spawn else "unless"} score @s eggs matches 1..',
               ])]
    EXPECTED += [f'{name}-check pass', f'{name}-check eggs {7 if spawn and mode != "creative" else 8}',
                 f'{name}-check pass']


def shown(lines):
    out = []
    for line in lines:
        parts = line.split(' ', 3)
        if len(parts) < 4 or not parts[1].endswith('-check') or parts[2] != 'chat':
            continue
        label = parts[1].removeprefix('step=')
        value = json.loads(parts[3])
        key = value.get('translate', '') if isinstance(value, dict) else ''
        if key.startswith('commands.execute.conditional.'):
            out.append(f'{label} {"pass" if ".pass" in key else "fail"}')
        elif key.startswith('commands.clear.test.'):
            out.append(f'{label} eggs {value["with"][0]}')
    return out


def check(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the intended cases: {native!r}'
    return None


def main():
    lines = net.one_player('spawn-egg-fluid-parity', SCRIPT, shown, check, __doc__, step_seconds=0.5)
    print(f'spawn eggs in fluids: {len(CASES)} cases, {len(lines)} results match vanilla')


if __name__ == '__main__':
    main()
