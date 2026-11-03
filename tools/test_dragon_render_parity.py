#!/usr/bin/env python3
"""Compare death-ray counts, positions and colour gradients with vanilla 26.3.

The companion Rust regression checks frozen flight poses across render frames.
This harness captures real vanilla ray triangles, not a reimplementation.
"""
import json
import os

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base
    from . import test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base
    import test_model_baking_parity as baking


def main():
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'dragon-render-parity'
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    classes, dependencies, oracle = harness.compile_oracle(
        build, 'VanillaDragonRenderOracle', jars, baking.client_libraries(), lock['minecraft']['java_major'])
    harness.java_run(build, classes, dependencies, 'VanillaDragonRenderOracle', [build / 'client.json'], 'client')
    (build / 'rust.json').unlink(missing_ok=True)
    harness.run_logged(
        ['cargo', 'test', '--locked', '-p', 'horizon-client', '--lib',
         'game::dragon::tests::', '--', '--include-ignored'],
        build, 'rust', env=dict(os.environ, HORIZON_DRAGON_RENDER_OUTPUT=str(build / 'rust.json')))
    expected = json.loads((build / 'client.json').read_text())
    actual = json.loads((build / 'rust.json').read_text())
    assert len(actual) == len(expected) == 8
    vertices = 0
    for case, (left, right) in enumerate(zip(expected, actual)):
        assert len(left) == len(right), (case, len(left), len(right))
        for index, (a, b) in enumerate(zip(left, right)):
            assert a[3] == b[3], (case, index, 'colour', a[3], b[3])
            # JOML and glam accumulate different f32 rounding across up to
            # 60 rotations. Allow two millimetres, while counts/RGBA stay exact.
            assert max(abs(x - y) for x, y in zip(a[:3], b[:3])) < .002, (case, index, a, b)
        vertices += len(left)
    lab.write_json(report, dict(schema=1, minecraft=lock['minecraft']['version'], cases=8, vertices=vertices,
                               scope='Death-ray triangle positions, counts and RGBA gradients; no framebuffer comparison.',
                               sources_sha256=base.source_hashes([
                                   oracle, __file__, lab.ROOT / 'crates/horizon-client/src/game/dragon.rs',
                                   lab.ROOT / 'crates/horizon-client-render/src/gfx.rs',
                                   lab.ROOT / 'crates/horizon-client-render/src/gfx/oit_features.rs',
                                   lab.ROOT / 'crates/horizon-client-render/src/entity.wgsl'])))
    print(f'dragon render: {vertices} vertices match vanilla across 8 death times')


if __name__ == '__main__':
    main()
