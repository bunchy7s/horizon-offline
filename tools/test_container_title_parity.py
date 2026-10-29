#!/usr/bin/env python3
"""Compare the titles container menus open with against vanilla 26.3 (#3245).

The Java oracle (VanillaContainerTitleOracle) loads each container block
entity with BlockEntity.loadStatic, applies the component patch of the item it
was placed from (BlockEntity.applyComponentsFromItemStack: a renamed shulker
box or copper chest gives its custom_name to CustomName), and prints the menu
provider's getDisplayName: BaseContainerBlockEntity.getName, the custom name
or getDefaultName, and for a double chest ChestBlock's MENU_PROVIDER_COMBINER
(the right half's name, else the left's, else container.chestDouble). That is
the title of the ClientboundOpenScreenPacket a server sends, which the
client's screen draws. The Rust example (container_title_fixture) places the
item with horizon_client::block_entity_item as the game does and names the
menu with horizon_client::containers, which the server's
`lan_open_container` sends. Compared for every container block: chests,
trapped and copper chests (every weathering state, waxed too), barrels,
shulker boxes of any colour, dispensers, droppers, hoppers, the three
furnaces, brewing stands and crafters, unnamed, named in their saved data,
placed from a renamed item, and double chests named on either half.
"""
import argparse
import json
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

POS = 'x:0,y:64,z:0'

# (block, its block entity type).
BLOCKS = [
    ('chest', 'chest'), ('trapped_chest', 'trapped_chest'),
    ('copper_chest', 'chest'), ('exposed_copper_chest', 'chest'), ('weathered_copper_chest', 'chest'),
    ('oxidized_copper_chest', 'chest'), ('waxed_copper_chest', 'chest'), ('waxed_oxidized_copper_chest', 'chest'),
    ('barrel', 'barrel'), ('shulker_box', 'shulker_box'), ('red_shulker_box', 'shulker_box'),
    ('black_shulker_box', 'shulker_box'), ('dispenser', 'dispenser'), ('dropper', 'dropper'), ('hopper', 'hopper'),
    ('furnace', 'furnace'), ('smoker', 'smoker'), ('blast_furnace', 'blast_furnace'),
    ('brewing_stand', 'brewing_stand'), ('crafter', 'crafter'),
]
NAME = '"minecraft:custom_name"'
# (saved NBT body, the placed item's component patch or None).
VARIANTS = [
    ('', None),
    ('CustomName:"Loot"', None),
    ('', '{%s:"Loot"}' % NAME),
    ('', '{%s:"Box of §cred"}' % NAME),
    ('CustomName:"Old"', '{%s:"New"}' % NAME),
    ('CustomName:"Old"', '{"!minecraft:custom_name":{}}'),
    ('', '{"minecraft:lore":["a"]}'),
]
# Double chests: (block, block entity type), then the halves' names.
DOUBLES = [('chest', 'chest'), ('trapped_chest', 'trapped_chest')]
HALVES = [(None, None), ('Right', None), (None, 'Left'), ('Right', 'Left')]


def be(type_id, body=''):
    return '{id:"minecraft:%s",%s%s}' % (type_id, POS, ',' + body if body else '')


def named(type_id, name):
    return be(type_id, 'CustomName:"%s"' % name if name else '')


def cases():
    out = []
    for block, type_id in BLOCKS:
        for body, patch in VARIANTS:
            case = {'block': 'minecraft:' + block, 'nbt': be(type_id, body)}
            if patch is not None:
                case['patch'] = patch
            out.append(case)
    for block, type_id in DOUBLES:
        for first, second in HALVES:
            out.append({'block': 'minecraft:' + block, 'first': named(type_id, first), 'second': named(type_id, second)})
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('container-title-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaContainerTitleOracle')
    all_cases = cases()
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in all_cases), encoding='utf-8')
    vanilla.run('VanillaContainerTitleOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'container_title_fixture', work / 'cases.jsonl', work / 'rust.txt')
    errors = sum(line == 'error' for line in (work / 'native.txt').read_text(encoding='utf-8').split('\n'))
    if errors:
        print(f'{errors} cases fail in vanilla')
    total = lib.compare(all_cases, work / 'native.txt', work / 'rust.txt',
                        lambda case, line: case['block'] + (' double' if 'first' in case else ''),
                        lambda case: json.dumps(case, ensure_ascii=False), args.limit)
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
