#!/usr/bin/env python3
"""Login's resolved server host/port versus vanilla ServerNameResolver (no live DNS)."""
import json
import os
import subprocess

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-server-address-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaServerAddressOracle.java'
    _, game = lab.config()
    manifest = json.loads((game / 'manifest.json').read_text(encoding='utf-8'))
    for artifact in lab.client_library_artifacts(manifest):
        library = game / 'libraries/client' / artifact['path']
        if lab.digest(library, 'sha1') != artifact['sha1']:
            raise ValueError(f'Client dependency checksum changed: {library}')
        libraries.append(library)
    classpath = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        result = subprocess.run(list(map(str, command)), cwd=lab.ROOT,
                                capture_output=True, text=True)
        (build / (name + '.log')).write_text(result.stdout + result.stderr, encoding='utf-8')
        result.check_returncode()
        return [line for line in result.stdout.splitlines() if line.startswith('server_address=')]

    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none',
         '-cp', classpath, '-d', classes, oracle], 'compile-java')
    expected = run(['java', '-cp', str(classes) + os.pathsep + classpath,
                    'dev.horizonlab.parity.VanillaServerAddressOracle'], 'vanilla')
    actual = run(['cargo', 'test', '-p', 'horizon-protocol', '--lib',
                  'resolver::tests::login_handshake_address_matches_vanilla', '--', '--nocapture'], 'rust')
    if len(expected) != 3 or actual != expected:
        raise AssertionError(f'vanilla: {expected!r}; Horizon: {actual!r}')
    report.write_text(json.dumps({'passed': True, 'cases': expected}, indent=2) + '\n', encoding='utf-8')
    print(f'Server address parity: {len(expected)} cases passed')


if __name__ == '__main__':
    main()
