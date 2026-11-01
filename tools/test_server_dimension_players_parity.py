#!/usr/bin/env python3
"""Players in different dimensions on a dedicated server, against vanilla's, seen by vanilla clients.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first, with Alice and Bob as operators in creative. LocatorBarClient joins them both
(players made of vanilla's protocol code) and records, step by step, the other players each is sent and loses
(ClientboundAddEntityPacket, ClientboundRemoveEntitiesPacket) and the dimensions it respawns into
(ClientboundRespawnPacket), with the locator bar's waypoints, while they change dimensions (ServerPlayer.teleport
through `execute in ... run tp`, and an End portal): Bob goes to the Nether and back, Alice to the End, Bob
follows her there, both come back to the Overworld, then Bob steps into an End portal and comes back. Between
the steps each client sends its position (the reminder a still client sends every second). Each player's lines in each step are compared exactly.
"""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server
    from .test_locator_bar_parity import Client, generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server
    from test_locator_bar_parity import Client, generate_world, server_dir

NAME = 'server-dimension-players-parity'
NETHER, END, OVERWORLD = 'minecraft:the_nether', 'minecraft:the_end', 'minecraft:overworld'
# (label, what the players do).
SCRIPT = [
    ('bob-nether', [f'cmd Bob execute in {NETHER} run tp @s 0.5 100 0.5']),
    ('bob-back', [f'cmd Bob execute in {OVERWORLD} run tp @s 2.5 100 0.5']),
    ('alice-end', [f'cmd Alice execute in {END} run tp @s 0.5 100 0.5']),
    ('bob-end', [f'cmd Bob execute in {END} run tp @s 2.5 100 0.5']),
    ('alice-back', [f'cmd Alice execute in {OVERWORLD} run tp @s 0.5 100 0.5']),
    ('bob-back-again', [f'cmd Bob execute in {OVERWORLD} run tp @s 2.5 100 0.5']),
    # The dedicated server console has no player as its command source.
    ('console-bob-nether', [f'console execute in {NETHER} run tp Bob 2.5 100 0.5']),
    ('console-bob-end', [f'console execute in {END} run tp Bob 2.5 100 0.5']),
    ('console-bob-back', [f'console execute in {OVERWORLD} run tp Bob 2.5 100 0.5']),
    # A destination entity supplies its own dimension; a selector can move
    # more than one player in the same command.
    ('console-alice-end', [f'console execute in {END} run tp Alice 0.5 100 0.5']),
    ('console-bob-to-alice', ['console tp Bob Alice']),
    ('console-alice-back', [f'console execute in {OVERWORLD} run tp Alice 0.5 100 0.5']),
    ('console-bob-back2', [f'console execute in {OVERWORLD} run tp Bob 2.5 100 0.5']),
    # Through an End portal (Entity.handlePortal), and back by command.
    ('bob-portal', ['cmd Bob setblock 2 101 3 minecraft:end_portal', 'cmd Bob tp @s 2.5 101 3.5']),
    ('bob-portal-back', [f'cmd Bob execute in {OVERWORLD} run tp @s 2.5 100 0.5']),
    # Sent far into the Nether, then teleported within it before the far chunks load: a portal there
    # still takes him (the abandoned arrival does not hold him).
    ('bob-retarget', [f'cmd Bob execute in {NETHER} run tp @s 8192.5 100 0.5', 'cmd Bob tp @s 0.5 100 0.5']),
    ('bob-retarget-portal', ['cmd Bob setblock 0 101 3 minecraft:end_portal', 'cmd Bob tp @s 0.5 101 3.5']),
    ('bob-retarget-back', [f'cmd Bob execute in {OVERWORLD} run tp @s 2.5 100 0.5']),
]
PLAYERS = ('Alice', 'Bob')
STEP_TICKS = 100


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
            try:
                client.tell('players on')
                client.tell('step joins')
                for name, x in zip(PLAYERS, (0.5, 2.5)):
                    client.tell(f'join {name}')
                    server.wait_for(rf'{name} joined the game', timeout=120)
                    client.tell(f'fly {name} on')
                    client.tell(f'cmd {name} tp @s {x} 100 0.5')
                client.wait(server, PLAYERS, STEP_TICKS)
                for step, actions in SCRIPT:
                    client.tell(f'step {step}')
                    for action in actions:
                        if action.startswith('console '):
                            server.console(action.removeprefix('console '))
                        else:
                            client.tell(action)
                    # Standing still, a client still sends its position every 20 ticks
                    # (LocalPlayer.sendPosition): the server then tracks to it what it has sent the
                    # chunks of since (ChunkMap.move).
                    client.wait(server, PLAYERS, STEP_TICKS // 2)
                    for name in PLAYERS:
                        client.tell(f'still {name}')
                    client.wait(server, PLAYERS, STEP_TICKS // 2)
                client.end()
            finally:
                client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    return out.read_text(encoding='utf-8').splitlines()


SHOWN = ('add', 'remove', 'respawn', 'track', 'untrack', 'update')


def shown(lines):
    """Each player's adds, removes, respawns and waypoints (as the step leaves them), step by step; sorted within a
    step (the packets of one step may leave in another order within the tick)."""
    steps = {}
    for line in lines:
        parts = line.split(' ', 2)
        if len(parts) == 3 and parts[2].split(' ')[0] in SHOWN:
            steps.setdefault((parts[0], parts[1]), []).append(line)
    return [l for key in steps for l in sorted(lab.settled(steps[key]))]


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
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

    world = generate_world(vanilla_server, build)
    vanilla_all = run_side('vanilla', vanilla_server, client, build, world)
    horizon_all = run_side('horizon', horizon_server, client, build, world)
    errors = [l for l in vanilla_all + horizon_all if ' error ' in f' {l} ' or 'disconnected' in l]
    vanilla, horizon = shown(vanilla_all), shown(horizon_all)
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=1, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:6000])
    if sum(' respawn ' in l for l in vanilla) < 6:
        raise AssertionError(f'vanilla sent too few respawns: the script did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        lines=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='Two players on a dedicated server changing dimensions (Nether, End, back): the other players each '
              'is sent and loses, the dimensions it respawns into and its locator bar waypoints.'))
    print(f'server dimension players: {len(vanilla)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
