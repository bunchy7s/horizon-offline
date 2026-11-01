#!/usr/bin/env python3
"""Compare thrown tridents' block step with vanilla.

`ThrownTrident.tick` delegates to `AbstractArrow.tick`, whose block phase is a
zero-width `level().clip(new ClipContext(position(), position().add(movement),
ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, this))`, followed by
`AbstractArrow.onHitBlock` (0.05 back along `Math.signum` per axis, velocity
zero, in ground, `shakeTime = 7`). A trident whose centre ray passes next to a
block must fly on; a sweep of its 0.5 x 0.5 box instead stops it in the air
beside the block.

The vanilla side is `VanillaArrowOracle`: the block phase it ticks is
`AbstractArrow`'s, shared verbatim with `ThrownTrident` (the trident only adds
the `inGroundTime > 4`/Loyalty pre-step before `super.tick()`), so we do not
duplicate the oracle for the trident entity. Horizon's side is
`examples/probe/trident_probe.rs`, which calls `arrows::clip_block_stop`, the
same function the game's trident tick calls.

Fixtures: a stone block at x 12..13, y 20..21, z 10..11 in the probe's test
scene, and a projectile starting at (10.5, 20.52, z) with a 2.5-block first
movement, so the centre ray is inside the block's z span for z in [10, 11).
The z offsets are 10.55 and 10.85 (hit), 11.05, 11.15 and 11.30 (0.05, 0.15
and 0.30 outside: miss), two diagonal passes near the block's corner and one
three-tick flight that has to keep falling after a miss.
"""
import json
import os
import platform
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'tools'))

import lab  # noqa: E402
import oracle_harness  # noqa: E402
import test_model_baking_parity as baking  # noqa: E402
import test_rust_parity as base  # noqa: E402


def bits(value):
    return struct.pack('>d', value).hex()


def vec(values):
    return [bits(v) for v in values]


def row(position, velocity, ops, seed):
    return {'seed': str(seed), 'position': vec(position), 'velocity': vec(velocity),
            'crit': False, 'no_physics': False, 'intangible': False, 'ops': ops}


def fixtures():
    wall = {'mode': 'block', 'name': 'minecraft:stone', 'position': [12, 20, 10]}

    def ticks(n):
        return [{'mode': 'tick'} for _ in range(n)]

    cases = []
    for i, z in enumerate((10.55, 10.85, 11.05, 11.15, 11.30)):
        cases.append((f'z={z:5.2f}', row((10.5, 20.52, z), (2.5, 0.0, 0.0),
                                         [{'mode': 'level_seed', 'value': str(1000 + i)}, wall] + ticks(1), 1000 + i)))
    # Diagonal passes near the block's corner: the ray reaches x 12 after 0.6 of
    # its 2.5-block move, so it is inside z [10, 11) only for z < 10.4.
    for i, z in enumerate((10.35, 10.55)):
        cases.append((f'corner z={z:5.2f}', row((10.5, 20.52, z), (2.5, 0.0, 1.0),
                                                [{'mode': 'level_seed', 'value': str(2000 + i)}, wall] + ticks(1), 2000 + i)))
    # A miss keeps flying, with the air drag and gravity after each move.
    cases.append(('miss, 3 ticks', row((10.5, 20.52, 11.05), (2.5, 0.0, 0.0),
                                       [{'mode': 'level_seed', 'value': '3000'}, wall] + ticks(3), 3000)))
    return cases


def oracle_lines(build):
    """One final `arrow` line per fixture from VanillaArrowOracle's output."""
    cases, current = [], []
    for line in (build / 'client.txt').read_text().splitlines():
        if line == 'end':
            cases.append(current)
            current = []
        elif line.startswith('arrow '):
            current.append(line.split())
    return cases


def probe_lines(build):
    out = []
    for line in (build / 'rust-client.txt').read_text().splitlines():
        fields = line.split()
        if len(fields) != 5 or fields[0] != 'trident':
            raise AssertionError(f'probe line: {line!r}')
        out.append(fields)
    return out


def main():
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'trident-parity'
    build.mkdir(parents=True, exist_ok=True)
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    oracle = ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaArrowOracle.java'
    sources = [Path(__file__), ROOT / 'tools/oracle_harness.py', ROOT / 'tools/lab.py',
               ROOT / 'tools/test_rust_parity.py', ROOT / 'tools/test_model_baking_parity.py', oracle,
               ROOT / 'Cargo.toml', ROOT / 'Cargo.lock',
               ROOT / 'crates/horizon-client/examples/probe/trident_probe.rs',
               ROOT / 'crates/horizon-client/examples/probe/main.rs',
               ROOT / 'crates/horizon-client/src/arrows.rs',
               ROOT / 'crates/horizon-client/src/game/trident.rs']
    tested = base.source_hashes(sources)
    cases = fixtures()
    (build / 'inputs.txt').write_text(''.join(json.dumps(row, separators=(',', ':')) + '\n'
                                              for _, row in cases))
    input_hash = lab.digest(build / 'inputs.txt', 'sha256')
    classes, dependencies, _ = oracle_harness.compile_oracle(
        build, 'VanillaArrowOracle', jars, libraries, lock['minecraft']['java_major'])
    probe = oracle_harness.build_probe(build, 'trident_probe')
    oracle_harness.run_logged([*probe, resources, build, '--describe'], build, 'describe')
    oracle_harness.run_logged([*probe, resources, build, 'client'], build, 'rust')
    oracle_harness.java_run(build, classes, dependencies, 'VanillaArrowOracle',
                            ['client', build, resources], 'client')
    vanilla, rust = oracle_lines(build), probe_lines(build)
    if not (len(vanilla) == len(rust) == len(cases)):
        raise AssertionError(f'case count: {len(cases)} fixtures, {len(vanilla)} vanilla, {len(rust)} rust')
    failures = []
    for (name, _), ref, got in zip(cases, vanilla, rust):
        state = ref[-1]  # the arrow line printed after the fixture's last op
        expected = (state[1], state[2], state[5], state[8])
        actual = (got[1], got[2], got[3], got[4])
        if expected != actual:
            failures.append((name, expected, actual))
    if failures:
        report = '\n'.join(f'  {name}: vanilla {exp} != rust {act}' for name, exp, act in failures)
        raise AssertionError(f'{len(failures)} of {len(cases)} trident block steps differ:\n{report}')
    base.ensure_sources_unchanged(sources, tested)
    if input_hash != lab.digest(build / 'inputs.txt', 'sha256'):
        raise AssertionError('Trident fixtures changed during verification')
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'],
              'manifest_sha1': lock['minecraft']['manifest_sha1'], 'platform': platform.platform(),
              'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
              'fixtures': len(cases), 'matched_output_lines': len(cases),
              'sources_sha256': tested, 'input_sha256': input_hash,
              'client_jar_sha256': lab.digest(jars['client'], 'sha256'), 'scope': __doc__}
    lab.write_json(build / 'report.json', report)
    print(f'{len(cases)} trident block steps match the pinned client: {build / "report.json"}', flush=True)


if __name__ == '__main__':
    main()
