#!/usr/bin/env python3
"""Compare entity model geometry and setupAnim poses with the vanilla 26.3 client.

The Java oracle bakes each vanilla model layer, builds the model's own render
state class with the generated walk/age/head/attack inputs (plus per-species
state fields), runs the real `setupAnim` and writes every part's pose.
`examples/probe/entity_anim_fixture` replays the same inputs through Horizon's
`mobs::anim::setup` and the loaded model tree; the two must agree on every part
pose and on the baked polygon count and vertex sums.

Both sides then draw the posed model under LivingEntityRenderer's root transform
(vanilla: ModelPart.render's PoseStack walk and Cube.compile into a capturing
VertexConsumer; Horizon: `mobs::render::draw`) and compare every vertex's normal
as the signed bytes each puts in its vertex buffer: vanilla's BufferBuilder.setNormal
truncates clamp(c) * 127 toward zero (normalIntValue) for the entity format's byte
normal, Horizon packs its RGBA8_SNORM `Vertex.normal` the same way. A one-step
difference is accepted only where vanilla's float lies within --normal-edge of a
truncation step (float rounding); any other difference fails. A checksum of each
drawn part's posed positions is compared too.

  test_entity_model_parity.py [--filter SUBSTRING] [--detail LAYER] [--tolerance EPS]
                              [--normal-edge EPS] [--position-tolerance EPS]
"""
import argparse
import json
import math
import os
from pathlib import Path
import random
import re
import subprocess
import sys

try:
    from . import lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, test_rust_parity as base, test_model_baking_parity as baking

CLIENT = lab.GAME / 'sources' / 'client' / 'net' / 'minecraft' / 'client'

# Models that no renderer builds with a plain `new Model(context.bakeLayer(CONSTANT))`.
EXTRAS = [
    ('minecraft:zombie#main', 'ZombieModel'), ('minecraft:zombie_baby#main', 'BabyZombieModel'),
    ('minecraft:husk#main', 'ZombieModel'), ('minecraft:husk_baby#main', 'BabyZombieModel'),
    ('minecraft:zombie_villager#main', 'ZombieVillagerModel'),
    ('minecraft:zombie_villager_baby#main', 'BabyZombieVillagerModel'),
    ('minecraft:zombified_piglin#main', 'AdultZombifiedPiglinModel'),
    ('minecraft:zombified_piglin_baby#main', 'BabyZombifiedPiglinModel'),
    ('minecraft:skeleton#main', 'SkeletonModel'), ('minecraft:stray#main', 'SkeletonModel'),
    ('minecraft:wither_skeleton#main', 'SkeletonModel'),
    ('minecraft:spider#main', 'SpiderModel'), ('minecraft:cave_spider#main', 'SpiderModel'),
    ('minecraft:enderman#main', 'EndermanModel'), ('minecraft:endermite#main', 'EndermiteModel'),
    ('minecraft:piglin#main', 'AdultPiglinModel'), ('minecraft:piglin_brute#main', 'AdultPiglinModel'),
    ('minecraft:piglin_baby#main', 'BabyPiglinModel'),
    ('minecraft:hoglin#main', 'HoglinModel'), ('minecraft:zoglin#main', 'HoglinModel'),
    ('minecraft:hoglin_baby#main', 'BabyHoglinModel'), ('minecraft:zoglin_baby#main', 'BabyHoglinModel'),
    ('minecraft:guardian#main', 'GuardianModel'), ('minecraft:elder_guardian#main', 'GuardianModel'),
    ('minecraft:evoker#main', 'IllagerModel'), ('minecraft:illusioner#main', 'IllagerModel'),
    ('minecraft:pillager#main', 'IllagerModel'), ('minecraft:vindicator#main', 'IllagerModel'),
    ('minecraft:llama#main', 'LlamaModel'), ('minecraft:trader_llama#main', 'LlamaModel'),
    ('minecraft:llama_baby#main', 'BabyLlamaModel'), ('minecraft:trader_llama_baby#main', 'BabyLlamaModel'),
    # LlamaDecorLayer: the carpet and the trader's blanket.
    ('minecraft:llama#decor', 'LlamaModel'), ('minecraft:llama_baby#decor', 'BabyLlamaModel'),
    ('minecraft:donkey#main', 'DonkeyModel'), ('minecraft:mule#main', 'DonkeyModel'),
    ('minecraft:donkey_baby#main', 'BabyDonkeyModel'), ('minecraft:mule_baby#main', 'BabyDonkeyModel'),
    ('minecraft:skeleton_horse#main', 'HorseModel'), ('minecraft:zombie_horse#main', 'HorseModel'),
    ('minecraft:skeleton_horse_baby#main', 'BabyHorseModel'), ('minecraft:zombie_horse_baby#main', 'BabyHorseModel'),
    ('minecraft:cave_spider#main', 'SpiderModel'),
    ('minecraft:stray#main', 'SkeletonModel'),
    # BreezeWindLayer and BreezeEyesLayer bake their own layers into a BreezeModel;
    # the creaking's emissive eyes layer is a CreakingModel.
    ('minecraft:breeze#wind', 'BreezeModel'), ('minecraft:breeze#eyes', 'BreezeModel'),
    ('minecraft:creaking#eyes', 'CreakingModel'),
    # The warden's emissive layers: its model over each layer's own parts.
    ('minecraft:warden#tendrils', 'WardenModel'), ('minecraft:warden#heart', 'WardenModel'),
    ('minecraft:warden#bioluminescent', 'WardenModel'), ('minecraft:warden#pulsating_spots', 'WardenModel'),
]
# Layers a renderer reuses for another entity under a different constant.
SKIP_LAYERS = re.compile(r'(banner|bell|book|skull|head|chest|shulker_box|boat|cushion|conduit|leash|arrow|trident|'
                         r'shield|elytra|spin_attack|ears|cape|decorated_pot|minecart|wind_charge|evoker_fangs|'
                         r'llama_spit|shulker_bullet|bee_stinger|armor_stand|copper_golem_\w+|#(helmet|chestplate|leggings|'
                         r'boots|outer|armor|decor|saddle|collar|wool|wool_undercoat|eyes|wind|inner|pattern|tendrils|'
                         r'heart|bioluminescent|pulsating_spots|water_patch|flag|cage|shell)$)')


def layer_constant(name):
    """minecraft:cow_baby#main -> constant COW_BABY (the main layer of a model)."""
    return name.split(':')[1].split('#')[0].upper()


def renderer_pairs():
    pairs = {}
    pattern = re.compile(r'new (\w+)\(\s*(?:context|this\.context|\w*)\.bakeLayer\(ModelLayers\.(\w+)\)')
    for path in sorted((CLIENT / 'renderer').rglob('*.java')):
        for match in pattern.finditer(path.read_text()):
            pairs[match.group(2)] = match.group(1)
    return pairs


def layers(models):
    chosen = {}
    pairs = renderer_pairs()
    for key in models:
        if not key.endswith('#main') or SKIP_LAYERS.search(key):
            continue
        constant = layer_constant(key)
        if constant in pairs:
            chosen[key] = pairs[constant]
    for key, cls in EXTRAS:
        if key in models:
            chosen[key] = cls
    return chosen


# The renderer's root transform for the normal comparison (keys starting with
# '@' are not render-state fields): LivingEntityRenderer's body yaw, and the
# renderer's own scale (uniform like an age scale, or non-uniform like
# CreeperRenderer's swell, which makes PoseStack.Pose's normals untrusted).
ROOT_YAWS = [0.0, 37.5, -112.25, 90.0, 180.0, 213.7, -45.0]
ROOT_SCALES = [[1.0, 1.0, 1.0], [1.0, 1.0, 1.0], [0.5, 0.5, 0.5], [1.18, 0.92, 1.18], [1.0, 1.0, 1.0]]
# No vanilla setupAnim scales a part negatively, so '@partScale' sets one part's scale after
# setupAnim on both sides: PoseStack.Pose.scale flips the normal matrix's signs when |x|=|y|=|z|
# (trusted), and scales it by 1/x, 1/y, 1/z and normalises otherwise.
PART_SCALES = [[-1.0, -1.0, -1.0], [-1.0, 1.0, 1.0], [1.0, -1.0, 1.0], [-0.75, 1.25, 1.0], [0.6, 0.6, 0.6]]


def scaled_part(model):
    """A part with cubes, preferably with children, whose name is unique in the tree (Horizon's
    poses are keyed by name)."""
    names, found = {}, []
    def walk(name, node, depth):
        names[name] = names.get(name, 0) + 1
        if depth and node.get('polygons'):
            found.append((not node.get('children'), depth, name))
        for key, child in sorted(node.get('children', {}).items()):
            walk(key, child, depth + 1)
    walk('root', model, 0)
    found = [f for f in found if names[f[2]] == 1]
    return min(found)[2] if found else None


def cases(layer, model=None):
    rng = random.Random(0x414e494d + sum(map(ord, layer)))
    baby = '_baby#' in layer
    base_cases = [
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        (1.0, 0.5, 10.0, 0.0, 0.0, 0.0),
        (3.7, 0.6, 57.3, 20.0, -15.0, 0.0),
        (12.25, 1.0, 123.5, -35.0, 40.0, 0.0),
        (7.5, 0.3, 33.3, 5.0, 5.0, 0.35),
        (2.0, 0.0, 77.7, 45.0, -60.0, 0.8),
    ]
    for _ in range(14):
        base_cases.append((rng.uniform(0, 30), rng.uniform(0, 1), rng.uniform(0, 400),
                           rng.uniform(-50, 50), rng.uniform(-70, 70),
                           rng.choice([0.0, rng.uniform(0.01, 0.99)])))
    cases_out = []
    for index, (pos, speed, age, y, x, attack) in enumerate(base_cases):
        cases_out.append({'walkAnimationPos': pos, 'walkAnimationSpeed': speed, 'ageInTicks': age,
               'yRot': y, 'xRot': x, 'swingAnimation': attack, 'isBaby': baby, 'ageScale': 0.5 if baby else 1.0,
               'entityId': 7, 'yHeadRot': y, 'yBodyRot': 0.0,
               '@rootYaw': ROOT_YAWS[index % len(ROOT_YAWS)], '@rootScale': ROOT_SCALES[index % len(ROOT_SCALES)]})
    name = layer.split(':')[1].split('#')[0].removesuffix('_baby')
    cases_out = [{**c, **DEFAULTS.get(name, {})} for c in cases_out]
    yield from cases_out
    yield from species_cases(layer, cases_out)
    # Exercise render-state extraction with a live walk clock, including the
    # first mounted frame. Existing setupAnim cases bypass anim::state.
    if name == 'skeleton':
        for passenger in [False, True]:
            for alive in [False, True]:
                for partial in [0.0, 0.5, 1.0]:
                    yield {**cases_out[2], 'isPassenger': passenger,
                           '@walkTicks': [0.5, 1.0, 0.75],
                           '@alive': alive, '@partial': partial}
    part = model and scaled_part(model)
    if part:
        for index, scale in enumerate(PART_SCALES):
            yield {**cases_out[2 + index % 2], '@partScale': [part, *scale]}


T, F = True, False
FOX = [{'isCrouching': T, 'crouchAmount': 0.8}, {'isSleeping': T}, {'isSitting': T}, {'isPouncing': T, 'crouchAmount': 0.6},
       {'isFaceplanted': T}, {'headRollAngle': 0.3}, {'isCrouching': T, 'isPouncing': T, 'crouchAmount': 0.4}]
ILLAGER = [{'armPose': pose, 'maxCrossbowChargeDuration': 25, 'ticksUsingItem': 10.0}
           for pose in ['NEUTRAL', 'CROSSED', 'ATTACKING', 'SPELLCASTING', 'BOW_AND_ARROW', 'CROSSBOW_HOLD',
                        'CROSSBOW_CHARGE', 'CELEBRATING']] + [{'isRiding': T, 'armPose': 'CROSSED'}]
FELINE = [{'isCrouching': T}, {'isSprinting': T}, {'isSitting': T}, {'lieDownAmount': 0.7, 'lieDownAmountTail': 0.5},
          {'relaxStateOneAmount': 0.6}, {'isSitting': T, 'lieDownAmount': 1.0, 'lieDownAmountTail': 0.2}]
ZOMBIE = [{'isAggressive': T}, {'isCrouching': T}, {'isPassenger': T}, {'isAggressive': T, 'isPassenger': T, 'isCrouching': T}]
SKELETON = ZOMBIE + [{'isAggressive': T, 'isHoldingBow': T, 'rightArmPose': 'BOW_AND_ARROW'}, {'isHoldingBow': T},
                     {'isAggressive': T, 'isHoldingBow': T, 'rightArmPose': 'BOW_AND_ARROW', 'isCrouching': T}]
PIGLIN = [{'armPose': pose, 'maxCrossbowChageDuration': 25.0, 'ticksUsingItem': 12.0}
          for pose in ['DEFAULT', 'DANCING', 'ATTACKING_WITH_MELEE_WEAPON', 'CROSSBOW_HOLD', 'CROSSBOW_CHARGE', 'ADMIRING_ITEM']]
EQUINE = [{'eatAnimation': 0.6}, {'standAnimation': 0.5}, {'eatAnimation': 0.3, 'standAnimation': 0.4, 'feedingAnimation': 0.7},
          {'isInWater': T}, {'standAnimation': 1.0, 'eatAnimation': 1.0}]
WATER = [{'isInWater': T}, {'isInWater': F}]
GHAST_BODY = [{'bodyItem': 'minecraft:white_harness'}, {'isRidden': T}, {'bodyItem': 'minecraft:red_harness', 'isRidden': T},
              {'isLeashHolder': T, 'bodyItem': 'minecraft:blue_harness'}]
SPECIES = {
    'horse': EQUINE, 'donkey': EQUINE + [{'hasChest': T}], 'mule': EQUINE + [{'hasChest': T}], 'zombie_horse': EQUINE, 'skeleton_horse': EQUINE,
    'iron_golem': [{'attackTicksRemaining': 7.0}, {'offerFlowerTick': 30}, {'attackTicksRemaining': 0.0, 'offerFlowerTick': 0}],
    'hoglin': [{'attackAnimationRemainingTicks': t} for t in (3, 7, 10)],
    'zoglin': [{'attackAnimationRemainingTicks': t} for t in (3, 7, 10)],
    'ravager': [{'stunnedTicksRemaining': 20.0}, {'attackTicksRemaining': 8.0}, {'attackTicksRemaining': 3.0},
                {'roarAnimation': 0.5}],
    'vex': [{'isCharging': T}, {'isCharging': F}],
    'squid': [{'tentacleAngle': 0.7}], 'glow_squid': [{'tentacleAngle': 1.2}],
    'dolphin': [{'isMoving': T}, {'isMoving': F}],
    'cod': WATER, 'salmon': WATER, 'tropical_fish_small': WATER,
    'allay': [{'isDancing': T, 'isSpinning': T, 'spinningProgress': 0.6}, {'isDancing': T, 'isSpinning': F, 'spinningProgress': 0.2},
              {'holdingAnimationProgress': 0.8}, {'holdingAnimationProgress': 1.0}],
    'camel': [{'sitAnimationState': 2}, {'sitPoseAnimationState': 3}, {'sitUpAnimationState': 1}, {'idleAnimationState': 4},
              {'dashAnimationState': 2}, {'jumpCooldown': 20.0}],
    'chicken': [{'flap': 2.0, 'flapSpeed': 0.7}, {'flap': 5.3, 'flapSpeed': 1.0}],
    'wolf': [{'isSitting': T}, {'isAngry': T, 'tailAngle': 1.5393804}, {'shakeAnim': 0.9}, {'interest': 0.6},
             {'isSitting': T, 'shakeAnim': 0.5, 'interest': 0.3}],
    'zombie': ZOMBIE, 'husk': ZOMBIE, 'drowned': ZOMBIE, 'zombie_villager': ZOMBIE, 'zombified_piglin': ZOMBIE, 'giant': ZOMBIE,
    'skeleton': SKELETON, 'stray': SKELETON, 'wither_skeleton': SKELETON, 'bogged': SKELETON,
    'piglin': PIGLIN, 'piglin_brute': PIGLIN,
    'cat': FELINE, 'ocelot': FELINE,
    'breeze': [{'idle': 3}, {'shoot': 2}, {'slide': 4}, {'slideBack': 1}, {'inhale': 5}, {'longJump': 2},
               {'idle': 1, 'shoot': 6}, {'idle': 9, 'slideBack': 7, 'longJump': 3}, {'idle': 2, 'inhale': 0, 'slide': 8}],
    'creaking': [{'canMove': F}, {'attackAnimationState': 2}, {'invulnerabilityAnimationState': 3}, {'deathAnimationState': 1}],
    'evoker': ILLAGER, 'pillager': ILLAGER, 'vindicator': ILLAGER, 'illusioner': ILLAGER,
    'sheep': [{'eatTick': t} for t in (2, 10, 36, 39)],
    'guardian': [{'tailAnimation': 4.0, 'spikesAnimation': 0.3}, {'tailAnimation': 0.0, 'spikesAnimation': 0.0}],
    'elder_guardian': [{'tailAnimation': 4.0, 'spikesAnimation': 0.3}],
    'shulker': [{'peekAmount': v} for v in (0.0, 0.2, 0.5, 1.0)],
    'witch': [{'isHoldingItem': T}, {'isHoldingItem': F}],
    'goat': [{'hasLeftHorn': T, 'hasRightHorn': T}, {'hasLeftHorn': F, 'hasRightHorn': T, 'rammingXHeadRot': 0.6},
             {'hasLeftHorn': T, 'hasRightHorn': F}],
    'polar_bear': [{'standScale': 0.0}, {'standScale': 0.5}, {'standScale': 1.0}],
    'panda': [{'isUnhappy': T}, {'isSneezing': T, 'sneezeTime': 5}, {'isSneezing': T, 'sneezeTime': 16},
              {'sitAmount': 0.6, 'isEating': T}, {'sitAmount': 1.0, 'isScared': T}, {'sitAmount': 0.3},
              {'lieOnBackAmount': 0.5}, {'rollAmount': 0.7}],
    'llama': [{'hasChest': T}, {'hasChest': F}],
    'sniffer': [{'isSearching': T}, {'diggingAnimationState': 2}, {'sniffingAnimationState': 5}, {'risingAnimationState': 1},
                {'feelingHappyAnimationState': 3}, {'scentingAnimationState': 0}, {'diggingAnimationState': 50, 'isSearching': T},
                {'feelingHappyAnimationState': 30, 'scentingAnimationState': 20}],
    'happy_ghast': GHAST_BODY, 'happy_ghast_ropes': GHAST_BODY, 'happy_ghast_baby_ropes': GHAST_BODY,
    'happy_ghast_harness': GHAST_BODY, 'happy_ghast_baby_harness': GHAST_BODY,
    'trader_llama': [{'hasChest': T}, {'hasChest': F}],
    'turtle': [{'isOnLand': T}, {'isOnLand': T, 'isLayingEgg': T}, {'isOnLand': F, 'hasEgg': T}, {'isOnLand': T, 'hasEgg': T}],
    'strider': [{'isRidden': T}, {'isRidden': F}],
    'parrot': [{'pose': pose, 'flapAngle': 0.4} for pose in ['STANDING', 'FLYING', 'ON_SHOULDER', 'SITTING', 'PARTY']],
    'bee': [{'hasStinger': T}, {'hasStinger': F, 'isOnGround': T}, {'isAngry': T}, {'rollAmount': 0.6, 'isAngry': T},
            {'isOnGround': T, 'hasStinger': T}],
    'bat': [{'isResting': T, 'restAnimationState': 3}, {'flyAnimationState': 2}, {'isResting': F, 'flyAnimationState': 0}],
    'armadillo': [{'isHidingInShell': T}, {'isHidingInShell': T, 'rollUpAnimationState': 2},
                  {'rollOutAnimationState': 4}, {'peekAnimationState': 1}],
    'axolotl': [{'playingDeadFactor': 0.0, 'inWaterFactor': 1.0, 'onGroundFactor': 0.0, 'movingFactor': 1.0},
                {'inWaterFactor': 1.0, 'movingFactor': 0.0}, {'onGroundFactor': 1.0, 'movingFactor': 1.0},
                {'onGroundFactor': 1.0, 'movingFactor': 0.0}, {'inWaterFactor': 0.5, 'onGroundFactor': 0.5, 'movingFactor': 0.5},
                {'playingDeadFactor': 1.0, 'inWaterFactor': 1.0},
                {'walkAnimationState': 1}, {'swimAnimation': 2}, {'idleOnGroundAnimationState': 3},
                {'idleUnderWaterAnimationState': 2}, {'idleUnderWaterOnGroundAnimationState': 1}, {'playDeadAnimationState': 4}],
    'frog': [{'isSwimming': T}, {'isSwimming': F}, {'jumpAnimationState': 2}, {'croakAnimationState': 1},
             {'tongueAnimationState': 3}, {'swimIdleAnimationState': 2, 'isSwimming': T}],
    'rabbit': [{'hopAnimationState': 2}, {'idleHeadTiltAnimationState': 1}],
    # EndCrystalModel (not a mob): the base hidden for a placed crystal.
    'end_crystal': [{'showsBottom': F}, {'showsBottom': F, 'ageInTicks': 91234.5}],
    'fox': FOX,
    # CopperGolemModel: the idle head spin and the four chest interactions (started N ticks ago).
    'copper_golem': [{'idleAnimationState': 4}, {'idleAnimationState': 37}, {'interactionGetItem': 3},
                     {'interactionGetItem': 30}, {'interactionGetNoItem': 12}, {'interactionDropItem': 8},
                     {'interactionDropItem': 45}, {'interactionDropNoItem': 20}],
    'warden': [{'tendrilAnimation': 0.7}, {'tendrilAnimation': 1.0, 'heartAnimation': 0.4}, {'heartAnimation': 1.0}]
              + [{state: start} for state in ('roarAnimationState', 'sniffAnimationState', 'emergeAnimationState',
                                              'diggingAnimationState', 'attackAnimationState', 'sonicBoomAnimationState')
                 for start in (50, 20, 100)]
              + [{'attackAnimationState': 50, 'tendrilAnimation': 0.3, 'roarAnimationState': 40}],
}


# Render-state defaults that are not zero/false in vanilla.
DEFAULTS = {'creaking': {'canMove': T}, 'wolf': {'tailAngle': 0.6283185307179586}, 'axolotl': {'inWaterFactor': 1.0}, 'bee': {'hasStinger': T}, 'sheep': {'eatTick': 0},
            'goat': {'hasLeftHorn': T, 'hasRightHorn': T}, 'parrot': {'pose': 'STANDING'},
            'phantom': {'flapOffset': 21.0}, 'guardian': {'tailAnimation': 1.3, 'spikesAnimation': 1.0},
            'elder_guardian': {'tailAnimation': 1.3, 'spikesAnimation': 1.0}, 'end_crystal': {'showsBottom': T}}


def species_cases(layer, base_cases):
    name = layer.split(':')[1].split('#')[0].removesuffix('_baby')
    for extra in SPECIES.get(name, []):
        for index in (2, 3, 6):
            yield {**base_cases[index], **extra}
    if name == 'bat':
        # Entity clocks, rather than injected render clocks: flight, rest, transitions and
        # unrelated flag bits. Exercise the server's tick and the client's visual update.
        for flags in ([0] * 4, [1] * 4, [0, 0, 1, 1, 0, 0], [1, 1, 0, 0, 1, 1], [2, 3, 2, 2]):
            for server_tick in (False, True):
                for partial in (0.0, 0.5):
                    yield {**base_cases[0], 'ageInTicks': len(flags) + partial,
                           '@batTicks': flags, '@batServerTick': server_tick}


def compare_vertices(v, r, args, stats):
    """The posed model's per-vertex normals, as the signed bytes both clients put in the vertex
    buffer (BufferBuilder.setNormal's normalIntValue; Horizon's RGBA8_SNORM `Vertex.normal`), and
    a checksum of each drawn part's posed positions."""
    bad = []
    vanilla, rust = v.get('vertices', {}), r.get('vertices', {})
    for path, ve in vanilla.items():
        ru = rust.get(path)
        if ru is None:
            # Horizon does not draw a part scaled to zero: vanilla's vertices all collapse there.
            if ve['d']:
                stats['degenerate'] += len(ve['n']) // 3
            else:
                bad.append((path, 'drawn by vanilla, not by Horizon'))
            continue
        if len(ve['n']) != len(ru['n']):
            bad.append((path, f'{len(ve["n"]) // 3} vertices != {len(ru["n"]) // 3}'))
            continue
        stats['parts'] += 1
        first = None
        for i in range(0, len(ve['n']), 3):
            stats['vertices'] += 1
            exact = True
            for c in range(3):
                a, b = ve['n'][i + c], ru['n'][i + c]
                f = ve['f'][i // 12 * 3 + c]
                if a == b:
                    continue
                exact = False
                stats['max_bytes'] = max(stats['max_bytes'], abs(a - b))
                # A one-byte step where vanilla's float sits within float rounding of a
                # truncation step (normalIntValue truncates c * 127 toward zero).
                edge = abs(f * 127.0 - round(f * 127.0)) <= args.normal_edge and abs(a - b) == 1
                if edge:
                    stats['edge'] += 1
                elif first is None:
                    first = (path, f'vertex {i // 3} normal {ve["n"][i:i + 3]} != {ru["n"][i:i + 3]}'
                                   f' (vanilla float {ve["f"][i // 12 * 3:i // 12 * 3 + 3]})')
            stats['exact'] += exact
        if first:
            bad.append(first)
        count = len(ve['n']) // 3
        if abs(ve['p'] - ru['p']) > args.position_tolerance * count * (1.0 + abs(ve['p']) / max(count, 1)):
            bad.append((path, f'posed position sum {ve["p"]!r} != {ru["p"]!r}'))
    for path in rust:
        if path not in vanilla:
            bad.append((path, 'drawn by Horizon, not by vanilla'))
    return bad


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='')
    parser.add_argument('--detail', default='')
    parser.add_argument('--tolerance', type=float, default=5e-7)
    parser.add_argument('--normal-edge', type=float, default=1e-4,
                        help='accept a one-byte normal step when vanilla\'s float * 127 is this close to an integer')
    parser.add_argument('--position-tolerance', type=float, default=1e-5,
                        help='per-vertex tolerance of the posed position checksums, in blocks')
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-entity-model-parity'; build.mkdir(parents=True, exist_ok=True)
    resources = lab.RESOURCES
    models = json.loads((resources / 'entity-models.json').read_text())
    chosen = {k: v for k, v in layers(models).items() if args.filter in k}
    extra_cases = json.loads((Path(__file__).with_suffix('.cases.json')).read_text()) \
        if Path(__file__).with_suffix('.cases.json').exists() else {}
    lines = []
    for layer, cls in sorted(chosen.items()):
        for case in cases(layer, models[layer]):
            lines.append(f'{layer} {cls}|{json.dumps(case)}')
        for case in extra_cases.get(layer.split(':')[1].split('#')[0], []):
            lines.append(f'{layer} {cls}|{json.dumps(case)}')
    inputs = build / 'inputs.txt'; inputs.write_text('\n'.join(lines) + '\n')
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaEntityAnimOracle.java'
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', dependencies, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaEntityAnimOracle',
         inputs, build / 'vanilla.jsonl', jars['client']], 'vanilla')
    run(['cargo', 'run', '--quiet', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe', '--',
         'entity_anim_fixture', resources, build / 'vanilla.jsonl', build / 'rust.jsonl'], 'rust')
    vanilla = [json.loads(l) for l in (build / 'vanilla.jsonl').read_text().splitlines()]
    rust = [json.loads(l) for l in (build / 'rust.jsonl').read_text().splitlines()]
    assert len(vanilla) == len(rust), (len(vanilla), len(rust))
    errors, skipped, mismatched, checked = {}, set(), {}, 0
    stats = dict.fromkeys(['parts', 'vertices', 'exact', 'edge', 'degenerate', 'max_bytes'], 0)
    for v, r in zip(vanilla, rust):
        layer = v['layer']
        if 'error' in v:
            errors[layer] = v['error']; continue
        if 'skip' in r:
            skipped.add(layer); continue
        checked += 1
        bad = []
        if 'walk' in v:
            for a, b in zip(v['walk'], r['walk']):
                if abs(a - b) > args.tolerance * (1.0 + abs(a)):
                    bad.append(('@walk', f'{v["walk"]} != {r["walk"]}'))
        for path, expected in v['parts'].items():
            got = r['parts'].get(path)
            if got is None:
                bad.append((path, 'missing in Horizon')); continue
            names = ['x', 'y', 'z', 'xRot', 'yRot', 'zRot', 'xScale', 'yScale', 'zScale', 'visible', 'polys', 'vsum']
            for i, (a, b) in enumerate(zip(expected, got)):
                if i == 11 and abs(a - b) <= 1e-3 * max(1, abs(a)):
                    continue
                if i == 9:
                    a, b = bool(a), bool(b)
                    if a != b:
                        bad.append((path, f'visible {a} != {b}'))
                    continue
                if abs(a - b) > args.tolerance * (1.0 + abs(a)):
                    bad.append((path, f'{names[i]} {a!r} != {b!r}'))
        for path in r['parts']:
            if path not in v['parts']:
                bad.append((path, 'extra in Horizon'))
        bad += compare_vertices(v, r, args, stats)
        if bad:
            mismatched.setdefault(layer, []).append((v['case'], bad))
    print(f'{checked} cases compared over {len(chosen) - len(skipped) - len(errors)} layers')
    print(f'normals: {stats["vertices"]} vertices in {stats["parts"]} drawn parts, {stats["exact"]} with identical '
          f'packed bytes, {stats["edge"]} components one step apart at a truncation edge, max byte difference '
          f'{stats["max_bytes"]} (max normal error {stats["max_bytes"] / 127:.5f}); '
          f'{stats["degenerate"]} collapsed vertices of zero-scaled parts left out')
    if errors:
        print('oracle errors:'); [print(' ', k, v) for k, v in sorted(errors.items())]
    if skipped:
        print('layers without a Horizon species:', ', '.join(sorted(skipped)))
    for layer, items in sorted(mismatched.items()):
        parts = {}
        for case, bad in items:
            for path, why in bad:
                parts.setdefault(path, (why, case))
        total = sum(1 for x in vanilla if x['layer'] == layer)
        print(f'MISMATCH {layer}: {len(items)}/{total} cases, {len(parts)} parts')
        detailed = args.detail == layer
        for path, (why, case) in list(parts.items())[:(10 ** 6 if detailed else 4)]:
            print(f'    {path or "<root>"}: {why}' + (f'   case={json.dumps(case)}' if detailed else ''))
    if mismatched or errors:
        sys.exit(f'{len(mismatched)} layers differ from vanilla')
    print('entity model parity ok')


if __name__ == '__main__':
    main()
