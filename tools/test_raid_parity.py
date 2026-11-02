#!/usr/bin/env python3
"""Compare pillager patrols, raids and the raids saved data with vanilla.

Java (VanillaRaidOracle) runs vanilla's PatrolSpawner, Raids/Raid and
Raids.CODEC on a stand-in ServerLevel built from each case: boxes of blocks and
light, one biome, village sections (the point of interest distance tracker),
village points of interest, unloaded and non-ticking chunks, the overworld
clock, local difficulty inputs and at most one player. Rust (raid_fixture) runs
horizon_client::patrol and horizon_client::raid on the same world. Modes:

- patrol: PatrolSpawner.tick over many ticks (countdown, daylight, the one in
  five draw, the player, village distance, chunk and environment attribute
  checks, group size from the local difficulty, the members' heightmap
  positions and spawn rules, leader, banner and target), then the next draw of
  the level random, which pins every draw in between;
- raid: Raids.tick over scripted ticks: omens through createOrExtendRaid (the
  village centre from points of interest, the omen absorbed, the raid trigger
  stat), the countdown and its cached spawn position, wave composition by
  difficulty and bonus wave, findRandomSpawnPos, joining, leaders and riders,
  the boss bar (progress bits, title, visibility, the player), the horn with
  its seed and position, raiders leaving (far, gone, removed, idle outside the
  village), hurt, killed, joining and loaded raiders, victory with Hero of the
  Village, loss when the village goes, the celebration, peaceful, the raids
  rule, unloaded and non-ticking chunks and the timeout;
- codec: Raids.CODEC decoding generated and mutated raids.dat data (missing,
  mistyped and partly broken fields, number type conversions, duplicate ids)
  with resultOrPartial, then encoding it back.

finalizeSpawn runs with a fixed random in the oracle (the patrol spawner's own
draws are compared); raid buffs draw from each raider's unseeded random and
are not compared. A patrol leader's target is drawn from the pillager's
unseeded random; only that it lies around the world origin is compared.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

GROUND = ['minecraft:grass_block', 'minecraft:dirt', 'minecraft:sand', 'minecraft:stone', 'minecraft:snow_block',
          'minecraft:podzol', 'minecraft:mycelium', 'minecraft:gravel', 'minecraft:red_sand', 'minecraft:moss_block']
ODD = ['minecraft:water', 'minecraft:lava', 'minecraft:oak_leaves', 'minecraft:magma_block', 'minecraft:ice',
       'minecraft:glass', 'minecraft:bedrock', 'minecraft:snow[layers=1]', 'minecraft:snow[layers=8]',
       'minecraft:powder_snow', 'minecraft:oak_slab[type=bottom]', 'minecraft:oak_slab[type=top]', 'minecraft:barrier',
       'minecraft:short_grass', 'minecraft:rail', 'minecraft:cobweb', 'minecraft:oak_fence', 'minecraft:scaffolding',
       'minecraft:sweet_berry_bush', 'minecraft:fire', 'minecraft:spawner', 'minecraft:white_carpet']
BIOMES = ['minecraft:plains', 'minecraft:desert', 'minecraft:mushroom_fields', 'minecraft:forest', 'minecraft:snowy_plains',
          'minecraft:taiga', 'minecraft:savanna', 'minecraft:deep_dark', 'minecraft:river', 'minecraft:jungle']
DIFFICULTIES = ['peaceful', 'easy', 'normal', 'hard']
RAIDERS = ['pillager', 'vindicator', 'evoker', 'witch', 'ravager']


def box(x0, y0, z0, x1, y1, z1):
    return f'{x0},{y0},{z0},{x1},{y1},{z1}'


def terrain(rng, w, cx, cz, top=63, span=220):
    """Ground boxes around (cx, cz): a base with rises, dips, water and odd blocks."""
    ground = rng.choice(GROUND)
    k = [f'{box(cx - span, -64, cz - span, cx + span, top, cz + span)}/{ground}']
    for _ in range(rng.randrange(0, 14)):
        x, z = cx + rng.randrange(-span, span), cz + rng.randrange(-span, span)
        sx, sz = rng.randrange(1, 30), rng.randrange(1, 30)
        kind = rng.random()
        if kind < 0.3:
            h = top + rng.randrange(1, 12) if rng.random() < 0.8 else top + rng.randrange(100, 140)
            k.append(f'{box(x, top + 1, z, x + sx, h, z + sz)}/{rng.choice(GROUND)}')
        elif kind < 0.55:
            d = top - rng.randrange(1, 8)
            k.append(f'{box(x, d, z, x + sx, top, z + sz)}/minecraft:water')
        elif kind < 0.65:
            k.append(f'{box(x, top - rng.randrange(1, 120), z, x + sx, top, z + sz)}/minecraft:air')
        else:
            y = top + 1 + rng.randrange(0, 3)
            k.append(f'{box(x, y, z, x + sx, y + rng.randrange(0, 3), z + sz)}/{rng.choice(ODD)}')
    for i, v in enumerate(k):
        w[f'k{i}'] = v


def lights(rng, w, cx, cz, span=80):
    n = 0
    for _ in range(rng.randrange(0, 6)):
        x, z = cx + rng.randrange(-span, span), cz + rng.randrange(-span, span)
        w[f'l{n}'] = f'{box(x, -64, z, x + rng.randrange(1, 40), 320, z + rng.randrange(1, 40))}/{rng.randrange(0, 16)},{rng.randrange(0, 16)}'
        n += 1


def tokens(name, w):
    return name + ' ' + ' '.join(f'{k}={v}' for k, v in w.items())


def patrol_cases(rng, count):
    out = []
    for i in range(count):
        w = {'mode': 'patrol', 'seed': rng.randrange(-2**40, 2**40)}
        px, pz = rng.randrange(-300, 300), rng.randrange(-300, 300)
        terrain(rng, w, px, pz, span=90)
        lights(rng, w, px, pz, span=60)
        w['biome'] = rng.choice(BIOMES)
        w['difficulty'] = rng.choice(DIFFICULTIES + ['hard', 'normal'])
        w['darken'] = rng.choice([0, 0, 0, 0, 0, 1, 3, 4, 5, 11])
        w['sky'] = rng.choice([15, 15, 15, 7, 0])
        w['light'] = rng.choice([0, 0, 0, 0, 0, 5, 8, 9, 15])
        w['clock'] = rng.choice([0, 1000, 119999, 120000, 150000, 2000000, 5000000, 5000000, rng.randrange(0, 10**7), rng.randrange(0, 10**7)])
        w['inhabited'] = rng.choice([0, 0, 72000, 3600000, rng.randrange(0, 4 * 10**6)])
        w['moon'] = rng.choice(['0.0', '0.25', '0.5', '0.75', '1.0'])
        w['dim'] = rng.choice(['minecraft:overworld'] * 12 + ['minecraft:the_nether', 'minecraft:the_end'])
        if rng.random() < 0.85:
            w['p0'] = f'{px + 0.5},{rng.choice([64, 64, 70, 40])},{pz + 0.5}' + rng.choice([''] * 8 + ['/spectator', '/creative'])
        if rng.random() < 0.3:
            vx, vz = (px >> 4) + rng.randrange(-4, 5), (pz >> 4) + rng.randrange(-4, 5)
            w['vil'] = f'{vx},{rng.randrange(2, 6)},{vz}'
        if rng.random() < 0.35:
            w['unloaded'] = ';'.join(f'{(px >> 4) + rng.randrange(-5, 6)},{(pz >> 4) + rng.randrange(-5, 6)}'
                                     for _ in range(rng.randrange(1, 8)))
        if rng.random() < 0.05:
            w['patrols'] = 0
        if rng.random() < 0.05:
            w['enemies'] = 0
        w['next'] = rng.choice([0, 0, 1, 2, 7, -5])
        w['ticks'] = rng.choice([1, 2, 3, 12001, 26000, 40000, 66000, 90000, 130000])
        out.append(tokens(f'patrol{i}', w))
    return out


def raid_cases(rng, count):
    out = []
    for i in range(count):
        cx, cz = rng.randrange(-200, 200), rng.randrange(-200, 200)
        w = {'mode': 'raid', 'rseed': rng.randrange(-2**40, 2**40)}
        terrain(rng, w, cx, cz, span=110)
        w['difficulty'] = rng.choice(['easy', 'normal', 'hard', 'hard', 'normal'])
        w['biome'] = rng.choice(BIOMES)
        if rng.random() < 0.06:
            w['dim'] = rng.choice(['minecraft:the_nether', 'minecraft:the_end'])
        # The village: a few sections around the centre, points of interest in them.
        sx, sz = cx >> 4, cz >> 4
        sections = {(sx, 4, sz)}
        for _ in range(rng.randrange(0, 6)):
            sections.add((sx + rng.randrange(-2, 3), rng.choice([3, 4, 4, 5]), sz + rng.randrange(-2, 3)))
        w['vil'] = ';'.join(f'{a},{b},{c}' for a, b, c in sorted(sections))
        pois = [(cx + rng.randrange(-40, 41), 64 + rng.randrange(-3, 4), cz + rng.randrange(-40, 41))
                for _ in range(rng.randrange(0, 12))]
        if pois:
            w['poi'] = ';'.join(f'{a},{b},{c}' for a, b, c in pois)
        if rng.random() < 0.15:
            w['unloaded'] = ';'.join(f'{sx + rng.randrange(-8, 9)},{sz + rng.randrange(-8, 9)}' for _ in range(rng.randrange(1, 10)))
        if rng.random() < 0.15:
            w['noticking'] = ';'.join(f'{sx + rng.randrange(-8, 9)},{sz + rng.randrange(-8, 9)}' for _ in range(rng.randrange(1, 10)))
        far = rng.random() < 0.15
        w['p0'] = f'{cx + (rng.randrange(80, 150) if far else rng.randrange(-20, 20)) + 0.5},64,{cz + rng.randrange(-20, 20) + 0.5}' \
            + rng.choice([''] * 10 + ['/spectator'])
        ev = []

        def at(t, e):
            ev.append((t, e))
        omen_pos = f'{cx + rng.randrange(-30, 31)},64,{cz + rng.randrange(-30, 31)}'
        at(0, f'omen:{rng.choice([0, 0, 0, 1, 2, 4])}:{omen_pos}')
        style = rng.choice(['win', 'win', 'win', 'lose', 'leave', 'misc', 'idle', 'timeout' if rng.random() < 0.15 else 'win'])
        ticks = rng.choice([700, 1500, 3000, 4500, 6000])
        hero = rng.random() < 0.8
        if style == 'win':
            step = rng.choice([7, 25, 60, 130])
            for t in range(301, ticks, step):
                if rng.random() < 0.3:
                    at(t, f'hurtall:{rng.choice(["0.5", "4", "11.25", "30"])}')
                at(t, 'killall:p' if hero else 'killall')
            if rng.random() < 0.4:
                at(rng.randrange(310, 1200), f'omen:{rng.randrange(0, 3)}:{omen_pos}')
        elif style == 'lose':
            for t in range(301, ticks, 97):
                at(t, 'killall:p')
            gone = rng.randrange(320, 2000)
            for a, b, c in sorted(sections):
                at(gone, f'unvil:{a},{b},{c}')
        elif style == 'leave':
            at(320, f'move:0:{cx + rng.choice([-120, 130, 200])},64,{cz}')
            at(321, f'move:1:{cx + 40},64,{cz + 60}')
            at(330, 'age:1:700')
            at(330, 'idle:1:2500')
            at(340, 'age:2:900')
            at(345, 'gone:2')
            at(350, 'discard:3')
            at(360, 'hurt:1:3.5')
            at(400, 'kill:1:p')
            for t in range(1500, ticks, 113):
                at(t, 'killall:p')
        elif style == 'idle':
            for j in range(4):
                at(310, f'age:{j}:{rng.choice([500, 601, 650, 2000])}')
                at(310, f'idle:{j}:{rng.choice([2000, 2401, 3000])}')
                at(310, f'move:{j}:{cx + rng.randrange(-90, 90)},64,{cz + rng.randrange(-90, 90)}')
            for t in range(1400, ticks, 211):
                at(t, 'killall:p')
        elif style == 'timeout':
            ticks = 48100
        else:  # misc
            at(rng.randrange(1, 400), f'join:{rng.choice(RAIDERS)}:{cx + rng.randrange(-30, 30)},64,{cz + rng.randrange(-30, 30)}')
            at(rng.randrange(300, 700), f'loadmob:{rng.choice(RAIDERS[:3])}:{cx},64,{cz}:{rng.randrange(0, 3)}:leader')
            at(rng.randrange(300, 700), f'loadmob:{rng.choice(RAIDERS)}:{cx + 3},64,{cz}:{rng.randrange(0, 3)}')
            at(rng.randrange(1, 900), f'join:{rng.choice(RAIDERS)}:{cx + rng.randrange(-30, 30)},64,{cz + rng.randrange(-30, 30)}')
            for _ in range(rng.randrange(1, 5)):
                at(rng.randrange(1, 2000), rng.choice([
                    f'diff:{rng.choice(DIFFICULTIES)}', 'rules:0', 'rules:1', f'unload:{sx},{sz}', f'load:{sx},{sz}',
                    f'notick:{sx + rng.randrange(-6, 7)},{sz + rng.randrange(-6, 7)}', 'pdead', 'palive',
                    f'pmove:{cx + rng.randrange(-120, 120) + 0.5},64,{cz + 0.5}', f'setomen:{rng.randrange(0, 6)}', 'stop',
                    f'vil:{sx + rng.randrange(-3, 4)},4,{sz + rng.randrange(-3, 4)}',
                    f'hurt:{rng.randrange(0, 6)}:{rng.choice(["1", "2.5", "50"])}', f'kill:{rng.randrange(0, 6)}',
                    f'omen:{rng.randrange(0, 3)}:{cx + rng.randrange(-200, 200)},64,{cz + rng.randrange(-200, 200)}',
                ]))
            for t in range(400, ticks, 157):
                at(t, 'killall:p')
        w['ticks'] = ticks
        for j, (t, e) in enumerate(sorted(ev, key=lambda x: x[0])):
            w[f'ev{j}'] = f'{t}:{e}'
        out.append(tokens(f'raid{i}', w))
    return out


def snbt_number(rng, v, kind):
    """A number as SNBT of the field's type, or of another numeric type."""
    if rng.random() < 0.85:
        return {'b': f'{v}b', 'i': str(v), 'l': f'{v}L', 'f': f'{v}f'}[kind]
    t = rng.choice(['b', 's', 'i', 'l', 'f', 'd'])
    if t == 'b':
        return f'{max(-128, min(127, v))}b'
    if t == 's':
        return f'{max(-32768, min(32767, v))}s'
    if t == 'l':
        return f'{v}L'
    if t == 'f':
        return f'{v}.5f' if rng.random() < 0.5 else f'{v}f'
    if t == 'd':
        return f'{v}.75d' if rng.random() < 0.5 else f'{v}d'
    return str(v)


def raid_snbt(rng, rid):
    fields = {
        'id': snbt_number(rng, rid, 'i'),
        'started': snbt_number(rng, rng.randrange(0, 2), 'b'),
        'active': snbt_number(rng, rng.randrange(0, 2), 'b'),
        'ticks_active': snbt_number(rng, rng.randrange(0, 50000), 'l'),
        'raid_omen_level': snbt_number(rng, rng.randrange(0, 6), 'i'),
        'groups_spawned': snbt_number(rng, rng.randrange(0, 9), 'i'),
        'cooldown_ticks': snbt_number(rng, rng.randrange(0, 301), 'i'),
        'post_raid_ticks': snbt_number(rng, rng.randrange(0, 41), 'i'),
        'total_health': rng.choice(['0.0f', '96.0f', '322.5f', '24.0f', f'{rng.randrange(0, 500)}f', f'{rng.randrange(0, 500)}.25d', '7L']),
        'group_count': snbt_number(rng, rng.choice([0, 3, 5, 7]), 'i'),
        'status': '"' + rng.choice(['ongoing', 'victory', 'loss', 'stopped']) + '"',
        'center': rng.choice(['[I;{0},{1},{2}]', '[{0},{1},{2}]', '[I;{0},{1},{2}]', '[L;{0}L,{1}L,{2}L]', '[B;{3}b,{3}b,{3}b]'])
        .format(rng.randrange(-30000, 30000), rng.randrange(-64, 320), rng.randrange(-30000, 30000), rng.randrange(-100, 100)),
        'heroes_of_the_village': '[' + ','.join(
            rng.choice(['[I;{0},{1},{2},{3}]', '[I;{0},{1},{2},{3}]', '[{0},{1},{2},{3}]'])
            .format(*(rng.randrange(-2**31, 2**31) for _ in range(4))) for _ in range(rng.randrange(0, 20))) + ']',
    }
    # Mutations: a missing field, a wrong type, a bad status or centre, broken heroes.
    roll = rng.random()
    if roll < 0.08:
        del fields[rng.choice(list(fields))]
    elif roll < 0.14:
        fields[rng.choice(list(fields))] = rng.choice(['"text"', '{}', '[]', '[I;1,2]'])
    elif roll < 0.18:
        fields['status'] = '"won"'
    elif roll < 0.22:
        fields['center'] = '[I;1,2,3,4]'
    elif roll < 0.3:
        fields['heroes_of_the_village'] = '[[I;1,2,3,4],[I;1,2],"x",[I;5,6,7,8]]'
    elif roll < 0.34:
        fields['extra'] = '1b'
    return '{' + ','.join(f'{k}:{v}' for k, v in fields.items()) + '}'


def codec_cases(rng, count):
    out = []
    for i in range(count):
        n = rng.choice([0, 1, 1, 2, 3, 5, 12, 30])
        ids = [rng.choice([rng.randrange(1, 40), rng.randrange(-5, 5), rng.randrange(0, 1000)]) for _ in range(n)]
        raids = ','.join(raid_snbt(rng, rid) for rid in ids)
        top = {'raids': f'[{raids}]', 'next_id': snbt_number(rng, rng.randrange(0, 100), 'i'), 'tick': snbt_number(rng, rng.randrange(0, 10**6), 'i')}
        roll = rng.random()
        if roll < 0.05:
            del top['raids']
        elif roll < 0.1:
            del top[rng.choice(['next_id', 'tick'])]
        elif roll < 0.13:
            top['raids'] = '"none"'
        elif roll < 0.16:
            top['raids'] = '[1,2,3]'
        text = '{' + ','.join(f'{k}:{v}' for k, v in top.items()) + '}'
        out.append(f'codec{i} mode=codec snbt={text.encode().hex()}')
    return out


def corpus(seed=26_3):
    rng = random.Random(seed)
    return patrol_cases(rng, 1000) + raid_cases(rng, 400) + codec_cases(rng, 500)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--only', help='only cases whose name starts with this')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-raid-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaRaidOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/raid_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/raid.rs', lab.ROOT / 'crates/horizon-client/src/patrol.rs']
    tested = base.source_hashes(sources)
    inputs = build / 'inputs.txt'
    cases = corpus()
    if args.only:
        cases = [c for c in cases if c.startswith(args.only)]
    inputs.write_text(''.join(line + '\n' for line in cases), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        with (build / (name + '.log')).open('w', encoding='utf-8') as log:
            base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    run(['java', '-Xmx3G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
         'dev.horizonlab.parity.VanillaRaidOracle', inputs, build / 'server.txt'], 'server')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples/client_probe'
    run([binary, 'raid_fixture', resources, inputs, build / 'rust.txt'], 'rust')
    vanilla = (build / 'server.txt').read_text().splitlines()
    errors = [l for l in vanilla if ' error ' in l]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    rust = (build / 'rust.txt').read_text().splitlines()
    bad = []
    for a, b in zip(vanilla, rust):
        if a == b:
            continue
        xs, ys = a.split(' ; '), b.split(' ; ')
        i = next((i for i, (p, q) in enumerate(zip(xs, ys)) if p != q), min(len(xs), len(ys)))
        bad.append((xs[0].split(' ')[0], ' ; '.join(xs[max(0, i - 2):i + 2]), ' ; '.join(ys[max(0, i - 2):i + 2])))
    if bad or len(vanilla) != len(rust):
        report = '\n'.join(f'  {n}\n    vanilla {a[:1200]}\n    rust    {b[:1200]}' for n, a, b in bad[:12])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} cases differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged(sources, tested)
    count = {m: sum(1 for c in cases if f' mode={m}' in c) for m in ('patrol', 'raid', 'codec')}
    spawns = sum(line.count(' ; spawn minecraft:pillager') for line in vanilla if line.startswith('patrol'))
    waves = sum(line.count(' ; horn ') for line in vanilla if line.startswith('raid'))
    raiders = sum(line.count(' ; spawn ') for line in vanilla if line.startswith('raid'))
    victories = sum('effect minecraft:hero_of_the_village' in line for line in vanilla)
    losses = sum(' loss ' in line for line in vanilla)
    fresh = sum(line.split(' ', 1)[1].startswith('fresh') for line in vanilla if line.startswith('codec'))
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': count,
        'patrol_spawns': spawns, 'raid_horns': waves, 'raiders_spawned': raiders, 'hero_effects': victories, 'losses': losses,
        'codec_fresh': fresh, 'sources_sha256': tested,
        'scope': 'PatrolSpawner.tick and spawnPatrolMember; Raids.tick, createOrExtendRaid and getNearbyRaid; Raid.tick, '
                 'spawnGroup, joinRaid, addWaveMob, removeFromRaid, updateRaiders, updateBossbar, playSound, findRandomSpawnPos, '
                 'moveRaidCenterToNearbyVillageSection, absorbRaidOmen, leaders and heroes; Raids.CODEC with resultOrPartial. '
                 'finalizeSpawn runs with a fixed random; raid buffs and patrol targets come from unseeded entity randoms.'})
    print(f'{count["patrol"]:,} patrol, {count["raid"]:,} raid and {count["codec"]:,} codec cases match vanilla '
          f'({spawns:,} patrol pillagers, {waves:,} raid waves, {raiders:,} raiders, {victories} victories, {losses} losses)')


if __name__ == '__main__':
    main()
