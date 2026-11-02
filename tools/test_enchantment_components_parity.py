#!/usr/bin/env python3
"""Compare persistent enchantment levels and display names with vanilla 26.3."""
import os
from pathlib import Path
import subprocess

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def corpus():
    values = [
        '-2147483648', '-1', '0', '1', '10', '11', '254', '255', '256', '32767',
        '2147483647', '1b', '127b', '-128b', '255s', '256s', '1L', '4294967297L',
        '4294967296L', '9223372036854775807L', '1.9f', '255.9f', '256.1f', '-0.1f',
        '1.9d', '255.9d', '256d', '"255"', '{}', '[]',
    ]
    cases = [(value, f'{{sharpness:3,looting:{value}}}') for value in values]
    cases += [('only-invalid', '{looting:256}'), ('only-maximum', '{looting:255}')]
    keys = ['minecraft:enchantments', 'minecraft:stored_enchantments']
    for index, key in enumerate(keys):
        other = keys[1 - index]
        for label, levels in cases:
            yield (f'{index}:{label}\t{{"minecraft:damage":12,"{key}":'
                   f'{levels},"{other}":{{mending:1}}}}')


def main():
    build = lab.LOCAL / 'enchantment-components-parity'
    build.mkdir(parents=True, exist_ok=True)
    report_path = build / 'report.json'
    report_path.unlink(missing_ok=True)
    lock, jars, libraries = base.verify_artifacts()
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEnchantmentComponentsOracle.java'
    probe = lab.ROOT / 'crates/horizon-items/examples/enchantment_components_probe.rs'
    sources = [Path(__file__), Path(lab.__file__), Path(base.__file__), oracle,
               oracle.with_name('ParityData.java'), probe, lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-items/src/item_nbt.rs',
               lab.ROOT / 'crates/horizon-world/src/nbt.rs',
               lab.ROOT / 'crates/horizon-world/src/datafix/dynamic.rs',
               lab.ROOT / 'crates/horizon-client-core/src/lang.rs',
               lab.ROOT / 'crates/horizon-items/src/item_data.rs',
               lab.ROOT / 'crates/horizon-items/src/enchant.rs']
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    fixtures = list(corpus())
    inputs.write_text('\n'.join(fixtures) + '\n', encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name, **kwargs):
        with (build / f'{name}.log').open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT, **kwargs)

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
         '-sourcepath', lab.ROOT / 'mods/horizon/src/test/java', '-cp', str(jars['server']) + os.pathsep + deps,
         '-d', classes, oracle], 'compile-java')
    os.environ.setdefault('CARGO_TARGET_DIR', str(lab.LOCAL / 'parity-target'))
    env = dict(os.environ)
    run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-items',
         '--example', 'enchantment_components_probe'], 'compile-rust', env=env)
    binary = lab.cargo_target() / 'fast/examples' / ('enchantment_components_probe.exe' if os.name == 'nt' else 'enchantment_components_probe')
    run([binary, lab.RESOURCES, inputs, build / 'rust.txt'], 'rust')
    references = []
    for side, jar in jars.items():
        run(['java', '-Xmx1G', '-cp', os.pathsep.join(map(str, [classes, jar, *libraries])),
             'dev.horizonlab.parity.VanillaEnchantmentComponentsOracle', inputs, build / f'{side}.txt'], side)
        matched = base.compare(build / f'{side}.txt', build / 'rust.txt')
        references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'), 'matched_lines': matched})
        print(f'{side}: {matched} enchantment component lines match Rust', flush=True)
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(report_path, {'schema': 1, 'minecraft': lock['minecraft']['version'], 'fixtures': len(fixtures),
                                'references': references, 'sources_sha256': tested,
                                'scope': 'ItemStack.CODEC partial decoding of enchantments and stored_enchantments; '
                                         'numeric NBT conversions, invalid entries, unrelated components, '
                                         'and Enchantment.getFullname including untranslated high levels.'})
    print(f'Enchantment component parity passed: {report_path}')


if __name__ == '__main__':
    main()
