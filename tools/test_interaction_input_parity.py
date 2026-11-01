#!/usr/bin/env python3
"""Compare queued interaction ordering with the real Minecraft.handleKeybinds."""
import itertools
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness


def corpus():
    rng = random.Random(0x434C49434B)
    for using, destroying, attack_down, use_down, entity in itertools.product([False, True], repeat=5):
        for clicks in [[], [1], [0, 1], [1, 0], [2, 1, 0], [0, 0, 1, 1, 2]]:
            for delay in [0, 3]:
                yield dict(using=using, destroying=destroying, attack_down=attack_down,
                           use_down=use_down, entity=entity, clicks=clicks, delay=delay,
                           dig_on_attack=rng.choice([False, True]),
                           begin_using=rng.choice([False, True]))
    # A player interaction returning PASS must go directly to the held item.
    # The level still reports a solid block underneath the entity hit.
    for entity in [False, True]:
        for clicks in [[1], [1, 1], [0, 1], [1, 2]]:
            yield dict(using=False, destroying=False, attack_down=False, use_down=False,
                       entity=entity, clicks=clicks, delay=0, dig_on_attack=False,
                       begin_using=False, passing=True)
    # A keybind (the inventory or the chat) opened a screen while the player was
    # using an item: Gui.setScreen runs KeyMapping.releaseAll, so key.use reads
    # up and the same handleKeybinds call releases the use, even though the mouse
    # button is still held. The screen is open, so no click acts this tick.
    for using in [False, True]:
        for attack_down, use_down in [(True, True), (False, False)]:
            for delay in [0, 3]:
                yield dict(using=using, destroying=False, attack_down=attack_down,
                           use_down=use_down, entity=False, clicks=[], delay=delay,
                           dig_on_attack=False, begin_using=False, screen=True)


def main():
    def inputs(build):
        rows = list(corpus())
        path = build / "inputs.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
        return path, len(rows)

    harness.differential(
        "interaction-input-parity", "VanillaInteractionInputOracle", "interaction_input_fixture", inputs,
        "Invokes Minecraft.handleKeybinds itself with real KeyMapping click counters. "
        "Compares attack/use/pick ordering, release gating, drained clicks during use, "
        "held-button repetition and use cooldowns for block/entity targets, and the "
        "release a screen opened from a keybind (inventory, chat) causes in the same "
        "handleKeybinds call through Gui.setScreen's KeyMapping.releaseAll. "
        "Includes passing entity interactions over solid blocks: only the held "
        "item may be tried afterward. "
        "Game-mode side effects and world simulation are controlled fixtures.",
        sources=[lab.ROOT / "crates/horizon-client-input/src/interaction_input.rs",
                 lab.ROOT / "crates/horizon-client/src/game/remote.rs",
                 lab.ROOT / "crates/horizon-client/src/game/hand.rs",
                 lab.ROOT / "mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java"])


if __name__ == "__main__":
    main()
