#!/usr/bin/env python3
"""Vanilla's server management protocol (JSON-RPC over a WebSocket), Horizon's against vanilla's.

The pinned vanilla server (`net.minecraft.server.Main nogui`) and Horizon's (`horizon-server run`, which
serves the protocol beside the game it supervises) start in folders with the same lists (ops.json,
whitelist.json, banned-players.json, banned-ips.json) and the same `management-server-*` settings (a fixed
secret, one allowed origin, and TLS, vanilla's default, with a PKCS12 keystore `keytool` makes). Each is then
sent the same things over TLS, and every answer is compared exactly:

- HTTP requests without the secret, with a wrong one, and with it in the WebSocket subprotocol from an
  origin that isn't allowed: the status and the JSON body;
- the handshake with the secret in the subprotocol from the allowed origin: the subprotocol answered;
- over a WebSocket with the bearer token, a sequence of messages: malformed JSON, a bare number, unknown and
  unparsable method names, a bad request id, wrong parameters, a notification (no answer), a batch, an
  answer to nothing, `rpc.discover` (its document must also equal `reports/json-rpc-api-schema.json` from
  vanilla's data generator), `server/status`, the four lists, `players`, kicking nobody, `server/save` and
  `server/system_message` with no one online, the four lists changed (`add`, `remove`, `set`,
  `clear`; with no profile service, players named resolve to offline players on both), and `server/stop`;
- the TLS version each server agrees on.

A parse error's `data` is Gson's message in vanilla and serde's in Horizon: left out. The notifications vanilla
sends meanwhile (`server/started`, `server/saving`...) are set aside: Horizon doesn't send them yet.
"""
import base64
import json
import os
from pathlib import Path
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import time

try:
    from . import lab, oracle_harness as oh, test_rust_parity as base
    from .server_bench import DONE, Server
except ImportError:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import lab
    import oracle_harness as oh
    import test_rust_parity as base
    from server_bench import DONE, Server

NAME = 'management-protocol-parity'
EXE = '.exe' if os.name == 'nt' else ''
SECRET = 'abcdefghijABCDEFGHIJ0123456789abcdefghij'
ORIGIN = 'http://localhost:3000'
KIM = {'uuid': '93e029a5-04a0-349d-90b2-f7e145b2644f', 'name': 'Kim0'}
LOU = {'uuid': '94d84bd8-1dc1-3f9b-88b6-c0c9dc9aa57d', 'name': 'Lou0'}
LISTS = {
    'ops.json': [dict(KIM, level=3, bypassesPlayerLimit=True)],
    'whitelist.json': [LOU],
    'banned-players.json': [dict(LOU, created='2026-10-31 14:00:00 +0000', source='Server',
                                 expires='2026-11-30 12:30:00 +0000', reason='griefing')],
    'banned-ips.json': [{'ip': '203.0.113.7', 'created': '2026-10-31 14:00:00 +0000', 'source': 'Server',
                         'expires': 'forever', 'reason': 'Banned by an operator.'}],
}
KEYSTORE_PASSWORD = 'harness-keystore-password'
# Nothing listens there: neither server finds a profile service, so names resolve to offline players on both,
# whatever the network answers.
NO_SERVICES = 'http://127.0.0.1:9'
# What both servers are sent over the socket, in order; None marks what must get no answer.
MESSAGES = [
    '{nope',
    '5',
    '{"jsonrpc":"2.0","id":1,"method":"minecraft:nope"}',
    '{"jsonrpc":"2.0","id":2,"method":"Bad Name"}',
    '{"jsonrpc":"2.0","id":{"a":1},"method":"minecraft:players"}',
    '{"jsonrpc":"2.0","id":3,"method":"rpc.discover","params":{}}',
    '{"jsonrpc":"2.0","id":4,"method":"minecraft:players","params":[1]}',
    '{"jsonrpc":"2.0","id":5,"method":"minecraft:server/save"}',
    '{"jsonrpc":"2.0","id":6,"method":"minecraft:server/save","params":{"other":true}}',
    '{"jsonrpc":"2.0","id":7,"method":"minecraft:server/save","params":[true,false]}',
    '{"jsonrpc":"2.0","method":"minecraft:server/status"}',
    '{"jsonrpc":"2.0","id":8,"result":true}',
    '{"jsonrpc":"2.0","id":9}',
    '[{"jsonrpc":"2.0","id":10,"method":"minecraft:allowlist"},{"jsonrpc":"2.0","method":"minecraft:bans"},'
    '{"jsonrpc":"2.0","id":"eleven","method":"minecraft:operators","params":[]}]',
    '[]',
    '{"jsonrpc":"2.0","id":12,"method":"rpc.discover"}',
    '{"jsonrpc":"2.0","id":13,"method":"server/status"}',
    '{"jsonrpc":"2.0","id":null,"method":"minecraft:allowlist"}',
    '{"jsonrpc":"2.0","id":14,"method":"minecraft:bans"}',
    '{"jsonrpc":"2.0","id":15,"method":"minecraft:ip_bans"}',
    '{"jsonrpc":"2.0","id":16,"method":"minecraft:operators"}',
    '{"jsonrpc":"2.0","id":17,"method":"minecraft:players"}',
    '{"jsonrpc":"2.0","id":18,"method":"minecraft:players/kick","params":{"kick":[{"player":{"name":"Nobody"}}]}}',
    '{"jsonrpc":"2.0","id":19,"method":"minecraft:server/save","params":[false]}',
    '{"jsonrpc":"2.0","id":20,"method":"minecraft:server/system_message",'
    '"params":[{"message":{"literal":"Hello"},"overlay":false}]}',
    '{"jsonrpc":"2.0","id":21,"method":"minecraft:server/system_message",'
    '"params":[{"message":{"translatable":"chat.type.announcement","translatableParams":["a","b"]},'
    '"overlay":true,"receivingPlayers":[]}]}',
    '{"jsonrpc":"2.0","id":22,"method":"minecraft:server/system_message","params":[{"message":{},"overlay":false}]}',
    # The lists changed: players found by name (offline players here) or by an id the user cache knows.
    '{"jsonrpc":"2.0","id":23,"method":"minecraft:allowlist/add","params":[[{"name":"Kim0"}]]}',
    '{"jsonrpc":"2.0","id":24,"method":"minecraft:allowlist/remove","params":{"players":[{"name":"Lou0"}]}}',
    '{"jsonrpc":"2.0","id":25,"method":"minecraft:allowlist/set","params":[[{"name":"Ann0"},'
    '{"id":"93e029a5-04a0-349d-90b2-f7e145b2644f"}]]}',
    '{"jsonrpc":"2.0","id":26,"method":"minecraft:allowlist/clear"}',
    '{"jsonrpc":"2.0","id":27,"method":"minecraft:bans/add","params":[[{"player":{"name":"Kim0"},"reason":"spam",'
    '"expires":"2027-01-01T00:00:00Z"},{"player":{"name":"Ann0"}}]]}',
    '{"jsonrpc":"2.0","id":28,"method":"minecraft:bans/remove","params":[[{"name":"Lou0"}]]}',
    '{"jsonrpc":"2.0","id":29,"method":"minecraft:bans/set","params":[[{"player":{"name":"Kim0"},"reason":"spam",'
    '"expires":"2027-01-01T00:00:00Z"},{"player":{"name":"Ann0"},"source":"Moderator"}]]}',
    '{"jsonrpc":"2.0","id":30,"method":"minecraft:bans"}',
    '{"jsonrpc":"2.0","id":31,"method":"minecraft:bans/clear"}',
    '{"jsonrpc":"2.0","id":32,"method":"minecraft:ip_bans/add","params":[[{"ip":"198.51.100.4","reason":"proxy",'
    '"expires":"2027-01-01T00:00:00Z"},{"ip":"not an address"},{"player":{"name":"Nobody"}}]]}',
    '{"jsonrpc":"2.0","id":33,"method":"minecraft:ip_bans/remove","params":[["203.0.113.7"]]}',
    '{"jsonrpc":"2.0","id":34,"method":"minecraft:ip_bans/set","params":[[{"ip":"198.51.100.4","reason":"proxy",'
    '"expires":"2027-01-01T00:00:00Z"},{"ip":"2001:db8::1","source":"Bot"},{"ip":"bad"}]]}',
    '{"jsonrpc":"2.0","id":35,"method":"minecraft:ip_bans/clear"}',
    '{"jsonrpc":"2.0","id":36,"method":"minecraft:operators/add","params":[[{"player":{"name":"Lou0"},'
    '"permissionLevel":2},{"player":{"name":"Ann0"}}]]}',
    '{"jsonrpc":"2.0","id":37,"method":"minecraft:operators/set","params":[[{"player":{"name":"Lou0"},'
    '"permissionLevel":2,"bypassesPlayerLimit":false},{"player":{"name":"Ann0"},"bypassesPlayerLimit":true},'
    '{"player":{"id":"93e029a5-04a0-349d-90b2-f7e145b2644f"}}]]}',
    '{"jsonrpc":"2.0","id":38,"method":"minecraft:operators/remove","params":[[{"name":"Lou0"}]]}',
    '{"jsonrpc":"2.0","id":39,"method":"minecraft:operators/clear"}',
]


def management_properties(port, keystore):
    return {
        'management-server-enabled': 'true',
        'management-server-host': '127.0.0.1',
        'management-server-port': str(port),
        'management-server-secret': SECRET,
        'management-server-tls-enabled': 'true',
        'management-server-tls-keystore': str(keystore),
        'management-server-tls-keystore-password': KEYSTORE_PASSWORD,
        'management-server-allowed-origins': ORIGIN,
    }


def make_keystore(work):
    """A PKCS12 keystore with a self-signed certificate for 127.0.0.1, made with the JDK's keytool, and a TLS
    client context that trusts it."""
    keytool = Path(shutil.which('java')).resolve().parent / f'keytool{EXE}'
    keystore, certificate = work / 'management.p12', work / 'management.pem'
    common = ['-storetype', 'PKCS12', '-keystore', str(keystore), '-storepass', KEYSTORE_PASSWORD]
    subprocess.run([keytool, '-genkeypair', '-alias', 'management', '-keyalg', 'RSA', '-keysize', '2048',
                    '-dname', 'CN=localhost', '-ext', 'SAN=dns:localhost,ip:127.0.0.1', '-validity', '2',
                    '-keypass', KEYSTORE_PASSWORD, *common], check=True, capture_output=True)
    subprocess.run([keytool, '-exportcert', '-rfc', '-alias', 'management', '-file', str(certificate), *common],
                   check=True, capture_output=True)
    context = ssl.create_default_context(cafile=str(certificate))
    # A refused request's connection may end without TLS's close_notify.
    context.options |= getattr(ssl, 'OP_IGNORE_UNEXPECTED_EOF', 0)
    return keystore, context


def connect(port, context):
    sock = socket.create_connection(('127.0.0.1', port), timeout=60)
    return context.wrap_socket(sock, server_hostname='127.0.0.1')


def write_lists(folder):
    for file, entries in LISTS.items():
        (folder / file).write_text(json.dumps(entries, indent=2))
    (folder / 'eula.txt').write_text('eula=true\n')


class Socket:
    """A WebSocket client: the handshake, masked text frames out, text frames in."""

    def __init__(self, port, context, headers):
        self.sock = connect(port, context)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f'GET / HTTP/1.1', f'Host: 127.0.0.1:{port}', 'Upgrade: websocket', 'Connection: Upgrade',
                 f'Sec-WebSocket-Key: {key}', 'Sec-WebSocket-Version: 13', *headers]
        self.sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
        self.buffer = b''
        while b'\r\n\r\n' not in self.buffer:
            data = self.sock.recv(4096)
            if not data:
                break
            self.buffer += data
        head, _, self.buffer = self.buffer.partition(b'\r\n\r\n')
        self.head = head.decode('iso-8859-1').split('\r\n')

    def header(self, name):
        for line in self.head[1:]:
            k, _, v = line.partition(':')
            if k.strip().lower() == name.lower():
                return v.strip()
        return None

    def send(self, text):
        payload = text.encode()
        mask = os.urandom(4)
        length = len(payload)
        head = bytes([0x81]) + (bytes([0x80 | length]) if length < 126 else
                                bytes([0x80 | 126]) + struct.pack('>H', length) if length < 65536 else
                                bytes([0x80 | 127]) + struct.pack('>Q', length))
        self.sock.sendall(head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))

    def _read(self, n):
        while len(self.buffer) < n:
            data = self.sock.recv(65536)
            if not data:
                raise EOFError('the server closed the socket')
            self.buffer += data
        out, self.buffer = self.buffer[:n], self.buffer[n:]
        return out

    def receive(self, timeout):
        """The next text message, or None if none comes within the timeout."""
        self.sock.settimeout(timeout)
        try:
            first = self._read(2)
        except socket.timeout:
            return None
        self.sock.settimeout(60)
        length = first[1] & 0x7f
        if length == 126:
            length = struct.unpack('>H', self._read(2))[0]
        elif length == 127:
            length = struct.unpack('>Q', self._read(8))[0]
        payload = self._read(length)
        if first[0] & 0x0f != 1:
            raise AssertionError(f'expected a text frame, got opcode {first[0] & 0x0f}')
        return payload.decode()


def http_status(port, context, headers):
    """The status line and body of a request the server must refuse."""
    with connect(port, context) as sock:
        lines = ['GET / HTTP/1.1', f'Host: 127.0.0.1:{port}', 'Upgrade: websocket', 'Connection: Upgrade',
                 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==', 'Sec-WebSocket-Version: 13', *headers]
        sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
        data = b''
        while True:
            chunk = sock.recv(4096)
            if not chunk:
                break
            data += chunk
            head, _, body = data.partition(b'\r\n\r\n')
            if head and b'Content-Length' in head:
                length = int(next(l.split(b':')[1] for l in head.split(b'\r\n')
                                  if l.lower().startswith(b'content-length')))
                if len(body) >= length:
                    break
    head, _, body = data.partition(b'\r\n\r\n')
    return head.split(b'\r\n')[0].split(b' ')[1].decode(), body.decode()


def normal(text):
    """An answer as compared: parsed, with a parse error's message left out."""
    if text is None:
        return None
    value = json.loads(text)
    if isinstance(value, dict) and value.get('error', {}).get('code') == -32700:
        value['error'].pop('data', None)
    return value


def session(port, context, log):
    """Everything one server answers."""
    seen = {}
    for label, headers in [('no secret', []), ('wrong secret', ['Authorization: Bearer nope']),
                           ('origin not allowed', [f'Sec-WebSocket-Protocol: minecraft-v1,{SECRET}',
                                                   'Origin: http://evil.example'])]:
        seen[label] = http_status(port, context, headers)
    browser = Socket(port, context, [f'Sec-WebSocket-Protocol: minecraft-v1, {SECRET}', f'Origin: {ORIGIN}'])
    seen['subprotocol'] = (browser.head[0].split(' ')[1], browser.header('Sec-WebSocket-Protocol'))
    browser.sock.close()
    ws = Socket(port, context, [f'Authorization: Bearer {SECRET}'])
    if ws.head[0].split(' ')[1] != '101':
        raise AssertionError(f'the handshake failed: {ws.head}')
    seen['tls'] = ws.sock.version()
    notifications = []

    def answer(message):
        """The answer to a message, the notifications that come first set aside."""
        ws.send(message)
        while True:
            text = ws.receive(timeout=3 if '"id"' not in message or '"result"' in message else 60)
            log.write(f'> {message}\n< {text}\n')
            value = normal(text)
            if isinstance(value, dict) and 'method' in value and 'id' not in value:
                notifications.append(value['method'])
                continue
            return value

    seen['answers'] = [answer(message) for message in MESSAGES]
    seen['stop'] = answer('{"jsonrpc":"2.0","id":99,"method":"minecraft:server/stop"}')
    seen['notifications'] = notifications
    return seen


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / NAME
    shutil.rmtree(build, ignore_errors=True)
    build.mkdir(parents=True)
    oh.run_logged(['cargo', 'build', '--profile', 'fast', '-p', 'horizon-client', '--bin', 'horizon-client',
                   '-p', 'horizon-server', '--bin', 'horizon-server'], build, 'compile-rust')
    target = lab.cargo_target() / 'fast'
    work = Path(tempfile.mkdtemp(prefix='hz-management-'))
    os.environ['HORIZON_API_DISCOVERY'] = NO_SERVICES
    try:
        keystore, context = make_keystore(work)
        sides = {}
        for side in ('vanilla', 'horizon'):
            folder = work / side
            folder.mkdir()
            write_lists(folder)
            port, management = lab.free_port(), lab.free_port()
            props = {'server-port': str(port), 'online-mode': 'false', 'level-type': 'minecraft:flat',
                     'generate-structures': 'false', 'view-distance': '3', 'simulation-distance': '3',
                     **management_properties(management, keystore)}
            if side == 'vanilla':
                (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
                command = ['java', '-Xmx2g', '-Duser.timezone=UTC', f'-Dminecraft.api.discovery.host={NO_SERVICES}',
                           '-cp',
                           os.pathsep.join(map(str, [jars['server'], *libraries])), 'net.minecraft.server.Main',
                           'nogui']
            else:
                typed = ('server-port', 'online-mode', 'view-distance', 'simulation-distance')
                toml = ''.join(f'"{k}" = {json.dumps(v)}\n' for k, v in props.items() if k not in typed)
                (folder / 'server.toml').write_text(
                    f'[server]\nport = {port}\nonline_mode = false\nview_distance = 3\nsimulation_distance = 3\n\n'
                    f'[properties]\n{toml}\n'
                    f'[engine]\npath = {json.dumps(str(target / f"horizon-client{EXE}"))}\n'
                    f'resources = {json.dumps(str(lab.RESOURCES.resolve()))}\nrestart_on_crash = false\n\n'
                    '[backups]\nevery = "0"\n\n[panel]\nbind = ""\n')
                command = [target / f'horizon-server{EXE}', '--dir', folder, 'run']
            server = Server(command, folder, build / f'{side}.log')
            try:
                server.wait_for(DONE, 600)
                with (build / f'{side}-session.log').open('w') as log:
                    sides[side] = session(management, context, log)
                server.process.wait(timeout=180)
            finally:
                server.stop()
        vanilla, horizon = sides['vanilla'], sides['horizon']
        failures = []
        for key in ('no secret', 'wrong secret', 'origin not allowed', 'subprotocol', 'tls', 'stop'):
            if vanilla[key] != horizon[key]:
                failures.append(f'{key}: vanilla {vanilla[key]!r}, Horizon {horizon[key]!r}')
        for message, v, h in zip(MESSAGES, vanilla['answers'], horizon['answers']):
            if v != h:
                failures.append(f'{message}\n  vanilla {json.dumps(v)[:400]}\n  Horizon {json.dumps(h)[:400]}')
        report = lab.LOCAL / 'datagen/out/reports/json-rpc-api-schema.json'
        discovered = horizon['answers'][MESSAGES.index('{"jsonrpc":"2.0","id":12,"method":"rpc.discover"}')]
        if json.loads(report.read_text()) != discovered.get('result'):
            failures.append(f'rpc.discover differs from {report}')
        (build / 'report.json').write_text(json.dumps(sides, indent=1))
        if failures:
            sys.exit(f'{NAME}: {len(failures)} differences (logs in {build}):\n' + '\n'.join(failures))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    print(f'{NAME}: {len(MESSAGES) + 6} exchanges over TLS answered as vanilla answers them; rpc.discover matches '
          'the report')


if __name__ == '__main__':
    main()
