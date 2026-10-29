#!/usr/bin/env python3
"""Compare the native client's actual resident-scene collision adapter with vanilla."""
import json
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_native_client as client
except ImportError:
    import lab
    import test_rust_parity as base
    import test_native_client as client


def bits(value):
    return struct.pack('>d', value).hex()


def corpus(radius):
    def row(x, y, z, delta, step=0.6, ground=True, width=0.6000000238418579, height=1.7999999523162842):
        return {'min': list(map(bits, [x-width/2, y, z-width/2])),
                'max': list(map(bits, [x+width/2, y+height, z+width/2])),
                'delta': list(map(bits, delta)), 'step': struct.pack('>f', step).hex(), 'grounded': ground}
    # All gallery placements, all approach axes, ledges, ceilings and embedded starts.
    # A four-block-wide ghast in midair: land, walk, jump and leave its edges.
    for x in [10.3, 12.0, 13.7, 14.4]:
        for y in [84.0, 84.5]:
            for delta in [(0, -0.08, 0), (0, -1, 0), (0.2, -0.08, 0), (-0.2, -0.08, 0), (0, 0.42, 0)]:
                fixture = row(x, y, 12.0, delta, ground=y == 84.0)
                fixture['entities'] = [list(map(bits, [10, 80, 10, 14, 84, 14]))]
                yield fixture
    for index in range(16):
        x, z = index % 4 * 4 - 7, index // 4 * 4 + 14
        for dx, dz in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)]:
            for y in [3.9999999, 4, 4.5, 5, 5.5, 6]:
                for step in [0, 0.5, 0.6, 1, 1.5]:
                    yield row(x+0.5-dx, y, z+0.5-dz, (dx*1.5, -0.08, dz*1.5), step)
        for dy in [-2, -0.5, -1e-8, 0, 0.5, 2]:
            yield row(x+0.5, 4.25, z+0.5, (0, dy, 0))
    # Resident/unloaded boundaries, negative chunks, below/above the section and pyramid.
    edges = [-16*radius-0.5, -16*radius, -16, -10, -1, 0, 1, 10, 15, 16, 16*(radius+1)]
    for x in edges:
        for z in edges:
            for y in [-17, -1, 0, 3, 4, 10, 11, 16, 32]:
                yield row(x, y, z, (1, -0.2, 0.5), ground=False)
    for delta in [(0, 0, 0), (-0.0, -0.0, -0.0), (1e-200, 0, 0), (1e-8, -1e-8, 0), (12, -9, -8)]:
        for ground in [True, False]:
            yield row(-11, 4, 0, delta, ground=ground)
    rng = random.Random(0x5343454e45 + radius)
    for _ in range(3000):
        yield row(rng.uniform(-20, 34), rng.uniform(-1, 14), rng.uniform(-20, 34),
                  [rng.uniform(-3, 3), rng.uniform(-2, 1), rng.uniform(-3, 3)],
                  step=rng.choice([0, 0.5, 0.6, 1, 1.5]), ground=rng.choice([True, False]),
                  width=rng.choice([0.2, 0.6, 1.2]), height=rng.choice([0.4, 1.5, 1.8]))


def main():
    lock, jars, libraries = base.verify_artifacts()
    root = lab.LOCAL/'rust-native-scene-collision'; root.mkdir(exist_ok=True)
    report = root/'report.json'; report.unlink(missing_ok=True)
    resources = lab.LOCAL/'horizon-client'/lock['minecraft']['version']
    resources_before = client.resource_fingerprint(resources)
    oracle = lab.ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaSceneCollisionOracle.java'
    helpers = [oracle.with_name(name+'.java') for name in [
        'VanillaRegistryCollisionScanOracle', 'VanillaHeightmapOracle', 'VanillaCollisionOracle',
        'VanillaBlockStateCodecOracle', 'VanillaSuffocationOracle', 'VanillaFaceSupportOracle']]
    sources = [Path(__file__), lab.ROOT/'tools/test_native_client.py', lab.ROOT/'tools/test_rust_parity.py',
               lab.ROOT/'tools/lab.py', lab.ROOT/'tools/prepare_native_client.py', oracle, *helpers,
               lab.ROOT/'Cargo.toml', lab.ROOT/'Cargo.lock',
               lab.ROOT/'crates/horizon-client/examples/probe/scene_collision_probe.rs']
    for crate in ['horizon-core', 'horizon-world', 'horizon-assets', 'horizon-client', 'horizon-client-render']:
        sources += [lab.ROOT/'crates'/crate/'Cargo.toml', *sorted((lab.ROOT/'crates'/crate/'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    def run(command, name):
        with (root/(name+'.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600,
                           stdout=log, stderr=subprocess.STDOUT)
    classes = root/'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp',
         str(jars['server'])+os.pathsep+dependencies, '-d', classes, oracle, *helpers], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-client', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    executable = Path(metadata['target_directory'])/'debug/examples'/('client_probe.exe' if os.name=='nt' else 'client_probe')
    cases = []
    for radius in [1, 2]:
        build = root/str(radius); build.mkdir(exist_ok=True)
        fixtures = list(corpus(radius))
        (build/'inputs.txt').write_text(''.join(json.dumps(f, separators=(',', ':'))+'\n' for f in fixtures))
        input_hash = lab.digest(build/'inputs.txt', 'sha256')
        run([executable, 'scene_collision_probe',resources, build, radius], f'rust-{radius}')
        scene_hash = lab.digest(build/'scene.json', 'sha256')
        references = []
        for side, jar in jars.items():
            run(['java', '-Xmx768M', '-cp', os.pathsep.join([str(classes), str(jar), dependencies]),
                 'dev.horizonlab.parity.VanillaSceneCollisionOracle', side, build, resources], f'{side}-{radius}')
            matched = base.compare(build/(side+'.txt'), build/'rust.txt')
            if matched != len(fixtures): raise AssertionError('Incomplete collision output')
            references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'),
                               'matched_output_lines': matched, 'output_sha256': lab.digest(build/(side+'.txt'), 'sha256')})
            print(f'radius {radius}, {side}: {matched:,} scene movement/occupancy results match', flush=True)
        if input_hash != lab.digest(build/'inputs.txt', 'sha256') or scene_hash != lab.digest(build/'scene.json', 'sha256'):
            raise AssertionError('Scene fixtures changed during validation')
        cases.append({'radius': radius, 'fixtures': len(fixtures), 'input_sha256': input_hash, 'scene_sha256': scene_hash,
                      'rust_output_sha256': lab.digest(build/'rust.txt', 'sha256'), 'references': references})
    base.ensure_sources_unchanged(sources, tested)
    if resources_before != client.resource_fingerprint(resources): raise AssertionError('Resources changed during validation')
    lab.write_json(report, {'schema': 1, 'minecraft': lock['minecraft']['version'],
                           'manifest_sha1': lock['minecraft']['manifest_sha1'], 'platform': platform.platform(),
                           'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                           'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
                           'sources_sha256': tested, 'resources': resources_before, 'cases': cases,
                           'scope': 'Actual client Assets loader and TestWorld resident chunks feed registered collision shapes, block scanning, Entity.collide step resolution and noCollision. Native independently constructs both scene sizes and all 16 gallery states/positions; compares exact movement bits and occupancy before/after against both pinned JARs. Includes partial blocks, pyramid ledges, negative/unloaded chunk boundaries, initial overlaps, vertical/horizontal/diagonal/tiny/zero movement and step heights. Position collision contexts, absent other entities/block entities and no border are explicit scene conditions. This validates the collision adapter, not player travel, gravity, tick scheduling, fluid/entity gameplay, rendering pixels or a complete playable game.'})
    print(f'Scoped native scene collision parity passed: {report}', flush=True)


if __name__ == '__main__':
    main()
