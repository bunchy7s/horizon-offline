#!/usr/bin/env python3
"""Peaceful /summon admission, with plain and NBT forms on real fresh servers.

Compare command feedback before interpreting entity existence or Health. The
two-tick observation is a command-admission control, not a despawn/lifetime test.
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

SPECIES = ('camel_husk', 'camel', 'husk', 'zombie_horse')
HEALTH = {'camel_husk': 32, 'camel': 32, 'husk': 20, 'zombie_horse': 25}
MESSAGE = re.compile(r'\[Server thread/INFO\]: (?:System chat: )?(.*)$')


def run_side(command, out, label):
    directory = out / label
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = {'server-ip': '127.0.0.1', 'server-port': str(free_port()),
             'online-mode': 'false', 'white-list': 'false', 'level-type': 'minecraft:flat',
             'generate-structures': 'false', 'level-seed': '42', 'view-distance': '2',
             'simulation-distance': '2', 'spawn-protection': '0',
             'pause-when-empty-seconds': '-1', 'max-tick-time': '-1'}
    (directory / 'server.properties').write_text(
        ''.join(f'{k}={escape(v)}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server(command, directory, label)
    serial, records = 0, []

    def reply(text, timeout=180):
        nonlocal serial
        serial += 1
        marker = f'peaceful_summon_end_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$', timeout=timeout)
        return [m.group(1) for line in server.lines[start:] if (m := MESSAGE.search(line))
                and not m.group(1).endswith(f'[Server] {marker}')]

    def advance(ticks):
        start = game_time(server)
        reply(f'tick step {ticks}')
        deadline = time.monotonic() + 180
        while game_time(server) < start + ticks:
            assert time.monotonic() < deadline, f'{label}: tick step stalled'
        assert game_time(server) == start + ticks

    try:
        server.wait_for(r'Done \(')
        reply('tick freeze')
        reply('tick rate 200')
        assert any('false' in m for m in reply('gamerule minecraft:spawn_mobs false'))
        reply('forceload add 0 0 31 31')
        deadline = time.monotonic() + 300
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'{label}: summon area did not load'
            ready = [reply(f'execute if loaded {x} 200 {z}', remaining)
                     for x, z in ((0, 0), (16, 0), (0, 16), (16, 16))]
            if all(any('Test passed' in m for m in messages) for messages in ready):
                break
        assert any('filled' in m for m in reply('fill 0 198 0 31 199 31 stone'))
        reply('fill 0 200 0 31 210 31 air')
        for difficulty in ('peaceful', 'normal'):
            reply(f'difficulty {difficulty}')
            assert any(difficulty in m.lower() for m in reply('difficulty'))
            for species in SPECIES:
                for form in ('plain', 'nbt'):
                    reply('kill @e[type=!minecraft:player]')
                    # Dead mobs remain addressable until their death animation expires.
                    advance(25)
                    selector = f'@e[type=minecraft:{species},limit=1]'
                    suffix = ' {NoAI:1b,NoGravity:1b}' if form == 'nbt' else ''
                    commands = [f'summon minecraft:{species} 10.5 200 10.5{suffix}',
                                f'execute if entity {selector}']
                    case = {'difficulty': difficulty, 'species': species, 'form': form}
                    for text in commands:
                        records.append({**case, 'command': text, 'messages': reply(text)})
                    advance(2)
                    for text in (f'execute if entity {selector}', f'data get entity {selector} Health'):
                        records.append({**case, 'command': text, 'messages': reply(text)})
        return records
    finally:
        server.stop()


def validate_reference(records):
    for i in range(0, len(records), 4):
        case = records[i:i + 4]
        first = case[0]
        rejected = first['difficulty'] == 'peaceful' and first['species'] == 'husk'
        summoned = any('Summoned new' in m for m in first['messages'])
        assert summoned != rejected, case
        if rejected:
            assert any('Peaceful' in m for m in first['messages']), case
        for row in case[1:3]:
            assert any(('Test failed' if rejected else 'Test passed') in m for m in row['messages']), case
        if rejected:
            assert any('No entity' in m for m in case[3]['messages']), case
        else:
            health = [float(m.group(1)) for line in case[3]['messages']
                      if (m := re.search(r'has the following entity data: ([\d.]+)f$', line))]
            assert health == [HEALTH[first['species']]], case


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon server (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'peaceful-summon-parity')
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
    cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
    commands = {'vanilla': ['java', '-Xmx2g', '-cp', cp, 'net.minecraft.server.Main', 'nogui'],
                'horizon': [binary, '--resources', lab.RESOURCES.resolve(), '--dedicated', '--nogui']}
    results = {label: run_side(command, out, label) for label, command in commands.items()}
    expected, actual = results['vanilla'], results['horizon']
    assert len(expected) == len(actual) == 64
    validate_reference(expected)
    failures = [{'vanilla': v, 'horizon': h} for v, h in zip(expected, actual) if v != h]
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'],
                                       'observations': len(expected), 'failures': failures,
                                       'scope': 'Summon admission; not field, despawn or natural-spawn parity.'})
    print(f'peaceful summon: {len(expected) - len(failures)}/{len(expected)} observations match', flush=True)
    for failure in failures:
        print(json.dumps(failure), flush=True)
    assert not failures, f'{len(failures)} summon observations differ; see {out}'


if __name__ == '__main__':
    main()
