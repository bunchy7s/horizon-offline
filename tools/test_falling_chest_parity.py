#!/usr/bin/env python3
"""Falling chests create block entities and retain carried inventory on landing.

Fresh Java/Horizon dedicated servers, bounded loaded-coordinate readiness and
actual tick completion. Direct chest/data and ordinary falling sand controls
keep landing, block-entity creation and inventory transfer distinguishable.
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

MESSAGE = re.compile(r'\[Server thread/INFO\]: (?:System chat: )?(.*)$')
ITEMS = '[{Slot:0b,id:"minecraft:diamond",count:1}]'
LOADED = [f'execute if loaded {x} 200 {z}' for x, z in ((0, 0), (16, 0), (0, 16), (16, 16))]


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
        marker = f'falling_chest_end_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$', timeout=timeout)
        return [m.group(1) for line in server.lines[start:] if (m := MESSAGE.search(line))
                and not m.group(1).endswith(f'[Server] {marker}')]

    def record(text):
        records.append({'command': text, 'messages': reply(text)})

    def advance(ticks):
        start = game_time(server)
        reply(f'tick step {ticks}')
        deadline = time.monotonic() + 180
        while game_time(server) < start + ticks:
            assert time.monotonic() < deadline, f'{label}: tick step stalled'
        assert game_time(server) == start + ticks

    def inventory(pos):
        record(f'execute if data block {pos} Items[{{Slot:0b,id:"minecraft:diamond",count:1}}]')
        for field in ('Slot', 'id', 'count'):
            record(f'data get block {pos} Items[0].{field}')

    try:
        server.wait_for(r'Done \(')
        reply('tick freeze')
        reply('tick rate 200')
        reply('gamerule minecraft:spawn_mobs false')
        reply('forceload add 0 0 31 31')
        deadline = time.monotonic() + 300
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'{label}: landing area did not load'
            ready = [reply(c, timeout=remaining) for c in LOADED]
            if all(any('Test passed' in line for line in lines) for lines in ready):
                break
        records += [{'command': c, 'messages': lines} for c, lines in zip(LOADED, ready)]
        reply('fill 0 198 0 31 199 31 stone')
        reply('fill 0 200 0 31 224 31 air')
        reply('kill @e[type=minecraft:falling_block]')
        # Existing placement and data paths are controls, not part of this fix.
        reply('setblock 6 200 10 chest')
        record('execute if block 6 200 10 chest')
        record('data get block 6 200 10 Items')
        reply(f'data modify block 6 200 10 Items set value {ITEMS}')
        inventory('6 200 10')
        for name, x, block, data in (('empty', 10, 'chest', ''),
                                     ('filled', 12, 'chest', ',TileEntityData:{Items:' + ITEMS + '}'),
                                     ('sand', 14, 'sand', '')):
            tag = f'falling_chest_{name}'
            reply(f'summon falling_block {x}.5 203 10.5 '
                  f'{{BlockState:"minecraft:{block}",Tags:["{tag}"]{data}}}')
            record(f'execute if entity @e[type=minecraft:falling_block,tag={tag}]')
            if data:
                # The supplied data is present before landing on both servers.
                for field in ('id', 'count'):
                    record(f'data get entity @e[tag={tag},limit=1] TileEntityData.Items[0].{field}')
            advance(30)
            record(f'execute if block {x} 200 10 {block}')
            record(f'execute if entity @e[type=minecraft:falling_block,tag={tag}]')
            if name == 'empty':
                record(f'data get block {x} 200 10 Items')
            elif name == 'filled':
                inventory(f'{x} 200 10')
        return records
    finally:
        server.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon server (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'falling-chest-parity')
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
    assert len(expected) == len(actual) == 26
    # Validate the reference's positive and negative controls independently.
    seen_entities = set()
    for row in expected:
        text, messages = row['command'], row['messages']
        if text.startswith('execute if entity'):
            outcome = 'Test failed' if text in seen_entities else 'Test passed'
            assert any(outcome in m for m in messages), row
            seen_entities.add(text)
            continue
        assert messages and not any('not a block entity' in m for m in messages), row
        if text.startswith('execute '):
            assert any('Test passed' in m for m in messages), row
    failures = [{'vanilla': v, 'horizon': h} for v, h in zip(expected, actual) if v != h]
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'],
                                       'observations': len(expected), 'failures': failures})
    print(f'falling chest: {len(expected) - len(failures)}/{len(expected)} observations match', flush=True)
    for failure in failures:
        print(json.dumps(failure), flush=True)
    assert not failures, f'{len(failures)} falling-chest observations differ; see {out}'


if __name__ == '__main__':
    main()
