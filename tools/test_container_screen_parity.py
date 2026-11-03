#!/usr/bin/env python3
"""Compare the survival and container screens' mouse handling with the pinned
client: the real AbstractContainerScreen.mouseClicked/mouseReleased (the slot
id a click targets, -999 outside the window, and the ContainerInput it sends:
PICKUP, QUICK_MOVE from the event's own Shift modifiers, THROW, the quick
craft drag a held stack starts, the drop its release outside makes, and the
CLONE a Creative player's pick-item button asks for) and hasClickedOutside,
with AbstractRecipeBookScreen's and RecipeBookComponent's rule for the recipe
book screens."""
import json
import random

try:
    from . import lab, oracle_harness as harness
except ImportError:
    import lab, oracle_harness as harness

# imageWidth, imageHeight; the recipe book screens only compare hasClickedOutside.
SCREENS = {'chest': None, 'hopper': (176, 133), 'shulker': (176, 167), 'beacon': (230, 219),
           'merchant': (276, 166), 'inventory': (176, 166), 'crafting': (176, 166), 'furnace': (176, 166)}
CLICKING = ('chest', 'hopper', 'shulker', 'beacon')
BOOKS = ('inventory', 'crafting', 'furnace')


def quarter(rng, lo, hi):
    return rng.randrange(int(lo * 4), int(hi * 4)) / 4


def point(rng, left, top, width, height):
    """Mostly near the window's edges and the recipe book's area."""
    pick = rng.random()
    if pick < 0.5:
        x = rng.choice([left, left + width, left - 147]) + quarter(rng, -3, 3)
        y = quarter(rng, top - 20, top + height + 20)
    elif pick < 0.8:
        x = quarter(rng, left - 30, left + width + 30)
        y = rng.choice([top, top + height]) + quarter(rng, -3, 3)
    else:
        x = quarter(rng, left - 200, left + width + 60)
        y = quarter(rng, top - 40, top + height + 40)
    return x, y


def pick_case(rng):
    """A container screen clicked with the pick-item button (the middle mouse).

    `mouseClicked` reads `keyPickItem.matchesMouse(event) && player.hasInfiniteMaterials()`
    before anything else: a player with infinite materials clones the hovered stack
    (`CLONE`, or with slot id -999 outside the window) instead of picking it up, and a
    held stack starts a quick craft drag of its own type instead of a plain one. The
    release then answers with `CLONE` as long as the carried stack is not empty and the
    event is the pick-item button, whether or not the press clicked anything, so a
    Survival player's middle release clones a held stack too (and the server refuses it).
    """
    kind = rng.choice(list(CLICKING))
    width, height = SCREENS[kind] or (176, 114 + 18 * rng.randint(1, 6))
    left, top = rng.randrange(0, 300), rng.randrange(0, 120)
    sx, sy = rng.randrange(0, width - 16), rng.randrange(0, height - 16)
    case = dict(kind=kind, image=[left, top, width, height], slot=[sx, sy],
                creative=rng.random() < 0.5, events=[])

    def over(on_slot):
        """A point over the menu's slot (7), or one it does not hover."""
        if on_slot:
            return quarter(rng, left + sx - 1, left + sx + 17), quarter(rng, top + sy - 1, top + sy + 17)
        while True:
            x, y = point(rng, left, top, width, height)
            if not (left + sx - 1 <= x < left + sx + 17 and top + sy - 1 <= y < top + sy + 17):
                return x, y

    for _ in range(rng.randrange(2, 6)):
        on_slot = rng.random() < 0.6
        carried = rng.random() < 0.5
        x, y = over(on_slot)
        case['events'].append(['press', x, y, 2, rng.choice([0, 0, 1, 2]), carried, on_slot])
        if rng.random() < 0.25:
            # Another button's click in between: it leaves the pick-item drag's button alone.
            bx, by = over(on_slot)
            case['events'].append([rng.choice(['press', 'release']), bx, by, rng.choice([1, 3]),
                                   rng.choice([0, 0, 1, 2]), carried, on_slot])
        if rng.random() < 0.5:
            x, y = over(on_slot)
        case['events'].append(['release', x, y, 2, rng.choice([0, 0, 1, 2]), carried, on_slot])
    return case


def cases(rng):
    for n in range(4000):
        if n % 4 == 3:
            yield pick_case(rng)
            continue
        kind = rng.choice(list(SCREENS))
        width, height = SCREENS[kind] or (176, 114 + 18 * rng.randint(1, 6))
        left, top = rng.randrange(0, 300), rng.randrange(0, 120)
        sx, sy = rng.randrange(0, width - 16), rng.randrange(0, height - 16)
        case = dict(kind=kind, image=[left, top, width, height], slot=[sx, sy], events=[])
        if kind in BOOKS:
            case['book'] = [rng.random() < 0.7, rng.random() < 0.2]
        for _ in range(rng.randrange(4, 24)):
            if kind not in CLICKING:
                case['events'].append(['point', *point(rng, left, top, width, height)])
                continue
            on_slot = rng.random() < 0.3
            if on_slot:
                x = quarter(rng, left + sx - 1, left + sx + 17)
                y = quarter(rng, top + sy - 1, top + sy + 17)
            else:
                while True:
                    x, y = point(rng, left, top, width, height)
                    if not (left + sx - 1 <= x < left + sx + 17 and top + sy - 1 <= y < top + sy + 17):
                        break
            case['events'].append([rng.choice(['press', 'release']), x, y, rng.choice([1, 1, 3]),
                                   rng.choice([0, 0, 1, 2, 3]), rng.random() < 0.5, on_slot])
        yield case


def make_cases(build):
    path = build / 'cases.jsonl'
    count = 0
    with path.open('w') as out:
        for case in cases(random.Random(0x434c4b53)):
            out.write(json.dumps(case, separators=(',', ':')) + '\n')
            count += 1
    return path, count


def main():
    scope = ('The real AbstractContainerScreen.mouseClicked and mouseReleased of ContainerScreen (one to six rows), '
             'HopperScreen, ShulkerBoxScreen and BeaconScreen with one menu slot, press and release of the left, right '
             'and pick-item (middle) buttons with the carried stack empty or not, the event Shift modifiers (left, '
             'right, both, none) and a player with and without infinite materials, '
             'recording every slotClicked (slot id, menu button, ContainerInput); and hasClickedOutside of those screens, '
             'MerchantScreen and the recipe book screens (InventoryScreen, CraftingScreen, FurnaceScreen) with the book '
             'visible or not and its selected tab hovered or not. Drags (mouseDragged, quickCraftToSlots) and double '
             'clicks are not exercised; the menu clicks themselves are menu_click\'s.')
    harness.differential(
        'rust-container-screen-parity', 'VanillaContainerScreenOracle', 'container_screen_probe', make_cases, scope,
        sources=[lab.ROOT / 'crates/horizon-client/src/container_screen.rs',
                 lab.ROOT / 'crates/horizon-client/src/menu.rs'])


if __name__ == '__main__':
    main()
