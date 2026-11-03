#!/usr/bin/env python3
"""Splitscreen end to end: real games, driven like a player would, checked from outside.

    python3 tools/splitscreen_e2e.py                 # every scenario
    python3 tools/splitscreen_e2e.py --only join     # some scenarios by name
    python3 tools/splitscreen_e2e.py -v --only join  # -v: every command and its reply
    python3 tools/splitscreen_e2e.py --only native_pointer  # macOS, real OS input; desktop must be idle

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
windows are real and can take keyboard focus. The opt-in native_pointer scenario also moves
the real pointer and sends OS keys; it requires macOS Accessibility permission and is excluded
from the default sweep.

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

    def __init__(self, players, split=True, pick=False, seed=None, args=(), ready=True, home=None, saved_split=False, **env):
        """`pick`: the others stop at the profile picker (`HORIZON_SPLIT_PICK`) instead of
        joining as P2, P3...; `seed(home)` fills the game folder first; `args` go to the game;
        `ready=False`: return once P1's game answers, before it is in a world."""
        self.home = Path(home) if home is not None else Path(tempfile.mkdtemp(prefix='horizon-split-e2e-'))
        self.home.mkdir(parents=True, exist_ok=True)
        if seed:
            seed(self.home)
        if saved_split:
            (self.home / 'horizon-extras.txt').write_text('splitscreen:true\n')
        self.sockets = Path(tempfile.mkdtemp(prefix='hzse-'))  # short: Unix socket paths
        if pick:
            env['HORIZON_SPLIT_PICK'] = '1'
        self.env = lab.game_env(HORIZON_HOME=self.home, HORIZON_SPLIT=str(players), **env)
        if not split:
            self.env.pop("HORIZON_SPLIT", None)
        try:
            launched = self.ctl(1, 'launch', *(['--wait'] if ready else []), '--world', 'Split', '--creative',
                     '--resources', str(RESOURCES), '--width', str(WIDTH), '--height', str(HEIGHT),
                     '--render-distance', '4', '--name', 'P1', '--no-pause', *args)
            self.pid = launched.get('launch', launched)['pid']
            if not ready:
                self.wait(lambda: self.alive(1), "P1's game")
                return
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
        """Click player n's button `label` (exactly, else the first whose label starts with it)
        once it's drawn and enabled: `ui` lists the last drawn frame's widgets, which lag a page
        change (by most of a second on GNOME Wayland)."""
        found = []
        def ready():
            widgets = [w for w in self.ctl(n, 'ui')['widgets'] if w.get('enabled')]
            exact = [w['label'] for w in widgets if w.get('label') == label]
            prefix = [w['label'] for w in widgets if w.get('label', '').startswith(label)]
            found[:] = (exact or prefix)[:1]
            return bool(found)
        self.wait(ready, f'P{n} button {label!r}')
        self.ctl(n, 'click-button', found[0])

    def labels(self, n=1):
        return [w.get('label', '') for w in self.ctl(n, 'ui')['widgets']]

    def page(self, n=1):
        """Open player n's Splitscreen Setup screen (Horizon Settings > Splitscreen...), on or
        off; its widgets."""
        def shown():
            labels = self.labels(n)
            return 'Done' in labels and any(l in labels for l in ['Identify', 'Splitscreen: Off', 'Splitscreen: On'])
        if shown():
            return self.ctl(n, 'ui')['widgets']
        self.ctl(n, 'options', 'horizon')
        self.press(n, 'Splitscreen...')
        self.wait(shown, 'the Setup screen', seconds=5)
        return self.ctl(n, 'ui')['widgets']

    def setup(self, n=1):
        """Open player n's Splitscreen Setup screen, splitscreen on and another player in."""
        self.page(n)
        self.wait(lambda: 'Apply' in self.labels(n), 'the Setup screen', seconds=5)

    def play_on(self, player, where, by=1):
        """Move player `player` to `where` ('Window 2', 'New window', 'Fill Monitor 2') on player
        `by`'s Setup screen and Apply; a window not open yet is a new one."""
        self.setup(by)
        self.press(by, f'P{player}')
        if where.startswith('Window '):
            # One window button cycles through the windows: on until it names this one.
            for _ in range(16):
                windows = [l for l in self.labels(by) if l.startswith('Window ')]
                if not windows or where in windows:
                    break
                self.press(by, windows[0])
        self.press(by, where if where in self.labels(by) else 'New window')
        self.press(by, 'Apply')
        self.close_page(by)

    def close_page(self, n=1):
        """Back to playing: Esc through the options and the pause menu."""
        for _ in range(6):
            if self.ctl(n, 'ui').get('screen') not in ('options', 'paused'):
                return
            self.ctl(n, 'key', 'esc', 'tap')
            time.sleep(0.5)

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


def scenario_controls():
    print('controls: the 2-sharing layout square, dragging a player between windows, Names, Identify')
    g = Game(2, saved_split=True)
    def widgets():
        return g.ctl(1, 'ui')['widgets']
    def labels():
        return [w['label'] for w in widgets()]
    def enabled(label):
        return next(w['enabled'] for w in widgets() if w['label'] == label)
    def centre(label):
        x, y, w, h = next(w['bounds'] for w in widgets() if w['label'] == label)
        return x + w / 2, y + h / 2
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 playing')
        half = g.size(1)
        whole = (2 * half[0], half[1])
        g.ctl(1, 'options', 'horizon')
        g.press(1, 'Splitscreen...')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'active', 'Setup active')
        # Two sharing a window: Auto (side by side here), Side by side, Top and bottom.
        g.press(1, '2 sharing: Auto')
        g.press(1, '2 sharing: Side by side')
        g.press(1, 'Apply')
        stacked = (whole[0], half[1] // 2)
        check('top and bottom', g.settle([1, 2], [stacked, stacked]), [stacked, stacked])
        g.press(1, '2 sharing: Top and bottom')
        g.press(1, 'Apply')
        check('auto: side by side again', g.settle([1, 2], [half, half]), [half, half])
        # Drag P2 into a window of its own, then onto P1's tile: back in Window 1.
        g.press(1, 'P2')
        g.press(1, 'New window')
        g.press(1, 'Apply')
        check('P2 in its own window', g.settle([1], [whole]), [whole])
        g.wait(lambda: 'P2' in labels() and not enabled('Apply'), 'the applied map')
        (x2, y2), (x1, y1) = centre('P2'), centre('P1')
        g.ctl(1, 'drag-at', str(x2), str(y2), str((x1 + x2) / 2), str((y1 + y2) / 2), str(x1), str(y1))
        g.wait(lambda: enabled('Apply'), 'the drop makes a draft')
        g.press(1, 'Apply')
        check('dragged onto P1: both in Window 1', g.settle([1, 2], [half, half]), [half, half])
        # Names on and off (saved), Identify.
        g.press(1, 'Names: On')
        g.wait(lambda: 'Names: Off' in labels(), 'Names: Off')
        check('Names turns off', 'Names: Off' in labels(), True)
        g.press(1, 'Names: Off')
        g.wait(lambda: 'Names: On' in labels(), 'Names: On')
        g.press(1, 'Identify')
        g.press(1, 'Done')
    finally:
        g.quit()


def scenario_three():
    print('three: the three layouts')
    g = Game(3, saved_split=True)
    try:
        w2, h = g.size(2)  # one wide on top by default: P2 has a quarter
        layouts = [
            ('one tall on the left', [(w2, 2 * h), (w2, h), (w2, h)]),
            ('side by side', None),
            ('one wide on top', [(2 * w2, h), (w2, h), (w2, h)]),
        ]
        for name, want in layouts:
            g.setup()
            g.press(1, '3 sharing:')
            g.press(1, 'Apply')
            g.close_page()
            got = g.settle([1, 2, 3], want) if want else g.settle([1, 2, 3], [g.size(1)] * 3)
            if want:
                check(name, got, want)
            else:
                widths = [x for x, _ in got]
                check(f'{name}: three columns, full height',
                      max(widths) - min(widths) <= 1 and all(y == 2 * h for _, y in got), True)
    finally:
        g.quit()


def scenario_live():
    print('live: splitscreen off and on in a world')
    g = Game(2, saved_split=True)
    try:
        half = g.size(1)
        # With another player in, the switch is greyed: it turns off once P1 plays alone.
        enabled = [w.get('enabled') for w in g.page() if w.get('label') == 'Splitscreen: On']
        check('others in: the switch is greyed', enabled, [False])
        g.close_page()
        g.ctl(2, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(2), "P2's pause menu", seconds=5)
        g.press(2, 'Leave')
        g.wait(lambda: not g.alive(2), 'P2 gone', seconds=10)
        # Reopen repeatedly after the compositor was removed: Windows must refresh monitor
        # handles from P1's window even with no compositor surface left.
        for _ in range(3):
            g.page()
            check('alone: Setup shows idle instead of window previews',
                  (g.ctl(1, 'state').get('menu') or {}).get('status'), 'idle')
            check('idle: no player tile to drag', 'P1' in g.labels(), False)
            g.close_page()
        g.page()
        g.ctl(1, 'click-button', 'Splitscreen: On')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'off', 'off indicator')
        g.close_page()
        full = g.settle([1], [(2 * half[0], half[1])])
        check('off: P1 has the whole window', full, [(2 * half[0], half[1])])
        g.page()
        g.ctl(1, 'click-button', 'Splitscreen: Off')
        g.wait(lambda: (g.ctl(1, 'state').get('menu') or {}).get('status') == 'idle', 'idle again without a compositor')
        g.close_page()
        time.sleep(1)
        status = g.ctl(1, 'status')
        check('on again: P1 still renders the whole window', tuple(status['renderer']['size']),
              (2 * half[0], half[1]))
        check('on again: frames keep coming', status['fps'] > 10, True)
    finally:
        g.quit()


def scenario_profile_availability():
    print('profile_availability: long profile lists and a profile freed while another picker stays open')
    def seed(home):
        for name in ['Ada', 'Bea', 'Cal', 'Dee', 'Eve', 'Fay', 'Gia', 'Hal', 'Ivy']:
            (home / 'splitscreen' / 'profiles' / name).mkdir(parents=True)
    g = Game(3, pick=True, seed=seed)
    try:
        menu = lambda n: (g.ctl(n, 'state').get('menu') or {})
        g.wait(lambda: all(menu(n).get('mode') == 'splitscreen-profiles' for n in [2, 3]),
               'both pickers')
        g.press(2, 'Next')
        g.press(2, 'Next')
        g.press(2, 'Ivy')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Ivy playing')
        g.wait(lambda: 'Ivy' in menu(3).get('taken', []), 'Ivy disabled in the open picker')
        check('the ninth profile joins', True, True)
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'Ivy leaving')
        g.wait(lambda: 'Ivy' not in menu(3).get('taken', []), 'Ivy available without reopening')
        g.press(3, 'Next')
        g.press(3, 'Next')
        g.press(3, 'Ivy')
        g.wait(lambda: g.ctl(3, 'state').get('player') is not None, 'Ivy joining again')
        check('the open picker can select the freed profile', True, True)
    finally:
        g.quit()


def scenario_gui_scales():
    print('gui_scales: each player keeps its own GUI scale across turns and settings changes')
    # Room for scale 2 in a half (vanilla's 320 x 240 minimum) on a 100% display too.
    g = Game(2, args=('--width', '2000', '--height', '1200'))
    try:
        g.ctl(1, 'options-set', 'guiScale', '1')
        g.ctl(2, 'options-set', 'guiScale', '2')
        for n in [1, 2]:
            g.ctl(n, 'key', 'esc', 'tap')
        def sizes():
            return [g.ctl(n, 'ui')['gui_size'] for n in [1, 2]]
        g.wait(lambda: sizes()[0][0] == 2 * sizes()[1][0], 'independent GUI scales')
        check('P1 GUI twice as wide at scale 1', sizes()[0][0], 2 * sizes()[1][0])
        g.ctl(2, 'options-set', 'guiScale', '1')
        g.wait(lambda: sizes()[0] == sizes()[1], 'P2 scale changed independently')
        check('equal scales give equal GUI sizes', sizes()[0], sizes()[1])
    finally:
        g.quit()


def scenario_fullscreen():
    print('fullscreen: a profile that was fullscreen joins')

    def seed(home):
        profile = home / 'splitscreen/profiles/P2'
        profile.mkdir(parents=True)
        (profile / 'options.txt').write_text('fullscreen:true\n')

    g = Game(2, saved_split=True, seed=seed)
    try:
        time.sleep(3)
        value = g.ctl(1, 'options-get', 'fullscreen')['options']['fullscreen']['value']
        check("P1's window stays windowed (P1's fullscreen option)", value, False)
        # P2's F11 in a window of its own: that window, and only it, goes fullscreen and back.
        g.play_on(2, 'Window 2')
        windowed = g.size(2)
        g.ctl(2, 'key', 'f11', 'tap')
        try:
            g.wait(lambda: g.size(2)[0] > windowed[0], "P2's window fullscreen", seconds=10)
        except RuntimeError:
            # macOS ignores a fullscreen request while another window's transition runs: again.
            g.ctl(2, 'key', 'f11', 'tap')
            g.wait(lambda: g.size(2)[0] > windowed[0], "P2's window fullscreen", seconds=20)
        check("P2's F11: its window fills the screen", g.size(2)[0] > windowed[0], True)
        value = g.ctl(1, 'options-get', 'fullscreen')['options']['fullscreen']['value']
        check("P1's window stays windowed", value, False)
        full = g.size(2)
        time.sleep(2)  # macOS ignores a change while its fullscreen animation runs
        g.ctl(2, 'key', 'f11', 'tap')
        try:
            g.wait(lambda: g.size(2) == windowed, "P2's window back", seconds=10)
        except RuntimeError:
            print(f'    (windowed {windowed}, fullscreen {full}, now {g.size(2)})')
        check("P2's F11 again: back to its window", g.size(2), windowed)
    finally:
        g.quit()


def scenario_leave():
    print("leave: from the pause menu, or switch profile from Splitscreen Settings")
    g = Game(3, saved_split=True)
    try:
        def settings(n):
            """Player n's pause menu, then its Splitscreen Settings."""
            g.ctl(n, 'key', 'esc', 'tap')
            g.wait(lambda: 'Splitscreen Settings' in g.labels(n), f"P{n}'s pause menu", seconds=5)
            g.press(n, 'Splitscreen Settings')

        def menu(n):
            return (g.ctl(n, 'state').get('menu') or {}).get('mode')
        g.ctl(3, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(3), "P3's pause menu", seconds=5)
        g.press(3, 'Leave')
        g.wait(lambda: not g.alive(3), 'P3 gone', seconds=10)
        w, h = g.size(1)
        check('Leave: P3 left, P1 and P2 share the window', g.settle([1, 2], [(w, h), (w, h)]), [(w, h), (w, h)])
        # P2's Splitscreen Settings: its profiles, still playing beneath.
        settings(2)
        g.wait(lambda: menu(2) == 'splitscreen-profiles', "P2's profiles", seconds=10)
        check('P2 still plays while it picks', g.ctl(2, 'state').get('player') is not None, True)
        check('its own profile marked, not pickable',
              [x.get('enabled') for x in g.ctl(2, 'ui')['widgets'] if x.get('label') == 'P2 - you'], [False])
        # Cancel: nothing changed.
        g.press(2, 'Cancel')
        time.sleep(1)
        check('Cancel: P2 plays on as P2', (g.alive(2), menu(2), g.size(2)), (True, None, (w, h)))
        g.close_page(2)
        # Again, RB to a new window, then a new profile: P2 comes back as Cam, there.
        settings(2)
        g.wait(lambda: menu(2) == 'splitscreen-profiles', "P2's profiles", seconds=10)
        g.ctl(2, 'pad', 'rb', 'tap')
        whole = (2 * w, h)
        check('RB on the page: P2 to a new window, P1 alone', g.settle([1], [whole]), [whole])
        g.press(2, 'New Profile...')
        g.ctl(2, 'type', 'Cam')
        g.press(2, 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None and menu(2) is None, 'Cam in the world',
               seconds=20)
        check('Cam plays in the new window', (g.size(1), g.size(2) != (w, h)), (whole, True))
        # P1's Splitscreen Settings is the Setup screen.
        settings(1)
        g.wait(lambda: 'Apply' in g.labels(1), "P1's Setup screen", seconds=5)
        g.press(1, 'Done')
        g.close_page(1)
    finally:
        g.quit()


def scenario_signedin():
    print('signedin: P1 signed into an account, another player joins under its own profile')
    session = Path(tempfile.mkdtemp(prefix='horizon-session-')) / 'session.json'
    session.write_text(json.dumps({'name': 'Signed', 'uuid': '069a79f444e94726a5befca90e38aaf5',
                                   'access_token': 'not-a-real-token', 'user_type': 'msa'}))
    session.chmod(0o600)
    g = Game(2, args=('--session-file', str(session)))
    try:
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'P2 in the world', seconds=20)
        time.sleep(2)
        check('P2 plays', g.ctl(2, 'state').get('player') is not None, True)
        check('P1 still plays (not replaced by its own name joining)',
              g.ctl(1, 'state').get('player') is not None and g.size(1) == g.size(2), True)
    finally:
        g.quit()


def scenario_monitor():
    print('monitor: a player fills another monitor, another joins it (skipped with one monitor)')
    g = Game(2, saved_split=True)
    try:
        half = g.size(1)
        g.setup(1)
        g.press(1, 'P2')
        fill = [l for l in g.labels(1) if l.startswith('Fill Monitor')]
        if not fill:
            print('  skip one monitor only')
            return
        g.press(1, fill[0])
        g.press(1, 'Apply')
        g.close_page(1)
        g.wait(lambda: g.size(2) != half, "P2's window to fill the monitor", seconds=15)
        time.sleep(1)
        size = g.size(2)
        check('P2 fills the monitor: a window of its own', size != g.size(1), True)
        g.play_on(1, 'Window 2')
        # Halves side by side, or top and bottom on a portrait monitor.
        halves = [(size[0] // 2, size[1]), (size[0], size[1] // 2)]
        g.wait(lambda: g.size(1) in halves and g.size(2) == g.size(1), 'P1 beside P2 on the monitor', seconds=15)
        check('P1 joins it: halves of the monitor', g.size(1) in halves and g.size(2) == g.size(1), True)
    finally:
        g.quit()


def scenario_eight():
    print('eight: eight players in one window')
    g = Game(8, saved_split=True)
    try:
        sizes = g.sizes(range(1, 9))
        near = lambda a, b: abs(a - b) <= 1  # thirds of an odd width differ by a pixel
        # Rows of three, three and two: six equal views, then two wider ones; all the same height.
        check(f'rows of 3, 3 and 2: {sizes}',
              all(near(x[0], sizes[0][0]) and near(x[1], sizes[0][1]) for x in sizes[:6])
              and near(sizes[6][0], sizes[7][0]) and sizes[6][0] > sizes[0][0] + 100
              and all(near(x[1], sizes[0][1]) for x in sizes), True)
        drawing = lambda: all(g.ctl(n, 'status')['fps'] > 10 for n in range(1, 9))
        try:
            g.wait(drawing, 'every view drawing', seconds=20)
        except RuntimeError:
            pass
        check('every view draws', drawing(), True)
    finally:
        g.quit()


def scenario_sixteen():
    print('sixteen: sixteen players in a 4x4 grid, P16 moves to its own window and leaves')
    # Room for P1's menus in a sixteenth of the window on a 100% display too.
    g = Game(16, saved_split=True, args=('--width', '2000', '--height', '1200'))
    try:
        players = range(1, 17)
        g.wait(lambda: all(g.ctl(n, 'state').get('player') is not None for n in players),
               'all sixteen players admitted to the world', seconds=60)
        check('all sixteen players enter the world', True, True)
        sizes = g.sizes(players)
        near = lambda a, b: abs(a - b) <= 1
        check('all sixteen views have equal dimensions',
              all(near(w, sizes[0][0]) and near(h, sizes[0][1]) for w, h in sizes), True)
        drawing = lambda: all(g.ctl(n, 'status')['fps'] > 0 for n in players)
        g.wait(drawing, 'all sixteen views drawing', seconds=30)
        check('every player renders', drawing(), True)
        print('  FPS per player:', [round(g.ctl(n, 'status')['fps'], 1) for n in players])
        g.play_on(16, 'New window')
        check('P16 gets its own larger view', g.size(16)[0] > sizes[15][0], True)
        g.ctl(16, 'disconnect')
        g.wait(lambda: not g.alive(16), 'P16 leaving')
        check('P15 still plays after P16 leaves', g.ctl(15, 'state').get('player') is not None, True)
    finally:
        g.quit()


def scenario_pointer():
    print('pointer: only the player with the keyboard and mouse grabs the pointer')
    g = Game(2)
    try:
        time.sleep(3)
        grabbed = lambda n: g.ctl(n, 'state').get('pointer_grabbed')  # noqa: E731
        g.ctl(1, 'mouse', 'left', 'click')
        time.sleep(0.5)
        check('P1 plays with the mouse, P2 never grabs it', (grabbed(1), grabbed(2)), (True, False))
        # P2's menus (its controller) leave P1's pointer alone.
        g.ctl(2, 'key', 'e', 'tap')
        time.sleep(0.6)
        g.ctl(2, 'key', 'esc', 'tap')
        time.sleep(0.6)
        check("after P2's inventory: still P1's", (grabbed(1), grabbed(2)), (True, False))
    finally:
        g.quit()


def scenario_pausemenu():
    print("pausemenu: Splitscreen Settings in the pause menu only while another player plays")
    g = Game(2, saved_split=True, pick=True)
    try:
        def paused(n):
            """Player n's pause menu: its buttons, then closed again."""
            g.ctl(n, 'key', 'esc', 'tap')
            g.wait(lambda: 'Options...' in g.labels(n), f"P{n}'s pause menu", seconds=5)
            labels = g.labels(n)
            g.close_page(n)
            return labels
        menu = lambda: (g.ctl(2, 'state').get('menu') or {}).get('mode')  # noqa: E731
        g.wait(lambda: menu() == 'splitscreen-profiles', "P2's picker")
        # P2 still choosing: nobody else plays yet, so the pause menu has no Splitscreen Settings;
        # Setup stays reachable from Horizon Settings.
        check('alone: no Splitscreen Settings in the pause menu', 'Splitscreen Settings' in paused(1), False)
        g.page(1)
        check('alone: Horizon Settings > Splitscreen... still there', 'Done' in g.labels(1), True)
        g.close_page(1)
        g.ctl(2, 'click-button', 'New Profile...')
        g.ctl(2, 'type', 'Dee')
        g.ctl(2, 'click-button', 'Create')
        g.wait(lambda: g.ctl(2, 'state').get('player') is not None, 'Dee in the world')
        time.sleep(1)
        check('joined: in P1\'s pause menu', 'Splitscreen Settings' in paused(1), True)
        check("joined: in Dee's", 'Splitscreen Settings' in paused(2), True)
        # Dee leaves: P1 alone again, the pause menu drops it.
        g.ctl(2, 'key', 'esc', 'tap')
        g.wait(lambda: 'Leave' in g.labels(2), "Dee's pause menu", seconds=5)
        g.press(2, 'Leave')
        g.wait(lambda: not g.alive(2), 'Dee gone', seconds=10)
        time.sleep(1)
        check('alone again: no Splitscreen Settings in the pause menu', 'Splitscreen Settings' in paused(1), False)
    finally:
        g.quit()



def scenario_native_pointer():
    print('native_pointer: OS keys and mouse drive the owning view across joins, menus and handoffs')
    if sys.platform != 'darwin':
        raise RuntimeError('native_pointer requires macOS; portable capture tests run separately')
    from native_input import MacInput
    g = Game(2, saved_split=True)
    native = None
    try:
        native = MacInput(g.pid)
        g.wait(lambda: all(g.ctl(n, 'state').get('player') for n in [1, 2]), 'both players loaded')
        native.activate()
        size = (g.size(1)[0] + g.size(2)[0], g.size(1)[1])
        point = lambda x, y: native.point(size, x, y)
        state = lambda n: g.ctl(n, 'state')
        grabbed = lambda: [state(n)['pointer_grabbed'] for n in [1, 2]]
        check('P1 keeps capture after P2 joins without another click', grabbed(), [True, False])
        # OS mouse motion, unlike horizon-ctl mouse-move, passes through Split.device_event.
        def look(owner):
            other = 3 - owner
            before = [state(n)['player']['yaw'] for n in [1, 2]]
            for _ in range(3):
                native.relative(20, 0)
            time.sleep(0.3)
            after = [state(n)['player']['yaw'] for n in [1, 2]]
            check(f'native mouse turns P{owner}', abs(after[owner-1] - before[owner-1]) > 0.1, True)
            check(f'P{other} does not turn with P{owner} mouse', after[other-1], before[other-1])
        look(1)
        g.ctl(1, 'command', 'gamemode spectator @a')
        g.ctl(1, 'command', 'tp P1 0 150 0 0 0')
        g.ctl(1, 'command', 'tp P2 20 150 0 0 0')
        time.sleep(0.5)

        def walk(owner):
            other = 3 - owner
            # Spectator movement decelerates after a release; measure fresh movement
            # after that inertia settles, while checking held keys independently.
            def settled():
                before = state(other)['player']['position']
                time.sleep(0.15)
                after = state(other)['player']['position']
                return all(abs(after[i] - before[i]) < 0.001 for i in [0, 2])
            g.wait(settled, f"P{other}'s previous movement settles", seconds=5)
            before = [state(n)['player']['position'] for n in [1, 2]]
            native.key('w', True)
            try:
                g.wait(lambda: 'KeyW' in g.ctl(owner, 'status')['keys'], f'OS W reaches P{owner}')
                check(f'OS W reaches only P{owner}', 'KeyW' in g.ctl(other, 'status')['keys'], False)
                time.sleep(0.4)
            finally:
                native.key('w', False)
            time.sleep(0.3)
            after = [state(n)['player']['position'] for n in [1, 2]]
            moved = lambda n: sum((after[n-1][i] - before[n-1][i]) ** 2 for i in [0, 2]) > 0.01
            check(f'P{owner} actually moves with OS W', moved(owner), True)
            check(f'P{other} stays put', moved(other), False)

        walk(1)
        g.ctl(2, 'pad', 'start', 'tap')  # A controller player's pause menu.
        walk(1)
        check("P2's menu leaves native capture with P1", grabbed(), [True, False])
        look(1)
        g.ctl(2, 'pad', 'start', 'tap')
        # A click on P2 cannot steal a locked P1 pointer.
        native.click(point(0.75, 0.4))
        check('clicking another view while captured keeps P1 ownership', grabbed(), [True, False])
        native.tap('escape')
        native.click(point(0.75, 0.4))
        check('Esc then click hands native capture to P2', grabbed(), [False, True])
        walk(2)
        look(2)
        # Transfer while W is still physically held: the old player must not keep walking.
        native.key('w', True)
        g.wait(lambda: 'KeyW' in g.ctl(2, 'status')['keys'], 'P2 holds W')
        native.tap('escape')
        native.click(point(0.25, 0.1))
        native.tap('escape')  # Close P1's own pause menu.
        check('handoff releases the previous owners held W', 'KeyW' in g.ctl(2, 'status')['keys'], False)
        native.key('w', False)
        walk(1)
        check('P1 recaptures after the round trip', grabbed(), [True, False])
        # Separate native windows keep the same ownership rules and deliver
        # key releases to the old player when its window loses the keyboard.
        g.play_on(2, 'New window')
        native.activate()
        for _ in range(3):
            native.relative(15, 0)
        check('moving P2 to another window keeps P1 capture', grabbed(), [True, False])
        walk(1)
        native.tap('escape')
        native.click(native.point(g.size(2), 0.5, 0.2, title='Window 2'), title='Window 2')
        native.tap('escape')  # P2 kept its own pause menu from the previous handoff.
        check('clicking Window 2 gives P2 native capture', grabbed(), [False, True])
        walk(2)
        look(2)
        native.tap('escape')
        native.click(native.point(g.size(1), 0.5, 0.1, title='Window 1'), title='Window 1')
        native.tap('escape')
        check('clicking Window 1 restores P1 capture', grabbed(), [True, False])
        walk(1)
        g.ctl(2, 'disconnect')
        g.wait(lambda: not g.alive(2), 'P2 left')
        time.sleep(0.3)
        before = state(1)['player']['yaw']
        for _ in range(3):
            native.relative(15, 0)
        time.sleep(0.3)
        check('native mouse still turns P1 after the compositor is removed',
              abs(state(1)['player']['yaw'] - before) > 0.1, True)
    finally:
        if native is not None:
            native.release()
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
    'controls': scenario_controls,
    'three': scenario_three,
    'live': scenario_live,
    'profile_availability': scenario_profile_availability,
    'gui_scales': scenario_gui_scales,
    'fullscreen': scenario_fullscreen,
    'leave': scenario_leave,
    'signedin': scenario_signedin,
    'monitor': scenario_monitor,
    'eight': scenario_eight,
    'sixteen': scenario_sixteen,
    'pointer': scenario_pointer,
    'native_pointer': scenario_native_pointer,
    'pausemenu': scenario_pausemenu,
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
    for name in args.only or [n for n in SCENARIOS if n != 'native_pointer']:
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
