#!/usr/bin/env python3
"""Compare arrows (AbstractArrow.tick) over the native client's resident scene.

Real vanilla `Arrow` entities tick in an independently built copy of the
scene (`VanillaArrowOracle`, client and server jars); Horizon's
`arrows::tick` runs over the scene itself (`examples/probe/arrow_probe.rs`). Every
operation prints the arrow's position, motion, rotation, ground state, life,
shake, stuck block, fire ticks, water flags, fluid heights, removal, crit flag,
fall distance and old position (what rendering interpolates from) by exact
bits, then its sounds, particles and game events in order, then its pickup
rule, whether its item is intangible and what a player touching it took.
Cases: flights in the air with and without crits or physics, into and through
water (inertia, bubbles, splashes, currents of flowing water, eyes),
kelp, seagrass and waterlogged blocks, bubble columns with and without drag
and with air or water above, sticking in full and partial blocks, falling out
when the block goes, starting inside a block, and despawning after 1200 ticks;
burning arrows: lava and fire blocks lighting them, water putting them out,
Flame (ignited at spawn), and TNT, campfires, candles and candle cakes hit by
burning and non-burning arrows; pickup: arrows from no one, a player or a mob,
from intangible ammo (creative, Infinity, multishot extras) or a dispenser,
touched in flight, while shaking and when stuck, by survival and creative
players with empty, partly and fully filled inventories, and without physics.
Entities: a player-sized box in the way that takes the arrow's damage (the
arrow is spent with its hit sound) or refuses it (a raised shield: the arrow
turns back, and drops its pickup when left almost still), struck squarely or
grazed within ProjectileUtil's margin, crits drawing from the arrow's random.
No rain.
"""
import json
import math
import os
from pathlib import Path
import platform
import random
import struct
import subprocess
try:
    from . import lab, test_rust_parity as base, test_native_client as client
except ImportError:
    import lab
    import test_rust_parity as base
    import test_native_client as client


def bits(value): return struct.pack('>d', value).hex()
def vec(values): return list(map(bits, values))
def op(mode, value=None, **extra): return dict(mode=mode, value=value, **extra)


def corpus():
    rng = random.Random(0x4152524f57)
    def place(name, positions, properties=None):
        return [op('block', name='minecraft:' + name, position=list(p), **({'properties': properties} if properties else {})) for p in positions]
    def restore(positions):
        return [op('block', name='minecraft:air', position=list(p)) for p in positions]
    def arrow(position, velocity, ops, crit=False, no_physics=False, intangible=False):
        return dict(seed=str(rng.randrange(-1 << 63, 1 << 63)), position=vec(position), velocity=vec(velocity),
                    crit=crit, no_physics=no_physics, intangible=intangible, ops=[op('level_seed', str(rng.randrange(1 << 48)))] + ops)
    ticks = lambda n: [op('tick') for _ in range(n)]
    def aim(speed, yaw, pitch):
        import math
        y, p = math.radians(yaw), math.radians(pitch)
        return (-math.sin(y) * math.cos(p) * speed, -math.sin(p) * speed, math.cos(y) * math.cos(p) * speed)
    # Flights in the air onto the floor (y 4), sticking, then resting.
    for _ in range(40):
        start = (rng.uniform(17, 30), rng.uniform(5, 14), rng.uniform(2, 30))
        v = aim(rng.choice([.3, 1, 1.6, 2.4, 3]), rng.uniform(-180, 180), rng.uniform(-60, 45))
        yield arrow(start, v, ticks(60), crit=rng.random() < .5)
    # Without physics: straight through blocks, no gravity, inverted yaw.
    for _ in range(6):
        start = (rng.uniform(17, 30), rng.uniform(5, 8), rng.uniform(2, 30))
        yield arrow(start, aim(1.5, rng.uniform(-180, 180), rng.uniform(-30, 30)), ticks(25), no_physics=True)
    # Water: a pool three deep, shot into from above, through it and from inside.
    pool = [(x, y, z) for x in range(20, 29) for y in range(4, 7) for z in range(20, 29)]
    for level in ['0', '0', '0', '2', '6', '8']:
        for _ in range(8):
            start = rng.choice([(rng.uniform(20, 29), rng.uniform(8, 14), rng.uniform(16, 19)),
                                (rng.uniform(21, 28), rng.uniform(4.2, 6.5), rng.uniform(21, 28))])
            v = aim(rng.choice([.05, .4, 1.2, 2.5, 3]), rng.uniform(-60, 60), rng.uniform(-50, 20))
            ops = place('water', pool, {'level': level}) + ticks(70) + restore(pool)
            yield arrow(start, v, ops, crit=rng.random() < .3)
    # Currents: random flowing water around a slow arrow.
    for _ in range(16):
        cells = [(x, 4, z) for x in range(21, 28) for z in range(21, 28)]
        ops = [op('block', name='minecraft:water', position=list(c), properties={'level': rng.choice(['0', '1', '3', '5', '7', '9'])}) for c in cells]
        ops += ticks(40) + restore(cells)
        yield arrow((rng.uniform(21.3, 27.7), rng.uniform(4.05, 4.6), rng.uniform(21.3, 27.7)), aim(rng.choice([0, .01, .2]), rng.uniform(-180, 180), 0), ops)
    # Kelp, seagrass and waterlogged blocks are water too.
    for name, properties in [('kelp', {'age': '5'}), ('kelp_plant', {}), ('seagrass', {}), ('oak_slab', {'type': 'bottom', 'waterlogged': 'true'}),
                             ('oak_fence', {'waterlogged': 'true'}), ('iron_bars', {'waterlogged': 'true'})]:
        blocks = [(x, y, z) for x in range(22, 27) for y in range(4, 6) for z in range(22, 27)]
        for _ in range(4):
            v = aim(rng.choice([.6, 1.5, 2.8]), rng.uniform(-40, 40), rng.uniform(5, 60))
            ops = place(name, blocks, properties) + ticks(50) + restore(blocks)
            yield arrow((rng.uniform(22, 27), rng.uniform(7, 10), rng.uniform(18, 20)), v, ops)
    # Bubble columns: up or down, open above or under water.
    for drag in ['true', 'false']:
        for roof in [None, 'water', 'stone']:
            column = [(24, y, 24) for y in range(4, 9)]
            ops = place('bubble_column', column, {'drag': drag})
            if roof:
                ops += place(roof, [(24, 9, 24)])
            for start, v in [((24.5, 12, 24.5), (0, -.3, 0)), ((24.5, 4.2, 24.5), (0, .4, 0)), ((24.2, 6, 24.7), (.02, 0, -.01)), ((22, 9, 24.5), (.5, -.2, 0))]:
                yield arrow(start, v, ops + ticks(60) + restore(column + [(24, 9, 24)]))
    # Sticking in partial blocks, then the block goes (or changes) under it.
    shapes = [('stone', {}), ('oak_slab', {'type': 'top'}), ('oak_slab', {'type': 'bottom'}), ('oak_stairs', {'facing': 'east'}),
              ('oak_fence', {}), ('glass_pane', {'north': 'true', 'south': 'true'}), ('oak_trapdoor', {'half': 'top'}),
              ('lantern', {}), ('iron_chain', {'axis': 'y'}), ('white_carpet', {}), ('snow', {'layers': '3'}), ('anvil', {'facing': 'north'})]
    for name, properties in shapes:
        for after in ['air', 'glass', None]:
            target = [(25, 9, 25)]
            v = rng.choice([(0, -1.2, 0), (.9, .1, 0), (0, .1, -.9), (.7, -.7, .2)])
            start = (25.5 - v[0] * 3, 9.5 - v[1] * 3, 25.5 - v[2] * 3)
            ops = place(name, target, properties) + ticks(20)
            if after:
                ops += place(after, target)
            ops += ticks(40) + restore(target)
            yield arrow(start, v, ops, crit=True)
    # Starting inside a block: stopped in place.
    for name in ['stone', 'oak_slab', 'oak_fence']:
        yield arrow((25.5, 9.25, 25.5), (.4, 0, .1), place(name, [(25, 9, 25)]) + ticks(10) + restore([(25, 9, 25)]))
    # Fire: lava pools and fire blocks light arrows (LAVA_IGNITE 15 s, fireIgnite
    # 8 s after counting up from below zero), water puts them out with the
    # extinguish sound, burning arrows keep their fire while stuck.
    def ignite(seconds):
        return op('ignite', struct.pack('>f', seconds).hex())
    lava = [(x, y, z) for x in range(20, 27) for y in range(4, 6) for z in range(20, 27)]
    for level in ['0', '0', '4']:
        for _ in range(6):
            v = aim(rng.choice([.4, 1.3, 2.6]), rng.uniform(-50, 50), rng.uniform(0, 50))
            ops = place('lava', lava, {'level': level}) + ticks(60) + restore(lava)
            yield arrow((rng.uniform(20, 27), rng.uniform(7, 11), rng.uniform(16, 19)), v, ops)
    for _ in range(6):
        # Through lava, then into water beyond it (and the other way round).
        lava_wall = [(23, y, z) for y in range(4, 9) for z in range(20, 27)]
        water_wall = [(26, y, z) for y in range(4, 9) for z in range(20, 27)]
        first, second = rng.choice([('lava', 'water'), ('water', 'lava')])
        ops = place(first, lava_wall) + place(second, water_wall) + ticks(40) + restore(lava_wall + water_wall)
        yield arrow((20.5, 6.5, rng.uniform(21, 26)), aim(rng.choice([.8, 2.0, 3.0]), -90, rng.uniform(-5, 5)), ops)
    for name in ['fire', 'soul_fire']:
        flames = [(x, 4, z) for x in range(22, 27) for z in range(22, 27)]
        base = [] if name == 'fire' else place('soul_sand', [(x, 3, z) for x in range(22, 27) for z in range(22, 27)])
        for _ in range(5):
            ops = base + place(name, flames) + ticks(50) + restore(flames)
            if name == 'soul_fire':
                ops += place('stone', [(x, 3, z) for x in range(22, 27) for z in range(22, 27)])
            v = aim(rng.choice([.3, 1.0, 2.0]), rng.uniform(-40, 40), rng.uniform(10, 60))
            yield arrow((rng.uniform(22, 27), rng.uniform(6, 9), rng.uniform(19, 21)), v, ops)
    # Flame arrows (100 s): flying, into water, stuck.
    pool = [(x, y, z) for x in range(20, 29) for y in range(4, 7) for z in range(20, 29)]
    for target in [None, 'water']:
        for _ in range(5):
            ops = [ignite(100.0)] + (place('water', pool) if target else []) + ticks(70) + (restore(pool) if target else [])
            v = aim(rng.choice([.5, 1.5, 3.0]), rng.uniform(-60, 60), rng.uniform(-20, 40))
            yield arrow((rng.uniform(20, 28), rng.uniform(7, 12), rng.uniform(16, 19)), v, ops, crit=rng.random() < .5)
    # Burning (and not burning) arrows hitting the blocks a burning projectile lights.
    lights = [('tnt', {}), ('campfire', {'lit': 'false'}), ('campfire', {'lit': 'true'}), ('campfire', {'lit': 'false', 'waterlogged': 'true'}),
              ('soul_campfire', {'lit': 'false'}), ('candle', {'candles': '3', 'lit': 'false'}), ('red_candle', {'lit': 'false', 'waterlogged': 'true'}),
              ('candle', {'lit': 'true'}), ('candle_cake', {'lit': 'false'}), ('blue_candle_cake', {'lit': 'true'}), ('stone', {})]
    for name, properties in lights:
        for burning in [False, True]:
            target = [(25, 9, 25)]
            v = rng.choice([(0, -1.2, 0), (.9, -.2, 0), (0, -.3, -.9)])
            start = (25.5 - v[0] * 3, 9.3 - v[1] * 3, 25.5 - v[2] * 3)
            ops = ([ignite(rng.choice([1.0, 8.5, 100.0]))] if burning else []) + place(name, target, properties) + ticks(30) + restore(target)
            yield arrow(start, v, ops)
    # Pickup: who shot it (setOwner), intangible ammo, dispensers' ALLOWED,
    # then touches in flight, while shaking and once stuck.
    def touch(creative, inventory):
        return op('touch', creative=creative, inventory=inventory)
    sources = [(None, False, None), ('player', False, None), ('mob', False, None), ('player', True, None),
               (None, True, None), (None, False, 1), ('mob', False, 2), ('player', False, 2), ('none', False, 0)]
    for owner, intangible, pickup in sources:
        for creative in [False, True]:
            for inventory in ['empty', 'partial', 'full']:
                start = (rng.uniform(18, 30), rng.uniform(5.5, 7), rng.uniform(4, 30))
                v = aim(rng.choice([.3, .8, 1.5]), rng.uniform(-180, 180), rng.uniform(30, 89))
                ops = ([op('owner', owner)] if owner else []) + ([op('pickup', pickup)] if pickup is not None else [])
                ops += [touch(creative, inventory)] + ticks(1)
                # Until it sticks, touching every tick (shake 7 counts down).
                ops += [o for _ in range(14) for o in (op('tick'), touch(creative, inventory))]
                ops += ticks(3) + [touch(creative, inventory), touch(not creative, inventory)] + ticks(2)
                yield arrow(start, v, ops, crit=rng.random() < .3, intangible=intangible)
    # Without physics: touched in flight.
    for owner in [None, 'player']:
        for creative in [False, True]:
            ops = ([op('owner', owner)] if owner else []) + [o for _ in range(4) for o in (op('tick'), touch(creative, 'empty'))]
            yield arrow((rng.uniform(18, 30), 6, rng.uniform(4, 30)), aim(.4, rng.uniform(-180, 180), 0), ops, no_physics=True)
    # An entity in the way (a player-sized box on the floor) that takes or
    # refuses the damage: squarely, grazing within the margin (older arrows),
    # missing, and slow enough to drop on the spot.
    def target(x, z, takes):
        return op('target', position=vec((x, 4, z)), takes=takes)
    for takes in [True, False]:
        for _ in range(24):
            tx, tz = rng.uniform(21, 27), rng.uniform(21, 27)
            yaw, distance = rng.uniform(-180, 180), rng.uniform(1.5, 9)
            start = (tx + math.sin(math.radians(yaw)) * distance, rng.uniform(4.2, 7.5), tz - math.cos(math.radians(yaw)) * distance)
            aim_at = (tx + rng.uniform(-.75, .75), rng.uniform(4, 6.2), tz + rng.uniform(-.75, .75))
            d = [aim_at[i] - start[i] for i in range(3)]
            speed = rng.choice([.4, 1.1, 2.2, 3]) / math.sqrt(sum(c * c for c in d))
            yield arrow(start, [c * speed for c in d], [target(tx, tz, takes)] + ticks(45), crit=rng.random() < .5)
        for pickup in [0, 1]:
            for speed in [.002, .03, .2]:
                # Struck in the first tick; the slowest, turned back at a
                # tenth of its speed, is left almost still and stops.
                ops = [op('pickup', pickup), target(25.5, 25.5, takes)] + ticks(6)
                yield arrow((25.199, 4.5, 25.5), (speed, 0, 0), ops)
    # Despawning 1200 ticks after sticking in the floor.
    for _ in range(2):
        yield arrow((rng.uniform(18, 30), 6, rng.uniform(4, 30)), (0, -1, 0), ticks(1215))


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'rust-arrow-parity'
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    resource_hash = client.resource_fingerprint(resources)
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaArrowOracle.java'
    helpers = [oracle.with_name(name + '.java') for name in ['VanillaSceneCollisionOracle', 'VanillaRegistryCollisionScanOracle',
        'VanillaHeightmapOracle', 'VanillaCollisionOracle', 'VanillaBlockStateCodecOracle', 'VanillaSuffocationOracle',
        'VanillaFaceSupportOracle', 'VanillaEntityAttributeOracle', 'VanillaBlockLandingOracle', 'VanillaMovementRegistryOracle', 'VanillaDamageSourceOracle']]
    sources = [Path(__file__), lab.ROOT / 'tools/test_native_client.py', lab.ROOT / 'tools/test_rust_parity.py', lab.ROOT / 'tools/lab.py',
               oracle, *helpers, lab.ROOT / 'Cargo.toml', lab.ROOT / 'Cargo.lock', lab.ROOT / 'crates/horizon-client/examples/probe/arrow_probe.rs']
    for crate in ['horizon-core', 'horizon-world', 'horizon-assets', 'horizon-client', 'horizon-client-render']:
        sources += [lab.ROOT / 'crates' / crate / 'Cargo.toml', *sorted((lab.ROOT / 'crates' / crate / 'src').rglob('*.rs'))]
    tested = base.source_hashes(sources)
    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            subprocess.run(list(map(str, command)), cwd=lab.ROOT, check=True, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, libraries))
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-cp', str(jars.get('server', jars['client'])) + os.pathsep + dependencies,
         '-d', classes, oracle, *helpers], 'compile-java')
    fixtures = list(corpus())
    operations = sum(len(f['ops']) for f in fixtures)
    expected = 3 * operations + len(fixtures)
    (build / 'inputs.txt').write_text(''.join(json.dumps(f, separators=(',', ':')) + '\n' for f in fixtures))
    input_hash = lab.digest(build / 'inputs.txt', 'sha256')
    run(['cargo', 'build', '--locked', '-p', 'horizon-client', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=lab.ROOT))
    binary = Path(metadata['target_directory']) / 'debug/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary, 'arrow_probe',resources, build, '--describe'], 'describe')
    references = []
    for side, jar in jars.items():
        run(['java', '-Xmx768M', '-cp', os.pathsep.join([str(classes), str(jar), dependencies]),
             'dev.horizonlab.parity.VanillaArrowOracle', side, build, resources], side)
        run([binary, 'arrow_probe',resources, build, side], 'rust-' + side)
        matched = base.compare(build / (side + '.txt'), build / ('rust-' + side + '.txt'))
        if matched != expected:
            raise AssertionError('Incomplete arrow output')
        references.append({'side': side, 'jar_sha256': lab.digest(jar, 'sha256'), 'output_sha256': lab.digest(build / (side + '.txt'), 'sha256'),
                           'matched_output_lines': matched, 'rust_output_sha256': lab.digest(build / ('rust-' + side + '.txt'), 'sha256')})
        print(f'{side}: {matched:,} arrow states match', flush=True)
    base.ensure_sources_unchanged(sources, tested)
    if resource_hash != client.resource_fingerprint(resources) or input_hash != lab.digest(build / 'inputs.txt', 'sha256'):
        raise AssertionError('Arrow inputs changed during verification')
    lab.write_json(report, {'schema': 2, 'minecraft': lock['minecraft']['version'], 'manifest_sha1': lock['minecraft']['manifest_sha1'],
        'platform': platform.platform(), 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'java': subprocess.check_output(['java', '--version'], text=True).splitlines()[0], 'fixtures': len(fixtures),
        'operations': operations, 'sources_sha256': tested, 'resources': resource_hash, 'input_sha256': input_hash,
        'references': references, 'scope': __doc__})
    print(f'Arrow parity passed: {report}', flush=True)


if __name__ == '__main__':
    main()
