#!/usr/bin/env python3
"""Shared runner for the audio differential harnesses.

Each harness writes an input file, runs a Java oracle against the pinned
client (the real SoundManager / SoundEngine / MusicManager / ambient handlers;
OpenAL runs on OpenAL Soft's "No Output" device so channel parameters can be
read back from real AL sources) and a Rust example replaying the same inputs
through Horizon's audio code, then requires byte-identical output.
"""
import json
import os
from pathlib import Path
import platform
import subprocess

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

RESOURCES = lab.RESOURCES
COMMON_SOURCES = [
    lab.ROOT / 'tools/audio_harness.py',
    lab.ROOT / 'tools/test_rust_parity.py',
    lab.ROOT / 'tools/lab.py',
    lab.ROOT / 'Cargo.toml',
    lab.ROOT / 'Cargo.lock',
    lab.ROOT / 'crates/horizon-client/Cargo.toml',
    lab.ROOT / 'crates/horizon-client-render/Cargo.toml',
    lab.ROOT / 'crates/horizon-client-render/src/sound_events.rs',
    lab.ROOT / 'crates/horizon-client-render/src/sound_engine.rs',
]


def require_sounds():
    if not (RESOURCES / 'assets/minecraft/sounds.json').is_file():
        raise SystemExit('Sound assets missing: run cargo run -p horizon-prepare -- sounds first')


def run_harness(name, oracle, example, inputs_text, extra_sources=(), java_args=(), rust_args=(), scope='', timeout=1800):
    """Run oracle and the client probe `example` on inputs; returns the number of matched lines."""
    require_sounds()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / f'rust-{name}'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle_path = lab.ROOT / f'mods/horizon/src/test/java/dev/horizonlab/parity/{oracle}.java'
    example_path = lab.ROOT / f'crates/horizon-client/examples/probe/{example}.rs'
    sources = [*COMMON_SOURCES, oracle_path, example_path, *[Path(s) for s in extra_sources]]
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    inputs.write_text(inputs_text)
    input_hash = lab.digest(inputs, 'sha256')

    def run(command, log, env=None):
        with (build / (log + ".log")).open("w") as out:
            try:
                subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=timeout, stdout=out, stderr=subprocess.STDOUT, env=env)
            except subprocess.CalledProcessError:
                raise SystemExit(f"{log} failed: see {build / (log + '.log')}") from None

    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', lab.ROOT / 'mods/horizon/src/test/java',
         '-cp', dependencies, '-d', classes, oracle_path], 'compile-java')
    run(['cargo', 'build', '--locked', '-p', 'horizon-client', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('client_probe.exe' if os.name == 'nt' else 'client_probe')
    run([binary, example, *rust_args, inputs, build / 'rust.txt'], 'rust')
    env = dict(os.environ, ALSOFT_DRIVERS='null')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, f'dev.horizonlab.parity.{oracle}', *java_args, inputs, build / 'client.txt'],
        'client', env=env)
    matched = base.compare(build / 'client.txt', build / 'rust.txt')
    base.ensure_sources_unchanged(sources, tested)
    if input_hash != lab.digest(inputs, 'sha256'):
        raise AssertionError('Input fixtures changed')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'], platform=platform.platform(),
        rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        sources_sha256=tested, input_sha256=input_hash, client_jar_sha256=lab.digest(jars['client'], 'sha256'),
        sounds_json_sha256=lab.digest(RESOURCES / 'assets/minecraft/sounds.json', 'sha256'),
        matched_output_lines=matched, output_sha256=lab.digest(build / 'rust.txt', 'sha256'), scope=scope))
    return matched, report


def bits(value):
    import struct
    return struct.pack('>f', value).hex()
