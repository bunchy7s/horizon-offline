#!/usr/bin/env python3
"""Server benchmark: the same bots on Horizon's server and on vanilla 26.3's (docs/server.md, section 9).

Every run starts a fresh server from the same server.properties (seed, view and simulation distance 10,
offline, creative so the bots can fly), waits for `Done`, lets `horizon-bots` join one a second, warms up,
then measures for a while:

- MSPT (average, P50, P95, P99) from each server's own `/tick query`, asked every 5 seconds on the console:
  vanilla's ServerTickRateManager numbers, which Horizon reproduces, so both measure themselves the same way;
- RSS and CPU of the server's whole process tree, once a second;
- chunks the bots receive per second (new terrain while the `explore` bots fly outwards), chunks sent again
  while a bot still held them, and the moves the server refused (pull-backs; "moved too quickly" and
  "moved wrongly" lines);
- the start time to `Done`, and every bot that failed to join or was dropped.

    python3 tools/server_bench.py --servers horizon,vanilla --bots 1,10,25 --behaviour walk,explore

The bots can run on another machine, so they don't take the server's CPU: `--bots-via 'ssh macmini'` with
`--bots-bin` naming horizon-bots there and `--connect` the address it reaches this machine at. `--nice 19`
runs every server at the lowest priority (a shared machine). The explore bots fly as fast as each server
sends them chunks, so a faster server gets more terrain to generate; `--explore-speed` slows them to a rate
both servers keep up with, for equal work. After the runs, the work table compares the servers at each bot
count: chunks sent and new chunks generated a second, against the busiest server's. A run whose work is more
than 10 % apart isn't a fair comparison of its tick times.

The vanilla side runs the pinned server jar from tools/lab.py prepare (your own copy). It needs Java 25. The
Horizon side runs target/release/horizon-server with the engine beside it; build both first:
    cargo build --release -p horizon-client --bin horizon-client -p horizon-server -p horizon-bots

Two more Java servers, both dev-only downloads into .local/server-bench/jars (never committed or shipped):
- `paper`: Paper's newest build for the pinned version, from PaperMC's download API, checked against the
  SHA-256 it publishes, with Paper's own defaults except `connection-throttle` (off: the bots all join from
  one address). Paperclip patches the pinned server jar (your copy, put in
  its cache so nothing is fetched from Mojang).
- `fabric`: the pinned vanilla server on the pinned Fabric loader (tools/benchmark-deps/fabric.lock.json) with Lithium from
  tools/benchmark-deps/mods.lock.json (and Fabric API only if Lithium asks for it), checked against the lock's SHA-512.
Both run with the same heap as vanilla (`--heap`).

`pumpkin` runs Pumpkin (github.com/Pumpkin-MC/Pumpkin, GPL-3.0), a binary you built yourself and name with
`--pumpkin`: a black box, nothing of it is read or shipped. A first start in a scratch folder (on 127.0.0.1
only, without telemetry) writes its default configuration; every run gets that file with the same settings
as the others (seed, view and simulation distance 10, creative, offline, no encryption, 127.0.0.1) and
Pumpkin's Bedrock and LAN listeners, query, plugins and telemetry off. docs/performance/server-vs-pumpkin.md
has the comparison and its commands.
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
import urllib.request
import zipfile

try:
    from . import lab, runtime, test_rust_parity as base
except ImportError:
    import lab
    import runtime
    import test_rust_parity as base

ROOT = lab.ROOT
EXE = '.exe' if os.name == 'nt' else ''
# Where horizon-server and horizon-bots are (the engine is found beside horizon-server or on the PATH).
BIN = Path(os.environ.get('HORIZON_BENCH_BIN', ROOT / 'target/release'))

PROPERTIES = {
    'level-seed': '12345',
    'view-distance': '10',
    'simulation-distance': '10',
    'online-mode': 'false',
    'white-list': 'false',
    'gamemode': 'creative',
    'force-gamemode': 'true',
    'spawn-protection': '0',
    'max-players': '200',
    'difficulty': 'easy',
    'sync-chunk-writes': 'true',
}


def cpu_seconds(text):
    """ps's cumulative CPU time: [[DD-]HH:]MM:SS[.ss]."""
    days, _, rest = text.rpartition('-')
    parts = [float(p) for p in rest.split(':')]
    seconds = 0.0
    for p in parts:
        seconds = seconds * 60 + p
    return seconds + (float(days) * 86400 if days else 0)


def tree_usage(root):
    """RSS (bytes) and CPU seconds of a process and all its descendants."""
    out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,rss=,time='], capture_output=True, text=True).stdout
    rows = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 4:
            rows[int(parts[0])] = (int(parts[1]), int(parts[2]) * 1024, cpu_seconds(parts[3]))
    tree, frontier = {root}, [root]
    while frontier:
        parent = frontier.pop()
        for pid, (ppid, _, _) in rows.items():
            if ppid == parent and pid not in tree:
                tree.add(pid)
                frontier.append(pid)
    rss = sum(rows[p][1] for p in tree if p in rows)
    cpu = sum(rows[p][2] for p in tree if p in rows)
    return rss, cpu


class Server:
    """A server with its console on stdin and its output collected."""

    def __init__(self, command, directory, log_path):
        self.lines = []
        self.lock = threading.Lock()
        self.log = log_path.open('w', encoding='utf-8')
        self.process = subprocess.Popen([str(c) for c in command], cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding='utf-8', errors='replace',
                                        env=lab.game_env())
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.process.stdout:
            with self.lock:
                self.lines.append(line.rstrip('\n'))
            self.log.write(line)
            self.log.flush()

    def count(self):
        with self.lock:
            return len(self.lines)

    def since(self, n):
        with self.lock:
            return self.lines[n:]

    def wait_for(self, pattern, timeout):
        regex = re.compile(pattern)
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(regex.search(l) for l in self.since(0)):
                return
            if self.process.poll() is not None:
                raise RuntimeError('the server exited:\n' + '\n'.join(self.since(0)[-20:]))
            time.sleep(0.1)
        raise RuntimeError(f'no {pattern!r} within {timeout} s')

    def console(self, line):
        self.process.stdin.write(line + '\n')
        self.process.stdin.flush()

    def stop(self):
        if self.process.poll() is None:
            try:
                self.console('stop')
                self.process.wait(timeout=180)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()
        self.log.close()


# The line every server logs once it is up: vanilla, Fabric and Horizon `[time] [Server thread/INFO]: Done (`,
# Paper `[time INFO]: Done (`.
DONE = r'INFO\]: Done \('
# Pumpkin's, once it listens.
PUMPKIN_READY = r'Server is now running|Done \('
TICK_AVERAGE = re.compile(r'Average time per tick: ([0-9.]+) ?ms')
TICK_PERCENTILES = re.compile(r'P50: ([0-9.]+) ?ms P95: ([0-9.]+) ?ms P99: ([0-9.]+) ?ms')


# The newest Paper build for the pinned version, from PaperMC's download API (dev-only, never committed).
PAPER_API = 'https://fill.papermc.io/v3/projects/paper/versions/{version}/builds/latest'
JARS = lab.LOCAL / 'server-bench/jars'


def paper_jar():
    """Paper's newest build for the pinned version, downloaded once and checked against its SHA-256."""
    lock, _ = lab.config()
    version = lock['minecraft']['version']
    request = urllib.request.Request(PAPER_API.format(version=version),
                                     headers={'User-Agent': 'horizon-server-bench (github.com/horizon-rust/horizon)'})
    with urllib.request.urlopen(request, timeout=30) as response:
        build = json.load(response)
    download = build['downloads']['server:default']
    jar = runtime.fetch(download['url'], JARS / download['name'], download['checksums']['sha256'], 'sha256')
    return jar, build['id']


def fabric_mods():
    """Lithium from tools/benchmark-deps/mods.lock.json, plus Fabric API if Lithium depends on it (checked, cached)."""
    lock = json.loads((ROOT / 'tools/benchmark-deps/mods.lock.json').read_text())
    def fetch(name):
        project = lock['projects'][name]
        return runtime.fetch(project['url'], JARS / project['filename'], project['hashes']['sha512'], 'sha512')
    jars = [fetch('lithium')]
    with zipfile.ZipFile(jars[0]) as archive:
        depends = json.loads(archive.read('fabric.mod.json')).get('depends', {})
    if any(d == 'fabric-api' or d.startswith('fabric-') and d != 'fabricloader' for d in depends):
        jars.append(fetch('fabric-api'))
    return jars, {'fabric_loader': lock['fabric_loader'], 'lithium': lock['projects']['lithium']['version']}


# What Pumpkin's first start gets: its own defaults listen on every address (and for Bedrock and LAN
# players) and send telemetry. It fills in the rest of the file.
PUMPKIN_SEED = '''[networking.java]
address = "127.0.0.1:{port}"
online_mode = false
encryption = false

[networking.query]
enabled = false
address = "127.0.0.1:{query}"

[networking.bedrock]
enabled = false

[networking.bedrock.nethernet]
enabled = false
address = "127.0.0.1:0"

[networking.lan_broadcast]
enabled = false

[networking.management]
enabled = false

[plugins]
enabled = false

[telemetry]
enabled = false
endpoint = ""
'''


def pumpkin_defaults(exe, out):
    """Pumpkin's default configuration files (name → text), written by a first start in a scratch folder
    from PUMPKIN_SEED."""
    directory = out / 'pumpkin-defaults'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    (directory / 'pumpkin.toml').write_text(PUMPKIN_SEED.format(port=lab.free_port(), query=lab.free_port()))
    server = Server([exe], directory, out / 'pumpkin-defaults.log')
    try:
        server.wait_for(PUMPKIN_READY, 600)
    finally:
        server.stop()
    files = {p.relative_to(directory): p.read_text() for p in directory.rglob('*.toml')}
    if not any('[networking.java]' in t for t in files.values()):
        raise RuntimeError(f'Pumpkin wrote no [networking.java] configuration in {directory}')
    shutil.rmtree(directory)
    return files


def pumpkin_config(text, port, args):
    """Pumpkin's default configuration with the benchmark's settings (PROPERTIES), listening on 127.0.0.1 only,
    with no Bedrock or LAN listener, query, plugins or telemetry."""
    top = {'seed': f'"{args.seed}"', 'default_difficulty': '"Easy"', 'default_gamemode': '"Creative"',
           'force_gamemode': 'true', 'spawn_protection': '0', 'white_list': 'false', 'use_tty': 'false'}
    sections = {
        'logging': {'color': 'false'},
        'networking.java': {'address': f'"127.0.0.1:{port}"', 'online_mode': 'false', 'encryption': 'false',
                            'max_players': '200', 'view_distance': PROPERTIES['view-distance'],
                            'simulation_distance': PROPERTIES['simulation-distance']},
        'networking.java.authentication': {'enabled': 'false'},
        'networking.java.authentication.textures': {'enabled': 'false'},
        'networking.query': {'enabled': 'false', 'address': f'"127.0.0.1:{port + 1}"'},
        'networking.management': {'enabled': 'false'},
        'networking.bedrock': {'enabled': 'false'},
        'networking.bedrock.nethernet': {'enabled': 'false', 'address': '"127.0.0.1:0"'},
        'networking.lan_broadcast': {'enabled': 'false'},
        'plugins': {'enabled': 'false'},
        'telemetry': {'enabled': 'false', 'endpoint': '""'},
    }
    def setting(body, key, value):
        return re.sub(r'(?m)^' + re.escape(key) + r' = .*$', f'{key} = {value}', body)
    head, _, _ = text.partition('\n[')
    rest = text[len(head):]
    for key, value in top.items():
        head = setting(head, key, value)
    for name, settings in sections.items():
        m = re.search(r'(?ms)^\[' + re.escape(name) + r'\]\n(.*?)(?=^\[|\Z)', rest)
        if not m:
            continue
        body = m.group(1)
        for key, value in settings.items():
            body = setting(body, key, value)
        rest = rest[:m.start(1)] + body + rest[m.end(1):]
    return head + rest


def tick_query(server):
    """Asks `tick query` and reads the average and percentiles it prints (both servers print the same)."""
    start = server.count()
    server.console('tick query')
    deadline = time.time() + 5
    average = percentiles = None
    while time.time() < deadline and (average is None or percentiles is None):
        for line in server.since(start):
            if (m := TICK_AVERAGE.search(line)) and average is None:
                average = float(m.group(1))
            if (m := TICK_PERCENTILES.search(line)) and percentiles is None:
                percentiles = tuple(float(g) for g in m.groups())
        time.sleep(0.05)
    if average is None or percentiles is None:
        return None
    return {'average': average, 'p50': percentiles[0], 'p95': percentiles[1], 'p99': percentiles[2]}


def server_command(kind, directory, args):
    java = ['java', f'-Xms{args.heap}', f'-Xmx{args.heap}', '-Duser.timezone=UTC']
    if kind == 'vanilla':
        _, jars, libraries = base.verify_artifacts()
        classpath = os.pathsep.join(map(str, [jars['server'], *libraries]))
        return [*java, '-cp', classpath, 'net.minecraft.server.Main', 'nogui']
    if kind == 'paper':
        lock, jars, _ = base.verify_artifacts()
        jar, _ = paper_jar()
        # Paperclip's cache, libraries and patched jar are shared by every run; the pinned server jar (your
        # copy) seeds its cache, so it never downloads Mojang's.
        shared = lab.LOCAL / 'server-bench/paper-shared'
        cache = shared / 'cache'
        cache.mkdir(parents=True, exist_ok=True)
        mojang = cache / f'mojang_{lock["minecraft"]["version"]}.jar'
        if not mojang.exists():
            shutil.copyfile(jars['server'].parent / 'server.jar', mojang)
        for name in ('cache', 'libraries', 'versions'):
            (shared / name).mkdir(exist_ok=True)
            (directory / name).symlink_to(shared / name, target_is_directory=True)
        # Paper refuses a second login from one address within 4 s (bukkit.yml's connection-throttle), and
        # every bot comes from the same one. Off, as on vanilla; Paper fills in the rest of the file.
        (directory / 'bukkit.yml').write_text('settings:\n  connection-throttle: -1\n')
        return [*java, '-jar', jar, '--nogui']
    if kind == 'fabric':
        _, jars, libraries = base.verify_artifacts()
        mods = directory / 'mods'
        mods.mkdir()
        for jar in fabric_mods()[0]:
            shutil.copyfile(jar, mods / jar.name)
        classpath = os.pathsep.join(map(str, [jars['server'], *libraries, *runtime.fabric_libraries()]))
        return [*java, '-Dfabric.log.disableAnsi=true', '-cp', classpath,
                'net.fabricmc.loader.impl.launch.knot.KnotServer', 'nogui']
    if kind == 'pumpkin':
        if not args.pumpkin:
            raise RuntimeError('--pumpkin names the Pumpkin binary to run')
        if args.pumpkin_files is None:
            args.pumpkin_files = pumpkin_defaults(args.pumpkin, directory.parent)
        for name, text in args.pumpkin_files.items():
            (directory / name).parent.mkdir(parents=True, exist_ok=True)
            (directory / name).write_text(pumpkin_config(text, args.port, args) if '[networking.java]' in text else text)
        return [args.pumpkin]
    server = BIN / f'horizon-server{EXE}'
    subprocess.run([server, '--dir', directory, 'import', directory / 'server.properties'], check=True,
                   capture_output=True)
    toml = directory / 'server.toml'
    # The same settings, without the web panel.
    toml.write_text(re.sub(r'(?m)^bind = "127\.0\.0\.1:8130"', 'bind = ""', toml.read_text()))
    if args.anticheat:
        # Every check on with its defaults (docs/anticheat.md), to measure what it costs. The bots move
        # by their own rules, not vanilla's: the prediction plays every move again but, with this
        # tolerance, flags none (an honest player's moves agree).
        toml.write_text(toml.read_text()
                        + '\n[anticheat]\nenabled = true\n[anticheat.prediction]\ntolerance = 1000.0\n')
    else:
        # The anticheat is on by default: off, to measure the server without it.
        toml.write_text(toml.read_text() + '\n[anticheat]\nenabled = false\n')
    return [server, '--dir', directory, 'run']


def flag_count(directory):
    """The lines of Horizon's anticheat log: one a flag (none when it is off)."""
    log = directory / 'logs/anticheat.log'
    return len(log.read_text().splitlines()) if log.exists() else 0


def bots_command(args):
    """horizon-bots here, or through `--bots-via` (for example `ssh macmini`) on another machine."""
    if args.bots_via:
        return [*args.bots_via.split(), args.bots_bin or 'horizon-bots']
    return [args.bots_bin or BIN / f'horizon-bots{EXE}']


def run_once(kind, bots, behaviour, args, out):
    name = f'{kind}-{behaviour}-{bots}'
    directory = out / name
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    port = args.port = lab.free_port()
    props = dict(PROPERTIES, **{'server-port': str(port), 'level-seed': str(args.seed)})
    if args.ground:
        props.update({'level-type': 'minecraft\\:flat', 'generate-structures': 'false', 'gamemode': 'survival'})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    command = server_command(kind, directory, args)
    if args.nice:
        command = ['nice', '-n', str(args.nice), *command]
    started = time.time()
    server = Server(command, directory, out / f'{name}.log')
    result = {'server': kind, 'bots': bots, 'behaviour': behaviour}
    try:
        server.wait_for(PUMPKIN_READY if kind == 'pumpkin' else DONE, 600)
        result['start_seconds'] = round(time.time() - started, 2)
        # Every bot appears on the world spawn itself, not spread around it, so both servers see the same paths.
        server.console('gamerule respawn_radius 0')
        for rule in filter(None, args.gamerule.split(',')):
            server.console('gamerule ' + rule.replace('=', ' '))
        time.sleep(5)
        report = out / f'{name}-bots.json'
        total = bots * args.join_every + args.warmup + args.seconds
        # The summary comes back on stdout after the per-second lines, so it works over ssh too.
        # No bots: the idle server, measured the same way.
        bot_process = subprocess.Popen(['true'] if bots == 0 else [
             *bots_command(args), '--server', f'{args.connect}:{port}', '--bots', str(bots),
             '--behaviour', behaviour, '--join-every', str(args.join_every), '--seconds', str(int(total)),
             '--seed', str(args.seed), '--report', '-',
             *(['--explore-speed', str(args.explore_speed)] if args.explore_speed else []),
             *(['--height', '0', '--ground'] if args.ground else [])],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        bot_lines = []
        threading.Thread(target=lambda: bot_lines.extend(bot_process.stdout), daemon=True).start()
        # All bots in, then the warm-up.
        time.sleep(bots * args.join_every + args.warmup)
        measure_start = time.time()
        rss, cpu, ticks = [], [], []
        _, last_cpu = tree_usage(server.process.pid)
        last_time = time.time()
        next_query = time.time()
        while time.time() - measure_start < args.seconds:
            time.sleep(1)
            now = time.time()
            r, c = tree_usage(server.process.pid)
            rss.append(r)
            cpu.append((c - last_cpu) / (now - last_time) * 100)
            last_cpu, last_time = c, now
            if now >= next_query:
                if (t := tick_query(server)) is not None:
                    ticks.append(t)
                next_query = now + 5
        bot_process.wait(timeout=total + 120)
        time.sleep(0.5)
        measured, rest = [], []
        for line in list(bot_lines):
            try:
                sample = json.loads(line)
            except ValueError:
                sample = None
            if isinstance(sample, dict) and 't' in sample:
                measured.append(sample)
            else:
                rest.append(line)
        window = [l for l in measured if l['t'] >= measured[-1]['t'] - args.seconds] if measured else []
        summary_text = ''.join(rest)
        summary = json.loads(summary_text) if summary_text.strip() else {}
        report.write_text(json.dumps(summary, indent=2) + '\n')
        mean = lambda xs: round(statistics.fmean(xs), 2) if xs else None
        per_bot = summary.get('per_bot', [])
        by_behaviour = {}
        for b in per_bot:
            by_behaviour.setdefault(b['behaviour'], []).append(b)
        result.update({
            'mspt': {k: mean([t[k] for t in ticks]) for k in ('average', 'p50', 'p95', 'p99')},
            'mspt_worst_p99': max((t['p99'] for t in ticks), default=None),
            # /tick query reports times, not the rate: 20 while the average fits in 50 ms.
            'tps': round(min(20.0, 1000 / max(statistics.fmean([t['average'] for t in ticks]), 1e-9)), 2) if ticks else None,
            'rss_mb': {'mean': mean([r / 1e6 for r in rss]), 'max': round(max(rss) / 1e6, 1) if rss else None},
            'cpu_percent': mean(cpu),
            'chunks_per_second': mean([l['chunks_per_second'] for l in window]),
            # Over the measured window: different chunks reaching the bots, chunks they had dropped and got
            # again (walking back), and ticks the bots stood still waiting for their own chunk.
            **({k + '_per_second': round((window[-1][k] - window[0][k]) / max(len(window) - 1, 1), 2)
                for k in ('unique', 'forgotten', 'waited_ticks')} if len(window) > 1 else {}),
            'chunks_total': summary.get('chunks'),
            # Per bot over the whole run: every chunk received, the different ones, and those sent again
            # while the bot still held them.
            'chunks_per_bot': {k: mean([b['chunks'] for b in v]) for k, v in by_behaviour.items()},
            'unique_chunks_per_bot': {k: mean([b['unique_chunks'] for b in v]) for k, v in by_behaviour.items()},
            'resent': summary.get('resent'),
            'pulled_back': summary.get('pulled_back'),
            'moved_too_quickly': sum('moved too quickly!' in l for l in server.since(0)),
            'anticheat_flags': flag_count(directory),
            'moved_wrongly': sum('moved wrongly!' in l for l in server.since(0)),
            'bots_joined': summary.get('joined'),
            # From the bot's first packet to its spawn position, in ms.
            'join_ms': summary.get('join_time'),
            'bots_failed': [b for b in summary.get('per_bot', []) if b.get('error')],
            'tick_queries': len(ticks),
        })
    except Exception as e:  # a failed run is reported, and the next one still runs
        result['error'] = str(e)
    finally:
        server.stop()
    return result


# How far apart two servers' chunks a second (sent, and new) may be and still count as the same work: 10 %,
# or a chunk a second when there are few.
EQUAL_WORK = 0.10


def work(runs):
    """Each pass and bot count that ran on more than one server: every server's chunks sent and new chunks a
    second as a share of the busiest one's, the ticks a second its bots stood waiting for their own chunk,
    and whether the work was equal (within EQUAL_WORK on both)."""
    groups = {}
    for r in runs:
        if 'error' not in r:
            groups.setdefault((r['behaviour'], r['bots'], r.get('round', 0)), []).append(r)
    rows = []
    for (behaviour, bots, round_), group in groups.items():
        if len(group) < 2:
            continue
        row = {'behaviour': behaviour, 'bots': bots, 'round': round_, 'servers': {}, 'equal': True}
        for key in ('chunks_per_second', 'unique_per_second'):
            top = max(r.get(key) or 0 for r in group)
            for r in group:
                value = r.get(key) or 0
                share = round(value / top, 3) if top else 1.0
                row['servers'].setdefault(r['server'], {})[key] = {'value': value, 'share': share}
                if top - value > max(EQUAL_WORK * top, 1.0):
                    row['equal'] = False
        for r in group:
            row['servers'][r['server']]['waited_ticks_per_second'] = r.get('waited_ticks_per_second')
        rows.append(row)
    return rows


def versions(kinds):
    """The Paper build and the Fabric loader and Lithium versions that ran, for the report."""
    out = {}
    if 'paper' in kinds:
        out['paper_build'] = paper_jar()[1]
    if 'fabric' in kinds:
        out.update(fabric_mods()[1])
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--servers', default='horizon,vanilla', help='horizon, vanilla, paper, fabric (Lithium), pumpkin')
    parser.add_argument('--bots', default='1,10,25,50')
    parser.add_argument('--behaviour', default='walk,explore', help='one pass per behaviour (comma-separated)')
    parser.add_argument('--join-every', type=float, default=1.0)
    parser.add_argument('--warmup', type=int, default=60)
    parser.add_argument('--seconds', type=int, default=300)
    parser.add_argument('--seed', type=int, default=12345)
    parser.add_argument('--explore-speed', type=float, default=0,
                        help='blocks a second the explore bots fly (default: the bots\' 22, as fast as chunks '
                             'come); a speed both servers keep up with makes the explore pass equal work')
    parser.add_argument('--gamerule', default='',
                        help="more game rules for both servers, 'name=value,...' (locator_bar=false: Horizon has "
                             "no locator bar yet)")
    parser.add_argument('--heap', default='4g', help='the Java heap, -Xms and -Xmx (vanilla, Paper, Fabric)')
    parser.add_argument('--out', type=Path, default=None, help='the report (JSON)')
    parser.add_argument('--nice', type=int, default=0, help='run the servers under nice -n N')
    parser.add_argument('--anticheat', action='store_true',
                        help="turn Horizon's anticheat on (server.toml [anticheat]); its flags are counted")
    parser.add_argument('--ground', action='store_true',
                        help='a flat survival world, the bots walking on its ground and saying so in every move '
                             "(what the anticheat's ground check judges)")
    parser.add_argument('--bots-via', default='', help="a command prefix to run the bots elsewhere ('ssh host')")
    parser.add_argument('--bots-bin', default='', help='horizon-bots (on the --bots-via machine)')
    parser.add_argument('--connect', default='127.0.0.1', help='the address the bots reach the server at')
    parser.add_argument('--pumpkin', type=Path, default=None, help='the Pumpkin binary (--servers pumpkin)')
    parser.add_argument('--repeat', type=int, default=1,
                        help='rounds: every server and bot count once per round, the servers alternated')
    args = parser.parse_args()
    args.pumpkin_files = None
    out = lab.LOCAL / 'server-bench'
    out.mkdir(parents=True, exist_ok=True)
    runs = []
    for round_ in range(args.repeat):
        for behaviour in args.behaviour.split(','):
            for bots in [int(b) for b in args.bots.split(',')]:
                for kind in args.servers.split(','):
                    print(f'{kind}, {bots} {behaviour} bots...', flush=True)
                    result = run_once(kind, bots, behaviour, args, out)
                    result['round'] = round_
                    runs.append(result)
                    print(json.dumps(result), flush=True)
    report = {
        'kind': 'horizon-server-benchmark',
        'date': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'machine': {'system': platform.system(), 'machine': platform.machine(), 'cpus': os.cpu_count()},
        'commit': subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip(),
        'java': subprocess.run(['java', '--version'], capture_output=True, text=True).stdout.splitlines()[:1],
        'versions': versions(args.servers.split(',')),
        'settings': {**{k: v for k, v in vars(args).items() if k != 'pumpkin_files'}, 'out': str(args.out),
                     'pumpkin': str(args.pumpkin) if args.pumpkin else None, 'properties': PROPERTIES},
        'runs': runs,
        'work': work(runs),
    }
    path = args.out or out / 'report.json'
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2) + '\n')
    print(f'\n{"server":8} {"pass":8} {"bots":>4} {"start s":>8} {"MSPT":>6} {"P95":>6} {"P99":>6} {"RSS MB":>7} '
          f'{"CPU %":>6} {"chunks/s":>8} {"/bot":>6} {"resent":>6} {"pulled":>6} {"joined":>6} {"join ms":>7}')
    for r in runs:
        if 'error' in r:
            print(f'{r["server"]:8} {r["behaviour"]:8} {r["bots"]:>4}  failed: {r["error"].splitlines()[0]}')
            continue
        # A mixed pass: every bot's.
        per_bot = r['chunks_per_bot'].get(r['behaviour']) or round((r['chunks_total'] or 0) / max(r['bots'], 1))
        print(f'{r["server"]:8} {r["behaviour"]:8} {r["bots"]:>4} {r["start_seconds"]:>8} {r["mspt"]["average"]!s:>6} '
              f'{r["mspt"]["p95"]!s:>6} {r["mspt"]["p99"]!s:>6} {r["rss_mb"]["mean"]!s:>7} {r["cpu_percent"]!s:>6} '
              f'{r["chunks_per_second"]!s:>8} {per_bot!s:>6} {r["resent"]!s:>6} {r["pulled_back"]!s:>6} '
              f'{r["bots_joined"]!s:>6} {(r.get("join_ms") or {}).get("p50_ms")!s:>7}')
    if report['work']:
        print(f'\nWork, against the busiest server (chunks a second sent, new; ticks a second bots waited):')
        for row in report['work']:
            cells = '  '.join(
                f'{server} {w["chunks_per_second"]["value"]} ({w["chunks_per_second"]["share"]:.0%}), '
                f'{w["unique_per_second"]["value"]} ({w["unique_per_second"]["share"]:.0%}), '
                f'{w["waited_ticks_per_second"]}' for server, w in row['servers'].items())
            print(f'{row["behaviour"]:8} {row["bots"]:>4}  {"equal" if row["equal"] else "NOT EQUAL"}  {cells}')
    print(f'\nWrote {path}')


if __name__ == '__main__':
    sys.exit(main())
