#!/usr/bin/env python3
"""Horizon's client joins vanilla's online-mode server and Horizon's, with no real account.

A small mock of the Minecraft services (dev only: the discovery document and
the session service's join and hasJoined endpoints) stands in for Mojang's.
The pinned vanilla 26.3 dedicated server runs in online mode and finds the
mock through authlib's own override, `-Dminecraft.api.discovery.host`;
Horizon's client finds it through `HORIZON_API_DISCOVERY` and signs in with a
session file holding a mock token, as the launcher hands it over.

The client must answer the server's encryption request: generate the shared
secret, hash the server id with it and the server's public key, join through
the mock, RSA-wrap the secret and the challenge for the server, then encrypt
the connection with AES/CFB8. The server decrypts the secret with its private
key, computes the hash itself and asks the mock whether the player joined
with that hash: so vanilla's own code checks the hash, the RSA wrapping and
the cipher. Then the client plays as in the dedicated-server harness (stands
in the world with its chunks, chats, runs /list, leaves).

Also checked, with vanilla's messages:
- a token the session service rejects ("Failed to log in: Invalid session");
- a join the session service forgets (the server: "Failed to verify username!");
- no account at all (the client refuses before joining);
- a verified player the whitelist refuses;
- `whitelist add` in online mode: a player who joined before is known by name
  and let in; another name is looked up through the services' profile lookup,
  and one with no account is refused ("That player does not exist").

Horizon's dedicated server then runs the same steps in online mode
(`HORIZON_API_DISCOVERY` points it at the mock too): it must give the client
the same answers and log the same lines about the player as vanilla's
("UUID of player ...", "Disconnecting ...", "... lost connection: ...").

Chat is unsigned here: the mock serves no Mojang public keys, so the server
can't validate profile keys and doesn't enforce secure chat.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import shutil
import re
import subprocess
import sys
import threading
import time
import urllib.parse

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .test_dedicated_server_parity import Server, free_port
    from .integration_env import run_game
except ImportError:
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from test_dedicated_server_parity import Server, free_port
    from integration_env import run_game

NAME = 'online-mode-parity'
PLAYER = 'Tester'
PLAYER_UUID = '5f3c1e6a2b9d4c7e8a1f0b2c3d4e5f60'
GOOD, FORGET, BAD = 'mock-token-good', 'mock-token-forget', 'mock-token-rejected'
PROPERTIES = {
    'online-mode': 'true',
    'white-list': 'false',
    'level-seed': '12345',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'max-players': '5',
    'pause-when-empty-seconds': '-1',
}


def dashed(u):
    return f'{u[:8]}-{u[8:12]}-{u[12:16]}-{u[16:20]}-{u[20:]}'


class Services:
    """The mock session service. Records what was asked, never the tokens."""

    def __init__(self):
        self.joined = {}
        self.log = []
        self.lock = threading.Lock()
        services = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status, body=None):
                data = b'' if body is None else json.dumps(body).encode()
                self.send_response(status)
                if body is not None:
                    self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                url = urllib.parse.urlparse(self.path)
                query = dict(urllib.parse.parse_qsl(url.query))
                if url.path == '/minecraft/client':
                    return self.reply(200, services.discovery())
                if url.path.startswith('/users/profiles/minecraft/'):
                    # profiles/getByName: the account with that name, ignoring case.
                    name = urllib.parse.unquote(url.path.rsplit('/', 1)[1])
                    with services.lock:
                        services.log.append(('lookup', name))
                    if name.lower() == PLAYER.lower():
                        return self.reply(200, {'id': PLAYER_UUID, 'name': PLAYER})
                    return self.reply(404, {'error': 'NOT_FOUND', 'errorMessage': f'Couldn\'t find any profile with name {name}'})
                if url.path == '/session/minecraft/hasJoined':
                    with services.lock:
                        ok = services.joined.get(query.get('username')) == query.get('serverId')
                        services.log.append(('verify', query.get('username'), query.get('serverId'), ok))
                    if ok:
                        return self.reply(200, {'id': PLAYER_UUID, 'name': query['username'], 'properties': []})
                    return self.reply(204)
                # A bare `GET /` is a local tool probing every new port (T3 Code does, on the Mac mini), not
                # the game: none of the services has a root. `/publickeys` (getPublicKeys) is both servers
                # fetching the services' keys in the background, at any moment of a scenario: no login step.
                if url.path not in ('/', '/publickeys'):
                    with services.lock:
                        services.log.append(('unknown', url.path))
                self.reply(404, {'error': 'NotFound', 'path': url.path})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length', 0))) or b'{}')
                if self.path != '/session/minecraft/join':
                    with services.lock:
                        services.log.append(('unknown', self.path))
                    return self.reply(404, {'error': 'NotFound'})
                token, profile, server_id = body.get('accessToken'), body.get('selectedProfile'), body.get('serverId')
                accepted = token in (GOOD, FORGET) and profile == PLAYER_UUID
                with services.lock:
                    services.log.append(('join', profile, server_id, accepted))
                    if token == GOOD and accepted:
                        services.joined[PLAYER] = server_id
                if not accepted:
                    return self.reply(403, {'error': 'ForbiddenOperationException', 'errorMessage': 'Invalid token'})
                self.reply(204)

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def discovery(self):
        def endpoints(**names):
            return {'endpoints': {k: {'uri': self.url + v} for k, v in names.items()}}
        return {'environment': 'mock', 'product': 'minecraft', 'discovery': {
            'authentication': endpoints(getPublicKeys='/publickeys'),
            'session': endpoints(join='/session/minecraft/join', verify='/session/minecraft/hasJoined',
                                 getProfileById='/session/minecraft/profile/{profileId}'),
            'player': {'endpoints': {}},
            'profiles': {'endpoints': {'getTexture': {'validUris': [self.url + '/texture/{textureId}']},
                                       'getByName': {'uri': self.url + '/users/profiles/minecraft/{name}'}}},
            'telemetry': {'endpoints': {}},
        }}

    def take_log(self):
        with self.lock:
            log, self.log = self.log, []
            return log

    def stop(self):
        self.server.shutdown()


def session_file(directory, token):
    """What the launcher writes: owner-only, read once by the game."""
    path = directory / 'session.json'
    path.unlink(missing_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump({'name': PLAYER, 'uuid': dashed(PLAYER_UUID), 'access_token': token, 'user_type': 'msa'}, f)
    return path


def run_client(binary, build, port, label, token, discovery):
    directory = build / f'client-{label}'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir()
    report = directory / 'report.json'
    env = lab.game_env(HORIZON_NET_REPORT=report, HORIZON_API_DISCOVERY=discovery)
    command = [binary, '--resources', lab.RESOURCES, '--render-distance', 2,
               '--server', f'127.0.0.1:{port}', '--name', PLAYER]
    if token:
        command += ['--session-file', session_file(directory, token)]
    with (build / f'client-{label}.log').open('w') as log:
        run_game([str(c) for c in command], cwd=directory, env=env, stdout=log, stderr=subprocess.STDOUT,
                       timeout=300, check=True)
    got = json.loads(report.read_text())
    got['session file left'] = (directory / 'session.json').exists()
    return got

# The server's lines about the player, as both servers print them.
PLAYER_LINE = re.compile(r'^\[\d\d:\d\d:\d\d\] \[([^\]]+)\]: (.*(?:Tester|Username|Disconnecting|verify username).*)$')


def player_lines(lines, starts=()):
    """The lines about the player, login by login (from each index in `starts` to the next), with two races of
    vanilla's own taken out:
    - the server's thread and the login's authenticator each log in their own order, but which of the two logs
      first is a race (the authenticator disconnects a player, then logs why, while the server thread may already
      log the lost connection): each login's lines are grouped by thread;
    - a player the server disconnects ("Disconnecting ...: reason") closes the connection once it reads why, and
      when that comes before the server closes it, the lost connection is logged as "Disconnected": the kick's
      reason stands for both."""
    def thread(line):
        # `[User Authenticator #2/INFO]`: the thread, without the level; the login's authenticator first.
        name = (m := PLAYER_LINE.match(line)) and m.group(1).rsplit('/', 1)[0] or ''
        return name == 'Server thread', name

    bounds = [0, *starts, len(lines)]
    out = []
    for a, b in zip(bounds, bounds[1:]):
        login = []
        for line in sorted(lines[a:b], key=thread):
            if m := PLAYER_LINE.match(line):
                text = re.sub(r'/127\.0\.0\.1:\d+', '/127.0.0.1:_', m.group(2))
                text = re.sub(r'entity id \d+ at \(.*\)', 'entity id _ at _', text)
                login.append(f'[{m.group(1)}] {text}')
        kicks = {l.rsplit('): ', 1)[1] for l in login if '] Disconnecting ' in l}
        if len(kicks) == 1:
            login = [l.removesuffix('Disconnected') + next(iter(kicks)) if l.endswith(') lost connection: Disconnected')
                     else l for l in login]
        out += login
    return out


def wait_after(server, mark, pattern, timeout=30):
    """A line matching `pattern` among those the server printed after `mark`."""
    regex = re.compile(pattern)
    deadline = time.time() + timeout
    while time.time() < deadline:
        if any(regex.search(l) for l in server.lines[mark:]):
            return
        time.sleep(0.2)
    raise AssertionError(f'no {pattern!r} after line {mark} of {server.log_path.name}')


def run_side(label, command, binary, build, services, discovery):
    """The six logins against one server; its checks and its log about the player."""
    port = free_port()
    directory = build / label
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir()
    props = dict(PROPERTIES, **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    os.environ.update(lab.game_env(HORIZON_API_DISCOVERY=discovery))
    server = Server(command, directory, f'{label}-online')
    checks = []
    starts = []

    def check(name, actual, expected):
        checks.append(f'{name}: ' + ('ok' if actual == expected else f'expected {expected!r}, got {actual!r}'))

    def login(name, token):
        """One client run, over once the server logged the end of its connection: the server notices the
        closed connection at its next tick, and the next run's lines must not come before that one."""
        start = len(server.lines)
        starts.append(start)
        got = run_client(binary, build, port, f'{label}-{name}', token, discovery)
        wait_after(server, start, r'lost connection: ')
        return got

    try:
        server.wait_for(r'Done \(', timeout=600)
        services.take_log()

        # 1. Signed in: the full encrypted handshake, then play.
        got = login('signed-in', GOOD)
        server.wait_for(r'Tester left the game', timeout=30)
        log = services.take_log()
        joins = [e for e in log if e[0] == 'join']
        verifies = [e for e in log if e[0] == 'verify']
        check('signed-in: connected throughout', got['error'], None)
        check('signed-in: joined the session service once', [(e[1], e[3]) for e in joins], [(PLAYER_UUID, True)])
        check('signed-in: server verified the same hash',
              [(e[1], e[2], e[3]) for e in verifies], [(PLAYER, joins[0][2] if joins else None, True)])
        check('signed-in: own chat shown', any('<Tester> hello from the horizon client' in c for c in got['chat']), True)
        check('signed-in: /list reply shown',
              any('There are 1 of a max of 5 players online: Tester' in c for c in got['chat']), True)
        check('signed-in: chunks received', got['chunks'] > 0, True)
        check('signed-in: session file deleted', got['session file left'], False)
        lines = '\n'.join(server.lines)
        check('signed-in: server log', [k for k in [f'UUID of player Tester is {dashed(PLAYER_UUID)}',
                                                    'Tester joined the game',
                                                    '<Tester> hello from the horizon client',
                                                    'Tester left the game'] if k in lines], [
            f'UUID of player Tester is {dashed(PLAYER_UUID)}', 'Tester joined the game',
            '<Tester> hello from the horizon client', 'Tester left the game'])

        # 2. A token the session service rejects: vanilla's message, no key sent.
        mark = len(server.lines)
        got = login('rejected', BAD)
        check('rejected token: client message', got['error'],
              'disconnected: Failed to log in: Invalid session (Try restarting your game and the launcher)')
        check('rejected token: no verification', [e[0] for e in services.take_log()], ['join'])

        # 3. The session service forgets the join: the server refuses the player.
        got = login('forgotten', FORGET)
        server.wait_for(r'Failed to verify username', timeout=30)
        check('forgotten join: client message', got['error'], 'disconnected: Failed to verify username!')
        check('forgotten join: server verified and refused',
              [(e[0], e[-1]) for e in services.take_log()], [('join', True), ('verify', False)])

        # 4. No account: refused by the client before anything is sent to the services.
        got = login('no-account', None)
        check('no account: client message', got['error'] or '', 'disconnected: This server is in online mode. '
              'Sign in with your Microsoft account in the Horizon launcher to join it.')
        check('no account: no service calls', services.take_log(), [])
        check('no player joined after the first', 'joined the game' in '\n'.join(server.lines[mark:]), False)

        # 5. Verified, but not on the whitelist.
        server.console('whitelist on')
        server.wait_for(r'Whitelist is now turned on', timeout=30)
        got = login('not-whitelisted', GOOD)
        server.wait_for(r'Disconnecting Tester', timeout=30)
        check('not whitelisted: client message', got['error'], 'disconnected: You are not white-listed on this server!')
        check('not whitelisted: verified first', [(e[0], e[-1]) for e in services.take_log()],
              [('join', True), ('verify', True)])

        # 6. Whitelisted by name: the server knows the player from their first join (its name cache), so it
        #    asks the services nothing, and lets them in.
        mark = len(server.lines)
        server.console('whitelist add tester')
        wait_after(server, mark, r'Added Tester to the whitelist')
        check('whitelist add: known player, no lookup', services.take_log(), [])
        got = login('whitelisted', GOOD)
        wait_after(server, mark, r'Tester left the game')
        check('whitelisted: connected throughout', got['error'], None)
        services.take_log()
        # A name nobody joined with: the server looks the account up, and there is none.
        mark = len(server.lines)
        server.console('whitelist add Nobody')
        wait_after(server, mark, r'That player does not exist')
        check('whitelist add: lookup failure logged',
              any("Couldn't find profile with name: nobody" in l for l in server.lines[mark:]), True)
        check('whitelist add: no such account', services.take_log(), [('lookup', 'nobody')])
        time.sleep(1)
    finally:
        server.stop()
    return checks, player_lines(server.lines, starts)


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    build.mkdir(parents=True, exist_ok=True)
    report = build / 'report.json'
    report.unlink(missing_ok=True)
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client'], build,
                  'compile-rust')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'],
                                                  cwd=lab.ROOT, text=True))
    binary = Path(metadata['target_directory']) / 'fast' / ('horizon-client.exe' if os.name == 'nt' else 'horizon-client')
    dependencies = os.pathsep.join(map(str, [jars['server'], *libraries]))

    services = Services()
    discovery = services.url + '/minecraft/client'
    vanilla = ['java', '-Xmx2g', '-Duser.timezone=UTC', f'-Dminecraft.api.discovery.host={discovery}',
               '-cp', dependencies, 'net.minecraft.server.Main', 'nogui']
    horizon = [binary, '--resources', lab.LOCAL / 'horizon-client/26.3', '--dedicated', '--nogui']
    checks, transcripts = [], {}
    try:
        for label, command in [('vanilla', vanilla), ('horizon', horizon)]:
            side_checks, transcripts[label] = run_side(label, command, binary, build, services, discovery)
            checks += [f'{label}: {c}' for c in side_checks]
    finally:
        services.stop()
    for label in transcripts:
        (build / f'{label}.txt').write_text('\n'.join(transcripts[label]) + '\n', encoding='utf-8')
    if 'horizon' in transcripts:
        import difflib
        diff = [l for l in difflib.unified_diff(transcripts['vanilla'], transcripts['horizon'], 'vanilla', 'horizon',
                                                n=0, lineterm='') if l[:1] in '+-' and l[:3] not in ('+++', '---')]
        checks.append('server log about the player: ' + ('ok' if not diff else 'differs:\n  ' + '\n  '.join(diff)))

    for line in checks:
        print(line)
    failed = [c for c in checks if not c.endswith(': ok')]
    if failed:
        raise AssertionError(f'{len(failed)} of {len(checks)} checks failed (logs in {build})')
    lab.write_json(report, dict(
        schema=1, minecraft=lock['minecraft']['version'], platform=platform.platform(),
        java=subprocess.check_output(['java', '--version'], text=True).splitlines()[0], checks=len(checks),
        scope='Online-mode login against vanilla\'s dedicated server and Horizon\'s, with a mock session service: '
              'server hash, RSA key packet, AES/CFB8, session join and verify, play; rejected token, forgotten '
              'join, no account, whitelist, whitelist add by account name; the servers\' log lines about the player.'))
    print(f'online mode: {len(checks)} checks ok')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
