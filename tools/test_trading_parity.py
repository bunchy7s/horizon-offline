#!/usr/bin/env python3
"""Compare villager and wandering trader trading with vanilla.

Java (VanillaTradingOracle) runs vanilla's villagers, wandering traders and
MerchantMenu on a real flat server level; Rust (trading_fixture) runs
horizon_client::merchant and the merchant kind of horizon_client::menu on the
same cases:

- gen: offers of every profession at every level and villager type, and of
  wandering traders, generated from the world's trade set random sequences
  (several merchants per world share them), as saved (Offers NBT), then the
  next draw of every sequence;
- price: Villager.updateSpecialPrices under gossip (reputation) and Hero of
  the Village, with uses and demand;
- restock: Villager.shouldRestock / restock over simulated game times and days
  (twice a day, demand catch-up);
- trade: opening the trading screen, selecting offers (tryMoveItems), clicks
  on the payment and result slots (pickup, shift-click, swap), the TRADE
  reputation event, closing: the menu and inventory, the villager's experience,
  level-ups with their new offers and regeneration, offer uses and prices, the
  experience orbs and the merchant's random;
- nbt: villagers and wandering traders saved by vanilla, read and written back
  (Offers, Xp, VillagerData, LastRestock, RestocksToday).

Every line of both outputs must match.
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

PROFESSIONS = ['armorer', 'butcher', 'cartographer', 'cleric', 'farmer', 'fisherman', 'fletcher', 'leatherworker',
               'librarian', 'mason', 'shepherd', 'toolsmith', 'weaponsmith']
TYPES = ['desert', 'jungle', 'plains', 'savanna', 'snow', 'swamp', 'taiga']
GOSSIP = ['major_negative', 'minor_negative', 'minor_positive', 'major_positive', 'trading']


def wanted(resources):
    """Items each profession's trades ask for (any level), and the wandering trader's."""
    root = resources / 'data/minecraft/villager_trade'
    out = {}
    for d in sorted(root.iterdir()):
        items = set()
        for f in d.rglob('*.json'):
            t = json.loads(f.read_text())
            for k in ('wants', 'additional_wants'):
                if k in t and 'components' not in t[k]:
                    items.add(t[k]['id'])
        out[d.name] = sorted(items)
    return out


def spec(rng, level=None):
    prof = rng.choice(PROFESSIONS + ['none', 'nitwit'] if rng.random() < 0.05 else PROFESSIONS)
    return f'{prof}:{rng.choice(TYPES)}:{level or rng.randint(1, 5)}'


def corpus(wants):
    rng = random.Random(0x5452414445)
    cases = []
    seeds = [0, 1, -1, 12345, -9223372036854775808, 9223372036854775807]
    n = 0
    # Every profession, type and level, alone in its world.
    for prof in PROFESSIONS:
        for t in TYPES:
            for seed in seeds[:3] + [rng.randrange(-2**63, 2**63)]:
                cases.append(f'gen g{n} seed={seed} v={prof}:{t}:5')
                n += 1
    for k in range(int(os.environ.get('TRADING_GEN', '1500'))):
        merchants = [('wt' if rng.random() < 0.15 else spec(rng)) for _ in range(rng.randint(1, 6))]
        cases.append(f'gen g{n} seed={rng.randrange(-2**63, 2**63)} ' + ' '.join(f'v={m}' for m in merchants))
        n += 1
    for k in range(int(os.environ.get('TRADING_PRICE', '1000'))):
        kv = [f'seed={rng.randrange(-2**63, 2**63)}', f'v={spec(rng)}']
        for _ in range(rng.choice([0, 1, 1, 2, 3])):
            kv.append(f'gossip={rng.choice(GOSSIP)}:{rng.choice([1, 5, 20, 25, 50, 100, rng.randint(1, 200)])}')
        if rng.random() < 0.4:
            kv.append(f'hero={rng.choice([0, 0, 1, 2, 4, 9])}')
        for _ in range(rng.choice([0, 0, 1, 2])):
            kv.append(f'uses={rng.randrange(4)}:{rng.randint(1, 16)}')
        for _ in range(rng.choice([0, 0, 1, 2])):
            kv.append(f'demand={rng.randrange(4)}:{rng.randint(-20, 40)}')
        cases.append(f'price p{k} ' + ' '.join(kv))
    for k in range(int(os.environ.get('TRADING_RESTOCK', '600'))):
        steps, time, day = [], rng.choice([0, 100, 24000, 500000]), rng.randint(0, 3)
        for _ in range(rng.randint(3, 14)):
            if rng.random() < 0.45:
                steps.append(f'use:{rng.randrange(2)}:{rng.randint(1, 12)}')
            else:
                time += rng.choice([1, 100, 2399, 2401, 3000, 6000, 12000, 12001, 24000, 30000])
                if rng.random() < 0.4:
                    day += rng.choice([1, 1, 2])
                steps.append(f'check:{time}:{day}')
        kv = [f'seed={rng.randrange(-2**63, 2**63)}', f'v={spec(rng)}', f'steps={",".join(steps)}']
        if rng.random() < 0.3:
            kv.append(f'restocks={rng.randrange(3)}')
        if rng.random() < 0.3:
            kv.append(f'lastrestock={rng.randrange(0, 40000)}')
        cases.append(f'restock r{k} ' + ' '.join(kv))
    for k in range(int(os.environ.get('TRADING_TRADE', '1500'))):
        wt = rng.random() < 0.12
        m = 'wt' if wt else spec(rng, rng.choice([1, 1, 2, 3, 4, 5]))
        prof = 'wandering_trader' if wt else m.split(':')[0]
        pool = wants.get(prof, []) + ['minecraft:emerald'] * 4
        inv, used = [], set()
        for _ in range(rng.randint(2, 10)):
            slot = rng.randrange(36)
            if slot in used or slot == 8:
                continue
            used.add(slot)
            item = rng.choice(pool)
            inv.append(f'{slot}:{item}*{rng.choice([1, 5, 16, 32, 64, 64, rng.randint(1, 64)])}')
        steps = []
        for _ in range(rng.randint(2, 12)):
            r = rng.random()
            if r < 0.3:
                steps.append(f'select:{rng.randrange(8)}')
            elif r < 0.45:
                steps.append('click:2:0:quick_move')
            elif r < 0.55:
                steps.append('click:2:0:pickup')
            elif r < 0.65:
                steps.append(f'click:{rng.randrange(3, 39)}:0:pickup')
            elif r < 0.72:
                steps.append(f'click:{rng.choice([0, 1])}:{rng.choice([0, 1])}:pickup')
            elif r < 0.78:
                steps.append(f'click:{rng.choice([0, 1, 2])}:0:quick_move')
            elif r < 0.82:
                steps.append(f'click:{rng.choice([0, 1, 2, rng.randrange(3, 39)])}:{rng.randrange(9)}:swap')
            elif r < 0.92:
                steps.append('tick')
            else:
                steps.append(f'click:{rng.randrange(3, 39)}:0:quick_move')
        if rng.random() < 0.3:
            steps.append('close')
        if prof in ('none', 'nitwit'):
            # No offers: the screen stays shut (the player's own menu is not modelled).
            steps = []
        kv = [f'seed={rng.randrange(-2**63, 2**63)}', f'vseed={rng.randrange(-2**47, 2**47)}', f'v={m}',
              f'inv={",".join(inv)}' if inv else 'inv=0:minecraft:dirt*1', f'steps={",".join(steps)}']
        if not wt and rng.random() < 0.4:
            level = int(m.split(':')[2])
            lo, hi = [0, 10, 70, 150, 250, 250][level - 1], [10, 70, 150, 250, 250, 260][level - 1]
            kv.append(f'xp={rng.choice([lo, max(lo, hi - 1), max(lo, hi - 5), rng.randint(lo, hi)])}')
        if not wt and rng.random() < 0.3:
            kv.append(f'gossip={rng.choice(GOSSIP)}:{rng.randint(1, 100)}')
        if rng.random() < 0.15:
            kv.append(f'hero={rng.randrange(3)}')
        cases.append(f'trade t{k} ' + ' '.join(kv))
    for k in range(int(os.environ.get('TRADING_NBT', '400'))):
        m = 'wt' if rng.random() < 0.1 else spec(rng)
        kv = [f'seed={rng.randrange(-2**63, 2**63)}', f'v={m}']
        if m != 'wt':
            for _ in range(rng.choice([0, 1, 2])):
                kv.append(f'uses={rng.randrange(2)}:{rng.randint(1, 16)}')
            if rng.random() < 0.5:
                kv.append(f'demand=0:{rng.randint(-10, 30)}')
            if rng.random() < 0.5:
                kv.append(f'xp={rng.randint(0, 260)}')
            if rng.random() < 0.5:
                kv.append(f'lastrestock={rng.randrange(0, 10**6)}')
                kv.append(f'restocks={rng.randrange(3)}')
        cases.append(f'nbt n{k} ' + ' '.join(kv))
    # The restock gaps exactly: the second restock of a day needs more than
    # 2400 ticks since the last one and a third never happens, while a new day
    # needs more than 12000; and the cheapest offers under a major-positive
    # reputation with Hero of the Village, where the discount cannot take a
    # price below one item.
    cases.append('restock b0 seed=3 v=farmer:plains:1 lastrestock=100 restocks=0 '
                 'steps=use:0:4,check:2400:0,check:2401:0,use:0:1,check:2500:0,use:0:1,check:2520:0')
    cases.append('restock b1 seed=3 v=farmer:plains:1 lastrestock=100 restocks=2 '
                 'steps=use:0:4,check:3000:0,use:0:1,check:12100:0,check:12101:0')
    cases.append('restock b2 seed=3 v=librarian:plains:3 lastrestock=0 restocks=2 '
                 'steps=use:1:6,check:12000:1,check:12001:1,use:1:1,check:12600:1,check:24001:2')
    cases.append('price b3 seed=5 v=fisherman:plains:1 gossip=major_positive:100 hero=4')
    cases.append('price b4 seed=5 v=shepherd:plains:1 gossip=major_positive:100 gossip=major_positive:100 '
                 'hero=1 uses=0:3 uses=1:3')
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prepare', action='store_true')
    parser.add_argument('--java-only', action='store_true', help='run the oracle and stop')
    args = parser.parse_args()
    build = lab.LOCAL / 'rust-trading-parity'
    build.mkdir(parents=True, exist_ok=True)
    if args.prepare:
        lab.prepare()
    lock, jars, libraries = base.verify_artifacts()
    resources = lab.RESOURCES
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaTradingOracle.java'
    sources = [oracle, Path(__file__), lab.ROOT / 'crates/horizon-client/examples/probe/trading_fixture.rs',
               lab.ROOT / 'crates/horizon-client/src/merchant.rs', lab.ROOT / 'crates/horizon-client/src/menu.rs']
    tested = base.source_hashes([s for s in sources if s.exists()])
    inputs = build / 'inputs.txt'
    cases = corpus(wanted(resources))
    inputs.write_text(''.join(line + '\n' for line in cases), encoding='utf-8')
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    deps = os.pathsep.join(map(str, libraries))

    def run(command, name):
        path = build / (name + '.log')
        with path.open('w', encoding='utf-8') as log:
            try:
                base.run(list(map(str, command)), stdout=log, stderr=subprocess.STDOUT)
            except subprocess.CalledProcessError:
                log.flush()
                raise SystemExit(f'{name} failed:\n' + path.read_text(errors='replace')[-4000:])
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'),
         '-cp', str(jars['server']) + os.pathsep + deps, '-d', classes, oracle], 'compile-java')
    universe = build / 'universe'
    stamp = build / 'server.inputs'
    reuse = (os.environ.get('TRADING_REUSE') and stamp.exists() and (build / 'server.txt').exists()
             and stamp.read_text() == inputs.read_text())
    if not reuse:
        run(['java', '-Xmx2G', '-cp', os.pathsep.join([str(classes), str(jars['server']), deps]),
             'dev.horizonlab.parity.VanillaTradingOracle', inputs, build / 'server.txt', universe], 'server')
        stamp.write_text(inputs.read_text())
    vanilla = (build / 'server.txt').read_text().splitlines()
    errors = [l for l in vanilla if ' error ' in l]
    if errors:
        raise SystemExit(f'oracle errors ({len(errors)}): {errors[:5]}')
    if args.java_only:
        print(f'{len(vanilla)} oracle lines')
        return
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    binary = lab.cargo_target() / 'fast/examples/client_probe'
    run([binary, 'trading_fixture', resources, inputs, build / 'server.txt', build / 'rust.txt'], 'rust')
    rust = (build / 'rust.txt').read_text().splitlines()
    bad = [(a, b) for a, b in zip(vanilla, rust) if a != b]
    if bad or len(vanilla) != len(rust):
        report = '\n'.join(f'  vanilla {a[:1500]}\n  rust    {b[:1500]}' for a, b in bad[:10])
        raise SystemExit(f'{len(bad)} of {len(vanilla)} lines differ (rust {len(rust)} lines):\n{report}')
    base.ensure_sources_unchanged([s for s in sources if s.exists()], tested)
    kinds = {}
    for c in cases:
        kinds[c.split()[0]] = kinds.get(c.split()[0], 0) + 1
    lab.write_json(build / 'report.json', {
        'schema': 1, 'minecraft': lock['minecraft']['version'], 'platform': platform.platform(), 'cases': kinds,
        'lines': len(vanilla), 'sources_sha256': tested,
        'scope': 'AbstractVillager.addOffersFromTradeSet over the trade_set / villager_trade registries and their '
                 'random sequences, VillagerTrade.getOffer with its item modifiers, Villager.updateSpecialPrices, '
                 'shouldRestock / restock / catchUpDemand, MerchantMenu (tryMoveItems, the result slot, quick move), '
                 'notifyTrade / rewardTradeXp with level-ups, and the Offers / Xp / LastRestock / RestocksToday NBT.'})
    print(f'{len(cases):,} trading cases match vanilla ({len(vanilla):,} lines: {kinds})')


if __name__ == '__main__':
    main()
