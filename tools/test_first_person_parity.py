#!/usr/bin/env python3
"""Compare the local player's hand with the pinned client, tick by tick over
scripted clicks: Minecraft.startAttack, continueAttack and startUseItem with
handleKeybinds' attack and use part (missTime, rightClickDelay, held
buttons), MultiPlayerGameMode's block breaking (survival progress, creative
instant breaking and its destroy delay, swords that cannot break in
creative, stopping), LivingEntity.swing with SwingState and haste or mining
fatigue durations, Player's attack strength and item swap tickers, and
FirstPersonHandsAndItems' equip heights (swap animation, itemUsed after
placing or using an item, busy hands)."""
import json
import random
import struct

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def f32(v):
    return struct.unpack('>f', struct.pack('>f', v))[0]


# What the probe needs about the scripted blocks and items, checked against
# the client through the progress and attack speed both print:
# Block.destroyTime and requiresCorrectToolForDrops,
BLOCKS = {'minecraft:dirt': (0.5, False), 'minecraft:stone': (1.5, True), 'minecraft:glass': (0.3, False),
          'minecraft:bedrock': (-1.0, False), 'minecraft:short_grass': (0.0, False), 'minecraft:oak_planks': (2.0, False)}
# the Tool component's speed and correct-for-drops per item and block (1.0
# and incorrect otherwise),
TOOLS = {'minecraft:diamond_pickaxe minecraft:stone': (8.0, True), 'minecraft:diamond_shovel minecraft:dirt': (8.0, True),
         'minecraft:diamond_pickaxe minecraft:glass': (1.0, False)}
# and ATTACK_SPEED with the main-hand item's modifier (4.0 without one).
SPEEDS = {'minecraft:diamond_sword': 4.0 + f32(-2.4), 'minecraft:diamond_pickaxe': 4.0 + f32(-2.8),
          'minecraft:diamond_shovel': 4.0 + f32(-3.0)}
ITEMS = [None, ['minecraft:diamond_sword', 1], ['minecraft:diamond_pickaxe', 1], ['minecraft:diamond_shovel', 1],
         ['minecraft:dirt', 64], ['minecraft:dirt', 2], ['minecraft:bread', 3], ['minecraft:bow', 1], ['minecraft:shield', 1]]
TARGETS = [(0, 64, 0), (1, 64, 0), (0, 65, 2)]


def step(hit=('miss',), ac=0, uc=0, ad=False, ud=False, busy=False, using=False, uses=None, shrink=False, set=None,
         rot=None):
    """rot: the player's (xRot, yRot) from this step on (LocalPlayer.applyInput's hand sway)."""
    return dict(hit=list(hit), ac=ac, uc=uc, ad=ad, ud=ud, busy=busy, using=using,
                uses=uses or dict(entity='pass', block='pass', item='pass'), shrink=shrink, set=set,
                rot=None if rot is None else [lab.fbits(rot[0]), lab.fbits(rot[1])])


def case(steps, creative=False, main=None, off=None, blocks=None, haste=None, fatigue=None):
    return dict(creative=creative, main=main, off=off, haste=haste, fatigue=fatigue,
                blocks=[[*p, b] for p, b in (blocks or {}).items()], steps=steps,
                rust=dict(blocks={k: [lab.fbits(v[0]), v[1]] for k, v in BLOCKS.items()},
                          tools={k: [lab.fbits(v[0]), v[1]] for k, v in TOOLS.items()},
                          attack_speed={k: lab.dbits(v) for k, v in SPEEDS.items()}))


def block_hit(pos, face=1):
    return ('block', *pos, face)


def scenarios():
    stone = {TARGETS[0]: 'minecraft:stone', TARGETS[1]: 'minecraft:dirt', TARGETS[2]: 'minecraft:glass'}
    # Consecutive instant survival breaks have no destroy delay. Cover both
    # zero hardness and a shovel with haste reaching one-tick destroy progress.
    for block, main, haste in [('minecraft:short_grass', None, None),
                                ('minecraft:dirt', ['minecraft:diamond_shovel', 1], 9)]:
        steps = [step(block_hit(p), ac=int(i == 0), ad=True) for i, p in enumerate(TARGETS)]
        yield case(steps + [step()] * 6, main=main, blocks={p: block for p in TARGETS}, haste=haste)
    for creative in (False, True):
        for main in ITEMS:
            # Hold attack on blocks until they break, then on the air behind.
            for target, block in [(TARGETS[1], 'minecraft:dirt'), (TARGETS[0], 'minecraft:stone'), (TARGETS[0], 'minecraft:bedrock'),
                                  (TARGETS[2], 'minecraft:short_grass'), (TARGETS[2], 'minecraft:glass')]:
                steps = [step(block_hit(target), ac=1, ad=True)] + [step(block_hit(target), ad=True)] * 40 + [step()] * 8
                yield case(steps, creative, main, blocks={target: block})
            # Sweep across three blocks while holding, as when mining a tunnel.
            steps = [step(block_hit(TARGETS[0]), ac=1, ad=True)]
            for i in range(60):
                steps.append(step(block_hit(TARGETS[(i // 7) % 3]), ad=True))
            yield case(steps + [step()] * 6, creative, main, blocks=stone)
            # Clicking the air, quickly and slowly; and clicking an entity.
            for gap in (0, 1, 2, 3, 5, 9, 10, 11, 15):
                steps = []
                for _ in range(5):
                    steps += [step(ac=1)] + [step()] * gap
                yield case(steps + [step()] * 8, creative, main)
                steps = []
                for _ in range(5):
                    steps += [step(('entity', True), ac=1)] + [step(('entity', True))] * gap
                yield case(steps + [step(('entity', True), ac=1, ad=True)] + [step(('entity', True), ad=True)] * 8, creative, main)
            # Placing and using: held right button on a block and in the air.
            for result in ('success', 'server', 'consume', 'pass', 'fail'):
                for shrink in (False, True):
                    uses = dict(entity=result, block=result, item=result)
                    steps = [step(block_hit(TARGETS[0]), uc=1, ud=True, uses=uses, shrink=shrink)]
                    steps += [step(block_hit(TARGETS[0]), ud=True, uses=uses, shrink=shrink)] * 14 + [step(uses=uses)] * 8
                    yield case(steps, creative, main, off=['minecraft:shield', 1], blocks=stone)
                    steps = [step(uc=1, ud=True, uses=uses, shrink=shrink)] + [step(ud=True, uses=uses, shrink=shrink)] * 10
                    yield case(steps + [step(uses=uses)] * 8, creative, main, blocks=stone)
                    steps = [step(('entity', inr), uc=1, ud=True, uses=uses, shrink=shrink) for inr in (True, False, True)]
                    yield case(steps + [step(('entity', True), uses=uses)] * 8, creative, main)
            # Eating or drawing a bow: using the item, then releasing.
            uses = dict(entity='pass', block='pass', item='consume')
            steps = [step(uc=1, ud=True, uses=uses)] + [step(ud=True, using=True, ac=k % 2, ad=True, uses=uses) for k in range(12)]
            steps += [step(using=True, uses=uses), step(uses=uses)] + [step()] * 8
            yield case(steps, creative, main, off=['minecraft:bread', 3])
    # Changing the held item: other items, the same item with another count,
    # emptying the hand, while the attack recharges or the hands are busy.
    for a in ITEMS:
        for b in ITEMS:
            for busy in (False, True):
                steps = [step()] * 25 + [step(set=dict(hand='main', stack=b), busy=busy)] + [step(busy=busy)] * 25
                steps += [step(set=dict(hand='off', stack=a))] + [step()] * 12 + [step(ac=1)] + [step()] * 30
                yield case(steps, False, a)
    # Haste and mining fatigue change the swing's length.
    for haste in (None, 0, 1, 2, 4):
        for fatigue in (None, 0, 1, 2):
            steps = [step(block_hit(TARGETS[1]), ac=1, ad=True)] + [step(block_hit(TARGETS[1]), ad=True)] * 60 + [step(ac=1)] + [step()] * 20
            yield case(steps, False, ['minecraft:diamond_shovel', 1], blocks={TARGETS[1]: 'minecraft:dirt'}, haste=haste, fatigue=fatigue)
            yield case(steps, True, None, blocks={TARGETS[1]: 'minecraft:dirt'}, haste=haste, fatigue=fatigue)


def turning_cases():
    """The hands lag behind the view: turns, snaps, a yaw past 180 and back, pitch to its limits."""
    for main in (None, ['minecraft:diamond_sword', 1]):
        steps = [step(rot=(0.0, 0.0))]
        steps += [step(rot=(0.0, 4.5 * i)) for i in range(1, 40)]
        steps += [step()] * 6
        steps += [step(rot=(90.0, 179.5)), step(rot=(-90.0, 185.25)), step(rot=(-12.75, 900.0))] + [step()] * 8
        steps += [step(rot=(30.0 - i, -720.0 + 33.3 * i), ac=i % 2, ad=True) for i in range(30)] + [step()] * 4
        yield case(steps, False, main)


def random_cases(rng):
    results = ['success', 'server', 'consume', 'pass', 'fail']
    for _ in range(3000):
        blocks = {p: rng.choice(list(BLOCKS) + ['minecraft:air']) for p in TARGETS}
        steps = []
        for _ in range(rng.randrange(5, 80)):
            rot = (rng.uniform(-90.0, 90.0), rng.uniform(-1080.0, 1080.0)) if rng.random() < 0.3 else None
            hit = rng.choice([('miss',), ('entity', rng.random() < 0.7)] + [block_hit(p, rng.randrange(6)) for p in TARGETS] * 2)
            setting = None
            if rng.random() < 0.04:
                setting = dict(hand=rng.choice(['main', 'off']), stack=rng.choice(ITEMS))
            steps.append(step(hit, ac=rng.choice([0] * 6 + [1, 1, 2]), uc=rng.choice([0] * 8 + [1, 2]), ad=rng.random() < 0.5,
                              ud=rng.random() < 0.3, busy=rng.random() < 0.05, using=rng.random() < 0.08,
                              uses=dict(entity=rng.choice(results), block=rng.choice(results), item=rng.choice(results)),
                              shrink=rng.random() < 0.3, set=setting, rot=rot))
        yield case(steps, rng.random() < 0.4, rng.choice(ITEMS), rng.choice(ITEMS), blocks,
                   rng.choice([None] * 6 + [0, 1, 3]), rng.choice([None] * 6 + [0, 2]))


def make_cases(build):
    rng = random.Random(0x48414e44)
    path = build / 'cases.jsonl'
    cases = build / 'cases'
    cases.mkdir(exist_ok=True)
    count = 0
    with path.open('w') as out:
        for generator in (scenarios(), turning_cases(), random_cases(rng)):
            for c in generator:
                text = json.dumps(c, separators=(',', ':'))
                out.write(text + '\n')
                if count < 20000:
                    (cases / str(count)).write_text(text)
                count += 1
    return path, count


def main():
    scope = ('Real Minecraft.startAttack, continueAttack and startUseItem (with the attack and use part of handleKeybinds and '
             'missTime mirrored in order), MultiPlayerGameMode.startDestroyBlock, continueDestroyBlock, stopDestroyBlock, '
             'destroyBlock and attack, LivingEntity.swing with SwingState and getModifiedSwingDuration under haste and mining '
             'fatigue, Player.getItemSwapScale and resetAttackStrengthTicker with the ticker lines of Player.tick, and '
             'FirstPersonHandsAndItems.tick and itemUsed, on a LocalPlayer shell with a real Inventory, effects and attribute '
             'modifiers, over scripted clicks, held buttons, crosshair targets, item changes and use results. Level effects '
             '(particles, game events), packets and the outcome of using items on blocks, entities and the air are scripted '
             'or absorbed; every vanilla item plays the swap animation unscaled except spears, which are not scripted. Steps also '
             'turn the player: the real LocalPlayer.applyInput moves xBob and yBob, the hands\' lag behind the view.')
    harness.differential(
        'rust-first-person-parity', 'VanillaFirstPersonOracle', 'first_person_probe', make_cases, scope,
        sources=[lab.ROOT / 'crates/horizon-client/src/first_person.rs', lab.ROOT / 'crates/horizon-client/src/attack_strength.rs',
                 lab.ROOT / 'crates/horizon-client-render/src/camera.rs',
                 lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java'])


if __name__ == '__main__':
    main()
