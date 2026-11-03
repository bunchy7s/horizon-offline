#!/usr/bin/env python3
"""Manual placement in fresh flat servers, with automatic structures off and on.

Compare console feedback from the pinned Java dedicated server and Horizon.
Readiness is four bounded `execute if loaded` probes, never a fixed tick wait.
This checks command availability/errors and the template's villager, not the
geometry or processors of generated structures.
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
    from .test_dedicated_server_parity import Server, free_port
    from .test_server_world_gen_parity import escape, structure_chunks
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port
    from test_server_world_gen_parity import escape, structure_chunks

SEED = -9147835479224470908
LOADED = [f'execute if loaded {x} 200 {z}' for x, z in ((0, 0), (16, 0), (0, 16), (16, 16))]
COMMANDS = [
    'place template minecraft:igloo/bottom 8 200 8',
    'place jigsaw minecraft:trail_ruins/tower minecraft:road_connector 0 8 200 8',
    'place jigsaw minecraft:trail_ruins/tower minecraft:road_connector 1 8 200 8',
    'place jigsaw minecraft:trail_ruins/tower audit:missing 1 8 200 8',
    'place jigsaw audit:missing minecraft:road_connector 1 8 200 8',
    'place structure minecraft:buried_treasure 8 200 8',
    'place structure audit:missing 8 200 8',
    'execute if entity @e[type=minecraft:villager]',
]
MESSAGE = re.compile(r'\[Server thread/INFO\]: (?:System chat: )?(.*)$')


def run_case(command, out, label, structures):
    directory = out / label / str(structures).lower()
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = {
        'server-ip': '127.0.0.1', 'server-port': str(free_port()),
        'online-mode': 'false', 'level-type': 'minecraft:flat',
        'level-seed': str(SEED), 'generate-structures': str(structures).lower(),
        'view-distance': '2', 'simulation-distance': '2', 'spawn-protection': '0',
        'pause-when-empty-seconds': '-1', 'max-tick-time': '-1',
    }
    (directory / 'server.properties').write_text(
        ''.join(f'{k}={escape(v)}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server(command, directory, f'{label}-{str(structures).lower()}')
    serial = 0

    def reply(text, timeout=180):
        nonlocal serial
        serial += 1
        marker = f'flat_place_end_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$', timeout=timeout)
        messages = [m.group(1) for line in server.lines[start:] if (m := MESSAGE.search(line))]
        return [m for m in messages if not m.endswith(f'[Server] {marker}')]

    try:
        server.wait_for(r'Done \(')
        reply('tick freeze')
        reply('gamerule minecraft:spawn_mobs false')
        reply('forceload add -32 -32 63 63')
        deadline = time.monotonic() + 300
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'{label}: force-loaded placement area did not become ready'
            ready = [reply(c, timeout=remaining) for c in LOADED]
            if all(any('Test passed' in line for line in lines) for lines in ready):
                break
        # Inspect saved starts before manual placement can change the world.
        automatic = None
        if not structures:
            reply('save-all flush')
            automatic = structure_chunks(directory / 'world')
        records = [{'command': c, 'messages': lines} for c, lines in zip(LOADED, ready)]
        records += [{'command': c, 'messages': reply(c)} for c in COMMANDS]
        return {'records': records, 'automatic': automatic}
    finally:
        server.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='use an already built Horizon server')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'flat-place-parity')
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
    classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
    commands = {
        'vanilla': ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', classpath,
                    'net.minecraft.server.Main', 'nogui'],
        'horizon': [binary, '--resources', lab.RESOURCES.resolve(), '--dedicated', '--nogui'],
    }
    results, summary, failures = {}, {}, []
    for structures in (False, True):
        key = str(structures).lower()
        results[key] = {label: run_case(command, out, label, structures)
                        for label, command in commands.items()}
        expected, actual = (results[key][label] for label in ('vanilla', 'horizon'))
        mismatches = [(v, h) for v, h in zip(expected['records'], actual['records']) if v != h]
        assert len(expected['records']) == len(actual['records']) == 12
        summary[key] = {'observations': 12, 'mismatches': len(mismatches)}
        print(f'flat structures={key}: {12 - len(mismatches)}/12 observations match', flush=True)
        failures += [{'structures': structures, 'vanilla': v, 'horizon': h} for v, h in mismatches]
        if not structures:
            empty = 'chunks with structure starts: 0, with references: 0'
            assert expected['automatic'] == actual['automatic'] == empty, results[key]
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'seed': SEED,
        'cases': summary, 'failures': failures,
        'scope': 'Manual flat-world placement feedback, readiness and entity control; '
                 'disabled automatic structure starts/references before manual commands. '
                 'No structure geometry or processor parity claim.',
    })
    for failure in failures:
        print(json.dumps(failure, ensure_ascii=False), flush=True)
    assert not failures, f'{len(failures)} placement observations differ; see {out}'


if __name__ == '__main__':
    main()
