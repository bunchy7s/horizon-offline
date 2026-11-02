"""Command scripts for tools/test_command_exec_parity.py.

Each group is a list of operations: `cmd` runs a command as the player and
records its messages and results (`record: false` runs setup silently),
`tick` advances the game, `settle` loads the terrain and entities around the
player, `portal` sends the player through the nether portal it stands in,
and `falling` counts physical falling blocks by dimension, state and gravity.
Everything happens high above the terrain (y 200 and up, where both worlds
are air) after the world's entities are removed, so the results do not
depend on world generation.
"""


def cmd(command, record=True):
    op = {'op': 'cmd', 'cmd': command}
    if not record:
        op['record'] = False
    return op


def setup(*commands):
    return [cmd(c, False) for c in commands]


BASE = (0, 200, 0)


def group_setup():
    return [
        *setup('/gamerule spawn_mobs false', '/gamerule mob_drops false', '/gamerule entity_drops false',
               '/gamerule advance_time false', '/gamerule advance_weather false', '/gamerule spawn_wandering_traders false',
               '/gamerule spawn_patrols false', '/gamerule spawn_phantoms false', '/gamerule spawn_wardens false',
               '/gamerule show_advancement_messages false', '/gamemode creative', '/tp @s 8.5 230 8.5 0 0'),
        {'op': 'settle'},
        *setup('/kill @e[type=!player]'),
        {'op': 'tick', 'n': 25},
        *setup('/kill @e[type=!player]'),
        {'op': 'tick', 'n': 25},
        *setup('/fill 0 200 0 15 228 15 air', '/time set 1000', '/weather clear', '/difficulty normal', '/clear @s'),
        # Before the gamerules above, while the oracle waits for its entities to load (a number of ticks that
        # depends on the machine), mobs spawn and some die with their drops on: a bat's death rolled
        # minecraft:entities/bat on 1 vanilla run in 24, one more sequence for the ticking group's
        # /random reset * to count. Every run starts the script without them.
        *setup('/random reset *'),
    ]


def client_commands():
    return [
        cmd('/version'),
        cmd('/spectate'),
        cmd('/spectate @s'),
        cmd('/gamemode spectator'),
        cmd('/spectate'),
        cmd('/spectate @s'),
        cmd('/spectate @s @s'),
        cmd('/gamemode creative'),
        cmd('/posteffect list @s'),
        cmd('/posteffect remove @s minecraft:blur'),
        cmd('/posteffect clear @s'),
        cmd('/posteffect add @s minecraft:blur'),
        cmd('/posteffect add @s minecraft:blur'),
        cmd('/posteffect add @a minecraft:invert'),
        cmd('/posteffect list @s'),
        cmd('/posteffect remove @s minecraft:blur'),
        cmd('/posteffect remove @p minecraft:blur'),
        cmd('/posteffect list @p'),
        cmd('/posteffect clear @s'),
        cmd('/posteffect clear @s'),
        cmd('/waypoint list'),
        cmd('/waypoint modify @s color red'),
        cmd('/waypoint modify @s color dark_aqua'),
        cmd('/waypoint modify @s color hex 12ab3f'),
        cmd('/waypoint modify @s color hex F0A'),
        cmd('/waypoint list'),
        cmd('/waypoint modify @s color reset'),
        cmd('/waypoint modify @s style set minecraft:bowtie'),
        cmd('/waypoint modify @s style reset'),
        cmd('/waypoint list'),
        cmd('/dialog show @s minecraft:server_links'),
        cmd('/dialog show @a minecraft:quick_actions'),
        cmd('/dialog show @s {type:"minecraft:notice",title:"Hello"}'),
        cmd('/dialog clear @s'),
        cmd('/dialog clear @a'),
        cmd('/kick @s'),
        cmd('/kick @s Go away'),
        cmd('/kick Player'),
        cmd('/fetchprofile name Notch'),
        {'op': 'tick', 'n': 5, 'record': True},
        cmd('/fetchprofile id 069a79f4-44e9-4726-a5be-fca90e38aaf5'),
        {'op': 'tick', 'n': 5, 'record': True},
        cmd('/unpublish'),
        cmd('/debug stop'),
        cmd('/debug start'),
        cmd('/debug start'),
        cmd('/debug stop'),
        cmd('/debug stop'),
        cmd('/jfr stop'),
    ]


def game_state():
    return [
        cmd('/gamemode survival'),
        cmd('/gamemode survival'),
        cmd('/gamemode adventure @s'),
        cmd('/gamemode creative @a'),
        cmd('/defaultgamemode survival'),
        cmd('/defaultgamemode creative'),
        cmd('/difficulty'),
        cmd('/difficulty hard'),
        cmd('/difficulty hard'),
        cmd('/difficulty normal'),
        cmd('/seed'),
        cmd('/time set 1000'),
        cmd('/time query daytime'),
        cmd('/time set noon'),
        cmd('/time set night'),
        cmd('/time add 100'),
        cmd('/time set 1000'),
        cmd('/weather rain'),
        cmd('/weather thunder 600'),
        cmd('/weather clear 1d'),
        cmd('/gamerule keep_inventory'),
        cmd('/gamerule keep_inventory true'),
        cmd('/gamerule keep_inventory'),
        cmd('/gamerule keep_inventory false'),
        cmd('/gamerule random_tick_speed 5'),
        cmd('/gamerule random_tick_speed'),
        cmd('/gamerule random_tick_speed 3'),
        cmd('/list'),
        cmd('/say Hello world'),
        cmd('/me waves'),
        cmd('/msg @s secret'),
        cmd('/tellraw @s {"text":"raw","color":"gold","bold":true}'),
        cmd('/tellraw @s ["a",{"text":"b","italic":true},{"translate":"block.minecraft.stone"}]'),
        cmd('/title @s title "Hi"'),
        cmd('/title @s subtitle "Sub"'),
        cmd('/title @s actionbar "Bar"'),
        cmd('/title @s times 10 20 30'),
        cmd('/title @s clear'),
        cmd('/title @s reset'),
        cmd('/help gamemode'),
        cmd('/random value 5..5'),
        cmd('/random roll 7..7'),
        cmd('/tick query'),
        cmd('/stopwatch create harness:a'),
        cmd('/stopwatch create harness:a'),
        cmd('/stopwatch query harness:b'),
    ]


def player_state():
    return [
        *setup('/gamemode survival'),
        cmd('/experience add @s 10'),
        cmd('/experience add @s 3 levels'),
        cmd('/experience query @s levels'),
        cmd('/experience query @s points'),
        cmd('/experience set @s 0 levels'),
        cmd('/xp set @s 5 points'),
        cmd('/experience query @s points'),
        cmd('/effect give @s minecraft:speed 30 1'),
        cmd('/effect give @s minecraft:speed 10 0'),
        cmd('/effect give @s minecraft:night_vision infinite 0 true'),
        cmd('/effect clear @s minecraft:speed'),
        cmd('/effect clear @s minecraft:speed'),
        cmd('/effect clear @s'),
        cmd('/effect clear @s'),
        cmd('/give @s minecraft:diamond 3'),
        cmd('/give @s minecraft:diamond_sword[minecraft:custom_name="Blade"]'),
        cmd('/give @s minecraft:diamond[item_name={text:"ItemName",color:"red"}]'),
        cmd('/give @s minecraft:stone 130'),
        cmd('/clear @s minecraft:diamond 0'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/clear @s minecraft:stone 64'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item replace entity @s weapon.mainhand with minecraft:iron_pickaxe'),
        cmd('/enchant @s minecraft:efficiency 3'),
        cmd('/enchant @s minecraft:sharpness'),
        cmd('/enchant @s minecraft:efficiency 9'),
        cmd('/item replace entity @s armor.head with minecraft:golden_helmet'),
        cmd('/clear @s minecraft:golden_helmet 0'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_count",count:1}'),
        cmd('/clear @s'),
        cmd('/clear @s'),
        # Inventory.clearOrCountMatchingItems visits the off hand as well as
        # the main inventory, and consumes limited clears in inventory-slot order.
        cmd('/item replace entity @s weapon.offhand with minecraft:shield'),
        cmd('/clear @s minecraft:shield 0'),
        cmd('/clear @s minecraft:shield 1'),
        cmd('/clear @s minecraft:shield 0'),
        cmd('/item replace entity @s weapon.offhand with minecraft:stone 5'),
        cmd('/clear @s minecraft:stone 2'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/clear @s'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item replace entity @s armor.head with minecraft:iron_helmet'),
        cmd('/item replace entity @s armor.feet with minecraft:iron_boots'),
        cmd('/clear @s * 1'),
        cmd('/clear @s minecraft:iron_boots 0'),
        cmd('/clear @s minecraft:iron_helmet 0'),
        cmd('/clear @s'),
        cmd('/attribute @s minecraft:max_health get'),
        cmd('/attribute @s minecraft:max_health base get'),
        cmd('/attribute @s minecraft:max_health base set 30'),
        cmd('/attribute @s minecraft:max_health modifier add harness:extra 4 add_value'),
        cmd('/attribute @s minecraft:max_health modifier value get harness:extra'),
        cmd('/attribute @s minecraft:max_health get'),
        cmd('/attribute @s minecraft:max_health modifier remove harness:extra'),
        cmd('/attribute @s minecraft:max_health base reset'),
        cmd('/spawnpoint @s 8 230 8'),
        cmd('/setworldspawn 8 230 8'),
        cmd('/tp @s 8.5 230 8.5'),
        cmd('/tp @s ~1 ~ ~'),
        cmd('/teleport @s 8.5 230 8.5 90 0'),
        cmd('/rotate @s 45 10'),
        cmd('/damage @s 2'),
        cmd('/damage @s 1 minecraft:magic'),
        *setup('/gamemode creative'),
    ]


def world_blocks():
    return [
        cmd('/setblock 2 205 2 minecraft:stone'),
        cmd('/setblock 2 205 2 minecraft:stone'),
        cmd('/setblock 2 205 2 minecraft:oak_stairs[facing=east]'),
        cmd('/execute if block 2 205 2 minecraft:oak_stairs[facing=east]'),
        cmd('/execute if block 2 205 2 minecraft:stone'),
        cmd('/execute unless block 2 205 2 minecraft:stone'),
        cmd('/fill 0 201 0 4 203 4 minecraft:glass'),
        cmd('/fill 0 201 0 4 203 4 minecraft:glass'),
        cmd('/fill 0 201 0 4 203 4 minecraft:stone hollow'),
        cmd('/fill 0 201 0 4 203 4 minecraft:dirt replace minecraft:stone'),
        cmd('/fill 0 201 0 4 203 4 minecraft:air destroy'),
        cmd('/execute if blocks 0 201 0 4 203 4 0 210 0 all'),
        cmd('/setblock 3 206 3 minecraft:chest'),
        cmd('/item replace block 3 206 3 container.0 with minecraft:apple 5'),
        cmd('/data get block 3 206 3 Items[0].count'),
        cmd('/data merge block 3 206 3 {CustomName:"Box"}'),
        cmd('/clone 0 205 0 4 207 4 6 205 6'),
        cmd('/clone 0 205 0 4 207 4 6 205 6 masked'),
        cmd('/execute if blocks 0 205 0 4 207 4 6 205 6 all'),
        cmd('/setblock 3 206 3 minecraft:air'),
        # A container keeps its loot table and seed until its items change
        # (RandomizableContainer.tryLoadLootTable / trySaveLootTable).
        cmd('/setblock 7 206 7 minecraft:chest{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}'),
        cmd('/data get block 7 206 7'),
        cmd('/data get block 7 206 7 LootTableSeed'),
        cmd('/setblock 7 206 7 minecraft:air'),
        # A hopper placed into power turns `enabled=false` and keeps its block entity.
        cmd('/setblock 5 205 5 minecraft:redstone_block'),
        cmd('/setblock 5 206 5 minecraft:hopper'),
        cmd('/execute if block 5 206 5 minecraft:hopper[enabled=false]'),
        cmd('/item replace block 5 206 5 container.0 with minecraft:diamond 3'),
        cmd('/data get block 5 206 5 Items[0].count'),
        cmd('/fill 5 205 4 5 205 4 minecraft:hopper'),
        cmd('/data get block 5 205 4 Items'),
        cmd('/fill 0 200 0 15 228 15 air'),
        cmd('/execute if block 0 300 0 air'),
        cmd('/setblock 0 400 0 stone'),
        cmd('/worldborder get'),
        cmd('/worldborder set 1000'),
        cmd('/worldborder set 1000'),
        cmd('/worldborder center 10 20'),
        cmd('/worldborder center 10 20'),
        cmd('/worldborder damage amount 0.5'),
        cmd('/worldborder damage buffer 3'),
        cmd('/worldborder warning distance 10'),
        cmd('/worldborder warning time 20'),
        cmd('/worldborder add 100'),
        cmd('/worldborder get'),
        cmd('/worldborder set 59999968'),
        cmd('/worldborder center 0 0'),
    ]


def entities():
    return [
        cmd('/summon minecraft:pig 4.5 210 4.5 {NoAI:1b,NoGravity:1b,Tags:["a","h"]}'),
        cmd('/summon minecraft:cow 6.5 210 4.5 {NoAI:1b,NoGravity:1b,CustomName:"Daisy",Tags:["h"]}'),
        cmd('/summon minecraft:armor_stand 8.5 210 4.5 {NoGravity:1b}'),
        cmd('/execute if entity @e[tag=h,type=minecraft:pig]'),
        cmd('/execute if entity @e[tag=h,type=minecraft:sheep]'),
        cmd('/tag @e[tag=h,type=minecraft:pig] list'),
        cmd('/tag @e[tag=h,type=minecraft:cow] add b'),
        cmd('/tag @e[tag=h,type=minecraft:cow] add b'),
        cmd('/tag @e[tag=b] list'),
        cmd('/tag @e[tag=b] remove b'),
        cmd('/data get entity @e[tag=h,type=minecraft:pig,limit=1] Tags'),
        cmd('/data get entity @e[tag=h,type=minecraft:cow,limit=1] CustomName'),
        cmd('/data merge entity @e[tag=h,type=minecraft:pig,limit=1] {Tags:["a","c"]}'),
        cmd('/attribute @e[tag=h,type=minecraft:pig,limit=1] minecraft:movement_speed base get'),
        cmd('/effect give @e[tag=h] minecraft:glowing 10'),
        cmd('/damage @e[tag=h,type=minecraft:cow,limit=1] 1'),
        cmd('/ride @s mount @e[tag=h,type=minecraft:pig,limit=1]'),
        cmd('/ride @s dismount'),
        cmd('/ride @s dismount'),
        cmd('/spectate @e[tag=h,type=minecraft:pig,limit=1]'),
        cmd('/gamemode spectator'),
        cmd('/spectate @e[tag=h,type=minecraft:pig,limit=1]'),
        cmd('/spectate'),
        cmd('/gamemode creative'),
        cmd('/waypoint modify @e[tag=h,type=minecraft:pig,limit=1] color red'),
        cmd('/waypoint modify @e[type=minecraft:armor_stand,limit=1] color red'),
        cmd('/fetchprofile entity @e[tag=h,type=minecraft:pig,limit=1]'),
        cmd('/kill @e[tag=h,type=minecraft:pig]'),
        cmd('/kill @e[tag=h,type=minecraft:pig]'),
        cmd('/execute as @e[tag=h] run say mob'),
        cmd('/execute store result score @s harness_score run data get entity @s SelectedItemSlot'),
        cmd('/kill @e[tag=h]'),
        # A live rider travels with its vehicle in `/data get`, the way `saveWithoutId`
        # writes it (#1519): the pig is mounted, so its health is the nested value.
        cmd('/summon minecraft:cow 4.5 210 8.5 {Tags:["root"],NoAI:1b,Passengers:[{id:"minecraft:pig",NoAI:1b}]}'),
        cmd('/data get entity @e[tag=root,limit=1] Passengers[0].Health'),
        cmd('/data get entity @e[tag=root,limit=1] Passengers[0].id'),
        cmd('/kill @e[tag=root]'),
        # Saved attribute bases are read for every mob, then Health or the max
        # health when there is none (LivingEntity.readAdditionalSaveData, #1483).
        cmd('/summon minecraft:cow 4.5 210 8.5 {Tags:["attr"],NoAI:1b,NoGravity:1b,'
            'attributes:[{id:"minecraft:max_health",base:40.0d},{id:"minecraft:movement_speed",base:0.5d}]}'),
        cmd('/data get entity @e[tag=attr,limit=1] Health'),
        cmd('/attribute @e[tag=attr,limit=1] minecraft:max_health base get'),
        cmd('/attribute @e[tag=attr,limit=1] minecraft:movement_speed base get'),
        cmd('/kill @e[tag=attr]'),
        cmd('/summon minecraft:zombie 4.5 210 8.5 {Tags:["attr"],NoAI:1b,NoGravity:1b,Health:7.0f,'
            'attributes:[{id:"minecraft:attack_damage",base:9.0d}]}'),
        cmd('/data get entity @e[tag=attr,limit=1] Health'),
        cmd('/attribute @e[tag=attr,limit=1] minecraft:attack_damage base get'),
        cmd('/kill @e[tag=attr]'),
        # A summoned mob is not persistent: vanilla never calls
        # `Mob.setPersistenceRequired` for a mob it spawns (#2787).
        cmd('/summon minecraft:zombie 4.5 210 12.5'),
        cmd('/execute if entity @e[type=minecraft:zombie,nbt={PersistenceRequired:1b}]'),
        cmd('/kill @e[type=minecraft:zombie]'),
        # An End crystal is an entity of its own: a type selector finds it,
        # reads it and kills it (#1531).
        cmd('/summon minecraft:end_crystal 4.5 210 8.5'),
        cmd('/execute if entity @e[type=minecraft:end_crystal]'),
        cmd('/data get entity @e[type=minecraft:end_crystal,limit=1] Pos'),
        cmd('/kill @e[type=minecraft:end_crystal]'),
        cmd('/execute if entity @e[type=minecraft:end_crystal]'),
        cmd('/summon minecraft:ender_dragon 4.5 210 8.5'),
        cmd('/execute if entity @e[type=minecraft:ender_dragon]'),
        cmd('/data get entity @e[type=minecraft:ender_dragon,limit=1] Pos'),
        cmd('/kill @e[type=minecraft:ender_dragon]'),
        # A primed TNT is an entity too: a type selector finds, tags, reads,
        # moves and kills it.
        cmd('/summon minecraft:tnt 4.5 210 8.5 {fuse:2000s,Tags:["t"]}'),
        cmd('/execute if entity @e[type=minecraft:tnt]'),
        cmd('/tag @e[type=minecraft:tnt,limit=1] list'),
        cmd('/execute if entity @e[type=minecraft:tnt,tag=t]'),
        cmd('/data get entity @e[type=minecraft:tnt,limit=1] Tags'),
        cmd('/tp @e[type=minecraft:tnt] 4.5 215 8.5'),
        cmd('/kill @e[type=minecraft:tnt]'),
        cmd('/execute if entity @e[type=minecraft:tnt]'),
        # So is a falling block, named after its block (`getTypeName`).
        cmd('/summon minecraft:falling_block 4.5 210 8.5 {BlockState:{Name:"minecraft:sand"},NoGravity:1b,Tags:["f"]}'),
        cmd('/execute if entity @e[type=minecraft:falling_block,tag=f]'),
        cmd('/tag @e[type=minecraft:falling_block,limit=1] list'),
        cmd('/data get entity @e[type=minecraft:falling_block,limit=1] BlockState.Name'),
        cmd('/tp @e[type=minecraft:falling_block] 4.5 215 8.5'),
        cmd('/kill @e[type=minecraft:falling_block]'),
        cmd('/execute if entity @e[type=minecraft:falling_block]'),
        # A firework rocket, with what `addAdditionalSaveData` writes.
        cmd('/summon minecraft:firework_rocket 4.5 210 8.5 {LifeTime:2000,NoGravity:1b,Tags:["r"]}'),
        cmd('/execute if entity @e[type=minecraft:firework_rocket,tag=r]'),
        cmd('/tag @e[type=minecraft:firework_rocket,limit=1] list'),
        cmd('/data get entity @e[type=minecraft:firework_rocket,limit=1] LifeTime'),
        cmd('/data get entity @e[type=minecraft:firework_rocket,limit=1] FireworksItem.id'),
        cmd('/data get entity @e[type=minecraft:firework_rocket,limit=1] ShotAtAngle'),
        cmd('/tp @e[type=minecraft:firework_rocket] 4.5 215 8.5'),
        cmd('/kill @e[type=minecraft:firework_rocket]'),
        cmd('/execute if entity @e[type=minecraft:firework_rocket]'),
        # An ominous item spawner, with an item or empty (`ItemStack.EMPTY`,
        # which saves no `item` and spawns nothing when its time comes).
        cmd('/summon minecraft:ominous_item_spawner 4.5 210 8.5 {spawn_item_after_ticks:2000L,item:{id:"minecraft:diamond",count:2},Tags:["o"]}'),
        cmd('/execute if entity @e[type=minecraft:ominous_item_spawner,tag=o]'),
        cmd('/tag @e[type=minecraft:ominous_item_spawner,limit=1] list'),
        cmd('/data get entity @e[type=minecraft:ominous_item_spawner,limit=1] item'),
        cmd('/data get entity @e[type=minecraft:ominous_item_spawner,limit=1] spawn_item_after_ticks'),
        cmd('/tp @e[type=minecraft:ominous_item_spawner] 4.5 215 8.5'),
        cmd('/kill @e[type=minecraft:ominous_item_spawner]'),
        cmd('/summon minecraft:ominous_item_spawner 4.5 210 8.5'),
        cmd('/data get entity @e[type=minecraft:ominous_item_spawner,limit=1] item'),
        cmd('/kill @e[type=minecraft:ominous_item_spawner]'),
        cmd('/summon minecraft:ominous_item_spawner 4.5 210 8.5 {spawn_item_after_ticks:5L}'),
        {'op': 'tick', 'n': 10},
        cmd('/execute if entity @e[type=minecraft:ominous_item_spawner]'),
        cmd('/execute if entity @e[type=minecraft:item,distance=..3]'),
        # And the thrown items.
        *[c for kind in ('snowball', 'egg', 'ender_pearl') for c in (
            cmd(f'/summon minecraft:{kind} 4.5 210 8.5 {{NoGravity:1b}}'),
            cmd(f'/execute if entity @e[type=minecraft:{kind}]'),
            cmd(f'/data get entity @e[type=minecraft:{kind},limit=1] Item.id'),
            cmd(f'/kill @e[type=minecraft:{kind}]'),
        )],
        *setup('/kill @e[type=minecraft:armor_stand]'),
        {'op': 'tick', 'n': 25},
        cmd('/execute if entity @e[tag=h]'),
    ]


def wither_spawn():
    """A wither charging up (`Invul`) neither moves toward its target nor is hurt by
    the blast that ends the charge (#3322): `WitherBoss.customServerAiStep` sets no
    alternative target while invulnerable, and `ServerLevel.explode`'s entity query
    skips the source."""
    wither = '@e[type=minecraft:wither,limit=1]'
    return [
        *setup('/fill 0 209 4 9 209 20 minecraft:stone',
               '/summon minecraft:pig 4.5 210 18.5 {NoAI:1b,Tags:["wither_prey"]}',
               '/summon minecraft:wither 4.5 210 8.5 {Invul:20}'),
        {'op': 'tick', 'n': 10},
        cmd(f'/data get entity {wither} Pos'),
        cmd(f'/data get entity {wither} Invul'),
        {'op': 'tick', 'n': 12},
        cmd(f'/data get entity {wither} Invul'),
        cmd(f'/data get entity {wither} Health'),
        *setup('/kill @e[type=minecraft:wither]', '/kill @e[tag=wither_prey]', '/fill 0 200 0 15 228 20 air'),
        {'op': 'tick', 'n': 5},
    ]


def riding_teleport():
    """Player teleports remove the vehicle, leave it behind and keep the destination (#1473)."""
    horse = '@e[tag=tp_horse,limit=1]'
    operations = setup(
        '/summon minecraft:horse 4.5 220 4.5 {Tame:1b,NoAI:1b,NoGravity:1b,Tags:["tp_horse"]}',
        '/summon minecraft:marker 8.5 220 0.5 {Tags:["tp_destination"]}',
    )
    for teleport in ('/tp @s 8.5 220 0.5', '/teleport @s ~4 220 ~-4',
                     '/tp @s @e[tag=tp_destination,limit=1]'):
        operations += [
            *setup('/tp @s 4.5 220 4.5'),
            cmd(f'/ride @s mount {horse}'),
            cmd('/execute on vehicle run say mounted'),
            # An invalid destination must not dismount the player.
            cmd('/tp @s 30000000 220 0'),
            cmd('/execute on vehicle run say still mounted'),
            cmd(teleport),
            cmd('/execute on vehicle run say still riding after teleport'),
            cmd('/data get entity @s Pos'),
            cmd(f'/data get entity {horse} Pos'),
            {'op': 'tick', 'n': 60},
            cmd('/execute on vehicle run say still riding after ticks'),
            cmd('/data get entity @s Pos'),
            cmd(f'/data get entity {horse} Pos'),
            cmd('/ride @s dismount'),
        ]
    return operations + setup('/kill @e[tag=tp_horse]', '/kill @e[tag=tp_destination]',
                              '/tp @s 8.5 230 8.5 0 0')


def mob_riding():
    """Mobs mount and dismount mobs with /ride, NBT passengers included (#1520)."""
    rider, vehicle = '@e[tag=ride_r,limit=1]', '@e[tag=ride_v,limit=1]'
    return [
        *setup('/summon minecraft:cow 4.5 220 12.5 {NoAI:1b,NoGravity:1b,Tags:["ride_v"]}',
               '/summon minecraft:pig 6.5 220 12.5 {NoAI:1b,NoGravity:1b,Tags:["ride_r"]}'),
        cmd(f'/ride {rider} dismount'),
        cmd(f'/ride {rider} mount @s'),
        cmd(f'/ride {rider} mount {rider}'),
        cmd(f'/ride {rider} mount {vehicle}'),
        cmd(f'/ride {rider} mount {vehicle}'),
        cmd(f'/ride {vehicle} mount {rider}'),
        cmd('/execute as @e[tag=ride_r] on vehicle run say carrying'),
        cmd(f'/ride {rider} dismount'),
        cmd(f'/data get entity {rider} Pos'),
        cmd('/execute as @e[tag=ride_r] on vehicle run say still carrying'),
        *setup('/kill @e[tag=ride_r]', '/kill @e[tag=ride_v]'),
        *setup('/summon minecraft:cow 4.5 220 12.5 {NoAI:1b,NoGravity:1b,Tags:["ride_v"],'
               'Passengers:[{id:"minecraft:pig",NoAI:1b,NoGravity:1b,Tags:["ride_r"]}]}'),
        cmd(f'/ride {rider} dismount'),
        cmd(f'/data get entity {rider} Pos'),
        *setup('/kill @e[tag=ride_r]', '/kill @e[tag=ride_v]'),
    ]


def camel_riders():
    """A husk riding a camel (#3199): the camel and its rider never push each other
    (Entity.push skips passengers of the same vehicle), so the camel stays near
    where it was summoned instead of sliding off backwards, and it never sits down
    with a controlling passenger (CamelAi.RandomSitting needs none)."""
    camel = '@e[tag=camel_v,limit=1]'
    return [
        # A walled pen, and a camel that cannot walk: only a push would move it.
        *setup('/fill 0 200 0 15 203 15 minecraft:glass hollow', '/fill 1 203 1 14 203 14 minecraft:air',
               '/scoreboard objectives add camel_pose dummy',
               '/summon minecraft:camel 8.0 201 8.0 {Tags:["camel_v"],'
               'Passengers:[{id:"minecraft:husk",Tags:["camel_r"]}]}',
               f'/attribute {camel} minecraft:movement_speed base set 0'),
        {'op': 'tick', 'n': 100},
        cmd('/execute as @e[tag=camel_r] on vehicle run say carrying'),
        cmd('/execute if entity @e[tag=camel_v,x=8,y=201,z=8,distance=..4]'),
        {'op': 'tick', 'n': 500},
        *setup(f'/execute store result score camel camel_pose run data get entity {camel} LastPoseTick'),
        # Negative while sitting.
        cmd('/execute if score camel camel_pose matches ..-1'),
        cmd('/execute as @e[tag=camel_r] on vehicle run say still carrying'),
        *setup('/kill @e[tag=camel_r]', '/kill @e[tag=camel_v]', '/scoreboard objectives remove camel_pose',
               '/fill 0 200 0 15 203 15 minecraft:air'),
    ]


def scores():
    return [
        cmd('/scoreboard objectives add harness dummy'),
        cmd('/scoreboard objectives add harness dummy'),
        cmd('/scoreboard objectives add harness_h health "Health"'),
        cmd('/scoreboard objectives list'),
        cmd('/scoreboard players set @s harness 5'),
        cmd('/scoreboard players add @s harness 3'),
        cmd('/scoreboard players remove @s harness 1'),
        cmd('/scoreboard players get @s harness'),
        cmd('/scoreboard players set fake harness 9'),
        cmd('/scoreboard players operation @s harness += fake harness'),
        cmd('/scoreboard players get @s harness'),
        cmd('/scoreboard players list'),
        cmd('/scoreboard players list @s'),
        cmd('/scoreboard players reset fake harness'),
        cmd('/scoreboard players get fake harness'),
        cmd('/scoreboard objectives setdisplay list harness'),
        cmd('/scoreboard objectives setdisplay sidebar harness'),
        cmd('/scoreboard objectives modify harness displayname "Shown"'),
        cmd('/scoreboard objectives modify harness rendertype hearts'),
        cmd('/execute store result score @s harness run scoreboard players get @s harness'),
        cmd('/execute if score @s harness matches 15'),
        cmd('/team add red_team'),
        cmd('/team add red_team'),
        cmd('/team modify red_team color red'),
        cmd('/team modify red_team prefix "[R] "'),
        cmd('/team join red_team @s'),
        cmd('/team list'),
        cmd('/team list red_team'),
        cmd('/waypoint list'),
        cmd('/team leave @s'),
        cmd('/team remove red_team'),
        cmd('/scoreboard objectives remove harness'),
        cmd('/scoreboard objectives remove harness_h'),
        cmd('/bossbar add harness:bar "Bar"'),
        cmd('/bossbar add harness:bar "Bar"'),
        cmd('/bossbar set harness:bar value 30'),
        cmd('/bossbar set harness:bar max 60'),
        cmd('/bossbar set harness:bar color red'),
        cmd('/bossbar set harness:bar style notched_10'),
        cmd('/bossbar set harness:bar players @s'),
        cmd('/bossbar get harness:bar value'),
        cmd('/bossbar get harness:bar players'),
        cmd('/bossbar list'),
        cmd('/bossbar remove harness:bar'),
        cmd('/bossbar remove harness:bar'),
    ]


def execute():
    return [
        *setup('/scoreboard objectives add exec dummy', '/scoreboard players set @s exec 4', '/scoreboard players set fake exec 9'),
        cmd('/execute as @s run say hi'),
        cmd('/execute as @s at @s run tp @s ~ ~ ~'),
        cmd('/execute positioned 4 210 4 run setblock ~ ~ ~ minecraft:stone'),
        cmd('/execute positioned 4 210 4 if block ~ ~ ~ minecraft:stone'),
        cmd('/execute positioned as @s run tp @s ~ ~ ~'),
        cmd('/execute align xyz run summon minecraft:armor_stand ~ ~-20 ~ {NoGravity:1b,Tags:["ex"],UUID:[I;1,2,3,4]}'),
        cmd('/summon minecraft:armor_stand 3 210 3 {NoGravity:1b,Tags:["implicit_tp"]}'),
        cmd('/execute as @e[tag=implicit_tp,limit=1] run tp 4 210 4'),
        cmd('/data get entity @e[tag=implicit_tp,limit=1] Pos'),
        cmd('/kill @e[tag=implicit_tp]'),
        cmd('/execute if entity @e[tag=ex]'),
        # A UUID names any entity (ServerLevel.getEntity), not only mobs (#4966).
        cmd('/execute if entity 00000001-0000-0002-0000-000300000004'),
        cmd('/data get entity 00000001-0000-0002-0000-000300000004 Tags'),
        cmd('/summon minecraft:item_display 2.5 210 2.5 {UUID:[I;5,6,7,8],Tags:["uuid_display"]}'),
        cmd('/data get entity 00000005-0000-0006-0000-000700000008 Tags'),
        cmd('/tp 00000005-0000-0006-0000-000700000008 2.5 215 2.5'),
        cmd('/data get entity 00000005-0000-0006-0000-000700000008 Pos'),
        cmd('/kill 00000005-0000-0006-0000-000700000008'),
        # The killed id stays in the UUID map. A re-summon of that UUID must
        # still resolve to the entity the level holds.
        cmd('/summon minecraft:item_display 2.5 210 2.5 {UUID:[I;5,6,7,8],Tags:["uuid_display_2"]}'),
        cmd('/data get entity 00000005-0000-0006-0000-000700000008 Tags'),
        cmd('/kill 00000005-0000-0006-0000-000700000008'),
        cmd('/summon minecraft:marker 2.5 210 8.5 {UUID:[I;13,14,15,16],Tags:["uuid_marker"]}'),
        cmd('/execute as 0000000d-0000-000e-0000-000f00000010 run data get entity @s Tags'),
        cmd('/kill 0000000d-0000-000e-0000-000f00000010'),
        cmd('/data get entity 00000015-0000-0016-0000-001700000018'),
        cmd('/summon minecraft:pig 2.5 210 6.5 {NoAI:1b,NoGravity:1b,UUID:[I;9,10,11,12]}'),
        cmd('/kill 00000009-0000-000a-0000-000b0000000c'),
        # OwnableEntity.getOwner → EntityReference.getLivingEntity: an armor
        # stand is an owner, a marker is not.
        cmd('/summon minecraft:wolf 2.5 210 14.5 {NoAI:1b,NoGravity:1b,Sitting:1b,Tags:["uuid_pet"],Owner:[I;1,2,3,4]}'),
        cmd('/execute as @e[tag=uuid_pet,limit=1] on owner run data get entity @s Tags'),
        cmd('/summon minecraft:marker 4.5 210 14.5 {UUID:[I;17,18,19,20]}'),
        cmd('/summon minecraft:wolf 6.5 210 14.5 {NoAI:1b,NoGravity:1b,Sitting:1b,Tags:["uuid_pet_marker"],Owner:[I;17,18,19,20]}'),
        cmd('/execute as @e[tag=uuid_pet_marker,limit=1] on owner run say reached'),
        cmd('/kill @e[tag=uuid_pet]'),
        cmd('/kill @e[tag=uuid_pet_marker]'),
        cmd('/kill 00000011-0000-0012-0000-001300000014'),
        cmd('/execute as @e[tag=ex] run data get entity @s Pos'),
        cmd('/data get entity @e[tag=ex,limit=1]'),
        cmd('/data get entity @e[tag=ex,limit=1] Pose'),
        cmd('/execute anchored eyes run tp @s ^ ^ ^'),
        cmd('/execute rotated 90 0 run tp @s ~ ~ ~ ~ ~'),
        cmd('/execute rotated as @e[tag=ex,limit=1] run tp @s ~ ~ ~ ~ ~'),
        cmd('/execute facing 0 230 0 run tp @s ~ ~ ~ ~ ~'),
        cmd('/execute facing entity @e[tag=ex,limit=1] feet run tp @s ~ ~ ~ ~ ~'),
        cmd('/execute in minecraft:the_nether run setblock 0 200 0 minecraft:stone'),
        cmd('/execute in minecraft:the_end if block 0 64 0 minecraft:air'),
        cmd('/execute in minecraft:the_end run say elsewhere'),
        cmd('/execute in minecraft:nowhere run say nowhere'),
        cmd('/execute in minecraft:overworld run setblock 5 210 5 minecraft:dirt'),
        cmd('/execute if dimension minecraft:overworld'),
        cmd('/execute if dimension minecraft:the_end'),
        cmd('/execute if loaded 8 230 8'),
        cmd('/execute if loaded 100000 230 100000'),
        cmd('/execute if score @s exec matches 4'),
        cmd('/execute if score @s exec < fake exec'),
        cmd('/execute if score @s exec = fake exec'),
        cmd('/execute unless score @s exec matches 1..3'),
        cmd('/execute if score nobody exec matches 1'),
        cmd('/execute if data entity @s Inventory'),
        cmd('/execute if data entity @e[tag=ex,limit=1] Tags[0]'),
        cmd('/execute if data block 4 210 4 Items'),
        cmd('/execute if data storage harness:s value'),
        cmd('/execute if predicate minecraft:tool/can_shear'),
        cmd('/execute if predicate {condition:"minecraft:random_chance",chance:1.0}'),
        cmd('/execute if predicate {condition:"minecraft:entity_properties",entity:"this",predicate:{type:"minecraft:player"}}'),
        cmd('/execute if items entity @s weapon.mainhand minecraft:air'),
        cmd('/execute if items entity @s container.* *'),
        cmd('/execute store result score @s exec run data get entity @s SelectedItemSlot'),
        cmd('/execute store success score @s exec run setblock 4 210 4 minecraft:stone'),
        cmd('/scoreboard players get @s exec'),
        cmd('/execute store result storage harness:s value int 2 run scoreboard players get fake exec'),
        cmd('/data get storage harness:s value'),
        cmd('/execute store result entity @e[tag=ex,limit=1] Rotation[0] float 1 run scoreboard players get fake exec'),
        cmd('/data get entity @e[tag=ex,limit=1] Rotation'),
        cmd('/execute store result block 4 210 4 x int 1 run scoreboard players get fake exec'),
        cmd('/execute summon minecraft:armor_stand run tag @s add summoned'),
        cmd('/execute if entity @e[tag=summoned]'),
        cmd('/execute as @e[tag=ex] on passengers run say passenger'),
        cmd('/execute on vehicle run say vehicle'),
        cmd('/execute as @e[tag=ex] run return 3'),
        cmd('/execute if entity @s run return fail'),
        cmd('/return 7'),
        cmd('/return fail'),
        cmd('/return run scoreboard players get fake exec'),
        cmd('/execute if biome 8 230 8 #minecraft:is_overworld'),
        cmd('/execute if block 8 0 8 #minecraft:base_stone_overworld'),
        cmd('/execute as @a run tellraw @s "each"'),
        *setup('/kill @e[type=minecraft:armor_stand]', '/setblock 4 210 4 minecraft:air', '/setblock 5 210 5 minecraft:air',
               '/scoreboard objectives remove exec', '/tp @s 8.5 230 8.5 0 0'),
        {'op': 'tick', 'n': 25},
        *setup('/gamemode spectator'),
        cmd('/execute in minecraft:the_nether if dimension minecraft:the_nether run say source dimension'),
        cmd('/execute in minecraft:the_nether at @s if dimension minecraft:overworld run say entity dimension'),
        cmd('/execute in minecraft:the_nether positioned as @s if dimension minecraft:the_nether run say position only'),
        cmd('/execute in minecraft:the_nether if loaded 0 100 0'),
        cmd('/execute in minecraft:the_nether run tp @s 0 100 0'),
        {'op': 'settle'},
        cmd('/execute if dimension minecraft:the_nether run say dimension switched'),
        cmd('/data get entity @s Pos'),
        # Back to the terrain loaded since the start (x and z scaled by 8: 12 230 4). The fresh
        # ocean chunks at 644 100 -316 broke some of their kelp and seagrass while they loaded,
        # on some runs only (vanilla loads chunks asynchronously): that rolled those blocks' loot
        # and added their random sequences to the count of /random reset *.
        cmd('/execute in minecraft:the_nether positioned 1 230 0 in minecraft:overworld run tp @s ~ ~ ~'),
        {'op': 'settle'},
        cmd('/execute if dimension minecraft:overworld run say relative dimension'),
        cmd('/data get entity @s Pos'),
        cmd('/execute in minecraft:overworld positioned 80 100 -40 in minecraft:the_nether run tp @s ~ ~ ~ 45 30'),
        {'op': 'settle'},
        cmd('/data get entity @s Pos'),
        cmd('/data get entity @s Rotation'),
        cmd('/execute in minecraft:overworld run tp @s 8.5 230 8.5'),
        {'op': 'settle'},
        cmd('/execute if dimension minecraft:overworld run say dimension restored'),
        cmd('/data get entity @s Pos'),
        *setup('/gamemode creative'),
    ]


def levels():
    """A nether portal round trip: on the way back the Overworld's entities (the
    pig left by `entities`) are found and a summon names what it summoned. An
    unbounded selector also reaches the level the player is not in, while a
    bounded one does not."""
    return [
        *setup('/fill 2 231 12 5 236 12 minecraft:obsidian', '/fill 3 232 12 4 235 12 minecraft:nether_portal[axis=x]',
               '/tp @s 3.5 232 12.5 0 0',
               # A cow in the Overworld, in a chunk that stays loaded while the
               # player is in the nether.
               '/forceload add 0 0',
               '/summon minecraft:cow 10.5 210 4.5 {NoAI:1b,NoGravity:1b,Tags:["remote_cow"]}'),
        {'op': 'portal'},
        {'op': 'settle'},
        cmd('/execute if dimension minecraft:the_nether run say through the portal'),
        cmd('/data get entity @e[tag=remote_cow,limit=1] Pos'),
        cmd('/data get entity @e[tag=remote_cow,distance=..128,limit=1] Pos'),
        {'op': 'portal'},
        {'op': 'settle'},
        cmd('/execute if dimension minecraft:overworld run say back through the portal'),
        cmd('/data get entity @s Pos'),
        cmd('/data get entity @s Rotation'),
        cmd('/execute if entity @e[type=minecraft:pig]'),
        cmd('/summon minecraft:armor_stand 4.5 210 4.5 {NoGravity:1b,Tags:["lv"],CustomName:"Back"}'),
        *setup('/kill @e[tag=lv]', '/fill 2 231 12 5 236 12 minecraft:air', '/tp @s 8.5 230 8.5 0 0'),
        {'op': 'settle'},
    ]


def end_portal_entities():
    """Non-player exit travel with two ticking levels, and a falling block
    crossing the portal's inside shape on the tick it reaches the floor."""
    end = '/execute in minecraft:the_end run'
    local = 'distance=..1000000'
    return [
        *setup('/forceload add 0 0', '/setblock 8 229 8 minecraft:stone', '/setworldspawn 8 230 8',
               f'{end} tp @s 8.5 230 8.5'),
        {'op': 'settle'},
        *setup(f'{end} forceload add 0 0',
               f'{end} fill 2 209 2 6 209 6 minecraft:stone',
               f'{end} fill 2 210 2 6 215 6 minecraft:air',
               f'{end} setblock 4 210 4 minecraft:end_portal',
               f'{end} summon minecraft:pig 4.5 210.5 4.5 {{NoAI:1b,NoGravity:1b,Tags:["end_portal_mob"]}}'),
        {'op': 'tick', 'n': 10},
        cmd(f'{end} execute if entity @e[tag=end_portal_mob,{local}]'),
        cmd(f'/execute in minecraft:overworld if entity @e[tag=end_portal_mob,{local}]'),
        *setup(f'{end} summon minecraft:falling_block 4.5 213 4.5 {{BlockState:{{id:"minecraft:sand"}},Time:1,Motion:[0.0d,-2.0d,0.0d],Tags:["end_portal_falling"]}}'),
        {'op': 'tick', 'n': 10},
        cmd('/execute in minecraft:overworld if block 8 230 8 minecraft:sand'),
        cmd(f'{end} execute if block 4 210 4 minecraft:end_portal'),
        *setup('/execute in minecraft:overworld run setblock 8 230 8 minecraft:air',
               f'{end} summon minecraft:falling_block 4.5 210.5 4.5 {{BlockState:{{id:"minecraft:gravel"}},Time:1,NoGravity:1b,Tags:["end_portal_float"]}}'),
        {'op': 'tick', 'n': 5},
        {'op': 'falling', 'dimension': 'minecraft:the_end', 'block': 'minecraft:gravel', 'no_gravity': True},
        {'op': 'falling', 'dimension': 'minecraft:overworld', 'block': 'minecraft:gravel', 'no_gravity': True, 'discard': True},
        # FallingBlock.isFree(end_portal) is false: a placed egg above it
        # stays a block in vanilla, despite the portal's empty collision shape.
        *setup(f'{end} setblock 4 211 4 minecraft:dragon_egg'),
        {'op': 'tick', 'n': 5},
        cmd(f'{end} execute if block 4 211 4 minecraft:dragon_egg'),
        *setup('/kill @e[tag=end_portal_mob]',
               f'{end} fill 2 209 2 6 215 6 minecraft:air',
               f'{end} forceload remove 0 0',
               '/execute in minecraft:overworld run fill 8 229 8 8 231 8 minecraft:air',
               '/execute in minecraft:overworld run tp @s 8.5 230 8.5'),
        {'op': 'settle'},
    ]


def items_and_loot():
    return [
        *setup('/clear @s', '/gamemode survival'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_count",count:3}'),
        cmd('/item replace entity @s weapon.mainhand with minecraft:stone 10'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_count",count:3}'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_count",count:5,add:true}'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:limit_count",limit:{max:4}}'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_name",name:"Rock"}'),
        cmd('/data get entity @s SelectedItem'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_name",name:{text:"Item",color:"red"},target:"item_name"}'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_lore",lore:["one","two"],mode:"replace_all"}'),
        cmd('/item modify entity @s weapon.mainhand [{type:"minecraft:set_count",count:2},{type:"minecraft:set_custom_data",tag:{a:1b}}]'),
        cmd('/data get entity @s SelectedItem'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_components",components:{"minecraft:max_stack_size":16}}'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:filtered",item_filter:{items:"minecraft:dirt"},on_pass:{type:"minecraft:set_count",count:1}}'),
        cmd('/clear @s minecraft:stone 0'),
        cmd('/item replace entity @s weapon.mainhand with minecraft:diamond_sword'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_damage",damage:0.5}'),
        cmd('/data get entity @s SelectedItem.components'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_enchantments",enchantments:{"minecraft:sharpness":3}}'),
        cmd('/data get entity @s SelectedItem.components."minecraft:enchantments"'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:set_enchantments",enchantments:{"minecraft:sharpness":1},add:true}'),
        cmd('/data get entity @s SelectedItem.components."minecraft:enchantments"'),
        cmd('/item modify entity @s weapon.mainhand {type:"minecraft:toggle_tooltips",toggles:{"minecraft:enchantments":false}}'),
        cmd('/item modify entity @s armor.head {type:"minecraft:set_count",count:1}'),
        cmd('/item modify entity @s weapon.offhand {type:"minecraft:set_count",count:1}'),
        cmd('/item modify entity @s hotbar.* {type:"minecraft:set_count",count:1}'),
        cmd('/item modify entity @s weapon.mainhand minecraft:nonexistent'),
        cmd('/item replace entity @s hotbar.1 from entity @s weapon.mainhand'),
        cmd('/item replace entity @s hotbar.2 from entity @s weapon.mainhand {type:"minecraft:set_count",count:1}'),
        cmd('/clear @s minecraft:diamond_sword 0'),
        cmd('/setblock 3 206 3 minecraft:barrel'),
        cmd('/item replace block 3 206 3 container.4 with minecraft:emerald 7'),
        cmd('/item modify block 3 206 3 container.4 {type:"minecraft:set_count",count:2}'),
        cmd('/item modify block 3 206 3 container.5 {type:"minecraft:set_count",count:2}'),
        cmd('/item modify block 3 206 3 container.99 {type:"minecraft:set_count",count:2}'),
        cmd('/item modify block 3 207 3 container.0 {type:"minecraft:set_count",count:2}'),
        cmd('/data get block 3 206 3 Items'),
        cmd('/data get block 3 206 3'),
        cmd('/item replace entity @s hotbar.3 from block 3 206 3 container.4'),
        cmd('/item replace block 3 206 3 container.0 from entity @s hotbar.3'),
        cmd('/data get block 3 206 3 Items'),
        cmd('/loot give @s loot minecraft:blocks/stone'),
        cmd('/loot give @s loot minecraft:blocks/dirt'),
        cmd('/loot insert 3 206 3 loot minecraft:blocks/oak_planks'),
        cmd('/loot replace block 3 206 3 container.8 loot minecraft:blocks/cobblestone'),
        cmd('/loot replace entity @s hotbar.5 loot minecraft:blocks/glass'),
        cmd('/loot spawn 3 210 3 loot minecraft:blocks/stone'),
        cmd('/loot give @s loot {pools:[{rolls:1,entries:[{type:"minecraft:item",name:"minecraft:apple",functions:[{type:"minecraft:set_count",count:4}]}]}]}'),
        cmd('/setblock 2 205 2 minecraft:stone'),
        cmd('/loot give @s mine 2 205 2'),
        cmd('/loot give @s mine 2 205 2 mainhand'),
        cmd('/loot give @s mine 2 205 2 minecraft:diamond_pickaxe[minecraft:enchantments={"minecraft:silk_touch":1}]'),
        cmd('/data get block 3 206 3 Items'),
        cmd('/clear @s minecraft:cobblestone 0'),
        cmd('/clear @s minecraft:apple 0'),
        cmd('/give @s minecraft:written_book[minecraft:written_book_content={title:"T",author:"A",pages:["p"]}]'),
        cmd('/give @s minecraft:potion[minecraft:potion_contents={potion:"minecraft:swiftness"}] 2'),
        cmd('/give @s minecraft:air'),
        cmd('/give @s minecraft:stone 6401'),
        cmd('/clear @s #minecraft:logs'),
        cmd('/clear @s *[minecraft:custom_data]'),
        cmd('/clear @s'),
        # A chest or hopper minecart's slots (ContainerEntity.getChestVehicleSlot): container.N below its size.
        *setup('/summon minecraft:chest_minecart 4.5 210 4.5 {NoGravity:1b,Tags:["c"]}',
               '/summon minecraft:hopper_minecart 6.5 210 4.5 {NoGravity:1b,Tags:["h"]}'),
        cmd('/item replace entity @e[tag=c,limit=1] container.0 with minecraft:diamond 3'),
        cmd('/item replace entity @e[tag=c,limit=1] container.26 with minecraft:emerald 2'),
        cmd('/item replace entity @e[tag=c,limit=1] container.27 with minecraft:emerald 2'),
        cmd('/item replace entity @e[tag=h,limit=1] container.4 with minecraft:gold_ingot 5'),
        cmd('/item replace entity @e[tag=h,limit=1] container.5 with minecraft:gold_ingot 5'),
        cmd('/item modify entity @e[tag=c,limit=1] container.0 {type:"minecraft:set_count",count:2}'),
        cmd('/item replace entity @s hotbar.0 from entity @e[tag=c,limit=1] container.26'),
        cmd('/data get entity @e[tag=c,limit=1] Items'),
        cmd('/execute if items entity @e[tag=h,limit=1] container.* minecraft:gold_ingot'),
        cmd('/clear @s'),
        *setup('/kill @e[tag=c]', '/kill @e[tag=h]'),
        *setup('/setblock 3 206 3 minecraft:air', '/setblock 2 205 2 minecraft:air', '/kill @e[type=minecraft:item]', '/gamemode creative'),
    ]


def world_misc():
    return [
        cmd('/setblock 1 210 1 minecraft:stone keep'),
        cmd('/setblock 1 210 1 minecraft:dirt keep'),
        cmd('/setblock 1 210 1 minecraft:dirt destroy'),
        cmd('/setblock 1 210 1 minecraft:sand strict'),
        cmd('/setblock 1 210 1 minecraft:air replace'),
        cmd('/fill 0 210 0 2 212 2 minecraft:stone outline'),
        cmd('/fill 0 210 0 2 212 2 minecraft:glass keep'),
        cmd('/fill 0 210 0 2 212 2 minecraft:dirt replace #minecraft:base_stone_overworld'),
        cmd('/fill 0 210 0 2 212 2 minecraft:air strict'),
        # A connected server loads more terrain than the oracle's 5x5 core.
        # Keep the far corner unambiguously unloaded in both executions.
        cmd('/fill 0 210 0 1000 300 1000 minecraft:stone'),
        cmd('/setblock 0 211 0 minecraft:oak_sign{front_text:{messages:["a","b","c","d"]}}'),
        cmd('/data get block 0 211 0 front_text.messages'),
        cmd('/clone 0 211 0 0 211 0 1 211 1 replace move'),
        cmd('/clone 1 211 1 1 211 1 0 211 0 filtered minecraft:oak_sign'),
        cmd('/clone 0 211 0 0 211 0 0 211 0'),
        cmd('/clone 0 211 0 0 211 0 0 212 0 replace force'),
        cmd('/clone from minecraft:overworld 0 211 0 0 211 0 to minecraft:overworld 2 211 2'),
        cmd('/fill 0 210 0 2 212 2 minecraft:air'),
        cmd('/fillbiome 0 200 0 15 215 15 minecraft:desert'),
        cmd('/execute if biome 4 205 4 minecraft:desert'),
        cmd('/fillbiome 0 200 0 15 215 15 minecraft:plains replace minecraft:desert'),
        cmd('/fillbiome 0 200 0 15 215 15 minecraft:plains replace minecraft:desert'),
        cmd('/fillbiome 0 0 0 1000 300 1000 minecraft:plains'),
        cmd('/forceload query'),
        cmd('/forceload add 0 0'),
        cmd('/forceload add 0 0'),
        cmd('/forceload query 0 0'),
        cmd('/forceload query 40 40'),
        cmd('/forceload add 32 32 47 47'),
        cmd('/forceload query'),
        cmd('/forceload remove 32 32'),
        cmd('/forceload remove all'),
        cmd('/forceload query'),
        cmd('/forceload add 0 0 1000 1000'),
        cmd('/particle minecraft:flame 4 210 4'),
        cmd('/particle minecraft:flame 4 300 4'),
        cmd('/particle minecraft:flame 4 300 4 0 0 0 0 1 force'),
        cmd('/particle minecraft:flame 4 300 4 0 0 0 0 1 force @e[type=minecraft:pig]'),
        cmd('/particle minecraft:dust{color:[1.0,0.0,0.0],scale:1.0} 4 210 4 0 0 0 0 5 force @s'),
        cmd('/particle minecraft:block{block_state:"minecraft:stone"} 4 210 4 1 1 1 0.1 10 normal @a'),
        cmd('/playsound minecraft:block.note_block.harp master @s'),
        cmd('/playsound minecraft:block.note_block.harp record @s 4 210 4 1 2 0'),
        cmd('/playsound minecraft:block.note_block.harp master @a 1000 210 1000'),
        cmd('/stopsound @s'),
        cmd('/stopsound @s master'),
        cmd('/stopsound @s * minecraft:block.note_block.harp'),
        cmd('/stopsound @s music minecraft:music.game'),
        cmd('/swing @s'),
        cmd('/swing @s offhand'),
        cmd('/swing @e[type=minecraft:pig]'),
        cmd('/summon minecraft:armor_stand 4.5 210 4.5 {NoGravity:1b,Tags:["w"],CustomName:{text:"Stand",color:"blue"}}'),
        cmd('/swing @e[tag=w]'),
        cmd('/summon minecraft:lightning_bolt 4.5 300 4.5'),
        # Primed TNT is no command target in Horizon: still "Summoned new
        # Primed TNT" (it explodes high in the air, far from the rest).
        cmd('/summon minecraft:tnt 60.5 300 60.5 {fuse:1s}'),
        cmd('/summon minecraft:pig 4.5 210 4.5 {NoAI:1b,Invulnerable:1b,Tags:["w"],Passengers:[{id:"minecraft:chicken",NoAI:1b,Tags:["w"]}]}'),
        cmd('/summon minecraft:pig 4.5 400 4.5'),
        cmd('/summon minecraft:pig 30000000 210 4.5'),
        cmd('/summon minecraft:player'),
        cmd('/summon minecraft:marker 4.5 210 4.5 {Tags:["w","mk"]}'),
        cmd('/gamemode spectator'),
        cmd('/spectate @e[tag=mk,limit=1]'),
        cmd('/spectate @e[type=minecraft:armor_stand,tag=w,limit=1]'),
        cmd('/spectate @e[type=minecraft:pig,tag=w,limit=1] @s'),
        cmd('/spectate @e[type=minecraft:chicken,tag=w,limit=1] Player'),
        cmd('/spectate'),
        cmd('/spectate @e[tag=nothing,limit=1]'),
        cmd('/gamemode creative'),
        # The waypoint manager is a HashSet of entities (identity hash order),
        # so at most one transmitter at a time. Range changes take effect when
        # the entity ticks.
        cmd('/attribute @s minecraft:waypoint_transmit_range base set 0'),
        cmd('/waypoint list'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        cmd('/attribute @e[type=minecraft:pig,tag=w,limit=1] minecraft:waypoint_transmit_range base set 100'),
        cmd('/attribute @e[type=minecraft:pig,tag=w,limit=1] minecraft:waypoint_transmit_range get'),
        cmd('/waypoint list'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        cmd('/waypoint modify @e[type=minecraft:pig,tag=w,limit=1] color gold'),
        cmd('/waypoint modify @e[tag=mk,limit=1] color gold'),
        cmd('/waypoint list'),
        cmd('/waypoint modify @e[tag=w] color gold'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:max_health get'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale modifier add harness:big 1 add_multiplied_base'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale get'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale modifier add harness:big 1 add_value'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale modifier value get harness:big'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale modifier remove harness:big'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:scale modifier remove harness:big'),
        cmd('/attribute @e[type=minecraft:armor_stand,tag=w,limit=1] minecraft:attack_damage get'),
        cmd('/attribute @e[tag=mk,limit=1] minecraft:scale get'),
        cmd('/attribute @e[type=minecraft:pig,tag=w,limit=1] minecraft:waypoint_transmit_range base set 0'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        cmd('/waypoint modify @e[type=minecraft:pig,tag=w,limit=1] style set minecraft:bowtie'),
        cmd('/waypoint list'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        cmd('/kill @e[type=minecraft:pig,tag=w]'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        {'op': 'tick', 'n': 25},
        cmd('/waypoint list'),
        cmd('/attribute @s minecraft:waypoint_transmit_range base reset'),
        {'op': 'tick', 'n': 1},
        cmd('/waypoint list'),
        cmd('/dialog show @s minecraft:custom_options'),
        cmd('/dialog show @s minecraft:nonexistent'),
        cmd('/dialog show @s {type:"minecraft:confirmation",title:"Sure?",yes:{label:"Yes"},no:{label:"No"}}'),
        cmd('/dialog show @s {type:"minecraft:notice"}'),
        cmd('/dialog show @e[tag=w] minecraft:server_links'),
        cmd('/dialog clear @e[tag=w]'),
        cmd('/dialog clear @s'),
        cmd('/spreadplayers 8 8 0 1 false @s'),
        cmd('/spreadplayers 8 8 5 1 false @s'),
        cmd('/spreadplayers 8 8 0 1 under 100 false @s'),
        cmd('/kill @e[tag=w]'),
        cmd('/tp @s 8.5 230 8.5 0 0'),
        cmd('/teleport @s 8.5 230 8.5 facing 0 230 0'),
        cmd('/teleport @s 8.5 230 8.5 facing entity @s'),
        cmd('/tp @s 30000000 230 0'),
        cmd('/tp 8.5 230 8.5'),
        cmd('/rotate @s facing 0 230 0'),
        cmd('/rotate @s ~10 ~'),
        cmd('/tp @s 8.5 230 8.5 0 0'),
        cmd('/locate biome minecraft:plains'),
        cmd('/locate biome #minecraft:is_ocean'),
        cmd('/locate structure minecraft:village_plains'),
        cmd('/locate structure #minecraft:village'),
        cmd('/locate structure minecraft:end_city'),
        cmd('/locate poi minecraft:meeting'),
        cmd('/place template minecraft:nonexistent 4 210 4'),
        cmd('/place feature minecraft:oak 4 210 4'),
        cmd('/place jigsaw minecraft:village/plains/town_centers minecraft:bottom 1 4 210 4'),
        cmd('/place structure minecraft:nonexistent'),
        *setup('/kill @e[type=!minecraft:player]', '/fill 0 200 0 15 228 15 air', '/forceload remove all', '/tp @s 8.5 230 8.5 0 0'),
        {'op': 'tick', 'n': 25},
    ]


def progression():
    return [
        *setup('/gamemode survival', '/experience set @s 0 levels', '/experience set @s 0 points'),
        cmd('/advancement grant @s only minecraft:story/mine_stone'),
        cmd('/advancement grant @s only minecraft:story/mine_stone'),
        cmd('/advancement revoke @s only minecraft:story/mine_stone'),
        cmd('/advancement revoke @s only minecraft:story/mine_stone'),
        cmd('/advancement grant @s only minecraft:story/upgrade_tools minecraft:stone_pickaxe'),
        cmd('/advancement grant @s only minecraft:story/upgrade_tools minecraft:stone_pickaxe'),
        cmd('/advancement grant @s only minecraft:story/upgrade_tools minecraft:nonexistent'),
        cmd('/advancement revoke @s only minecraft:story/upgrade_tools minecraft:stone_pickaxe'),
        cmd('/advancement grant @s until minecraft:story/smelt_iron'),
        cmd('/advancement grant @s from minecraft:story/root'),
        cmd('/advancement revoke @s through minecraft:story/iron_tools'),
        cmd('/advancement grant @s everything'),
        cmd('/advancement grant @s everything'),
        cmd('/experience query @s levels'),
        cmd('/experience query @s points'),
        cmd('/advancement revoke @s everything'),
        cmd('/advancement revoke @s everything'),
        cmd('/advancement grant @s only minecraft:nonexistent'),
        cmd('/recipe give @s minecraft:oak_planks'),
        cmd('/recipe give @s minecraft:oak_planks'),
        cmd('/recipe take @s minecraft:oak_planks'),
        cmd('/recipe take @s minecraft:oak_planks'),
        cmd('/recipe give @s *'),
        cmd('/recipe give @s *'),
        cmd('/recipe take @s *'),
        cmd('/recipe take @s *'),
        cmd('/experience add @s -5 levels'),
        cmd('/experience add @s 1000'),
        cmd('/experience query @s levels'),
        cmd('/experience set @s 100 points'),
        cmd('/experience set @s 1000 points'),
        cmd('/experience add @e[type=!minecraft:player] 1'),
        *setup('/gamemode creative'),
    ]


def functions_and_packs():
    return [
        cmd('/function minecraft:nonexistent'),
        cmd('/function #minecraft:load'),
        cmd('/function #minecraft:tick'),
        cmd('/debug function minecraft:nonexistent'),
        cmd('/schedule function minecraft:nonexistent 10'),
        cmd('/schedule clear minecraft:nonexistent'),
        cmd('/datapack list'),
        cmd('/datapack list available'),
        cmd('/datapack list enabled'),
        cmd('/datapack enable vanilla'),
        # Disabling vanilla reloads without its data; enable it again before
        # anything observes the empty registries.
        cmd('/datapack disable vanilla'),
        {'op': 'tick', 'n': 20},
        cmd('/datapack list enabled'),
        cmd('/datapack enable vanilla'),
        {'op': 'tick', 'n': 20},
        cmd('/datapack list'),
        cmd('/datapack enable "file/missing"'),
        cmd('/datapack enable trade_rebalance'),
        cmd('/datapack disable trade_rebalance'),
        cmd('/reload'),
        {'op': 'tick', 'n': 5, 'record': True},
    ]


def ticking():
    return [
        # Isolate named-stream commands from earlier loot rolls, including
        # ocean vegetation updated while asynchronous terrain loads settle.
        *setup('/random reset *'),
        cmd('/tick query'),
        cmd('/tick rate 40'),
        cmd('/tick query'),
        cmd('/tick rate 20'),
        cmd('/tick freeze'),
        cmd('/tick query'),
        cmd('/tick freeze'),
        cmd('/tick step'),
        cmd('/tick step 5'),
        cmd('/tick step stop'),
        cmd('/tick step stop'),
        cmd('/tick unfreeze'),
        cmd('/tick unfreeze'),
        cmd('/tick step'),
        cmd('/tick sprint stop'),
        cmd('/tick rate 1'),
        cmd('/tick rate 20'),
        cmd('/stopwatch create harness:t'),
        cmd('/stopwatch restart harness:t'),
        cmd('/stopwatch restart harness:missing'),
        cmd('/stopwatch remove harness:t'),
        cmd('/stopwatch remove harness:t'),
        cmd('/stopwatch remove harness:a'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random roll 1..6 harness:dice'),
        cmd('/random reset harness:seq'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random reset harness:seq 42'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random reset harness:seq 42 false false'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random reset *'),
        cmd('/random value 1..100 harness:seq'),
        cmd('/random roll 1..2147483647 harness:big'),
        cmd('/random value -2147483648..2147483647 harness:big'),
        cmd('/time query day'),
        cmd('/time set 0'),
        cmd('/time set day'),
        cmd('/time set midnight'),
        cmd('/time set sunset'),
        cmd('/time add -500'),
        cmd('/time set 1000'),
    ]


def compute():
    return [
        cmd('/compute default integer 5'),
        cmd('/compute default integer {type:"minecraft:add",left:2,right:3}'),
        cmd('/compute default integer {type:"minecraft:add",inputs:[2,3,4]}'),
        cmd('/compute default integer {type:"minecraft:add",inputs:[2147483647,1]}'),
        cmd('/compute default integer {type:"minecraft:mul",inputs:[6,7]}'),
        cmd('/compute default integer {type:"minecraft:avg",inputs:[1,2,4]}'),
        cmd('/compute default integer {type:"minecraft:div",left:7,right:2}'),
        cmd('/compute default integer {type:"minecraft:div",left:7,right:0}'),
        cmd('/compute default integer {type:"minecraft:mod",left:-7,right:3}'),
        cmd('/compute default integer {type:"minecraft:floor_mod",left:-7,right:3}'),
        cmd('/compute default integer {type:"minecraft:floor_div",left:-7,right:2}'),
        cmd('/compute default integer {type:"minecraft:sub",left:-2147483648,right:1}'),
        cmd('/compute default integer {type:"minecraft:pow",left:2,right:10}'),
        cmd('/compute default integer {type:"minecraft:pow",base:2,exponent:10}'),
        cmd('/compute default integer {type:"minecraft:pow",base:0,exponent:0}'),
        cmd('/compute default integer {type:"minecraft:pow",base:2,exponent:40}'),
        cmd('/compute default integer {type:"minecraft:max",values:[1,9,4]}'),
        cmd('/compute default integer {type:"minecraft:max",inputs:[1,9,4]}'),
        cmd('/compute default integer {type:"minecraft:min",inputs:[1,9,4]}'),
        cmd('/compute default integer {type:"minecraft:abs",input:-4}'),
        cmd('/compute default integer {type:"minecraft:abs",input:-2147483648}'),
        cmd('/compute default integer {type:"minecraft:negate",input:4}'),
        cmd('/compute default integer {type:"minecraft:from_float",input:2.7}'),
        cmd('/compute default integer {type:"minecraft:from_float",input:-2.7}'),
        cmd('/compute default integer {type:"minecraft:from_float",input:{type:"minecraft:div",left:1,right:0}}'),
        cmd('/compute default integer {type:"minecraft:uniform",min:3,max:3}'),
        cmd('/compute default integer {type:"minecraft:conditional",condition:{type:"minecraft:inverted",term:{type:"minecraft:match_block",blocks:"minecraft:stone"}},on_true:5}'),
        cmd('/compute default integer {type:"minecraft:nope"}'),
        cmd('/compute default integer minecraft:brewing/uses_default'),
        cmd('/compute default integer minecraft:cooking/time_coal'),
        cmd('/compute default integer minecraft:cooking/fast_burn_time_reduction_factor'),
        cmd('/compute default integer minecraft:nonexistent'),
        cmd('/compute default float 2.5'),
        cmd('/compute default float 2.5 2'),
        cmd('/compute default float 3'),
        cmd('/compute default float -0.5'),
        cmd('/compute default float 1e30'),
        cmd('/compute default float {type:"minecraft:div",left:1,right:0}'),
        cmd('/compute default float {type:"minecraft:mod",left:1,right:0}'),
        cmd('/compute default float {type:"minecraft:sqrt",value:2}'),
        cmd('/compute default float {type:"minecraft:sqrt",input:2}'),
        cmd('/compute default float {type:"minecraft:sqrt",input:2} 1000'),
        cmd('/compute default float {type:"minecraft:sqrt",input:-1}'),
        cmd('/compute default float {type:"minecraft:mul",left:1.5,right:4}'),
        cmd('/compute default float {type:"minecraft:mul",inputs:[1.5,4]}'),
        cmd('/compute default float {type:"minecraft:sub",left:1.5,right:4}'),
        cmd('/compute default float {type:"minecraft:floor",input:-1.5}'),
        cmd('/compute default float {type:"minecraft:ceil",input:-1.5}'),
        cmd('/compute default float {type:"minecraft:round",input:2.5}'),
        cmd('/compute default float {type:"minecraft:round",input:-2.5}'),
        cmd('/compute default float {type:"minecraft:truncate",input:-2.7}'),
        cmd('/compute default float {type:"minecraft:from_int",input:3}'),
        cmd('/compute default float {type:"minecraft:avg",inputs:[1,2]}'),
        cmd('/compute default float {type:"minecraft:length",inputs:[3,4]}'),
        cmd('/compute default float {type:"minecraft:pow",base:2,exponent:0.5} 100'),
        cmd('/compute default float {type:"minecraft:sin",input:1} 1000'),
        cmd('/compute default float {type:"minecraft:cos",input:1} 1000'),
        cmd('/compute default float {type:"minecraft:abs",input:-3.25}'),
        cmd('/compute default float {type:"minecraft:negate",input:3.25}'),
        cmd('/compute default float {type:"minecraft:max",inputs:[]}'),
        cmd('/compute default float minecraft:brewing/speed_default'),
        cmd('/compute default float minecraft:cooking/speed_default 100'),
        cmd('/compute block 8 229 8 float minecraft:cooking/speed_default'),
        *setup('/setblock 8 229 8 minecraft:smoker'),
        cmd('/compute block 8 229 8 float minecraft:cooking/speed_default'),
        cmd('/compute block 8 229 8 integer minecraft:cooking/time_coal'),
        *setup('/setblock 8 229 8 minecraft:composter[level=0]'),
        cmd('/compute block 8 229 8 integer minecraft:compostable/low'),
        *setup('/setblock 8 229 8 minecraft:air'),
        cmd('/compute entity @s integer {type:"minecraft:add",inputs:[1,1]}'),
        cmd('/compute entity @e[type=minecraft:pig,limit=1] integer 1'),
        cmd('/compute block 8 400 8 integer 1'),
        cmd('/compute block 100000 64 100000 integer 1'),
        cmd('/execute store result score @s cmp run compute default integer 5'),
    ]


def social():
    return [
        *setup('/scoreboard objectives add trig trigger', '/scoreboard objectives add plain dummy'),
        cmd('/trigger trig'),
        cmd('/scoreboard players enable @s trig'),
        cmd('/scoreboard players enable @s trig'),
        cmd('/scoreboard players enable @s plain'),
        cmd('/trigger trig'),
        cmd('/trigger trig'),
        cmd('/scoreboard players enable @s trig'),
        cmd('/trigger trig add 5'),
        cmd('/scoreboard players enable @s trig'),
        cmd('/trigger trig set 3'),
        cmd('/scoreboard players get @s trig'),
        cmd('/trigger plain'),
        cmd('/scoreboard players display name @s trig "Shown"'),
        cmd('/scoreboard players display numberformat @s trig styled {color:"red"}'),
        cmd('/scoreboard players display numberformat @s trig fixed "X"'),
        cmd('/scoreboard players display numberformat @s trig blank'),
        cmd('/scoreboard players display numberformat @s trig'),
        cmd('/scoreboard objectives modify trig numberformat fixed "Y"'),
        cmd('/scoreboard objectives modify trig displayautoupdate true'),
        cmd('/scoreboard objectives setdisplay sidebar'),
        cmd('/scoreboard objectives setdisplay sidebar.team.red trig'),
        cmd('/scoreboard players operation @s trig = @s trig'),
        cmd('/scoreboard players operation @s trig /= @s plain'),
        cmd('/scoreboard players operation @s trig >< @s plain'),
        cmd('/scoreboard players list'),
        cmd('/scoreboard players reset @s'),
        cmd('/scoreboard players list @s'),
        cmd('/teammsg hello'),
        cmd('/team add blue "Blue Team"'),
        cmd('/team join blue'),
        cmd('/teammsg hello team'),
        cmd('/tm short'),
        cmd('/team modify blue friendlyFire false'),
        cmd('/team modify blue friendlyFire false'),
        cmd('/team modify blue seeFriendlyInvisibles false'),
        cmd('/team modify blue nametagVisibility hideForOtherTeams'),
        cmd('/team modify blue deathMessageVisibility never'),
        cmd('/team modify blue collisionRule pushOwnTeam'),
        cmd('/team modify blue suffix " [B]"'),
        cmd('/team modify blue displayName "Blues"'),
        cmd('/team modify blue color reset'),
        cmd('/team list blue'),
        cmd('/team empty blue'),
        cmd('/team empty blue'),
        cmd('/team join blue fake1 fake2'),
        cmd('/team list blue'),
        cmd('/team leave fake1'),
        cmd('/team remove blue'),
        cmd('/team list'),
        cmd('/msg @s'),
        cmd('/tell Player hi'),
        cmd('/w @a hi all'),
        cmd('/me'),
        cmd('/say @s says'),
        cmd('/tellraw @s {"selector":"@s"}'),
        cmd('/tellraw @s {"score":{"name":"@s","objective":"trig"}}'),
        cmd('/tellraw @s {"nbt":"SelectedItemSlot","entity":"@s"}'),
        cmd('/tellraw @s {"nbt":"Inventory","entity":"@s","plain":true}'),
        cmd('/tellraw @s {"nbt":"value","storage":"harness:s","color":"red"}'),
        cmd('/tellraw @s {"nbt":"nothing","entity":"@s"}'),
        cmd('/tellraw @s {"text":"click","click_event":{"action":"run_command","command":"/say x"},"underlined":true}'),
        cmd('/title @s title {"selector":"@s"}'),
        cmd('/list uuids'),
        cmd('/help'),
        cmd('/help execute'),
        cmd('/help scoreboard players'),
        cmd('/help nonexistent'),
        *setup('/scoreboard objectives remove trig', '/scoreboard objectives remove plain'),
    ]


def test_command():
    # `/test run|runmultiple` of an existing instance start a real test (a
    # tick-based run, not compared here); `verify`, `create`, `locate` and
    # `pos` with test instance blocks around are not ported.
    return [
        cmd('/test clearall'),
        cmd('/test stop'),
        cmd('/test pos'),
        cmd('/test locate minecraft:always_pass'),
        cmd('/test runclosest'),
        cmd('/test resetclosest'),
        cmd('/test runfailed'),
        cmd('/test clearthese'),
        cmd('/test runthese'),
        cmd('/test runthat 2 true'),
        cmd('/test runfailed true'),
        cmd('/test resetthese'),
        cmd('/test resetthat'),
        cmd('/test clearthat'),
        cmd('/test clearall 10'),
        cmd('/test pos myPos'),
        cmd('/test locate *'),
        cmd('/test run minecraft:nothing*'),
        cmd('/test locate nothing'),
        *setup('/summon minecraft:marker 8 230 8 {Tags:["tp"]}'),
        cmd('/execute as @e[tag=tp] run test pos'),
        cmd('/execute positioned 0 230 0 run test pos'),
        cmd('/execute as @e[tag=tp] run kill @e[tag=nothing]'),
        cmd('/execute as @e[tag=tp] at @s run kill @e[tag=nothing]'),
        cmd('/execute positioned 1 2 3 run kill @e[tag=nothing]'),
        *setup('/kill @e[tag=tp]'),
    ]


def server_clock():
    """Only explicit tick operations advance the script's authoritative clock."""
    return [
        *setup('/scoreboard objectives add harness_clock dummy',
               '/execute store result score before harness_clock run time query gametime'),
        {'op': 'tick', 'n': 3},
        *setup('/execute store result score elapsed harness_clock run time query gametime',
               '/scoreboard players operation elapsed harness_clock -= before harness_clock'),
        cmd('/scoreboard players get elapsed harness_clock'),
        # The connected owner is the only player; @s and @a use the same body.
        cmd('/execute as @a run say connected owner'),
        *setup('/scoreboard objectives remove harness_clock'),
    ]


def wither_breaks():
    """WitherBoss.customServerAiStep 20 ticks after it is hurt: Level.destroyBlock around it, which leaves
    each cell's fluid (FluidState.createLegacyBlock), so water stays and a waterlogged block turns to water."""
    return [
        *setup('/fill 4 206 4 12 217 12 minecraft:glass hollow', '/fill 5 207 5 11 216 11 minecraft:water',
               '/setblock 7 210 7 minecraft:stone', '/setblock 9 211 9 minecraft:oak_stairs[waterlogged=true]',
               '/setblock 7 212 9 minecraft:oak_slab[waterlogged=false]'),
        {'op': 'tick', 'n': 10},
        *setup('/summon minecraft:wither 8.5 210 8.5 {NoGravity:1b,Tags:["wb"]}'),
        cmd('/damage @e[tag=wb,limit=1] 1'),
        # WaterAvoidingRandomFlyingGoal may set off on a stroll (the wither's random is unseeded in vanilla),
        # which moves the box destroyBlocksTick breaks. Hold it in place each tick until then: customServerAiStep
        # runs before travel, so the break sees the teleported position.
        *[op for _ in range(25) for op in (*setup('/tp @e[tag=wb] 8.5 210 8.5'), {'op': 'tick', 'n': 1})],
        {'op': 'tick', 'n': 15},
        cmd('/execute if block 8 212 8 minecraft:water'),
        cmd('/execute if block 9 211 9 minecraft:water'),
        cmd('/execute if block 9 211 9 minecraft:oak_stairs'),
        cmd('/execute if block 7 210 7 minecraft:stone'),
        cmd('/execute if block 7 212 9 minecraft:oak_slab'),
        # The broken cells' neighbours were updated (setBlock flag 3): the pool flows back in.
        cmd('/execute if block 7 210 7 minecraft:water'),
        cmd('/execute if block 7 212 9 minecraft:water'),
        cmd('/execute if block 6 210 6 minecraft:water'),
        *setup('/kill @e[tag=wb]', '/fill 4 206 4 12 217 12 minecraft:air'),
        {'op': 'tick', 'n': 25},
    ]


def tnt_minecart_rails():
    """A primed TNT minecart (MinecartTNT.isPrimed) on a powered activator rail inside an obsidian box: its
    blast neither resists on nor breaks a rail or the block under one (getBlockExplosionResistance and
    shouldBlockExplode, #4992), but breaks the stone beside them; the obsidian holds."""
    return [
        *setup('/fill 3 203 3 7 207 7 minecraft:obsidian', '/fill 4 204 4 6 206 6 minecraft:air',
               '/setblock 5 204 5 minecraft:redstone_block', '/setblock 5 205 5 minecraft:activator_rail',
               '/setblock 4 204 5 minecraft:stone', '/setblock 2 205 5 minecraft:stone'),
        cmd('/summon minecraft:tnt_minecart 5.5 205.0625 5.5'),
        {'op': 'tick', 'n': 5},
        cmd('/execute if entity @e[type=minecraft:tnt_minecart]'),
        {'op': 'tick', 'n': 100},
        cmd('/execute if entity @e[type=minecraft:tnt_minecart]'),
        cmd('/execute if block 5 205 5 minecraft:activator_rail'),
        cmd('/execute if block 5 204 5 minecraft:redstone_block'),
        cmd('/execute if block 4 204 5 minecraft:stone'),
        cmd('/execute if block 3 205 5 minecraft:obsidian'),
        cmd('/execute if block 2 205 5 minecraft:stone'),
        *setup('/kill @e[type=!player]', '/fill 2 203 3 7 207 7 minecraft:air'),
        {'op': 'tick', 'n': 5},
    ]


def ender_dragon():
    """`/kill` on the fight's dragon ends the fight (#3319): `EnderDragon.kill` removes it and calls
    `EnderDragonFight.setDragonKilled`, so no new dragon takes its place, the exit portal opens and the
    first dragon leaves its egg. The player stays over the exit portal, in the fight (`validPlayer`)."""
    end = '/execute in minecraft:the_end run'
    return [
        *setup('/gamemode creative', f'{end} tp @s 0.5 100 0.5 0 0'),
        {'op': 'settle'},
        # updatePlayers, the arena scan and createNewDragon once the 17 by 17 arena chunks are block ticking
        # (EnderDragonFight.isArenaLoaded). The oracle generates them on worker threads: a tick op with a
        # `record` key gives it a moment of wall time before its ticks.
        *[{'op': 'tick', 'n': 20, 'record': False}] * 10,
        cmd('/execute if entity @e[type=minecraft:ender_dragon]'),
        cmd('/kill @e[type=minecraft:ender_dragon]'),
        {'op': 'tick', 'n': 40},
        cmd('/execute if entity @e[type=minecraft:ender_dragon]'),
        cmd(f'{end} fill -3 0 -3 3 120 3 minecraft:air replace minecraft:end_portal'),
        cmd(f'{end} fill -3 0 -3 3 120 3 minecraft:air replace minecraft:dragon_egg'),
        *setup('/execute in minecraft:overworld run tp @s 8.5 230 8.5 0 0'),
        {'op': 'settle'},
    ]


def place_structure():
    """`/place structure` (PlaceCommand.placeStructure): the id is resolved, a
    `StructureStart` is generated for the position's chunk with vanilla's
    `biome -> true` predicate, and its `placeInChunk` runs for every chunk of the
    start's box in the live level. Buried treasure's one piece digs down from the
    ocean floor and leaves a single chest, so a chest count in its chunk compares
    the generated start and its placement without knowing the terrain height."""
    return [
        *setup('/gamemode creative', '/tp @s 8.5 230 8.5 0 0'),
        {'op': 'settle'},
        *setup('/kill @e[type=!minecraft:player]',
               '/fill 0 0 0 15 120 15 minecraft:air replace minecraft:chest'),
        cmd('/place structure minecraft:buried_treasure 4 210 4'),
        # One chest between y 0 and 120 of chunk (0, 0): the piece's column is
        # inside that chunk whatever height its dig reached.
        cmd('/fill 0 0 0 15 120 15 minecraft:air replace minecraft:chest'),
    ]


def tame_wolf_kills():
    # LivingEntity.resolvePlayerResponsibleForDamage: a tame wolf's hit is its
    # owner's (the harness player, OfflinePlayer:Player), so the kill drops
    # experience and ServerPlayer.awardKillScore counts it (mob_kills,
    # totalKillCount); the killed statistic stays the wolf's
    # (Player.killedEntity). A wild wolf's kill is nobody's.
    owner = '[I;-1608632253,-450807400,-1786055591,-2146546415]'
    return [
        *setup('/gamerule mob_drops true', '/scoreboard objectives add wolf_kills minecraft.killed:minecraft.zombie',
               '/scoreboard objectives add wolf_mob_kills minecraft.custom:minecraft.mob_kills',
               '/scoreboard objectives add wolf_total totalKillCount'),
        cmd('/summon minecraft:wolf 4.5 210 4.5 {NoAI:1b,NoGravity:1b,Tags:["wild"]}'),
        cmd('/summon minecraft:zombie 6.5 210 4.5 {NoAI:1b,NoGravity:1b,Tags:["prey"]}'),
        cmd('/damage @e[tag=prey,limit=1] 100 minecraft:mob_attack by @e[tag=wild,limit=1]'),
        {'op': 'tick', 'n': 25},
        cmd('/execute if entity @e[type=minecraft:experience_orb,limit=1]'),
        cmd('/scoreboard players get @s wolf_kills'),
        cmd('/scoreboard players get @s wolf_mob_kills'),
        cmd('/scoreboard players get @s wolf_total'),
        cmd(f'/summon minecraft:wolf 4.5 210 8.5 {{NoAI:1b,NoGravity:1b,Owner:{owner},Tags:["pet"]}}'),
        cmd('/summon minecraft:zombie 6.5 210 8.5 {NoAI:1b,NoGravity:1b,Tags:["prey"]}'),
        cmd('/damage @e[tag=prey,limit=1] 100 minecraft:mob_attack by @e[tag=pet,limit=1]'),
        {'op': 'tick', 'n': 25},
        cmd('/execute if entity @e[type=minecraft:experience_orb,limit=1]'),
        cmd('/scoreboard players get @s wolf_kills'),
        cmd('/scoreboard players get @s wolf_mob_kills'),
        cmd('/scoreboard players get @s wolf_total'),
        *setup('/kill @e[type=!player]', '/scoreboard objectives remove wolf_kills',
               '/scoreboard objectives remove wolf_mob_kills', '/scoreboard objectives remove wolf_total',
               '/gamerule mob_drops false'),
    ]


def experience_orb_fall():
    """`ExperienceOrb.tick`'s fall and landing (the dragon-death XP rain, #4781):
    gravity 0.03 (skipped while the orb is inside blocks), one drag of
    `getAirDrag()` times the ground's friction scaled over every axis, and a
    landing bounce of `-fallSpeed * 0.4` gated on `fallSpeed < -getGravity()` —
    not a fixed restitution. The orb spawns 16 blocks from the player, out of
    the 8 block follow radius, 45 blocks above a stone platform; the track
    draws no random, so both games compare to the digit."""
    return [
        *setup('/fill 20 214 20 28 214 28 minecraft:stone'),
        cmd('/summon minecraft:experience_orb 24.5 260 24.5 {Value:1s}'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        {'op': 'tick', 'n': 1},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Motion'),
        {'op': 'tick', 'n': 64},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Motion'),
        {'op': 'tick', 'n': 20},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Motion'),
        {'op': 'tick', 'n': 35},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Motion'),
        *setup('/kill @e[type=minecraft:experience_orb]', '/fill 20 214 20 28 214 28 minecraft:air'),
        {'op': 'tick', 'n': 5},
    ]


def creative_shulker_drops():
    """ShulkerBoxBlock.playerWillDestroy: Creative keeps a filled box as one item,
    even with block_drops off. Empty boxes and survival use the ordinary drop rules."""
    colors = ('white orange magenta light_blue yellow lime pink gray light_gray cyan '
              'purple blue brown green red black').split()
    blocks = ['shulker_box', *(f'{color}_shulker_box' for color in colors)]
    item = '@e[type=minecraft:item,limit=1]'
    contents = ('Items:[{Slot:0b,id:"minecraft:diamond_sword",count:1,components:{'
                '"minecraft:custom_name":"Blade","minecraft:enchantments":{"minecraft:sharpness":5}}},'
                '{Slot:26b,id:"minecraft:apple",count:16}],')
    out = []
    for block in blocks:
        for mode, filled, drops in [('creative', True, True), ('creative', False, True),
                                    ('creative', True, False), ('survival', True, True),
                                    ('survival', False, True), ('survival', True, False)]:
            out += setup('/kill @e[type=minecraft:item]', f'/gamemode {mode}',
                         f'/gamerule block_drops {str(drops).lower()}',
                         f'/setblock 4 210 4 minecraft:{block}'
                         '{' + (contents if filled else '') + 'TestOnly:7}')
            out += [{'op': 'break', 'pos': [4, 210, 4]},
                    cmd('/execute if block 4 210 4 minecraft:air'),
                    cmd('/execute if entity @e[type=minecraft:item]')]
            if (mode == 'creative' and filled) or (mode == 'survival' and drops):
                out += [cmd(f'/data get entity {item} Item'),
                        cmd(f'/execute if data entity {item} Item.components."minecraft:block_entity_data"')]
                if mode == 'creative':
                    out += [cmd(f'/data get entity {item} Pos'), cmd(f'/data get entity {item} PickupDelay')]
    out += setup('/kill @e[type=minecraft:item]', '/gamemode creative', '/gamerule block_drops true')
    return out


def piglin_pickups():
    """PiglinAi.pickUpItem/stopHoldingOffHandItem: babies keep admired gold, adults barter it."""
    ops = [*setup('/fill 2 209 2 6 212 6 minecraft:glass hollow',
                  '/fill 3 209 3 5 209 5 minecraft:stone')]
    for baby, held in ((False, "crossbow"), (True, None), (True, "golden_sword"),
                       (True, "crossbow")):
        # NoAI would skip admiration expiry. A small pen keeps both the piglin
        # and its item together without suppressing the server's normal ticks.
        # Finish the previous corpse's death ticks before clearing its drops.
        ops += [
            *setup('/kill @e[type=!player]'),
            {'op': 'tick', 'n': 25},
            *setup('/kill @e[type=minecraft:item]',
                   '/summon minecraft:piglin 4.5 210 4.5 '
                   '{Tags:["barter"],IsImmuneToZombification:1b,PersistenceRequired:1b,drop_chances:{mainhand:0.0f},IsBaby:'
                   + ('1b' if baby else '0b')
                   + (',equipment:{mainhand:{id:"minecraft:' + held + '",count:1}}' if held else '') + '}'),
            cmd('/execute if items entity @e[tag=barter,limit=1] weapon.mainhand *'),
            *setup('/summon minecraft:item 4.5 210 4.5 '
                   '{PickupDelay:0s,Item:{id:"minecraft:gold_ingot",count:1}}'),
            {'op': 'tick', 'n': 40},
            cmd('/execute if items entity @e[tag=barter,limit=1] weapon.offhand minecraft:gold_ingot'),
            cmd('/execute if entity @e[type=minecraft:item]'),
            {'op': 'tick', 'n': 140},
            cmd('/execute if items entity @e[tag=barter,limit=1] weapon.offhand *'),
            cmd('/execute if items entity @e[tag=barter,limit=1] weapon.mainhand minecraft:gold_ingot'),
            cmd('/execute if entity @e[type=minecraft:item]'),
        ]
        if baby:
            ops.append(cmd('/execute if data entity @e[tag=barter,limit=1] Inventory[{id:"minecraft:golden_sword"}]'))
    # IGNORED_BY_PIGLIN_BABIES contains leather: adults take it, babies leave it.
    for baby in (False, True):
        ops += [
            *setup('/kill @e[type=!player]'),
            {'op': 'tick', 'n': 25},
            *setup('/kill @e[type=minecraft:item]',
                   '/summon minecraft:piglin 4.5 210 4.5 '
                   '{Tags:["barter"],IsImmuneToZombification:1b,IsBaby:' + ('1b' if baby else '0b') + '}',
                   '/summon minecraft:item 4.5 210 4.5 '
                   '{PickupDelay:0s,Item:{id:"minecraft:leather",count:1}}'),
            {'op': 'tick', 'n': 40},
            cmd('/execute if entity @e[type=minecraft:item]'),
            cmd('/execute if items entity @e[tag=barter,limit=1] weapon.mainhand minecraft:leather'),
        ]
    ops += [*setup('/kill @e[type=!player]'), {'op': 'tick', 'n': 25},
            *setup('/kill @e[type=minecraft:item]', '/fill 2 209 2 6 212 6 minecraft:air')]
    return ops


def experience_orb_stuck():
    """`ExperienceOrb.tick`'s push out of a block, the one path of the tick that
    draws a random: a 3x3 block placed over a settled orb leaves it colliding,
    and with no player within the 8 block follow radius the server nudges it
    towards the nearest free side (`Entity.moveTowardsClosestSpace`),
    `nextFloat() * 0.2 + 0.1` at a time, gravity skipped while its box is inside
    blocks. The orb's random is unseeded in vanilla (`RandomSource.create()`),
    so the outcome must not depend on the draws: a ring of stone one block
    higher keeps the top the only free side all the way out. Without it, once
    the orb's middle rose past y 216 with its box still in the block, north
    tied with south, west and east, won as the first, and some draws slid the
    orb off the block (at z 23.25, 22.x or down to y 215 on the floor).
    An orb that is never pushed stays boxed in at y 215 instead of resting in
    the ring at 216. 25 blocks from the player, out of the follow radius."""
    return [
        *setup('/fill 30 214 20 36 214 26 minecraft:stone'),
        cmd('/summon minecraft:experience_orb 33.5 215 23.5 {Value:1s}'),
        {'op': 'tick', 'n': 2},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        *setup('/fill 32 215 22 34 216 24 minecraft:stone', '/setblock 33 216 23 minecraft:air'),
        {'op': 'tick', 'n': 60},
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Pos'),
        cmd('/data get entity @e[type=minecraft:experience_orb,limit=1] Motion'),
        *setup('/kill @e[type=minecraft:experience_orb]', '/fill 30 214 20 36 216 26 minecraft:air'),
        {'op': 'tick', 'n': 5},
    ]


def groups():
    return [
        ('setup', group_setup()),
        ('server clock', server_clock()),
        ('client commands', client_commands()),
        ('game state', game_state()),
        ('player state', player_state()),
        ('blocks and border', world_blocks()),
        ('entities', entities()),
        ('wither spawn', wither_spawn()),
        ('riding teleport', riding_teleport()),
        ('mob riding', mob_riding()),
        ('camel riders', camel_riders()),
        ('scores, teams and bossbars', scores()),
        ('execute', execute()),
        ('levels', levels()),
        ('end portal entities', end_portal_entities()),
        ('items and loot', items_and_loot()),
        ('world', world_misc()),
        ('progression', progression()),
        ('functions and packs', functions_and_packs()),
        ('ticking and random', ticking()),
        ('compute', compute()),
        ('social and scoreboard', social()),
        ('test', test_command()),
        ('wither breaks', wither_breaks()),
        ('tnt minecart rails', tnt_minecart_rails()),
        ('creative shulker drops', creative_shulker_drops()),
        ('ender dragon', ender_dragon()),
        ('place structure', place_structure()),
        ('tame wolf kills', tame_wolf_kills()),
        ('experience orb fall', experience_orb_fall()),
        ('piglin pickups', piglin_pickups()),
        ('experience orb stuck', experience_orb_stuck()),
    ]
