#!/usr/bin/env python3
"""Capture real carried grass/cross models from a vanilla 26.3 server.

Uses fresh scratch homes and a scratch flat world. The vanilla server supplies
carriedBlockState metadata (including properties); Horizon renders the frames.
Screenshots are evidence for visual review, not an automatic framebuffer diff.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import lab
from integration_env import game_environment
from test_dedicated_server_parity import Server
from test_rust_parity import verify_artifacts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, default=lab.ROOT / 'target/fast')
    parser.add_argument('--out', type=Path, default=lab.LOCAL / 'enderman-render-e2e')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    _, jars, libraries = verify_artifacts()
    with tempfile.TemporaryDirectory(prefix='hz-enderman-') as temporary:
        work = Path(temporary)
        directory = work / 'server'
        directory.mkdir()
        port = lab.free_port()
        props = {'server-port': str(port), 'server-ip': '127.0.0.1', 'online-mode': 'false', 'white-list': 'false',
                 'level-type': 'minecraft\\:flat', 'generate-structures': 'false', 'gamemode': 'creative',
                 'view-distance': '3', 'simulation-distance': '3', 'spawn-protection': '0',
                 'pause-when-empty-seconds': '-1'}
        (directory / 'server.properties').write_text(''.join(f'{k}={v}\n' for k, v in props.items()))
        (directory / 'eula.txt').write_text('eula=true\n')
        server = Server(['java', '-Xmx2g', '-cp', os.pathsep.join(map(str, [jars['server'], *libraries])),
                         'net.minecraft.server.Main', 'nogui'], directory, 'vanilla')
        socket = work / 'ctl.sock'
        env = game_environment(os.environ, work / 'client')
        game = None
        log = (out / 'client.log').open('w')

        def ctl(*words):
            reply = subprocess.run([str(args.bin_dir / 'horizon-ctl'), '--socket', str(socket),
                                    '--timeout', '30', *map(str, words)], env=env, cwd=work,
                                   check=True, capture_output=True, text=True, timeout=40)
            value = json.loads(reply.stdout)
            if 'error' in value:
                raise RuntimeError(value)
            return value

        try:
            server.wait_for(r'Done \(', timeout=60)
            game = subprocess.Popen([str(args.bin_dir / 'horizon-client'), '--resources', str(lab.RESOURCES),
                                     '--server', f'127.0.0.1:{port}', '--name', 'RenderTest', '--offscreen',
                                     '--width', '1280', '--height', '720', '--render-distance', '4',
                                     '--control-socket', str(socket)], cwd=work, env=env, stdout=log, stderr=log)
            for _ in range(150):
                if game.poll() is not None:
                    raise RuntimeError((out / 'client.log').read_text())
                if socket.exists():
                    break
                time.sleep(0.1)
            ctl('wait-for', 'loaded', '--timeout', '60')
            for command in ['gamerule spawn_mobs false', 'gamerule advance_time false', 'time set 6000',
                            'weather clear', 'fill -9 100 -9 9 100 9 smooth_quartz',
                            'fill -9 101 -9 9 108 9 air', 'tp RenderTest 0 101 -3 0 10']:
                server.console(command)
            ctl('wait', 20)
            ctl('options-set', 'chatVisibility', '2')
            ctl('key', 'f1', 'tap')
            ctl('options-set', 'maxFps', '60')
            for label, blocks in [('grass-poppy', ['grass_block', 'poppy']),
                                  ('leaves-pumpkin', ['spruce_leaves', 'carved_pumpkin'])]:
                server.console('kill @e[type=minecraft:enderman]')
                ctl('wait', 25)
                for x, block in zip([-1.2, 1.2], blocks):
                    props = ',properties:{facing:"east"}' if block == 'carved_pumpkin' else ''
                    server.console(f'summon enderman {x} 101 2 {{NoAI:1b,PersistenceRequired:1b,Rotation:[180f,0f],'
                                   f'carriedBlockState:{{id:"minecraft:{block}"{props}}}}}')
                ctl('wait', 20)
                entities = ctl('entities')
                assert len([e for e in entities['entities'] if e['type'] == 'minecraft:enderman']) == 2, entities
                assert not any('Failed to decode' in line for line in server.lines), '\n'.join(server.lines[-20:])
                (out / f'{label}.json').write_text(json.dumps(entities, indent=2))
                ctl('screenshot', out / f'{label}.png')
            print(f'Carried-block screenshots: {out}', flush=True)
        finally:
            if game is not None:
                try:
                    ctl('quit')
                    game.wait(timeout=15)
                except (subprocess.SubprocessError, RuntimeError):
                    game.terminate()
                    game.wait(timeout=10)
            log.close()
            server.stop()
            (out / 'server.log').write_text(server.log_path.read_text())


if __name__ == '__main__':
    main()
