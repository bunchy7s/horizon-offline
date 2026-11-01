#!/usr/bin/env python3
"""Compare a player's right click on a block with vanilla's, for items that are
both food and blocks (sweet and glow berries, carrots, potatoes), for sweet
berry bushes, for spawn eggs used on a monster spawner, for shears carving
pumpkins or cropping a growing plant's head (kelp, weeping and twisting vines,
cave vines) to its top age, for copper golem statues turned to their next pose,
for blocks placed where mobs and armor stands stand, for respawn anchors and for
beds in the Nether and the End.

Every case has a block at the origin, something above it, and a player three
blocks south, survival or creative, hungry or full, sneaking or not, who
right-clicks the top face of the origin (for pumpkins, any face, turned any
way) holding an item, sometimes with another (or nothing) in the main hand
and the item in the off hand, the main hand trying first. Vanilla runs the
real `ServerPlayerGameMode.useItemOn` over the headless level of the
dispenser harness (`VanillaDispenserOracle.useItem`): the block's `useItemOn`
and `useWithoutItem` (picking a bush's berries, `SweetBerryBushBlock.useItemOn`
passing bone meal on, `PumpkinBlock.useItemOn` carving with shears and
dropping the seeds from the carved side), then `ItemStack.useOn`
(`BlockItem.place` with its `mayPlaceOn` rules, `BoneMealItem.useOn`,
`ShearsItem.useOn` cropping a growing plant's head, eating a consumable that
could not be placed, and `SpawnEggItem.useOn` setting a spawner's mob). Respawn
anchors are charged with glowstone, set the respawn point in the Nether and
explode elsewhere (`RespawnAnchorBlock`, through the stand-in level's real
`ServerExplosion`, with water at the anchor). Respawn cases call
`ServerPlayer.findRespawnAndUseSpawnBlock` on an anchor among blocks
(`RespawnAnchorBlock.findStandUpPosition` through `DismountHelper`): the
position, the yaw that faces the anchor and the charge it uses; others on a
bed's head set from some yaw (`AbstractBedBlock.findStandUpPosition`: intact,
bunk, only the foot left or gone, crowded around and on top). Beds and straw
beds clicked (head or foot, whole or broken) in the Nether and the End blow up
or fall apart (`AbstractBedBlock.useWithoutItem` under the dimension's
`bed_rule`: `BedBlock.destroyOnUse`, `StrawBedBlock.destroyBed`). Horizon runs
`use_item_as_player` on that hit in a headless game
(`HORIZON_DISPENSE_FIXTURE`). Both print block changes, dropped items, level
events, sounds, game events, the used statistic, the held stack, the item the
player started using, spawners' next spawn data, potentials and delay, the
off hand's stack,
player's respawn point, system messages and the level random afterwards.
"""
import json
import os
import random
try:
    from . import block_update_harness as h, lab, test_dispenser_parity as d, test_rust_parity as base
except ImportError:
    import block_update_harness as h
    import lab
    import test_dispenser_parity as d
    import test_rust_parity as base

# Blocks the item is used on: the supports of the planted blocks and others.
GROUNDS = ['grass_block', 'dirt', 'coarse_dirt', 'podzol', 'rooted_dirt', 'moss_block', 'pale_moss_block', 'mud',
           'muddy_mangrove_roots', 'farmland', 'stone', 'sand', 'gravel', 'clay', 'snow_block', 'oak_planks']
ITEMS = ['sweet_berries', 'sweet_berries', 'sweet_berries', 'glow_berries', 'carrot', 'potato', 'bone_meal',
         'wheat_seeds', 'beetroot_seeds', 'apple']
# What is in the way above the clicked block.
ABOVE = ['air'] * 8 + ['short_grass', 'snow', 'stone', 'water', 'dead_bush', 'fern']
EGGS = ['zombie', 'skeleton', 'spider', 'cave_spider', 'pig', 'creeper', 'blaze', 'ghast', 'villager', 'bee', 'cod']
# What stands around an anchor that explodes.
AROUND = ['stone', 'dirt', 'oak_planks', 'cobblestone', 'glass', 'obsidian', 'netherrack', 'water', 'tnt', 'oak_log',
          'grass_block', 'soul_sand', 'gravel', 'crying_obsidian']


STAND = ['air'] * 6 + ['stone', 'stone', 'oak_slab', 'snow', 'lava', 'magma_block', 'fire', 'cactus', 'water',
         'ladder', 'oak_trapdoor', 'short_grass', 'white_carpet', 'campfire', 'sweet_berry_bush',
         'oak_fence', 'powder_snow', 'farmland', 'soul_sand', 'cobweb', 'end_portal']


def respawn_case(rng, states, case):
    """Respawning at an anchor with blocks around it."""
    charges = rng.choice([0, 1, 1, 2, 4])
    cells = [[0, 0, 0, states.get('respawn_anchor', charges=str(charges))]]
    for dy in (-2, -1, 0, 1, 2):
        for dx in range(-1, 2):
            for dz in range(-1, 2):
                if (dx, dy, dz) == (0, 0, 0):
                    continue
                if dy == -1 or dy == -2:
                    name = rng.choice(['stone'] * 6 + STAND)
                else:
                    name = rng.choice(['air'] * 3 + STAND)
                if name == 'air':
                    continue
                state = states.any(rng, name) if name in ('oak_trapdoor', 'oak_slab', 'snow', 'campfire', 'water', 'ladder') else states.get(name)
                cells.append([dx, dy, dz, state])
    if rng.random() < 0.6:
        case['dim'] = 'nether'
    case['respawn_at'] = {'forced': rng.random() < 0.2, 'consume': rng.random() < 0.8}
    case['cells'] = cells


BED_COLORS = ['red', 'white', 'black', 'cyan', 'lime']
FACINGS = {'north': (0, -1), 'south': (0, 1), 'west': (-1, 0), 'east': (1, 0)}


def bed_respawn_case(rng, states, case):
    """Respawning at a bed's head set from some yaw: intact, a bunk bed, only the foot left or
    gone, with blocks around and on it, in the Overworld (and a few in the Nether or the End)."""
    name = 'straw_bed' if rng.random() < 0.15 else rng.choice(BED_COLORS) + '_bed'
    facing = rng.choice(list(FACINGS))
    fx, fz = FACINGS[facing]
    bed = {}
    kept = rng.random()
    if kept < 0.75:
        bed[(0, 0, 0)] = states.get(name, facing=facing, part='head', occupied='false')
    elif kept < 0.85:
        bed[(0, 0, 0)] = states.get(rng.choice(['stone', 'white_carpet', 'oak_slab']))
    if kept < 0.9 or rng.random() < 0.5:
        bed[(-fx, 0, -fz)] = states.get(name, facing=facing, part='foot', occupied='false')
    if rng.random() < 0.2:
        # A bunk bed: another bed below.
        bed[(0, -1, 0)] = states.get(name, facing=facing, part='head', occupied='false')
        bed[(-fx, -1, -fz)] = states.get(name, facing=facing, part='foot', occupied='false')
    crowd = rng.choice([0.0, 0.1, 0.3, 0.6, 0.95])
    cells = [[*k, v] for k, v in bed.items()]
    if (0, 0, 0) not in bed:
        cells.insert(0, [0, 0, 0, states.get('air')])
    for dy in (-2, -1, 0, 1, 2):
        for dx in range(-3, 4):
            for dz in range(-3, 4):
                if (dx, dy, dz) in bed or (dx, dy, dz) == (0, 0, 0):
                    continue
                if dy == -1 and (dx, 0, dz) in bed:
                    name2 = 'stone'
                elif dy < 0:
                    name2 = rng.choice(['stone'] * 8 + STAND)
                elif rng.random() < crowd:
                    name2 = rng.choice(['stone'] * 4 + STAND)
                else:
                    continue
                if name2 == 'air':
                    continue
                state = states.any(rng, name2) if name2 in ('oak_trapdoor', 'oak_slab', 'snow', 'campfire', 'water', 'ladder') else states.get(name2)
                cells.append([dx, dy, dz, state])
    if rng.random() < 0.1:
        case['dim'] = rng.choice(['nether', 'end'])
    yaw = rng.choice([0.0, 90.0, -90.0, 180.0, round(rng.uniform(-720, 720), rng.choice([0, 2]))])
    case['respawn_at'] = {'forced': rng.random() < 0.15, 'consume': rng.random() < 0.8, 'yaw': yaw}
    case['cells'] = cells


def bed_case(rng, states, case):
    """A bed (or straw bed) clicked in the Nether or the End, among blocks."""
    name = 'straw_bed' if rng.random() < 0.25 else rng.choice(BED_COLORS) + '_bed'
    facing = rng.choice(list(FACINGS))
    dx, dz = FACINGS[facing]
    # The clicked block is the origin; the other half lies along the facing.
    part = rng.choice(['head', 'foot'])
    other = (-dx, 0, -dz) if part == 'head' else (dx, 0, dz)
    cells = [[0, 0, 0, states.get(name, facing=facing, part=part, occupied='false')],
             [0, -1, 0, states.get(rng.choice(['netherrack', 'end_stone', 'obsidian', 'stone']))],
             [other[0], -1, other[2], states.get('stone')]]
    if rng.random() < 0.9:
        cells.append([*other, states.get(name, facing=facing, part='foot' if part == 'head' else 'head', occupied='false')])
    taken = {tuple(c[:3]) for c in cells}
    for _ in range(rng.choice([0, 3, 10, 30])):
        p = (rng.randrange(-3, 4), rng.randrange(-2, 3), rng.randrange(-3, 4))
        if p in taken:
            continue
        taken.add(p)
        cells.append([*p, states.get(rng.choice(AROUND))])
    case['dim'] = rng.choice(['nether', 'end'])
    item = rng.choice(['-', '-', 'apple', 'stone', 'glowstone'])
    case['use_item'] = {'item': '-' if item == '-' else d_plain(item, 1), 'creative': rng.random() < 0.2,
                        'sneaking': rng.random() < 0.1, 'food': 20, 'distance': 12}
    if rng.random() < 0.2:
        case['rules'] = {'block_explosion_drop_decay': False}
    case['cells'] = cells


def anchor_case(rng, states, case):
    """A respawn anchor clicked in the Nether or the Overworld, with blocks
    and water around it."""
    charges = rng.choice([0, 0, 1, 2, 3, 4, 4])
    cells = [[0, 0, 0, states.get('respawn_anchor', charges=str(charges))],
             [0, -1, 0, states.get(rng.choice(['netherrack', 'stone', 'soul_sand', 'obsidian', 'soul_soil']))]]
    taken = {(0, 0, 0), (0, -1, 0)}
    for _ in range(rng.choice([0, 3, 8, 20, 40])):
        p = (rng.randrange(-3, 4), rng.randrange(-2, 3), rng.randrange(-3, 4))
        if p in taken:
            continue
        taken.add(p)
        name = rng.choice(AROUND)
        state = states.any(rng, 'water') if name == 'water' and rng.random() < 0.5 else states.get(name)
        cells.append([*p, state])
    if rng.random() < 0.2:
        # Water that would flow into the anchor's place.
        d = rng.choice([(1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, 1, 0)])
        cells = [c for c in cells if tuple(c[:3]) != d]
        cells.append([*d, states.any(rng, 'water') if rng.random() < 0.4 else states.get('water')])
    nether = rng.random() < 0.5
    if nether:
        case['dim'] = 'nether'
    item = rng.choice(['glowstone'] * 5 + ['apple', '-', 'stone', 'glowstone_dust'])
    count = rng.choice([1, 1, 16, 64])
    use = {'item': '-' if item == '-' else d_plain(item, count), 'creative': rng.random() < 0.2,
           'sneaking': rng.random() < 0.1, 'food': 20,
           # Away from the blast: the stand-in player is not an entity of the level.
           'distance': 3 if nether else 12}
    if rng.random() < 0.3:
        use['respawn'] = rng.choice([[0, 0, 0], [5, 0, 5]])
    if rng.random() < 0.4:
        # Glowstone in the off hand: the main hand passes it on (PASS).
        use['offhand'] = d_plain(rng.choice(['glowstone', 'glowstone', 'apple']), rng.choice([1, 16]))
    if rng.random() < 0.2:
        case['rules'] = {'block_explosion_drop_decay': False}
    case['use_item'] = use
    case['cells'] = cells


def pumpkin_case(rng, states, case):
    """Shears (or something else) used on a pumpkin (`PumpkinBlock.useItemOn`):
    any face, the player turned any way, worn shears (the stand-in player
    has no entity id for the break event, so they keep one use)."""
    cells = [[0, 0, 0, states.get(rng.choice(['pumpkin'] * 6 + ['carved_pumpkin', 'melon']))]]
    if rng.random() < 0.3:
        cells.append([0, 1, 0, states.get(rng.choice(['stone', 'snow', 'short_grass']))])
    damage = rng.choice([0, 0, 0, 100, 236])
    shears = d_plain('shears', 1) if damage == 0 else \
        '{id:"minecraft:shears",count:1,components:{"minecraft:damage":%d}}' % damage
    item = rng.choice([shears] * 6 + [d_plain('stick', 1), d_plain('pumpkin', 3), '-'])
    use = {'item': item, 'creative': rng.random() < 0.2, 'sneaking': rng.random() < 0.15, 'food': 20,
           'yaw': rng.choice([0, 30, 90, 135, 180, 200, 270, 315, -45, -100]),
           'face': rng.choice(['up', 'up', 'down', 'north', 'south', 'west', 'east'])}
    if rng.random() < 0.3:
        # Shears in the off hand, something else or nothing in the main hand.
        use.update(offhand=item, item=rng.choice(['-', '-', d_plain('stick', 1), d_plain('stone', 16)]))
    case['use_item'] = use
    case['cells'] = cells


# Growing plant heads (kelp, the two nether vines, cave vines) and the plants' bodies.
HEADS = ['kelp', 'weeping_vines', 'twisting_vines', 'cave_vines']
BODIES = ['kelp_plant', 'weeping_vines_plant', 'twisting_vines_plant', 'cave_vines_plant']


def crop_case(rng, states, case):
    """Shears (or something else) on a growing plant's head (`ShearsItem.useOn`): a head below its
    top age is cropped to `GrowingPlantHeadBlock.getMaxAgeState` (age 25) with a
    `block.growing_plant.crop` sound, a `block_change` game event and one point of wear; a head
    already at 25, or a plant's body (`GrowingPlantBodyBlock`), takes nothing (`Item.useOn`)."""
    head = rng.random() < 0.85
    name = rng.choice(HEADS) if head else rng.choice(BODIES)
    props = {}
    if name in ('cave_vines', 'cave_vines_plant'):
        # Berry-less: a berry-bearing vine picks its berries first (`useWithoutItem`), which is
        # another arm's.
        props['berries'] = 'false'
    if head:
        # Heads carry `AGE_25`; the bodies below them carry no age at all.
        props['age'] = str(rng.choice([0, 3, 11, 24, 25, 25]))
    state = states.get(name, **props) if props else states.any(rng, name)
    cells = [[0, 0, 0, state]]
    if rng.random() < 0.3:
        cells.append([0, -1, 0, states.get(rng.choice(['stone', 'dirt', 'sand', 'water']))])
    damage = rng.choice([0, 0, 0, 100, 236])
    shears = d_plain('shears', 1) if damage == 0 else \
        '{id:"minecraft:shears",count:1,components:{"minecraft:damage":%d}}' % damage
    item = rng.choice([shears] * 6 + [d_plain('stick', 1), d_plain('stick', 1), '-'])
    use = {'item': item, 'creative': rng.random() < 0.2, 'sneaking': rng.random() < 0.15, 'food': 20}
    if rng.random() < 0.2:
        # Shears in the off hand, something else or nothing in the main hand.
        use.update(offhand=item, item=rng.choice(['-', d_plain('stick', 1)]))
    case['use_item'] = use
    case['cells'] = cells


STATUES = [f'{wax}{age}copper_golem_statue' for wax in ('', 'waxed_')
           for age in ('', 'exposed_', 'weathered_', 'oxidized_')]


def statue_case(rng, states, case):
    """A copper golem statue right-clicked (`CopperGolemStatueBlock.useItemOn`, the weathering
    one's): anything but an axe, an empty hand too, turns it to its next pose; honeycomb waxes a
    statue that still weathers instead (`HoneycombItem.useOn`), and an axe scrapes or unwaxes it."""
    statue = rng.choice(STATUES)
    case['cells'] = [[0, 0, 0, states.any(rng, statue)]]
    items = ['-', '-', 'stick', 'honeycomb', 'honeycomb', 'stone', 'copper_ingot', 'diamond_pickaxe',
             'copper_golem_statue']
    if statue != 'copper_golem_statue':
        # An axe scrapes or unwaxes the others; on the unaffected one it brings the golem back
        # (`CopperGolemStatueBlockEntity.removeStatue`), which Horizon does not do yet.
        items.append('iron_axe')
    item = rng.choice(items)
    use = {'item': '-' if item == '-' else d_plain(item, rng.choice([1, 16])), 'creative': rng.random() < 0.2,
           'sneaking': rng.random() < 0.15, 'food': 20,
           'face': rng.choice(['up', 'up', 'north', 'south', 'west', 'east'])}
    if rng.random() < 0.2:
        # The item in the off hand, nothing in the main hand.
        use.update(offhand=use['item'], item='-')
    case['use_item'] = use


# Entities a block may be placed into (`Entity.blocksBuilding`), and blocks to place.
BLOCKERS = ['skeleton', 'zombie', 'pig', 'chicken', 'cow', 'armor_stand', 'slime', 'bat']
PLACED = ['obsidian', 'stone', 'oak_slab', 'oak_fence', 'white_carpet', 'torch', 'glass', 'oak_trapdoor', 'snow']


def blocked_case(rng, states, case):
    """A block placed on top of the origin where a mob or an armor stand stands, in its box or beside it
    (`BlockItem.canPlace`'s `Level.isUnobstructed`)."""
    case['cells'] = [[0, 0, 0, states.get(rng.choice(['stone', 'grass_block', 'dirt']))]]
    entities = []
    for _ in range(rng.choice([1, 1, 2])):
        kind = rng.choice(BLOCKERS)
        offset = [rng.choice([0, 0, 0.3, -0.45, 0.7, -1.2]), rng.choice([0, 0, 0.5, 0.95, 1.02, -0.5]),
                  rng.choice([0, 0, 0.3, -0.6, 0.9])]
        e = {'type': f'minecraft:{kind}', 'pos': [0.5 + offset[0], 1 + offset[1], 0.5 + offset[2]]}
        if kind == 'armor_stand' and rng.random() < 0.3:
            e['marker'] = True
        if kind in ('pig', 'cow', 'chicken') and rng.random() < 0.3:
            e['baby'] = True
        entities.append(e)
    case['entities'] = entities
    case['use_item'] = {'item': d_plain(rng.choice(PLACED), rng.choice([1, 16])), 'creative': rng.random() < 0.3,
                        'sneaking': False, 'food': 20}


def cases(facts, count, seed):
    rng = random.Random(seed)
    states = d.States(facts)
    for n in range(count):
        case = {**h.environment(rng), 'pos': [0, 0, 0]}
        case.pop('raining', None)
        case.pop('thundering', None)
        if rng.random() < 0.1:
            bed_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.1:
            respawn_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.1:
            bed_respawn_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.15:
            anchor_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.1:
            blocked_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.08:
            statue_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.12:
            pumpkin_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        if rng.random() < 0.08:
            crop_case(rng, states, case)
            case['items'] = []
            case['mode'] = 'dispense'
            yield case
            continue
        cells = []
        item = rng.choice(ITEMS)
        if rng.random() < 0.15:
            # A spawn egg on a fresh monster spawner (SpawnEggItem.useOn).
            cells.append([0, 0, 0, states.get('spawner')])
            if rng.random() < 0.3:
                cells.append([0, 1, 0, states.get(rng.choice(['stone', 'water', 'short_grass']))])
            item = rng.choice([f'{e}_spawn_egg' for e in EGGS] + ['apple', '-'])
            if rng.random() < 0.15:
                case['rules'] = {'spawner_blocks_work': False}
        elif rng.random() < 0.3:
            # A sweet berry bush on grass: pick it, bone meal it, or plant against it.
            age = rng.randrange(4)
            cells.append([0, 0, 0, states.get('sweet_berry_bush', age=str(age))])
            cells.append([0, -1, 0, states.get(rng.choice(['grass_block', 'dirt', 'podzol', 'farmland']))])
            item = rng.choice(['bone_meal', 'bone_meal', 'sweet_berries', 'apple', '-', 'stick'])
        else:
            ground = rng.choice(GROUNDS)
            cells.append([0, 0, 0, states.any(rng, ground) if ground == 'farmland' else states.get(ground)])
            above = rng.choice(ABOVE)
            if above != 'air':
                cells.append([0, 1, 0, states.get(above)])
            if item == 'glow_berries' and rng.random() < 0.5:
                # A ceiling two blocks up for the cave vines to hang from.
                cells.append([0, 2, 0, states.get(rng.choice(['stone', 'dirt', 'oak_leaves']))])
            if item == 'bone_meal' and ground in ('grass_block', 'moss_block', 'pale_moss_block'):
                # Bone meal on spreading blocks is the bone meal harness's.
                item = 'sweet_berries'
        held = '-' if item == '-' else d.plain(item, rng.choice([1, 1, 2, 16, 64]))
        case['use_item'] = {'item': held, 'creative': rng.random() < 0.2, 'sneaking': rng.random() < 0.2,
                            'food': rng.choice([20, 20, 19, 10, 0])}
        if rng.random() < 0.3:
            # The item in the off hand, something else or nothing in the main hand.
            main = rng.choice(['-', '-', 'stick', 'apple', 'stone', 'bone_meal', 'shield'])
            case['use_item'].update(offhand=held, item='-' if main == '-' else d.plain(main, rng.choice([1, 16])))
        case['cells'] = cells
        case['items'] = []
        case['mode'] = 'dispense'
        yield case


def d_plain(item, count):
    return d.plain(item, count)


def kind(case, facts):
    first = facts.states[case['cells'][0][3]][0].split(':')[1]
    if 'respawn_at' in case:
        return f'respawn at {first}'
    item = case['use_item']['item']
    item = item.split('"')[1].split(':')[1] if item != '-' else 'empty hand'
    if 'entities' in case:
        return f'{item} placed by ' + ', '.join(e['type'].split(':')[1] for e in case['entities'])
    return f'{item} on {first}'


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 4000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'use-item-on-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, args.count, args.seed))
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in all_cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    d.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
           '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaDispenserOracle.java')])
    d.run(['java', '-Xmx3G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaDispenserOracle',
           str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    d.run(['cargo', 'build', '-q', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
          env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    d.run([str(lab.cargo_target() / 'fast/horizon-client'), '--resources', str(resources)], cwd=work,
          env=lab.game_env(HORIZON_DISPENSE_FIXTURE=f'{work / "cases.jsonl"},{work / "rust.txt"}'))
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix) + ('D ',)
    native = h.split_cases((work / 'native.txt').read_text(), ignore)
    rust = h.split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(all_cases), (len(native), len(rust), len(all_cases))
    groups, errors = {}, {}
    planted = eaten = respawned = exploded = 0
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        planted += any(line.startswith('set 0,1,0') for line in a)
        eaten += any(line.startswith('held') and not line.endswith('using -') for line in a)
        respawned += any(line.startswith('respawn ') and line != 'respawn none' and 'respawn_at' in case for line in a)
        exploded += any(line.endswith(' minecraft:explode') for line in a)
        if not d.same(a, b):
            groups.setdefault(kind(case, facts), []).append((case, a, b))
    for name, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {name}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + d.describe(case, facts) + ' use=' + json.dumps(case.get('use_item', case.get('respawn_at'))))
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors); '
          f'vanilla planted in {planted}, started eating in {eaten}, exploded {exploded} times and found a respawn '
          f'spot in {respawned}')
    h.finish(total + skipped)


if __name__ == '__main__':
    main()
