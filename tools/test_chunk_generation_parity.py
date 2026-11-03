#!/usr/bin/env python3
"""Compare whole generated chunks block-for-block with vanilla.

Vanilla side: VanillaChunkGenerationOracle drives the actual ChunkGenerator
stages (structure starts and references, biomes, terrain with surface and
carvers, then biome decoration with every placed feature and structure piece)
through real WorldGenRegion instances. Horizon side: the chunk_dump probe
runs the same stages in the same order. Every block state, quart biome and
block entity (canonical NBT) of each target chunk is compared.

Cases: terrain sites for several seeds across the overworld, the Nether and the
End, plus rectangles around vanilla structure starts found by scanning each
seed (villages, mineshafts, strongholds, ancient cities, trial chambers,
monuments, mansions, ...), and the fixed sites of REGRESSIONS.

usage: test_chunk_generation_parity.py [--seeds N] [--per-structure K]
       [--filter REGEX] [--dimension overworld|the_nether|the_end] [--keep]
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
import random
import re
import subprocess
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

SEEDS = [12345, -7046029254386353131, 987654321987, 4242, -1, 0x5DEECE66D, 20260607, 777]
# Sites from bug reports, compared on every run: seed, dimension, chunk
# rectangle x0,z0,x1,z1 and label.
REGRESSIONS = [
    (-1860902098141295193, 'the_nether', '-1,6,3,9', 'bastion_remnant across 20 chunks (#249)'),
]
RESOURCES = lab.RESOURCES
ORACLE = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaChunkGenerationOracle.java'


def build_dir():
    path = lab.LOCAL / 'chunk-generation-parity'
    path.mkdir(parents=True, exist_ok=True)
    return path


class Oracle:
    def __init__(self):
        self.lock, self.jars, self.libraries = base.verify_artifacts()
        self.dependencies = os.pathsep.join(map(str, [self.jars['server'], *self.libraries]))
        self.classes = build_dir() / 'classes'
        self.classes.mkdir(exist_ok=True)
        result = subprocess.run(['javac', '--release', str(self.lock['minecraft']['java_major']), '-proc:none', '-cp', self.dependencies,
                                 '-d', str(self.classes), str(ORACLE)], capture_output=True, text=True)
        if result.returncode:
            raise SystemExit('javac failed:\n' + result.stderr[-4000:])

    def run(self, out, args):
        result = subprocess.run(['java', '-Xmx8G', '-cp', str(self.classes) + os.pathsep + self.dependencies,
                                 'dev.horizonlab.parity.VanillaChunkGenerationOracle', *map(str, [args[0], out, *args[1:]])],
                                capture_output=True, text=True, env=dict(os.environ))
        if result.returncode:
            output = (result.stdout + result.stderr).splitlines()
            lines = [l for l in output if 'Exception' in l or 'Caused' in l or '\tat ' in l]
            # A JVM that dies without an exception (killed for memory, no heap to reserve) has none of those lines:
            # say how it ended and show its last output, or the failure reads as an empty message.
            how = f'exit {result.returncode}' + (' (killed by signal %d)' % -result.returncode if result.returncode < 0 else '')
            raise SystemExit(f'oracle failed ({how}): ' + ' | '.join(lines[:10] or output[-10:]))


def rust_binary():
    subprocess.run(['cargo', 'build', '--release', '-q', '-p', 'horizon-world', '--example', 'world_probe'], check=True, cwd=lab.ROOT)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    return Path(metadata['target_directory']) / 'release/examples/world_probe'


def run_rust(binary, out, seed, dimension, rects, scan=False):
    env = dict(os.environ)
    if scan:
        env['HORIZON_SCAN'] = '1'
    with open(out, 'w') as handle:
        result = subprocess.run([str(binary), 'chunk_dump', str(RESOURCES), str(seed), dimension, *rects], stdout=handle, stderr=subprocess.PIPE, text=True, env=env, cwd=lab.ROOT)
    if result.returncode:
        raise SystemExit('chunk_dump failed: ' + result.stderr[-2000:])
    return result.stderr


def parse(path):
    chunks, current = {}, None
    for line in Path(path).read_text().splitlines():
        if line.startswith('chunk '):
            _, x, z = line.split()
            current = (int(x), int(z))
            chunks[current] = {'q': {}, 'y': {}, 'be': {}, 't': [], 'p': []}
        elif line.startswith('q '):
            _, q, row = line.split(' ', 2)
            chunks[current]['q'][int(q)] = row
        elif line.startswith('y '):
            _, y, row = line.split(' ', 2)
            chunks[current]['y'][int(y)] = row
        elif line.startswith('t '):
            chunks[current]['t'].append(line[2:])
        elif line.startswith('p '):
            chunks[current]['p'].append(line[2:])
        elif line.startswith('be '):
            head, *forms = line.split('\t')
            parts = head.split(' ')
            chunks[current]['be'][(int(parts[1]), int(parts[2]), int(parts[3]))] = forms
    return chunks


def parse_nbt(text, i=0):
    """Canonical NBT text -> Python values (typed scalars as (type, text) tuples)."""
    c = text[i]
    if c == '{':
        i += 1
        value = {}
        while text[i] != '}':
            j = text.index('=', i)
            key = text[i:j]
            value[key], i = parse_nbt(text, j + 1)
            i += 1  # comma
        return value, i + 1
    if c == '[':
        i += 1
        items = []
        while text[i] != ']':
            item, i = parse_nbt(text, i)
            items.append(item)
            i += 1
        return items, i + 1
    if c == '"':
        i += 1
        out = []
        while text[i] != '"':
            if text[i] == '\\':
                i += 1
            out.append(text[i])
            i += 1
        return ('str', ''.join(out)), i + 1
    if c in 'BIL' and text[i + 1] == '[':
        j = text.index(']', i)
        return (c, text[i + 2:j]), j + 1
    j = i + 1
    while j < len(text) and text[j] not in ',}]':
        j += 1
    return (c, text[i + 1:j]), j


def nbt_matches(vanilla, rust, default):
    """Horizon stores the placed data only; vanilla saves defaults too. Every key
    Horizon sets must match, and every other vanilla key must be what vanilla
    loads for an absent key (the oracle's second form)."""
    if isinstance(vanilla, dict) and isinstance(rust, dict):
        default = default if isinstance(default, dict) else {}
        if 'LootTableSeed' in rust and 'LootTable' not in rust:
            rust = {k: v for k, v in rust.items() if k != 'LootTableSeed'}  # vanilla saves the seed only with a table
        for key, value in vanilla.items():
            if key in ('x', 'y', 'z') or (key == 'components' and value == {}):
                continue
            if key in rust:
                if not nbt_matches(value, rust[key], default.get(key)):
                    return False
            elif value != default.get(key):
                return False
        return all(key in vanilla or key in ('x', 'y', 'z') or nbt_matches(default.get(key), rust[key], None) or (key, rust[key]) == VAULT_REWARD
                   or omitted_on_save(key, rust) for key in rust)
    if isinstance(vanilla, tuple) and vanilla[0] == 'str' and isinstance(rust, dict) and set(rust) == {'text'}:
        # A literal text component saves as its plain string: {text:"a"} is "a".
        return vanilla == rust['text']
    if isinstance(vanilla, list) and isinstance(rust, list):
        return len(vanilla) == len(rust) and all(nbt_matches(a, b, None) for a, b in zip(vanilla, rust))
    if isinstance(vanilla, tuple) and isinstance(rust, tuple) and vanilla[0] in 'bsil' and rust[0] in 'bsil':
        # Structure templates keep their stored integer width; vanilla re-saves block entities with its own.
        return vanilla[1] == rust[1]
    return vanilla == rust


def omitted_on_save(key, rust):
    """Template keys that vanilla's codecs leave out when re-saving the loaded block entity."""
    if key == 'patterns' and rust[key] == []:
        return True  # BannerBlockEntity saves its patterns only when there are some
    # SignText saves filtered_messages only when some line differs from messages.
    return key == 'filtered_messages' and rust[key] == rust.get('messages')


def entity_matches(vanilla, rust):
    if vanilla is None:
        return False
    if rust is None:
        # A DUMMY pending entry in a proto chunk: vanilla creates the default block entity on demand.
        return nbt_matches(parse_nbt(vanilla[0])[0], {}, parse_nbt(vanilla[1])[0])
    return nbt_matches(parse_nbt(vanilla[0])[0], parse_nbt(rust[0])[0], parse_nbt(vanilla[1])[0])


def state_names():
    facts = json.loads((RESOURCES / 'facts.json').read_text())
    return [s[3].removeprefix('Block{').replace('}', '', 1) for s in facts['states']]


AIR = '0'
# A vault's config omits its default reward table when saved by vanilla; templates spell it out.
VAULT_REWARD = ('loot_table', ('str', 'minecraft:chests/trial_chambers/reward'))


def compare(vanilla, rust, names, label, limit=6, quiet=False):
    """Returns (differing chunks, differing blocks, differing biome cells, differing block entities)."""
    chunks = blocks = biomes = entities = 0
    for pos in sorted(vanilla):
        a, b = vanilla[pos], rust.get(pos)
        if b is None:
            print(f'  {label} chunk {pos}: missing from Horizon output')
            chunks += 1
            continue
        block_diffs, biome_diffs, entity_diffs = [], [], []
        for y in sorted(a['y']):
            ra, rb = a['y'][y], b['y'].get(y)
            if ra == rb:
                continue
            if rb is None:
                # Above the noise height (Nether roof, End sky) Horizon keeps no section; vanilla's is all air.
                if set(ra.split(' ')) != {AIR}:
                    block_diffs.append((pos[0] * 16, y, pos[1] * 16, 'non-air section', 'section missing'))
                continue
            for i, (u, v) in enumerate(zip(ra.split(' '), rb.split(' '))):
                if u != v:
                    block_diffs.append((pos[0] * 16 + i % 16, y, pos[1] * 16 + i // 16, names[int(u)], names[int(v)]))
        for q in sorted(a['q']):
            if q not in b['q']:
                continue  # biomes of sections Horizon does not generate (above the noise height)
            if a['q'][q] != b['q'].get(q):
                for i, (u, v) in enumerate(zip(a['q'][q].split(' '), b['q'].get(q, '').split(' '))):
                    if u != v:
                        biome_diffs.append((pos[0] * 4 + i % 4, q, pos[1] * 4 + i // 4, u, v))
        for key in sorted(set(a['be']) | set(b['be'])):
            if not entity_matches(a['be'].get(key), b['be'].get(key)):
                entity_diffs.append((key, a['be'].get(key, [None])[0], (b['be'].get(key) or [None])[0]))
        # Scheduled ticks run once the chunk is live (leaves at a tree's edge, lake fluids).
        if a.get('t', []) != b.get('t', []):
            ta, tb = a.get('t', []), b.get('t', [])
            first = next((i for i, (u, v) in enumerate(zip(ta, tb)) if u != v), min(len(ta), len(tb)))
            block_diffs.append((pos[0] * 16, 0, pos[1] * 16, f'{len(ta)} ticks, #{first}: {ta[first] if first < len(ta) else "-"}',
                                f'{len(tb)} ticks, #{first}: {tb[first] if first < len(tb) else "-"}'))
        # Postprocessing marks: their fluids tick and their blocks are reshaped once the chunk is live.
        if a.get('p', []) != b.get('p', []):
            pa, pb = a.get('p', []), b.get('p', [])
            first = next((i for i, (u, v) in enumerate(zip(pa, pb)) if u != v), min(len(pa), len(pb)))
            extra = Counter(pb) - Counter(pa)
            missing = Counter(pa) - Counter(pb)
            block_diffs.append((pos[0] * 16, 0, pos[1] * 16,
                                f'{len(pa)} postprocessing marks, #{first}: {pa[first] if first < len(pa) else "-"}'
                                f' ({sum(missing.values())} not in Horizon, e.g. {next(iter(missing), "-")})',
                                f'{len(pb)} marks, #{first}: {pb[first] if first < len(pb) else "-"}'
                                f' ({sum(extra.values())} not in vanilla, e.g. {next(iter(extra), "-")})'))
        if block_diffs or biome_diffs or entity_diffs:
            chunks += 1
            blocks += len(block_diffs)
            biomes += len(biome_diffs)
            entities += len(entity_diffs)
            if quiet:
                continue
            print(f'  {label} chunk {pos}: {len(block_diffs)} blocks, {len(biome_diffs)} biome cells, {len(entity_diffs)} block entities')
            for d in block_diffs[:limit]:
                print(f'    block {d[0]} {d[1]} {d[2]}: vanilla {d[3]} / horizon {d[4]}')
            for d in biome_diffs[:2]:
                print(f'    biome quart {d[0]} {d[1]} {d[2]}: vanilla {d[3]} / horizon {d[4]}')
            for d in entity_diffs[:3]:
                print(f'    block entity {d[0]}: vanilla {d[1]} / horizon {d[2]}')
    return chunks, blocks, biomes, entities


def scan(oracle, binary, seed, dimension, radius):
    out = build_dir() / f'scan-{dimension}-{seed}.txt'
    oracle.run(out, ['scan', seed, dimension, -radius, -radius, radius, radius])
    starts = []
    for line in out.read_text().splitlines():
        t = line.split()
        if t and t[0] == 'start':
            starts.append((t[1], int(t[2]), int(t[3]), tuple(map(int, t[4:10]))))
    return starts


def pick_structures(starts, per_kind, rng):
    by_kind = defaultdict(list)
    for s in starts:
        by_kind[s[0]].append(s)
    picked = []
    for kind, items in sorted(by_kind.items()):
        items.sort(key=lambda s: abs(s[1]) + abs(s[2]))
        # Nearest first, then random ones, so that variants (village types) differ.
        chosen = items[:1] + rng.sample(items[1:], min(per_kind - 1, len(items) - 1))
        picked.extend(chosen[:per_kind])
    return picked


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seeds', type=int, default=3)
    parser.add_argument('--seed', type=int, action='append', help='run only this seed (repeatable)')
    parser.add_argument('--per-structure', type=int, default=2)
    parser.add_argument('--sites', type=int, default=6, help='terrain sites per overworld seed')
    parser.add_argument('--radius', type=int, default=1, help='chunk radius around each site')
    parser.add_argument('--scan-radius', type=int, default=180)
    parser.add_argument('--limit', type=int, default=6, help='block differences shown per chunk')
    parser.add_argument('--summary', action='store_true', help='one line per differing site')
    parser.add_argument('--rect', action='append', help='compare only this x0,z0,x1,z1 chunk rectangle (repeatable)')
    parser.add_argument('--filter', help='regex over structure names')
    parser.add_argument('--dimension', action='append')
    parser.add_argument('--phase', choices=['terrain', 'structures', 'full'], default='full', help='stop before features, or run structure pieces only')
    parser.add_argument('--terrain-only', action='store_true', help='skip the structure sites')
    parser.add_argument('--structures-only', action='store_true')
    args = parser.parse_args()
    os.environ['HORIZON_PHASE'] = args.phase
    dimensions = args.dimension or ['overworld', 'the_nether', 'the_end']
    oracle = Oracle()
    binary = rust_binary()
    names = state_names()
    build = build_dir()
    totals = Counter()
    biome_seen = set()
    structure_seen = Counter()
    started = time.time()
    seeds = list(args.seed or SEEDS[:args.seeds])
    sampled = set(seeds)
    if not args.seed and not args.rect:
        seeds += [s for s, *_ in REGRESSIONS if s not in sampled]
    for seed in seeds:
        rng = random.Random(seed)
        for dimension in dimensions:
            rects, labels = [], []
            if args.rect:
                rects, labels = list(args.rect), [f'rect {r}' for r in args.rect]
            else:
                for s, d, rect, label in REGRESSIONS:
                    if (s, d) == (seed, dimension) and not (args.filter and not re.search(args.filter, label)):
                        rects.append(rect)
                        labels.append(label)
            r = args.radius
            if seed in sampled and not args.structures_only and not args.rect:
                spread = {'overworld': 5000, 'the_nether': 1500, 'the_end': 1800}[dimension]
                sites = [(0, 0)] + [(rng.randint(-spread, spread) // 16, rng.randint(-spread, spread) // 16) for _ in range(args.sites if dimension == 'overworld' else 2)]
                for cx, cz in sites:
                    rects.append(f'{cx - r},{cz - r},{cx + r},{cz + r}')
                    labels.append(f'terrain {cx},{cz}')
            if seed in sampled and not args.terrain_only and not args.rect:
                starts = scan(oracle, binary, seed, dimension, args.scan_radius if dimension == 'overworld' else 80)
                for kind, cx, cz, box in pick_structures(starts, args.per_structure, rng):
                    if args.filter and not re.search(args.filter, kind):
                        continue
                    rects.append(f'{cx - r},{cz - r},{cx + r},{cz + r}')
                    labels.append(f'{kind} {cx},{cz}')
                    structure_seen[kind] += 1
            if not rects:
                continue
            tag = f'{dimension}-{seed}-{args.phase}' + ('-iso' if os.environ.get('HORIZON_ISOLATED') else '') + ('-rect' if args.rect else '')
            vanilla_path, rust_path = build / f'vanilla-{tag}.txt', build / f'rust-{tag}.txt'
            t0 = time.time()
            # Vanilla and Horizon generate the same chunks independently: run them side by side.
            with ThreadPoolExecutor(2) as pool:
                rust_run = pool.submit(run_rust, binary, rust_path, seed, dimension, rects)
                if not (os.environ.get('REUSE_VANILLA') and vanilla_path.exists()):
                    oracle.run(vanilla_path, ['dump', seed, dimension, *rects])
                t1 = time.time()
                stderr = rust_run.result()
            t2 = time.time()
            vanilla, rust = parse(vanilla_path), parse(rust_path)
            for line in stderr.splitlines()[:5]:
                print('  horizon:', line)
            counts = [0, 0, 0, 0]
            for rect, label in zip(rects, labels):
                x0, z0, x1, z1 = map(int, rect.split(','))
                part = {k: v for k, v in vanilla.items() if x0 <= k[0] <= x1 and z0 <= k[1] <= z1}
                one = compare(part, rust, names, f'{tag} {label}', limit=args.limit, quiet=args.summary)
                if one[0]:
                    print(f'{tag} {label}: {one[0]} chunks differ ({one[1]} blocks, {one[2]} biome cells, {one[3]} block entities)')
                counts = [a + b for a, b in zip(counts, one)]
            totals.update(dict(zip(['chunks', 'blocks', 'biomes', 'entities'], counts)))
            totals['compared'] += len(vanilla)
            for chunk in vanilla.values():
                for row in chunk['q'].values():
                    biome_seen.update(row)
            print(f'{tag}: {len(rects)} sites, {len(vanilla)} chunks, differing chunks {counts[0]} (blocks {counts[1]}, biome cells {counts[2]}, block entities {counts[3]}); vanilla {t1 - t0:.0f}s, both sides {t2 - t0:.0f}s', flush=True)
    print(f'Compared {totals["compared"]} chunks across {len(biome_seen)} biomes and {sum(structure_seen.values())} structure sites in {time.time() - started:.0f}s')
    print('structure sites:', dict(sorted(structure_seen.items())))
    print(f'Differences: {totals["chunks"]} chunks, {totals["blocks"]} blocks, {totals["biomes"]} biome cells, {totals["entities"]} block entities')
    if totals['chunks']:
        sys.exit(1)


if __name__ == '__main__':
    main()
