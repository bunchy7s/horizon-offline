"""The test arrangement of tools/test_save_roundtrip_parity.py: one of every vanilla 26.3
block entity type and many entity types, placed by console commands in a flat world.

BLOCK_ENTITIES entries are (block id and states, block entity NBT in SNBT or '').
ENTITIES entries are (entity id, extra SNBT fields). Entities get deterministic UUIDs
and NoAI/Silent/NoGravity so that the two saves can be matched one to one.
"""

SWORD = ('{Slot:0b,id:"minecraft:iron_sword",count:1,components:{"minecraft:damage":7,'
         '"minecraft:custom_name":{text:"Blade",italic:0b},"minecraft:enchantments":{"minecraft:sharpness":3}}}')
STONE = '{Slot:5b,id:"minecraft:stone",count:64}'
APPLE = '{Slot:1b,id:"minecraft:apple",count:3}'
NAME = 'CustomName:{text:"Named",color:"red"}'
LOCKED = 'lock:{components:{"minecraft:custom_name":{text:"Key"}}}'
LOOT = 'LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:12345L'
SIGN_SIDE = ('{messages:[{text:"one",color:"red"},"two",{text:"three",bold:1b},""],color:"blue",'
             'has_glowing_text:1b}')
BOOK = ('{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":'
        '{title:{raw:"Title"},author:"Author",pages:[{raw:"page one"},{raw:"page two"}]}}}')

FULL = [
    ('chest', f'{{Items:[{SWORD},{STONE}],{NAME},{LOCKED}}}'),
    ('chest[facing=east]', f'{{{LOOT}}}'),
    ('trapped_chest', f'{{Items:[{SWORD},{APPLE}]}}'),
    ('ender_chest', ''),
    ('barrel[facing=up]', f'{{Items:[{STONE}],{NAME}}}'),
    ('barrel[facing=up]', f'{{{LOOT}}}'),
    ('dispenser[facing=up]', f'{{Items:[{APPLE}],{NAME}}}'),
    ('dropper[facing=up]', f'{{Items:[{STONE}]}}'),
    ('hopper', f'{{Items:[{STONE}],TransferCooldown:3,{NAME}}}'),
    ('red_shulker_box', f'{{Items:[{SWORD},{STONE}],{NAME}}}'),
    ('furnace', f'{{Items:[{{Slot:0b,id:"minecraft:iron_ore",count:5}},{{Slot:1b,id:"minecraft:coal",count:2}}],'
                'lit_time_remaining:100,lit_total_time:200,cooking_time_spent:50,cooking_total_time:200,'
                'speed_multiplier:1.5f,RecipesUsed:{"minecraft:iron_ingot":2}}'),
    ('blast_furnace', '{Items:[{Slot:2b,id:"minecraft:iron_ingot",count:9}],RecipesUsed:{"minecraft:copper_ingot":4}}'),
    ('smoker', f'{{{NAME},cooking_total_time:100}}'),
    ('brewing_stand', f'{{Items:[{{Slot:0b,id:"minecraft:potion",count:1}},{{Slot:3b,id:"minecraft:nether_wart",count:2}},'
                      '{Slot:4b,id:"minecraft:blaze_powder",count:1}],BrewTime:200,Fuel:10}'),
    # The beef must not finish cooking while the scene runs: its item would land
    # near the sculk sensors at a time that depends on how long each game ran.
    ('campfire', '{Items:[{Slot:0b,id:"minecraft:beef",count:1}],CookingTimes:[I;10,0,0,0],'
                 'CookingTotalTimes:[I;60000,0,0,0]}'),
    ('soul_campfire', ''),
    ('chiseled_bookshelf', '{Items:[{Slot:0b,id:"minecraft:book",count:1},{Slot:3b,id:"minecraft:book",count:1}],'
                           'last_interacted_slot:3}'),
    ('oak_shelf', f'{{Items:[{APPLE}],align_items_to_bottom:1b}}'),
    ('crafter', f'{{crafting_ticks_remaining:3,disabled_slots:[I;2,4],triggered:1,Items:[{APPLE}]}}'),
    ('decorated_pot', '{sherds:{back:"minecraft:brick",left:"minecraft:brick",right:"minecraft:angler_pottery_sherd",'
                      'front:{id:"minecraft:brick"}},item:{id:"minecraft:diamond",count:2}}'),
    ('suspicious_sand', '{item:{id:"minecraft:diamond",count:1}}'),
    ('suspicious_gravel', f'{{{LOOT}}}'),
    ('jukebox', '{RecordItem:{id:"minecraft:music_disc_13",count:1},ticks_since_song_started:5L}'),
    ('lectern', f'{{Book:{BOOK},Page:1}}'),
    ('beacon', f'{{Levels:0,primary_effect:"minecraft:speed",secondary_effect:"minecraft:regeneration",{NAME}}}'),
    ('bell', ''),
    ('white_banner', '{patterns:[{pattern:"minecraft:stripe_top",color:"red"},'
                     '{pattern:"minecraft:creeper",color:"black"}],CustomName:{text:"Flag"}}'),
    ('skeleton_skull', '{note_block_sound:"minecraft:block.note_block.imitate.skeleton",custom_name:{text:"Skully"}}'),
    ('creeper_head', ''),
    ('oak_sign', f'{{front_text:{SIGN_SIDE},back_text:{{messages:["a","b","c","d"],color:"black",has_glowing_text:0b}},is_waxed:1b}}'),
    ('oak_hanging_sign', f'{{front_text:{SIGN_SIDE}}}'),
    ('spawner', '{SpawnData:{entity:{id:"minecraft:pig"}},Delay:20s,MinSpawnDelay:200s,MaxSpawnDelay:800s,'
                'SpawnCount:4s,MaxNearbyEntities:6s,RequiredPlayerRange:16s,SpawnRange:4s}'),
    ('trial_spawner', '{required_player_range:10,target_cooldown_length:1200}'),
    ('vault', '{config:{loot_table:"minecraft:chests/trial_chambers/reward",key_item:{id:"minecraft:trial_key",count:1}},'
              'server_data:{rewarded_players:[[I;1,2,3,4]],total_ejections_needed:2}}'),
    ('command_block', f'{{Command:"say hi",auto:1b,TrackOutput:0b,{NAME}}}'),
    ('chain_command_block', '{Command:"say chain",conditionMet:1b}'),
    ('comparator', '{OutputSignal:5}'),
    ('conduit', ''),
    ('daylight_detector', ''),
    ('enchanting_table', f'{{{NAME}}}'),
    ('end_gateway', '{Age:20L,exit_portal:[I;0,70,0],ExactTeleport:1b}'),
    ('end_portal', ''),
    ('jigsaw', '{name:"minecraft:a",target:"minecraft:b",pool:"minecraft:empty",final_state:"minecraft:stone",'
               'joint:"aligned",placement_priority:1,selection_priority:2}'),
    ('structure_block', '{name:"test:s",author:"me",metadata:"meta",mode:"SAVE",posX:1,posY:2,posZ:3,sizeX:4,sizeY:5,'
                        'sizeZ:6,rotation:"CLOCKWISE_90",mirror:"LEFT_RIGHT",ignoreEntities:0b,integrity:0.5f,seed:9L}'),
    ('sculk_sensor', '{last_vibration_frequency:3}'),
    ('calibrated_sculk_sensor', ''),
    ('sculk_catalyst', ''),
    ('sculk_shrieker[can_summon=true]', '{warning_level:2}'),
    ('creaking_heart', ''),
    ('beehive', '{bees:[{entity_data:{id:"minecraft:bee"},ticks_in_hive:100,min_ticks_in_hive:2400}],flower_pos:[I;1,2,3]}'),
    ('bee_nest', ''),
    ('test_block', '{mode:"accept",message:"hello",powered:1b}'),
    ('test_instance_block', ''),
    ('copper_golem_statue', ''),
    ('potent_sulfur', '{countdown:7}'),
]

# The same blocks with no NBT: vanilla fills every field with its default, which
# Horizon must reproduce when it loads and saves them.
BLOCK_ENTITIES = FULL + [(block, '') for block, nbt in FULL if nbt]

# (entity id, extra SNBT). Each also gets UUID, Pos, NoAI, Silent, a name and a tag.
ENTITIES = [
    ('pig', ''), ('cow', 'Age:-100'), ('sheep', 'Color:5b,Sheared:1b'), ('chicken', 'EggLayTime:1000'),
    ('horse', 'Tame:1b,Variant:258,Temper:30'), ('wolf', 'Health:12f,CollarColor:3b'), ('cat', ''),
    ('villager', 'VillagerData:{profession:"minecraft:farmer",level:2,type:"minecraft:plains"},Xp:15,'
                 'Offers:{Recipes:[{buy:{id:"minecraft:wheat",count:20},sell:{id:"minecraft:emerald",count:1},'
                 'maxUses:16,uses:2,xp:2,priceMultiplier:0.05f,demand:0}]}'),
    ('zombie', 'IsBaby:1b,ArmorItems:[{},{},{},{id:"minecraft:iron_helmet",count:1}],HandItems:[{id:"minecraft:iron_sword",count:1},{}]'),
    ('skeleton', ''), ('creeper', 'powered:1b,Fuse:20s'), ('spider', ''), ('enderman', ''),
    ('bee', 'HasNectar:1b,Anger:0'), ('axolotl', 'Variant:2'), ('frog', ''), ('fox', 'Type:"snow"'),
    ('parrot', 'Variant:3'), ('llama', 'Strength:3,Variant:1'), ('rabbit', 'RabbitType:2'),
    ('allay', ''), ('iron_golem', 'PlayerCreated:1b'), ('snow_golem', 'Pumpkin:0b'),
    ('armor_stand', 'ShowArms:1b,Small:1b,Pose:{Head:[10f,20f,30f]},ArmorItems:[{},{},{},{id:"minecraft:diamond_helmet",count:1}]'),
    ('item', 'Item:{id:"minecraft:diamond",count:5},Age:100,PickupDelay:20s'),
    ('experience_orb', 'Value:7s,Age:30s'),
    ('item_frame', 'Facing:3b,Item:{id:"minecraft:apple",count:1},ItemRotation:2b,ItemDropChance:0.5f'),
    ('glow_item_frame', 'Facing:2b'),
    ('painting', 'Facing:3b,variant:"minecraft:kebab"'),
    ('arrow', 'inGround:1b,life:5s,damage:2.0d'),
    # Time starts far below 600, after which FallingBlockEntity drops itself as an item.
    ('falling_block', 'BlockState:{id:"minecraft:anvil",properties:{facing:"east"}},Time:-100000,HurtEntities:1b,FallHurtAmount:2f'),
    ('tnt', 'fuse:50s,block_state:{id:"minecraft:tnt",properties:{unstable:"true"}}'),
    ('oak_boat', ''), ('chest_boat', ''),
    ('minecart', ''), ('chest_minecart', 'Items:[{Slot:0b,id:"minecraft:apple",count:2}]'),
    ('hopper_minecart', ''), ('furnace_minecart', 'Fuel:100s'), ('tnt_minecart', 'TNTFuse:-1'),
    ('text_display', 'text:{text:"Hello"},billboard:"center",line_width:100,background:1234,alignment:"left"'),
    ('block_display', 'block_state:{id:"minecraft:oak_stairs",properties:{facing:"east"}},transformation:{translation:[1f,0f,0f],left_rotation:[0f,0f,0f,1f],right_rotation:[0f,0f,0f,1f],scale:[2f,2f,2f]}'),
    ('item_display', 'item:{id:"minecraft:stone",count:1},item_display:"head"'),
    ('interaction', 'width:2f,height:3f,response:1b'),
    ('marker', 'data:{a:1,b:"two"}'),
    ('area_effect_cloud', 'Radius:3f,Duration:200,Particle:{type:"minecraft:entity_effect",color:16711680},Potion:"minecraft:poison"'),
    ('end_crystal', 'ShowBottom:0b'),
    ('snowball', ''), ('ender_pearl', ''), ('small_fireball', ''), ('wither_skull', 'dangerous:1b'),
    ('glow_squid', ''), ('turtle', 'HomePosX:1,HomePosY:-60,HomePosZ:1,HasEgg:1b'),
    ('ghast', ''), ('piglin', ''), ('hoglin', ''), ('strider', 'Saddle:1b'), ('warden', ''),
    ('trader_llama', ''), ('mooshroom', 'Type:"brown"'), ('panda', 'MainGene:"lazy"'),
    ('copper_golem', ''), ('creaking', ''),
    # Riders (an entity saves inside its vehicle). The riders
    # have their own UUIDs, so a dropped or reordered one shows in the vehicle's Passengers diff.
    ('pig', 'Passengers:[{id:"minecraft:zombie",UUID:[I;28673,0,0,1],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b}]'),
    ('oak_boat', 'Passengers:[{id:"minecraft:pig",UUID:[I;28673,0,0,2],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b},{id:"minecraft:chicken",UUID:[I;28673,0,0,3],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b}]'),
    ('minecart', 'Passengers:[{id:"minecraft:cow",UUID:[I;28673,0,0,4],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b}]'),
    ('horse', 'Tame:1b,Passengers:[{id:"minecraft:skeleton",UUID:[I;28673,0,0,5],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b,Passengers:[{id:"minecraft:bat",UUID:[I;28673,0,0,6],NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b}]}]'),
]

# Leads. A leashed mob saves `leash` as [I;x,y,z] (a fence knot at that block) or
# {UUID:[I;..]} (a holder entity); vanilla restores the holder on the first tick after
# loading and drops the lead if the holder is not found, so the knot and the holder
# are summoned first, with fixed UUIDs, and within a few blocks of what they hold.
# SETBLOCKS run before the entities.
SETBLOCKS = ['setblock 40 -60 40 minecraft:oak_fence']
_NOAI = 'NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b'
_first = len(ENTITIES)
ENTITIES += [
    ('leash_knot', 'block_pos:[I;40,-60,40],Pos:[40.5d,-59.5d,40.5d]'),
    ('cow', 'Pos:[42.5d,-60.0d,40.5d],leash:[I;40,-60,40]'),
    ('pig', 'Pos:[44.5d,-60.0d,44.5d]'),
    ('oak_boat', f'Pos:[44.5d,-60.0d,42.5d],leash:{{UUID:[I;28672,0,0,{_first + 2}]}}'),
]
LEADS = 2  # the leashed mobs above that vanilla must still have leashed when it saves
