#!/usr/bin/env python3
"""Both players' ordinary walking activates a sculk sensor on the server.

Exercise Entity.move movement emission through real client input, alongside
sculk's deterministic vanilla oracle. The host's world runs on its server
thread; the guest uses the same server-player path.
"""
import json
import math
import subprocess
import threading

import lab
import lan_e2e as lan


def until(game, observe, predicate, ticks=100):
    value = observe()
    for _ in range(ticks):
        if predicate(value):
            return value
        game.wait(1)
        value = observe()
    assert predicate(value), value
    return value


def check_walk(host, guest, out, role):
    player = host if role == 'host' else guest
    hx, y, hz = (math.floor(v) for v in lan.pos_of(host))
    x, z = hx + 6, hz + (-8 if role == 'host' else 8)
    sensor = (x + 3, y, z + 1)
    player('command', command='gamemode survival')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 5} {y - 1} {z + 5} minecraft:stone')
    host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')
    start = (x + 0.5, y, z + 0.5)
    player('command', command=f'tp @s {start[0]} {start[1]} {start[2]} 0 0')
    until(player, lambda: lan.pos_of(player), lambda p: math.dist(p, start) < 0.1)
    player.raw('look', 0, 0)
    player.wait(2)
    host('command', command=f'setblock {sensor[0]} {sensor[1]} {sensor[2]} minecraft:sculk_sensor')
    phase = lambda game: game.raw('getblock', *sensor)['block']
    try:
        until(host, lambda: phase(host), lambda s: 'sculk_sensor_phase=inactive' in s)
        player.wait(10)
        assert 'sculk_sensor_phase=inactive' in phase(host), 'setup activated the sensor'
        # The sensor stays active 30 ticks. Each client samples it every tick from inside one ctl script, so a
        # runner that answers control requests slowly (frames drawn on the CPU) can't miss that window between
        # separate requests; the other client samples alongside the walk.
        probe = f'getblock {sensor[0]} {sensor[1]} {sensor[2]}'
        other = guest if player is host else host
        seen = {}

        def watch(viewer, lines):
            seen[viewer.name] = [r['block'] for r in viewer.script(lines, every=True) if 'block' in r]
        watcher = threading.Thread(target=watch, args=(other, ['wait 1', probe] * 100))
        watcher.start()
        try:
            watch(player, ['key w down', *['wait 1', probe] * 15, 'key w up', *['wait 1', probe] * 85])
        finally:
            player.raw('key', 'w', 'up')
            watcher.join()
        moved = {'client': math.dist(lan.pos_of(player), start),
                 'server': math.dist(lan.server_player(host, player)['position'], start)}
        assert all(d > 1 for d in moved.values()), moved
        active = {name: next((i for i, s in enumerate(states) if 'sculk_sensor_phase=active' in s), None)
                  for name, states in seen.items()}
        assert len(active) == 2 and all(i is not None for i in active.values()), f'sensor never active: {seen}'
        return {'moved': moved, 'first_active_sample': active}
    finally:
        player.raw('key', 'w', 'up')
        host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')


def main():
    subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client', '-p', 'horizon-client-lib',
                    '--bin', 'horizon-client', '--bin', 'horizon-ctl'], cwd=lab.ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=lab.ROOT, text=True))
    ctl = lab.ROOT / metadata['target_directory'] / 'fast' / 'horizon-ctl'
    checks = ['sculk-walk-host', 'sculk-walk-guest']
    for role, name in zip(('host', 'guest'), checks):
        lan.CHECK_FNS[name] = lambda h, g, o, role=role: check_walk(h, g, o, role)
    # Use the runner's current display (xvfb on Linux), with fresh isolated homes.
    lan.game_env = lambda _screen, home: lab.game_env(
        HORIZON_HOME=home, HORIZON_TELEMETRY_URL='off')
    out = lab.LOCAL / 'server-player-sculk-parity'
    out.mkdir(parents=True, exist_ok=True)
    # lan.Game launches each client with lab.game_env and private scratch homes.
    report = lan.run(ctl, out, lab.RESOURCES.resolve(), checks)
    lab.write_json(out / 'report.json', report)
    assert all(r['pass'] for r in report.values()), report


if __name__ == '__main__':
    main()
