#!/usr/bin/env python3
"""Compare actual vanilla and Horizon client boat ticks across ownership changes.

An empty stand-in vanilla client level isolates cached motion from block effects
and displacement, while AbstractBoat.tick and floatBoat remain real. Horizon
loads its resources headlessly and runs tick_boat plus the actual remote boat
ticker. Both report velocity and angular-velocity bits after each tick, including
controlled ticks and dismount/reboard transitions. Real ice/control/network
behavior is covered separately by lan_e2e --check boat_ice_reboard.
"""
import json
import os
from pathlib import Path
import subprocess
try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
    from .integration_env import run_game
except ImportError:
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking
    from integration_env import run_game


def cases():
    out = []
    for spin in (0.0, 0.125, -0.5, 2.0, -17.0):
        for velocity in ([0.0, 0.0, 0.0], [0.125, -0.0625, -0.25], [-1.0, 0.5, 0.75]):
            for owned in ([False], [True], [True, False], [True, False, True],
                          [False, True, True], [True, True, False, False, True]):
                out.append(dict(name=f'authority-{len(out)}', spin=spin, velocity=velocity, owned=owned))
    # A rider who leaves while the boat is moving, sits out long enough for the
    # empty boat to stop, and boards again (#6589): the re-boarded boat must
    # keep the velocity it had while empty, not the one the rider left with.
    for spin in (0.0, 0.125, -17.0):
        for velocity in ([1.0, 0.25, -0.5], [-0.125, 0.0, 0.0625]):
            for empty in (5, 25, 60):
                for back in (1, 5, 12):
                    owned = [True] * 8 + [False] + [False] * empty + [True] * back
                    out.append(dict(name=f'reboard-{len(out)}', spin=spin, velocity=velocity, owned=owned))
            # Empty from the first tick, then boarded: the boat has only ever
            # moved while nobody rode it.
            out.append(dict(name=f'reboard-{len(out)}', spin=spin, velocity=velocity,
                            owned=[False] * 40 + [True] * 8))
    return out


def main():
    lock, jars, _ = base.verify_artifacts()
    work = lab.LOCAL / 'boat-authority-parity'
    work.mkdir(parents=True, exist_ok=True)
    (work / 'report.json').unlink(missing_ok=True)
    inputs = cases()
    lab.write_json(work / 'inputs.json', inputs)
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    sources = lab.ROOT / 'mods/horizon/src/test/java'
    package = sources / 'dev/horizonlab/parity'
    dependencies = os.pathsep.join(map(str, [jars['client'], *baking.client_libraries()]))

    def run(command, name):
        with (work / f'{name}.log').open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=900)
        if result.returncode:
            raise RuntimeError(f'{name} failed: {(work / f"{name}.log").read_text()[-4000:]}')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn',
         '-sourcepath', sources, '-cp', dependencies, '-d', classes,
         *(package / f'{name}.java' for name in ('ParityData', 'VanillaVehicleContainerOracle', 'VanillaBoatAuthorityOracle'))],
        'compile-java')
    run(['java', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaBoatAuthorityOracle',
         work / 'inputs.json', work / 'vanilla.json'], 'vanilla')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], 'compile-rust')
    binary = lab.cargo_target() / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    with (work / 'rust.log').open('w') as log:
        run_game([str(binary), '--headless', '--resources', str(lab.RESOURCES)], cwd=lab.ROOT,
                 env=lab.game_env(HORIZON_BOAT_AUTHORITY_FIXTURE=work), stdout=log,
                 stderr=subprocess.STDOUT, check=True, timeout=180)
    native = json.loads((work / 'vanilla.json').read_text())
    rust = json.loads((work / 'rust.json').read_text())
    assert len(native) == len(rust) == len(inputs)
    errors = [(case['name'], n, r) for case, n, r in zip(inputs, native, rust) if n != r]
    if errors:
        raise AssertionError(f'{len(errors)}/{len(inputs)} boat authority cases differ: {errors[:2]}')
    ticks = sum(len(case['owned']) for case in inputs)
    lab.write_json(work / 'report.json', dict(minecraft=lock['minecraft']['version'], cases=len(inputs), ticks=ticks,
                                            output_sha256=lab.digest(work / 'rust.json', 'sha256'), scope=__doc__))
    print(f'boat authority: {len(inputs)}/{len(inputs)} cases and {ticks} tick states match vanilla bit-for-bit')


if __name__ == '__main__':
    main()
