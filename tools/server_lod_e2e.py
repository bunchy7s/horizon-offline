#!/usr/bin/env python3
"""Far terrain on a dedicated server (`horizon-lod-distance`, crates/horizon-client/src/lod_net.rs), end to end.

    python3 tools/server_lod_e2e.py --bin-dir target/fast

A real Horizon client (horizon-ctl, offscreen) with LOD Distance 32 joins a dedicated server:
- `horizon-lod-distance=16`: the server samples the client's far terrain (`status`'s lod.source is "server"), no
  farther than 16 chunks (effective_distance 16), and far tiles are drawn (active_tiles > 0);
- no `horizon-lod-distance` (off by default): the server offers none, so the client draws only the chunks it was
  sent (source "server-terrain", `server_terrain.rs`), nothing the server sampled.

Each server and game runs in fresh folders with fresh HORIZON_HOME and XDG folders (lab.game_env), on a free
port. Build first:
    cargo build --profile fast -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl
"""
import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from lan_e2e import Failed, Game  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
}
CLIENT_LOD = 32


def server(bins, folder, out, name, extra):
    folder.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **extra, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    s = Server(command, folder, out / f'{name}.log')
    s.wait_for(DONE, 300)
    return s, port


def far_terrain(bins, out, port, name, wanted, seconds):
    """Join with LOD Distance CLIENT_LOD; the state's lod part once `wanted` holds of it (or after `seconds`)."""
    game = Game(name, bins / f'horizon-ctl{EXE}', lab.RESOURCES)
    for folder in (game.work, game.work / 'home'):
        folder.mkdir(parents=True, exist_ok=True)
        (folder / 'horizon-graphics.txt').write_text(f'lodDistance:{CLIENT_LOD}\n')
    try:
        game.pid = game.raw('launch', '--offscreen', '--no-pause')['pid']
        game.raw('connect', f'127.0.0.1:{port}', name)
        game.raw('wait-for', 'loaded', '--timeout', 300)
        deadline = time.time() + seconds
        while True:
            lod = game('status')['lod']
            if wanted(lod) or time.time() > deadline:
                break
            game.wait(20)
        game.shot(out, 'far')
        return lod
    finally:
        (out / f'{name}-game.log').write_text(game.close())
        shutil.rmtree(game.work, ignore_errors=True)


def check(bins, scratch, out):
    s, port = server(bins, scratch / 'on', out, 'on', {'horizon-lod-distance': '16'})
    try:
        lod = far_terrain(bins, out, port, 'Far', lambda l: l['source'] == 'server' and l['active_tiles'] > 20, 90)
    finally:
        s.stop()
    print(f'on: {lod}')
    if lod['distance'] != CLIENT_LOD:
        raise Failed(f'on: the client did not take LOD Distance {CLIENT_LOD}: {lod}')
    if lod['source'] != 'server' or lod['effective_distance'] != 16:
        raise Failed(f'on: far terrain is not the server\'s, 16 chunks: {lod}')
    if lod['active_tiles'] <= 20:
        raise Failed(f'on: no far tiles were drawn: {lod}')
    print('on: the server samples far terrain, capped at its 16 chunks')
    s, port = server(bins, scratch / 'off', out, 'off', {})
    try:
        lod = far_terrain(bins, out, port, 'Near', lambda l: l['source'] != 'none', 15)
    finally:
        s.stop()
    print(f'off: {lod}')
    if lod['source'] != 'server-terrain':
        raise Failed(f'off: expected only the chunks the server sent (server-terrain): {lod}')
    print('off: only the chunks the server sent')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-lod-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-lod-e2e-') as scratch:
        try:
            check(bins, Path(scratch), out)
        except (Failed, RuntimeError, AssertionError) as e:
            sys.exit(f'server_lod_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
