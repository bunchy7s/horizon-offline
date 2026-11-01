#!/usr/bin/env python3
"""Play with friends end to end, without the internet.

(a) The code alone: a host Horizon opens a world and shares it (`friends
host`) with no relay anywhere; a guest Horizon joins with the full invite
code (`connect friend:CODE`) by dialling the endpoints in it. (b) A
self-hosted relay run locally (`horizon-relay`): the host also gets a short
code from it, and a guest started with HORIZON_FRIENDS_PATH=relay joins with
the short code through it. Each time both sides must see the other's chat
line, the other moving, a block the host places and the block the guest
breaks; then the host stops sharing and a guest with the old code is told
why it can't join. `--baseline` first runs the same checks over plain LAN.
The router is never asked to open a port (HORIZON_FRIENDS_UPNP=0) and STUN
is off: nothing leaves the machine.

    python3 tools/test_friends.py --client target/fast/horizon-client --relay target/fast/horizon-relay

The host runs on its integrated server thread by default (the window is a
client of it). `--in-frame` is DEPRECATED (never use it, it will be removed soon);
`--server-thread` is accepted for compatibility.

`--role host|guest` runs one side only (two machines, e.g. over a tailnet):
the host prints the code, the guest takes it with `--code` (`--relay-url`
for a relay elsewhere, `--path relay` to force it). Needs a display on Linux
(xvfb-run -a).
"""
import argparse
import json
import math
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

try:
    from . import lab
    from .integration_env import game_environment, require_server_reports
except ImportError:
    import lab
    from integration_env import game_environment, require_server_reports

ROOT = Path(__file__).resolve().parents[1]


def command(endpoint, line, check=True):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(60)
        connection.connect(str(endpoint))
        connection.sendall((line + '\n').encode())
        with connection.makefile('r') as reply:
            result = json.loads(reply.readline())
    if check:
        assert 'error' not in result, (line, result)
    return result


def wait_for(endpoint, line, predicate, timeout=60, what=None):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            last = command(endpoint, line, check=False)
        except (FileNotFoundError, ConnectionRefusedError):
            time.sleep(0.1)
            continue
        try:
            if predicate(last):
                return last
        except (KeyError, TypeError, IndexError):
            pass
        time.sleep(0.1)
    raise AssertionError(f'timed out waiting for {what or line}: {last}')


def health(relay_url):
    with urllib.request.urlopen(f'http://{relay_url}/health', timeout=5) as r:
        return json.loads(r.read())


class Game:
    """A Horizon client with a control socket in its own home."""

    # Extra words before and after the client (`--xvfb`, `--offscreen`).
    prefix = []
    extra = []
    # The host's world on its server thread (`--server-thread`), else in the window.
    server_thread = True

    def __init__(self, client, resources, work, name, env):
        # Either way explicitly, whatever the game's default.
        env = dict(env, HORIZON_SERVER_THREAD='1' if Game.server_thread and name == 'host' else '0')
        self.home = work / name
        self.home.mkdir()
        self.endpoint = self.home / 'c.sock'
        self.log = (self.home / 'client.log').open('w+')
        self.process = subprocess.Popen(Game.prefix + [
            str(client), '--resources', str(resources), '--render-distance', '2', '--test-terrain',
            '--control-socket', str(self.endpoint), '--timeout-seconds', '600',
        ] + Game.extra, cwd=self.home,
            env={**game_environment({**os.environ, **env}, self.home), 'HORIZON_HOME': str(self.home)},
            stdout=self.log, stderr=subprocess.STDOUT)
        wait_for(self.endpoint, 'screen', lambda v: v['screen'] == 'title', 120, f'{name} title')

    def __call__(self, line, check=True):
        return command(self.endpoint, line, check)

    def wait(self, line, predicate, timeout=60, what=None):
        return wait_for(self.endpoint, line, predicate, timeout, what)

    def stop(self):
        if self.process.poll() is None:
            try:
                self('quit', check=False)
                self.process.wait(timeout=20)
            except Exception:
                self.process.terminate()
                try:
                    self.process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.process.kill()
        self.log.seek(0)
        return self.log.read()


def host_world(host):
    host('open Friends 0 creative')
    host.wait('status', lambda v: v['player'] is not None and v['pending_chunks'] == 0, 120, 'host world')
    host('friends host')
    return host.wait('friends status', lambda v: v['code'], 30, 'invite code')['code']


def guest_pos(host, name):
    guests = host('lan status')['guests']
    return next(g['position'] for g in guests if g['name'] == name)


def open_ground(host, guest, name):
    """Puts the guest on a cleared, flat floor beside the host and returns where it stands.

    A player new to the world spawns at a random free spot near the world spawn, as in vanilla
    (`PlayerSpawnFinder`), often against a hill: walking and aiming need known, open ground.
    Commands, not the control socket: the server has the last word on where the guest stands.
    """
    hp = host('status')['player']['position']
    x, y, z = math.floor(hp[0]) - 6, math.floor(hp[1]), math.floor(hp[2])
    host(f'command fill {x - 2} {y - 1} {z - 2} {x + 4} {y - 1} {z + 8} minecraft:stone')
    host(f'command fill {x - 2} {y} {z - 2} {x + 4} {y + 3} {z + 8} minecraft:air')
    spot = [x + 0.5, float(y), z + 0.5]
    host(f'command tp {name} {spot[0]} {spot[1]} {spot[2]} 0 0')
    guest.wait('status', lambda v: math.dist(v['player']['position'], spot) < 0.01 and v['player']['on_ground'], 20,
               'the guest on its ground')
    return spot


def play_together(host, guest, name, round_index):
    """Chat, movement and blocks seen from both sides."""
    host.wait('lan status', lambda v: any(g['name'] == name and g['placed'] for g in v['guests']), 60,
              'the guest in the host world')
    guest.wait('status', lambda v: v['server']['joined'] and v['player'] is not None, 60, 'guest joined')
    # Chat both ways.
    guest(f'chat hello from {name} {round_index}')
    host.wait('chat-log 20', lambda v: any(f'hello from {name} {round_index}' in l for l in v['chat']), 30,
              'guest chat on the host')
    host(f'chat hello from the host {round_index}')
    guest.wait('status', lambda v: any(f'hello from the host {round_index}' in l for l in v['server']['chat']), 30,
               'host chat on the guest')
    # The guest walks; the host sees it move.
    open_ground(host, guest, name)
    before = guest_pos(host, name)
    guest('look 0 0')
    guest('key w down')
    try:
        host.wait('lan status', lambda v: math.dist(before, next(g['position'] for g in v['guests'] if g['name'] == name)) > 1.0,
                  20, 'guest movement on the host')
    finally:
        guest('key w up')
    # The host moves; the guest sees the host's player there.
    hp = host('status')['player']['position']
    target = [hp[0] + 3.0, hp[1], hp[2] + 1.0]
    # The /tp command, not the control socket's local move: on the server thread the window is a
    # client, and its server has the last word on where the host stands.
    host(f'command tp @s {target[0]} {target[1]} {target[2]}')
    guest.wait('status', lambda v: any(math.dist(p, target) < 0.6 for p in v['server']['player_positions']), 20,
               'host movement on the guest')
    # A block the host places, which the guest sees and then breaks.
    gp = guest.wait('status', lambda v: v['player']['on_ground'] and math.hypot(v['player']['velocity'][0],
                    v['player']['velocity'][2]) < 1e-3, 10, 'the guest standing still')['player']['position']
    feet = [math.floor(gp[0]), math.floor(gp[1]), math.floor(gp[2])]
    block = [feet[0] + 2, feet[1] + 1, feet[2]]
    # Clear the line of sight (the test terrain has hills), then the block at eye level.
    # As commands, for the same reason as the /tp above.
    host(f'command fill {feet[0] + 1} {feet[1]} {feet[2]} {feet[0] + 1} {feet[1] + 2} {feet[2]} minecraft:air')
    host(f'command setblock {block[0]} {block[1]} {block[2]} minecraft:gold_block')
    guest.wait(f'getblock {block[0]} {block[1]} {block[2]}', lambda v: v['block'] == 'minecraft:gold_block', 20,
               'the host block on the guest')
    eye = [gp[0], gp[1] + 1.62, gp[2]]
    dx, dy, dz = block[0] + 0.5 - eye[0], block[1] + 0.5 - eye[1], block[2] + 0.5 - eye[2]
    yaw = math.degrees(math.atan2(-dx, dz))
    pitch = -math.degrees(math.atan2(dy, math.hypot(dx, dz)))
    guest(f'look {yaw:.3f} {pitch:.3f}')
    guest.wait('status', lambda v: v['target']['pos'] == block, 10, 'the guest aiming at the block')
    # Held for a few ticks, as a player clicks (creative breaks at once).
    guest('mouse left down')
    try:
        host.wait(f'getblock {block[0]} {block[1]} {block[2]}', lambda v: v['block'] == 'minecraft:air', 20,
                  'the guest breaking the block, on the host')
    finally:
        guest('mouse left up')
    guest.wait(f'getblock {block[0]} {block[1]} {block[2]}', lambda v: v['block'] == 'minecraft:air', 20,
               'the broken block on the guest')


def join(guest, code, name):
    guest(f'connect friend:{code} {name}')


def leave(host, guest, name):
    guest('disconnect')
    host.wait('lan status', lambda v: all(g['name'] != name for g in v['guests']), 30, 'the guest leaving')


def start_relay(args, work):
    """A relay on a free local port, as anyone would run their own."""
    port = lab.free_port()
    url = f'127.0.0.1:{port}'
    log = (work / 'relay.log').open('w+')
    process = subprocess.Popen([str(args.relay.resolve())], env=dict(os.environ, PORT=str(port)),
                               stdout=log, stderr=subprocess.STDOUT)
    deadline = time.monotonic() + 10
    while True:
        try:
            health(url)
            return process, log, url
        except OSError:
            assert time.monotonic() < deadline, 'the relay did not start'
            time.sleep(0.1)


def set_relay(game, url):
    """What typing the relay on Join a friend saves (horizon.txt)."""
    path = game.home / 'horizon.txt'
    lines = [l for l in (path.read_text().splitlines() if path.exists() else []) if not l.startswith('friendsRelay:')]
    path.write_text('\n'.join(lines + [f'friendsRelay:{url}']) + '\n')


def full(args, work):
    # Tests never touch the real router or ask the internet. Both games run on
    # this machine, so the code also holds 127.0.0.1 (a firewall such as
    # macOS's may hold back connections to the LAN address from an unsigned
    # binary started over SSH, which can't show its prompt).
    env = {'HORIZON_FRIENDS_UPNP': '0', 'HORIZON_STUN_SERVERS': 'none', 'HORIZON_FRIENDS_LOOPBACK': '1'}
    host = Game(args.client, args.resources, work, 'host', env)
    games = [host]
    relay = None
    try:
        # (a) The code alone: no relay anywhere.
        code = host_world(host)
        status = host('friends status')
        assert status['short'] is None and len(code) > 30, status
        print(f'host shares the world: a {len(code.replace("-", ""))}-symbol code, no relay; {status["summary"]}')
        if args.baseline:
            # The same checks over plain LAN, without the tunnel: the behaviour
            # the tunnel must reproduce.
            guest = Game(args.client, args.resources, work, 'guest-lan', env)
            games.append(guest)
            guest(f'connect 127.0.0.1:{host("lan status")["port"]} Plain')
            play_together(host, guest, 'Plain', 0)
            print('plain LAN: chat, movement and blocks both ways')
            leave(host, guest, 'Plain')
            guest.stop()
        guest = Game(args.client, args.resources, work, 'guest-code', env)
        games.append(guest)
        join(guest, code, 'Guest')
        play_together(host, guest, 'Guest', 1)
        assert host('friends status')['direct'] == 1, host('friends status')
        print('code alone: chat, movement and blocks both ways, no relay involved')
        leave(host, guest, 'Guest')
        host('friends stop')
        assert host('friends status')['code'] is None
        assert host('lan status') == {'open': False}, 'sharing opened LAN, so stopping closes it'
        join(guest, code, 'Late')
        reason = guest.wait('screen', lambda v: v['menu']['disconnected'], 30, 'the ended code')['menu']['disconnected']
        assert "Couldn't reach your friend's game" in reason, reason
        print(f'after stopping: "{reason}"')
        guest.stop()
        if Game.server_thread:
            # Leaving the world ends its code; coming back shares a new one.
            host('friends host')
            host.wait('friends status', lambda v: v['code'], 30, 'invite code')
            host('disconnect')
            host.wait('screen', lambda v: v['screen'] == 'title', 60, 'the title screen')
            assert host('friends status')['code'] is None, 'the code outlived its world'
            assert host_world(host) != code
            host('friends stop')
            print('leaving the world ends its code')
        # (b) A self-hosted relay, run locally: the short code, relayed.
        relay, relay_log, relay_url = start_relay(args, work)
        set_relay(host, relay_url)
        host('friends host')
        status = host.wait('friends status', lambda v: v['short'], 30, 'the short code')
        short = status['short']
        guest = Game(args.client, args.resources, work, 'guest-relay', dict(env, HORIZON_FRIENDS_PATH='relay'))
        games.append(guest)
        set_relay(guest, relay_url)
        before = health(relay_url)['relayed_bytes']
        join(guest, short, 'Relayed')
        play_together(host, guest, 'Relayed', 2)
        relayed_bytes = health(relay_url)['relayed_bytes'] - before
        status = host('friends status')
        assert status['relayed'] == 1, status
        # Render distance 2: the join is some tens of kB of compressed chunks.
        assert relayed_bytes > 32 * 1024, f'the relayed session carried only {relayed_bytes} bytes'
        print(f'self-hosted relay, short code {short[:4]}-****: chat, movement and blocks both ways; '
              f'relay carried {relayed_bytes} bytes')
        leave(host, guest, 'Relayed')
        host('friends stop')
        join(guest, short, 'Late')
        reason = guest.wait('screen', lambda v: v['menu']['disconnected'], 30, 'the ended invite')['menu']['disconnected']
        assert 'invite has ended' in reason, reason
        print(f'after stopping: "{reason}"')
        return code + short
    except Exception:
        for g in games:
            print(f'--- {g.home.name} log tail ---')
            print('\n'.join(g.stop().splitlines()[-25:]))
        raise
    finally:
        for g in games:
            g.stop()
        if relay:
            relay.terminate()
            relay.wait(timeout=10)
            relay_log.seek(0)
            text = relay_log.read()
            print(f'relay log: {text.splitlines()}')
            # Counts only: no code, no address, no name.
            for secret in ['127.0.0.1', 'Guest', 'Relayed', 'Late']:
                assert secret not in text, f'the relay logged {secret!r}'


def one_side(args, work):
    env = {'HORIZON_FRIENDS_UPNP': '0', 'HORIZON_STUN_SERVERS': 'none'}
    if args.relay_url:
        env['HORIZON_RELAY_URL'] = args.relay_url
    if args.path:
        env['HORIZON_FRIENDS_PATH'] = args.path
    game = Game(args.client, args.resources, work, args.role, env)
    try:
        if args.role == 'host':
            code = host_world(game)
            print(f'CODE {code}', flush=True)
            game.wait('lan status', lambda v: any(g['name'] == 'Remote' for g in v['guests']), args.wait, 'a guest')
            game.wait('lan status', lambda v: all(g['name'] != 'Remote' for g in v['guests']), args.wait, 'the guest leaving')
            print(f'host: {game("friends status")}')
        else:
            join(game, args.code, 'Remote')
            game.wait('status', lambda v: v['server']['joined'] and v['player'] is not None, 60, 'joined')
            game('chat hello across machines')
            v = game.wait('status', lambda v: v['server']['chunks'] > 0, 30, 'chunks')
            print(f'guest joined: {v["server"]["address"]}, {v["server"]["chunks"]} chunks')
            game('key w down')
            time.sleep(1.0)
            game('key w up')
            time.sleep(args.hold)
            game('disconnect')
    finally:
        game.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--client', type=Path, required=True)
    parser.add_argument('--relay', type=Path, help='horizon-relay binary (started on a free local port)')
    parser.add_argument('--relay-url', help='--role: a relay already running elsewhere, HOST:PORT')
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--role', choices=['host', 'guest'])
    parser.add_argument('--code')
    parser.add_argument('--path', choices=['relay', 'direct'])
    parser.add_argument('--wait', type=float, default=600)
    parser.add_argument('--hold', type=float, default=0, help='guest: stay this many seconds before leaving')
    parser.add_argument('--baseline', action='store_true', help='first run the same checks over plain LAN')
    parser.add_argument('--offscreen', action='store_true', help='run the games without a window')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--server-thread', dest='server_thread', action='store_true', default=True,
                      help='host on its server thread (default; retained for existing callers)')
    mode.add_argument('--in-frame', dest='server_thread', action='store_false',
                      help='DEPRECATED, never use: unsupported, will be removed soon')
    parser.add_argument('--xvfb', action='store_true', help='run the games under xvfb-run (headless Linux)')
    args = parser.parse_args()
    Game.prefix = ['xvfb-run', '-a'] if args.xvfb else []
    Game.extra = ['--offscreen'] if args.offscreen else []
    Game.server_thread = args.server_thread
    if not args.server_thread:
        print('DEPRECATED: --in-frame is unsupported and will be removed soon. Do not use it.', file=sys.stderr)
    args.client, args.resources = args.client.resolve(), args.resources.resolve()
    try:
        require_server_reports(args.resources)
    except ValueError as error:
        parser.error(str(error))
    # Short: Unix socket paths are limited to about 100 bytes.
    with tempfile.TemporaryDirectory(prefix='hzf-', dir='/tmp') as directory:
        work = Path(directory)
        print(f'Friends host mode: {"server-thread" if Game.server_thread else "in-frame"}')
        if args.role:
            return one_side(args, work)
        assert args.relay, 'give --relay (the relay binary, run locally for the relay half)'
        full(args, work)
    print('Play with friends passed: the code alone and a self-hosted relay; chat, movement and blocks both ways')


if __name__ == '__main__':
    main()
