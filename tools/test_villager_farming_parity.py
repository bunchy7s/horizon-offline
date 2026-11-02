#!/usr/bin/env python3
"""Compare villager farming classification with the actual vanilla 26.3 block objects.

Also runs the content-mod farming regressions. Outputs are private dev data, not game resources.
"""
import os
from pathlib import Path

try:
    from . import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking


def main():
    build = Path(os.environ.get("HORIZON_FARMING_PARITY_DIR", lab.LOCAL / "villager-farming-parity"))
    build.mkdir(parents=True, exist_ok=True)
    lock, jars, _ = base.verify_artifacts()
    classes, dependencies, _ = oracle_harness.compile_oracle(
        build, "VanillaVillagerFarmingOracle", jars, baking.client_libraries(), lock["minecraft"]["java_major"])
    expected = build / "vanilla.txt"
    oracle_harness.java_run(build, classes, dependencies, "VanillaVillagerFarmingOracle", [expected], "vanilla")
    env = dict(lab.game_env(), HORIZON_FARMING_ORACLE=str(expected))
    oracle_harness.run_logged([
        "cargo", "test", "--locked", "-p", "horizon-client-lib", "--lib",
        "mobs::villager_brain::farming_tests", "--", "--include-ignored", "--nocapture",
    ], build, "rust", env=env)
    print(f"{len(expected.read_text().splitlines())} vanilla block states match; content-mod farming regressions pass")


if __name__ == "__main__":
    main()
