#!/usr/bin/env python3
"""Frozen piston client lifecycle against an actual vanilla 26.3 server.

Checks extension/retraction in two directions and an unfreeze control. Each
frozen snapshot waits 20 client ticks: a piston must remain a moving block
until the server's third step, rather than finishing between server steps.
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, default=ROOT / 'target/fast/horizon-client')
    parser.add_argument('--ctl', type=Path, default=ROOT / 'target/fast/horizon-ctl')
    parser.add_argument('--resources', type=Path, default=ROOT / '.local/horizon-client/26.3')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    output = (args.output or Path(tempfile.mkdtemp(prefix='piston-step-'))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    _, jars, libraries = base.verify_artifacts()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    command = ['java', '-Xmx2G', '-Duser.timezone=UTC', '-cp', dependencies,
               'net.minecraft.server.Main', 'nogui']
    port = server_dir(output / 'server')
    server = Server(command, output / 'server', 'vanilla')
    env = game_environment(os.environ, output)
    for key in ('HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD', 'HORIZON_LAUNCHER_DESKTOP'):
        env[key] = str(output / key)
    results = []
    # Keep AF_UNIX paths short even in nested worktrees.
    with tempfile.TemporaryDirectory(prefix='pstep-', dir='/tmp') as socket_home:
        socket = Path(socket_home) / 'ctl.sock'

        def ctl(*commands, timeout=500):
            r = subprocess.run([str(args.ctl.resolve()), '--socket', str(socket), *map(str, commands)],
                               cwd=output, env=env, capture_output=True, text=True, timeout=timeout)
            data = json.loads(r.stdout)
            if r.returncode or data.get('error'):
                raise RuntimeError(f'{commands}: {data}; {r.stderr}')
            return data

        def step():
            before = game_time(server)
            server.console('tick step 1')
            deadline = time.monotonic() + 20
            while (after := game_time(server)) == before:
                if time.monotonic() > deadline:
                    raise TimeoutError('vanilla did not step')
                time.sleep(0.02)
            assert after == before + 1, (before, after)

        def snapshot(label, pos, expected, progress=None, frozen=True):
            start = len(server.lines)
            xyz = ' '.join(map(str, pos))
            marker = f'PISTON_CHECK_{len(results)}'
            server.console(f'execute if block {xyz} minecraft:{expected} run say {marker}')
            before = game_time(server)
            assert any(marker in line for line in server.lines[start:]), (label, server.lines[start:])
            native_progress = None
            if progress is not None:
                start = len(server.lines)
                server.console(f'data get block {xyz} progress')
                game_time(server)
                values = [float(m.group(1)) for line in server.lines[start:]
                          if (m := re.search(r'block data: (-?\d+(?:\.\d+)?)f', line))]
                assert values == [progress], (label, server.lines[start:])
                native_progress = values[0]
            ctl('wait', 20)
            local = ctl('block', *pos)
            after = game_time(server)
            assert (before == after) if frozen else (after > before), (label, before, after)
            ctl('look-at', 1.0, 81.0, 0.5)
            ctl('screenshot', output / f'{label}.png')
            result = dict(label=label, expected=expected, client=local['block'],
                          native_saved_progress=native_progress, game_time_before=before,
                          game_time_after=after, pass_=local['block'] == f'minecraft:{expected}')
            results.append(result)
            (output / 'results.json').write_text(json.dumps(results, indent=2))
            print(json.dumps(result), flush=True)

        try:
            server.wait_for(r'Done \(')
            ctl('launch', '--game', args.game.resolve(), '--offscreen', '--no-pause', '--name', 'PistonTester',
                '--resources', args.resources.resolve(), '--render-distance', 2, '--width', 960, '--height', 720)
            ctl('connect', f'127.0.0.1:{port}', 'PistonTester')
            ctl('wait-ready', 400)
            for line in ('op PistonTester', 'gamerule spawn_mobs false',
                         'fill -5 79 -6 8 79 6 minecraft:stone', 'setblock 4 80 -4 minecraft:stone',
                         'tp PistonTester 4.5 81 -3.5'):
                server.console(line)
            game_time(server)
            ctl('wait', 10)
            ctl('key', 'f1', 'tap')
            for facing, direction in [('east', (1, 0, 0)), ('up', (0, 1, 0))]:
                front = tuple(a + b for a, b in zip((0, 80, 0), direction))
                dest = tuple(a + 2 * b for a, b in zip((0, 80, 0), direction))
                server.console('fill -3 80 -2 3 85 2 minecraft:air')
                server.console('tick freeze')
                game_time(server)
                ctl('wait', 5)
                server.console(f'setblock 0 80 0 minecraft:sticky_piston[facing={facing}]')
                server.console('setblock ' + ' '.join(map(str, front)) + ' minecraft:stone')
                server.console('setblock -1 80 0 minecraft:redstone_block')
                snapshot(f'{facing}-powered', dest, 'air')
                for i in range(1, 4):
                    step()
                    snapshot(f'{facing}-extend-{i}', dest, 'moving_piston' if i < 3 else 'stone',
                             (i - 1) * 0.5 if i < 3 else None)
                server.console('setblock -1 80 0 minecraft:air')
                snapshot(f'{facing}-unpowered', front, 'piston_head')
                for i in range(1, 4):
                    step()
                    snapshot(f'{facing}-retract-{i}', front, 'moving_piston' if i < 3 else 'stone',
                             (i - 1) * 0.5 if i < 3 else None)
                server.console('tick unfreeze')
                server.console('setblock -1 80 0 minecraft:redstone_block')
                ctl('wait', 20)
                snapshot(f'{facing}-unfrozen', dest, 'stone', frozen=False)
        finally:
            try:
                ctl('quit')
            except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError):
                pass
            server.stop()
    passed = sum(r['pass_'] for r in results)
    print(f'{passed}/{len(results)} client piston snapshots match vanilla: {output}')
    assert len(results) == 18 and passed == len(results), results


if __name__ == '__main__':
    main()
