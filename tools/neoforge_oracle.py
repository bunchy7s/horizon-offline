#!/usr/bin/env python3
"""Dev-only NeoForge oracle for Horizon's mod-port harnesses (docs/modpacks.md).

    neoforge_oracle.py export --jars DIR [--deps DIR] [--namespaces a,b] --out FACTS.json

installs a NeoForge server under .local/neoforge-oracle (once), builds the exporter mod in
mods/horizon/src/neoforge-oracle, starts the server headless with the given mod jars, and writes every
block, item, recipe and tag of the mods' namespaces (and the vanilla blocks, for templates) to
FACTS.json. The jars are the user's own; nothing here is shipped or committed.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LOCAL = ROOT / '.local' / 'neoforge-oracle'
SOURCE = ROOT / 'mods' / 'horizon' / 'src' / 'neoforge-oracle'
# ATM10 8.x: Minecraft 1.21.1 on NeoForge 21.1.259 (docs/modpacks.md).
NEOFORGE = os.environ.get('HORIZON_NEOFORGE', '21.1.259')
MAVEN = 'https://maven.neoforged.net/releases/net/neoforged/neoforge'


def install():
    """The NeoForge server, installed once per version."""
    home = LOCAL / NEOFORGE
    args = home / f'libraries/net/neoforged/neoforge/{NEOFORGE}/unix_args.txt'
    if args.exists():
        return home
    home.mkdir(parents=True, exist_ok=True)
    installer = LOCAL / f'neoforge-{NEOFORGE}-installer.jar'
    if not installer.exists():
        url = f'{MAVEN}/{NEOFORGE}/neoforge-{NEOFORGE}-installer.jar'
        with urllib.request.urlopen(url, timeout=120) as r:
            installer.write_bytes(r.read())
    with (LOCAL / f'install-{NEOFORGE}.log').open('w') as log:
        subprocess.run(['java', '-jar', str(installer), '--installServer', str(home)],
                       cwd=LOCAL, stdout=log, stderr=subprocess.STDOUT, check=True)
    return home


def client():
    """The NeoForge client (tools/perf_bench.py --game neoforge), installed once per version: the installer's
    own files, then the vanilla libraries its version inherits, fetched as the launcher does (sha1-checked)."""
    sys.path.insert(0, str(ROOT / 'tools'))
    import hashlib
    import lab
    home = LOCAL / f'client-{NEOFORGE}'
    profile = home / 'versions' / f'neoforge-{NEOFORGE}' / f'neoforge-{NEOFORGE}.json'
    if not profile.exists():
        install()  # the installer jar
        home.mkdir(parents=True, exist_ok=True)
        (home / 'launcher_profiles.json').write_text('{"profiles":{}}')
        with (LOCAL / f'install-client-{NEOFORGE}.log').open('w') as log:
            subprocess.run(['java', '-jar', str(LOCAL / f'neoforge-{NEOFORGE}-installer.jar'), '--installClient',
                            str(home)], cwd=LOCAL, stdout=log, stderr=subprocess.STDOUT, check=True)
    neo = json.loads(profile.read_text())
    base = json.loads((home / 'versions' / neo['inheritsFrom'] / f'{neo["inheritsFrom"]}.json').read_text())
    jars = []
    for manifest in (neo, base):
        names = {library['name'] for library in manifest['libraries']}
        for library in manifest['libraries']:
            artifact = library.get('downloads', {}).get('artifact')
            if not artifact or not lab.library_allowed(library) or not lab.native_library_matches(library, None, names):
                continue
            path = home / 'libraries' / artifact['path']
            if not path.exists():
                with urllib.request.urlopen(artifact['url'], timeout=120) as r:
                    data = r.read()
                if hashlib.sha1(data).hexdigest() != artifact['sha1']:
                    sys.exit(f'{artifact["url"]}: sha1 mismatch')
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            if path not in jars:
                jars.append(path)
    return home, neo, base, jars


def client_command(java, heap, game_dir, natives, assets, jvm=(), game=()):
    """The command the launcher would run for the NeoForge client profile, with no account."""
    home, neo, base, jars = client()
    import lab
    values = {'library_directory': str(home / 'libraries'), 'classpath_separator': os.pathsep,
              'version_name': f'neoforge-{NEOFORGE}', 'natives_directory': str(natives),
              'launcher_name': 'horizon-bench', 'launcher_version': '1'}
    expand = lambda s: s.replace('${', '{').format(**values)
    command = [str(java), f'-Xmx{heap}']
    for argument in base['arguments']['jvm']:
        if isinstance(argument, dict):
            if lab.library_allowed(argument):
                command += [argument['value']] if isinstance(argument['value'], str) else argument['value']
        elif argument == '${classpath}':
            # Every library; not the vanilla jar, which NeoForge's patched client replaces.
            command.append(os.pathsep.join(map(str, jars)))
        else:
            command.append(expand(argument))
    command += [expand(a) for a in neo['arguments']['jvm']]
    command += [*jvm, neo['mainClass']]
    command += ['--username', 'Bench', '--uuid', '00000000000000000000000000000000', '--accessToken', '0',
                '--version', f'neoforge-{NEOFORGE}', '--gameDir', str(game_dir), '--assetsDir', str(assets),
                '--assetIndex', base['assetIndex']['id'], '--userType', 'msa', '--versionType', 'release',
                *neo['arguments']['game'], *game]
    return home, command


def build_bench(home):
    """The NeoForge benchmark driver (mods/horizon/src/neoforge-bench), against the client."""
    return build(home, ROOT / 'mods/horizon/src/neoforge-bench', 'horizon-bench-neoforge.jar', client_classpath(home))


def client_classpath(home):
    libs = home / 'libraries'
    first = [libs / f'net/neoforged/neoforge/{NEOFORGE}/neoforge-{NEOFORGE}-client.jar',
             libs / f'net/neoforged/neoforge/{NEOFORGE}/neoforge-{NEOFORGE}-universal.jar']
    rest = sorted(p for p in libs.rglob('*.jar') if p not in first and 'slim' not in p.name
                  and 'extra' not in p.name and not p.name.endswith('-unpacked.jar'))
    return os.pathsep.join(map(str, first + rest))


def classpath(home):
    libs = home / 'libraries'
    first = [libs / f'net/neoforged/neoforge/{NEOFORGE}/neoforge-{NEOFORGE}-server.jar',
             libs / f'net/neoforged/neoforge/{NEOFORGE}/neoforge-{NEOFORGE}-universal.jar']
    rest = sorted(p for p in libs.rglob('*.jar') if p not in first and 'slim' not in p.name
                  and 'extra' not in p.name and not p.name.endswith('-unpacked.jar'))
    return os.pathsep.join(map(str, first + rest))


def build(home, source=SOURCE, name='horizon-oracle.jar', cp=None):
    """A dev-only mod jar (by default the exporter), rebuilt when its sources change."""
    out = LOCAL / 'build'
    jar = out / name
    sources = sorted((source / 'java').rglob('*.java'))
    resources = sorted(p for p in (source / 'resources').rglob('*') if p.is_file())
    newest = max(p.stat().st_mtime for p in sources + resources)
    if jar.exists() and jar.stat().st_mtime > newest:
        return jar
    classes = out / f'classes-{Path(name).stem}'
    shutil.rmtree(classes, ignore_errors=True)
    classes.mkdir(parents=True)
    javac = subprocess.run(['javac', '--release', '21', '-proc:none', '-nowarn', '-cp', cp or classpath(home),
                            '-d', str(classes), *map(str, sources)], capture_output=True, text=True)
    if javac.returncode:
        sys.exit(f'{name} does not compile:\n{javac.stdout}{javac.stderr}')
    with zipfile.ZipFile(jar, 'w', zipfile.ZIP_DEFLATED) as z:
        for p in sorted(classes.rglob('*.class')):
            z.write(p, p.relative_to(classes).as_posix())
        for p in resources:
            z.write(p, p.relative_to(source / 'resources').as_posix())
    return jar


SERVER_PROPERTIES = """level-type=minecraft\\:flat
online-mode=false
server-port=0
spawn-protection=0
view-distance=2
simulation-distance=2
generate-structures=false
spawn-monsters=false
spawn-animals=false
"""


def instance(name, jars, extra=()):
    """A server folder with `jars` (and the exporter) in mods/."""
    home = install()
    run = LOCAL / 'runs' / name
    shutil.rmtree(run, ignore_errors=True)
    (run / 'mods').mkdir(parents=True)
    (run / 'libraries').symlink_to(home / 'libraries')
    (run / 'eula.txt').write_text('eula=true\n')
    (run / 'server.properties').write_text(SERVER_PROPERTIES)
    for jar in [*jars, *extra]:
        (run / 'mods' / Path(jar).name).symlink_to(Path(jar).resolve())
    return home, run


def start(home, run, props, log_name='server.log', timeout=900):
    """Runs the server to its end; returns (seconds, the log)."""
    args = home / f'libraries/net/neoforged/neoforge/{NEOFORGE}/unix_args.txt'
    command = ['java', '-Xmx4G', *[f'-D{k}={v}' for k, v in props.items()], f'@{args}', 'nogui']
    began = time.monotonic()
    with (run / log_name).open('w') as log:
        subprocess.run(command, cwd=run, stdin=subprocess.DEVNULL, stdout=log,
                       stderr=subprocess.STDOUT, timeout=timeout)
    return time.monotonic() - began, (run / log_name).read_text(errors='replace')


def mod_ids(jar):
    """The mod ids a jar declares (its neoforge.mods.toml)."""
    import tomllib
    with zipfile.ZipFile(jar) as z:
        for name in ('META-INF/neoforge.mods.toml', 'META-INF/mods.toml'):
            if name in z.namelist():
                try:
                    toml = tomllib.loads(z.read(name).decode('utf-8', 'replace'))
                except tomllib.TOMLDecodeError:
                    return []
                return [m.get('modId', '') for m in toml.get('mods', [])]
    return []


def export(args):
    jars = sorted(Path(args.jars).glob('*.jar'))
    deps = sorted(Path(args.deps).glob('*.jar')) if args.deps else []
    namespaces = args.namespaces or ','.join(i for j in jars for i in mod_ids(j))
    home = install()
    exporter = build(home)
    home, run = instance('export', jars, [*deps, exporter])
    out = Path(args.out).resolve()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.unlink(missing_ok=True)
    seconds, log = start(home, run, {'horizon.oracle.out': out, 'horizon.oracle.namespaces': namespaces})
    if not out.exists():
        tail = '\n'.join(log.splitlines()[-40:])
        sys.exit(f'the oracle wrote nothing; the end of {run / "server.log"}:\n{tail}')
    facts = json.loads(out.read_text())
    mods = sum(1 for b in facts['blocks'] if not b.startswith('minecraft:'))
    print(f'{out}: {mods} mod blocks, {len(facts["items"])} items, {len(facts["recipes"])} recipes '
          f'({namespaces}); server ran {seconds:.1f} s')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    e = sub.add_parser('export', help='write the facts of the mods in --jars')
    e.add_argument('--jars', required=True, help='the mods to export')
    e.add_argument('--deps', help='their required libraries (loaded, not exported)')
    e.add_argument('--namespaces', help='comma-separated; default: the jars\' mod ids')
    e.add_argument('--out', required=True)
    args = parser.parse_args()
    {'export': export}[args.command](args)


if __name__ == '__main__':
    main()
