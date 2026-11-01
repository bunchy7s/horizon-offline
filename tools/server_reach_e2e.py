#!/usr/bin/env python3
"""Vanilla's attack reach on Horizon's dedicated server, end to end.

ServerGamePacketListenerImpl.handleAttack drops an attack unless `Player.isWithinAttackRange(mainHandItem,
targetBox, 3.0)`: without an `attack_range` item, the eyes within the entity interaction range (3, 5 in
creative) plus 3 of the target's box. A client can send any entity id from any distance; the server must
refuse what the range doesn't allow.

    python3 tools/server_reach_e2e.py --bin-dir target/fast

A survival dedicated server in a flat world; pairs of horizon-bots stand on the ground (`--behaviour stand`),
placed by the console's /tp 200 blocks from the other pairs (side by side first, so the attacker knows its
victim's entity id, as a cheat would). In each pair the attacker sends
ServerboundAttackPacket at its victim twice a second (`--attack`) from:

- 3 blocks (survival): hits;
- 6 blocks (survival): hits; the victim's box is 5.7 from the eyes, within vanilla's 3 + 3;
- 7 blocks (survival): refused (6.7 from the eyes);
- 50 blocks (survival): refused;
- 7 blocks (creative, 5 + 3): hits;
- 9 blocks (creative): refused.

A hit shows in the victim's health (SetHealth, in the bots' report). The server runs in a fresh folder with
fresh HORIZON_HOME and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'survival',
    'difficulty': 'easy',
    'pvp': 'true',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
}
# (distance, attacker's game mode, hits)
CASES = [
    (3, 'survival', True),
    (6, 'survival', True),
    (7, 'survival', False),
    (50, 'survival', False),
    (7, 'creative', True),
    (9, 'creative', False),
]
# A flat world's grass top is at y = -61.
GROUND = -60
SECONDS = 16


class Failed(Exception):
    pass


def bot(bins, port, out, name, *extra):
    report = out / f'{name}.json'
    subprocess.run([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                    '--behaviour', 'stand', '--seconds', str(SECONDS), '--name', name, '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=SECONDS + 180)
    return json.loads(report.read_text())['per_bot'][0]


def run(bins, folder, out):
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / 'server.log')
    try:
        server.wait_for(DONE, 300)
        with ThreadPoolExecutor(2 * len(CASES)) as pool:
            jobs = []
            for i, _ in enumerate(CASES):
                jobs.append((pool.submit(bot, bins, port, out, f'V{i}'),
                             pool.submit(bot, bins, port, out, f'A{i}', '--attack', f'V{i}0',
                                         '--attack-after', '6')))
            # Bots are named PREFIX0: place each pair once both are in, side by side first, so the
            # attacker sees its victim (AddEntity gives it the id a cheat would have kept), then apart.
            for i, (_, mode, _) in enumerate(CASES):
                for name in (f'V{i}0', f'A{i}0'):
                    server.wait_for(rf'{name} joined the game', 60)
                z = 200 * i + 0.5
                server.console(f'gamemode survival V{i}0')
                server.console(f'gamemode {mode} A{i}0')
                server.console(f'tp V{i}0 0.5 {GROUND} {z}')
                server.console(f'tp A{i}0 1.5 {GROUND} {z}')
            time.sleep(2)
            for i, (distance, _, _) in enumerate(CASES):
                server.console(f'tp A{i}0 {0.5 + distance} {GROUND} {200 * i + 0.5}')
            results = [(v.result(), a.result()) for v, a in jobs]
    finally:
        server.stop()
    failed = []
    for (distance, mode, hits), (victim, attacker) in zip(CASES, results):
        health = victim['health']
        hit = health is not None and health < 20
        line = (f'{mode} attacker at {distance} blocks: {attacker["attacks"]} attacks, victim health {health} '
                f'({"hit" if hit else "refused"}, expected {"hit" if hits else "refused"})')
        print(line)
        if attacker['attacks'] == 0 or health is None:
            failed.append(f'{line}: the bots never fought ({attacker["error"]}, {victim["error"]})')
        elif hit != hits:
            failed.append(line)
    if failed:
        raise Failed('\n'.join(failed))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-reach-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-reach-e2e-') as scratch:
        try:
            run(bins, Path(scratch) / 'server', out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_reach_e2e failed (logs in {out}):\n{e}')
    print('server_reach_e2e: every attack within vanilla\'s range hit, every one beyond it was refused')


if __name__ == '__main__':
    main()
