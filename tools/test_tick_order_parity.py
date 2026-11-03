#!/usr/bin/env python3
"""Where the order entities join the world changes what they do, on a dedicated server, against vanilla's (#6509).

Vanilla ticks the entities of a level in the order they joined, whatever their kind. Horizon ticks one kind after
another (boats and minecarts first, then bolts, bobbers, tridents, rockets, items, arrows, mobs, orbs, falling blocks,
TNT). #6509 says to change the cross-kind order only once a harness shows where vanilla differs; this is that harness,
and it changes nothing. It measures, it does not fix.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve the same flat world with
nobody on it. Every case is a small cell of two kinds, 10 blocks or more from the next cell and inside force-loaded
(ticking) chunks, summoned by console in one order and then the other (`a-first`, `b-first`: the ids rise with the join).
Ticking is frozen and stepped one tick at a time; after each tick the Pos and Motion (and Health, fuse, orb value,
item count where the kind has one) of every entity of every cell are read. Families:

- wind: a wind charge bursts beside one entity of each kind `wind_late` covers;
- blast: primed TNT explodes beside one entity of each kind;
- hit: an arrow, snowball or trident flies head-on into a pig, boat or minecart that moves toward it;
- ride: a pig rides a moving boat, minecart or pig (`/ride`), the rider or the vehicle joined first;
- push: a boat, minecart or pig runs into a pig, boat, minecart or itself; two items or two orbs that merge.

Per cell the report gives: Horizon against vanilla (first tick that differs, values off, largest difference), and
whether the join order changes the result on vanilla itself (`order matters`) and on Horizon. A cell where vanilla's
result does not depend on the order is a control: Horizon must match it. KNOWN lists the cells where Horizon still
differs from vanilla and why: they are printed on every run so they stay visible, and do not fail the harness; a cell
that matches again is reported and should leave the list. A cell that differs and is not in KNOWN fails, and so does a
cell in which nothing moves on vanilla (it would match vacuously).
"""
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time

try:
    from . import lab, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
    from .test_locator_bar_parity import generate_world
except ImportError:
    import lab
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port
    from test_locator_bar_parity import generate_world

NAME = 'tick-order-parity'
TOLERANCE = 1e-6
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-seed': '12345',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'survival',
    'difficulty': 'normal',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'pause-when-empty-seconds': '-1',
    'spawn-monsters': 'false',
    'spawn-animals': 'false',
    'spawn-npcs': 'false',
}
# Cells where Horizon differs from vanilla on current main, and why. Filled from the measured run (see the pull request).
R_order_wind = (
    "vanilla's result depends on which joined first; Horizon ticks this kind in its own fixed place relative to the wind charge")
R_order_blast = (
    "vanilla's result depends on which joined first; Horizon ticks this kind in its own fixed place relative to the primed TNT")
R_order_other = (
    "vanilla's result depends on which joined first; Horizon's does not (it ticks by kind)")
R_same = (
    "differs the same in both orders, so not a join-order result (vanilla's does not depend on it): placement or damage, left as measured")
KNOWN = {
    'wind:wind_charge+trident:a-first': R_order_wind,
    'wind:wind_charge+firework_rocket:a-first': R_order_wind,
    'wind:wind_charge+arrow:a-first': R_order_wind,
    'wind:wind_charge+oak_boat:a-first': R_order_wind,
    'wind:wind_charge+minecart:a-first': R_order_wind,
    'wind:wind_charge+snowball:a-first': R_order_wind,
    'blast:tnt+trident:a-first': R_order_blast,
    'blast:tnt+firework_rocket:a-first': R_order_blast,
    'blast:tnt+arrow:a-first': R_order_blast,
    'blast:tnt+falling_block:a-first': R_order_blast,
    'blast:tnt+snowball:a-first': R_order_blast,
    'blast:tnt+pig:a-first': R_same,
    'hit:arrow+pig:a-first': R_order_other,
    'hit:arrow+oak_boat:a-first': R_same,
    'hit:snowball+pig:a-first': R_order_other,
    'hit:snowball+oak_boat:a-first': R_same,
    'hit:snowball+minecart:a-first': R_same,
    'hit:trident+pig:a-first': R_order_other,
    'hit:trident+oak_boat:a-first': R_order_other,
    'hit:trident+minecart:a-first': R_order_other,
    'ride:pig+oak_boat:a-first': R_same,
    'ride:pig+minecart:a-first': R_same,
    'ride:pig+pig:a-first': R_same,
    'push:pig+pig:a-first': R_order_other,
    'wind:wind_charge+tnt:b-first': R_order_wind,
    'wind:wind_charge+falling_block:b-first': R_order_wind,
    'wind:wind_charge+experience_orb:b-first': R_order_wind,
    'blast:tnt+pig:b-first': R_same,
    'hit:arrow+oak_boat:b-first': R_same,
    'hit:snowball+oak_boat:b-first': R_same,
    'hit:snowball+minecart:b-first': R_same,
    'hit:trident+pig:b-first': R_order_other,
    'hit:trident+oak_boat:b-first': R_order_other,
    'hit:trident+minecart:b-first': R_order_other,
    'ride:pig+oak_boat:b-first': R_same,
    'ride:pig+minecart:b-first': R_same,
    'ride:pig+pig:b-first': R_same,
    'push:pig+pig:b-first': R_order_other,
}

CELL = 10  # blocks between cell origins, 20 for the blasts' reach (see cell_origin)
PER_ROW = 10
STILL = 'NoGravity:1b'
EXTRA = {
    'falling_block': '{BlockState:{Name:"minecraft:sand"},Time:1,NoGravity:1b}',
    'firework_rocket': '{LifeTime:200,NoGravity:1b}',
    'tnt': '{fuse:200s,NoGravity:1b}',
    'pig': '{NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b}',
    'item': '{Item:{id:"minecraft:stone",count:1},NoGravity:1b,PickupDelay:32767s,Age:-32768s}',
    'experience_orb': '{NoGravity:1b,Value:1s,Age:-30000s}',
}
TARGETS = ['trident', 'firework_rocket', 'arrow', 'tnt', 'oak_boat', 'minecart', 'falling_block', 'experience_orb',
           'snowball', 'pig', 'item']


WALKER = '{NoGravity:1b,Silent:1b,PersistenceRequired:1b}'  # a pig that moves on its Motion (no NoAI)


def nbt(kind, motion=None, more=''):
    """The summon NBT of `kind` as `wind_late` has it, with a Motion and extra fields if wanted."""
    base_nbt = WALKER if (kind == 'pig' and motion) else EXTRA.get(kind, '{' + STILL + '}')
    fields = base_nbt[1:-1]
    if more:
        fields += ',' + more
    if motion:
        fields += ',Motion:[%sd,%sd,%sd]' % motion
    return '{' + fields + '}'


def cells():
    """Every cell: name, family, ticks, and the entities in the order they join as (tag, kind, dx, dy, dz, nbt),
    plus console commands to run once everyone has joined. Entities are 'a' and 'b'; `a-first` summons a before b."""
    out = []

    def add(family, a, b, ticks, first, entities, after=()):
        """`entities` is {'a': (...), 'b': (...)}; `first` is which one joins first."""
        order = [first, 'b' if first == 'a' else 'a']
        out.append(dict(name=f'{family}:{a}+{b}:{first}-first', family=family, ticks=ticks,
                        entities=[entities[who] for who in order], after=list(after)))

    for first in ('a', 'b'):
        for kind in TARGETS:
            add('wind', 'wind_charge', kind, 8, first, {
                'a': ('a', 'wind_charge', 0.5, 1.4, 0.5, '{Motion:[0.0d,-0.5d,0.0d]}'),
                'b': ('b', kind, 1.3, 1.0, 0.5, nbt(kind))})
        for kind in TARGETS:
            add('blast', 'tnt', kind, 6, first, {
                'a': ('a', 'tnt', 0.5, 1.0, 0.5, '{fuse:2s,NoGravity:1b}'),
                'b': ('b', kind, 1.8, 1.0, 0.5, nbt(kind))})
        for shot in ('arrow', 'snowball', 'trident'):
            for target in ('pig', 'oak_boat', 'minecart'):
                add('hit', shot, target, 8, first, {
                    'a': ('a', shot, 0.5, 1.2, 0.5, nbt(shot, ('0.5', '0.0', '0.0'))),
                    'b': ('b', target, 4.5, 1.0, 0.5, nbt(target, ('-0.15', '0.0', '0.0')))})
        for vehicle in ('oak_boat', 'minecart', 'pig'):
            add('ride', 'pig', vehicle, 8, first, {
                'a': ('a', 'pig', 1.5, 1.0, 0.5, nbt('pig')),
                'b': ('b', vehicle, 0.5, 1.0, 0.5, nbt(vehicle, ('0.3', '0.0', '0.0')))},
                after=['ride SEL_a mount SEL_b'])
        for mover, still in [('oak_boat', 'pig'), ('minecart', 'pig'), ('pig', 'pig'), ('oak_boat', 'oak_boat'),
                             ('minecart', 'minecart')]:
            add('push', mover, still, 8, first, {
                'a': ('a', mover, 0.5, 1.0, 0.5, nbt(mover, ('0.3', '0.0', '0.0'))),
                'b': ('b', still, 2.5, 1.0, 0.5, nbt(still))})
        merging_item = '{Item:{id:"minecraft:stone",count:1},NoGravity:1b,PickupDelay:0s,Age:0s}'
        add('push', 'item', 'item', 8, first, {
            'a': ('a', 'item', 0.5, 1.0, 0.5, merging_item[:-1] + ',Motion:[0.2d,0.0d,0.0d]}'),
            'b': ('b', 'item', 1.5, 1.0, 0.5, merging_item)})
        add('push', 'experience_orb', 'experience_orb', 8, first, {
            'a': ('a', 'experience_orb', 0.5, 1.0, 0.5, '{NoGravity:1b,Value:1s,Age:0s,Motion:[0.2d,0.0d,0.0d]}'),
            'b': ('b', 'experience_orb', 1.5, 1.0, 0.5, '{NoGravity:1b,Value:2s,Age:0s}')})
    return out


def selector(index, entity):
    """The entity as the nearest of its kind to where it joined: tags are not found on every kind in Horizon
    (rockets, tridents, falling blocks), so the selector uses the type and the start."""
    who, kind, dx, dy, dz, _ = entity
    ox, oy, oz = cell_origin(index)
    return (f'@e[type=minecraft:{kind},x={ox + dx},y={oy + dy},z={oz + dz},distance=..8,sort=nearest,limit=1]')


def cell_origin(index):
    return (PER_ROW and (index % PER_ROW) * 20, 100, (index // PER_ROW) * 20)


# ---- console ------------------------------------------------------------------------------------------------

class Console:
    """Commands sent to a server's console in a batch, each followed by a `say` marker, so the lines a command
    printed are known exactly."""

    def __init__(self, server):
        self.server = server
        self.count = 0

    def run(self, commands, timeout=120):
        start = len(self.server.lines)
        marks = []
        for command in commands:
            self.count += 1
            mark = f'TICKORDERMARK{self.count}X'
            marks.append(mark)
            self.server.process.stdin.write(command + '\nsay ' + mark + '\n')
        self.server.process.stdin.flush()
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(marks[-1] in line for line in self.server.lines[start:]) if marks else True:
                break
            if self.server.process.poll() is not None:
                raise AssertionError(f'{self.server.log_path.name}: server exited:\n' + '\n'.join(self.server.lines[-20:]))
            time.sleep(0.003)
        else:
            raise AssertionError(f'{self.server.log_path.name}: no answer in {timeout} s')
        replies, current, index = [], [], 0
        for line in self.server.lines[start:]:
            if index < len(marks) and marks[index] in line:
                replies.append(current)
                current, index = [], index + 1
            elif index < len(marks):
                current.append(line)
        return replies

    def one(self, command):
        return self.run([command])[0]

    def game_time(self):
        for line in self.one('time query gametime'):
            if m := re.search(r'time is (\d+)', line):
                return int(m.group(1))
        raise AssertionError('no game time')

    def step(self, timeout=60):
        before = self.game_time()
        self.one('tick step 1')
        deadline = time.time() + timeout
        while self.game_time() < before + 1:
            if time.time() > deadline:
                raise AssertionError('tick step did not finish')
            time.sleep(0.01)


FIELDS = [('Pos', r'\bPos:\s*\[([^\]]*)\]'), ('Motion', r'\bMotion:\s*\[([^\]]*)\]'),
          ('Health', r'\bHealth:\s*([-\d.eE]+)f'), ('fuse', r'\b[fF]use:\s*(-?\d+)s'),
          ('Value', r'\bValue:\s*(-?\d+)s'), ('count', r'\b[cC]ount:\s*(-?\d+)')]


def without_passengers(text):
    """The text with the `Passengers: [...]` list cut out: the rider's own Pos, Health... are not the vehicle's."""
    start = text.find('Passengers:')
    if start < 0:
        return text
    depth, i = 0, text.index('[', start)
    for j in range(i, len(text)):
        depth += {'[': 1, ']': -1}.get(text[j], 0)
        if depth == 0:
            return text[:start] + text[j + 1:]
    return text


def read_entity(lines, origin):
    text = ' '.join(lines)
    if 'entity data' not in text:
        return {'present': 'gone'}
    text = without_passengers(text)
    out = {'present': 'yes'}
    for key, pattern in FIELDS:
        if m := re.search(pattern, text):
            out[key] = m.group(1)
    if 'Pos' in out:  # relative to the cell, so the same cell in the other order compares
        xyz = numbers(out['Pos'])
        out['Pos'] = ', '.join(f'{c - o:.9f}' for c, o in zip(xyz, origin))
    return out


def run_side(label, command, build, world, all_cells):
    directory = build / label
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    port = free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    shutil.copytree(world, directory / 'world')
    (directory / 'world' / 'session.lock').unlink(missing_ok=True)
    server = Server(command, directory, label)
    ticks = max(c['ticks'] for c in all_cells)
    try:
        server.wait_for(r'Done \(', timeout=300)
        console = Console(server)
        rows = max(i // PER_ROW for i in range(len(all_cells))) + 1
        x_end, z_end = PER_ROW * 20 + 16, rows * 20 + 16
        for x0 in range(-16, x_end, 160):
            for z0 in range(-16, z_end, 160):
                console.one(f'forceload add {x0} {z0} {min(x0 + 159, x_end)} {min(z0 + 159, z_end)}')
        time.sleep(3)
        console.one('tick freeze')
        console.one('gamerule mob_griefing true')
        for x0 in range(-16, x_end, 120):
            console.one(f'fill {x0} 99 -16 {min(x0 + 119, x_end)} 99 {z_end} minecraft:bedrock')
            console.one(f'fill {x0} 100 -16 {min(x0 + 119, x_end)} 108 {z_end} minecraft:air')
        for index, cell in enumerate(all_cells):
            ox, oy, oz = cell_origin(index)
            commands = []
            for who, kind, dx, dy, dz, data in cell['entities']:
                x, y, z = ox + dx, oy + dy, oz + dz
                commands.append(f'summon minecraft:{kind} {x} {y} {z} {data}')
            by_who = {e[0]: selector(index, e) for e in cell['entities']}
            commands += [c.replace('SEL_a', by_who['a']).replace('SEL_b', by_who['b']) for c in cell['after']]
            console.run(commands)
        history = {}

        def probe(tick):
            commands = [f'data get entity {selector(index, e)}' for index, cell in enumerate(all_cells)
                        for e in sorted(cell['entities'])]
            replies = console.run(commands)
            for (index, cell), pair in zip(enumerate(all_cells), zip(replies[0::2], replies[1::2])):
                history.setdefault(cell['name'], {})[tick] = {who: read_entity(lines, cell_origin(index))
                                                              for who, lines in zip(('a', 'b'), pair)}
        probe(0)
        for tick in range(1, ticks + 1):
            console.step()
            probe(tick)
        return history
    finally:
        server.stop()


# ---- comparison ---------------------------------------------------------------------------------------------

def numbers(text):
    return [float(t) for t in re.findall(r'-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?', text)]


def values(history):
    """cell -> {(tick, entity, field): text}"""
    return {name: {(tick, who, key): text for tick, pair in ticks.items() for who, fields in pair.items()
                   for key, text in fields.items()} for name, ticks in history.items()}


def compare(one, other):
    """(values off, first tick that differs, largest difference, total) between two cells' values, by key: a value
    one side lacks counts as infinitely far."""
    off, first, worst = 0, None, 0.0
    for key in sorted(set(one) | set(other), key=lambda k: (int(k[0]), k[1], k[2])):
        left, right = one.get(key), other.get(key)
        if left == right:
            continue
        a, b = numbers(left or ''), numbers(right or '')
        delta = max((abs(x - y) for x, y in zip(a, b)), default=0.0) if left and right and len(a) == len(b) else float('inf')
        if delta > TOLERANCE or left is None or right is None:
            off += 1
            first = int(key[0]) if first is None else first
            worst = max(worst, delta)
    return off, first, worst, len(set(one) | set(other))


def moved(rows, whos):
    """Values of `whos` that changed between the first tick and the last, in kinds of change that count."""
    first, last = {}, {}
    for (tick, who, key), text in sorted(rows.items(), key=lambda kv: int(kv[0][0])):
        if who in whos and key in ('Pos', 'Motion', 'Health', 'present', 'Value', 'count'):
            first.setdefault((who, key), text)
            last[(who, key)] = text
    return sum(1 for k in first if first[k] != last[k])


def report(vanilla, horizon):
    """Rows of the table and the cells that fail."""
    v, h = values(vanilla), values(horizon)
    lines, failed, fixed, vacuous = [], [], [], []
    names = list(v)
    for name in names:
        off, first, worst, total = compare(v[name], h[name])
        other = name.replace('a-first', 'b-first') if 'a-first' in name else name.replace('b-first', 'a-first')
        van_matters = compare(v[name], v[other])[0] > 0
        hor_matters = compare(h[name], h[other])[0] > 0
        status = 'match' if off == 0 else f'DIFF first tick {first}, {off} of {total} values, largest {worst:.4g}'
        lines.append(f'{name:46} {status:62} order matters: vanilla {"yes" if van_matters else "no "}, horizon '
                     f'{"yes" if hor_matters else "no"}')
        # a wind burst does not push an item: that cell is a control and may stay still
        if moved(v[name], 'b' if name.startswith(('wind', 'blast')) else 'ab') == 0 and not name.startswith('wind:wind_charge+item'):
            vacuous.append(name)
        if off and name not in KNOWN:
            failed.append(name)
        if not off and name in KNOWN:
            fixed.append(name)
    return lines, failed, fixed, vacuous


def main():
    noise = '--noise' in sys.argv
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    (build / 'report.json').unlink(missing_ok=True)
    subprocess.run(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], cwd=lab.ROOT,
                   check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    vanilla_server = ['java', '-Xmx2g', '-Duser.timezone=UTC', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']

    all_cells = cells()
    world = generate_world(vanilla_server, build)
    vanilla = run_side('vanilla', vanilla_server, build, world, all_cells)
    horizon = run_side('horizon', horizon_server, build, world, all_cells)
    if noise:
        again = run_side('vanilla2', vanilla_server, build, world, all_cells)
        itself = sum(compare(a, b)[0] > 0 for a, b in zip(values(vanilla).values(), values(again).values()))
        print(f'vanilla differs from itself in {itself} of {len(all_cells)} cells')
    lines, failed, fixed, vacuous = report(vanilla, horizon)
    text = '\n'.join(lines)
    (build / 'table.txt').write_text(text + '\n', encoding='utf-8')
    (build / 'values.json').write_text(json.dumps(dict(vanilla=vanilla, horizon=horizon)), encoding='utf-8')
    print(text)
    differing = [line for line in lines if ' DIFF ' in line]
    print(f'\n{len(lines) - len(differing)} of {len(lines)} cells identical to vanilla; the {len(differing)} that differ '
          f'are listed above ({len([n for n in KNOWN if n in values(vanilla)])} recorded in KNOWN).')
    if fixed:
        print('matching vanilla now, take them out of KNOWN: ' + ', '.join(fixed))
    if vacuous:
        raise AssertionError('nothing moved on vanilla in: ' + ', '.join(vacuous))
    if failed:
        raise AssertionError(f'{len(failed)} cells differ and are not in KNOWN:\n' + '\n'.join(failed))
    lab.write_json(build / 'report.json', dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0],
        cells=len(lines), differing=len(differing), known=sorted(KNOWN),
        scope='The order entities join a level and what they then do on a dedicated server, per tick, in ticking '
              'chunks: wind bursts, TNT blasts, projectile hits, riders and vehicles, pushes and merges, each with '
              'the kinds joined in both orders, against vanilla.'))
    print(f'tick order: {len(lines) - len(differing)} of {len(lines)} cells identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
