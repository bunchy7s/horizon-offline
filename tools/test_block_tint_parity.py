#!/usr/bin/env python3
"""Compare level-free block-model tints with actual 26.3 BlockTintSources.

Every state of blocks with registered sources, plus untinted carried blocks,
is checked at indices -1, 0, 1 and 2. Geometry and framebuffer are separate checks.
"""
import json
try:
    from . import lab, oracle_harness as harness, test_section_mesh_parity as mesh
except ImportError:
    import lab, oracle_harness as harness, test_section_mesh_parity as mesh

BLOCKS = """large_fern tall_grass fern short_grass potted_fern bush grass_block
pink_petals wildflowers spruce_leaves birch_leaves oak_leaves jungle_leaves
acacia_leaves dark_oak_leaves vine mangrove_leaves leaf_litter water_cauldron
water bubble_column redstone_wire sugar_cane attached_melon_stem
attached_pumpkin_stem melon_stem pumpkin_stem lily_pad stone poppy red_mushroom
brown_mushroom crimson_fungus warped_roots carved_pumpkin""".split()


def make_input(build):
    states = mesh.States(lab.RESOURCES)
    ids = sorted({state for name in BLOCKS for state in states.by_name["minecraft:" + name]})
    rows = [[state, index] for state in ids for index in [-1, 0, 1, 2]]
    path = build / "input.json"
    path.write_text(json.dumps(dict(resources=str(lab.RESOURCES), cases=rows)))
    return path, len(rows)


def main():
    harness.differential("block-tint-parity", "VanillaBlockTintOracle", "block_tint_probe",
        make_input, "BlockTintSource.color(state), all states of tinted blocks and untinted carried blocks; per-face tint indices, no level/biome. Not model or framebuffer parity.",
        sources=[lab.ROOT / "tools/test_block_tint_parity.py", lab.ROOT / "crates/horizon-client-render/src/mesher.rs",
                 lab.ROOT / "crates/horizon-client/src/item_render.rs"])


if __name__ == "__main__":
    main()
