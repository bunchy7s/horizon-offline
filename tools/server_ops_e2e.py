#!/usr/bin/env python3
"""An operator's commands on Horizon's dedicated server, typed by a Horizon player, end to end.

Vanilla decides what an operator may run by its permission level (ops.json, `op-permission-level`):
`/kick`, `/ban`, `/ban-ip`, `/pardon`, `/pardon-ip`, `/banlist`, `/op`, `/deop` and `/whitelist` need 3 (admins),
`/tp` and `/gamemode` need 2 (game masters). A server's operators moderate it with them; the console is not the
only way.

    python3 tools/server_ops_e2e.py --bin-dir target/fast

A survival dedicated server (offline mode, a flat world) with the player Opal in ops.json, at level 2, 3 and 4 in
turn. Opal joins with Horizon's game, driven by horizon-ctl, and types each command in the chat as a player does;
horizon-bots players stand in as the moderated ones. Each command her level allows must give vanilla's feedback in
her chat and change what it changes (the bot leaves, banned-players.json, banned-ips.json, whitelist.json,
ops.json); at level 2 the admins' commands must do nothing.

The game and the server run with fresh HORIZON_HOME, launcher and XDG folders and without audio. Logs go to --out.
Build first:
    cargo build --profile fast -p horizon-client -p horizon-client-lib --bins
    cargo build --profile fast -p horizon-bots
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from agent_control_e2e import Failed  # noqa: E402
from lan_e2e import Game  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PLAYER = 'Opal'
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'survival',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
}
IP = '203.0.113.7'
# (command, the level it needs, a line Opal's chat must show when hers is enough).
COMMANDS = [
    ('tp Kim0 0.5 -60 8.5', 2, 'Teleported Kim0 to 0.500000, -60.000000, 8.500000'),
    ('gamemode creative Kim0', 2, "Set Kim0's game mode to Creative Mode"),
    ('op Kim0', 3, 'Made Kim0 a server operator'),
    ('deop Kim0', 3, 'Made Kim0 no longer a server operator'),
    ('whitelist add Kim0', 3, 'Added Kim0 to the whitelist'),
    ('whitelist remove Kim0', 3, 'Removed Kim0 from the whitelist'),
    ('ban-ip ' + IP + ' spam bots', 3, f'Banned IP {IP}: spam bots'),
    ('banlist ips', 3, 'There are 1 ban(s):'),
    ('pardon-ip ' + IP, 3, f'Unbanned IP {IP}'),
    ('kick Kim0 spamming chat', 3, 'Kicked Kim0: spamming chat'),
    ('ban Lou0 cheating', 3, 'Banned Lou0: cheating'),
    ('banlist', 3, 'There are 1 ban(s):'),
    ('pardon Lou0', 3, 'Unbanned Lou0'),
]


def type_command(game, command):
    """Opens the chat with `/`, types the rest and sends it, as a player does."""
    game.raw('key', 'slash', 'tap')
    game.until(lambda: game('screen').get('screen'), lambda s: s == 'chat', ticks=10)
    game.raw('type', command)
    game.raw('key', 'enter', 'tap')
    game.wait(10)


def chat(game):
    return game.raw('messages', 50)['chat']


def bot(bins, port, name, seconds):
    return subprocess.Popen([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                             '--behaviour', 'stand', '--seconds', str(seconds), '--name', name],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def run(bins, level, out):
    work = Path(tempfile.mkdtemp(prefix=f'hz-server-ops-{level}-'))
    folder = work / 'server'
    folder.mkdir()
    port = lab.free_port()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (folder / 'eula.txt').write_text('eula=true\n')
    (folder / 'ops.json').write_text(json.dumps([{'uuid': str(lab.offline_uuid(PLAYER)), 'name': PLAYER,
                                                  'level': level, 'bypassesPlayerLimit': False}]))
    command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
    server = Server(command, folder, out / f'server-{level}.log')
    game = Game(f'opal{level}', bins / 'horizon-ctl', lab.RESOURCES.resolve())
    for key in ('XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'HORIZON_LAUNCHER_HOME'):
        game.d.env[key] = str(game.work / key.lower())
    game.d.env['HORIZON_LAUNCHER_NO_SELF_UPDATE'] = '1'
    bots = []
    failed = []
    try:
        server.wait_for(DONE, 300)
        game('launch', wait=False, offscreen=sys.platform == 'darwin')
        game.raw('connect', f'127.0.0.1:{port}', PLAYER)
        game.raw('wait-for', 'loaded', '--timeout', 300)
        server.wait_for(rf'{PLAYER} joined the game', 60)
        bots = [bot(bins, port, name, 600) for name in ('Kim', 'Lou')]
        for name in ('Kim0', 'Lou0'):
            server.wait_for(rf'{name} joined the game', 120)
        game.wait(20)
        for command, needs, expected in COMMANDS:
            type_command(game, command)
            lines = chat(game)
            shown = any(expected in line for line in lines)
            print(f'[level {level}] /{command}: {"ran" if shown else "did not run"}')
            if shown != (level >= needs):
                failed.append(f'level {level}: /{command} {"ran" if shown else "gave no " + repr(expected)}; '
                              f'last chat: {lines[-3:]}')
        files = {name: json.loads((folder / name).read_text()) if (folder / name).exists() else []
                 for name in ('banned-players.json', 'banned-ips.json', 'whitelist.json', 'ops.json')}
        if [e['name'] for e in files['ops.json']] != [PLAYER]:
            failed.append(f'level {level}: ops.json after /op and /deop: {files["ops.json"]}')
        if files['banned-players.json'] or files['banned-ips.json'] or files['whitelist.json']:
            failed.append(f'level {level}: bans or whitelist left after pardon and remove: {files}')
        log = server.since(0)
        for name, how in [('Kim0', 'lost connection: spamming chat'), ('Lou0', 'lost connection')]:
            if any(f'{name} {how}' in line for line in log) != (level >= 3):
                failed.append(f'level {level}: {name} {"left" if level < 3 else "stayed"}')
    finally:
        (out / f'client-{level}.log').write_text(game.close())
        for b in bots:
            b.kill()
        server.stop()
        shutil.rmtree(game.work, ignore_errors=True)
        shutil.rmtree(work, ignore_errors=True)
    return failed


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-ops-e2e'))
    parser.add_argument('--level', type=int, action='append', choices=[2, 3, 4],
                        help='only this ops.json level (repeatable; default 2, 3 and 4)')
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    failed = []
    for level in args.level or [2, 3, 4]:
        try:
            failed += run(bins, level, out)
        except (Failed, AssertionError, RuntimeError) as e:
            failed.append(f'level {level}: {e}')
    if failed:
        sys.exit(f'server_ops_e2e failed (logs in {out}):\n' + '\n'.join(failed))
    print('server_ops_e2e: operators at levels 3 and 4 ran every moderation command, one at level 2 none of them')


if __name__ == '__main__':
    main()
