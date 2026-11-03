#!/usr/bin/env python3
"""Leaving a world and opening it again keeps its entities as they were (#374).

Vanilla's integrated server unloads every entity with the level when the player saves
and quits (`MinecraftServer.stopServer`), so a world reopened in the same session holds
each entity once. Horizon kept non-mob entities (minecarts, boats, armor stands...) in
memory after Save and Quit to Title; opening the world loaded them again from disk and
the next save wrote both copies, doubling them each time, until the game crawled.

The game runs offscreen (macOS) or on the virtual screen (Linux) with a control socket:
a minecart is summoned beside the player (seed 7 also has mineshaft chest minecarts in
view), then the world is closed through the pause menu and reopened from the title three
times. The minecarts must be the same ones each time, each once.

  test_world_reopen_parity.py [--bin-dir DIR] [--keep]
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / '.local' / 'horizon-client' / '26.3'
RUN = ROOT / '.local' / 'world-reopen'
CYCLES = 3


def game_env():
    """macOS renders offscreen; Linux uses the virtual screen (as test_save_roundtrip_parity), or renders
    offscreen too where there is no vscreen."""
    env = lab.game_env()
    if sys.platform == 'darwin' or not shutil.which('vscreen'):
        return env, ['--offscreen']
    subprocess.run(['vscreen', 'up'], check=False, capture_output=True)
    text = subprocess.run(['vscreen', 'env'], capture_output=True, text=True, check=True).stdout
    env.pop('DISPLAY', None)
    for match in re.finditer(r'(\w+)=(\S+)', text.split('unset')[0]):
        env[match.group(1)] = match.group(2)
    return env, []


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build them with the fast profile)')
    parser.add_argument('--keep', action='store_true', help='keep the run folder')
    args = parser.parse_args()
    bin_dir = Path(args.bin_dir).resolve() if args.bin_dir else None
    if bin_dir is None:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib', '--bins'], cwd=ROOT, check=True)
        bin_dir = ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'fast'
    shutil.rmtree(RUN, ignore_errors=True)
    RUN.mkdir(parents=True)
    socket = Path(tempfile.mkdtemp(prefix='hzro')) / 'control.sock'  # sockets need short paths
    env, extra = game_env()

    def ctl(*words, check=True):
        result = subprocess.run([str(bin_dir / 'horizon-ctl'), '--socket', str(socket), '--timeout', '600', *words],
                                cwd=RUN, env=env, capture_output=True, text=True)
        if check and result.returncode:
            sys.exit(f'horizon-ctl {" ".join(words)} failed: {result.stdout}{result.stderr}')
        return json.loads(result.stdout) if result.stdout.strip().startswith('{') else result.stdout

    def carts():
        """The loaded minecarts once their list stops changing: on a loaded runner the mineshaft's chunks and their
        entities keep arriving after the world counts as loaded, so one reading can be short of a cart."""
        last = None
        for _ in range(30):
            found = sorted((c['kind'], *(round(v, 1) for v in c['position'])) for c in ctl('status')['carts'])
            if found == last:
                return found
            last = found
            ctl('wait', '40')
        return last

    ctl('launch', '--resources', str(RESOURCES), '--world', 'reopen', '--seed', '7', '--creative',
        '--render-distance', '4', *extra)
    failures = 0
    try:
        ctl('wait-ready', '300')
        ctl('wait-for', 'loaded', '--timeout', '300')  # as after each reopen below
        ctl('command', 'summon minecraft:minecart ~2 ~ ~2')
        ctl('wait', '20')
        expected = carts()
        print('minecarts:', len(expected))
        if not any(kind == 'minecraft:minecart' for kind, *_ in expected):
            sys.exit(f'the summoned minecart is not there: {expected}')
        for cycle in range(1, CYCLES + 1):
            ctl('pause')
            ctl('click-button', 'Save and Quit to Title')
            ctl('wait-for', 'screen', 'title', '--timeout', '120')
            ctl('open', 'reopen')
            ctl('wait-for', 'loaded', '--timeout', '300')
            ctl('wait', '20')
            got = carts()
            ok = got == expected
            failures += not ok
            print(f'reopen {cycle}: {len(got)} minecarts', 'ok' if ok else f'FAIL (expected {len(expected)})')
    finally:
        ctl('quit', check=False)
        if not args.keep and not failures:
            shutil.rmtree(RUN, ignore_errors=True)
    print('world reopen:', 'PASS' if not failures else f'{failures} FAILED')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
