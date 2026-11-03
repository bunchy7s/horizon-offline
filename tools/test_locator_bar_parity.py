#!/usr/bin/env python3
"""The locator bar on a dedicated server, against vanilla's, seen by vanilla clients.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first (players appear at its spawn), with the same server.properties (offline,
creative, view distance 4) and two operators. LocatorBarClient, two
players made of vanilla's protocol code (any byte vanilla would not read is an error), joins Alice and then
Bob, who fly as creative players do (a player floating without flying falls on the server every tick), and
records every ClientboundTrackedWaypointPacket each of them receives. Bob then:

- teleports one block, three blocks, out of Alice's view, one chunk on, really far (an azimuth) and turns
  around her, and back;
- turns invisible and visible, joins a red team (by the console), has it turned blue, leaves it;
- crouches and stands up, wears a carved pumpkin and takes it off;
- turns spectator and moves, then creative again;

the console turns the `locator_bar` game rule off and on, and Bob leaves. The waypoint packets each player
received in each step are compared exactly: operation, player, style, colour and the block, chunk or angle.
"""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port, wait_ticks
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port, wait_ticks

NAME = 'locator-bar-parity'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-seed': '12345',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'pause-when-empty-seconds': '-1',
}
PLAYERS = ['Alice', 'Bob']

# (label, [what the console does], [what the players do]); every step then waits for its packets.
SCRIPT = [
    ('alice-joins', [], ['join Alice', 'fly Alice on', 'cmd Alice tp @s 0.5 100 0.5']),
    ('bob-joins', [], ['join Bob', 'fly Bob on', 'cmd Bob tp @s 2.5 100 0.5']),
    ('near', [], ['cmd Bob tp @s 5.5 100 0.5']),
    ('one-block', [], ['cmd Bob tp @s 6.5 100 0.5']),
    ('three-blocks', [], ['cmd Bob tp @s 9.5 100 0.5']),
    ('out-of-view', [], ['cmd Bob tp @s 100.5 100 0.5']),
    ('next-chunk', [], ['cmd Bob tp @s 116.5 100 0.5']),
    ('really-far', [], ['cmd Bob tp @s 1000.5 100 0.5']),
    ('turn-around', [], ['cmd Bob tp @s 1000.5 100 300.5']),
    ('half-degree', [], ['cmd Bob tp @s 1000.5 100 302.5']),
    ('invisible', [], ['cmd Bob effect give @s minecraft:invisibility infinite 0 true']),
    ('visible', [], ['cmd Bob effect clear @s']),
    ('back-near', [], ['cmd Bob tp @s 3.5 100 0.5']),
    ('red-team', ['team add red', 'team modify red color red', 'team join red Bob'], []),
    ('blue-team', ['team modify red color blue'], []),
    ('no-team', ['team leave Bob'], []),
    # Crouching needs the ground: a flying player who holds shift goes down instead.
    ('landed', [], ['cmd Bob tp @s 3.5 -60 0.5', 'fly Bob off']),
    ('crouch', [], ['shift Bob on']),
    ('stand', [], ['shift Bob off']),
    ('flying-again', [], ['fly Bob on', 'cmd Bob tp @s 3.5 100 0.5']),
    ('pumpkin', [], ['cmd Bob item replace entity @s armor.head with minecraft:carved_pumpkin']),
    ('no-pumpkin', [], ['cmd Bob item replace entity @s armor.head with minecraft:air']),
    ('rule-off', ['gamerule locator_bar false'], []),
    ('moves-without', [], ['cmd Bob tp @s 4.5 100 0.5']),
    ('rule-on', ['gamerule locator_bar true'], []),
    ('spectator', [], ['cmd Bob gamemode spectator', 'cmd Bob tp @s 5.5 100 0.5']),
    ('creative', [], ['cmd Bob gamemode creative', 'cmd Bob tp @s 6.5 100 0.5']),
    ('bob-leaves', [], ['quit Bob']),
]
STEP_TICKS = 40
# Vanilla's server moves a player who just joined on their own first ticks (a tiny step and back), which tracks
# them where they spawned; Horizon does not simulate its guests. These steps compare what the bar shows at
# their end (Bob has moved by then) rather than every packet.
SETTLING_STEPS = {'alice-joins', 'bob-joins'}


def shown(lines):
    """The transcript with each settling step reduced to what each player's bar shows at its end."""
    out, state = [], {}

    def flush(step):
        for (receiver, source), rest in sorted(state.items()):
            out.append(f'{receiver} step={step} shows {source} {rest}')
        state.clear()

    current = None
    for line in lines:
        receiver, step, *packet = line.split(' ')
        step = step.removeprefix('step=')
        if current in SETTLING_STEPS and step != current:
            flush(current)
        current = step
        if step not in SETTLING_STEPS or len(packet) < 3:
            out.append(line)
        elif packet[0] == 'untrack':
            state.pop((receiver, packet[1]), None)
        else:
            state[(receiver, packet[1])] = ' '.join(packet[2:])
    if current in SETTLING_STEPS:
        flush(current)
    return out


def ops_json():
    """Both players at level 4, by their offline UUIDs."""
    import hashlib
    import json
    import uuid as uuidlib
    entries = []
    for name in PLAYERS:
        digest = bytearray(hashlib.md5(f'OfflinePlayer:{name}'.encode()).digest())
        digest[6] = (digest[6] & 0x0F) | 0x30
        digest[8] = (digest[8] & 0x3F) | 0x80
        entries.append({'uuid': str(uuidlib.UUID(bytes=bytes(digest))), 'name': name, 'level': 4,
                        'bypassesPlayerLimit': False})
    return json.dumps(entries, indent=2)


def server_dir(directory, world=None):
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()),
                                                 encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    (directory / 'ops.json').write_text(ops_json())
    if world is not None:
        shutil.copytree(world, directory / 'world')
        (directory / 'world' / 'session.lock').unlink(missing_ok=True)
    return port


class Client:
    """A LocatorBarClient process driven by a script on its stdin. A step ends by the server's clock, not the wall's:
    every script line has run (`sync`), the server has ticked `ticks` more times, and each client holds what the
    server sent until then (a last `sync`). A server that stalls under load still gets all its ticks in the step,
    where a fixed sleep filed a stalled server's packets under the next step."""

    def __init__(self, command, port, out, log):
        self.log = log
        self.process = subprocess.Popen([*map(str, command), '127.0.0.1', str(port), str(out)], cwd=lab.ROOT,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        text=True)

    def tell(self, line):
        self.process.stdin.write(line + '\n')
        self.process.stdin.flush()

    def sync(self, names):
        """Every line told so far has run for these joined players, and they have received what the server sent
        before."""
        for name in names:
            self.tell(f'sync {name}')
        for name in names:
            while (line := self.process.stdout.readline()) != f'synced {name}\n':
                if not line:
                    raise AssertionError(f'the client exited (see {self.log.name})')
                self.log.write(line)

    def wait(self, server, names, ticks):
        self.sync(names)
        wait_ticks(server, ticks)
        self.sync(names)

    def end(self):
        self.tell('end')
        self.log.write(self.process.communicate(timeout=60)[0])

    def kill(self):
        if self.process.poll() is None:
            self.process.kill()


def generate_world(server_command, build):
    """The flat world both servers load, made once by vanilla: players appear at its spawn itself
    (respawn_radius 0), on the ground at 0 -60 0, and the chunks around it are saved."""
    directory = build / 'generate'
    server_dir(directory)
    server = Server(server_command, directory, 'generate')
    try:
        server.wait_for(r'Done \(')
        for command, reply in [('gamerule respawn_radius 0', 'respawn_radius'),
                               ('setworldspawn 0 -60 0', 'Set the world spawn'),
                               ('forceload add -96 -96 96 96', 'force loaded'),
                               ('save-all flush', 'Saved the game')]:
            server.console(command)
            server.wait_for(reply, timeout=120)
        server.console('forceload remove all')
    finally:
        server.stop()
    return directory / 'world'


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = server_dir(directory, world)
    out = build / f'{label}.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = Client(client_command, port, out, log)
            joined = []
            try:
                for step, console, actions in SCRIPT:
                    client.tell(f'step {step}')
                    for command in console:
                        server.console(command)
                    for action in actions:
                        client.tell(action)
                        verb, *rest = action.split()
                        if verb == 'join':
                            # Joining takes the world's first chunks: wait for the server to place the player.
                            server.wait_for(rf'{rest[0]} joined the game', timeout=120)
                            joined.append(rest[0])
                        elif verb == 'quit':
                            joined.remove(rest[0])
                    client.wait(server, joined, STEP_TICKS)
                client.end()
            finally:
                client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    return out.read_text(encoding='utf-8').splitlines()


def sides(name):
    """The vanilla and Horizon dedicated servers and the LocatorBarClient that joins them, built for harness
    `name`: (lock, build directory, vanilla server, Horizon server, client) commands."""
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / name
    build.mkdir(parents=True, exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/LocatorBarClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.LocatorBarClient']
    return lock, build, vanilla_server, horizon_server, client


# How many times one_player runs vanilla's side before giving up on a run its `check` rejects.
VANILLA_ATTEMPTS = 5


def one_player(name, script, shown, check, scope, step_seconds=4.0):
    """A harness of one player, Alice: on each server she joins, then runs `script`, a list of (step, client
    actions) that each last `step_seconds` of the server's time (20 ticks a second). The lines `shown(transcript)` keeps must match vanilla's exactly, and
    `check(vanilla's shown lines, vanilla's transcript)` returns why vanilla's run is no proof (the script did
    not run), or None."""
    lock, build, vanilla_server, horizon_server, client = sides(name)
    ticks = round(step_seconds * 20)

    def run(label, server_command, world):
        directory = build / label
        port = server_dir(directory, world)
        out = build / f'{label}.txt'
        out.unlink(missing_ok=True)
        server = Server(server_command, directory, label)
        try:
            server.wait_for(r'Done \(')
            with (build / f'{label}-client.log').open('w') as log:
                session = Client(client, port, out, log)
                try:
                    for line in ['players on', 'state on', 'step joins', 'join Alice']:
                        session.tell(line)
                    server.wait_for(r'Alice joined the game', timeout=120)
                    session.wait(server, ['Alice'], ticks)
                    for step, actions in script:
                        session.tell(f'step {step}')
                        for action in actions:
                            session.tell(action)
                        session.wait(server, ['Alice'], ticks)
                    session.end()
                finally:
                    session.kill()
        finally:
            server.stop()
        if not out.exists():
            raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
        return out.read_text(encoding='utf-8').splitlines()

    world = generate_world(vanilla_server, build)
    # Vanilla's run is the specification only when `check` accepts it. On a loaded machine a real-time step can
    # miss its moment (a bobber that lands before the armor stand is there), so a run it rejects is made again,
    # up to VANILLA_ATTEMPTS times, before Horizon's is compared with it; Horizon's run is never retried.
    for attempt in range(1, VANILLA_ATTEMPTS + 1):
        vanilla_all = run('vanilla', vanilla_server, world)
        why = check(shown(vanilla_all), vanilla_all)
        if why is None:
            break
        print(f'{name}: vanilla run {attempt} of {VANILLA_ATTEMPTS} is no proof ({why})', flush=True)
    else:
        raise AssertionError(f'{why}: the script did not run on vanilla in {VANILLA_ATTEMPTS} runs (see {build})')
    horizon_all = run('horizon', horizon_server, world)
    errors = [l for l in vanilla_all + horizon_all if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla_all), shown(horizon_all)
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    lab.write_json(build / 'report.json', dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'), scope=scope))
    return vanilla


def main():
    lock, build, vanilla_server, horizon_server, client = sides(NAME)
    report = build / 'report.json'
    world = generate_world(vanilla_server, build)
    vanilla = run_side('vanilla', vanilla_server, client, build, world)
    horizon = run_side('horizon', horizon_server, client, build, world)
    errors = [l for l in vanilla + horizon if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla), shown(horizon)
    # Each player's packets in their order; which player's line comes first in a step doesn't matter.
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if len(vanilla) < 20:
        raise AssertionError(f'only {len(vanilla)} waypoint packets: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        packets=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='Locator bar waypoints (ClientboundTrackedWaypointPacket) two vanilla-codec players receive on a '
              'dedicated server: block, chunk and azimuth connections as one moves, invisibility, team colours, '
              'crouching, a carved pumpkin, the locator_bar game rule, spectator mode and leaving.'))
    print(f'locator bar: {len(vanilla)} waypoint packets identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
