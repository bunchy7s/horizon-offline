#!/usr/bin/env python3
"""Compare the creative inventory's tabs and search with the vanilla client's.

The Java oracle (VanillaCreativeSearchOracle) writes the creative-inventory.json
that horizon-prepare exports (VanillaCreativeTabsExporter: every
tab built by vanilla's CreativeModeTabs over the vanilla data pack's
registries), then what the vanilla client shows: every tab's display items
and, for each search query, CreativeModeInventoryScreen.refreshSearchResults'
items, searched through the client's own SessionSearchTrees.
examples/probe/creative_search_fixture loads the export into
horizon_client::creative::Creative. The tabs it shows (id, title, icon,
position, every stack with its components), the search tab's stacks and the
results of every query must agree. The queries are QUERIES below, then every
search stack's item path and tooltip lines, every item tag with and without
"#", and half of each as a prefix. Both run twice: as a player without
permissions, then as an operator with the Operator Items Tab option on
(CreativeModeInventoryScreen.hasPermissions), where the operator tab and its
stacks in the search tab appear.

  test_creative_parity.py [--detail N]
"""
import argparse
import json
import os
import subprocess
import sys

try:
    from . import interaction_lab as lib, lab, test_rust_parity as base, test_model_baking_parity as baking
except ImportError:
    import interaction_lab as lib
    import lab
    import test_rust_parity as base
    import test_model_baking_parity as baking

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
# Players' reports (anvil, crafting table, smithing table), other blocks of the
# functional tab, case, spaces, colons and tag searches.
QUERIES = ['', 'anvil', 'Anvil', 'ANVIL', 'crafting', 'crafting table', 'smithing', 'smithing table', 'table', 'chest',
           'furnace', 'bed', 'sign', 'banner', 'ominous', 'painting', 'loom', 'grindstone', 'lectern', 'stone', 'oak',
           'a', 'e', ' ', '  ', 'xyz', 'potion', 'healing', 'splash', 'sharpness', 'enchanted', 'book', 'mending',
           'goat horn', 'ponder', 'firework', 'flight', 'music disc', 'disc', 'spawn egg', 'egg', 'stew', 'arrow',
           'tipped', 'light gray', 'red ', ' red', 'minecraft:', 'minecraft:anvil', 'minecraft:table', 'mine:craft',
           ':anvil', ':', 'anvil:', 'minecraft: anvil', ' minecraft : anvil ', 'diamond_sword', 'diamond sword',
           'attack', 'when in main hand', '+', '-3', 'command', 'barrier', 'debug', 'light', 'structure', 'jigsaw',
           '#', '#logs', '#minecraft:logs', '#minecraft:', '#:planks', '#wool', '#LOGS', '#swords', '#anvil', '#xyz',
           '#minecraft', '#enchantable/', 'é', 'İ', 'Σ']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--detail', type=int, default=5)
    args = parser.parse_args()
    lock, jars, _ = base.verify_artifacts()
    libraries = baking.client_libraries()
    build = lab.LOCAL / 'creative-parity'; build.mkdir(parents=True, exist_ok=True)
    queries = build / 'queries.json'; queries.write_text(json.dumps(QUERIES))
    classes = build / 'classes'; classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['client'], *libraries]))

    def run(command, name):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log, stderr=subprocess.STDOUT)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:]); sys.exit(f'{name} failed')
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         *(PARITY / f'{n}.java' for n in ('ParityData', 'VanillaCreativeTabsExporter', 'VanillaCreativeSearchOracle'))],
        'compile-java')
    bad = 0
    for mode in ('', 'permissions'):
        print(f'== {mode or "no permissions"}')
        bad += compare(args, build, classes, dependencies, queries, run, mode)
    if bad:
        sys.exit(1)
    print('creative parity ok')


def compare(args, build, classes, dependencies, queries, run, mode):
    suffix = '-' + mode if mode else ''
    run(['java', '-Xmx2G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaCreativeSearchOracle',
         queries, build / 'creative-inventory.json', build / f'vanilla{suffix}.json', *([mode] if mode else [])],
        'vanilla' + suffix)
    lib.cargo_example('horizon-client', 'creative_search_fixture', lib.RESOURCES, build / 'creative-inventory.json',
                      build / f'vanilla{suffix}.json', build / f'rust{suffix}.json', *([mode] if mode else []))
    vanilla = json.loads((build / f'vanilla{suffix}.json').read_text())
    rust = json.loads((build / f'rust{suffix}.json').read_text())
    bad = 0
    shown = [{k: t[k] for k in ('id', 'title', 'icon', 'row', 'column', 'items')} for t in vanilla['tabs'] if t['shown']]
    if [t['id'] for t in shown] != [t['id'] for t in rust['tabs']]:
        bad += 1
        print('TABS vanilla', [t['id'] for t in shown], '\n     horizon', [t['id'] for t in rust['tabs']])
    for v, r in zip(shown, rust['tabs']):
        for k in ('title', 'icon', 'row', 'column'):
            if v[k] != r[k]:
                bad += 1; print(f'TAB {v["id"]} {k}: vanilla {v[k]!r} | horizon {r[k]!r}')
        if v['items'] != r['items']:
            bad += 1
            i = next((i for i, (a, b) in enumerate(zip(v['items'], r['items'])) if a != b), min(len(v['items']), len(r['items'])))
            print(f'TAB {v["id"]}: {len(v["items"])} vanilla stacks, {len(r["items"])} horizon; first difference at {i}:')
            print('   vanilla', v['items'][i] if i < len(v['items']) else None)
            print('   horizon', r['items'][i] if i < len(r['items']) else None)
        print(f'  {v["id"]}: {len(v["items"])} stacks')
    search = next(t['items'] for t in vanilla['tabs'] if t['type'] == 'SEARCH')
    if search != rust['search']:
        bad += 1; print(f'SEARCH TAB: {len(search)} vanilla stacks, {len(rust["search"])} horizon')
    wrong = [(v, r) for v, r in zip(vanilla['searches'], rust['searches']) if v != r]
    for v, r in wrong[:args.detail]:
        missing = [search[i] for i in v['results'] if i not in r['results']][:3]
        extra = [search[i] for i in r['results'] if i not in v['results']][:3]
        print(f'SEARCH {v["query"]!r}: vanilla {len(v["results"])} results, horizon {len(r["results"])}; '
              f'missing {missing} extra {extra}')
    bad += len(wrong) + abs(len(vanilla['searches']) - len(rust['searches']))
    for q in ('anvil', 'crafting', 'smithing'):
        hits = next(v['results'] for v in vanilla['searches'] if v['query'] == q)
        print(f'  search {q!r}: {len(hits)} results, ' + ', '.join(json.loads('"' + search[i].split('id:"')[1].split('"')[0] + '"')
                                                                      for i in hits[:6]))
    print(f'{len(vanilla["searches"]) - len(wrong)}/{len(vanilla["searches"])} searches match; {len(shown)} tabs, '
          f'{len(search)} search stacks')
    return bad


if __name__ == '__main__':
    main()
