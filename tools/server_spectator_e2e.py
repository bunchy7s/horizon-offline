#!/usr/bin/env python3
"""Spectators stay hidden from the other players on a dedicated server, end to end (#4204).

    python3 tools/server_spectator_e2e.py --bin-dir target/fast

Vanilla's `ServerPlayer.broadcastToPlayer` never sends a spectator to a player who isn't one, so no name tag of
a spectator ever shows to them (a client that draws invisible players' names or armour would show it, if it were
sent invisible); a spectator still sees everyone. Two real Horizon clients (horizon-ctl, offscreen)
join a dedicated server: Pat stays in survival, Ops goes in and out of spectator mode from the console. Pat's
client must know of Ops (`entities`) and show Ops's name tag (`status`'s name_tags) exactly while Ops isn't a
spectator: right after the switch,
after a teleport onto Pat, a death and respawn, a trip to the Nether and back, Pat joining again, and Ops joining
again as a spectator. Ops, spectating, sees Pat's.

The server and the games run in fresh folders with fresh HORIZON_HOME and XDG folders (lab.game_env), on a free
port. Build first:
    cargo build --profile fast -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl
"""
import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lab  # noqa: E402
from lan_e2e import Failed, Game  # noqa: E402
from server_bench import DONE, Server  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
PROPERTIES = {
    'online-mode': 'false',
    'white-list': 'false',
    'level-type': 'minecraft\\:flat',
    'generate-structures': 'false',
    'gamemode': 'survival',
    'view-distance': '4',
    'simulation-distance': '4',
    'spawn-protection': '0',
    'spawn-monsters': 'false',
}


class Run:
    def __init__(self, bins, folder, out):
        self.bins, self.out, self.games = bins, out, {}
        folder.mkdir(parents=True)
        self.port = lab.free_port()
        props = dict(PROPERTIES, **{'server-port': str(self.port)})
        (folder / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        (folder / 'eula.txt').write_text('eula=true\n')
        command = [bins / f'horizon-client{EXE}', '--resources', lab.RESOURCES, '--dedicated', '--nogui']
        self.server = Server(command, folder, out / 'server.log')
        self.server.wait_for(DONE, 300)

    def join(self, name):
        game = Game(name, self.bins / f'horizon-ctl{EXE}', lab.RESOURCES)
        self.games[name] = game
        game.pid = game.raw('launch', '--offscreen', '--no-pause')['pid']
        game.raw('connect', f'127.0.0.1:{self.port}', name)
        game.raw('wait-for', 'loaded', '--timeout', 300)
        return game

    def leave(self, name):
        game = self.games.pop(name)
        (self.out / f'{name}-{time.time():.0f}.log').write_text(game.close())
        shutil.rmtree(game.work, ignore_errors=True)
        time.sleep(2)

    def console(self, line, wait=1.0):
        self.server.console(line)
        time.sleep(wait)

    def tags(self, name):
        return self.games[name]('status')['name_tags'] or []

    def players(self, name):
        """How many players other than its own `name`'s client knows of (`entities`): a spectator it was sent,
        even invisible and without a name tag, counts."""
        return len(self.games[name].d('entities', radius=64, type='minecraft:player')['entities'])

    def shows(self, viewer, name, label):
        """`viewer`'s client shows `name`'s tag within 10 s."""
        deadline = time.time() + 10
        while name not in self.tags(viewer) or self.players(viewer) != 1:
            if time.time() > deadline:
                raise Failed(f'{label}: {viewer} never showed {name}\'s name tag')
            time.sleep(0.25)
        print(f'{label}: {viewer} shows {name}')

    def hidden(self, viewer, name, label):
        """`viewer`'s client shows no tag of `name` once a second has passed (the tracker's check of a player
        who moved), and for the next two."""
        time.sleep(1.5)
        for _ in range(8):
            if name in self.tags(viewer):
                raise Failed(f'{label}: {viewer} shows the spectator {name}\'s name tag')
            if self.players(viewer):
                raise Failed(f'{label}: {viewer}\'s client was sent the spectator {name}')
            time.sleep(0.25)
        print(f'{label}: {viewer} shows no tag of {name}')

    def close(self):
        for name in list(self.games):
            self.leave(name)
        self.server.stop()


def check(run):
    run.join('Pat')
    ops = run.join('Ops')
    for line in ('op Ops', 'time set day', 'gamerule spawn_mobs false', 'tp Ops Pat', 'tp Ops ~3 ~ ~'):
        run.console(line)
    run.shows('Pat', 'Ops', 'both in survival')
    run.console('gamemode spectator Ops')
    run.hidden('Pat', 'Ops', 'Ops turned spectator')
    run.shows('Ops', 'Pat', 'Ops spectating')
    run.console('tp Ops Pat')
    run.hidden('Pat', 'Ops', 'spectator teleported onto Pat')
    run.console('kill Ops', 2)
    ops.raw('respawn')
    run.console('tp Ops Pat')
    run.hidden('Pat', 'Ops', 'spectator respawned')
    run.console('execute in minecraft:the_nether run tp Ops 0 100 0', 4)
    run.console('execute in minecraft:overworld run tp Ops Pat', 4)
    run.hidden('Pat', 'Ops', 'spectator back from the Nether')
    run.leave('Pat')
    run.join('Pat')
    run.hidden('Pat', 'Ops', 'Pat joined again')
    run.leave('Ops')
    run.join('Ops')
    run.console('tp Ops Pat')
    run.hidden('Pat', 'Ops', 'Ops joined again as a spectator')
    run.console('gamemode survival Ops')
    run.shows('Pat', 'Ops', 'Ops back in survival')
    run.console('gamemode spectator Ops')
    run.hidden('Pat', 'Ops', 'Ops spectator again')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', default=str(lab.ROOT / 'target/fast'))
    parser.add_argument('--out', default=str(lab.LOCAL / 'server-spectator-e2e'))
    args = parser.parse_args()
    bins, out = Path(args.bin_dir).resolve(), Path(args.out)
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    os.environ.update(lab.game_env())
    with tempfile.TemporaryDirectory(prefix='server-spectator-e2e-') as scratch:
        run = Run(bins, Path(scratch) / 'server', out)
        try:
            check(run)
        except (Failed, RuntimeError, AssertionError, KeyError) as e:
            sys.exit(f'server_spectator_e2e failed (logs in {out}): {e}')
        finally:
            run.close()
    print('server_spectator_e2e: passed')


if __name__ == '__main__':
    main()
