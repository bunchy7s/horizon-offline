#!/usr/bin/env python3
"""Compare living-mob dragon contacts with the real Java 26.3 methods.

Checks health, resulting velocity and revenge attribution for seven contact
scenarios. Does not claim parity of the whole fight, AI pathfinding, or gear
with post-attack enchantments. The game integration check is separate.
"""
import argparse
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile

try:
    from . import lab, test_rust_parity as base
    from .integration_env import game_environment
except ImportError:
    import lab
    import test_rust_parity as base
    from integration_env import game_environment


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--classpath', help='already verified vanilla server and libraries')
    parser.add_argument('--resources', type=Path, help='also check the integrated-server game path')
    parser.add_argument('--binary', type=Path, help='already built game for --resources')
    args = parser.parse_args()
    build = lab.LOCAL / 'dragon-contacts-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    cp = args.classpath
    major = '25'
    if not cp:
        lock, jars, libraries = base.verify_artifacts()
        cp = os.pathsep.join(map(str, [jars['server'], *libraries]))
        major = str(lock['minecraft']['java_major'])
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    source = lab.ROOT / 'mods/horizon/src/test/java'
    oracle = source / 'dev/horizonlab/parity'
    def run(command, name, **kwargs):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), stdout=log,
                           stderr=subprocess.STDOUT, check=True, timeout=240, **kwargs)
    run(['javac', '--release', major, '-proc:none', '-cp', cp, '-d', classes,
         oracle / 'VanillaCommandExecOracle.java',
         oracle / 'VanillaDragonContactsOracle.java'], 'compile-java')
    script = build / 'script.jsonl'
    script.write_text('{"op":"dragon_contacts"}\n')
    expected_path = build / 'java.json'
    actual_path = build / 'rust.json'
    for p in [expected_path, actual_path]:
        p.unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix='hz-dragon-contacts-') as scratch:
        env = game_environment(os.environ, scratch)
        run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + cp,
             'dev.horizonlab.parity.VanillaCommandExecOracle',
             Path(scratch) / 'universe', script, expected_path, '1'], 'java', env=env)
    run(['cargo', 'test', '--locked', '-p', 'horizon-client-lib', '--lib',
         'mobs::dragon_contacts::', '--', '--include-ignored'], 'rust',
        env=dict(os.environ, HORIZON_DRAGON_CONTACT_OUTPUT=str(actual_path)))
    expected = json.loads(expected_path.read_text())
    actual = json.loads(actual_path.read_text())
    assert len(actual) == len(expected) == 7
    for want, got in zip(expected, actual):
        for key in ['case', 'health', 'revenge']:
            assert want[key] == got[key], (key, want, got)
        for a, b in zip(want['velocity'], got['velocity']):
            assert math.isclose(a, b, rel_tol=0, abs_tol=1e-12), (want, got)
    integration = False
    if args.resources:
        if not args.binary:
            parser.error('--resources requires --binary')
        ops = []
        def cmd(text):
            ops.append({'op': 'cmd', 'cmd': text, 'record': False})
        for c in ['/gamemode creative', '/gamerule spawn_mobs false',
                  '/gamerule mob_griefing false',
                  '/execute in minecraft:the_end run tp @s 8.5 205 8.5']:
            cmd(c)
        ops += [{'op': 'settle'}, {'op': 'tick', 'n': 260}]
        cmd('/kill @e[type=minecraft:ender_dragon]')
        cmd('/summon minecraft:ender_dragon 8 200 8 {DragonPhase:10}')
        cmd('/summon minecraft:enderman 8 200 1.5 {NoGravity:1b,Tags:["contact_test"]}')
        ops.append({'op': 'tick', 'n': 1})
        cmd('/data merge entity @e[tag=contact_test,limit=1] {Invulnerable:1b}')
        ops += [{'op': 'tick', 'n': 24}, {'op': 'mobs'}]
        game_script = build / 'game-script.jsonl'
        game_script.write_text(''.join(json.dumps(op) + '\n' for op in ops))
        game_output = build / 'game.jsonl'
        with tempfile.TemporaryDirectory(prefix='hz-dragon-integration-') as scratch:
            env = game_environment(os.environ, scratch)
            env.update(HORIZON_COMMAND_SCRIPT=str(game_script),
                       HORIZON_COMMAND_OUTPUT=str(game_output), HORIZON_HARNESS_TIMEOUT='120')
            run([args.binary.resolve(), '--resources', args.resources.resolve(),
                 '--world', 'contacts', '--seed', '1', '--radius', '4'], 'game', env=env, cwd=scratch)
        observations = [json.loads(line) for line in game_output.read_text().splitlines()]
        mobs = observations[-1]['near']
        endermen = [m for m in mobs if m['kind'] == 'minecraft:enderman']
        assert len(endermen) == 1, endermen
        mob = endermen[0]
        assert mob['health'] < 40 and str(mob['target']).startswith('Mob('), mob
        integration = True
    report.write_text(json.dumps({'passed': True, 'cases': len(actual),
                                 'integrated_server': integration}, indent=2) + '\n')
    print('dragon contacts: 7 Java/Rust cases match')


if __name__ == '__main__':
    main()
