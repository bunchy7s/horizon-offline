#!/usr/bin/env python3
"""Goat voice event names and metadata on real Java/Horizon dedicated servers.

Silent ambient suppression is a separate gap: record those two controls, but
gate only their hurt/death phases. Death follows 21 actual cooldown ticks.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port, game_time
    from .test_server_world_gen_parity import escape
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port, game_time
    from test_server_world_gen_parity import escape

PLAYER = 'Tester'
CASES = [('adult', False, False, False), ('screaming_adult', True, False, False),
         ('baby', False, True, False), ('screaming_baby', True, True, False),
         ('silent', False, False, True), ('screaming_silent', True, False, True)]
MESSAGE = re.compile(r'\[Server thread/INFO\]: (?:System chat: )?(.*)$')


def run_side(label, command, client_command, out):
    directory = out / label
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    port = free_port()
    props = {'server-ip': '127.0.0.1', 'server-port': str(port), 'online-mode': 'false', 'white-list': 'false',
             'level-type': 'minecraft:flat', 'generate-structures': 'false', 'level-seed': '42',
             'gamemode': 'creative', 'force-gamemode': 'true', 'allow-flight': 'true',
             'view-distance': '2', 'simulation-distance': '2', 'spawn-protection': '0',
             'pause-when-empty-seconds': '-1', 'max-tick-time': '-1'}
    (directory / 'server.properties').write_text(
        ''.join(f'{k}={escape(v)}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    transcript = out / f'{label}-packets.txt'
    server = Server(command, directory, label)
    client = None
    serial = 0

    def reply(text):
        nonlocal serial
        serial += 1
        marker = f'goat_voice_barrier_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$', timeout=180)
        return [m.group(1) for line in server.lines[start:] if (m := MESSAGE.search(line))
                and not m.group(1).endswith(f'[Server] {marker}')]

    def advance(ticks):
        start = game_time(server)
        reply(f'tick step {ticks}')
        deadline = time.monotonic() + 180
        while game_time(server) < start + ticks:
            assert time.monotonic() < deadline, f'{label}: tick step stalled'
        assert game_time(server) == start + ticks

    def phase(name):
        server.console(f'say goat_voice_{name}')
        server.wait_for(rf'<{PLAYER}> goat voice phase {name}', timeout=120)

    flags = []
    try:
        server.wait_for(r'Done \(')
        with (out / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), PLAYER,
                                       str(transcript), 'goat-voices'], cwd=lab.ROOT,
                                      stdout=log, stderr=subprocess.STDOUT)
            server.wait_for(rf'<{PLAYER}> summon the goats', timeout=180)
            reply('tick freeze')
            reply('gamerule minecraft:spawn_mobs false')
            reply('kill @e[type=!minecraft:player]')
            reply('forceload add 0 0 15 15')
            deadline = time.monotonic() + 180
            while not any('Test passed' in line for line in reply('execute if loaded 0 220 0')):
                assert time.monotonic() < deadline, f'{label}: scene chunk did not load'
            reply('fill 0 219 0 15 219 15 minecraft:stone')
            reply(f'tp {PLAYER} 8.5 220 11.5')
            for i, (_, screaming, baby, silent) in enumerate(CASES):
                for j in range(3):
                    tag = f'goat_voice_{i}_{j}'
                    nbt = ('{NoAI:1b,NoGravity:1b,PersistenceRequired:1b,'
                           f'Age:{-24000 if baby else 0},Silent:{int(silent)}b,'
                           f'IsScreamingGoat:{int(screaming)}b,Tags:["goat_voice_{i}","{tag}"]}}')
                    reply(f'summon minecraft:goat {2.5 + i * 2} 220 {2.5 + j * 2} {nbt}')
                    observed = reply('execute if entity @e[tag=' + tag + ',nbt={'
                                     f'IsScreamingGoat:{int(screaming)}b' + '}]')
                    assert any('Test passed' in line for line in observed), (tag, observed)
                    flags.append(tag)
            advance(1)
            server.wait_for(rf'<{PLAYER}> goat voices tracked', timeout=120)
            reply('tick rate 200')
            phase('ambient')
            advance(2400)
            phase('hurt')
            for i in range(len(CASES)):
                reply(f'damage @e[tag=goat_voice_{i},limit=1] 1 minecraft:generic')
            # A one-tick interval leaves hurt immunity active and invalidates death controls.
            advance(21)
            phase('death')
            for i in range(len(CASES)):
                reply(f'damage @e[tag=goat_voice_{i},limit=1] 100 minecraft:generic')
            advance(1)
            server.console('say goat_voice_done')
            client.wait(timeout=180)
    finally:
        if client is not None and client.poll() is None:
            client.kill()
            client.wait()
        server.stop()
    lines = transcript.read_text().splitlines()
    assert not any(line.startswith('error ') for line in lines), lines[-10:]
    sounds = []
    for line in lines:
        if not line.startswith('play-goat-voice '):
            continue
        phase_name, name, category, volume, pitch, x, z = line.split()[1:]
        index = round((float(x) - 2.5) / 2)
        assert 0 <= index < len(CASES) and abs(float(x) - (2.5 + 2 * index)) < 0.2, line
        sounds.append({'case': CASES[index][0], 'phase': phase_name, 'name': name,
                       'category': int(category), 'volume': float(volume), 'pitch': float(pitch)})
    return {'flags': flags, 'sounds': sounds}


def checks(result):
    failures, excluded = [], {}
    for name, screaming, baby, silent in CASES:
        for phase in ('ambient', 'hurt', 'death'):
            sounds = [s for s in result['sounds'] if s['case'] == name and s['phase'] == phase
                      and s['name'].endswith(('.ambient',) if phase == 'ambient' else ('.hurt', '.death'))]
            if silent and phase == 'ambient':
                excluded[name] = sounds
                continue
            if silent:
                valid = not sounds
            else:
                expected = 'minecraft:entity.goat.' + ('screaming.' if screaming else '') + phase
                lo, hi = (1.3, 1.7) if baby else (0.8, 1.2)
                valid = bool(sounds) and all(s['name'] == expected and s['category'] == 6
                                           and s['volume'] == 1 and lo - 1e-6 <= s['pitch'] <= hi + 1e-6
                                           for s in sounds)
            if not valid:
                failures.append({'case': name, 'phase': phase, 'sounds': sounds})
    return {'checks': len(result['flags']) + 16, 'failures': failures, 'excluded_silent_ambient': excluded}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon game (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'goat-voices-parity')
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    if args.binary:
        binary = args.binary.resolve(strict=True)
    else:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client',
                        '--bin', 'horizon-client'], cwd=lab.ROOT, check=True)
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
        binary = Path(metadata['target_directory']) / 'fast' / (
            'horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    classes = out / 'classes'
    classes.mkdir(exist_ok=True)
    source = lab.ROOT / 'mods/horizon/src/test/java'
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-sourcepath', source, '-cp', cp, '-d', classes,
                    source / 'dev/horizonlab/parity/VanillaProtocolClient.java'], check=True)
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + cp,
              'dev.horizonlab.parity.VanillaProtocolClient']
    commands = {'vanilla': ['java', '-Xmx2g', '-cp', cp, 'net.minecraft.server.Main', 'nogui'],
                'horizon': [binary, '--resources', lab.RESOURCES.resolve(),
                            '--dedicated', '--nogui']}
    results = {label: run_side(label, command, client, out) for label, command in commands.items()}
    report = {label: checks(result) for label, result in results.items()}
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'], **report})
    assert not report['vanilla']['failures'], report['vanilla']
    for label, case in report.items():
        print(f"{label}: {case['checks'] - len(case['failures'])}/{case['checks']} scoped checks pass", flush=True)
        for failure in case['failures']:
            print(json.dumps(failure), flush=True)
    assert not report['horizon']['failures'], f'scoped goat voice failures; see {out}'


if __name__ == '__main__':
    main()
