#!/usr/bin/env python3
"""Compare Horizon's environment attributes with the vanilla 26.3 attribute system.

The Java oracle builds vanilla's `EnvironmentAttributeSystem` from the data pack
(dimension attributes, one biome's attributes, the dimension timelines and the
weather layers) and prints every visual/gameplay attribute for a grid of
dimensions, world times, rain/thunder levels and biomes.
`examples/probe/environment_fixture` samples the same cases through Horizon's
`Environment::sample` + `apply_weather`; every attribute Horizon computes must
agree with vanilla.

  test_environment_render_parity.py [--tolerance EPS]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking


def cases(resources):
    biomes = sorted(p.stem for p in (resources / 'data/minecraft/worldgen/biome').glob('*.json'))
    out = []
    for dim in ['overworld', 'the_nether', 'the_end']:
        for ticks in list(range(0, 48000, 250)) + [23999, 24001, 1234567]:
            for rain, thunder in [(0, 0), (0.5, 0), (1, 0), (1, 1), (0.7, 0.3)] if dim == 'overworld' else [(0, 0)]:
                out.append(f'{dim} {ticks} {rain} {thunder} plains')
    own = {'the_nether': {'nether_wastes', 'soul_sand_valley', 'crimson_forest', 'warped_forest', 'basalt_deltas'},
           'the_end': {'the_end', 'small_end_islands', 'end_midlands', 'end_highlands', 'end_barrens'}}
    for dim in ['overworld', 'the_nether', 'the_end']:
        for biome in biomes:
            if dim in own and biome not in own[dim]:
                continue
            if dim == 'overworld' and any(biome in b for b in own.values()):
                continue
            for ticks in (0, 6000, 12500, 18000):
                for rain, thunder in [(0, 0), (1, 0.4)] if dim == 'overworld' else [(0, 0)]:
                    out.append(f'{dim} {ticks} {rain} {thunder} {biome}')
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tolerance', type=float, default=2e-5)
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-environment-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
    inputs = build / 'inputs.txt'; inputs.write_text('\n'.join(cases(resources)) + '\n')
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEnvironmentOracle.java'
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', lab.ROOT / 'mods/horizon/src/test/java',
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.horizonlab.parity.VanillaEnvironmentOracle', inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'environment_fixture', resources, inputs, build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust), (len(vanilla), len(rust))
    bad = {}
    uncovered = set()
    for v, r in zip(vanilla, rust):
        for key, expected in v.items():
            if key == 'case':
                continue
            if key not in r:
                uncovered.add(key); continue
            got = r[key]
            pairs = zip(expected, got) if isinstance(expected, list) else [(expected, got)]
            if any(abs(a - b) > args.tolerance * (1 + abs(a)) for a, b in pairs):
                bad.setdefault(key, []).append((v['case'], expected, got))
    print(f'{len(vanilla)} cases compared')
    for key, items in sorted(bad.items()):
        print(f'MISMATCH {key}: {len(items)} cases, e.g. {items[0]}')
    if uncovered:
        print('attributes Horizon does not model:', ', '.join(sorted(uncovered)))
    if bad:
        sys.exit(f'{len(bad)} attributes differ from vanilla')
    print('environment parity ok')


if __name__ == '__main__':
    main()
