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

    def __init__(self, players, split=True, saved_split=False, **env):
        self.home = Path(tempfile.mkdtemp(prefix='horizon-split-e2e-'))
        if saved_split:
            (self.home / 'horizon-extras.txt').write_text('splitscreen:true\n')
        self.sockets = Path(tempfile.mkdtemp(prefix='hzse-'))  # short: Unix socket paths
        self.env = lab.game_env(HORIZON_HOME=self.home, HORIZON_SPLIT=str(players), **env)
        if not split:
            self.env.pop("HORIZON_SPLIT", None)
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

    def press(self, n, label):
        """Click a button once it's drawn and enabled: `ui` lists the last drawn frame's
        widgets, which lag a page change (by most of a second on GNOME Wayland)."""
        def ready():
            return any(w['label'] == label and w['enabled'] for w in self.ctl(n, 'ui')['widgets'])
        self.wait(ready, f'P{n} button {label!r}')
        self.ctl(n, 'click-button', label)

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
        g.ctl(1, 'options', 'horizon')
        labels = lambda: [w['label'] for w in g.ctl(1, 'ui')['widgets']]
        g.wait(lambda: 'Splitscreen...' in labels(), 'Horizon Settings')
        g.ctl(1, 'click-button', 'Splitscreen...')
        g.wait(lambda: any(s.startswith('Splitscreen: ') for s in labels()), 'saved switch')
        check('testing override leaves the saved switch off', 'Splitscreen: Off' in labels(), True)
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


def scenario_picker():
    print('picker: a joining player makes a profile and plays as it')
    g = Game(2, HORIZON_SPLIT_PICK='1')
    try:
        def picking():
            return (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles'
        g.wait(picking, 'the picker')
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Sam')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Sam in the world')
        check('Sam has its profile folder', (g.home / 'splitscreen/profiles/Sam/options.txt').exists(), True)
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'Sam gone', seconds=15)
    finally:
        g.quit()


def scenario_cancel():
    print('cancel: a joining player cancels its pick and leaves; P1 gets its window back')
    g = Game(2, HORIZON_SPLIT_PICK='1')
    try:
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode') == 'splitscreen-profiles',
               'the picker')
        w, h = g.size(1)
        g.ctl(2, 'click-button', 'Cancel')
        g.wait(lambda: not g.alive(2), 'P2 gone after Cancel', seconds=15)
        check('P1 alone: its whole window again', g.settle([1], [(2 * w, h)]), [(2 * w, h)])
    finally:
        g.quit()


def scenario_switch():
    print('switch: pause menu changes profile, Cancel keeps it, saved progress survives')
    g = Game(2)
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 playing')
        half = g.size(2)

        def settings():
            g.ctl(2, 'key', 'esc', 'tap')
            g.wait(lambda: any(w['label'] == 'Splitscreen Settings'
                              for w in g.ctl(2, 'ui')['widgets']), 'pause settings row')
            g.ctl(2, 'click-button', 'Splitscreen Settings')
            g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode')
                   == 'splitscreen-profiles', 'profile settings')

        settings()
        check('current test profile appears even without a stored folder',
              'P2' in g.ctl(2, 'state')['menu']['profiles'], True)
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Alex')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Alex playing')
        g.ctl(2, 'command', 'experience set @s 7 levels')
        g.wait(lambda: g.ctl(2, 'state')['player']['xp']['level'] == 7, 'Alex experience')
        settings()
        g.ctl(2, 'click-button', 'Cancel')
        check('Cancel keeps the current player', g.ctl(2, 'state')['player']['xp']['level'], 7)
        # Cancel returns to the pause menu; resume before opening it again.
        g.ctl(2, 'key', 'esc', 'tap')
        settings()
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Sam')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None
               and g.ctl(2, 'state')['player']['xp']['level'] == 0, 'Sam playing')
        check('switch keeps the view', g.size(2), half)
        settings()
        check('old profile is free', 'Alex' not in g.ctl(2, 'state')['menu']['taken'], True)
        g.ctl(2, 'click-button', 'Alex')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None
               and g.ctl(2, 'state')['player']['xp']['level'] == 7, 'Alex saved experience restored')
        check('switching back restores its progress', g.ctl(2, 'state')['player']['xp']['level'], 7)
    finally:
        g.quit()


def scenario_bumpers():
    print('bumpers: reorder views, move into a new window, return and join there')
    g = Game(2, HORIZON_SPLIT_PICK='1')
    try:
        g.wait(lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode')
               == 'splitscreen-profiles', 'the picker')
        half = g.size(1)
        whole = (2 * half[0], half[1])
        # From second place to first, then second again: neither opens a new window.
        g.ctl(2, 'pad', 'lb', 'tap')
        time.sleep(0.5)
        g.ctl(2, 'pad', 'rb', 'tap')
        time.sleep(0.5)
        check('LB then RB keeps both views in Window 1', g.sizes([1, 2]), [half, half])
        g.ctl(2, 'pad', 'rb', 'tap')
        check('next RB opens Window 2', g.settle([1], [whole]), [whole])
        own = g.size(2)
        g.ctl(2, 'pad', 'lb', 'tap')
        check('LB returns beside P1', g.settle([1, 2], [half, half]), [half, half])
        g.ctl(2, 'pad', 'rb', 'tap')
        g.settle([1, 2], [whole, own])
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Ann')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Ann playing')
        check('Ann joins in the chosen window', g.settle([1, 2], [whole, own]), [whole, own])
    finally:
        g.quit()


def scenario_toggle():
    print('toggle: default off, saved switch, joined guard, solo rendering after leaving')
    def labels(g):
        return [w['label'] for w in g.ctl(1, 'ui')['widgets']]
    def page(g):
        g.ctl(1, 'options', 'horizon')
        g.wait(lambda: 'Splitscreen...' in labels(g), 'Horizon Settings')
        g.ctl(1, 'click-button', 'Splitscreen...')
        g.wait(lambda: any(s.startswith('Splitscreen: ') for s in labels(g)), 'switch')
    g = Game(1, split=False)
    try:
        whole = g.size(1)
        page(g)
        check('default off without the testing variable', 'Splitscreen: Off' in labels(g), True)
        g.ctl(1, 'click-button', 'Splitscreen: Off')
        g.wait(lambda: 'Splitscreen: On' in labels(g), 'enabled')
        # Close all options pages to persist, then return.
        for _ in range(3):
            g.ctl(1, 'key', 'esc', 'tap')
        check('saved enabled', 'splitscreen:true' in (g.home / 'horizon-extras.txt').read_text(), True)
        page(g)
        g.ctl(1, 'click-button', 'Splitscreen: On')
        for _ in range(3):
            g.ctl(1, 'key', 'esc', 'tap')
        check('saved disabled', 'splitscreen:false' in (g.home / 'horizon-extras.txt').read_text(), True)
        check('solo keeps its full view', g.size(1), whole)
    finally:
        g.quit()
    g = Game(2, saved_split=True)
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 playing')
        half = g.size(1)
        page(g)
        switches = [w for w in g.ctl(1, 'ui')['widgets'] if w['label'] == 'Splitscreen: On']
        check('joined: switch disabled', [w['enabled'] for w in switches], [False])
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'P2 gone')
        g.wait(lambda: any(w['label'] == 'Splitscreen: On' and w['enabled']
                          for w in g.ctl(1, 'ui')['widgets']), 'switch available alone')
        g.ctl(1, 'click-button', 'Splitscreen: On')
        check('off: full window restored', g.settle([1], [(2 * half[0], half[1])]),
              [(2 * half[0], half[1])])
        g.ctl(1, 'click-button', 'Splitscreen: Off')
        check('on alone: full window stays', g.size(1), (2 * half[0], half[1]))
    finally:
        g.quit()


def scenario_setup():
    print('setup: preview, Revert, Apply, Done, and solo return while the page stays open')
    g = Game(2, saved_split=True)
    def labels():
        return [w['label'] for w in g.ctl(1, 'ui')['widgets']]
    def enabled(label):
        return next(w['enabled'] for w in g.ctl(1, 'ui')['widgets'] if w['label'] == label)
    def page():
        g.ctl(1, 'options', 'horizon')
        g.press(1, 'Splitscreen...')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'active', 'Setup active')
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 playing')
        half = g.size(1)
        whole = (2 * half[0], half[1])
        g.ctl(1, 'key', 'esc', 'tap')
        g.wait(lambda: 'Splitscreen Settings' in labels(), 'P1 pause shortcut')
        g.press(1, 'Splitscreen Settings')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'active', 'Setup active')
        g.press(1, 'P2')
        g.press(1, 'New window')
        check('preview leaves the real views alone', g.sizes([1, 2]), [half, half])
        check('draft enables Apply', enabled('Apply'), True)
        g.press(1, 'Revert')
        check('Revert clears the draft', enabled('Apply'), False)
        g.press(1, 'P2')
        g.press(1, 'New window')
        g.press(1, 'Apply')
        check('Apply gives P1 its whole window', g.settle([1], [whole]), [whole])
        own = g.size(2)
        check('P2 gets a separate window', own != half, True)
        g.wait(lambda: not enabled('Apply'), 'the applied draft is current')
        g.press(1, 'P2')
        g.press(1, 'Window 1')
        g.press(1, 'Done')
        check('Done discards unapplied placement', g.sizes([1, 2]), [whole, own])
        page()
        g.press(1, 'P2')
        g.press(1, 'Window 1')
        g.press(1, 'Apply')
        check('Apply reunites both views', g.settle([1, 2], [half, half]), [half, half])
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'P2 gone')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'idle', 'Setup idle')
        check('idle map has no stale player tiles', 'P2' in labels(), False)
        check('idle P1 has the full window', g.settle([1], [whole]), [whole])
        for _ in range(3):
            g.press(1, 'Done')
            g.ctl(1, 'options', 'horizon')
            g.press(1, 'Splitscreen...')
            g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'idle', 'reopened idle')
        g.press(1, 'Splitscreen: On')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'off', 'Setup off')
        g.press(1, 'Splitscreen: Off')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'idle', 'Setup idle again')
    finally:
        g.quit()


def scenario_setup_leave():
    print('setup_leave: a middle player leaving invalidates the draft instead of moving the wrong seat')
    g = Game(3, saved_split=True)
    try:
        g.wait(lambda: g.ctl(3, 'state').get('player') is not None, 'P3 playing')
        g.ctl(1, 'options', 'horizon')
        g.press(1, 'Splitscreen...')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'active', 'Setup active')
        g.press(1, 'P3')
        g.press(1, 'New window')
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'P2 gone')
        def reset():
            widgets = g.ctl(1, 'ui')['widgets']
            return any(w['label'] == 'Apply' and not w['enabled'] for w in widgets)
        g.wait(reset, 'stale draft discarded')
        labels = [w['label'] for w in g.ctl(1, 'ui')['widgets']]
        check('remaining seat stays on the map', 'P3' in labels and 'P2' not in labels, True)
        check('P3 shares the original window', g.size(3), g.size(1))
    finally:
        g.quit()


SCENARIOS = {
    'alone': scenario_alone,
    'join': scenario_join,
    'four': scenario_four,
    'windows': scenario_windows,
    'picker': scenario_picker,
    'cancel': scenario_cancel,
    'switch': scenario_switch,
    'bumpers': scenario_bumpers,
    'toggle': scenario_toggle,
    'setup': scenario_setup,
    'setup_leave': scenario_setup_leave,
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
