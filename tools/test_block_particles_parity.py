#!/usr/bin/env python3
"""Compare terrain-particle placement with real vanilla ClientLevel effect methods."""
import itertools
import json
try:
    from . import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base, test_breaking_model_parity as breaking
except ImportError:
    import lab, oracle_harness as harness, test_model_baking_parity as baking, test_rust_parity as base, test_breaking_model_parity as breaking


def main():
    lock, jars, _ = base.verify_artifacts()
    resources = lab.RESOURCES
    build = lab.LOCAL / "block-particles-parity"
    build.mkdir(exist_ok=True)
    paths = [lab.ROOT / p for p in ["tools/test_block_particles_parity.py", "tools/test_breaking_model_parity.py", "tools/oracle_harness.py", "tools/lab.py", "crates/horizon-client-render/src/particles.rs", "crates/horizon-client/src/game.rs", "crates/horizon-items/src/blocks.rs", "crates/horizon-client-render/src/mesher.rs", "crates/horizon-client/examples/probe/block_particles_fixture.rs", "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaBlockParticlesOracle.java", "crates/horizon-core/src/random.rs", "Cargo.lock", "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaGameplayFactsExporter.java"]]
    tested = base.source_hashes(paths)
    rows = [dict(case=i, seed=(-1 if i % 2 else 1) * i * 987654321, **row) for i, row in enumerate(breaking.cases(resources))]
    rows[0]["seed"] = -(2**63)
    rows[-1]["seed"] = 2**63 - 1
    inputs = build / "inputs.jsonl"
    inputs.write_text("".join(json.dumps(row)+"\n" for row in rows))
    classes, dependencies, _ = harness.compile_oracle(build, "VanillaBlockParticlesOracle", jars, baking.client_libraries(), lock["minecraft"]["java_major"])
    command = harness.build_probe(build, "block_particles_fixture")
    harness.run_logged([*command, resources, inputs, build / "rust.jsonl"], build, "rust")
    harness.java_run(build, classes, dependencies, "VanillaBlockParticlesOracle", [inputs, build / "client.jsonl"], "client")
    count = hits = burst = 0
    max_error = 0.0
    with (build / "client.jsonl").open() as native, (build / "rust.jsonl").open() as rust:
        for v, r in itertools.zip_longest(native, rust):
            assert v is not None and r is not None, "output length differs"
            v, r = json.loads(v), json.loads(r)
            assert v["case"] == r["case"], "case order differs"
            assert v["next"] == r["next"], (rows[count], "position RNG consumption differs")
            for key in ["hits", "destroy"]:
                assert len(v[key]) == len(r[key]), (rows[count], key, "count", len(v[key]), len(r[key]))
                for vp, rp in zip(v[key], r[key]):
                    if vp is None or rp is None:
                        assert vp == rp, (rows[count], key, "empty shape differs")
                        continue
                    assert len(vp) == len(rp) == 3
                    for expected, got in zip(vp, rp):
                        error = abs(expected-got)
                        max_error = max(max_error, error)
                        assert error <= 1e-10, (rows[count], key, expected, got, error)
                    if key == "hits": hits += 1
                    else: burst += 1
            count += 1
    assert count == len(rows)
    base.ensure_sources_unchanged(paths, tested)
    lab.write_json(build / "report.json", dict(cases=count, hits=hits, burst=burst, max_error=max_error, sources_sha256=tested, gameplay_facts_sha256=lab.digest(resources / "gameplay-facts.json", "sha256"), client_jar_sha256=lab.digest(jars["client"], "sha256"), scope="Actual ClientLevel mining and destruction enqueue positions/count/order, outline offsets, six hit faces, position RNG draws. Sprite/tint lookup is stubbed; particle-local RNG, velocities, ticks and complete framebuffer parity are not compared."))
    print(f"{count:,} block-particle cases / {hits:,} mining crumbs / {burst:,} burst crumbs match vanilla; max error {max_error:g}")


if __name__ == "__main__":
    main()
