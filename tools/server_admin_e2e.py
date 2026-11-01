#!/usr/bin/env python3
"""horizon-server's admin API on a real server, end to end (docs/api/server.md).

    python3 tools/server_admin_e2e.py --bin-dir target/fast

`horizon-server run` supervises Horizon's dedicated server (offline mode, a flat world); two horizon-bots
players join. Through the control API, with the token from .horizon/api.json, the run lists the players, then
ops and deops, whitelists, bans and pardons by name and by address, kicks, and turns the whitelist on and off,
checking each reply (`changed`, the console's lines), vanilla's files, the server's log and `GET players`.
It also checks the Prometheus text, and that a call without the token is refused and a name with a space is
not run; then reads the settings and changes the difficulty live, and backs up, downloads and deletes a backup,
and runs the same moderation from the `horizon-server` CLI.
The API listens on the address and with the token server.toml's [api] fixes, as a moderation bot's would.
An event stream (server-sent events) opened before the bots join must see them join, Kim0's chat (sent as a raw
chat packet) and the chat filter's refusal of the next one (`chat_filtered`, no `chat`), Lou0's death, the kick and the leaves, typed, with the same events polled as JSON. Last,
`horizon-server mcp` bans and pardons, reads events and refuses an unconfirmed stop, and `horizon-server
console` attaches through a pipe, runs two commands and shows what they log.

Everything runs in a fresh temporary folder with fresh HORIZON_HOME and XDG folders and no audio. Logs go
to --out. Build first:
    cargo build --profile fast -p horizon-client --bins -p horizon-bots -p horizon-server
"""
import argparse
import http.client
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
IP = '203.0.113.7'
TOKEN = 'the-moderation-bots-token'


class Failed(Exception):
    pass


def check(condition, what):
    if not condition:
        raise Failed(what)
    print(f'ok: {what}')


class Api:
    def __init__(self, folder):
        endpoint = json.loads((folder / '.horizon' / 'api.json').read_text())
        self.host, port = endpoint['address'].rsplit(':', 1)
        self.port = int(port)
        self.token = endpoint['token']

    def call(self, method, path, body=None, token=None, raw=False):
        conn = http.client.HTTPConnection(self.host, self.port, timeout=60)
        headers = {'Authorization': f'Bearer {self.token if token is None else token}',
                   'Content-Type': 'application/json'}
        conn.request(method, path, json.dumps(body) if body is not None else None, headers)
        response = conn.getresponse()
        data = response.read()
        conn.close()
        return response.status, data if raw else data.decode()

    def __call__(self, method, path, body=None):
        status, text = self.call(method, path, body)
        if status != 200:
            raise Failed(f'{method} {path} {body}: {status} {text}')
        return json.loads(text)


CHAT = 'hello from Kim0'
# A slur in leetspeak, for the chat filter to refuse (docs/chat-filter.md).
FILTERED = 'n1gg3r'


def chat_packet(message):
    """ServerboundChatPacket, unsigned: the message, the time, salt 0, no signature, nothing seen, checksum 0."""
    text = message.encode()
    return (bytes([len(text)]) + text + int(time.time() * 1000).to_bytes(8, 'big') + bytes(8) + bytes([0])
            + bytes([0]) + bytes(3) + bytes([0])).hex()


def bot(bins, port, name, *extra):
    return subprocess.Popen([bins / f'horizon-bots{EXE}', '--server', f'127.0.0.1:{port}', '--bots', '1',
                             '--behaviour', 'stand', '--seconds', '600', '--name', name, *extra],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class Stream:
    """`GET /api/v1/events` as server-sent events, read on a thread."""

    def __init__(self, api, types):
        self.events = []
        self.conn = http.client.HTTPConnection(api.host, api.port, timeout=600)
        self.conn.request('GET', f'/api/v1/events?types={types}',
                          headers={'Authorization': f'Bearer {api.token}', 'Accept': 'text/event-stream'})
        self.response = self.conn.getresponse()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        kind = None
        for raw in self.response:
            line = raw.decode().rstrip('\n')
            if line.startswith('event: '):
                kind = line[7:]
            elif line.startswith('data: '):
                event = json.loads(line[6:])
                if event['type'] != kind:
                    raise Failed(f'event {kind} carries {event}')
                self.events.append(event)

    def seen(self, kind, name):
        return [e for e in self.events if e['type'] == kind and e.get('name') == name]


def wait_until(what, test, timeout=120):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if test():
            return
        time.sleep(0.5)
    raise Failed(f'not within {timeout} s: {what}')


def listed(folder, file):
    path = folder / file
    return [e.get('name') or e.get('ip') for e in json.loads(path.read_text())] if path.exists() else []


def online(api):
    return sorted(p['name'] for p in api('GET', '/api/v1/players')['online'])


def scenario(bins, folder, server, port):
    api = Api(folder)
    wait_until('the supervisor says running', lambda: api('GET', '/api/v1/status')['phase'] == 'running')
    check(api.call('GET', '/api/v1/players', token='nope')[0] == 401, 'a call without the token is refused')
    stream = Stream(api, 'join,leave,kick,chat,chat_filtered,death')
    raw = f'60:9:{chat_packet(CHAT)},80:9:{chat_packet(FILTERED)}'
    bots = [bot(bins, port, 'Kim', '--raw', raw), bot(bins, port, 'Lou')]
    try:
        wait_until('Kim0 and Lou0 online', lambda: online(api) == ['Kim0', 'Lou0'])
        wait_until('both joins streamed', lambda: stream.seen('join', 'Kim0') and stream.seen('join', 'Lou0'), 30)
        check(stream.seen('join', 'Kim0')[0]['address'] == '127.0.0.1', 'the join event has the address')
        wait_until("Kim0's chat streamed", lambda: stream.seen('chat', 'Kim0'), 30)
        check(stream.seen('chat', 'Kim0')[0]['message'] == CHAT, f'the chat event has the message ({stream.events})')
        wait_until("Kim0's slur refused", lambda: stream.seen('chat_filtered', 'Kim0'), 30)
        refused = stream.seen('chat_filtered', 'Kim0')[0]
        check((refused['place'], refused['hit'], refused['action']) == ('chat', 1, 'refused')
              and 'message' not in refused, f'the chat_filtered event, without the text ({refused})')
        check(all(e['message'] != FILTERED for e in stream.seen('chat', 'Kim0')), 'the refused message is no chat event')
        log = folder / 'logs' / 'chat-filter.log'
        check(log.exists() and 'Kim0' in log.read_text() and FILTERED in log.read_text(),
              'logs/chat-filter.log keeps the refused text')
        server.wait_for(r"\[chat-filter\] Refused Kim0's chat \(hit 1: refused\)", 30)
        players = api('GET', '/api/v1/players')
        check(players['max_players'] == 20 and players['ops'] == [] and players['bans'] == [],
              f'players: 20 slots, no operators, no bans ({players})')
        check(all(p['address'] == '127.0.0.1' and p['op_level'] == 0 for p in players['online']),
              'online players have their address and no op level')
        wait_until('the sampler sees 2 players',
                   lambda: 'horizon_players_online 2\n' in api.call('GET', '/api/v1/metrics/prometheus')[1])
        text = api.call('GET', '/api/v1/metrics/prometheus')[1]
        check('horizon_up 1\n' in text and 'horizon_tps ' in text and 'horizon_mspt{stat="p95"}' in text,
              'Prometheus text: up, TPS and MSPT')

        def act(path, body, changed, line):
            reply = api('POST', path, body)
            check(reply['changed'] == changed and any(line in o for o in reply['output']),
                  f'{path} {body}: changed {changed}, "{line}" ({reply})')

        act('/api/v1/players/op', {'name': 'Kim0'}, True, 'Made Kim0 a server operator')
        check(listed(folder, 'ops.json') == ['Kim0'], 'ops.json has Kim0')
        check([p['op_level'] for p in api('GET', '/api/v1/players')['online'] if p['name'] == 'Kim0'] == [4],
              'Kim0 shows op level 4')
        act('/api/v1/players/op', {'name': 'Kim0'}, False, 'Nothing changed')
        act('/api/v1/players/deop', {'name': 'Kim0'}, True, 'Made Kim0 no longer a server operator')
        act('/api/v1/whitelist/add', {'name': 'Lou0'}, True, 'Added Lou0 to the whitelist')
        check(listed(folder, 'whitelist.json') == ['Lou0'], 'whitelist.json has Lou0')
        act('/api/v1/whitelist/remove', {'name': 'Lou0'}, True, 'Removed Lou0 from the whitelist')
        act('/api/v1/players/ban-ip', {'ip': IP, 'reason': 'spam bots'}, True, f'Banned IP {IP}: spam bots')
        check([b['ip'] for b in api('GET', '/api/v1/players')['ip_bans']] == [IP], 'players lists the IP ban')
        act('/api/v1/players/pardon-ip', {'ip': IP}, True, f'Unbanned IP {IP}')
        check(listed(folder, 'banned-ips.json') == [], 'banned-ips.json is empty again')

        status, text = api.call('POST', '/api/v1/players/ban', {'name': 'Kim0 extra'})
        check(status == 400, f'a name with a space is refused ({status} {text})')
        act('/api/v1/players/kick', {'name': 'Nobody'}, False, 'No player was found')
        act('/api/v1/players/kick', {'name': 'Kim0', 'reason': 'spamming chat'}, True, 'Kicked Kim0: spamming chat')
        server.wait_for(r'Kim0 lost connection: spamming chat', 30)
        wait_until('the kick and the leave streamed', lambda: stream.seen('kick', 'Kim0') and stream.seen('leave', 'Kim0'), 30)
        check(stream.seen('kick', 'Kim0')[0]['reason'] == 'spamming chat' == stream.seen('leave', 'Kim0')[0]['reason'],
              'kick and leave events give the reason')
        api('POST', '/api/v1/command', {'line': 'kill Lou0'})
        wait_until("Lou0's death streamed", lambda: stream.seen('death', 'Lou0'), 30)
        check(stream.seen('death', 'Lou0')[0]['message'] == 'Lou0 was killed', f'the death message ({stream.events})')
        polled = api('GET', '/api/v1/events?since=0&types=death')['events']
        check([(e['type'], e['name']) for e in polled] == [('death', 'Lou0')], f'events polled as JSON ({polled})')
        act('/api/v1/players/ban', {'name': 'Lou0', 'reason': 'cheating'}, True, 'Banned Lou0: cheating')
        server.wait_for(r'Lou0 lost connection', 30)
        bans = api('GET', '/api/v1/players')['bans']
        check([(b['name'], b['reason'], b['source']) for b in bans] == [('Lou0', 'cheating', 'Server')],
              'players lists Lou0 banned for cheating by Server')
        wait_until('no one online', lambda: online(api) == [], 30)
        act('/api/v1/players/pardon', {'name': 'Lou0'}, True, 'Unbanned Lou0')

        act('/api/v1/whitelist', {'enabled': True}, True, 'Whitelist is now turned on')
        check(api('GET', '/api/v1/players')['whitelist']['enabled'] is True, 'players says the whitelist is on')
        act('/api/v1/whitelist', {'enabled': False}, True, 'Whitelist is now turned off')
        check(listed(folder, 'banned-players.json') == [], 'banned-players.json is empty again')

        config = api('GET', '/api/v1/config')['config']
        check(config['api']['token'] == '(set)' and config['world']['difficulty'] == 'easy',
              'config: the token hidden, the difficulty easy')
        outcome = api('PATCH', '/api/v1/config', {'changes': {'world.difficulty': 'hard', 'server.max_players': 30}})
        check(outcome['applied'] == ['world.difficulty'] and outcome['restart'] == ['server.max_players'],
              f'config: the difficulty applies now, max_players at the next start ({outcome})')
        said = api('POST', '/api/v1/command', {'line': 'difficulty'})['output']
        check(any('The difficulty is Hard' in line for line in said), f'the game plays on Hard ({said})')
        check('difficulty = "hard"' in (folder / 'server.toml').read_text(), 'server.toml says hard')
        status, _ = api.call('PATCH', '/api/v1/config', {'changes': {'engine.path': '/bin/sh'}})
        check(status == 403, 'the API cannot change the program the server runs')

        name = api('POST', '/api/v1/backups', {'name': 'e2e'})['name']
        check(name in [b['name'] for b in api('GET', '/api/v1/backups')['backups']], f'backup {name} is listed')
        status, data = api.call('GET', f'/api/v1/backups/download?name={name}', raw=True)
        check(status == 200 and data == (folder / 'backups' / name).read_bytes() and data[:2] == b'PK',
              f'the download is the zip ({len(data)} bytes)')
        api('DELETE', f'/api/v1/backups?name={name}')
        check(not (folder / 'backups' / name).exists(), 'the backup is deleted')

        cli = [bins / f'horizon-server{EXE}', '--dir', folder]

        def run(*words):
            return subprocess.run([str(w) for w in cli + list(words)], capture_output=True, text=True,
                                  env=lab.game_env(), timeout=120)
        out = run('ban', 'Kim0', 'griefing', 'again')
        check(out.returncode == 0 and 'Banned Kim0: griefing again' in out.stdout,
              f'horizon-server ban ({out.stdout!r} {out.stderr!r})')
        out = run('players')
        check('Banned: Kim0 (griefing again)' in out.stdout, f'horizon-server players ({out.stdout!r})')
        out = run('pardon', 'Kim0')
        check(out.returncode == 0 and 'Unbanned Kim0' in out.stdout, f'horizon-server pardon ({out.stdout!r})')
        out = run('players', '--json')
        check(out.returncode == 0 and json.loads(out.stdout)['bans'] == [], 'horizon-server players --json')

        mcp = subprocess.Popen([str(bins / f'horizon-server{EXE}'), '--dir', str(folder), 'mcp'],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=lab.game_env())

        def rpc(i, method, params):
            mcp.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': i, 'method': method, 'params': params}) + '\n')
            mcp.stdin.flush()
            reply = json.loads(mcp.stdout.readline())
            check(reply['id'] == i and 'result' in reply, f'mcp {method} answers ({reply})')
            return reply['result']

        def tool(i, tool_name, **args):
            result = rpc(i, 'tools/call', {'name': tool_name, 'arguments': args})
            return result['isError'], json.loads(result['content'][0]['text'])

        rpc(1, 'initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 'e2e'}})
        names = [t['name'] for t in rpc(2, 'tools/list', {})['tools']]
        check({'players', 'kick', 'ban', 'events', 'set_config', 'restore'} <= set(names), f'mcp tools ({names})')
        error, banned = tool(3, 'ban', name='Kim0', reason='an agent decided')
        check(not error and banned['changed'] and 'Banned Kim0: an agent decided' in banned['output'],
              f'mcp ban ({banned})')
        error, _ = tool(4, 'pardon', name='Kim0')
        check(not error and listed(folder, 'banned-players.json') == [], 'mcp pardon')
        error, deaths = tool(5, 'events', since=0, types='death')
        check(not error and [e['name'] for e in deaths['events']] == ['Lou0'], f'mcp events ({deaths})')
        error, refused = tool(6, 'stop')
        check(error and 'confirm' in refused['error'], f'mcp stop without confirm is refused ({refused})')
        mcp.stdin.close()
        mcp.wait(timeout=30)

        console = subprocess.Popen([str(bins / f'horizon-server{EXE}'), '--dir', str(folder), 'console'],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                   text=True, env=lab.game_env())
        console.stdin.write('say hello from the console\nlist\n')
        console.stdin.flush()
        time.sleep(3)  # the lines come back through the event stream
        out, _ = console.communicate('', timeout=60)
        check(console.returncode == 0 and 'Attached' in out and 'Detached' in out, f'console attaches and detaches ({out!r})')
        check('[Server] hello from the console' in out and 'There are 0 of a max of 20 players online' in out,
              f'console commands run and their lines show ({out[-600:]!r})')
    finally:
        for b in bots:
            b.kill()
    api('POST', '/api/v1/stop')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-admin-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    work = Path(tempfile.mkdtemp(prefix='hz-server-admin-'))
    folder = work / 'server'
    folder.mkdir()
    port, api_port = lab.free_port(), lab.free_port()
    (folder / 'eula.txt').write_text('eula=true\n')
    (folder / 'server.toml').write_text(f'''[server]
port = {port}
online_mode = false
whitelist = false
view_distance = 3
simulation_distance = 3

[properties]
level-type = "minecraft:flat"
generate-structures = false
spawn-protection = 0

[engine]
path = {json.dumps(str(bins / f'horizon-client{EXE}'))}
resources = {json.dumps(str(lab.RESOURCES.resolve()))}
restart_on_crash = false

[backups]
every = "0"

[panel]
bind = ""

[api]
bind = "127.0.0.1:{api_port}"
token = "{TOKEN}"
''')
    server = Server([bins / f'horizon-server{EXE}', '--dir', folder, 'run'], folder, out / 'server.log')
    try:
        server.wait_for(DONE, 300)
        endpoint = json.loads((folder / '.horizon' / 'api.json').read_text())
        if (endpoint['address'], endpoint['token']) != (f'127.0.0.1:{api_port}', TOKEN):
            raise Failed(f'the API is not where [api] says: {endpoint}')
        scenario(bins, folder, server, port)
        server.process.wait(timeout=120)
    except (Failed, RuntimeError, OSError, subprocess.TimeoutExpired) as e:
        sys.exit(f'server_admin_e2e failed (logs in {out}): {e}')
    finally:
        server.stop()
        shutil.rmtree(work, ignore_errors=True)
    print('server_admin_e2e: the admin API listed, opped, whitelisted, banned, pardoned and kicked as vanilla does, '
          'changed settings and backups, and streamed what happened')


if __name__ == '__main__':
    main()
