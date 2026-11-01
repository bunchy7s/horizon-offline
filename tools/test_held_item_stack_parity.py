#!/usr/bin/env python3
"""Each arm resolves its own held item, including left-handed entities (#5049)."""
import itertools
import json

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab
    import oracle_harness as harness


def make_input(build):
    items = ['air', 'diamond_sword', 'totem_of_undying', 'shield', 'crossbow']
    cases = [dict(main=main, off=off, left=left)
             for main, off, left in itertools.product(items, items, [False, True])]
    path = build / 'cases.jsonl'
    path.write_text(''.join(json.dumps(case) + '\n' for case in cases))
    return path, len(cases)


def main():
    harness.differential(
        'held-item-stack-parity', 'VanillaHeldItemStackOracle', 'held_item_stack_probe',
        make_input, 'Actual ArmedEntityRenderState extraction passes each arm its own item; '
        'model resolution is recorded, not rendered. Covers empty hands and both main arms. '
        'Off-hand components are outside this identity-only regression.',
        sources=[lab.ROOT / 'tools/test_held_item_stack_parity.py',
                 lab.ROOT / 'crates/horizon-client/src/mobs/render.rs',
                 lab.ROOT / 'crates/horizon-client/src/mobs.rs',
                 lab.ROOT / 'crates/horizon-client/src/game/mob_blocks.rs'], expected_lines=100)


if __name__ == '__main__':
    main()
