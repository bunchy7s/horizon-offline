#!/usr/bin/env python3
"""Compare rain and snow column instances and quads (positions, texture coordinates, alpha, light) with 26.3."""
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def make_input(build):
    rng = random.Random(71)
    cases = []
    for kind in (0, 1):
        for ticks in (-9223372036854775808, -1, 0, 1, 511, 512, 131071, 131072, 24000 * 37 + 5, 9_000_000_123, 9223372036854775807):
            for x, z in ((0, 0), (1, 0), (0, -1), (3, 7), (-10, 10), (10, -10), (-4, -9), (1_000_003, -2_000_011), (30_000_000, -30_000_000), (2_147_483_631, -2_147_483_632)):
                for bottom, top in ((60, 80), (64, 65), (-64, -44), (300, 310)):
                    light = (rng.randrange(16) << 4) | (rng.randrange(16) << 20)
                    partial = rng.choice(("0.0", "0.5", "0.999", f"{rng.random():.6f}"))
                    cam = (x + rng.uniform(-10.5, 10.5), rng.uniform(bottom - 5, top + 5), z + rng.uniform(-10.5, 10.5))
                    camera = " ".join(f"{c:.9f}" for c in cam)
                    radius = rng.choice((3, 5, 10))
                    intensity = rng.choice(("1.0", "0.25", f"{rng.random():.6f}"))
                    cases.append(f"{kind} {ticks} {x} {bottom} {top} {z} {light} {partial} {camera} {radius} {intensity}\n")
    # The camera's own column: the size table divides 0 by 0.
    cases.append("0 100 5 60 80 5 240 0.0 5.5 70.0 5.5 10 1.0\n")
    inputs = build / "inputs.txt"
    inputs.write_text("".join(cases))
    return inputs, len(cases)


def main():
    harness.differential("weather-render-parity", "VanillaWeatherRenderOracle", "weather_fixture", make_input,
                         "Rain/snow column instances and quad vertices; not full framebuffer parity.",
                         sources=[lab.ROOT / "crates/horizon-client-render/src/weather.rs"])


if __name__ == "__main__":
    main()
