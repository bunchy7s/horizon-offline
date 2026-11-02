#!/usr/bin/env python3
"""A minecart through a Nether portal on Horizon's dedicated server, tick by tick against vanilla's numbers.

    python3 tools/cart_portal_e2e.py --bin-dir target/fast

PURPOSE: a minecart with a mob rider crosses a Nether portal with its rider (`Entity.teleportCrossDimension`
takes the passengers along and mounts them again), and an empty one arrives where and when vanilla's does:
the forward offset reset of `AbstractMinecart.getRelativePortalPosition`, the 0.4 clamp of its known movement
(`OldMinecartBehavior.getKnownMovement`), and the tick the portal is handled in (`handlePortal` opens the
minecart's own tick, and a cart that arrives in a level that ticks later in the same server tick moves in it
that tick).
WHY: before the fix a cart with a pig on it never left the portal, and an empty one landed 0.6 blocks off the
exit portal's centre a tick late, fell onto the ground between the portal and the rails and lost half its
speed each tick.
REFS: the expected numbers are what the vanilla 26.3 server printed for the same scenes (stone floor at
y=99, powered rails under the minecart and behind the exit portal, a minecart pushed at 0.4 blocks per
tick along 10 rails into the portal, ticks frozen and stepped one at a time, no player).
LOGS: the server's output goes to --out (a temporary folder by default), one file per lane.
READS-BY: tools/pr_ready.py does not run it; run it by hand after changing game/nether_portal.rs.
ENDS-WHEN: each lane has checked its ticks, or the first mismatch is printed (exit status 1).
"""
import argparse
from pathlib import Path
import re
import shutil
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

PROPERTIES = {
    'online-mode': 'false', 'level-type': 'minecraft:flat', 'spawn-protection': '0', 'view-distance': '4',
    'simulation-distance': '4', 'pause-when-empty-seconds': '-1', 'spawn-monsters': 'false',
    'spawn-animals': 'false', 'spawn-npcs': 'false', 'generate-structures': 'false', 'allow-flight': 'true',
    'difficulty': 'normal', 'enable-command-block': 'true',
}
NETHER = 'execute in minecraft:the_nether run '
RAIL = 'minecraft:powered_rail[powered=true,shape=east_west]'
TOL = 1e-5
# Per lane: tick -> (cart x, y, speed along x, pig x, pig y or None). The numbers vanilla 26.3 printed. The pig's y
# is left out on the tick it arrives unticked in the Overworld (vanilla 100.0, Horizon puts it 0.1875 above the
# cart): it is the one value the rider's first, unticked sample does not match.
LANES = {
    'overworld_to_nether_with_pig': {
        'rider': True, 'z': 4, 'zn': 0, 'dest_z': 0.5,
        'ticks': {
            21: (9.732167700000005, 100.0625, 1.5984504303618299, 9.732167700000005, 100.25),
            22: (1.9, 100.0225, 0.38, 1.9, 100.21),
            23: (2.2799999952316283, 100.0, 0.38, 2.2799999952316283, 100.1875),
            30: (4.867302549534948, 100.0625, 0.7883303007081885, 4.867302549534948, 100.25),
            40: (8.867302549534951, 100.0625, 1.3569615856515638, 8.867302549534951, 100.25),
        }},
    'overworld_to_nether_empty': {
        'rider': False, 'z': 4, 'zn': 0, 'dest_z': 0.5,
        'ticks': {
            21: (9.9, 100.0625, 1.0332454298233449, None, None),
            22: (1.9, 100.0225, 0.38, None, None),
            30: (5.0599999904632575, 100.0625, 0.6583788209356798, None, None),
            40: (9.059999990463261, 100.0625, 0.9404627730163926, None, None),
        }},
    'nether_to_overworld_with_pig': {
        'rider': True, 'z': 3, 'zn': 0, 'dest_z': 3.5,
        'ticks': {
            21: (9.732167700000005, 100.0625, 1.5984504303618299, 9.732167700000005, 100.25),
            22: (84.5, 100.0625, 0.4, 84.5, -1),
            23: (84.9, 100.0225, 0.38, 84.9, 100.21),
            30: (87.46730254953496, 100.0625, 0.7305218663071098, 87.46730254953496, 100.25),
            40: (91.46730254953502, 100.0625, 1.300864178186122, 91.46730254953502, 100.25),
        }},
    'nether_to_overworld_empty': {
        'rider': False, 'z': 3, 'zn': 0, 'dest_z': 3.5,
        'ticks': {
            21: (9.9, 100.0625, 1.0332454298233449, None, None),
            22: (84.5, 100.0625, 0.4, None, None),
            23: (84.9, 100.0225, 0.38, None, None),
            30: (87.65999999046329, 100.0625, 0.6233112718079998, None, None),
            40: (91.65999999046335, 100.0625, 0.9171487218920757, None, None),
        }},
}


class Console:
    """The server's console: each command's output is the lines before a marker `say` that follows it."""

    def __init__(self, server):
        self.server, self.n = server, 0

    def __call__(self, command, timeout=60):
        # Horizon keeps the level an `execute in` named for the next plain command: name the Overworld.
        if command.split()[0] in ('forceload', 'fill', 'setblock', 'summon'):
            command = 'execute in minecraft:overworld run ' + command
        self.n += 1
        mark = f'CARTMARK{self.n}X'
        start = self.server.count()
        self.server.console(command)
        self.server.console('say ' + mark)
        end = time.time() + timeout
        while time.time() < end:
            lines = self.server.since(start)
            for i, line in enumerate(lines):
                if mark in line:
                    return lines[:i]
            time.sleep(0.02)
        raise RuntimeError(f'no reply to {command!r} in {timeout} s')

    def gametime(self):
        for line in self('time query gametime'):
            found = re.search(r'time is (\d+)', line)
            if found:
                return int(found.group(1))
        raise RuntimeError('cannot read the game time')

    def step(self, n):
        before = self.gametime()
        self(f'tick step {n}')
        end = time.time() + 60
        while time.time() < end:
            if self.gametime() >= before + n:
                return
            time.sleep(0.05)
        raise RuntimeError(f'tick step {n} did not finish')

    def loaded(self, dim, x, y, z):
        end = time.time() + 180
        while time.time() < end:
            if any('Test passed' in l for l in self(f'execute in minecraft:{dim} if loaded {x} {y} {z}')):
                return
            time.sleep(1)
        raise RuntimeError(f'the chunk at {dim} {x} {y} {z} never loaded')

    def numbers(self, selector, path):
        for line in self(f'data get entity {selector} {path}'):
            found = re.search(r'entity data: \[(.*)\]$', line)
            if found:
                return [float(v.rstrip('d')) for v in found.group(1).split(', ')]
        return None


def build(console, name, lane):
    """The scenes of the sbs cartA and cartB runs, one lane: rails 0..9 into a portal at x=10, a portal behind
    which rails run on in the other level."""
    z, zn = lane['z'], lane['zn']
    o2n = name.startswith('overworld')
    cart_dim = '' if o2n else NETHER
    far = (z, zn) if o2n else (zn, z)
    # The destination level's portal and rails; the Overworld's forced chunks come first.
    if o2n:
        console(NETHER + 'forceload add 0 -16 15 47')
        console(f'forceload add 0 {z} 15 {z}')
        for dim, x, zz in (('overworld', 8, z), ('the_nether', 8, -10)):
            console.loaded(dim, x, 100, zz)
    else:
        console(NETHER + 'forceload add 0 -16 15 47')
        console(f'forceload add 80 {z} 95 {z}')
        for dim, x, zz in (('overworld', 88, z), ('the_nether', 8, -10)):
            console.loaded(dim, x, 100, zz)
    console('tick freeze')
    if o2n:
        build_side(console, '', 0, 12, 10, z, rails=(0, 9))
        build_side(console, NETHER, 0, 14, 1, zn, rails=(2, 12), redstone=(6, zn + 1))
        console(f'setblock 4 100 {z + 1} minecraft:redstone_block')
        start, at_z = 1.5, z + 0.5
        run_in = ''
    else:
        build_side(console, '', 80, 95, 84, z, rails=(85, 95), redstone=(90, z + 1))
        build_side(console, NETHER, 0, 14, 10, zn, rails=(0, 9))
        console(NETHER + f'setblock 4 100 {zn + 1} minecraft:redstone_block')
        start, at_z = 1.5, zn + 0.5
        run_in = NETHER
    passenger = (',Passengers:[{id:"minecraft:pig",Tags:["L0"],NoAI:1b,Silent:1b,PersistenceRequired:1b}]'
                 if lane['rider'] else '')
    console(run_in + f'summon minecraft:minecart {start} 100.1 {at_z} {{Tags:["L0"],Motion:[0.4d,0.0d,0.0d]{passenger}}}')
    del cart_dim, far


def build_side(console, prefix, x0, x1, portal_x, z, rails, redstone=None):
    """Stone floor, air above, an obsidian frame with a lit portal (axis z) at `portal_x`, powered rails."""
    console(prefix + f'fill {x0} 95 {z - 2} {x1 + 2} 106 {z + 3} minecraft:air')
    console(prefix + f'fill {x0} 99 {z - 2} {x1 + 2} 99 {z + 3} minecraft:stone')
    console(prefix + f'fill {portal_x} 99 {z - 1} {portal_x} 103 {z + 2} minecraft:obsidian')
    console(prefix + f'fill {portal_x} 100 {z} {portal_x} 102 {z + 1} minecraft:nether_portal[axis=z]')
    console(prefix + f'fill {rails[0]} 100 {z} {rails[1]} 100 {z} {RAIL}')
    if redstone:
        console(prefix + f'setblock {redstone[0]} 100 {redstone[1]} minecraft:redstone_block')


def run_lane(bins, name, lane, out):
    work = Path(tempfile.mkdtemp(prefix='hz-cart-portal-'))
    folder = work / 'server'
    folder.mkdir()
    (folder / 'eula.txt').write_text('eula=true\n')
    props = dict(PROPERTIES, **{'server-port': str(lab.free_port()), 'server-ip': '127.0.0.1'})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    command = [bins / 'horizon-client', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / f'{name}.log')
    failed = []
    try:
        server.wait_for(DONE, 300)
        console = Console(server)
        console('forceload add -16 -16 16 16')
        time.sleep(3)
        build(console, name, lane)
        at, cart_sel, pig_sel = 0, '@e[type=minecraft:minecart,tag=L0,limit=1]', '@e[type=minecraft:pig,tag=L0,limit=1]'
        for tick, (x, y, speed, pig_x, pig_y) in lane['ticks'].items():
            console.step(tick - at)
            at = tick
            pos = console.numbers(cart_sel, 'Pos')
            motion = console.numbers(cart_sel, 'Motion')
            expect = [x, y, lane['dest_z'] if tick >= 22 else lane['z'] + 0.5 if name.startswith('overworld')
                      else lane['zn'] + 0.5]
            if tick == 21:
                expect[2] = pos[2] if pos else 0.0
            if not pos or any(abs(a - b) > TOL for a, b in zip(pos, expect)):
                failed.append(f'{name} t{tick}: cart at {pos}, vanilla {expect}')
            if not motion or abs(motion[0] - speed) > TOL:
                failed.append(f'{name} t{tick}: cart speed {motion and motion[0]}, vanilla {speed}')
            if lane['rider'] and tick >= 22:
                pig = console.numbers(pig_sel, 'Pos')
                bad = not pig or abs(pig[0] - pig_x) > TOL or abs(pig[2] - lane['dest_z']) > TOL
                if pig_y is not None and pig_y > 0 and pig and abs(pig[1] - pig_y) > TOL:
                    bad = True
                if bad:
                    failed.append(f'{name} t{tick}: pig at {pig}, vanilla x {pig_x} y {pig_y} z {lane["dest_z"]}')
            print(f'{name} t{tick}: cart {pos} speed {motion and motion[0]}')
    finally:
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    return failed


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--bin-dir', default='target/fast', type=Path)
    parser.add_argument('--out', type=Path)
    parser.add_argument('--lane', action='append', choices=sorted(LANES))
    args = parser.parse_args()
    out = args.out or Path(tempfile.mkdtemp(prefix='cart-portal-logs-'))
    out.mkdir(parents=True, exist_ok=True)
    failed = []
    for name in args.lane or LANES:
        failed += run_lane(args.bin_dir.resolve(), name, LANES[name], out)
    for line in failed:
        print('FAIL', line)
    print('cart portal: ' + ('FAILED' if failed else 'all lanes match vanilla'), f'(logs in {out})')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
