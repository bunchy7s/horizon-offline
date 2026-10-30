#!/usr/bin/env python3
"""The dedicated server against vanilla's, seen from a vanilla client.

The pinned server JAR first generates a world (offline mode, a fixed seed,
respawn_radius 0) and stops; a vanilla dedicated server and Horizon's
(`horizon-client --dedicated --nogui`) then serve copies of it with the same
server.properties, server-icon.png and lists. VanillaProtocolClient, a
headless client made of vanilla's own codecs (any byte vanilla would not
read is an error), pings each server, joins it in offline mode through
login, configuration and play, accepts the spawn teleport (then, standing
still until its death, sends its position every 20 ticks as LocalPlayer
does: the server checks then which entities it sees), receives the
chunks around the spawn (their bytes are compared: the world is loaded from
the same files), chats, runs /list and commands that fail (`/random value
7..7`, `/random value x`, `/trigger nope`), trades, is made an operator by
the console, runs /kill, runs a command while dead, respawns (as vanilla's
LevelLoadTracker waits: until the "level chunks load start" event and the
chunk under the player), sets a furnace
beside itself, gives itself raw iron and coal, sets and asks its own score
(`@s`), opens the furnace, shift-clicks
the ore and the coal in and the ingot out, right-clicks the ground with a
snowball in the main hand, then in the off hand, throws an ender pearl at
its feet, and leaves. The
console then runs `list` and `say` and `stop`. Compared, all of them
exactly:

- the status response (MOTD, players, version, favicon) and the ping;
- the legacy (pre-1.7, 0xFE) ping reply;
- the login and configuration packets in order (compression, profile,
  brand (Horizon's server says "horizon", vanilla's "vanilla"), features,
  known packs, every registry, tags);
- the play packets up to the first chunk batch, in order, with entity ids,
  keep-alive ids and times left out (the recipe property sets and
  advancements, which vanilla keeps in hash maps, sorted); the command tree
  node by node; the chunks sent and their bytes; the replies to the chat
  message, /list and the failing commands, as text components;
- the reply to the command run while dead (ServerGamePacketListenerImpl
  runs a dead player's commands, #973);
- after /kill, the packets from the death screen to the respawned
  inventory (PlayerList.respawn; chat, chunk data, light and keep-alives
  left out, runs of a packet counted), then that the level loads where the
  player respawned (guests' death messages aren't Horizon's yet: left out);
- the furnace menu: its type and title, its contents, the slots each
  shift-click changes and its lit and cooking totals;
- the snowballs each right click throws: one, though the client sends the
  block's part and the item's of each hand as separate packets
  (handleUseItemOn, handleUseItem), and that the player is their owner;
- the ender pearl's teleport of the player who threw it, and the 5 damage;
- the console lines about the player (an operator's command feedback, which
  vanilla broadcasts to the console, included), the chat, `list`, `say` and
  `stop`;
- the files the servers leave: server.properties (but the date line),
  eula.txt, ops.json, whitelist.json, banned-players.json, banned-ips.json,
  usercache.json (but the date the player's entry expires).

Online mode needs Mojang's session servers and is not tested.
"""
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import tempfile
import time
import zlib

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .integration_env import game_environment, run_game
    from .lab import free_port  # the other server harnesses import it from here
except ImportError:
    from integration_env import game_environment, run_game
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from lab import free_port

NAME = 'dedicated-server-parity'
SEED = 12345
PLAYER = 'Tester'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-seed': str(SEED),
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'motd': '\\u00a7aHorizon \\u00a7rparity \\u00e9t\\u00e9',
    'pause-when-empty-seconds': '-1',
}


def icon_png():
    """A 64x64 RGBA PNG (server-icon.png)."""
    rows = b''.join(b'\0' + b''.join(bytes((x * 4, y * 4, (x ^ y) * 4, 255)) for x in range(64)) for y in range(64))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 64, 64, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def write_server_dir(directory, port, world=None):
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    (directory / 'server-icon.png').write_bytes(icon_png())
    if world is not None:
        shutil.copytree(world, directory / 'world')
        for lock in (directory / 'world').glob('session.lock'):
            lock.unlink()


class Server:
    """A server process with its console on stdin and its log collected."""

    def __init__(self, command, directory, log_name):
        self.log_path = directory.parent / f'{log_name}.log'
        self.log = self.log_path.open('w', encoding='utf-8')
        self.lines = []
        self.scratch = tempfile.TemporaryDirectory(prefix='hz-server-')
        env = game_environment(os.environ, self.scratch.name)
        self.process = subprocess.Popen(list(map(str, command)), cwd=directory, env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding='utf-8', errors='replace')
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _read(self):
        for line in self.process.stdout:
            self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def wait_for(self, pattern, timeout=300):
        deadline = time.time() + timeout
        regex = re.compile(pattern)
        while time.time() < deadline:
            if any(regex.search(l) for l in self.lines):
                return
            if self.process.poll() is not None:
                time.sleep(0.5)
                raise AssertionError(f'{self.log_path.name}: server exited ({self.process.returncode}):\n'
                                     + '\n'.join(self.lines[-30:]))
            time.sleep(0.2)
        raise AssertionError(f'{self.log_path.name}: no line matching {pattern!r} in {timeout} s:\n' + '\n'.join(self.lines[-30:]))

    def console(self, command):
        self.process.stdin.write(command + '\n')
        self.process.stdin.flush()

    def stop(self, timeout=120):
        if self.process.poll() is None:
            try:
                self.console('stop')
                self.process.wait(timeout=timeout)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()
        self.reader.join(timeout=10)
        self.log.close()
        self.scratch.cleanup()


def settle(server, ticks=600, timeout=600):
    """Waits until every force-loaded chunk is loaded, then for `ticks` game
    ticks, so their fluids have finished flowing however slow the machine."""
    deadline = time.time() + timeout
    offsets = range(-80, 81, 16)
    for poll in range(1_000_000):
        start = len(server.lines)
        for dx in offsets:
            for dz in offsets:
                server.console(f'execute unless loaded ~{dx} ~ ~{dz} run say unloaded')
        server.console(f'say poll {poll}')
        server.wait_for(rf'\[Server\] poll {poll}$', timeout=max(1, deadline - time.time()))
        if not any(l.endswith('[Server] unloaded') for l in server.lines[start:]):
            break
        time.sleep(2)

    until = game_time(server, deadline - time.time()) + ticks
    while game_time(server, deadline - time.time()) < until:
        time.sleep(2)


def game_time(server, timeout=300):
    """The server's game time, asked from its console: the console runs its queue in order, so every console
    command sent before has run."""
    deadline = time.time() + timeout
    start = len(server.lines)
    server.console('time query gametime')
    while time.time() < deadline:
        for line in server.lines[start:]:
            if m := re.search(r'The (?:game )?time is (\d+)', line):
                return int(m.group(1))
        time.sleep(0.05)
    raise AssertionError(f'{server.log_path.name}: no game time in {timeout:.0f} s')


def wait_ticks(server, count, timeout=300):
    """Waits until the server's game clock has advanced `count` ticks: a server that stalls under load still gets
    as many ticks as an idle one, where a sleep gives it fewer. (Not while ticking is frozen.)"""
    deadline = time.time() + timeout
    until = game_time(server, timeout) + count
    while game_time(server, max(1.0, deadline - time.time())) < until:
        if time.time() > deadline:
            raise AssertionError(f'{server.log_path.name}: the game clock did not advance {count} ticks in {timeout} s')
        time.sleep(0.2)


def legacy_ping(port):
    """The reply to a 1.6 client's 0xFE 0x01 0xFA MC|PingHost query (LegacyQueryHandler)."""
    host = 'localhost'
    data = (b'\xfe\x01\xfa' + struct.pack('>H', 11) + 'MC|PingHost'.encode('utf-16-be')
            + struct.pack('>H', 7 + 2 * len(host)) + bytes([78]) + struct.pack('>H', len(host))
            + host.encode('utf-16-be') + struct.pack('>I', port))
    with socket.create_connection(('127.0.0.1', port), timeout=10) as s:
        s.sendall(data)
        reply = b''
        while True:
            try:
                chunk = s.recv(4096)
            except TimeoutError:
                return reply.hex() + ' <no close>'
            if not chunk:
                break
            reply += chunk
    return reply.hex()


CONSOLE_LINE = re.compile(r'^\[\d\d:\d\d:\d\d\] \[([^\]]+)\]: (.*)$')
KEPT = re.compile(r'Tester|\[Server\]|\[Not Secure\]|There are|Stopping|Saving|ThreadedAnvilChunkStorage|All dimensions|Unknown or incomplete')


# Guests' death messages and advancement announcements aren't Horizon's yet (the client's
# own transcript leaves the death message out as well).
NOT_YET = re.compile(r'Tester was killed|Tester has made the advancement')


def console_lines(lines):
    """The console lines the comparison covers, timestamps and ports aside."""
    out = []
    for line in lines:
        m = CONSOLE_LINE.match(line)
        if not m or not KEPT.search(m.group(2)) or NOT_YET.search(m.group(2)):
            continue
        text = re.sub(r'/127\.0\.0\.1:\d+', '/127.0.0.1:_', m.group(2))
        text = re.sub(r'entity id \d+', 'entity id _', text)
        out.append(f'[{m.group(1)}] {text}')
    # Vanilla saves its levels in the order of an identity-hashed map, which
    # changes between runs: each block of saves is compared sorted.
    i = 0
    while i < len(out):
        j = i
        while j < len(out) and 'Saving chunks for level' in out[j]:
            j += 1
        out[i:j] = sorted(out[i:j])
        i = j + 1 if j == i else j
    return out


# What a failure's diff doesn't say: warnings, errors, panics and why a player left.
NOTABLE = re.compile(r'/(WARN|ERROR)\]|Exception|panicked|lost connection')


def notable_lines(path, limit=20):
    """A server's notable lines from its `Done (` on (the startup warnings are every run's), or a client's."""
    try:
        lines = path.read_text(encoding='utf-8', errors='replace').splitlines()
    except OSError:
        return []
    start = next((i for i, l in enumerate(lines) if 'Done (' in l), 0)
    return [f'{path.name}: {l[:300]}' for l in lines[start:] if NOTABLE.search(l)][-limit:]


def server_files(directory):
    out = []
    for name in ['server.properties', 'eula.txt', 'ops.json', 'whitelist.json', 'banned-players.json', 'banned-ips.json',
                 'usercache.json']:
        path = directory / name
        if not path.exists():
            out.append(f'{name}: missing')
            continue
        text = path.read_bytes().decode('iso-8859-1')
        if name in ('server.properties', 'eula.txt'):
            # The second comment line is the date Properties.store wrote.
            text = re.sub(r'^(#[^\n]*\n)#[^\n]*\n', r'\1#<date>\n', text)
            # Random per server, and each side has its own port.
            text = re.sub(r'\n(management-server-secret|server-port)=[^\n]*', r'\n\1=<varies>', text)
        if name == 'usercache.json':
            # A month from when the player joined.
            text = re.sub(r'"expiresOn":"[^"]*"', '"expiresOn":"<a month on>"', text)
        out.append(f'{name}: {text!r}')
    return out


def run_side(label, command, directory, client):
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        # A librarian beside the spawn for the client to trade with (no AI:
        # it stays put; experience keeps its profession).
        server.console('summon minecraft:villager ~1 ~ ~ {NoAI:1b,Xp:1,VillagerData:'
                       '{profession:"minecraft:librarian",level:2,type:"minecraft:plains"}}')
        # The saved world still has fluid ticks due in the area the client is sent (a lava fall
        # that never settles), and they run once their chunk ticks. When that is, against when the
        # chunk is sent, follows how fast the server loads the chunks around the joining player:
        # each run, vanilla sends a different set of them already flowed. Frozen until the client
        # has its chunks, both servers send the world as saved; blocks tick again from the chat on.
        server.console('tick freeze')
        port = int(PROPERTIES_PORTS[label])
        legacy = legacy_ping(port)
        # The client asks to be made an operator once it has traded, runs /kill and respawns.
        failure = []

        def join():
            try:
                client(port, directory.parent / f'{label}-client.txt')
            except BaseException as error:  # noqa: BLE001 - reported on the main thread
                failure.append(error)

        joined = threading.Thread(target=join)
        joined.start()
        deadline = time.time() + 300
        frozen = True
        while joined.is_alive() and not any('<Tester> ready to respawn' in l for l in server.lines):
            if frozen and any('<Tester> hello from the parity client' in l for l in server.lines):
                server.console('tick unfreeze')
                frozen = False
            if time.time() > deadline:
                break
            time.sleep(0.2)
        if joined.is_alive():
            server.console(f'op {PLAYER}')
        joined.join()
        if failure:
            raise failure[0]
        # The client has gone; `list` must find no one, once the server has handled its leaving (a busy machine
        # took longer than a second).
        server.wait_for(rf'{PLAYER} left the game', timeout=60)
        server.console('list')
        server.console('say console says hi')
        server.wait_for(r'\[Server\] console says hi', timeout=30)
    finally:
        server.stop()
    transcript = (directory.parent / f'{label}-client.txt').read_text(encoding='utf-8').splitlines()
    return (['legacy-ping ' + legacy] + transcript + ['console ' + l for l in console_lines(server.lines)]
            + ['file ' + l for l in server_files(directory)])


PROPERTIES_PORTS = {}


def horizon_client_joins(vanilla_server, binary, world, build, reference):
    """Horizon's client joins a vanilla dedicated server headless: it must
    land where the vanilla client did, receive as many chunks, see its chat
    and /list reply, and leave as a vanilla client does (the server's log)."""
    label = 'vanilla-for-horizon-client'
    PROPERTIES_PORTS[label] = port = free_port()
    directory = build / label
    write_server_dir(directory, port, world)
    client_dir = build / 'horizon-client'
    shutil.rmtree(client_dir, ignore_errors=True)
    client_dir.mkdir()
    report = build / 'horizon-client-report.json'
    report.unlink(missing_ok=True)
    server = Server(vanilla_server, directory, label)
    try:
        server.wait_for(r'Done \(')
        env = lab.game_env(HORIZON_NET_REPORT=report)
        with (build / 'horizon-client-run.log').open('w') as log:
            run_game([str(c) for c in [binary, '--resources', lab.RESOURCES, '--render-distance', 2,
                                             '--server', f'127.0.0.1:{port}', '--name', PLAYER]],
                           cwd=client_dir, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=300, check=True)
        server.wait_for(r'left the game', timeout=30)
    finally:
        server.stop()
    got = json.loads(report.read_text())
    move = next(l for l in reference if l.startswith('play-move'))
    want_position = [float(v) for v in re.match(r'play-move \(([^,]+), ([^,]+), ([^)]+)\)', move).groups()]
    want_chunks = int(next(l for l in reference if l.startswith('play-counts')).split()[-1])
    expected = {
        'connected throughout': None,
        'spawn position': want_position,
        'chunks received': want_chunks,
        'own chat shown': True,
        '/list reply shown': True,
        'server log': ['logged in', 'Tester joined the game', '[Not Secure] <Tester> hello from the horizon client',
                       'Tester lost connection: Disconnected', 'Tester left the game'],
    }
    log = '\n'.join(server.lines)
    actual = {
        'connected throughout': got['error'],
        'spawn position': got['position'],
        'chunks received': got['chunks'],
        'own chat shown': any('<Tester> hello from the horizon client' in c for c in got['chat']),
        '/list reply shown': any('There are 1 of a max of 5 players online: Tester' in c for c in got['chat']),
        'server log': [k for k in expected['server log'] if k in log],
    }
    return [f'horizon-client {k}: {"ok" if actual[k] == v else f"expected {v!r}, got {actual[k]!r}"}'
            for k, v in expected.items()]


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-sourcepath',
                   lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                  build, 'compile-java')
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], build,
                  'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.RESOURCES, '--dedicated', '--nogui']

    # The world, generated once by vanilla.
    generate = build / 'generate'
    write_server_dir(generate, free_port())
    server = Server(vanilla_server, generate, 'generate')
    try:
        server.wait_for(r'Done \(')
        server.console('gamerule respawn_radius 0')
        server.wait_for(r'respawn_radius', timeout=30)
        # Vanilla keeps no spawn chunks: force-load the area the client is
        # sent and its neighbours (the console stands at the world spawn) so
        # both servers load full chunks instead of finishing proto-chunks.
        # Random ticks change blocks while the client is being sent its
        # chunks, more of them the busier the machine: none, so both servers
        # send the world as it was saved.
        server.console('gamerule random_tick_speed 0')
        server.wait_for(r'random_tick_speed', timeout=30)
        server.console('forceload add ~-80 ~-80 ~80 ~80')
        server.wait_for(r'chunks in the Overworld are now force loaded|forceloaded|force loaded', timeout=60)
        settle(server)
        server.console('forceload remove all')
        server.console('save-all flush')
        server.wait_for(r'Saved the game', timeout=120)
    finally:
        server.stop()
    world = generate / 'world'
    (world / 'playerdata').mkdir(exist_ok=True)

    def client(port, out):
        try:
            oh.run_logged(['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', str(classes) + os.pathsep + dependencies,
                           'dev.horizonlab.parity.VanillaProtocolClient', '127.0.0.1', port, PLAYER, out], build,
                          out.stem, timeout=300)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            log = (build / f'{out.stem}.log').read_text(encoding='utf-8', errors='replace').splitlines()
            raise AssertionError(f'{out.stem}: {error}:\n' + '\n'.join(log[-30:])) from None

    results = {}
    for label, command in [('vanilla', vanilla_server), ('horizon', horizon_server)]:
        PROPERTIES_PORTS[label] = free_port()
        directory = build / label
        write_server_dir(directory, PROPERTIES_PORTS[label], world)
        results[label] = run_side(label, command, directory, client)
        (build / f'{label}.txt').write_text('\n'.join(results[label]) + '\n', encoding='utf-8')
    client_checks = horizon_client_joins(vanilla_server, binary, world, build, results['vanilla'])
    vanilla, horizon = results['vanilla'], results['horizon']
    # The one intended difference: MinecraftServer.getServerModName is "vanilla" there, and Horizon's server names
    # itself "horizon" as a modded server does (#3756). Check that, then compare the rest as vanilla's.
    brand = 'config minecraft:custom_payload BrandPayload[brand={}]'
    errors_brand = [] if brand.format('horizon') in horizon else ['error: Horizon\'s server did not send the brand "horizon"']
    horizon = [brand.format('vanilla') if l == brand.format('horizon') else l for l in horizon]
    errors = [l for l in vanilla + horizon if l.startswith('error')] + [l for l in client_checks if not l.endswith(': ok')] + errors_brand
    for line in client_checks:
        print(line)
    differences = []
    import difflib
    for line in difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=0, lineterm=''):
        differences.append(line[:400])
    # Name the chunks that differ instead of the whole line.
    chunk_lines = [dict(re.findall(r'(-?\d+,-?\d+)=(\w+)', next((l for l in side if l.startswith('play-chunks')), '')))
                   for side in (vanilla, horizon)]
    for key in sorted(set(chunk_lines[0]) | set(chunk_lines[1])):
        a, b = chunk_lines[0].get(key), chunk_lines[1].get(key)
        if a != b:
            differences.append(f'chunk {key}: vanilla {a} horizon {b}')
    if differences or errors:
        notes = [line for name in ('vanilla', 'vanilla-client', 'horizon', 'horizon-client')
                 for line in notable_lines(build / f'{name}.log')]
        raise AssertionError(f'{len([d for d in differences if d[:1] in "+-" and d[:3] not in ("+++", "---")])} differing lines '
                             f'(see {build}/vanilla.txt and horizon.txt):\n' + '\n'.join(differences[:80] + notes))
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='Dedicated server seen by a vanilla-codec client: status, legacy ping, login, configuration, play '
              'start (recipes, advancements, command tree), chunks, chat, /list, failing commands\' replies, '
              'console list/say/stop, files written. Offline mode.'))
    print(f'dedicated server: {len(vanilla)} transcript lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
