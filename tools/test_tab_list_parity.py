#!/usr/bin/env python3
"""Compare the player list (tab list) with the vanilla 26.3 client.

The Java oracle (VanillaTabListOracle) runs vanilla's own
PlayerTabOverlay.extractRenderState, frame after frame, on a stand-in client
whose connection lists the case's players (game mode, latency, display name,
list order, hat layer, online mode for heads, skins from each profile's signed or
unsigned `textures` property through a real SkinManager), whose level holds some of
them (upside-down Dinnerbone and Grumm) and whose real Scoreboard has the
case's teams (colour, prefix, suffix), `list` objective (integer or hearts,
number formats) and scores, with the header, footer, row background option
and Hud ticks. It records every fill, text (as styled runs), sprite and
face blit. `examples/probe/tab_list_fixture` lays out the same frames with
`horizon_client::tab_list`; the ordered call lists must agree.

Downloaded skin images are compared too: VanillaSkinImageOracle decodes
PNGs as NativeImage does and runs SkinTextureDownloader.processLegacySkin;
`examples/probe/skin_image_fixture` runs `horizon_client::skins`. Pixels must agree.

  test_tab_list_parity.py [--count N] [--detail N]
"""
import argparse
import base64
import struct
import zlib
import json
import os
import random
import subprocess
import sys
import uuid

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
COLORS = ['black', 'dark_blue', 'dark_green', 'dark_aqua', 'dark_red', 'dark_purple', 'gold', 'gray', 'dark_gray',
          'blue', 'green', 'aqua', 'red', 'light_purple', 'yellow', 'white']
WORDS = ['Welcome', 'to', 'the', 'server', 'Horizon', 'players', 'online', 'mc.example.net', 'Have', 'fun!',
         'Supercalifragilisticexpialidocious', 'a', 'é', 'Ünïcödé', '§aGreen', '§lBold', '[VIP]', '1234567890']
NAMES = ['Steve', 'Alex', 'Notch', 'jeb_', 'Dinnerbone', 'Grumm', 'xX_Pro_Xx', 'a', 'Zed', 'zebra', 'Émile', 'ÆON',
         'bob', 'Carl', '_under', '123', 'MMMMMMMMMMMMMMMM', 'iiiiiiii', 'Wide_W', 'ßtraße', 'Ⅻ', 'Mix3d_Case']


def component(rng, words=3, depth=0):
    text = ' '.join(rng.choice(WORDS) for _ in range(words))
    kind = rng.random()
    if kind < 0.3:
        return text
    c = {'text': text}
    if rng.random() < 0.5:
        c['color'] = rng.choice(COLORS + ['#12ABEF'])
    for flag in ('bold', 'italic', 'underlined', 'strikethrough', 'obfuscated'):
        if rng.random() < 0.15:
            c[flag] = rng.random() < 0.8
    if depth < 2 and rng.random() < 0.4:
        c['extra'] = [component(rng, rng.randint(1, 3), depth + 1) for _ in range(rng.randint(1, 2))]
    if rng.random() < 0.08:
        c = {'translate': 'multiplayer.player.list.hp', 'with': [text], 'color': rng.choice(COLORS)}
    return c


def number_format(rng):
    kind = rng.random()
    if kind < 0.4:
        return None
    if kind < 0.55:
        return {'type': 'blank'}
    if kind < 0.85:
        style = {'color': rng.choice(COLORS)}
        if rng.random() < 0.4:
            style['bold'] = True
        return {'type': 'styled', 'style': style}
    return {'type': 'fixed', 'value': component(rng, 1)}


TEXTURE_URIS = ['https://textures.minecraft.net/texture/{textureId}', 'http://textures.minecraft.net/texture/{textureId}']


def textures_property(rng, name, hashes):
    """A `textures` profile property: base64 JSON naming skin, cape and
    elytra URLs (some outside the allowed domains or malformed), signed
    validly, invalidly or not at all."""
    def url():
        h = rng.choice(hashes)
        return rng.choice([
            'http://textures.minecraft.net/texture/' + h, 'https://textures.minecraft.net/texture/' + h,
            'http://textures.minecraft.net/texture/' + h + '.png', 'https://textures.minecraft.net/texture/' + h + '?v=2',
            'https://evil.example.net/texture/' + h, 'http://textures.minecraft.net.evil/texture/' + h,
            'http://textures.minecraft.net/texture/a/b/' + h + '#frag'])
    textures = {}
    if rng.random() < 0.9:
        textures['SKIN'] = {'url': url()}
        if rng.random() < 0.4:
            textures['SKIN']['metadata'] = {'model': 'slim'}
    if rng.random() < 0.25:
        textures['CAPE'] = {'url': url()}
    if rng.random() < 0.08:
        textures['ELYTRA'] = {'url': url()}
    if rng.random() < 0.03:
        textures['SKIN'] = {}
    payload = {'timestamp': rng.randint(0, 2**41), 'profileId': uuid.UUID(int=rng.getrandbits(128)).hex,
               'profileName': name, 'textures': textures}
    if rng.random() < 0.2:
        payload['signatureRequired'] = True
    text = rng.choice([json.dumps(payload)] * 12 + ['not json', 'null', '', '{"textures": {}}', '[1, 2]'])
    value = base64.b64encode(text.encode()).decode()
    kind = rng.random()
    if kind < 0.05:
        value = value.rstrip('=')
    elif kind < 0.08:
        value = rng.choice(['%%%', 'abc', 'YQ==YQ==', value + '!'])
    return {'value': value, 'signature': rng.choice([None, 'valid', 'valid', 'invalid'])}


def cases(count):
    rng = random.Random(0x544142)
    out = []
    for n in range(count):
        size = rng.choice([1, 1, 2, 3, 5, 8, 12, 19, 20, 21, 25, 40, 41, 60, 79, 80, 85, 100])
        names = []
        while len(names) < size:
            name = rng.choice(NAMES) if rng.random() < 0.5 else ''.join(
                rng.choice('abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_') for _ in range(rng.randint(1, 16)))
            if name.lower() not in {x.lower() for x in names}:
                names.append(name)
        hashes = [''.join(rng.choice('0123456789abcdef') for _ in range(64)) for _ in range(rng.randint(1, 12))]
        players = []
        for name in names:
            players.append({
                'uuid': str(uuid.UUID(int=rng.getrandbits(128))), 'name': name,
                'gameMode': rng.choice([0, 0, 0, 1, 2, 3]),
                'latency': rng.choice([-1, 0, 1, 149, 150, 299, 300, 599, 600, 999, 1000, 5000, rng.randint(0, 2000)]),
                'displayName': component(rng, rng.randint(1, 2)) if rng.random() < 0.2 else None,
                'listOrder': rng.choice([0, 0, 0, 1, -1, 5, 2147483647, -2147483648]),
                'showHat': rng.random() < 0.8, 'inLevel': rng.random() < 0.5,
                'textures': textures_property(rng, name, hashes) if rng.random() < 0.75 else None,
            })
        downloads = {h: rng.choice(['ok'] * 20 + ['fail'] * 3 + ['pending']) for h in hashes if rng.random() < 0.9}
        teams = []
        for t in range(rng.choice([0, 0, 1, 2, 4])):
            members = [p['name'] for p in players if rng.random() < 0.3]
            teams.append({'name': rng.choice(['red', 'Blue', 'team' + str(t), 'ä', 'z', 'A']) + str(t),
                          'color': rng.choice([None] + COLORS),
                          'prefix': rng.choice(['', '[R] ', {'text': '★ ', 'color': 'gold', 'bold': True}, component(rng, 1)]),
                          'suffix': rng.choice(['', ' ✔', {'text': ' (afk)', 'italic': True}]), 'members': members})
        objective = None
        if rng.random() < 0.7:
            objective = {'name': 'obj', 'renderType': rng.choice(['integer', 'hearts', 'hearts']), 'numberFormat': number_format(rng)}
        frames = []
        tick = rng.randint(0, 100000)
        values = {}
        for f in range(rng.randint(1, 6)):
            scores = {}
            for p in players:
                if objective and (f == 0 and rng.random() < 0.85 or rng.random() < 0.3):
                    v = rng.choice([0, 1, 2, 7, 19, 20, 21, 40, 99, 100, -3, 1234567, rng.randint(-50, 300)])
                    if p['name'] in values and rng.random() < 0.5:
                        v = values[p['name']] + rng.choice([-3, -1, 1, 2, 10])
                    values[p['name']] = v
                    scores[p['name']] = {'value': v, 'format': number_format(rng) if rng.random() < 0.2 else None}
            reset = [p['name'] for p in players if objective and rng.random() < 0.03]
            for r in reset:
                values.pop(r, None)
            tick += rng.choice([1, 1, 2, 3, 5, 10, 21, 40])
            frames.append({'tick': tick, 'visible': rng.random() < 0.85, 'scores': scores, 'reset': reset})
        header = footer = None
        if rng.random() < 0.6:
            header = rng.choice([component(rng, rng.randint(1, 30)), 'Line one\nLine two', '\n', 'trailing\n', '  two  spaces  ',
                                 {'text': 'Header', 'extra': ['\n', component(rng, 12)]}])
        if rng.random() < 0.5:
            footer = rng.choice([component(rng, rng.randint(1, 30)), 'Footer', 'x\n\ny'])
        out.append({'width': rng.choice([427, 320, 480, 640, 961, 160, 90, 300]), 'onlineMode': rng.random() < 0.7,
                    'backgroundForChatOnly': rng.random() < 0.8, 'opacity': rng.choice([0.5, 0.0, 1.0, 0.37]),
                    'header': header, 'footer': footer, 'teams': teams, 'objective': objective, 'players': players,
                    'frames': frames, 'downloads': downloads,
                    'local': rng.choice(players)['uuid'] if rng.random() < 0.3 else None,
                    'validUris': rng.choice([TEXTURE_URIS] * 6 + [TEXTURE_URIS[:1], []])})
    return out


def png(width, height, color_type, depth, rows):
    """A PNG of the given colour type (0 grey, 2 RGB, 4 grey+alpha, 6 RGBA)."""
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    raw = b''.join(b'\0' + row for row in rows)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, depth, color_type, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def skin_images(count):
    """Downloaded skins: 64x32 legacy and 64x64 ones in every 8-bit colour
    type (and 16-bit RGBA), with opaque, clear and mixed hat layers, and a
    few sizes vanilla refuses."""
    rng = random.Random(0x534B494E)
    out = []
    for _ in range(count):
        width, height = rng.choice([(64, 32)] * 5 + [(64, 64)] * 5 + [(32, 32), (64, 48), (128, 64), (64, 128), (1, 1)])
        color_type, depth = rng.choice([(6, 8)] * 6 + [(2, 8), (0, 8), (4, 8), (6, 16)])
        channels = {0: 1, 2: 3, 4: 2, 6: 4}[color_type]
        alpha = rng.choice(['opaque', 'clear', 'mixed', 'random'])
        rows = []
        for y in range(height):
            row = bytearray()
            for x in range(width):
                px = [rng.randrange(256) for _ in range(channels)]
                if color_type in (4, 6):
                    a = {'opaque': rng.randint(128, 255), 'clear': 0, 'mixed': rng.choice([0, 255, 127, 128]),
                         'random': rng.randrange(256)}[alpha]
                    px[-1] = a
                for v in px:
                    row += bytes([v, rng.randrange(256)]) if depth == 16 else bytes([v])
            rows.append(bytes(row))
        out.append(base64.b64encode(png(width, height, color_type, depth, rows)).decode())
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=1000)
    parser.add_argument('--detail', type=int, default=3)
    parser.add_argument('--images', type=int, default=400)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-tab-list-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
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
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaTabListOracle', 'VanillaSkinImageOracle'))], 'compile-java')
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaTabListOracle',
         inputs, build / 'vanilla.jsonl', resources / 'assets'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'tab_list_fixture', resources, inputs, build / 'rust.jsonl'], 'rust')
    images = build / 'skins.txt'; images.write_text(''.join(i + '\n' for i in skin_images(args.images)))
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaSkinImageOracle',
         images, build / 'vanilla-skins.txt'], 'vanilla-skins')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'skin_image_fixture', images, build / 'rust-skins.txt'], 'rust-skins')
    vanilla_skins = (build / 'vanilla-skins.txt').read_text().splitlines()
    rust_skins = (build / 'rust-skins.txt').read_text().splitlines()
    assert len(vanilla_skins) == len(rust_skins) == args.images
    skin_bad = [i for i, (a, b) in enumerate(zip(vanilla_skins, rust_skins)) if a != b]
    for i in skin_bad[:args.detail]:
        a, b = vanilla_skins[i], rust_skins[i]
        if 'refused' in (a, b):
            print(f'SKIN MISMATCH image {i}: vanilla {a[:20]}, horizon {b[:20]}')
        else:
            a, b = base64.b64decode(a), base64.b64decode(b)
            k = next(k for k in range(min(len(a), len(b))) if a[k] != b[k]) // 4
            print(f'SKIN MISMATCH image {i}: pixel ({k % 64}, {k // 64}) vanilla {a[k*4:k*4+4].hex()} horizon {b[k*4:k*4+4].hex()}')
    print(f'{args.images - len(skin_bad)}/{args.images} processed skin images match '
          f'({sum(v == "refused" for v in vanilla_skins)} refused)')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust)
    errors, bad, frames, calls = [], [], 0, 0
    for v, r in zip(vanilla, rust):
        if 'error' in v:
            errors.append(v['error']); continue
        for i, (a, b) in enumerate(zip(v['frames'], r['frames'])):
            frames += 1
            calls += len(a)
            if a != b:
                first = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                bad.append((v['case'], i, first, a[first:first + 2], b[first:first + 2], len(a), len(b)))
                break
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for case, i, k, a, b, la, lb in bad[:args.detail]:
        print('MISMATCH', json.dumps(case)[:3000])
        print(f'   frame {i}, call {k} ({la} vanilla, {lb} Horizon calls)\n   vanilla {a}\n   horizon {b}')
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} player lists match ({frames} frames, {calls} calls); '
          f'{len(errors)} oracle errors')
    if bad or errors or skin_bad:
        sys.exit(1)
    print('tab list parity ok')


if __name__ == '__main__':
    main()
