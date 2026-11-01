#!/usr/bin/env python3
"""Compare the menu a container vehicle opens against vanilla 26.3.

The Java oracle (VanillaVehicleContainerOracle) creates every vehicle on the
shared stand-in server level and asks its MenuProvider for createMenu - the
menu a server sends in ClientboundOpenScreenPacket - and prints its MenuType,
the container's slot count and the title's translation key
(Entity.getDisplayName: the vehicle's custom name or its type's name). The Rust
example (vehicle_container_fixture) answers with
horizon_client::containers::vehicle_menu and vehicle_menu_name, the menu
`lan_open_vehicle_container` sends. Compared for every chest boat wood type,
the bamboo chest raft, the chest minecart and the hopper minecart.

What it does not cover: the interaction itself (the right-click/sneak/riding
paths - the live before/after runs cover those), the GUI drawing, and the
custom name a renamed vehicle would show (Horizon's vehicles keep no custom
name).
"""
import argparse
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

WOODS = ['oak', 'spruce', 'birch', 'jungle', 'acacia', 'dark_oak', 'mangrove',
         'cherry', 'pale_oak', 'poplar']
CASES = (['minecraft:%s_chest_boat' % wood for wood in WOODS]
         + ['minecraft:bamboo_chest_raft',
            'minecraft:chest_minecart',
            'minecraft:hopper_minecart'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--limit', type=int, default=4)
    args = parser.parse_args()
    work = lib.work_dir('vehicle-container-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaVehicleContainerOracle')
    (work / 'cases.txt').write_text(''.join(c + '\n' for c in CASES), encoding='utf-8')
    vanilla.run('VanillaVehicleContainerOracle', work / 'cases.txt', work / 'native.txt')
    lib.cargo_example('horizon-client', 'vehicle_container_fixture', work / 'cases.txt', work / 'rust.txt')
    errors = sum(line == 'error' for line in (work / 'native.txt').read_text(encoding='utf-8').split('\n'))
    if errors:
        print(f'{errors} cases fail in vanilla')
    total = lib.compare(CASES, work / 'native.txt', work / 'rust.txt', lambda case, line: case, limit=args.limit)
    raise SystemExit(1 if total or errors else 0)


if __name__ == '__main__':
    main()
