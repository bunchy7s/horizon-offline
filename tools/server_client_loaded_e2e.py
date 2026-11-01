#!/usr/bin/env python3
"""ServerGamePacketListenerImpl.hasClientLoaded on a dedicated server, end to end.

    python3 tools/server_client_loaded_e2e.py --bin-dir target/fast

Until the client says the level around it is loaded (ServerboundPlayerLoadedPacket), or 60 ticks
pass, vanilla ignores the player's moves and actions. A bot that never sends that packet climbs from
spawn (0.5 blocks a tick): after 1.5 s the server still has it where it spawned; after 6 s its moves
are taken. A bot that sends it moves at once.

horizon-bots stand in for the clients (`--no-loaded`). The server runs in a fresh folder with fresh
HORIZON_HOME and XDG folders (lab.game_env), on a free port. Build first:
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


def bot(bins, port, name, *extra):
    return subprocess.Popen(
        [bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1', '--behaviour', 'idle',
         '--seconds', '10', '--name', name, *extra],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def height(s, player):
    """The player's y as the server has it (`data get entity <player> Pos[1]`)."""
    mark = s.count()
    s.console(f'data get entity {player} Pos[1]')
    deadline = time.time() + 30
    while time.time() < deadline:
        for line in s.since(mark):
            m = re.search(rf'{player} has the following entity data: (-?[\d.]+)d', line)
            if m:
                return float(m[1])
            if 'No entity was found' in line:
                raise Failed(f'{player} is not on the server')
        time.sleep(0.1)
    raise Failed(f'no answer to `data get entity {player} Pos[1]`')


def watch(s, port, bins, name, *extra):
    """The bot's height on the server when it joins, 1.5 s later and 6 s later."""
    b = bot(bins, port, name, *extra)
    try:
        s.wait_for(rf'{name}0 joined the game', 60)
        start = height(s, f'{name}0')
        time.sleep(1.5 - 0.1)
        early = height(s, f'{name}0')
        time.sleep(4.5)
        late = height(s, f'{name}0')
    finally:
        b.kill()
        b.wait()
    return start, early, late


def check(bins, scratch, out):
    s, port = server(bins, scratch / 'server', out, 'server', {})
    try:
        late = watch(s, port, bins, 'Late', '--no-loaded')
        loaded = watch(s, port, bins, 'Loaded')
    finally:
        s.stop()
    start, early, after = late
    if early != start:
        raise Failed(f'a client that has not loaded moved from y {start} to {early} within 1.5 s')
    if after <= start + 1.0:
        raise Failed(f'after 6 s the server still ignores its moves (y {start} → {after})')
    print(f'not loaded: still at y {early} after 1.5 s, moved to y {after} after 6 s (the 60-tick timeout)')
    start, early, _ = loaded
    if early <= start + 1.0:
        raise Failed(f'a loaded client did not move within 1.5 s (y {start} → {early})')
    print(f'loaded: moved from y {start} to {early} within 1.5 s')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-client-loaded-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-client-loaded-e2e-') as scratch:
        try:
            check(bins, Path(scratch), out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_client_loaded_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
