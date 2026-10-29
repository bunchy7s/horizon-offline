#!/usr/bin/env python3
"""Compare firework rocket and firework star tooltips with the pinned vanilla client.

Random rockets (flight durations 0 to 3 and higher, up to six explosions, runs of equal ones) and stars, with dye and
custom colours, fade colours, trails and twinkles, some without the component (a rocket's default) and some whose
tooltip_display hides it. Vanilla prints what `Fireworks.addToTooltip` and `FireworkExplosion.addToTooltip` add
(VanillaFireworkTooltipOracle); Horizon what `item_tooltip::firework_lines` gives (examples/probe/firework_tooltip_fixture).
"""
import random
try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness

SHAPES = ['small_ball', 'large_ball', 'star', 'creeper', 'burst']
DYES = [0xF0F0F0, 0xEB8844, 0xC354CD, 0x6689D3, 0xDECF2A, 0x41CD34, 0xD88198, 0x434343, 0xABABAB, 0x287697,
        0x7B2FBE, 0x253192, 0x51301A, 0x3B511A, 0xB3312C, 0x1E1B1B]
COUNT = 3000


def explosion(rng):
    def colors():
        return [rng.choice(DYES) if rng.random() < 0.8 else rng.randrange(1 << 24) for _ in range(rng.choice([0, 1, 1, 2, 3]))]
    fields = [f'shape:"{rng.choice(SHAPES)}"']
    for key, values in (('colors', colors()), ('fade_colors', colors() if rng.random() < 0.4 else [])):
        if values:
            fields.append(f'{key}:[I;{",".join(map(str, values))}]')
    if rng.random() < 0.4:
        fields.append('has_trail:1b')
    if rng.random() < 0.4:
        fields.append('has_twinkle:1b')
    return '{' + ','.join(fields) + '}'


def stack(rng):
    components = []
    if rng.random() < 0.7:
        item = 'firework_rocket'
        if rng.random() < 0.9:
            explosions = []
            for _ in range(rng.choice([0, 0, 1, 1, 2, 3, 6])):
                # Runs of equal explosions are counted.
                explosions.append(explosions[-1] if explosions and rng.random() < 0.4 else explosion(rng))
            flight = rng.choice([0, 1, 1, 2, 3, 3, 7, 255])
            components.append(f'"minecraft:fireworks":{{flight_duration:{flight},explosions:[{",".join(explosions)}]}}')
    else:
        item = 'firework_star'
        if rng.random() < 0.9:
            components.append(f'"minecraft:firework_explosion":{explosion(rng)}')
    if rng.random() < 0.1:
        hidden = rng.choice(['minecraft:fireworks', 'minecraft:firework_explosion'])
        components.append(f'"minecraft:tooltip_display":{{hidden_components:["{hidden}"]}}')
    body = f',components:{{{",".join(components)}}}' if components else ''
    return f'{{id:"minecraft:{item}",count:1{body}}}'


def cases(build):
    rng = random.Random(0x46495245)
    path = build / 'cases.txt'
    resources = lab.LOCAL / 'horizon-client' / '26.3'
    path.write_text(f'#resources {resources}\n' + ''.join(stack(rng) + '\n' for _ in range(COUNT)))
    return path, COUNT


if __name__ == '__main__':
    harness.differential(
        'firework-tooltip-parity', 'VanillaFireworkTooltipOracle', 'firework_tooltip_fixture', cases,
        'Firework rocket and star tooltip lines: flight duration, explosions (counted runs), colours, fade, trail, '
        'twinkle, defaults and tooltip_display',
        sources=[lab.ROOT / 'crates/horizon-items/src/enchant.rs',
                 lab.ROOT / 'crates/horizon-client/src/item_tooltip.rs', lab.ROOT / 'crates/horizon-items/src/item_data.rs',
                 lab.ROOT / 'crates/horizon-client-render/src/particles.rs'],
        expected_lines=COUNT)
