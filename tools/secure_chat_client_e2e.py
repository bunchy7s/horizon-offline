#!/usr/bin/env python3
"""Horizon's client signing its chat for vanilla's dedicated server, with no real account.

A mock of the Minecraft services stands in for Mojang's: its key set holds a key pair made here, its
session service lets the player join, and its certificates endpoint (`player` / `getCertificates`) hands
out a fresh chat key pair whose public key the mock signs (publicKeySignatureV2: SHA1withRSA over the
profile id, the expiry and the key), as Mojang's does. Vanilla's server finds the mock through
`-Dminecraft.api.discovery.host`, Horizon's client through `HORIZON_API_DISCOVERY`; the client signs in
with a session file naming a made-up access token, as the launcher would hand it over.

Horizon's headless client (`HORIZON_NET_REPORT`, the net harness) joins, sends two chat messages, a
`/msg` to itself (a signed command argument) and `/list`, then leaves. Two servers:

- online mode with enforce-secure-profile: the client sends its chat session, the server logs the chat
  as secure (no "[Not Secure]"), the whisper arrives, and nothing is refused or disconnected;
- offline mode: the server plays the account under its offline id, so the client keeps no session and
  chats unsigned as before ("[Not Secure]" in the log).

    python3 tools/secure_chat_client_e2e.py --binary target/fast/horizon-client \\
        --resources .local/horizon-client/26.3 --server-jar .local/minecraft/26.3/jars/server.jar \\
        --java /path/to/java25

Needs `openssl` on PATH (the mock's RSA). Logs and reports go to --out.
"""
import argparse
import base64
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import uuid
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from integration_env import run_game  # noqa: E402
from test_dedicated_server_parity import Server  # noqa: E402

PLAYER = 'Tester'
PLAYER_ID = uuid.UUID(hashlib.md5(b'account:Tester').hexdigest())
HELD = 'held back by Chat Delay, then flushed'
AFTER = 'signed after the flush, which the last seen must count'
CHATS = ['hello from a signed client', 'a second message, after the first was seen', HELD, AFTER]
# Setting Chat Delay to 0 flushes the held message. If the flush didn't count it as seen, the next
# message's last-seen update would no longer match the server's and it would disconnect the client.
CLICKED = 'a clicked whisper the player never read'
# Commands the player didn't type: one with no issue goes, one the server doesn't know asks and goes on yes, one that
# needs permissions asks (and is /stop: it must not go on no), and one with a message argument only reaches the chat box.
CLICKS = ['.click list', '.click nosuchcommand', '.answer yes', '.click stop', '.answer no',
          f'.click msg {PLAYER} {CLICKED}', '.answer yes']
PARSE_ERRORS = 'multiplayer.confirm_command.parse_errors'
SIGNATURE_REQUIRED = 'multiplayer.confirm_command.signature_required'
PROMPTS = {
    # An op on the online server, where the account's id is known: /stop is in its tree. Offline, it isn't.
    'online': [None, PARSE_ERRORS, 'multiplayer.confirm_command.permissions_required', SIGNATURE_REQUIRED],
    'offline': [None, PARSE_ERRORS, PARSE_ERRORS, SIGNATURE_REQUIRED],
}
LINES = [*CHATS[:2], '.delay 60', HELD, '.delay 0', AFTER, f'/msg {PLAYER} a whisper with a signed argument', '/list',
         *CLICKS]
PROPERTIES = {
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'spawn-protection': '0',
    'view-distance': '3',
    'simulation-distance': '3',
    'pause-when-empty-seconds': '-1',
}
SCENARIOS = {
    'online': {'online-mode': 'true', 'enforce-secure-profile': 'true'},
    'offline': {'online-mode': 'false'},
}


def openssl(*args, data=None):
    return subprocess.run(['openssl', *args], input=data, capture_output=True, check=True).stdout


class Keys:
    """An RSA key pair in a scratch folder, through the openssl command."""

    def __init__(self, directory, name, bits):
        self.pem = directory / f'{name}.pem'
        openssl('genpkey', '-algorithm', 'RSA', '-pkeyopt', f'rsa_keygen_bits:{bits}', '-out', str(self.pem))
        self.private_der = openssl('pkcs8', '-topk8', '-nocrypt', '-in', str(self.pem), '-outform', 'DER')
        self.public_der = openssl('pkey', '-in', str(self.pem), '-pubout', '-outform', 'DER')

    def sign_sha1(self, data):
        return openssl('dgst', '-sha1', '-sign', str(self.pem), data=data)


def pem(label, der):
    body = base64.b64encode(der).decode()
    lines = [body[i:i + 76] for i in range(0, len(body), 76)]
    return f'-----BEGIN {label}-----\n' + '\n'.join(lines) + f'\n-----END {label}-----\n'


def iso(moment):
    return moment.strftime('%Y-%m-%dT%H:%M:%S.%fZ')


class Services:
    """Discovery, the key set, join and hasJoined, and the certificates endpoint."""

    def __init__(self, scratch):
        self.scratch = scratch
        self.key = Keys(scratch, 'services', 2048)
        self.joined = {}
        self.certificates = []
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
                if url.path == '/publickeys':
                    key = {'publicKey': base64.b64encode(services.key.public_der).decode()}
                    return self.reply(200, {'profilePropertyKeys': [key], 'playerCertificateKeys': [key]})
                if url.path == '/session/minecraft/hasJoined':
                    with services.lock:
                        ok = query.get('username') == PLAYER and services.joined.get(PLAYER_ID.hex) == query.get('serverId')
                    if ok:
                        return self.reply(200, {'id': PLAYER_ID.hex, 'name': PLAYER, 'properties': []})
                    return self.reply(204)
                self.reply(404, {'error': 'NotFound'})

            def do_POST(self):
                length = int(self.headers.get('Content-Length', 0))
                raw = self.rfile.read(length) if length else b''
                if self.path == '/session/minecraft/join':
                    body = json.loads(raw or b'{}')
                    if body.get('accessToken') != 'mock-token':
                        return self.reply(403, {'error': 'ForbiddenOperationException'})
                    with services.lock:
                        services.joined[body.get('selectedProfile')] = body.get('serverId')
                    return self.reply(204)
                if self.path == '/player/certificates':
                    if self.headers.get('Authorization') != 'Bearer mock-token':
                        return self.reply(401, {'error': 'Unauthorized'})
                    return self.reply(200, services.certificate())
                self.reply(404, {'error': 'NotFound'})

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.url = f'http://127.0.0.1:{self.server.server_address[1]}'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def certificate(self):
        """KeyPairResponse: a new chat key pair, signed for the player's profile id."""
        with self.lock:
            n = len(self.certificates)
            self.certificates.append(n)
        key = Keys(self.scratch, f'player-{n}', 2048)
        now = datetime.now(timezone.utc)
        expires = now + timedelta(hours=48)
        millis = int(expires.timestamp() * 1000)
        payload = PLAYER_ID.bytes + struct.pack('>q', millis) + key.public_der
        return {
            'keyPair': {'privateKey': pem('RSA PRIVATE KEY', key.private_der),
                        'publicKey': pem('RSA PUBLIC KEY', key.public_der)},
            'publicKeySignature': base64.b64encode(b'v1 is not used').decode(),
            'publicKeySignatureV2': base64.b64encode(self.key.sign_sha1(payload)).decode(),
            'expiresAt': iso(expires.replace(microsecond=millis % 1000 * 1000)),
            'refreshedAfter': iso(now + timedelta(hours=40)),
        }

    def discovery(self):
        def endpoints(**names):
            return {'endpoints': {k: {'uri': self.url + v} for k, v in names.items()}}
        return {'environment': 'mock', 'product': 'minecraft', 'discovery': {
            'authentication': endpoints(getPublicKeys='/publickeys'),
            'session': endpoints(join='/session/minecraft/join', verify='/session/minecraft/hasJoined',
                                 getProfileById='/session/minecraft/profile/{profileId}'),
            'player': endpoints(getCertificates='/player/certificates'),
            'profiles': {'endpoints': {'getTexture': {'validUris': [self.url + '/texture/{textureId}']}}},
            'telemetry': {'endpoints': {}},
        }}

    def stop(self):
        self.server.shutdown()


def join(args, port, out, label, discovery):
    """The headless client joins with the mock account, chats and leaves: its report."""
    home = Path(tempfile.mkdtemp(prefix=f'secure-chat-{label}-'))
    report = out / f'{label}-report.json'
    session = home / 'session.json'
    session.write_text(json.dumps({'name': PLAYER, 'uuid': str(PLAYER_ID), 'access_token': 'mock-token',
                                   'user_type': 'msa'}))
    env = dict(os.environ, HORIZON_NET_REPORT=str(report), HORIZON_NET_CHAT=';'.join(LINES),
               HORIZON_API_DISCOVERY=discovery, RUST_LOG='info')
    with (out / f'{label}-client.log').open('w') as log:
        run_game([str(args.binary), '--resources', str(args.resources), '--render-distance', '2',
                  '--server', f'127.0.0.1:{port}', '--name', PLAYER, '--session-file', str(session)],
                 cwd=home, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=300, check=True)
    shutil.rmtree(home, ignore_errors=True)
    return json.loads(report.read_text(encoding='utf-8'))


def run(args, scenario, services, out):
    discovery = services.url + '/minecraft/client'
    directory = out / f'server-{scenario}'
    shutil.rmtree(directory, ignore_errors=True)
    directory.mkdir(parents=True)
    port = lab.free_port()
    props = dict(PROPERTIES, **SCENARIOS[scenario], **{'server-port': str(port)})
    (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
    (directory / 'eula.txt').write_text('eula=true\n')
    if scenario == 'online':
        (directory / 'ops.json').write_text(json.dumps(
            [{'uuid': str(PLAYER_ID), 'name': PLAYER, 'level': 4, 'bypassesPlayerLimit': False}]))
    command = [str(args.java), '-Xmx2g', f'-Dminecraft.api.discovery.host={discovery}', '-jar',
               str(args.server_jar), 'nogui']
    server = Server(command, directory, f'vanilla-{scenario}')
    try:
        server.wait_for(r'Done \(', timeout=600)
        report = join(args, port, out, scenario, discovery)
        server.wait_for(rf'{PLAYER} (left the game|lost connection)', timeout=60)
    finally:
        server.stop()
    log = server.lines
    problems = []
    if report.get('error'):
        problems.append(f'client: {report["error"]}')
    chat = report.get('chat', [])
    secure = scenario == 'online'
    if report.get('signed') != secure:
        problems.append(f'the client {"did not sign" if secure else "signed"} its chat')
    for line in CHATS:
        logged = [l for l in log if f'<{PLAYER}> {line}' in l]
        if not logged:
            problems.append(f'the server did not log {line!r}')
        elif secure == any('[Not Secure]' in l for l in logged):
            problems.append(f'the server logged {line!r} as {"not " if secure else ""}secure: {logged}')
        if not any(line in c for c in chat):
            problems.append(f'the client was not shown {line!r}')
    if not any('a whisper with a signed argument' in c for c in chat):
        problems.append('the whisper did not arrive')
    if report.get('prompts') != PROMPTS[scenario]:
        problems.append(f'clicked commands asked {report.get("prompts")}, not {PROMPTS[scenario]}')
    if not any('nosuchcommand' in c for c in chat):
        problems.append('the unknown command, answered yes, did not reach the server')
    if report.get('chat_box') != f'/msg {PLAYER} {CLICKED}':
        problems.append(f'the chat box holds {report.get("chat_box")!r}')
    if any(CLICKED in c for c in chat) or any(CLICKED in l for l in log):
        problems.append('the clicked whisper was sent without the player sending it')
    left = next(i for i, l in enumerate(log) if re.search(rf'{PLAYER} (left the game|lost connection)', l))
    if any('Stopping the server' in l for l in log[:left]):
        problems.append('/stop ran though it was answered no')
    refused = [c for c in chat if re.search(r'(?i)chat disabled|not secure|validation|invalid|missing profile', c)]
    refused += [l for l in log if re.search(r'chat_validation_failed|invalid_signature|missingProfileKey|'
                                            r'Failed to validate|unsigned', l)]
    if refused:
        problems.append(f'refusals: {refused}')
    (out / f'{scenario}-summary.json').write_text(json.dumps({'report': report, 'log': log}, indent=2),
                                                  encoding='utf-8')
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--resources', type=Path, default=lab.RESOURCES)
    parser.add_argument('--server-jar', type=Path, required=True)
    parser.add_argument('--java', type=Path, default=Path('java'))
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'secure-chat-client-e2e')
    parser.add_argument('--scenario', choices=SCENARIOS, action='append')
    args = parser.parse_args()
    args.binary, args.server_jar, args.resources = (p.resolve() for p in (args.binary, args.server_jar,
                                                                           args.resources))
    shutil.rmtree(args.out, ignore_errors=True)
    args.out.mkdir(parents=True)
    scratch = Path(tempfile.mkdtemp(prefix='secure-chat-keys-'))
    services = Services(scratch)
    failures = {}
    try:
        for scenario in args.scenario or SCENARIOS:
            problems = run(args, scenario, services, args.out)
            print(f'{scenario}: {"ok" if not problems else "FAILED"}')
            for p in problems:
                print(f'  {p}')
            if problems:
                failures[scenario] = problems
    finally:
        services.stop()
        shutil.rmtree(scratch, ignore_errors=True)
    if failures:
        raise SystemExit(f'secure chat client e2e failed (logs in {args.out})')
    print(f'certificates handed out: {len(services.certificates)}')


if __name__ == '__main__':
    main()
