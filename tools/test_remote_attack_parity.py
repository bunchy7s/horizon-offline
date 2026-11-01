#!/usr/bin/env python3
"""Compare what a hit leaves on the attacking client with the pinned client.

MultiPlayerGameMode.attack runs Player.attack on the attacking client as well as
sending the packet. The oracle runs that real method on a LocalPlayer shell in a
ClientLevel shell, against a RemotePlayer and a zombie; the probe asks what
Game::remote_player_attack asks. Their velocity and sprint flag after the hit,
and the attack strength the hit used, must be identical.

A client's Player.attack is not a server's: held items and effects don't change
its attributes, enchantments don't count, and neither ATTACK_DAMAGE nor
ATTACK_KNOCKBACK is synced by a vanilla server. So the corpus crosses the weapons
and effects with the attribute values, to show which of them matter.
"""
import itertools
import json
import struct

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def bits(value):
    return f"{struct.unpack('>Q', struct.pack('>d', value))[0]:016x}"


def stack(item, enchantments=None):
    components = '' if enchantments is None else ',components:{"minecraft:enchantments":{%s}}' % ','.join(
        f'"minecraft:{name}":{level}' for name, level in enchantments.items())
    return '{id:"minecraft:%s",count:1%s}' % (item, components)


ITEMS = ['-', stack('wooden_sword'), stack('diamond_sword'), stack('wooden_sword', {'knockback': 2}),
         stack('diamond_sword', {'sharpness': 5}), stack('stick', {'knockback': 1})]
EFFECTS = [None, 'weakness', 'strength']
# Around the 0.9 of a knockback attack, with the delays of the speeds below.
TICKERS = [0, 1, 2, 3, 4, 5, 11, 12, 13, 100]
# Player's base, a sword's 1.6 as a server syncs it, and a 1.8-style server's.
SPEEDS = [4.0, 1.6, 1024.0]
# Player's base; the others only reach a client from a server that sends them.
DAMAGES = [1.0, 0.0, 7.0]
KNOCKBACKS = [0.0, 1.0]
VELOCITIES = [(0.25, -0.0784000015258789, -0.1), (-0.3, 0.42, 0.0)]


def cases():
    for target, item, effect, ticker, speed, damage, knockback, sprint, spin, velocity in itertools.product(
            ('player', 'zombie'), ITEMS, EFFECTS, TICKERS, SPEEDS, DAMAGES, KNOCKBACKS, (True, False), (None, 8.0),
            VELOCITIES):
        yield dict(target=target, item=item, effect=effect, ticker=ticker, speed=bits(speed), damage=bits(damage),
                   knockback=bits(knockback), sprint=sprint, spin=spin, vel=[bits(v) for v in velocity])


def make_input(build):
    path = build / 'inputs.jsonl'
    corpus = list(cases())
    path.write_text(''.join(json.dumps(c, separators=(',', ':')) + '\n' for c in corpus))
    return path, len(corpus)


def main():
    harness.differential(
        'rust-remote-attack', 'VanillaRemoteAttackOracle', 'remote_attack_probe', make_input,
        'The real Player.attack on a LocalPlayer shell in a ClientLevel, against a RemotePlayer and a zombie: the '
        'attack strength it used and the velocity and sprint flag it leaves, across targets, weapons (enchanted too), '
        'Weakness and Strength through forceAddEffect, attack strength tickers, ATTACK_SPEED, ATTACK_DAMAGE and '
        'ATTACK_KNOCKBACK values, sprinting and riptide spins. setSprinting is recorded rather than run. The packets '
        'and the server\'s view of the hit are server_knockback\'s; the path through Game to this decision is '
        'lan_e2e\'s.',
        sources=[lab.ROOT / 'crates/horizon-client/src/attack_strength.rs',
                 lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java'],
        expected_lines=sum(1 for _ in cases()))


if __name__ == '__main__':
    main()
