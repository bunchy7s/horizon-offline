#!/usr/bin/env python3
"""Entities other than mobs and items, tracked to a vanilla client, against vanilla's dedicated server.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat
world vanilla generated first, with the same server.properties (offline, view distance 4). A vanilla-codec
client (VanillaProtocolClient in its entity mode) joins, and once its chunks are in it asks for the scene: the
console freezes the ticks, builds a wall and a rail and summons, around the spawn, every kind of minecart (one
set rolling along the rail), boats and a raft (one falling onto the ground), armor stands (posed, small, with
arms, wearing a helmet, one falling), a chicken jockey, a slime, a pig whose scale `/attribute` sets, item
frames (one holding a turned diamond), paintings, primed TNT (one falling), a floating falling block, sand
falling into a cobweb, sand that falls onto a torch and breaks into an item (added after the tick's tracking pass in
Horizon), an arrow that lands, a burning arrow and an end crystal. The client records for 20 seconds, the ticks
running for 200 more after 3, what ClientPacketListener would make of the packets (EntityTranscript): each
entity's spawn
packet (position, rotations, data, motion), the entity data it was paired with and all it was sent since, its
equipment, its attribute instances (ClientboundUpdateAttributesPacket: the ones its first tick creates, the
type's set by finalizeSpawn, the baby speed modifier, effects', equipment's and /attribute's) and passengers, and
where its position syncs leave it; and the attribute instances the player itself is sent from its login on. Both
transcripts must match exactly, entity by entity (named by type and the block it was added in), once the amounts
of vanilla's random spawn modifiers are normalised (only client-syncable attributes are sent, so follow range and
knockback resistance never show).
"""
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server
    from .test_locator_bar_parity import generate_world, server_dir
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server
    from test_locator_bar_parity import generate_world, server_dir

NAME = 'server-entities-parity'
# Ticks the scene runs for once it is summoned: 10 seconds, within the client's 20.
STEPS = 200
PLAYER = 'Tester'

# Around the spawn at 0 -60 0 on the flat world's grass: a wall to the north for the frames and paintings,
# a rail to the south.
SCENE = [
    'fill -7 -60 -6 7 -57 -6 minecraft:stone',
    'fill -7 -60 9 7 -60 9 minecraft:rail',
    # Minecarts.
    'summon minecraft:minecart -5.5 -60 3.5',
    'summon minecraft:chest_minecart -3.5 -60 3.5',
    'summon minecraft:furnace_minecart -1.5 -60 3.5',
    'summon minecraft:hopper_minecart 1.5 -60 3.5',
    'summon minecraft:tnt_minecart 3.5 -60 3.5',
    'summon minecraft:command_block_minecart 5.5 -60 1.5 {Command:"say hi"}',
    'summon minecraft:minecart -6.5 -60 9.5 {Motion:[0.5d,0d,0d]}',
    # Boats.
    'summon minecraft:oak_boat -5.5 -60 6.5 {Rotation:[45f,0f]}',
    'summon minecraft:birch_chest_boat -2.5 -60 6.5',
    'summon minecraft:bamboo_raft 0.5 -60 6.5',
    'summon minecraft:oak_boat 4.5 -56 6.5',
    # Armor stands.
    'summon minecraft:armor_stand -2.5 -60 -2.5 {Rotation:[90f,0f]}',
    'summon minecraft:armor_stand 6.5 -57 -2.5',
    'summon minecraft:armor_stand -4.5 -60 -2.5 {ShowArms:1b,Small:1b,NoBasePlate:1b,'
    'Pose:{Head:[10f,20f,0f],RightArm:[-30f,0f,15f]},equipment:{head:{id:"minecraft:diamond_helmet",count:1}}}',
    # A chicken jockey (passengers).
    'summon minecraft:chicken -6.5 -60 -2.5 {NoAI:1b,Passengers:[{id:"minecraft:husk",IsBaby:1b,NoAI:1b}]}',
    # Attributes: a slime's size sets its bases, /attribute a pig's scale.
    'summon minecraft:slime 2.5 -60 1.5 {Size:1,NoAI:1b}',
    'summon minecraft:pig -0.5 -60 1.5 {NoAI:1b}',
    'attribute @e[type=minecraft:pig,limit=1,x=-0.5,y=-60,z=1.5,distance=..1] minecraft:scale base set 1.5',
    # Frames and paintings on the wall's south face.
    'summon minecraft:item_frame -3.5 -59 -4.97 {block_pos:[I;-4,-59,-5],Facing:3b,'
    'Item:{id:"minecraft:diamond",count:1},ItemRotation:3b}',
    'summon minecraft:glow_item_frame -1.5 -59 -4.97 {block_pos:[I;-2,-59,-5],Facing:3b}',
    'summon minecraft:painting 1.5 -59 -4.97 {block_pos:[I;1,-59,-5],facing:0b,variant:"minecraft:kebab"}',
    'summon minecraft:painting 4 -59 -4.97 {block_pos:[I;4,-59,-5],facing:0b,variant:"minecraft:courbet"}',
    # Primed TNT that won't go off (one falling), a floating falling block, an arrow that lands, an end crystal.
    'summon minecraft:tnt 6.5 -60 6.5 {fuse:2000s}',
    'summon minecraft:tnt 6.5 -56 4.5 {fuse:2000s}',
    # Passengers of primed TNT (EntityType.loadEntityRecursive).
    'summon minecraft:tnt 4.5 -60 4.5 {fuse:2000s,Passengers:[{id:"minecraft:pig",NoAI:1b}]}',
    'summon minecraft:falling_block 2.5 -58 -0.5 {BlockState:{Name:"minecraft:sand"},NoGravity:1b}',
    # Sand dropped into a cobweb, which holds it (WebBlock.entityInside).
    'setblock -7 -58 0 minecraft:cobweb',
    'summon minecraft:falling_block -6.5 -54 0.5 {BlockState:{Name:"minecraft:sand"}}',
    # Sand that falls once the ticks run (its scheduled tick), lands on a torch and breaks into an item
    # (FallingBlockEntity.tick: spawnAtLocation), added in the middle of a tick.
    'setblock 6 -60 -4 minecraft:torch',
    'setblock 6 -55 -4 minecraft:sand',
    'summon minecraft:arrow 0.5 -55 -1.5 {Motion:[0.3d,0d,0d]}',
    # A burning arrow (a Flame bow's): its shared flag of fire (Entity.DATA_SHARED_FLAGS_ID) reaches the client.
    'summon minecraft:arrow 0.5 -55 2.5 {Motion:[0.3d,0d,0d],Fire:2000s}',
    'summon minecraft:end_crystal -6.5 -60 6.5 {ShowBottom:0b}',
]


# finalizeSpawn's random modifiers on client-syncable attributes (a leader zombie's max health); their amounts
# differ from one server to the other.
RANDOM_MODIFIERS = re.compile(r'(id=minecraft:(?:random_spawn_bonus|leader_zombie_bonus), amount=)[^,]+')
# An item's yaw and its sideways motion come from the level's random (ItemEntity's constructor), and so do where
# it slides to and the motion last synced as it comes to rest (vanilla's own runs differ there); where it was added,
# its upward motion then and the height it rests at don't.
ITEM_ROTATION = re.compile(r' rot \S+')
ITEM_SIDEWAYS = re.compile(r'(motion|now) \([^,]+, ([^,]+), [^)]+\)')
ITEM_LAST_MOTION = re.compile(r'(\| now .* motion )\([^)]+\)')


def scene_lines(lines):
    """The transcript's entities in the scene's area (the world's own animals and slimes wander elsewhere), the
    amounts of random spawn modifiers and an item's random yaw and sideways motion normalised."""
    out = []
    for line in lines:
        m = re.match(r'play-entities entity \S+ (-?\d+),-?\d+,(-?\d+)', line)
        if m and -8 <= int(m.group(1)) <= 8 and -7 <= int(m.group(2)) <= 10 or line.startswith('play-entities self '):
            line = RANDOM_MODIFIERS.sub(r'\1~', line)
            if line.startswith('play-entities entity minecraft:item '):
                line = ITEM_SIDEWAYS.sub(r'\1 (~, \2, ~)', ITEM_ROTATION.sub(' rot ~', line))
                line = ITEM_LAST_MOTION.sub(r'\1~', line)
            out.append(line)
    return out


def run_side(label, server_command, client_command, build, world):
    directory = build / label
    port = server_dir(directory, world)
    out = build / f'{label}-client.txt'
    out.unlink(missing_ok=True)
    server = Server(server_command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = subprocess.Popen([*map(str, client_command), '127.0.0.1', str(port), PLAYER, str(out), 'entities'],
                                      cwd=lab.ROOT, stdout=log, stderr=subprocess.STDOUT, text=True)
            try:
                server.wait_for(rf'<{PLAYER}> summon the entities', timeout=180)
                # Frozen, so that every entity is paired as it was summoned whenever each server tracks it in
                # its tick; then they fall and roll.
                server.console('tick freeze')
                for command in SCENE:
                    server.console(command)
                time.sleep(3)
                # A fixed number of ticks, so that what still changes (a TNT fuse) ends the same.
                server.console(f'tick step {STEPS}')
                client.wait(timeout=120)
            finally:
                if client.poll() is None:
                    client.kill()
    finally:
        server.stop()
    if not out.exists():
        raise AssertionError(f'{label}: the client wrote no transcript (see {build}/{label}-client.log)')
    lines = out.read_text(encoding='utf-8').splitlines()
    (build / f'{label}.txt').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    return lines


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaProtocolClient.java'
    classes = build / 'classes'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir()
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-nowarn', '-sourcepath',
                    lab.ROOT / 'mods/horizon/src/test/java', '-cp', dependencies, '-d', classes, oracle],
                   cwd=lab.ROOT, check=True, capture_output=True)
    # SERVER_ENTITIES_SIDES=vanilla (or horizon) runs one side and reads the other's last transcript.
    sides = [s for s in os.environ.get('SERVER_ENTITIES_SIDES', 'vanilla,horizon').split(',') if s]
    if 'horizon' in sides:
        subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                       check=True, capture_output=True)
    import json
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    client = ['java', '-Xmx1g', '-Duser.timezone=UTC', '-cp', str(classes) + os.pathsep + dependencies,
              'dev.horizonlab.parity.VanillaProtocolClient']

    world = generate_world(vanilla_server, build)
    results = {}
    for label, command in [('vanilla', vanilla_server), ('horizon', horizon_server)]:
        if label in sides:
            results[label] = run_side(label, command, client, build, world)
        else:
            results[label] = (build / f'{label}.txt').read_text(encoding='utf-8').splitlines()
    vanilla, horizon = scene_lines(results['vanilla']), scene_lines(results['horizon'])
    errors = [l for l in results['vanilla'] + results['horizon'] if l.startswith('error')]
    if errors or vanilla != horizon:
        import difflib
        diff = '\n'.join(difflib.unified_diff(vanilla, horizon, 'vanilla', 'horizon', n=0, lineterm=''))
        raise AssertionError(f'{len(errors)} errors, transcripts differ (see {build}/vanilla.txt and horizon.txt):\n'
                             + '\n'.join(errors[:10]) + '\n' + diff[:8000])
    if len(vanilla) < len(SCENE) - 2:
        raise AssertionError(f'only {len(vanilla)} entities tracked: the scene did not run (see {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        entities=len(vanilla), output_sha256=lab.digest(build / 'horizon.txt', 'sha256'),
        scope='Minecarts (one rolling), boats (one falling), armor stands, a chicken jockey, a slime, a pig with a '
              'scale, item frames, paintings, primed TNT, falling blocks (one held by a cobweb, one breaking into an '
              'item on a torch), arrows (one burning) and an end crystal '
              'tracked to a vanilla-codec client by a dedicated server: spawn packets, entity data, equipment, '
              'attributes, passengers and the positions the syncs leave them at.'))
    print(f'server entities: {len(vanilla)} entities identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
