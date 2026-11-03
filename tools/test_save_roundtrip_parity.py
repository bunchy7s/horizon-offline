#!/usr/bin/env python3
"""Save-format differential test: vanilla 26.3 world -> Horizon -> vanilla.

1. A headless vanilla server (flat world, offline) is filled with one of every block
   entity type and many entity types (tools/save_roundtrip_scene.py), a bot player
   joins so a player file exists, and the world is saved (world V0).
2. Horizon opens a copy of V0 under a virtual screen, rewrites every resident chunk and
   entity section (`save-all`) and quits; this is world H.
3. The NBT of V0 and H is compared: chunks (block states and biomes decoded, so palette
   order does not matter), block entities, entities, heightmaps, light, ticks, structure
   data, level.dat, data/*.dat, players/data/*.dat. Differences must be explained by
   IGNORED below (vanilla-irrelevant: elapsed time, ordering, derived data) or the test
   fails.
4. The vanilla server loads H: its log must show no errors, upgrades or DataFixer
   activity, and `data get` of every block entity and entity must print the same
   data as the server that loaded V0 (so vanilla reads Horizon's files as it reads its own).

  test_save_roundtrip_parity.py [--keep] [--skip-horizon] [--bin-dir DIR] [--stage gen|resave|compare|all]
                                [--world DIR] (compare an existing V0/H pair: --vanilla DIR --horizon DIR)

Run files stay under .local/save-roundtrip/run. On Linux the Horizon client runs on the
virtual screen (`vscreen`), never on a real monitor, with a null ALSA device; on macOS it
renders offscreen (`--offscreen`, no window) without audio, from the logged-in session.
"""
import argparse
import collections
import json
import os
import queue
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import anvil_dump  # noqa: E402
import save_roundtrip_scene as scene  # noqa: E402

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parent.parent
LOCAL = ROOT / '.local'
RESOURCES = lab.RESOURCES
SERVER = lab.GAME / 'jars' / 'server.jar'
RUN = LOCAL / 'save-roundtrip' / 'run'

# Game rules Horizon's player changes before saving: rule -> (vanilla's value, Horizon's).
# The new value must be in Horizon's save, and nothing else may differ (#74).
HORIZON_RULES = {'max_entity_cramming': (24, 30)}

# Differences that do not change what vanilla reads: path pattern (list indices as [])
# -> reason. Anything else is a failure.
IGNORED = {
    r'^chunk\.LastUpdate$': 'game time of the save (Horizon saves at its own time)',
    r'^chunk\.InhabitedTime$': 'ticks the chunk was loaded; differs by the time Horizon ran',
    r'^chunk\.isLightOn$': 'light is recomputed by vanilla when absent',
    # Vanilla's server (view distance 4) leaves a border of proto chunks; Horizon's
    # render distance 6 may finish some, as vanilla does when it loads the world at
    # that distance. Their blocks are still compared; only what a full chunk saves
    # differently (no worldgen heightmaps, entities in entities/) is accepted.
    r'^chunk\.promoted\.(Status|Heightmaps\.(OCEAN_FLOOR_WG|WORLD_SURFACE_WG)|entities)$':
        'a proto chunk vanilla left at its view distance, finished by Horizon',
    # Single player's server thread sends the chunks its player tracks (view distance 6
    # and its outer ring), as vanilla's integrated server does, so it finishes some that
    # vanilla's server (view distance 4) left before their terrain: those had no blocks,
    # light or heightmaps to compare yet.
    r'^chunk\.blank\.':
        'a chunk vanilla left before its terrain (no blocks yet), finished by Horizon',
    r'^chunk\.unlit\.':
        'a chunk vanilla left before its light, finished (and lit) by Horizon',
    # Horizon keeps simulating between loading and saving (it needs a few seconds of
    # chunk generation first), so counters and positions of live things move on.
    r'^chunk\.block_entities\.\(2, -60, 5\)\.(cooking_time_spent|lit_time_remaining|Items\[\]\.count)$':
        'the lit furnace keeps smelting',
    r'^chunk\.block_entities\.\(14, -60, 5\)\.CookingTimes$':
        'the lit campfire keeps cooking',
    r'^chunk\.block_entities\..*\.(ticks_since_song_started|Age|BrewTime|warning_level|ticks_in_hive|last_vibration_frequency|listener\..*|RecipesUsed\..*|'
    r'server_data\.state_updating_resumes_at)$':
        'block entity timers advance',
    r'^entity\.(Passengers\[\]\.)*(Age|Motion\[\]|Rotation\[\]|Pos\[\]|OnGround|HurtTime|Health|TimeInOverworld|fall_distance|'
    r'wasOnGround|Fire|Air|DeathTime|Fuse|life|inGround|ticks_in_ground|Time)$': 'entity simulation advances',
    r'^level\.dat\.Data\.(Time|LastPlayed)$': 'clock and wall time of the save',
    r'^level\.dat\.Data\.(ServerBrands.*|singleplayer_uuid)$': 'a client world names its brand and its player',
    r'^entity\.listener\.selector\.(event|tick)$': 'the warden listener holds the last vibration it heard; timing',
    r'^players/data/.*\.dat\.warden_spawn_tracker\.': 'counts ticks since the last warning; timing',
    r'^players/data/.*\.dat\.recipeBook\.(recipes|toBeDisplayed)': 'which recipes unlock depends on the pickup history of the bot',
    r'^players/data/.*\.dat\.(Health|Pos\[\]|foodSaturationLevel|foodLevel|foodExhaustionLevel|Motion\[\]|OnGround)$':
        'the player keeps simulating',
}

# Real differences that are known and not fixed yet: (pattern -> what is wrong). They are
# reported, and do not fail the run, so that new differences still do.
KNOWN_GAPS = {
}

SUSPICIOUS = re.compile(
    r"(/ERROR\]|/WARN\]|login failed|Exception|Couldn't load|Failed to|Skipping|Invalid|Unknown|corrupt|"
    r'Recreating|upgrad|DataFix|out of bounds|missing|Mismatch|Discarding)', re.IGNORECASE)
BENIGN = re.compile(r'(Unpacking |OFFLINE/INSECURE|online-mode|authenticate usernames|hackers|'
                    r'Failed to load properties|Failed to load eula|Unknown or incomplete command|'
                    r'Can.t keep up|Ignoring unknown|^WARNING: |No key layers|has the following|udev library)')


# ---------------------------------------------------------------- vanilla server

class Vanilla:
    def __init__(self, directory, port=None, world='world', extra=None):
        port = port or lab.free_port()
        directory.mkdir(parents=True, exist_ok=True)
        (directory / 'eula.txt').write_text('eula=true\n')
        props = {
            'server-port': port, 'online-mode': 'false', 'white-list': 'false', 'enforce-whitelist': 'false',
            'level-name': world, 'level-seed': '4242', 'spawn-protection': '0', 'view-distance': '4',
            'simulation-distance': '4', 'sync-chunk-writes': 'true', 'enable-query': 'false',
            'enable-rcon': 'false', 'management-server-enabled': 'false', 'pause-when-empty-seconds': '-1',
            'level-type': 'minecraft:flat', 'generate-structures': 'false', 'spawn-monsters': 'false',
            'spawn-npcs': 'false', 'spawn-animals': 'false', 'max-tick-time': '-1',
        }
        props.update(extra or {})
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        self.port = port
        self.process = subprocess.Popen(
            ['java', '-Xmx2G', '-jar', str(SERVER), '--nogui'], cwd=directory, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        self.lines = []
        self.queue = queue.Queue()
        self.counter = 0
        threading.Thread(target=self._pump, daemon=True).start()
        self.wait('Done (', 240)

    def _pump(self):
        for line in self.process.stdout:
            line = line.rstrip('\n')
            self.lines.append(line)
            self.queue.put(line)

    def wait(self, pattern, timeout=60):
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            try:
                line = self.queue.get(timeout=1)
            except queue.Empty:
                if self.process.poll() is not None:
                    break
                continue
            seen.append(line)
            if pattern in line:
                return seen
        raise TimeoutError(f'vanilla: no "{pattern}" in {timeout}s; last lines: {seen[-5:]}')

    def run(self, command, timeout=60):
        """Run a console command; return the log lines it produced."""
        self.counter += 1
        marker = f'@@{self.counter}@@'
        self.process.stdin.write(command + '\n' + 'say ' + marker + '\n')
        self.process.stdin.flush()
        out = self.wait(marker, timeout)
        return [re.sub(r'^\[[0-9:]+\] \[[^\]]*\]: (System chat: )?', '', l) for l in out[:-1]]

    def stop(self):
        try:
            self.process.stdin.write('stop\n')
            self.process.stdin.flush()
            self.process.wait(90)
        except Exception:
            self.process.kill()

    def problems(self):
        return [l for l in self.lines if SUSPICIOUS.search(l) and not BENIGN.search(l)]


def command_failed(lines):
    return any(re.search(r'(Unknown|Expected|Incorrect|Invalid|Could not|Unable|Failed|That block|error)', l, re.I)
               for l in lines)


UUID_BASE = 0x7000


def entity_uuid(i):
    return f'[I;{UUID_BASE},0,0,{i}]'


def entity_command(i, kind, extra):
    x = 3 + 3 * (i % 10)
    z = 36 + 3 * (i // 10)
    # `summon` places the entity at the command's coordinates, not at NBT Pos, so a
    # scene entry that has to stand somewhere (next to a fence) names it in Pos.
    at = re.search(r'Pos:\[(-?[0-9.]+)d,(-?[0-9.]+)d,(-?[0-9.]+)d\]', extra)
    if at:
        x, z = at.group(1), at.group(3)
    nbt = (f'UUID:{entity_uuid(i)},NoAI:1b,Silent:1b,NoGravity:1b,Invulnerable:1b,Tags:["rt"],'
           f'CustomName:{{text:"E{i}"}}' + (',' + extra if extra else ''))
    if kind in ('item_frame', 'glow_item_frame', 'painting'):
        nbt += f',block_pos:[I;{x},-60,{z}],Pos:[{x}.5d,-60.0d,{z}.5d]'
    return f'summon minecraft:{kind} {x} {at.group(2) if at else -60} {z} {{{nbt}}}'


def block_position(i):
    return 2 + 3 * (i % 10), -60, 2 + 3 * (i // 10)


SCOREBOARD = [
    'scoreboard objectives add kills totalKillCount {"text":"Kills","color":"red"}',
    'scoreboard objectives add d dummy',
    'scoreboard objectives add hearts dummy "Hearts"',
    'scoreboard objectives modify hearts rendertype hearts',
    'scoreboard objectives add t trigger',
    'scoreboard objectives add mined minecraft.mined:minecraft.stone',
    'scoreboard objectives modify d displayautoupdate true',
    'scoreboard objectives modify d numberformat styled {"color":"gold","bold":true}',
    'scoreboard objectives modify kills numberformat blank',
    'scoreboard objectives setdisplay sidebar d',
    'scoreboard objectives setdisplay below_name hearts',
    'scoreboard objectives setdisplay sidebar.team.red kills',
    'scoreboard players set #fake d 5',
    'scoreboard players set Alex d -3',
    'scoreboard players set Alex kills 7',
    'scoreboard players set Alex hearts 0',
    'scoreboard players set Steve hearts 12',
    'scoreboard players add {uuid0} d 2',
    'scoreboard players display name Alex d {"text":"Alexandra","italic":true}',
    'scoreboard players display numberformat Alex hearts fixed "lots"',
    'scoreboard players enable Alex t',
    'scoreboard players set Steve t 3',
    'team add red {"text":"Red Team"}',
    'team modify red color red',
    'team modify red prefix "[R] "',
    'team modify red suffix {"text":" *","color":"gold"}',
    'team modify red friendlyFire false',
    'team modify red seeFriendlyInvisibles false',
    'team modify red nametagVisibility hideForOtherTeams',
    'team modify red deathMessageVisibility never',
    'team modify red collisionRule pushOwnTeam',
    'team join red Alex',
    'team join red #fake',
    'team add blue',
    'team join blue {uuid0}',
    'team join blue Steve',
]


def normalize_scoreboard(data):
    """Each holder's scores sit in an identity-hash map in vanilla: compare them as a set."""
    scores = data.get('data', {}).get('PlayerScores') if isinstance(data, dict) else None
    if isinstance(scores, list):
        order = []
        for row in scores:
            if row.get('Name') not in order:
                order.append(row.get('Name'))
        data['data']['PlayerScores'] = sorted(
            scores, key=lambda r: (order.index(r.get('Name')), json.dumps(r, sort_keys=True, default=str)))
    return data


def build_world(directory):
    """Fill a fresh vanilla world; returns the failed scene commands."""
    server = Vanilla(directory)
    failed = []
    # A bot joins first so the player file exists and the chunks around spawn are loaded.
    server.run('forceload add 0 0 63 63')
    for rule in ('spawn_mobs', 'spawn_monsters', 'spawn_wandering_traders', 'spawn_patrols', 'spawn_phantoms',
                 'spawn_wardens', 'raids'):
        server.run(f'gamerule {rule} false')
    server.run('gamerule random_tick_speed 0')
    time.sleep(3)
    server.run('fill 0 -61 0 47 -61 47 minecraft:stone')
    for i, (block, nbt) in enumerate(scene.BLOCK_ENTITIES):
        x, y, z = block_position(i)
        command = f'setblock {x} {y} {z} minecraft:{block}{nbt}'
        out = server.run(command)
        if command_failed(out):
            failed.append((command, out))
    for command in scene.SETBLOCKS:
        out = server.run(command)
        if command_failed(out):
            failed.append((command, out))
    for i, (kind, extra) in enumerate(scene.ENTITIES):
        command = entity_command(i, kind, extra)
        out = server.run(command)
        if command_failed(out):
            failed.append((command, out))
    # A scoreboard with every kind of saved field (scoreboard.dat). No criteria that
    # Horizon's own player (not the bot) would update while it runs.
    for command in SCOREBOARD:
        out = server.run(command.replace('{uuid0}', entity_uuid_text(0)))
        if command_failed(out):
            failed.append((command, out))
    server.run('time set 6000')
    server.run('gamerule advance_time false')
    server.run('gamerule advance_weather false')
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import vanilla_login
    threading.Thread(target=lambda: vanilla_login.join('127.0.0.1', server.port, 'RoundTrip', 25), daemon=True).start()
    time.sleep(6)
    server.run('item replace entity RoundTrip container.0 with minecraft:iron_sword[damage=3]')
    server.run('give RoundTrip minecraft:apple 7')
    server.run('xp set RoundTrip 5 levels')
    time.sleep(20)
    server.run('save-all flush')
    time.sleep(2)
    server.stop()
    return failed, server.problems()


def view_of_world(directory):
    """Run vanilla on a world; return ({label: data text}, problem log lines)."""
    server = Vanilla(directory)
    views = {}
    try:
        server.run('forceload add 0 0 63 63')
        time.sleep(3)
        for i, (block, _) in enumerate(scene.BLOCK_ENTITIES):
            x, y, z = block_position(i)
            out = server.run(f'data get block {x} {y} {z}')
            views[f'block {i} {block}'] = ' '.join(out)
        for i, (kind, _) in enumerate(scene.ENTITIES):
            out = server.run(f'data get entity {entity_uuid_text(i)}')
            views[f'entity {i} {kind}'] = ' '.join(out)
    finally:
        server.stop()
    return views, server.problems()


def entity_uuid_text(i):
    def u(v):
        return f'{v & 0xffffffff:08x}'
    return f'{u(UUID_BASE)}-{u(0)[:4]}-{u(0)[4:]}-{u(0)[:4]}-{u(0)[4:]}{u(i)}'


# ---------------------------------------------------------------- Horizon

def vscreen_env():
    text = subprocess.run(['vscreen', 'env'], capture_output=True, text=True, check=True).stdout
    env = lab.game_env()
    env.pop('DISPLAY', None)
    for match in re.finditer(r'(\w+)=(\S+)', text.split('unset')[0]):
        env[match.group(1)] = match.group(2)
    return env


def horizon_resave(source, destination, bin_dir):
    shutil.rmtree(destination, ignore_errors=True)
    saves = destination / 'saves'
    saves.mkdir(parents=True)
    shutil.copytree(source, saves / 'world')
    socket = Path(tempfile.mkdtemp(prefix='hzrt')) / 'control.sock'  # sockets need short paths
    ctl = [str(bin_dir / 'horizon-ctl'), '--socket', str(socket)]
    extra = []
    # Offscreen on macOS, and on a Linux without vscreen (never a window on a real monitor).
    if sys.platform == 'darwin' or not shutil.which('vscreen'):
        env = lab.game_env()
        extra = ['--offscreen']
    else:
        alsa = destination / 'null-asound.conf'
        alsa.write_text('pcm.!default { type null }\nctl.!default { type null }\n')
        env = vscreen_env()
        env['ALSA_CONFIG_PATH'] = str(alsa)
        subprocess.run(['vscreen', 'up'], check=False, capture_output=True)
    launch = subprocess.run(
        ctl + ['launch', '--resources', str(RESOURCES), '--world', 'world', '--render-distance', '6'] + extra,
        cwd=destination, env=env, capture_output=True, text=True)
    print(launch.stdout.strip())
    if launch.returncode:
        sys.exit('horizon failed to launch: ' + launch.stdout + launch.stderr)
    steps = [['wait-ready', '240']]
    steps += [['command', f'gamerule {rule} {new}'] for rule, (_, new) in HORIZON_RULES.items()]
    steps += [['save-all'], ['quit']]
    for step in steps:
        result = subprocess.run(ctl + step, cwd=destination, env=env, capture_output=True, text=True)
        print(' '.join(step), '->', result.stdout.strip() or result.stderr.strip())
        if step[0] != 'quit' and result.returncode:
            sys.exit('horizon-ctl failed: ' + result.stdout)
    time.sleep(3)
    return saves / 'world'


# ---------------------------------------------------------------- NBT comparison

def unwrap(v):
    """26.3 mixed-list wrappers ({"": value}) and plain python values."""
    if isinstance(v, dict):
        return {k: unwrap(x) for k, x in v.items()}
    if isinstance(v, list):
        return [unwrap(x[''] if isinstance(x, dict) and list(x) == [''] else x) for x in v]
    return v


def short(v):
    s = json.dumps(v, default=str)
    return s if len(s) < 160 else s[:157] + '...'


def diff(a, b, path, out):
    if type(a) is not type(b):
        out.append((path, a, b))
    elif isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k not in a:
                out.append((f'{path}.{k}', None, b[k]))
            elif k not in b:
                out.append((f'{path}.{k}', a[k], None))
            else:
                diff(a[k], b[k], f'{path}.{k}', out)
    elif isinstance(a, list):
        if len(a) != len(b):
            out.append((f'{path}[len]', len(a), len(b)))
            if all(not isinstance(x, (dict, list)) for x in a + b):
                out.append((path, a, b))
                return
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f'{path}[{i}]', out)
    elif a != b:
        out.append((path, a, b))


def pattern_of(path):
    return re.sub(r'\[\d+\]', '[]', path)


def ignored(path):
    for pattern, why in IGNORED.items():
        if re.search(pattern, pattern_of(path.split(' @')[0])):
            return why
    return None


def state_text(entry):
    if isinstance(entry, str):
        return entry
    props = entry.get('properties', entry.get('Properties', {}))
    return entry.get('id', entry.get('Name')) + ('[' + ','.join(f'{k}={v}' for k, v in sorted(props.items())) + ']' if props else '')


def unpack(longs, bits, count):
    per = 64 // bits
    out = []
    for word in longs:
        word &= (1 << 64) - 1
        for i in range(per):
            if len(out) == count:
                return out
            out.append((word >> (i * bits)) & ((1 << bits) - 1))
    return out


def decode_container(container, count, minimum, default):
    palette = [state_text(e) for e in container.get('palette', [default])] if count == 4096 else \
        list(container.get('palette', [default]))
    if len(palette) == 1:
        return [palette[0]] * count
    bits = max(minimum, (len(palette) - 1).bit_length())
    return [palette[i] if i < len(palette) else None for i in unpack(container['data'][1], bits, count)]


def normalize_chunk(root):
    root = unwrap(root)
    sections = {}
    for s in root.get('sections', []):
        y = s['Y'][1] if isinstance(s['Y'], tuple) else s['Y']
        blocks = decode_container(s['block_states'], 4096, 4, 'minecraft:air')
        biomes = decode_container(s['biomes'], 64, 1, 'minecraft:plains') if 'biomes' in s else None
        sections[y] = {'blocks': blocks, 'biomes': biomes, 'BlockLight': s.get('BlockLight'),
                       'SkyLight': s.get('SkyLight')}
    out = {k: v for k, v in root.items() if k != 'sections'}
    out['block_entities'] = {(e['x'], e['y'], e['z']): e for e in root.get('block_entities', [])}
    for key in ('block_ticks', 'fluid_ticks'):
        out[key] = sorted(root.get(key, []), key=lambda t: json.dumps(t, sort_keys=True, default=str))
    return out, sections


# ChunkStatus before NOISE: nothing has filled the chunk's blocks yet.
BLANK_STATUSES = {'minecraft:empty', 'minecraft:structure_starts', 'minecraft:structure_references',
                  'minecraft:biomes'}
# ChunkStatus LIGHT and later: the chunk's light is saved.
LIT_STATUSES = {'minecraft:light', 'minecraft:spawn', 'minecraft:full'}


def compare_chunks(vanilla, horizon, report):
    for key in sorted(set(vanilla) | set(horizon)):
        if key not in horizon:
            report.append((f'chunk {key}', 'present', 'missing from Horizon'))
            continue
        if key not in vanilla:
            report.append((f'chunk {key}', 'missing from vanilla', 'present'))
            continue
        va, vs = normalize_chunk(vanilla[key])
        ha, hs = normalize_chunk(horizon[key])
        out = []
        diff(va, ha, 'chunk', out)
        promoted = va.get('Status') != 'minecraft:full' and ha.get('Status') == 'minecraft:full'
        if promoted:
            rule = re.compile(r'^chunk\.(Status|Heightmaps\.\w+_WG|entities)$')
            out = [(path.replace('chunk.', 'chunk.promoted.', 1) if rule.match(path) else path, p, q)
                   for path, p, q in out]
        for y in sorted(set(vs) | set(hs)):
            empty = {'blocks': ['minecraft:air'] * 4096, 'biomes': None, 'BlockLight': None, 'SkyLight': None}
            a, b = vs.get(y, empty), hs.get(y, empty)
            blocks = [(i, p, q) for i, (p, q) in enumerate(zip(a['blocks'], b['blocks'])) if p != q]
            if blocks:
                i, p, q = blocks[0]
                out.append((f'chunk.sections.blocks[{y}]', f'{len(blocks)} differ, first #{i} {p}', q))
            if a['biomes'] is not None and b['biomes'] is not None and a['biomes'] != b['biomes']:
                out.append((f'chunk.sections.biomes[{y}]', 'differ', ''))
            for light in ('BlockLight', 'SkyLight'):
                if a[light] != b[light]:
                    out.append((f'chunk.sections.{light}[{y}]', None if a[light] is None else 'present',
                                None if b[light] is None else 'present'))
        # Before its terrain a chunk has no blocks, light or heightmaps yet.
        if promoted and va.get('Status') in BLANK_STATUSES:
            blank = re.compile(r'^chunk\.(Heightmaps|sections)\.')
            out = [(path.replace('chunk.', 'chunk.blank.', 1) if blank.match(path) else path, p, q)
                   for path, p, q in out]
        # Before its light step a chunk saves no light; Horizon lights what it finishes.
        elif promoted and va.get('Status') not in LIT_STATUSES:
            unlit = re.compile(r'^chunk\.sections\.(SkyLight|BlockLight)\[')
            out = [(path.replace('chunk.', 'chunk.unlit.', 1) if unlit.match(path) else path, p, q)
                   for path, p, q in out]
        for path, p, q in out:
            report.append((f'{path} @{key}', p, q))


def chunk_of(entity):
    pos = entity.get('Pos')
    pos = pos[1] if isinstance(pos, tuple) else pos
    x, z = (p[1] if isinstance(p, tuple) else p for p in (pos[0], pos[2]))
    return int(x // 16), int(z // 16)


def compare_entities(vanilla, horizon, report, chunks=None):
    def collect(regions):
        out = {}
        for key, root in regions.items():
            for e in unwrap(root).get('Entities', []):
                uuid = tuple(e['UUID'][1]) if 'UUID' in e else None
                out[uuid] = e
        return out
    a, b = collect(vanilla), collect(horizon)
    # Vanilla drops a lead whose holder it cannot find, which would leave nothing to compare.
    leads = sum('leash' in e for e in a.values())
    if leads != scene.LEADS:
        report.append(('vanilla kept leads in the scene', scene.LEADS, leads))
    # Entities Horizon adds in the chunks vanilla never generated (chunk-generation
    # spawns) are not part of the scene; anywhere else they are a difference.
    vanilla_chunks = {chunk_of(e) for e in a.values()} | set(chunks or ())
    extra = [u for u in b if u not in a and not (u and u[0] == UUID_BASE)
             and chunk_of(b[u]) not in vanilla_chunks]
    if extra:
        print(f'  note: Horizon saved {len(extra)} entities in chunks vanilla never generated')
    for uuid in extra:
        del b[uuid]
    for uuid in sorted(set(a) | set(b), key=str):
        if uuid not in b:
            report.append((f'entity {uuid} {a[uuid].get("id")}', 'present', 'missing from Horizon'))
        elif uuid not in a:
            report.append((f'entity {uuid} {b[uuid].get("id")}', 'missing from vanilla', 'present'))
        else:
            out = []
            diff(a[uuid], b[uuid], 'entity', out)
            for path, p, q in out:
                report.append((f'{path} @{a[uuid].get("id")}#{uuid[-1]}', p, q))


def load_regions(folder):
    out = {}
    if folder.is_dir():
        for f in sorted(folder.glob('r.*.*.mca')):
            out.update({(f.name, *k): v for k, v in anvil_dump.region_chunks(str(f)).items()})
    return out


def compare_dat(name, a, b, report):
    out = []
    diff(unwrap(a), unwrap(b), name, out)
    report.extend(out)


def dimension(world):
    d = world / 'dimensions' / 'minecraft' / 'overworld'
    return d if d.is_dir() else world


def compare_worlds(vanilla, horizon):
    report = []
    va, ha = dimension(vanilla), dimension(horizon)
    compare_chunks(load_regions(va / 'region'), load_regions(ha / 'region'), report)
    chunks = {(k[1], k[2]) for k in load_regions(va / 'region')}
    compare_entities(load_regions(va / 'entities'), load_regions(ha / 'entities'), report, chunks)
    for rel in ['level.dat'] + sorted(
            str(p.relative_to(vanilla)) for pattern in ('data/**/*.dat', 'players/**/*.dat')
            for p in vanilla.glob(pattern)):
        a, b = vanilla / rel, horizon / rel
        if not b.exists():
            report.append((rel, 'present', 'missing from Horizon'))
        elif a.exists():
            va, hb = anvil_dump.read_dat(str(a)), anvil_dump.read_dat(str(b))
            if rel.endswith('scoreboard.dat'):
                va, hb = normalize_scoreboard(va), normalize_scoreboard(hb)
            compare_dat(rel, va, hb, report)
    for rule, (old, new) in HORIZON_RULES.items():
        changed = (f'data/minecraft/game_rules.dat.data.minecraft:{rule}', old, new)
        if changed in report:
            report.remove(changed)
        else:
            report.append((changed[0], f'{new} set in Horizon', 'not saved'))
    return report


def summarize(report):
    groups = collections.OrderedDict()
    for path, a, b in report:
        why = ignored(path)
        groups.setdefault((pattern_of(path.split(' @')[0]), why), []).append((path, a, b))
    failures = 0
    for (pattern, why), items in groups.items():
        if why:
            print(f'  ignored  {pattern}  x{len(items)}  ({why})')
            continue
        gap = next((g for p, g in KNOWN_GAPS.items() if re.search(p, pattern)), None)
        if gap:
            print(f'  KNOWN GAP  {pattern}  x{len(items)}  ({gap})')
            continue
        failures += len(items)
        path, a, b = items[0]
        print(f'  DIFF     {pattern}  x{len(items)}\n           e.g. {path}\n           vanilla {short(a)}\n           horizon {short(b)}')
    return failures


# ---------------------------------------------------------------- main

def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--keep', action='store_true')
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build them with the fast profile)')
    parser.add_argument('--vanilla', help='an existing vanilla world folder (skips generation)')
    parser.add_argument('--horizon', help='an existing Horizon-saved world folder (skips the Horizon run)')
    parser.add_argument('--skip-vanilla-load', action='store_true')
    parser.add_argument('--gen-only', action='store_true', help='stop after building the vanilla world')
    args = parser.parse_args()
    RUN.mkdir(parents=True, exist_ok=True)
    failures = 0
    if args.vanilla:
        v0 = Path(args.vanilla)
    else:
        shutil.rmtree(RUN / 'v0', ignore_errors=True)
        failed, problems = build_world(RUN / 'v0')
        v0 = RUN / 'v0' / 'world'
        for command, out in failed:
            print('scene command failed:', command[:100], out[:2])
        for line in problems:
            print('vanilla log:', line)
    if args.gen_only:
        return
    if args.horizon:
        h = Path(args.horizon)
    else:
        bin_dir = Path(args.bin_dir).resolve() if args.bin_dir else None
        if bin_dir is None:
            subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib', '--bins'], cwd=ROOT, check=True)
            bin_dir = ROOT / os.environ.get('CARGO_TARGET_DIR', 'target') / 'fast'
        h = horizon_resave(v0, RUN / 'h', bin_dir)
    print('comparing', v0, 'with', h)
    report = compare_worlds(v0, h)
    failures += summarize(report)
    if not args.skip_vanilla_load:
        shutil.rmtree(RUN / 'v1', ignore_errors=True)
        shutil.copytree(h, RUN / 'v1' / 'world')
        views1, problems1 = view_of_world(RUN / 'v1')
        shutil.rmtree(RUN / 'v0view', ignore_errors=True)
        shutil.copytree(v0, RUN / 'v0view' / 'world')
        views0, problems0 = view_of_world(RUN / 'v0view')
        untimed = re.compile(r'^\[[0-9:]+\] ')
        seen0 = {untimed.sub('', line) for line in problems0}
        for line in problems1:
            if untimed.sub('', line) not in seen0:
                print('vanilla log (Horizon save):', line)
                failures += 1
        volatile = re.compile(r'(cooking_time_spent|lit_time_remaining|ticks_since_song_started|Age|BrewTime|'
                              r'ticks_in_hive|triggered|state_updating_resumes_at|LastExecution|last_vibration_frequency|warning_level): -?[0-9]+[a-zA-Z]?')
        # A lit campfire keeps cooking while each server runs before `data get`: its
        # progress counters advance by however many ticks that took (stage 3 checks
        # that Horizon saves them exactly as loaded).
        volatile2 = re.compile(r'(RecipesUsed: \{[^}]*\}|listener: \{.*\}(?=, x)|CookingTimes: \[I;[^\]]*\])')
        # The lit furnace keeps smelting while each game runs: its slots move on.
        volatile_items = re.compile(r'Items: \[[^\]]*\]')
        for label in views0:
            if label.startswith('entity'):
                # Entity data moves with the simulation: only require vanilla to load it.
                if 'falling_block' in label:
                    continue  # lands in Horizon (known gap above)
                if 'No entity was found' in views1.get(label, '') and 'No entity was found' not in views0[label]:
                    print(f'VANILLA cannot find {label} in the Horizon save')
                    failures += 1
                continue
            def stable(text):
                text = volatile2.sub('', volatile.sub('', text))
                return volatile_items.sub('', text) if label == 'block 10 furnace' else text
            if stable(views0[label]) != stable(views1.get(label, '')):
                print(f'VANILLA VIEW differs: {label}\n   V0 {views0[label][:900]}\n   H  {str(views1.get(label))[:900]}')
                failures += 1
    print('save round-trip parity', 'FAILED' if failures else 'ok', f'({failures} unexplained differences)')
    if not args.keep and not failures:
        shutil.rmtree(RUN, ignore_errors=True)
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
