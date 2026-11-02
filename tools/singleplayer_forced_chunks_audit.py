#!/usr/bin/env python3
"""Check #1527: per-dimension /forceload add/remove across normal restarts.

Build horizon-client and horizon-ctl with the fast profile first, then run:
  python3 tools/singleplayer_forced_chunks_audit.py --case force-save
Runs singleplayer on its server thread, using scratch homes.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

from agent_control_e2e import Cli
import lab


DIMENSIONS = ('the_nether', 'the_end', 'overworld')


def query(cli, dimension, expected):
    since = cli('events')['last']
    cli('command', command='forceload query')
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        reply = cli('events', since=since)
        for event in reply['events']:
            text = event.get('text', '')
            if event.get('event') != 'chat' or f'minecraft:{dimension}' not in text:
                continue
            if expected:
                if all(f'[{x}, {z}]' in text for x, z in expected):
                    return
            elif 'No force loaded chunks' in text:
                return
        time.sleep(0.1)
    raise AssertionError(f'{dimension}: expected {expected}, got {reply}')


def run(bin_dir, resources):
    with tempfile.TemporaryDirectory(prefix='hz-force-save-') as scratch:
        work = Path(scratch)
        previous = None
        for stage in range(3):
            # Every process gets fresh homes; copy only this fixture's saved world.
            env = lab.game_env({k: v for k, v in os.environ.items() if k not in (
                'HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME')})
            home = Path(env['HORIZON_HOME'])
            if previous is not None:
                shutil.copytree(previous, home / 'saves')
            socket = work / f'{stage}.sock'
            cli = Cli(bin_dir / 'horizon-ctl', socket, home, env)
            args = [str(bin_dir / 'horizon-client'), '--resources', str(resources),
                    '--world', 'forced', '--creative', '--seed', '4242', '--offscreen',
                    '--render-distance', '4', '--control', '--control-socket', str(socket)]
            with (work / f'{stage}.log').open('w') as log:
                process = subprocess.Popen(args, cwd=home, env=env, stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 120
                    while not socket.exists():
                        if process.poll() is not None or time.monotonic() > deadline:
                            raise RuntimeError('game did not open its control socket')
                        time.sleep(0.1)
                    cli('wait_ready', seconds=240)
                    for i, dim in enumerate(DIMENSIONS):
                        cli('command', command=f'execute in minecraft:{dim} run tp @s 0 100 0')
                        for _ in range(30):
                            cli('wait', ticks=20)
                            if cli('state').get('dimension') == f'minecraft:{dim}':
                                break
                        else:
                            raise AssertionError(f'did not enter {dim}')
                        cli('wait_ready', seconds=240)
                        expected = [(i - 1, -1)] if stage == 1 else []
                        query(cli, dim, expected)
                        if stage == 0:
                            cli('command', command=f'forceload add {(i - 1) * 16} -1')
                            query(cli, dim, [(i - 1, -1)])
                        elif stage == 1:
                            cli('command', command=f'forceload remove {(i - 1) * 16} -1')
                            query(cli, dim, [])
                    cli('quit')
                    assert process.wait(timeout=120) == 0
                except Exception:
                    log.flush()
                    print((work / f'{stage}.log').read_text(errors='replace')[-6000:])
                    raise
                finally:
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=30)
            previous = home / 'saves'
        print('PASS force-save: three dimensions, add/remove/restart')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--case', choices=['force-save'], default='force-save')
    parser.add_argument('--bin-dir', type=Path, default=Path(__file__).resolve().parents[1] / 'target/fast')
    args = parser.parse_args()
    run(args.bin_dir.resolve(), lab.RESOURCES.resolve())


if __name__ == '__main__':
    main()
