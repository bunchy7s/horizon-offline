#!/usr/bin/env python3
"""Generate crates/horizon-protocol/src/adapter/v<OLD>_ids.rs: how an older release numbers the registry entries
the protocol sends by id (block states, items, entity types, ...) against the pinned one, from both versions'
data generator reports (registries.json, blocks.json).

    generate_registry_id_adapter.py --old ~/code/horizon-parity/versions/26.2

`--old` (and `--new`, the pinned version in .local by default) is a directory `tools/version_port.py prepare
VERSION --download` made. Entries are matched by name; block states by block name and properties. Each
registry gets two tables of runs `(from, to, length)`: `<NAME>_UP` from the old server's ids to the pinned ones,
`<NAME>_DOWN` back. An id in no run has no entry in the other version. Only registries whose ids differ are written; the rest map to themselves.
"""

import argparse
import json
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
PER_LINE = 16
NONE = 65535
# Registries the protocol sends by numeric id (ByteBufCodecs.registry / holderRegistry / idMapper over a
# registry), as the adapter reads them.
REGISTRIES = [
    "minecraft:item",
    "minecraft:entity_type",
    "minecraft:particle_type",
    "minecraft:sound_event",
    "minecraft:data_component_type",
    "minecraft:command_argument_type",
    "minecraft:custom_stat",
    "minecraft:map_decoration_type",
    "minecraft:block",
    "minecraft:block_entity_type",
    "minecraft:menu",
    "minecraft:mob_effect",
    "minecraft:attribute",
    "minecraft:stat_type",
    "minecraft:fluid",
    "minecraft:game_event",
    "minecraft:recipe_serializer",
    "minecraft:slot_display",
    "minecraft:recipe_display",
    "minecraft:recipe_book_category",
    "minecraft:villager_type",
    "minecraft:villager_profession",
    "minecraft:position_source_type",
    "minecraft:consume_effect_type",
    "minecraft:enchantment_effect_component_type",
]


def entries(report, registry):
    """{name: id} of one registry, checked to number its entries 0..n."""
    found = {name: e["protocol_id"] for name, e in report.get(registry, {}).get("entries", {}).items()}
    assert sorted(found.values()) == list(range(len(found))), registry
    return found


def states(blocks):
    """{(block, properties): state id} over every state of every block."""
    found = {}
    for name, block in blocks.items():
        for state in block["states"]:
            key = (name, tuple(sorted(state.get("properties", {}).items())))
            found[key] = state["id"]
    assert sorted(found.values()) == list(range(len(found)))
    return found


def table(source, target, renames=None):
    """For each id of `source` (in order), the id `target` gives the same key (or the key it was renamed
    to), or NONE."""
    renames = renames or {}
    by_id = sorted(source.items(), key=lambda kv: kv[1])
    return [target.get(renames.get(key, key), NONE) for key, _ in by_id]


def runs(values):
    """The mapping as runs (from, to, length): ids from..from+length map to to..to+length. Ids in no run
    have no counterpart."""
    found = []
    for i, v in enumerate(values):
        if v == NONE:
            continue
        if found and found[-1][0] + found[-1][2] == i and found[-1][1] + found[-1][2] == v:
            found[-1][2] += 1
        else:
            found.append([i, v, 1])
    return found


def rust_table(name, values):
    assert all(0 <= v <= NONE for v in values)
    cells = [f"({a}, {b}, {n})" for a, b, n in runs(values)]
    lines = ["#[rustfmt::skip]", f"pub const {name}: &[(u16, u16, u16)] = &["]
    for i in range(0, len(cells), PER_LINE // 2):
        lines.append("    " + ", ".join(cells[i:i + PER_LINE // 2]) + ",")
    lines.append("];")
    return lines


def const_name(registry):
    return registry.split(":", 1)[1].upper()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old", type=Path, required=True, help="the older version's prepared directory")
    parser.add_argument("--new", type=Path, help="the pinned version's prepared directory (default: .local)")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--rename", action="append", default=[], metavar="REGISTRY:OLD=NEW",
                        help="an entry the pinned version knows by another name, e.g. "
                             "minecraft:command_argument_type:minecraft:color=minecraft:team_color")
    args = parser.parse_args()
    renames = {}
    for spec in args.rename:
        head, new = spec.split("=", 1)
        registry, old_path = head.rsplit(":", 1)
        registry, old_ns = registry.rsplit(":", 1)
        renames.setdefault(registry, {})[f"{old_ns}:{old_path}"] = new
    old_reports = args.old / "datagen/reports"
    new_reports = args.new / "datagen/reports" if args.new else ROOT / ".local/datagen/out/reports"
    old_info = lab.version_info(args.old / "jars/client.jar")
    info = lab.version_info(args.new / "jars/client.jar") if args.new else lab.version_info()
    old_reg = json.loads((old_reports / "registries.json").read_text())
    new_reg = json.loads((new_reports / "registries.json").read_text())
    module = "v" + old_info["id"].replace(".", "_") + "_ids"
    out = args.out or ROOT / f"crates/horizon-protocol/src/adapter/{module}.rs"
    lines = [
        f"//! Registry ids of {old_info['id']} against the pinned {info['name']}: block states and the registries",
        "//! the protocol sends by id. Generated by tools/generate_registry_id_adapter.py from both versions'",
        "//! registries and blocks reports; do not edit by hand.",
        "",
    ]
    old_states = states(json.loads((old_reports / "blocks.json").read_text()))
    new_states = states(json.loads((new_reports / "blocks.json").read_text()))
    lines.append(f"/// Block states: {len(old_states)} in {old_info['id']}, {len(new_states)} pinned.")
    lines.append(f"pub const OLD_STATES: u32 = {len(old_states)};")
    lines.append(f"pub const PINNED_STATES: u32 = {len(new_states)};")
    lines.extend(rust_table("BLOCK_STATE_UP", table(old_states, new_states)))
    lines.extend(rust_table("BLOCK_STATE_DOWN", table(new_states, old_states)))
    same = []
    for registry in REGISTRIES:
        o, n = entries(old_reg, registry), entries(new_reg, registry)
        if o == n:
            same.append(registry)
            continue
        name = const_name(registry)
        lines.append(f"/// `{registry}`: {len(o)} in {old_info['id']}, {len(n)} pinned.")
        forward = renames.get(registry, {})
        backward = {new: old for old, new in forward.items()}
        lines.extend(rust_table(f"{name}_UP", table(o, n, forward)))
        lines.extend(rust_table(f"{name}_DOWN", table(n, o, backward)))
    lines.append(f"/// Every table above, as the adapter reads them.")
    lines.append("pub const IDS: super::ids::Ids = super::ids::Ids {")
    lines.append("    quirks: super::ids::Quirks::NONE,")
    lines.append("    states: (OLD_STATES, PINNED_STATES),")
    lines.append("    block_state: super::ids::Remap::Runs { up: BLOCK_STATE_UP, down: BLOCK_STATE_DOWN },")
    for registry in REGISTRIES:
        field = registry.split(":", 1)[1]
        if registry in same:
            lines.append(f"    {field}: super::ids::Remap::Same,")
        else:
            name = const_name(registry)
            lines.append(f"    {field}: super::ids::Remap::Runs {{ up: {name}_UP, down: {name}_DOWN }},")
    lines.append("    // Not a registry: set by hand where a version numbers EntityDataSerializers differently.")
    lines.append("    entity_data_serializer: super::ids::Remap::Same,")
    lines.append("};")
    out.write_text("\n".join(lines) + "\n")
    print(out, "same:", ", ".join(same))


if __name__ == "__main__":
    main()
