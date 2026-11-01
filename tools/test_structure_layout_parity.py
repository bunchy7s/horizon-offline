#!/usr/bin/env python3
"""Compare Horizon's structure starts, piece by piece, with the starts a real server saved.

Expected side: the clean-room structure oracle (horizon-parity `cleanroom/structures/oracle/`), which ran
vanilla 1.21.1 servers and kept every saved start. Its `base_pieces` are the no-mod world's start of each
located structure (villages, outposts, trial chambers, shipwrecks, ruined portals, desert pyramids): the
vanilla layout. Its results live outside the repository; put them (or link them) under
`.local/structure-oracle/<profile>/seed-<seed>.json.gz`, or point HORIZON_STRUCTURE_ORACLE at that folder.

Horizon side: the structure_pieces probe generates the same seed and lists the starts each chunk owns.
Jigsaw pieces must match exactly: element (template location), rotation, position, ground level delta and
bounding box, in saved order. Other pieces (shipwrecks, ruined portals, desert pyramids) match by type and
horizontal box: their height is settled when they are placed, after the start is created, and the oracle
saved them placed.

The oracle's game is 1.21.1 and Horizon's 26.3. KNOWN lists the starts where 26.3 itself differs from
1.21.1, each checked against 26.3 vanilla (VanillaChunkGenerationOracle's `scan`, HORIZON_PIECES=1), whose
pieces equal Horizon's there; they are reported, not failed.

Mod profiles compare the mod world's `pieces` instead, through the path players' packs take: the mod jars
(`.local/structure-oracle/jars/`, or HORIZON_STRUCTURE_JARS) are carried by `horizon-port-pack carry` into
a fresh work folder, and an overlay of the 26.3 resources adds the carried structures, sets, pools,
processor lists, biome tags and templates. The jars are the player's; nothing of them enters the
repository. A profile whose oracle run added a data pack of the spec team's own (`lithostitched-probe`)
carries that pack as one more mod, built at run time from the oracle's profile file
(`.local/structure-oracle/profiles/<profile>.json`).

Lithostitched lays jigsaws out its own way (spec-lithostitched OPEN-L5) and Horizon keeps vanilla's
layouts, so the `lithostitched-probe` profile checks its pool elements' limits instead of the layout: in
every start Horizon builds, no `limited` or `guaranteed` element's template appears more often than its
limit, and how many starts hold one is printed beside the oracle's count.

The `yung-probe` profiles check YUNG's API's draws with a data pack of the spec team's own (spec-yungsapi
§3, probe experiments e1-e9: fallback, max_count, empty elements, min_required_depth, ignore_bounds,
max_y, is_priority). The pack is carried as one more mod, built at run time from the oracle's profile file
(`.local/structure-oracle/profiles/<profile>.json`), and its starts are read from the oracle's census of
every start (`all_starts`).

usage: test_structure_layout_parity.py [--profile lithostitched|betterdungeons|lithostitched-probe|yung-probe|yung-probe2]
                                       [--filter REGEX] [--verbose]
"""
import base64
import argparse
import gzip
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

ORACLE = Path(os.environ.get('HORIZON_STRUCTURE_ORACLE', lab.LOCAL / 'structure-oracle'))
JARS = Path(os.environ.get('HORIZON_STRUCTURE_JARS', ORACLE / 'jars'))
# Jars of a modpack's mods the oracle cache does not hold (Lithostitched, CTOV): a pack's jar folder.
PACK_JARS = Path(os.environ.get('HORIZON_STRUCTURE_PACK_JARS', ORACLE / 'pack-jars'))


def jar_path(jar):
    return JARS / jar if (JARS / jar).exists() else PACK_JARS / jar
# Per profile: which saved pieces are expected, the mods in the pack (id: jar), those carried, and the
# structures compared (a regex; None: every start that has the pieces).
PROFILES = {
    'lithostitched': {'side': 'base_pieces', 'mods': {}, 'carry': [], 'structures': None},
    'betterdungeons': {'side': 'pieces', 'mods': {'yungsapi': 'YungsApi-1.21.1-NeoForge-5.1.9.jar',
                                                  'betterdungeons': 'YungsBetterDungeons-1.21.1-NeoForge-5.1.4.jar'},
                       'carry': ['betterdungeons'], 'structures': r'^betterdungeons:(small|skeleton|zombie)_dungeon$'},
    # The probe pack's limited (limit 1) and guaranteed (count 1) elements on vanilla's plains houses.
    'lithostitched-probe': {'side': 'pieces', 'mods': {'lithostitched': 'lithostitched-1.8.1-neoforge-21.1.jar'},
                            'packs': {'probe': 'lithostitched-probe'}, 'carry': ['lithostitched', 'probe'],
                            'structures': r'^minecraft:village_plains$',
                            'limits': {'minecraft:village/taiga/houses/taiga_small_house_1': 1,
                                       'minecraft:village/snowy/houses/snowy_library_1': 1}},
    **{name: {'side': 'all_starts', 'mods': {'yungsapi': 'YungsApi-1.21.1-NeoForge-5.1.9.jar'},
              'packs': {'probe': name}, 'carry': ['probe'], 'structures': r'^probe:'}
       for name in ['yung-probe', 'yung-probe2']},
}
BUNDLES = {'structure': 'structures', 'structure_set': 'structure_sets', 'template_pool': 'template_pools',
           'processor_list': 'processor_lists'}
# (profile, seed, structure id, start chunk x, z) -> how 26.3 vanilla differs from 1.21.1 there.
TRIAL_CHAMBERS = '26.3 vanilla lays it out as Horizon does, unlike 1.21.1'
SULFUR_CAVES = "the start is in 26.3's sulfur_caves, a biome the mod's 1.21.1 tag cannot list"
RUINED_PORTAL = "26.3 vanilla's portal is Horizon's; 1.21.1 picked another"
KNOWN = {
    ('lithostitched', 1234, 'minecraft:village_plains', -24, 47): 'no start in 26.3 vanilla (nor in Horizon)',
    ('lithostitched', 1234, 'minecraft:ruined_portal', -16, 51): RUINED_PORTAL,
    ('lithostitched', 42, 'minecraft:ruined_portal', -19, -38): RUINED_PORTAL,
    ('betterdungeons', 1234, 'betterdungeons:small_dungeon', 162, 10): SULFUR_CAVES,
    ('betterdungeons', 42, 'betterdungeons:small_dungeon', -29, -28): SULFUR_CAVES,
    **{('lithostitched', seed, 'minecraft:trial_chambers', x, z): TRIAL_CHAMBERS
       for seed, x, z in [(1234, 13, 21), (2, -28, 13), (2, 11, 16), (2, 146, 44), (42, 17, 7), (42, 38, -25)]},
}


def element(saved):
    kind = saved.get('element_type')
    if kind in ('minecraft:single_pool_element', 'minecraft:legacy_single_pool_element', 'yungsapi:yung_single_element'):
        location = saved['location']
        return location if ':' in location else 'minecraft:' + location
    if kind == 'minecraft:feature_pool_element':
        return 'feature:' + saved['feature']
    if kind == 'minecraft:list_pool_element':
        return 'list:' + '+'.join(element(e) for e in saved['elements'])
    if kind == 'minecraft:empty_pool_element':
        return 'empty'
    return kind


def expected_piece(saved):
    box = saved['BB']
    piece = {'type': saved['id'], 'box': box if saved['id'] == 'minecraft:jigsaw' else [box[0], box[2], box[3], box[5]]}
    if saved['id'] == 'minecraft:jigsaw':
        piece.update(element=element(saved['pool_element']), rotation=saved['rotation'],
                     position=[saved['PosX'], saved['PosY'], saved['PosZ']], ground_level_delta=saved['ground_level_delta'])
    return piece


# horizon-port-pack's jar census (inventory.json); the carry does not read it.
CENSUS = ['blockstates', 'item_models', 'recipes', 'loot_tables', 'tags', 'worldgen', 'structures', 'languages',
          'textures', 'sounds', 'classes', 'mixin_configs']


def pack_jar(work, mod, profile):
    """The data pack an oracle profile added to its world, as a jar of mod `mod`."""
    spec = json.loads((ORACLE / 'profiles' / f'{profile}.json').read_text())
    files = {**spec['config_files'], **{path: base64.b64decode(data) for path, data in spec.get('binary_files', {}).items()}}
    jar = work / f'{mod}.jar'
    with zipfile.ZipFile(jar, 'w') as z:
        for path, content in files.items():
            parts = Path(path).parts
            if parts[:2] == ('world', 'datapacks') and len(parts) > 3:
                z.writestr(str(Path(*parts[3:])), content)
    return jar


def carry(profile, mods, carried, packs=None):
    """`horizon-port-pack carry` of the profile's mods and packs; the carried mods' data folders."""
    work = lab.LOCAL / 'structure-layout' / profile / 'work'
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    mods = {key: str(jar_path(jar)) for key, jar in mods.items()}
    mods.update({key: str(pack_jar(work, key, name)) for key, name in (packs or {}).items()})
    records = [{'key': key, 'name': key, 'version': '', 'path': jar, 'tier': 'data', 'confidence': 'known',
                'class': 'content' if key in carried else 'library', 'source': 'file', 'license': '',
                'client': '', 'server': '', 'depends': [], 'layer': 0, 'jar': jar,
                'info': {'mod_id': key, 'name': key, 'version': '', 'license': '', 'depends': [],
                         'census': dict.fromkeys(CENSUS, 0)}}
               for key, jar in mods.items()]
    (work / 'inventory.json').write_text(json.dumps(records))
    subprocess.run(['cargo', 'run', '--release', '-q', '-p', 'horizon-port-pack', '--', 'carry', str(work),
                    '--resources', str(lab.RESOURCES), *carried], check=True, cwd=lab.ROOT, stdout=subprocess.DEVNULL)
    data = [work / 'mods' / key / 'carry' / 'data' for key in carried]
    if not all(d.is_dir() for d in data):
        raise SystemExit(f'horizon-port-pack carried nothing into {work / "mods"}')
    return data


def overlay(profile, data_dirs):
    """The 26.3 resources plus the carried structure data, in a folder the probe reads as its resources."""
    root = lab.LOCAL / 'structure-layout' / profile / 'resources'
    shutil.rmtree(root, ignore_errors=True)
    worldgen = root / 'worldgen'
    (worldgen / 'structure').mkdir(parents=True)
    (root / 'facts.json').symlink_to(lab.RESOURCES / 'facts.json')
    source = lab.RESOURCES / 'worldgen'
    merged = {name: json.loads((source / f'{name}.json').read_text()) for name in [*BUNDLES.values(), 'biome_tags']}
    additions = []
    for entry in source.iterdir():
        if entry.name != 'structure' and entry.stem not in merged:
            (worldgen / entry.name).symlink_to(entry)
    for namespace in (source / 'structure').iterdir():
        (worldgen / 'structure' / namespace.name).symlink_to(namespace)
    for data in data_dirs:
        for path in sorted(p for p in data.rglob('*') if p.is_file()):
            parts = path.relative_to(data).parts
            if len(parts) < 3:
                continue
            namespace, kind = parts[0], parts[1]
            if kind == 'structure' and path.suffix == '.nbt':
                target = worldgen / 'structure' / namespace / Path(*parts[2:])
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(path)
            elif path.suffix == '.json' and kind == 'worldgen' and parts[2] in BUNDLES:
                text = path.read_text()
                if 'horizon:add_to' in text:
                    value = json.loads(text)
                    additions.append((value.get('horizon:priority', 1000), BUNDLES[parts[2]], value))
                else:
                    merged[BUNDLES[parts[2]]][f'{namespace}:' + '/'.join(parts[3:])[:-5]] = text
            elif path.suffix == '.json' and parts[1:4] == ('tags', 'worldgen', 'biome'):
                tag = f'{namespace}:' + '/'.join(parts[4:])[:-5]
                value = json.loads(path.read_text())
                if tag in merged['biome_tags'] and not value.get('replace'):
                    value['values'] = json.loads(merged['biome_tags'][tag])['values'] + value['values']
                merged['biome_tags'][tag] = json.dumps(value)
    # A carried file with `horizon:add_to` adds its entries to other sets, pools or processor lists, by
    # priority, as the game's mod_content.rs merges them.
    lists = {'structure_sets': 'structures', 'template_pools': 'elements', 'processor_lists': 'processors'}
    for _, bundle, addition in sorted(additions, key=lambda a: a[0]):
        targets = addition['horizon:add_to']
        key = lists[bundle]
        for target in [targets] if isinstance(targets, str) else targets:
            if target in merged[bundle]:
                value = json.loads(merged[bundle][target])
                value[key] = value.get(key, []) + addition.get(key, [])
                merged[bundle][target] = json.dumps(value)
    for name, bundle in merged.items():
        (worldgen / f'{name}.json').write_text(json.dumps(bundle))
    return root


def probe_binary():
    subprocess.run(['cargo', 'build', '--release', '-q', '-p', 'horizon-world', '--example', 'world_probe'], check=True, cwd=lab.ROOT)
    name = 'world_probe.exe' if os.name == 'nt' else 'world_probe'
    return lab.cargo_target() / 'release/examples' / name


def horizon_starts(binary, resources, seed, dimension, chunks):
    result = subprocess.run([str(binary), 'structure_pieces', str(resources), str(seed), dimension,
                             *(f'{x},{z}' for x, z in chunks)], capture_output=True, text=True, cwd=lab.ROOT)
    if result.returncode:
        raise SystemExit('structure_pieces failed: ' + result.stderr[-2000:])
    starts, current = {}, None
    for line in result.stdout.splitlines():
        fields = line.split()
        if fields[0] == 'start':
            current = starts.setdefault((fields[1], int(fields[2]), int(fields[3])), [])
        elif fields[0] == 'piece':
            box = list(map(int, fields[8:14]))
            piece = {'type': fields[1], 'box': box if fields[1] == 'minecraft:jigsaw' else [box[0], box[2], box[3], box[5]]}
            if fields[1] == 'minecraft:jigsaw':
                piece.update(element=fields[2], rotation=fields[3], position=list(map(int, fields[4:7])),
                             ground_level_delta=int(fields[7]))
            current.append(piece)
    return starts


def compare(expected, actual):
    """Index of the first differing piece and what differs, or None."""
    for i, (want, got) in enumerate(zip(expected, actual)):
        if want != got:
            return i, {k: (want.get(k), got.get(k)) for k in want.keys() | got.keys() if want.get(k) != got.get(k)}
    if len(expected) != len(actual):
        return min(len(expected), len(actual)), {'pieces': (len(expected), len(actual))}
    return None


def check_limits(binary, resources, results, profile, pattern):
    """Each limited template at most its limit in every start Horizon builds; starts holding it, Horizon
    beside the oracle."""
    limits, failed = profile['limits'], []
    holding = {template: [0, 0] for template in limits}
    starts = 0
    for path in results:
        oracle = json.load(gzip.open(path))
        seed = oracle['seed']
        wanted = [s for s in oracle['structures'] if s.get('pieces') and re.search(pattern, s['id'])
                  and re.search(profile['structures'], s['id'])]
        chunks = sorted({tuple(s['start_chunk']) for s in wanted})
        actual = horizon_starts(binary, resources, seed, 'overworld', chunks)
        for start in wanted:
            got = actual.get((start['id'], *start['start_chunk']))
            if got is None:
                print(f'skip  seed {seed} {start["id"]} chunk {start["start_chunk"]}: no start in Horizon')
                continue
            starts += 1
            for template, limit in limits.items():
                have = sum(p.get('element') == template for p in got)
                want = sum(element(p['pool_element'].get('delegate', p['pool_element'])) == template
                           for p in start['pieces'] if p['id'] == 'minecraft:jigsaw')
                holding[template][0] += have > 0
                holding[template][1] += want > 0
                if have > limit:
                    failed.append(f'seed {seed} {start["id"]} chunk {start["start_chunk"]}: {have} {template}')
    for line in failed:
        print(f'FAIL  {line} (over its limit)')
    for template, (ours, theirs) in holding.items():
        print(f'{template} (limit {limits[template]}): in {ours} of {starts} Horizon starts; the oracle: {theirs}')
    return 1 if failed or not starts else 0


def census_starts(oracle):
    """The oracle's census of every start (`all_starts`), as `structures` entries with their `all_starts`."""
    for chunk in oracle['all_starts']:
        for start in chunk['starts']:
            if 'children' in start:
                yield {'id': start['id'], 'dimension': chunk['dimension'], 'start_chunk': chunk['chunk'],
                       'all_starts': start['children']}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--profile', default='lithostitched', choices=sorted(PROFILES))
    parser.add_argument('--filter', default='', help='only structure ids matching this regex')
    parser.add_argument('--verbose', action='store_true')
    args = parser.parse_args()
    results = sorted((ORACLE / args.profile).glob('seed-*.json.gz'))
    if not results:
        print(f'SKIP: no oracle results in {ORACLE / args.profile} (see this file\'s docstring)')
        return 0
    profile = PROFILES[args.profile]
    missing = [jar for jar in profile['mods'].values() if not jar_path(jar).exists()]
    missing += [str(ORACLE / 'profiles' / f'{name}.json') for name in profile.get('packs', {}).values()
                if not (ORACLE / 'profiles' / f'{name}.json').exists()]
    if missing:
        print(f'SKIP: not found: {", ".join(missing)}')
        return 0
    resources = lab.RESOURCES
    if profile['carry']:
        data = carry(args.profile, profile['mods'], profile['carry'], profile.get('packs'))
        resources = overlay(args.profile, data)
    binary = probe_binary()
    if 'limits' in profile:
        return check_limits(binary, resources, results, profile, args.filter)
    matched, known, failed = 0, 0, []
    for path in results:
        oracle = json.load(gzip.open(path))
        seed = oracle['seed']
        by_dimension = {}
        for start in oracle['structures'] if profile['side'] != 'all_starts' else census_starts(oracle):
            if start.get(profile['side']) is None or not re.search(args.filter, start['id']) \
                    or not re.search(profile['structures'] or '', start['id']):
                continue
            by_dimension.setdefault(start['dimension'], []).append(start)
        for dimension, starts in by_dimension.items():
            chunks = sorted({tuple(s['start_chunk']) for s in starts})
            actual = horizon_starts(binary, resources, seed, dimension.removeprefix('minecraft:'), chunks)
            for start in starts:
                key = (start['id'], *start['start_chunk'])
                expected = [expected_piece(p) for p in start[profile['side']]]
                got = actual.get(key)
                label = f'seed {seed} {start["id"]} chunk {start["start_chunk"]}'
                difference = (0, {'start': 'missing in Horizon'}) if got is None else compare(expected, got)
                reason = KNOWN.get((args.profile, seed, *key))
                if difference is None:
                    matched += 1
                    if args.verbose:
                        print(f'ok    {label}: {len(expected)} pieces')
                elif reason:
                    known += 1
                    print(f'known {label}: {reason}')
                else:
                    failed.append(label)
                    index, fields = difference
                    print(f'FAIL  {label}: {len(expected)} pieces expected, {len(got or [])} placed; piece {index}: {fields}')
    total = matched + known + len(failed)
    print(f'{matched} of {total} starts match the oracle piece for piece; {known} known version differences')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
