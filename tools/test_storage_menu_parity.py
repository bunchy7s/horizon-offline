#!/usr/bin/env python3
"""Compare the menu a storage block opens against vanilla 26.3.

The Java oracle (VanillaStorageMenuOracle) builds each block's block entity
and its menu with BaseContainerBlockEntity.createMenu, the menu a server
sends in ClientboundOpenScreenPacket, and prints its MenuType and whether its
first slot takes a shulker box. A shulker box opens a ShulkerBoxMenu
(minecraft:shulker_box), whose ShulkerBoxSlots refuse shulker boxes; chests
and barrels open a 9x3 ChestMenu. The Rust example (storage_menu_fixture)
answers with horizon_client::containers::block_menu_type, the menu type the
server's lan_open_container sends; the server refuses shulker boxes in the
menus of that type (lan::containers, ShulkerBoxSlot). Compared for chests,
trapped and copper chests, barrels, shulker boxes of every colour,
dispensers, droppers and hoppers.
"""
import argparse
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

COLOURS = ['white', 'orange', 'magenta', 'light_blue', 'yellow', 'lime', 'pink', 'gray', 'light_gray', 'cyan',
           'purple', 'blue', 'brown', 'green', 'red', 'black']
BLOCKS = (['chest', 'trapped_chest', 'barrel', 'dispenser', 'dropper', 'hopper', 'shulker_box']
          + [f'{c}_shulker_box' for c in COLOURS]
          + [f'{w}copper_chest' for w in ('', 'exposed_', 'weathered_', 'oxidized_')]
          + [f'waxed_{w}copper_chest' for w in ('', 'exposed_', 'weathered_', 'oxidized_')])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('storage-menu-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaStorageMenuOracle')
    cases = ['minecraft:' + b for b in BLOCKS]
    (work / 'cases.txt').write_text(''.join(c + '\n' for c in cases), encoding='utf-8')
    vanilla.run('VanillaStorageMenuOracle', work / 'cases.txt', work / 'native.txt')
    lib.cargo_example('horizon-client', 'storage_menu_fixture', work / 'cases.txt', work / 'rust.txt')
    errors = sum(line == 'error' for line in (work / 'native.txt').read_text(encoding='utf-8').split('\n'))
    if errors:
        print(f'{errors} cases fail in vanilla')
    total = lib.compare(cases, work / 'native.txt', work / 'rust.txt', lambda case, line: case, limit=args.limit)
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
