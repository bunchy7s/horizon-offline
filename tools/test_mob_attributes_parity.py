#!/usr/bin/env python3
"""Compare mob default attributes, dimensions (adult and baby) and the entity inventory with vanilla."""
import argparse
import math
import os
from pathlib import Path
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

# Entity types that are not mobs (or are players): Horizon models them outside the mob table.
# Horizon models these differently on purpose: slimes/magma/sulfur cubes are created already sized (size 1,
# vanilla applies setSize at spawn), and the phantom keeps its move control speed in attrs.speed.
KNOWN = {('minecraft:slime', k) for k in ('max_health', 'movement_speed', 'attack_damage')} | \
    {('minecraft:magma_cube', k) for k in ('max_health', 'movement_speed', 'attack_damage')} | \
    {('minecraft:sulfur_cube', k) for k in ('max_health', 'movement_speed')} | {('minecraft:phantom', 'movement_speed')}


def parse(path):
    rows = {}
    for line in path.read_text(encoding='utf-8').splitlines():
        id, *fields = line.split('\t')
        row = {'raw': fields}
        for f in fields:
            key, _, rest = f.partition(' ')
            if key == 'attrs':
                row['attrs'] = {k: float(v) for k, v in (p.split('=') for p in rest.split())}
            elif key in ('dim', 'baby'):
                try:
                    row[key] = [float(x) for x in rest.split()]
                except ValueError:
                    pass
            elif key == 'scale':
                row['scale'] = float(rest)
        rows[id] = row
    return rows


def close(a, b):
    return a == b or abs(a - b) <= 1e-6 * max(1.0, abs(a), abs(b)) or (math.isnan(a) and math.isnan(b))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-mob-attributes-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaMobAttributesOracle.java'
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(cmd, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            subprocess.run(list(map(str, cmd)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]), 'dev.horizonlab.parity.VanillaMobAttributesOracle', build / 'vanilla.txt'], 'java')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'fast/examples/client_probe'
    resources = lab.RESOURCES
    run([binary, 'mob_attributes_fixture', resources, build / 'rust.txt'], 'rust')
    vanilla, rust = parse(build / 'vanilla.txt'), parse(build / 'rust.txt')
    problems = []
    living = [i for i, r in vanilla.items() if 'attrs' in r]
    missing = [i for i in vanilla if i not in rust]
    (build / 'missing.txt').write_text('\n'.join(missing) + '\n')
    failed = [i for i, r in vanilla.items() if 'raw' in r and r['raw'][:1] == ['FAIL']]
    for id, v in vanilla.items():
        r = rust.get(id)
        if r is None:
            continue
        if 'dim' in v and 'dim' in r and not all(close(a, b) for a, b in zip(v['dim'], r['dim'])):
            problems.append(f'{id}: dim vanilla={v["dim"]} rust={r["dim"]}')
        if 'baby' in v and not all(close(a, b) for a, b in zip(v['baby'], r['baby'])):
            problems.append(f'{id}: baby vanilla={v["baby"]} rust={r["baby"]}')
        for k, x in v.get('attrs', {}).items():
            if (id, k) not in KNOWN and k in r['attrs'] and not close(x, r['attrs'][k]):
                problems.append(f'{id}: {k} vanilla={x} rust={r["attrs"][k]}')
    print(f'vanilla types {len(vanilla)} (living {len(living)}), Horizon mob types {len(rust)}, not in mob table {len(missing)}: {" ".join(m.split(":")[1] for m in missing)}')
    print('oracle could not instantiate:', ' '.join(f.split(':')[1] for f in failed))
    if problems:
        print('\n'.join(problems))
        raise SystemExit(f'{len(problems)} divergences')
    print('Mob attribute/dimension parity passed')


if __name__ == '__main__':
    main()
