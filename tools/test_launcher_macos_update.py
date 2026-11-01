#!/usr/bin/env python3
"""Opt-in macOS updater regression, compiling the EXACT product module without GPUI.

    python3 tools/test_launcher_macos_update.py --run
    python3 tools/test_launcher_macos_update.py --run --keychain

Requires an approved build window, cached lockfile dependencies and macOS developer tools.
Without --run, only prints this help. --run tests filesystem/query/signature seams; adding
--keychain runs signed synthetic bundles, real lsof and real Keychain with UUID keys under
dev.playhorizon.launcher.diagnostic ONLY, removing and checking those keys afterwards.
No real launcher, OAuth, accounts, network or player data. Synthetic peers model multiple
GUI/MCP processes sharing a vnode; actual launcher startup/MCP wiring needs separate QA.
The scratch crate uses normal Rust module layout and byte-identical copies of the product
module and its tests, SHA-256 checked against the starting snapshot before invoking cargo.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import sys
import tempfile
import uuid

try:
    from . import lab
except ImportError:
    import lab

ROOT = Path(__file__).resolve().parents[1]
MODULE = ROOT / 'launcher/src/mac_update.rs'
LOCK = ROOT / 'launcher/Cargo.lock'
PROBE = r'''
#[allow(dead_code)]
mod mac_update;
use std::{io::{self, BufRead, Write}, path::PathBuf, process::Command};
use std::os::unix::process::CommandExt;
const SERVICE: &str = "dev.playhorizon.launcher.diagnostic";
const MARKER: &str = "horizon synthetic updater regression";
fn entry(key: &str) -> Result<keyring::Entry, String> {
    if key.len() != 36 || !key.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("expected diagnostic UUID".into());
    }
    keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())
}
fn main() {
    let app = PathBuf::from(std::env::args_os().nth(1).expect("scratch app path"));
    mac_update::cleanup(&app);
    println!("READY {}", std::process::id());
    io::stdout().flush().unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (op, arg) = line.split_once(' ').unwrap_or((&line, ""));
        let result: Result<(), String> = (|| match op {
            "update" => mac_update::replace_bundle(std::path::Path::new(arg), &app),
            "cleanup" => { mac_update::cleanup(&app); Ok(()) },
            "write" => {
                let e = entry(arg)?;
                match e.get_password() {
                    Err(keyring::Error::NoEntry) => e.set_password(MARKER).map_err(|e| e.to_string()),
                    _ => Err("diagnostic key was not absent before write".into()),
                }
            },
            "read" => {
                if entry(arg)?.get_password().map_err(|e| e.to_string())? != MARKER {
                    return Err("diagnostic marker differs".into());
                }
                Ok(())
            },
            "delete" => {
                let e = entry(arg)?;
                match e.get_password() {
                    Err(keyring::Error::NoEntry) => return Ok(()),
                    Ok(value) if value == MARKER => (),
                    Ok(_) => return Err("unexpected diagnostic value; preserved".into()),
                    Err(error) => return Err(error.to_string()),
                }
                match e.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => (),
                    Err(e) => return Err(e.to_string()),
                }
                match e.get_password() {
                    Err(keyring::Error::NoEntry) => Ok(()),
                    _ => Err("diagnostic key absence not verified".into()),
                }
            },
            "exec" => Err(Command::new(app.join("Contents/MacOS/horizon-launcher"))
                .arg(&app).exec().to_string()),
            _ => Err("unknown probe command".into()),
        })();
        match result { Ok(()) => println!("OK"), Err(e) => println!("ERR {e}") }
        io::stdout().flush().unwrap();
    }
}
'''


def run(*args, env):
    subprocess.run([str(a) for a in args], env=env, check=True, timeout=180)


def pins(text):
    return {tuple(re.findall(r'^(?:name|version|source|checksum) = "([^"]*)"$', block, re.M))
            for block in text.split('[[package]]')[1:] if '\nchecksum = ' in block}


class Peer:
    def __init__(self, app, env):
        self.proc = subprocess.Popen([str(app / 'Contents/MacOS/horizon-launcher'), str(app)],
                                     env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

    def line(self):
        if not select.select([self.proc.stdout], [], [], 45)[0]:
            raise TimeoutError(f'probe {self.proc.pid} did not reply (Keychain may need interaction)')
        line = self.proc.stdout.readline().strip()
        assert line, f'probe {self.proc.pid} exited without reply'
        return line

    def ask(self, command, expected='OK'):
        self.proc.stdin.write(command + '\n')
        self.proc.stdin.flush()
        answer = self.line()
        assert answer == expected, f'{command.split()[0]}: {answer}'

    def close(self):
        if not self.proc.stdin.closed:
            self.proc.stdin.close()
        try:
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=10)


def keychain_scenario(work, binary, env):
    peers, keys = [], [str(uuid.uuid4()) for _ in range(3)]
    app = work / 'Applications/Horizon.app'
    # Recovery custody if an assertion or native permission prompt interrupts the run.
    print(json.dumps({'diagnostic_service': 'dev.playhorizon.launcher.diagnostic',
                      'diagnostic_uuids': keys, 'scratch': str(work)}), flush=True)

    def bundle(path, version):
        (path / 'Contents/MacOS').mkdir(parents=True)
        (path / 'Contents/Resources').mkdir()
        shutil.copy2(binary, path / 'Contents/MacOS/horizon-launcher')
        (path / 'Contents/Resources/version').write_text(version)
        (path / 'Contents/Info.plist').write_text('''<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>horizon-launcher</string>
<key>CFBundleIdentifier</key><string>dev.playhorizon.launcher.diagnostic</string>
<key>CFBundlePackageType</key><string>APPL</string></dict></plist>''')
        run('/usr/bin/codesign', '--force', '--sign', '-', path, env=env)
        return path

    def start():
        p = Peer(app, env)
        peers.append(p)
        assert p.line() == f'READY {p.proc.pid}'
        return p

    def retired():
        return set(app.parent.glob('.Horizon.app.old-*'))

    bundle(app, 'v1')
    v2 = bundle(work / 'v2/Horizon.app', 'v2')
    v3 = bundle(work / 'v3/Horizon.app', 'v3')
    try:
        a, b = start(), start()  # two users of the same original executable, like GUI + MCP
        a.ask(f'update {v2}')
        a.ask(f'write {keys[0]}')  # the causal failure occurs HERE if the old executable was deleted
        reader = start()
        reader.ask(f'read {keys[0]}')
        old_v1 = retired()
        assert len(old_v1) == 1
        a.ask(f'update {v3}')  # updater PID unchanged, while a new-generation reader is alive
        b.ask(f'write {keys[1]}')
        reader_v3 = start()
        reader_v3.ask(f'read {keys[1]}')
        assert len(retired()) == 2 and old_v1 < retired()
        b.close()
        a.ask('cleanup')
        assert len(retired()) == 2, 'one original user remains, plus the v2 reader'
        reader.close()
        a.ask('cleanup')
        assert retired() == old_v1, 'only the intermediate generation is unused'
        a.ask('exec', f'READY {a.proc.pid}')  # same PID, new vnode, startup cleanup
        assert not retired(), 'exec must release the last original executable mapping'
        a.ask(f'write {keys[2]}')
        reader_v3.ask(f'read {keys[2]}')
        (v2 / 'Contents/Resources/version').write_text('tampered')
        a.proc.stdin.write(f'update {v2}\n')
        a.proc.stdin.flush()
        assert a.line().startswith('ERR '), 'tampered signed source must be refused'
        assert (app / 'Contents/Resources/version').read_text() == 'v3'
        run('/usr/bin/codesign', '--verify', '--deep', '--strict', app, env=env)
    finally:
        # A fresh process has a valid executable even if the test exposed an unsafe old updater.
        removed, cleanup_errors = [], []
        try:
            try:
                cleaner = start()
            except Exception as error:
                cleanup_errors = [{'uuid': key, 'error': str(error)} for key in keys]
            else:
                for key in keys:
                    try:
                        cleaner.ask(f'delete {key}')
                        removed.append(key)
                    except Exception as error:
                        cleanup_errors.append({'uuid': key, 'error': str(error)})
        finally:
            for p in peers:
                p.close()
            print(json.dumps({'diagnostic_uuids_removed_and_absent': removed,
                              'unverified_cleanup': cleanup_errors}), flush=True)
        assert not cleanup_errors, 'diagnostic cleanup incomplete; reconcile UUIDs in the receipt'
    return {'synthetic_keys_removed_and_absent': len(keys), 'shared_vnode_users': 2,
            'updates': 2, 'exec_preserved_pid': True, 'tampered_source_refused': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--run', action='store_true', help='explicitly build/run in an approved QA window')
    parser.add_argument('--keychain', action='store_true', help='also access ONLY synthetic diagnostic UUIDs')
    args = parser.parse_args()
    if not args.run:
        if args.keychain:
            parser.error('--keychain requires --run')
        parser.print_help()
        return
    if sys.platform != 'darwin':
        parser.error('macOS is required')
    sources = [MODULE, MODULE.parent / 'mac_update/tests.rs', LOCK, Path(__file__).resolve(),
               Path(lab.__file__).resolve()]
    before = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
    lock = LOCK.read_text()
    print(json.dumps({'starting_snapshot_sha256': before}), flush=True)
    with tempfile.TemporaryDirectory(prefix='horizon-macos-update-') as tmp:
        work = Path(tmp)
        env = dict(os.environ, CARGO_BUILD_JOBS='1', CARGO_TARGET_DIR=str(work / 'target'))
        for key in ['HORIZON_HOME', 'HORIZON_LAUNCHER_HOME', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME',
                    'HORIZON_LAUNCHER_TRASH', 'HORIZON_LAUNCHER_CLIPBOARD', 'HORIZON_LAUNCHER_DESKTOP']:
            env[key] = str(work / key.lower())
            Path(env[key]).mkdir()
        # Force fresh homes before the official helper preserves caller-provided folders.
        env = lab.game_env(env)
        (work / 'src/mac_update').mkdir(parents=True)
        (work / 'src/main.rs').write_text(PROBE)
        # Normal `mod mac_update;` resolves its children under src/mac_update/.
        # An absolute #[path] would instead make `mod tests;` seek a sibling tests.rs.
        for source, target in [(MODULE, work / 'src/mac_update.rs'),
                               (sources[1], work / 'src/mac_update/tests.rs')]:
            target.write_bytes(source.read_bytes())
            expected = before[str(source.relative_to(ROOT))]
            assert hashlib.sha256(target.read_bytes()).hexdigest() == expected, \
                f'copied source differs from starting snapshot: {source}'
        manifest = work / 'Cargo.toml'
        manifest.write_text('''[package]
name = "horizon-macos-update-regression"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
getrandom = "=0.3.4"
keyring = { version = "=3.6.3", features = ["apple-native"] }
''')
        (work / 'Cargo.lock').write_text(lock)
        # Resolve only, offline; reject ANY transitive version/checksum drift before compiling.
        subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1',
                        '--manifest-path', str(manifest)], env=env, check=True,
                       stdout=subprocess.DEVNULL, timeout=60)
        resolved = pins((work / 'Cargo.lock').read_text())
        assert resolved and resolved <= pins(lock), 'probe dependency drift from launcher/Cargo.lock'
        run('cargo', 'test', '--offline', '--locked', '-j1', '--manifest-path', manifest, env=env)
        result = {'module_tests': 'see cargo counts above', 'keychain': 'not requested'}
        if args.keychain:
            run('cargo', 'build', '--offline', '--locked', '-j1', '--manifest-path', manifest, env=env)
            result['keychain'] = keychain_scenario(work, work / 'target/debug/horizon-macos-update-regression', env)
        after = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sources}
        assert before == after, 'source/lock snapshot changed during regression'
        print(json.dumps({'snapshot_sha256': before, 'registry_pins': len(resolved), **result}, indent=2))


if __name__ == '__main__':
    main()
