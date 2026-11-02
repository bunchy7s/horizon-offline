#!/usr/bin/env python3
"""DataFixer differential test: worlds saved by older game versions, upgraded by
vanilla 26.3 and by Horizon, must give the same data.

For every sample world in datafix/worlds beside the shared vanilla data (made by
tools/datafix_worlds.py with the official server jars of 1.18.2 to 26.2, and
generated automatically when missing), two upgrades run on a copy:

  vanilla  VanillaDataFixOracle: FileFixerUpper, then DataFixers on level.dat, every
           chunk, entity chunk and POI chunk, player, saved data, advancements and
           statistics file, exactly as 26.3 does when it opens the world
  horizon  the `datafix_world` probe of horizon-world, which does the same with
           horizon_world::datafix

Then every file is compared: region files chunk by chunk, NBT files and JSON
files value by value (compound and object key order does not matter). The only
tolerated differences are listed in IGNORED (wall-clock time).

tools/parity.py runs it in PARITY_SHARDS parts, each upgrading its share of
the worlds (PARITY_SHARD=i/n). The vanilla outputs stay in .local/datafix/run
so that the oracle's results are reused while its inputs are unchanged.

  test_datafix_parity.py [--keep] [--skip-build] [VERSION...]
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import anvil_dump  # noqa: E402
import parity_shard  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local' / 'datafix'
WORLDS = (ROOT / '.local' / 'minecraft').resolve().parent / 'datafix' / 'worlds'
RUN = LOCAL / 'run'

PARITY_SHARDS = 6
# What the datafix_world probe can reach, checked by tools/parity.py: it reruns
# this harness only when these sources (or the harness, oracle and data) change.
PARITY_RUST_SCOPE = [
    'crates/horizon-world/src/datafix/', 'crates/horizon-world/src/nbt.rs', 'crates/horizon-world/src/region.rs',
    'crates/horizon-world/src/compression.rs', 'crates/horizon-core/src/position.rs', 'crates/horizon-core/src/position/',
    'crates/horizon-core/src/geometry/', 'crates/horizon-core/src/version.rs',
]

IGNORED = {
    r'^level\.dat\.Data\.LastPlayed$': 'wall-clock time of the upgrade',
}


def world_path(name):
    return (LOCAL / 'worlds-pre-1.17' if (LOCAL / 'worlds-pre-1.17' / name).is_dir() else WORLDS) / name


def java_classpath():
    import oracle_harness as harness
    import test_rust_parity as base
    import test_model_baking_parity as baking
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = LOCAL / 'build' / parity_shard.label()  # shards compile apart
    build.mkdir(parents=True, exist_ok=True)
    classes, dependencies, _ = harness.compile_oracle(build, 'VanillaDataFixOracle', jars, libraries,
                                                      lock['minecraft']['java_major'])
    return str(classes) + os.pathsep + dependencies


def build_rust():
    subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'horizon-world', '--example', 'world_probe'],
                   cwd=ROOT, check=True)
    meta = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'],
                                              cwd=ROOT))
    return Path(meta['target_directory']) / 'release' / 'examples' / 'world_probe'


def run_vanilla(classpath, world, out, target=None):
    # The oracle replaces OUT itself: left in place, an unchanged run's outputs can be reused.
    log = out.parent / (out.name + '.log')
    with log.open('w') as f:
        result = subprocess.run(['java', '-Xmx3g', '-Djava.awt.headless=true', '-cp', classpath,
                                 'dev.horizonlab.parity.VanillaDataFixOracle', str(world), str(out), *([str(target)] if target else [])],
                                stdout=f, stderr=subprocess.STDOUT)
    if result.returncode:
        # The traceback of a failed check names the command, not why the JVM stopped: show its exit and last output.
        how = f'exit {result.returncode}' + (' (killed by signal %d)' % -result.returncode if result.returncode < 0 else '')
        tail = '\n'.join(log.read_text(errors='replace').splitlines()[-20:])
        raise SystemExit(f'the vanilla oracle failed on {world.name} ({how}); the end of {log}:\n{tail}')


def run_horizon(binary, world, out, target=None):
    shutil.rmtree(out, ignore_errors=True)
    log = out.parent / (out.name + '.log')
    with log.open('w') as f:
        subprocess.run([str(binary), 'datafix_world', str(world), str(out), *([str(target)] if target else [])], check=True,
                       stdout=f, stderr=subprocess.STDOUT)


# ---------------------------------------------------------------- comparison

def norm(v):
    """Python values from anvil_dump, with 26.3 mixed-list wrappers removed."""
    if isinstance(v, dict):
        return {k: norm(x) for k, x in v.items()}
    if isinstance(v, list):
        return [norm(x[''] if isinstance(x, dict) and list(x) == [''] else x) for x in v]
    return v


def diff(a, b, path, out):
    if type(a) is not type(b):
        out.append((path, a, b))
    elif isinstance(a, dict):
        for k in sorted(set(a) | set(b), key=str):
            if k not in a:
                out.append((f'{path}.{k}', None, b[k]))
            elif k not in b:
                out.append((f'{path}.{k}', a[k], None))
            else:
                diff(a[k], b[k], f'{path}.{k}', out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append((f'{path}[len]', len(a), len(b)))
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f'{path}[{i}]', out)
    elif a != b:
        out.append((path, a, b))


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8'))


def files(world):
    out = set()
    for p in world.rglob('*'):
        if p.is_file() and p.name not in ('session.lock', 'datafix-source.json') and not p.name.endswith('_old'):
            out.add(p.relative_to(world))
    return out


def compare(vanilla, horizon):
    report = []
    a, b = files(vanilla), files(horizon)
    for rel in sorted(a | b, key=str):
        name = str(rel)
        if rel not in b:
            report.append((name, 'present', 'missing from Horizon'))
            continue
        if rel not in a:
            report.append((name, 'missing from vanilla', 'present'))
            continue
        va, ha = vanilla / rel, horizon / rel
        if name.endswith('.mca'):
            vc, hc = anvil_dump.region_chunks(str(va)), anvil_dump.region_chunks(str(ha))
            for key in sorted(set(vc) | set(hc)):
                label = f'{name}{list(key)}'
                if key not in hc:
                    report.append((label, 'present', 'missing from Horizon'))
                elif key not in vc:
                    report.append((label, 'missing from vanilla', 'present'))
                else:
                    out = []
                    diff(norm(vc[key]), norm(hc[key]), 'chunk', out)
                    report.extend((f'{p} @{label}', x, y) for p, x, y in out)
        elif name.endswith('.dat'):
            out = []
            diff(norm(anvil_dump.read_dat(str(va))), norm(anvil_dump.read_dat(str(ha))), name, out)
            report.extend(out)
        elif name.endswith('.json'):
            out = []
            diff(read_json(va), read_json(ha), name, out)
            report.extend(out)
        elif va.read_bytes() != ha.read_bytes():
            report.append((name, 'bytes differ', ''))
    return report


def pattern_of(path):
    return re.sub(r'\[\d+\]', '[]', path.split(' @')[0])


def summarize(report):
    groups = {}
    for path, a, b in report:
        groups.setdefault(pattern_of(path), []).append((path, a, b))
    failures = 0
    for pattern, items in groups.items():
        why = next((w for p, w in IGNORED.items() if re.search(p, pattern)), None)
        if why:
            print(f'  ignored  {pattern}  x{len(items)}  ({why})')
            continue
        failures += len(items)
        path, a, b = items[0]
        short = lambda v: (s := json.dumps(v, default=str))[:200] + ('...' if len(s) > 200 else '')
        print(f'  DIFF  {pattern}  x{len(items)}\n        e.g. {path}\n        vanilla {short(a)}\n        horizon {short(b)}')
    return failures


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('versions', nargs='*')
    p.add_argument('--keep', action='store_true')
    p.add_argument('--vanilla-only', action='store_true', help='only run the oracle')
    p.add_argument('--binary', help='a prebuilt horizon-world probe binary')
    p.add_argument('--band', type=int, metavar='TARGET', help='check the data fixes up to this DataVersion only (not the '
                   'file fixes), on the sample worlds saved before it, for developing the fixes of a range of versions')
    args = p.parse_args()
    import datafix_worlds
    import file_lock
    WORLDS.parent.mkdir(parents=True, exist_ok=True)
    with open(WORLDS.parent / 'generate.lock', 'w') as handle:
        file_lock.lock(handle)  # another worktree may be generating them
        missing = [v for v in [*datafix_worlds.VERSIONS, *datafix_worlds.VARIANTS] if not (datafix_worlds.worlds_dir(v) / v / 'level.dat').exists()]
        if missing:
            print('generating sample worlds:', ' '.join(missing), flush=True)
            subprocess.run([sys.executable, str(ROOT / 'tools' / 'datafix_worlds.py'), *missing], cwd=ROOT, check=True)
    worlds = sorted(p.name for d in (WORLDS, LOCAL / 'worlds-pre-1.17') if d.is_dir() for p in d.iterdir() if p.is_dir())
    band = args.band
    if band and not args.versions:  # worlds in the layout of before 1.16, saved before the target
        worlds = [w for w in worlds if json.loads((world_path(w) / 'datafix-source.json').read_text())['data_version'] < band]
    versions = parity_shard.shard(args.versions or worlds)
    classpath = java_classpath()
    binary = None if args.vanilla_only else Path(args.binary) if args.binary else build_rust()
    total = 0
    for version in versions:
        world = world_path(version)
        source = json.loads((world / 'datafix-source.json').read_text())
        print(f'{version} (DataVersion {source["data_version"]})')
        RUN.mkdir(parents=True, exist_ok=True)
        tag = f'{version}-band{band}' if band else version
        run_vanilla(classpath, world, RUN / f'{tag}-vanilla', band)
        if binary is None:
            continue
        run_horizon(binary, world, RUN / f'{tag}-horizon', band)
        failures = summarize(compare(RUN / f'{tag}-vanilla', RUN / f'{tag}-horizon'))
        print(f'  {version}: {"ok" if not failures else f"{failures} differences"}')
        total += failures
    if binary is not None:
        print('datafix parity', 'FAILED' if total else 'ok', f'({total} differences over {len(versions)} worlds)')
        if not total and not args.keep:
            for version in versions:  # vanilla outputs stay, for reuse
                shutil.rmtree(RUN / f'{version}-band{band}-horizon' if band else RUN / f'{version}-horizon', ignore_errors=True)
        sys.exit(1 if total else 0)


if __name__ == '__main__':
    main()
