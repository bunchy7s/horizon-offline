#!/usr/bin/env python3
"""A player riding a minecart (or a boat) through a Nether portal, on vanilla's dedicated server and Horizon's.

    python3 tools/cart_portal_player_e2e.py --bin-dir target/fast [--side vanilla|horizon] [--vehicle minecart|boat]

PURPOSE: a vanilla-codec client (VanillaProtocolClient in its ride mode) joins a flat world, the console puts it
in a minecart pushed at 0.4 along 10 rails into a Nether portal (powered rails behind the exit portal), freezes
the ticks and steps them one at a time. Per tick the harness prints what the server holds (the vehicle's
position and speed, the player's level and vehicle) and puts a chat line "RIDE T<n>" in the client's stream, so
the client's transcript (RideTranscript: respawn, spawn, passengers, motion, teleports) is cut by tick. Horizon
must match vanilla on the tick the vehicle crosses, where it lands, the speed it keeps and the ride link; the
client must see the level change and be riding again.
WHY: a minecart with only a player on it stayed at the portal on Horizon (#6021); mob riders and empty carts
crossed since #6991.
REFS: Entity.handlePortal, Entity.teleportCrossDimension (the passengers are taken off, teleported and put on
the vehicle again), AbstractMinecart.tick, ServerPlayer.teleport; cart_portal_e2e.py for the scene.
LOGS: --out (default a temporary folder): <side>.log (server), <side>-client.log, <side>-client.txt (transcript),
<side>.table (per tick).
READS-BY: run by hand after changing game/nether_portal.rs or game/server_travel.rs.
ENDS-WHEN: both sides ran and each client transcript holds what vanilla sent (exit status 1 if not), or the one side asked for has printed its summary.
difference), or the one side asked for has printed its table.
"""
import argparse
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
import test_rust_parity as base  # noqa: E402
from test_dedicated_server_parity import Server as _Server  # noqa: E402
from test_locator_bar_parity import generate_world, server_dir  # noqa: E402
from cart_portal_e2e import Console, build_side, NETHER, TOL  # noqa: E402

PLAYER = 'Tester'
LANE_Z, DEST_Z = 4, 0.5
TICKS = 24
NAME = 'cart-portal-player'


class Server(_Server):
    """The shared server process, with the line counter the console helper reads."""

    def count(self):
        return len(self.lines)

    def since(self, n):
        return self.lines[n:]


def server_say(console, line):
    console.server.console(f'tellraw @a "{line}"')


def scene(console, vehicle):
    """Overworld: stone floor, rails 1..9 with a stone block behind them, a portal at x=10 (axis z) at z=4.
    Nether: the exit portal at x=1 and powered rails behind it, as in cart_portal_e2e's overworld_to_nether lane.
    The ticks are NOT frozen: a ridden vehicle ticks while the world is frozen, so a frozen scene would run the
    ride in real time and the portal on stepped ticks only."""
    z = LANE_Z
    console(NETHER + 'forceload add 0 -16 15 47')
    console(f'forceload add 0 {z} 15 {z}')
    for dim, x, zz in (('overworld', 8, z), ('the_nether', 8, -10)):
        console.loaded(dim, x, 100, zz)
    build_side(console, '', 0, 12, 10, z, rails=(1, 9))
    build_side(console, NETHER, 0, 14, 1, 0, rails=(2, 12), redstone=(6, 1))
    console(f'setblock 0 100 {z} minecraft:stone')
    # Unpowered rails: the vehicle waits until the player is in it.
    console(f'fill 1 100 {z} 9 100 {z} minecraft:powered_rail[powered=false,shape=east_west]')
    if vehicle == 'minecart':
        console(f'summon minecraft:minecart 1.5 100.1 {z + 0.5} {{Tags:["L0"]}}')
    else:
        console(f'summon minecraft:oak_boat 1.5 100.1 {z + 0.5} {{Tags:["L0"],Rotation:[-90f,0f]}}')
    # The client right-clicks the vehicle, as a player boards one (the console's /ride does not take a server's player).
    console(f'tp {PLAYER} 1.5 100.1 {z - 0.5}')
    time.sleep(2)
    server_say(console, 'RIDE BOARD')
    time.sleep(2)
    # The rails come alive: a vehicle at rest on powered rails is pushed off the stone behind it.
    if vehicle == 'minecart':
        console(f'setblock 4 100 {z + 1} minecraft:redstone_block')
    else:
        # A boat on ice slides on: give it a push along x once the player sits in it.
        console(f'fill 1 99 {z} 9 99 {z} minecraft:blue_ice')
        console(f'data merge entity @e[tag=L0,limit=1] {{Motion:[0.4d,0.0d,0.0d]}}')


def text(console, selector, path):
    for line in console(f'data get entity {selector} {path}'):
        found = re.search(r'entity data: (.*)$', line)
        if found:
            return found.group(1).strip()
    return None


def floats(value):
    if value is None:
        return None
    return [float(v.rstrip('dfb')) for v in re.findall(r'-?[\d.]+(?:E-?\d+)?[dfb]?', value.strip('[]'))]


def run_side(label, command, client_command, build, world, vehicle, out):
    directory = build / label
    port = server_dir(directory, world)
    client_out = out / f'{label}-client.txt'
    client_out.unlink(missing_ok=True)
    server = Server(command, directory, label)
    rows = []
    try:
        server.wait_for(r'Done \(', 300)
        with (out / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), PLAYER, str(client_out), 'ride'],
                                      cwd=lab.ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)
            try:
                server.wait_for(rf'<{PLAYER}> summon the entities', timeout=180)
                console = Console(server)
                console(f'gamemode survival {PLAYER}')
                scene(console, vehicle)
                sel = '@e[tag=L0,limit=1]'
                for tick in range(1, TICKS + 1):
                    time.sleep(0.5)
                    server.console(f'tellraw @a "RIDE T{tick}"')
                    cart = floats(text(console, sel, 'Pos'))
                    motion = floats(text(console, sel, 'Motion'))
                    player = floats(text(console, PLAYER, 'Pos'))
                    dim = text(console, PLAYER, 'Dimension')
                    riding = text(console, PLAYER, 'RootVehicle')
                    rows.append((tick, cart, motion and motion[0], player, dim, riding is not None))
                server.console('tellraw @a "RIDE END"')
                client.wait(timeout=120)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    table = out / f'{label}.table'
    with table.open('w') as f:
        for tick, cart, speed, player, dim, riding in rows:
            f.write(f't{tick} cart {cart} speed {speed} player {player} dim {dim} riding {riding}\n')
    ride = []
    if client_out.exists():
        ride = [l.split(' ', 1)[1] if ' ' in l else l for l in client_out.read_text().splitlines() if l.startswith('play-ride')]
    (out / f'{label}-ride.txt').write_text('\n'.join(ride) + '\n')
    return rows, ride


def check_ride(ride, vehicle):
    """What vanilla 26.3 sent the client for a ridden cart through the portal (its transcript): the level change,
    then the cart added at the exit portal's centre (x 1.5, z 0.5) with motion 0.4 along x, then the player on it.
    Only the minecart is held to these numbers; a boat needs water to reach the portal."""
    problems = []
    if vehicle != 'minecart':
        return problems
    lines = list(ride)
    if 'respawn into the_nether' not in lines:
        return ['the client never saw the level change']
    after = lines[lines.index('respawn into the_nether'):]
    adds = [l for l in after if l.startswith('add minecart1 at ')]
    if not adds:
        return ['the cart did not arrive in the Nether']
    found = re.match(r'add minecart1 at (\S+) (\S+) (\S+) motion \((\S+), (\S+), (\S+)\)', adds[0])
    x, y, z, mx = (float(found.group(i)) for i in (1, 2, 3, 4))
    # Vanilla's client is sent the cart as it stands on arrival (1.5, 100.0625, 0.4) or, when the server ticks it
    # first, after that tick (cart_portal_e2e's tick 22: 1.9, 100.0225, 0.38): either is the same arrival.
    arrivals = ((1.5, 100.0625, 0.4), (1.9, 100.0225, 0.38))
    if abs(z - 0.5) > TOL or not any(abs(x - ax) < 2e-3 and abs(y - ay) < 2e-3 and abs(mx - am) < 2e-3
                                     for ax, ay, am in arrivals):
        problems.append(f'the cart arrived at {x} {y} {z} with motion {mx}; vanilla: {arrivals} at z 0.5')
    if 'passengers of minecart1 [player]' not in after:
        problems.append('the player is not riding the cart after the level change')
    return problems


def compile_bot(build):
    lock, jars, libraries = base.verify_artifacts()
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = lab_path(jars['server'], libraries)
    sources = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, sources],
                   cwd=lab.ROOT, check=True, capture_output=True)
    return dependencies, classes


def lab_path(server_jar, libraries):
    import os
    return os.pathsep.join(map(str, [server_jar, *libraries]))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    parser.add_argument('--bin-dir', default='target/fast', type=Path)
    parser.add_argument('--side', action='append', choices=['vanilla', 'horizon'])
    parser.add_argument('--vehicle', default='minecart', choices=['minecart', 'boat'])
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    import os
    out = args.out or Path(tempfile.mkdtemp(prefix='cart-portal-player-'))
    out.mkdir(parents=True, exist_ok=True)
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    dependencies, classes = compile_bot(build)
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [args.bin_dir.resolve() / 'horizon-client', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', str(classes) + os.pathsep + dependencies,
              'dev.horizonlab.parity.VanillaProtocolClient']
    world = generate_world(vanilla_server, build)
    results = {}
    for label, command in (('vanilla', vanilla_server), ('horizon', horizon_server)):
        if label in (args.side or ['vanilla', 'horizon']):
            results[label] = run_side(label, command, client, build, world, args.vehicle, out)
            print(f'{label}: table {out / (label + ".table")}, client {out / (label + "-ride.txt")}')
    status = 0
    for label, (rows, ride) in results.items():
        for problem in check_ride(ride, args.vehicle):
            status = 1
            print(f'FAIL {label}: {problem}')
    for label, (rows, ride) in results.items():
        after = ride[ride.index('respawn into the_nether'):] if 'respawn into the_nether' in ride else []
        arrival = [l for l in after if 'minecart1' in l or l.startswith('player position')][:4]
        print(f'{label}: after the level change the client saw: ' + ' | '.join(arrival))
    print('cart portal player: ' + ('FAILED' if status else 'the client sees what vanilla sends'), f'(logs in {out})')
    return status


if __name__ == '__main__':
    sys.exit(main())
