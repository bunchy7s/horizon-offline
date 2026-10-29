#!/usr/bin/env python3
"""Compare enchanting-table offers (costs, clues, applied enchantments) with vanilla.

Java loads the vanilla data pack's enchantment registry and runs
EnchantmentHelper as EnchantmentMenu does; Rust runs horizon_client::stations
on the prepared resources. Every enchantable item and the book are tested
across seeds and bookshelf counts.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus(items):
    rng = random.Random(0x454E4348414E54)
    seeds = [-2147483648, -1, 0, 1, 2147483647, *[rng.randrange(-2**31, 2**31) for _ in range(27)]]
    for item in items:
        for seed in seeds:
            for bookcases in [0, 1, 4, 8, 12, 15, 30]:
                yield f"{seed} {bookcases} {item}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    args = parser.parse_args()
    build = lab.LOCAL / 'agents/stations/enchanting-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    catalog = json.loads((resources / 'items.json').read_text())
    items = sorted(k for k, v in catalog.items() if 'minecraft:enchantable' in v)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEnchantingOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/enchanting_fixture.rs',
               *sorted((lab.ROOT / 'crates/horizon-client/src').glob('*.rs')),
               *sorted((lab.ROOT / 'crates/horizon-client-render/src').glob('*.rs'))]
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    fixtures = list(corpus(items))
    inputs.write_text(''.join(line + '\n' for line in fixtures), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none','-sourcepath',str(lab.ROOT/'mods/horizon/src/test/java'), '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary, 'enchanting_fixture',resources, inputs, build / 'rust.txt'], 'rust')
    references = []
    for side in ['server']:
        run(['java', '-Xmx1G', '-cp', os.pathsep.join([str(classes), str(jars[side]), deps]), 'dev.horizonlab.parity.VanillaEnchantingOracle', inputs, build / f'{side}.txt'], side)
        matched = base.compare(build / f'{side}.txt', build / 'rust.txt')
        references.append({'side': side, 'jar_sha256': lab.digest(jars[side], 'sha256'), 'matched_output_lines': matched})
        print(f'{side}: {matched:,} enchanting offer lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'fixtures': len(fixtures),
              'items': len(items), 'references': references, 'sources_sha256': tested,
              'scope': 'EnchantmentHelper.getEnchantmentCost / selectEnchantment on the data-driven enchantment registry and '
                       '#in_enchanting_table, EnchantmentMenu clue draws and book removal, for every enchantable item and the book.'}
    lab.write_json(build / 'report.json', report)
    print(f'Enchanting parity passed: {build / "report.json"}')


if __name__ == '__main__':
    main()
