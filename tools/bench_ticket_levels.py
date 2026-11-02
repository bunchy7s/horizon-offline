#!/usr/bin/env python3
"""Compare actual ticket-map code against a Git revision, without game resources.

    python3 tools/bench_ticket_levels.py --base origin/main --output tickets.json

Requires a POSIX host. Compiles the actual modules with rustc -O and the real FxHasher.
Alternates their order, discards a warm-up pair, and checks identical level sums.
Reports isolated single-thread process CPU times, not server MSPT or wall times.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = 'crates/horizon-client/src/ticket_levels.rs'
HASH = 'crates/horizon-core/src/fast_hash.rs'
HARNESS = r'''
pub fn benchmark(players: i32, spacing: i32, distance: i32, rounds: usize) {
    use std::hint::black_box;
    // ISO C process CPU time excludes descheduling on a shared machine.
    // The benchmark process is single-threaded; POSIX CLOCKS_PER_SEC is 1 MHz.
    unsafe extern "C" { fn clock() -> std::os::raw::c_long; }
    fn cpu_time() -> f64 {
        // SAFETY: clock takes no pointers and only reads this process's CPU time.
        let ticks = unsafe { clock() };
        assert!(ticks >= 0, "process CPU clock unavailable");
        ticks as f64 / 1_000_000.0
    }
    let source = Sources {
        players: (0..players).map(|i| ((i % 10) * spacing, (i / 10) * spacing)).collect(),
        view_distance: distance, simulation_distance: distance, ..Default::default()
    };
    let queries: Vec<_> = source.players.iter().flat_map(|&(x,z)| {
        (-distance-2..=distance+2).flat_map(move |dz| {
            (-distance-2..=distance+2).map(move |dx| (x+dx,z+dz))
        })
    }).collect();
    let start = cpu_time();
    for _ in 0..rounds { black_box(compute(black_box(&source))); }
    let build = (cpu_time()-start)*1e6 / rounds as f64;
    let map = compute(&source);
    let mut checksum = 0u64;
    let start = cpu_time();
    for _ in 0..rounds {
        for &(x,z) in &queries {
            let (l,s) = black_box(map.get(black_box(x),black_box(z)));
            checksum = checksum.wrapping_add(u64::from(l)*256+u64::from(s));
        }
    }
    black_box(checksum);
    let lookup = (cpu_time()-start)*1e6 / rounds as f64;
    let mut updates = Vec::new();
    for moving in [0,1,players] {
        let mut tickets = ChunkTickets::default();
        tickets.update(Some(source.clone()));
        let start = cpu_time();
        for tick in 0..rounds {
            let mut s = source.clone();
            for p in s.players.iter_mut().take(moving as usize) { p.0 += (tick%2) as i32; }
            tickets.update(black_box(Some(s)));
            black_box(tickets.levels());
        }
        updates.push((cpu_time()-start)*1e6 / rounds as f64);
    }
    println!("{build} {lookup} {} {} {} {checksum}",updates[0],updates[1],updates[2]);
}
'''
MAIN = r'''
mod fast_hash;
mod ticket_levels;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ticket_levels::benchmark(args[0].parse().unwrap(),args[1].parse().unwrap(),
        args[2].parse().unwrap(),args[3].parse().unwrap());
}
'''


def compile_version(directory, source, hasher):
    directory.mkdir()
    (directory / 'ticket_levels.rs').write_text(source + HARNESS)
    (directory / 'fast_hash.rs').write_text(hasher)
    (directory / 'main.rs').write_text(MAIN)
    binary = directory / 'bench'
    subprocess.run(['rustc', '--edition=2024', '-O', '-Awarnings',
                    str(directory / 'main.rs'), '-o', str(binary)], check=True)
    return binary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', default='origin/main')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--rounds', type=int, default=150)
    parser.add_argument('--samples', type=int, default=7)
    args = parser.parse_args()
    if os.name != 'posix':
        parser.error('process CPU timing requires a POSIX host')
    if args.rounds < 1 or args.samples < 1:
        parser.error('rounds and samples must be positive')
    revision = subprocess.check_output(['git', 'rev-parse', args.base], cwd=ROOT, text=True).strip()
    old = subprocess.check_output(['git', 'show', f'{revision}:{SOURCE}'], cwd=ROOT, text=True)
    old_hash = subprocess.check_output(['git', 'show', f'{revision}:{HASH}'], cwd=ROOT, text=True)
    result = {'base': revision, 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
              'machine': platform.platform(), 'clock': 'single-thread process CPU time (C clock)',
              'candidate_source_sha256': hashlib.sha256((ROOT/SOURCE).read_bytes()).hexdigest(),
              'rounds': args.rounds, 'samples': args.samples, 'cases': []}
    with tempfile.TemporaryDirectory(prefix='ticket-levels-bench-') as scratch:
        binaries = [compile_version(Path(scratch)/'base', old, old_hash),
                    compile_version(Path(scratch)/'candidate', (ROOT/SOURCE).read_text(), (ROOT/HASH).read_text())]
        cases = [(1, 3, 10), (10, 3, 10), (100, 3, 10),
                 (100, 75, 2), (100, 75, 10), (100, 150, 10), (100, 75, 32), (100, 1200, 10)]
        keys = ['build_us', 'lookup_us', 'stationary_us', 'one_moving_us', 'all_moving_us']
        for count, spacing, distance in cases:
            samples = [[], []]
            checksums = set()
            for sample in range(args.samples+1):
                for version in ([0, 1] if sample%2 == 0 else [1, 0]):
                    values = subprocess.check_output([str(binaries[version]), str(count), str(spacing),
                                                      str(distance), str(args.rounds)], text=True).split()
                    checksums.add(int(values[-1]))
                    if sample:
                        samples[version].append(dict(zip(keys, map(float, values[:-1]))))
            if len(checksums) != 1:
                raise RuntimeError(f'level checksum differs for {count}, {spacing}, {distance}')
            case = {'players': count, 'spacing_blocks': spacing*16, 'distance': distance,
                    'base_samples': samples[0], 'candidate_samples': samples[1]}
            for version, name in enumerate(['base', 'candidate']):
                case[name] = {key: statistics.median(s[key] for s in samples[version]) for key in keys}
            result['cases'].append(case)
            print(f'{count} players, spacing {spacing*16}, distance {distance}: '+', '.join(
                f'{key} {case["base"][key]:.2f} -> {case["candidate"][key]:.2f}' for key in keys), flush=True)
    if args.output:
        args.output.write_text(json.dumps(result, indent=2)+'\n')


if __name__ == '__main__':
    main()
