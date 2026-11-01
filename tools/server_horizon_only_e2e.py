#!/usr/bin/env python3
"""`horizon-clients-only`, Horizon's own server.properties key, end to end on a dedicated server.

    python3 tools/server_horizon_only_e2e.py --bin-dir target/fast

- off (the default): a client that sends no brand joins, as on vanilla.
- on (`horizon-clients-only=true`): a client with no brand, or with vanilla's "vanilla", is disconnected during
  configuration with "This server is for the Horizon client: playhorizon.dev", and the console says why; a
  client whose `minecraft:brand` is "horizon" joins. (Any client can send that brand: the gate keeps out
  players who didn't install Horizon, not cheaters.)

horizon-bots stand in for the clients (`--brand`). Each server runs in a fresh folder with fresh HORIZON_HOME
and XDG folders (lab.game_env), on a free port. Build first:
    cargo build --profile fast -p horizon-client --bin horizon-client
    cargo build --profile fast -p horizon-bots
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
REASON = 'This server is for the Horizon client: playhorizon.dev'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'view-distance': '3',
    'simulation-distance': '3',
}


class Failed(Exception):
    pass


def bot(bins, port, out, name, *extra):
    """One bot for a few seconds: its report's entry."""
    report = out / f'{name}.json'
    subprocess.run([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                    '--behaviour', 'idle', '--seconds', '3', '--name', name, '--report', report, *extra],
                   check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=180)
    return json.loads(report.read_text())['per_bot'][0]


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


def joined(entry):
    return entry['joined_after_ms'] > 0 and not entry['error']


def check(bins, scratch, out):
    s, port = server(bins, scratch / 'off', out, 'off', {})
    try:
        plain = bot(bins, port, out, 'Plain')
    finally:
        s.stop()
    if not joined(plain):
        raise Failed(f'off: a client without a brand was refused: {plain["error"]}')
    print('off: a client without a brand joins')
    s, port = server(bins, scratch / 'on', out, 'on', {'horizon-clients-only': 'true'})
    try:
        results = {name: bot(bins, port, out, name, *extra) for name, extra in
                   [('None', ()), ('Vanilla', ('--brand', 'vanilla')), ('Horizon', ('--brand', 'horizon'))]}
        lines = s.since(0)
    finally:
        s.stop()
    for name in ('None', 'Vanilla'):
        entry = results[name]
        if joined(entry) or REASON not in (entry['error'] or ''):
            raise Failed(f'on: {name} was not turned away with the reason: {entry["error"]}')
        if not any(f'Disconnecting {name}0' in l and 'horizon-clients-only' in l for l in lines):
            raise Failed(f'on: the console did not say why {name}0 was turned away')
    if not joined(results['Horizon']):
        raise Failed(f'on: the Horizon client was refused: {results["Horizon"]["error"]}')
    print('on: no brand and "vanilla" are turned away with the reason; "horizon" joins')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-horizon-only-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-horizon-only-e2e-') as scratch:
        try:
            check(bins, Path(scratch), out)
        except (Failed, RuntimeError) as e:
            sys.exit(f'server_horizon_only_e2e failed (logs in {out}): {e}')


if __name__ == '__main__':
    main()
