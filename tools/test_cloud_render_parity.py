#!/usr/bin/env python3
"""Compare fancy cloud exterior/interior faces, nearest-first mesh traversal and the
cell radius of the Cloud Distance option (independent of the render distance) with 26.3."""
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def make_input(build):
    cases = []
    for x in (-130, -2, -1, 0, 1, 2, 130):
        for z in (-130, -2, -1, 0, 1, 2, 130):
            for bottom in (-5, -4, 0, 1):
                for empty in range(16):
                    cases.append(f"0 {x} {z} {bottom} {empty}\n")
    for radius in range(11):
        for bottom in (-5, -4, 0, 1):
            cases.append(f"1 {radius} 0 {bottom} 0\n")
    # CloudRenderer.prepare: radius from options.cloudRange (2..128 chunks).
    for cloud_range in (2, 3, 4, 7, 8, 12, 32, 64, 127, 128):
        cases.append(f"2 {cloud_range} 0 -5 0\n")
    inputs = build / "inputs.txt"
    inputs.write_text("".join(cases))
    return inputs, len(cases)


def main():
    harness.differential("cloud-render-parity", "VanillaCloudRenderOracle", "cloud_fixture", make_input,
                         "Fancy cloud face culling, interior winding flags and mesh traversal; not full framebuffer parity.",
                         sources=[lab.ROOT / "crates/horizon-client-render/src/clouds.rs"])


if __name__ == "__main__":
    main()
