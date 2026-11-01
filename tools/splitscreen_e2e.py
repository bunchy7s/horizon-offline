#!/usr/bin/env python3
"""Splitscreen end to end: real games, driven like a player would, checked from outside.

    python3 tools/splitscreen_e2e.py                 # every scenario
    python3 tools/splitscreen_e2e.py --only join     # some scenarios by name
    python3 tools/splitscreen_e2e.py -v --only join  # -v: every command and its reply

How it works
- `Game(players)` launches P1's game through horizon-ctl (`target/fast/horizon-ctl`, built with
  `cargo build --profile fast -p horizon-client`) with `HORIZON_SPLIT=players`: the game adds the
  others itself, as P2, P3...
- Every player has its own control socket (`c.sock`, `c.sock2`...). `g.ctl(n, words...)` sends one
  horizon-ctl command to player n (docs/horizon-ctl-commands.md).
- Checks look from outside: each player's view size (`g.size(n)`, from `status`) and its `state`.
  `check(name, got, want)` records a failure and goes on; a `RuntimeError` (a `wait` that timed
  out) ends that scenario as failed.
- Each game gets fresh scratch folders (`lab.game_env`) and runs inside them: never your real game
  data. Needs a desktop session (real windows) and the vanilla data in .local/.

Adding a scenario: write `scenario_<name>()` that prints `"<name>: what it checks"` first, uses
`Game` in a try/finally with `g.quit()`, checks with `check`, and add it to `SCENARIOS`.

Don't rebuild the game while it runs: the next launch takes the new binary or times out. The
windows are real and can take keyboard focus.

Exit status 0 when every check passed.
"""

import argparse
import contextlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
CTL = lab.cargo_target() / ('fast/horizon-ctl' + ('.exe' if os.name == 'nt' else ''))
RESOURCES = ROOT / '.local/horizon-client/26.3'
# Logical; the views are checked as fractions of what P1 gets alone, so any scale works.
WIDTH, HEIGHT = 1000, 600
VERBOSE = False

failures = []


def check(name, got, want):
    ok = got == want
    print(f'  {"ok  " if ok else "FAIL"} {name}: {got}' + ('' if ok else f', expected {want}'))
    if not ok:
        failures.append(name)


class Game:
    """One splitscreen session: P1 launched with `players` (HORIZON_SPLIT), each on its socket."""

    def __init__(self, players, **env):
        self.home = Path(tempfile.mkdtemp(prefix='horizon-split-e2e-'))
        self.sockets = Path(tempfile.mkdtemp(prefix='hzse-'))  # short: Unix socket paths
        self.env = lab.game_env(HORIZON_HOME=self.home, HORIZON_SPLIT=str(players), **env)
        try:
            self.ctl(1, 'launch', '--wait', '--world', 'Split', '--creative',
                     '--resources', str(RESOURCES), '--width', str(WIDTH), '--height', str(HEIGHT),
                     '--render-distance', '4', '--name', 'P1', '--no-pause')
            self.wait(lambda: all(self.alive(n) for n in range(1, players + 1)), 'every player in')
        except BaseException:
            # A launch that never got ready leaves no game behind.
            self.quit()
            raise

    def ctl(self, n, *words):
        socket = self.sockets / ('c.sock' if n == 1 else f'c.sock{n}')
        # The reply through a file, not a pipe: on Windows the game `launch` starts inherits the
        # pipe, which then never closes while it runs.
        with tempfile.TemporaryFile('w+') as out:
            subprocess.run([str(CTL), '--socket', str(socket), *words], env=self.env,
                           cwd=self.home, stdout=out, stderr=subprocess.DEVNULL)
            out.seek(0)
            reply = json.loads(out.read() or '{}')
        if VERBOSE:
            print(f'P{n} {" ".join(words)} -> {json.dumps(reply)[:400]}', file=sys.stderr, flush=True)
        if 'error' in reply:
            raise RuntimeError(f'P{n} {" ".join(words)}: {reply["error"]}')
        return reply

    def alive(self, n):
        try:
            self.ctl(n, 'status')
            return True
        except (RuntimeError, json.JSONDecodeError):
            return False

    def size(self, n):
        return tuple(self.ctl(n, 'status')['renderer']['size'])

    def sizes(self, players):
        return [self.size(n) for n in players]

    @staticmethod
    def wait(done, what, seconds=30):
        end = time.time() + seconds
        while time.time() < end:
            if done():
                return
            time.sleep(0.3)
        raise RuntimeError(f'timed out waiting for {what}')

    def settle(self, players, want):
        """Wait until the players' views have the sizes `want`."""
        try:
            self.wait(lambda: self.sizes(players) == want, 'the layout', seconds=8)
        except RuntimeError:
            pass
        return self.sizes(players)

    def quit(self):
        """Save and quit, and wait until the game is gone (the next scenario starts clean)."""
        try:
            self.ctl(1, 'quit')
            self.wait(lambda: not self.alive(1), 'the game to quit')
        except (RuntimeError, json.JSONDecodeError):
            # Not answering (still starting, or stuck): stop it by its socket (`--control-socket`).
            with contextlib.suppress(OSError):  # no pkill on Windows
                subprocess.run(['pkill', '-f', str(self.sockets)], capture_output=True)
        shutil.rmtree(self.sockets, ignore_errors=True)


def scenario_join():
    print('join: a second player in the same window, then it leaves')
    g = Game(2)
    try:
        halves = g.sizes([1, 2])
        check('two take equal halves', halves[0] == halves[1], True)
        w, h = halves[0]
        check('side by side in a wide window (each taller than wide)', w < h, True)
        check('P2 plays in the world', g.ctl(2, 'state').get('player') is not None, True)
        # A player's quit leaves splitscreen; only P1's ends the game.
        g.ctl(2, 'quit')
        g.wait(lambda: not g.alive(2), 'P2 gone after its quit', seconds=15)
        check('P1 alone: its whole window again', g.settle([1], [(2 * w, h)]), [(2 * w, h)])
        check('P1 plays on', g.ctl(1, 'state').get('player') is not None, True)
    finally:
        g.quit()


def scenario_four():
    print('four: quarters, and P4 disconnects')
    g = Game(4)
    try:
        quarters = g.sizes([1, 2, 3, 4])
        check('four take equal quarters', len(set(quarters)), 1)
        w, h = quarters[0]
        g.ctl(4, 'disconnect')
        three = g.settle([1, 2, 3], [(2 * w, h), (w, h), (w, h)])
        check('three: one wide on top, two below', three, [(2 * w, h), (w, h), (w, h)])
    finally:
        g.quit()


def scenario_alone():
    print('alone: watching the controllers, nobody joins, P1 plays as without splitscreen')
    g = Game(1)
    try:
        w, h = g.size(1)
        check('P1 has its whole window', (w, h), (2 * WIDTH, 2 * HEIGHT) if w > WIDTH else (WIDTH, HEIGHT))
        check('P1 plays', g.ctl(1, 'state').get('player') is not None, True)
    finally:
        g.quit()


def scenario_windows():
    print('windows: P3 in a window of its own, then it leaves and the window closes')
    g = Game(3, HORIZON_SPLIT_WINDOWS='1,1,2')
    try:
        halves = g.sizes([1, 2])
        check('P1 and P2 share Window 1 in halves', halves[0] == halves[1], True)
        own = g.size(3)
        check('P3 has Window 2 to itself (not a half of Window 1)', own != halves[0], True)
        g.ctl(3, 'quit')
        g.wait(lambda: not g.alive(3), 'P3 gone after its quit', seconds=15)
        check('P1 and P2 still share Window 1', g.settle([1, 2], halves), halves)
    finally:
        g.quit()


SCENARIOS = {
    'alone': scenario_alone,
    'join': scenario_join,
    'four': scenario_four,
    'windows': scenario_windows,
}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--only', nargs='+', choices=list(SCENARIOS), help='run only these')
    ap.add_argument('--verbose', '-v', action='store_true', help='print every command and its reply')
    args = ap.parse_args()
    global VERBOSE
    VERBOSE = args.verbose
    sys.stdout.reconfigure(line_buffering=True)
    if not CTL.exists():
        sys.exit(f'{CTL} is missing: cargo build --profile fast -p horizon-client')
    if not (RESOURCES / 'assets').exists():
        sys.exit(f'no prepared resources in {RESOURCES}')
    for name in args.only or list(SCENARIOS):
        print(f'[{name}]')
        try:
            SCENARIOS[name]()
        except RuntimeError as e:
            print(f'  FAIL {name}: {e}')
            failures.append(name)
    if failures:
        sys.exit(f'{len(failures)} check(s) failed: {", ".join(failures)}')
    print('splitscreen: every check passed')


if __name__ == '__main__':
    main()
