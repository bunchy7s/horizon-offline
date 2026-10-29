#!/usr/bin/env python3
"""Compare Horizon's ParticleEngine with the pinned client's ParticleProviders:
every supported particle type is built through its real registered provider
(real ParticleResources, sprite sets bound to stand-in sprites named after the
vanilla particles/*.json textures) in a stand-in ClientLevel (air, optionally a
stone floor at y 63), then ticked; position, motion, age, lifetime, size,
colour, roll, sprite, UVs and light are compared tick by tick, with the
particles and sounds each one asks the level for and every random draw.

Each vanilla Particle owns a clock-seeded RandomSource.create(); the oracle
recovers that seed and runs first, and the probe replays the same seed."""
import json
import platform
import random
import struct
import subprocess

try:
    from . import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import lab, oracle_harness as harness, test_rust_parity as base, test_model_baking_parity as baking

STONE = 1  # Block.stateById(1) in 26.3

# Types Horizon implements, with the options each is built with.
SIMPLE = """angry_villager ash bubble bubble_column_up bubble_pop campfire_cosy_smoke campfire_signal_smoke
cherry_leaves cloud composter crimson_spore crit current_down damage_indicator dolphin dripping_dripstone_lava
dripping_dripstone_water dripping_honey dripping_lava dripping_obsidian_tear dripping_water egg_crack enchant
enchanted_hit end_rod explosion explosion_emitter falling_dripstone_lava falling_dripstone_water falling_honey
falling_lava falling_nectar falling_obsidian_tear falling_spore_blossom falling_water firework flame copper_fire_flame
gust gust_emitter_large gust_emitter_small happy_villager heart infested landing_honey landing_lava
landing_obsidian_tear large_smoke lava mycelium nautilus note pale_oak_leaves poof portal rain reverse_portal
sculk_charge_pop sculk_soul smoke small_flame small_gust sneeze snowflake sonic_boom soul soul_fire_flame splash
spore_blossom_air totem_of_undying underwater vault_connection warped_spore white_ash white_smoke witch
sweep_attack glow wax_on wax_off electric_spark scrape squid_ink glow_squid_ink item_slime item_snowball item_cobweb
sulfur_cube_goo firefly trial_spawner_detection trial_spawner_detection_ominous trial_omen raid_omen ominous_spawning
spit fishing dust_plume red_poplar_leaves orange_poplar_leaves yellow_poplar_leaves pause_mob_growth
reset_mob_growth sulfur_bubbles""".split()


def options(kind, rng):
    if kind in ('block', 'block_crumble', 'dust_pillar', 'falling_dust', 'block_marker'):
        return dict(state=STONE)
    if kind == 'dust':
        return dict(color=rng.choice([0xFF0000, 0x3A9E2C, 0xFFFFFF]), scale=rng.choice([1.0, 0.5, 2.0]))
    if kind == 'dust_color_transition':
        return dict(color=0x39D7E0, to=0xFF0000, scale=rng.choice([1.0, 2.0]))
    if kind in ('effect', 'instant_effect'):
        return dict(color=rng.choice([0xFFFFFFFF, 0xFF3366CC]), scale=rng.choice([1.0, 0.5, 2.5]))
    if kind == 'entity_effect':
        return dict(color=rng.choice([0xFF3366CC, 0x26AA00FF]))
    if kind == 'tinted_leaves':
        return dict(color=0xFF48B518)
    if kind == 'flash':
        return dict(color=rng.choice([0xFF3366CC, 0x00FFAA00]))
    if kind == 'item':
        return dict(item=rng.choice(['minecraft:apple', 'minecraft:cooked_beef', 'minecraft:egg']))
    if kind == 'dragon_breath':
        return dict(scale=rng.choice([1.0, 0.25]))
    if kind == 'sculk_charge':
        return dict(scale=rng.choice([0.0, 1.5707964, 3.1415927]))
    if kind == 'shriek':
        return dict(scale=rng.choice([0, 5]))
    if kind == 'trail':
        return dict(target=[2.5, 67.0, -3.25], color=0x7F3F1F, duration=rng.choice([10, 37]))
    return {}


def cases(rng):
    kinds = SIMPLE + ['block', 'block_crumble', 'dust_pillar', 'falling_dust', 'dust', 'dust_color_transition', 'effect',
                      'instant_effect', 'entity_effect', 'tinted_leaves', 'dragon_breath', 'sculk_charge', 'shriek', 'trail',
                      'block_marker', 'flash', 'item']
    # Dyadic values: serde_json's decimal parsing may round others a unit off.
    starts = [([0.3125, 66.6875, -0.40625], False), ([0.3125, 64.046875, -0.40625], True), ([-5.75, 64.59375, 3.125], True)]
    velocities = [[0.0, 0.0, 0.0], [0.078125, 0.125, -0.046875], [0.0, -0.296875, 0.0], [0.5, 0.0, 0.25]]
    for kind in kinds:
        for (pos, floor) in starts:
            for vel in velocities:
                yield dict(type=kind, options=options(kind, rng), pos=pos, vel=vel, floor=floor,
                           seed=rng.randrange(-2**63, 2**63), ticks=80)
    # In still water (sources from y 64 to 67): the bubbles rise until they leave it.
    for kind in ['sulfur_bubbles', 'bubble', 'bubble_column_up']:
        for (pos, floor) in [([0.3125, 64.046875, -0.40625], True), ([-5.75, 66.59375, 3.125], False)]:
            for vel in velocities:
                yield dict(type=kind, options={}, pos=pos, vel=vel, floor=floor, water=True,
                           seed=rng.randrange(-2**63, 2**63), ticks=80)


def make_cases(build):
    path = build / 'cases.jsonl'
    count = 0
    with path.open('w') as out:
        for case in cases(random.Random(0x50415254)):
            out.write(json.dumps(case, separators=(',', ':')) + '\n')
            count += 1
    return path, count


def decode(token):
    if len(token) == 16:
        try:
            return f'{struct.unpack(">d", bytes.fromhex(token))[0]!r}'
        except ValueError:
            return token
    if len(token) == 8:
        try:
            return f'{struct.unpack(">f", bytes.fromhex(token))[0]!r}'
        except ValueError:
            return token
    return token


# Particles whose tick calls Math.cos/sin/pow. On x86-64 HotSpot's intrinsics
# (Intel's libm) round a fraction of a percent of arguments a unit apart from
# the platform libm horizon_core::jmath uses, so their motion is compared to
# 1e-12 relative error; everything else about them, and every other type, exactly.
LIBM = {'cherry_leaves', 'pale_oak_leaves', 'tinted_leaves', 'red_poplar_leaves', 'orange_poplar_leaves',
        'yellow_poplar_leaves'}


def sections(path):
    out, name = {}, None
    for line in path.read_text(encoding='utf-8', errors='replace').splitlines():
        if line.startswith('== '):
            name = int(line[3:])
            out[name] = []
        elif name is not None:
            out[name].append(line)
    return out


def close(a, b):
    ta, tb = a.split(), b.split()
    if len(ta) != len(tb):
        return False
    for x, y in zip(ta, tb):
        if x == y:
            continue
        if len(x) != 16 or len(y) != 16:
            return False
        try:
            fx, fy = (struct.unpack('>d', bytes.fromhex(t))[0] for t in (x, y))
        except ValueError:
            return False
        if not abs(fx - fy) <= 1e-12 * max(1.0, abs(fx)):
            return False
    return True


def tolerated(build, cases_path):
    """Lines of LIBM cases that differ only within the libm tolerance, or None."""
    rows = cases_path.read_text().splitlines()
    left, right = sections(build / 'client.txt'), sections(build / 'rust.txt')
    if left.keys() != right.keys():
        return None
    count = 0
    for name in left:
        a, b = left[name], right[name]
        if a == b:
            continue
        if json.loads(rows[name])['type'] not in LIBM or len(a) != len(b):
            return None
        for x, y in zip(a, b):
            if x != y:
                if not close(x, y):
                    return None
                count += 1
    return count


def explain(build, cases_path):
    rows = cases_path.read_text().splitlines()
    left, right = sections(build / 'client.txt'), sections(build / 'rust.txt')
    report, count, kinds = '', 0, {}
    for name in left:
        if left[name] != right.get(name):
            count += 1
            kind = json.loads(rows[name])['type']
            kinds[kind] = kinds.get(kind, 0) + 1
            if kinds[kind] <= 1:
                a, b = left[name], right.get(name, [])
                diff = next(((x, y) for x, y in zip(a, b) if x != y), (f'<{len(a)} lines>', f'<{len(b)} lines>'))
                pretty = [' '.join(decode(t) for t in s.split()) for s in diff]
                report += f'\n  case {name} {rows[name][:160]}\n    vanilla: {pretty[0]}\n    rust:    {pretty[1]}'
    return f'\n{count} differing cases by type: {kinds}' + report


def main():
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    name, oracle, probe = 'particle-providers-parity', 'VanillaParticleProvidersOracle', 'particle_providers_probe'
    build = lab.LOCAL / name
    build.mkdir(exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    sources = [lab.ROOT / p for p in [
        'tools/test_particle_providers_parity.py', 'tools/oracle_harness.py', 'tools/test_rust_parity.py', 'tools/lab.py',
        f'mods/horizon/src/test/java/dev/horizonlab/parity/{oracle}.java', 'Cargo.toml', 'Cargo.lock',
        'crates/horizon-client/Cargo.toml', 'crates/horizon-client-render/src/particles.rs',
        f'crates/horizon-client/examples/probe/{probe}.rs', 'crates/horizon-core/src/random.rs']]
    tested = base.source_hashes(sources)
    cases_path, units = make_cases(build)
    classes, dependencies, _ = harness.compile_oracle(build, oracle, jars, libraries, lock['minecraft']['java_major'])
    command = harness.build_probe(build, probe)
    # The oracle runs first: the probe replays the particle seeds it recovered.
    harness.java_run(build, classes, dependencies, oracle, [cases_path, build / 'client.txt'], 'client')
    harness.run_logged([*command, cases_path, build / 'client.txt', build / 'rust.txt'], build, 'rust')
    tolerance = 0
    try:
        matched = base.compare(build / 'client.txt', build / 'rust.txt')
    except AssertionError as error:
        tolerance = tolerated(build, cases_path)
        if tolerance is None:
            raise AssertionError(str(error) + explain(build, cases_path)) from None
        matched = sum(len(v) + 1 for v in sections(build / 'client.txt').values())
    base.ensure_sources_unchanged(sources, tested)
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        rustc=subprocess.check_output(['rustc', '--version'], text=True).strip(), units=units,
        sources_sha256=tested, input_sha256=lab.digest(cases_path, 'sha256'),
        client_jar_sha256=lab.digest(jars['client'], 'sha256'), matched_output_lines=matched,
        output_sha256=lab.digest(build / 'rust.txt', 'sha256'), libm_tolerated_lines=tolerance,
        scope=('Real ParticleResources providers for every type Horizon implements, ticked in an air level with an optional '
               'stone floor and, for the bubbles, still water: construction (provider, level, SpellParticle and particle-local random draws), per-tick '
               'position, previous position, motion, age, lifetime, removal, onGround, quad size at partial 0 and 0.5, '
               'colour, alpha, roll, sprite, UVs and light at partial 0.5 (unloaded chunk), and the particles and sounds '
               'requested from the level. Flowing fluids, block light, render layers/facing and the ParticleEngine group limits '
               'are not exercised.')))
    print(f'{units:,} particle cases match the pinned client ({matched:,} output lines, {tolerance} within '
          f'the libm tolerance): {report}', flush=True)


if __name__ == '__main__':
    main()
