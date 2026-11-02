#!/usr/bin/env python3
"""Compare block-entity rendering with the vanilla 26.3 client renderers.

The Java oracle (VanillaBlockEntityRenderOracle) builds each case's vanilla
block entity, sets its animation fields, runs the real BlockEntityRenderer's
extractRenderState and submit, and captures what it submits: model vertices
after setupAnim and the pose stack (sprite-local UVs, tint, normal, light),
text calls and items. `examples/probe/block_entity_render_fixture` replays the case
through Horizon's BlockEntityRenderer. Quads are matched by texture and
position; colours compare vanilla's tint times the entity shader's
`minecraft_mix_light` for the vanilla normal with Horizon's baked colour.

  test_block_entity_render_parity.py [--filter SUBSTRING] [--detail N] [--tolerance EPS]
"""
import argparse
import json
import math
import os
from pathlib import Path
import random
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
DYES = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan', 'purple',
        'blue', 'brown', 'green', 'red', 'black']
HORIZONTAL = ['north', 'south', 'west', 'east']
ALL = HORIZONTAL + ['up', 'down']
NO_OVERLAY = 10 << 16


def cases():
    rng = random.Random(0x42454e44)
    out = []

    def add(block, **extra):
        case = {'block': block, 'pos': [rng.randint(-40, 40), rng.randint(-20, 90), rng.randint(-40, 40)],
                'light': [rng.randint(0, 15), rng.randint(0, 15)], 'partial': round(rng.random(), 4),
                'camera': [0.5, 70.0, 0.5]}
        case.update(extra)
        out.append(case)

    for block in ['chest', 'trapped_chest', 'copper_chest', 'exposed_copper_chest', 'weathered_copper_chest',
                  'oxidized_copper_chest', 'waxed_copper_chest', 'waxed_exposed_copper_chest',
                  'waxed_weathered_copper_chest', 'waxed_oxidized_copper_chest']:
        for facing in HORIZONTAL:
            for kind in ['single', 'left', 'right']:
                add(f'minecraft:{block}[facing={facing},type={kind}]', open=rng.choice([0.0, 0.25, 0.7, 1.0]))
    for facing in HORIZONTAL:
        add(f'minecraft:ender_chest[facing={facing}]', open=rng.choice([0.0, 0.5, 1.0]))
    texts = [['Hello', '', 'World', ''], ['A much longer line that cannot fit', 'mid', 'iii', 'WWWWWWWWWWWWWWW'],
             ['', '', '', ''], ['x', 'Sign text', '#1 (test)', '!']]
    for wood in ['oak', 'spruce', 'cherry', 'crimson', 'bamboo', 'pale_oak']:
        signs = [f'minecraft:{wood}_sign[rotation={r}]' for r in (0, 3, 8, 13)] + \
                [f'minecraft:{wood}_wall_sign[facing={f}]' for f in HORIZONTAL] + \
                [f'minecraft:{wood}_hanging_sign[rotation={r},attached={a}]' for r, a in ((0, 'false'), (6, 'true'), (11, 'false'))] + \
                [f'minecraft:{wood}_wall_hanging_sign[facing={f}]' for f in HORIZONTAL]
        for block in signs:
            side = lambda: {'lines': rng.choice(texts), 'color': rng.choice(DYES), 'glowing': rng.random() < 0.4}
            add(block, sign={'front': side(), 'back': side(), 'near': rng.random() < 0.5})
    patterns = ['minecraft:stripe_bottom', 'minecraft:creeper', 'minecraft:gradient', 'minecraft:flower',
                'minecraft:border', 'minecraft:skull', 'minecraft:globe', 'minecraft:piglin']
    for color in ['white', 'red', 'light_gray', 'black']:
        for r in (0, 5, 12):
            add(f'minecraft:{color}_banner[rotation={r}]',
                patterns=[[rng.choice(patterns), rng.choice(DYES)] for _ in range(rng.randint(0, 3))])
        for f in HORIZONTAL:
            add(f'minecraft:{color}_wall_banner[facing={f}]',
                patterns=[[rng.choice(patterns), rng.choice(DYES)] for _ in range(rng.randint(0, 2))])
    add('minecraft:blue_banner[rotation=2]', patterns=[[rng.choice(patterns), rng.choice(DYES)] for _ in range(17)])
    # The loom's result preview (GuiBannerResultRenderer in a 20x40 picture in
    # picture, lit with Lighting.Entry.ITEMS_FLAT): up to the loom's six layers, and 17.
    for n in [0, 1, 2, 3, 6, 6, 17]:
        out.append({'block': 'loom_result', 'loom': {'base': rng.choice(DYES),
                    'patterns': [[rng.choice(patterns), rng.choice(DYES)] for _ in range(n)]}})
    for skull in ['skeleton_skull', 'wither_skeleton_skull', 'zombie_head', 'creeper_head', 'dragon_head',
                  'piglin_head', 'player_head']:
        wall = skull.replace('_skull', '_wall_skull').replace('_head', '_wall_head')
        for r in (0, 4, 9):
            add(f'minecraft:{skull}[rotation={r}]', skullAnimation=[rng.random() < 0.5, rng.randint(0, 80)])
        for f in HORIZONTAL:
            add(f'minecraft:{wall}[facing={f}]', skullAnimation=[rng.random() < 0.5, rng.randint(0, 80)])
    for color in [None, 'white', 'light_gray', 'purple', 'black']:
        name = 'shulker_box' if color is None else f'{color}_shulker_box'
        for f in ALL:
            p = rng.choice([0.0, 0.0, 0.3, 1.0])
            add(f'minecraft:{name}[facing={f}]', progress=p, progressOld=max(0.0, p - 0.1))
    for attach in ['floor', 'ceiling', 'single_wall', 'double_wall']:
        for f in HORIZONTAL:
            shake = rng.choice([None] + HORIZONTAL)
            extra = {'bellTicks': rng.randint(0, 50)}
            if shake:
                extra['shake'] = shake
            add(f'minecraft:bell[attachment={attach},facing={f}]', **extra)
    sherds = ['', 'minecraft:brick', 'minecraft:angler_pottery_sherd', 'minecraft:skull_pottery_sherd',
              'minecraft:heart_pottery_sherd', 'minecraft:flow_pottery_sherd']
    for f in HORIZONTAL:
        for wobble in [None, ['POSITIVE', 0.3], ['NEGATIVE', 0.6], ['POSITIVE', 1.2]]:
            extra = {'sherds': [rng.choice(sherds) for _ in range(4)]}
            if wobble:
                extra['wobble'] = wobble
            add(f'minecraft:decorated_pot[facing={f}]', **extra)
    for active in (False, True):
        for _ in range(4):
            add('minecraft:conduit', conduit={'tickCount': rng.randint(0, 400), 'active': active,
                                              'hunting': rng.random() < 0.5, 'activeRotation': rng.uniform(0, 300)},
                yaw=round(rng.uniform(-180, 180), 2), pitch=round(rng.uniform(-90, 90), 2))
    for _ in range(6):
        o = rng.random()
        add('minecraft:enchanting_table', book={'time': rng.randint(0, 2000), 'flip': rng.uniform(-3, 3),
                                               'oFlip': rng.uniform(-3, 3), 'open': o, 'oOpen': max(0.0, o - 0.1),
                                               'rot': rng.uniform(-3.1, 3.1), 'oRot': rng.uniform(-3.1, 3.1)})
    for f in HORIZONTAL:
        add(f'minecraft:lectern[facing={f},has_book=true]')
        add(f'minecraft:lectern[facing={f},has_book=false]')
    for prefix in ['', 'exposed_', 'weathered_', 'oxidized_', 'waxed_', 'waxed_exposed_',
                   'waxed_weathered_', 'waxed_oxidized_']:
        for pose in ['standing', 'running', 'sitting', 'star']:
            add(f'minecraft:{prefix}copper_golem_statue[facing={rng.choice(HORIZONTAL)},copper_golem_pose={pose}]')
    for _ in range(8):
        add('minecraft:beacon', gameTime=rng.randint(0, 10 ** 6),
            camera=[rng.uniform(-300, 300), 70.0, rng.uniform(-300, 300)],
            beam=[[0xFF000000 | rng.randint(0, 0xFFFFFF), rng.randint(1, 30)] for _ in range(rng.randint(1, 3))])
    for block in ['end_portal', 'end_gateway', 'campfire[facing=north]', 'spawner',
                  'trial_spawner', 'vault', 'suspicious_sand', 'oak_shelf[facing=east]']:
        add('minecraft:' + block)
    # End gateways: the beam while spawning (age < 200) or cooling down, in End and
    # Overworld build heights, and the faces Block.shouldRenderFace leaves against neighbours.
    neighbours = [{}, {'up': 'minecraft:bedrock', 'down': 'minecraft:bedrock'},
                  {'north': 'minecraft:end_gateway', 'east': 'minecraft:glass', 'west': 'minecraft:oak_slab[type=top]',
                   'south': 'minecraft:end_stone', 'down': 'minecraft:oak_stairs[facing=east,half=top]'}]
    for age, cooldown in [(0, 0), (37, 0), (150, 0), (199, 0), (120, 25), (5000, 1), (5000, 17), (5000, 40), (5000, 0)]:
        add('minecraft:end_gateway', gateway={'age': age, 'cooldown': cooldown}, gameTime=rng.randint(0, 10 ** 6),
            level=rng.choice([[0, 256], [-64, 384]]), neighbours=rng.choice(neighbours))
    for n in neighbours:
        add('minecraft:end_portal', neighbours=n)
    return out


def light_coords(light):
    """Horizon light (sky*16 low byte, block*16 next) as vanilla light coords."""
    return ((light & 0xFF) << 16) | ((light >> 8) & 0xFF)


def lights():
    def norm(v):
        n = math.sqrt(sum(x * x for x in v))
        return [x / n for x in v]
    return norm([0.2, 1.0, -0.7]), norm([-0.2, 1.0, 0.7])


def items_flat_lights():
    """Lighting.Entry.ITEMS_FLAT: rotationY(-PI/8).rotateX(3PI/4) of the diffuse lights."""
    def rotate(v):
        a, b = -math.pi / 8, math.pi * 3 / 4
        x, y, z = v
        y, z = y * math.cos(b) - z * math.sin(b), y * math.sin(b) + z * math.cos(b)
        return [x * math.cos(a) + z * math.sin(a), y, -x * math.sin(a) + z * math.cos(a)]
    return tuple(rotate(l) for l in lights())


L0, L1 = lights()
FLAT = items_flat_lights()


def mix(normal, lit=(L0, L1)):
    d = lambda l: max(0.0, sum(a * b for a, b in zip(l, normal)))
    return min(1.0, (d(lit[0]) + d(lit[1])) * 0.6 + 0.4)


def vanilla_quads(submits, lit=(L0, L1)):
    quads, other = [], []
    for s in submits:
        custom = s['type'] == 'submitCustomGeometry'
        if s['type'] != 'submitModel' and not custom:
            other.append(s)
            continue
        verts = s['vertices']
        for i in range(0, len(verts), 4):
            q = verts[i:i + 4]
            tint = q[0][3] & 0xFFFFFFFF
            # Custom geometry (beacon beams) is unlit; models get the entity shader's mix light.
            b = 1.0 if custom else mix(q[0][8:11], lit)
            ch = lambda shift: int(((tint >> shift) & 255) * b)
            quads.append({'texture': s['texture'], 'v': [v[:3] + v[4:6] for v in q],
                          'rgb': [ch(16), ch(8), ch(0)], 'alpha': tint >> 24, 'light': q[0][7],
                          'overlay': q[0][6], 'type': s['renderType'], 'custom': custom})
    return quads, other


def area(q):
    import itertools
    p = [v[:3] for v in q]
    a = [p[1][i] - p[0][i] for i in range(3)]
    b = [p[2][i] - p[0][i] for i in range(3)]
    c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    a2 = [p[3][i] - p[0][i] for i in range(3)]
    d = [b[1] * a2[2] - b[2] * a2[1], b[2] * a2[0] - b[0] * a2[2], b[0] * a2[1] - b[1] * a2[0]]
    return math.sqrt(sum(x * x for x in c)) + math.sqrt(sum(x * x for x in d))


def pair(vq, rq):
    """Matches quads of the same texture globally by ascending vertex distance."""
    candidates = []
    for i, a in enumerate(vq):
        for j, b in enumerate(rq):
            if a['texture'] == b['texture']:
                d = sum((a['v'][k][c] - b['v'][k][c]) ** 2 for k in range(4) for c in range(5))
                candidates.append((d, i, j))
    candidates.sort()
    match, used = {}, set()
    for d, i, j in candidates:
        if i not in match and j not in used:
            match[i] = j
            used.add(j)
    return [(a, rq[match[i]] if i in match else None) for i, a in enumerate(vq)], \
        [b for j, b in enumerate(rq) if j not in used]


def compare_case(v, r, tol):
    problems = []
    vq, other = vanilla_quads(v['submits'], FLAT if 'loom' in v['case'] else (L0, L1))
    for s in other:
        if s['type'] == 'submitText':
            continue
        problems.append(f'unsupported vanilla submit {s["type"]}')
    rq = r.get('quads', [])
    if len(vq) != len(rq):
        tex = lambda qs: sorted({q['texture'] for q in qs})
        problems.append(f'{len(vq)} vanilla quads {tex(vq)} vs {len(rq)} Horizon quads {tex(rq)}')
    pairs, extra = pair(vq, rq)
    for b in extra[:1]:
        problems.append(f'extra Horizon quad {b["texture"]} {b["v"]}')
    for a, b in pairs:
        if b is None:
            problems.append(f'missing {a["texture"]} quad {a["v"]}')
            break
        bad = [(i, j) for i in range(4) for j in range(5) if abs(a['v'][i][j] - b['v'][i][j]) > tol * (1 + abs(a['v'][i][j]))]
        if bad:
            problems.append(f'{a["texture"]} vertices {a["v"]} != {b["v"]}')
            break
        if a['custom']:
            # Horizon packs the beam sprite's v range into the light bytes (its shader wraps v).
            got = b['color']
            if [got & 255, (got >> 8) & 255, (got >> 16) & 255, got >> 24] != a['rgb'] + [a['alpha']]:
                problems.append(f'{a["texture"]} colour {a["rgb"]}/{a["alpha"]} != {got:#x}')
                break
            continue
        if a['overlay'] != NO_OVERLAY:
            problems.append(f'overlay {a["overlay"]}')
        got = b['color']
        rgb = [got & 255, (got >> 8) & 255, (got >> 16) & 255]
        # A zero-area quad shows nothing: vanilla shades it by its face normal, Horizon cannot derive one.
        if area(a['v']) > 1e-9 and (any(abs(x - y) > 1 for x, y in zip(a['rgb'], rgb)) or (got >> 24) != a['alpha']):
            problems.append(f'{a["texture"]} colour {a["rgb"]}/{a["alpha"]} != {rgb}/{got >> 24} at {a["v"]}')
            break
        if a['light'] != light_coords(b['light']):
            problems.append(f'{a["texture"]} light {a["light"]:#x} != {light_coords(b["light"]):#x}')
            break
    vt = [s for s in v['submits'] if s['type'] == 'submitText' and s['text']]
    rt = r.get('texts', [])
    if len(vt) != len(rt):
        problems.append(f'{len(vt)} vanilla texts {[t["text"] for t in vt]} vs {len(rt)} Horizon texts {[t["text"] for t in rt]}')
    for a, b in zip(vt, rt):
        for k in ('text', 'x', 'y', 'color', 'outline', 'light'):
            av, bv = a[k], b[k]
            if k in ('color', 'outline'):
                av &= 0xFFFFFFFF
            if (abs(av - bv) > 1e-4) if isinstance(av, float) else av != bv:
                problems.append(f'text {a["text"]!r} {k} {av!r} != {bv!r}')
        if any(abs(x - y) > tol * (1 + abs(x)) for x, y in zip(a['matrix'], b['matrix'])):
            problems.append(f'text {a["text"]!r} matrix {a["matrix"]} != {b["matrix"]}')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--detail', type=int, default=2)
    parser.add_argument('--tolerance', type=float, default=2e-5)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-block-entity-render-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
    chosen = [c for c in cases() if args.filter in c['block']]
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaBlockEntityRenderOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaBlockEntityRenderOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    textures = sorted({s['texture'] for v in vanilla for s in v.get('submits', []) if 'texture' in s})
    (build / 'textures.txt').write_text(''.join(t + '\n' for t in textures))
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'block_entity_render_fixture', resources, inputs, build / 'textures.txt', build / 'rust.jsonl'], 'rust')
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust), (len(vanilla), len(rust))
    groups, errors, skipped, passed = {}, {}, {}, 0
    for v, r in zip(vanilla, rust):
        block = v['case']['block'].split('[')[0]
        if 'error' in v:
            errors.setdefault(block, v['error']); continue
        if 'skip' in r:
            if not v['submits']:
                passed += 1  # vanilla submits nothing either (e.g. a lectern without a book)
            else:
                skipped.setdefault(r['skip'], set()).add(block)
            continue
        problems = compare_case(v, r, args.tolerance)
        if problems:
            groups.setdefault(block, []).append((v['case'], problems))
        else:
            passed += 1
    for reason, blocks in sorted(skipped.items()):
        print(f'NOT RENDERED by Horizon ({reason}): {", ".join(sorted(blocks))}')
    for block, error in sorted(errors.items()):
        print(f'ORACLE ERROR {block}: {error}')
    for block, items in sorted(groups.items()):
        print(f'MISMATCH {block}: {len(items)} cases')
        for case, problems in items[:args.detail]:
            print('   ', json.dumps(case))
            for p in problems[:4]:
                print('      ', p[:600])
    total = len(vanilla)
    print(f'{passed}/{total} cases match; {sum(len(b) for b in skipped.values())} not rendered; {len(errors)} oracle errors')
    if groups or errors or skipped:
        sys.exit(1)
    print('block entity render parity ok')


if __name__ == '__main__':
    main()
