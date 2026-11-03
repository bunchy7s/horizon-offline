#!/usr/bin/env python3
"""Projectile-triggered big-dripleaf tilt on fresh Java/Horizon servers.

Arrow, snowball, ender pearl and powered-arrow hit scenes plus an unhit control.
Only block tilt and its reset timing are compared, not sounds or vibrations.
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

SCENES = [('arrow', 2, 2), ('snowball', 6, 2), ('ender_pearl', 10, 2),
          ('powered_arrow', 2, 6), ('no_hit', 6, 6)]
CHECKPOINTS = (1, 2, 40, 100, 101, 130)
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
        marker = f'dripleaf_projectile_end_{serial}'
        start = len(server.lines)
        server.console(text)
        server.console(f'say {marker}')
        server.wait_for(re.escape(f'[Server] {marker}') + '$', timeout=timeout)
        return [m.group(1) for line in server.lines[start:] if (m := MESSAGE.search(line))
                and not m.group(1).endswith(f'[Server] {marker}')]

    def record(text, tick=None):
        records.append({'command': text, 'tick': tick, 'messages': reply(text)})

    def advance(ticks):
        start = game_time(server)
        reply(f'tick step {ticks}')
        deadline = time.monotonic() + 180
        while game_time(server) < start + ticks:
            assert time.monotonic() < deadline, f'{label}: tick step stalled'
        assert game_time(server) == start + ticks

    def snapshot(tick):
        for name, x, z in SCENES:
            tilt = 'full' if name != 'no_hit' and 1 <= tick <= 100 else 'none'
            record(f'execute if block {x} 200 {z} big_dripleaf[tilt={tilt}]', tick)

    try:
        server.wait_for(r'Done \(')
        reply('tick freeze')
        reply('tick rate 200')
        reply('forceload add 0 0 15 31')
        deadline = time.monotonic() + 300
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'{label}: scene chunks did not load'
            ready = [reply(f'execute if loaded 0 200 {z}', remaining) for z in (0, 16)]
            if all(any('Test passed' in m for m in messages) for messages in ready):
                break
        for text in ('gamerule minecraft:random_tick_speed 0', 'gamerule minecraft:spawn_mobs false',
                     'gamerule minecraft:mob_griefing false', 'fill 0 198 0 15 199 31 stone',
                     'fill 0 200 0 15 204 31 air'):
            record(text)
        for _, x, z in SCENES:
            record(f'setblock {x} 199 {z} clay')
            record(f'setblock {x} 200 {z} big_dripleaf[facing=north]')
        record('setblock 1 200 6 redstone_block')
        snapshot(0)
        for name, x, z in SCENES:
            kind = name if name in ('snowball', 'ender_pearl') else 'arrow'
            y, nbt = (204, '{Motion:[0d,0d,0d],NoGravity:1b}') if name == 'no_hit' else (
                201.5, '{Motion:[0d,-1d,0d]}')
            messages = reply(f'summon {kind} {x + 0.5} {y} {z + 0.5} {nbt}')
            assert any('Summoned new' in m for m in messages), messages
        previous = 0
        for tick in CHECKPOINTS:
            advance(tick - previous)
            snapshot(tick)
            previous = tick
        return records
    finally:
        server.stop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon server (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'dripleaf-projectiles-parity')
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
    assert len(expected) == len(actual) == 51
    for row in expected:
        assert row['messages'], row
        if row['tick'] is not None:
            assert any('Test passed' in m for m in row['messages']), row
    failures = [{'vanilla': v, 'horizon': h} for v, h in zip(expected, actual) if v != h]
    lab.write_json(out / 'records.json', results)
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'],
                                       'observations': len(expected), 'failures': failures,
                                       'scope': 'Projectile-hit tilt and reset timing; no sound/vibration claim.'})
    print(f'dripleaf projectiles: {len(expected) - len(failures)}/{len(expected)} observations match', flush=True)
    for failure in failures:
        print(json.dumps(failure), flush=True)
    assert not failures, f'{len(failures)} dripleaf projectile observations differ; see {out}'


if __name__ == '__main__':
    main()
