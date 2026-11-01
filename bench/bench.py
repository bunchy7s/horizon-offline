#!/usr/bin/env python3
"""Server load bench: the same horizon-bots load on Horizon, vanilla 26.3 or Pumpkin.

    python3 bench/bench.py --server horizon --exe target/release/horizon-server --bots 50
    python3 bench/bench.py --server vanilla --jar ~/mc/server.jar --bots 50 --pre-generated ~/worlds/seed12345
    python3 bench/bench.py --server pumpkin --exe ./pumpkin --bots 50

Every run uses a fresh scratch folder (bound to 127.0.0.1, offline, creative so the bots can fly), starts the
server, lets `horizon-bots` join one every --join-every seconds, warms up, then measures for --seconds:

- TPS: `time query gametime` at the start and the end of the window, as game ticks per wall-clock second;
- MSPT: `tick query` every 5 s (average, P50, P95, P99 over the last 100 ticks), averaged over the window;
- RSS and CPU of the server's whole process tree, once a second (needs `ps`: Linux and macOS);
- bandwidth: `bytes_in`/`bytes_out` from the bots' report, per player and second online (each bot's whole time
  on the server, its join included), when the bots report them (they do not yet: the field is null until S-6); join time: the bots' `joined_after_ms`.

It writes one JSON report (--out) and prints the Markdown table of the design (docs/design.md, section 1).

Vanilla is the owner's own server.jar (--jar): nothing of Mojang's is fetched or committed. Pumpkin is a released
binary run as a black box (--exe); its Bedrock listeners, LAN broadcast and telemetry are turned off in the
config this script writes (--pumpkin-config adds a TOML file of your own on top if a release names a key
differently). `--pre-generated DIR` copies a finished world into every run's scratch folder as `world`, so runs
repeat on the same terrain. Build the bots first: cargo build --release -p horizon-bots
"""

import argparse
import datetime
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parent.parent
EXE = '.exe' if os.name == 'nt' else ''
SERVERS = ('horizon', 'vanilla', 'pumpkin')

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
    'server-ip': '127.0.0.1',
    'enable-query': 'false',
    'enable-rcon': 'false',
}

# Pumpkin's two config files. Bedrock, LAN broadcast and telemetry (plugins' included) are off.
PUMPKIN_CONFIGURATION = '''\
server_address = "127.0.0.1:{port}"
seed = "{seed}"
max_players = 200
view_distance = {view}
simulation_distance = {view}
online_mode = false
default_difficulty = "Easy"
allow_chat_reports = false
encryption = false
lan_broadcast = false

[networking.bedrock]
enabled = false

[networking.lan_broadcast]
enabled = false

[networking.query]
enabled = false

[networking.rcon]
enabled = false
'''
PUMPKIN_FEATURES = '''\
[telemetry]
enabled = false

[plugins]
enabled = false
'''

DONE = re.compile(r'Done|listening|Server started', re.IGNORECASE)
TICK_AVERAGE = re.compile(r'Average time per tick: ([0-9.]+) ?ms')
TICK_PERCENTILES = re.compile(r'P50: ([0-9.]+) ?ms P95: ([0-9.]+) ?ms P99: ([0-9.]+) ?ms')
GAMETIME = re.compile(r'The time is (-?\d+)')


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def parse_tick_query(lines):
    """MSPT numbers from the lines `tick query` printed, or None until both lines are there."""
    average = percentiles = None
    for line in lines:
        if (m := TICK_AVERAGE.search(line)) and average is None:
            average = float(m.group(1))
        if (m := TICK_PERCENTILES.search(line)) and percentiles is None:
            percentiles = tuple(float(g) for g in m.groups())
    if average is None or percentiles is None:
        return None
    return {'average': average, 'p50': percentiles[0], 'p95': percentiles[1], 'p99': percentiles[2]}


def parse_gametime(lines):
    """The game time `time query gametime` printed, or None."""
    for line in lines:
        if m := GAMETIME.search(line):
            return int(m.group(1))
    return None


def tps(start_ticks, end_ticks, seconds):
    """Game ticks per wall-clock second over the window, or None when a query went unanswered."""
    if start_ticks is None or end_ticks is None or seconds <= 0:
        return None
    return round((end_ticks - start_ticks) / seconds, 2)


def percentile(values, p):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(p / 100 * len(ordered)))]


def cpu_seconds(text):
    """ps's cumulative CPU time: [[DD-]HH:]MM:SS[.ss]."""
    days, _, rest = text.rpartition('-')
    seconds = 0.0
    for part in rest.split(':'):
        seconds = seconds * 60 + float(part)
    return seconds + (float(days) * 86400 if days else 0)


def tree_usage(root):
    """RSS (bytes) and CPU seconds of a process and all its descendants; None without `ps`."""
    try:
        out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,rss=,time='], capture_output=True, text=True).stdout
    except OSError:
        return None
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
    return sum(rows[p][1] for p in tree if p in rows), sum(rows[p][2] for p in tree if p in rows)


def prepare(kind, directory, port, args):
    """Writes the server's settings into its scratch folder and returns the command that starts it."""
    directory = Path(directory)
    if args.pre_generated:
        shutil.copytree(args.pre_generated, directory / 'world')
    props = dict(PROPERTIES, **{'server-port': str(port), 'level-seed': str(args.seed),
                                'view-distance': str(args.view_distance),
                                'simulation-distance': str(args.view_distance)})
    if kind == 'pumpkin':
        config = directory / 'config'
        config.mkdir()
        (config / 'configuration.toml').write_text(PUMPKIN_CONFIGURATION.format(
            port=port, seed=args.seed, view=args.view_distance))
        (config / 'features.toml').write_text(PUMPKIN_FEATURES)
        if args.pumpkin_config:
            with (config / 'configuration.toml').open('a') as f:
                f.write('\n' + Path(args.pumpkin_config).read_text())
        return [Path(args.exe).resolve()]
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    if kind == 'vanilla':
        return ['java', f'-Xms{args.heap}', f'-Xmx{args.heap}', '-jar', Path(args.jar).resolve(), 'nogui']
    server = Path(args.exe).resolve()
    subprocess.run([server, '--dir', directory, 'import', directory / 'server.properties'], check=True,
                   capture_output=True)
    toml = directory / 'server.toml'
    # No web panel in a bench.
    toml.write_text(re.sub(r'(?m)^bind = "127\.0\.0\.1:8130"', 'bind = ""', toml.read_text()))
    return [server, '--dir', directory, 'run']


class Server:
    """A server with its console on stdin and its output collected."""

    def __init__(self, command, directory, log_path):
        self.lines = []
        self.lock = threading.Lock()
        self.log = log_path.open('w', encoding='utf-8')
        env = dict(os.environ, HORIZON_NO_AUDIO='1', HORIZON_HOME=str(directory / 'horizon-home'),
                   HORIZON_LAUNCHER_NO_SELF_UPDATE='1')
        self.process = subprocess.Popen([str(c) for c in command], cwd=directory, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                        encoding='utf-8', errors='replace', env=env)
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

    def wait_for(self, regex, timeout):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if any(regex.search(line) for line in self.since(0)):
                return
            if self.process.poll() is not None:
                raise RuntimeError('the server exited:\n' + '\n'.join(self.since(0)[-20:]))
            time.sleep(0.1)
        raise RuntimeError(f'the server was not up within {timeout} s')

    def ask(self, command, parse, timeout=5):
        """Sends a console command and returns parse(its output lines), or None if nothing parsed in time."""
        start = self.count()
        try:
            self.process.stdin.write(command + '\n')
            self.process.stdin.flush()
        except (BrokenPipeError, OSError):
            return None
        deadline = time.time() + timeout
        while time.time() < deadline:
            if (value := parse(self.since(start))) is not None:
                return value
            time.sleep(0.05)
        return None

    def stop(self):
        if self.process.poll() is None:
            try:
                self.process.stdin.write('stop\n')
                self.process.stdin.flush()
                self.process.wait(timeout=60)
            except (BrokenPipeError, OSError, subprocess.TimeoutExpired):
                self.process.kill()
                self.process.wait()
        self.log.close()


def bots_command(args, port, total):
    return [str(args.bots_bin), '--server', f'127.0.0.1:{port}', '--bots', str(args.bots),
            '--behaviour', args.behaviour, '--join-every', str(args.join_every), '--seconds', str(int(total)),
            '--seed', str(args.seed), '--view-distance', str(args.view_distance), '--report', '-']


def split_bot_output(lines):
    """The per-second samples (JSON lines with a `t`) and the final summary the bots printed."""
    samples, rest = [], []
    for line in lines:
        try:
            sample = json.loads(line)
        except ValueError:
            sample = None
        if isinstance(sample, dict) and 't' in sample:
            samples.append(sample)
        else:
            rest.append(line)
    text = ''.join(rest)
    return samples, (json.loads(text) if text.strip() else {})


def bytes_per_player_second(per_bot, run_seconds, join_every):
    """The bots' bytes over the seconds they were connected: bot i connects i * join_every after the first and
    stays to the end of the run. So it is each bot's average from its login on, its join included, not the
    measuring window's alone (the bots report one total at the end). None until the bots count bytes (S-6)."""
    counted = [(i, b) for i, b in enumerate(per_bot) if 'bytes_in' in b and not b.get('error')]
    online = sum(max(run_seconds - i * join_every, 0) for i, _ in counted)
    if not counted or online <= 0:
        return None
    return {k: round(sum(b.get(k, 0) for _, b in counted) / online) for k in ('bytes_in', 'bytes_out')}


def summarize(kind, args, ticks, rss, cpu, gametime, window_seconds, summary, start_seconds, run_seconds):
    mean = lambda xs: round(statistics.fmean(xs), 2) if xs else None
    joins = [b['joined_after_ms'] for b in summary.get('per_bot', []) if b.get('joined_after_ms', 0) > 0]
    bandwidth = bytes_per_player_second(summary.get('per_bot', []), run_seconds, args.join_every)
    return {
        'server': kind,
        'bots': args.bots,
        'behaviour': args.behaviour,
        'start_seconds': start_seconds,
        'tps': tps(*gametime, window_seconds),
        'mspt': {k: mean([t[k] for t in ticks]) for k in ('average', 'p50', 'p95', 'p99')},
        'mspt_worst_p99': max((t['p99'] for t in ticks), default=None),
        'rss_mb': {'mean': mean([r / 1e6 for r in rss]), 'max': round(max(rss) / 1e6, 1) if rss else None},
        'cpu_percent': mean(cpu),
        'bytes_per_player_second': bandwidth,
        'join_ms': {'p50': percentile(joins, 50), 'worst': max(joins, default=None)},
        'bots_joined': summary.get('joined'),
        'bots_failed': [b for b in summary.get('per_bot', []) if b.get('error')],
        'chunks_total': summary.get('chunks'),
        'tick_queries': len(ticks),
    }


def run_once(kind, args, out):
    scratch = Path(tempfile.mkdtemp(prefix=f'horizon-bench-{kind}-'))
    result = {'server': kind, 'bots': args.bots, 'behaviour': args.behaviour}
    server = None
    try:
        port = free_port()
        command = prepare(kind, scratch, port, args)
        started = time.time()
        server = Server(command, scratch, out / f'{kind}-{args.bots}.log')
        server.wait_for(DONE, args.start_timeout)
        start_seconds = round(time.time() - started, 2)
        total = args.bots * args.join_every + args.warmup + args.seconds
        bot_process = subprocess.Popen(bots_command(args, port, total), stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True)
        bot_lines = []
        threading.Thread(target=lambda: bot_lines.extend(bot_process.stdout), daemon=True).start()
        time.sleep(args.bots * args.join_every + args.warmup)
        first_time = server.ask('time query gametime', parse_gametime)
        began = time.time()
        rss, cpu, ticks = [], [], []
        usage = tree_usage(server.process.pid)
        last_cpu, last_time = (usage[1] if usage else 0), time.time()
        next_query = time.time()
        while time.time() - began < args.seconds:
            time.sleep(1)
            now = time.time()
            if usage and (u := tree_usage(server.process.pid)):
                rss.append(u[0])
                cpu.append((u[1] - last_cpu) / (now - last_time) * 100)
                last_cpu, last_time = u[1], now
            if now >= next_query:
                if (t := server.ask('tick query', parse_tick_query)) is not None:
                    ticks.append(t)
                next_query = now + 5
        window = time.time() - began
        last = server.ask('time query gametime', parse_gametime)
        bot_process.wait(timeout=total + 120)
        time.sleep(0.5)
        _, summary = split_bot_output(list(bot_lines))
        result.update(summarize(kind, args, ticks, rss, cpu, (first_time, last), window, summary, start_seconds,
                                total))
    except Exception as e:  # a failed run is reported, and the next one still runs
        result['error'] = str(e)
    finally:
        if server:
            server.stop()
        shutil.rmtree(scratch, ignore_errors=True)
    return result


def table(runs):
    """The Markdown table of the design: one column per server, one row per measure."""
    ok = [r for r in runs if 'error' not in r]
    head = '| | ' + ' | '.join(r['server'] for r in ok) + ' |\n|---|' + '---|' * len(ok) + '\n'
    def cell(r, path):
        value = r
        for key in path:
            value = value.get(key) if isinstance(value, dict) else None
        return '-' if value is None else str(value)
    rows = [
        ('Players', ('bots',)),
        ('TPS', ('tps',)),
        ('MSPT average', ('mspt', 'average')),
        ('MSPT p99 (mean of 5 s queries)', ('mspt', 'p99')),
        ('MSPT p99 (worst query)', ('mspt_worst_p99',)),
        ('RSS mean MB', ('rss_mb', 'mean')),
        ('CPU %', ('cpu_percent',)),
        ('Bytes in per player and s online', ('bytes_per_player_second', 'bytes_in')),
        ('Join p50 ms', ('join_ms', 'p50')),
        ('Join worst ms', ('join_ms', 'worst')),
        ('Start s', ('start_seconds',)),
    ]
    body = ''.join(f'| {name} | ' + ' | '.join(cell(r, path) for r in ok) + ' |\n' for name, path in rows)
    failed = ''.join(f'\n{r["server"]} failed: {r["error"].splitlines()[0]}\n' for r in runs if 'error' in r)
    return head + body + failed


def check(args):
    for kind in args.server:
        if kind not in SERVERS:
            sys.exit(f'--server: {kind!r} is not one of {", ".join(SERVERS)}')
    if 'vanilla' in args.server and not args.jar:
        sys.exit('vanilla needs --jar, your own server.jar')
    if any(k != 'vanilla' for k in args.server) and not args.exe:
        sys.exit('horizon and pumpkin need --exe, the server binary')
    if not Path(args.bots_bin).exists():
        sys.exit(f'{args.bots_bin} is missing: cargo build --release -p horizon-bots')
    if args.pre_generated and not Path(args.pre_generated).is_dir():
        sys.exit(f'--pre-generated {args.pre_generated} is not a folder')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--server', type=lambda s: s.split(','), default=['horizon'],
                        help='horizon, vanilla or pumpkin (comma-separated, one run each)')
    parser.add_argument('--exe', help='the horizon-server or Pumpkin binary')
    parser.add_argument('--jar', help="vanilla's server.jar (your own copy)")
    parser.add_argument('--bots-bin', default=str(ROOT / 'target/release' / f'horizon-bots{EXE}'))
    parser.add_argument('--bots', type=int, default=50)
    parser.add_argument('--behaviour', default='mixed')
    parser.add_argument('--join-every', type=float, default=0.5)
    parser.add_argument('--warmup', type=int, default=30)
    parser.add_argument('--seconds', type=int, default=60)
    parser.add_argument('--seed', type=int, default=12345)
    parser.add_argument('--view-distance', type=int, default=10)
    parser.add_argument('--heap', default='4g', help='the Java heap for vanilla')
    parser.add_argument('--pre-generated', help='a finished world folder, copied into every run as `world`')
    parser.add_argument('--pumpkin-config', help='extra TOML appended to Pumpkin\'s configuration.toml')
    parser.add_argument('--start-timeout', type=int, default=600)
    parser.add_argument('--out', type=Path, default=ROOT / '.local/bench', help='the folder for report.json and logs')
    args = parser.parse_args(argv)
    check(args)
    args.out.mkdir(parents=True, exist_ok=True)
    runs = []
    for kind in args.server:
        print(f'{kind}, {args.bots} {args.behaviour} bots...', flush=True)
        runs.append(run_once(kind, args, args.out))
        print(json.dumps(runs[-1]), flush=True)
    report = {
        'kind': 'horizon-bench',
        'date': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
        'machine': {'system': platform.system(), 'machine': platform.machine(), 'cpus': os.cpu_count()},
        'settings': {k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()},
        'runs': runs,
    }
    path = args.out / 'report.json'
    path.write_text(json.dumps(report, indent=2) + '\n')
    print('\n' + table(runs) + f'\nWrote {path}')
    return 1 if any('error' in r for r in runs) else 0


if __name__ == '__main__':
    sys.exit(main())
