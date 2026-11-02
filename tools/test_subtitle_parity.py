#!/usr/bin/env python3
"""Compare Horizon's subtitle overlay with the vanilla 26.3 client's SubtitleOverlay.

The Java oracle (VanillaSubtitleOracle) runs vanilla's own SubtitleOverlay with a
stand-in client: sounds the sound engine reports to its listeners (subtitles from
sounds.json, unknown keys and sounds without one; positions; ranges, infinite for
relative and unattenuated sounds; looping) at controlled times of Util.getMillis,
and frames that render it for a listener transform (position, forward and up
vectors), the subtitle option turned on or off (it starts and stops listening as
it renders), the notification display time, the text background opacity and its
"chat only" setting, and a GUI size. Each frame records the fills and texts in
order with the pose: which subtitles show, their order, merging of repeats,
expiry, the fade by age (looping sounds stay bright longer), the direction
arrows and the box width. `examples/probe/subtitle_fixture` replays the cases through
`horizon_client::subtitles`; the frames must agree exactly.

  test_subtitle_parity.py [--count N] [--detail N]
"""
import argparse
import json
import math
import os
import random
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'


def subtitle_keys(resources):
    sounds = json.loads((resources / 'assets/minecraft/sounds.json').read_text())
    keys = sorted({v['subtitle'] for v in sounds.values() if isinstance(v, dict) and 'subtitle' in v})
    return keys


def unit(rng):
    while True:
        v = [rng.uniform(-1, 1) for _ in range(3)]
        n = math.sqrt(sum(x * x for x in v))
        if n > 0.1:
            return [x / n for x in v]


def listener(rng, pos):
    yaw, pitch = rng.uniform(-180, 180), rng.uniform(-90, 90)
    y, p = math.radians(yaw), math.radians(pitch)
    forward = [-math.sin(y) * math.cos(p), -math.sin(p), math.cos(y) * math.cos(p)]
    up = [-math.sin(y) * -math.sin(p), math.cos(p), math.cos(y) * -math.sin(p)]
    if rng.random() < 0.2:
        forward, up = rng.choice([([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]), ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                                  ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0])])
    return {'pos': pos, 'forward': forward, 'up': up}


def cases(count, keys):
    rng = random.Random(0x5355425449544c45)
    out = []
    for n in range(count):
        pool = rng.sample(keys, rng.choice([1, 3, 8, 20])) + ['subtitles.horizon.unknown']
        t = rng.randint(0, 10_000_000)
        pos = [rng.uniform(-100, 100), rng.uniform(-60, 300), rng.uniform(-100, 100)]
        here = listener(rng, pos)
        mult = rng.choice([0.5, 1.0, 1.0, 1.5, 2.0, 3.5, 10.0])
        show = rng.random() < 0.9
        opacity, chat_only = rng.choice([0.5, 0.0, 1.0, 0.25]), rng.random() < 0.6
        size = rng.choice([(427, 240), (320, 240), (480, 270), (961, 501)])
        ops = [{'op': 'time', 'ms': t, 'sub': rng.randint(0, 999999)}]
        locations = [[pos[i] + rng.uniform(-30, 30) for i in range(3)] for _ in range(6)]

        def frame():
            nonlocal here
            if rng.random() < 0.3:
                here = listener(rng, [p + rng.uniform(-2, 2) for p in here['pos']])
            return {'op': 'frame', **here, 'show': show, 'mult': mult, 'opacity': opacity, 'chat_only': chat_only,
                    'w': size[0], 'h': size[1]}
        ops.append(frame())
        for _ in range(rng.choice([5, 20, 60])):
            r = rng.random()
            if r < 0.45:
                at = rng.choice(locations) if rng.random() < 0.6 else [here['pos'][i] + rng.uniform(-20, 20) for i in range(3)]
                if rng.random() < 0.05:
                    at = list(here['pos'])
                key = rng.choice(pool) if rng.random() < 0.9 else None
                rng_range = rng.choice([16.0, 16.0, 8.0, 48.0, 2.5, 100.0, None])
                ops.append({'op': 'play', 'key': key, 'at': at, 'range': rng_range, 'looping': rng.random() < 0.15})
            elif r < 0.75:
                t += rng.choice([0, 1, 16, 50, 499, 500, 501, 1000, 2999, 3000, 3001, rng.randint(0, 8000)])
                ops.append({'op': 'time', 'ms': t, 'sub': rng.randint(0, 999999)})
            else:
                if rng.random() < 0.08:
                    show = not show
                if rng.random() < 0.05:
                    mult = rng.choice([0.5, 1.0, 2.0, 10.0])
                ops.append(frame())
        ops.append(frame())
        out.append({'case': n, 'ops': ops})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1500)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    resources = lab.RESOURCES
    build = lab.LOCAL / 'subtitle-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')

    chosen = cases(args.count, subtitle_keys(resources))
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityFont', 'VanillaSubtitleOracle'))], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaSubtitleOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary, 'subtitle_fixture',resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text(encoding='utf-8').splitlines()]
    assert len(vanilla) == len(rust) == len(chosen)
    errors = [v for v in vanilla if 'error' in v]
    bad, frames, drawn = [], 0, 0
    for c, v, r in zip(chosen, vanilla, rust):
        if 'error' in v:
            continue
        frames += len(v['frames'])
        drawn += sum(1 for f in v['frames'] if f)
        for i, (a, b) in enumerate(zip(v['frames'], r['frames'])):
            if a != b:
                bad.append((c, i, a, b))
                break
        else:
            if len(v['frames']) != len(r['frames']):
                bad.append((c, len(r['frames']), v['frames'][len(r['frames']):][:1], []))
    for v in errors[:3]:
        print('ORACLE ERROR', v['case'], v['error'])
    for c, i, a, b in bad[:args.detail]:
        first = next((j for j, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        print(f'MISMATCH case {c["case"]} frame {i} at draw {first} ({len(a)} vanilla, {len(b)} Horizon draws)')
        print('   vanilla', json.dumps(a[first:first + 2], ensure_ascii=False))
        print('   horizon', json.dumps(b[first:first + 2], ensure_ascii=False))
    print(f'{len(chosen) - len(bad) - len(errors)}/{len(chosen)} subtitle cases match '
          f'({frames} frames, {drawn} with subtitles); {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('subtitle parity ok')


if __name__ == '__main__':
    main()
