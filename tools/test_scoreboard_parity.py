#!/usr/bin/env python3
"""Compare Horizon's scoreboard model and scoreboard.dat with vanilla's Scoreboard.

The Java oracle (VanillaScoreboardOracle) runs vanilla's own Scoreboard class
(what ServerScoreboard extends) on scripted cases: objectives with every kind of
criteria (custom, team kill and statistics, spelled every way byName accepts),
display names, render types, auto-updated displays and number formats; scores
set, added, read, locked, given displays and formats, reset one by one or all
at once, and changed through forAllObjectives as the gameplay criteria are;
display slots; teams with every option, joined, left, removed and emptied by a
dead entity (Scoreboard.entityRemoved). Each case records every result, dumps
of each collection in vanilla's iteration order (objectivesByName and
playerScores as fastutil open hash maps, teamsByName, each team's HashSet of
players), and the ScoreboardSaveData.Packed codec output that
data/minecraft/scoreboard.dat holds, before and after loading it back.
`examples/probe/scoreboard_fixture` runs the same cases on
`horizon_client::scoreboard`, reloads vanilla's NBT, and vanilla reloads
Horizon's NBT; all must agree. The one order vanilla leaves to identity hash
codes (each holder's scores within PlayerScores) is compared as a set.

  test_scoreboard_parity.py [--count N] [--detail N]
"""
import argparse
import json
import os
import random
import subprocess
import sys
from pathlib import Path

try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'
COLORS = ['black', 'dark_blue', 'dark_green', 'dark_aqua', 'dark_red', 'dark_purple', 'gold', 'gray', 'dark_gray',
          'blue', 'green', 'aqua', 'red', 'light_purple', 'yellow', 'white']
CUSTOM = ['dummy', 'trigger', 'deathCount', 'playerKillCount', 'totalKillCount', 'health', 'food', 'air', 'armor',
          'xp', 'level']
SLOTS = ['list', 'sidebar', 'below_name'] + [f'sidebar.team.{c}' for c in COLORS]


def criteria_pool(registries):
    out = list(CUSTOM)
    out += [f'teamkill.{c}' for c in COLORS] + [f'killedByTeam.{c}' for c in COLORS]
    out += ['teamkill.pink', 'killedByTeam.', 'nonsense', 'DeathCount', 'minecraft.flown:minecraft.stone']
    for ty, ids in [('mined', ['stone', 'oak_log', 'diamond_ore']), ('crafted', ['stick', 'crafting_table']),
                    ('used', ['bow', 'diamond_pickaxe']), ('broken', ['iron_sword']), ('picked_up', ['dirt']),
                    ('dropped', ['cobblestone']), ('killed', ['zombie', 'creeper']), ('killed_by', ['skeleton']),
                    ('custom', ['jump', 'deaths', 'play_time', 'time_since_death'])]:
        for i in ids:
            out += [f'minecraft.{ty}:minecraft.{i}', f'{ty}:{i}', f'minecraft.{ty}:{i}', f'.{ty}:.{i}', f'{ty}:minecraft.{i}']
    out += [f'minecraft.custom:minecraft.{k.split(":")[1]}' for k in registries['custom_stats']]
    out += ['minecraft.mined:minecraft.not_a_block', 'minecraft.custom:minecraft.stone', 'minecraft.mined:Stone',
            'Minecraft.mined:stone', 'minecraft.killed:minecraft.player', 'minecraft.crafted:minecraft.air']
    return out


def components(rng, name=None):
    choices = [
        lambda: name or rng.choice(['A', 'Kills', '', 'Ünïcode ✓', 'x y']),
        lambda: {'text': name or rng.choice(['Hi', 'Score', ''])},
        lambda: {'text': rng.choice(['Red', 'B']), 'color': rng.choice(COLORS + ['#FF5555', '#00aa00', '#123456'])},
        lambda: {'text': 'Bold', 'bold': rng.random() < 0.7, 'italic': rng.random() < 0.3},
        lambda: {'translate': rng.choice(['gui.done', 'deathScreen.title']), 'color': 'gold'},
        lambda: {'translate': 'chat.square_brackets', 'with': [rng.choice(['x', {'text': 'y'}, {'text': 'z', 'color': 'red'}])]},
        lambda: [{'text': 'a', 'color': 'blue'}, 'b', {'text': 'c', 'underlined': True}],
        lambda: ['plain', 'list'],
        lambda: {'text': 'p', 'extra': [{'text': 'q', 'color': 'green'}, 'r']},
        lambda: {'type': 'text', 'text': 'typed'},
        lambda: {'keybind': 'key.jump'},
        lambda: {'text': 'strike', 'strikethrough': True, 'obfuscated': False},
        lambda: {'text': name or 'E', 'hover_event': {'action': 'show_entity', 'id': 'minecraft:pig',
                                                     'uuid': [rng.randint(-9, 9) for _ in range(4)]}},
        lambda: {'text': 'tip', 'hover_event': {'action': 'show_text', 'value': {'text': 'hi', 'color': 'red'}}},
    ]
    return rng.choice(choices)()


def number_format(rng):
    return rng.choice([
        None, None, {'type': 'minecraft:blank'}, {'type': 'blank'},
        {'type': 'minecraft:fixed', 'value': components(rng)},
        {'type': 'minecraft:styled', 'color': rng.choice(COLORS)},
        {'type': 'styled', 'bold': True, 'color': '#ABCDEF'},
        {'type': 'minecraft:styled'},
    ])


def holders(rng):
    base = ['Player', 'Alex', 'Steve', '#fake', '#hidden_total', 'Aa', 'BB', 'AaAa', 'BBBB', 'AaBB', '*x',
            '0b5a4a94-b0d1-4c66-8f56-0d1d8a3c5d10', 'Notch', 'jeb_', 'Dinnerbone', '§cRed', 'Ünï']
    return base + [f'p{i}' for i in range(rng.choice([4, 12, 40]))]


def cases(count, registries):
    rng = random.Random(0x53434f5245)
    crit = criteria_pool(registries)
    valid_crit = CUSTOM + [f'teamkill.{c}' for c in COLORS] + [f'killedByTeam.{c}' for c in COLORS] + [
        'minecraft.mined:minecraft.stone', 'mined:stone', 'minecraft.custom:minecraft.jump', 'killed:zombie']
    out = []
    for n in range(count):
        objectives = [f'o{i}' for i in range(rng.choice([3, 8, 20, 40]))] + ['Aa', 'BB', 'kills', 'hp']
        teams = [f't{i}' for i in range(rng.choice([2, 6, 30, 60]))] + ['Aa', 'BB', 'red']
        hs = holders(rng)
        ops = []
        if n < 12:
            # Every criteria name, each case a slice.
            for c in crit[n::12]:
                ops.append({'op': 'crit', 'name': c})
        if rng.random() < 0.15:
            # Bulk: enough teams and members to rehash teamsByName and resize the players' HashSets.
            for t in rng.sample(teams, len(teams)):
                ops.append({'op': 'team_add', 'name': t})
                if rng.random() < 0.2:
                    ops.append({'op': 'team_rm', 'name': rng.choice(teams)})
            for h in rng.sample(hs, len(hs)):
                ops.append({'op': 'join', 'holder': h, 'team': rng.choice(teams[:2])})
            ops.append({'op': 'dump'})
        length = rng.choice([20, 60, 150, 400])
        for _ in range(length):
            r = rng.random()
            h = rng.choice(hs)
            dname = components(rng, h) if rng.random() < 0.3 else None
            o = rng.choice(objectives)
            if r < 0.12:
                ops.append({'op': 'obj_add', 'name': o, 'criteria': rng.choice(crit if rng.random() < 0.3 else valid_crit),
                            'display': components(rng, o), 'render': rng.choice(['integer', 'hearts']),
                            'auto': rng.random() < 0.4, 'fmt': number_format(rng)})
            elif r < 0.15:
                ops.append({'op': 'obj_rm', 'name': o})
            elif r < 0.19:
                field = rng.choice(['display', 'render', 'auto', 'fmt'])
                value = {'display': components(rng), 'render': rng.choice(['integer', 'hearts']),
                         'auto': rng.random() < 0.5, 'fmt': number_format(rng)}[field]
                ops.append({'op': 'obj_mod', 'name': o, 'field': field, 'value': value})
            elif r < 0.22:
                ops.append({'op': 'slot', 'slot': rng.choice(SLOTS), 'name': None if rng.random() < 0.2 else o})
            elif r < 0.40:
                ops.append({'op': rng.choice(['set', 'add', 'set', 'add', 'get']), 'holder': h, 'name': o,
                            'value': rng.choice([0, 1, -1, 5, 100, 2147483647, -2147483648, rng.randint(-1000, 1000)]),
                            'force': rng.random() < 0.3, 'dname': dname})
            elif r < 0.46:
                kind = rng.choice(['lock', 'unlock', 'sdisplay', 'sfmt', 'info'])
                op = {'op': kind, 'holder': h, 'name': o}
                if kind == 'sdisplay':
                    op['value'] = None if rng.random() < 0.3 else components(rng)
                if kind == 'sfmt':
                    op['value'] = number_format(rng)
                ops.append(op)
            elif r < 0.52:
                ops.append({'op': 'reset', 'holder': h, 'name': None if rng.random() < 0.3 else o})
            elif r < 0.62:
                ops.append({'op': 'all', 'criteria': rng.choice(valid_crit), 'holder': h, 'dname': dname,
                            'kind': rng.choice(['inc', 'inc', 'add', 'set', 'reset']), 'value': rng.randint(-50, 50)})
            elif r < 0.68:
                ops.append({'op': 'team_add', 'name': rng.choice(teams)})
            elif r < 0.70:
                ops.append({'op': 'team_rm', 'name': rng.choice(teams)})
            elif r < 0.80:
                ops.append({'op': 'join', 'holder': h, 'team': rng.choice(teams)})
            elif r < 0.83:
                ops.append({'op': 'leave', 'holder': h})
            elif r < 0.91:
                field = rng.choice(['display', 'color', 'ff', 'si', 'prefix', 'suffix', 'nametag', 'death', 'collision'])
                vis = ['always', 'never', 'hideForOtherTeams', 'hideForOwnTeam']
                value = {'display': components(rng), 'color': rng.choice(COLORS + [None]), 'ff': rng.random() < 0.5,
                         'si': rng.random() < 0.5, 'prefix': None if rng.random() < 0.2 else components(rng),
                         'suffix': None if rng.random() < 0.2 else components(rng), 'nametag': rng.choice(vis),
                         'death': rng.choice(vis),
                         'collision': rng.choice(['always', 'never', 'pushOtherTeams', 'pushOwnTeam'])}[field]
                ops.append({'op': 'team_set', 'name': rng.choice(teams), 'field': field, 'value': value})
            elif r < 0.94:
                ops.append({'op': 'removed', 'holder': h})
            elif r < 0.97:
                ops.append({'op': 'allied', 'a': h, 'b': rng.choice(hs)})
            else:
                ops.append({'op': 'dump'})
        ops.append({'op': 'dump'})
        out.append({'case': n, 'probes': hs, 'ops': ops})
    return out


def normalize(pack):
    """PlayerScores keeps each holder's scores in an identity-hash map: sort within a holder."""
    if isinstance(pack, dict) and 'PlayerScores' in pack:
        rows = pack['PlayerScores']
        order = []
        groups = {}
        for row in rows:
            name = row.get('Name')
            if name not in groups:
                order.append(name)
                groups[name] = []
            groups[name].append(row)
        pack = dict(pack, PlayerScores=[r for name in order for r in sorted(groups[name], key=json.dumps)])
    return pack


def first_difference(a, b, path=''):
    if type(a) is not type(b):
        return path, a, b
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                return f'{path}.{k}', a.get(k, '<absent>'), b.get(k, '<absent>')
            d = first_difference(a[k], b[k], f'{path}.{k}')
            if d:
                return d
        return None
    if isinstance(a, list):
        if len(a) != len(b):
            return f'{path} (length {len(a)} vs {len(b)})', a, b
        for i, (x, y) in enumerate(zip(a, b)):
            d = first_difference(x, y, f'{path}[{i}]')
            if d:
                return d
        return None
    return None if a == b else (path, a, b)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--count', type=int, default=500)
    parser.add_argument('--detail', type=int, default=4)
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'scoreboard-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))

    def run(command, name, **kw):
        with (build / (name + '.log')).open('w') as log:
            result = subprocess.run(list(map(str, command)), cwd=lab.ROOT, timeout=3600, stdout=log,
                                    stderr=subprocess.STDOUT, **kw)
        if result.returncode:
            print((build / (name + '.log')).read_text()[-4000:])
            sys.exit(f'{name} failed')

    oracle = ['java', '-Xmx1G', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaScoreboardOracle']
    run(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies, '-d', classes,
         PARITY / 'VanillaScoreboardOracle.java'], 'compile-java')
    run([*oracle, 'export', build], 'export')
    registries = json.loads((build / 'registries.json').read_text())
    chosen = cases(args.count, registries)
    inputs = build / 'inputs.jsonl'
    inputs.write_text(''.join(json.dumps(c, ensure_ascii=False) + '\n' for c in chosen), encoding='utf-8')
    run([*oracle, 'run', inputs, build / 'vanilla.jsonl'], 'vanilla')
    run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client-lib', '--example', 'client_probe'], 'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast/examples' / ('client_probe.exe' if os.name=='nt' else 'client_probe')
    run([binary, 'scoreboard_fixture',build / 'registries.json', inputs, build / 'rust.jsonl', build / 'vanilla.jsonl'], 'rust')
    read = lambda p: [json.loads(l) for l in p.read_text(encoding='utf-8').splitlines()]
    vanilla, rust = read(build / 'vanilla.jsonl'), read(build / 'rust.jsonl')
    assert len(vanilla) == len(rust)
    (build / 'horizon-nbt.jsonl').write_text(''.join(json.dumps({'case': r['case'], 'nbt': r['nbt']}) + '\n' for r in rust))
    run([*oracle, 'reload', build / 'horizon-nbt.jsonl', build / 'vanilla-reload.jsonl'], 'vanilla-reload')
    reloads = {r['case']: r for r in read(build / 'vanilla-reload.jsonl')}
    bad, errors, ops = [], [], 0
    for v, r in zip(vanilla, rust):
        if 'error' in v:
            errors.append((v['case'], v['error']))
            continue
        ops += len(v['out'])
        checks = [('results', v['out'], r['out']), ('pack', normalize(v['pack']), normalize(r['pack'])),
                  ('reload', normalize(v['reload']), normalize(r['reload'])), ('reload_dump', v['reload_dump'], r['reload_dump']),
                  # Loading inserts in the saved order, so the maps may iterate differently than before saving.
                  ('horizon loads vanilla NBT', normalize(v['reload']), normalize(r.get('vreload'))),
                  ('vanilla loads horizon NBT', normalize(v['reload']), normalize(reloads.get(v['case'], {}).get('pack')))]
        for what, a, b in checks:
            d = first_difference(a, b)
            if d:
                bad.append((v['case'], what, d))
                break
    for case, e in errors[:5]:
        print('ORACLE ERROR', case, e)
    ops_of = {c['case']: c['ops'] for c in chosen}
    for case, what, (path, a, b) in bad[:args.detail]:
        print(f'MISMATCH case {case} in {what} at {path}')
        if what == 'results' and path.startswith('['):
            i = int(path[1:path.index(']')])
            print('   op', json.dumps(ops_of[case][i], ensure_ascii=False)[:400])
        print('   vanilla', json.dumps(a, ensure_ascii=False)[:600])
        print('   horizon', json.dumps(b, ensure_ascii=False)[:600])
    print(f'{len(vanilla) - len(bad) - len(errors)}/{len(vanilla)} scoreboard cases match ({ops} operations); '
          f'{len(errors)} oracle errors')
    if bad or errors:
        sys.exit(1)
    print('scoreboard parity ok')


if __name__ == '__main__':
    main()
