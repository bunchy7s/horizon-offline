#!/usr/bin/env python3
"""Compare rendered tooltip continuity with vanilla's real slot hit-testing.

Runs an isolated integrated-server client. Samples quarter-pixel positions
between Creative grid slots, grid rows, and hotbar slots, plus Survival and
outside-slot controls. Requires prepared 26.3 resources, built binaries and Pillow.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

from PIL import Image
import lab
import oracle_harness as oracle
import test_rust_parity as base
from bundle_tooltip_e2e import longest_tooltip_border

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, default=ROOT / 'target/fast/horizon-client')
    parser.add_argument('--ctl', type=Path, default=ROOT / 'target/fast/horizon-ctl')
    parser.add_argument('--resources', type=Path, default=ROOT / '.local/horizon-client/26.3')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    output = (args.output or Path(tempfile.mkdtemp(prefix='creative-hover-'))).resolve()
    output.mkdir(parents=True, exist_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    classes, dependencies, _ = oracle.compile_oracle(
        output, 'VanillaSlotHoverOracle', jars, libraries, lock['minecraft']['java_major'])
    homes = {}
    for key in ('HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME',
                'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD', 'HORIZON_LAUNCHER_DESKTOP'):
        homes[key] = output / key
        homes[key].mkdir(exist_ok=True)
    env = lab.game_env(**homes)
    results = []
    # UNIX socket names must fit even when the checkout/output path is long.
    with tempfile.TemporaryDirectory(prefix='hover-', dir='/tmp') as socket_home:
        socket = Path(socket_home) / 'ctl.sock'

        def ctl(*commands, timeout=60):
            r = subprocess.run([str(args.ctl.resolve()), '--socket', str(socket), *map(str, commands)],
                               cwd=output, env=env, capture_output=True, text=True, timeout=timeout)
            data = json.loads(r.stdout)
            if r.returncode or data.get('error'):
                raise RuntimeError(f'{commands}: {data}; {r.stderr}')
            return data

        try:
            ctl('launch', '--game', args.game.resolve(), '--world', output / 'world', '--creative',
                '--offscreen', '--no-pause', '--wait', '--width', 960, '--height', 720,
                '--resources', args.resources.resolve(), timeout=500)
            ctl('command', 'gamerule spawn_mobs false')
            ctl('command', 'item replace entity @s hotbar.0 with minecraft:diamond')
            ctl('command', 'item replace entity @s hotbar.1 with minecraft:emerald')
            ctl('wait', 5)
            for mode in ('creative', 'survival'):
                if mode == 'survival':
                    ctl('command', 'gamemode survival')
                ctl('key', 'e', 'tap')
                ctl('wait', 2)
                ui = ctl('ui')
                slots = {int(w['id'][5:]): w for w in ui['widgets']
                         if w['kind'] == 'slot' and w['id'].startswith('slot-')}
                assert slots[0]['stack'] and slots[1]['stack'], 'hotbar updates did not arrive'
                anchors = [('hotbar', 0, False)]
                if mode == 'creative':
                    assert slots[900]['stack'] and slots[901]['stack'] and slots[909]['stack']
                    anchors += [('grid', 900, False), ('rows', 900, True)]
                points = []
                # Centers prove that items really have tooltips; exact and
                # fractional boundary samples expose the two-pixel gap.
                offsets = [8, 15.75, 16, 16.25, 16.5, 16.75, 17, 17.25, 17.5, 17.75, 18, 26]
                for label, slot, vertical in anchors:
                    x, y, _, _ = slots[slot]['bounds']
                    for offset in offsets:
                        points.append((f'{label}-{offset}', x + (8 if vertical else offset),
                                       y + (offset if vertical else 8)))
                points.append(('outside', 1, 1))
                case = {'slots': [[i, *w['bounds'][:2]] for i, w in slots.items()],
                        'points': [[x, y] for _, x, y in points]}
                source, native = output / f'{mode}.jsonl', output / f'{mode}-native.txt'
                source.write_text(json.dumps(case) + '\n')
                oracle.java_run(output, classes, dependencies, 'VanillaSlotHoverOracle',
                                [source, native], mode + '-native')
                expected = list(map(int, native.read_text().splitlines()))
                assert len(expected) == len(points)
                for (label, x, y), slot in zip(points, expected):
                    ctl('scroll-at', x, y, 0)
                    ctl('wait', 2)
                    screenshot = output / f'{mode}-{label}.png'
                    ctl('screenshot', screenshot)
                    with Image.open(screenshot) as image:
                        scale = image.width / ui['gui_size'][0]
                        border = longest_tooltip_border(image)
                    visible = border >= 20 * scale
                    wanted = slot >= 0 and bool(slots[slot]['stack'])
                    results.append(dict(mode=mode, point=label, native_slot=slot, expected=wanted,
                                        visible=visible, border_pixels=border, screenshot=str(screenshot)))
                ctl('key', 'esc', 'tap')
        finally:
            try:
                ctl('quit')
            except (RuntimeError, subprocess.TimeoutExpired, json.JSONDecodeError):
                pass
    failures = [r for r in results if r['visible'] != r['expected']]
    (output / 'results.json').write_text(json.dumps(dict(results=results, failures=failures), indent=2))
    print(f'{len(results)-len(failures)}/{len(results)} rendered tooltip samples match vanilla: {output}')
    assert not failures, failures


if __name__ == '__main__':
    main()
