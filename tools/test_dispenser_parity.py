#!/usr/bin/env python3
"""Compare vanilla dispensers and droppers with Horizon's, one activation per case.

A dispenser or dropper at the origin faces a random direction and holds the
case's items; the block in front (and below it) comes from a palette of blocks
the behaviours care about, and some cases put a target entity in front. Vanilla
`DispenserBlock.dispenseFrom` runs over the headless level of the block update
oracle (`VanillaDispenserOracle`); Horizon runs its game glue as a component probe on a server thread
(`HORIZON_DISPENSE_FIXTURE`). Both print block changes, container contents,
spawned entities (type, position, motion), the targets afterwards, level events,
sounds, game events and the level random. Motion drawn from an entity's own
unseeded random (vanilla marks it "~") is compared within its spread.
"""
import json
import os
import random
import subprocess
try:
    from . import block_update_harness as h, lab, test_rust_parity as base
    from .integration_env import run_game
except ImportError:
    from integration_env import run_game
    import block_update_harness as h
    import lab
    import test_rust_parity as base

DIRECTIONS = ['down', 'up', 'north', 'south', 'west', 'east']
STEP = {'down': (0, -1, 0), 'up': (0, 1, 0), 'north': (0, 0, -1), 'south': (0, 0, 1), 'west': (-1, 0, 0), 'east': (1, 0, 0)}


def potion(item, name):
    return '{id:"minecraft:%s",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:%s"}}}' % (item, name)


def plain(item, count=1):
    return '{id:"minecraft:%s",count:%d}' % (item, count)


WOODS = ['oak', 'spruce', 'birch', 'jungle', 'acacia', 'cherry', 'dark_oak', 'mangrove', 'pale_oak', 'poplar']

# Items with the blocks and entities in front that exercise their behaviour:
# one entry per DispenseItemBehavior.bootStrap registration family, plus the
# defaults of DispenserBlock.getDefaultDispenseMethod (equippables, sulfur
# cube swallowables, spawn eggs, a plain drop).
ITEMS = {
    'drop': [plain('stone', 5), plain('diamond'), plain('stick', 64), plain('oak_sapling', 3), plain('redstone', 2), plain('lead'),
             plain('wheat_seeds', 7)],
    'projectile': [plain('arrow', 3), potion('tipped_arrow', 'poison'), plain('spectral_arrow'), plain('egg', 2), plain('blue_egg'),
                   plain('brown_egg'), plain('snowball', 4), plain('experience_bottle'), potion('splash_potion', 'healing'),
                   potion('lingering_potion', 'swiftness'), plain('firework_rocket', 2), plain('fire_charge', 2), plain('wind_charge', 3)],
    'bucket': [plain('water_bucket'), plain('lava_bucket'), plain('powder_snow_bucket'), plain('salmon_bucket'), plain('cod_bucket'),
               plain('pufferfish_bucket'), plain('tropical_fish_bucket'), plain('axolotl_bucket'), plain('tadpole_bucket'),
               plain('sulfur_cube_bucket'), plain('bucket'), plain('bucket', 3)],
    'block': [plain('bone_meal', 4), plain('tnt', 2), plain('flint_and_steel'), plain('glowstone', 3), plain('honeycomb', 2),
              plain('glass_bottle', 2), potion('potion', 'water'), potion('potion', 'healing'), plain('shulker_box'),
              plain('red_shulker_box'), plain('lime_shulker_box'), plain('carved_pumpkin'), plain('wither_skeleton_skull'),
              plain('shears'), plain('brush')],
    'vehicle': [plain('oak_boat'), plain('bamboo_raft'), plain('bamboo_chest_raft'), plain('minecart'), plain('chest_minecart'),
                plain('furnace_minecart'), plain('tnt_minecart'), plain('hopper_minecart'), plain('command_block_minecart'),
                plain('armor_stand', 2), 'BOAT', 'CHEST_BOAT'],
    'spawn': [plain('pig_spawn_egg'), plain('sheep_spawn_egg', 2), plain('zombie_spawn_egg'), plain('cow_spawn_egg'),
              plain('chicken_spawn_egg'), plain('skeleton_spawn_egg'), plain('creeper_spawn_egg'), plain('villager_spawn_egg'),
              plain('wolf_spawn_egg'), plain('horse_spawn_egg'), plain('squid_spawn_egg'), plain('slime_spawn_egg')],
    'equipment': [plain('iron_helmet'), plain('diamond_chestplate'), plain('leather_leggings'), plain('golden_boots'),
                  plain('chainmail_helmet'), plain('netherite_boots'), plain('turtle_helmet'), plain('saddle'),
                  plain('iron_horse_armor'), plain('diamond_horse_armor'), plain('leather_horse_armor'), plain('white_carpet'),
                  plain('red_carpet'), plain('chest'), plain('carved_pumpkin'), plain('elytra'), plain('wolf_armor'), plain('shears'),
                  plain('brush'), plain('player_head'), plain('zombie_head'), plain('white_harness')],
    'swallow': [plain('slime_block'), plain('oak_planks', 3), plain('packed_ice'), plain('soul_sand'), plain('magma_block'),
                plain('white_wool'), plain('mud', 2), plain('andesite'), plain('iron_block'), plain('mushroom_stem'),
                plain('honeycomb_block'), plain('tube_coral_block'), plain('tnt'), plain('stone')],
}

FRONT = {
    'drop': ['air', 'stone', 'water', 'chest', 'hopper'],
    'projectile': ['air', 'air', 'water', 'stone', 'glass'],
    'bucket': ['air', 'air', 'water', 'water', 'lava', 'powder_snow', 'oak_slab', 'seagrass', 'stone', 'cauldron', 'kelp',
               'grass_block', 'fire'],
    'block': ['air', 'grass_block', 'wheat', 'oak_sapling', 'tnt', 'campfire', 'candle', 'respawn_anchor', 'beehive', 'bee_nest',
              'copper_block', 'exposed_cut_copper_stairs', 'dirt', 'coarse_dirt', 'rooted_dirt', 'water', 'seagrass', 'stone',
              'sweet_berry_bush', 'cocoa', 'obsidian', 'soul_campfire', 'candle_cake', 'netherrack', 'moss_block'],
    'vehicle': ['air', 'air', 'water', 'rail', 'powered_rail', 'activator_rail', 'stone', 'detector_rail'],
    'spawn': ['air', 'air', 'water', 'stone', 'grass_block'],
    'equipment': ['air'],
    'swallow': ['air'],
}
# The blocks a behaviour reacts to, drawn for that item most of the time.
FRONT_FOR = {
    'flint_and_steel': ['air', 'air', 'air', 'campfire', 'soul_campfire', 'candle', 'blue_candle', 'candle_cake',
                        'red_candle_cake', 'tnt', 'stone', 'obsidian', 'netherrack', 'water'],
    'glowstone': ['respawn_anchor', 'respawn_anchor', 'respawn_anchor', 'air', 'stone', 'glowstone'],
    'bone_meal': ['grass_block', 'wheat', 'carrots', 'potatoes', 'beetroots', 'oak_sapling', 'birch_sapling', 'spruce_sapling',
                  'cherry_sapling', 'mangrove_propagule', 'moss_block', 'sweet_berry_bush', 'cocoa', 'bamboo_sapling', 'bamboo',
                  'pumpkin_stem', 'melon_stem', 'torchflower_crop', 'pitcher_crop', 'rooted_dirt', 'glow_lichen', 'small_dripleaf',
                  'big_dripleaf', 'crimson_nylium', 'warped_fungus', 'brown_mushroom', 'kelp', 'sea_pickle', 'seagrass', 'water',
                  'short_grass', 'fern', 'dandelion', 'pink_petals', 'cave_vines', 'twisting_vines', 'weeping_vines', 'azalea',
                  'pale_moss_block', 'stone', 'air'],
    'honeycomb': ['copper_block', 'exposed_copper', 'weathered_cut_copper', 'oxidized_cut_copper_slab', 'copper_door',
                  'exposed_copper_trapdoor', 'copper_bulb', 'lightning_rod', 'waxed_copper_block', 'copper_grate', 'stone', 'air'],
    'glass_bottle': ['beehive', 'beehive', 'bee_nest', 'bee_nest', 'water', 'water', 'air', 'stone', 'seagrass'],
    'potion': ['dirt', 'coarse_dirt', 'rooted_dirt', 'dirt', 'stone', 'air', 'mud', 'water'],
    'shears': ['beehive', 'bee_nest', 'beehive', 'air', 'air', 'stone', 'pumpkin'],
    'brush': ['air', 'air', 'suspicious_sand', 'stone'],
    'tnt': ['air', 'air', 'stone', 'water', 'tnt'],
    'shulker_box': ['air', 'air', 'stone', 'water', 'short_grass', 'snow'],
    'carved_pumpkin': ['air', 'air', 'stone', 'snow'],
    'wither_skeleton_skull': ['air', 'air', 'stone'],
    'armor_stand': ['air', 'air', 'stone', 'water', 'short_grass'],
}
# Structures a behaviour completes (golems, the wither, a nether portal).
SCENES = {'carved_pumpkin': ['snow_golem', 'iron_golem', 'copper_golem', 'copper_golem'], 'wither_skeleton_skull': ['wither'],
          'flint_and_steel': ['portal']}
COPPER_BLOCKS = ['copper_block', 'exposed_copper', 'weathered_copper', 'oxidized_copper', 'waxed_copper_block',
                 'waxed_exposed_copper', 'waxed_weathered_copper', 'waxed_oxidized_copper']
# Plants that need soil (or a log) under them to exist at all.
ROOTED = {'wheat', 'carrots', 'potatoes', 'beetroots', 'oak_sapling', 'birch_sapling', 'spruce_sapling', 'cherry_sapling',
          'mangrove_propagule', 'sweet_berry_bush', 'cocoa', 'bamboo_sapling', 'bamboo', 'pumpkin_stem', 'melon_stem',
          'torchflower_crop', 'pitcher_crop', 'small_dripleaf', 'big_dripleaf', 'warped_fungus', 'brown_mushroom', 'kelp',
          'sea_pickle', 'seagrass', 'short_grass', 'fern', 'dandelion', 'pink_petals', 'twisting_vines', 'azalea'}
BELOW = ['air', 'stone', 'water', 'rail', 'grass_block', 'soul_sand', 'dirt', 'farmland', 'netherrack']

TARGETS = {
    'iron_helmet': ['zombie', 'skeleton', 'armor_stand', 'pig', 'copper_golem'],
    'chainmail_helmet': ['zombie', 'armor_stand', 'villager'],
    'diamond_chestplate': ['zombie', 'armor_stand'],
    'leather_leggings': ['skeleton', 'armor_stand'],
    'golden_boots': ['zombie', 'armor_stand'],
    'netherite_boots': ['zombie', 'piglin', 'armor_stand'],
    'turtle_helmet': ['armor_stand', 'zombie'],
    'saddle': ['pig', 'horse', 'strider', 'donkey', 'camel', 'llama', 'mule', 'skeleton_horse', 'nautilus'],
    'iron_horse_armor': ['horse', 'donkey'],
    'diamond_horse_armor': ['horse', 'zombie_horse'],
    'leather_horse_armor': ['horse'],
    'white_carpet': ['llama', 'horse', 'trader_llama'],
    'red_carpet': ['llama', 'trader_llama'],
    'chest': ['donkey', 'mule', 'llama', 'horse', 'trader_llama'],
    'carved_pumpkin': ['zombie', 'armor_stand', 'snow_golem', 'copper_golem'],
    'elytra': ['armor_stand', 'zombie'],
    'wolf_armor': ['wolf'],
    'white_harness': ['happy_ghast'],
    'shears': ['sheep', 'sheep', 'mooshroom', 'bogged', 'snow_golem', 'sulfur_cube', 'pig', 'cow', 'copper_golem', 'copper_golem'],
    'brush': ['armadillo'],
    'tnt': ['sulfur_cube'],
    'flint_and_steel': ['sulfur_cube'],
    'player_head': ['armor_stand', 'zombie'],
    'zombie_head': ['armor_stand', 'skeleton'],
}
for _block in ('stone', 'slime_block', 'oak_planks', 'packed_ice', 'soul_sand', 'magma_block', 'white_wool', 'mud', 'andesite',
               'iron_block', 'mushroom_stem', 'honeycomb_block', 'tube_coral_block'):
    TARGETS[_block] = ['sulfur_cube']
SWALLOWED = ['tnt', 'stone', 'slime_block', 'magma_block', 'white_wool', 'iron_block']


def item_choice(rng, kind):
    item = rng.choice(ITEMS[kind])
    if item == 'BOAT':
        return plain(rng.choice(WOODS) + '_boat')
    if item == 'CHEST_BOAT':
        return plain(rng.choice(WOODS) + '_chest_boat')
    return item


class States:
    def __init__(self, facts):
        self.facts = facts

    def get(self, name, **props):
        name = name if ':' in name else 'minecraft:' + name
        best = self.facts.defaults[name]
        if not props:
            return best
        for index in self.facts.by_block[name]:
            have = self.facts.states[index][4].get('properties', {})
            if all(have.get(k) == v for k, v in props.items()):
                return index
        raise KeyError((name, props))

    def any(self, rng, name):
        name = name if ':' in name else 'minecraft:' + name
        return rng.choice(self.facts.by_block[name])


def build_scene(cells, states, rng, scene, front, facing):
    """Put the rest of a structure the behaviour's block completes around the front."""
    x, y, z = front
    across = rng.choice([(1, 0, 0), (0, 0, 1)])

    def put(at, name, **props):
        if at != (0, 0, 0) and at != front:
            h.put(cells, at, states.get(name, **props))

    def side(at, k):
        return (at[0] + across[0] * k, at[1], at[2] + across[2] * k)
    if scene == 'snow_golem':
        put((x, y - 1, z), 'snow_block')
        put((x, y - 2, z), 'snow_block')
    elif scene == 'copper_golem':
        put((x, y - 1, z), rng.choice(COPPER_BLOCKS))
    elif scene == 'iron_golem':
        put((x, y - 1, z), 'iron_block')
        put((x, y - 2, z), 'iron_block')
        put(side((x, y - 1, z), -1), 'iron_block')
        put(side((x, y - 1, z), 1), 'iron_block')
    elif scene == 'wither':
        put((x, y - 1, z), 'soul_sand')
        put((x, y - 2, z), 'soul_sand')
        put(side((x, y - 1, z), -1), rng.choice(['soul_sand', 'soul_soil']))
        put(side((x, y - 1, z), 1), 'soul_sand')
        put(side((x, y, z), -1), 'wither_skeleton_skull')
        put(side((x, y, z), 1), rng.choice(['wither_skeleton_skull', 'wither_skeleton_skull', 'skeleton_skull']))
    elif scene == 'portal':
        # An obsidian frame around a 2x3 inside that holds the front block.
        width, height = rng.choice([(2, 3), (2, 3), (3, 4)])
        left, low = rng.randrange(width), rng.randrange(height)
        for i in range(-1, width + 1):
            for j in range(-1, height + 1):
                at = side((x, y + j - low, z), i - left)
                edge = i in (-1, width) or j in (-1, height)
                if edge and not (i in (-1, width) and j in (-1, height)):
                    put(at, 'obsidian')
                elif not edge:
                    if at not in ((0, 0, 0), front):
                        h.put(cells, at, states.get('air'))


def halves(states, name):
    """Whether a block comes in an upper and a lower half (tall plants, dripleaves, pitchers)."""
    name = name if ':' in name else 'minecraft:' + name
    return any(states.facts.states[i][4].get('properties', {}).get('half') == 'upper' for i in states.facts.by_block[name])


def front_state(states, rng, name):
    """Mostly the default state, sometimes a random one (charges, honey, ages, lit...)."""
    if halves(states, name):
        # A lone half is not a world vanilla makes; the pair is placed by put_front.
        return states.get(name)
    if name in ('respawn_anchor', 'beehive', 'bee_nest', 'campfire', 'candle', 'wheat', 'sweet_berry_bush', 'rail', 'powered_rail',
                'candle_cake', 'soul_campfire') or rng.random() < 0.3:
        return states.any(rng, name)
    return states.get(name)


def cases(facts, count, seed, only):
    rng = random.Random(seed)
    states = States(facts)
    kinds = list(ITEMS)
    for n in range(count):
        kind = kinds[n % len(kinds)]
        item = item_choice(rng, kind)
        if only and only not in item:
            continue
        bare = item.split('"')[1].split(':')[1]
        facing = rng.choice(DIRECTIONS)
        block = 'dropper' if rng.random() < 0.12 else 'dispenser'
        cells = [[0, 0, 0, states.get(block, facing=facing, triggered='false')]]
        dx, dy, dz = STEP[facing]
        front = (dx, dy, dz)
        below = (dx, dy - 1, dz)
        palette = FRONT_FOR[bare] if bare in FRONT_FOR and rng.random() < 0.8 else FRONT[kind]
        front_name = rng.choice(palette)
        if facing == 'up' and front_name in ROOTED:
            # Nothing grows out of a dispenser: vanilla never lets these stand on one.
            front_name = 'air'
        above = (dx, dy + 1, dz)
        if halves(states, front_name) and front_name != 'pitcher_crop':
            if above == (0, 0, 0):
                front_name = 'air'
            else:
                # The upper half matches the lower one in everything but its half.
                lower = states.facts.states[states.get(front_name)][4].get('properties', {})
                h.put(cells, above, states.get(front_name, **{**lower, 'half': 'upper'}))
        if front_name != 'air':
            h.put(cells, front, front_state(states, rng, front_name))
        if below != (0, 0, 0) and rng.random() < 0.8:
            name = rng.choice(BELOW)
            if name != 'air':
                h.put(cells, below, front_state(states, rng, name))
        scene = rng.choice(SCENES[bare]) if bare in SCENES and rng.random() < 0.45 else None
        # A few random blocks around (portal frames, fire supports, neighbours).
        for _ in range(rng.randrange(0, 6)):
            at = (rng.randrange(-2, 3), rng.randrange(-2, 3), rng.randrange(-2, 3))
            if at in ((0, 0, 0), front):
                continue
            h.put(cells, at, states.get(rng.choice(['stone', 'obsidian', 'dirt', 'water', 'soul_sand', 'grass_block', 'oak_planks'])))
        if scene:
            build_scene(cells, states, rng, scene, front, facing)
        items = []
        slots = rng.sample(range(9), rng.choice([1, 1, 1, 2, 3, 9]))
        for i, slot in enumerate(slots):
            items.append([0, 0, 0, slot, item if i == 0 or rng.random() < 0.6 else plain('cobblestone', rng.randrange(1, 65))])
        case = {**h.environment(rng), 'mode': 'dispense', 'pos': [0, 0, 0], 'cells': cells, 'items': items}
        case.pop('raining', None)
        case.pop('thundering', None)
        if rng.random() < 0.08:
            case['rules'] = {'tnt_explodes': False}
        if kind in ('equipment', 'swallow') or (bare in TARGETS and rng.random() < 0.5):
            entities = []
            for _ in range(rng.choice([1, 1, 1, 2])):
                target = {'type': 'minecraft:' + rng.choice(TARGETS.get(bare, ['zombie'])),
                          'pos': [dx + 0.5, dy + rng.choice([0.0, 0.0, 0.25]), dz + 0.5], 'yaw': rng.choice([0.0, 90.0, 180.0])}
                for flag, odds in (('baby', 0.2), ('tame', 0.6), ('sheared', 0.25), ('chest', 0.15), ('loot', 0.5), ('brown', 0.3)):
                    if rng.random() < odds:
                        target[flag] = True
                if target['type'] == 'minecraft:sulfur_cube' and rng.random() < 0.5:
                    target['equip'] = {'body': plain(rng.choice(SWALLOWED))}
                if target['type'] == 'minecraft:copper_golem' and rng.random() < 0.7:
                    # The poppy an iron golem offered sits in the antenna (saddle) slot.
                    target['equip'] = {'saddle': plain(rng.choice(['poppy', 'poppy', 'dandelion']))}
                # Leads: tied to an earlier target, or that target holds this one's.
                holders = [i for i, e in enumerate(entities) if e['type'] != 'minecraft:armor_stand']
                if holders and rng.random() < 0.3:
                    target['leash'] = rng.choice(holders)
                entities.append(target)
            if bare == 'shears' and len(entities) == 1 and rng.random() < 0.3:
                # A holder out of the dispenser's reach whose lead runs to the target.
                entities.append({'type': 'minecraft:' + rng.choice(['cow', 'wolf']), 'pos': [dx + 3.5, dy, dz + 3.5], 'yaw': 0.0})
                entities[0]['leash'] = 1
            case['entities'] = entities
        if block == 'dropper' and rng.random() < 0.5:
            h.put(cells, front, states.get(rng.choice(['chest', 'hopper', 'barrel', 'dispenser'])))
            if rng.random() < 0.5:
                items.append([dx, dy, dz, 0, plain(rng.choice(['stone', 'diamond', 'stick']), rng.randrange(1, 65))])
        yield case


# Items whose behaviour runs Mob.finalizeSpawn (spawn eggs, mob buckets). The
# finalization draws belong to the entity spawning port; for these cases the
# level random's end state is not compared, everything else is.
FINALIZED = ('_spawn_egg', 'salmon_bucket', 'cod_bucket', 'pufferfish_bucket', 'tropical_fish_bucket', 'axolotl_bucket',
             'tadpole_bucket', 'sulfur_cube_bucket')


def finalizes(case):
    return any(any(tag in i[4] for tag in FINALIZED) for i in case['items'] if i[:3] == [0, 0, 0])

def spawned_type(case):
    item = next(i[4] for i in case['items'] if i[:3] == [0, 0, 0]).split('"')[1].split(':')[1]
    return 'minecraft:' + item.removesuffix('_spawn_egg').removesuffix('_bucket')


def rider(line, spawned):
    parts = line.split(' ')
    return parts[0] == 'N' and parts[1] not in (spawned, 'minecraft:item')


def voice(line):
    parts = line.split(' ')
    if parts[0] == 'S' and len(parts) == 3 and parts[2].lstrip('~').startswith('minecraft:entity.'):
        parts[2] = 'entity-voice'
    return ' '.join(parts)


def run(command, **kwargs):
    runner = run_game if str(command[0]).endswith('/horizon-client') else subprocess.run
    done = runner(command, capture_output=True, text=True, **kwargs)
    if done.returncode:
        print((done.stdout + done.stderr)[-4000:])
        raise SystemExit(f'{command[0]} failed')
    return done


def numbers_close(java, rust):
    """Token-wise equality; a "~" token is an unseeded entity draw and only needs to fall in its spread."""
    a, b = java.split(' '), rust.split(' ')
    if len(a) != len(b):
        return False
    for x, y in zip(a, b):
        if x.startswith('~'):
            try:
                # An item's own motion spans 0.2 per axis, and a shearing
                # sheep pushes each wool with its own random by up to 0.1 more.
                if abs(float(x[1:]) - float(y)) > 0.4:
                    return False
            except ValueError:
                return False
        elif x != y:
            return False
    return True


def same(java, rust):
    java = sorted(java)
    rust = sorted(rust)
    if len(java) != len(rust):
        return False
    pending = list(rust)
    for line in java:
        if line in pending:
            pending.remove(line)
            continue
        if '~' not in line:
            return False
        match = next((r for r in pending if numbers_close(line, r)), None)
        if match is None:
            return False
        pending.remove(match)
    return True


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 6000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'dispenser-parity'
    work.mkdir(parents=True, exist_ok=True)
    all_cases = list(cases(facts, args.count, args.seed, args.only))
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in all_cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
         '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaDispenserOracle.java')])
    run(['java', '-Xmx3G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaDispenserOracle',
         str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    run(['cargo', 'build', '-q', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
        env={**os.environ, 'CARGO_BUILD_JOBS': os.environ.get('CARGO_BUILD_JOBS', '3')})
    run([str(lab.cargo_target() / 'fast/horizon-client'), '--resources', str(resources)], cwd=work,
        env=lab.game_env(HORIZON_DISPENSE_FIXTURE=f'{work / "cases.jsonl"},{work / "rust.txt"}'))
    # Loot table names (D) are vanilla's bookkeeping; the drops themselves are compared.
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix) + ('D ',)
    native = h.split_cases((work / 'native.txt').read_text(), ignore)
    rust = h.split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(all_cases), (len(native), len(rust), len(all_cases))
    groups, errors = {}, {}
    unseeded = 0
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        if finalizes(case):
            unseeded += 1
            # Riders finalizeSpawn adds (a baby zombie's chicken, a spider's
            # skeleton) depend on whether the mob came out a baby and on the
            # jockey odds, both finalization draws: only the egg's own mob is compared.
            spawned = spawned_type(case)
            a = [line for line in a if not rider(line, spawned)]
            b = [line for line in b if not rider(line, spawned)]
            # EntityType.spawn plays the new mob's ambient sound: which one (a
            # wolf pants or barks, in its sound variant) comes from the mob's
            # own unseeded random, so only that a voice plays, and where, is compared.
            a = [voice(line) for line in a if not line.startswith('rng ')]
            b = [voice(line) for line in b if not line.startswith('rng ')]
        if not same(a, b):
            item = next(i[4] for i in case['items'] if i[:3] == [0, 0, 0])
            block = facts.states[case['cells'][0][3]][0].split(':')[1]
            groups.setdefault(f'{block} {item.split(chr(34))[1]}', []).append((case, a, b))
    for kind, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {kind}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + describe(case, facts))
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors; '
          f'{unseeded} spawning cases compared without the final level random)')
    (work / 'differences.json').write_text(json.dumps({k: [[c, a, b] for c, a, b in v[:5]] for k, v in groups.items()}))
    h.finish(total + skipped)


def describe(case, facts):
    cells = ' '.join(f'{c[0]},{c[1]},{c[2]}={facts.describe(c[3])}' for c in case['cells'])
    extra = ''
    if case.get('entities'):
        extra += ' entities=' + json.dumps(case['entities'])
    if case.get('rules'):
        extra += ' rules=' + json.dumps(case['rules'])
    return f'{cells} items={json.dumps(case["items"])}{extra} seed={case["seed"]}'


if __name__ == '__main__':
    main()
