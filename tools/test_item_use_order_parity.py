#!/usr/bin/env python3
"""Item use must precede a later move, on real vanilla and Horizon servers.

A vanilla-codec client writes UseItem, ClientTickEnd, MovePlayerPosRot and
ClientTickEnd together. The wind charge must originate at the old position
and inherit its old movement, standing or gliding. A deferred UseItem whose
later move overtakes it launches from eight blocks away with eight blocks of
horizontal velocity (#5828).

Spawn X is compared by block and velocity X rounded to a block/tick because
throw inaccuracy is random. This tests the old/new source, not that RNG or
client flight physics. Real integrated-server flight is checked separately.
"""
import os
from pathlib import Path
import subprocess

try:
    from . import lab, test_rust_parity as base, test_server_knockback_parity as net
except ImportError:
    import lab
    import test_rust_parity as base
    import test_server_knockback_parity as net

NAME = 'item-use-order-parity'


def cases():
    result = {}
    for glide in (False, True):
        result['gliding' if glide else 'standing'] = [
            'cmd Alice kill @e[type=!player]',
            'cmd Alice clear @s',
            'cmd Alice tp @s 0.5 -60 0.5 0 0',
            'pos Alice 0.5 -60 0.5 0 0 1', 'ticks 2',
            'cmd Alice item replace entity @s armor.chest with minecraft:elytra',
            'cmd Alice tp @s 0.5 100 0.5 0 90',
            'pos Alice 0.5 100 0.5 0 90 0',
            *(['glide Alice'] if glide else []), 'ticks 2',
            'cmd Alice give @s minecraft:wind_charge 1',
            'use-pos Alice 8.5 100 0.5 0 90 0', 'ticks 4',
        ]
    return result


def shown(lines):
    result = {}
    for line in lines:
        who, step, text = line.split(' ', 2)
        step = step.removeprefix('step=')
        if step in net.CASES and text.startswith(('wind source', 'error', 'disconnected')):
            result.setdefault(step, []).append(f'{who} {text}')
    return result


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/KnockbackClient.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn',
                    '-sourcepath', lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies,
                    '-d', classes, oracle], cwd=lab.ROOT, check=True, capture_output=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'],
                   cwd=lab.ROOT, check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-cp', str(classes) + os.pathsep + dependencies,
              'dev.horizonlab.parity.KnockbackClient']
    net.PLAYERS = ('Alice',)
    net.PROPERTIES['allow-flight'] = 'true'  # the scripted client holds its airborne position
    net.SETUP = ['cmd Alice gamerule spawn_mobs false']
    net.CASES = cases()
    world = net.generate_world(vanilla_server, build)
    native = net.run_side('vanilla', vanilla_server, client, build, world)
    rust = net.run_side('horizon', horizon_server, client, build, world)
    expected = {case: ['Alice wind source x=0 vx=0'] for case in net.CASES}
    assert shown(native) == expected, f'vanilla did not exercise the old source: {shown(native)}'
    assert any('step=gliding gliding true' in line for line in native), 'vanilla never started gliding'
    assert any('step=gliding gliding true' in line for line in rust), 'Horizon never started gliding'
    assert shown(rust) == expected, f'item use overtaken by a move: {shown(rust)}'
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'], cases=len(net.CASES),
                               scope=__doc__, output_sha256=lab.digest(build / 'horizon.txt', 'sha256')))
    print(f'item use order: {len(net.CASES)} cases match vanilla')


if __name__ == '__main__':
    main()
