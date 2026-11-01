#!/usr/bin/env python3
"""Small rules of vanilla's ServerGamePacketListenerImpl on Horizon's dedicated server, end to end.

    python3 tools/server_packet_rules_e2e.py --bin-dir target/fast

A survival dedicated server in a flat world, a world border 20 blocks wide around 0 0 (no damage):
- handleAttack: a player outside the world border isn't hit, from 3 blocks away; a pair inside it fights;
- handleSetCarriedItem: hotbar slot 12 is refused, and the console says "<name> tried to set an invalid
  carried item";
- handleChangeGameMode: a player without the permission asks for creative; the console says
  "Player <name> tried to change game mode to Creative Mode without required permissions";
- handleSetBeaconPacket: a player opens a beacon and sets Speed without paying; vanilla disconnects them
  (multiplayer.disconnect.generic) and warns "Player <name> tried to set invalid beacon effects".

horizon-bots stand in for the clients (`--attack`, `--raw`). The server runs in a fresh folder with fresh
HORIZON_HOME and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import re
import shutil
import struct
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
# A flat world's grass top is at y = -61.
GROUND = -60
BEACON = (2, GROUND, 0)
SECONDS = 14
SPEED = 0  # minecraft:speed in the mob_effect registry


class Failed(Exception):
    pass


def serverbound_ids():
    """The play state's serverbound packet ids, from horizon-protocol's table."""
    text = (lab.ROOT / 'crates/horizon-protocol/src/ids.rs').read_text()
    play = text[text.index('pub mod play {'):]
    block = play[play.index('pub mod serverbound {'):]
    block = block[:block.index('}')]
    return {m[1]: int(m[2]) for m in re.finditer(r'pub const (\w+): i32 = (\d+);', block)}


def varint(n):
    out = b''
    n &= 0xFFFFFFFF
    while True:
        if n < 0x80:
            return out + bytes([n])
        out += bytes([n & 0x7F | 0x80])
        n >>= 7


def block_pos(x, y, z):
    return struct.pack('>q', ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF))


def bot(bins, port, out, name, behaviour, *extra):
    report = out / f'{name}.json'
    subprocess.run([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                    '--behaviour', behaviour, '--height', '0', '--seconds', str(SECONDS), '--name', name,
                    '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=SECONDS + 180)
    return json.loads(report.read_text())['per_bot'][0]


def run(bins, folder, out):
    ids = serverbound_ids()
    use_on = (varint(0) + block_pos(*BEACON) + varint(1) + struct.pack('>fff', 0.5, 1.0, 0.5)
              + b'\0\0' + varint(1))
    # Speed as the primary power, no secondary, and nothing paid.
    beacon = b'\1' + varint(SPEED) + b'\0'
    raw = ','.join([
        f'40:{ids["SET_CARRIED_ITEM"]}:{struct.pack(">h", 12).hex()}',
        f'40:{ids["CHANGE_GAME_MODE"]}:{varint(1).hex()}',
        f'60:{ids["USE_ITEM_ON"]}:{use_on.hex()}',
        f'80:{ids["SET_BEACON"]}:{beacon.hex()}',
    ])
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / 'server.log')
    try:
        server.wait_for(DONE, 300)
        for line in ('setworldspawn 0 -60 0', 'gamerule respawn_radius 0', 'worldborder center 0 0',
                     'worldborder set 20', 'worldborder damage amount 0',
                     'setblock {} {} {} minecraft:beacon'.format(*BEACON)):
            server.console(line)
        with ThreadPoolExecutor(5) as pool:
            # (victim, attacker): outside the border, then a pair inside it.
            pairs = [(pool.submit(bot, bins, port, out, f'V{i}', 'stand'),
                      pool.submit(bot, bins, port, out, f'A{i}', 'stand', '--attack', f'V{i}0',
                                  '--attack-after', '6'))
                     for i in range(2)]
            rules = pool.submit(bot, bins, port, out, 'R', 'idle', '--raw', raw)
            # Side by side first, so each attacker sees its victim (and its entity id), then apart.
            for i in range(2):
                for name in (f'V{i}0', f'A{i}0'):
                    server.wait_for(rf'{name} joined the game', 60)
                z = 5.5 * (1 if i else -1)
                server.console(f'tp V{i}0 0.5 {GROUND} {z}')
                server.console(f'tp A{i}0 1.5 {GROUND} {z}')
            time.sleep(2)
            server.console(f'tp V00 12.5 {GROUND} -5.5')
            server.console(f'tp A00 9.5 {GROUND} -5.5')
            server.console(f'tp V10 -3.5 {GROUND} 5.5')
            server.console(f'tp A10 -0.5 {GROUND} 5.5')
            results = [(v.result(), a.result()) for v, a in pairs]
            rule_bot = rules.result()
        lines = server.since(0)
    finally:
        server.stop()
    failed = []
    for (victim, attacker), (where, hits) in zip(results, [('outside the border', False),
                                                            ('inside the border', True)]):
        health = victim['health']
        hit = health is not None and health < 20
        line = f'a victim {where}: {attacker["attacks"]} attacks, health {health}'
        print(line)
        if attacker['attacks'] == 0 or health is None:
            failed.append(f'{line}: the bots never fought ({attacker["error"]}, {victim["error"]})')
        elif hit != hits:
            failed.append(f'{line}: expected {"a hit" if hits else "no hit"}')
    for want in ('R0 tried to set an invalid carried item',
                 'Player R0 tried to change game mode to Creative Mode without required permissions',
                 'Player R0 tried to set invalid beacon effects'):
        if any(want in l for l in lines):
            print(f'console: {want}')
        else:
            failed.append(f'the console never said "{want}"')
    if rule_bot['error']:
        print(f'R0 was disconnected: {rule_bot["error"]}')
    else:
        failed.append('R0 was not disconnected after the unpaid beacon')
    if failed:
        raise Failed('\n'.join(failed))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-packet-rules-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-packet-rules-e2e-') as scratch:
        try:
            run(bins, Path(scratch) / 'server', out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_packet_rules_e2e failed (logs in {out}):\n{e}')
    print('server_packet_rules_e2e: the border, carried item, game mode and beacon rules hold')


if __name__ == '__main__':
    main()
