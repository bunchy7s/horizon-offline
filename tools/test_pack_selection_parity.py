#!/usr/bin/env python3
"""Compare the resource pack screen's selection with the vanilla client's.

Each case is a pack repository (packs required or not, added at the top or
the bottom, fixed in place or not, compatible, too old or too new), an
initial selection as options.txt stores it, and the actions the screen's
entries take on a click or key press: select (an incompatible pack only when
its confirmation is accepted), unselect, move up, move down. The Java oracle
(VanillaPackSelectionOracle) runs vanilla's PackRepository and
PackSelectionModel; examples/probe/pack_selection_fixture runs
horizon_client::packs::SelectionModel, which the game's pack screen uses.
After every action both lists, in order, with each entry's canSelect,
canUnselect, canMoveUp and canMoveDown, and the selection committed at the
end must agree.

  test_pack_selection_parity.py [--count N] [--detail N]
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
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
IDS = ['vanilla', 'programmer_art', 'file/Faithful.zip', 'file/alpha', 'file/Zeta pack', 'file/zeta', 'file/é',
       'file/0', 'server', 'world', 'fabric', 'file/a.zip', 'file/A.zip', 'file/b']


def cases(count):
    rng = random.Random(0x5041434B)
    out = []
    for _ in range(count):
        ids = rng.sample(IDS, rng.randint(1, 9))
        packs = [{'id': i, 'required': rng.random() < 0.2, 'position': rng.choice(['top', 'top', 'bottom']),
                  'fixed': rng.random() < 0.2, 'compat': rng.choice(['compatible'] * 4 + ['old', 'new'])} for i in ids]
        selected = rng.sample(ids + ['file/missing'], rng.randint(0, len(ids)))
        ops = [{'list': rng.choice(['selected', 'unselected']), 'index': rng.randint(-2, 8),
                'op': rng.choice(['activate', 'activate', 'up', 'down']), 'confirm': rng.random() < 0.5}
               for _ in range(rng.randint(0, 16))]
        out.append({'packs': packs, 'selected': selected, 'ops': ops})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=4000)
    parser.add_argument('--detail', type=int, default=3)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-pack-selection-parity'; build.mkdir(parents=True, exist_ok=True)
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
         *(PARITY / f'{n}.java' for n in ('ParityData', 'VanillaPackSelectionOracle'))], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaPackSelectionOracle',
         inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'pack_selection_fixture', inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust) == len(chosen)
    errors = [v['error'] for v in vanilla if 'error' in v]
    bad = [(c, v, r) for c, v, r in zip(chosen, vanilla, rust) if 'error' not in v and v != r]
    for e in sorted(set(errors))[:5]:
        print('ORACLE ERROR', e)
    for c, v, r in bad[:args.detail]:
        print('MISMATCH', json.dumps(c))
        step = next((i for i, (a, b) in enumerate(zip(v['steps'], r['steps'])) if a != b), None)
        if step is None:
            print(f'   committed: vanilla {v["committed"]} | horizon {r["committed"]}')
        else:
            print(f'   after action {step}: vanilla {v["steps"][step]}\n   horizon {r["steps"][step]}')
    steps = sum(len(v.get('steps', [])) for v in vanilla)
    print(f'{len(chosen) - len(bad) - len(errors)}/{len(chosen)} pack selections match ({steps} states); {len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('pack selection parity ok')


if __name__ == '__main__':
    main()
