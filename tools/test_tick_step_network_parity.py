#!/usr/bin/env python3
"""Compare /tick step broadcasts and late-join state on real 26.3 servers.

The native-codec client records the actual ClientboundTickingStepPacket counts.
One-tick steps are synchronized by server game time, so the network pass must
still send the requested count after the server has consumed it. A late join
receives the remaining count instead of replaying a completed step command.
"""
import platform
import time

try:
    from . import lab, test_rust_parity as base, test_locator_bar_parity as net
    from .test_dedicated_server_parity import Server, game_time
except ImportError:
    import lab
    import test_rust_parity as base
    import test_locator_bar_parity as net
    from test_dedicated_server_parity import Server, game_time


def run_side(label, command, client_command, build, world):
    directory = build / label
    port = net.server_dir(directory, world)
    out = build / f'{label}.txt'
    server = Server(command, directory, label)
    try:
        server.wait_for(r'Done \(')
        with (build / f'{label}-client.log').open('w') as log:
            client = net.Client(client_command, port, out, log)
            try:
                for line in ['ticksteps on', 'step joins', 'join Alice', 'fly Alice on']:
                    client.tell(line)
                server.wait_for('Alice joined the game', timeout=120)
                client.wait(server, ['Alice'], 5)
                server.console('tick freeze')
                game_time(server)
                client.sync(['Alice'])
                for name, ticks in [('single', 1), ('five', 5), ('single-again', 1)]:
                    client.tell(f'step {name}')
                    before = game_time(server)
                    server.console(f'tick step {ticks}')
                    deadline = time.monotonic() + 20
                    while game_time(server) < before + ticks:
                        if time.monotonic() > deadline:
                            raise TimeoutError(f'{label}: {name} did not finish')
                        time.sleep(0.02)
                    client.sync(['Alice'])
                client.tell('step long-step')
                server.console('tick step 1000')
                game_time(server)
                client.sync(['Alice'])
                client.tell('step stop')
                server.console('tick step stop')
                game_time(server)
                client.sync(['Alice'])
                client.tell('step late-join')
                client.tell('join Bob')
                server.wait_for('Bob joined the game', timeout=120)
                client.sync(['Alice', 'Bob'])
                # A rejected step while unfrozen sends no update.
                client.tell('step unfrozen')
                server.console('tick unfreeze')
                server.console('tick step 1')
                game_time(server)
                client.sync(['Alice', 'Bob'])
                # The step notification must precede the piston event it causes.
                # Both clients are nearby so the server sends them the event.
                for command in ['tp Alice 0.5 82 -4', 'tp Bob 2.5 82 -4', 'tick freeze',
                                'setblock 0 80 0 sticky_piston[facing=east]',
                                'setblock 1 80 0 stone', 'setblock -1 80 0 redstone_block']:
                    server.console(command)
                game_time(server)
                client.sync(['Alice', 'Bob'])
                client.tell('step piston')
                before = game_time(server)
                server.console('tick step 1')
                deadline = time.monotonic() + 20
                while game_time(server) == before:
                    if time.monotonic() > deadline:
                        raise TimeoutError(f'{label}: piston step did not run')
                    time.sleep(0.02)
                client.sync(['Alice', 'Bob'])
                client.end()
            finally:
                client.kill()
    finally:
        server.stop()
    observations = [line for line in out.read_text().splitlines()
                    if ' tick-step ' in line or ' piston-event ' in line]
    # Separate receiver threads may interleave, but preserve each one's wire order.
    return sorted(observations, key=lambda line: line.split(' ', 1)[0])


def main():
    lock, build, vanilla, horizon, client = net.sides('tick-step-network-parity')
    sources = [lab.ROOT / 'crates/horizon-client/src/clock.rs',
               lab.ROOT / 'crates/horizon-client/src/lan/hub.rs',
               lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/LocatorBarClient.java',
               lab.ROOT / 'tools/test_locator_bar_parity.py',
               lab.ROOT / 'tools/test_dedicated_server_parity.py', lab.ROOT / __file__]
    hashes = base.source_hashes(sources)
    world = net.generate_world(vanilla, build)
    expected = [
        'Alice step=joins tick-step 0',
        'Alice step=single tick-step 1',
        'Alice step=five tick-step 5',
        'Alice step=single-again tick-step 1',
        'Alice step=long-step tick-step 1000',
        'Alice step=stop tick-step 0',
        'Alice step=piston tick-step 1',
        'Alice step=piston piston-event 0 5',
        'Bob step=late-join tick-step 0',
        'Bob step=piston tick-step 1',
        'Bob step=piston piston-event 0 5',
    ]
    reference = run_side('vanilla', vanilla, client, build, world)
    if reference != expected:
        raise AssertionError(f'vanilla scenario did not establish the contract: {reference}')
    actual = run_side('horizon', horizon, client, build, world)
    if actual != reference:
        raise AssertionError(f'Horizon differs from vanilla:\n{actual}\nexpected:\n{reference}')
    base.ensure_sources_unchanged(sources, hashes)
    report = {'schema': 1, 'minecraft': lock['minecraft']['version'],
              'platform': platform.platform(), 'sources_sha256': hashes,
              'matched_output_lines': len(actual),
              'scope': 'Actual dedicated servers and native-codec client. Requested counts for '
                       'single/multiple/repeated steps, cancellation, rejected unfrozen steps, '
                       'remaining ticks on a late join, and step-before-piston-event ordering for '
                       'both clients. Does not render a graphical client.'}
    lab.write_json(build / 'report.json', report)
    print(f'Tick-step network parity passed: {len(actual)} native packet observations')


if __name__ == '__main__':
    main()
