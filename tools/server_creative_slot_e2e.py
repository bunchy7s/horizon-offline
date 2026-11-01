#!/usr/bin/env python3
"""ServerboundSetCreativeModeSlotPacket on a dedicated server, against vanilla's rules, end to end.

    python3 tools/server_creative_slot_e2e.py --bin-dir target/fast

A creative bot sends creative-slot packets, then the console reads its inventory:
- stone ×64 into slot 36 (the first hotbar slot) and ×1 into 45 (the offhand): kept;
- a diamond sword ×2 into 37 and ender pearls ×17 into 38 (more than their stack size): ignored
  (handleSetCreativeModeSlot's `validData`);
- stone into 0 (the crafting result) and 46 (past the inventory menu): ignored (`validSlot` is 1..45).
A second bot sends stone ×100: its codec takes counts of 1 to 99, so vanilla fails the packet and
disconnects the player.

horizon-bots stand in for the clients (`--give`). The server runs in a fresh folder with fresh HORIZON_HOME
and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_horizon_only_e2e import Failed, server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
STONE, DIAMOND_SWORD, ENDER_PEARL = 1, 1050, 1265
GIVE = [(36, STONE, 64), (45, STONE, 1), (37, DIAMOND_SWORD, 2), (38, ENDER_PEARL, 17), (0, STONE, 5),
        (46, STONE, 7)]
# Inventory NBT slots: the hotbar's 0..8 are menu slots 36..44. The offhand (menu 45) is saved in `equipment`.
KEPT = {0: ('minecraft:stone', 64)}


def bot(bins, port, out, name, give, seconds):
    report = out / f'{name}.json'
    spec = ','.join(f'{s}:{i}:{c}' for s, i, c in give)
    return report, subprocess.Popen(
        [bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1', '--behaviour', 'idle',
         '--height', '0', '--seconds', str(seconds), '--name', name, '--give', spec, '--report', report],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def entity_data(s, player, path):
    """The console's answer to `data get entity <player> <path>`."""
    mark = s.count()
    s.console(f'data get entity {player} {path}')
    deadline = time.time() + 30
    while time.time() < deadline:
        for line in s.since(mark):
            if 'has the following entity data' in line:
                return line.split('entity data:', 1)[1]
            if 'No entity was found' in line:
                raise Failed(f'{player} is not on the server')
        time.sleep(0.1)
    raise Failed(f'no answer to `data get entity {player} {path}`')


def parse_items(data):
    """`{slot: (id, count)}` from an Inventory list (one SNBT compound per item)."""
    items = {}
    for entry in re.findall(r'\{[^{}]*\}', data):
        slot = re.search(r'Slot: (-?\d+)b', entry)
        item = re.search(r'id: "([^"]+)"', entry)
        count = re.search(r'count: (\d+)', entry)
        if slot and item:
            items[int(slot[1])] = (item[1], int(count[1]) if count else 1)
    return items


def check(bins, scratch, out):
    s, port = server(bins, scratch / 'server', out, 'server', {})
    try:
        report, give = bot(bins, port, out, 'Give', GIVE, 20)
        s.wait_for(r'Give0 joined the game', 60)
        time.sleep(4)
        items = parse_items(entity_data(s, 'Give0', 'Inventory'))
        offhand = entity_data(s, 'Give0', 'equipment.offhand')
        over_report, over = bot(bins, port, out, 'Over', [(36, STONE, 100)], 8)
        over.wait(timeout=120)
        give.wait(timeout=120)
        lines = s.since(0)
    finally:
        s.stop()
    if items != KEPT:
        raise Failed(f'the inventory is {items}, not {KEPT}')
    count = re.search(r'count: (\d+)', offhand)
    if 'minecraft:stone' not in offhand or (count and count[1] != '1'):
        raise Failed(f'the offhand (slot 45) holds {offhand.strip()}, not one stone')
    print('slots 1..45 within the stack size are kept; slot 0, slot 46 and oversized stacks are ignored')
    entry = json.loads(over_report.read_text())['per_bot'][0]
    if 'count 100' not in (entry['error'] or ''):
        raise Failed(f'a stack of 100 did not disconnect the player: {entry["error"]!r}')
    if not any('Over0 sent an invalid creative stack' in l for l in lines):
        raise Failed('the console did not say why Over0 was disconnected')
    print(f'a stack of 100 disconnects: {entry["error"]}')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-creative-slot-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-creative-slot-e2e-') as scratch:
        try:
            check(bins, Path(scratch), out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_creative_slot_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
