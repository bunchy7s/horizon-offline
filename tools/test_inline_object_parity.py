#!/usr/bin/env python3
"""Compare inline atlas sprite shadows and bounds with vanilla's real font renderer."""
import itertools
import json

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking


def make_input(build):
    rows = []
    for world, shadow, base, custom, color in itertools.product(
            (False, True), (False, True), (0xFFFFFFFF, 0x80123456, 0x04000000),
            (None, 0, -65536, -2147418368), (None, '#4080c0')):
        row = dict(world=world, shadow=shadow, base=base, x=-3.25, y=10.5,
                   bold=True, italic=True)
        if custom is not None:
            row['shadow_color'] = custom
        if color is not None:
            row['color'] = color
        rows.append(row)
    path = build / 'inputs.jsonl'
    path.write_text(''.join(json.dumps(row) + '\n' for row in rows))
    return path, len(rows)


def main():
    lock, jars, _ = base.verify_artifacts()
    build = lab.LOCAL / 'inline-object-parity'
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    sources = [lab.ROOT / path for path in (
        'tools/test_inline_object_parity.py',
        'crates/horizon-client-render/src/hud.rs',
        'crates/horizon-client/examples/probe/inline_object_fixture.rs',
        'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaInlineObjectOracle.java')]
    tested = base.source_hashes(sources)
    inputs, count = make_input(build)
    classes, dependencies, _ = harness.compile_oracle(
        build, 'VanillaInlineObjectOracle', jars, baking.client_libraries(), lock['minecraft']['java_major'])
    harness.java_run(build, classes, dependencies, 'VanillaInlineObjectOracle',
                     [inputs, build / 'client.txt'], 'client')
    # Text geometry needs neither an audio device nor controller system libraries.
    harness.run_logged(['cargo', 'build', '--locked', '--no-default-features',
                        '-p', 'horizon-client-lib', '--example', 'client_probe'], build, 'compile-rust')
    harness.run_logged([lab.cargo_target() / 'debug/examples/client_probe', 'inline_object_fixture',
                        inputs, build / 'rust.txt'], build, 'rust')
    matched = base.compare(build / 'client.txt', build / 'rust.txt')
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], cases=count,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'), sources_sha256=tested,
        matched_output_lines=matched,
        scope='Actual Font.prepareText and AtlasGlyphProvider.Instance/PlainTextRenderable.render '
              'with a synthetic atlas region. Sprite bounds, shadow offset, tint, alpha, suppression '
              'and world depth; excludes atlas loading, player skin lookup and GPU sampling.'))
    print(f'{count} inline object cases match vanilla ({matched} lines): {report}')


if __name__ == '__main__':
    main()
