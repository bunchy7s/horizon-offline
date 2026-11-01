#!/usr/bin/env python3
"""A spear's Lunge on the ground and in a jump, on a dedicated server, against vanilla's, seen by a vanilla client.

A vanilla dedicated server and Horizon's (`horizon-client --dedicated --nogui`) each serve a copy of one flat world
vanilla generated first, with Alice an operator in survival, holding a spear with Lunge III. LocatorBarClient (a
player made of vanilla's protocol code) jabs (ServerboundPlayerActionPacket STAB) standing, in the tick of a jump,
and a few ticks into one, sending one position a tick as LocalPlayer does. `ApplyEntityImpulse.apply` answers with a
ClientboundSetEntityMotionPacket, which replaces the client's velocity: the motions Alice is sent are compared. The
y keeps the jump because handlePlayerPositionChange makes the server's copy jump too (`jumpFromGround`).
"""
import sys

try:
    from . import test_locator_bar_parity as locator
except ImportError:
    import test_locator_bar_parity as locator

NAME = 'lunge-motion-parity'
# server_dir and generate_world write these server.properties.
locator.PROPERTIES = dict(locator.PROPERTIES, gamemode='survival')
SCRIPT = [
    # The jump ends on the ground: the join's position said otherwise, and a server's copy goes by what it is told.
    ('setup', ['cmd Alice give @s minecraft:netherite_spear[enchantments={lunge:3}]', 'arc Alice -1']),
    ('ground', ['stab Alice']),
    ('same-tick', ['arc Alice 0']),
    ('rising', ['arc Alice 3']),
    ('falling', ['arc Alice 8']),
    # jumpFromGround gives a sprinting server copy 0.2 forward too.
    ('sprinting', ['sprint Alice', 'arc Alice 3']),
]


def shown(lines):
    """The motions Alice is sent from the first jab on."""
    return [line for line in lines if len(parts := line.split(' ', 3)) >= 3 and parts[1] != 'step=setup'
            and parts[2] == 'motion']


def check(vanilla, vanilla_all):
    steps = {line.split()[1] for line in vanilla}
    missing = {f'step={step}' for step, _ in SCRIPT[1:]} - steps
    return f'vanilla sent no motion in {sorted(missing)}' if missing else None


def main():
    lines = locator.one_player(NAME, SCRIPT, shown, check, "A spear's Lunge standing and at three points of a jump "
                               'on a dedicated server: the motion the player is sent.', step_seconds=2.0)
    print(f'lunge motion: {len(lines)} lines identical')


if __name__ == '__main__':
    try:
        main()
    except AssertionError as error:
        sys.exit(f'{NAME}: {error}')
