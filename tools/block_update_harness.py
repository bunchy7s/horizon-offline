#!/usr/bin/env python3
"""Shared driver of the block update / random tick / redstone differential harnesses.

Cases are JSON lines understood by `VanillaBlockUpdateOracle` (vanilla block code
over a map-backed headless ServerLevel) and `block_update_fixture` (the Rust
`Behaviours` over the same kind of level). Both print one result block per case
(state changes, pending scheduled ticks, side-effect log, RNG fingerprint); this
module generates cases, runs both sides and reports differing cases grouped by
the block under test.
"""
import argparse
import json
import os
import random
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

NEIGHBOUR_PALETTE = [
    'stone', 'dirt', 'grass_block', 'water', 'lava', 'sand', 'oak_log', 'oak_planks', 'farmland', 'snow', 'snow_block',
    'ice', 'packed_ice', 'soul_sand', 'netherrack', 'mycelium', 'podzol', 'gravel', 'glass', 'oak_leaves', 'fire',
    'redstone_block', 'redstone_wire', 'redstone_torch', 'torch', 'lever', 'iron_block', 'magma_block', 'bedrock',
    'cobblestone', 'obsidian', 'tnt', 'oak_slab', 'oak_stairs', 'hopper', 'bamboo', 'cactus', 'sugar_cane', 'kelp',
    'seagrass', 'vine', 'moss_block', 'dripstone_block', 'amethyst_block', 'calcite', 'clay', 'coarse_dirt', 'rooted_dirt',
]

DIRS = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)]


class Facts:
    def __init__(self, resources):
        facts = json.loads((resources / 'facts.json').read_text())
        self.states = facts['states']
        self.defaults = facts['defaults']
        self.by_block = {}
        for index, state in enumerate(self.states):
            self.by_block.setdefault(state[0], []).append(index)
        self.palette = [name for name in ('minecraft:' + n for n in NEIGHBOUR_PALETTE) if name in self.by_block]
        self.names = list(self.by_block)

    def describe(self, index):
        return self.states[index][3].replace('Block{', '').replace('}', '', 1)

    def neighbour(self, rng, own):
        roll = rng.random()
        if roll < 0.30:
            return rng.choice(self.by_block[rng.choice(self.palette)])
        if roll < 0.45:
            return rng.choice(self.by_block[self.states[own][0]])
        if roll < 0.60:
            return rng.choice(self.by_block[rng.choice(self.names)])
        return None

    def cells(self, rng, origin_state, density=0.55, face_density=0.8, extent=2):
        cells = [[0, 0, 0, origin_state]]
        for x in range(-extent, extent + 1):
            for y in range(-extent, extent + 1):
                for z in range(-extent, extent + 1):
                    if (x, y, z) == (0, 0, 0):
                        continue
                    face = abs(x) + abs(y) + abs(z) == 1
                    if rng.random() > (face_density if face else density):
                        continue
                    state = self.neighbour(rng, origin_state)
                    if state is not None:
                        cells.append([x, y, z, state])
        return cells


def environment(rng):
    case = {'seed': rng.randrange(1 << 40), 'time': rng.randrange(0, 100000)}
    roll = rng.random()
    if roll < 0.5:
        case['light'] = [15, 0]
    elif roll < 0.7:
        case['light'] = [0, 0]
    elif roll < 0.85:
        case['light'] = [rng.randrange(16), rng.randrange(16)]
    else:
        case['light'] = [15, 15]
    if rng.random() < 0.1:
        case['raining'] = True
        case['thundering'] = rng.random() < 0.3
    return case


def put(cells, pos, state):
    for cell in cells:
        if cell[:3] == list(pos):
            cell[3] = state
            return
    cells.append([*pos, state])


def argument_parser(description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--count', type=int, default=40000, help='cases to generate')
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--only', help='substring of the block name under test')
    parser.add_argument('--show', type=int, default=1, help='examples printed per block')
    parser.add_argument('--events', action='store_true', help='deprecated; side-effect logs are always compared')
    parser.add_argument('--ignore', default='', help='comma separated line prefixes to ignore when comparing (e.g. S,G)')
    return parser


def blocks_of(facts):
    return facts.names


def run(command, **kwargs):
    done = subprocess.run(command, capture_output=True, text=True, **kwargs)
    if done.returncode:
        print((done.stdout + done.stderr)[-3000:])
        raise SystemExit(f'{command[0]} failed')
    return done


# The oracle's failure when a sapling's TreeGrower.growTree looks up Registries.FEATURE.
TREE_LOOKUP_ERROR = 'error IllegalStateException net.minecraft.core.RegistryAccess.lambda$lookupOrThrow$0('


def execute(name, cases, args, facts, origin_of=None):
    """Run both sides; return failing cases, counting mismatches and oracle errors."""
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.LOCAL / 'horizon-client' / lock['minecraft']['version']
    work = lab.LOCAL / name
    work.mkdir(parents=True, exist_ok=True)
    (work / 'cases.jsonl').write_text(''.join(json.dumps(case) + '\n' for case in cases))
    classes = work / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    java_root = lab.ROOT / 'mods/horizon/src/test/java'
    run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none', '-sourcepath', str(java_root), '-nowarn',
                    '-cp', dependencies, '-d', str(classes), str(java_root / 'dev/horizonlab/parity/VanillaBlockUpdateOracle.java')])
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaBlockUpdateOracle',
                    str(resources), str(work / 'cases.jsonl'), str(work / 'native.txt')])
    run(['cargo', 'run', '-q', '--release', '-p', 'horizon-world', '--example', 'world_probe', '--',
                    'block_update_fixture', str(resources), str(work / 'cases.jsonl'), str(work / 'rust.txt')], cwd=lab.ROOT, env={**os.environ, 'CARGO_BUILD_JOBS': '3'})
    ignore = tuple(prefix + ' ' for prefix in args.ignore.split(',') if prefix)
    native = split_cases((work / 'native.txt').read_text(), ignore)
    rust = split_cases((work / 'rust.txt').read_text(), ignore)
    assert len(native) == len(rust) == len(cases), (len(native), len(rust), len(cases))
    groups = {}
    errors = {}
    trees = 0
    for case, a, b in zip(cases, native, rust):
        if a and a[0].startswith('error'):
            # TreeGrower.growTree looks its feature up in the level's registries; the oracle's level has no
            # feature registry, and the probe no feature table (it reports X grow_tree). Neither side can grow
            # the tree, so the pair is not compared. Any other oracle error still fails.
            if a[0].startswith(TREE_LOOKUP_ERROR) and 'X grow_tree' in b:
                trees += 1
                continue
            errors.setdefault(a[0], []).append(case)
            continue
        if a != b:
            groups.setdefault(origin_name(case, facts), []).append((case, a, b))
    total = sum(len(v) for v in groups.values())
    for kind, entries in sorted(errors.items(), key=lambda entry: -len(entry[1])):
        print(f'oracle error x{len(entries)}: {kind} (e.g. {facts.describe(entries[0]["cells"][0][3])})')
    for block, entries in sorted(groups.items(), key=lambda entry: -len(entry[1])):
        print(f'{block}: {len(entries)} differing')
        for case, a, b in entries[:args.show]:
            print('  case ' + describe_case(case, facts))
            print('   native ' + ' | '.join(a))
            print('   rust   ' + ' | '.join(b))
    skipped = sum(len(v) for v in errors.values())
    if trees:
        print(f'{trees} saplings reached TreeGrower.growTree, which neither side runs here: not compared')
    compared = len(cases) - trees
    print(f'{compared - total - skipped}/{compared - skipped} cases match ({skipped} oracle errors)')
    (work / 'differences.json').write_text(json.dumps({k: [[c, a, b] for c, a, b in v[:5]] for k, v in groups.items()}))
    return total + skipped


def split_cases(text, ignore):
    out = []
    for block in text.split('case ')[1:]:
        lines = block.splitlines()[1:]
        out.append([line for line in lines if not line.startswith(ignore)] if ignore else lines)
    return out


def origin_name(case, facts):
    pos = case.get('pos', [0, 0, 0])
    for cell in case['cells']:
        if cell[:3] == pos:
            return facts.states[cell[3]][0]
    return 'minecraft:air'


def describe_case(case, facts):
    pos = case.get('pos', [0, 0, 0])
    parts = [case['mode'], facts.describe(next(c[3] for c in case['cells'] if c[:3] == pos))]
    if case['mode'] == 'shape':
        parts.append(f'dir={case["dir"]} neighbor={facts.describe(case["neighbor"])}')
    if case['mode'] == 'neighbor':
        parts.append(f'from={case["from"]}')
    if case.get('light') not in (None, [15, 0]):
        parts.append(f'light={case["light"]}')
    if case.get('raining'):
        parts.append('raining')
    near = []
    for dx, dy, dz in DIRS:
        at = [pos[0] + dx, pos[1] + dy, pos[2] + dz]
        state = next((c[3] for c in case['cells'] if c[:3] == at), None)
        near.append('air' if state is None else facts.describe(state))
    parts.append('faces=' + ';'.join(near))
    return ' '.join(parts) + f' seed={case["seed"]} time={case.get("time")}'


def load_facts():
    lock, _, _ = base.verify_artifacts()
    return Facts(lab.LOCAL / 'horizon-client' / lock['minecraft']['version'])


def state_pool(facts, only):
    return [i for i, s in enumerate(facts.states) if not only or only in s[0]]


def finish(differing):
    raise SystemExit(1 if differing else 0)
