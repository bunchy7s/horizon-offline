#!/usr/bin/env python3
"""A ghast killed by the fireball the player struck back grants Return to Sender.

Vanilla 26.3: the player's attack deflects the fireball (`Projectile.deflect`, the
player is its owner now). `LargeFireball.onHitEntity` hurts the ghast with
`damageSources().fireball(this, owner)`; `Ghast.hurtServer` takes a reflected
fireball (`isReflectedFireball`: a large fireball, caused by a player) as 1000
damage, and `LivingEntity.hurtServer` remembers the player (`setLastHurtByPlayer`).
On death, `LivingEntity.die` credits that player (`awardKillScore`), whose
`player_killed_entity` trigger matches `nether/return_to_sender`'s killing blow:
a `#is_projectile` damage whose direct entity is a `minecraft:fireball`.

The ghast died, but the advancement never came on the default single player
(the host on its server thread plays as a guest there): the fireball's hit did
not record which player hurt the ghast, as an arrow's does, so the kill went to
nobody (#4212). Both single-player paths: the server thread and in-frame.

  test_return_to_sender_parity.py [--bin-dir DIR] [--keep]
"""
import argparse
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from test_world_reopen_parity import game_env  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
RESOURCES = ROOT / '.local' / 'horizon-client' / '26.3'
RUN = ROOT / '.local' / 'return-to-sender'
MODES = {'server thread': [], 'in-frame': ['--in-frame']}
TITLE = 'advancements.nether.return_to_sender.title'


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build them with the fast profile)')
    parser.add_argument('--keep', action='store_true', help='keep the run folder')
    args = parser.parse_args()
    bin_dir = Path(args.bin_dir).resolve() if args.bin_dir else None
    if bin_dir is None:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--bins'], cwd=ROOT, check=True)
        bin_dir = ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'fast'
    shutil.rmtree(RUN, ignore_errors=True)
    RUN.mkdir(parents=True)
    env, extra = game_env()
    failures = 0
    for index, (mode, flags) in enumerate(MODES.items()):
        folder = RUN / f'h{index}'
        folder.mkdir()
        socket = Path(tempfile.mkdtemp(prefix='hzrs')) / 'control.sock'  # sockets need short paths

        def ctl(*words, check=True):
            result = subprocess.run([str(bin_dir / 'horizon-ctl'), '--socket', str(socket), '--timeout', '600',
                                     *map(str, words)], cwd=folder, env=env, capture_output=True, text=True)
            if check and result.returncode:
                sys.exit(f'horizon-ctl {" ".join(map(str, words))} failed: {result.stdout}{result.stderr}')
            return json.loads(result.stdout) if result.stdout.strip().startswith('{') else result.stdout

        # The game keeps launch's output handles, so it writes to a file: a pipe would not close.
        with open(folder / 'launch.txt', 'w') as out:
            subprocess.run([str(bin_dir / 'horizon-ctl'), '--socket', str(socket), '--timeout', '600', 'launch',
                            '--resources', str(RESOURCES), '--world', 'world', '--seed', '1', '--render-distance', '4',
                            *flags, *extra], cwd=folder, env=env, stdout=out, stderr=subprocess.STDOUT, check=True)
        try:
            ctl('wait-ready', '300')
            x, _, z = (math.floor(v) for v in ctl('status')['player']['position'])
            y = 200
            # A glass floor in the sky: the player at its south end, facing a ghast that
            # does not move or shoot, and a still fireball just in front of the player.
            ctl('command', 'effect give @s resistance infinite 255 true')
            ctl('command', f'fill {x - 3} {y - 1} {z - 2} {x + 3} {y - 1} {z + 12} minecraft:glass')
            ctl('tp', x + 0.5, y, z + 0.5)
            ctl('wait', '10')
            ctl('command', f'summon minecraft:ghast {x + 0.5} {y - 1} {z + 10.5} {{NoAI:1b}}')
            ctl('command', f'summon minecraft:fireball {x + 0.5} {y + 1.3} {z + 2.5} '
                           '{Motion:[0.0d,0.0d,0.0d],acceleration_power:0.0d}')
            ctl('wait', '5')
            # Aim at the fireball's middle: struck back along the view, it flies into the ghast.
            ctl('look-at', x + 0.5, y + 1.8, z + 2.5)
            ctl('attack')
            ctl('wait', '60')
            ghasts = [e for e in ctl('entities')['entities'] if e.get('type') == 'minecraft:ghast']
            granted = TITLE in json.dumps(ctl('messages')['styled'])
        finally:
            ctl('quit', check=False)
        ok = not ghasts and granted
        failures += not ok
        print(f'{mode}:', 'ok' if ok else f'FAIL: ghast alive {bool(ghasts)}, Return to Sender granted {granted}')
    if not args.keep and not failures:
        shutil.rmtree(RUN, ignore_errors=True)
    print('return to sender:', 'PASS' if not failures else f'{failures} FAILED')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
