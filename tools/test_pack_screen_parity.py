#!/usr/bin/env python3
"""Compare the resource pack screen's drawing with the vanilla client's.

Each case is a title (the resource pack screen's, or the data pack screen's
Create World opens), a pack repository (titles and descriptions as text components,
with legacy formatting codes, colours and styles; pack sources; compatible,
too old, too new or unknown packs; required, fixed and positioned packs;
icons), an initial selection, a GUI size, in a world or not, and what the
player does: clicks on the lists (an entry's icon arrows, an entry, a
header, the scroll bar, empty space), on the search box, typing and erasing
in it, the mouse wheel over the lists, and the ConfirmScreen an incompatible
pack opens (yes, no, or left open). The Java oracle
(VanillaPackScreenOracle) runs a real PackSelectionScreen through these and
records every GuiGraphicsExtractor call of the screen shown at the end
(widgets, list backgrounds, scissor, separators, scroll bars, entries with
their incompatibility tint, hover overlay and arrows, the selection
outline, the search box, the buttons and their tooltip);
examples/probe/pack_screen_fixture runs horizon_client::pack_screen, which the
game's pack screen is, through the same actions. The call lists must agree.

  test_pack_screen_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random
import subprocess
import sys

try:
    from . import interaction_lab as lib, lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import interaction_lab as lib
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
IDS = ['vanilla', 'programmer_art', 'file/Faithful.zip', 'file/alpha', 'file/Zeta pack', 'file/zeta', 'file/é', 'file/0',
       'server', 'world', 'fabric', 'file/a.zip', 'file/A.zip', 'file/b', 'file/HC', 'file/Stay True 1.21.zip']
WORDS = ['Faithful', 'pack', 'textures', 'the', 'default', 'look', 'of', 'Minecraft', 'x32', 'by', 'Vattic',
         'Programmer', 'Art', 'resolution', 'a', 'very', 'loooooooooooooooooooooongword', '§4red', '§lbold§r', '1.21', '!',
         'Stay', 'True', 'é', 'Ünïcödé']
COLORS = ['red', 'gold', 'aqua', 'gray', '#12AB34', 'dark_purple', 'yellow']


def words(rng, lo, hi):
    return ' '.join(rng.choice(WORDS) for _ in range(rng.randint(lo, hi)))


def component(rng, lo, hi):
    kind = rng.random()
    if kind < 0.45:
        return words(rng, lo, hi)
    if kind < 0.6:
        return words(rng, lo, hi) + '\n' + words(rng, 1, hi)
    if kind < 0.85:
        part = {'text': words(rng, lo, hi)}
        for key in ('bold', 'italic', 'underlined'):
            if rng.random() < 0.2:
                part[key] = True
        if rng.random() < 0.6:
            part['color'] = rng.choice(COLORS)
        if rng.random() < 0.4:
            part['extra'] = [{'text': ' ' + words(rng, 1, 4), 'color': rng.choice(COLORS)}]
        return part
    return [words(rng, 1, 3), {'text': ' ' + words(rng, lo, hi), 'color': rng.choice(COLORS)}]


def cases(count):
    rng = random.Random(0x50534352)
    out = []
    for n in range(count):
        width, height = rng.choice([(320, 240), (427, 240), (480, 270), (640, 360), (854, 480), (400, 160), (570, 320)])
        ids = rng.sample(IDS, rng.randint(1, 14))
        packs = []
        for i in ids:
            packs.append({'id': i, 'title': component(rng, 1, 3) if rng.random() < 0.8 else i,
                          'description': component(rng, 0, 14),
                          'source': rng.choice(['default'] * 4 + ['builtin', 'world', 'server', 'feature']),
                          'compat': rng.choice(['compatible'] * 5 + ['old', 'new', 'unknown']),
                          'required': rng.random() < 0.15, 'position': rng.choice(['top', 'top', 'bottom']),
                          'fixed': rng.random() < 0.15, 'icon': rng.random() < 0.4})
        selected = rng.sample(ids, rng.randint(0, len(ids)))
        cx = width // 2
        left, right = cx - 15 - 200, cx + 15
        top, bottom = 49, height - 33

        def point():
            r = rng.random()
            if r < 0.6:
                x0 = rng.choice([left, right])
                row = rng.randint(0, 9)
                y = top + 2 + 13 + 36 * row + rng.randint(-2, 35)
                x = x0 + rng.choice([rng.randint(0, 40), rng.randint(0, 200), rng.randint(2, 36), rng.randint(194, 199)])
                return x, min(y, bottom - 1)
            if r < 0.7:
                return rng.randint(cx - 100, cx + 99), rng.randint(30, 44)
            if r < 0.8:
                return rng.choice([left, right]) + rng.randint(0, 199), rng.randint(top, top + 16)
            return rng.randint(0, width - 1), rng.randint(0, bottom - 1)
        def safe(x, y):
            # Not the Open Pack Folder button: vanilla would open the folder.
            return not (cx - 154 <= x < cx - 4 and height - 26 <= y < height - 6)
        actions = []
        for _ in range(rng.choice([0, 1, 2, 4, 8, 14])):
            r = rng.random()
            if r < 0.7:
                x, y = point() if r < 0.6 else (rng.randint(cx - 160, cx + 160), rng.randint(height // 2 - 30, height // 2 + 60))
                if safe(x, y):
                    actions.append({'type': 'click', 'x': x, 'y': y})
            elif r < 0.8:
                x, y = point()
                actions.append({'type': 'scroll', 'x': x, 'y': y, 'amount': rng.choice([-3, -1, 1, 2, 0.5])})
            elif r < 0.93:
                actions.append({'type': 'type', 'text': rng.choice(['a', 'e', 'pack', 'file', 'Faith', 'z', ' ', 'zzz', 'x32 by'])})
            else:
                actions.append({'type': 'erase', 'count': rng.randint(1, 4)})
        mouse = rng.choice([point(), point(), point(), (rng.randint(0, width - 1), rng.randint(0, height - 1)),
                            (rng.randint(cx - 154, cx + 154), height - 26 + rng.randint(0, 19)),
                            (rng.randint(cx - 160, cx + 160), rng.randint(height // 2 - 30, height // 2 + 60))])
        out.append({'width': width, 'height': height, 'inWorld': rng.random() < 0.3, 'packs': packs, 'selected': selected,
                    'actions': actions, 'mouse': list(mouse), 'blink': rng.choice([0, 100, 400, 650]),
                    # Create World's Data Packs screen is the same screen under its own title.
                    'title': 'dataPack.title' if n % 4 == 3 else 'resourcePack.title'})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=3000)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'pack-screen-parity'; build.mkdir(parents=True, exist_ok=True)
    (build / 'resourcepacks').mkdir(exist_ok=True)
    chosen = cases(args.count)
    for c in chosen:
        c['dir'] = str(build / 'resourcepacks')
    inputs = build / 'inputs.jsonl'; inputs.write_text(''.join(json.dumps(c) + '\n' for c in chosen))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaPackScreenOracle'))], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaPackScreenOracle',
         inputs, build / 'vanilla.jsonl', lab.RESOURCES / 'assets'], 'vanilla')
    lib.cargo_example('horizon-client', 'pack_screen_fixture', lib.RESOURCES, inputs, build / 'rust.jsonl')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    errors = [v['error'] for v in vanilla if 'error' in v]
    bad = []
    for c, v, r in zip(chosen, vanilla, rust):
        if 'error' in v or v == r:
            continue
        bad.append((c, v, r))
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e[:1500])
    for c, v, r in bad[:args.detail]:
        brief = {k: c[k] for k in ('title', 'width', 'height', 'inWorld', 'selected', 'actions', 'mouse', 'blink')}
        print('MISMATCH', json.dumps(brief)[:1500])
        if v.get('screen') != r.get('screen'):
            print(f'   screen: vanilla {v.get("screen")} | horizon {r.get("screen")}')
        a, b = v.get('draws', []), r.get('draws', [])
        i = next((i for i in range(max(len(a), len(b))) if i >= len(a) or i >= len(b) or a[i] != b[i]), None)
        if i is not None:
            print(f'   call {i}: vanilla {json.dumps(a[i]) if i < len(a) else None}')
            print(f'   call {i}: horizon {json.dumps(b[i]) if i < len(b) else None}')
    print(f'{len(chosen) - len(bad) - len(errors)}/{len(chosen)} pack screens match; {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('pack screen parity ok')


if __name__ == '__main__':
    main()
