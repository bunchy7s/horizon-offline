"""Shared plumbing for the interaction/inventory differential harnesses
(test_recipe_parity, test_menu_click_parity, test_block_interaction_parity):
compile and run a vanilla Java oracle headlessly, run the matching Rust
example, and report differing lines grouped by a key."""
import json
import os
from pathlib import Path
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base

RESOURCES = lab.RESOURCES
PARITY = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity'


def work_dir(name):
    path = lab.LOCAL / name
    path.mkdir(parents=True, exist_ok=True)
    return path


class Vanilla:
    """Compiles oracles against the pinned server jar and runs them."""

    def __init__(self, work):
        self.lock, self.jars, self.libraries = base.verify_artifacts()
        self.work = work
        self.classes = work / 'classes'
        self.classes.mkdir(exist_ok=True)
        self.deps = os.pathsep.join(map(str, [self.jars['server'], *self.libraries]))

    def compile(self, *oracles):
        sources = [PARITY / (name + '.java') for name in oracles]
        done = subprocess.run(['javac', '--release', str(self.lock['minecraft']['java_major']), '-proc:none',
                               '-sourcepath', str(lab.ROOT / 'mods/horizon/src/test/java'), '-nowarn', '-Xmaxerrs', '30',
                               '-cp', self.deps, '-d', str(self.classes), *map(str, sources)], capture_output=True, text=True)
        if done.returncode:
            print(done.stderr.replace(str(lab.ROOT) + '/', ''))
            raise SystemExit('javac failed')

    def run(self, oracle, *args, heap='2G'):
        subprocess.run(['java', f'-Xmx{heap}', '-cp', str(self.classes) + os.pathsep + self.deps,
                        'dev.horizonlab.parity.' + oracle, *map(str, args)], check=True)


def cargo_example(package, example, *args, profile=None):
    command = ['cargo', 'run', '-q', '-j3']
    if profile:
        command += ['--profile', profile]
    probe = package.removeprefix('horizon-') + '_probe'  # each crate's probe example: client_probe, world_probe
    if package == 'horizon-client':
        package = 'horizon-client-lib'
    command += ['-p', package, '--example', probe, '--', example, *map(str, args)]
    subprocess.run(command, check=True, cwd=lab.ROOT)


def compare(cases, native_path, rust_path, key, describe=None, limit=3):
    """Print differing cases grouped by key(case, native_line); returns the count."""
    native = Path(native_path).read_text().split('\n')
    rust = Path(rust_path).read_text().split('\n')
    if len(native) != len(rust):
        print(f'line counts differ: native {len(native)} rust {len(rust)}')
    groups = {}
    for case, a, b in zip(cases, native, rust):
        if a != b:
            groups.setdefault(key(case, a), []).append((case, a, b))
    describe = describe or (lambda case: case if isinstance(case, str) else json.dumps(case))
    total = sum(len(v) for v in groups.values())
    for name, entries in sorted(groups.items(), key=lambda e: -len(e[1])):
        print(f'{name}: {len(entries)} differing')
        for case, a, b in entries[:limit]:
            print(f'  case   {describe(case)}\n  native {a}\n  rust   {b}')
    print(f'{len(cases) - total}/{len(cases)} cases match')
    return total
