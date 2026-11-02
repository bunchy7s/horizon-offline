#!/usr/bin/env python3
"""Check that a boat in flowing water drifts with the current (#6435).

A headless single-player game opens a scratch world; a one-deep, three-wide stone
trench is filled from a single source block at its head, an oak boat is summoned
onto the flowing water, and its `Pos` is read every ten ticks. Vanilla's
`Entity.updateFluidInteraction` pushes a boat by the water current it sits in
(`AbstractBoat` does not opt out of `isPushedByFluid`), with water's 0.014 scale,
and `AbstractBoat.tick` runs it before `floatBoat` scales the velocity; a boat
that stays put is missing that step.

The reported case rides the boat, but a ridden boat ticks through the same
`tick_boat` (the rider's client moves it), so the unridden boat covers the same
current step.

    python3 tools/boat_current_e2e.py --bin-dir target/fast
"""
import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

WORLD = 'BoatCurrent'
SAMPLE_TICKS = 10
SAMPLES = 4
MIN_DRIFT = 1.0
BOAT = '@e[type=minecraft:oak_boat,limit=1,sort=nearest]'


class Failed(Exception):
    pass


def ctl(ctl_path, socket, cwd, env, *words):
    result = subprocess.run([str(ctl_path), '--socket', str(socket), *map(str, words)], cwd=cwd, env=env,
                            capture_output=True, text=True, timeout=900)
    lines = result.stdout.strip().splitlines()
    reply = json.loads(lines[-1]) if lines else {'error': result.stderr.strip()}
    if result.returncode or 'error' in reply:
        raise Failed(f'{" ".join(map(str, words))}: exit {result.returncode}: {reply}')
    return reply


def command(ctl_path, socket, cwd, env, text):
    return ctl(ctl_path, socket, cwd, env, 'command', text)


def boat_pos(ctl_path, socket, cwd, env):
    reply = command(ctl_path, socket, cwd, env, f'data get entity {BOAT} Pos')
    for line in reply.get('chat') or []:
        if '[' in line and ']' in line:
            body = line[line.index('[') + 1: line.rindex(']')]
            return [float(v.strip().removesuffix('d')) for v in body.split(',')]
    raise Failed(f'no boat position in {reply}')


def build_trench(ctl_path, socket, cwd, env):
    # The chunk has to be loaded before a fill reaches it: aim the player at the
    # trench, then retry until the floor lands.
    command(ctl_path, socket, cwd, env, 'tp @s 2.5 160 1')
    ctl(ctl_path, socket, cwd, env, 'wait-for', 'loaded', '--timeout', '120')
    ctl(ctl_path, socket, cwd, env, 'wait', '60')
    for _ in range(5):
        reply = command(ctl_path, socket, cwd, env, 'fill 0 150 -1 9 150 3 stone')
        if not any('not loaded' in line for line in reply.get('chat') or []):
            break
        ctl(ctl_path, socket, cwd, env, 'wait', '40')
    else:
        raise Failed('the trench floor never loaded')
    command(ctl_path, socket, cwd, env, 'fill -1 151 -1 -1 151 3 stone')
    command(ctl_path, socket, cwd, env, 'fill 0 151 -1 9 151 -1 stone')
    command(ctl_path, socket, cwd, env, 'fill 0 151 3 9 151 3 stone')
    command(ctl_path, socket, cwd, env, 'fill 0 151 0 0 151 2 water')
    # A source spreads one block every five ticks; nine blocks settle in 45.
    ctl(ctl_path, socket, cwd, env, 'wait', '120')
    # Stand on the south wall, out of the current, next to the boat.
    command(ctl_path, socket, cwd, env, 'tp @s 2.5 152 3.5')
    ctl(ctl_path, socket, cwd, env, 'wait', '20')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', default='target/fast', help='folder with horizon-client and horizon-ctl')
    parser.add_argument('--resources', default=str(lab.RESOURCES), help='the prepared resources (default: .local)')
    parser.add_argument('--keep', action='store_true', help='keep the scratch game home')
    args = parser.parse_args()
    bindir = Path(args.bin_dir).resolve()
    client, ctl_path = bindir / 'horizon-client', bindir / 'horizon-ctl'
    for binary in (client, ctl_path):
        if not binary.exists():
            raise Failed(f'{binary} is missing; build it (cargo build --profile fast -p horizon-client '
                         '-p horizon-client-lib --bins) or pass --bin-dir')
    resources = Path(args.resources).resolve()
    if not resources.exists():
        raise Failed(f'{resources} is missing; run tools/lab.py prepare')
    work = Path(tempfile.mkdtemp(prefix='hz-boat-current-'))
    (work / '.local/horizon-client').mkdir(parents=True)
    lab.link_dir(resources, work / '.local/horizon-client' / lab.VERSION)
    (work / 'options.txt').write_text('onboardAccessibility:false\n')
    env = lab.game_env(home=work, HORIZON_TELEMETRY_URL='off')
    socket = work / 'c.sock'
    launch = subprocess.run([str(ctl_path), '--socket', str(socket), 'launch', '--world', WORLD, '--seed', '7',
                             '--creative', '--headless', '--wait'],
                            cwd=work, env=env, capture_output=True, text=True, timeout=900)
    if launch.returncode:
        log = next(iter(sorted(work.glob('*.log'))), None)
        tail = log.read_text(errors='replace')[-2000:] if log else launch.stderr[-2000:]
        raise Failed(f'launch failed ({launch.returncode}): {tail}')
    try:
        ctl(ctl_path, socket, work, env, 'wait', '20')
        build_trench(ctl_path, socket, work, env)
        command(ctl_path, socket, work, env, 'summon minecraft:oak_boat 2.5 151.0 1.9')
        ctl(ctl_path, socket, work, env, 'wait', '5')
        xs = []
        for _ in range(SAMPLES):
            xs.append(boat_pos(ctl_path, socket, work, env)[0])
            ctl(ctl_path, socket, work, env, 'wait', str(SAMPLE_TICKS))
        drift = xs[-1] - xs[0]
        print(f'boat x over {(SAMPLES - 1) * SAMPLE_TICKS} ticks: {xs[0]:.3f} -> {xs[-1]:.3f} ({drift:+.3f})')
        if drift < MIN_DRIFT:
            raise Failed(f'the flowing water did not push the boat (drift {drift:+.3f}, want >= {MIN_DRIFT}); '
                         'Entity.baseTick\'s fluid step is missing from the boat tick')
        print('PASS: the boat drifts with the flowing water')
        return 0
    finally:
        subprocess.run([str(ctl_path), '--socket', str(socket), 'quit'], cwd=work, env=env,
                       capture_output=True, timeout=120)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Failed as error:
        print(f'FAIL: {error}')
        sys.exit(1)
