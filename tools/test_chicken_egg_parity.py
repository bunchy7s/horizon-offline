#!/usr/bin/env python3
"""Adult chicken egg timers and item output tick independently of NoAI.

Fresh dedicated servers cover both AI modes, baby and jockey exclusions.
No sound, vibration, or random-distribution parity claim.
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

CASES = [(scene, no_ai) for scene in ('countdown', 'lay', 'baby', 'jockey') for no_ai in (0, 1)]
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
        marker = f'chicken_egg_end_{serial}'
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
            assert remaining > 0, f'{label}: chicken scene did not load'
            ready = [reply(f'execute if loaded {x} 200 {z}', remaining)
                     for x, z in ((0, 0), (16, 0), (0, 16), (16, 16))]
            if all(any('Test passed' in m for m in messages) for messages in ready):
                break
        assert any('filled' in m for m in reply('fill 0 198 0 31 199 31 stone'))
        reply('scoreboard objectives add egg_probe dummy')
        for scene, no_ai in CASES:
            reply('kill @e[type=!minecraft:player]')
            advance(25)
            reply('kill @e[type=minecraft:item]')
            assert any('Test failed' in m for m in reply('execute if entity @e[tag=subject]'))
            assert any('Test failed' in m for m in reply('execute if entity @e[type=minecraft:item]'))
            reply('fill 0 200 0 31 206 31 air')
            case = {'scene': scene, 'no_ai': no_ai}

            def record(text, field, tick):
                records.append({**case, 'field': field, 'tick': tick,
                                'command': text, 'messages': reply(text)})

            initial = 3 if scene == 'lay' else 30
            extra = ',Age:-10000' if scene == 'baby' else ',IsChickenJockey:1b' if scene == 'jockey' else ''
            nbt = (f'{{Tags:["subject","egg_{scene}_{no_ai}"],NoAI:{no_ai}b,NoGravity:1b,Silent:1b,'
                   f'PersistenceRequired:1b,EggLayTime:{initial}{extra}}}')
            assert any('Summoned new' in m for m in reply(f'summon minecraft:chicken 10.5 200 10.5 {nbt}'))
            previous = 0
            for tick in (0, 1, 3, 10):
                if tick:
                    advance(tick - previous)
                previous = tick
                reply('execute store result score subjects egg_probe run execute if entity @e[tag=subject]')
                record('execute if score subjects egg_probe matches 1', 'subject_count', tick)
                timer = ('6000..11999' if tick == 3 else '5993..11992') if scene == 'lay' and tick >= 3 else str(
                    initial - tick if scene in ('countdown', 'lay') else initial)
                reply('execute store result score timer egg_probe run data get entity @e[tag=subject,limit=1] EggLayTime')
                timer_test = f'execute if score timer egg_probe matches {timer}'
                if scene == 'lay' and tick == 3:
                    reply('scoreboard players operation reset egg_probe = timer egg_probe')
                elif scene == 'lay' and tick == 10:
                    reply('scoreboard players remove reset egg_probe 7')
                    timer_test += ' if score timer egg_probe = reset egg_probe'
                record(timer_test, 'egg_timer', tick)
                reply('execute store result score eggs egg_probe run execute if entity '
                      '@e[type=minecraft:item,nbt={Item:{id:"minecraft:egg",count:1}}]')
                eggs = int(scene == 'lay' and tick >= 3)
                record(f'execute if score eggs egg_probe matches {eggs}', 'egg_items', tick)
                condition = 'if' if no_ai else 'unless'
                record(f'execute {condition} entity @e[tag=subject,nbt={{NoAI:1b}}]', 'no_ai', tick)
        return records
    finally:
        server.stop()


def validate_reference(records):
    for row in records:
        assert len(row['messages']) == 1 and row['messages'][0].startswith('Test passed'), row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='prebuilt Horizon server (otherwise build fast)')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'chicken-egg-parity')
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
    lab.write_json(out / 'records.json', results)
    expected, actual = results['vanilla'], results['horizon']
    assert len(expected) == len(actual) == 128
    validate_reference(expected)
    failures = [{'vanilla': v, 'horizon': h} for v, h in zip(expected, actual) if v != h]
    lab.write_json(out / 'report.json', {'schema': 1, 'minecraft': lock['minecraft']['version'],
                                       'observations': len(expected), 'failures': failures,
                                       'scope': 'Egg timer/output; no sound/vibration/random-distribution claim.'})
    print(f'{len(expected) - len(failures)}/{len(expected)} observations match', flush=True)
    for failure in failures:
        print(json.dumps(failure), flush=True)
    assert not failures, f'{len(failures)} chicken egg observations differ; see {out}'


if __name__ == '__main__':
    main()
