#!/usr/bin/env python3
"""Compare auto-jump with the pinned client: the real LocalPlayer.updateAutoJump
(and canAutoJump) on a player standing in an independently built copy of the
client's resident test scene, against MotionWorld::update_auto_jump over the
client's actual resources, collision registry and block-collision scan, for
steps, slabs, stairs, fences, walls, snow, carpets, honey, low ceilings and
overhangs approached from every angle with varied speeds, input vectors,
sizes, jump boost and the canAutoJump preconditions."""
import json
import math
from pathlib import Path
import platform
import random
import subprocess

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking, \
        test_native_client as client
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking
    import test_native_client as client


BLOCKS = [
    ('stone', {}), ('stone', {}), ('stone', {}),
    ('stone_slab', {'type': 'bottom'}), ('stone_slab', {'type': 'top'}), ('stone_slab', {'type': 'double'}),
    ('oak_stairs', {'facing': 'north', 'half': 'bottom'}), ('oak_stairs', {'facing': 'east', 'half': 'bottom'}),
    ('oak_stairs', {'facing': 'south', 'half': 'top'}), ('oak_stairs', {'facing': 'west', 'half': 'bottom', 'shape': 'outer_left'}),
    ('oak_fence', {}), ('oak_fence', {'north': 'true', 'south': 'true'}), ('oak_fence_gate', {}),
    ('cobblestone_wall', {}), ('cobblestone_wall', {'up': 'true', 'east': 'tall'}),
    ('snow', {'layers': '1'}), ('snow', {'layers': '4'}), ('snow', {'layers': '8'}),
    ('white_carpet', {}), ('honey_block', {}), ('soul_sand', {}), ('farmland', {}), ('dirt_path', {}),
    ('chest', {}), ('enchanting_table', {}), ('oak_trapdoor', {}), ('oak_trapdoor', {'half': 'top'}),
    ('oak_trapdoor', {'open': 'true', 'facing': 'east'}), ('daylight_detector', {}), ('glass', {}), ('iron_bars', {}),
    ('lantern', {}), ('cake', {}), ('anvil', {}), ('hopper', {}), ('oak_leaves', {}), ('cobweb', {}), ('ladder', {}),
    ('red_bed', {'part': 'foot'}),
]

ORIGIN = (20, 4, -4)


def placed(rng, count):
    blocks = {}
    ox, oy, oz = ORIGIN
    for _ in range(count):
        x, y, z = ox + rng.randint(-3, 3), oy + rng.choice([0, 0, 0, 1, 1, 2, 3]), oz + rng.randint(-3, 3)
        name, props = rng.choice(BLOCKS)
        blocks[(x, y, z)] = [x, y, z, 'minecraft:' + name, props]
    return list(blocks.values())


DIMS = [(0.6, 1.8), (0.6, 1.8), (0.6, 1.5), (0.6, 0.6), (0.3, 0.9), (1.2, 3.6), (0.6000000238418579, 1.7999999523162842)]


def player(rng, x, y, z, yaw):
    w, h = rng.choice(DIMS)
    return dict(pos=[lab.dbits(x), lab.dbits(y), lab.dbits(z)], ground=rng.random() < 0.9, shift=rng.random() < 0.08,
                yr=lab.fbits(yaw), xr=lab.fbits(rng.choice([0.0, 30.0, -45.0, 90.0, rng.uniform(-90, 90)])),
                speed=lab.fbits(rng.choice([0.1, 0.13000001, 0.03, 0.0, 0.2, rng.uniform(0, 0.3)])),
                move=[lab.fbits(v) for v in rng.choice([(0.0, 1.0), (0.0, 1.0), (1.0, 0.0), (-1.0, 0.0), (0.70710677, 0.70710677),
                                                     (0.0, -1.0), (0.0, 0.0), (0.3, 0.3)])],
                width=lab.fbits(w), height=lab.fbits(h), boost=rng.choice([None] * 6 + [0, 1, 4, 255]),
                enabled=rng.random() < 0.93, time=rng.choice([0] * 12 + [1, -3, 2]),
                passenger=rng.random() < 0.2)


def cases(rng):
    ox, oy, oz = ORIGIN
    # Walk straight at each block from the four sides, at several speeds.
    for name, props in BLOCKS:
        for yaw, (dx, dz) in [(0.0, (0, 1)), (90.0, (-1, 0)), (180.0, (0, -1)), (-90.0, (1, 0)), (45.0, (-0.7, 0.7))]:
            for step in (0.1, 0.215, 0.03, 0.0):
                for lift in (0, 1):
                    blocks = [[ox, oy + lift, oz, 'minecraft:' + name, props]]
                    if lift:
                        blocks.append([ox, oy, oz, 'minecraft:stone', {}])
                    x = ox + 0.5 - dx * 1.1
                    z = oz + 0.5 - dz * 1.1
                    p = player(rng, x, float(oy), z, yaw)
                    p.update(ground=True, shift=False, enabled=True, time=0, boost=None, move=[lab.fbits(0.0), lab.fbits(1.0)],
                             width=lab.fbits(0.6), height=lab.fbits(1.8), xr=lab.fbits(0.0))
                    yield dict(blocks=blocks, player=p, xa=lab.fbits(dx * step), za=lab.fbits(dz * step))
    # Riding something: canAutoJump refuses whatever the rest of the state says, so a step the
    # same walk jumps from on foot is never jumped here.
    for name, props in BLOCKS:
        for step in (0.215, 0.1):
            blocks = [[ox, oy, oz, 'minecraft:' + name, props]]
            p = player(rng, ox + 0.5, float(oy), oz - 0.6, 0.0)
            p.update(passenger=True, ground=True, shift=False, enabled=True, time=0, boost=None,
                     move=[lab.fbits(0.0), lab.fbits(1.0)], width=lab.fbits(0.6), height=lab.fbits(1.8), xr=lab.fbits(0.0))
            yield dict(blocks=blocks, player=p, xa=lab.fbits(0.0), za=lab.fbits(step))
    # Ceilings above the player and above the step.
    for ceiling in range(2, 6):
        for step_height in (0, 1):
            blocks = [[ox, oy + step_height, oz, 'minecraft:stone', {}], [ox - 1, oy + ceiling, oz, 'minecraft:stone', {}],
                      [ox, oy + ceiling + step_height, oz, 'minecraft:stone_slab', {'type': 'top'}]]
            p = player(rng, ox - 0.5, float(oy), oz + 0.5, -90.0)
            p.update(ground=True, shift=False, enabled=True, time=0, boost=None, move=[lab.fbits(0.0), lab.fbits(1.0)],
                     width=lab.fbits(0.6), height=lab.fbits(1.8))
            yield dict(blocks=blocks, player=p, xa=lab.fbits(0.2), za=lab.fbits(0.0))
    for _ in range(9000):
        blocks = placed(rng, rng.choice([1, 2, 4, 8, 16, 30]))
        x = ox + rng.uniform(-2, 3)
        z = oz + rng.uniform(-2, 3)
        y = float(oy) + rng.choice([0.0, 0.0, 0.0, 0.5, 1.0, 0.0625, 0.9375])
        yaw = rng.choice([0.0, 90.0, 180.0, -90.0, 45.0, rng.uniform(-360, 360)])
        p = player(rng, x, y, z, yaw)
        move = rng.choice([0.0, 0.0, 0.01, 0.1, 0.215, 0.3, rng.uniform(0, 0.4)])
        angle = rng.uniform(0, 6.283)

        yield dict(blocks=blocks, player=p, xa=lab.fbits(math.cos(angle) * move), za=lab.fbits(math.sin(angle) * move))


def main():
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'rust-auto-jump-parity'
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    resources_before = client.resource_fingerprint(resources)
    oracle = 'VanillaAutoJumpOracle'
    parity = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
    sources = [Path(__file__).resolve(), lab.ROOT / 'tools/oracle_harness.py', lab.ROOT / 'tools/lab.py',
               parity / (oracle + '.java'), parity / 'VanillaSceneCollisionOracle.java',
               parity / 'VanillaRegistryCollisionScanOracle.java', lab.ROOT / 'Cargo.lock',
               lab.ROOT / 'crates/horizon-client/examples/probe/auto_jump_probe.rs',
               lab.ROOT / 'crates/horizon-client/src/auto_jump.rs', lab.ROOT / 'crates/horizon-client/src/motion.rs']
    tested = base.source_hashes(sources)
    fixtures = list(cases(random.Random(0x4155544f)))
    inputs = build / 'cases.jsonl'
    inputs.write_text(''.join(json.dumps(f, separators=(',', ':')) + '\n' for f in fixtures))
    input_hash = lab.digest(inputs, 'sha256')
    classes, dependencies, _ = harness.compile_oracle(build, oracle, jars, libraries, lock['minecraft']['java_major'])
    command = harness.build_probe(build, 'auto_jump_probe')
    harness.run_logged([*command, resources, inputs, build / 'rust.txt', build / 'scene.json'], build, 'rust', timeout=3600)
    harness.java_run(build, classes, dependencies, oracle, [inputs, build / 'client.txt', resources, build / 'scene.json'], 'client')
    matched = harness.compare(build / 'client.txt', build / 'rust.txt', 2 * len(fixtures))
    base.ensure_sources_unchanged(sources, tested)
    if input_hash != lab.digest(inputs, 'sha256') or resources_before != client.resource_fingerprint(resources):
        raise AssertionError('Fixtures or resources changed during validation')
    jumps = sum(1 for line in (build / 'rust.txt').read_text().splitlines() if line == 'jump 1')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], manifest_sha1=lock['minecraft']['manifest_sha1'],
        platform=platform.platform(), rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0], fixtures=len(fixtures),
        auto_jumps=jumps, sources_sha256=tested, input_sha256=input_hash, resources=resources_before,
        client_jar_sha256=lab.digest(jars['client'], 'sha256'), matched_output_lines=matched,
        output_sha256=lab.digest(build / 'rust.txt', 'sha256'),
        scope='The actual LocalPlayer.updateAutoJump with canAutoJump (option, autoJumpTime, onGround, '
              'isStayingOnGroundSurface, passenger, input move vector, getBlockJumpFactor over the real level) on a '
              'LocalPlayer shell in an independently constructed copy of the resident test scene with each case\'s '
              'real block states placed; collisions through CollisionGetter.getCollisions/BlockCollisions with the '
              'player\'s EntityCollisionContext and real collision shapes, jump boost from a real MobEffectInstance. '
              'Rust runs MotionWorld::update_auto_jump over the client\'s loaded resources, collision registry and '
              'block-collision scan. Riders are compared through canAutoJump with the case\'s passenger flag (the movement '
              'world drives an unmounted player); entity collisions (none in the scene), the minecart paths and '
              'vehicle movement are not exercised.'))
    print(f'{len(fixtures):,} auto-jump cases ({jumps:,} jumps) match the pinned client: {report}', flush=True)


if __name__ == '__main__':
    main()
