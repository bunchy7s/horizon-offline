"""Compare a player's melee hit with a mace (and other weapons) with vanilla's.

Every case has a stone floor, a target mob standing on it or hanging in the
air (health raised so no hit kills it, sometimes in armor) or an armor stand
(a living target whose own `hurtServer` wobbles on a first punch and breaks on
a second within five ticks; its hit still runs `itemAttackInteraction`, so a
smash knocks the creatures around it away and a Wind Burst launches the
attacker), other mobs and armor stands around it, and a player near the target holding a mace with
Density, Breach, Wind Burst and Unbreaking in any mix, or a sword, an axe, a
stick or nothing. The player has fallen some distance (sometimes gliding or
flying), moves, and may still have the impact point of an earlier impulse.

Vanilla runs, over the headless level of the dispenser harness
(`VanillaDispenserOracle.attack`), the steps of `Player.attack` after the
attack strength scaled the base damage: `Item.getAttackDamageBonus`
(`MaceItem`'s smash bonus with Density), the item's damage source
(`mace_smash` or `player_attack`), the target's real `hurtServer` (armor
thinned by Breach in `CombatRules.getDamageAfterAbsorb`), then
`itemAttackInteraction`: `ItemStack.hurtEnemy` (`MaceItem.hurtEnemy`: the
stopped fall, the waived fall damage, the smash sounds, the dust-pillar level
event and the knockback around the target), the post-attack enchantments
(Wind Burst's `ExplodeEffect` on the player) and `postHurtEnemy` (the fall
reset and the weapon's wear with Unbreaking). Horizon runs the same steps
through the helpers `Game::attack` uses (`smash_damage_bonus`,
`hurt_by_player`, `weapon_hit`) in a headless game
(`HORIZON_DISPENSE_FIXTURE`). Both print every target's health and
velocity, the player's velocity, fall distance, impulse impact and grace
time and its landing-particle flag, sounds, level and game events, the used
statistic, the held stack and the level random afterwards.
"""
import json
import math
import os
import random
import re
try:
    from . import block_update_harness as h, lab, test_dispenser_parity as d, test_rust_parity as base
except ImportError:
    import block_update_harness as h
    import lab
    import test_dispenser_parity as d
    import test_rust_parity as base

TARGETS = ['pig', 'cow', 'sheep', 'zombie', 'husk', 'skeleton', 'zombie', 'pig']
AROUND = ['pig', 'cow', 'zombie', 'sheep', 'armor_stand']
ARMOR = ['iron', 'diamond', 'golden', 'chainmail', 'leather']
FALLS = [0.0, 0.5, 1.0, 1.5, 1.5000001, 2.0, 2.75, 3.0, 3.25, 5.0, 5.0001, 6.5, 8.0, 8.5, 12.0, 25.3, 60.0]


def stack(item, enchantments=None, damage=0):
    components = []
    if enchantments:
        levels = ','.join(f'"minecraft:{e}":{lvl}' for e, lvl in enchantments.items())
        components.append('"minecraft:enchantments":{%s}' % levels)
    if damage:
        components.append(f'"minecraft:damage":{damage}')
    extra = ',components:{%s}' % ','.join(components) if components else ''
    return '{id:"minecraft:%s",count:1%s}' % (item, extra)


def weapon(rng):
    roll = rng.random()
    if roll < 0.75:
        enchantments = {}
        for name, top in [('density', 5), ('breach', 4), ('wind_burst', 3), ('unbreaking', 3)]:
            if rng.random() < 0.35:
                enchantments[name] = rng.randint(1, top)
        return stack('mace', enchantments, rng.choice([0, 0, 0, 7, 300]))
    if roll < 0.85:
        return stack(rng.choice(['diamond_sword', 'iron_axe', 'stone_pickaxe']),
                     {'breach': 2} if rng.random() < 0.2 else None, rng.choice([0, 0, 10]))
    return rng.choice([stack('stick'), '-'])


def cases(facts, count, seed):
    rng = random.Random(seed)
    states = d.States(facts)
    floors = [states.get(b) for b in ['stone', 'grass_block', 'dirt', 'sand', 'oak_planks']]
    for n in range(count):
        case = {**h.environment(rng), 'pos': [0, 0, 0]}
        case.pop('raining', None)
        case.pop('thundering', None)
        floor = rng.choice(floors)
        case['cells'] = [[x, -1, z, floor] for x in range(-6, 7) for z in range(-6, 7)]
        if rng.random() < 0.2:
            # A wall the burst and the dust have to get past.
            wall = states.get(rng.choice(['stone', 'glass', 'oak_planks']))
            x = rng.choice([-3, -2, 2, 3])
            case['cells'] += [[x, y, z, wall] for y in range(0, 3) for z in range(-2, 3)]
        item = weapon(rng)
        wind = 'wind_burst' in item
        grounded = rng.random() < 0.75
        tx, tz = rng.uniform(-0.5, 0.5), rng.uniform(-0.5, 0.5)
        ty = 0.0 if grounded else rng.choice([0.5, 1.25, 2.0])
        stand = rng.random() < 0.3
        target = {'type': 'minecraft:armor_stand' if stand else 'minecraft:' + rng.choice(TARGETS),
                  'pos': [tx, ty, tz], 'yaw': rng.uniform(-180, 180)}
        if stand:
            # ArmorStand.hurtServer wobbles on a first punch and breaks on a
            # second one within five ticks of the last (lastHit).
            target['last_hit'] = case['time'] - rng.choice([1, 3, 5, 6, 10, 1000])
        else:
            target['health'] = 1000.0
        if grounded:
            target['ground'] = True
        if target['type'] in ('minecraft:zombie', 'minecraft:husk', 'minecraft:skeleton') and rng.random() < 0.6:
            material = rng.choice(ARMOR)
            target['equip'] = {'chest': d.plain(f'{material}_chestplate')}
            if rng.random() < 0.5:
                target['equip']['head'] = d.plain(f'{material}_helmet')
        if not stand and rng.random() < 0.15:
            target['knockback_resistance'] = rng.choice([0.25, 0.5, 1.0])
        entities = [target]
        for _ in range(rng.choice([0, 1, 2, 3, 5])):
            kind = rng.choice(AROUND)
            near = rng.random() < 0.8
            spread = 3.2 if near else 5.0
            e = {'type': 'minecraft:' + kind, 'pos': [tx + rng.uniform(-spread, spread), rng.choice([0.0, 0.0, 0.5, 1.0]),
                                                      tz + rng.uniform(-spread, spread)], 'yaw': rng.uniform(-180, 180)}
            if kind == 'armor_stand':
                # A marker is spared by the smash; Wind Burst's explosion would
                # need its zero-size box.
                if not wind and rng.random() < 0.4:
                    e['marker'] = True
            else:
                e['health'] = 1000.0
                if rng.random() < 0.2:
                    e['knockback_resistance'] = rng.choice([0.4, 1.0])
            if e['pos'][1] == 0.0 and rng.random() < 0.8:
                e['ground'] = True
            entities.append(e)
        case['entities'] = entities
        angle = rng.uniform(0, 6.283)
        reach = rng.uniform(0.6, 2.8)
        px, pz = tx + math.cos(angle) * reach, tz + math.sin(angle) * reach
        py = ty + rng.choice([0.0, 0.3, 1.0, 1.7, 2.5])
        fall = rng.choice(FALLS) if rng.random() < 0.7 else round(rng.uniform(0, 30), 3)
        attack = {'item': item, 'creative': rng.random() < 0.15, 'player': [px, py, pz],
                  'motion': [rng.uniform(-0.3, 0.3), -rng.uniform(0, 2.5), rng.uniform(-0.3, 0.3)],
                  'fall': fall, 'fall_flying': rng.random() < 0.08, 'flying': rng.random() < 0.04,
                  'base': round(rng.uniform(1.2, 7.0), 3)}
        if rng.random() < 0.3:
            attack['impact'] = [px + rng.uniform(-1, 1), py + rng.choice([-3.0, -0.5, 0.0, 2.0]), pz + rng.uniform(-1, 1)]
        case['attack'] = attack
        case['items'] = []
        case['mode'] = 'dispense'
        yield case


def kind(case):
    item = case['attack']['item']
    name = item.split('"')[1].split(':')[1] if item != '-' else 'empty hand'
    smash = case['attack']['fall'] > 1.5 and not case['attack']['fall_flying']
    if name == 'mace':
        name += ' smash' if smash else ' hit'
        if 'wind_burst' in item:
            name += ' with wind burst'
    if case['entities'][0]['type'] == 'minecraft:armor_stand':
        name += ' on an armor stand'
    return name


def weapon_part(lines):
    """A hit armor stand's lines without what its own `hurtServer` adds.

    The weapon's part is what this harness compares: the smash, Wind Burst
    and the wear. The stand's wobble and break have their own effects, which
    Horizon does not copy yet: the `entity_damage` / `entity_die` game events,
    the hit sound (vanilla's client plays it for entity event 32), the stand
    item the break pops (place, motion and random draws) and so the random
    stream after a break, which Unbreaking's roll for the weapon's wear
    reads: the held stack's damage is left out then.
    """
    broke = any(line.startswith('T 0 ') and line.endswith(' removed') for line in lines)

    def own(line):
        return ((line.startswith('G ') and line.endswith(('minecraft:entity_damage', 'minecraft:entity_die')))
                or (line.startswith('N minecraft:item ') and '"minecraft:armor_stand"' in line)
                or (line.startswith('S ') and line.endswith('minecraft:entity.armor_stand.hit'))
                or (broke and line.startswith('rng ')))

    def held(line):
        if broke and line.startswith('held '):
            line = re.sub(r'minecraft:damage:\d+,?', '', line).replace('components:{},', '')
        return line
    return [held(line) for line in lines if not own(line)]


def main():
    parser = h.argument_parser(__doc__)
    args = parser.parse_args()
    if args.count == 40000:
        args.count = 3000
    lock, jars, libraries = base.verify_artifacts()
    facts = h.load_facts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / 'mace-parity'
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
    smashed = burst = 0
    for case, a, b in zip(all_cases, native, rust):
        if a and a[0].startswith('error'):
            errors.setdefault(a[0], []).append(case)
            continue
        smashed += any('source=minecraft:mace_smash hurt=true' in line for line in a)
        burst += any(line.endswith('minecraft:entity.wind_charge.wind_burst') for line in a)
        if case['entities'][0]['type'] == 'minecraft:armor_stand':
            a, b = weapon_part(a), weapon_part(b)
        if not d.same(a, b):
            groups.setdefault(kind(case), []).append((case, a, b))
    for name, entries in sorted(errors.items(), key=lambda e: -len(e[1])):
        print(f'oracle error x{len(entries)}: {name}')
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case entities=' + json.dumps(case['entities']) + ' attack=' + json.dumps(case['attack']) + f' seed={case["seed"]}')
            print('   native ' + ' | '.join(sorted(a)))
            print('   rust   ' + ' | '.join(sorted(b)))
    total = sum(len(v) for v in groups.values())
    skipped = sum(len(v) for v in errors.values())
    print(f'{len(all_cases) - total - skipped}/{len(all_cases) - skipped} cases match ({skipped} oracle errors); '
          f'vanilla smashed in {smashed} and burst in {burst}')
    h.finish(total + skipped)


if __name__ == '__main__':
    main()
