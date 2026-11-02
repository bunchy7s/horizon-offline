#!/usr/bin/env python3
"""Compare item tooltip display visibility with the pinned vanilla client."""
import json
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def cases(build):
    values = [
        (False, False, False),
        (False, False, True),
        (False, True, False),
        (False, True, True),
        (True, False, False),
        (True, False, True),
        (True, True, False),
        (True, True, True),
    ]
    path = build / 'cases.txt'
    # Lowercase: Rust's bool parser takes only true/false (Java's parseBoolean ignores case).
    names = [
        'Plain name',
        {'text': '', 'extra': [
            {'text': 'Red', 'color': '#ff0000', 'italic': False},
            {'text': 'Blue', 'color': '#0000ff'}]},
        {'text': 'Gold', 'color': 'gold', 'bold': True, 'italic': False},
        [{'text': 'List ', 'bold': True}, {'text': 'tail', 'bold': False}],
    ]
    path.write_text('\n'.join('\t'.join(str(v).lower() for v in row) for row in values) + '\n'
                    + ''.join('name\t' + json.dumps(name, ensure_ascii=False) + '\n' for name in names))
    return path, len(values) + len(names)


if __name__ == '__main__':
    harness.differential(
        'tooltip-display-parity', 'VanillaTooltipDisplayOracle', 'tooltip_display_fixture', cases,
        'Tooltip visibility and styled custom-name colour, italic and bold inheritance',
        sources=[
            lab.ROOT / 'crates/horizon-client/src/game/stations.rs',
            lab.ROOT / 'crates/horizon-client/src/item_tooltip.rs',
            lab.ROOT / 'crates/horizon-items/src/item_data.rs',
        ],
        expected_lines=12)
