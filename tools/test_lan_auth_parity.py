#!/usr/bin/env python3
"""LAN authentication failure: real vanilla login listener versus Horizon's network thread."""
import json
import re
from pathlib import Path

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .test_model_baking_parity import client_libraries
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from test_model_baking_parity import client_libraries


def main():
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'rust-lan-auth-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes, dependencies, oracle = oh.compile_oracle(
        build, 'VanillaLanAuthOracle', jars, client_libraries(), lock['minecraft']['java_major'])
    oh.java_run(build, classes, dependencies, 'VanillaLanAuthOracle', [], 'vanilla')
    oh.run_logged(['cargo', 'test', '-p', 'horizon-client', '--lib',
                   'net::tests::lan_login_continues_after_authentication_failure', '--', '--nocapture'],
                  build, 'rust', timeout=180)
    pattern = r'lan_auth=(LAN|OTHER):(true|false):(true|false)'
    expected = re.findall(pattern, (build / 'vanilla.log').read_text(encoding='utf-8'))
    actual = re.findall(pattern, (build / 'rust.log').read_text(encoding='utf-8'))
    if len(expected) != 2 or actual != expected:
        raise AssertionError(f'vanilla: {expected!r}; Horizon: {actual!r}')
    report.write_text(json.dumps({'passed': True, 'cases': expected,
        'tested_source_hashes': base.source_hashes([
            Path(__file__), oracle, lab.ROOT / 'crates/horizon-client/src/net.rs'])}, indent=2) + '\n',
        encoding='utf-8')
    print('LAN authentication parity: both server kinds match vanilla')


if __name__ == '__main__':
    main()
