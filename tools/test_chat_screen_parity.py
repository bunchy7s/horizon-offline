#!/usr/bin/env python3
"""Compare the chat screen's input with the vanilla client's.

Each case opens the chat screen (empty or with "/", a GUI size, the sent
message history, automatic suggestions on or off, the chat background
option) and types, presses keys (editing, selection, word moves, select
all, history up and down, Tab and Shift+Tab through suggestions, Escape)
and clicks (on the suggestion list, on the input). The Java oracle
(VanillaChatScreenOracle) drives a real ChatScreen, whose CommandSuggestions
parse with vanilla's client command tree and ask the server's dispatcher
for the suggestions a node leaves to it; examples/probe/chat_screen_fixture runs
horizon_client::chat::ChatScreen on Horizon's dispatcher. After the actions
the input's value, cursor, selection and scroll and the screen's GUI calls
(the input background, the EditBox with its argument colouring, cursor,
selection and suggestion text, then the suggestion list or the usage lines)
must agree.

  test_chat_screen_parity.py [--count N] [--detail N]
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
TEXTS = ['/give @p minecraft:dia', '/tp ', '/gamemode cr', 'hello world', '/effect give @s ', '/execute as @a run ',
         '/time set da', '/xyz', '/give Steve stone 64 extra', '/tell Al', '/scoreboard objectives add k dummy',
         '/summon minecraft:zom', '/setblock ~ ~1 ~ minecraft:oak_stairs[facing=', '/team join r', '/kill @e[type=',
         '/', 'a', ' ', 'S', '@', '~ ', 'minecraft:', '/weather ', '/say this is a long message that goes on and on and on '
         'until it no longer fits the chat box', '/particle ', '/enchant @s sharp', '/difficulty ', '/gamerule ',
         '/attribute @s ', '/fill 0 0 0 1 1 1 stone repl', '/clear @p diamond 3', '/locate structure vil']
KEYS = ['tab', 'tab', 'tab', 'shift+tab', 'up', 'down', 'left', 'right', 'home', 'end', 'backspace', 'backspace',
        'ctrl+backspace', 'delete', 'ctrl+left', 'ctrl+right', 'shift+left', 'shift+right', 'shift+home', 'ctrl+a', 'escape']
HISTORY = ['/time set day', 'hi', '/give @p diamond 64', '/gamemode creative', 'a longer message in the history', '/tp ~ ~10 ~']


def cases(count):
    rng = random.Random(0x43484154)
    out = []
    for _ in range(count):
        width, height = rng.choice([(320, 240), (427, 240), (480, 270), (640, 360), (854, 480), (240, 180)])
        actions = []
        for _ in range(rng.choice([1, 2, 3, 5, 8, 12])):
            r = rng.random()
            if r < 0.45:
                actions.append({'type': 'type', 'text': rng.choice(TEXTS)})
            elif r < 0.85:
                key = rng.choice(KEYS)
                if key == 'escape' and rng.random() < 0.7:
                    key = 'tab'
                actions.append({'type': 'key', 'key': key})
            else:
                x = rng.randint(0, width // 2)
                y = rng.choice([rng.randint(height - 15 - 120, height - 15), rng.randint(height - 14, height - 1)])
                actions.append({'type': 'click', 'x': x, 'y': y})
        mouse = rng.choice([(rng.randint(0, width - 1), rng.randint(0, height - 1)),
                            (rng.randint(0, width // 2), rng.randint(height - 15 - 120, height - 15)), (0, 0)])
        out.append({'width': width, 'height': height, 'initial': rng.choice(['', '', '', '/']),
                    'history': rng.sample(HISTORY, rng.randint(0, len(HISTORY))), 'autoSuggest': rng.random() < 0.85,
                    'chatOnly': rng.random() < 0.7, 'opacity': rng.choice([0.0, 0.5, 1.0]), 'actions': actions,
                    'mouse': list(mouse), 'blink': rng.choice([0, 100, 400, 650])})
    # Always exercise registry-backed suggestions and their replacement ranges,
    # before and after Tab, including the redirect through execute's root.
    for text in ['/give @s diamond_sw', '/setblock ~ ~ ~ sto', '/fill ~ ~ ~ ~1 ~1 ~1 sto',
                 '/setblock ~ ~ ~ oak_stairs[fa', '/clear @s diamond_sw',
                 '/execute run give @s diamond_sw', '/fill ']:
        for keys in [[], ['tab'], ['tab', 'tab', 'shift+tab']]:
            out.append({'width': 640, 'height': 360, 'initial': '', 'history': [], 'autoSuggest': True,
                        'chatOnly': True, 'opacity': 0.5, 'mouse': [0, 0], 'blink': 0,
                        'actions': [{'type': 'type', 'text': text}]
                                   + [{'type': 'key', 'key': key} for key in keys]})
    # The box holds 256 characters: a longer paste is cut there, in the middle
    # of a selection the room left by the selection is kept, and once full no
    # key adds anything.
    for text, keys in [('a' * 300, ['end', 'backspace']),
                       ('b' * 255 + 'cdefghij', ['left', 'left', 'ctrl+a', 'delete', 'down']),
                       ('c' * 256, ['home', 'delete', 'right', 'end', 'backspace']),
                       ('/' + 'd' * 300, ['end', 'ctrl+backspace', 'tab'])]:
        out.append({'width': 320, 'height': 240, 'initial': '', 'history': [], 'autoSuggest': True,
                    'chatOnly': True, 'opacity': 0.5, 'mouse': [0, 0], 'blink': 0,
                    'actions': [{'type': 'type', 'text': text}]
                               + [{'type': 'key', 'key': key} for key in keys]})
    # Characters the box never takes (the section sign, DEL, control codes)
    # beside ones it does, and a value that already holds a `§`: the box draws
    # its value as the literal component vanilla builds from it, so the code
    # is a character on the screen, not a format.
    for text in ['ab\u00a7cd\u007f' + 'e\u0001f\u001fg\u00c9h', '\u00a7', 'x' * 250 + '\u00a7y' * 20]:
        out.append({'width': 427, 'height': 240, 'initial': '\u00a7z', 'history': [], 'autoSuggest': False,
                    'chatOnly': False, 'opacity': 1.0, 'mouse': [10, 30], 'blink': 400,
                    'actions': [{'type': 'type', 'text': text}, {'type': 'key', 'key': 'left'}]})
    # A command typed with a colour code in it, as a command block's editor
    # opens with: the code is drawn, the text after it keeps its own colour.
    for initial in ['/say \u00a7chello', '/tellraw @a {"text":"\u00a7lbold"}', '\u00a7']:
        out.append({'width': 640, 'height': 360, 'initial': initial, 'history': [], 'autoSuggest': True,
                    'chatOnly': True, 'opacity': 0.5, 'mouse': [0, 0], 'blink': 0,
                    'actions': [{'type': 'key', 'key': 'left'}, {'type': 'key', 'key': 'left'},
                                {'type': 'key', 'key': 'end'}]})
    # A long value with codes in it, past the box's width: the box scrolls to
    # the cursor and the drawn window keeps the codes it contains.
    for keys in ([], ['home'], ['end'], ['shift+home'], ['ctrl+a'], ['home', 'shift+end']):
        out.append({'width': 320, 'height': 240, 'initial': '\u00a7a' + 'x' * 100 + '\u00a7b' + 'y' * 100,
                    'history': [], 'autoSuggest': True, 'chatOnly': True, 'opacity': 0.5, 'mouse': [0, 0],
                    'blink': 0, 'actions': [{'type': 'key', 'key': 'end'}]
                                            + [{'type': 'key', 'key': key} for key in keys]})
    # A history entry longer than the box is recalled cut to its length, and
    # the draft typed before going back through the history is not lost when
    # the history is walked forward again.
    out.append({'width': 640, 'height': 360, 'initial': '', 'history': ['h' * 300, '/time set day'],
                'autoSuggest': True, 'chatOnly': True, 'opacity': 0.5, 'blink': 0,
                'mouse': [0, 0],
                'actions': [{'type': 'type', 'text': 'draft'}, {'type': 'key', 'key': 'up'},
                            {'type': 'key', 'key': 'up'}, {'type': 'key', 'key': 'home'},
                            {'type': 'key', 'key': 'down'}, {'type': 'key', 'key': 'down'}]})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=3000)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'chat-screen-parity'; build.mkdir(parents=True, exist_ok=True)
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
         *(PARITY / f'{n}.java' for n in ('ParityData', 'ParityFont', 'VanillaPackScreenOracle', 'VanillaCommandParseOracle',
                                          'VanillaChatScreenOracle'))], 'compile-java')
    java = ['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies]
    run([*java, 'dev.horizonlab.parity.VanillaCommandParseOracle', 'registries', build], 'registries')
    run([*java, 'dev.horizonlab.parity.VanillaChatScreenOracle', inputs, build / 'vanilla.jsonl'], 'vanilla')
    lib.cargo_example('horizon-client', 'chat_screen_fixture', lib.RESOURCES, build / 'registries.json', inputs, build / 'rust.jsonl')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen), (len(vanilla), len(rust), len(chosen))
    errors = [v['error'] for v in vanilla if 'error' in v]
    bad = [(c, v, r) for c, v, r in zip(chosen, vanilla, rust) if 'error' not in v and v != r]
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e[:1500])
    for c, v, r in bad[:args.detail]:
        print('MISMATCH', json.dumps({k: c[k] for k in c if k != 'history'})[:1500])
        for k in ('closed', 'value', 'cursor', 'highlight', 'display'):
            if v.get(k) != r.get(k):
                print(f'   {k}: vanilla {v.get(k)!r} | horizon {r.get(k)!r}')
        a, b = v.get('draws', []), r.get('draws', [])
        i = next((i for i in range(max(len(a), len(b))) if i >= len(a) or i >= len(b) or a[i] != b[i]), None)
        if i is not None:
            print(f'   call {i}: vanilla {json.dumps(a[i]) if i < len(a) else None}')
            print(f'   call {i}: horizon {json.dumps(b[i]) if i < len(b) else None}')
    print(f'{len(chosen) - len(bad) - len(errors)}/{len(chosen)} chat screens match; {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('chat screen parity ok')


if __name__ == '__main__':
    main()
