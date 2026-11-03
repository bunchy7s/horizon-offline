#!/usr/bin/env python3
"""Compare selected-slot drops with Minecraft.handleKeybinds in all four game modes.

The oracle invokes the unmodified native key handler, consuming real drop-key
clicks, and MultiPlayerGameMode.dropItem's real prediction and swing dispatch.
Other keys are idle and execution stops at the following item-use boundary.
Checks Q/Ctrl+Q, slots 0/8, empty/single/partial/full stacks and repeated drops.
Checks swing attempts and drop packet counts, including packets without swings
for empty slots. Packet delivery, animation ticks, item entities and statistics
are outside scope.
"""
import os
import subprocess
try:
    from . import lab, test_rust_parity as base
except ImportError:
    import lab
    import test_rust_parity as base


def main():
    lock, jars, libraries = base.verify_artifacts()
    build = lab.LOCAL / 'held-drop-parity'
    build.mkdir(parents=True, exist_ok=True)
    classes = build / 'classes'
    classes.mkdir(exist_ok=True)
    # Minecraft's key handler also links client-only libraries.
    _, game_dir = lab.config()
    libraries += sorted((game_dir / 'libraries/client').rglob('*.jar'))
    classpath = os.pathsep.join(map(str, [jars['client'], *libraries]))
    oracle = lab.ROOT / 'mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeldDropOracle.java'
    subprocess.run(['javac', '--release', str(lock['minecraft']['java_major']), '-proc:none',
                    '-cp', classpath, '-d', str(classes), str(oracle)], check=True)
    subprocess.run(['java', '-Xmx512M', '-cp', str(classes) + os.pathsep + classpath,
                    'dev.horizonlab.parity.VanillaHeldDropOracle', str(build / 'vanilla.txt')], check=True)
    with (build / 'rust.txt').open('w') as out:
        subprocess.run(['cargo', 'run', '--quiet', '-p', 'horizon-client-lib', '--example',
                        'client_probe', '--', 'held_drop_fixture'], cwd=lab.ROOT, stdout=out, check=True)
    matched = base.compare(build / 'vanilla.txt', build / 'rust.txt')
    print(f'Held drop parity passed: {matched} inventory/drop/swing/packet snapshots match vanilla')


if __name__ == '__main__':
    main()
