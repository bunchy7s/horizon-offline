#!/usr/bin/env python3
"""A connected client advances ghast fireballs and thrown items between tracker packets.

AbstractHurtingProjectile.tick/applyInertia run on clients as well as servers, and
a thrown item ticks itself on the client too. Like lan_e2e's falling-entity check,
sample motion more often than the tracker interval; an interpolation-only client
stalls for most of that interval (#1762).
"""
import json
import math
import subprocess

import lab
import lan_e2e as lan


def check_motion(host, guest, out):
    x, y, z = (math.floor(v) for v in lan.pos_of(guest))
    for game in (host, guest):
        game('command', command='gamemode creative')
    tag = 'remote_fireball_parity'
    # A slow fireball (acceleration_power 0.02: under 0.4 blocks a tick, against ~2 by default) stays in the cleared
    # corridor and in the guest's tracking range for every sample, however many ticks a loaded host takes to answer.
    host('command', command=f'fill {x + 2} {y + 18} {z - 1} {x + 96} {y + 23} {z + 1} minecraft:air')
    host('command', command=f'summon minecraft:fireball {x + 3.5} {y + 20} {z + 0.5} '
                            f'{{Tags:["{tag}"],Motion:[0.1d,0.0d,0.0d],acceleration_power:0.02d}}')
    samples = []
    try:
        # One ctl process for all samples: a ctl process per sample took 10-16 ticks on a loaded runner, by
        # which time the fireball had left the cleared corridor and exploded.
        replies = guest.script(['status', 'wait 1'] * 16, every=True)
        for status in (reply for reply in replies if 'projectiles' in reply):
            balls = [p for p in status['projectiles'] if p['kind'] == 'LargeFireball']
            samples.append(balls[0]['position'][0] if balls else None)
    finally:
        host('command', command=f'kill @e[tag={tag}]')
    seen = [p for p in samples if p is not None]
    assert len(seen) >= 6, f'fireball visible in only {len(seen)} samples: {samples}'
    moves = sum(b != a for a, b in zip(seen, seen[1:]))
    assert moves >= len(seen) - 2, f'fireball stalled between tracker updates: {samples}'
    return {'positions': samples}


def check_thrown(host, guest, out):
    """A thrown egg, an ender pearl and a wind charge fly on between tracker packets.

    The server sends a thrown item's position every 10 ticks; a client that only
    eased towards it hung in the air for half a second (#1762). Vanilla's client
    ticks the projectile itself, as Entity's no-op interpolation handler does.
    """
    x, y, z = (math.floor(v) for v in lan.pos_of(guest))
    for game in (host, guest):
        game('command', command='gamemode survival')
    results = {}
    for item, kind in (('minecraft:egg', 'Egg'), ('minecraft:ender_pearl', 'EnderPearl'),
                        ('minecraft:wind_charge', 'WindCharge')):
        # A clear lane along +X at the guest's eye height, with room to fall.
        host('command', command=f'fill {x + 2} {y - 7} {z - 2} {x + 60} {y - 7} {z + 2} minecraft:stone')
        host('command', command=f'fill {x + 2} {y - 6} {z - 2} {x + 60} {y + 3} {z + 2} minecraft:air')
        guest('command', command=f'tp @s {x + 0.5:.2f} {y:.2f} {z + 0.5:.2f}')
        guest.wait(20)
        guest('command', command='clear @s')
        guest('command', command=f'item replace entity @s weapon.mainhand with {item}')
        guest.wait(10)
        # The server throws along the rotation it has: let it arrive first.
        guest.raw('look-at', f'{x + 40:.2f}', f'{y + 1.62:.2f}', f'{z + 0.5:.2f}')
        guest.wait(5)
        guest.raw('use-item')
        samples = []
        for _ in range(6):
            shots = [p for p in guest('status')['projectiles'] if p['kind'] == kind]
            samples.append(shots[0]['position'][0] if shots else None)
            guest.wait(2)
        seen = [p for p in samples if p is not None]
        moves = sum(b != a for a, b in zip(seen, seen[1:]))
        results[item] = {'positions': samples, 'moves': moves}
        assert len(seen) >= 4, f'{item} visible in only {len(seen)} samples: {samples}'
        assert moves >= len(seen) - 2, f'{item} stalled between tracker updates: {samples}'
    return results


def main():
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib',
                    '--bin', 'horizon-client', '--bin', 'horizon-ctl'], cwd=lab.ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=lab.ROOT, text=True))
    ctl = lab.ROOT / metadata['target_directory'] / 'fast' / 'horizon-ctl'
    lan.CHECK_FNS['remote-fireball'] = check_motion
    lan.CHECK_FNS['remote-thrown'] = check_thrown
    # Use the runner's current display (xvfb on Linux), with fresh isolated homes.
    lan.game_env = lambda _screen, home: lab.game_env(
        HORIZON_HOME=home, HORIZON_TELEMETRY_URL='off')
    out = lab.LOCAL / 'remote-fireball-parity'
    out.mkdir(parents=True, exist_ok=True)
    # lan.Game launches each client with lab.game_env and private scratch homes.
    report = lan.run(ctl, out, lab.RESOURCES.resolve(), ['remote-fireball', 'remote-thrown'])
    lab.write_json(out / 'report.json', report)
    assert all(r['pass'] for r in report.values()), report


if __name__ == '__main__':
    main()
