#!/usr/bin/env python3
"""Compare the in-game HUD with the vanilla 26.3 client's Hud.

The Java oracle (VanillaHudOracle) runs vanilla's own code on a stand-in
client whose player answers from the case, and records every
GuiGraphicsExtractor blit (sprite, texture, position, size, region, colour,
pose), fill, text call (string, style runs, position, colour, shadow, pose)
and hotbar item (slot, position, seed, pose). `examples/probe/hud_fixture` lays
out the same state with `horizon_client::hud_layout`, `hud_overlays` and
`chat`; the ordered call lists (and any state read back) must agree.

Case kinds (`tools/hud_parity_cases.py`), drawn round-robin:
  decorations  extractCrosshair, extractItemHotbar, extractPlayerHealth,
               extractVehicleHealth, ExperienceBar and the level
  effects      extractEffects (MobEffectInstance order, blinking icons)
  title        setTitle/setTimes/setOverlayMessage and Hud.tick, then
               extractOverlayMessage and extractTitle (fades, backdrops)
  selected     Hud.tick's tool highlight, then extractSelectedItemName
  boss         BossHealthOverlay
  jump         JumpableVehicleBar
  sidebar      displayScoreboardSidebar (teams, number formats)
  decor        GuiGraphicsExtractor.itemDecorations (count, bar, cooldown)
  overlays     extractCameraOverlays (vignette, spyglass and scopeScale,
               pumpkin, powder snow, portal, nausea) and extractSleepOverlay
  chat         ChatComponent: wrapping (StringSplitter, indents), fades,
               tags, scale, width, height and spacing options, focused
               (the chat screen open: foreground, scrolled, scroll bar)

  test_hud_parity.py [--count N] [--detail N] [--kind K]
"""
import argparse
import json
import os
import random
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
    from .hud_parity_cases import KINDS
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking
    from hud_parity_cases import KINDS

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'


def cases(count, kinds):
    rng = random.Random(0x485544)
    out = []
    for n in range(count):
        kind = kinds[n % len(kinds)]
        case = KINDS[kind](rng, n)
        case['kind'] = kind
        out.append(case)
    return out


def close(p, q):
    if (isinstance(p, float) or isinstance(q, float)) and not isinstance(p, bool) and not isinstance(q, bool):
        return abs(p - q) <= 1e-4 * (1 + abs(p))
    return p == q


IDENTITY = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]


def normal(d):
    d = dict(d)
    d.setdefault('matrix', IDENTITY)
    if d['type'] == 'sprite':
        d.setdefault('region', None)
        d.setdefault('pipeline', 'pipeline/gui_textured')
    if d['type'] == 'text':
        d.setdefault('runs', None)
    return d


def same(x, y):
    if x.keys() != y.keys():
        return False
    for k in x:
        p, q = x[k], y[k]
        if isinstance(p, list) and isinstance(q, list):
            if len(p) != len(q) or not all(close(a, b) for a, b in zip(p, q)):
                return False
        elif not close(p, q):
            return False
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=2200)
    parser.add_argument('--detail', type=int, default=3)
    parser.add_argument('--kind', action='append', choices=sorted(KINDS), help='only these kinds (repeatable)')
    args = parser.parse_args()
    kinds = args.kind or list(KINDS)
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-hud-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
    chosen = cases(args.count, kinds)
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaHudOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaHudOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'hud_fixture', resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust)
    errors, bad = [], []
    totals = {}
    for v, r in zip(vanilla, rust):
        kind = v['case']['kind']
        totals.setdefault(kind, [0, 0])[1] += 1
        if 'error' in v:
            errors.append(v['error']); continue
        a = [normal(d) for d in v['draws']]
        b = [normal(d) for d in r['draws']]
        # State the case reads back besides the draws.
        for key in v.keys() - {'case', 'draws'}:
            if key not in r or not close(v[key], r[key]):
                a = a + [{'type': 'state', key: v[key]}]
                b = b + [{'type': 'state', key: r.get(key)}]
        if len(a) != len(b) or not all(same(x, y) for x, y in zip(a, b)):
            first = next((i for i, (x, y) in enumerate(zip(a, b)) if not same(x, y)), min(len(a), len(b)))
            bad.append((v['case'], first, a[first:first + 2], b[first:first + 2], len(a), len(b)))
        else:
            totals[kind][0] += 1
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for case, i, a, b, la, lb in bad[:args.detail]:
        print('MISMATCH', json.dumps(case))
        print(f'   at call {i} ({la} vanilla, {lb} Horizon calls)\n   vanilla {a}\n   horizon {b}')
    for kind, (ok, n) in sorted(totals.items()):
        print(f'  {kind:12} {ok}/{n}')
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} HUD states match; {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('hud parity ok')


if __name__ == '__main__':
    main()
