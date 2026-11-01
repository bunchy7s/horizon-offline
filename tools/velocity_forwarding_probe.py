#!/usr/bin/env python3
"""Records what a Velocity proxy forwards, and joins through it (docs/velocity-forwarding.md).

Dev tool, standard library only. Two commands, run against a Velocity in offline mode with
`player-info-forwarding-mode = "modern"` and its only server pointed at --listen:

    velocity_forwarding_probe.py backend --listen 25642 --data 01 [--out capture.json]
        a stand-in backend: asks the proxy for forwarding with DATA (hex), records the answer, then kicks;
    velocity_forwarding_probe.py join --proxy 25641 [--name Tester] [--protocol 777]
        a minimal offline client that logs in through the proxy so that it connects to the backend. Pointed
        at a Horizon server directly it answers the forwarding request as a client does ("not understood"),
        or with --forge-secret FILE as a stand-in proxy that signs --forge-name/-uuid/-address.

Pass --secret FILE to `backend` to also check the signature (HMAC-SHA256 of the data) against Velocity's
`forwarding.secret`. The proxy throttles repeated logins from one address, so wait a few seconds between runs.
"""
import argparse
import hashlib
import hmac
import json
import socket
import struct
import uuid
import zlib


def varint(n):
    out = b""
    n &= 0xFFFFFFFF
    while True:
        b, n = n & 0x7F, n >> 7
        out += bytes([b | 0x80]) if n else bytes([b])
        if not n:
            return out


def parse_varint(buf, i=0):
    n = shift = 0
    while True:
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            return n, i
        shift += 7


def string(s):
    b = s.encode()
    return varint(len(b)) + b


def parse_string(buf, i):
    n, i = parse_varint(buf, i)
    return buf[i : i + n].decode(), i + n


def recv_exact(sock, n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def read_varint(sock):
    n = shift = 0
    while True:
        b = recv_exact(sock, 1)[0]
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            return n
        shift += 7


def send_packet(sock, pid, body=b"", threshold=None):
    data = varint(pid) + body
    if threshold is not None:
        data = varint(0) + data if len(data) < threshold else varint(len(data)) + zlib.compress(data)
    sock.sendall(varint(len(data)) + data)


def recv_packet(sock, threshold=None):
    data = recv_exact(sock, read_varint(sock))
    if threshold is not None:
        size, i = parse_varint(data)
        data = data[i:] if size == 0 else zlib.decompress(data[i:])
    pid, i = parse_varint(data)
    return pid, data[i:]


def backend(args):
    srv = socket.socket()
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", args.listen))
    srv.listen(1)
    c, _ = srv.accept()
    record = {"request_data_hex": args.data}
    pid, body = recv_packet(c)  # handshake
    protocol, i = parse_varint(body)
    host, i = parse_string(body, i)
    record["handshake"] = {"protocol": protocol, "host": host, "next_state": parse_varint(body, i + 2)[0]}
    pid, body = recv_packet(c)  # login start
    record["login_start_hex"] = body.hex()
    send_packet(c, 4, varint(1) + string("velocity:player_info") + bytes.fromhex(args.data))
    pid, body = recv_packet(c)  # login plugin response
    transaction, i = parse_varint(body)
    record["transaction"], record["success"] = transaction, body[i]
    payload = body[i + 1 :]
    record["payload_hex"] = payload.hex()
    if args.secret:
        secret = open(args.secret, "rb").read().strip()
        record["signature_ok"] = hmac.new(secret, payload[32:], hashlib.sha256).digest() == payload[:32]
    send_packet(c, 0, string('{"text":"recorded"}'))
    c.close()
    json.dump(record, open(args.out, "w"), indent=1)
    print(json.dumps(record, indent=1))


def forged(args):
    """A forwarded payload as a proxy signs it, for a stand-in proxy (join --forge-secret)."""
    data = varint(1) + string(args.forge_address) + uuid.UUID(args.forge_uuid).bytes + string(args.forge_name) + varint(0)
    secret = open(args.forge_secret, "rb").read().strip()
    return hmac.new(secret, data, hashlib.sha256).digest() + data


def join(args):
    s = socket.create_connection(("127.0.0.1", args.proxy), timeout=20)
    send_packet(s, 0, varint(args.protocol) + string("localhost") + struct.pack(">H", args.proxy) + varint(2))
    send_packet(s, 0, string(args.name) + bytes(16))
    threshold = None
    while True:
        try:
            pid, body = recv_packet(s, threshold)
        except (EOFError, ConnectionError, socket.timeout):
            print("closed")
            return
        if pid == 3:  # set compression
            threshold = parse_varint(body)[0]
        elif pid == 4:  # login plugin request: answer it as a client or a stand-in proxy does
            transaction, i = parse_varint(body)
            if args.forge_secret and parse_string(body, i)[0] == "velocity:player_info":
                send_packet(s, 2, varint(transaction) + b"\x01" + forged(args), threshold)
            else:
                send_packet(s, 2, varint(transaction) + b"\x00", threshold)  # "not understood"
        elif pid == 2:  # login finished: acknowledge, then wait in configuration while the proxy connects
            send_packet(s, 3, b"", threshold)
        print("packet", pid, len(body))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("backend")
    b.add_argument("--listen", type=int, default=25642)
    b.add_argument("--data", default="01", help="the request's data, in hex")
    b.add_argument("--secret", help="Velocity's forwarding.secret, to check the signature")
    b.add_argument("--out", default="capture.json")
    b.set_defaults(run=backend)
    j = sub.add_parser("join")
    j.add_argument("--proxy", type=int, default=25641)
    j.add_argument("--name", default="Tester")
    j.add_argument("--protocol", type=int, default=777)
    j.add_argument("--forge-secret", help="answer the forwarding request as a proxy with this secret file would")
    j.add_argument("--forge-name", default="Forged")
    j.add_argument("--forge-uuid", default="11111111-2222-3333-4444-555555555555")
    j.add_argument("--forge-address", default="203.0.113.7")
    j.set_defaults(run=join)
    args = p.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
