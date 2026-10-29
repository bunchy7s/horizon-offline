#!/usr/bin/env python3
"""Compare a text display's text geometry with the vanilla 26.3 client's TextDisplayRenderer.

The Java oracle (VanillaTextDisplayRenderOracle) sets a text display's synced data
(text, line width, background, opacity, flags), runs the real
TextDisplayRenderer.submitInner (line split, alignment, default background from the
options) and turns its submits into vertices as TextFeatureRenderer does, with glyphs
baked from the real font providers. `examples/probe/text_display_render_fixture`
decodes the same entity data from vanilla's wire bytes and lays the text out with
`horizon_client::text_display`. Each case's pose must agree, and its quads
[x, y, z, ARGB] in text units as multisets (Horizon draws all shadows first): the
depths keep the background behind the shadow, behind the glyph and its bold copy.

  test_text_display_render_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random

try:
    from . import lab, test_rust_parity as base, oracle_harness as harness, test_model_baking_parity as baking
except ImportError:
    import lab
    import test_rust_parity as base
    import oracle_harness as harness
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
# Glyphs of the `default` include (ascii, accented, nonlatin_european); no Unihex.
CHARS = ('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789'
         ' .,:;!?\'"()[]{}<>-_+=*/\\|@#$%^&~`' 'äöüßéèêñçÅØæ' 'АБВГДабвгдΩλπ')
COLORS = ['gold', 'red', 'aqua', 'gray', 'dark_gray', 'green', 'white', 'black', '#FFAA00', '#123456', '#E0E0FF']
LEGACY = '0123456789abcdeflmnor'


def words(rng, count):
    return ' '.join(''.join(rng.choice(CHARS) for _ in range(rng.randint(1, 12))) for _ in range(count))


def part(rng):
    text = words(rng, rng.randint(1, 4))
    if rng.random() < 0.15:
        text = text.replace(' ', '\n', 1)
    if rng.random() < 0.2:
        # Servers colour holograms with legacy codes inside the text.
        text = ''.join(c if rng.random() > 0.15 else '§' + rng.choice(LEGACY) + c for c in text)
    out = {'text': text}
    for key in ('bold', 'italic', 'underlined', 'strikethrough'):
        if rng.random() < 0.3:
            out[key] = rng.random() < 0.8
    if rng.random() < 0.6:
        out['color'] = rng.choice(COLORS)
    return out


def cases(count):
    rng = random.Random(0x7E57D15)
    fixed = [
        {'text': {'text': 'CityBuild', 'color': 'gold', 'bold': True}, 'flags': 1},
        {'text': {'text': '', 'extra': [{'text': 'CityBuild\n', 'color': 'gold', 'bold': True},
                                        {'text': 'Spieler online: 42', 'color': 'gray'}]}, 'flags': 1},
        {'text': {'text': 'Underlined', 'underlined': True, 'strikethrough': True}, 'flags': 1},
        {'text': {'text': 'see through'}, 'flags': 3},
        {'text': {'text': 'default background'}, 'flags': 4},
        {'text': {'text': 'faint'}, 'flags': 1, 'opacity': 4, 'background': 0},
    ]
    out = []
    for n in range(count):
        c = dict(fixed[n]) if n < len(fixed) else {}
        if n >= len(fixed):
            parts = [part(rng) for _ in range(rng.choice([1, 1, 2, 3]))]
            c['text'] = parts[0] if len(parts) == 1 else {'text': '', 'extra': parts, **({'color': rng.choice(COLORS)} if rng.random() < 0.3 else {})}
            c['flags'] = rng.randrange(32)
            if rng.random() < 0.6:
                c['line_width'] = rng.choice([200, 100, 60, 30, 8, 400])
            if rng.random() < 0.7:
                c['background'] = rng.choice([0, 0x40000000, -0x40000000, 0x7F00FF00, rng.randint(-2**31, 2**31 - 1)])
            if rng.random() < 0.5:
                c['opacity'] = rng.choice([-1, 127, -128, 0, 3, 4, 25, 26, 64, rng.randint(-128, 127)])
        c['chat_only'] = rng.random() < 0.3
        c['background_opacity'] = rng.choice([0.0, 0.5, 1.0, 0.25])
        out.append(c)
    return out


def canonical(quads):
    """Each quad as its sorted corners, rounded below the 0.001 bold step; the quads sorted."""
    return sorted(tuple(sorted((round(x, 4), round(y, 4), round(z, 4), color) for x, y, z, color in q)) for q in quads)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=600)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'text-display-render-parity'
    build.mkdir(parents=True, exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    sources = [PARITY / 'VanillaTextDisplayRenderOracle.java', lab.ROOT / 'crates/horizon-client/src/text_display.rs',
               lab.ROOT / 'crates/horizon-client-render/src/hud.rs', lab.ROOT / 'crates/horizon-client-render/src/font.rs']
    tested = base.source_hashes(sources)
    chosen = cases(args.count)
    inputs = build / 'cases.jsonl'
    inputs.write_text(''.join(json.dumps(c, ensure_ascii=False) + '\n' for c in chosen), encoding='utf-8')
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    harness.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies,
                        '-d', classes, *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaTextDisplayRenderOracle'))],
                       build, 'compile-java')
    harness.run_logged(['java', '-Xmx1G', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
                        'dev.horizonlab.parity.VanillaTextDisplayRenderOracle', inputs, build / 'vanilla.jsonl'], build, 'vanilla')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text(encoding='utf-8').splitlines()]
    errors = [(c, v['error']) for c, v in zip(chosen, vanilla) if 'error' in v]
    for c, e in errors[:3]:
        print('ORACLE ERROR', json.dumps(c, ensure_ascii=False), e)
    wire = build / 'wire.jsonl'
    wire.write_text(''.join(json.dumps({'wire': v.get('wire', ''), 'chat_only': c['chat_only'],
                                        'background_opacity': c['background_opacity']}) + '\n'
                            for c, v in zip(chosen, vanilla)))
    command = harness.build_probe(build, 'text_display_render_fixture')
    harness.run_logged([*command, lab.RESOURCES, wire, build / 'rust.jsonl'], build, 'rust')
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text(encoding='utf-8').splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    bad, quads = [], 0
    for c, v, r in zip(chosen, vanilla, rust):
        if 'error' in v:
            continue
        quads += len(v['quads'])
        pose = all(abs(a - b) <= 2e-5 for a, b in zip(v['pose'], r['pose']))
        if not pose or canonical(v['quads']) != canonical(r['quads']):
            bad.append((c, v, r, pose))
    for c, v, r, pose in bad[:args.detail]:
        print('MISMATCH', json.dumps(c, ensure_ascii=False))
        if not pose:
            print('   pose vanilla', v['pose'], '\n   pose horizon', r['pose'])
        a, b = canonical(v['quads']), canonical(r['quads'])
        only_v = [q for q in a if q not in b][:3]
        only_r = [q for q in b if q not in a][:3]
        print(f'   {len(a)} vanilla quads, {len(b)} Horizon; only vanilla {only_v}; only Horizon {only_r}')
    print(f'{len(chosen) - len(bad) - len(errors)}/{len(chosen)} text display cases match ({quads} quads); '
          f'{len(errors)} oracle errors')
    if bad or errors:
        raise SystemExit(1)
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(build / 'report.json', {'minecraft': lab.VERSION, 'cases': len(chosen), 'quads': quads, 'pass': True,
                                           'sources_sha256': tested})
    print('text display render parity ok')


if __name__ == '__main__':
    main()
