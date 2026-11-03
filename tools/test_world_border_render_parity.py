#!/usr/bin/env python3
"""Compare the world border as the client shows it with vanilla 26.3.

The Java oracle (VanillaWorldBorderRenderOracle) drives a vanilla WorldBorder
through each case's size changes (static, growing, shrinking), centre and
warning changes and ticks, and for every frame runs WorldBorderRenderer's
extract, prepare (the cached wall vertex buffer), prepareDynamicTransforms
(colour, model offset, scrolling texture) and draw (which walls), and
Hud.extractVignette (the red warning vignette colour). `examples/
world_border_render_fixture` replays the same cases with `horizon_client::
worldborder`, `border_render` and `hud_overlays::vignette`; every value must agree
bit for bit.

  test_world_border_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'


def cases(count):
    rng = random.Random(0x424F52)
    out = []
    for n in range(count):
        limit = rng.choice([29999984] * 6 + [1000, 50, 300000])
        size = rng.choice([rng.uniform(1, 40), rng.uniform(40, 600), rng.uniform(600, 20000), 59999968.0, 1.0, 16.0, 200.0])
        size = round(size, rng.choice([0, 1, 3, 9]))
        far = rng.random() < 0.1
        center = [rng.choice([0.0, 0.5, round(rng.uniform(-500, 500), 2), round(rng.uniform(-3e7, 3e7), 1) if far else 7.25])
                  for _ in range(2)]
        case = {'absoluteMaxSize': limit, 'centerX': center[0], 'centerZ': center[1],
                'warningBlocks': rng.choice([5, 5, 0, 1, 20, 100]), 'warningTime': rng.choice([300, 300, 15, 0, 1200]),
                'size': size, 'ops': []}
        current = size
        for _ in range(rng.randint(1, 8)):
            kind = rng.random()
            if kind < 0.25:
                to = min(59999968.0, round(max(1.0, current * rng.choice([0.25, 0.5, 2.0, 4.0]) + rng.uniform(-5, 5)), 2))
                to = rng.choice([to, current]) if rng.random() < 0.1 else to
                case['ops'].append({'op': 'lerp', 'from': current, 'to': to, 'ticks': rng.choice([1, 2, 20, 100, 1200, 72000])})
                current = to
            elif kind < 0.33:
                current = round(rng.uniform(1, 300), 2)
                case['ops'].append({'op': 'size', 'size': current})
            elif kind < 0.38:
                case['ops'].append({'op': 'center', 'x': round(rng.uniform(-100, 100), 2), 'z': round(rng.uniform(-100, 100), 2)})
            elif kind < 0.42:
                case['ops'].append({'op': 'warning', 'blocks': rng.choice([0, 5, 30]), 'time': rng.choice([0, 15, 300, 6000])})
            elif kind < 0.45:
                case['ops'].append({'op': 'invalidate'})
            elif kind < 0.6:
                case['ops'].append({'op': 'tick', 'n': rng.choice([1, 1, 2, 5, 19, 50, 600])})
            for _ in range(rng.randint(1, 4)):
                rd = 16 * rng.choice([2, 4, 8, 12, 16, 32])
                half = min(current / 2, limit)
                cx, cz = case['centerX'], case['centerZ']
                if rng.random() < 0.8:
                    side = rng.randrange(4)
                    edge = rng.uniform(-1.5, 1.5) * rd
                    along = rng.uniform(-half - rd, half + rd)
                    x, z = [(cx + half + edge, cz + along), (cx - half - edge, cz + along),
                            (cx + along, cz + half + edge), (cx + along, cz - half - edge)][side]
                else:
                    x, z = cx + rng.uniform(-3 * rd, 3 * rd), cz + rng.uniform(-3 * rd, 3 * rd)
                y = rng.choice([64.0, 70.62, -60.3, 319.9, round(rng.uniform(-64, 320), 3)])
                case['ops'].append({'op': 'frame', 'partial': rng.choice([0.0, 1.0, round(rng.random(), 4)]),
                                    'camera': [round(x, rng.choice([1, 3, 7])), y, round(z, rng.choice([1, 3, 7]))],
                                    'renderDistance': rd, 'depthFar': float(max(rd * 4, rng.choice([128, 256, 512]))),
                                    'millis': rng.randint(0, 10 ** 12), 'vignette': rng.choice([0.0, 1.0, round(rng.random(), 4)])})
        out.append(case)
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1500)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-world-border-render-parity'; build.mkdir(parents=True, exist_ok=True)
    chosen = cases(args.count)
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         PARITY / 'VanillaWorldBorderRenderOracle.java'], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaWorldBorderRenderOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'world_border_render_fixture', inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust)
    errors, bad, frames, drawn = [], [], 0, 0
    for v, r in zip(vanilla, rust):
        if 'error' in v:
            errors.append(v['error']); continue
        frames += len(v['frames'])
        drawn += sum(f['drawn'] for f in v['frames'])
        for i, (a, b) in enumerate(zip(v['frames'], r['frames'])):
            if a != b:
                keys = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
                bad.append((v['case'], i, {k: (a.get(k), b.get(k)) for k in keys}))
                break
        else:
            if len(v['frames']) != len(r['frames']):
                bad.append((v['case'], len(r['frames']), 'frame count'))
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for case, i, diff in bad[:args.detail]:
        print('MISMATCH', json.dumps(case))
        print(f'   at frame {i}: {json.dumps(diff)[:1500]}')
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} border cases match '
          f'({frames} frames, {drawn} with the wall drawn); {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('world border parity ok')


if __name__ == '__main__':
    main()
