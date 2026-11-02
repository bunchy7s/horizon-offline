#!/usr/bin/env python3
"""FireworkRocketEntity event 17 renders the configured burst before tracked removal.

Exercise both hosting paths and subsequent trail/fade frames through real clients.
"""
import json
import subprocess

import lab
import lan_e2e as lan


def main():
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib',
                    '--bin', 'horizon-client', '--bin', 'horizon-ctl'], cwd=lab.ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=lab.ROOT, text=True))
    ctl = lab.ROOT / metadata['target_directory'] / 'fast' / 'horizon-ctl'
    # CI supplies the display; each game still gets fresh scratch homes.
    lan.game_env = lambda _screen, home: lab.game_env(
        HORIZON_HOME=home, HORIZON_TELEMETRY_URL='off')
    out = lab.LOCAL / 'remote-fireworks-parity'
    out.mkdir(parents=True, exist_ok=True)
    report = lan.run(ctl, out, lab.RESOURCES.resolve(), ['fireworks'])
    lab.write_json(out / 'report.json', report)
    assert all(r['pass'] for r in report.values()), report


if __name__ == '__main__':
    main()
