#!/usr/bin/env python3
"""Compare a new world's spawn with vanilla's, seed by seed.

A new overworld's spawn comes from two steps (MinecraftServer.setInitialSpawn):

1. the origin: NoiseBasedChunkGenerator.getOrigin runs NoiseSpawnFinder over
   the noise settings' `spawn_target` points (a coarse then a fine radial
   search of the climate fields) and takes the chunk of the best column;
2. the spawn: an 11 by 11 chunk spiral around that chunk, the first chunk
   with a column PlayerSpawnFinder.getSpawnPosInChunk accepts (a full-topped
   block under open sky, no liquid on top) gives the world spawn.

Origins: VanillaWorldSpawnOracle calls NoiseSpawnFinder.findSpawnPosition and
getOrigin for every seed; the spawn_origin example asks Horizon's generator.
World spawns: the pinned vanilla dedicated server creates each world and
stops; Horizon's (`horizon-client --dedicated --nogui`) creates its own from
the same server.properties and stops; the `spawn` position of both
level.dat files is compared.

Trees of neighbouring chunks overlap on chunk corners, and which one is
placed first depends on the order the chunks are decorated. With several
worker threads vanilla's order changes from run to run (seed
-2080727421395798566 spawned at Y 64, 66 or 72), so its server runs with one
(`-Dmax.bg.threads`, Util.maxAllowedExecutorThreads): the chunks are then
decorated in its scheduler's order, the one Horizon follows
(decoration_order::vanilla_order), and every run gives the same spawn.

Seeds: the player report's seed (1891623539983467248: Horizon spawned at
-48 -48, vanilla at 152 145), the chunk generation harness's seeds and
--random seeds from a fixed generator.

tools/parity.py runs it in PARITY_SHARDS parts, each creating the worlds of
every PARITY_SHARDS-th seed: each seed is a vanilla and a Horizon server start,
~15 s, and 60 seeds in one run were the longest harness (~25 min).

Under tools/parity.py's oracle reuse (not --fresh or --all) a seed's vanilla
spawn is kept and reused: vanilla's server is ~11 s of a seed's ~13 s, and its
spawn depends only on what the key holds (see vanilla_cache). tools/parity_shim.py
can't do it: a server's console is a pipe, and it never caches those runs.

usage: test_world_spawn_parity.py [--random N] [--jobs N] [--origins-only]
"""
import argparse
import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

try:
    from . import anvil_dump, lab, oracle_harness as oh, parity_shard, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
except ImportError:
    import anvil_dump
    import lab
    import oracle_harness as oh
    import parity_shard
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port

NAME = 'world-spawn-parity'
PARITY_SHARDS = 4
REPORT = 1891623539983467248
SEEDS = [REPORT, 12345, -7046029254386353131, 987654321987, 4242, -1, 0x5DEECE66D, 20260607, 777, 0]
RESOURCES = lab.RESOURCES


def seeds(count):
    rng = random.Random(20261014)
    return SEEDS + [rng.randrange(-2**63, 2**63) for _ in range(count)]


def target_dir(profile='release'):
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    return Path(metadata['target_directory']) / profile


def origins(build, classes, dependencies, all_seeds):
    vanilla_out = build / 'origins-vanilla.txt'
    oh.run_logged(['java', '-cp', str(classes) + os.pathsep + dependencies, 'dev.horizonlab.parity.VanillaWorldSpawnOracle',
                   vanilla_out, *all_seeds], build, 'origins-vanilla')
    rust = subprocess.run([target_dir() / 'examples/world_probe', 'spawn_origin', RESOURCES, *map(str, all_seeds)],
                          capture_output=True, text=True, check=True, cwd=lab.ROOT).stdout
    vanilla = vanilla_out.read_text()
    parse = lambda text: {int(l.split()[0]): tuple(map(int, l.split()[1:])) for l in text.splitlines() if l}
    return parse(vanilla), parse(rust)


def properties(seed):
    """server.properties of a seed's world, without the port."""
    return {'online-mode': 'false', 'level-seed': str(seed), 'view-distance': '4', 'simulation-distance': '4',
            'pause-when-empty-seconds': '-1'}


def vanilla_cache(lock, command, classpath):
    """A function giving the file that keeps a seed's vanilla spawn, or None when every vanilla world is created.

    Only under tools/parity.py's oracle reuse (PARITY_SHIM_CACHE; a harness failing on reused results is rerun
    without it). The key holds what vanilla's spawn depends on: the server jar (verify_artifacts checked it and its
    libraries against the lock), the JDK, the server's command line and the seed's server.properties."""
    root = os.environ.get('PARITY_SHIM_CACHE')
    if not root or os.environ.get('PARITY_JAVA_CACHE') == '0':
        return None
    java = subprocess.run(['java', '-version'], capture_output=True, text=True, check=True).stderr
    server = {'jar': lock['minecraft']['server_sha1'], 'java': java,
              'command': [part for part in command if part != classpath]}

    def record(seed):
        identity = json.dumps({**server, 'properties': properties(seed)}, sort_keys=True)
        return Path(root) / f'world-spawn-{hashlib.sha256(identity.encode()).hexdigest()}.json'
    return record


def world_spawn(command, directory, seed, log_name):
    """Creates a world with `seed` in `directory`, stops, returns its level.dat spawn position."""
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    props = {**properties(seed), 'server-port': str(free_port())}
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()), encoding='iso-8859-1')
    (directory / 'eula.txt').write_text('eula=true\n')
    server = Server(command, directory, log_name)
    try:
        server.wait_for(r'Done \(', timeout=900)
    finally:
        server.stop()
    data = anvil_dump.read_dat(directory / 'world/level.dat')['Data']
    return tuple(data['spawn']['pos'][1])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--random', type=int, default=50, help='random seeds besides the fixed ones')
    parser.add_argument('--jobs', type=int, default=max(1, min(4, int(os.environ.get('FLEET_LOAD_JOBS', '4')) // 4)),
                        help='worlds created at once (each side)')
    parser.add_argument('--origins-only', action='store_true', help='compare the spawn search origins only')
    parser.add_argument('--seed', type=int, action='append', help='only these seeds (repeatable)')
    args = parser.parse_args()
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME / parity_shard.label()  # shards run at once
    build.mkdir(parents=True, exist_ok=True)
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    oh.run_logged(['javac', '--release', lock['minecraft']['java_major'], '-proc:none', '-nowarn', '-cp', dependencies,
                   '-d', classes, lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaWorldSpawnOracle.java'],
                  build, 'compile-java')
    oh.run_logged(['cargo', 'build', '--release', '-p', 'horizon-world', '--example', 'world_probe'], build, 'compile-probe')
    all_seeds = parity_shard.shard(args.seed or seeds(args.random))
    started = time.time()

    vanilla, rust = origins(build, classes, dependencies, all_seeds)
    origin_diffs = [s for s in all_seeds if vanilla.get(s) != rust.get(s)]
    for s in origin_diffs:
        print(f'origin {s}: vanilla column {vanilla.get(s)} / horizon {rust.get(s)}')
    print(f'Origins: {len(all_seeds) - len(origin_diffs)}/{len(all_seeds)} seeds match '
          f'(column x z, chunk x z) in {time.time() - started:.0f}s', flush=True)
    if args.origins_only:
        sys.exit(1 if origin_diffs else 0)

    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], build,
                  'compile-rust', timeout=3600)
    binary = target_dir('fast') / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    # One worker thread: the chunks decorated in a fixed order (see above).
    vanilla_server = ['java', '-Xmx2g', '-Dmax.bg.threads=1', '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon_server = [binary, '--resources', RESOURCES, '--dedicated', '--nogui']
    os.environ.update(lab.game_env())
    cached = vanilla_cache(lock, vanilla_server, dependencies)
    reused = []

    def vanilla_spawn(seed):
        record = cached and cached(seed)
        if record and record.exists():
            reused.append(seed)
            return tuple(json.loads(record.read_text()))
        spawn = world_spawn(vanilla_server, build / f'vanilla-{seed}', seed, f'vanilla-{seed}')
        if record:
            temporary = record.with_suffix(f'.{os.getpid()}.tmp')
            temporary.write_text(json.dumps(spawn))
            temporary.replace(record)
        return spawn

    def one(seed):
        t0 = time.time()
        a = vanilla_spawn(seed)
        b = world_spawn(horizon_server, build / f'horizon-{seed}', seed, f'horizon-{seed}')
        return seed, a, b, time.time() - t0

    spawn_diffs = []
    with ThreadPoolExecutor(args.jobs) as pool:
        for seed, a, b, took in pool.map(one, all_seeds):
            ok = a == b
            if not ok:
                spawn_diffs.append(seed)
            print(f'{"ok  " if ok else "DIFF"} {seed}: vanilla {a[0]} {a[1]} {a[2]} / horizon {b[0]} {b[1]} {b[2]} '
                  f'(origin chunk {vanilla[seed][2]} {vanilla[seed][3]}; {took:.0f}s)', flush=True)
            if ok:
                for path in build.glob(f'*-{seed}*'):
                    shutil.rmtree(path, ignore_errors=True)
    print(f'World spawns: {len(all_seeds) - len(spawn_diffs)}/{len(all_seeds)} seeds match in {time.time() - started:.0f}s'
          + (f' (vanilla reused for {len(reused)})' if reused else ''))
    if origin_diffs or spawn_diffs:
        sys.exit(1)


if __name__ == '__main__':
    main()
