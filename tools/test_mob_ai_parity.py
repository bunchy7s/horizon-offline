#!/usr/bin/env python3
"""Differential mob AI harness.

The Java oracle (VanillaMobAiOracle) runs vanilla mobs in a real, headless, flat vanilla server
level: each scenario builds a scene, places a target player and mobs with seeded random sources
and ticks the mobs the way ServerLevel.tick does. The Rust probe (mob_ai_probe) replays the same
scenarios on Horizon's mob AI. Both print every mob's AI-visible state after every tick (position,
motion, rotations, running goals and target goals, attack target, navigation path, move and look
controls, movement input, brain activities and memories) and the sounds played; this harness
reports, per scenario and mob, how many ticks match before the first divergence and what differs.

  python3 tools/test_mob_ai_parity.py [--only NAME ...] [--rust-only] [--java-only] [--report-only] [--trace]

The oracle traces every mob random draw with its call stack; --trace builds the probe with
the rng-trace feature and line tables so the report shows Horizon's draws next to vanilla's.
"""
import argparse
import os
import shutil
import struct
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, oracle_harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab
    import oracle_harness
    import test_rust_parity as base
    import test_model_baking_parity as baking

FLOOR = -60  # top of the flat preset's grass layer + 1


def ring(x0, z0, size, state, y=FLOOR):
    return [f'fill {x0 - size} {y} {z0 - size} {x0 + size} {y} {z0 + size} {state}']


# Each scenario is (name, ticks, options, lines); positions are relative to the scenario origin
# and translated by the generator so scenarios never share chunks.
def scenarios():
    s = []

    def add(name, ticks, lines, opts=''):
        s.append((name, ticks, opts, lines))

    # Passive and hostile land mobs on open grass with a survival player nearby.
    for species, seed in [('zombie', 1), ('skeleton', 2), ('creeper', 3), ('spider', 4), ('husk', 5),
                          ('cow', 6), ('pig', 7), ('chicken', 8), ('sheep', 9), ('wandering_trader', 10),
                          ('villager', 11), ('drowned', 12), ('enderman', 13), ('witch', 14)]:
        add(f'{species}_player', 200, [
            'player 6.5 -60 0.5',
            f'mob minecraft:{species} 0.5 -60 0.5 0 {seed} persistent',
        ])
        add(f'{species}_alone', 200, [
            f'mob minecraft:{species} 0.5 -60 0.5 90 {seed * 7 + 1} persistent',
        ])
    # Drowned in a pool with the player on the shore and in the water.
    add('drowned_pool', 200, ['fill -6 -64 -6 6 -61 6 minecraft:water',
                              'player 9.5 -60 0.5',
                              'mob minecraft:drowned 0.5 -63 0.5 0 21 persistent'])
    add('drowned_pool_swimmer', 200, ['fill -6 -64 -6 6 -61 6 minecraft:water',
                                      'player 3.5 -62 0.5',
                                      'mob minecraft:drowned -3.5 -63 0.5 0 22 persistent'])
    # Night: undead do not burn, monsters seek darkness.
    add('zombie_night', 200, ['player 10.5 -60 0.5', 'mob minecraft:zombie 0.5 -60 0.5 0 31 persistent'], 'time=18000')
    add('skeleton_day_burn', 120, ['mob minecraft:skeleton 0.5 -60 0.5 0 32 persistent'])
    # NoAI only skips serverAiStep: Mob.aiStep's burnUndead still sets a NoAI zombie alight (#3430).
    add('zombie_noai_day_burn', 120, ['mob minecraft:zombie 0.5 -60 0.5 0 33 persistent noai'])
    # The wither (summoned, not built: no invulnerable spawn phase).
    add('wither_player', 200, ['player 10.5 -60 0.5', 'mob minecraft:wither 0.5 -58 0.5 0 41 persistent'])
    # A villager with a job site, a bed and a bell.
    add('villager_village', 300, [
        'block 4 -60 0 minecraft:composter',
        'block -4 -60 2 minecraft:red_bed[facing=north,part=foot]',
        'block -4 -60 1 minecraft:red_bed[facing=north,part=head]',
        'block 0 -60 -6 minecraft:bell',
        'player 12.5 -60 0.5',
        'mob minecraft:villager 0.5 -60 0.5 0 51 persistent',
    ])
    # Village goals of iron golems (GolemRandomStrollInVillageGoal, MoveBackToVillageGoal; POIs: beds, a bell, a composter, a door).
    VILLAGE = ['block 4 -60 0 minecraft:composter',
               'block -4 -60 2 minecraft:red_bed[facing=north,part=foot]',
               'block -4 -60 1 minecraft:red_bed[facing=north,part=head]',
               'block 0 -60 -6 minecraft:bell',
               'block 8 -60 -3 minecraft:oak_door[facing=east,half=lower]',
               'block 8 -59 -3 minecraft:oak_door[facing=east,half=upper]']
    for i, (x, seed) in enumerate([(0.5, 240), (6.5, 241)]):
        add(f'iron_golem_village_{i}', 400, VILLAGE + [f'mob minecraft:iron_golem {x} -60 3.5 0 {seed} persistent'], 'difficulty=0')
    add('iron_golem_outside_village', 400, VILLAGE + ['mob minecraft:iron_golem 30.5 -60 3.5 0 242 persistent'], 'difficulty=0')
    # RandomStandGoal (rearing, with the ambient sound) of every equine, left alone for 800 ticks.
    for i, species in enumerate(['horse', 'donkey', 'mule', 'zombie_horse', 'skeleton_horse', 'llama']):
        # The zombie horse at night: by day it burns, and a burning horse's hurts are another matter.
        add(f'{species}_rears_alone', 800, [f'mob minecraft:{species} 0.5 -60 0.5 0 {260 + i} persistent'],
            'difficulty=0' + (' time=18000' if species == 'zombie_horse' else ''))
    # Enough of them for the one in 400 draw to land a few times (the rearing and its sound).
    for i, species in enumerate(['horse', 'donkey', 'mule', 'zombie_horse', 'skeleton_horse']):
        add(f'{species}_herd_rears', 1200, [f'mob minecraft:{species} {-10.5 + 3 * k} -60 {-3.5 + 7 * (k % 2)} 0 {300 + 10 * i + k} persistent'
                                            for k in range(8)], 'difficulty=0' + (' time=18000' if species == 'zombie_horse' else ''))
    add('baby_horse_rears_alone', 800, ['mob minecraft:horse 0.5 -60 0.5 0 270 persistent baby'], 'difficulty=0')
    # The warden: idle, sniffing out a player, roaring and fighting (WardenAi).
    # Without finalizeSpawn's dig cooldown a warden digs straight back down. warden_player
    # and warden_cow stop before the first blow kills the player or the cow (the probe has no dying
    # player, nor the attack knockback attribute or a target killed within the attacker's tick).
    add('warden_digs', 140, ['mob minecraft:warden 0.5 -60 0.5 0 60'])
    add('warden_alone', 400, ['mob minecraft:warden 0.5 -60 0.5 0 61 digcooldown'])
    add('warden_player', 565, ['player 5.5 -60 0.5', 'mob minecraft:warden 0.5 -60 0.5 0 62 digcooldown'])
    add('warden_player_far', 600, ['player 14.5 -60 3.5', 'mob minecraft:warden 0.5 -60 0.5 90 63 digcooldown'])
    add('warden_creative', 300, ['player 4.5 -60 0.5 creative', 'mob minecraft:warden 0.5 -60 0.5 0 64 digcooldown'])
    add('warden_persistent', 400, ['player 7.5 -60 0.5', 'mob minecraft:warden 0.5 -60 0.5 0 65 persistent digcooldown'])
    add('warden_emerging', 300, ['player 6.5 -60 0.5', 'mob minecraft:warden 0.5 -60 0.5 0 66 emerge'])
    add('warden_cow', 176, ['mob minecraft:warden 0.5 -60 0.5 0 67 digcooldown', 'mob minecraft:cow 4.5 -60 0.5 0 68 persistent'])
    # Two villagers gossip (TradeWithVillager): what one heard of the player
    # passes to the other, weakened, once in 1200 ticks; idle and at the bell.
    gossips = ('gossip=player:minor_negative:100 gossip=player:major_positive:15 '
               'gossip=player:trading:20 gossip=player:minor_positive:3')
    for name, opts, extra in [('villager_gossip_idle', 'time=100', []),
                              ('villager_gossip_bell', 'time=9500', ['block 0 -60 -6 minecraft:bell'])]:
        add(name, 400, extra + [
            'player 14.5 -60 0.5',
            f'mob minecraft:villager 0.5 -60 0.5 0 61 persistent {gossips}',
            'mob minecraft:villager 2.5 -60 0.5 180 62 persistent gossip=player:minor_positive:50',
        ], opts)
    # Two villagers with bread breed (VillagerMakeLove): they eat, a baby is
    # born and takes a free bed, the parents rest for 6000 ticks.
    # PoiManager.take hands out the free beds of a section in hash order:
    # the parents claim the two near them, the child gets the only one left.
    beds = []
    for x in (-6, 3, 6):
        beds += [f'block {x} -60 6 minecraft:red_bed[facing=north,part=foot]',
                 f'block {x} -60 5 minecraft:red_bed[facing=north,part=head]']
    for name, seeds in [('villager_breed', (71, 72)), ('villager_breed_2', (73, 74))]:
        add(name, 900, beds + [
            'player 14.5 -60 0.5',
            f'mob minecraft:villager 0.5 -60 0.5 0 {seeds[0]} persistent inv=bread*3',
            f'mob minecraft:villager 2.5 -60 0.5 180 {seeds[1]} persistent inv=bread*3 food=4',
        ], 'time=100')
    # Too little food: no breeding.
    add('villager_breed_hungry', 400, beds[:2] + [
        'mob minecraft:villager 0.5 -60 0.5 0 75 persistent inv=bread*2',
        'mob minecraft:villager 2.5 -60 0.5 180 76 persistent inv=bread*3',
    ], 'time=100')
    # Breezes: idle, and fighting a player near, in, and beyond the inner jump circle.
    add('breeze_alone', 200, ['mob minecraft:breeze 0.5 -60 0.5 90 61 persistent'])
    add('breeze_player', 200, ['player 8.5 -60 0.5', 'mob minecraft:breeze 0.5 -60 0.5 0 62 persistent'])
    add('breeze_close', 200, ['player 2.5 -60 1.5', 'mob minecraft:breeze 0.5 -60 0.5 0 63 persistent'])
    add('breeze_far', 200, ['player 13.5 -60 6.5', 'mob minecraft:breeze 0.5 -60 0.5 0 64 persistent'])
    # Creakings: the player faces +z, so it watches one and turns its back on the other.
    add('creaking_alone', 200, ['mob minecraft:creaking 0.5 -60 0.5 90 71 persistent'])
    add('creaking_watched', 200, ['player 0.5 -60 -7.5', 'mob minecraft:creaking 0.5 -60 0.5 180 72 persistent'])
    add('creaking_behind', 200, ['player 0.5 -60 8.5', 'mob minecraft:creaking 0.5 -60 0.5 0 73 persistent'])
    # Woken while watched, then free to hunt and hit once the player turns away.
    add('creaking_attack', 240, ['player 0.5 -60 -7.5 turn=40,180', 'mob minecraft:creaking 0.5 -60 0.5 180 74 persistent'])
    # A creaking heart in a pale oak trunk: by night it spawns its creaking near a player,
    # by day it sleeps, without logs it is uprooted, with no player near it waits.
    TRUNK = ['block 3 -60 3 minecraft:pale_oak_log[axis=y]', 'block 3 -58 3 minecraft:pale_oak_log[axis=y]']
    add('heart_night', 300, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'time=18000 spawning')
    add('heart_day', 120, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'spawning')
    add('heart_uprooted', 60, ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5'], 'time=18000 spawning')
    # A player hits the heart's creaking: no damage, but the heart bleeds resin and trails.
    add('heart_hurt', 260, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 0.5 -60 -6.5',
                            'hurt 60 0', 'hurt 64 0', 'hurt 200 0'], 'time=18000 spawning')
    add('heart_far', 120, TRUNK + ['heart 3 -59 3 minecraft:creaking_heart[axis=y,creaking_heart_state=awake,natural=true]', 'player 40.5 -60 0.5'], 'time=18000 spawning')
    # Llamas and trader llamas: strolling, looking at the player, a baby following its parent and a
    # llama hunting a wolf (LlamaAttackWolfGoal) and spitting at it.
    for species, seed in [('llama', 61), ('trader_llama', 62)]:
        add(f'{species}_player', 200, ['player 6.5 -60 0.5', f'mob minecraft:{species} 0.5 -60 0.5 0 {seed} persistent'])
        add(f'{species}_alone', 200, [f'mob minecraft:{species} 0.5 -60 0.5 90 {seed * 7 + 1} persistent'])
    add('llama_family', 200, ['mob minecraft:llama 0.5 -60 0.5 0 63 persistent',
                              'mob minecraft:llama 6.5 -60 2.5 0 64 persistent baby'])
    add('llama_wolf', 200, ['mob minecraft:llama 0.5 -60 0.5 0 65 persistent',
                            'mob minecraft:wolf 8.5 -60 0.5 0 66 persistent'])
    # Sniffers on diggable grass: scenting, sniffing, searching and digging.
    add('sniffer_player', 400, ['player 8.5 -60 0.5', 'mob minecraft:sniffer 0.5 -60 0.5 0 71 persistent'])
    for i, seed in enumerate([72, 73, 74]):
        add(f'sniffer_alone_{i}', 600, [f'mob minecraft:sniffer 0.5 -60 0.5 {90 * i} {seed} persistent'])
    add('sniffer_baby', 300, ['mob minecraft:sniffer 0.5 -60 0.5 0 75 persistent baby'])
    # Happy ghasts float above the grass; ghastlings follow players and adults.
    add('happy_ghast_player', 200, ['player 6.5 -60 0.5', 'mob minecraft:happy_ghast 0.5 -55 0.5 0 81 persistent'])
    add('happy_ghast_alone', 200, ['mob minecraft:happy_ghast 0.5 -55 0.5 90 82 persistent'])
    # The driver's seat overlaps the front of the ghast: it must not push its own rider.
    # One tick isolates riding input and turning from later passenger repositioning.
    for name, yaw in [('hover', 0), ('turn', 90)]:
        add(f'happy_ghast_ridden_{name}', 1, ['player 0.5 -51.6 2.2',
            f'mob minecraft:happy_ghast 0.5 -55 0.5 {yaw} 82 persistent ridden'])
    add('ghastling_player', 200, ['player 8.5 -60 0.5', 'mob minecraft:happy_ghast 0.5 -57 0.5 0 83 persistent baby'])
    add('ghastling_alone', 200, ['mob minecraft:happy_ghast 0.5 -57 0.5 0 84 persistent baby'])
    add('ghastling_adult', 200, ['mob minecraft:happy_ghast 10.5 -55 0.5 0 85 persistent',
                                 'mob minecraft:happy_ghast 0.5 -57 0.5 0 86 persistent baby'])
    # Breeding, temptation and growing up.
    add('sniffer_love', 300, ['mob minecraft:sniffer 0.5 -60 0.5 0 91 persistent inlove',
                              'mob minecraft:sniffer 4.5 -60 0.5 0 92 persistent inlove'])
    add('sniffer_tempted', 200, ['player 7.5 -60 0.5 hand=minecraft:torchflower_seeds',
                                 'mob minecraft:sniffer 0.5 -60 0.5 0 93 persistent'])
    add('llama_love', 300, ['mob minecraft:llama 0.5 -60 0.5 0 94 persistent tame inlove strength=3',
                            'mob minecraft:llama 4.5 -60 0.5 0 95 persistent tame inlove strength=2'])
    add('llama_tempted', 200, ['player 7.5 -60 0.5 hand=minecraft:hay_block',
                               'mob minecraft:llama 0.5 -60 0.5 0 96 persistent'])
    add('ghastling_tempted', 200, ['player 8.5 -60 0.5 hand=minecraft:snowball',
                                   'mob minecraft:happy_ghast 0.5 -57 0.5 0 97 persistent baby'])
    add('happy_ghast_tempted', 200, ['player 12.5 -60 0.5 hand=minecraft:snowball',
                                     'mob minecraft:happy_ghast 0.5 -55 0.5 0 98 persistent'])
    add('ghastling_grows', 200, ['player 8.5 -60 0.5',
                                 'mob minecraft:happy_ghast 0.5 -57 0.5 0 99 persistent age=-40'])
    # Llama caravans: two llamas fall in behind one on the player's lead; a wandering trader's two
    # llamas on its leads.
    add('llama_caravan', 300, ['player 0.5 -60 0.5',
                               'mob minecraft:llama 3.5 -60 0.5 0 101 persistent leashed strength=3',
                               'mob minecraft:llama 9.5 -60 2.5 0 102 persistent strength=2',
                               'mob minecraft:llama 13.5 -60 -1.5 0 103 persistent strength=1'])
    add('trader_llamas', 300, ['player 14.5 -60 0.5',
                               'mob minecraft:wandering_trader 0.5 -60 0.5 0 104 persistent',
                               'mob minecraft:trader_llama 3.5 -60 1.5 0 105 persistent leashto=0 strength=2',
                               'mob minecraft:trader_llama 3.5 -60 -1.5 0 106 persistent leashto=0 strength=4'])
    # Death must detach immediately and drop exactly one lead throughout the corpse animation.
    add('leash_death_trader_llama', 10, [
        'player 14.5 -60 0.5',
        'mob minecraft:wandering_trader 0.5 -60 0.5 0 104 persistent',
        'mob minecraft:trader_llama 3.5 -60 1.5 0 105 persistent leashto=0 strength=2 dead',
    ])
    add('leash_death_player_llama', 10, [
        'player 0.5 -60 0.5',
        'mob minecraft:llama 3.5 -60 0.5 0 101 persistent leashed strength=3 dead',
    ])
    # Copper golems sorting chests (TransportItemsBetweenContainers): from a copper chest to a chest,
    # skipping a chest without the carried item, queueing at a chest another golem has open, a double
    # chest, a blocked copper chest, a part-full chest, and idling with nothing to carry.
    def chest(x, z, slots='-', kind='chest', facing='north', half='single', y=-60):
        return f'chest {x} {y} {z} minecraft:{kind}[facing={facing},type={half},waterlogged=false] {slots}'
    golem = 'mob minecraft:copper_golem {} -60 {} {} {} persistent cooldown=0'
    add('golem_copper_to_chest', 600, [chest(4, 0, 'cobblestone*20,-,iron_ingot*5', 'copper_chest'),
                                       chest(-4, 3), golem.format(0.5, 0.5, 0, 111)])
    add('golem_skips_other_items', 700, [chest(4, 0, 'cobblestone*40', 'copper_chest'),
                                         chest(-3, 0, 'dirt*3'), chest(-6, 5, 'cobblestone*10'),
                                         golem.format(0.5, 0.5, 90, 112)])
    add('golem_queue', 600, [chest(3, 0, 'oak_log*48,stick*7', 'copper_chest'), chest(-4, 0),
                             golem.format(0.5, 0.5, 0, 113), golem.format(0.5, 3.5, 180, 114)])
    add('golem_double_chest', 600, [chest(4, 0, 'apple*30', 'exposed_copper_chest'),
                                    chest(-4, 2, '-', 'chest', 'north', 'left'), chest(-5, 2, 'apple*2', 'chest', 'north', 'right'),
                                    golem.format(0.5, 0.5, 0, 115)])
    add('golem_blocked_chest', 400, [chest(3, 0, 'sand*5', 'copper_chest'), 'block 3 -59 0 minecraft:stone',
                                     chest(-5, -2, 'gravel*9', 'oxidized_copper_chest'), chest(0, 6),
                                     golem.format(0.5, 0.5, 0, 116)])
    add('golem_part_full', 500, [chest(-3, 1, 'cobblestone*60,cobblestone*64,-,cobblestone*63'),
                                 'mob minecraft:copper_golem 0.5 -60 0.5 0 117 persistent cooldown=0 hand=cobblestone*16'])
    add('golem_idle', 400, ['player 4.5 -60 2.5', 'mob minecraft:copper_golem 0.5 -60 0.5 0 118 persistent'])
    # The player feeds animals their food (Animal.mobInteract): an adult falls in love once, a baby
    # grows each time, and each plays its own eating sound or none (Animal.playEatingSound).
    for species, food, seed in [('cow', 'wheat', 120), ('pig', 'carrot', 121), ('sheep', 'wheat', 122),
                                ('chicken', 'wheat_seeds', 123)]:
        for baby in ('', ' baby'):
            add(f'feed_{species}{baby.replace(" ", "_")}', 80, [
                'player 3.5 -60 0.5',
                f'mob minecraft:{species} 0.5 -60 0.5 0 {seed} persistent{baby}',
                f'feed 20 0 minecraft:{food}', f'feed 40 0 minecraft:{food}',
            ])
    # The player uses a sword on untamed horses (#2697): Horse, AbstractChestedHorse and
    # ZombieHorse.mobInteract make them mad (makeMad: rear, angry sound) instead of taking the
    # rider, not while one still stands; an untamed skeleton horse ignores it (SkeletonHorse).
    # The zombie horse goes at night (it burns by day), the skeleton horse stops before its
    # first step sound (Horizon plays the block's).
    for species, seed, babies, ticks, opts in [
            ('horse', 180, True, 80, ''), ('donkey', 181, True, 80, ''), ('mule', 182, False, 80, ''),
            ('zombie_horse', 183, False, 80, 'time=18000'), ('llama', 184, True, 80, ''),
            ('skeleton_horse', 185, False, 45, '')]:
        for baby in ('', ' baby') if babies else ('',):
            add(f'sword_{species}{baby.replace(" ", "_")}', ticks, [
                'player 3.5 -60 0.5',
                f'mob minecraft:{species} 0.5 -60 0.5 0 {seed} persistent{baby}',
                'interact 20 0 minecraft:iron_sword', 'interact 30 0 minecraft:iron_sword',
                'interact 40 0 minecraft:iron_sword',
            ], opts)
    # Striders: standing on a lava pool (floatStrider), walking from cold grass to it
    # (StriderGoToLavaGoal, suffocating), and drawn by a player holding warped fungus.
    LAVA = 'fill -5 -63 -5 5 -61 5 minecraft:lava'
    add('strider_lava', 200, [LAVA, 'mob minecraft:strider 0.5 -61 0.5 0 131 persistent'])
    add('strider_lava_player', 200, [LAVA, 'player 8.5 -60 0.5',
                                     'mob minecraft:strider -1.5 -61 0.5 90 132 persistent'])
    add('strider_shore', 300, [LAVA, 'mob minecraft:strider 9.5 -60 0.5 90 133 persistent'])
    add('strider_cold', 200, ['mob minecraft:strider 0.5 -60 0.5 0 134 persistent'])
    add('strider_tempted', 200, [LAVA, 'player 9.5 -60 0.5 hand=minecraft:warped_fungus',
                                 'mob minecraft:strider 0.5 -61 0.5 0 135 persistent'])
    # Wild ocelots (Ocelot.registerGoals + reassessTrustingGoals): alone, fleeing a survival player
    # inside 16 blocks (OcelotAvoidEntityGoal, sprinting inside 7), ignoring a creative one, and drawn
    # by raw cod (OcelotTemptGoal). Rabbits wait for RaidGardenGoal, whose first canUse draws from
    # the rabbit's random.
    add('ocelot_alone', 200, ['mob minecraft:ocelot 0.5 -60 0.5 0 181 persistent'])
    add('ocelot_player', 200, ['player 6.5 -60 0.5', 'mob minecraft:ocelot 0.5 -60 0.5 0 182 persistent'])
    add('ocelot_player_far', 200, ['player 14.5 -60 0.5', 'mob minecraft:ocelot 0.5 -60 0.5 0 183 persistent'])
    add('ocelot_creative', 200, ['player 6.5 -60 0.5 creative', 'mob minecraft:ocelot 0.5 -60 0.5 0 184 persistent'])
    add('ocelot_tempted', 200, ['player 8.5 -60 0.5 hand=minecraft:cod',
                                'mob minecraft:ocelot 0.5 -60 0.5 0 185 persistent'])
    # Bats: hanging under a stone ceiling alone, woken by a player passing beneath, flying off with
    # nothing above, and fluttering about a closed cave where they may hang up again.
    CEILING = 'fill -6 -57 -6 6 -57 6 minecraft:stone'
    add('bat_hanging', 200, [CEILING, 'mob minecraft:bat 0.5 -57.9 0.5 0 141 persistent'])
    add('bat_woken', 200, [CEILING, 'player 1.5 -60 0.5', 'mob minecraft:bat 0.5 -57.9 0.5 0 142 persistent'])
    add('bat_flying', 300, ['mob minecraft:bat 0.5 -57 0.5 0 143 persistent'])
    CAVE = ['fill -5 -60 -5 5 -54 5 minecraft:stone', 'fill -4 -60 -4 4 -55 4 minecraft:air']
    for i, seed in enumerate([144, 145, 146]):
        add(f'bat_cave_{i}', 400, CAVE + [f'mob minecraft:bat 0.5 -58 0.5 {90 * i} {seed} persistent'])
    # Bees: wandering over grass (HoverRandomPos), pollinating a flower bed until they carry nectar,
    # drawn to a player holding a flower, staying home at night and in the rain (no hive to find),
    # drowning in a pool, and angered by a hit: they chase and sting the player once, then die of it.
    FLOWERS = 'fill 3 -60 2 5 -60 4 minecraft:dandelion'
    for i, seed in enumerate([151, 152]):
        add(f'bee_alone_{i}', 400, [f'mob minecraft:bee 0.5 -58 0.5 {90 * i} {seed} persistent'])
    for i, seed in enumerate([153, 154]):
        add(f'bee_flowers_{i}', 700, [FLOWERS, f'mob minecraft:bee 0.5 -58 0.5 {180 * i} {seed} persistent'])
    add('bee_tempted', 200, ['player 6.5 -60 0.5 hand=minecraft:dandelion', 'mob minecraft:bee 0.5 -58 0.5 0 155 persistent'])
    add('bee_night', 300, [FLOWERS, 'mob minecraft:bee 0.5 -58 0.5 0 156 persistent'], 'time=18000')
    add('bee_rain', 300, [FLOWERS, 'mob minecraft:bee 0.5 -58 0.5 0 157 persistent'], 'rain')
    add('bee_pool', 200, ['fill -3 -63 -3 3 -61 3 minecraft:water', 'mob minecraft:bee 0.5 -62 0.5 0 158 persistent'])
    add('bee_sting', 400, ['player 4.5 -60 0.5', 'mob minecraft:bee 0.5 -58 0.5 0 159 persistent', 'hurt 20 0'])
    # Bees and their hive: home at night and in the rain (BeeLocateHiveGoal, BeeGoToHiveGoal,
    # BeeEnterHiveGoal), out again once the morning comes, and nectar turned into honey.
    HIVE = 'block 4 -60 0 minecraft:beehive[facing=west,honey_level=0]'
    add('bee_hive_night', 300, [HIVE, 'mob minecraft:bee 0.5 -58 0.5 0 161 persistent'], 'time=13000')
    add('bee_hive_rain', 300, [HIVE, 'mob minecraft:bee 0.5 -58 0.5 0 162 persistent'], 'rain')
    add('bee_hive_morning', 1000, [HIVE, 'mob minecraft:bee 0.5 -58 0.5 0 163 persistent'], 'time=22900 daycycle')
    add('bee_hive_honey', 2600, [HIVE, 'mob minecraft:bee 0.5 -58 0.5 0 164 persistent nectar'])
    # Parrots (#3091): FlyingMoveControl(10, false) and FlyingPathNavigation. ParrotWanderGoal flies
    # to the top of a tree (an empty block over leaves or a log), and a parrot in the air glides
    # down (calculateFlapping). On Peaceful: Parrot.getAmbient's 1-in-1000 imitation roll is the
    # level's random there, which the probe doesn't share.
    TREE = ['fill 4 -60 4 4 -56 4 minecraft:oak_log', 'fill 2 -55 2 6 -55 6 minecraft:oak_leaves[persistent=true]']
    for i, seed in enumerate([181, 182]):
        add(f'parrot_alone_{i}', 400, [f'mob minecraft:parrot 0.5 -60 0.5 {90 * i} {seed} persistent'], 'difficulty=0')
    # FollowPlayerRiddenEntityGoal: a dolphin swims after the boat a player rides along a pool.
    for i, (vx, vz, dx, seed) in enumerate([(0.15, 0.0, 0.5, 200), (0.0, 0.12, 2.5, 201), (-0.15, 0.0, 0.5, 202)]):
        add(f'dolphin_boat_{i}', 240, ['fill -40 -64 -40 40 -61 40 minecraft:water',
                                       f'player {-8.5 if vx >= 0 and vz == 0 else (8.5 if vx < 0 else 0.5)} -61 {-8.5 if vz > 0 else 0.5} boat={vx},{vz}',
                                       f'mob minecraft:dolphin {dx} -63 0.5 0 {seed} persistent'], 'difficulty=0')
    add('dolphin_boat_far', 160, ['fill -40 -64 -40 40 -61 40 minecraft:water', 'player -20.5 -61 0.5 boat=0.1,0',
                                  'mob minecraft:dolphin 20.5 -63 0.5 0 203 persistent'], 'difficulty=0')
    # FollowMobGoal: a parrot flies after a mob of another kind within 7 blocks and keeps about 3 away.
    for i, (cx, seed) in enumerate([(5.5, 190), (3.5, 191), (-6.5, 192)]):
        add(f'parrot_cow_{i}', 300, [f'mob minecraft:parrot 0.5 -59 0.5 0 {seed} persistent',
                                      f'mob minecraft:cow {cx} -60 0.5 0 {seed + 10} persistent noai'], 'difficulty=0')
    add('parrot_cow_far', 200, ['mob minecraft:parrot 0.5 -59 0.5 0 195 persistent',
                                'mob minecraft:cow 12.5 -60 0.5 0 196 persistent noai'], 'difficulty=0')
    add('parrot_tree', 400, TREE + ['mob minecraft:parrot 0.5 -60 0.5 0 183 persistent'], 'difficulty=0')
    add('parrot_falling', 100, ['mob minecraft:parrot 0.5 -50 0.5 0 184 persistent'], 'difficulty=0')
    # A wolf the player hits (#3185, #2820): any wolf with a target is angry (the angry texture):
    # NeutralMob.updatePersistentAnger in Wolf.aiStep restarts the 20-39 s timer, one random draw,
    # every tick it has one, not once when hurt.
    add('wolf_hurt', 200, ['player 5.5 -60 0.5', 'mob minecraft:wolf 0.5 -60 0.5 0 181 persistent', 'hurt 20 0'])
    # A tamed wolf its owner hits (#4119): TamableAnimal.canAttack never lets it target its owner,
    # so HurtByTargetGoal doesn't start and the wolf stays calm; a second wolf of the same owner
    # isn't called either.
    add('wolf_owner_hurt', 120, ['player 5.5 -60 0.5', 'mob minecraft:wolf 0.5 -60 0.5 0 185 persistent owned',
                                 'mob minecraft:wolf 0.5 -60 3.5 0 186 persistent owned', 'hurt 20 0'])
    # A wet wolf (#2885): Wolf.aiStep starts the shake of a wet wolf standing still on the ground,
    # Wolf.tick plays its sound (two draws) the same tick and cancels it again in water or rain.
    add('wolf_shake', 200, ['fill 0 -61 0 0 -61 0 minecraft:water', 'mob minecraft:wolf 0.5 -61 0.5 0 183 persistent'])
    add('wolf_rain', 100, ['mob minecraft:wolf 0.5 -60 0.5 0 184 persistent'], 'rain')
    # Mobs dropped into deep water (#542): FloatGoal users float at the surface (their navigation
    # can float, so Mob.jumpInLiquid adds 0.04 a tick), zombies sink.
    for species, seed in [('pig', 171), ('cow', 172), ('wolf', 173), ('cat', 174), ('horse', 175),
                          ('witch', 176), ('vindicator', 177), ('zombie', 178)]:
        add(f'{species}_deep_water', 160, ['fill -4 -64 -4 4 -61 4 minecraft:water',
                                           f'mob minecraft:{species} 0.5 -62 0.5 0 {seed} persistent'])
    # A ghast floating near a player (RandomFloatAroundGoal, GhastMoveControl, GhastLookGoal) and shooting it.
    add('ghast_player', 120, ['player 7.5 -56 0.5', 'mob minecraft:ghast 0.5 -56 0.5 0 273 persistent'])
    # AbstractHorse.hurtServer (#3432): every hurt draws nextInt(3), and a 0 rears the horse
    # (standIfPossible). A llama makes the draw too but never rears (canPerformRearing). A horse
    # scenario waits for #2720: a rearing horse is immobile (AbstractHorse.isImmobile).
    add('llama_hurt', 200, ['player 4.5 -60 0.5', 'mob minecraft:llama 0.5 -60 0.5 0 182 persistent']
        + [f'hurt {t} 0' for t in range(10, 190, 25)])
    # Constructor path maluses: an Animal avoids fire (FIRE -1, FIRE_IN_NEIGHBOR 16).
    add('pig_tempt_fire', 100, ['player 9.5 -60 0.5 hand=minecraft:carrot', 'fill 3 -61 -3 3 -61 3 minecraft:netherrack',
                                'fill 3 -60 -3 3 -60 3 minecraft:fire', 'mob minecraft:pig 0.5 -60 0.5 0 221 persistent'])
    # A baby villager grows up: Villager.ageBoundaryReached rebuilds its brain, so the adult schedule
    # (work, meet) replaces the baby's (play) at once.
    for seed in (201, 202):
        add(f'villager_grows_up_{seed}', 120, ['player 8.5 -60 0.5',
                                               f'mob minecraft:villager 0.5 -60 0.5 0 {seed} persistent age=-30'], 'time=2000')
    # Baby villagers play tag (PlayTagWithOtherKids): one picks a friend to chase, the chased one
    # runs off to a spot in the village (isFriendChasingMe).
    add('villager_kids_tag', 400, ['block 0 -60 -6 minecraft:bell', 'player 12.5 -60 0.5',
                                   'mob minecraft:villager 0.5 -60 0.5 0 231 persistent age=-24000',
                                   'mob minecraft:villager 2.5 -60 0.5 0 232 persistent age=-24000',
                                   'mob minecraft:villager 1.5 -60 2.5 0 233 persistent age=-24000'], 'time=2000')
    # A skeleton by day chases a player past the edge of a roof (GroundPathNavigation.trimPath with
    # avoidSun: the path stops at the first node the sky sees).
    add('skeleton_sun_path', 120, ['fill -5 -57 -5 5 -57 5 minecraft:stone', 'fill -5 -56 -5 5 -56 5 minecraft:air',
                                   'player 12.5 -60 0.5', 'mob minecraft:skeleton 0.5 -60 0.5 0 211 persistent'], 'time=1000')
    # A zombified piglin runs Zombie.registerGoals too (the turtle egg goal draws its start delay).
    add('zombified_piglin_player', 120, ['player 7.5 -60 0.5', 'mob minecraft:zombified_piglin 0.5 -60 0.5 0 241 persistent'])
    add('wither_skeleton_player', 100, ['player 9.5 -60 0.5', 'mob minecraft:wither_skeleton 0.5 -60 0.5 0 222 persistent'])
    # Chickens fleeing a hit (PanicGoal) beside a pool.
    for seed in range(191, 193):
        add(f'chicken_panic_{seed}', 60, ['fill -3 -61 -6 3 -61 6 minecraft:water',
                                          'player 12.5 -60 6.5', f'mob minecraft:chicken 0.5 -60 6.5 0 {seed} persistent']
            + [f'hurt {t} 0' for t in range(5, 60, 30)])
    add('chicken_panic_dry', 60, ['player 12.5 -60 0.5', 'mob minecraft:chicken 0.5 -60 0.5 0 195 persistent', 'hurt 5 0'])
    add('pig_panic_dry', 60, ['player 12.5 -60 0.5', 'mob minecraft:pig 0.5 -60 0.5 0 196 persistent', 'hurt 5 0'])
    # A wild cat avoids the player it sees (Cat.reassessTameGoals adds that goal last) and sprints while
    # it dashes at 1.33 (Cat.customServerAiStep).
    add('cat_player', 120, ['player 6.5 -60 0.5', 'mob minecraft:cat 0.5 -60 0.5 0 261 persistent'])
    # Axolotls (#64): idle swimming, hunting a cod, and playing dead after hits, in a pool sunk into
    # the ground so no water flows out of it (the probe doesn't tick fluids).
    POOL = 'fill -5 -63 -5 5 -61 5 minecraft:water'
    add('axolotl_hunting', 45, [POOL, 'player 12.5 -60 0.5', 'mob minecraft:axolotl 0.5 -62 0.5 0 12 persistent',
                                'mob minecraft:cod 2.5 -62 0.5 0 13 persistent noai'])
    add('axolotl_play_dead', 300, [POOL, 'player 12.5 -60 0.5', 'mob minecraft:axolotl 0.5 -62 0.5 0 28 persistent']
        + [f'hurt {t} 0' for t in range(4, 160, 12)])
    # Tadpoles (TadpoleAi): swimming about and looking at a player, following frog food, panicking when hit.
    add('tadpole_idle', 200, [POOL, 'player 3.5 -60 0.5', 'mob minecraft:tadpole 0.5 -62 0.5 0 301 persistent'])
    add('tadpole_tempted', 200, [POOL, 'player 4.5 -60 0.5 hand=minecraft:slime_ball',
                                 'mob minecraft:tadpole 0.5 -62 0.5 0 302 persistent'])
    add('tadpole_panic', 100, [POOL, 'player 12.5 -60 0.5', 'mob minecraft:tadpole 0.5 -62 0.5 0 303 persistent',
                               'hurt 5 0'])
    # Fish swimming at random (#3631): RandomSwimmingGoal's position search in a sealed pool.
    for species, seed in [('cod', 181), ('salmon', 182), ('tropical_fish', 183)]:
        add(f'{species}_random_swim', 120, [POOL, 'player 12.5 -60 0.5',
                                            f'mob minecraft:{species} 0.5 -62 0.5 0 {seed} persistent'])
    # FollowFlockLeaderGoal: fish of one kind form a school and the followers swim after their leader.
    for species, seed in [('cod', 210), ('salmon', 220), ('tropical_fish', 230)]:
        # No player near: fleeing it is another goal's matter.
        add(f'{species}_school', 400, [POOL]
            + [f'mob minecraft:{species} {x} -62 {z} {90 * i} {seed + i} persistent'
               for i, (x, z) in enumerate([(-3.5, -2.5), (-1.5, 2.5), (0.5, -0.5), (2.5, 3.5), (3.5, -3.5)])])
    # A jobless villager outside a stone box, its composter inside and the doorway on the far
    # side: the path around needs more than the 256 nodes of the follow range (16 * 16); the
    # villager's setRequiredPathLength(48) raises PathFinder.maxVisitedNodes to 768.
    add('villager_job_in_box', 300, [
        'fill -4 -60 -4 4 -58 4 minecraft:stone',
        'fill -3 -60 -3 3 -58 3 minecraft:air',
        'fill -4 -60 0 -4 -59 0 minecraft:air',
        'block 0 -60 0 minecraft:composter',
        'mob minecraft:villager 10.5 -60 0.5 90 54 persistent',
    ])
    # Villagers that slept today and saw no golem summon an iron golem (Villager.spawnGolemIfNeeded):
    # five gossiping (TradeWithVillager, at tick 200 here), or three panicking at a zombie in a pen
    # (VillagerPanicTrigger, at tick 100 by night). They all remember it (GOLEM_DETECTED_RECENTLY).
    # Under a glass roof three blocks up the golem still fits (IronGolem.checkSpawnObstruction);
    # on dirt paths it cannot stand. The scenarios stop on the summoning tick: the golem's own AI
    # (MoveBackToVillageGoal, GolemRandomStrollInVillageGoal) is not ported yet.
    golem_callers = [f'mob minecraft:villager {x} -60 {z} {r} {seed} persistent slept=1' for x, z, r, seed in
                     [(0.5, 0.5, 0, 81), (2.5, 0.5, 180, 82), (0.5, 2.5, 90, 83), (2.5, 2.5, 270, 84), (1.5, -1.5, 0, 85)]]
    add('villager_golem_gossip', 200, ['player 14.5 -60 0.5'] + golem_callers, 'time=100')
    add('villager_golem_roof', 200, ['fill -10 -57 -12 13 -57 13 minecraft:glass', 'player 14.5 -60 0.5'] + golem_callers,
        'time=100')
    add('villager_golem_paths', 400, ['fill -10 -61 -12 13 -61 13 minecraft:dirt_path', 'player 14.5 -60 0.5'] + golem_callers,
        'time=100')
    add('villager_golem_panic', 100, ['fill -2 -60 -3 4 -60 4 minecraft:oak_fence', 'fill -1 -60 -2 3 -60 3 minecraft:air',
                                      'player 14.5 -60 0.5'] + golem_callers[:3]
        + ['mob minecraft:zombie 6.5 -60 0.5 90 86 persistent noai'], 'time=18000')
    # Mob.checkDespawn removes disallowed types before persistence/NoAI checks.
    # Keep the non-Peaceful controls and the allowed hostile/neutral exceptions.
    for difficulty in range(4):
        for flags in ('', 'persistent', 'named', 'noai', 'persistent noai'):
            suffix = flags.replace(' ', '_') or 'ordinary'
            add(f'peaceful_zombie_{difficulty}_{suffix}', 1, [
                'player 6.5 -60 0.5',
                f'mob minecraft:zombie 0.5 -60 0.5 0 1 {flags}',
            ], f'difficulty={difficulty} time=18000')
    # Not slime, piglin or bee: a slime made without finalizeSpawn keeps 20 health, the probe
    # doesn't model a piglin's brain, and a NoAI bee's flower cooldown differs (none of it Peaceful).
    for species in ('skeleton', 'creeper', 'warden', 'wither', 'shulker', 'wolf', 'iron_golem', 'cow'):
        add(f'peaceful_type_{species}', 1, [
            'player 6.5 -60 0.5',
            f'mob minecraft:{species} 0.5 -60 0.5 0 1 persistent noai',
        ], 'difficulty=0 time=18000')

    return s


def write_scenarios(path, chosen):
    lines = []
    for i, (name, ticks, opts, body) in chosen:
        ox, oz = 512 * (i + 1), 0
        lines.append(f'scenario {name} ticks={ticks} {opts}'.rstrip())
        lines.append(f'origin {ox} {oz}')
        for line in body:
            f = line.split()
            # `at TICK block ...` changes blocks before that tick; `watch` boxes stay relative.
            head, f = (f[:2], f[2:]) if f[0] == 'at' else ([], f)
            if f[0] == 'player':
                f[1] = repr(float(f[1]) + ox)
                f[3] = repr(float(f[3]) + oz)
            elif f[0] == 'mob':
                f[2] = repr(float(f[2]) + ox)
                f[4] = repr(float(f[4]) + oz)
            elif f[0] in ('block', 'heart', 'chest'):
                f[1] = str(int(f[1]) + ox)
                f[3] = str(int(f[3]) + oz)
            elif f[0] == 'fill':
                for k in (1, 4):
                    f[k] = str(int(f[k]) + ox)
                for k in (3, 6):
                    f[k] = str(int(f[k]) + oz)
            lines.append(' '.join(head + f))
        lines.append('end')
    path.write_text('\n'.join(lines) + '\n')


def parse(path):
    """{scenario: {tick: {mob: {field: value}}, sounds: {tick: [..]}}}"""
    result, cur = {}, None
    for line in path.read_text().splitlines():
        if line.startswith('== '):
            cur = result.setdefault(line[3:], {'ticks': {}, 'sounds': {}})
            continue
        head, *fields = line.split('\t')
        parts = head.split(' ', 2)
        t = int(parts[0])
        if parts[1] == 'sound':
            cur['sounds'].setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'block':
            cur.setdefault('blocks', {}).setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'trace':
            cur.setdefault('trace', {}).setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'heart':
            cur.setdefault('hearts', {}).setdefault(t, []).append(line.split(' ', 2)[2])
            continue
        if parts[1] == 'hive':
            cur.setdefault('hives', {}).setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'leads':
            cur.setdefault('leads', {}).setdefault(t, []).append(parts[2])
            continue
        if parts[1] == 'chest':
            cur.setdefault('chests', {}).setdefault(t, []).append(parts[2])
            continue
        mob = int(parts[1])
        row = {}
        if len(parts) > 2:
            row['removed'] = parts[2]
        for f in fields:
            k, _, v = f.partition(' ')
            row[k] = v
        cur['ticks'].setdefault(t, {})[mob] = row
    return result


def decode(value):
    """Hex float/double fields made readable."""
    def one(x):
        try:
            if len(x) > 8:
                return f'{struct.unpack(">d", int(x, 16).to_bytes(8, "big"))[0]!r}'
            return f'{struct.unpack(">f", int(x, 16).to_bytes(4, "big"))[0]:.9g}'
        except ValueError:
            return x
    out = []
    for word in value.split(' '):
        out.append(','.join(one(p) if all(c in '0123456789abcdef' for c in p) and len(p) >= 5 else p for p in word.split(',')))
    return ' '.join(out)


MULT, MASK = 0x5DEECE66D, (1 << 48) - 1


def draws(before, after, limit=20000):
    """How many LegacyRandomSource steps lead from one state to the next."""
    try:
        s, goal = int(before, 16), int(after, 16)
    except (TypeError, ValueError):
        return '?'
    for n in range(limit):
        if s == goal:
            return n
        s = (s * MULT + 11) & MASK
    return f'>{limit}'


def compare(vanilla, rust, fields=None):
    """Returns (report lines, totals)."""
    report, totals = [], {'mob_ticks': 0, 'matched': 0, 'scenarios': 0, 'clean': 0}
    for name, v in vanilla.items():
        r = rust.get(name)
        totals['scenarios'] += 1
        if r is None:
            report.append(f'{name}: missing from Horizon')
            continue
        mobs = sorted({m for t in v['ticks'].values() for m in t})
        clean = True
        for mob in mobs:
            ticks = sorted(v['ticks'])
            first = None
            for t in ticks:
                a = v['ticks'][t].get(mob)
                b = r['ticks'].get(t, {}).get(mob)
                totals['mob_ticks'] += 1
                if a == b or (b is not None and fields and all(a.get(k) == b.get(k) for k in fields)):
                    if first is None:
                        totals['matched'] += 1
                    continue
                if first is None:
                    first = t
                    diffs = []
                    for k in (a or {}):
                        if b is None or a.get(k) != b.get(k):
                            show = (lambda x: x) if k == 'rng' else decode
                            diffs.append(f'{k}: vanilla {show(a.get(k, "?"))} | horizon {show((b or {}).get(k, "?"))}')
                    for k in (b or {}):
                        if a is None or k not in a:
                            diffs.append(f'{k}: vanilla ? | horizon {decode(b[k])}')
                    prev_v = v['ticks'].get(t - 1, {}).get(mob, {}).get('rng')
                    prev_r = r['ticks'].get(t - 1, {}).get(mob, {}).get('rng')
                    report.append(f'{name} mob {mob}: first divergence at tick {t} of {len(ticks)} '
                                  f'(random draws this tick: vanilla {draws(prev_v, (a or {}).get("rng"))}, '
                                  f'horizon {draws(prev_r, (b or {}).get("rng"))})')
                    for line in [x for x in v.get('trace', {}).get(t, []) if x.split(' ', 1)[0] == str(mob)][:40]:
                        report.append('      vanilla ' + line)
                    # Present when the probe is built with the rng-trace feature (see --trace).
                    for line in [x for x in r.get('trace', {}).get(t, []) if x.split(' ', 1)[0] == str(mob)][:40]:
                        report.append('      horizon ' + line)
                    report.extend('    ' + d for d in diffs[:12])
            if first is not None:
                clean = False
        # Creaking hearts (state, block entity, level random): first differing tick.
        for t in sorted(set(v.get('hearts', {})) | set(r.get('hearts', {}))):
            if v.get('hearts', {}).get(t) != r.get('hearts', {}).get(t):
                report.append(f'{name}: hearts differ at tick {t}: vanilla {v.get("hearts", {}).get(t)} | horizon {r.get("hearts", {}).get(t)}')
                clean = False
                break
        # Sounds, watched block changes, chest contents and hives: first differing tick.
        for kind in ('sounds', 'blocks', 'chests', 'hives', 'leads'):
            va, ra = v.get(kind, {}), r.get(kind, {})
            for t in sorted(set(va) | set(ra)):
                if va.get(t, []) != ra.get(t, []):
                    report.append(f'{name}: {kind} differ at tick {t}: vanilla {va.get(t, [])} | horizon {ra.get(t, [])}')
                    clean = False
                    break
        totals['clean'] += clean
        if clean:
            report.append(f'{name}: all {len(v["ticks"])} ticks match')
    return report, totals


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--only', nargs='*')
    parser.add_argument('--java-only', action='store_true')
    parser.add_argument('--rust-only', action='store_true', help='reuse the last vanilla output')
    parser.add_argument('--report-only', action='store_true', help='print divergences without failing')
    parser.add_argument('--trace', action='store_true',
                        help="also trace Horizon's mob random draws (rng-trace probe with line tables)")
    args = parser.parse_args()
    build = lab.LOCAL / 'mob-ai-parity'
    build.mkdir(parents=True, exist_ok=True)
    chosen = [(i, s) for i, s in enumerate(scenarios()) if not args.only or any(o in s[0] for o in args.only)]
    inputs = build / 'scenarios.txt'
    write_scenarios(inputs, chosen)
    lock, jars, _ = base.verify_artifacts()
    if not args.rust_only:
        classes, dependencies, _ = oracle_harness.compile_oracle(build, 'VanillaMobAiOracle', jars, baking.client_libraries(),
                                                                lock['minecraft']['java_major'])
        # The oracle's server world: cleared before each run, never left in /tmp.
        world = build / 'world'
        shutil.rmtree(world, ignore_errors=True)
        # Each scenario forces 9x9 chunks 512 blocks past the last one, so a run generates ~80 chunks per scenario;
        # 3g ran out of heap in PoiManager.tick on a busy runner (#2160's CI, 143 scenarios).
        # 4g then ran out within ~3 minutes on a fresh run once the scenarios grew (2026-10-30); 8g passes.
        oracle_harness.run_logged(['java', '-Xmx8g', '-Djava.awt.headless=true', '-cp', str(classes) + os.pathsep + dependencies,
                                   'dev.horizonlab.parity.VanillaMobAiOracle', inputs, build / 'vanilla.txt', world],
                                  build, 'vanilla', timeout=3600)
    if not args.java_only:
        command = ['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '--example', 'client_probe']
        target, env = lab.cargo_target(), None
        if args.trace:
            target = lab.LOCAL / 'mob-ai-trace-target'
            command += ['--features', 'rng-trace']
            env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_PROFILE_RELEASE_DEBUG='line-tables-only',
                       CARGO_PROFILE_RELEASE_LTO='false', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16')
        oracle_harness.run_logged(command, build, 'compile-rust', env=env)
        binary = target / 'fast/examples/client_probe'
        oracle_harness.run_logged([binary, 'mob_ai_probe', lab.RESOURCES, inputs, build / 'horizon.txt'], build, 'horizon')
    if args.java_only:
        return
    names = {s[0] for _, s in chosen}
    vanilla = {k: v for k, v in parse(build / 'vanilla.txt').items() if k in names}
    report, totals = compare(vanilla, parse(build / 'horizon.txt'))
    (build / 'report.txt').write_text('\n'.join(report) + '\n')
    print('\n'.join(report))
    print(f"{totals['clean']}/{totals['scenarios']} scenarios match entirely; "
          f"{totals['matched']}/{totals['mob_ticks']} mob-ticks match before each mob's first divergence")
    if not args.report_only and totals['clean'] != totals['scenarios']:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
