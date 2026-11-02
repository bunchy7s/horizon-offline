#!/usr/bin/env python3
"""Compare the player's recipe book with vanilla's ServerRecipeBook.

Each case runs a sequence of operations on a fresh book: awarding recipes
(advancement rewards, /recipe give, crafting; special recipes are never
learnt, and the count is the displays added), taking them back, the client
having seen a highlighted recipe, the open and filter settings of the four
book types (through the serverbound settings packet's codec), and saving and
loading the player's recipeBook compound (with unknown ids dropped, and a
compound missing a list ignored). The Java oracle
(VanillaRecipeBookOracle) drives ServerRecipeBook and its codec;
examples/probe/recipe_book_fixture drives horizon_client::recipe_book. After each
operation the count, the known and highlighted recipes, the settings and
the saved compound are compared.
"""
import argparse
import json
import random
try:
    from . import interaction_lab as lib
except ImportError:
    import interaction_lab as lib

TYPES = ['CRAFTING', 'FURNACE', 'BLAST_FURNACE', 'SMOKER']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, default=5)
    parser.add_argument('--cases', type=int, default=1500)
    parser.add_argument('--limit', type=int, default=3)
    args = parser.parse_args()
    work = lib.work_dir('recipe-book-parity')
    vanilla = lib.Vanilla(work)
    vanilla.compile('ParityData', 'VanillaMenuOracle', 'VanillaRecipeBookOracle')
    ids = sorted('minecraft:' + p.stem for p in (lib.RESOURCES / 'data/minecraft/recipe').glob('*.json'))
    rng = random.Random(args.seed)
    cases = []
    # Every book and flag combination crosses the wire, then survives a save/load.
    for book_type in TYPES:
        for opened in (False, True):
            for filtering in (False, True):
                cases.append({'ops': [{'op': 'set', 'type': book_type, 'open': opened,
                                       'filtering': filtering}, {'op': 'save'}, {'op': 'load'}]})
    for _ in range(args.cases):
        ops, known = [], set()
        for _ in range(rng.randrange(1, 12)):
            kind = rng.choices(['give', 'take', 'seen', 'set', 'save', 'load'], [10, 4, 4, 3, 3, 2])[0]
            if kind in ('give', 'take'):
                pool = list(known) if kind == 'take' and known and rng.random() < 0.7 else ids
                chosen = ids if rng.random() < 0.03 else rng.sample(pool, min(len(pool), rng.choice([1, 1, 2, 5, 20])))
                if rng.random() < 0.2:
                    chosen = chosen + chosen[:1]
                ops.append({'op': kind, 'ids': chosen})
                if kind == 'give':
                    known.update(chosen)
            elif kind == 'seen':
                ops.append({'op': kind, 'id': rng.choice(list(known) or ids)})
            elif kind == 'set':
                ops.append({'op': kind, 'type': rng.choice(TYPES), 'open': rng.random() < 0.5, 'filtering': rng.random() < 0.5})
            elif kind == 'save':
                ops.append({'op': kind})
            else:
                op = {'op': kind}
                if rng.random() < 0.2:
                    op['drop'] = rng.choice(['recipes', 'toBeDisplayed'])
                ops.append(op)
        cases.append({'ops': ops})
    # An empty batch, a recipe the book never had, a highlight taken back
    # twice, and the settings of all four book types carried through a save
    # and a load: states the random batches leave out.
    cases.append({'ops': [{'op': 'give', 'ids': []}, {'op': 'take', 'ids': []}, {'op': 'save'}, {'op': 'load'}]})
    cases.append({'ops': [{'op': 'take', 'ids': ids[:4]}, {'op': 'seen', 'id': ids[0]},
                          {'op': 'seen', 'id': ids[0]}, {'op': 'save'}, {'op': 'load'}, {'op': 'seen', 'id': ids[1]}]})
    cases.append({'ops': [{'op': 'give', 'ids': ids[:6]},
                          *[{'op': 'set', 'type': t, 'open': o, 'filtering': f}
                            for t in TYPES for o, f in ((True, False), (False, True))],
                          {'op': 'save'}, {'op': 'load'}, {'op': 'save'}, {'op': 'load', 'drop': 'toBeDisplayed'}]})
    (work / 'cases.jsonl').write_text(''.join(json.dumps(c) + '\n' for c in cases))
    vanilla.run('VanillaRecipeBookOracle', work / 'cases.jsonl', work / 'native.txt')
    lib.cargo_example('horizon-client', 'recipe_book_fixture', lib.RESOURCES, work / 'cases.jsonl', work / 'rust.txt')

    def by_case(path):
        out, cur = [], []
        for line in path.read_text().splitlines():
            cur.append(line)
            if line == 'end' or line.startswith('error-case'):
                out.append(cur)
                cur = []
        return out
    native, rust = by_case(work / 'native.txt'), by_case(work / 'rust.txt')
    bad = {}
    for case, a, b in zip(cases, native, rust):
        if a != b:
            step = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            op = case['ops'][step]['op'] if step < len(case['ops']) else 'end'
            bad.setdefault(op, []).append((case, step, a, b))
    total = sum(len(v) for v in bad.values())
    for key, entries in sorted(bad.items(), key=lambda e: -len(e[1])):
        print(f'{key}: {len(entries)} differing')
        for case, step, a, b in entries[:args.limit]:
            print(f'  ops    {json.dumps(case["ops"][:step + 1])[:600]}')
            print(f'  native {a[step] if step < len(a) else "-"}\n  rust   {b[step] if step < len(b) else "-"}')
    errors = sum(1 for c in native if c and c[-1].startswith('error-case'))
    print(f'{len(cases) - total}/{len(cases)} cases match ({errors} oracle errors)')
    raise SystemExit(1 if total or errors or len(native) != len(cases) else 0)


if __name__ == '__main__':
    main()
