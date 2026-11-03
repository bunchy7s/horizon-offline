#!/usr/bin/env python3
"""Exercise first-tick projectile events through the headless game fixture.

Counts actual dispatches, rather than selected vibrations, so duplicate launch
notifications cannot hide behind the warden listener's selection rules.
"""
import argparse
import json
import math
import os
from pathlib import Path
import tempfile

import integration_env
import lab


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    scenarios = []
    for name, flag in [('summoned', ''), ('explicit_false', ',HasBeenShot:0b'),
                       ('already_shot', ',HasBeenShot:1b'), ('player_launch', None),
                       ('mob_launch', None)]:
        lines = [f'scenario {name} ticks=6 traceevents', 'origin 0 0',
                 'player 4.5 200 10.5 survival',
                 'fill 0 199 0 47 199 31 minecraft:stone',
                 'fill 29 199 9 31 203 11 minecraft:glass',
                 'fill 30 200 10 30 202 10 minecraft:air']
        if name == 'mob_launch':
            lines += ['mob minecraft:snow_golem 4.5 200 10.5 0 123 noai persistent']
        lines += ['mob minecraft:warden 30.5 200 10.5 0 123 persistent']
        action = ('throw_snowball' if name == 'player_launch' else 'mob_snowball') if flag is None else (
            'summon_snowball '
            '{Owner:$owner,Motion:[0.0d,-0.6d,0.0d]' + flag + '}')
        lines += ['at 3 ' + action, 'end']
        scenarios += lines
    inputs, output = out / 'scenarios.txt', out / 'horizon.txt'
    inputs.write_text('\n'.join(scenarios) + '\n')
    with tempfile.TemporaryDirectory(prefix='hz-shoot-home-') as home, (out / 'game.log').open('w') as log:
        env = dict(os.environ, HOME=home, HORIZON_SCULK_FIXTURE=f'{inputs},{output}')
        result = integration_env.run_game([args.bin.resolve(), '--resources', lab.LOCAL / 'horizon-client/26.3'],
                                          env=env, stdout=log, stderr=log, timeout=180)
        if result.returncode:
            raise RuntimeError(f'game failed ({result.returncode}); see {out / "game.log"}')
    cases = {}
    for line in output.read_text().splitlines():
        if line.startswith('== '):
            current = cases.setdefault(line[3:], [])
        else:
            tick, kind, *rest = line.split(' ', 2)
            current.append((int(tick), kind, rest[0] if rest else ''))
    failures = []
    for name, rows in cases.items():
        failures.extend(f'{name}: fixture failed: {data}' for _, kind, data in rows if kind == 'failed')
        shots = [(t, data) for t, kind, data in rows if kind == 'game' and data.endswith('minecraft:projectile_shoot')]
        expected = [] if name == 'already_shot' else [3]
        if [t for t, _ in shots] != expected:
            failures.append(f'{name}: shot ticks {[t for t, _ in shots]}, expected {expected}')
        if name in ('player_launch', 'mob_launch') and shots:
            position = next(json.loads(data)['pos'] for t, kind, data in rows if t == 3 and kind == 'projectile')
            at = ','.join(str(math.floor(v)) for v in position)
            if shots[0][1].split()[0] != at:
                failures.append(f'{name}: shoot origin is not the projectile position after moving')
        if name in ('summoned', 'explicit_false', 'already_shot'):
            selected = [json.loads(data)['selector'] for t, kind, data in rows if t == 3 and kind == 'listener']
            event = selected[0]['event'] if selected and selected[0] else None
            expected_event = None if name == 'already_shot' else 'minecraft:projectile_shoot'
            if event == 'minecraft:projectile_shoot':
                info = selected[0]
                if any(abs(a - b) > 1e-9 for a, b in zip(info['pos'], [30.5, 204.37629999399186, 10.5])):
                    failures.append(f'{name}: shoot position differs from the Java reference')
                if abs(info['distance'] - 1.9112998) > 1e-6:
                    failures.append(f'{name}: listener distance differs from the Java reference')
            if event != expected_event:
                failures.append(f'{name}: first-tick selector {event}, expected {expected_event}')
            tendrils = [t for t, kind, data in rows if kind == 'entity_event' and data == '61']
            expected_tick = 5 if name == 'already_shot' else 4
            if not tendrils or tendrils[0] != expected_tick:
                failures.append(f'{name}: first event 61 at {tendrils[:1]}, expected {expected_tick}')
    if len(cases) != 5:
        failures.append(f'expected five scenarios, got {len(cases)}')
    (out / 'results.json').write_text(json.dumps({'scenarios': len(cases), 'failures': failures}, indent=2) + '\n')
    print('\n'.join(failures) if failures else f'PASS: {len(cases)} projectile scenarios')
    return bool(failures)


if __name__ == '__main__':
    raise SystemExit(main())
