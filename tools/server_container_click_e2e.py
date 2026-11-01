#!/usr/bin/env python3
"""ServerboundContainerClickPacket's slot range on a dedicated server, end to end.

    python3 tools/server_container_click_e2e.py --bin-dir target/fast

AbstractContainerMenu.isValidSlotIndex: a click is taken on -1, -999 or a slot below the menu's slot
count, and ignored past it. A chest's menu has 63 slots (27 + the player's 36). A bot opens a chest and
throws (click type 4, button 1: the whole stack) from slot 63, past the menu: nothing happens, and the
stone in the player's first inventory row (slot 9) stays. Before the fix, slot 63 reached that stone and
threw it.

horizon-bots stand in for the client (`--click`). The server runs in a fresh folder with fresh HORIZON_HOME
and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
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
CHEST = (2, -60, 0)
THROW = 4


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


def check(bins, scratch, out):
    s, port = server(bins, scratch / 'server', out, 'server', {})
    try:
        for line in ('setworldspawn 0 -60 0', 'gamerule respawn_radius 0',
                     'setblock {} {} {} minecraft:chest'.format(*CHEST)):
            s.console(line)
        click = '{},{},{}:{}:1:{}'.format(*CHEST, 63, THROW)
        bot = subprocess.Popen(
            [bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1', '--behaviour', 'idle',
             '--height', '0', '--seconds', '8', '--name', 'Click', '--click', click],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            s.wait_for(r'Click0 joined the game', 60)
            # Ten stacks: the hotbar (slots 0 to 8), then the main inventory's first slot, 9.
            s.console('give Click0 minecraft:stone 640')
            time.sleep(1)
            if not re.search(r'Slot: 9b', entity_data(s, 'Click0', 'Inventory')):
                raise Failed('`give` did not fill slot 9')
            time.sleep(5)
            items = entity_data(s, 'Click0', 'Inventory')
        finally:
            bot.kill()
            bot.wait()
    finally:
        s.stop()
    if not re.search(r'Slot: 9b', items) or 'minecraft:stone' not in items:
        raise Failed(f'a click on slot 63 of a chest threw the stone in inventory slot 9: {items.strip()}')
    print('a click past the chest menu (slot 63) is ignored: the stone in slot 9 stays')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-container-click-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-container-click-e2e-') as scratch:
        try:
            check(bins, Path(scratch), out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_container_click_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
