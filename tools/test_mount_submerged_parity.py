#!/usr/bin/env python3
"""A rider whose eyes are in water gets off a mount of the `dismounts_underwater` tag, on real vanilla and
Horizon servers.

LivingEntity.baseTick: a rider whose eyes are in water (not in a bubble column) stops riding a vehicle whose
type is in data/minecraft/tags/entity_type/dismounts_underwater.json (camel, chicken, donkey, horse, llama,
mule, pig, ravager, spider, strider, trader llama, zombie horse, happy ghast). A vanilla-codec client rides
each mount with `/ride`; water that stops below the rider's eyes keeps it on, deeper water throws it off, and
a mount outside the tag (a skeleton horse, a cow) keeps its rider underwater.
"""
try:
    from . import test_locator_bar_parity as net
except ImportError:
    import test_locator_bar_parity as net

SCRIPT = [('setup', ['cmd Alice gamerule spawn_mobs false'])]
EXPECTED = []
PROBES = []

# (entity type, the tag throws its rider off in deep water)
MOUNTS = [
    ('pig', True), ('horse', True), ('donkey', True), ('mule', True), ('llama', True),
    ('trader_llama', True), ('camel', True), ('chicken', True), ('spider', True),
    ('strider', True), ('ravager', True), ('zombie_horse', True),
    ('skeleton_horse', False), ('cow', False),
]


def probe(label, mounted, player='Alice'):
    PROBES.append((label, player))
    EXPECTED.append(f'{label} mounted={str(mounted).lower()}')
    SCRIPT.append((label, []))


for kind, ejects in MOUNTS:
    SCRIPT.append((kind + '-setup', [
        'cmd Alice kill @e[type=!player]',
        'cmd Alice fill -6 79 -6 6 95 6 minecraft:air',
        'cmd Alice fill -6 79 -6 6 95 6 minecraft:stone hollow',
        'cmd Alice fill -6 95 -6 6 95 6 minecraft:air',
        'cmd Alice tp @s 0.5 80 0.5',
        f'cmd Alice summon minecraft:{kind} 0.5 80 0.5 {{NoAI:1b,Silent:1b}}',
        f'cmd Alice ride @s mount @e[type=minecraft:{kind},limit=1]',
    ]))
    for n in range(2):
        SCRIPT.append((f'{kind}-dry-wait-{n}', []))
    probe(kind + '-dry', True)
    # Water to the mount's back: the rider's eyes are above it.
    SCRIPT.append((kind + '-shallow-fill', ['cmd Alice fill -5 80 -5 5 80 5 minecraft:water']))
    for n in range(2):
        SCRIPT.append((f'{kind}-shallow-wait-{n}', []))
    probe(kind + '-shallow', True)
    # Water over the rider's head.
    SCRIPT.append((kind + '-deep-fill', ['cmd Alice fill -5 80 -5 5 92 5 minecraft:water']))
    for n in range(3):
        SCRIPT.append((f'{kind}-deep-wait-{n}', []))
    probe(kind + '-deep', not ejects)


def shown(lines):
    # Each client's transcript is grouped under its name, so replay that
    # player's real passenger packets up to each observation step separately.
    order = {label: i for i, (label, _) in enumerate(SCRIPT)}
    observations = []
    for label, player in PROBES:
        mounted = None
        for line in lines:
            fields = line.split()
            if (len(fields) == 4 and fields[0] == player and fields[2] == 'passenger'
                    and order.get(fields[1].removeprefix('step='), -1) <= order[label]):
                mounted = fields[3] == 'true'
        value = str(mounted).lower() if mounted is not None else 'missing mount'
        observations.append(f'{label} mounted={value}')
    return observations


def check(native, all_lines):
    if native != EXPECTED:
        return f'vanilla did not exercise the expected immersion outcomes: {native!r}'
    return None


def main():
    lines = net.one_player('mount-submerged-parity', SCRIPT, shown, check, __doc__, step_seconds=1.0)
    print(f'mount submersion: {len(lines)} passenger observations match vanilla')


if __name__ == '__main__':
    main()
