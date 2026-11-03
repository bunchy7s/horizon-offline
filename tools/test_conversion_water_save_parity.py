#!/usr/bin/env python3
"""Saved InWaterTime uses cached eye immersion, without changing conversion.

Real dedicated servers: dry, feet-only, submerged and water-removed zombie
scenes, plus dry controls for the four other Zombie-derived entity types.
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

CASES = [('primary', 'zombie', scene) for scene in ('dry', 'feet', 'submerged', 'removed')]
CASES += [('family', kind, 'dry') for kind in ('husk', 'drowned', 'zombie_villager', 'zombified_piglin')]
MESSAGE = re.compile(r'\[Server thread/INFO\]: (?:System chat: )?(.*)$')


def run_side(command, out, label):
    directory = out / label
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = {'server-ip': '127.0.0.1', 'server-port': str(free_port()),
             'online-mode': 'false', 'white-list': 'false', 'level-type': 'minecraft:flat',
             'generate-structures': 'false', 'level-seed': '42', 'difficulty': 'normal',
             'view-distance': '2', 'simulation-distance': '2', 'spawn-protection': '0',
             'pause-when-empty-seconds': '-1', 'max-tick-time': '-1'}
    (directory / 'server.properties').write_text(
        ''.join(f'{k}={escape(v)}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server(command, directory, label)
    serial, records = 0, []

    def reply(text, timeout=180):
        nonlocal serial
        serial += 1
        marker = f'conversion_water_save_end_{serial}'
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
        assert any('normal' in m.lower() for m in reply('difficulty'))
        reply('forceload add 0 0 31 31')
        deadline = time.monotonic() + 300
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'{label}: water scene did not load'
            ready = [reply(f'execute if loaded {x} 200 {z}', remaining)
                     for x, z in ((0, 0), (16, 0), (0, 16), (16, 16))]
            if all(any('Test passed' in m for m in messages) for messages in ready):
                break
        assert any('filled' in m for m in reply('fill 0 198 0 31 199 31 stone'))
        for group, kind, scene in CASES:
            reply('kill @e[type=!minecraft:player]')
            advance(25)
            assert any('Test failed' in m for m in reply('execute if entity @e[tag=subject]'))
            reply('fill 0 200 0 31 206 31 air')
            if scene != 'dry':
                top = 200 if scene == 'feet' else 202
                reply(f'fill 9 200 9 11 {top} 11 water')
            case = {'group': group, 'kind': kind, 'scene': scene}

            def record(text, field, tick):
                records.append({**case, 'field': field, 'tick': tick,
                                'command': text, 'messages': reply(text)})

            nbt = '{Tags:["subject"],NoAI:1b,NoGravity:1b,Silent:1b,InWaterTime:37'
            nbt += ',DrownedConversionTime:12}' if group == 'primary' else '}'
            record(f'summon minecraft:{kind} 10.5 200 10.5 {nbt}', 'summon', 0)
            record('execute if entity @e[tag=subject]', 'exists', 0)
            for tick in (0, 2, 4):
                if tick:
                    if tick == 4 and scene == 'removed':
                        reply('fill 9 200 9 11 202 11 air')
                    advance(2)
                record('execute if block 10 201 10 water', 'eye_block', tick)
                for field in ('Pos', 'InWaterTime') + (('DrownedConversionTime',) if group == 'primary' else ()):
                    record(f'data get entity @e[tag=subject,limit=1] {field}', field, tick)
        return records
    finally:
        server.stop()


def validate_reference(records):
    for row in records:
        field, tick, scene, lines = (row[k] for k in ('field', 'tick', 'scene', 'messages'))
        if field == 'summon':
            valid = any('Summoned new' in line for line in lines)
        elif field == 'exists':
            valid = any('Test passed' in line for line in lines)
        elif field == 'eye_block':
            wet = scene == 'submerged' or (scene == 'removed' and tick != 4)
            valid = any(('Test passed' if wet else 'Test failed') in line for line in lines)
        elif field == 'Pos':
            positions = [list(map(lambda n: float(n.strip().rstrip('d')), m.group(1).split(',')))
                         for line in lines if (m := re.search(r'data: \[([^]]+)\]$', line))]
            valid = positions == [[10.5, 200.0, 10.5]]
        else:
            values = [int(m.group(1)) for line in lines if (m := re.search(r'data: (-?\d+)$', line))]
            expected = 12 if field == 'DrownedConversionTime' else (
                37 if tick > 0 and (scene == 'submerged' or scene == 'removed' and tick == 2) else -1)
            valid = values == [expected]
        assert valid, row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon server (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'conversion-water-save-parity')
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
    assert len(expected) == len(actual) == 100
    validate_reference(expected)
    failures = [{'vanilla': v, 'horizon': h} for v, h in zip(expected, actual) if v != h]
    groups = {}
    for group in ('primary', 'family'):
        total = sum(row['group'] == group for row in expected)
        failed = sum(row['vanilla']['group'] == group for row in failures)
        groups[group] = {'observations': total, 'failures': failed}
        print(f'{group}: {total - failed}/{total} observations match', flush=True)
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'],
                                       'groups': groups, 'failures': failures,
                                       'scope': 'Saved eye-water predicate; no conversion/defaults/packets/reload claim.'})
    for failure in failures:
        print(json.dumps(failure), flush=True)
    assert not failures, f'{len(failures)} saved water observations differ; see {out}'


if __name__ == '__main__':
    main()
