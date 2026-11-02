#!/usr/bin/env python3
"""Attribute swapping from a spear to a mace against an actual vanilla 26.3 server.

`Minecraft.tick` picks the crosshair target (`pick`, with the held item's
`attack_range`: a spear reaches 4.5 blocks) before `handleKeybinds` takes a
number key and then the attack click. A number key and a click in the same
tick therefore swing the new item at the target the spear picked, and the
server's `handleAttack` checks that item's range with a 3 block buffer. The
server is slowed to one tick a second so both inputs land in one client tick.

Cases: the swap and the click in one tick hit a pig 3.75 blocks away; the
swap a tick before the click misses it (the mace's own pick reaches 3); a
mace click at 2.25 blocks hits (the setup lands attacks at all). A pig
hovering at eye height inside the spear's 2 block minimum (its box, with the
0.125 margin, ends 1.475 along the ray): with a wall 3.5 away the spear's pick
ends on the wall (`getHitEntitiesAlong`'s block result), so the mace misses
the pig; with nothing behind it the spear's pick misses and the plain pick
finds the pig. A stick given an `attack_range` does not attack that pig,
inside its minimum (`startAttack`).
Requires prepared resources and built horizon-client/horizon-ctl binaries.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time

import lab
import test_rust_parity as base
from integration_env import game_environment
from test_dedicated_server_parity import Server, game_time
from test_locator_bar_parity import server_dir

ROOT = Path(__file__).resolve().parents[1]
EXE = '.exe' if os.name == 'nt' else ''
NAME = 'SwapTester'
# Tagged: a generated world has pigs of its own.
PIG = '@e[type=minecraft:pig,tag=swap,limit=1]'
# Player at 0.5 80 0.5 looking east; the pig's near face is `distance` - 0.45
# from the eyes' column.
OPEN = ['fill 2 80 -3 8 84 3 minecraft:air']
WALL = [*OPEN, 'fill 4 80 -3 4 84 3 minecraft:stone']
SWAP = ['key 2 tap', 'mouse left click']
APART = ['key 2 tap', 'wait 1', 'mouse left click']
EYE = 81.62
CASES = [
    # (label, terrain, pig distance, pig feet y, look height, inputs, expected hit)
    ('same-tick', OPEN, 4.2, 80, 80.45, SWAP, True),
    ('tick-apart', OPEN, 4.2, 80, 80.45, APART, False),
    ('mace-near', OPEN, 2.7, 80, 80.45, APART, True),
    ('min-reach-wall', WALL, 0.9, 80.8, EYE, SWAP, False),
    ('min-reach-open', OPEN, 0.9, 80.8, EYE, SWAP, True),
    ('ranged-stick', OPEN, 0.9, 80.8, EYE, ['key 3 tap', 'wait 1', 'mouse left click'], False),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, default=ROOT / f'target/fast/horizon-client{EXE}')
    parser.add_argument('--ctl', type=Path, default=ROOT / f'target/fast/horizon-ctl{EXE}')
    parser.add_argument('--resources', type=Path, default=ROOT / '.local/horizon-client/26.3')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    output = (args.output or Path(tempfile.mkdtemp(prefix='attack-swap-'))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    _, jars, libraries = base.verify_artifacts()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    command = ['java', '-Xmx2G', '-Duser.timezone=UTC', '-cp', dependencies,
               'net.minecraft.server.Main', 'nogui']
    port = server_dir(output / 'server')
    server = Server(command, output / 'server', 'vanilla')
    env = game_environment(os.environ, output)
    results = []
    # Keep AF_UNIX paths short even in nested worktrees.
    with tempfile.TemporaryDirectory(prefix='aswap-', dir=None if os.name == 'nt' else '/tmp') as socket_home:
        socket = Path(socket_home) / 'ctl.sock'

        def ctl(*commands, timeout=500):
            r = subprocess.run([str(args.ctl.resolve()), '--socket', str(socket), *map(str, commands)],
                               cwd=output, env=env, capture_output=True, text=True, timeout=timeout)
            data = json.loads(r.stdout)
            if r.returncode or data.get('error'):
                raise RuntimeError(f'{commands}: {data}; {r.stderr}')
            return data

        def pig_health():
            start = len(server.lines)
            server.console(f'data get entity {PIG} Health')
            game_time(server)
            values = [float(m.group(1)) for line in server.lines[start:]
                      if (m := re.search(r'entity data: (-?\d+(?:\.\d+)?)f', line))]
            assert len(values) == 1, server.lines[start:]
            return values[0]

        try:
            server.wait_for(r'Done \(')
            ctl('launch', '--game', args.game.resolve(), '--offscreen', '--no-pause', '--name', NAME,
                '--resources', args.resources.resolve(), '--render-distance', 2, '--width', 960, '--height', 720)
            ctl('connect', f'127.0.0.1:{port}', NAME)
            ctl('wait-ready', 400)
            for line in (f'op {NAME}', 'gamerule spawn_mobs false', f'gamemode survival {NAME}',
                         'kill @e[type=!minecraft:player]',
                         'fill -3 79 -3 8 79 3 minecraft:stone', 'fill -3 80 -3 8 84 3 minecraft:air',
                         f'tp {NAME} 0.5 80 0.5 -90 0',
                         f'item replace entity {NAME} hotbar.0 with minecraft:iron_spear',
                         f'item replace entity {NAME} hotbar.1 with minecraft:mace',
                         f'item replace entity {NAME} hotbar.2 with minecraft:stick'
                         '[minecraft:attack_range={min_reach:2.0f,max_reach:4.5f}]'):
                server.console(line)
            game_time(server)
            ctl('wait', 20)
            for label, terrain, distance, feet, look, inputs, expected in CASES:
                server.console(f'kill {PIG}')
                for line in terrain:
                    server.console(line)
                server.console(f'summon minecraft:pig {0.5 + distance} {feet} 0.5 '
                               '{NoAI:1b,Silent:1b,NoGravity:1b,Tags:["swap"]}')
                server.console(f'tp {NAME} 0.5 80 0.5 -90 0')
                game_time(server)
                ctl('key', '1', 'tap')
                # A full attack charge with the spear in hand.
                ctl('wait', 40)
                ctl('look-at', 0.5 + distance, look, 0.5)
                before = pig_health()
                server.console('tick rate 1')
                game_time(server)
                ctl('wait', 2)
                ctl('wait', 1)
                for action in inputs:
                    ctl(*action.split())
                server.console('tick rate 20')
                game_time(server)
                ctl('wait', 10)
                after = pig_health()
                hit = after < before
                result = dict(label=label, distance=distance, health_before=before, health_after=after,
                              hit=hit, expected=expected, pass_=hit == expected)
                results.append(result)
                (output / 'results.json').write_text(json.dumps(results, indent=2))
                print(json.dumps(result), flush=True)
        finally:
            try:
                ctl('quit')
            except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError):
                pass
            server.stop()
    passed = sum(r['pass_'] for r in results)
    print(f'{passed}/{len(results)} attack swaps match vanilla: {output}')
    assert len(results) == len(CASES) and passed == len(results), results


if __name__ == '__main__':
    main()
