#!/usr/bin/env python3
"""Compare the particle bursts entity events show on the client (villagers'
hearts, anger and happiness, love hearts, taming and trusting hearts and smoke,
the allay's and dolphin's) with the real handleEntityEvent of each entity:
particle, count, positions about the body, motion and every random draw."""
import json
import platform
import random
import struct
import subprocess

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

EVENTS = {
    'villager': [12, 13, 14, 42],
    'cow': [18], 'wolf': [6, 7, 18], 'cat': [6, 7, 18], 'parrot': [6, 7], 'nautilus': [6, 7, 18],
    'horse': [6, 7, 18], 'llama': [6, 7, 18], 'camel': [6, 7, 18],
    'ocelot': [40, 41, 18], 'dolphin': [38], 'allay': [18],
}
# Dyadic sizes and positions: serde_json's decimal parsing may round others a unit off.
SIZES = [(0.6, 1.95), (0.9, 1.4), (0.3, 0.85), (1.3964844, 1.6), (0.35, 0.6), (0.875, 0.9375)]


def cases(rng):
    for entity, events in EVENTS.items():
        for event in events:
            for _ in range(40):
                w, h = rng.choice(SIZES)
                feet = [rng.randrange(-4096, 4096) / 64 for _ in range(3)]
                yield dict(entity=entity, event=event, feet=feet, width=w, height=h,
                           seed=rng.randrange(-2**63, 2**63))


def make_cases(build):
    path = build / 'cases.jsonl'
    count = 0
    with path.open('w') as out:
        for case in cases(random.Random(0x454e5450)):
            out.write(json.dumps(case, separators=(',', ':')) + '\n')
            count += 1
    return path, count


def close(a, b):
    """Lines equal but for doubles within 1e-12 relative error. The motion is
    Gaussian, and MarsagliaPolarGaussian calls Math.log, which HotSpot's x86-64
    intrinsic rounds a unit apart from the platform libm horizon_core::jmath
    uses for a small fraction of arguments."""
    ta, tb = a.split(), b.split()
    if len(ta) != len(tb):
        return False
    for x, y in zip(ta, tb):
        if x == y:
            continue
        if len(x) != 16 or len(y) != 16:
            return False
        fx, fy = (struct.unpack('>d', bytes.fromhex(t))[0] for t in (x, y))
        if not abs(fx - fy) <= 1e-12 * max(1.0, abs(fx)):
            return False
    return True


def main():
    lock, jars, _ = base.verify_artifacts()
    name, oracle, probe = 'rust-entity-particles-parity', 'VanillaEntityParticlesOracle', 'entity_particles_probe'
    build = lab.LOCAL / name
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    sources = [lab.ROOT / p for p in [
        'tools/test_entity_particles_parity.py', 'tools/oracle_harness.py', 'tools/test_rust_parity.py', 'tools/lab.py',
        f'mods/horizon/src/test/java/dev/horizonlab/parity/{oracle}.java', 'Cargo.toml', 'Cargo.lock',
        'crates/horizon-client/Cargo.toml', 'crates/horizon-client-render/src/particles.rs',
        f'crates/horizon-client/examples/probe/{probe}.rs', 'crates/horizon-core/src/random.rs']]
    tested = base.source_hashes(sources)
    cases_path, units = make_cases(build)
    classes, dependencies, _ = harness.compile_oracle(build, oracle, jars, baking.client_libraries(),
                                                      lock['minecraft']['java_major'])
    command = harness.build_probe(build, probe)
    harness.run_logged([*command, cases_path, build / 'rust.txt'], build, 'rust')
    harness.java_run(build, classes, dependencies, oracle, [cases_path, build / 'client.txt'], 'client')
    vanilla = (build / 'client.txt').read_text().splitlines()
    rust = (build / 'rust.txt').read_text().splitlines()
    assert len(vanilla) == len(rust), f'{len(vanilla)} vanilla lines, {len(rust)} Rust lines'
    tolerated = 0
    for line, (a, b) in enumerate(zip(vanilla, rust), 1):
        if a != b:
            assert close(a, b), f'Parity mismatch at output line {line}: vanilla={a!r}, Rust={b!r}'
            tolerated += 1
    particles = sum(line.startswith('+ ') for line in vanilla)
    assert particles > units, 'too few particles to establish parity'
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), units=units,
        sources_sha256=tested, input_sha256=lab.digest(cases_path, 'sha256'),
        client_jar_sha256=lab.digest(jars['client'], 'sha256'), matched_output_lines=len(vanilla),
        libm_tolerated_lines=tolerated, output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        scope=('The real client handleEntityEvent of Villager (12, 13, 14, 42), Animal (18, love hearts), TamableAnimal '
               'and AbstractHorse (6, 7, taming), Ocelot (40, 41, trusting), Dolphin (38) and Allay (18), on entities '
               'allocated with a stand-in level, a position, dimensions and a seeded random: the particle type, '
               'count, positions, motion and draws on the entity random. Which server code fires each event, and '
               'the particles themselves once spawned, are covered elsewhere.')))
    print(f'{units:,} entity events, {particles:,} particles match the pinned client ({tolerated} lines within the '
          f'Math.log tolerance): {report}', flush=True)


if __name__ == '__main__':
    main()
