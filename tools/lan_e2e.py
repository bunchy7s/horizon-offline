#!/usr/bin/env python3
"""LAN multiplayer end to end: a host opens its world to LAN and a guest joins, both real games.

The host uses its integrated server thread by default; --in-frame compares the fallback, and
--server-thread remains accepted. Two Horizon clients run under horizon-ctl in scratch folders. The host
creates a survival world and opens it to LAN with commands allowed; the guest connects to it by
address. The script then checks, from both sides, what a guest's game used to get wrong (#366):

- tab:       the guest's player list holds the host and the guest, under their own names;
- self:      the guest's own model exists (third person, inventory preview);
- armour:    each side draws the other player in that player's own armour;
- nametags:  each side draws the other player's name above them;
- chest:     a chest the guest opens opens on the guest's screen, its lid opens on both sides and
             closes on both when the guest closes it; the host keeps playing with its mouse look (#687);
- crafting:  the guest crafts planks in the inventory's 2x2 grid and opens a crafting table, which
             leaves the host's screen and mouse look alone;
- spawned:   the host and a guest new to the world join on different spots (PlayerSpawnFinder), and the
             guest opens a crafting table, a chest, a furnace and a stonecutter where it joined;
- sign:      a sign the guest places doesn't open a sign editor on the host;
- damage:    a guest in survival takes fall damage;
- mobs:      with the host in creative, a zombie and a skeleton go for the guest and hurt them;
- hazards:   a guest in survival gets hungry sprint-jumping and is hurt by lava, a block at the eyes,
             drowning and the void (then respawns);
- pvp:       the guest and the host hit each other with a sword; with the pvp game rule off, or on one
             team without friendly fire, nothing;
- kills:     a pig the guest kills counts in the guest's killed statistic, not the host's (#484);
- leave:     a guest who leaves has leave_game in their saved statistics, and again after the next leave;

- arrows:    the guest and the host shoot each other with a bow; with the pvp game rule off, nothing (#484);
- looks:     mobs look on both sides as the server has them (their synched data): a calf is a
             baby, a red sheep is sheared, a size 4 slime, a tamed wolf sits with its blue collar, a
             cow on fire burns and goes out; then the wither's health drops when it is hurt, the
             sheep is dyed blue with its wool back and the calf grows up (#1367, #1368, #1370,
             #1371, #1385, #1390);
- leads:     the host and the guest each tie a cow to a lead and untie it: the rope shows on both
             sides, held by the player who tied it, and goes on both (#1360);
- respawn:   a player killed is drawn dead by the other side, and alive again once it respawns (the
             respawned player is a new entity to the others); the host dies, and on the server thread
             the guest too;
- hurt:      a pig the guest hits, and one the host hits, flashes red on the other side too (the
             server's damage event) (#848), and so does a hurt host or guest (#2827);
- containers: the guest's ender chest holds the guest's items, not the host's, and a double chest
             opens as 9x6 with the right half first;
- names:     a renamed shulker box, copper chest and chest the guest places open on the guest's screen
             with their custom names as the title (BaseContainerBlockEntity.getDisplayName, #3245);
- machines:  a hopper, a dispenser and a dropper open on the guest's screen with their 5 and 9
             slots, and keep what the guest shift-clicks into them (#848);
- brewing:   a brewing stand opens on the guest's screen; blaze powder, nether wart and a bottle the
             guest shift-clicks go to the fuel, ingredient and bottle slots, and the stand's fuel shows
             (#848);
- loom:      a loom opens on the guest's screen; the banner and dye the guest shift-clicks go to their
             slots, a pattern button the guest clicks makes the server's result, and the guest takes the
             patterned banner (#868);
- stonecutter: a stonecutter opens on the guest's screen with the recipes of the stone the guest puts
             in; the guest picks the slab and shift-clicks the result, and the server has cut every
             stone into slabs (#868);
- grindstone: a grindstone opens on the guest's screen; the enchanted sword the guest shift-clicks in
             shows the server's result, and the guest takes it with only its curse left and the
             experience orbs the server awards (#868);
- anvil:     each player repairs and renames a sword (ServerboundRenameItemPacket), takes it for
             the server's level cost, and the server has the repaired, renamed sword (#868);
             both players hear exactly one anvil sound for each result (#1907);
- enchanting: an enchanting table opens on the guest's screen with the server's offers for the
             sword the guest puts in; the guest clicks the first offer and the server enchants the
             sword for a level and a lapis, and counts it in the guest's enchant_item stat (#868);
- smithing:  a smithing table opens on the guest's screen; the guest upgrades a diamond sword to
             netherite and the server has it (#868);
- lectern:   a lectern's written book opens on the guest's screen; the guest turns its pages
             (ContainerButtonClick) and the server's lectern follows, then the guest takes the book
             and the server has it in the guest's inventory (#868);
- cartography: a cartography table opens on the guest's screen; the guest puts in a map the host
             made and paper, takes the zoomed-out map, and the server has a new map for it (#868);
- feedback:  the guest's command feedback names the guest and shows the numbers;
- chat:      a line the host sends reaches the guest's event log (`wait-for chat`), and on the
             server thread the host window's too;
- styles:    a styled tellraw reaches the host and the guest (each its own) with its colour, bold, click and hover
             events; each player's /give tells them of the Diamonds! advancement before the give's
             own message, and never echoes their command back to them as `[Name: ...]`;
- effects:   a beacon's power reaches every player in range, the host and the guest;
- distance:  (--server-thread) the server follows the host's render distance, past vanilla's 32 too
             (IntegratedServer.tickServer), and the guest's chunks follow the server's (#804)
- portals:   (--server-thread) the server holds a level per dimension: the guest goes to the Nether
             through a portal while the host stays, then the host goes to the End while the guest is in
             the Nether, and both come back.
- levels:    (--server-thread) the host and the guest each in a level of their own: `execute in`
             teleports into another level, and `execute in` and `at` change and test blocks there;
             mobs fall in two levels at once, with ids unique across them; the guest rejoins in the
             Nether, dies there and respawns in the Overworld, then in the Nether at a `spawnpoint`
             and at a respawn anchor set there; the host takes a Nether portal alone;
             the guest takes an End portal, an End gateway and the exit portal alone; and the host
             saves, quits and reopens with the guest in the Nether.
- commands:  the guest's chat completes commands and their arguments over the server's command
             tree (Tab), and runs what it completed;
- recipes:   the guest's recipe book lists the planks recipe the logs unlock, and clicking it
              places a log in the inventory's crafting grid (the server places it).
- recipe_removal: the host and guest each forget all and individual recipes while the book
              is open; it stays empty when reopened, and a later grant appears (#1495).
- recipe_targets: `/recipe give|take <other player>` changes that player's book, the host's
              for the guest and (server thread) the guest's for the host; their open book shows it.
- feedback:  the guest's command feedback names the guest and shows the numbers, and the host (an
             operator) is told of it as `[Guest: ...]`.
- clicks:    with real mouse presses (not click-slot, which skips the screen's press and drag
             handling), the guest puts a stack into a chest and a furnace, drags planks across
             its crafting grid and gathers a stack with a double click; the server has each.
- sleep:     at night the guest lies down in its own bed (not the host); alone it stays night, it
             gets up with Escape, and once the host sleeps too the night ends and both wake.
- bed-occupied: the bed the host sleeps in is occupied and the guest cannot lie down in it; it is
             free again after Escape, death, the bed broken and the sleeper leaving the game.
- sleep-reload: (--server-thread) a villager asleep in its bed is still asleep there, its bed
             occupied, after the host saves, quits and reopens; the guest's bed is saved free.
- predict:   with the host frozen, the oak door the guest opens opens on the guest's screen at once (its
             prediction) and creaks there at once, stays open once the server answers, and each side hears
             it once; an iron door stays shut (#869).
- push:      a cow standing in the guest pushes it just clear of the cow (the guest's own client
             pushes it, as in vanilla), not ever faster across the ground.
- eat:       holding right click with bread, the guest's own screen plays the eating (the item in use
             counts up) and the server eats one loaf and feeds the guest.
- consume-release: the host and the guest each cancel eating a golden apple and drinking milk from
              either hand: use ends on client and server, the item and effects stay, and holding
              use to completion still consumes the item (#1293).
- eating:    each side sees the other player eat: the item in use (LivingEntity's flags) on the
             player it draws, and no longer once the bread is eaten (#803);
- eat_full:  holding right click with melon slices until the food bar is full: the guest's own
             client eats each slice when the server says it is done (entity event 9,
             USE_ITEM_COMPLETE), so no other bite starts; held on chorus fruit, both fruits are
             eaten across its cooldown (#2471).
- xp:        experience orbs 4 blocks away fly to the guest, show on the guest's screen and give the
             guest their points (not the host).
- objects:   boats, minecarts, armor stands, item frames and paintings the host summons reach the
             guest as the host has them (kind, place, rotation, what they hold or wear, the variant).
- projectiles: primed TNT (one falling), an end crystal and an arrow that lands reach the guest where
              the host has them; a falling block lands on both sides.
- shulker_bullets: every shot a shulker fires has its own id and flies on both sides, following
              the same trajectory; the bullets disappear on both sides when they hit blocks (#1494).
- crystals:  the host and the guest punch placed End crystals in survival and creative; the blast
             kills the survival player, and attacks respect reach and blocks in front (#1532).
- thrown:    a splash potion and an eye of ender the host and the guest each throw fly on both
             sides (#1363, #1391);
- sparks:    level events' particles on both sides: the host's and the guest's bone meal on wheat
             (growth sparks), honeycomb on copper and an axe taking the wax off, and a spawner's
             burst when it spawns (#1395, #1401, #1402);
- fireball:  a fire charge a dispenser shoots onto stone lights a fire both sides see, and the fire
             goes out on its own within a minute (it ticks: FireBlock.tick on a floor that doesn't burn)


- blast:     primed TNT going off beside a guest in survival hurts the guest and pushes it away
             (ServerExplosion.hurtEntities takes every player); facing it behind a raised shield, the
             guest takes nothing.


- worn:      iron armour the guest puts on in its inventory protects it: a 10-damage blow takes 6
             (armour 15, LivingEntity.getDamageAfterArmorAbsorb), and the full 10 without it.
- plates:    a stone pressure plate the guest steps on is pressed on both sides, and comes up once
             the guest steps off (#848).
- swings:    each side sees the other's arm swing while it mines a block, and the guest sees its own
             swing when its hoe tills dirt (the server's use) (#848).
- cracks:    each side sees the other player's crack on the block they mine grow, and go once it breaks
             (#1096).
- time:      the guest's clock is the host's: it stops with `/time pause`, and catches up with the
             server's game time after the guest's game stalls for a few seconds (#848).
- growth:    far from the host, the chunks around the guest tick (#890): water placed there flows
             and wheat on farmland grows (random ticks), however the host runs its world.
- farmland:  the host and guest each trample planted farmland: the wheat breaks and drops its items,
             and both clients see dirt/air (#1492).
- ladder:    the guest climbs down ten blocks of ladder and lands unhurt: the server resets its fall
             distance on a ladder every tick, as LivingEntity.handleOnClimbable does (#1255).
- boats:     the host (single player, on its server thread or not) and the guest each right-click a
             boat on water and ride it, paddle it forward (the other side sees it move), sneak out,
             and break it with their fists (#1222).
- minecarts: the host and the guest each get into a minecart on rails, sneak out beside it, and break
             it (#1222).
- frames:    the host and the guest each put a diamond in an item frame and turn it, and put a
             helmet on an armor stand and take it back (#1222).
- hands:     the host and the guest each, at a cow that cares for none of it: draw a bow and shoot, and
             let go of a golden apple at once without eating it (the click goes on to the item);
             milk the cow with a bucket in the off hand and feed it wheat from the off hand; and
             punch the diamond out of an item frame, which then shows empty on both sides (#1350,
             #1355, #1362).
- mounts:    the host and the guest each open a tamed horse's, donkey's and llama's inventory
             (shift + right click, then the inventory key while riding), put on a saddle, armour
             or a carpet and fill the chest by shift-clicking.
- riding:    the host and the guest each saddle and ride a horse (walk, jump), a pig and a strider
             (with their sticks), a camel and a donkey, sit on a llama and fly a happy ghast, and
             get off by sneaking; the other side sees the animal move.
- portal_rider: (--server-thread) a survival guest rides a horse into a Nether portal: the horse goes
             through at once and carries it, so it arrives in the Nether still riding the same horse.
- riding_teleport: the host and the guest each /tp through chat while riding a horse; after 60
             ticks each stays dismounted at the destination and both sides see the horse left
             behind. The host also teleports a mounted guest by name.
- lightning: a bolt summoned beside the host, then the guest: both sides get the bolt and flash the
             sky (on its server thread the host's window is a client of it too).
- drops:     an item lies where the host, then the guest, places a block: it comes out of the block
             on the server, and both sides see it outside (#1252).
- underwater: the host and the guest each dig dirt by hand standing under water: five times slower
             than in the air on their own client as on the server, and it breaks (#1254).
- worldspawn: the host moves the world spawn (`setworldspawn`): the host and the guest, with no bed,
             respawn there (#1310).
- shield:    a vindicator's axe disables the host's, then the guest's, raised shield: the server
             counts its 5 s cooldown down once a tick (#1316).
- beds:      the host and the guest each set their respawn point at a bed and die elsewhere: they
             stand up beside the bed, or, when the bed is gone or boxed in by stone, come back at
             the world spawn (#1308).
- deathmining: the host and the guest each die while breaking obsidian and let go on the death
             screen: once respawned, the obsidian stays (#1315).
- contacts:  the host and the guest each stand on magma (and sneak on it), stand on a lit campfire,
             walk against a cactus and through grown sweet berry bushes: the server hurts them, but
             not the sneaking player or young bushes (#1305, #1311, #1312, #1313).
- absorption: the host and the guest each take Absorption II and a hit of 3: their own HUD has the
             server's golden hearts, 8 then 5 (#1306).
- breath:    the host and the guest each stay underwater for 15 s in a Respiration III helmet: their
             own bubbles follow the server's air, which the enchantment makes last (#1304).
- lava:      (--server-thread) the host and the guest each stand in a Nether lava current with Fire
             Resistance, which pushes them at the Nether's strength (#1309).
- flight:    a player in creative double-taps jump and flies, stays up, and stops flying in survival:
             the guest, and on the server thread the host's window too, take what may fly from the
             server's abilities (ClientPacketListener.handlePlayerAbilities).
- light:     the guest's light matches the host's: around a torch placed near it, under a roof and
             once a block of the roof is removed under the sky, and in the last chunk it tracks next to
             glowstone in a chunk it doesn't (only the server's light updates bring that one).
- totem:     the host and the guest each take a lethal hit holding a totem of undying in the off
             hand, then the main hand: the server uses it up and keeps them alive at 1 health with its
             effects, and each side hears the totem once (entity event 35) (#1292).
- keepinv:   the host and the guest each die holding diamonds and wearing a helmet: with keep_inventory
             off both drop where they died and the player respawns without them; with it on nothing
             drops and the player keeps them (#1284).
- signs:     the host and the guest each place an oak sign: the editor opens on their own screen
             (ClientboundOpenSignEditorPacket), the two lines they write reach the server
             (ServerboundSignUpdatePacket) and both sides show them (#1380).
- books:     the host and the guest each write in a book and quill on their own screen, sign it
             (ServerboundEditBookPacket), read the written book the server gave them
             (ClientboundOpenBookPacket), then drop it: the other side sees its title, author and
             page (#1366).
- elytra:    the host and the guest, high up with an Elytra on, jump in the air and glide
             (ServerboundPlayerCommandPacket START_FALL_FLYING): both their client and the server
             have them gliding, they go forward far more than down, and the glide ends on both once
             they land (#1288).
- helduse:   the host and the guest raise a shield, charge and fire a crossbow, throw a trident
             and scope with a spyglass: each use starts on their own client and on the server, and
             letting go ends it on both (RELEASE_USE_ITEM), so the shield lowers, the trident flies
             and the loaded crossbow fires on the next use (#1300, #1281, #1282, #1301).
- wind:      the host and the guest each throw a wind charge at their feet: the server's burst
             launches them about six blocks up (the push in ClientboundExplodePacket) and they land
             unhurt (#1386).
- fishing:   the host and the guest each cast a fishing rod: the hook stays out on the server and
             both sides draw it and it reels back in from the ground, and in open water a fish
             bites and reeling in catches it (#1348).
- maps:      the host and the guest each make a map and hold it: their client draws the terrain
             around them and their marker, from the server's map data (#1382).
- body:      the host and the guest each walk while drawing a bow (slowed), stand on powder snow
             with leather boots (held up) and without (sinking), and sprint-swim looking down
             (diving): their own client decides it (#1278, #1296, #1303).
- strikes:   a bolt strikes the host and the guest standing in water and hurts them, and a cave
             spider's bite poisons them (#1480, #1484).
- cursor:    the host and the guest each close their inventory with a stack on the cursor: it
             goes back into their inventory, none is thrown and the cursor is empty (#1283).
- knockback: the host hits the guest three times, then the guest the host, two seconds apart: each
             hit slides the victim as far as vanilla's does, 1.989 blocks (a guest's server-side
             velocity settles between hits; it used to add up, so each push went further).
- villagers: the guest sees each villager's and zombie villager's type, profession and level as the
             host has them, and an unemployed villager that takes a composter becomes a farmer on
             the guest too (the guest used to draw every villager as an unemployed plains one).
- stopsound: the host's and the guest's own /stopsound, by source and by event, reach their own
             client's sounds, and the host's /stopsound of the guest only the guest's (StopSoundCommand
             sends each target its packet).
- hero-trading: the host and the guest each trade 20 wheat for an emerald, paying 14 with Hero I
              and 12 with Hero III; clearing the effect restores the ordinary price (#1365).
- cure-discount: the host and the guest each cure a zombie villager with a golden apple and pay 14
              wheat instead of 20 for its emerald (the ZOMBIE_VILLAGER_CURED gossip, #3306).
- falling:   an item, a falling block and primed TNT the host summons high up fall smoothly on the
             guest, a little further every tick between the server's updates (every 20 ticks, 10 for
             TNT), as their own client tick moves them (they used to hang, then jump down once a
             second, #1568), and the TNT's fuse burns down on the guest too.
- instant_digging: an instant survival break leaves no repeated stone hit sounds or stale server dig (#1908).
- replant:   flowers and crops replanted after a short harvest click stay planted for both players;
             the server forgets the completed instant break (#1880).
- dig_stop:  a block the client breaks at once (a creative break, a dead bush) leaves the
             server no dig to keep: the block put back stays and no mining sound plays on (#1340, #1908).

Each check is reported on its own (a failing one doesn't stop the others) and the screenshots of
both sides go to --out (host-*.png, guest-*.png).

    python3 tools/lan_e2e.py                          # build with the fast profile, run every check
    python3 tools/lan_e2e.py --bin-dir target/fast    # prebuilt binaries
    python3 tools/lan_e2e.py --check chest --check tab
    python3 tools/lan_e2e.py --server-thread --check portals   # the host's world on its server thread

Needs the prepared resources in .local/horizon-client. On Linux the games run on vscreen (the
desktop), or headless (offscreen, no pause) where there is none; on macOS both games render offscreen. Games started here have no audio (HORIZON_NO_AUDIO=1).
"""
import argparse
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from agent_control_e2e import Cli, Failed, game_env  # noqa: E402
import lab  # noqa: E402
from integration_env import game_environment, require_server_reports  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
GUEST_NAME = 'Guest'
# Match normal single player; --in-frame is the comparison path.
SERVER_THREAD = True
CHECKS = ['tab', 'self', 'armour', 'nametags', 'walls', 'chest', 'crafting', 'spawned', 'sign', 'damage', 'mobs', 'hazards', 'pvp',
          'flow-push', 'bubble-column', 'arrows', 'kills', 'leave', 'hurt', 'looks', 'leads', 'respawn', 'containers', 'names', 'feedback', 'titles', 'chat', 'styles',
          'commands', 'recipes', 'machines', 'brewing', 'loom', 'stonecutter', 'grindstone', 'anvil', 'smithing',
          'enchanting', 'beacon', 'crafter', 'lectern', 'cartography', 'trials', 'clicks', 'sleep', 'sleep-villager', 'bed-occupied', 'sleep-reload', 'predict', 'push',
          'piston-push', 'piston-slime', 'eat', 'eating', 'xp', 'objects', 'projectiles', 'thrown', 'sparks', 'hud', 'sidebar', 'border', 'particles',
          'worn', 'blast', 'barter', 'deflect', 'fireball', 'light', 'time', 'swings', 'cracks', 'plates', 'effects',
          'portals', 'distance', 'growth', 'doors', 'flight', 'ladder', 'boats', 'minecarts', 'frames', 'hands',
          'lightning', 'riding', 'occupied_horse', 'mounts', 'worldspawn', 'beds', 'shield', 'deathmining', 'contacts', 'absorption',
          'breath', 'lava', 'levels', 'drops', 'underwater', 'fireworks']


class Game:
    """One client: its scratch folder, its socket and a CLI driver."""

    def __init__(self, name, ctl, resources):
        self.name = name
        self.work = Path(tempfile.mkdtemp(prefix=f'hzlan-{name}-'))
        (self.work / '.local/horizon-client').mkdir(parents=True)
        os.symlink(resources, self.work / '.local/horizon-client' / lab.VERSION)
        self.socket = self.work / 'c.sock'
        env = game_env(os.environ.get('VSCREEN', 'lan-e2e'), home=self.work / 'home')
        env = game_environment(env, self.work)
        self.d = Cli(ctl, self.socket, self.work, env)
        self.d.name = name

    def __call__(self, tool, **args):
        return self.d(tool, **args)

    def raw(self, *words):
        """A game command horizon-ctl has no table entry for (lan, connect): the words as typed."""
        result = subprocess.run(self.d.base + [str(w) for w in words], cwd=self.work, env=self.d.env,
                                capture_output=True, text=True, timeout=600)
        lines = result.stdout.strip().splitlines()
        reply = json.loads(lines[-1]) if lines else {'error': result.stderr.strip()}
        print(f'[{self.name}] {list(words)} -> {json.dumps(reply)[:400]}')
        if result.returncode or 'error' in reply:
            raise Failed(f'{" ".join(map(str, words))}: {reply}')
        return reply

    def shot(self, out, label):
        path = out / f'{self.name}-{label}.png'
        self.d('screenshot', path=path)
        return path

    def wait(self, ticks):
        self.d('wait', ticks=ticks)

    def script(self, lines, every=False):
        """Keep tick-sensitive input in one ctl process, through the normal control commands. Returns the last
        reply, or with every=True all of them in order."""
        result = subprocess.run(self.d.base + ['script'], input='\n'.join(lines) + '\n',
                                cwd=self.work, env=self.d.env, capture_output=True, text=True,
                                timeout=600)
        replies = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        ticks = [r['ticks'] for r in replies if 'ticks' in r]
        print(f'[{self.name}] script {lines} -> {replies[-1] if replies else result.stderr[:400]}; ticks={ticks}')
        assert result.returncode == 0 and replies and not any('error' in r for r in replies), result.stdout
        return replies if every else replies[-1]

    def until(self, observe, predicate, ticks=100):
        """Wait for an observable answer, within the existing tick budget."""
        last = observe()
        for _ in range(ticks):
            if predicate(last):
                return last
            self.wait(1)
            last = observe()
        assert predicate(last), f'{self.name}: state did not arrive within {ticks} ticks: {last}'
        return last

    def close(self):
        try:
            self.d('quit')
        except Exception:
            pass
        time.sleep(2)
        log = self.socket.with_suffix('.log')
        return log.read_text(errors='replace') if log.exists() else ''


def remote(guest):
    """The guest's view of the server (`status`'s server part)."""
    return guest('status')['server']


def dead(guest):
    """Whether the guest is dead: its death screen (the last health it was sent can stay above 0)."""
    return guest('state').get('screen') == 'dead' or remote(guest)['health'] <= 0


def back_to_game(game):
    """Close whatever screen a failed check left open, without opening the pause menu."""
    try:
        if game('state').get('screen') not in (None, 'playing'):
            game.raw('key', 'esc', 'tap')
            game.wait(2)
    except Failed:
        pass


def pos_of(game):
    return game('state')['player']['position']


def face_each_other(host, guest):
    """Put the guest 3 blocks east of the host, both looking at the other's head."""
    hx, hy, hz = pos_of(host)
    guest('command', command=f'tp @s {hx + 3:.2f} {hy:.2f} {hz:.2f}')
    guest.wait(30)
    host.raw('look-at', f'{hx + 3:.2f}', f'{hy + 1.6:.2f}', f'{hz:.2f}')
    guest.raw('look-at', f'{hx:.2f}', f'{hy + 1.6:.2f}', f'{hz:.2f}')
    host.wait(10)


def check_tab(host, guest, out):
    players = remote(guest)['players']
    names = sorted(p['name'] if isinstance(p, dict) else str(p) for p in players)
    assert names == sorted([host.player, GUEST_NAME]), f'guest tab list: {players}, host {host.player}'
    guest.raw('key', 'tab', 'down')
    guest.wait(5)
    guest.shot(out, 'tab')
    guest.raw('key', 'tab', 'up')
    return names


def check_self(host, guest, out):
    guest.raw('key', 'f5', 'tap')
    guest.wait(10)
    guest.shot(out, 'self-third-person')
    status = guest('state')
    guest.raw('key', 'f5', 'tap')
    guest.raw('key', 'f5', 'tap')
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    guest.shot(out, 'self-inventory')
    model = remote(guest).get('player_model')
    guest.raw('key', 'esc', 'tap')
    assert model, f'the guest has no player model: {status.get("camera")}'
    return 'model drawn'


def check_armour(host, guest, out):
    host('command', command='item replace entity @s armor.head with minecraft:golden_helmet')
    host('command', command='item replace entity @s armor.chest with minecraft:golden_chestplate')
    guest('command', command='item replace entity @s armor.chest with minecraft:diamond_chestplate')
    guest('command', command='item replace entity @s armor.legs with minecraft:diamond_leggings')
    face_each_other(host, guest)
    host.shot(out, 'armour')
    guest.shot(out, 'armour')
    if SERVER_THREAD:
        # The host's window is a client of its server thread: what it draws is what it was sent.
        seen = [e for e in remote(host).get('player_equipment', [])]
        assert all(any(item in json.dumps(e) for e in seen) for item in
                   ('diamond_chestplate', 'diamond_leggings')), f'the host draws the guest in {seen}'
    else:
        seen = lan_guest(host).get('model_armor')
        assert seen == [None, 'minecraft:diamond_chestplate', 'minecraft:diamond_leggings', None], \
            f'the host draws the guest in {seen}'
    host_gear = [e for e in remote(guest).get('player_equipment', [])]
    assert any('golden_chestplate' in json.dumps(e) for e in host_gear), f'the guest sees the host in {host_gear}'
    return 'each in their own'


def check_nametags(host, guest, out):
    face_each_other(host, guest)
    h = host('status').get('name_tags') or []
    g = guest('status').get('name_tags') or []
    host.shot(out, 'nametags')
    guest.shot(out, 'nametags')
    assert GUEST_NAME in h, f'host draws name tags {h}'
    assert host.player in g, f'guest draws name tags {g}'
    return {'host': h, 'guest': g}


def grab(game):
    """Mouse look on, as a player's grabbed pointer: an agent's mouse-move stands in for it."""
    game.raw('mouse-move', 1, 0)
    game.raw('mouse-move', -1, 0)


def host_unaffected(host, what):
    """What a guest opens is the guest's: the host keeps its world on screen and its mouse look (#687)."""
    state = host('state')
    assert state.get('screen') == 'playing', f'{what} opened the {state.get("screen")} screen on the host'
    assert state.get('pointer_grabbed'), f"{what} released the host's pointer"


def lid(game, pos):
    return game.raw('block', *pos).get('lid', 0.0)


def check_chest(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest = (x, y, z + 2)
    guest('command', command=f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest')
    guest.wait(10)
    grab(host)
    guest.raw('interact', *chest)
    screen = guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(20)
    host_unaffected(host, "the guest's chest")
    opened = {'guest': lid(guest, chest), 'host': lid(host, chest)}
    host.raw('look-at', chest[0] + 0.5, chest[1] + 0.5, chest[2] + 0.5)
    host.wait(2)
    host.shot(out, 'chest-open')
    guest.shot(out, 'chest-open')
    guest.raw('key', 'esc', 'tap')
    guest.wait(20)
    closed = {'guest': lid(guest, chest), 'host': lid(host, chest)}
    host.shot(out, 'chest-closed')
    assert screen, 'no container screen on the guest'
    assert opened['guest'] > 0.9 and opened['host'] > 0.9, f'lid while open: {opened}'
    assert closed['guest'] < 0.1 and closed['host'] < 0.1, f'lid after closing: {closed}'
    return {'open': opened, 'closed': closed}


def check_crafting(host, guest, out):
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_log 1')
    guest.wait(10)
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    inv = guest('inventory')
    slot = next(s['slot'] for s in inv['hotbar'] if s and s.get('item') == 'minecraft:oak_log')
    # Horizon's slot ids (click-slot): the hotbar is 0-8, the grid 100-108, the result 200.
    guest.raw('click-slot', slot)
    guest.raw('click-slot', 100)
    guest.wait(5)
    result = [s for s in guest('screen').get('slots', []) if s.get('slot') == 200]
    guest.shot(out, 'crafting-grid')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(5)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    planks = guest('inventory')['counts'].get('minecraft:oak_planks', 0)
    assert planks == 4, f'crafted {planks} planks (result slot: {result})'
    x, y, z = (int(v // 1) for v in pos_of(guest))
    table = (x, y, z - 2)
    guest('command', command=f'setblock {table[0]} {table[1]} {table[2]} minecraft:crafting_table')
    guest.wait(10)
    grab(host)
    guest.raw('interact', *table)
    guest.raw('wait-for', 'screen', 'inventory', '--timeout', 10)
    host_unaffected(host, "the guest's crafting table")
    size = guest('screen').get('craft_size')
    assert size == 3, f'the crafting table opened a {size}x{size} grid'
    # Two planks one above the other in the 3x3 grid: four sticks.
    slot = next(s['slot'] for s in guest('inventory')['hotbar'] if s and s.get('item') == 'minecraft:oak_planks')
    guest.raw('click-slot', slot)
    guest.raw('click-slot', 100, 'right')
    guest.raw('click-slot', 103, 'right')
    guest.raw('click-slot', slot)
    guest.wait(5)
    guest.shot(out, 'crafting-table')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(5)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    counts = guest('inventory')['counts']
    assert counts.get('minecraft:stick') == 4 and counts.get('minecraft:oak_planks') == 2, f'after the table: {counts}'
    return {'planks': planks, 'table': counts}


def check_spawned(host, guest, out):
    """Players new to a world don't share one spot: ServerPlayer.adjustSpawnLocation spreads them over the
    respawn radius (PlayerSpawnFinder). Two players standing in each other target each other, so a guest
    left in the host could open nothing it clicked. At its own spot, the guest opens a crafting table, a
    chest, a furnace and a stonecutter."""
    joined = {'host': [round(v, 2) for v in host.joined_at], 'guest': [round(v, 2) for v in guest.joined_at]}
    apart = abs(joined['host'][0] - joined['guest'][0]) + abs(joined['host'][2] - joined['guest'][2]) >= 0.6
    guest('command', command='gamemode survival')
    for game in (host, guest):
        x, y, z = game.joined_at
        game('command', command=f'tp @s {x} {y} {z}')
    guest.wait(20)
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    grab(host)
    opened = {}
    for block in ('crafting_table', 'chest', 'furnace', 'stonecutter'):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
        guest.wait(10)
        guest.raw('interact', *at)
        guest.wait(10)
        opened[block] = guest('state').get('screen')
        guest.shot(out, f'spawned-{block}')
        back_to_game(guest)
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    host_unaffected(host, "the guest's stations")
    face_each_other(host, guest)
    assert apart, f'the host and the guest joined on one spot: {joined}'
    want = {'crafting_table': 'inventory', 'chest': 'container', 'furnace': 'container', 'stonecutter': 'station'}
    assert opened == want, f'where it joined, the guest opened {opened}'
    return {'joined': joined, 'opened': opened}


def check_sign(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(guest))
    spot = (x + 2, y, z)
    guest('command', command=f'setblock {spot[0]} {spot[1] - 1} {spot[2]} minecraft:stone')
    guest('command', command=f'setblock {spot[0]} {spot[1]} {spot[2]} minecraft:air')
    guest('command', command='give @s minecraft:oak_sign 1')
    guest.wait(10)
    grab(host)
    guest.raw('place', *spot, '--item', 'minecraft:oak_sign')
    guest.wait(20)
    placed = host.raw('block', *spot).get('block')
    host.shot(out, 'sign')
    assert placed and 'sign' in placed, f'the guest placed {placed}'
    host_unaffected(host, "the guest's sign")
    guest.raw('key', 'esc', 'tap')
    guest.raw('wait-for', 'screen', 'playing')
    return placed


def effect_ids(game):
    return [e.get('id', '').removeprefix('minecraft:') for e in game('state')['player'].get('effects', [])]


def check_effects(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    bx, by, bz = x - 3, y, z - 3
    host('command', command=f'fill {bx - 1} {by - 1} {bz - 1} {bx + 1} {by - 1} {bz + 1} minecraft:iron_block')
    host('command', command=f'setblock {bx} {by} {bz} minecraft:beacon{{primary_effect:"minecraft:speed"}}')
    # BeaconBlockEntity.tick applies its powers every 80 ticks, once the beam has reached the sky.
    seen = {}
    for _ in range(15):
        host.wait(20)
        seen = {'host': effect_ids(host), 'guest': effect_ids(guest)}
        if all('speed' in v for v in seen.values()):
            break
    host.shot(out, 'effects')
    guest.shot(out, 'effects')
    assert 'speed' in seen['host'] and 'speed' in seen['guest'], f'beacon effects: {seen}'
    return seen


def wait_dimension(game, dimension, ticks):
    """Wait up to `ticks` for the game to be in `dimension` with its world loaded around it."""
    for _ in range(ticks // 20):
        state = game('state')
        if state.get('dimension') == dimension and state.get('player'):
            return state
        game.wait(20)
    return game('state')


def nether_portal(game, x, y, z):
    """A lit portal along x, two blocks wide and three tall, its inside starting at x y z."""
    game('command', command=f'fill {x - 1} {y - 1} {z} {x + 2} {y + 3} {z} minecraft:obsidian')
    game('command', command=f'fill {x} {y} {z} {x + 1} {y + 2} {z} minecraft:nether_portal[axis=x]')


def lan_guest(host):
    """The guest's entry in the host's `lan status` (on the server thread the owner is listed too)."""
    guests = host.raw('lan', 'status')['guests']
    return next(g for g in guests if g['name'] == GUEST_NAME)

def check_distance(host, guest, out):
    """IntegratedServer.tickServer: the server takes the host's render distance as its view distance
    (past 32, which Horizon's options allow) and sends it to every player (SetChunkCacheRadius)."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    seen = {}
    try:
        for distance in (48, 6):
            host.raw('render-distance', distance)
            # The pause screen doesn't hold the server here: it follows on its next tick.
            host.wait(20)
            guest.wait(20)
            seen[distance] = (host('status').get('render_distance'), guest('status').get('render_distance'))
    finally:
        host.raw('render-distance', 12)
    # A client streams out to min(its render distance, the server's view distance + 1).
    assert seen[48][0] == 48, f'the host at 48 streams {seen[48][0]} chunks out (all: {seen})'
    assert seen[6][0] == 6, f'the host at 6 streams {seen[6][0]} chunks out (all: {seen})'
    assert seen[6][1] == 7, f'the guest streams {seen[6][1]} with the server at 6 (all: {seen})'
    return seen


def check_portals(host, guest, out):
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    x, y, z = (int(v // 1) for v in pos_of(guest))
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    try:
        return portals(host, guest, out, x, y, z, hx, hy, hz)
    finally:
        # The guest comes home into its portal's frame (an obsidian ceiling three blocks up) and the
        # host onto its End portal at spawn: take both away so the next checks start in the open.
        home = 'execute in minecraft:overworld run'
        guest('command', command=f'{home} fill {x - 1} {y} {z + 3} {x + 2} {y + 3} {z + 3} minecraft:air')
        host('command', command=f'{home} setblock {hx} {hy} {hz} minecraft:air')
        guest('command', command=f'{home} tp @s {x + 0.5} {y} {z + 0.5}')
        guest.wait(20)


def portals(host, guest, out, x, y, z, hx, hy, hz):
    nether_portal(guest, x, y, z + 3)
    guest.wait(5)
    guest('command', command=f'tp @s {x + 0.5} {y} {z + 3.5}')
    # players_nether_portal_default_delay: 80 ticks in survival.
    there = wait_dimension(guest, 'minecraft:the_nether', 600)
    # On a server the guest is held where it entered until the chunks around the destination are
    # lit, then placed in its portal: wait for that before reading where it arrived.
    for _ in range(30):
        guest.wait(20)
        arrived = [int(v // 1) for v in pos_of(guest)]
        if guest.raw('block', *arrived).get('block') == 'minecraft:nether_portal':
            break
    guest.shot(out, 'portals-nether')
    host_dim = host('state').get('dimension')
    assert there.get('dimension') == 'minecraft:the_nether', f'the guest is in {there.get("dimension")}'
    assert host_dim == 'minecraft:overworld', f'the host went to {host_dim} with the guest'
    below = guest.raw('block', arrived[0], arrived[1] - 1, arrived[2])
    assert below.get('loaded') and below.get('block') != 'minecraft:air', f'under the guest in the Nether: {below}'
    # The host takes an End portal while the guest is in the Nether: three levels at once.
    host('command', command=f'setblock {hx} {hy} {hz} minecraft:end_portal')
    end = wait_dimension(host, 'minecraft:the_end', 600)
    host.wait(40)
    host.shot(out, 'portals-end')
    guest_dim = guest('state').get('dimension')
    assert end.get('dimension') == 'minecraft:the_end', f'the host is in {end.get("dimension")}'
    assert guest_dim == 'minecraft:the_nether', f'the guest left the Nether for {guest_dim}'
    # Both come back: the host through an exit portal, the guest out of its portal and in again.
    ex, ey, ez = (int(v // 1) for v in pos_of(host))
    host('command', command=f'setblock {ex + 2} {ey} {ez} minecraft:end_portal')
    host('command', command=f'tp @s {ex + 2.5} {ey} {ez + 0.5}')
    guest('command', command=f'tp @s {arrived[0] + 0.5} {arrived[1]} {arrived[2] - 2.5}')
    guest.wait(30)
    guest('command', command=f'tp @s {arrived[0] + 0.5} {arrived[1]} {arrived[2] + 0.5}')
    # ServerPlayer.showEndCredits: the host's first exit from the End rolls the poem and credits;
    # closing them (WinScreen.onClose sends PERFORM_RESPAWN) takes it home.
    credits = False
    for _ in range(60):
        host.wait(5)
        if host('screen').get('screen') == 'win':
            credits = True
            host.raw('key', 'esc', 'tap')
            break
    assert credits, 'the host left the End without the credits'
    home = wait_dimension(host, 'minecraft:overworld', 600)
    back = wait_dimension(guest, 'minecraft:overworld', 600)
    guest.wait(40)
    host.shot(out, 'portals-back')
    guest.shot(out, 'portals-back')
    assert home.get('dimension') == 'minecraft:overworld', f'the host stayed in {home.get("dimension")}'
    assert back.get('dimension') == 'minecraft:overworld', f'the guest stayed in {back.get("dimension")}'
    return {'guest_nether': arrived, 'host_end': [ex, ey, ez], 'credits': credits, 'back': pos_of(guest)}


NETHER, END, OVERWORLD = 'minecraft:the_nether', 'minecraft:the_end', 'minecraft:overworld'


def check_levels(host, guest, out):
    """(--server-thread) Each player in its own level (MinecraftServer.levels), the host and the guest
    apart: the steps run in order, each reported, and the check fails if any did."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    start = {g.name: pos_of(g) for g in (host, guest)}
    steps, failed = {}, []
    guest('command', command='gamemode creative')
    try:
        for name, step in LEVEL_STEPS:
            try:
                steps[name] = step(host, guest, out)
            except (Failed, AssertionError, StopIteration, KeyError, TypeError, IndexError) as e:
                steps[name], failed = repr(e), failed + [name]
                for g in (host, guest):
                    back_to_game(g)
            print(f'[lan] levels {name}: {"FAIL" if name in failed else "PASS"} {steps[name]}')
    finally:
        for g in (host, guest):
            back_to_game(g)
            if g('state').get('screen') == 'dead':
                g.raw('respawn')
                g.wait(20)
            x, y, z = start[g.name]
            g('command', command=f'execute in {OVERWORLD} run tp @s {x:.2f} {y:.2f} {z:.2f}')
        guest('command', command='gamemode survival')
        guest.wait(40)
    assert not failed, f'failed: {failed}; {steps}'
    return steps


def level_floor(game, x, y, z):
    """An obsidian floor under x y z with air above it, in the level `game` is in (fill needs the
    chunks loaded: ServerLevel.hasChunksAt)."""
    game('command', command=f'fill {x - 4} {y - 1} {z - 4} {x + 4} {y - 1} {z + 4} minecraft:obsidian')
    game('command', command=f'fill {x - 4} {y} {z - 4} {x + 4} {y + 4} {z + 4} minecraft:air')


def into_level(game, dimension, x, y, z):
    """`execute in DIMENSION run tp` to x y z, onto a floor built once the level has loaded around it."""
    game('command', command=f'execute in {dimension} run tp @s {x + 0.5} {y} {z + 0.5}')
    state = wait_dimension(game, dimension, 400)
    game.wait(20)
    level_floor(game, x, y, z)
    game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    game.wait(20)
    return state


def dims(host, guest):
    return host('state').get('dimension'), guest('state').get('dimension')


def level_execute(host, guest, out):
    """`execute in the_nether run tp` takes the guest alone. From the Nether, the guest's commands
    reach the host's level (CommandSourceStack.getLevel): `execute in overworld` and `execute at`
    the host put a block beside the host, and `execute if block` reads it."""
    x, z = (int(v // 1) for v in pos_of(guest)[::2])
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    there = into_level(guest, NETHER, x, 100, z)
    below = guest.raw('block', x, 99, z)
    gold = f'{hx} {hy} {hz + 2} minecraft:gold_block'
    try:
        guest('command', command=f'execute in {OVERWORLD} run setblock {gold}')
        guest('command', command=f'execute at {host.player} run setblock ~ ~ ~-2 minecraft:diamond_block')
        guest('command', command=f'execute in {OVERWORLD} if block {gold} in {NETHER} '
                                  f'run setblock {x} 99 {z} minecraft:gold_block')
        host.wait(10)
        placed = [host.raw('block', hx, hy, hz + 2), host.raw('block', hx, hy, hz - 2)]
        found = guest.raw('block', x, 99, z)
    finally:
        host('command', command=f'fill {hx} {hy} {hz - 2} {hx} {hy} {hz + 2} minecraft:air')
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert there.get('dimension') == NETHER and below.get('block') == 'minecraft:obsidian', f'{there} on {below}'
    assert [p.get('block') for p in placed] == ['minecraft:gold_block', 'minecraft:diamond_block'], \
        f'beside the host: {placed}'
    assert found.get('block') == 'minecraft:gold_block', f'execute in overworld if block: {found}'
    return {'guest': pos_of(guest)}


def level_ticking(host, guest, out):
    """Mobs and objects fall in both levels at once (each level with a player ticks); every entity
    has its own id across the levels."""
    gx, gy, gz = pos_of(guest)
    hx, hy, hz = pos_of(host)
    # Clear the whole walking area in each level: normal pig AI can leave a
    # narrow landing strip during the fall/observation window.
    for game, x, y, z in ((host, hx, hy, hz), (guest, gx, gy, gz)):
        bx, by, bz = int(x // 1), int(y // 1), int(z // 1)
        game('command', command=f'fill {bx - 12} {by - 1} {bz - 12} {bx + 12} {by - 1} {bz + 12} minecraft:stone')
        game('command', command=f'fill {bx - 12} {by} {bz - 12} {bx + 12} {by + 6} {bz + 12} minecraft:air')
    guest('command', command=f'summon minecraft:pig {gx + 2.5:.2f} {gy + 3:.2f} {gz:.2f}')
    guest('command', command=f'summon minecraft:armor_stand {gx - 2.5:.2f} {gy + 3:.2f} {gz:.2f}')
    host('command', command=f'summon minecraft:pig {hx + 2.5:.2f} {hy + 3:.2f} {hz - 3:.2f}')
    host('command', command=f'summon minecraft:armor_stand {hx - 2.5:.2f} {hy + 3:.2f} {hz - 3:.2f}')
    try:
        guest.wait(60)
        seen = {}
        for name, g, floor in (('guest', guest, gy), ('host', host, hy)):
            mobs = [m for m in g.raw('mobs', 8)['near'] if m['kind'] == 'minecraft:pig']
            stands = [o for o in g('status')['objects'] if o['kind'] == 'minecraft:armor_stand']
            seen[name] = {'pigs': [(m['id'], round(m['pos'][1], 2)) for m in mobs],
                          'pig_positions': [m['pos'] for m in mobs],
                          'stands': [(o['id'], round(o['position'][1], 2)) for o in stands], 'floor': floor}
    finally:
        guest('command', command='kill @e[type=!minecraft:player,distance=..8]')
        host('command', command='kill @e[type=!minecraft:player,distance=..8]')
    for name, s in seen.items():
        heights = [y for _, y in s['pigs'] + s['stands']]
        assert len(s['pigs']) == 1 and len(s['stands']) == 1, f'{name} sees {s}'
        assert all(abs(y - s['floor']) < 0.1 for y in heights), f"{name}'s did not fall to the floor: {seen}"
    ids = [i for s in seen.values() for i, _ in s['pigs'] + s['stands']]
    assert len(set(ids)) == len(ids), f'ids shared across the levels: {seen}'
    return seen


def level_rejoin(host, guest, out):
    """The guest leaves in the Nether and joins again there (its player data keeps its level)."""
    before = pos_of(guest)
    port = host.raw('lan', 'status')['port']
    guest.raw('disconnect')
    time.sleep(2)
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    after = pos_of(guest)
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert all(abs(a - b) < 1 for a, b in zip(before, after)), f'left at {before}, back at {after}'
    return {'at': after}


def level_death(host, guest, out):
    """The guest dies in the Nether and respawns in the Overworld; the host stays where it is."""
    guest('command', command='kill @s')
    guest.wait(20)
    guest.raw('respawn')
    back = wait_dimension(guest, OVERWORLD, 400)
    guest.wait(20)
    assert back.get('dimension') == OVERWORLD, f'the guest respawned in {back.get("dimension")}'
    assert dims(host, guest) == (OVERWORLD, OVERWORLD), f'host, guest in {dims(host, guest)}'
    return {'respawned': back['player']['position']}


def level_respawn_points(host, guest, out):
    """Respawn points outside the Overworld (ServerPlayer.findRespawnAndUseSpawnBlock): a point
    `spawnpoint` sets in the Nether (forced), then a charged respawn anchor the guest uses there,
    each bring the guest back in the Nether; the anchor loses a charge."""
    home = pos_of(guest)
    x, z = (int(v // 1) for v in home[::2])
    into_level(guest, NETHER, x, 100, z)
    results = {}
    try:
        guest('command', command=f'spawnpoint @s {x} 100 {z}')
        results['spawnpoint'] = die_and_respawn(guest)
        guest('command', command=f'setblock {x + 2} 100 {z} minecraft:respawn_anchor[charges=2]')
        guest.wait(10)
        guest.raw('interact', x + 2, 100, z)
        guest.wait(10)
        results['anchor'] = die_and_respawn(guest)
        results['charges'] = guest.raw('block', x + 2, 100, z).get('state')
    finally:
        sx, sy, sz = (int(v // 1) for v in pos_of(host))
        guest('command', command=f'execute in {OVERWORLD} run spawnpoint @s {sx} {sy} {sz}')
        guest('command', command=f'execute in {NETHER} run setblock {x + 2} 100 {z} minecraft:air')
        guest('command', command=f'execute in {OVERWORLD} run tp @s {home[0]:.2f} {home[1]:.2f} {home[2]:.2f}')
        wait_dimension(guest, OVERWORLD, 400)
    point, anchor = results['spawnpoint'], results['anchor']
    assert point[0] == NETHER and abs(point[1][0] - x - 0.5) < 0.6 and abs(point[1][2] - z - 0.5) < 0.6, \
        f'the spawnpoint brought the guest to {point}'
    assert anchor[0] == NETHER and abs(anchor[1][0] - x - 2.5) < 2 and abs(anchor[1][2] - z - 0.5) < 2, \
        f'the anchor brought the guest to {anchor}'
    assert 'charges=1' in (results['charges'] or ''), f'the anchor after the respawn: {results["charges"]}'
    return results


def die_and_respawn(guest):
    """Kill the guest and respawn it: the dimension and position it comes back at, once settled."""
    guest('command', command='kill @s')
    guest.wait(20)
    guest.raw('respawn')
    guest.wait(80)
    state = guest('state')
    return state.get('dimension'), state['player']['position']


def level_host_nether(host, guest, out):
    """The host goes through a Nether portal while the guest stays in the Overworld."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    nether_portal(host, hx, hy, hz - 3)
    try:
        host('command', command=f'tp @s {hx + 0.5} {hy} {hz - 2.5}')
        there = wait_dimension(host, NETHER, 600)
        host.wait(20)
        result = dims(host, guest)
        assert there.get('dimension') == NETHER, f'the host is in {there.get("dimension")}'
        assert result == (NETHER, OVERWORLD), f'host, guest in {result}'
        return {'host': there['player']['position']}
    finally:
        host('command', command=f'execute in {OVERWORLD} run fill {hx - 1} {hy} {hz - 3} {hx + 2} {hy + 3} {hz - 3} '
                                'minecraft:air')
        host('command', command=f'execute in {OVERWORLD} run tp @s {hx + 0.5} {hy} {hz + 0.5}')
        wait_dimension(host, OVERWORLD, 400)


def level_guest_end(host, guest, out):
    """The guest steps into an End portal alone and lands on the obsidian platform, an End gateway
    takes it across the End, and an exit portal takes it back to the Overworld."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {x + 2} {y} {z} minecraft:end_portal')
    try:
        guest('command', command=f'tp @s {x + 2.5} {y} {z + 0.5}')
        wait_dimension(guest, END, 600)
        # The arrival waits for the platform's chunks; its portal cooldown (10 ticks) runs out after,
        # and a player who stands in a portal while it runs stays (Entity.setAsInsidePortal).
        guest.wait(60)
        landed = pos_of(guest)
        assert dims(host, guest) == (OVERWORLD, END), f'host, guest in {dims(host, guest)}'
        assert [int(v // 1) for v in landed] == [100, 49, 0], f'the guest landed at {landed}'
    finally:
        guest('command', command=f'execute in {OVERWORLD} run setblock {x + 2} {y} {z} minecraft:air')
    # A gateway far out with an exact exit at the End spawn, as end_gateway_return's
    # (TheEndGatewayBlockEntity.getPortalPosition), takes the guest back there within the End.
    guest('command', command='tp @s 700.5 80 0.5')
    guest.wait(60)
    level_floor(guest, 700, 80, 0)
    guest('command', command='tp @s 700.5 80 0.5')
    guest.wait(20)
    guest('command', command='setblock 702 80 0 minecraft:end_gateway{exit_portal:[I;100,50,0],exact_teleport:1b}')
    guest('command', command='tp @s 702.5 80 0.5')
    gateway = None
    for _ in range(20):
        guest.wait(10)
        gateway = pos_of(guest)
        if abs(gateway[0] - 702.5) > 50:
            break
    assert dims(host, guest) == (OVERWORLD, END), f'host, guest in {dims(host, guest)}'
    assert abs(gateway[0] - 100.5) < 1 and abs(gateway[2] - 0.5) < 1 and 49 <= gateway[1] <= 50, \
        f'the gateway left the guest at {gateway}'
    guest.wait(40)
    ex, ey, ez = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {ex + 2} {ey} {ez} minecraft:end_portal')
    guest('command', command=f'tp @s {ex + 2.5} {ey} {ez + 0.5}')
    home = wait_dimension(guest, OVERWORLD, 600)
    guest.wait(20)
    assert dims(host, guest) == (OVERWORLD, OVERWORLD), f'host, guest in {dims(host, guest)}'
    return {'landed': landed, 'gateway': gateway, 'home': home['player']['position']}


def level_reload(host, guest, out):
    """The host saves and quits with the guest in the Nether, opens the world again and opens it to the
    LAN; the guest joins again where it was, in the Nether."""
    x, z = (int(v // 1) for v in pos_of(guest)[::2])
    into_level(guest, NETHER, x, 100, z)
    before = pos_of(guest)
    host.raw('disconnect')
    # The world saves on its server thread before the title screen shows.
    for attempt in range(60):
        time.sleep(1)
        try:
            host.raw('open', 'Lan')
            break
        except Failed:
            if attempt == 59:
                raise
    host('wait_ready', seconds=400)
    port = host.raw('lan', 'open', 0, 'commands')['port']
    back_to_game(guest)
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    after = pos_of(guest)
    assert dims(host, guest) == (OVERWORLD, NETHER), f'host, guest in {dims(host, guest)}'
    assert all(abs(a - b) < 1 for a, b in zip(before, after)), f'left at {before}, back at {after}'
    return {'at': after}


LEVEL_STEPS = [('execute', level_execute), ('ticking', level_ticking), ('rejoin', level_rejoin),
               ('death', level_death), ('respawn_points', level_respawn_points),
               ('host_nether', level_host_nether),
               ('guest_end', level_guest_end), ('reload', level_reload)]


def check_damage(host, guest, out):
    guest('command', command='gamemode survival')
    guest.wait(10)
    before = remote(guest)['health']
    x, y, z = pos_of(guest)
    guest('command', command=f'tp @s {x:.2f} {y + 12:.2f} {z:.2f}')
    guest.wait(50)
    after = remote(guest)['health']
    guest.shot(out, 'damage')
    assert after < before, f'health {before} -> {after} after a 12-block fall'
    # Into water: the fall does no damage.
    x, y, z = pos_of(guest)
    guest('command', command=f'setblock {int(x // 1)} {int(y // 1)} {int(z // 1)} minecraft:water')
    guest('command', command=f'tp @s {x:.2f} {y + 12:.2f} {z:.2f}')
    guest.wait(50)
    water = remote(guest)['health']
    assert water >= after, f'health {after} -> {water} after falling into water'
    return {'before': before, 'after': after, 'water': water}


def first_hurt(guest, before, ticks):
    """The guest's health once it drops below `before`, waiting at most `ticks`."""
    health = remote(guest)['health']
    for _ in range(ticks // 10):
        if health < before:
            break
        guest.wait(10)
        health = remote(guest)['health']
    return health


def check_mobs(host, guest, out):
    host('command', command='gamemode creative')
    host('command', command='difficulty normal')
    host('command', command='time set midnight')
    guest('command', command='gamemode survival')
    guest.wait(10)
    hurt = {}
    try:
        for mob, distance in (('zombie', 3), ('skeleton', 7)):
            x, y, z = pos_of(guest)
            before = remote(guest)['health']
            host('command', command=f'summon minecraft:{mob} {x:.2f} {y:.2f} {z + distance:.2f}')
            low = first_hurt(guest, before, 300)
            guest.shot(out, f'mobs-{mob}')
            host.shot(out, f'mobs-{mob}')
            host('command', command=f'kill @e[type=minecraft:{mob}]')
            hurt[mob] = (before, low)
    finally:
        host('command', command='gamemode survival')
        host('command', command='time set day')
    for mob, (before, low) in hurt.items():
        assert low < before, f'the {mob} never hurt the guest: health {before} -> {low} (all: {hurt})'
    return hurt


def check_hazards(host, guest, out):
    guest('command', command='gamemode survival')
    # A fresh life: full health, so no regeneration eats food while the guest runs.
    guest('command', command='kill @s')
    guest.wait(20)
    # ServerPlayer.doTick sends the dead player's health of 0 too.
    died = {'screen': guest('state').get('screen'), 'health': remote(guest)['health']}
    assert died == {'screen': 'dead', 'health': 0.0}, f'the guest was killed: {died}'
    guest.raw('respawn')
    guest.wait(40)
    # Sprint-jumping down a stone lane: ServerPlayer.jumpFromGround and checkMovementStatistics.
    guest('command', command='fill ~ ~-1 ~-1 ~60 ~-1 ~1 minecraft:stone')
    guest('command', command='fill ~ ~ ~-1 ~60 ~3 ~1 minecraft:air')
    guest.wait(10)
    x, y, z = pos_of(guest)
    guest.raw('look-at', f'{x + 60:.2f}', f'{y + 1.6:.2f}', f'{z:.2f}')
    food = lan_guest(host)['food']
    for k in ('w', 'ctrl', 'space'):
        guest.raw('key', k, 'down')
    guest.wait(160)
    for k in ('space', 'ctrl', 'w'):
        guest.raw('key', k, 'up')
    guest.wait(10)
    fed = lan_guest(host)['food']
    hunger = {'before': food, 'after': fed}
    hurt = {}
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    x, y, z = hx + 3, hy, hz
    for name, fill, ticks in (('lava', 'minecraft:lava', 60), ('wall', 'minecraft:stone', 60),
                              ('drowning', 'minecraft:water', 500), ('void', None, 60)):
        if dead(guest):
            guest.raw('respawn')
            guest.wait(40)
        # A stone box around the guest holds the lava and the water in.
        guest('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:stone')
        guest('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:air')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        guest.wait(100)
        before = remote(guest)['health']
        if fill:
            guest('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} {fill}')
        else:
            guest('command', command=f'tp @s {x + 0.5} -140 {z + 0.5}')
        hurt[name] = (before, first_hurt(guest, before, ticks))
        guest.shot(out, f'hazards-{name}')
        # The host tidies up: the void can kill the guest before this, the server drops a dead
        # guest's packets (lan_guest_packet), and the box left standing suffocated it in pvp.
        # Water puts the lava's fire out.
        host('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:water')
        guest.wait(5)
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone')
    guest.wait(20)
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    # [food, saturation, exhaustion]: 8 s of sprint-jumping is about 2 of exhaustion; every 4
    # of it takes a point of saturation, or of food once saturation is gone (FoodData.tick).
    spent = 4 * (food[0] + food[1] - fed[0] - fed[1]) + fed[2] - food[2]
    assert spent > 1.0, f'no hunger from moving: {hunger} ({spent:.2f} spent)'
    for name, (before, after) in hurt.items():
        assert after < before, f'{name} never hurt the guest: health {before} -> {after} (all: {hurt})'
    return {'died': died, 'hunger': hunger, **hurt}


def check_pvp(host, guest, out):
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:iron_sword')
    face_each_other(host, guest)
    hits = {}
    try:
        for name, rule, attacker in (('guest hits host', 'true', guest), ('host hits guest', 'true', host),
                                     ('pvp off', 'false', guest), ('guest hits teammate', 'true', guest),
                                     ('host hits teammate', 'true', host)):
            host('command', command=f'gamerule pvp {rule}')
            if name == 'guest hits teammate':
                # Player.canHarmPlayer: the victim's team without friendly fire spares its allies.
                host('command', command='team add parity_allies')
                host('command', command='team modify parity_allies friendlyFire false')
                host('command', command='team join parity_allies @a')
            face_each_other(host, guest)
            attacker.wait(30)  # a full attack charge
            victim = (lambda: host('state')['player']['health']) if attacker is guest else (lambda: remote(guest)['health'])
            before = victim()
            attacker.raw('attack')
            attacker.wait(10)
            hits[name] = (before, victim())
            attacker.shot(out, 'pvp-' + name.replace(' ', '-'))
    finally:
        host('command', command='gamerule pvp true')
        host('command', command='team remove parity_allies')
    assert hits['guest hits host'][1] < hits['guest hits host'][0], f'the guest did not hurt the host: {hits}'
    assert hits['host hits guest'][1] < hits['host hits guest'][0], f'the host did not hurt the guest: {hits}'
    # Natural regeneration may heal the victim meanwhile.
    assert hits['pvp off'][1] >= hits['pvp off'][0], f'pvp off still hurts: {hits}'
    for name in ('guest hits teammate', 'host hits teammate'):
        assert hits[name][1] == hits[name][0], f'friendly fire off still hurts ({name}): {hits}'
    return hits


def leave_game_total(host):
    """The leave_game statistic summed over the stats files the host's world holds."""
    total = 0
    for path in (host.work / 'saves' / 'Lan' / 'players' / 'stats').glob('*.json'):
        custom = json.loads(path.read_text()).get('stats', {}).get('minecraft:custom', {})
        total += custom.get('minecraft:leave_game', 0)
    return total


def check_leave(host, guest, out):
    # PlayerList.remove awards Stats.LEAVE_GAME before it saves the player: a guest who leaves has it in
    # their saved statistics, and it counts again the next time they leave.
    port = host.raw('lan', 'status')['port']
    before = leave_game_total(host)
    counts = []
    for _ in range(2):
        guest.raw('disconnect')
        for _ in range(20):
            time.sleep(0.5)
            if leave_game_total(host) > (counts[-1] if counts else before):
                break
        counts.append(leave_game_total(host))
        guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(20)
    assert counts == [before + 1, before + 2], f'leave_game was {before}, then {counts} after two leaves'
    return {'leave_game': counts}


def check_kills(host, guest, out):
    # A mob the guest kills is the guest's kill (#484): LivingEntity.die credits lastHurtByPlayer, so the
    # killed statistic (and its scoreboard criterion) counts for the guest, not the host.
    host('command', command='scoreboard objectives add parity_kills minecraft.killed:minecraft.pig')
    for g in (host, guest):
        g('command', command='gamemode survival')
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:iron_sword')
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    try:
        # Between the two, in front of the guest, with one health left.
        host('command', command=f'summon minecraft:pig {hx + 1.5:.2f} {hy:.2f} {hz:.2f} {{NoAI:1b,Health:1.0f}}')
        guest.wait(30)
        guest.raw('look-at', f'{hx + 1.5:.2f}', f'{hy + 0.5:.2f}', f'{hz:.2f}')
        guest.wait(10)
        guest.raw('attack')
        guest.wait(20)
        host('command', command=f'scoreboard players get {GUEST_NAME} parity_kills')
        host('command', command=f'scoreboard players get {host.player} parity_kills')
        host.wait(10)
        chat = host.raw('messages', 10)['chat']
        guest.shot(out, 'kills')
    finally:
        host('command', command='kill @e[type=minecraft:pig]')
        host('command', command='scoreboard objectives remove parity_kills')
    assert any(f'{GUEST_NAME} has 1 [parity_kills]' in line for line in chat), f"the guest's kill is not theirs: {chat}"
    assert not any(f'{host.player} has 1 [parity_kills]' in line for line in chat), f"the host got the guest's kill: {chat}"
    return chat


def check_arrows(host, guest, out):
    # A player's arrow on another player (#484): AbstractArrow.onHitEntity, refused by
    # ServerPlayer.canHarmPlayer with the pvp game rule off (the arrow turns back).
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:bow')
        g('command', command='give @s minecraft:arrow 16')
    hits = {}
    try:
        for name, rule, shooter in (('guest shoots host', 'true', guest), ('host shoots guest', 'true', host),
                                    ('pvp off', 'false', guest)):
            host('command', command=f'gamerule pvp {rule}')
            for g in (host, guest):
                g('command', command='effect give @s minecraft:instant_health 1 5')
            face_each_other(host, guest)
            shooter.wait(20)
            victim = (lambda: host('state')['player']['health']) if shooter is guest else (lambda: remote(guest)['health'])
            before = victim()
            shooter.raw('use-item', '--hold', 1.3)  # a full draw (BowItem: 20 ticks)
            shooter.wait(20)
            hits[name] = (before, victim())
            shooter.shot(out, 'arrows-' + name.replace(' ', '-'))
    finally:
        host('command', command='gamerule pvp true')
        for g in (host, guest):
            g('command', command='clear @s')
    assert hits['guest shoots host'][1] < hits['guest shoots host'][0], f"the guest's arrow did not hurt the host: {hits}"
    assert hits['host shoots guest'][1] < hits['host shoots guest'][0], f"the host's arrow did not hurt the guest: {hits}"
    assert hits['pvp off'][1] >= hits['pvp off'][0], f'pvp off, an arrow still hurts: {hits}'
    return hits


def dead_players(game, want, ticks=60):
    """The other players `game`'s client draws dead, once that is `want` (empty or not) or after `ticks`."""
    for _ in range(ticks // 5):
        dead = remote(game)['dead_players']
        if bool(dead) == want:
            return dead
        game.wait(5)
    return dead


def check_respawn(host, guest, out):
    # ServerLevel.removePlayerImmediately and addRespawnedPlayer: a respawned player is removed
    # from the other players and paired again as a new entity, so their clients drop the body.
    # The host window only has a client's view of the guest on the server thread.
    for g in (host, guest):
        back_to_game(g)
    results = {}
    sides = [('host', host, guest)] + ([('guest', guest, host)] if SERVER_THREAD else [])
    for name, dying, watching in sides:
        face_each_other(host, guest)
        dying('command', command='kill @s')
        dying.wait(30)
        dead = dead_players(watching, True)
        watching.shot(out, f'respawn-{name}-dead')
        dying.raw('respawn')
        dying.wait(40)
        back_to_game(dying)
        # Back within tracking range of the other player, wherever the respawn point is.
        x, y, z = pos_of(dying)
        watching('command', command=f'tp @s {x + 3:.2f} {y:.2f} {z:.2f}')
        watching.wait(40)
        alive = dead_players(watching, False)
        watching.shot(out, f'respawn-{name}-alive')
        results[name] = {'dead': dead, 'after_respawn': alive}
        assert dead, f'the {name} was not drawn dead: {results}'
        assert not alive, f'the {name} still lies dead after respawning: {results}'
    return results


def check_hurt(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    # Between the two, within both players' reach; bare hands leave it alive.
    host('command', command=f'summon minecraft:pig {hx + 1.5:.2f} {hy:.2f} {hz + 0.8:.2f} {{NoAI:1b}}')
    host.wait(30)

    def pig(game):
        return game.raw('entities', 6, 'pig')['entities'][0]
    seen = {}
    for name, attacker, watcher in (('guest hits', guest, host), ('host hits', host, guest)):
        attacker.wait(30)  # a full attack charge
        attacker.raw('attack', pig(attacker)['id'])
        seen[name] = {'attacker': pig(attacker)['hurt_time'], 'watcher': pig(watcher)['hurt_time']}
        attacker.shot(out, 'hurt-' + name.replace(' ', '-'))
        watcher.shot(out, 'hurt-' + name.replace(' ', '-') + '-seen')
        attacker.wait(20)
    host('command', command='kill @e[type=minecraft:pig]')
    # A hurt player flashes red for the other one too (#2827).
    # An in-frame host draws its guests as models of its own, not entities: only the screenshot.
    watched = ((('host hurt', host, guest),) if not SERVER_THREAD else
               (('host hurt', host, guest), ('guest hurt', guest, host)))
    for name, hurt, watcher in watched:
        hurt.wait(30)
        # Instant damage hurts on the next effect tick.
        hurt('command', command='effect give @s minecraft:instant_damage 1 0')
        for _ in range(10):
            other = watcher.raw('entities', 6, 'player')['entities'][0]
            if other['hurt_time'] > 0:
                break
            watcher.wait(1)
        seen[name] = {'watcher': other['hurt_time']}
        watcher.shot(out, 'hurt-' + name.replace(' ', '-') + '-seen')
    if not SERVER_THREAD:
        guest.wait(30)
        guest('command', command='effect give @s minecraft:instant_damage 1 0')
        guest.wait(2)
        host.shot(out, 'hurt-guest-hurt-seen')
    # LivingEntity.hurtTime: 10 when hurt, one less each tick; the reads come a few ticks later.
    for name, sides in seen.items():
        assert all(t > 0 for t in sides.values()), f'{name}: the hurt flash {seen}'
    return seen


def mob_looks(game, centre):
    """What `check_looks` compares of each mob around `centre`, by kind and column."""
    found = {}
    for m in game.raw('mobs', 32).get('near', []):
        if abs(m['pos'][0] - centre[0]) > 12 or abs(m['pos'][2] - centre[2]) > 14:
            continue
        key = f"{m['kind'].removeprefix('minecraft:')}@{round(m['pos'][0] - centre[0])}"
        found[key] = {'baby': m['baby'], 'color': m['color'], 'sheared': m['sheared'], 'size': m['size'],
                      'tame': m['tame'], 'sitting': m['sitting'], 'burning': m['burning'],
                      'health': round(m['health'])}
    return found


def check_looks(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    centre = (x + 0.5, y, z + 0.5)
    # Each mob in a column of its own (the key), standing on the stone floor.
    looks = {
        'cow@-5': ('{NoAI:1b,Fire:80s}', {'burning': True}),
        'cow@-3': ('{NoAI:1b,Age:-24000}', {'baby': True}),
        'cow@-1': ('{NoAI:1b}', {'baby': False}),
        'sheep@1': ('{NoAI:1b,Color:14b,Sheared:1b}', {'color': 14, 'sheared': True}),
        'slime@3': ('{NoAI:1b,Size:3}', {'size': 4}),
        'wolf@5': ('{NoAI:1b,Owner:[I;1,2,3,4],Sitting:1b,CollarColor:11b}',
                   {'tame': True, 'sitting': True, 'color': 11}),
    }
    sides = {'host': host, 'guest': guest}
    seen = {}
    try:
        for key, (tag, _) in looks.items():
            kind, dx = key.split('@')
            host('command', command=f'summon minecraft:{kind} {centre[0] + int(dx)} {y} {z + 4.5} {tag}')
        host('command', command=f'summon minecraft:wither {centre[0]} {y + 3} {z + 10.5} {{NoAI:1b,Invul:0}}')
        host.wait(30)
        seen['spawned'] = {name: mob_looks(g, centre) for name, g in sides.items()}
        # Changes after the spawn: the wither hurt, the sheep dyed and grown back, the calf grown up.
        host('command', command='damage @e[type=minecraft:wither,limit=1] 150 minecraft:generic')
        for kind, dx, tag in (('sheep', 1, '{Color:11b,Sheared:0b}'), ('cow', -3, '{Age:0}')):
            host('command', command=f'data merge entity @e[type=minecraft:{kind},x={centre[0] + dx},y={y},'
                                    f'z={z + 4.5},distance=..0.5,limit=1] {tag}')
        guest.raw('look-at', centre[0], y + 1, z + 4.5)
        host.raw('look-at', centre[0], y + 1, z + 4.5)
        # The fire (80 ticks) goes out in the meantime.
        host.wait(80)
        for name, g in sides.items():
            g.shot(out, 'looks')
        seen['later'] = {name: mob_looks(g, centre) for name, g in sides.items()}
    finally:
        # A killed slime splits: shrink it first so no children are left hopping into the next check.
        host('command', command=f'data merge entity @e[type=minecraft:slime,x={centre[0] + 3},y={y},z={z + 4.5},'
                                f'distance=..2,limit=1] {{Size:0}}')
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 12},y={y - 2},z={z - 2},dx=24,dy=12,dz=16]')
    wrong = []
    for name in sides:
        spawned, later = seen['spawned'][name], seen['later'][name]
        for key, (_, want) in looks.items():
            got = spawned.get(key)
            if got is None or any(got[k] != v for k, v in want.items()):
                wrong.append(f'{name} {key}: {got}, wants {want}')
        if later.get('cow@-5', {}).get('burning') is not False:
            wrong.append(f"{name}: the cow's fire never went out: {later.get('cow@-5')}")
        if [later.get('sheep@1', {}).get(k) for k in ('color', 'sheared')] != [11, False]:
            wrong.append(f"{name}: the sheep dyed blue with its wool back: {later.get('sheep@1')}")
        if later.get('cow@-3', {}).get('baby') is not False:
            wrong.append(f"{name}: the calf grown up: {later.get('cow@-3')}")
        withers = [v['health'] for k, v in spawned.items() if k.startswith('wither')]
        hurt = [v['health'] for k, v in later.items() if k.startswith('wither')]
        if withers != [300] or hurt != [150]:
            wrong.append(f"{name}: the wither's health {withers} then {hurt}, wants [300] then [150]")
    assert not wrong, '\n'.join(wrong)
    return seen['later']['guest']


def check_leads(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        at = (x + 0.5, y, z + 2.5)

        def cow(game):
            found = [m for m in game.raw('mobs', 32).get('near', []) if m['kind'] == 'minecraft:cow'
                     and abs(m['pos'][0] - at[0]) < 1 and abs(m['pos'][2] - at[2]) < 1]
            assert found, f'{game.name} sees no cow at {at}'
            return found[0]
        player('command', command='gamemode survival')
        player('command', command=f'summon minecraft:cow {at[0]} {y} {at[2]} {{NoAI:1b}}')
        player('command', command='item replace entity @s weapon.mainhand with minecraft:lead')
        player.wait(30)
        try:
            player.raw('look-at', f'{at[0]:.2f}', f'{y + 0.7:.2f}', f'{at[2]:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(20)
            tied = {'own': cow(player)['leash'], 'other': cow(other)['leash']}
            player.shot(out, 'leads-' + label)
            other.shot(out, f'leads-{label}-seen')
            # An empty hand unties it (the lead was used up).
            player.raw('mouse', 'right', 'click')
            player.wait(20)
            untied = {'own': cow(player)['leash'], 'other': cow(other)['leash']}
        finally:
            player('command', command=f'kill @e[type=!minecraft:player,x={at[0]},y={y},z={at[2]},distance=..6]')
            player('command', command='clear @s')
        result[label] = {'tied': tied, 'untied': untied}
        # The other side draws it held by the player who tied it (another player to it).
        assert tied['own'] == 'Player' and tied['other'], f"the {label}'s cow on its lead: {tied}"
        assert untied == {'own': None, 'other': None}, f"the {label}'s cow untied: {untied}"
    return result


def container_items(game):
    """The open container's stacks by slot (Horizon's container slots are 300 and up) and its size."""
    slots = [s for s in game('screen').get('slots', []) if s.get('slot', 0) >= 300]
    return {s['slot'] - 300: s['item'] for s in slots if s.get('item')}, len(slots)


def open_container(guest, pos, out, label, take=None):
    """Open the container at `pos` and read it; `take` shift-clicks that container slot out first."""
    guest.raw('interact', *pos)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    # The server's menu (an ender or double chest has no stand-in block entity: key i32::MIN),
    # then its contents, can come a few ticks after the screen.
    for _ in range(20):
        guest.wait(2)
        screen = guest('screen')
        items = {s['slot']: s.get('item') for s in screen.get('slots', [])}
        if screen.get('container') == [-2**31, 0, 0] and (take is None or items.get(300 + take)):
            break
    guest.wait(5)
    if take is not None:
        guest.raw('click-slot', 300 + take, 'shift')
        guest.wait(5)
    items = container_items(guest)
    guest.shot(out, label)
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    return items


def check_containers(host, guest, out):
    host('command', command='item replace entity @s enderchest.0 with minecraft:diamond')
    guest('command', command='item replace entity @s enderchest.4 with minecraft:emerald')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {x} {y} {z + 2} minecraft:ender_chest')
    guest.wait(10)
    ender, ender_size = open_container(guest, (x, y, z + 2), out, 'ender-chest')
    # Facing north, a left half pairs with the block to its east (ChestBlock.getConnectedDirection).
    guest('command', command=f'setblock {x} {y} {z - 2} minecraft:chest[facing=north,type=left]')
    guest('command', command=f'setblock {x + 1} {y} {z - 2} minecraft:chest[facing=north,type=right]')
    guest('command', command=f'item replace block {x + 1} {y} {z - 2} container.0 with minecraft:gold_ingot')
    guest('command', command=f'item replace block {x} {y} {z - 2} container.0 with minecraft:iron_ingot')
    guest.wait(10)
    double, double_size = open_container(guest, (x, y, z - 2), out, 'double-chest')
    # Taking from each writes back to the guest's ender inventory and to the right half.
    guest('command', command='clear @s')
    open_container(guest, (x, y, z + 2), out, 'ender-chest-take', take=4)
    open_container(guest, (x, y, z - 2), out, 'double-chest-take', take=0)
    taken = guest('inventory')['counts']
    after = (open_container(guest, (x, y, z + 2), out, 'ender-chest-after')[0],
             open_container(guest, (x, y, z - 2), out, 'double-chest-after')[0])
    host_ender = host('command', command='data get entity @s EnderItems')
    for at in ((x, y, z + 2), (x, y, z - 2), (x + 1, y, z - 2)):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    assert ender == {4: 'minecraft:emerald'} and ender_size == 27, f'the guest\'s ender chest: {ender} ({ender_size})'
    assert double_size == 54, f'the double chest opened with {double_size} slots'
    assert double == {0: 'minecraft:gold_ingot', 27: 'minecraft:iron_ingot'}, f'the double chest: {double}'
    assert 'emerald' not in json.dumps(host_ender), f'the guest\'s item went to the host: {host_ender}'
    assert taken.get('minecraft:emerald') == 1 and taken.get('minecraft:gold_ingot') == 1, f'taken: {taken}'
    assert after == ({}, {27: 'minecraft:iron_ingot'}), f'after taking: {after}'
    return {'ender': ender, 'double': double, 'taken': taken}


def check_names(host, guest, out):
    # BlockItem.updateBlockEntityComponents gives the item's custom_name to the block entity, and its
    # menu opens with it as the title (ClientboundOpenScreenPacket), shulker boxes and copper chests too.
    x, y, z = (int(v // 1) for v in pos_of(guest))
    titles = {}
    for i, item in enumerate(('red_shulker_box', 'copper_chest', 'chest')):
        spot = (x - 2 + 2 * i, y, z + 2)
        guest('command', command=f'setblock {spot[0]} {spot[1] - 1} {spot[2]} minecraft:stone')
        guest('command', command=f'setblock {spot[0]} {spot[1]} {spot[2]} minecraft:air')
        guest('command', command=f'give @s minecraft:{item}[minecraft:custom_name="Named {i}"]')
        guest.wait(10)
        guest.raw('place', *spot, '--item', f'minecraft:{item}')
        guest.wait(10)
        guest.raw('interact', *spot)
        guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
        title = guest.until(lambda: guest('screen').get('title'), bool, ticks=40)
        titles[item] = ''.join(span['text'] for span in title)
        guest.shot(out, f'names-{item}')
        guest.raw('key', 'esc', 'tap')
        guest.wait(5)
        guest('command', command=f'setblock {spot[0]} {spot[1]} {spot[2]} minecraft:air')
    want = {'red_shulker_box': 'Named 0', 'copper_chest': 'Named 1', 'chest': 'Named 2'}
    assert titles == want, f'the titles of the renamed containers: {titles}'
    return titles


def check_machines(host, guest, out):
    # After `recipes`: the stacks given here unlock recipes, which would move the book's first one.
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    seen = {}
    for block in ('hopper', 'dispenser', 'dropper'):
        guest('command', command='clear @s')
        guest('command', command='give @s minecraft:cobblestone 4')
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
        guest.wait(10)
        guest.raw('interact', *at)
        guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
        guest.wait(10)
        size = container_items(guest)[1]
        guest.shot(out, f'machines-{block}')
        guest.raw('click-slot', hotbar_slot(guest, 'minecraft:cobblestone'), 'shift')
        guest.wait(10)
        guest.raw('key', 'esc', 'tap')
        guest.wait(10)
        # Opened again, the menu holds what the server has.
        seen[block] = (size, open_container(guest, at, out, f'machines-{block}-after')[0])
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    want = {'hopper': (5, {0: 'minecraft:cobblestone'}), 'dispenser': (9, {0: 'minecraft:cobblestone'}),
            'dropper': (9, {0: 'minecraft:cobblestone'})}
    assert seen == want, f'the machines on the guest: {seen}'
    return seen


def check_brewing(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    loads = ('blaze_powder', 'nether_wart', 'glass_bottle')
    for item in loads:
        guest('command', command=f'give @s minecraft:{item}')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:brewing_stand')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    size = container_items(guest)[1]
    # BrewingStandMenu.quickMoveStack: blaze powder to the fuel slot, an ingredient to its slot,
    # a bottle to the bottle slots.
    for item in loads:
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    # BrewingStandBlockEntity.serverTick burns the blaze powder into 20 uses; the guest sees them
    # through the menu's data slots.
    guest.wait(10)
    info = guest.raw('menu')['info']
    guest.shot(out, 'brewing')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    items = open_container(guest, at, out, 'brewing-after')[0]
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'size': size, 'items': items, 'fuel': info and info.get('fuel')}
    want = {'size': 5, 'items': {0: 'minecraft:glass_bottle', 3: 'minecraft:nether_wart'}, 'fuel': 20}
    assert seen == want, f'the brewing stand on the guest: {seen}'
    return seen


def open_station(guest, block, out, label):
    """Place `block` two blocks south of the guest and open it: the station's position."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'station', '--timeout', 10)
    guest.wait(10)
    guest.shot(out, label)
    return at


def station_slots(guest):
    """The open station's stacks: input slots 600 and up, the result at 650."""
    menu = guest.raw('menu')
    return {s['slot']: (s['stack']['item'], s['stack']['count']) for s in menu['slots'] if s['slot'] >= 600}


def close_station(guest, at):
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')


def check_loom(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    for item in ('white_banner', 'red_dye'):
        guest('command', command=f'give @s minecraft:{item}')
    at = open_station(guest, 'loom', out, 'loom')
    # LoomMenu.quickMoveStack: a banner to the banner slot, a dye to the dye slot.
    for item in ('white_banner', 'red_dye'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    loaded = station_slots(guest)
    # LoomScreen's first pattern button (clickMenuButton 0): the server makes the result.
    guest.raw('click-gui', 67, 20)
    guest.wait(10)
    made = station_slots(guest).get(650)
    guest.shot(out, 'loom-pattern')
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    # The server's copy of the banner the guest took has the red layer.
    since = guest.raw('events')['last']
    guest('command', command='data get entity @s Inventory[{id:"minecraft:white_banner"}].components')
    layer = guest.raw('wait-for', 'chat', 'color: "red"', '--timeout', 10, '--since', since).get('ok')
    counts = guest('inventory')['counts']
    seen = {'loaded': loaded, 'made': made, 'left': left, 'layer': layer, 'counts': counts}
    want = {'loaded': {600: ('minecraft:white_banner', 1), 601: ('minecraft:red_dye', 1)},
            'made': ('minecraft:white_banner', 1), 'left': {}, 'layer': True,
            'counts': {'minecraft:white_banner': 1}}
    assert seen == want, f'the loom on the guest: {seen}'
    return seen


def own_reply(event):
    """A chat event that answers the guest's own command. On the server thread the guest also gets
    its own command's `[Guest: ...]` operator broadcast before #1373, a moment later, which an answer
    to the next command must not be taken for."""
    return event.get('event') == 'chat' and not event.get('text', '').startswith(f'[{GUEST_NAME}: ')


def server_count(guest, item):
    """How many of `item` the server says the guest has (`clear` with a count of 0 only counts)."""
    since = guest.raw('events')['last']
    guest('command', command=f'clear @s minecraft:{item} 0')
    for n in range(0, 65):
        reply = guest.raw('events', '--since', since)
        lines = [e.get('text', '') for e in reply['events'] if own_reply(e)]
        for line in lines:
            if 'matching item' in line or 'No items were found' in line:
                return 0 if 'No items' in line else int(line.split('Found ')[1].split(' ')[0])
        guest.wait(2)
    raise Failed(f'no answer to clear for {item}')


def check_stonecutter(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:stone 3')
    at = open_station(guest, 'stonecutter', out, 'stonecutter')
    # StonecutterMenu.quickMoveStack: stone has recipes, so it goes to the input.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:stone'), 'shift')
    guest.wait(10)
    recipes = guest.raw('menu')['info']['recipes']
    index = recipes.index('minecraft:stone_slab_from_stone_stonecutting')
    # StonecutterScreen's recipe grid: four columns of 16x18 cells from (52, 14).
    guest.raw('click-gui', 52 + index % 4 * 16 + 8, 14 + index // 4 * 18 + 9)
    guest.wait(10)
    picked = (guest.raw('menu')['info']['selected'], station_slots(guest).get(650))
    guest.shot(out, 'stonecutter-recipe')
    # A shift-click on the result cuts while there is stone (doClick repeats quickMoveStack).
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    seen = {'picked': picked, 'left': left,
            'server': {item: server_count(guest, item) for item in ('stone_slab', 'stone')}}
    want = {'picked': (index, ('minecraft:stone_slab', 2)), 'left': {},
            'server': {'stone_slab': 6, 'stone': 0}}
    assert seen == want, f'the stonecutter on the guest: {seen}'
    return seen


def server_reply(guest, command, *needles):
    """The first chat line holding every needle after the guest runs `command`."""
    since = guest.raw('events')['last']
    guest('command', command=command)
    for _ in range(0, 65):
        reply = guest.raw('events', '--since', since)
        for e in reply['events']:
            line = e.get('text', '')
            if own_reply(e) and all(n in line for n in needles):
                return line
        guest.wait(2)
    raise Failed(f'no answer to {command}')


def check_grindstone(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 0 levels')
    guest('command', command='xp set @s 0 points')
    guest('command', command='give @s minecraft:iron_sword[minecraft:enchantments={"minecraft:sharpness":3,'
                             '"minecraft:vanishing_curse":1},minecraft:damage=100]')
    at = open_station(guest, 'grindstone', out, 'grindstone')
    # GrindstoneMenu.quickMoveStack: with both inputs empty, into the first.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:iron_sword'), 'shift')
    guest.wait(10)
    loaded = station_slots(guest)
    guest.shot(out, 'grindstone-result')
    guest.raw('click-slot', 650, 'shift')
    # The orbs (sharpness III: 12 to 23 points) fly to the guest.
    guest.wait(60)
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}].components',
                         'minecraft:damage')
    # 16 points make exactly two levels and leave 0 points: count the levels too.
    xp = (server_reply(guest, 'xp query @s points', 'experience points')
          + server_reply(guest, 'xp query @s levels', 'experience levels'))
    seen = {'loaded': loaded, 'left': left, 'curse kept': 'vanishing_curse' in sword,
            'sharpness gone': 'sharpness' not in sword,
            'damage': re.search(r'minecraft:damage"?: 100\b', sword) is not None,
            'xp': any(int(n) > 0 for n in re.findall(r'has (\d+) experience', xp))}
    want = {'loaded': {600: ('minecraft:iron_sword', 1), 650: ('minecraft:iron_sword', 1)}, 'left': {},
            'curse kept': True, 'sharpness gone': True, 'damage': True, 'xp': True}
    assert seen == want, f'the grindstone on the guest: {seen} ({sword!r}, {xp!r})'
    return seen


def check_anvil(host, guest, out):
    # AnvilMenu.onTake emits one level event to everyone, including the user.
    # Exercise both the in-frame host and a remote user (both remote with --server-thread).
    return {actor.name: check_anvil_user(actor, (host, guest), out) for actor in (guest, host)}


def check_anvil_user(guest, listeners, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 5 levels')
    guest('command', command='give @s minecraft:iron_sword[minecraft:damage=200]')
    guest('command', command='give @s minecraft:iron_ingot 3')
    at = open_station(guest, 'anvil', out, 'anvil')
    # ItemCombinerMenu.quickMoveStack: into the first input that takes it.
    for item in ('iron_sword', 'iron_ingot'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    # The box shows "Iron Sword": erase it and type the new name.
    for _ in range(len('Iron Sword')):
        guest.raw('type-back')
    guest.raw('type', 'Blade')
    guest.wait(10)
    menu = guest.raw('menu')
    offered = (menu['info']['anvil_cost'], station_slots(guest).get(650))
    guest.shot(out, 'anvil-rename')
    sounds = ('minecraft:block.anvil.use', 'minecraft:block.anvil.destroy')
    before = {g.name: sum(heard(g, s) for s in sounds) for g in listeners}
    guest.raw('click-slot', 650, 'shift')
    guest.wait(20)
    sound_counts = {g.name: sum(heard(g, s) for s in sounds) - before[g.name] for g in listeners}
    assert sound_counts == {g.name: 1 for g in listeners}, f'anvil sounds: {sound_counts}'
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}].components',
                         'minecraft:damage')
    levels = server_reply(guest, 'xp query @s levels', 'experience levels')
    seen = {'offered': offered, 'left': left, 'name': 'Blade' in sword,
            'damage': re.search(r'minecraft:damage"?: 14\b', sword) is not None,
            'levels': int(re.search(r'has (\d+) experience', levels).group(1)),
            'ingots': server_count(guest, 'iron_ingot')}
    # Three ingots repair 62 each (a quarter of 250), plus the name: four levels.
    want = {'offered': (4, ('minecraft:iron_sword', 1)), 'left': {}, 'name': True, 'damage': True,
            'levels': 1, 'ingots': 0}
    assert seen == want, f'the anvil on the guest: {seen} ({sword!r})'
    return {**seen, 'heard': sound_counts}


def check_smithing(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    for item in ('netherite_upgrade_smithing_template', 'diamond_sword', 'netherite_ingot'):
        guest('command', command=f'give @s minecraft:{item}')
    at = open_station(guest, 'smithing_table', out, 'smithing')
    # SmithingMenu.canMoveIntoInputSlots: each to the slot whose recipe test takes it.
    for item in ('netherite_upgrade_smithing_template', 'diamond_sword', 'netherite_ingot'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(5)
    loaded = station_slots(guest)
    guest.shot(out, 'smithing-result')
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    seen = {'loaded': loaded, 'left': left,
            'server': {item: server_count(guest, item) for item in ('netherite_sword', 'diamond_sword', 'netherite_ingot')}}
    want = {'loaded': {600: ('minecraft:netherite_upgrade_smithing_template', 1), 601: ('minecraft:diamond_sword', 1),
                       602: ('minecraft:netherite_ingot', 1), 650: ('minecraft:netherite_sword', 1)},
            'left': {}, 'server': {'netherite_sword': 1, 'diamond_sword': 0, 'netherite_ingot': 0}}
    assert seen == want, f'the smithing table on the guest: {seen}'
    return seen


def check_enchanting(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='xp set @s 30 levels')
    guest('command', command='give @s minecraft:iron_sword')
    guest('command', command='give @s minecraft:lapis_lazuli 3')
    # EnchantmentMenu awards the enchant_item stat; a stat objective shows the guest's count.
    guest('command', command='scoreboard objectives add lanench minecraft.custom:minecraft.enchant_item')
    at = open_station(guest, 'enchanting_table', out, 'enchanting')
    # EnchantmentMenu.quickMoveStack: the sword to the item slot, the lapis to its own.
    for item in ('iron_sword', 'lapis_lazuli'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(10)
    costs = guest.raw('menu')['info']['costs']
    guest.shot(out, 'enchanting-offers')
    # EnchantmentScreen's first offer button (clickMenuButton 0).
    guest.raw('click-gui', 110, 23)
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    sword = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:iron_sword"}]', 'iron_sword')
    levels = server_reply(guest, 'xp query @s levels', 'experience levels')
    stat = server_reply(guest, 'scoreboard players get @s lanench', 'lanench')
    guest('command', command='scoreboard objectives remove lanench')
    seen = {'offered': costs[0] > 0, 'enchanted': 'minecraft:enchantments' in sword,
            'left': left.get(601), 'levels': int(re.search(r'has (\d+) experience', levels).group(1)),
            'lapis': server_count(guest, 'lapis_lazuli'), 'stat': ' has 1 ' in stat}
    want = {'offered': True, 'enchanted': True, 'left': ('minecraft:lapis_lazuli', 2), 'levels': 29, 'lapis': 2,
            'stat': True}
    assert seen == want, f'the enchanting table on the guest: {seen} ({costs}, {sword!r})'
    return seen


def check_beacon(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='effect clear @s')
    guest('command', command='give @s minecraft:iron_ingot')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    # A one-level pyramid under the beacon; its levels come with the first base check (80 ticks).
    guest('command', command=f'fill {x - 1} {y - 1} {z + 1} {x + 1} {y - 1} {z + 3} minecraft:iron_block')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:beacon')
    guest.wait(100)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'station', '--timeout', 10)
    guest.wait(10)
    # BeaconMenu.quickMoveStack: the ingot to the payment slot.
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:iron_ingot'), 'shift')
    guest.wait(10)
    paid = station_slots(guest).get(600)
    # BeaconScreen (54 px wider, 27 to the left): Haste, the first level's second power, then Done,
    # which sends ServerboundSetBeaconPacket and closes the menu.
    guest.raw('click-gui', 77 - 27 + 11, 22 + 11)
    guest.wait(5)
    guest.shot(out, 'beacon')
    guest.raw('click-gui', 164 - 27 + 11, 107 + 11)
    guest.wait(10)
    screen = guest('state').get('screen')
    effects = []
    # BeaconBlockEntity.applyEffects runs every 80 ticks.
    for _ in range(12):
        guest.wait(20)
        effects = effect_ids(guest)
        if 'haste' in effects:
            break
    seen = {'paid': paid, 'haste': 'haste' in effects, 'ingots': server_count(guest, 'iron_ingot'),
            'screen': screen}
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    want = {'paid': ('minecraft:iron_ingot', 1), 'haste': True, 'ingots': 0, 'screen': 'playing'}
    assert seen == want, f'the beacon on the guest: {seen} ({effects})'
    return seen


def check_crafter(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_planks 4')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:crafter')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    # CrafterScreen.slotClicked: an empty-handed click disables the first slot
    # (ServerboundContainerSlotStateChangedPacket); the planks then skip it.
    guest.raw('click-slot', 300)
    guest.wait(5)
    guest.raw('click-slot', hotbar_slot(guest, 'minecraft:oak_planks'), 'shift')
    guest.wait(10)
    guest.shot(out, 'crafter')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # Opening it again shows the server's crafter.
    items = open_container(guest, at, out, 'crafter-after')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'items': items[0] if isinstance(items, tuple) else items, 'planks': server_count(guest, 'oak_planks')}
    want = {'items': {1: 'minecraft:oak_planks'}, 'planks': 0}
    assert seen == want, f'the crafter on the guest: {seen}'
    return seen


def check_lectern(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    at = (x, y, z + 2)
    book = ('{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":'
            '{title:"Lan",author:"Host",pages:["one","two","three"]}}}')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:lectern[has_book=true]{{Book:{book}}}')
    guest.wait(10)
    guest.raw('interact', *at)
    guest.raw('wait-for', 'screen', 'book', '--timeout', 10)
    guest.wait(10)
    opened = guest.raw('book')
    guest.shot(out, 'lectern')
    # LecternScreen's page arrows: ContainerButtonClick 2 (forward) and 1 (back); the server's
    # LecternMenu.setData clamps to the book's last page and sends the page back (data slot 0).
    for button in ('next', 'next', 'next', 'back'):
        guest.raw('book', button)
        guest.wait(5)
    guest.wait(10)
    turned = guest.raw('book')
    page = server_reply(guest, f'data get block {at[0]} {at[1]} {at[2]} Page', 'block data')
    guest.shot(out, 'lectern-page')
    # Take Book: ContainerButtonClick 3; the server empties the lectern and closes the menu.
    guest.raw('book', 'take')
    guest.wait(20)
    screen = guest('state').get('screen')
    held = server_count(guest, 'written_book')
    has_book = server_reply(guest, f'execute if block {at[0]} {at[1]} {at[2]} minecraft:lectern[has_book=false]',
                            'Test passed')
    guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    seen = {'opened': (opened.get('page'), opened.get('pages')), 'turned': turned.get('page'),
            'server page': int(re.search(r'(-?\d+)\s*$', page).group(1)), 'screen': screen, 'held': held,
            'emptied': 'Test passed' in has_book}
    want = {'opened': (0, 3), 'turned': 1, 'server page': 1, 'screen': 'playing', 'held': 1, 'emptied': True}
    assert seen == want, f'the lectern on the guest: {seen} ({page!r})'
    return seen


def check_cartography(host, guest, out):
    # The host makes a map (EmptyMapItem.use), the guest gets a copy of it.
    host('command', command='clear @s')
    host('command', command='item replace entity @s weapon.mainhand with minecraft:map')
    host.wait(5)
    host.raw('use-item')
    host.wait(20)
    made = server_reply(host, 'data get entity @s Inventory[{id:"minecraft:filled_map"}].components', 'map_id')
    map_id = int(re.search(r'map_id"?: (\d+)', made).group(1))
    host('command', command='clear @s')
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command=f'give @s minecraft:filled_map[minecraft:map_id={map_id}]')
    guest('command', command='give @s minecraft:paper')
    at = open_station(guest, 'cartography_table', out, 'cartography')
    # CartographyTableMenu.quickMoveStack: a map with an id to the map slot, paper to the other.
    for item in ('filled_map', 'paper'):
        guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{item}'), 'shift')
        guest.wait(5)
    guest.wait(10)
    loaded = station_slots(guest)
    guest.shot(out, 'cartography-result')
    # The result's onTake uses the map and the paper; MapItem.onCraftedPostProcess gives the zoomed
    # out map a new id on the server.
    guest.raw('click-slot', 650, 'shift')
    guest.wait(10)
    left = station_slots(guest)
    close_station(guest, at)
    taken = server_reply(guest, 'data get entity @s Inventory[{id:"minecraft:filled_map"}].components', 'map_id')
    new_id = int(re.search(r'map_id"?: (\d+)', taken).group(1))
    seen = {'loaded': loaded, 'left': left, 'new map': new_id != map_id,
            'server': {item: server_count(guest, item) for item in ('filled_map', 'paper')}}
    want = {'loaded': {600: ('minecraft:filled_map', 1), 601: ('minecraft:paper', 1),
                       650: ('minecraft:filled_map', 1)},
            'left': {}, 'new map': True, 'server': {'filled_map': 1, 'paper': 0}}
    assert seen == want, f'the cartography table on the guest: {seen} ({made!r}, {taken!r})'
    return seen


def slot_point(game, slot):
    """The GUI point in the middle of slot `slot` of the open screen (`ui`'s slot-N)."""
    for w in game.raw('ui')['widgets']:
        if w.get('id') == f'slot-{slot}':
            x, y, width, height = w['bounds']
            return x + width / 2, y + height / 2
    raise Failed(f'no slot {slot} on the screen')


def mouse(game, *slots, right=False):
    """Press over the first slot, move over the others and release: one click, or a drag."""
    points = [f'{v:.1f}' for s in slots for v in slot_point(game, s)]
    game.raw('drag-at', *(['right'] if right else []), *points)
    game.wait(4)


def hotbar_slot(game, item):
    return next(s['slot'] for s in game('inventory')['hotbar'] if s and s.get('item') == item)


def check_clicks(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:cobblestone 10')
    guest('command', command='give @s minecraft:coal 3')
    guest('command', command='give @s minecraft:raw_iron 2')
    guest('command', command='give @s minecraft:oak_planks 2')
    guest.wait(10)
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest, furnace = (x, y, z + 2), (x, y, z - 2)
    guest('command', command=f'setblock {chest[0]} {chest[1]} {chest[2]} minecraft:chest')
    guest('command', command=f'setblock {furnace[0]} {furnace[1]} {furnace[2]} minecraft:furnace')
    guest.wait(10)
    # Pick the cobblestone up and put it down in the chest's first slot.
    guest.raw('interact', *chest)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    mouse(guest, hotbar_slot(guest, 'minecraft:cobblestone'))
    mouse(guest, 300)
    guest.wait(10)
    guest.shot(out, 'clicks-chest')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    in_chest, _ = open_container(guest, chest, out, 'clicks-chest-after')
    # Coal into the fuel slot, raw iron into the input.
    guest.raw('interact', *furnace)
    guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
    guest.wait(10)
    mouse(guest, hotbar_slot(guest, 'minecraft:coal'))
    mouse(guest, 301)
    mouse(guest, hotbar_slot(guest, 'minecraft:raw_iron'))
    mouse(guest, 300)
    guest.wait(10)
    guest.shot(out, 'clicks-furnace')
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # Opening the furnace again resends the whole menu: the server's own view.
    in_furnace, _ = open_container(guest, furnace, out, 'clicks-furnace-after')
    # Two planks dragged over the 2x2 grid's left column: one each, four sticks.
    guest.raw('key', 'e', 'tap')
    guest.raw('wait-for', 'screen', 'inventory')
    guest.wait(5)
    mouse(guest, hotbar_slot(guest, 'minecraft:oak_planks'))
    mouse(guest, 100, 102)
    guest.wait(10)
    grid = {s['slot']: (s.get('item'), s.get('count')) for s in guest('screen').get('slots', [])
            if s.get('slot') in (100, 102, 200) and s.get('item')}
    guest.shot(out, 'clicks-drag')
    guest.raw('click-slot', 200, 'shift')
    guest.wait(10)
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    # The chest's menu brings the whole inventory as the server has it.
    open_container(guest, chest, out, 'clicks-inventory')
    counts = guest('inventory')['counts']
    for at in (chest, furnace):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    assert in_chest == {0: 'minecraft:cobblestone'}, f'the chest holds {in_chest} after the guest put cobblestone in'
    assert in_furnace == {0: 'minecraft:raw_iron', 1: 'minecraft:coal'}, f'the furnace holds {in_furnace}'
    assert grid.get(100) == ('minecraft:oak_planks', 1) and grid.get(102) == ('minecraft:oak_planks', 1), \
        f'the drag left the grid as {grid}'
    assert counts.get('minecraft:stick') == 4 and 'minecraft:cobblestone' not in counts, f'inventory after: {counts}'
    return {'chest': in_chest, 'furnace': in_furnace, 'grid': grid, 'inventory': counts}


def check_feedback(host, guest, out):
    x, y, z = (round(v, 1) for v in pos_of(guest))
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:stick')
    guest('command', command=f'tp @s {x:.1f} {y:.1f} {z:.1f}')
    guest.wait(10)
    chat = guest.raw('messages', 5)['chat']
    # The host is an operator: broadcastToAdmins tells it, as `[Guest: ...]`.
    host_chat = host.raw('messages', 5)['chat']
    guest.raw('key', 't', 'tap')
    guest.wait(5)
    guest.shot(out, 'feedback')
    guest.raw('key', 'esc', 'tap')
    guest.wait(5)
    screen = guest('state').get('screen')
    replaced = f'Replaced 1 slot(s) on {GUEST_NAME} with [Stick]'
    assert any(replaced in line for line in chat), f'no "{replaced}" in {chat}'
    assert any(line.startswith(f'Teleported {GUEST_NAME} to {x:.6f}') for line in chat), f'teleport feedback: {chat}'
    assert screen in (None, 'playing'), f'the chat did not close: {screen}'
    told = f'[{GUEST_NAME}: Teleported {GUEST_NAME} to {x:.6f}'
    assert any(line.startswith(told) for line in host_chat), f'no "{told}" for the host: {host_chat}'
    return chat + host_chat


def chat_input(game):
    return game('screen').get('chat_input')


def complete(guest, text):
    """Opens the chat on `/`, types `text` and presses Tab: the input after it."""
    guest.raw('key', 'slash', 'tap')
    guest.until(lambda: guest('screen').get('screen'), lambda s: s == 'chat', ticks=3)
    guest.raw('type', text)
    guest.until(lambda: guest('screen').get('chat_suggestions'), bool, ticks=2)
    guest.raw('key', 'tab', 'tap')
    return guest.until(lambda: chat_input(guest), lambda value: value != '/' + text, ticks=2)


def check_commands(host, guest, out):
    literal = complete(guest, 'gamem')
    # The first Escape hides the suggestions, the next closes the chat.
    for _ in range(3):
        if guest('screen').get('screen') != 'chat':
            break
        guest.raw('key', 'esc', 'tap')
        guest.wait(3)
    argument = complete(guest, 'gamemode cr')
    guest.shot(out, 'commands')
    guest.raw('key', 'enter', 'tap')
    guest.wait(20)
    mode = guest('state').get('gamemode')
    screen = guest('state').get('screen')
    guest('command', command='gamemode survival')
    guest.wait(10)
    # A node the tree marks ask_server (a bossbar id): the guest asks the
    # server, which completes it from its own bossbars.
    host('command', command='bossbar add pt6:check "Check"')
    host.wait(5)
    guest.raw('key', 'slash', 'tap')
    guest.wait(3)
    guest.raw('type', 'bossbar get pt')
    guest.wait(10)
    guest.raw('key', 'tab', 'tap')
    guest.wait(2)
    asked = chat_input(guest)
    for _ in range(3):
        if guest('screen').get('screen') != 'chat':
            break
        guest.raw('key', 'esc', 'tap')
        guest.wait(3)
    host('command', command='bossbar remove pt6:check')
    assert literal == '/gamemode', f'Tab after /gamem: {literal!r}'
    assert argument == '/gamemode creative', f'Tab after /gamemode cr: {argument!r}'
    assert mode == 'creative', f'the completed command did not run: {mode}'
    assert screen in (None, 'playing'), f'the chat did not close: {screen}'
    assert asked == '/bossbar get pt6:check', f'Tab after /bossbar get pt: {asked!r}'
    return {'literal': literal, 'argument': argument, 'mode': mode, 'ask_server': asked}


def centre(rect):
    x, y, w, h = rect
    return x + w / 2, y + h / 2


def grid_items(game):
    slots = game('screen').get('slots') or []
    return {s['slot']: s.get('item') for s in slots if 100 <= s['slot'] < 109 and s.get('item')}


def check_recipes(host, guest, out):
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:oak_log 4')
    guest.wait(40)
    guest.raw('key', 'e', 'tap')
    guest.wait(5)
    try:
        book = guest('screen').get('recipe_book')
        assert book, 'the guest\'s inventory has no recipe book'
        if len(book['targets']) == 1:
            guest.raw('click-at', *centre(book['targets'][0]))
            guest.wait(5)
            book = guest('screen').get('recipe_book')
        known = book['known']
        assert known > 0, f'the guest\'s book is empty: {book}'
        guest.raw('click-at', *centre(book['targets'][1]))
        guest.wait(10)
        grid = grid_items(guest)
        guest.shot(out, 'recipes')
    finally:
        guest.raw('key', 'esc', 'tap')
        guest.wait(5)
    assert any('oak_log' in str(item) for item in grid.values()), f'nothing placed in the grid: {grid}'
    # #2019: the alternatives popup belongs to one screen. Check both the
    # in-frame host (or --server-thread host) and the remote guest, with both
    # inventory-close keys and a different recipe-book type on reopening.
    for game in (host, guest):
        game('command', command='recipe give @s minecraft:oak_planks')
        game('command', command='recipe give @s minecraft:birch_planks')
        game.wait(10)
        for close_key, reopen in (('e', 'inventory'), ('esc', 'furnace')):
            game.raw('key', 'e', 'tap')
            game.wait(5)
            book = game('screen')['recipe_book']
            if len(book['targets']) == 1:
                game.raw('click-at', *centre(book['targets'][0]))
                game.wait(5)
                book = game('screen')['recipe_book']
            for rect in book['targets']:
                if rect[2:] != [25.0, 25.0]:
                    continue
                game.raw('click-at', *centre(rect), 'right')
                game.wait(2)
                targets = game('screen')['recipe_book']['targets']
                if targets and all(r[2:] == [24.0, 24.0] for r in targets):
                    break
            else:
                raise AssertionError(f'{game.name}: no alternatives popup opened: {book}')
            game.shot(out, f'recipe-overlay-{close_key}-before')
            game.raw('key', close_key, 'tap')
            game.wait(5)
            if reopen == 'inventory':
                game.raw('key', 'e', 'tap')
            else:
                x, y, z = (int(v // 1) for v in pos_of(game))
                game('command', command=f'setblock {x} {y} {z - 2} minecraft:furnace')
                game.wait(5)
                game.raw('interact', x, y, z - 2)
            game.wait(5)
            targets = game('screen')['recipe_book']['targets']
            if reopen == 'furnace' and len(targets) == 1 and targets[0][2:] == [20.0, 18.0]:
                game.raw('click-at', *centre(targets[0]))
                game.wait(5)
                targets = game('screen')['recipe_book']['targets']
            game.shot(out, f'recipe-overlay-{close_key}-after')
            assert any(r[2:] == [20.0, 18.0] for r in targets), f'{game.name}: stale popup: {targets}'
            if reopen == 'inventory':
                assert len(targets) > 1, f'{game.name}: the book forgot its open setting'
            game.raw('key', 'esc', 'tap')
            game.wait(5)
    return {'known': known, 'grid': grid}


def check_titles(host, guest, out):
    # TitleCommand sends the guest its title packets; its HUD shows and clears them.
    for command in ('title @s times 10 400 10', 'title @s subtitle "Under"', 'title @s title "Over"',
                    'title @s actionbar "Bar"'):
        guest('command', command=command)
    guest.wait(5)
    shown = {k: guest.raw('messages', 1)[k] for k in ('title', 'subtitle', 'actionbar')}
    guest.shot(out, 'titles')
    guest('command', command='title @s clear')
    guest.wait(5)
    cleared = {k: guest.raw('messages', 1)[k] for k in ('title', 'subtitle')}
    assert shown == {'title': 'Over', 'subtitle': 'Under', 'actionbar': 'Bar'}, f'titles shown: {shown}'
    assert cleared == {'title': None, 'subtitle': None}, f'titles cleared: {cleared}'
    return {'shown': shown, 'cleared': cleared}


def check_chat(host, guest, out):
    # The agent's event log follows a remote client's chat: `wait-for chat` sees the line another
    # player sends (and on the server thread the host window's own line, back from the server).
    line = f'chat check {int(time.time())}'
    host.raw('chat', line)
    seen = {g.name: g.raw('wait-for', 'chat', line, '--timeout', 10, '--since', 0).get('ok')
            for g in ((guest, host) if SERVER_THREAD else (guest,))}
    guest.shot(out, 'chat')
    return seen


def styled_parts(component):
    """Every component in a chat line's tree (the line, its `extra` and `with` parts)."""
    if isinstance(component, dict):
        yield component
        for key in ('extra', 'with'):
            for part in component.get(key) or []:
                yield from styled_parts(part)
    elif isinstance(component, list):
        for part in component:
            yield from styled_parts(part)


def check_styles(host, guest, out):
    # ChatComponent keeps what the server sent: the style, the click and the hover event.
    stamp = f'STYLE {int(time.time())}'
    line = {'text': stamp, 'color': 'green', 'bold': True,
            'click_event': {'action': 'suggest_command', 'command': '/say clicked'},
            'hover_event': {'action': 'show_text', 'value': 'Tip'}}
    seen = {}
    for g in (host, guest):
        # Each to itself: tellraw reaches only the player whose command it is here.
        since = g.raw('events')['last']
        g('command', command='tellraw @s ' + json.dumps(line))
        g.raw('wait-for', 'chat', stamp, '--timeout', 10, '--since', since)
        part = next((p for c in g.raw('messages', 10)['styled'] for p in styled_parts(c)
                     if stamp in str(p.get('text', ''))), None) or {}
        seen[g.name] = {k: part.get(k) for k in ('color', 'bold', 'click_event', 'hover_event')}
        hover = seen[g.name]['hover_event'] or {}
        seen[g.name]['hover_event'] = [hover.get('action'), 'Tip' in str(hover.get('value'))]
    # InventoryChangeTrigger fires inside /give, before its message; the operator echo
    # (broadcastToAdmins) skips the player whose command it was.
    order = {}
    for g in (host, guest):
        for command in ('advancement revoke @s everything', 'clear @s'):
            g('command', command=command)
        g.wait(5)
        since = g.raw('events')['last']
        g('command', command='give @s minecraft:diamond')
        g.raw('wait-for', 'chat', 'Diamonds!', '--timeout', 10, '--since', since)
        g.wait(20)
        chat = g.raw('messages', 8)['chat']
        name = GUEST_NAME if g is guest else host.player
        gave = next((i for i, l in enumerate(chat) if l.startswith('Gave 1 [Diamond]')), None)
        made = next((i for i, l in enumerate(chat) if '[Diamonds!]' in l), None)
        echo = [l for l in chat if l.startswith(f'[{name}: ')]
        # Newest first: the give's message is newer than the advancement's.
        order[g.name] = {'gave': gave, 'advancement': made, 'echo': echo}
    guest.raw('key', 't', 'tap')
    guest.wait(5)
    guest.shot(out, 'styles')
    guest.raw('key', 'esc', 'tap')
    want = {'color': 'green', 'bold': True,
            'click_event': {'action': 'suggest_command', 'command': '/say clicked'},
            'hover_event': ['show_text', True]}
    for name, got in seen.items():
        assert got == want, f'{name} styled line: {got}'
    for name, o in order.items():
        assert o['gave'] is not None and o['advancement'] is not None, f'{name} chat: {o}'
        assert o['gave'] < o['advancement'], f'{name}: the give came before the advancement: {o}'
        assert not o['echo'], f'{name} heard its own command: {o}'
    return {'styles': seen, 'order': order}


def png_rows(path):
    """The rows of an 8-bit RGB or RGBA PNG, unfiltered, as bytes of 4 (or 3) per pixel."""
    data = Path(path).read_bytes()
    pos, idat, head = 8, b'', None
    while pos < len(data):
        size = int.from_bytes(data[pos:pos + 4], 'big')
        kind, body = data[pos + 4:pos + 8], data[pos + 8:pos + 8 + size]
        if kind == b'IHDR':
            head = body
        elif kind == b'IDAT':
            idat += body
        pos += 12 + size
    width, height = int.from_bytes(head[:4], 'big'), int.from_bytes(head[4:8], 'big')
    assert head[8] == 8 and head[9] in (2, 6) and head[12] == 0, f'unsupported PNG {head.hex()}'
    bpp = 4 if head[9] == 6 else 3
    raw, stride, rows, prev = zlib.decompress(idat), width * bpp, [], bytearray(width * bpp)
    for y in range(height):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif kind == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(bytes(line))
        prev = line
    return rows, bpp


def changed_pixels(a, b):
    (ra, bpp), (rb, _) = png_rows(a), png_rows(b)
    return sum(1 for la, lb in zip(ra, rb) for i in range(0, len(la), bpp)
               if max(abs(la[i + k] - lb[i + k]) for k in range(3)) > 24)


def sneaking_on_host(host, want, ticks=40):
    """Whether the host sees the guest crouching, once it matches `want` or after `ticks`."""
    for _ in range(ticks // 5):
        sneaking = lan_guest(host)['sneaking']
        if sneaking == want:
            return sneaking
        host.wait(5)
    return sneaking


def check_walls(host, guest, out):
    # A stone wall between the two: the host still sees the guest's name through it
    # (TEXT_SEE_THROUGH), but not once the guest crouches (Entity.isDiscrete).
    for g in (host, guest):
        back_to_game(g)
    # The trial spawner's zombies may have killed the guest.
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    face_each_other(host, guest)
    hx, hy, hz = pos_of(host)
    gx, gy, gz = pos_of(guest)
    # Halfway: the players stand 3 apart, so the wall's block touches neither.
    wall = int((hx + gx) / 2 // 1)
    host('command', command=f'fill {wall} {int(hy // 1)} {int(hz // 1) - 3} {wall} {int(hy // 1) + 4} '
                            f'{int(hz // 1) + 3} minecraft:stone')
    try:
        host.raw('look-at', f'{gx:.2f}', f'{gy + 2.3:.2f}', f'{gz:.2f}')
        # Chat lines fade out after 200 ticks; let them go so only the name tag can differ.
        host.wait(210)
        standing = host.shot(out, 'walls-standing')
        guest.raw('key', 'shift', 'down')
        crouched = sneaking_on_host(host, True)
        guest.wait(10)
        crouching = host.shot(out, 'walls-crouching')
        guest.raw('key', 'shift', 'up')
        stood = not sneaking_on_host(host, False)
        guest.wait(10)
        again = host.shot(out, 'walls-standing-again')
    finally:
        guest.raw('key', 'shift', 'up')
        host('command', command=f'fill {wall} {int(hy // 1)} {int(hz // 1) - 3} {wall} {int(hy // 1) + 4} '
                                f'{int(hz // 1) + 3} minecraft:air')
        face_each_other(host, guest)
    assert crouched and stood, f'the host never saw the guest crouch and stand: {crouched}, {stood}'
    seen, steady = changed_pixels(standing, crouching), changed_pixels(standing, again)
    assert seen > 50, f'no name tag through the wall: {seen} pixels differ standing vs crouching'
    assert steady < seen // 4, f'the view itself changed: {steady} pixels between two standing shots ({seen})'
    return {'standing_vs_crouching': seen, 'standing_twice': steady}


def state_of(game, pos, key):
    state = game.raw('block', *pos)['state']
    return dict(kv.split('=') for kv in state.partition('[')[2].rstrip(']').split(',') if kv).get(key)


def wait_state(game, pos, key, want, ticks):
    for _ in range(ticks // 10):
        if state_of(game, pos, key) == want:
            return want
        game.wait(10)
    return state_of(game, pos, key)


def check_piston_push(host, guest, out):
    """A moving piston pushes both players and a stationary mob on each host path."""
    hx, _, hz = pos_of(host)
    x, y, z = (int(hx // 16) * 16 + 8, 100, int(hz // 16) * 16 + 8)
    host('command', command=f'fill {x-4} {y-1} {z-4} {x+7} {y-1} {z+4} minecraft:stone')
    host('command', command=f'fill {x-4} {y} {z-4} {x+7} {y+5} {z+4} minecraft:air')
    host('command', command=f'setblock {x} {y} {z} minecraft:piston[facing=east]')
    results = {}
    for player, name in ((host, host.player), (guest, GUEST_NAME)):
        host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
        host.wait(10)
        host('command', command=f'tp @s {x+5.5} {y} {z+2.5}')
        host('command', command=f'tp {GUEST_NAME} {x+5.5} {y} {z-1.5}')
        host('command', command=f'tp {"@s" if player is host else GUEST_NAME} {x+1.65} {y} {z+0.5}')
        player.wait(15)
        before = pos_of(player)
        assert abs(before[0] - (x + 1.65)) < 0.1, f'{name} did not reach piston: {before}'
        host('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
        player.wait(30)
        after = pos_of(player)
        results[name] = after[0] - before[0]
        assert results[name] > 0.3, f'piston did not push {name}: {before} -> {after}'
    host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
    host.wait(10)
    host('command', command=f'tp {GUEST_NAME} {x+5.5} {y} {z-1.5}')
    host('command', command=f'tp @s {x+1.65} {y} {z+0.5}')
    host.wait(10)
    host.raw('key', 'e', 'tap')
    host.raw('wait-for', 'screen', 'inventory')
    before = pos_of(host)
    guest('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
    host.wait(30)
    after = pos_of(host)
    results['inventory'] = after[0] - before[0]
    assert results['inventory'] > 0.3, f'piston did not push player with inventory open: {before} -> {after}'
    host.raw('key', 'esc', 'tap')
    host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
    host.wait(10)
    host('command', command=f'tp @s {x+5.5} {y} {z+2.5}')
    host('command', command=f'summon minecraft:pig {x+1.65} {y} {z+0.5} {{NoAI:1b}}')
    host.wait(10)
    pigs = host('entities', radius=16, type='pig')['entities']
    pig = min(pigs, key=lambda p: abs(p['pos'][0] - (x + 1.65)) + abs(p['pos'][2] - (z + 0.5)))
    before = pig['pos'][0]
    host('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
    host.wait(30)
    pigs = host('entities', radius=16, type='pig')['entities']
    after = next(p['pos'][0] for p in pigs if p['id'] == pig['id'])
    results['pig'] = after - before
    assert results['pig'] > 0.3, f'piston did not push pig: {before} -> {after}'
    results.update(piston_push_at_border(host, guest))
    return results


def piston_push_at_border(host, guest):
    """A piston next to the world border: the border's wall stops the pushed
    body, as it stops ordinary movement (`Entity.collectCollidersIgnoringWorldBorder`)."""
    hx, _, hz = pos_of(host)
    x, y, z = (int(hx // 16) * 16 + 8, 100, int(hz // 16) * 16 + 8)
    host('command', command=f'fill {x-4} {y-1} {z-4} {x+7} {y-1} {z+4} minecraft:stone')
    host('command', command=f'fill {x-4} {y} {z-4} {x+7} {y+5} {z+4} minecraft:air')
    host('command', command=f'setblock {x} {y} {z} minecraft:piston[facing=east]')
    # The border's east wall stands at `x + 2`, the east face of the block the
    # head extends into: a body pushed east stops there instead of being
    # pushed out through the border. `worldborder center` centres integers on
    # the block, so the centre is `x - 8.5` and the wall lands on `x + 2`.
    wall = x + 2
    center = [x - 8.5, z + 0.5]
    results = {}
    try:
        host('command', command=f'worldborder center {x - 9} {z}')
        host('command', command='worldborder set 20.5')
        seen = None
        for _ in range(10):
            host.wait(2)
            seen = [g('status').get('border') for g in (host, guest)]
            if all(b and b['size'] == 20.5 and b['center'] == center for b in seen):
                break
        results['border'] = seen
        for player, name in ((host, host.player), (guest, GUEST_NAME)):
            host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
            host.wait(10)
            host('command', command=f'tp @s {x+5.5} {y} {z+2.5}')
            host('command', command=f'tp {GUEST_NAME} {x+5.5} {y} {z-1.5}')
            host('command', command=f'tp {"@s" if player is host else GUEST_NAME} {x+1.65} {y} {z+0.5}')
            player.wait(15)
            before = pos_of(player)
            assert abs(before[0] - (x + 1.65)) < 0.1, f'{name} did not reach the piston: {before}'
            host('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
            player.wait(30)
            after = pos_of(player)
            # The wall's inner face is at `wall`: the body's box stops there.
            results[name] = round(after[0] - before[0], 3)
            assert after[0] > before[0], f'the piston did not push {name}: {before} -> {after}'
            assert after[0] <= wall - 0.3 + 0.01, f'the border did not stop {name}: {before} -> {after}'
        host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
        host.wait(10)
    finally:
        host('command', command='worldborder center 0 0')
        host('command', command='worldborder set 59999968')
    return results


def check_piston_slime(host, guest, out):
    """A piston pushing a slime block: the block pushes the body in front of
    it and bounces it (`PistonMovingBlockEntity.moveCollidedEntities`)."""
    hx, _, hz = pos_of(host)
    x, y, z = (int(hx // 16) * 16 + 8, 100, int(hz // 16) * 16 + 8)
    results = {}

    def setup():
        # The previous case may have left a mob where the body stands.
        host('command', command='kill @e[type=minecraft:pig]')
        host('command', command=f'setblock {x} {y} {z} minecraft:air')
        host.wait(10)
        host('command', command=f'fill {x-4} {y-1} {z-4} {x+7} {y-1} {z+4} minecraft:stone')
        host('command', command=f'fill {x-4} {y} {z-4} {x+7} {y+5} {z+4} minecraft:air')
        host('command', command=f'setblock {x} {y} {z} minecraft:piston[facing=east]')
        host.wait(5)

    for player, name in ((host, host.player), (guest, GUEST_NAME)):
        setup()
        host('command', command=f'setblock {x+1} {y} {z} minecraft:slime_block')
        host('command', command=f'tp @s {x+5.5} {y} {z+2.5}')
        host('command', command=f'tp {GUEST_NAME} {x+5.5} {y} {z-1.5}')
        host('command', command=f'tp {"@s" if player is host else GUEST_NAME} {x+2.5} {y} {z+0.5}')
        player.wait(15)
        before = pos_of(player)
        assert abs(before[0] - (x + 2.5)) < 0.1, f'{name} did not reach the slime: {before}'
        host('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
        # The bounce sets the velocity along the push axis; it decays fast, so
        # sample it while the block is still moving.
        speed = 0.0
        for _ in range(6):
            player.wait(2)
            speed = max(speed, player('state')['player']['velocity'][0])
        after = pos_of(player)
        results[name] = round(after[0] - before[0], 3)
        # The block pushes the body out of its way, and the bounce launches it.
        assert results[name] > 1.5, f'the slime block did not push and bounce {name}: {before} -> {after}'
        assert speed > 0.05, f'the slime block did not bounce {name}: {speed}'
        host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
        player.wait(10)
    # A plain block only pushes: the body is not launched.
    setup()
    host('command', command=f'setblock {x+1} {y} {z} minecraft:stone')
    host('command', command=f'tp @s {x+2.5} {y} {z+0.5}')
    host.wait(15)
    before = pos_of(host)
    host('command', command=f'setblock {x-1} {y} {z} minecraft:redstone_block')
    speed = 0.0
    for _ in range(6):
        host.wait(2)
        speed = max(speed, host('state')['player']['velocity'][0])
    after = pos_of(host)
    results['stone'] = round(after[0] - before[0], 3)
    assert results['stone'] < 1.5, f'a plain block bounced the player: {before} -> {after}'
    host('command', command=f'setblock {x-1} {y} {z} minecraft:air')
    host.wait(10)
    return results


def check_plates(host, guest, out):
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    plate = (x + 2, y, z)
    guest('command', command=f'setblock {plate[0]} {plate[1]} {plate[2]} minecraft:stone_pressure_plate')
    guest.wait(10)
    guest('command', command=f'tp @s {plate[0] + 0.5} {plate[1]} {plate[2] + 0.5}')
    pressed = {'host': wait_state(host, plate, 'powered', 'true', 40),
               'guest': wait_state(guest, plate, 'powered', 'true', 40)}
    guest.shot(out, 'plate-pressed')
    guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    # PressurePlateBlock.getPressedTime: a stone plate checks again every 20 ticks.
    released = {'host': wait_state(host, plate, 'powered', 'false', 60),
                'guest': wait_state(guest, plate, 'powered', 'false', 60)}
    guest('command', command=f'setblock {plate[0]} {plate[1]} {plate[2]} minecraft:air')
    detail = {'pressed': pressed, 'released': released}
    assert pressed == {'host': 'true', 'guest': 'true'}, f'the plate under the guest: {detail}'
    assert released == {'host': 'false', 'guest': 'false'}, f'the plate after the guest left: {detail}'
    return detail


def check_trials(host, guest, out):
    # The host watches as a spectator, whom neither block detects: only the guest can wake them.
    hx, hy, hz = pos_of(host)
    host('command', command='gamemode spectator')
    host('command', command='difficulty normal')
    host('command', command='gamerule spawn_mobs true')
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    spawner, vault = (x, y, z + 3), (x, y, z - 2)
    # Open air from the vault to the spawner: the spawner wants to see the guest's eyes.
    guest('command', command=f'fill {x - 1} {y} {z - 2} {x + 1} {y + 2} {z + 3} minecraft:air')
    try:
        guest('command', command=f'setblock {spawner[0]} {spawner[1]} {spawner[2]} minecraft:trial_spawner'
                                 '{spawn_data:{entity:{id:"minecraft:zombie"}},required_player_range:6}')
        guest('command', command=f'setblock {vault[0]} {vault[1]} {vault[2]} minecraft:vault'
                                 '{config:{key_item:{id:"minecraft:trial_key",count:1}}}')
        trial = wait_state(host, spawner, 'trial_spawner_state', 'active', 100)
        active = wait_state(host, vault, 'vault_state', 'active', 60)
        host.shot(out, 'trials-active')
        guest('command', command='item replace entity @s weapon.mainhand with minecraft:trial_key')
        guest.wait(5)
        guest.raw('interact', *vault)
        unlocking = wait_state(host, vault, 'vault_state', 'unlocking', 20)
        guest.shot(out, 'trials-unlocking')
        rewarded = host('command', command=f'data get block {vault[0]} {vault[1]} {vault[2]} server_data.rewarded_players')
        host_uuid = host('command', command='data get entity @s UUID')
    finally:
        host('command', command='gamerule spawn_mobs false')
        host('command', command='kill @e[type=minecraft:zombie]')
        host('command', command='gamemode survival')
        # Back where it stood: a spectator drifts, and may be inside the ground when it lands.
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
        guest('command', command=f'setblock {spawner[0]} {spawner[1]} {spawner[2]} minecraft:air')
        guest('command', command=f'setblock {vault[0]} {vault[1]} {vault[2]} minecraft:air')
    uuid = lambda reply: re.findall(r'-?\d+', json.dumps(reply).partition('[I;')[2].partition(']')[0])
    assert trial == 'active', f'the trial spawner never saw the guest: {trial}'
    assert active == 'active', f'the vault never saw the guest: {active}'
    assert unlocking == 'unlocking', f'the guest\'s key did not unlock the vault: {unlocking}'
    assert uuid(rewarded) and uuid(rewarded) != uuid(host_uuid), f'rewarded {rewarded}, host {host_uuid}'
    return {'spawner': trial, 'vault': active, 'rewarded': uuid(rewarded), 'host': uuid(host_uuid)}


def bed_at(game, foot):
    """A bed whose foot is at `foot`, its head one block east."""
    x, y, z = foot
    game('command', command=f'setblock {x} {y} {z} minecraft:red_bed[facing=east,part=foot]')
    game('command', command=f'setblock {x + 1} {y} {z} minecraft:red_bed[facing=east,part=head]')
    return [x + 1, y, z]


def check_sleep(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
    host('command', command='time set midnight')
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    guest_bed = bed_at(guest, (gx, gy, gz + 2))
    host_bed = bed_at(host, (hx, hy, hz + 2))
    guest.wait(10)
    guest.raw('interact', *guest_bed)
    guest.wait(10)
    asleep = {'guest': guest('state').get('sleeping'), 'host': host('state').get('sleeping'),
              'server': lan_guest(host).get('sleeping')}
    guest.shot(out, 'sleep-guest')
    # The host draws the guest lying in its bed (#1045): in frame its model, on the server thread its
    # client's (SLEEPING_POS), and a client pairing with the sleeper later too.
    host.wait(5)
    lying = {'host': lan_guest(host).get('model_lying') if not SERVER_THREAD else bool(remote(host)['lying_players'])}
    host.shot(out, 'sleep-host-sees-guest')
    # Alone in bed: still night 120 ticks later (everyone must sleep).
    guest.wait(120)
    night = host('state')['time']['time_of_day']
    # Escape gets the guest up (InBedChatScreen), then back to bed.
    guest.raw('key', 'esc', 'tap')
    guest.wait(10)
    up = guest('state').get('sleeping')
    guest.raw('interact', *guest_bed)
    host.raw('interact', *host_bed)
    host.wait(5)
    # Both in bed: the guest draws the host lying too.
    lying['guest'] = bool(remote(guest)['lying_players'])
    host.wait(125)
    after = {'guest': guest('state').get('sleeping'), 'host': host('state').get('sleeping'),
             'time': host('state')['time']['time_of_day']}
    lying['after'] = bool(remote(guest)['lying_players'])
    guest.shot(out, 'sleep-morning')
    for at in (guest_bed, host_bed):
        guest('command', command=f'fill {at[0] - 1} {at[1]} {at[2]} {at[0]} {at[1]} {at[2]} minecraft:air')
    host('command', command='time set day')
    detail = {'asleep': asleep, 'night': night, 'up': up, 'after': after, 'lying': lying}
    assert asleep['guest'] == guest_bed and asleep['server'] == guest_bed, f'the guest is not in its bed: {detail}'
    assert asleep['host'] is None, f'the guest\'s bed put the host to sleep: {detail}'
    assert 13000 < night < 23000, f'one of two players asleep ended the night: {detail}'
    assert up is None, f'Escape did not get the guest up: {detail}'
    assert after['guest'] is None and after['host'] is None and after['time'] < 1000, f'no morning: {detail}'
    assert lying['host'] and lying['guest'], f'a sleeping player is drawn standing: {detail}'
    assert not lying['after'], f'a player up again is still drawn lying: {detail}'
    return detail


def check_sleep_villager(host, guest, out):
    """AbstractBedBlock.kickVillagerOutOfBed: using a bed a villager sleeps in wakes the villager and
    leaves the player up; the next use lies down. The sleeper's own body is drawn in its bed, in first
    person too (LevelExtractor skips the camera entity unless it sleeps)."""
    guest('command', command='gamemode survival')
    host('command', command='time set midnight')
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, z = gx, gz + 3
    guest('command', command=f'fill {x - 3} {gy - 1} {z - 2} {x + 4} {gy - 1} {z + 3} minecraft:stone')
    guest('command', command=f'fill {x - 3} {gy} {z - 2} {x + 4} {gy + 3} {z + 3} minecraft:air')
    bed = bed_at(guest, (x, gy, z))
    guest('command', command=f'summon minecraft:villager {x - 1.5} {gy} {z + 1.5}')

    def villager():
        found = guest('entities', radius=12, type='villager')['entities']
        return found[0]['pos'] if found else None

    def in_bed(at):
        # Lying at the head (setPosToBed), not standing beside it.
        return at is not None and abs(at[0] - bed[0] - 0.5) < 0.5 and abs(at[2] - bed[2] - 0.5) < 0.5 \
            and at[1] - gy > 0.4
    seen = {}
    try:
        for _ in range(90):
            if in_bed(villager()):
                break
            guest.wait(20)
        seen['villager_asleep'] = villager()
        assert in_bed(seen['villager_asleep']), f'the villager never lay down in the bed: {seen}'
        guest.raw('interact', *bed)
        guest.wait(10)
        seen['villager_kicked'] = villager()
        seen['up'] = guest('state').get('sleeping')
        assert not in_bed(seen['villager_kicked']), f'the bed did not wake its villager: {seen}'
        assert seen['up'] is None, f'the player lay down beside the villager: {seen}'
        guest.raw('interact', *bed)
        guest.wait(10)
        seen['asleep'] = guest('state').get('sleeping')
        seen['own_body_asleep'] = remote(guest).get('own_body')
        guest.shot(out, 'sleep-villager')
        guest.raw('key', 'esc', 'tap')
        guest.wait(10)
        seen['own_body_up'] = remote(guest).get('own_body')
    finally:
        guest('command', command='kill @e[type=minecraft:villager,distance=..16]')
        guest('command', command=f'fill {bed[0] - 1} {bed[1]} {bed[2]} {bed[0]} {bed[1]} {bed[2]} minecraft:air')
        host('command', command='time set day')
    assert seen['asleep'] == bed, f'the emptied bed did not take the player: {seen}'
    assert seen['own_body_asleep'] and not seen['own_body_up'], f'the sleeper\'s own body: {seen}'
    return seen
def check_bed_occupied(host, guest, out):
    """BedBlock.OCCUPIED (LivingEntity.startSleeping / stopSleeping): the bed the host sleeps in is
    occupied, head and foot, and the guest cannot lie down in it (AbstractBedBlock.useWithoutItem's
    "This bed is occupied"); the bed is free again after each way out of it: Escape, death, the bed
    broken, and the sleeper leaving the game. Then the guest sleeps in it."""
    port = host.raw('lan', 'status')['port']
    for g in (host, guest):
        g('command', command='gamemode survival')
    host('command', command='time set midnight')
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, z = gx, gz + 3
    guest('command', command=f'fill {x - 3} {gy - 1} {z - 2} {x + 4} {gy - 1} {z + 3} minecraft:stone')
    guest('command', command=f'fill {x - 3} {gy} {z - 2} {x + 4} {gy + 3} {z + 3} minecraft:air')
    bed = bed_at(guest, (x, gy, z))
    foot = [bed[0] - 1, bed[1], bed[2]]
    hx, hy, hz = pos_of(host)
    beside = f'tp @s {x + 0.5} {gy} {z + 1.5}'

    def occupied():
        return [state_of(host, bed, 'occupied'), state_of(host, foot, 'occupied')]

    def lie_down(game):
        game('command', command=beside)
        game.wait(10)
        game.raw('interact', *bed)
        game.wait(10)
        return game('state').get('sleeping')
    seen = {}
    try:
        host.wait(5)
        seen['free'] = occupied()
        seen['host_asleep'] = lie_down(host)
        seen['occupied'] = occupied()
        guest('command', command=f'tp @s {x + 0.5} {gy} {z - 0.5}')
        guest.wait(10)
        # The foot: the sleeper's body lies over the head.
        guest.raw('interact', *foot)
        guest.wait(5)
        seen['refused'] = {'sleeping': guest('state').get('sleeping'), 'actionbar': guest.raw('messages', 1)['actionbar']}
        guest.shot(out, 'bed-occupied')
        host.raw('key', 'esc', 'tap')
        host.wait(10)
        seen['after_escape'] = occupied()
        seen['asleep_again'] = lie_down(host)
        host('command', command='kill @s')
        host.wait(20)
        respawn_when_ready(host)
        host.wait(20)
        seen['after_death'] = occupied()
        seen['asleep_before_break'] = lie_down(host)
        guest('command', command=f'setblock {foot[0]} {foot[1]} {foot[2]} minecraft:air')
        guest.wait(10)
        seen['up_after_break'] = host('state').get('sleeping')
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
        bed_at(guest, (x, gy, z))
        guest.wait(5)
        seen['replaced'] = occupied()
        seen['guest_asleep'] = lie_down(guest)
        seen['occupied_by_guest'] = occupied()
        guest.raw('disconnect')
        time.sleep(2)
        seen['after_leave'] = occupied()
        guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(20)
        seen['screen_back'] = guest('state').get('screen')
        back_to_game(guest)
        seen['guest_back'] = lie_down(guest)
        guest.raw('key', 'esc', 'tap')
        guest.wait(10)
    finally:
        back_to_game(host)
        back_to_game(guest)
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
        guest('command', command=f'fill {foot[0]} {foot[1]} {foot[2]} {bed[0]} {bed[1]} {bed[2]} minecraft:air')
        host('command', command='time set day')
    free, taken = ['false', 'false'], ['true', 'true']
    assert seen['free'] == free and seen['host_asleep'] == bed, f'the host did not lie down: {seen}'
    assert seen['occupied'] == taken, f'the bed the host sleeps in is not occupied: {seen}'
    assert seen['refused']['sleeping'] is None, f'two players share a bed: {seen}'
    assert 'occupied' in (seen['refused']['actionbar'] or ''), f'no "This bed is occupied": {seen}'
    for way in ('after_escape', 'after_death', 'replaced', 'after_leave'):
        assert seen[way] == free, f'the bed stays occupied {way}: {seen}'
    assert seen['asleep_again'] == bed and seen['asleep_before_break'] == bed, f'the host could not lie down again: {seen}'
    assert seen['up_after_break'] is None, f'the host sleeps on in a broken bed: {seen}'
    assert seen['guest_asleep'] == bed and seen['occupied_by_guest'] == taken, f'the guest\'s bed: {seen}'
    assert seen['guest_back'] == bed, f'the bed the guest left is not free for it: {seen}'
    return seen


def check_sleep_reload(host, guest, out):
    """LivingEntity's sleeping_pos is saved: a villager asleep in its bed when the host saves and quits
    is still asleep in it, its bed still occupied, when the world opens again; the bed the guest slept
    in is saved free (ServerPlayer.disconnect gets every player up before the last save)."""
    if not SERVER_THREAD:
        return {'skipped': 'the reopen is the server thread\'s'}
    guest('command', command='gamemode survival')
    host('command', command='time set midnight')
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, z = gx, gz + 3
    guest('command', command=f'fill {x - 3} {gy - 1} {z - 2} {x + 4} {gy - 1} {z + 6} minecraft:stone')
    guest('command', command=f'fill {x - 3} {gy} {z - 2} {x + 4} {gy + 3} {z + 6} minecraft:air')
    villager_bed = bed_at(guest, (x, gy, z))
    guest_bed = bed_at(guest, (x, gy, z + 4))
    guest('command', command=f'summon minecraft:villager {x - 1.5} {gy} {z + 1.5}')
    sleeping_pos = lambda: host('command', command='data get entity @e[type=minecraft:villager,limit=1,sort=nearest] '
                                                   'sleeping_pos')
    seen = {}
    try:
        for _ in range(90):
            seen['villager_asleep'] = sleeping_pos()
            if str(villager_bed[0]) in json.dumps(seen['villager_asleep']):
                break
            guest.wait(20)
        seen['villager_bed'] = state_of(guest, villager_bed, 'occupied')
        guest('command', command=f'tp @s {x + 0.5} {gy} {z + 5.5}')
        guest.wait(10)
        guest.raw('interact', *guest_bed)
        guest.wait(10)
        seen['guest_asleep'] = guest('state').get('sleeping')
        seen['guest_bed'] = state_of(guest, guest_bed, 'occupied')
        host.raw('disconnect')
        for attempt in range(60):
            time.sleep(1)
            try:
                host.raw('open', 'Lan')
                break
            except Failed:
                if attempt == 59:
                    raise
        host('wait_ready', seconds=400)
        port = host.raw('lan', 'open', 0, 'commands')['port']
        back_to_game(guest)
        guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(20)
        seen['reopened'] = {'sleeping_pos': sleeping_pos(), 'villager_bed': state_of(host, villager_bed, 'occupied'),
                            'guest_bed': state_of(host, guest_bed, 'occupied'),
                            'guest': guest('state').get('sleeping')}
        guest.shot(out, 'sleep-reload')
    finally:
        back_to_game(guest)
        guest('command', command='kill @e[type=minecraft:villager,distance=..16]')
        for at in (villager_bed, guest_bed):
            guest('command', command=f'fill {at[0] - 1} {at[1]} {at[2]} {at[0]} {at[1]} {at[2]} minecraft:air')
        host('command', command='time set day')
    pos = lambda reply: [int(v) for v in re.findall(r'-?\d+', json.dumps(reply).partition('[I;')[2].partition(']')[0])]
    assert pos(seen['villager_asleep']) == villager_bed and seen['villager_bed'] == 'true', \
        f'the villager never lay down in its bed: {seen}'
    assert seen['guest_asleep'] == guest_bed and seen['guest_bed'] == 'true', f'the guest did not lie down: {seen}'
    reopened = seen['reopened']
    assert pos(reopened['sleeping_pos']) == villager_bed, f'the villager woke up with the reload: {seen}'
    assert reopened['villager_bed'] == 'true', f'the sleeping villager\'s bed is free after the reload: {seen}'
    assert reopened['guest_bed'] == 'false' and reopened['guest'] is None, f'the guest\'s bed stays occupied: {seen}'
    return seen


def check_predict(host, guest, out):
    # MultiPlayerGameMode.useItemOn under startPrediction (#869): the guest's door opens on its own screen
    # at once, with the host (the server) frozen, and the server's answer later agrees; an iron door
    # (no hand use) stays shut on both sides.
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    x, y, z = (int(v // 1) for v in pos_of(guest))
    wood, iron = (x - 1, y, z + 2), (x + 1, y, z + 2)
    for (bx, by, bz), kind in ((wood, 'oak_door'), (iron, 'iron_door')):
        for half, dy in (('lower', 0), ('upper', 1)):
            host('command', command=f'setblock {bx} {by + dy} {bz} minecraft:{kind}[facing=north,hinge=right,half={half}]')
    guest.wait(20)
    state = lambda game, at: game.raw('block', *at)['state']
    seen = {}
    creaks = lambda game: sum(n for s, n in game('status')['heard'].items() if 'door' in s)
    heard = {'guest': creaks(guest), 'host': creaks(host)}
    host.raw('server-hold', 'on')
    held_at = host.raw('simulation')['game_time']
    try:
        guest.raw('interact', *wood)
        seen['guest_frozen'] = (state(guest, wood), state(guest, (wood[0], wood[1] + 1, wood[2])))
        # level.playSound(player, ...): the clicker hears its own door at once.
        seen['guest_heard_frozen'] = creaks(guest) - heard['guest']
        guest.raw('interact', *iron)
        seen['guest_iron_frozen'] = state(guest, iron)
        assert host.raw('simulation')['game_time'] == held_at, 'server barrier advanced the world'
    finally:
        host.raw('server-hold', 'off')
    guest.wait(20)
    seen['guest_after'] = state(guest, wood)
    seen['host_after'] = state(host, wood)
    seen['iron_after'] = (state(guest, iron), state(host, iron))
    # ... and not a second time from the server; the host hears it once.
    seen['heard_after'] = {'guest': creaks(guest) - heard['guest'], 'host': creaks(host) - heard['host']}
    guest.shot(out, 'predict')
    for (bx, by, bz) in (wood, iron):
        host('command', command=f'fill {bx} {by} {bz} {bx} {by + 1} {bz} minecraft:air')
    assert all('open=true' in s for s in seen['guest_frozen']), f'the door waited for the server: {seen}'
    assert 'open=true' in seen['guest_after'] and 'open=true' in seen['host_after'], f'the door did not stay open: {seen}'
    assert 'open=false' in seen['guest_iron_frozen'], f'the guest opened an iron door by hand: {seen}'
    assert all('open=false' in s for s in seen['iron_after']), f'an iron door opened: {seen}'
    assert seen['guest_heard_frozen'] == 1 and seen['heard_after'] == {'guest': 1, 'host': 1}, \
        f'the door is not heard once on each side: {seen}'
    return seen


def check_push(host, guest, out):
    guest('command', command='gamemode survival')
    x, y, z = (int(v // 1) + 0.5 for v in pos_of(guest))
    guest('command', command=f'tp @s {x} {y - 0.5} {z}')
    guest.wait(20)
    x, y, z = pos_of(guest)
    # A cow that doesn't walk, 0.3 east of the guest: the two boxes overlap until their centres are
    # 0.75 apart, so the push stops a fraction of a block away.
    guest('command', command=f'summon minecraft:cow {x + 0.3:.2f} {y:.2f} {z:.2f} {{NoAI:1b}}')
    guest.wait(60)
    nx, _, nz = pos_of(guest)
    guest.shot(out, 'push')
    guest('command', command='kill @e[type=minecraft:cow]')
    moved = ((nx - x) ** 2 + (nz - z) ** 2) ** 0.5
    detail = {'from': [round(x, 3), round(z, 3)], 'to': [round(nx, 3), round(nz, 3)], 'moved': round(moved, 3)}
    assert nx < x, f'the cow did not push the guest west: {detail}'
    assert moved < 1.5, f'the cow pushed the guest too far: {detail}'
    return detail
def check_eat(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    # Hungry: the eating needs food below 20 (Player.canEat).
    guest('command', command='effect give @s minecraft:hunger 3 255')
    guest.wait(70)
    guest('command', command='effect clear @s')
    guest('command', command='give @s minecraft:bread 3')
    guest.wait(5)
    guest.raw('select', 'minecraft:bread')
    hungry = guest('state')['player']['food']
    # use-item --hold returns once released: watch the guest's screen meanwhile.
    hold = threading.Thread(target=guest.raw, args=('use-item', '--hold', 2.2))
    hold.start()
    using = []
    for _ in range(4):
        time.sleep(0.2)
        using.append(guest('state').get('using'))
    guest.shot(out, 'eat')
    hold.join()
    guest.wait(10)
    fed = guest('state')['player']['food']
    left = guest('inventory')['counts'].get('minecraft:bread', 0)
    detail = {'hungry': hungry, 'fed': fed, 'bread': left, 'using': using, 'after': guest('state').get('using')}
    assert hungry < 20, f'the guest is not hungry: {detail}'
    ticks = [u['ticks'] for u in using if u]
    assert len(ticks) >= 3 and ticks == sorted(ticks) and ticks[-1] > ticks[0], f'no eating on the guest\'s screen: {detail}'
    assert left == 2 and fed > hungry, f'the server did not eat one loaf: {detail}'
    assert detail['after'] is None, f'still eating after the release: {detail}'
    return detail


def watch_mining(miner, pos, seen):
    """Mine the block at `pos` (by hand, for seconds) and sample `seen()` meanwhile."""
    dig = threading.Thread(target=miner.raw, args=('break', *pos))
    dig.start()
    samples = []
    for _ in range(8):
        time.sleep(0.25)
        samples.append(seen())
    dig.join()
    return samples


def check_swings(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    # Stone by hand takes 7.5 seconds: ServerboundPunchPacket every tick keeps the arm swinging.
    guest('command', command=f'setblock {gx} {gy} {gz + 1} minecraft:stone')
    host('command', command=f'setblock {hx} {hy} {hz + 1} minecraft:stone')
    host.wait(10)
    def swinging(game):
        # On its server thread the host's window is a client of its server too.
        if game is host and not SERVER_THREAD:
            return lan_guest(host).get('model_swinging')
        return any(e.get('swinging') for e in remote(game).get('player_equipment', []))
    # Most samples: a swing restarts only past its first half (SwingState.startIfAble), so a sample can
    # fall between two.
    host_sees = watch_mining(guest, (gx, gy, gz + 1), lambda: swinging(host))
    guest_sees = watch_mining(host, (hx, hy, hz + 1), lambda: swinging(guest))
    # A hoe on dirt: the server tills it and swings the guest's hand, on the guest's screen too.
    # Where the guest stands now (an earlier check can leave it on a ledge it then drops from).
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    soil = (gx, gy - 1, gz + 1)
    guest('command', command=f'setblock {soil[0]} {soil[1]} {soil[2]} minecraft:dirt')
    # HoeItem.useOn tills only under air.
    guest('command', command=f'setblock {soil[0]} {soil[1] + 1} {soil[2]} minecraft:air')
    guest('command', command='give @s minecraft:wooden_hoe')
    guest.wait(10)
    guest.raw('select', 'minecraft:wooden_hoe')
    before = guest('state')['swings']
    guest.raw('look-at', soil[0] + 0.5, soil[1] + 1, soil[2] + 0.5)
    guest.wait(1)
    target = guest('state')['looking_at']['block']
    assert target and target['pos'] == list(soil) and target['face'] == 'up', target
    guest.raw('mouse', 'right', 'click')
    guest.wait(10)
    hoe = {'swings': guest('state')['swings'] - before, 'soil': host.raw('block', *soil)['block']}
    guest('command', command=f'setblock {soil[0]} {soil[1]} {soil[2]} minecraft:stone')
    detail = {'host_sees_guest': host_sees, 'guest_sees_host': guest_sees, 'hoe': hoe}
    assert sum(map(bool, host_sees)) >= 4, f'the host does not see the guest mine: {detail}'
    assert sum(map(bool, guest_sees)) >= 4, f'the guest does not see the host mine: {detail}'
    assert hoe == {'swings': 1, 'soil': 'minecraft:farmland'}, f'the guest\'s hoe: {detail}'
    return detail


def check_cracks(host, guest, out):
    # Another player's crack (#1096): ServerLevel.destroyBlockProgress sends each new stage to the
    # others (ClientboundBlockDestructionPacket), drawn by LevelRenderer from destroyingBlocks.
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
    face_each_other(host, guest)
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    guest('command', command=f'setblock {gx} {gy} {gz + 1} minecraft:stone')
    host('command', command=f'setblock {hx} {hy} {hz + 1} minecraft:stone')
    host.wait(10)
    shots = []

    def cracks(game, pos, name):
        stage = max((c['stage'] for c in game('status')['destroying'] if tuple(c['pos']) == pos), default=None)
        if stage is not None and stage >= 2 and name not in shots:
            shots.append(name)
            game.shot(out, name)
        return stage

    def watch(miner, watcher, pos, name):
        # The watcher looks at the block the other mines.
        watcher.raw('look-at', f'{pos[0] + 0.5:.2f}', f'{pos[1] + 0.5:.2f}', f'{pos[2] + 0.5:.2f}')
        watcher.wait(5)
        return watch_mining(miner, pos, lambda: cracks(watcher, pos, name))
    host_sees = watch(guest, host, (gx, gy, gz + 1), 'cracks-guest-mines')
    face_each_other(host, guest)
    guest_sees = watch(host, guest, (hx, hy, hz + 1), 'cracks-host-mines')
    host.wait(10)
    # The breaker's -1 (ServerLevel.destroyBlockProgress once the block is broken) clears each crack.
    gone = {name: [(c['pos'], game.raw('block', *c['pos'])['block']) for c in game('status')['destroying']]
            for name, game in (('host', host), ('guest', guest))}
    detail = {'host_sees_guest': host_sees, 'guest_sees_host': guest_sees, 'after': gone}
    stages = lambda s: [x for x in s if x is not None]
    assert len(stages(host_sees)) >= 3 and stages(host_sees) == sorted(stages(host_sees)), \
        f'the host does not see the guest\'s crack grow: {detail}'
    assert len(stages(guest_sees)) >= 3 and stages(guest_sees) == sorted(stages(guest_sees)), \
        f'the guest does not see the host\'s crack grow: {detail}'
    assert not any(gone.values()), f'a crack stays after the block broke: {detail}'
    return detail


def check_eating(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='clear @s')
        # Hungry: Player.canEat.
        g('command', command='effect give @s minecraft:hunger 3 255')
    host.wait(70)
    for g in (host, guest):
        g('command', command='effect clear @s')
        g('command', command='give @s minecraft:bread 2')
    host.wait(5)
    face_each_other(host, guest)

    def using(game):
        # On its server thread the host's window is a client of its server too.
        if game is host and not SERVER_THREAD:
            return lan_guest(host).get('model_using') or 0
        return max((e.get('using') or 0 for e in remote(game).get('player_equipment', [])), default=0)
    seen = {}
    for name, eater, watcher in (('host eats', host, guest), ('guest eats', guest, host)):
        eater.raw('select', 'minecraft:bread')
        hold = threading.Thread(target=eater.raw, args=('use-item', '--hold', 2.2))
        hold.start()
        samples = []
        for _ in range(4):
            time.sleep(0.3)
            samples.append(using(watcher))
        watcher.shot(out, 'eating-' + name.replace(' ', '-'))
        hold.join()
        eater.wait(10)
        seen[name] = {'during': samples, 'after': using(watcher)}
    for name, s in seen.items():
        assert sum(1 for u in s['during'] if u > 0) >= 3 and s['after'] == 0, f'{name}: {seen}'
    return seen


def check_xp(host, guest, out):
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='xp set @s 0 levels')
        g('command', command='xp set @s 0 points')
    # Out of the orbs' way wherever an earlier check left the host (ExperienceOrb follows the
    # nearest player who isn't a spectator).
    hx, hy, hz = pos_of(host)
    host('command', command='gamemode spectator')
    try:
        guest.wait(10)
        # `xp set` leaves Player.totalExperience (the score): experience from earlier checks stays in it.
        total = guest('state')['player']['xp']['total']
        host_total = host('state')['player']['xp']['total']
        x, y, z = pos_of(guest)
        # Three orbs of 7 points 4 blocks east of the guest (out of reach, within the 8 blocks orbs
        # follow from): they fly to the guest.
        # Freeze setup so command/IPC latency cannot let all three be collected
        # before the two-tick in-flight observation.
        host('command', command='tick freeze')
        for _ in range(3):
            guest('command', command=f'summon minecraft:experience_orb {x + 4:.2f} {y + 0.5:.2f} {z:.2f} {{Value:7s}}')
        host('command', command='tick step 2')
        guest.wait(2)
        seen = remote(guest).get('entities', {}).get('minecraft:experience_orb', 0)
        guest.shot(out, 'xp-orbs')
        host('command', command='tick unfreeze')
        guest.wait(60)
        xp = guest('state')['player']['xp']
        host_xp = host('state')['player']['xp']
        left = remote(guest).get('entities', {}).get('minecraft:experience_orb', 0)
    finally:
        host('command', command='tick unfreeze')
        host('command', command='gamemode survival')
        # Back where it stood: a spectator sinks (Player.travel keeps 0.6 of its fall speed each tick,
        # about 0.2 blocks from standing), and a later check that makes it survival again would land
        # it inside the floor, with the guest teleported beside it (swings).
        host('command', command=f'tp @s {hx:.2f} {hy:.2f} {hz:.2f}')
    detail = {'seen': seen, 'guest': xp, 'host': host_xp, 'left': left}
    # ExperienceOrb.scanForMerges: orbs of one value side by side can merge into one.
    assert 1 <= seen <= 3, f'the guest saw {seen} orbs: {detail}'
    assert xp['total'] - total == 21 and xp['level'] == 2, f'the guest\'s experience after three 7-point orbs: {detail}'
    assert host_xp['total'] == host_total, f'the host took the orbs: {detail}'
    assert left == 0, f'orbs left on the guest\'s screen: {detail}'
    return {'seen': seen, 'guest': xp, 'host': host_xp}


def check_time(host, guest, out):
    def times():
        return host('state')['time']['day_time'], guest('state')['time']['day_time']
    host('command', command='time set 1000')
    host('command', command='time pause')
    host.wait(60)
    paused = times()
    host('command', command='time resume')
    guest.wait(20)
    # A stalled game runs at most 10 missed ticks when it wakes (DeltaTracker); the server's
    # game time, sent every second, moves the guest's clock on (ClientClockManager.tick).
    guest.raw('stall', 4000)
    guest.wait(60)
    stalled = times()
    detail = {'paused': paused, 'after_stall': stalled}
    assert paused[0] == paused[1] and 1000 <= paused[0] < 1010, f'the clock while paused: {detail}'
    # The two reads are a tick or two apart.
    assert abs(stalled[0] - stalled[1]) <= 3, f'the guest\'s clock after its game stalled: {detail}'
    return detail


def objects_near(game, centre, key='objects'):
    """`status`'s objects within 8 blocks of centre, without their ids (each side numbers its own)."""
    out = []
    for o in game('status')[key]:
        if all(abs(o['position'][i] - centre[i]) <= 8 for i in (0, 2)):
            o = dict(o)
            o.pop('id')
            o['position'] = [round(v, 2) for v in o['position']]
            if 'yaw' in o:  # as the packets carry it, in 256ths of a turn
                o['yaw'] = round(o['yaw'] * 256 / 360) % 256
            out.append(o)
    return sorted(out, key=json.dumps)


def check_objects(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    commands = [
        f'fill {x - 6} {y} {z - 6} {x + 6} {y + 3} {z - 6} minecraft:stone',
        f'summon minecraft:minecart {x - 4.5} {y} {z + 3.5}',
        f'summon minecraft:chest_minecart {x - 2.5} {y} {z + 3.5}',
        f'summon minecraft:furnace_minecart {x + 1.5} {y} {z + 3.5}',
        f'summon minecraft:tnt_minecart {x + 4.5} {y} {z + 3.5}',
        f'summon minecraft:oak_boat {x - 4.5} {y} {z + 6.5} {{Rotation:[45f,0f]}}',
        f'summon minecraft:birch_chest_boat {x - 1.5} {y} {z + 6.5}',
        f'summon minecraft:bamboo_raft {x + 2.5} {y} {z + 6.5}',
        f'summon minecraft:armor_stand {x - 2.5} {y} {z - 2.5} {{Rotation:[90f,0f]}}',
        f'summon minecraft:armor_stand {x - 4.5} {y} {z - 2.5} {{ShowArms:1b,Small:1b,NoBasePlate:1b,'
        'equipment:{head:{id:"minecraft:diamond_helmet",count:1}}}',
        f'summon minecraft:item_frame {x - 3.5} {y + 1} {z - 4.97} {{block_pos:[I;{x - 4},{y + 1},{z - 5}],Facing:3b,'
        'Item:{id:"minecraft:diamond",count:1},ItemRotation:3b}',
        f'summon minecraft:glow_item_frame {x - 1.5} {y + 1} {z - 4.97} {{block_pos:[I;{x - 2},{y + 1},{z - 5}],Facing:3b}}',
        f'summon minecraft:painting {x + 1.5} {y + 1} {z - 4.97} {{block_pos:[I;{x + 1},{y + 1},{z - 5}],facing:0b,'
        'variant:"minecraft:kebab"}',
        f'summon minecraft:painting {x + 4} {y + 1} {z - 4.97} {{block_pos:[I;{x + 4},{y + 1},{z - 5}],facing:0b,'
        'variant:"minecraft:courbet"}',
    ]
    try:
        for command in commands:
            host('command', command=command)
        host.wait(60)
        guest.raw('look-at', x, y, z - 5)
        guest.wait(10)
        guest.shot(out, 'objects')
        host.shot(out, 'objects')
        mine, theirs = objects_near(host, (x, y, z)), objects_near(guest, (x, y, z))
    finally:
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 8},y={y - 2},z={z - 8},dx=16,dy=8,dz=16]')
        host('command', command=f'fill {x - 6} {y} {z - 6} {x + 6} {y + 3} {z - 6} minecraft:air')
    assert len(mine) == len(commands) - 1, f'the host has {len(mine)} objects: {mine}'
    assert theirs == mine, f'the guest sees {theirs}\nthe host has {mine}'
    return f'{len(theirs)} objects identical'


def check_projectiles(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    commands = [
        f'summon minecraft:tnt {x + 4.5} {y} {z + 4.5} {{fuse:2000s}}',
        f'summon minecraft:end_crystal {x - 4.5} {y} {z + 4.5} {{ShowBottom:0b}}',
        f'summon minecraft:tnt {x + 2.5} {y + 5} {z - 3.5} {{fuse:2000s}}',
        f'summon minecraft:arrow {x - 2.5} {y + 4} {z - 2.5} {{Motion:[0.2d,0d,0d]}}',
        f'summon minecraft:falling_block {x - 0.5} {y + 6} {z - 4.5} {{BlockState:{{Name:"minecraft:sand"}}}}',
    ]
    try:
        for command in commands:
            host('command', command=command)
        host.wait(80)
        guest.raw('look-at', x, y, z)
        guest.wait(5)
        guest.shot(out, 'projectiles')
        mine = objects_near(host, (x, y, z), 'projectiles')
        theirs = objects_near(guest, (x, y, z), 'projectiles')
    finally:
        host('command', command=f'kill @e[type=!minecraft:player,x={x - 8},y={y - 2},z={z - 8},dx=16,dy=10,dz=16]')
        host('command', command=f'setblock {x - 1} {y} {z - 5} minecraft:air')
    assert len(mine) == len(commands) - 1, f'the host has {len(mine)}: {mine}'
    # The fuse counts down every tick: the guest's, read a moment later, may be a tick or two behind.
    fuses = [(m.pop('fuse', 0), t.pop('fuse', 0)) for m, t in zip(mine, theirs)]
    assert theirs == mine, f'the guest sees {theirs}\nthe host has {mine}'
    assert all(abs(a - b) <= 2 for a, b in fuses), f'fuses (host, guest): {fuses}'
    return f'{len(theirs)} identical, fuses (host, guest) {fuses}'


def check_thrown(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        for item, kind in (('splash_potion', 'minecraft:splash_potion'), ('ender_eye', 'minecraft:eye_of_ender')):
            player('command', command=f'item replace entity @s weapon.mainhand with minecraft:{item}')
            player.wait(10)
            # Up at 45 degrees, into open sky.
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 11.6:.2f}', f'{z + 10.5:.2f}')
            player.raw('use-item')
            seen = {'own': 0, 'other': 0}
            for _ in range(10):
                player.wait(2)
                for side, game in (('own', player), ('other', other)):
                    if any(p['kind'] == kind for p in game('status')['projectiles']):
                        seen[side] += 1
                if all(seen.values()):
                    break
            player.shot(out, f'thrown-{label}-{item}')
            other.shot(out, f'thrown-{label}-{item}-seen')
            player.wait(100)  # it lands or drops
            result[f'{label} {item}'] = seen
    for name, seen in result.items():
        assert all(seen.values()), f'{name}: seen in flight (samples) {result}'
    return result


def made(game, kind):
    """How many `kind` particles `game` has made so far."""
    return game('status').get('particles', {}).get(kind, 0)


def sparked(player, other, kinds, act, ticks=20):
    """The `kinds` particles each side makes while `act` runs and for up to `ticks` after."""
    before = {(side, k): made(g, k) for side, g in (('own', player), ('other', other)) for k in kinds}
    act()
    seen = {}
    for _ in range(ticks // 2):
        player.wait(2)
        seen = {f'{side} {k}': made(g, k) - before[(side, k)]
                for side, g in (('own', player), ('other', other)) for k in kinds}
        if all(seen.values()):
            break
    return seen


def check_fireworks(host, guest, out):
    """Event 17 must arrive before removal, with the item's colored, trailing explosion (#1478).

    FireworkRocketEntity.explode / handleEntityEvent; run with and without --server-thread.
    The flash proves a configured explosion reached both clients; red pixels in later screenshots
    check that its sparks keep rendering after the tracked rocket disappears.
    """
    x, y, z = (int(v // 1) for v in pos_of(host))
    at = (x, y - 1, z + 3)
    host('command', command=f'fill {x - 5} {y - 1} {z - 2} {x + 5} {y - 1} {z + 8} minecraft:stone')
    host('command', command=f'fill {x - 5} {y} {z - 2} {x + 5} {y + 4} {z + 8} minecraft:air')
    host('command', command=f'fill {x - 2} {y + 2} {z + 1} {x + 2} {y + 2} {z + 5} minecraft:stone')
    for game, dx in ((host, -2), (guest, 2)):
        game('command', command='gamemode creative')
        game('command', command=f'tp @s {x + dx + 0.5} {y} {z + 0.5}')
        game.raw('look-at', x + 0.5, y + 1.5, z + 3.5)

    def reds(path):
        """The red pixels' positions, above the hotbar."""
        rows, bpp = png_rows(path)
        return {(y, i) for y, row in enumerate(rows[:len(rows) * 3 // 4]) for i in range(0, len(row), bpp)
                if row[i] > 80 and row[i] > 2 * row[i + 1] and row[i] > 2 * row[i + 2]}

    # Red where the view had none: the setup's own red chat lines (a fill that changed nothing) are in the
    # baseline, and fade out before the later shots, which a plain count difference would take as missing sparks.
    baseline = {g.name: reds(g.shot(out, 'fireworks-before')) for g in (host, guest)}
    before = {g.name: made(g, 'flash') for g in (host, guest)}
    host('command', command='item replace entity @s weapon.mainhand with minecraft:firework_rocket'
         '[fireworks={flight_duration:0,explosions:[{shape:"large_ball",colors:[I;16711680],has_trail:true}]}]')
    host.raw('interact', *at)
    # The held item's red texture must not count as a burst or a later spark.
    host('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    for _ in range(30):
        host.wait(2)
        if all(made(g, 'flash') > before[g.name] for g in (host, guest)):
            break
    flashes = {g.name: made(g, 'flash') - before[g.name] for g in (host, guest)}
    assert all(n == 1 for n in flashes.values()), f'configured burst missing or duplicated: {flashes}'
    # Capture both views before decoding PNGs: decoding must not consume the sparks' lifetime (48-59 ticks). A
    # screenshot takes ~0.6 s where the frame is drawn on the CPU (lavapipe), so the later shots come 4 ticks after
    # the burst's, not more: by then the tracked rocket is gone.
    burst_shots = {g.name: g.shot(out, 'fireworks-burst') for g in (host, guest)}
    host.wait(4)
    trail_shots = {g.name: g.shot(out, 'fireworks-trail-fade') for g in (host, guest)}
    burst = {name: len(reds(path) - baseline[name]) for name, path in burst_shots.items()}
    trails = {name: len(reds(path) - baseline[name]) for name, path in trail_shots.items()}
    rockets = {g.name: g('status')['rockets'] for g in (host, guest)}
    assert not any(rockets.values()), f'exploded rocket remains: {rockets}'
    assert all(n > 20 for n in burst.values()), f'no red burst: {burst}'
    assert all(n > 20 for n in trails.values()), f'no later red trail/fade: {trails}'
    return {'flashes': flashes, 'red_burst': burst, 'red_trail_fade': trails, 'rockets': rockets}


def check_sparks(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        at = (x, y, z + 2)
        player('command', command='gamemode survival')
        try:
            player('command', command=f'setblock {at[0]} {at[1] - 1} {at[2]} minecraft:farmland')
            player('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:wheat')
            player('command', command='item replace entity @s weapon.mainhand with minecraft:bone_meal 8')
            player.wait(10)

            def click(height):
                player.raw('look-at', f'{at[0] + 0.5:.2f}', f'{at[1] + height:.2f}', f'{at[2] + 0.5:.2f}')
                player.raw('mouse', 'right', 'click')
            result[f'{label} bone meal'] = sparked(player, other, ['happy_villager'], lambda: click(0.1))
            player('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:copper_block')
            player('command', command='item replace entity @s weapon.mainhand with minecraft:honeycomb')
            player.wait(10)
            result[f'{label} wax'] = sparked(player, other, ['wax_on'], lambda: click(0.5))
            player('command', command='item replace entity @s weapon.mainhand with minecraft:iron_axe')
            player.wait(10)
            result[f'{label} wax off'] = sparked(player, other, ['wax_off'], lambda: click(0.5))
            block = player.raw('block', *at).get('block', '')
            result[f'{label} copper'] = block
        finally:
            player('command', command=f'fill {at[0]} {at[1] - 1} {at[2]} {at[0]} {at[1]} {at[2]} minecraft:air')
            player('command', command=f'setblock {at[0]} {at[1] - 1} {at[2]} minecraft:stone')
            player('command', command='clear @s')
    # A spawner beside the host, at night: its burst when the zombie comes.
    x, y, z = spots['host']
    host('command', command='time set midnight')
    try:
        host('command', command=f'setblock {x} {y} {z + 3} minecraft:spawner{{Delay:20s,SpawnCount:1s,'
                                'SpawnData:{entity:{id:"minecraft:zombie",NoAI:1b}}}')
        result['spawner'] = sparked(host, guest, ['flame', 'smoke'], lambda: None, ticks=200)
    finally:
        host('command', command=f'setblock {x} {y} {z + 3} minecraft:air')
        host('command', command=f'kill @e[type=minecraft:zombie,x={x},y={y},z={z},distance=..12]')
        host('command', command='time set day')
    wrong = {k: v for k, v in result.items() if isinstance(v, dict) and not all(n > 0 for n in v.values())}
    assert not wrong, f'no particles: {wrong}\n{result}'
    return result


def hearts(game):
    """The player's maximum and current health as its own client holds them."""
    p = game('status')['player']
    return p.get('max_health'), round(p['health'], 1)


def check_hud(host, guest, out):
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        player('command', command='gamemode survival')
        try:
            for maximum in (40, 10):
                player('command', command=f'attribute @s minecraft:max_health base set {maximum}')
                player.wait(10)
                player('command', command='effect give @s minecraft:instant_health 1 10')
                seen = None
                for _ in range(15):
                    player.wait(2)
                    seen = hearts(player)
                    if seen == (maximum, maximum):
                        break
                result[f'{label} {maximum}'] = seen
                player.shot(out, f'hearts-{label}-{maximum}')
        finally:
            player('command', command='attribute @s minecraft:max_health base reset')
            player('command', command='effect give @s minecraft:instant_health 1 10')
    # BossBarCommands' new bar starts visible (ServerBossEvent's constructor).
    host('command', command='bossbar add lan_e2e:bar "LAN bar"')
    try:
        host('command', command='bossbar get lan_e2e:bar visible')
        host.wait(5)
        result['bar'] = [line for line in host.raw('messages', 5)['chat'] if 'lan_e2e:bar' in line or 'LAN bar' in line]
    finally:
        host('command', command='bossbar remove lan_e2e:bar')
    wrong = {k: v for k, v in result.items() if k != 'bar' and v != (int(k.split()[1]),) * 2}
    assert not wrong, f'hearts: {wrong}\n{result}'
    assert any('currently visible' in line for line in result['bar']), f'bar: {result["bar"]}'
    return result


def sidebars(host, guest, want, ticks=40):
    """Each side's sidebar (objective and scores) once both show `want` (host, guest), or after `ticks`."""
    seen = None
    for _ in range(ticks // 2):
        host.wait(2)
        seen = tuple(g('status').get('sidebar') for g in (host, guest))
        got = tuple(s and (s['objective'], s['scores']) for s in seen)
        if got == want:
            break
    return seen


def check_sidebar(host, guest, out):
    result = {}
    run = lambda *cmds: [host('command', command=c) for c in cmds]
    try:
        run('scoreboard objectives add audit dummy {"text":"Audit board","color":"aqua"}',
            'scoreboard players set Alpha audit 7', 'scoreboard players set Beta audit 3',
            'scoreboard objectives setdisplay sidebar audit')
        both = ('audit', {'Alpha': 7, 'Beta': 3})
        result['shown'] = sidebars(host, guest, (both, both))
        host.shot(out, 'sidebar')
        guest.shot(out, 'sidebar')
        run('scoreboard players set Alpha audit 12', 'scoreboard players reset Beta audit')
        both = ('audit', {'Alpha': 12})
        result['changed'] = sidebars(host, guest, (both, both))
        # The guest's team's sidebar replaces the sidebar for the guest only.
        run('team add red', 'team modify red color red', f'team join red {GUEST_NAME}',
            'scoreboard objectives add reds dummy', f'scoreboard players set {GUEST_NAME} reds 5',
            'scoreboard objectives setdisplay sidebar.team.red reds')
        result['team'] = sidebars(host, guest, (both, ('reds', {GUEST_NAME: 5})))
        run('scoreboard objectives remove reds', 'scoreboard objectives setdisplay sidebar')
        result['cleared'] = sidebars(host, guest, (None, None))
    finally:
        run('scoreboard objectives remove audit', 'scoreboard objectives remove reds', 'team remove red')
    want = {'shown': (('audit', {'Alpha': 7, 'Beta': 3}),) * 2, 'changed': (('audit', {'Alpha': 12}),) * 2,
            'team': (('audit', {'Alpha': 12}), ('reds', {GUEST_NAME: 5})), 'cleared': (None, None)}
    got = {k: tuple(s and (s['objective'], s['scores']) for s in v) for k, v in result.items()}
    wrong = {k: v for k, v in got.items() if v != want[k]}
    assert not wrong, f'sidebars: {wrong}\n{result}'
    return result


def fluid_pig_position(host, tag):
    """Read the tagged pig even after the current carries it past its spawn."""
    reply = server_reply(host, f'data get entity @e[type=pig,tag={tag},limit=1] Pos', 'entity data')
    values = re.findall(r'-?\d+(?:\.\d+)?', reply.rsplit(':', 1)[-1])
    assert len(values) == 3, f'no position for {tag}: {reply}'
    return tuple(map(float, values))


def clear_fluid_arena(host, x, y, z, tags, floor='minecraft:stone'):
    """Kill the tagged pigs and empty the arena, up in the air at y 100 so its
    walls never close around a player; the last clear takes the floor away too."""
    for tag in tags:
        host('command', command=f'kill @e[type=pig,tag={tag}]')
    host('command', command=f'fill {x-2} {y} {z-2} {x+9} {y+6} {z+7} minecraft:air')
    host('command', command=f'fill {x-2} {y-1} {z-2} {x+9} {y-1} {z+7} {floor}')
    host.wait(10)


def check_flow_push(host, guest, out):
    """A current pushes a tagged pig farther than a pool of source blocks."""
    hx, _, hz = pos_of(host)
    x, y, z = (int(hx // 16) * 16 + 8, 100, int(hz // 16) * 16 + 8)
    tags = ('horizon_flow_test', 'horizon_still_test')
    host('command', command='gamemode survival')
    try:
        clear_fluid_arena(host, x, y, z, tags)
        # Enclosed trough: its walls keep water inside the area cleaned below.
        host('command', command=f'fill {x-1} {y} {z-1} {x+8} {y+1} {z+1} minecraft:stone')
        host('command', command=f'fill {x} {y} {z} {x+7} {y+2} {z} minecraft:air')
        host('command', command=f'setblock {x} {y} {z} minecraft:water')
        # Enclosed source pool, whose centre has no horizontal current.
        host('command', command=f'fill {x+2} {y} {z+2} {x+6} {y+1} {z+6} minecraft:stone')
        host('command', command=f'fill {x+3} {y} {z+3} {x+5} {y} {z+5} minecraft:water')
        host('command', command=f'fill {x+3} {y+1} {z+3} {x+5} {y+3} {z+5} minecraft:air')
        host.wait(40)
        for tag, px, pz in [(tags[0], x+1.5, z+0.5), (tags[1], x+4.5, z+4.5)]:
            host('command', command=f'summon minecraft:pig {px} {y+0.1} {pz} {{attributes:[{{id:"minecraft:movement_speed",base:0.0d}}],Tags:["{tag}"]}}')
        host.wait(40)
        flowing = fluid_pig_position(host, tags[0])[0] - (x + 1.5)
        still = fluid_pig_position(host, tags[1])[0] - (x + 4.5)
        assert flowing - still > 0.2, f'the current did not carry the mob: {flowing} vs {still}'
        return {'flowing': round(flowing, 3), 'still': round(still, 3)}
    finally:
        clear_fluid_arena(host, x, y, z, tags, floor='minecraft:air')


def check_bubble_column(host, guest, out):
    """A bubble column lifts a tagged pig farther than an ordinary water source."""
    hx, _, hz = pos_of(host)
    x, y, z = (int(hx // 16) * 16 + 8, 100, int(hz // 16) * 16 + 8)
    tags = ('horizon_bubble_test', 'horizon_plain_test')
    host('command', command='gamemode survival')
    try:
        clear_fluid_arena(host, x, y, z, tags)
        for px in (x, x+3):
            host('command', command=f'fill {px-1} {y} {z-1} {px+1} {y} {z+1} minecraft:stone')
            host('command', command=f'setblock {px} {y} {z} minecraft:water')
        host('command', command=f'setblock {x} {y-1} {z} minecraft:soul_sand')
        host.wait(20)
        for tag, px in [(tags[0], x+0.5), (tags[1], x+3.5)]:
            host('command', command=f'summon minecraft:pig {px} {y+0.1} {z+0.5} {{attributes:[{{id:"minecraft:movement_speed",base:0.0d}}],Tags:["{tag}"]}}')
        samples = {tag: [] for tag in tags}
        for _ in range(8):
            host.wait(5)
            for tag in tags:
                samples[tag].append(fluid_pig_position(host, tag)[1])
        column, plain = (max(samples[tag]) for tag in tags)
        assert column - plain > 0.2, f'the bubble column did not carry the pig up: {column} vs {plain}'
        return {'column': round(column-y, 3), 'plain': round(plain-y, 3)}
    finally:
        clear_fluid_arena(host, x, y, z, tags, floor='minecraft:air')


def check_border(host, guest, out):
    spots = apart(host, guest)
    result = {}
    try:
        for label, player, other in (('host', host, guest), ('guest', guest, host)):
            x, y, z = spots[label]
            # A whole-block centre: the wall's collision box is the border's, rounded out to blocks.
            cx, cz = float(x), float(z)
            # The other player keeps out of harm outside the border meanwhile.
            other('command', command='gamemode creative')
            host('command', command=f'fill {x - 3} {y - 1} {z} {x + 4} {y - 1} {z + 10} minecraft:stone')
            host('command', command=f'fill {x - 3} {y} {z} {x + 4} {y + 3} {z + 10} minecraft:air')
            # A border 10 wide around the player, its wall 5 blocks ahead: both sides know it.
            host('command', command=f'worldborder center {cx} {cz}')
            host('command', command='worldborder set 10')
            seen = None
            for _ in range(10):
                player.wait(2)
                seen = [g('status').get('border') for g in (player, other)]
                if all(b and b['size'] == 10 and b['center'] == [cx, cz] for b in seen):
                    break
            result[f'{label} border'] = seen
            # Sprint-jumping into the wall: it stops the player at the border.
            player('command', command=f'tp @s {cx} {y} {cz} 0 0')
            player.wait(10)
            player.raw('look-at', f'{cx:.2f}', f'{y + 1.6:.2f}', f'{cz + 20:.2f}')
            for k in ('w', 'ctrl', 'space'):
                player.raw('key', k, 'down')
            player.wait(40)
            for k in ('space', 'ctrl', 'w'):
                player.raw('key', k, 'up')
            player.wait(10)
            result[f'{label} reached'] = round(pos_of(player)[2] - cz, 2)
            player.shot(out, f'border-{label}')
            # Two blocks outside, with no safe zone: the border hurts.
            host('command', command='worldborder damage buffer 0')
            host('command', command='worldborder damage amount 1')
            player('command', command='effect give @s minecraft:instant_health 1 5')
            player.wait(20)
            player('command', command=f'tp @s {cx} {y} {cz + 7}')
            player.wait(5)
            before = player('status')['player']['health']
            player.wait(40)
            result[f'{label} hurt'] = [before, player('status')['player']['health']]
            player('command', command=f'tp @s {cx} {y} {cz}')
            player('command', command='effect give @s minecraft:instant_health 1 5')
            host('command', command='worldborder damage amount 0.2')
            host('command', command='worldborder damage buffer 5')
            other('command', command='gamemode survival')
    finally:
        guest('command', command='gamemode survival')
        host('command', command='gamemode survival')
        host('command', command='worldborder damage amount 0.2')
        host('command', command='worldborder damage buffer 5')
        host('command', command='worldborder center 0 0')
        host('command', command='worldborder set 59999968')
    wrong = {}
    for label in ('host', 'guest'):
        x, y, z = spots[label]
        known = result[f'{label} border']
        if not all(b and b['size'] == 10 and b['center'] == [x, z] for b in known):
            wrong[f'{label} border'] = known
        if not 4.0 < result[f'{label} reached'] <= 4.71:
            wrong[f'{label} reached'] = result[f'{label} reached']
        before, after = result[f'{label} hurt']
        if not after < before:
            wrong[f'{label} hurt'] = result[f'{label} hurt']
    assert not wrong, f'border: {wrong}\n{result}'
    return result


def check_particles(host, guest, out):
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        # For the player alone (`@s`), then for every player near.
        before = {side: made(g, 'flame') for side, g in (('own', player), ('other', other))}
        player('command', command='particle minecraft:flame ~ ~1 ~3 .2 .2 .2 0 60 force @s')
        player.wait(10)
        result[f'{label} @s'] = {side: made(g, 'flame') - before[side] for side, g in (('own', player), ('other', other))}
        player.shot(out, f'flame-{label}')
        # No spread: one point, wherever the player stands.
        player('command', command='particle minecraft:flame ~ ~1 ~3 0 0 0 0 200 force @s')
        player.wait(3)
        player.shot(out, f'zero-spread-{label}')
        result[f'{label} all'] = sparked(player, other, ['dust'], lambda: player(
            'command', command='particle minecraft:dust{color:[1,0,0],scale:2} ~ ~1 ~3 0 0 0 0 10'))
    # A TNT's blast between the players: its burst on both sides.
    x, y, z = spots['host']
    host('command', command='gamemode creative')
    guest('command', command='gamemode creative')
    try:
        result['blast'] = sparked(host, guest, ['explosion_emitter'], lambda: host(
            'command', command=f'summon minecraft:tnt {x + 6.5} {y} {z + 4.5} {{fuse:10s}}'), ticks=40)
        host.shot(out, 'blast')
    finally:
        host('command', command='gamemode survival')
        guest('command', command='gamemode survival')
        host('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 16} {y - 1} {z + 8} minecraft:stone')
    # Without the server thread a guest's command reaches no host player (as `@a` doesn't).
    if not SERVER_THREAD:
        result['guest all'].pop('other dust')
    wrong = {k: v for k, v in result.items() if not k.endswith('@s') and not all(n > 0 for n in v.values())}
    wrong.update({k: v for k, v in result.items() if k.endswith('@s') and not (v['own'] >= 30 and v['other'] == 0)})
    assert not wrong, f'particles: {wrong}\n{result}'
    return result


def check_blast(host, guest, out):
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    host('command', command='gamemode creative')
    x, y, z = (int(v // 1) for v in pos_of(host))
    gx, gz = x + 6, z
    floor = f'fill {gx - 6} {y - 1} {gz - 4} {gx + 6} {y - 1} {gz + 4} minecraft:bedrock'
    detail = {}
    try:
        for shielded in (False, True):
            if remote(guest)['health'] <= 0:
                guest.raw('respawn')
                guest.wait(40)
            host('command', command=floor)
            host('command', command=f'fill {gx - 6} {y} {gz - 4} {gx + 6} {y + 3} {gz + 4} minecraft:air')
            guest('command', command=f'tp @s {gx + 0.5} {y} {gz + 0.5}')
            guest('command', command='effect give @s minecraft:instant_health 1 5')
            guest.wait(30)
            hold = None
            if shielded:
                guest('command', command='item replace entity @s weapon.offhand with minecraft:shield')
                guest.wait(5)
                guest.raw('look-at', gx + 5.5, y + 0.5, gz + 0.5)
                hold = threading.Thread(target=guest.raw, args=('use-item', '--hold', 2.5))
                hold.start()
                time.sleep(0.5)
            before = remote(guest)['health']
            at = pos_of(guest)
            # Five blocks off: a blast that hurts without killing (about 14 at full exposure).
            host('command', command=f'summon minecraft:tnt {gx + 5.5} {y} {gz + 0.5} {{fuse:10s}}')
            # The guest's own controller is busy holding the shield up: the host waits.
            host.wait(14)
            after = remote(guest)['health']
            moved = pos_of(guest)
            if hold:
                hold.join()
            guest.shot(out, f'blast-{"shield" if shielded else "open"}')
            detail['shield' if shielded else 'open'] = {
                'before': before, 'after': after, 'pushed': round(at[0] - moved[0], 2)}
    finally:
        host('command', command='gamemode survival')
        guest('command', command='clear @s')
        host('command', command=floor)
    open_, shield = detail['open'], detail['shield']
    assert open_['after'] < open_['before'], f'the blast never hurt the guest: {detail}'
    assert open_['pushed'] > 0, f'the blast never pushed the guest away: {detail}'
    assert shield['after'] >= shield['before'], f'the shield did not stop the blast: {detail}'
    return detail


def light_differences(host, guest, lo, hi):
    """Positions of the box loaded on both sides, and those whose sky or block light differ."""
    h, g = host.raw('light', *lo, *hi), guest.raw('light', *lo, *hi)
    both = [i for i in range(len(h['sky'])) if h['sky'][i] != '.' and g['sky'][i] != '.']
    differ = [i for i in both if (h['sky'][i], h['block'][i]) != (g['sky'][i], g['block'][i])]
    return h, g, both, differ


def same_light(host, guest, lo, hi, what, ticks=100):
    """Wait until the guest's light over the box is the host's; the number of positions compared."""
    for _ in range(ticks // 10):
        h, g, both, differ = light_differences(host, guest, lo, hi)
        if not differ:
            assert both, f'{what}: no position of the box is loaded on both sides'
            return len(both)
        host.wait(10)
    i = differ[0]
    nx, nz = hi[0] - lo[0] + 1, hi[2] - lo[2] + 1
    pos = (lo[0] + i % nx, lo[1] + i // (nx * nz), lo[2] + i // nx % nz)
    raise AssertionError(f'{what}: {len(differ)} of {len(both)} positions differ, first {pos}: host sky '
                         f'{h["sky"][i]} block {h["block"][i]}, guest sky {g["sky"][i]} block {g["block"][i]}')


def check_flight(host, guest, out):
    def double_jump(game):
        # LocalPlayer.aiStep: a second jump press within 7 ticks of the first toggles flight.
        lines = []
        for hold in (1, 4):
            lines += ['key space down', f'wait {hold}', 'key space up', 'wait 2']
        game.script(lines + ['wait 10'])
        return game('state')['player']

    def set_mode(game, mode):
        # On a client of a server the mode and its abilities (may fly) come back with the server's
        # answer, later when the machine is busy: a double jump before them is two plain jumps.
        game('command', command=f'gamemode {mode}')
        for _ in range(100):
            if game('state').get('gamemode') == mode:
                break
            game.wait(1)
        game.wait(10)

    players = [('guest', guest)] + ([('host', host)] if SERVER_THREAD else [])
    seen = {}
    # The rule is seven simulation ticks, independent of wall time. A slow
    # server clock leaves enough time to deliver the key events even on busy CI.
    host('command', command='tick rate 4')
    try:
        for name, game in players:
            set_mode(game, 'creative')
            ground = game('state')['player']
            up = double_jump(game)
            game.wait(20)
            later = game('state')['player']
            set_mode(game, 'survival')
            fell = game('state')['player']
            seen[name] = {'flying': up['flying'], 'rose': round(up['position'][1] - ground['position'][1], 2),
                          'stayed': round(later['position'][1] - up['position'][1], 2),
                          'survival_flying': fell['flying']}
    finally:
        host('command', command='tick rate 20')
        for _, game in players:
            game('command', command='gamemode survival')
    for name, s in seen.items():
        assert s['flying'] and s['rose'] > 1 and abs(s['stayed']) < 0.1, f'{name} does not fly in creative: {seen}'
        assert not s['survival_flying'], f'{name} still flies in survival: {seen}'
    return seen


def check_drops(host, guest, out):
    """A dropped item lying where a block is then placed comes out of it (#1252): the server moves
    it towards the closest open side (ItemEntity.tick's noPhysics, Entity.moveTowardsClosestSpace),
    and both sides see it outside the block. The host places one, then the guest."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -3), (guest, 3)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command=f'fill {x - 1} {y} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        game('command', command=f'summon minecraft:item {x + 0.5} {y} {z + 0.5} '
                                '{Item:{id:"minecraft:diamond",count:1},PickupDelay:32767}')
        game.wait(20)
        game('command', command=f'setblock {x} {y} {z} minecraft:stone')
        game.wait(60)
        for viewer in (host, guest):
            near = [e['pos'] for e in viewer.raw('entities', 24, 'minecraft:item')['entities']
                    if abs(e['pos'][0] - x - 0.5) < 2 and abs(e['pos'][2] - z - 0.5) < 2]
            seen[f'{game.name} places, {viewer.name} sees'] = ([x, y, z], near)
        game.shot(out, 'drops')
        host('command', command='kill @e[type=minecraft:item]')
        host('command', command=f'setblock {x} {y} {z} minecraft:air')
    for name, (block, near) in seen.items():
        inside = [p for p in near if all(block[i] <= p[i] < block[i] + 1 for i in range(3))]
        assert near and not inside, f'{name}: items {near} in block {block} (all: {seen})'
    return seen


def check_underwater(host, guest, out):
    """Digging dirt by hand underwater, standing on the bottom of a pool: five times slower than in
    the air (SUBMERGED_MINING_SPEED), on the player's own client as on the server. Halfway through
    the client is about halfway (it doesn't break the block early, which the server would put
    back), and the block breaks (#1254). The dirt is at eye height, the player's head under water.
    The host, then the guest."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='gamemode survival')
        game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
        game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y + 3} {z + 1} minecraft:stone')
        game('command', command=f'fill {x} {y} {z} {x} {y + 2} {z} minecraft:water')
        game('command', command=f'setblock {x + 1} {y + 1} {z} minecraft:dirt')
        game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        game.wait(40)
        game.raw('look-at', f'{x + 1.5:.2f}', f'{y + 1.5:.2f}', f'{z + 0.5:.2f}')
        game.wait(5)
        game.raw('mouse', 'left', 'down')
        # Dirt by hand: 15 ticks in the air, 75 under water.
        game.wait(35)
        halfway = {'mining': game('state').get('mining'), 'client': game.raw('block', x + 1, y + 1, z).get('block'),
                   'server': host.raw('block', x + 1, y + 1, z).get('block'), 'y': y + 1}
        game.wait(70)
        game.raw('mouse', 'left', 'up')
        game.wait(10)
        done = host.raw('block', x + 1, y + 1, z).get('block')
        game.shot(out, 'underwater')
        seen[game.name] = {'halfway': halfway, 'done': done}
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y + 3} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 2} {y - 1} {z + 1} minecraft:stone')
    for name, s in seen.items():
        h = s['halfway']
        assert h['client'] == 'minecraft:dirt' == h['server'], f'{name} broke it early: {seen}'
        assert h['mining'] and h['mining']['pos'][1] == h['y'] and 0.3 < h['mining']['progress'] < 0.7, f'{name} digs at the wrong speed: {seen}'
        # Broken: air, or the pool's water flowing in.
        assert s['done'] in ('minecraft:air', 'minecraft:water'), f'{name} never broke it: {seen}'
    return seen





def own(game):
    """The player's own state, as its client has it."""
    return game('state')['player']




def check_contacts(host, guest, out):
    """Blocks that hurt what touches them, on the server: standing on magma (but not sneaking on it)
    and on a lit campfire, walking against a cactus and through grown sweet berry bushes, for the
    host and the guest (#1305, #1311, #1312, #1313); a young bush pricks no one."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='gamerule natural_health_regeneration false')
    seen = {}
    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, y, z = hx + 2, hy, hz + dz
            game('command', command='gamemode survival')
            game('command', command='effect clear @s')
            for case, blocks, walk in (
                    ('magma', [(0, -1, 'minecraft:magma_block')], False),
                    ('sneaking', [(0, -1, 'minecraft:magma_block')], False),
                    ('campfire', [(0, 0, 'minecraft:campfire[lit=true]')], False),
                    ('cactus', [(2, -1, 'minecraft:sand'), (2, 0, 'minecraft:cactus')], True),
                    ('berries', [(k, -1, 'minecraft:grass_block') for k in range(1, 4)]
                     + [(k, 0, 'minecraft:sweet_berry_bush[age=3]') for k in range(1, 4)], True),
                    ('young', [(k, -1, 'minecraft:grass_block') for k in range(1, 4)]
                     + [(k, 0, 'minecraft:sweet_berry_bush[age=0]') for k in range(1, 4)], True)):
                game('command', command='effect give @s minecraft:instant_health 1 5 true')
                game('command', command=f'fill {x - 1} {y - 2} {z - 1} {x + 4} {y - 1} {z + 1} minecraft:stone')
                game('command', command=f'fill {x - 1} {y} {z - 1} {x + 4} {y + 2} {z + 1} minecraft:air')
                # A wall ends the walk before the player leaves the floor.
                game('command', command=f'fill {x + 5} {y - 1} {z - 1} {x + 5} {y + 2} {z + 1} minecraft:stone')
                for dx, dy, block in blocks:
                    game('command', command=f'setblock {x + dx} {y + dy} {z} {block}')
                stand = 0.5 if case == 'campfire' else 0.0
                # Sneaking from before the player lands on the magma.
                held = ['shift'] if case == 'sneaking' else []
                for k in held:
                    game.raw('key', k, 'down')
                game.wait(5)
                game('command', command=f'tp @s {x + 0.5} {y + stand} {z + 0.5}')
                game.raw('look-at', f'{x + 10:.2f}', f'{y + 1.6:.2f}', f'{z + 0.5:.2f}')
                game.wait(30)
                before = own(game)['health']
                keys = ['w'] if walk else []
                for k in keys:
                    game.raw('key', k, 'down')
                game.wait(60)
                # Read before letting go: standing up on the magma would hurt.
                after = own(game)['health']
                for k in keys + held:
                    game.raw('key', k, 'up')
                game.wait(10)
                seen[f'{game.name}-{case}'] = (before, after)
                game.shot(out, f'contacts-{case}')
            host('command', command=f'fill {x - 1} {y} {z - 1} {x + 5} {y + 2} {z + 1} minecraft:air')
    finally:
        host('command', command='gamerule natural_health_regeneration true')
    for case, (before, after) in seen.items():
        safe = case.endswith(('sneaking', 'young'))
        assert (after == before) if safe else (after < before), f'{case}: {before} -> {after} (all: {seen})'
    return seen


def server_player(host, game):
    """The player's entry on the server: the guest's in the host's `lan status`, on the server
    thread the host's too; the host's own world otherwise."""
    if game is host and not SERVER_THREAD:
        return own(host)
    name = GUEST_NAME if game is not host else host.player
    return next(g for g in host.raw('lan', 'status')['guests'] if g['name'] == name)


def check_shield(host, guest, out):
    """An axe disables a raised shield for 5 s (100 ticks), counted down once a tick on the
    server: a vindicator's axe hits each player's shield, then the cooldown left falls by about 40
    over 40 ticks (#1316)."""
    host('command', command='gamerule natural_health_regeneration false')
    rates = {}
    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, y, z = (int(v // 1) for v in pos_of(host))
            z += dz
            game('command', command='gamemode survival')
            game('command', command='effect give @s minecraft:resistance 60 4 true')
            game('command', command='effect give @s minecraft:instant_health 1 5 true')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
            game('command', command='item replace entity @s weapon.offhand with minecraft:shield')
            game('command', command=f'fill {x - 1} {y} {z - 1} {x + 4} {y + 2} {z + 1} minecraft:air')
            game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 4} {y - 1} {z + 1} minecraft:stone')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            game.wait(10)
            game.raw('look-at', f'{x + 2.5:.2f}', f'{y + 1.5:.2f}', f'{z + 0.5:.2f}')
            game.raw('mouse', 'right', 'down')
            game.wait(10)
            host('command', command=f'summon minecraft:vindicator {x + 2.5} {y} {z + 0.5} '
                 '{PersistenceRequired:1b,Rotation:[90f,0f]}')
            host('command', command='item replace entity @e[type=minecraft:vindicator] weapon.mainhand '
                 'with minecraft:iron_axe')
            first = None
            for _ in range(60):
                game.wait(5)
                first = server_player(host, game).get('shield_cooldown', 0)
                if first:
                    break
            host('command', command='kill @e[type=minecraft:vindicator]')
            game.raw('mouse', 'right', 'up')
            assert first, f'{game.name}: the vindicator never disabled the shield'
            start = server_player(host, game)['shield_cooldown']
            game.wait(40)
            end = server_player(host, game)['shield_cooldown']
            rates[game.name] = {'first': first, 'start': start, 'end': end}
            game.shot(out, 'shield')
            game('command', command='item replace entity @s weapon.offhand with minecraft:air')
            game('command', command='effect clear @s')
    finally:
        host('command', command='kill @e[type=minecraft:vindicator]')
        host('command', command='gamerule natural_health_regeneration true')
    for name, r in rates.items():
        assert r['first'] <= 100, f'{name}: {rates}'
        # Once a tick: 40 down over 40 ticks, give or take the ticks the two queries straddle.
        assert 30 <= r['start'] - r['end'] <= 50, f'{name}: {rates}'
    return rates


def respawn_when_ready(game):
    """Press Respawn once the death screen lets it (DeathScreen's 20-tick delay, counted from when
    the screen opens)."""
    for _ in range(20):
        try:
            return game.raw('respawn')
        except Failed as e:
            if 'not active yet' not in str(e):
                raise
            game.wait(10)
    return game.raw('respawn')


def check_world_spawn(host, guest, out):
    """`setworldspawn` moves the world spawn: the host and the guest, with no bed of their own,
    respawn there once killed (#1310)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    sx, sz = hx + 24, hz
    host('command', command='gamerule respawn_radius 0')
    host('command', command=f'fill {sx - 2} {hy - 1} {sz - 2} {sx + 2} {hy - 1} {sz + 2} minecraft:stone')
    host('command', command=f'fill {sx - 2} {hy} {sz - 2} {sx + 2} {hy + 3} {sz + 2} minecraft:air')
    seen = {}
    try:
        host('command', command=f'setworldspawn {sx} {hy} {sz}')
        for game in (host, guest):
            game('command', command='gamemode survival')
            game('command', command='kill @s')
            game.wait(20)
            respawn_when_ready(game)
            game.wait(60)
            seen[game.name] = [round(v, 2) for v in pos_of(game)]
            game.shot(out, 'world-spawn')
    finally:
        host('command', command=f'setworldspawn {hx} {hy} {hz}')
        host('command', command='gamerule respawn_radius 10')
        for game in (host, guest):
            game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        guest.wait(20)
    for name, at in seen.items():
        assert abs(at[0] - (sx + 0.5)) < 1 and abs(at[2] - (sz + 0.5)) < 1, f'{name}: {seen}, spawn {sx} {sz}'
    return seen


def check_beds(host, guest, out):
    """A bed sets the respawn point (day or night): respawning stands the player up beside it; a bed
    that is gone, or boxed in by stone so there is nowhere to stand up, sends the player to the world
    spawn instead (#1308). The host, then the guest, each for the three beds."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='gamerule respawn_radius 0')
    host('command', command=f'setworldspawn {hx} {hy} {hz}')
    host('command', command='time set 1000')
    seen = {}
    try:
        for game, dz in ((host, -6), (guest, 6)):
            game('command', command='gamemode survival')
            x, y, z = hx + 12, hy, hz + dz
            for case in ('intact', 'gone', 'boxed'):
                game('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 3} minecraft:stone')
                game('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 3} {z + 3} minecraft:air')
                game('command', command=f'setblock {x} {y} {z} minecraft:red_bed[facing=east,part=foot]')
                game('command', command=f'setblock {x + 1} {y} {z} minecraft:red_bed[facing=east,part=head]')
                game('command', command=f'tp @s {x + 0.5} {y} {z + 1.5}')
                game.wait(20)
                game.raw('interact', x + 1, y, z)
                game.wait(10)
                # Die elsewhere, by the world spawn.
                game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 3.5}')
                game.wait(10)
                if case == 'gone':
                    game('command', command=f'fill {x} {y} {z} {x + 1} {y} {z} minecraft:air')
                elif case == 'boxed':
                    game('command', command=f'fill {x - 2} {y} {z - 2} {x + 3} {y + 2} {z + 2} minecraft:stone replace minecraft:air')
                game.wait(5)
                game('command', command='kill @s')
                game.wait(20)
                respawn_when_ready(game)
                game.wait(60)
                at = [round(v, 2) for v in pos_of(game)]
                seen[f'{game.name}-{case}'] = {'at': at, 'bed': [x + 1, y, z], 'health': own(game)['health']}
                game.shot(out, f'beds-{case}')
    finally:
        host('command', command='gamerule respawn_radius 10')
        host('command', command='time set 1000')
        for game in (host, guest):
            game('command', command=f'tp @s {hx + 0.5} {hy} {hz + 0.5}')
        guest.wait(20)
    for case, s in seen.items():
        (px, py, pz), (bx, by, bz) = s['at'], s['bed']
        if case.endswith('intact'):
            near = ((px - bx - 0.5) ** 2 + (pz - bz - 0.5) ** 2) ** 0.5
            assert near < 2.6 and abs(py - by) < 0.6, f'{case}: not beside its bed: {seen}'
        else:
            assert abs(px - (hx + 0.5)) < 1 and abs(pz - (hz + 0.5)) < 1, f'{case}: not at the world spawn: {seen}'
        assert s['health'] > 0, f'{case}: died again after respawning: {seen}'
    return seen


def check_death_mining(host, guest, out):
    """A player who dies while breaking obsidian, and lets go of attack on the death screen, doesn't
    break it once respawned: death forgets the digging (#1315)."""
    host('command', command='gamerule keep_inventory true')
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    try:
        for game, dz in ((host, -3), (guest, 3)):
            x, y, z = hx + 2, hy, hz + dz
            game('command', command='gamemode survival')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:diamond_pickaxe')
            game('command', command=f'setblock {x + 2} {y} {z} minecraft:obsidian')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            game.wait(10)
            game.raw('look-at', f'{x + 2.5:.2f}', f'{y + 0.5:.2f}', f'{z + 0.5:.2f}')
            game.wait(5)
            game.raw('mouse', 'left', 'down')
            game.wait(10)
            game('command', command='kill @s')
            game.wait(10)
            # The death screen lets go of the button (Minecraft.setScreen); let go again once back.
            try:
                game.raw('mouse', 'left', 'up')
            except Failed:
                pass
            game.wait(10)
            respawn_when_ready(game)
            game.wait(2)
            try:
                game.raw('mouse', 'left', 'up')
            except Failed:
                pass
            # Obsidian takes a diamond pickaxe about 190 ticks.
            game.wait(240)
            seen[game.name] = host.raw('block', x + 2, y, z).get('block')
            game.shot(out, 'death-mining')
            host('command', command=f'setblock {x + 2} {y} {z} minecraft:air')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    finally:
        host('command', command='gamerule keep_inventory false')
    for name, block in seen.items():
        assert block == 'minecraft:obsidian', f'{name}: {seen}'
    return seen


def check_absorption(host, guest, out):
    """Absorption II's golden hearts on each player's own HUD: 8 of them, and 5 once a hit of 3 took
    its share: the client shows what the server's player has (#1306)."""
    seen = {}
    for game in (host, guest):
        game('command', command='gamemode survival')
        game('command', command='effect give @s minecraft:absorption 120 1')
        game.wait(10)
        given = own(game)['absorption']
        game('command', command='damage @s 3 minecraft:generic')
        game.wait(20)
        me = own(game)
        server = server_player(host, game)
        game.shot(out, 'absorption')
        game('command', command='effect clear @s minecraft:absorption')
        seen[game.name] = {'given': given, 'left': me['absorption'], 'health': me['health'],
                           'server': server.get('absorption')}
    for name, s in seen.items():
        assert s['given'] == 8.0 and s['left'] == 5.0 == s['server'] and s['health'] == 20.0, \
            f'{name}: {seen}'
    return seen


def check_breath(host, guest, out):
    """Underwater in a Respiration III helmet for 15 s: the enchantment makes the air last on the
    server, and each player's own bubbles follow the server's air (#1304)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='gamemode survival')
        game('command', command='item replace entity @s armor.head with '
                                'minecraft:diamond_helmet[minecraft:enchantments={"minecraft:respiration":3}]')
        game('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:stone')
        game('command', command=f'fill {x} {y} {z} {x} {y + 1} {z} minecraft:water')
        game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    host.wait(300)
    for game in (host, guest):
        game.shot(out, 'breath')
        seen[game.name] = {'own': own(game)['air'], 'server': server_player(host, game).get('air')}
    for game, dz in ((host, -4), (guest, 4)):
        x, y, z = hx + 3, hy, hz + dz
        game('command', command='item replace entity @s armor.head with minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y + 2} {z + 1} minecraft:air')
        host('command', command=f'fill {x - 1} {y - 1} {z - 1} {x + 1} {y - 1} {z + 1} minecraft:stone')
    guest.wait(40)
    # 300 ticks lose about 75 of the 300 with Respiration III, all of it without.
    for name, s in seen.items():
        assert s['server'] > 150 and abs(s['own'] - s['server']) <= 10, f'{name}: {seen}'
    return seen


def check_lava(host, guest, out):
    """(--server-thread) Standing in a settled Nether lava current with Fire Resistance: the
    Nether's FAST_LAVA pushes each player about three times as hard as the Overworld's (#1309)."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    seen = {}
    back = {g.name: pos_of(g) for g in (host, guest)}
    try:
        for game, z in ((host, 0), (guest, 6)):
            game('command', command=f'execute in minecraft:the_nether run tp @s 0.5 100 {z + 0.5}')
            state = wait_dimension(game, 'minecraft:the_nether', 600)
            assert state.get('dimension') == 'minecraft:the_nether', f'{game.name} is in {state.get("dimension")}'
            game.wait(40)
            # A one-wide obsidian channel along x: a source at x = -3, the current flowing east.
            game('command', command=f'fill -5 98 {z - 2} 6 104 {z + 2} minecraft:air')
            game('command', command=f'fill -5 98 {z - 1} 6 98 {z + 1} minecraft:obsidian')
            game('command', command=f'fill -5 99 {z - 1} 6 99 {z - 1} minecraft:obsidian')
            game('command', command=f'fill -5 99 {z + 1} 6 99 {z + 1} minecraft:obsidian')
            game('command', command=f'fill -4 99 {z} -4 99 {z} minecraft:obsidian')
            game('command', command='effect give @s minecraft:fire_resistance 120 0 true')
            game('command', command='gamemode survival')
            game('command', command=f'setblock -3 99 {z} minecraft:lava')
        host.wait(100)
        for game, z in ((host, 0), (guest, 6)):
            game('command', command=f'tp @s 0.5 99 {z + 0.5}')
        host.wait(60)
        for game, z in ((host, 0), (guest, 6)):
            v = own(game)['velocity']
            seen[game.name] = {'vx': v[0], 'level': game.raw('block', 0, 99, z).get('state')}
            game.shot(out, 'lava')
    finally:
        for game, z in ((host, 0), (guest, 6)):
            host('command', command=f'execute in minecraft:the_nether run fill -5 98 {z - 2} 6 104 {z + 2} minecraft:air')
            x, y, zz = back[game.name]
            game('command', command='effect clear @s minecraft:fire_resistance')
            game('command', command=f'execute in minecraft:overworld run tp @s {x:.2f} {y:.2f} {zz:.2f}')
        for game in (host, guest):
            wait_dimension(game, 'minecraft:overworld', 600)
        guest.wait(40)
    # The Overworld's current settles at about 0.0047 a tick, the Nether's at about 0.014.
    for name, s in seen.items():
        assert s['vx'] > 0.009, f'{name}: {seen}'
    return seen


def check_ladder(host, guest, out):
    """#1255: climbing down a ladder doesn't hurt where the guest lands."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    lx, lz = x + 6, z
    guest('command', command='gamemode survival')
    guest('command', command=f'fill {lx - 2} {y} {lz - 2} {lx + 2} {y + 12} {lz + 2} minecraft:air')
    guest('command', command=f'fill {lx - 2} {y - 1} {lz - 2} {lx + 2} {y - 1} {lz + 2} minecraft:stone')
    guest('command', command=f'fill {lx} {y} {lz + 1} {lx} {y + 10} {lz + 1} minecraft:stone')
    guest('command', command=f'fill {lx} {y} {lz} {lx} {y + 10} {lz} minecraft:ladder[facing=north]')
    guest.wait(10)
    try:
        before = remote(guest)['health']
        # No input on a ladder: the guest slides down at 0.15 a tick (LivingEntity.handleOnClimbable).
        guest('command', command=f'tp @s {lx + 0.5} {y + 10} {lz + 0.4} 0 0')
        for _ in range(40):
            guest.wait(5)
            if pos_of(guest)[1] < y + 0.01:
                break
        guest.wait(20)
        landed, after = pos_of(guest), remote(guest)['health']
        assert landed[1] < y + 0.01, f'the guest stayed on the ladder at {landed}'
        assert after >= before, f'climbing down ten blocks of ladder took the guest from {before} to {after} health'
        return {'landed': landed, 'health': after}
    finally:
        guest('command', command=f'fill {lx} {y} {lz} {lx} {y + 10} {lz + 1} minecraft:air')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')


def apart(host, guest):
    """The host where it stands and the guest 12 blocks east, each on a stone floor with room
    around: neither stands in the other's way (a ray from inside a player's box hits that player)."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    spots = {}
    for label, player, dx in (('host', host, 0), ('guest', guest, 12)):
        px = x + dx
        host('command', command=f'fill {px - 3} {y - 1} {z - 3} {px + 4} {y - 1} {z + 6} minecraft:stone')
        host('command', command=f'fill {px - 3} {y} {z - 3} {px + 4} {y + 3} {z + 6} minecraft:air')
        player('command', command=f'tp @s {px + 0.5} {y} {z + 0.5} 0 0')
        spots[label] = (px, y, z)
    host.wait(10)
    guest.wait(10)
    return spots


def board(player, kind, at):
    """Right-click the `kind` object near `at` and check `player` rides it."""
    found = [o for o in player('status')['objects'] if o['kind'].endswith(kind)
             and abs(o['position'][0] - at[0]) < 2 and abs(o['position'][2] - at[2]) < 2]
    assert found, f'no {kind} near {at}'
    vehicle = found[0]
    vp = vehicle['position']
    player.raw('look-at', f'{vp[0]:.2f}', f'{vp[1] + 0.3:.2f}', f'{vp[2]:.2f}')
    player.raw('mouse', 'right', 'click')
    player.wait(10)
    riding = player('status').get('riding')
    assert riding == vehicle['id'], f'right-clicking the {kind} {vehicle["id"]}: riding {riding}'
    return vehicle


def leave_vehicle(player, kind):
    player.raw('key', 'shift', 'down')
    player.wait(10)
    player.raw('key', 'shift', 'up')
    player.wait(10)
    riding = player('status').get('riding')
    assert riding is None, f'sneaking out of the {kind}: riding {riding}'


def break_vehicle(player, vehicle_id, kind):
    """Hit it with fists until it breaks: VehicleEntity.tick takes 1 off its damage every tick."""
    here = [o for o in player('status')['objects'] if o['id'] == vehicle_id]
    if not here:
        return
    vp = here[0]['position']
    # Keep the original ten hits/two-tick spacing in one connection. Starting
    # several ctl processes per hit lets the vehicle's damage decay between hits.
    player.script([f'look-at {vp[0]:.2f} {vp[1] + 0.3:.2f} {vp[2]:.2f}',
                   *[line for _ in range(10) for line in ('mouse left click', 'wait 2')]])
    assert not any(o['id'] == vehicle_id for o in player('status')['objects']), \
        f'ten hits left the {kind} in place'


def check_boats(host, guest, out):
    """#1222: the host (single player, on its server thread or not) and the guest each board a boat
    on water with a right click, paddle it forward (the other side sees it move), sneak out, and
    break it with their fists."""
    result = {}
    spots = apart(host, guest)
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        bx = x + 3
        player('command', command='gamemode survival')
        # A pool ten blocks long in a stone basin (the world may float), with a path beside it to
        # walk back to the boat.
        player('command', command=f'fill {bx - 3} {y} {z - 2} {bx + 2} {y + 3} {z + 10} minecraft:air')
        player('command', command=f'fill {bx - 3} {y - 2} {z - 2} {bx + 2} {y - 1} {z + 10} minecraft:stone')
        player('command', command=f'fill {bx - 1} {y - 1} {z - 1} {bx + 1} {y - 1} {z + 9} minecraft:water')
        player('command', command=f'summon minecraft:oak_boat {bx + 0.5} {y - 0.1} {z + 0.5}')
        player.wait(30)
        try:
            boat = board(player, 'oak_boat', (bx + 0.5, y, z + 0.5))
            start = boat['position']
            # The summoned boat faces south: forward paddles it along the pool.
            player.raw('key', 'w', 'down')
            player.wait(30)
            player.raw('key', 'w', 'up')
            player.wait(20)
            mine = [o for o in player('status')['objects'] if o['id'] == boat['id']]
            assert mine, f'the {label} lost its boat'
            moved = math.dist(start, mine[0]['position'])
            assert moved > 0.5, f'paddling forward moved the {label}\'s boat {moved:.2f} blocks'
            # The other side sees the boat where its rider took it (the server accepted its moves).
            theirs = [o for o in objects_near(other, mine[0]['position'], 'objects')
                      if o['kind'].endswith('oak_boat')]
            assert theirs, f'the other side sees no boat near the {label}\'s'
            far = min(math.dist(o['position'], mine[0]['position']) for o in theirs)
            assert far < 1.0, f'the other side sees the {label}\'s boat {far:.2f} blocks off'
            leave_vehicle(player, 'boat')
            player('command', command=f'tp @s {bx - 1.5} {y} {mine[0]["position"][2]:.2f}')
            player.wait(5)
            break_vehicle(player, boat['id'], 'boat')
            result[label] = {'boat': boat['id'], 'paddled': round(moved, 2), 'broken': True}
        finally:
            player('command', command=f'kill @e[type=minecraft:oak_boat,x={bx},y={y},z={z + 4},distance=..10]')
            player('command', command=f'fill {bx - 1} {y - 1} {z - 1} {bx + 1} {y - 1} {z + 9} minecraft:stone')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            player.wait(5)
    return result


def check_minecarts(host, guest, out):
    """#1222: the host and the guest each get into a minecart on rails with a right click, sneak out
    beside it, and break it with their fists."""
    result = {}
    spots = apart(host, guest)
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        cx = x + 3
        player('command', command='gamemode survival')
        player('command', command=f'fill {cx - 2} {y} {z - 2} {cx + 2} {y + 3} {z + 6} minecraft:air')
        player('command', command=f'fill {cx - 2} {y - 1} {z - 2} {cx + 2} {y - 1} {z + 6} minecraft:stone')
        player('command', command=f'fill {cx} {y} {z} {cx} {y} {z + 4} minecraft:rail')
        player('command', command=f'summon minecraft:minecart {cx + 0.5} {y} {z + 0.5}')
        player.wait(30)
        try:
            cart = board(player, 'minecart', (cx + 0.5, y, z + 0.5))
            seated = pos_of(player)
            assert abs(seated[0] - (cx + 0.5)) < 0.6, f'the {label} sits at {seated}, not in the cart'
            leave_vehicle(player, 'minecart')
            out_at = pos_of(player)
            assert math.dist(out_at, seated) > 0.3, f'the {label} stayed in its seat at {out_at}'
            player.wait(5)
            break_vehicle(player, cart['id'], 'minecart')
            result[label] = {'cart': cart['id'], 'out_at': [round(v, 2) for v in out_at], 'broken': True}
        finally:
            player('command', command=f'kill @e[type=minecraft:minecart,x={cx},y={y},z={z},distance=..8]')
            player('command', command=f'kill @e[type=minecraft:item,x={cx},y={y},z={z},distance=..8]')
            player('command', command=f'fill {cx} {y} {z} {cx} {y} {z + 4} minecraft:air')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            player.wait(5)
    return result


def check_frames(host, guest, out):
    """#1222: the host and the guest each put a diamond in an item frame and turn it, and put a
    helmet on an armor stand and take it back, with right clicks."""
    result = {}
    spots = apart(host, guest)
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        # Within a survival player's 3 blocks of reach: a frame on a wall, an armor stand beside it.
        player('command', command=f'fill {x - 2} {y} {z + 1} {x + 2} {y + 2} {z + 2} minecraft:air')
        player('command', command=f'fill {x - 2} {y} {z + 3} {x + 2} {y + 2} {z + 3} minecraft:stone')
        frame_kind = 'item_frame' if label == 'host' else 'glow_item_frame'
        player('command', command=f'summon minecraft:{frame_kind} {x + 0.5} {y + 1.5} {z + 2.97} '
               f'{{block_pos:[I;{x},{y + 1},{z + 2}],Facing:2b}}')
        player('command', command=f'summon minecraft:armor_stand {x - 1.5} {y} {z + 1.5}')
        player('command', command='clear @s')
        frame_name = f'{label} framed diamond'
        player('command', command='item replace entity @s weapon.mainhand with '
               f"minecraft:diamond[minecraft:custom_name='{frame_name}']")
        player.wait(30)
        try:
            frame = [o for o in player('status')['objects'] if o['kind'].endswith('item_frame')
                     and o['position'] == [x, y + 1, z + 2]]
            assert frame, f'the {label} sees no item frame at {[x, y + 1, z + 2]}'
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.5:.2f}', f'{z + 2.95:.2f}')
            assert frame_name not in (player('status').get('name_tags') or []), 'empty frame name visible'
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            frame = [o for o in player('status')['objects'] if o['id'] == frame[0]['id']][0]
            assert frame['item'] == 'minecraft:diamond', f'the {label}\'s frame holds {frame["item"]}'
            assert frame['rotation'] == 1, f'the {label}\'s frame turned to {frame["rotation"]}'
            assert frame_name in (player('status').get('name_tags') or []), \
                f"the {label}'s targeted frame has no item name"
            player.shot(out, f'{label}-framed-item-name')
            player.raw('key', 'f1', 'tap')
            player.wait(2)
            assert frame_name not in (player('status').get('name_tags') or []), 'F1 left the frame name visible'
            player.raw('key', 'f1', 'tap')
            player.raw('look-at', f'{x + 2:.2f}', f'{y + 1.5:.2f}', f'{z + 2.95:.2f}')
            assert frame_name not in (player('status').get('name_tags') or []), 'untargeted frame name visible'
            theirs = [o for o in other('status')['objects'] if o['kind'].endswith('item_frame')
                      and o['position'] == [x, y + 1, z + 2]]
            assert theirs and theirs[0]['item'] == 'minecraft:diamond' and theirs[0]['rotation'] == 1, \
                f'the other side sees the {label}\'s frame as {theirs}'
            player('command', command='item replace entity @s weapon.mainhand with minecraft:iron_helmet')
            player.wait(5)
            stand = [o for o in player('status')['objects'] if o['kind'] == 'minecraft:armor_stand'
                     and abs(o['position'][0] - (x - 1.5)) < 0.5 and abs(o['position'][2] - (z + 1.5)) < 0.5]
            assert stand, f'the {label} sees no armor stand'
            player.raw('look-at', f'{x - 1.5:.2f}', f'{y + 1.7:.2f}', f'{z + 1.5:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            worn = [o for o in player('status')['objects'] if o['id'] == stand[0]['id']][0]['equipment']
            assert 'minecraft:iron_helmet' in worn, f'the {label}\'s armor stand wears {worn}'
            player.raw('look-at', f'{x - 1.5:.2f}', f'{y + 1.7:.2f}', f'{z + 1.5:.2f}')
            player.raw('mouse', 'right', 'click')
            player.wait(10)
            worn = [o for o in player('status')['objects'] if o['id'] == stand[0]['id']][0]['equipment']
            assert 'minecraft:iron_helmet' not in worn, f'taking the helmet back left {worn}'
            result[label] = {'frame': 'diamond, turned', 'stand': 'helmet on and off'}
        finally:
            player('command', command=f'kill @e[type=!minecraft:player,x={x - 3},y={y - 1},z={z},dx=6,dy=4,dz=6]')
            player('command', command=f'fill {x - 2} {y} {z + 3} {x + 2} {y + 2} {z + 3} minecraft:air')
            player('command', command='clear @s')
    return result


def check_hands(host, guest, out):
    """#1350, #1355, #1362: right clicks a cow passes go on to the item, the off hand gets its
    turn on the cow, and a frame punched empty is empty for everyone."""
    result = {}
    spots = apart(host, guest)
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        player('command', command='clear @s')
        player('command', command='effect clear @s')
        # A cow two blocks ahead that stays put, and a framed diamond on a wall behind the player.
        player('command', command=f'summon minecraft:cow {x + 0.5} {y} {z + 3.5} {{NoAI:1b}}')
        player('command', command=f'fill {x - 1} {y} {z - 2} {x + 1} {y + 2} {z - 2} minecraft:stone')
        player('command', command=f'summon minecraft:item_frame {x + 0.5} {y + 1.5} {z - 0.97} '
               f'{{block_pos:[I;{x},{y + 1},{z - 1}],Facing:3b,Item:{{id:"minecraft:diamond",count:1}}}}')
        player.wait(20)
        counts = lambda: player('inventory')['counts']
        aim_cow = lambda: player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 0.8:.2f}', f'{z + 3.5:.2f}')
        try:
            player('command', command='give @s minecraft:bow')
            player('command', command='give @s minecraft:arrow 6')
            player.wait(5)
            player.raw('select', 'minecraft:bow')
            aim_cow()
            assert player('looking_at')['kind'] == 'entity', f'the {label} does not aim at the cow'
            # Half drawn: a full draw is a crit, up to 10 damage (AbstractArrow.onHitEntity), which
            # kills the 10 health cow one shot in five. Under 20 ticks the arrow does at most 6.
            player.raw('mouse', 'right', 'down')
            player.wait(10)
            drawn = player('state')['using']
            player.raw('mouse', 'right', 'up')
            player.wait(20)
            assert drawn, f'the {label}\'s bow is not drawn at the cow'
            assert counts().get('minecraft:arrow') == 5, f'the {label} shot no arrow: {counts()}'
            aim_cow()
            cow = player('looking_at').get('entity') or {}
            assert cow.get('health', 0) > 0, f'the {label}\'s arrow killed the cow: {cow}'
            player('command', command='clear @s')
            player('command', command='give @s minecraft:golden_apple')
            player.wait(5)
            player.raw('select', 'minecraft:golden_apple')
            aim_cow()
            player.raw('mouse', 'right', 'down')
            player.wait(5)
            player.raw('mouse', 'right', 'up')
            player.wait(40)
            assert counts().get('minecraft:golden_apple') == 1, f'the {label} ate the apple let go of: {counts()}'
            # The main hand empty, a bucket and then wheat in the off hand.
            player('command', command='clear @s')
            player('command', command='item replace entity @s weapon.offhand with minecraft:bucket')
            player.wait(5)
            aim_cow()
            player.raw('mouse', 'right', 'click')
            player.wait(15)
            offhand = player('inventory')['offhand']
            assert offhand and offhand['item'] == 'minecraft:milk_bucket', f'the {label}\'s off hand holds {offhand}'
            player('command', command='item replace entity @s weapon.offhand with minecraft:wheat 3')
            player.wait(5)
            aim_cow()
            player.raw('mouse', 'right', 'click')
            player.wait(15)
            offhand = player('inventory')['offhand']
            assert offhand and offhand['count'] == 2, f'the {label}\'s cow ate no off hand wheat: {offhand}'
            # The framed diamond, punched out.
            frame = [o for o in player('status')['objects'] if o['kind'].endswith('item_frame')
                     and o['position'] == [x, y + 1, z - 1]]
            assert frame and frame[0]['item'] == 'minecraft:diamond', f'the {label} sees the frame as {frame}'
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.5:.2f}', f'{z - 0.95:.2f}')
            player.raw('mouse', 'left', 'click')
            player.wait(20)
            for side, game in ((label, player), ('other side', other)):
                seen = [o for o in game('status')['objects'] if o['kind'].endswith('item_frame')
                        and o['position'] == [x, y + 1, z - 1]]
                assert seen and seen[0].get('item') is None, f'the {side} sees the punched frame as {seen}'
            result[label] = {'bow': 'shot', 'apple': 'kept', 'off hand': 'milked, fed', 'frame': 'emptied'}
        finally:
            player('command', command=f'kill @e[type=!minecraft:player,x={x - 3},y={y - 1},z={z - 3},dx=6,dy=4,dz=8]')
            player('command', command=f'fill {x - 1} {y} {z - 2} {x + 1} {y + 2} {z - 2} minecraft:air')
            player('command', command='clear @s')
    return result


# Animals a player rides: (kind, summon NBT, what readies it, what is held to ride, the test
# while riding). `move` walks forward, `jump` charges the horse's jump bar, `rise` holds jump on
# a happy ghast, `sit` only rides (a llama goes where it likes).
RIDES = [
    ('horse', '{Tame:1b}', 'minecraft:saddle', None, 'move jump'),
    ('pig', '', 'minecraft:saddle', 'minecraft:carrot_on_a_stick', 'move'),
    ('strider', '', 'minecraft:saddle', 'minecraft:warped_fungus_on_a_stick', 'move'),
    ('camel', '', 'minecraft:saddle', None, 'move'),
    ('donkey', '{Tame:1b}', 'minecraft:saddle', None, 'move'),
    ('llama', '{Tame:1b}', None, None, 'sit'),
    ('happy_ghast', '', 'minecraft:white_harness', None, 'rise'),
]
HEIGHTS = {'horse': 1.6, 'donkey': 1.5, 'pig': 0.9, 'strider': 1.7, 'camel': 2.375, 'llama': 1.87,
           'happy_ghast': 4.0}


def mob_seen(game, mob_id):
    found = [m for m in game.raw('mobs', 96).get('near', []) if m['id'] == mob_id]
    return found[0] if found else None


def hold(game, key, ticks):
    game.raw('key', key, 'down')
    game.wait(ticks)
    game.raw('key', key, 'up')


def ride(player, other, label, spot, kind, nbt, ready, stick, test):
    """Summon `kind` in front of `player`, ready it (saddle, harness), get on with a right click,
    run `test`, and get off by sneaking. The other side sees the animal move."""
    x, y, z = spot
    # In reach (3 blocks), clear of the player: a happy ghast is 4 blocks wide.
    at = (x + 0.5, z + {'happy_ghast': 3.2, 'camel': 3.0}.get(kind, 2.5))
    # The first item in hand before it appears: a happy ghast drifts out of reach quickly.
    player('command', command=f'item replace entity @s weapon.mainhand with {ready or stick or "minecraft:air"}')
    player('command', command=f'summon minecraft:{kind} {at[0]} {y} {at[1]} {nbt}')
    player.wait(3)
    found = [m for m in player.raw('mobs', 16).get('near', []) if m['kind'].endswith(kind) and m['health'] > 0]
    assert found, f'the {label} sees no {kind}'
    mob = found[0]
    def click(item):
        player('command', command=f'item replace entity @s weapon.mainhand with {item or "minecraft:air"}')
        player.wait(2)
        m = mob_seen(player, mob['id'])
        assert m, f'the {label} lost its {kind}'
        p = m['pos']
        # Back in front of it if it wandered off (a happy ghast drifts away fast).
        if abs(p[0] - (x + 0.5)) > 0.5 or abs(p[2] - at[1]) > 0.5:
            player('command', command=f'tp @s {p[0]:.2f} {y} {p[2] - (at[1] - z - 0.5):.2f} 0 0')
            player.wait(1)
        player.raw('look-at', f'{p[0]:.2f}', f'{p[1] + HEIGHTS[kind] / 2:.2f}', f'{p[2]:.2f}')
        player.raw('mouse', 'right', 'click')
        player.wait(5)
    try:
        if ready:
            click(ready)
        click(stick)
        riding = player('status').get('riding')
        assert riding == mob['id'], f'the {label} right-clicked the {kind} {mob["id"]}: riding {riding}'
        result = {'rode': True}
        # South, along the cleared floor.
        player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.6:.2f}', f'{z + 40.5:.2f}')
        player.wait(5)
        start = mob_seen(player, mob['id'])['pos']
        if 'move' in test:
            hold(player, 'w', 60)
            player.wait(5)
            mine = mob_seen(player, mob['id'])['pos']
            theirs = mob_seen(other, mob['id'])
            moved = math.dist(start, mine)
            assert moved > 1.0, f"the {label}'s {kind} moved {moved:.2f} blocks in 3 s of forward"
            assert theirs and math.dist(theirs['pos'], mine) < 1.5, \
                f"the other side sees the {label}'s {kind} at {theirs and theirs['pos']}, not {mine}"
            seated = pos_of(player)
            assert math.dist(seated[::2], mine[::2]) < 1.5, f'the {label} sits at {seated}, its {kind} at {mine}'
            result['moved'] = round(moved, 2)
        if 'jump' in test:
            base = mob_seen(player, mob['id'])['pos'][1]
            player.raw('key', 'space', 'down')
            player.wait(10)
            player.raw('key', 'space', 'up')
            top = base
            for _ in range(10):
                player.wait(2)
                top = max(top, mob_seen(player, mob['id'])['pos'][1])
            assert top - base > 0.4, f"the {label}'s {kind} jumped {top - base:.2f} blocks"
            result['jumped'] = round(top - base, 2)
        if 'rise' in test:
            base = mob_seen(player, mob['id'])['pos'][1]
            hold(player, 'space', 40)
            player.wait(5)
            rose = mob_seen(player, mob['id'])['pos'][1] - base
            assert rose > 0.5, f"the {label}'s {kind} rose {rose:.2f} blocks while it held jump"
            result['rose'] = round(rose, 2)
        hold(player, 'shift', 10)
        player.wait(10)
        riding = player('status').get('riding')
        assert riding is None, f'sneaking off the {kind}: the {label} still rides {riding}'
        return result
    finally:
        player('command', command=f'kill @e[type=minecraft:{kind},x={x},y={y},z={z},distance=..48]')
        player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
        player.wait(5)


def check_riding(host, guest, out):
    """The host (single player, on its server thread or not) and the guest each saddle and ride a
    horse (walk and jump), a pig and a strider (with their sticks), a camel and a donkey, sit on a
    llama, and harness and fly a happy ghast; they get off by sneaking."""
    spots = apart(host, guest)
    result = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        player('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 40} minecraft:stone')
        player('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 12} {z + 40} minecraft:air')
        player.wait(10)
        result[label] = {}
        # LAN_E2E_RIDES=pig,horse picks some of them.
        only = [k for k in os.environ.get('LAN_E2E_RIDES', '').split(',') if k]
        for kind, nbt, ready, stick, test in (r for r in RIDES if not only or r[0] in only):
            result[label][kind] = ride(player, other, label, (x, y, z), kind, nbt, ready, stick, test)
        player('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    return result


def check_occupied_horse(host, guest, out):
    """#1759: a mob passenger fills a zombie horse's seat, for both real players."""
    spots = apart(host, guest)
    result = {}
    tag = 'occupied_horse'
    host('command', command='time set midnight')
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        at = (x + 0.5, y, z + 2.5)
        player('command', command='gamemode creative')
        player('command', command='clear @s')
        result[label] = {}
        for occupied in (True, False):
            passengers = ',Passengers:[{id:"minecraft:zombie",NoAI:1b,PersistenceRequired:1b}]' if occupied else ''
            try:
                player('command', command=f'summon minecraft:zombie_horse {at[0]} {y} {at[2]} '
                                         f'{{Tame:1b,NoAI:1b,NoGravity:1b,Tags:["{tag}"]{passengers}}}')
                player.wait(5)
                horse = next(m for m in player.raw('mobs', 16)['near']
                             if m['kind'] == 'minecraft:zombie_horse' and m['health'] > 0
                             and math.dist(m['pos'], at) < 0.1)
                if occupied:
                    # Assert the setup too: a missing passenger would make this a false positive.
                    passengers_nbt = host('command', command=f'data get entity @e[tag={tag},limit=1] Passengers')
                    assert 'minecraft:zombie' in json.dumps(passengers_nbt), passengers_nbt
                player.raw('look-at', at[0], y + 0.8, at[2])
                player.raw('mouse', 'right', 'click')
                player.wait(10)
                riding = player('status').get('riding')
                expected = None if occupied else horse['id']
                assert riding == expected, f'{label}: occupied={occupied}, riding={riding}, expected={expected}'
                result[label]['occupied' if occupied else 'free'] = riding
                player.shot(out, f'occupied-horse-{label}-{occupied}')
                if not occupied:
                    hold(player, 'shift', 5)
            finally:
                player('command', command=f'kill @e[tag={tag}]')
                player('command', command=f'kill @e[type=minecraft:zombie,x={x},y={y},z={z},distance=..6]')
                player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
                player.wait(25)
    return result


# Mount inventories: (kind, summon NBT, what goes in by shift-click and the menu slot it lands
# in: 700 the saddle, 701 the body armour or carpet, 710 the chest's first slot).
MOUNT_MENUS = [
    ('horse', '{Tame:1b,NoAI:1b}', [('minecraft:saddle', 700), ('minecraft:iron_horse_armor', 701)]),
    ('donkey', '{Tame:1b,NoAI:1b,ChestedHorse:1b}', [('minecraft:saddle', 700), ('minecraft:dirt', 710)]),
    ('llama', '{Tame:1b,NoAI:1b,ChestedHorse:1b,Strength:3}',
     [('minecraft:red_carpet', 701), ('minecraft:dirt', 710)]),
]


def mount_screen(game, ticks=20):
    """The open mount menu's slots and items ({700: 'minecraft:saddle', ...}), waiting up to
    `ticks` for it to open; None when no mount menu opened."""
    for _ in range(ticks // 2):
        slots = {}
        for w in game.raw('ui').get('widgets', []):
            wid = w.get('id', '')
            if wid.startswith('slot-') and wid[5:].isdigit() and int(wid[5:]) >= 700:
                slots[int(wid[5:])] = (w.get('stack') or {}).get('item')
        if slots:
            return slots
        game.wait(2)
    return None


def check_mounts(host, guest, out):
    """The host and the guest each open a tamed horse's, donkey's and llama's inventory with
    shift + right click (`ClientboundMountScreenOpenPacket` for a server's player), put on a
    saddle, horse armour or a carpet and fill the chest by shift-clicking, close it, then ride the
    mount and open its inventory with the inventory key (OPEN_INVENTORY): it shows what was put
    in, and the server took the items from the inventory."""
    spots = apart(host, guest)
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        player('command', command='gamemode survival')
        result[label] = {}
        for kind, nbt, puts in MOUNT_MENUS:
            player('command', command='clear @s')
            for item, _ in puts:
                player('command', command=f'give @s {item}')
            player('command', command=f'summon minecraft:{kind} {x + 0.5} {y} {z + 2.5} {nbt}')
            player.wait(5)
            found = [m for m in player.raw('mobs', 16).get('near', [])
                     if m['kind'].endswith(kind) and m['health'] > 0]
            assert found, f'the {label} sees no {kind}'
            mob = found[0]
            p = mob['pos']
            aim = (f'{p[0]:.2f}', f'{p[1] + HEIGHTS[kind] / 2:.2f}', f'{p[2]:.2f}')
            try:
                player.raw('look-at', *aim)
                player.raw('key', 'shift', 'down')
                player.wait(3)
                player.raw('mouse', 'right', 'click')
                player.wait(5)
                player.raw('key', 'shift', 'up')
                slots = mount_screen(player)
                assert slots is not None, f"the {label}'s shift + right click on the {kind} opened no inventory"
                for item, _ in puts:
                    player.raw('click-slot', hotbar_slot(player, item), 'shift')
                    player.wait(5)
                slots = mount_screen(player)
                for item, slot in puts:
                    assert slots.get(slot) == item, f"the {label}'s {kind}: {item} shift-clicked, slot {slot} holds {slots.get(slot)}"
                player.raw('key', 'esc', 'tap')
                player.wait(5)
                for item, _ in puts:
                    left = server_count(player, item.split(':')[1])
                    assert left == 0, f'the server still counts {left} {item} on the {label}'
                player.raw('look-at', *aim)
                player.raw('mouse', 'right', 'click')
                player.wait(8)
                riding = player('status').get('riding')
                assert riding == mob['id'], f'the {label} right-clicked its {kind}: riding {riding}'
                player.raw('key', 'e', 'tap')
                again = mount_screen(player)
                assert again is not None, f'the inventory key on the {label}\'s {kind} opened no inventory'
                for item, slot in puts:
                    assert again.get(slot) == item, f"the {label}'s {kind} reopened: slot {slot} holds {again.get(slot)}, not {item}"
                player.raw('key', 'esc', 'tap')
                player.wait(3)
                hold(player, 'shift', 10)
                player.wait(5)
                result[label][kind] = {slot: item for item, slot in puts}
            finally:
                back_to_game(player)
                player('command', command=f'kill @e[type=minecraft:{kind},x={x},y={y},z={z},distance=..16]')
                player.wait(2)
                player('command', command=f'kill @e[type=minecraft:item,x={x},y={y},z={z},distance=..16]')
                player('command', command='clear @s')
                player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
                player.wait(5)
    return result


def check_lightning(host, guest, out):
    """A bolt summoned beside each player: both sides get the bolt entity and flash the sky (the
    host's window too on its server thread, FEATURES 4z(f))."""
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = (int(v // 1) for v in pos_of(player))
        before = {g.name: g('status')['lightning'] for g in (host, guest)}
        # No fire where it strikes: on stone with nothing around.
        player('command', command=f'fill {x + 4} {y - 1} {z - 2} {x + 8} {y - 1} {z + 2} minecraft:stone')
        player('command', command=f'fill {x + 4} {y} {z - 2} {x + 8} {y + 3} {z + 2} minecraft:air')
        player('command', command=f'summon minecraft:lightning_bolt {x + 6.5} {y} {z + 0.5}')
        seen = {}
        for _ in range(40):
            player.wait(1)
            seen = {g.name: g('status')['lightning'] for g in (host, guest)}
            if all(seen[n]['struck'] > before[n]['struck'] and seen[n]['flashes'] > before[n]['flashes']
                   for n in seen):
                break
        player('command', command=f'fill {x + 4} {y} {z - 2} {x + 8} {y + 3} {z + 2} minecraft:air')
        for n in seen:
            assert seen[n]['struck'] > before[n]['struck'], f"{label}'s bolt: the {n} got no bolt ({seen[n]})"
            assert seen[n]['flashes'] > before[n]['flashes'], f"{label}'s bolt: the {n}'s sky didn't flash ({seen[n]})"
        result[label] = {n: {'bolts': seen[n]['struck'] - before[n]['struck'],
                             'flashes': seen[n]['flashes'] - before[n]['flashes']} for n in seen}
    return result


def check_growth(host, guest, out):
    """Block ticks near a guest 1000 blocks from the host: only the guest's tickets tick them."""
    x0, y0, z0 = pos_of(guest)
    guest('command', command='gamemode creative')
    guest('command', command=f'tp @s {x0 + 1000:.2f} 200 {z0:.2f}')
    guest.wait(100)
    x, y, z = (int(v // 1) for v in pos_of(guest))
    # A platform in the air over wherever the guest landed: a water source two blocks east of the
    # guest, wheat on farmland two blocks west. Vanilla's water spreads only towards the nearest
    # drop (FlowingFluid.getSpread), so any of the source's four neighbours may take it. Wheat
    # grows only in daylight (raw brightness 9 or more).
    top = max(y + 12, 120)
    water = (x + 2, top + 1, z)
    beside = [(water[0] + dx, water[1], water[2] + dz) for dx, dz in ((1, 0), (-1, 0), (0, 1), (0, -1))]
    wheat = (x - 2, top + 1, z)
    flowed = []
    try:
        host('command', command='time set noon')
        guest('command', command=f'fill {x - 4} {top} {z - 2} {x + 4} {top} {z + 2} minecraft:stone')
        guest('command', command=f'setblock {wheat[0]} {top} {wheat[2]} minecraft:farmland[moisture=7]')
        guest('command', command=f'setblock {wheat[0]} {wheat[1]} {wheat[2]} minecraft:wheat[age=0]')
        guest('command', command='gamerule random_tick_speed 1000')
        guest('command', command=f'setblock {water[0]} {water[1]} {water[2]} minecraft:water')
        for _ in range(40):
            guest.wait(10)
            flowed = [p for p in beside if guest.raw('block', *p).get('block') == 'minecraft:water']
            if flowed:
                break
        age = wait_state(guest, wheat, 'age', '7', 2400)
        guest.shot(out, 'growth')
    finally:
        guest('command', command='gamerule random_tick_speed 3')
        host('command', command='time set day')
        guest('command', command=f'tp @s {x0:.2f} {y0:.2f} {z0:.2f}')
        guest('command', command='gamemode survival')
        guest.wait(40)
    detail = {'flowed': flowed, 'wheat age': age}
    assert flowed, f'the water beside the guest did not flow: {detail}'
    assert age == '7', f'the wheat beside the guest did not grow: {detail}'
    return detail


def check_doors(host, guest, out):
    """A villager shut in a room opens its oak door on its way to a free composter outside
    (InteractWithDoor), takes the composter as its job site and closes the door behind it; the
    guest sees the door open and close again."""
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, y, z = gx + 4, gy, gz - 8
    lower, upper, composter, marker = (x + 2, y, z), (x + 2, y + 1, z), (x + 7, y, z), (x + 7, y, z + 2)
    tag = 'dg_door_villager'
    farmer = (f'execute if entity @e[type=minecraft:villager,tag={tag},'
              f'nbt={{VillagerData:{{profession:"minecraft:farmer"}}}}] run setblock {marker[0]} {marker[1]} {marker[2]} '
              'minecraft:gold_block')
    seen = {'opened': False, 'farmer': False, 'closed': None}
    try:
        host('command', command='time set 2000')
        host('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 9} {y - 1} {z + 3} minecraft:stone')
        host('command', command=f'fill {x - 3} {y} {z - 3} {x + 9} {y + 3} {z + 3} minecraft:air')
        host('command', command=f'fill {x - 2} {y} {z - 2} {x + 2} {y + 2} {z + 2} minecraft:oak_planks')
        host('command', command=f'fill {x - 1} {y} {z - 1} {x + 1} {y + 1} {z + 1} minecraft:air')
        for (bx, by, bz), half in ((lower, 'lower'), (upper, 'upper')):
            host('command', command=f'setblock {bx} {by} {bz} minecraft:oak_door[facing=east,half={half},'
                                    'hinge=left,open=false,powered=false]')
        host('command', command=f'setblock {composter[0]} {composter[1]} {composter[2]} minecraft:composter')
        host('command', command=f'summon minecraft:villager {x + 0.5} {y} {z + 0.5} {{Tags:["{tag}"],'
                                'PersistenceRequired:1b,VillagerData:{profession:"minecraft:none",level:1,'
                                'type:"minecraft:plains"}}')
        for _ in range(120):
            guest.wait(10)
            seen['opened'] = seen['opened'] or state_of(guest, lower, 'open') == 'true'
            host('command', command=farmer)
            if host.raw('block', *marker).get('block') == 'minecraft:gold_block':
                seen['farmer'] = True
                break
        seen['closed'] = wait_state(guest, lower, 'open', 'false', 200) == 'false'
        seen['server_door'] = state_of(host, lower, 'open')
        guest.raw('look-at', f'{lower[0] + 0.5:.2f}', f'{lower[1] + 1:.2f}', f'{lower[2] + 0.5:.2f}')
        guest.wait(10)
        guest.shot(out, 'doors')
    finally:
        host('command', command=f'kill @e[type=minecraft:villager,tag={tag}]')
        host('command', command=f'fill {x - 3} {y} {z - 3} {x + 9} {y + 3} {z + 3} minecraft:air')
        host('command', command='time set day')
    assert seen['opened'], f'the villager never opened its door: {seen}'
    assert seen['farmer'], f'the villager did not take the composter outside: {seen}'
    assert seen['closed'] and seen['server_door'] == 'false', f'the villager left its door open: {seen}'
    return seen



def check_light(host, guest, out):
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    results = {}
    # A torch near the guest: the guest lights it itself, as vanilla's client does.
    torch = (gx + 3, gy, gz + 3)
    host('command', command=f'setblock {torch[0]} {torch[1]} {torch[2]} minecraft:torch')
    try:
        results['torch'] = same_light(host, guest, (gx - 12, gy - 1, gz - 12), (gx + 16, gy + 3, gz + 16), 'torch')
    finally:
        host('command', command=f'setblock {torch[0]} {torch[1]} {torch[2]} minecraft:air')
    # A roof over the guest, then a hole in it: sky light falls through.
    roof = f'{gx - 3} {gy + 4} {gz - 3} {gx + 3} {gy + 4} {gz + 3}'
    box = (gx - 6, gy - 1, gz - 6), (gx + 6, gy + 5, gz + 6)
    host('command', command=f'fill {roof} minecraft:stone')
    try:
        results['roof'] = same_light(host, guest, *box, 'roof')
        host('command', command=f'setblock {gx} {gy + 4} {gz} minecraft:air')
        results['hole'] = same_light(host, guest, *box, 'hole in the roof')
    finally:
        host('command', command=f'fill {roof} minecraft:air')
    # The last chunk the guest tracks eastwards, and glowstone high up just past it in a chunk it
    # doesn't track: the guest never sees that block, only the light updates of its border chunk.
    y = 200
    row = guest.raw('light', gx, y, gz, gx + 16 * 24, y, gz)['sky']
    assert '.' in row, 'the guest has every chunk 24 chunks east'
    untracked = (gx + row.index('.')) >> 4
    near = untracked * 16 + 1
    # On its server thread the host's own game is a client too, sent the same light updates.
    if not SERVER_THREAD:
        assert host.raw('light', near, y, gz, near, y, gz)['sky'] != '.', 'the host lacks the chunk past the guest'
    host('command', command=f'setblock {near} {y} {gz} minecraft:glowstone')
    try:
        lo, hi = (untracked * 16 - 16, y - 3, gz - 8), (untracked * 16 - 1, y + 3, gz + 8)
        # On the server thread the host's game is a client too, as dark as the guest until the light
        # update arrives: wait for it before comparing them.
        for _ in range(10):
            lit = guest.raw('light', near - 2, y, gz, near - 2, y, gz)['block']
            if lit == 'd':
                break
            host.wait(10)
        assert lit == 'd', f'the border chunk by the glowstone has block light {lit}, not 13, after 100 ticks'
        results['border'] = same_light(host, guest, lo, hi, 'border chunk')
    finally:
        host('command', command=f'setblock {near} {y} {gz} minecraft:air')
    results['border_cleared'] = same_light(host, guest, lo, hi, 'border chunk without the glowstone')
    return results


def check_fireball(host, guest, out):
    x, y, z = (int(v // 1) for v in pos_of(host))
    fx, fz = x + 4, z
    # A stone floor, a dispenser facing down three blocks over it, a fire charge in it, powered.
    host('command', command=f'fill {fx - 2} {y - 1} {fz - 2} {fx + 2} {y - 1} {fz + 2} minecraft:stone')
    host('command', command=f'fill {fx - 2} {y} {fz - 2} {fx + 2} {y + 4} {fz + 2} minecraft:air')
    host('command', command=f'setblock {fx} {y + 3} {fz} minecraft:dispenser[facing=down]')
    host('command', command=f'item replace block {fx} {y + 3} {fz} container.0 with minecraft:fire_charge')
    seen, gone = {}, None
    try:
        host('command', command=f'setblock {fx} {y + 4} {fz} minecraft:redstone_block')
        host.wait(10)
        seen = {'host': host.raw('block', fx, y, fz).get('block'),
                'guest': guest.raw('block', fx, y, fz).get('block')}
        # FireBlock.tick every 30-39 ticks ages it by one a third of the time; past age 3 it goes.
        for waited in range(40, 1240, 40):
            host.wait(40)
            if host.raw('block', fx, y, fz).get('block') == 'minecraft:air':
                gone = waited
                break
        guest.wait(5)
        seen['guest_after'] = guest.raw('block', fx, y, fz).get('block')
    finally:
        host('command', command=f'fill {fx - 2} {y} {fz - 2} {fx + 2} {y + 4} {fz + 2} minecraft:air')
    assert seen.get('host') == 'minecraft:fire' and seen.get('guest') == 'minecraft:fire', \
        f'no fire where the fire charge landed: {seen}'
    assert gone is not None, f'the fire on stone never went out (1200 ticks): {seen}'
    assert seen['guest_after'] == 'minecraft:air', f'the guest still sees the fire: {seen}'
    return {'out_after_ticks': gone, **seen}


def check_deflect(host, guest, out):
    """The host punches a ghast's fireball back: it flies along the host's view and kills the ghast
    (Player.attack: AIM_DEFLECT on #redirectable_projectile; Ghast.isReflectedFireball). The ghast is caged
    (a window toward the host); the game runs at 4 ticks a second, and once the fireball is within 4.5 blocks
    the host looks at the ghast and clicks."""
    back = pos_of(host)
    x, y, z = (int(v // 1) for v in back)
    guest('command', command='gamemode creative')
    host('command', command='gamemode survival')
    host('command', command='difficulty normal')
    gx, gz = x, z + 30
    gy = y + 20
    host('command', command=f'fill {gx - 3} {gy - 1} {gz - 3} {gx + 24} {gy - 1} {gz + 3} minecraft:glass')
    host('command', command=f'fill {gx - 3} {gy} {gz - 3} {gx + 24} {gy + 8} {gz + 3} minecraft:air')
    host('command', command=f'tp @s {gx + 0.5} {gy} {gz + 0.5} -90 0')
    host.wait(20)
    host.raw('hurtlog')
    # The ghast in a glass cage it can't drift out of, a 3 x 3 window toward the host.
    host('command', command=f'fill {gx + 13} {gy} {gz - 3} {gx + 19} {gy + 6} {gz + 3} minecraft:glass hollow')
    host('command', command=f'fill {gx + 13} {gy + 2} {gz - 1} {gx + 13} {gy + 4} {gz + 1} minecraft:air')
    host('command', command=f'summon minecraft:ghast {gx + 16.5} {gy + 1} {gz + 0.5}')
    eye = (gx + 0.5, gy + 1.62, gz + 0.5)
    host('command', command='tick rate 4')
    turned, near, ghast, hurt = None, None, None, []
    try:
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            # Aim and click at the projectile the client actually sees. Holding
            # non-player simulation keeps the reach window open while ordinary
            # player input and its attack packet cross the connection.
            balls = [q for q in host('status')['projectiles'] if q['kind'] == 'LargeFireball']
            near = min((math.dist(q['position'], eye) for q in balls), default=None)
            if near is not None and near < 4.5:
                ball = min(balls, key=lambda q: math.dist(q['position'], eye))
                host('command', command='tick freeze')
                held_tick = host.raw('simulation')['game_time']
                g = [m['pos'] for m in host.raw('mobs', 60).get('near', []) if m['kind'].endswith('ghast')][0]
                host.script([f'look-at {g[0]:.2f} {g[1] + 2:.2f} {g[2]:.2f}',
                             'wait 1', 'mouse left click', 'wait 1'])
                host.until(lambda: host.raw('simulation')['projectiles'],
                           lambda qs: any(q.get('id') == ball['id'] and q.get('player_owned') for q in qs),
                           ticks=60)
                assert host.raw('simulation')['game_time'] == held_tick, 'the deflection barrier advanced simulation'
                host('command', command='tick unfreeze')
                turned = near
                break
        host('command', command='tick rate 20')
        host.wait(60)
        ghast = [m for m in host.raw('mobs', 60).get('near', []) if m['kind'].endswith('ghast') and m['health'] > 0]
        hurt = [h for pl in host.raw('hurtlog')['players'] if pl['name'] == (host.player if SERVER_THREAD else 'host') for h in pl['hurts']
                if 'fireball' in h['source'] or 'explosion' in h['source']]
    finally:
        host('command', command='tick unfreeze')
        host('command', command='tick rate 20')
        host('command', command='kill @e[type=minecraft:ghast]')
        host('command', command=f'tp @s {back[0]} {back[1]} {back[2]}')
        host.until(lambda: pos_of(host), lambda p: math.dist(p, back) < 0.1)
        host('command', command=f'fill {gx - 3} {gy - 1} {gz - 3} {gx + 24} {gy + 8} {gz + 3} minecraft:air')
        guest('command', command='gamemode survival')
    assert turned is not None, f'no fireball came within 4.5 blocks (nearest {near})'
    assert not hurt, f'the host was hit: {hurt}'
    assert not ghast, f'the ghast lived: the fireball was not turned back ({ghast})'
    return {'struck_at': round(turned, 2)}


def check_barter(host, guest, out):
    """The guest, in gold boots, drops a gold ingot to a piglin: the piglin stays calm, takes it, and throws its
    barter toward the nearest player, the guest (PiglinAi.throwItemsTowardPlayer: NEAREST_VISIBLE_PLAYER), who
    picks it up. The host is nearby on the other side, so a barter thrown at the host misses the guest. A glass
    corridor (the hollow box) keeps the piglin between them."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    cz = z + 4
    host('command', command='gamemode creative')
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    guest('command', command='effect give @s minecraft:instant_health 1 5')
    host('command', command=f'fill {x + 1} {y - 1} {cz - 1} {x + 8} {y + 2} {cz + 1} minecraft:glass hollow')
    host('command', command=f'fill {x + 2} {y - 1} {cz} {x + 7} {y - 1} {cz} minecraft:stone')
    host('command', command=f'tp @s {x - 0.5} {y} {cz + 0.5} -90 0')
    guest('command', command=f'tp @s {x + 7.3} {y} {cz + 0.5} 90 30')
    guest('command', command='item replace entity @s armor.feet with minecraft:golden_boots')
    guest('command', command='give @s minecraft:gold_ingot 1')
    host('command', command=f'summon minecraft:piglin {x + 4.5} {y} {cz + 0.5} {{IsImmuneToZombification:1b}}')
    guest.wait(20)
    before = remote(guest)['health']
    kept = dict(guest('inventory')['counts'])
    try:
        guest.raw('select', 'minecraft:gold_ingot')
        guest.raw('key', 'q', 'tap')
        bought, trace = {}, []
        for _ in range(20):
            guest.wait(20)
            trace.append([(e['type'].split(':')[-1], round(e['pos'][0] - x, 1), round(e['pos'][2] - cz, 1))
                          for e in host.raw('entities', 24)['entities'] if e['type'] in ('minecraft:item', 'minecraft:piglin')])
            counts = guest('inventory')['counts']
            bought = {k: n - kept.get(k, 0) for k, n in counts.items()
                      if k not in ('minecraft:gold_ingot', 'minecraft:golden_boots') and n > kept.get(k, 0)}
            if bought:
                guest.wait(40)
                break
        counts = guest('inventory')['counts']
        bought = {k: n - kept.get(k, 0) for k, n in counts.items()
                  if k not in ('minecraft:gold_ingot', 'minecraft:golden_boots') and n > kept.get(k, 0)}
        piglins = [m for m in host.raw('mobs', 20).get('near', []) if m['kind'].split(':')[-1] == 'piglin']
        health = remote(guest)['health']
    finally:
        host('command', command=f'kill @e[type=minecraft:piglin,x={x},y={y - 2},z={cz - 2},dx=10,dy=6,dz=4]')
        # What lies on the floor, on the host's side of the piglin's start and on the guest's.
        floor = [host('command', command=f'kill @e[type=minecraft:item,x={x0},y={y - 2},z={cz - 3},dx={dx},dy=6,dz=6]')
                 ['chat'][0] for x0, dx in ((x - 2, 6.5), (x + 4.5, 6))]
        host('command', command=f'fill {x + 1} {y - 1} {cz - 1} {x + 8} {y + 2} {cz + 1} minecraft:air')
        host('command', command=f'fill {x - 8} {y - 1} {z - 8} {x + 8} {y - 1} {z + 8} minecraft:stone')
        guest('command', command='clear @s')
        host('command', command='gamemode survival')
    assert piglins, 'the piglin is gone'
    assert health >= before, f'the piglin hurt the guest in gold boots: {before} -> {health}'
    assert bought, f'no barter reached the guest (on the floor, host side then guest side: {floor}; items and the piglin, every second: {trace})'
    return {'bought': bought, 'floor': floor}


def check_worn(host, guest, out):
    pieces = ['iron_helmet', 'iron_chestplate', 'iron_leggings', 'iron_boots']
    guest('command', command='gamemode survival')
    taken = {}
    try:
        for worn in (False, True):
            guest('command', command='clear @s')
            if remote(guest)['health'] <= 0:
                guest.raw('respawn')
                guest.wait(40)
            if worn:
                for piece in pieces:
                    guest('command', command=f'give @s minecraft:{piece}')
                guest.wait(10)
                # Shift-clicked from the hotbar onto the body in the inventory menu (the hub moves them).
                guest.raw('key', 'e', 'tap')
                guest.raw('wait-for', 'screen', 'inventory')
                for piece in pieces:
                    guest.raw('click-slot', hotbar_slot(guest, f'minecraft:{piece}'), 'shift')
                    guest.wait(2)
                guest.raw('key', 'esc', 'tap')
                guest.wait(5)
            guest('command', command='effect give @s minecraft:instant_health 1 5')
            guest.wait(30)
            before = remote(guest)['health']
            if not SERVER_THREAD:
                host.raw('hurtlog')
            # A mob's blow (no attacker: no difficulty scaling); armour counts against it.
            guest('command', command='damage @s 10 minecraft:mob_attack')
            if SERVER_THREAD:
                # The server's hurt log isn't on a control socket here: the health the guest is
                # sent, read as soon as it drops (natural regeneration heals it again after).
                health = before
                for _ in range(40):
                    guest.wait(1)
                    health = remote(guest)['health']
                    if health < before:
                        break
                taken['worn' if worn else 'bare'] = [round(before - health, 2)]
                continue
            guest.wait(10)
            log = [h for p in host.raw('hurtlog')['players'] if p['name'] == GUEST_NAME for h in p['hurts']]
            taken['worn' if worn else 'bare'] = [(h['amount'], h['armor'], round(h['taken'], 2)) for h in log]
    finally:
        guest('command', command='clear @s')
        guest('command', command='effect give @s minecraft:instant_health 1 5')
    if SERVER_THREAD:
        # Natural regeneration may heal up to a point before the health arrives.
        assert len(taken['bare']) == 1 and 9.0 <= taken['bare'][0] <= 10.0, f'10 damage on no armour: {taken}'
        assert len(taken['worn']) == 1 and 5.0 <= taken['worn'][0] <= 6.0, \
            f'10 damage through iron armour (15) should take 6: {taken}'
        return taken
    assert taken['bare'] == [(10.0, 0.0, 10.0)], f'10 damage on no armour: {taken}'
    assert taken['worn'] == [(10.0, 15.0, 6.0)], f'10 damage through iron armour (15) should take 6: {taken}'
    return taken


CHECK_FNS = {'tab': check_tab, 'self': check_self, 'armour': check_armour,
             'nametags': check_nametags, 'chest': check_chest, 'crafting': check_crafting,
             'spawned': check_spawned,
             'sign': check_sign, 'damage': check_damage, 'mobs': check_mobs,
             'flow-push': check_flow_push,
             'bubble-column': check_bubble_column,
             'hazards': check_hazards, 'pvp': check_pvp, 'respawn': check_respawn,
             'containers': check_containers, 'names': check_names, 'feedback': check_feedback, 'titles': check_titles,
             'chat': check_chat, 'styles': check_styles, 'machines': check_machines, 'brewing': check_brewing,
             'loom': check_loom, 'stonecutter': check_stonecutter, 'grindstone': check_grindstone,
             'anvil': check_anvil, 'smithing': check_smithing, 'enchanting': check_enchanting,
             'beacon': check_beacon, 'crafter': check_crafter, 'lectern': check_lectern,
             'cartography': check_cartography, 'hurt': check_hurt, 'commands': check_commands,
             'recipes': check_recipes, 'trials': check_trials, 'walls': check_walls,
             'effects': check_effects, 'portals': check_portals, 'distance': check_distance,
             'clicks': check_clicks, 'sleep': check_sleep, 'sleep-villager': check_sleep_villager,
             'bed-occupied': check_bed_occupied, 'sleep-reload': check_sleep_reload,
             'push': check_push, 'piston-push': check_piston_push,
             'piston-slime': check_piston_slime,
             'eat': check_eat, 'eating': check_eating, 'xp': check_xp,
             'objects': check_objects, 'projectiles': check_projectiles, 'blast': check_blast,
             'light': check_light, 'plates': check_plates, 'swings': check_swings,
             'cracks': check_cracks, 'time': check_time, 'growth': check_growth, 'doors': check_doors,
             'flight': check_flight, 'fireball': check_fireball, 'deflect': check_deflect,
             'barter': check_barter, 'worn': check_worn, 'kills': check_kills, 'leave': check_leave,
             'arrows': check_arrows, 'predict': check_predict, 'ladder': check_ladder,
             'boats': check_boats, 'minecarts': check_minecarts, 'frames': check_frames, 'hands': check_hands,
             'lightning': check_lightning, 'riding': check_riding, 'occupied_horse': check_occupied_horse, 'mounts': check_mounts,
             'worldspawn': check_world_spawn, 'beds': check_beds, 'shield': check_shield, 'deathmining': check_death_mining,
             'contacts': check_contacts, 'absorption': check_absorption, 'breath': check_breath,
             'lava': check_lava, 'levels': check_levels, 'drops': check_drops,
             'underwater': check_underwater,
             'looks': check_looks,
             'leads': check_leads,
             'thrown': check_thrown,
             'sparks': check_sparks,
             'hud': check_hud,
             'sidebar': check_sidebar,
             'border': check_border,
             'particles': check_particles, 'fireworks': check_fireworks}


def run(ctl, out, resources, checks):
    host, guest = Game('host', ctl, resources), Game('guest', ctl, resources)
    # Either way explicitly: the game's default is the server thread.
    host.d.env['HORIZON_SERVER_THREAD'] = '1' if SERVER_THREAD else '0'
    # A headless run (HORIZON_E2E_HEADLESS, or a Linux without vscreen) has no focused window to draw
    # in: the games render offscreen and never pause on the focus loss, or the world would stand still
    # (a host paused before its world loads waits at "Connecting to the server..." for ever).
    headless = bool(os.environ.get('HORIZON_E2E_HEADLESS')) or (
        sys.platform.startswith('linux') and not shutil.which('vscreen'))
    launch = ['launch']
    if headless or sys.platform == 'darwin':
        launch.append('--offscreen')
    if headless:
        launch.append('--no-pause')
    results = {}
    try:
        host.pid = host.raw(*launch)['pid']
        # Guests' commands need the world's Allow Commands as well as the LAN's (#372): a creative
        # world has them; everyone plays in survival.
        host.raw('open', 'Lan', 5, 'creative')
        host('wait_ready', seconds=400)
        host('command', command='defaultgamemode survival')
        host('command', command='gamemode survival')
        host('command', command='time set day')
        # An open stone floor around the host, so the players see each other and nothing is in the way.
        host('command', command='fill ~-8 ~-1 ~-8 ~8 ~-1 ~8 minecraft:stone')
        host('command', command='fill ~-8 ~ ~-8 ~8 ~6 ~8 minecraft:air')
        host('command', command='gamerule spawn_mobs false')
        port = host.raw('lan', 'open', 0, 'commands')['port']
        host.player = host.raw('lan', 'status').get('host', 'Player')  # the host's profile name
        guest.pid = guest.raw(*launch)['pid']
        guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
        guest.raw('wait-for', 'loaded', '--timeout', 300)
        guest.wait(40)
        # Where each joined, before anything moves them (check_spawned).
        host.joined_at, guest.joined_at = pos_of(host), pos_of(guest)
        face_each_other(host, guest)
        host.shot(out, 'joined')
        guest.shot(out, 'joined')
        for name in checks:
            try:
                # Cases assert their own deaths, menus and held actions before returning.
                # Do not let those states turn later independent cases into false failures.
                for game in (host, guest):
                    game.raw('release')
                    back_to_game(game)
                    state = game('state')
                    assert state.get('player') is not None, f'{game.name} lost its session before {name}: {state}'
                    if state.get('screen') == 'dead' or state['player']['health'] <= 0:
                        game.raw('respawn')
                        game.until(lambda: game('state'),
                                   lambda s: s.get('screen') == 'playing' and s['player']['health'] > 0,
                                   ticks=40)
                detail = CHECK_FNS[name](host, guest, out)
                skipped = isinstance(detail, str) and detail.startswith('skipped')
                results[name] = {'pass': not skipped, 'skipped': skipped, 'detail': detail}
            except (Failed, AssertionError, StopIteration, KeyError, TypeError, IndexError) as e:
                results[name] = {'pass': False, 'detail': repr(e)}
                for g in (host, guest):
                    back_to_game(g)
            print(f'[lan] {name}: {"PASS" if results[name]["pass"] else "FAIL"} {results[name]["detail"]}')
    finally:
        logs = {g.name: g.close() for g in (guest, host)}
        for name, text in logs.items():
            (out / f'{name}.log').write_text(text)
        for g in (host, guest):
            shutil.rmtree(g.work, ignore_errors=True)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--bin-dir', help='prebuilt horizon-client and horizon-ctl (default: build with the fast profile)')
    parser.add_argument('--check', action='append', choices=CHECKS, help='run only these checks (repeatable)')
    parser.add_argument('--out', default=str(ROOT / '.local/lan-e2e'), help='screenshots and game logs go here')
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument('--server-thread', dest='server_thread', action='store_true', default=True,
                      help='host on its server thread (default; retained for existing callers)')
    mode.add_argument('--in-frame', dest='server_thread', action='store_false',
                      help='host the world in the window, for comparison')
    args = parser.parse_args()
    global SERVER_THREAD
    SERVER_THREAD = args.server_thread
    if args.bin_dir:
        bins = Path(args.bin_dir).resolve()
    else:
        subprocess.run(['cargo', 'build', '--locked', '--profile', 'fast', '-p', 'horizon-client',
                        '--bin', 'horizon-client', '--bin', 'horizon-ctl'],
                       cwd=ROOT, check=True)
        meta = json.loads(subprocess.run(['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=ROOT,
                                         capture_output=True, text=True, check=True).stdout)
        bins = Path(meta['target_directory']) / 'fast'
    resources = lab.RESOURCES.resolve()
    if not (resources / 'horizon-resources.json').exists():
        sys.exit(f'no prepared resources in {resources}')
    try:
        require_server_reports(resources)
    except ValueError as error:
        sys.exit(str(error))
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    print(f'LAN host mode: {"server-thread" if SERVER_THREAD else "in-frame"}')
    results = run(bins / 'horizon-ctl', out, resources, args.check or CHECKS)
    (out / 'results.json').write_text(json.dumps({'mode': 'server-thread' if SERVER_THREAD else 'in-frame',
                                               'checks': results}, indent=2) + '\n')
    print(json.dumps(results, indent=1))
    sys.exit(0 if results and all(r['pass'] for r in results.values()) else 1)



# Checks added after the list above (kept apart so open branches that extend it don't conflict).


def heard(game, sound):
    return game('status')['heard'].get(sound, 0)


def check_totem(host, guest, out):
    """LivingEntity.checkTotemDeathProtection on the server for each player: 100 damage holding a
    totem in the off hand, then in the main hand, uses the totem up, leaves 1 health with
    Regeneration II, Absorption II and Fire Resistance, and broadcasts entity event 35, which both
    clients play once (#1292)."""
    sound = 'minecraft:item.totem.use'
    seen = {}
    for game in (host, guest):
        for hand in ('offhand', 'mainhand'):
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='effect clear @s')
            game('command', command='effect give @s minecraft:instant_health 1 5 true')
            game('command', command=f'item replace entity @s weapon.{hand} with minecraft:totem_of_undying')
            game.wait(10)
            before = {g.name: heard(g, sound) for g in (host, guest)}
            game('command', command='damage @s 100 minecraft:generic')
            game.wait(10)
            me = own(game)
            case = f'{game.name}-{hand}'
            seen[case] = {
                'screen': game('state').get('screen'),
                'health': me['health'],
                'server_health': server_player(host, game)['health'],
                'effects': sorted(effect_ids(game)),
                'totems': server_count(game, 'totem_of_undying'),
                'heard': {g.name: heard(g, sound) - before[g.name] for g in (host, guest)},
            }
            game.shot(out, f'totem-{hand}')
            game('command', command='effect clear @s')
    for case, s in seen.items():
        # 1 health, and what Regeneration II and the full food bar gave back since.
        assert s['screen'] == 'playing' and 0 < s['health'] < 5 and 0 < s['server_health'] < 5, f'{case}: {seen}'
        assert s['effects'] == ['absorption', 'fire_resistance', 'regeneration'], f'{case}: {seen}'
        assert s['totems'] == 0 and s['heard'] == {'host': 1, 'guest': 1}, f'{case}: {seen}'
    return seen


def check_keep_inventory(host, guest, out):
    """ServerPlayer.die's dropAllDeathLoot for each player (Player.dropEquipment): with keep_inventory
    off, the diamonds and the helmet a player dies with lie where it died and it respawns with
    neither; with it on, nothing drops and it respawns with both (#1284)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    # Totem coverage leaves both players near one health. Protect the waiting
    # player from residual fire/damage while the other player's death is tested.
    for game in (host, guest):
        game('command', command='gamemode creative')
        game('command', command='effect clear @s')
        game('command', command='effect give @s minecraft:instant_health 1 5 true')
    host('command', command=f'setworldspawn {hx} {hy} {hz}')
    host('command', command='gamerule respawn_radius 0')
    # Keep the inactive player alive and away from the death drops. Earlier
    # fixtures may leave a personal respawn point or an unsafe waiting position.
    host('command', command=f'fill {hx - 6} {hy - 1} {hz - 6} {hx + 6} {hy - 1} {hz + 6} minecraft:stone')
    host('command', command=f'fill {hx - 6} {hy} {hz - 6} {hx + 6} {hy + 3} {hz + 6} minecraft:air')
    for game, dz in ((host, -3), (guest, 3)):
        waiting = (hx - 3.5, hy, hz + dz + 0.5)
        game('command', command=f'spawnpoint @s {hx} {hy} {hz}')
        game('command', command=f'tp @s {waiting[0]} {waiting[1]} {waiting[2]}')
        game.until(lambda: pos_of(game), lambda p: math.dist(p, waiting) < 0.1)
    try:
        for keep in ('false', 'true'):
            host('command', command=f'gamerule keep_inventory {keep}')
            for game, dz in ((host, -3), (guest, 3)):
                # 14 blocks from the world spawn: nothing dropped is picked up on respawning.
                x, z = hx + 14, hz + dz
                # Death drops scatter several blocks before landing. Keep the
                # entire observation area solid so they cannot fall off a ledge.
                host('command', command=f'fill {x - 6} {hy - 1} {z - 6} {x + 6} {hy - 1} {z + 6} minecraft:stone')
                host('command', command=f'fill {x - 6} {hy} {z - 6} {x + 6} {hy + 3} {z + 6} minecraft:air')
                host('command', command='kill @e[type=minecraft:item]')
                game('command', command='gamemode survival')
                game('command', command='clear @s')
                game('command', command='give @s minecraft:diamond 7')
                game('command', command='item replace entity @s armor.head with minecraft:iron_helmet')
                game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5}')
                game.wait(20)
                assert game('state').get('screen') == 'playing', f'{game.name} died during setup'
                game('command', command='kill @s')
                game.wait(30)
                assert game('state').get('screen') == 'dead', f'{game.name} did not die'
                dropped = sorted((e['item']['item'], e['item']['count'])
                                 for e in game.raw('entities', 8, 'minecraft:item')['entities'])
                game.shot(out, f'keepinv-{keep}')
                respawn_when_ready(game)
                game.wait(40)
                back_to_game(game)
                seen[f'{game.name}-keep-{keep}'] = {
                    'dropped': dropped,
                    'kept': {item: server_count(game, item) for item in ('diamond', 'iron_helmet')}}
                game('command', command='gamemode creative')
    finally:
        host('command', command='gamerule keep_inventory false')
        host('command', command='gamerule respawn_radius 10')
        host('command', command='kill @e[type=minecraft:item]')
        for game in (host, guest):
            game('command', command='clear @s')
            game('command', command='gamemode survival')
        face_each_other(host, guest)
    for case, s in seen.items():
        want = ({'dropped': [], 'kept': {'diamond': 7, 'iron_helmet': 1}} if case.endswith('true') else
                {'dropped': [('minecraft:diamond', 7), ('minecraft:iron_helmet', 1)],
                 'kept': {'diamond': 0, 'iron_helmet': 0}})
        assert s == want, f'{case}: {seen}'
    return seen


def check_signs(host, guest, out):
    """SignBlock.openTextEdit on the placing player's client, for the host and the guest: placing
    an oak sign opens its editor there and nowhere else, the lines typed into it go to the server
    when it closes, and both sides then show them (#1380)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    for game, other, dz in ((host, guest, -2), (guest, host, 2)):
        spot = (hx + 2, hy, hz + dz)
        for viewer in (game, other):
            back_to_game(viewer)
        # Earlier death/travel checks leave the players at different spawn points.
        # Give this placement case its own floor, line of sight and reachable target.
        host('command', command=f'fill {spot[0] - 3} {hy - 1} {spot[2] - 1} {spot[0] + 3} {hy - 1} {spot[2] + 1} minecraft:stone')
        host('command', command=f'fill {spot[0] - 3} {hy} {spot[2] - 1} {spot[0] + 3} {hy + 3} {spot[2] + 1} minecraft:air')
        for viewer, dx in ((game, -2), (other, 2)):
            at = (spot[0] + dx + 0.5, hy, spot[2] + 0.5)
            viewer('command', command=f'tp @s {at[0]} {at[1]} {at[2]}')
            viewer.until(lambda: pos_of(viewer), lambda p: math.dist(p, at) < 0.1)
        game('command', command=f'setblock {spot[0]} {spot[1] - 1} {spot[2]} minecraft:stone')
        game('command', command=f'setblock {spot[0]} {spot[1]} {spot[2]} minecraft:air')
        game('command', command='gamemode survival')
        game('command', command='clear @s')
        game('command', command='give @s minecraft:oak_sign 1')
        game.wait(10)
        game.raw('place', *spot, '--item', 'minecraft:oak_sign')
        game.until(lambda: game('state').get('screen'), lambda s: s == 'sign', ticks=10)
        # Drain the placement's initial block-entity data before typing. The
        # reply travels on the same connection after that data, unlike a local
        # screen check which may see the editor before the initial data arrives.
        server_reply(game, f'data get block {spot[0]} {spot[1]} {spot[2]} front_text.messages',
                     'following block data')
        opened = {g.name: g('state').get('screen') for g in (game, other)}
        game.shot(out, 'signs-editor')
        lines = [f'{game.name} wrote', 'line two']
        game.raw('type', lines[0])
        game.raw('key', 'enter', 'tap')
        game.raw('type', lines[1])
        game.raw('key', 'esc', 'tap')
        game.until(lambda: game('state').get('screen'), lambda s: s == 'playing', ticks=20)
        back_to_game(other)
        shown = {g.name: g.until(lambda: (g.raw('block', *spot).get('sign') or {}).get('front'),
                                lambda front: front == lines + ['', ''], ticks=20)
                 for g in (game, other)}
        seen[game.name] = {'opened': opened,
                           'shown': shown}
        other.shot(out, 'signs-shown')
    for name, s in seen.items():
        other = 'guest' if name == 'host' else 'host'
        want = [f'{name} wrote', 'line two', '', '']
        assert s['opened'] == {name: 'sign', other: 'playing'}, f'{name}: {seen}'
        assert s['shown'] == {name: want, other: want}, f'{name}: {seen}'
    return seen


def book_screen(game):
    """The open book (`book`'s page and page count), once `game` shows it."""
    game.raw('wait-for', 'screen', 'book', '--timeout', 10)
    return game.raw('book')


def check_books(host, guest, out):
    """Books in hand for the host and the guest: a book and quill opens its editor on the player's
    own client (LocalPlayer.openItemGui), what it writes and signs reaches the server
    (ServerboundEditBookPacket), and the written book opens on the player's client when the server
    says so (ServerPlayer.openItemGui's ClientboundOpenBookPacket); dropped, the other side sees the
    server's book (#1366)."""
    seen = {}
    for game, other in ((host, guest), (guest, host)):
        game('command', command='gamemode survival')
        game('command', command='clear @s')
        game('command', command='item replace entity @s weapon.mainhand with minecraft:writable_book')
        x, y, z = pos_of(game)
        game.raw('look-at', f'{x + 0.5:.2f}', f'{y + 30:.2f}', f'{z:.2f}')
        game.wait(10)
        case = seen[game.name] = {}
        game.raw('use-item')
        case['edit'] = book_screen(game)
        case['other_screen'] = other('state').get('screen')
        game.raw('type', f'{game.name} page one')
        game.shot(out, 'books-edit')
        game.raw('book', 'sign')
        game.raw('type', f'{game.name} book')
        game.raw('book', 'sign')
        game.wait(20)
        case['signed'] = {item: server_count(game, item) for item in ('writable_book', 'written_book')}
        game.raw('use-item')
        case['view'] = book_screen(game)
        game.shot(out, 'books-view')
        game.raw('book', 'done')
        game.wait(5)
        game.raw('key', 'q', 'tap')
        game.wait(30)
        books = [e['item'].get('components', '') for e in other.raw('entities', 16, 'minecraft:item')['entities']
                 if e['item']['item'] == 'minecraft:written_book']
        case['dropped'] = [all(f'{game.name} {t}' in c for t in ('page one', 'book')) for c in books]
        host('command', command='kill @e[type=minecraft:item]')
    for name, case in seen.items():
        assert case['edit'] == {'page': 0, 'pages': 1, 'lectern': None}, f'{name}: {seen}'
        assert case['other_screen'] == 'playing', f'{name}: {seen}'
        assert case['signed'] == {'writable_book': 0, 'written_book': 1}, f'{name}: {seen}'
        assert case['view'] == {'page': 0, 'pages': 1, 'lectern': None}, f'{name}: {seen}'
        assert case['dropped'] == [True], f'{name}: {seen}'
    return seen


CHECKS += ['totem', 'keepinv', 'signs', 'books']
CHECK_FNS.update(totem=check_totem, keepinv=check_keep_inventory, signs=check_signs, books=check_books)


def check_elytra(host, guest, out):
    """LivingEntity.updateFallFlying for each player: a jump while falling with an Elytra on starts a
    glide that the server takes too (Player.tryToStartFallFlying), the glide carries the player
    forward rather than down, and landing ends it on both sides (canGlide fails on the ground)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    try:
        for game, dx in ((host, -2), (guest, 2)):
            x, z = hx + dx, hz
            host('command', command=f'fill {x - 1} {hy - 1} {z - 1} {x + 1} {hy - 1} {z + 1} minecraft:stone')
            host('command', command=f'fill {x - 1} {hy} {z - 1} {x + 1} {hy + 3} {z + 1} minecraft:air')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='effect give @s minecraft:resistance 60 4 true')
            game('command', command='item replace entity @s armor.chest with minecraft:elytra')
            game('command', command=f'tp @s {x + 0.5} {hy + 60} {z + 0.5} 0 0')
            game.wait(6)
            start = pos_of(game)
            game.raw('key', 'space', 'down')
            game.wait(1)
            game.raw('key', 'space', 'up')
            game.wait(30)
            end = pos_of(game)
            glide = {'client': own(game).get('fall_flying'),
                     'server': server_player(host, game).get('fall_flying'),
                     'forward': round(math.hypot(end[0] - start[0], end[2] - start[2]), 1),
                     'down': round(start[1] - end[1], 1)}
            game.shot(out, 'elytra')
            game('command', command=f'tp @s {x + 0.5} {hy + 1} {z + 0.5}')
            game.wait(30)
            landed = {'on_ground': own(game).get('on_ground'),
                      'client': own(game).get('fall_flying'),
                      'server': server_player(host, game).get('fall_flying')}
            seen[game.name] = {'glide': glide, 'landed': landed}
    finally:
        for game in (host, guest):
            game('command', command='clear @s')
            game('command', command='effect clear @s')
        face_each_other(host, guest)
    for name, s in seen.items():
        g, l = s['glide'], s['landed']
        # Falling alone covers about 25 blocks in those 30 ticks and goes nowhere.
        assert g['client'] and g['server'] and g['forward'] > 8 and g['down'] < 20, f'{name} glide: {seen}'
        assert l['on_ground'] and not l['client'] and not l['server'], f'{name} landed: {seen}'
    return seen


CHECKS += ['elytra']
CHECK_FNS.update(elytra=check_elytra)


def server_using(host, game):
    """The item the server has the player using ({slot, ticks}), or None."""
    if game is host and not SERVER_THREAD:
        return host('state').get('using')
    return server_player(host, game).get('using')


def flying(game, kind):
    """How many of the projectiles this side draws are of `kind` (a trident, an arrow)."""
    return sum(kind in q['kind'].lower() for q in game('status')['projectiles'])


def check_held_use(host, guest, out):
    """Items used until the button lets go, for each player: the client starts the use too
    (Item.use runs on both sides), so letting go sends RELEASE_USE_ITEM and the server ends it."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    try:
        for game, dz in ((host, -3), (guest, 3)):
            x, z = hx, hz + dz
            host('command', command=f'fill {x - 1} {hy - 1} {z - 1} {x + 12} {hy - 1} {z + 1} minecraft:stone')
            host('command', command=f'fill {x - 1} {hy} {z - 1} {x + 12} {hy + 3} {z + 1} minecraft:air')
            host('command', command=f'fill {x + 12} {hy} {z - 1} {x + 12} {hy + 3} {z + 1} minecraft:stone')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5} -90 0')
            game.wait(10)
            s = {}
            # ShieldItem: raised while held, lowered once let go.
            game('command', command='item replace entity @s weapon.offhand with minecraft:shield')
            game.wait(5)
            game.raw('mouse', 'right', 'down')
            game.wait(10)
            s['shield'] = {'held': (bool(game('state').get('using')), bool(server_using(host, game)))}
            game.raw('mouse', 'right', 'up')
            game.wait(10)
            s['shield']['let_go'] = (bool(game('state').get('using')), bool(server_using(host, game)))
            game('command', command='item replace entity @s weapon.offhand with minecraft:air')
            # SpyglassItem: scoping while held.
            game('command', command='item replace entity @s weapon.mainhand with minecraft:spyglass')
            game.wait(5)
            game.raw('mouse', 'right', 'down')
            game.wait(10)
            s['spyglass'] = {'held': (bool(game('state').get('using')), bool(server_using(host, game)))}
            game.raw('mouse', 'right', 'up')
            game.wait(10)
            s['spyglass']['let_go'] = (bool(game('state').get('using')), bool(server_using(host, game)))
            # TridentItem.releaseUsing after 10 ticks: thrown.
            host('command', command='kill @e[type=minecraft:trident]')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:trident')
            game.wait(5)
            # /kill finds no thrown trident (Horizon's selectors miss them): count what's new.
            before = flying(game, 'trident')
            game.raw('use-item', '--hold', 1.2)
            game.wait(10)
            s['trident'] = {'kept': server_count(game, 'trident'),
                            'thrown': flying(game, 'trident') - before}
            host('command', command='kill @e[type=minecraft:trident]')
            # CrossbowItem: charged by holding (25 ticks), fired by the next use.
            host('command', command='kill @e[type=minecraft:arrow]')
            game('command', command='clear @s')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:crossbow')
            game('command', command='give @s minecraft:arrow 16')
            game.wait(5)
            before = flying(game, 'arrow')
            game.raw('use-item', '--hold', 1.6)
            game.wait(10)
            held = game('state').get('held') or {}
            s['crossbow'] = {'loaded': 'charged_projectiles' in str(held.get('components', '')),
                             'arrows': server_count(game, 'arrow')}
            game.raw('use-item')
            game.wait(10)
            held = game('state').get('held') or {}
            s['crossbow'].update(
                fired=flying(game, 'arrow') - before,
                emptied='charged_projectiles' not in str(held.get('components', '')))
            game.shot(out, 'helduse')
            seen[game.name] = s
    finally:
        host('command', command='kill @e[type=minecraft:arrow]')
        host('command', command='kill @e[type=minecraft:trident]')
        for game in (host, guest):
            back_to_game(game)
            game.raw('mouse', 'right', 'up')
            game('command', command='clear @s')
        face_each_other(host, guest)
    for name, s in seen.items():
        for item in ('shield', 'spyglass'):
            assert s[item] == {'held': (True, True), 'let_go': (False, False)}, f'{name} {item}: {seen}'
        assert s['trident'] == {'kept': 0, 'thrown': 1}, f'{name} trident: {seen}'
        assert s['crossbow'] == {'loaded': True, 'arrows': 15, 'fired': 1, 'emptied': True}, \
            f'{name} crossbow: {seen}'
    return seen


CHECKS += ['helduse']
CHECK_FNS.update(helduse=check_held_use)


def check_eat_full(host, guest, out):
    """Holding right click past a full food bar starts no other bite; chorus fruit's cooldown
    doesn't leave the use stuck (#2471)."""
    guest('command', command='gamemode survival')
    guest('command', command='clear @s')
    # Full health: natural regeneration at 18 food or more would eat into the bar.
    guest('command', command='effect give @s minecraft:instant_health 1 10')
    guest('command', command='effect give @s minecraft:hunger 60 127')
    for _ in range(400):
        if remote(guest)['food'] <= 17:
            break
        guest.wait(1)
    guest('command', command='effect clear @s')
    guest('command', command='give @s minecraft:melon_slice 16')
    guest.wait(5)
    guest.raw('select', 'minecraft:melon_slice')
    hungry = remote(guest)['food']
    # A slice (2 food) takes 32 ticks (1.6 s): the button stays down well past the slices that fill
    # the bar, with room for a slow server.
    slices = -(-(20 - hungry) // 2)
    hold = threading.Thread(target=guest.raw, args=('use-item', '--hold', slices * 4.0 + 6.0))
    hold.start()
    deadline = time.monotonic() + slices * 4.0 + 2.0
    while remote(guest)['food'] < 20 and time.monotonic() < deadline:
        time.sleep(0.1)
    # The bar is full and the button is still down: nothing is in use over the next 20 ticks
    # (sleeps, as `wait` is refused while the held use-item runs).
    using = []
    for _ in range(4):
        time.sleep(0.25)
        using.append(guest('state').get('using'))
    guest.shot(out, 'eat-full')
    hold.join()
    guest.wait(10)
    full = {'hungry': hungry, 'fed': remote(guest)['food'], 'using': using,
            'eaten': 16 - guest('inventory')['counts'].get('minecraft:melon_slice', 0), 'needed': slices}
    assert hungry < 20, f'the guest is not hungry: {full}'
    assert full['fed'] == 20 and full['eaten'] == slices, f'the bar did not fill: {full}'
    assert using == [None] * 4, f'another bite started with a full food bar: {full}'

    # Chorus fruit can always be eaten, then its group cools down for 20 ticks: the client waits
    # it out (MultiPlayerGameMode.useItem) and eats the second fruit with the button still held.
    guest('command', command='clear @s')
    guest('command', command='give @s minecraft:chorus_fruit 2')
    guest.wait(5)
    guest.raw('select', 'minecraft:chorus_fruit')
    guest.raw('use-item', '--hold', 7.0)
    guest.wait(10)
    chorus = {'left': guest('inventory')['counts'].get('minecraft:chorus_fruit', 0),
              'using': guest('state').get('using')}
    assert chorus == {'left': 0, 'using': None}, f'the held use did not eat both chorus fruits: {chorus}'
    return {'full': full, 'chorus': chorus}


CHECKS += ['eat_full']
CHECK_FNS.update(eat_full=check_eat_full)


def check_consume_release(host, guest, out):
    """#1293: releasing a consumable in either hand cancels it on client and server, while
    holding it to completion applies its effects and remainder (Consumable.onConsume)."""
    apart(host, guest)
    seen = {}
    try:
        for game in (host, guest):
            game('command', command='gamemode survival')
            x, y, z = pos_of(game)
            game.raw('look-at', x, y + 1.6, z + 2)
            for hand in ('mainhand', 'offhand'):
                for item in ('golden_apple', 'milk_bucket'):
                    case = f'{game.name}-{hand}-{item}'
                    game('command', command='clear @s')
                    game('command', command='item replace entity @s weapon.offhand with minecraft:air')
                    game('command', command='effect clear @s')
                    if item == 'milk_bucket':
                        game('command', command='effect give @s minecraft:speed 60')
                    game('command', command=f'item replace entity @s weapon.{hand} with minecraft:{item}')
                    game.wait(10)
                    assert game('looking_at')['kind'] == 'none', f'{case} is not aimed into the air'
                    before = set(effect_ids(game))
                    assert before == ({'speed'} if item == 'milk_bucket' else set()), f'{case}: {before}'
                    game.raw('mouse', 'right', 'down')
                    game.wait(5)
                    during = {'client': game('state').get('using'), 'server': server_using(host, game)}
                    game.raw('mouse', 'right', 'up')
                    game.wait(10)
                    released = {'client': game('state').get('using'), 'server': server_using(host, game)}
                    # Wait past the full use duration: the server must not finish eating after release.
                    game.wait(60)
                    effects = set(effect_ids(game))
                    inventory = game('inventory')
                    stack = inventory['offhand'] if hand == 'offhand' else inventory['held']
                    cancelled = {'during': during, 'released': released,
                                 'stack': stack, 'effects': sorted(effects)}
                    seen[case] = {'cancelled': cancelled}
                    assert during['client'] and during['server'], f'{case} did not start on both sides: {cancelled}'
                    assert released == {'client': None, 'server': None}, f'{case} did not cancel: {cancelled}'
                    assert stack and stack['item'] == f'minecraft:{item}' and stack['count'] == 1, \
                        f'{case} consumed the released item: {cancelled}'
                    assert effects == before, f'{case} applied consumption effects after release: {cancelled}'
                    game.shot(out, f'consume-release-{hand}-{item}')
                    # A sustained press must still consume normally, including the milk's empty bucket.
                    # The use ends with the server's USE_ITEM_COMPLETE, 32 server ticks after its
                    # UseItem: room for a busy server before letting go.
                    game.raw('mouse', 'right', 'down')
                    game.wait(50)
                    game.raw('mouse', 'right', 'up')
                    game.wait(10)
                    inventory = game('inventory')
                    stack = inventory['offhand'] if hand == 'offhand' else inventory['held']
                    completed = {'stack': stack, 'effects': sorted(effect_ids(game)),
                                 'using': (game('state').get('using'), server_using(host, game))}
                    seen[case]['completed'] = completed
                    assert completed['using'] == (None, None), \
                        f'{case} did not finish normally: {completed}'
                    if item == 'golden_apple':
                        assert inventory['counts'].get('minecraft:golden_apple', 0) == 0, \
                            f'{case} kept the eaten apple: {completed}'
                        assert set(completed['effects']) == {'regeneration', 'absorption'}, f'{case}: {completed}'
                    else:
                        assert not completed['effects'] and stack and stack['item'] == 'minecraft:bucket' \
                            and stack['count'] == 1, f'{case}: {completed}'
    finally:
        for game in (host, guest):
            game.raw('mouse', 'right', 'up')
            game('command', command='clear @s')
            game('command', command='item replace entity @s weapon.offhand with minecraft:air')
            game('command', command='effect clear @s')
        face_each_other(host, guest)
    return seen


CHECKS += ['consume-release']
CHECK_FNS.update({'consume-release': check_consume_release})


def check_wind(host, guest, out):
    """A wind charge each player throws at its own feet (ServerExplosion with TRIGGER_BLOCK): the
    server's burst pushes the thrower through its ClientboundExplodePacket (hitPlayers), which
    launches it about six blocks up, and onExplosionHit spares it the fall damage where it lands
    (#1386)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='gamerule natural_health_regeneration false')
    seen = {}
    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, z = hx, hz + dz
            host('command', command=f'fill {x - 2} {hy - 1} {z - 2} {x + 2} {hy - 1} {z + 2} minecraft:stone')
            host('command', command=f'fill {x - 2} {hy} {z - 2} {x + 2} {hy + 10} {z + 2} minecraft:air')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='effect clear @s')
            game('command', command='effect give @s minecraft:instant_health 1 5 true')
            game('command', command='give @s minecraft:wind_charge 2')
            game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5} 0 90')
            game.wait(20)
            start = pos_of(game)[1]
            game.raw('use-item')
            peak = start
            for _ in range(30):
                game.wait(2)
                peak = max(peak, pos_of(game)[1])
            game.wait(40)
            game.shot(out, 'wind')
            me = own(game)
            seen[game.name] = {'rose': round(peak - start, 1),
                               'landed': round(me['position'][1] - start, 1),
                               'health': me['health'],
                               'server_health': server_player(host, game)['health'],
                               'charges': server_count(game, 'wind_charge')}
    finally:
        host('command', command='gamerule natural_health_regeneration true')
        for game in (host, guest):
            game('command', command='clear @s')
        face_each_other(host, guest)
    for name, s in seen.items():
        # Vanilla rises about 6 blocks from a charge at the feet.
        assert 4 < s['rose'] < 8 and s['landed'] == 0, f'{name}: {seen}'
        assert s['health'] == 20 and s['server_health'] == 20 and s['charges'] == 1, f'{name}: {seen}'
    return seen


CHECKS += ['wind']
CHECK_FNS.update(wind=check_wind)


def bobbers(game):
    """The fishing bobbers this side draws, as (owner) ids: 0 for its own player's."""
    return [q.get('owner') for q in game('status')['projectiles'] if q['kind'] == 'minecraft:fishing_bobber']


def check_fishing(host, guest, out):
    """FishingHook for each player, on the server: a cast hook stays out (ticked by the server and
    tracked to the clients as a fishing bobber under its owner), lands on the ground and reels back
    in; cast into open water with Lure III, a fish bites and reeling in
    during the bite catches it for 1 (#1348)."""
    # Both players need a safe floor for the entire other player's fishing wait.
    apart(host, guest)
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    host('command', command='weather clear')
    for game in (host, guest):
        game('command', command='effect give @s minecraft:instant_health 1 5 true')
    seen = {}
    try:
        for game, other, dz in ((host, guest, -5), (guest, host, 5)):
            x, z = hx, hz + dz
            host('command', command=f'fill {x - 1} {hy - 1} {z - 1} {x + 14} {hy - 1} {z + 1} minecraft:stone')
            host('command', command=f'fill {x - 1} {hy} {z - 1} {x + 14} {hy + 4} {z + 1} minecraft:air')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:fishing_rod')
            game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5} -90 0')
            game.wait(10)
            # On the ground: the floor four blocks ahead.
            game.raw('look-at', f'{x + 4.5:.2f}', f'{hy:.2f}', f'{z + 0.5:.2f}')
            game.raw('use-item')
            game.wait(40)
            s = {'own': bobbers(game), 'other': bobbers(other)}
            game.shot(out, 'fishing-cast')
            game.raw('use-item')
            game.wait(10)
            s['reeled'] = {'own': bobbers(game), 'damage': (game('state').get('held') or {}).get('damage')}
            # Open water: a pool 7x7 and 3 deep, ahead.
            host('command', command=f'fill {x + 3} {hy - 3} {z - 3} {x + 9} {hy - 1} {z + 3} minecraft:water')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:fishing_rod[enchantments={lure:3}]')
            game.wait(10)
            game.raw('look-at', f'{x + 6.5:.2f}', f'{hy - 1:.2f}', f'{z + 0.5:.2f}')
            game.raw('use-item')
            bit = None
            for t in range(0, 900, 5):
                game.wait(5)
                if (game('status').get('fishing') or {}).get('nibble', 0) > 0:
                    bit = t
                    break
            game.raw('use-item')
            game.wait(40)
            counts = game.raw('inventory')['counts']
            s['caught'] = {'bit': bit is not None,
                           'damage': (game('state').get('held') or {}).get('damage'),
                           'items': sum(counts.values()) - 1}
            game.shot(out, 'fishing-caught')
            host('command', command=f'fill {x + 3} {hy - 3} {z - 3} {x + 9} {hy - 2} {z + 3} minecraft:stone')
            host('command', command=f'fill {x + 3} {hy - 1} {z - 3} {x + 9} {hy - 1} {z + 3} minecraft:stone')
            seen[game.name] = s
    finally:
        host('command', command='kill @e[type=minecraft:item]')
        for game in (host, guest):
            game('command', command='clear @s')
        face_each_other(host, guest)
    for name, s in seen.items():
        other = guest if name == 'host' else host
        assert s['own'] == [0], f'{name}: {seen}'
        # The in-frame host draws only its own hook.
        if other is guest or SERVER_THREAD:
            assert len(s['other']) == 1 and s['other'][0] != 0, f'{name}: {seen}'
        # FishingHook.retrieve costs 2 on the ground, but a resting hook's onGround
        # alternates every tick on the server (Entity.move with no vertical motion
        # clears it, the next tick's gravity sets it again): 2 or nothing.
        assert s['reeled']['own'] == [] and s['reeled']['damage'] in (None, 0, 2), f'{name}: {seen}'
        assert s['caught']['bit'] and s['caught']['damage'] == 1 and s['caught']['items'] >= 1, f'{name}: {seen}'
    return seen


CHECKS += ['fishing']
CHECK_FNS.update(fishing=check_fishing)


def check_maps(host, guest, out):
    """MapItem.update for each player, on the server: a map the player makes and holds reveals the
    terrain around them and shows their marker, which reach their client in
    ClientboundMapItemDataPacket (#1382)."""
    seen = {}
    try:
        for game in (host, guest):
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='item replace entity @s weapon.mainhand with minecraft:map')
            x, y, z = pos_of(game)
            game.raw('look-at', f'{x + 5:.2f}', f'{y + 1.6:.2f}', f'{z:.2f}')
            game.wait(5)
            game.raw('use-item')
            # 16 ticks sweep every column once; the marker goes out every 5th tick.
            game.wait(60)
            maps = game('status').get('maps', [])
            game.raw('look-at', f'{x + 1:.2f}', f'{y:.2f}', f'{z:.2f}')
            game.wait(5)
            game.shot(out, 'maps-held')
            seen[game.name] = [{'pixels': m['pixels'], 'decorations': m['decorations']} for m in maps]
    finally:
        for game in (host, guest):
            game('command', command='clear @s')
        face_each_other(host, guest)
    for name, maps in seen.items():
        # Revealed out to the server's view distance, at least a 32-block disk.
        assert len(maps) == 1 and maps[0]['pixels'] > 3000, f'{name}: {seen}'
        assert maps[0]['decorations'] == ['map/decorations/player'], f'{name}: {seen}'
    return seen


CHECKS += ['maps']
CHECK_FNS.update(maps=check_maps)


def check_body(host, guest, out):
    """What each player's own client decides for its body (#1278, #1303, #1296): walking while
    drawing a bow is slowed to a fifth (LocalPlayer.modifyInput), sprint-swimming follows the look
    down (Player.travel's look angle) and leather boots hold on powder snow
    (PowderSnowBlock.canEntityWalkOnPowderSnow)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}

    def walked(game, x, z, use):
        game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5} -90 0')
        game.wait(10)
        if use:
            game.raw('mouse', 'right', 'down')
            game.wait(5)
        a = pos_of(game)
        game.raw('move', 'forward', '0.8')
        game.wait(10)
        b = pos_of(game)
        game.raw('mouse', 'right', 'up')
        game.wait(5)
        return round(math.dist((a[0], a[2]), (b[0], b[2])), 2)

    try:
        for game, dz in ((host, -4), (guest, 4)):
            x, z = hx, hz + dz
            host('command', command=f'fill {x - 1} {hy - 1} {z - 1} {x + 10} {hy - 1} {z + 1} minecraft:stone')
            host('command', command=f'fill {x - 1} {hy} {z - 1} {x + 10} {hy + 3} {z + 1} minecraft:air')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='effect clear @s')
            s = {'walk': walked(game, x, z, False)}
            game('command', command='item replace entity @s weapon.mainhand with minecraft:bow')
            game('command', command='give @s minecraft:arrow 4')
            game.wait(5)
            s['drawing'] = walked(game, x, z, True)
            game('command', command='clear @s')
            # Powder snow under the feet: leather boots stand on it, bare feet sink.
            host('command', command=f'setblock {x + 3} {hy - 1} {z} minecraft:powder_snow')
            for boots in ('leather_boots', 'air'):
                game('command', command=f'item replace entity @s armor.feet with minecraft:{boots}')
                game('command', command=f'tp @s {x + 3.5} {hy} {z + 0.5} -90 0')
                game.wait(30)
                s[boots] = round(pos_of(game)[1] - hy, 2)
            host('command', command=f'setblock {x + 3} {hy - 1} {z} minecraft:stone')
            # Deep water: sprint-swimming while looking down dives.
            wx, wz = x, z + 2 * (1 if dz > 0 else -1) * 3
            host('command', command=f'fill {wx - 1} {hy - 14} {wz - 2} {wx + 12} {hy + 2} {wz + 2} minecraft:stone')
            host('command', command=f'fill {wx} {hy - 13} {wz - 1} {wx + 11} {hy - 1} {wz + 1} minecraft:water')
            game.wait(10)
            game('command', command=f'tp @s {wx + 1.5} {hy - 6} {wz + 0.5} -90 60')
            game.wait(10)
            a = pos_of(game)
            game.raw('move', 'forward', '1.5', '--sprint')
            game.wait(5)
            s['dive'] = round(pos_of(game)[1] - a[1], 2)
            game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5} -90 0')
            game.wait(5)
            host('command', command=f'fill {wx - 1} {hy - 14} {wz - 2} {wx + 12} {hy + 2} {wz + 2} minecraft:air')
            host('command', command=f'fill {wx - 1} {hy - 14} {wz - 2} {wx + 12} {hy - 1} {wz + 2} minecraft:stone')
            game.shot(out, 'body')
            seen[game.name] = s
    finally:
        for game in (host, guest):
            back_to_game(game)
            game.raw('mouse', 'right', 'up')
            game('command', command='clear @s')
        face_each_other(host, guest)
    for name, s in seen.items():
        assert s['walk'] > 2.5 and s['drawing'] < s['walk'] * 0.4, f'{name} drawing a bow: {seen}'
        assert s['leather_boots'] > -0.01 and s['air'] < -0.5, f'{name} powder snow: {seen}'
        assert s['dive'] < -2, f'{name} sprint-swimming down: {seen}'
    return seen


CHECKS += ['body']
CHECK_FNS.update(body=check_body)


def check_strikes(host, guest, out):
    """What a mob or the sky does to every player in reach, the server's players too (#1480,
    #1484): a bolt striking a player standing in water costs them health (Entity.thunderHit), and
    a cave spider's bite poisons them (CaveSpider.doHurtTarget on Normal)."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    seen = {}
    try:
        host('command', command='difficulty normal')
        host('command', command='time set 18000')
        for game, dz in ((host, -5), (guest, 5)):
            x, z = hx, hz + dz
            host('command', command=f'fill {x - 3} {hy - 1} {z - 3} {x + 3} {hy + 3} {z + 3} minecraft:stone')
            host('command', command=f'fill {x - 2} {hy} {z - 2} {x + 2} {hy + 3} {z + 2} minecraft:air')
            host('command', command=f'fill {x - 2} {hy} {z - 2} {x + 2} {hy} {z + 2} minecraft:water')
            game('command', command='gamemode survival')
            game('command', command='clear @s')
            game('command', command='effect clear @s')
            game('command', command='effect give @s minecraft:instant_health 1 5')
            game('command', command=f'tp @s {x + 0.5} {hy} {z + 0.5}')
            game.wait(20)
            before = game('state')['player'].get('health')
            game('command', command=f'summon minecraft:lightning_bolt {x + 0.5} {hy} {z + 0.5}')
            game.wait(20)
            s = {'bolt': (before, game('state')['player'].get('health'))}
            host('command', command=f'summon minecraft:cave_spider {x + 1.5} {hy} {z + 0.5}')
            poisoned = False
            for _ in range(40):
                game.wait(5)
                if 'poison' in effect_ids(game):
                    poisoned = True
                    break
            s['poisoned'] = poisoned
            s['bitten'] = game('state')['player'].get('health')
            host('command', command='kill @e[type=minecraft:cave_spider]')
            game.wait(5)
            game.shot(out, 'strikes')
            host('command', command=f'fill {x - 3} {hy - 1} {z - 3} {x + 3} {hy + 3} {z + 3} minecraft:air')
            host('command', command=f'fill {x - 3} {hy - 1} {z - 3} {x + 3} {hy - 1} {z + 3} minecraft:stone')
            seen[game.name] = s
    finally:
        host('command', command='kill @e[type=minecraft:cave_spider]')
        host('command', command='time set day')
        for game in (host, guest):
            back_to_game(game)
            game('command', command='effect clear @s')
            game('command', command='effect give @s minecraft:instant_health 1 5')
        face_each_other(host, guest)
    for name, s in seen.items():
        a, b = s['bolt']
        assert a is not None and b is not None and a - b >= 3, f'{name} struck by lightning: {seen}'
        assert s['poisoned'], f'{name} bitten by a cave spider: {seen}'
    return seen


CHECKS += ['strikes']
CHECK_FNS.update(strikes=check_strikes)


def check_cursor(host, guest, out):
    """Closing the inventory with a stack on the cursor puts it back into the player's inventory
    (AbstractContainerMenu.removed, placeItemBackInInventory) and empties the cursor, for each
    player: nothing is thrown out and no copy stays on the cursor (#1283)."""
    seen = {}
    try:
        for game in (host, guest):
            game('command', command='clear @s')
            host('command', command='kill @e[type=minecraft:item]')
            game('command', command='item replace entity @s hotbar.0 with minecraft:diamond 7')
            game.wait(10)
            game.raw('open-inventory')
            game.raw('wait-for', 'screen', 'inventory', '--timeout', 10)
            game.raw('click-slot', 0)
            game.wait(5)
            held = (game('inventory').get('carried') or {}).get('count')
            game.raw('key', 'esc', 'tap')
            game.wait(40)
            inv = game('inventory')
            seen[game.name] = {'held': held, 'carried': inv.get('carried'),
                               'client': inv['counts'].get('minecraft:diamond', 0),
                               'server': server_count(game, 'diamond'),
                               'thrown': len(game.raw('entities', 8, 'minecraft:item')['entities'])}
    finally:
        for game in (host, guest):
            back_to_game(game)
            game('command', command='clear @s')
        host('command', command='kill @e[type=minecraft:item]')
    for name, s in seen.items():
        assert s == {'held': 7, 'carried': None, 'client': 7, 'server': 7, 'thrown': 0}, f'{name}: {seen}'
    return seen


CHECKS += ['cursor']
CHECK_FNS.update(cursor=check_cursor)


def check_digging(host, guest, out):
    """The server breaks a block a player digs only once that player's client is done
    (ServerPlayerGameMode: STOP_DESTROY_BLOCK, or the delayed destroy), never on its own count, so
    the player's client always breaks it first, with its particles and sound. The guest holds
    attack across a wall of six oak logs with a copper axe and hears each one break; then it digs
    stone with a wooden pickaxe and its game stalls for a second halfway: the stone is still there
    on both sides when it resumes, and the guest's own crack finishes it."""
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, y, z = gx + 3, gy, gz + 4
    heard = lambda sound: guest('status')['heard'].get(sound, 0)
    at = lambda b: (f'{b[0] + 0.02:.2f}', f'{b[1] + 0.5:.2f}', f'{b[2] + 0.5:.2f}')
    guest('command', command='gamemode survival')
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:copper_axe')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 4} {y + 3} {z + 2} minecraft:air')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 4} {y - 1} {z + 2} minecraft:stone')
    host('command', command=f'fill {x + 2} {y} {z - 1} {x + 2} {y + 1} {z + 1} minecraft:oak_log')
    guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
    guest.wait(30)
    logs = [(x + 2, y + dy, z + dz) for dz in (-1, 0, 1) for dy in (0, 1)]
    before = heard('minecraft:block.wood.break')
    guest.raw('look-at', *at(logs[0]))
    guest.raw('mouse', 'left', 'down')
    left = list(logs)
    deadline = time.time() + 20
    while left and time.time() < deadline:
        if guest.raw('block', *left[0]).get('block') != 'minecraft:oak_log':
            left.pop(0)
            if left:
                guest.raw('look-at', *at(left[0]))
    guest.raw('mouse', 'left', 'up')
    guest.wait(10)
    seen = {'logs_left': left, 'log_breaks_heard': heard('minecraft:block.wood.break') - before}

    # Stone with a wooden pickaxe: 23 ticks. The guest stalls for a second after about 8.
    stone = (x + 2, y, z)
    guest('command', command='item replace entity @s weapon.mainhand with minecraft:wooden_pickaxe')
    host('command', command=f'setblock {stone[0]} {stone[1]} {stone[2]} minecraft:stone')
    guest.wait(10)
    before = heard('minecraft:block.stone.break')
    guest.raw('look-at', *at(stone))
    guest.raw('mouse', 'left', 'down')
    guest.wait(8)
    guest.raw('stall', 1000)
    resumed = {'guest': guest.raw('block', *stone).get('block'), 'server': host.raw('block', *stone).get('block'),
               'mining': guest('state').get('mining')}
    # Let go once it breaks: past it the crosshair reaches the floor.
    deadline = time.time() + 5
    while guest.raw('block', *stone).get('block') == 'minecraft:stone' and time.time() < deadline:
        pass
    guest.raw('mouse', 'left', 'up')
    guest.wait(10)
    seen.update(resumed=resumed, stone_after=host.raw('block', *stone).get('block'),
                stone_breaks_heard=heard('minecraft:block.stone.break') - before)
    guest.shot(out, 'digging')
    assert not left and seen['log_breaks_heard'] == 6, f'the server broke logs before the guest did: {seen}'
    assert resumed['guest'] == resumed['server'] == 'minecraft:stone', f'the server broke the stone while the guest stalled: {seen}'
    assert seen['stone_after'] == 'minecraft:air' and seen['stone_breaks_heard'] == 1, f'the guest did not break the stone: {seen}'
    return seen


CHECKS += ['digging']
CHECK_FNS.update(digging=check_digging)


def check_instant_digging(host, guest, out):
    """#1908: an instant survival START has no STOP. ServerPlayerGameMode.tick
    forgets a dig once its block is air; it must neither hit air nor break a replacement.
    Exercise the in-frame host and network guest (both are clients with --server-thread).
    """
    result = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = (int(v // 1) for v in pos_of(player))
        cell = (x, y + 1, z + 2)
        player('command', command='gamemode survival')
        player('command', command='clear @s')
        host('command', command=f'fill {x - 1} {y} {z + 1} {x + 1} {y + 3} {z + 4} minecraft:air')
        host('command', command=f'setblock {x} {y} {z + 2} minecraft:sand')
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:dead_bush')
        player.wait(10)
        before = heard(player, 'minecraft:block.stone.hit')
        player.raw('look-at', x + 0.5, y + 1.4, z + 2.5)
        player.raw('mouse', 'left', 'click')
        player.wait(20)
        broken = host.raw('block', *cell).get('block')
        hits = heard(player, 'minecraft:block.stone.hit') - before
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:dead_bush')
        player.wait(20)
        replacement = host.raw('block', *cell).get('block')
        result[label] = {'broken': broken, 'stone_hits': hits, 'replacement': replacement}
        assert broken == 'minecraft:air', f'{label}: the bush did not break: {result}'
        assert hits == 0, f'{label}: hitting air after an instant break: {result}'
        assert replacement == 'minecraft:dead_bush', f'{label}: the server kept digging: {result}'
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:air')
    return result


CHECKS += ['instant_digging']
CHECK_FNS.update(instant_digging=check_instant_digging)


def check_vehicles(host, guest, out):
    """The host and the guest each (#1551):
    - in creative, break a block and have it put back: the server's dig ends with the break
      (ServerPlayerGameMode.handleBlockBreakAction), so the block stays;
    - land on a boat on the ground and stand on it (AbstractBoat.canBeCollidedWith);
    - sit in a boat that lava reaches: lavaHurt twice a tick breaks it at once, and the rider is out;
    - see a minecart in lava break, and a TNT minecart in lava prime and go off;
    - stand half in a minecart off rails and push it away (AbstractMinecart.push from the server's
      player)."""
    result = {}
    spots = apart(host, guest)
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        seen = result[label] = {}
        near = lambda kind, at: [o for o in host('status')['objects'] if o['kind'].endswith(kind)
                                 and abs(o['position'][0] - at[0]) < 2 and abs(o['position'][2] - at[2]) < 2]
        try:
            cell = (x, y + 1, z + 2)
            player('command', command='gamemode creative')
            player('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:stone')
            player.wait(5)
            player.raw('look-at', f'{x + 0.5:.2f}', f'{y + 1.5:.2f}', f'{z + 2.5:.2f}')
            player.raw('mouse', 'left', 'click')
            player.wait(5)
            seen['broken'] = host.raw('block', *cell).get('block')
            player('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:stone')
            player.wait(20)
            seen['put_back'] = [g.raw('block', *cell).get('block') for g in (host, player)]
            player('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:air')

            player('command', command='gamemode survival')
            player('command', command='effect give @s minecraft:fire_resistance 300 0 true')
            boat_at = (x + 0.5, y, z + 3.5)
            player('command', command=f'summon minecraft:oak_boat {boat_at[0]} {y} {boat_at[2]}')
            player.wait(10)
            player('command', command=f'tp @s {boat_at[0]} {y + 1.2:.2f} {boat_at[2]}')
            player.wait(20)
            seen['on_boat'] = [round(pos_of(player)[1] - y, 3), round(server_player(host, player)['position'][1] - y, 3)]

            player('command', command=f'tp @s {x + 0.5} {y} {z + 1.5} 0 0')
            player.wait(10)
            board(player, 'oak_boat', boat_at)
            player('command', command=f'setblock {x} {y} {z + 3} minecraft:lava')
            player.wait(10)
            seen['rider'] = {'riding': player('status').get('riding'), 'boats': len(near('oak_boat', boat_at))}

            player('command', command='gamemode creative')
            player('command', command=f'summon minecraft:minecart {boat_at[0]} {y} {boat_at[2]}')
            player.wait(10)
            seen['minecart_in_lava'] = len(near('minecart', boat_at))
            booms = player('status')['heard'].get('minecraft:entity.generic.explode', 0)
            player('command', command=f'summon minecraft:tnt_minecart {boat_at[0]} {y} {boat_at[2]}')
            player.wait(60)
            seen['tnt_minecart'] = {'left': len(near('tnt_minecart', boat_at)),
                                    'booms': player('status')['heard'].get('minecraft:entity.generic.explode', 0) - booms}

            player('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 3} {z + 6} minecraft:air')
            player('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 6} minecraft:stone')
            player('command', command='gamemode survival')
            cart_at = (x + 0.5, y, z + 1.6)
            player('command', command=f'summon minecraft:minecart {cart_at[0]} {y} {cart_at[2]}')
            player.wait(10)
            before = near('minecart', cart_at)[0]['position']
            # Half into the cart from the north: it is pushed south until they are apart.
            player('command', command=f'tp @s {x + 0.5} {y} {before[2] - 0.4:.2f} 0 0')
            player.wait(20)
            after = near('minecart', cart_at)[0]['position']
            seen['cart_pushed'] = round(after[2] - before[2], 3)
            player.shot(out, f'vehicles-{label}')
        finally:
            player('command', command='kill @e[type=minecraft:minecart]')
            player('command', command='kill @e[type=minecraft:oak_boat]')
            player('command', command='kill @e[type=minecraft:item]')
            player('command', command=f'fill {x - 3} {y} {z - 3} {x + 4} {y + 3} {z + 6} minecraft:air')
            player('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 4} {y - 1} {z + 6} minecraft:stone')
            player('command', command='effect clear @s')
            player('command', command='gamemode survival')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            player.wait(5)
    for label, seen in result.items():
        assert seen['broken'] == 'minecraft:air', f'{label}: the creative click broke nothing: {seen}'
        assert seen['put_back'] == ['minecraft:stone'] * 2, f'{label}: the server kept digging: {seen}'
        assert all(0.5 < dy < 0.65 for dy in seen['on_boat']), f'{label}: not standing on the boat: {seen}'
        assert seen['rider'] == {'riding': None, 'boats': 0}, f'{label}: lava left the boat or its rider: {seen}'
        assert seen['minecart_in_lava'] == 0, f'{label}: a minecart outlived lava: {seen}'
        assert seen['tnt_minecart'] == {'left': 0, 'booms': 1}, f'{label}: the TNT minecart in lava: {seen}'
        assert seen['cart_pushed'] > 0.1, f'{label}: the minecart stayed in the player: {seen}'
    return result


CHECKS += ['vehicles']
CHECK_FNS.update(vehicles=check_vehicles)


# How far a plain wooden sword hit slides a standing player in vanilla: LivingEntity.knockback(0.4) from
# the server copy's (0, -0.0784, 0), then the victim's own client.
VANILLA_PUSH = 1.989


def check_knockback(host, guest, out):
    if dead(guest):
        guest.raw('respawn')
        guest.wait(40)
    host('command', command='gamerule pvp true')
    for g in (host, guest):
        g('command', command='gamemode survival')
        g('command', command='item replace entity @s weapon.mainhand with minecraft:wooden_sword')
        g('command', command='effect give @s minecraft:resistance 1000 4 true')
    face_each_other(host, guest)
    pushes = {}
    try:
        for name, attacker, victim in (('host hits guest', host, guest), ('guest hits host', guest, host)):
            # The victim is never teleported (a teleport resets its velocity): the attacker steps up to it from
            # the west, then the east, then the west again.
            for side in (-3, 3, -3):
                vx, vy, vz = pos_of(victim)
                attacker('command', command=f'tp @s {vx + side:.2f} {vy:.2f} {vz:.2f}')
                attacker.wait(40)  # a full charge, and the last push long settled
                attacker.raw('look-at', f'{vx:.2f}', f'{vy + 1.6:.2f}', f'{vz:.2f}')
                attacker.wait(5)
                before = pos_of(victim)
                attacker.raw('attack')
                victim.wait(30)
                after = pos_of(victim)
                pushes.setdefault(name, []).append(round(math.dist(before[0::2], after[0::2]), 3))
            attacker.shot(out, 'knockback-' + name.replace(' ', '-'))
    finally:
        for g in (host, guest):
            g('command', command='effect clear @s minecraft:resistance')
    for name, distances in pushes.items():
        assert all(abs(d - VANILLA_PUSH) < 0.03 for d in distances), f'{name}: not {VANILLA_PUSH} each time: {pushes}'
    return pushes


CHECKS += ['knockback']
CHECK_FNS.update(knockback=check_knockback)



def check_fixture_input(host, guest, out, kind, role):
    """Complement seeded rules with real client input and authoritative observations."""
    player = host if role == 'host' else guest
    hx, y, hz = (int(v // 1) for v in pos_of(host))
    x, z = hx + 6, hz + (-8 if role == 'host' else 8)
    sensor, dispenser, button = (x + 3, y, z + 1), (x, y, z + 3), (x, y, z + 2)
    player('command', command='gamemode survival')
    player('command', command='item replace entity @s weapon.mainhand with minecraft:air')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 5} {y - 1} {z + 5} minecraft:stone')
    host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')
    start = (x + 0.5, y, z + 0.5)
    player('command', command=f'tp @s {start[0]} {start[1]} {start[2]} 0 0')
    player.until(lambda: pos_of(player), lambda p: math.dist(p, start) < 0.1)
    player.raw('look', 0, 0)
    try:
        if kind == 'sculk':
            player.wait(2)  # finish setup movement before installing the listener
            host.raw('setblock', *sensor, 'minecraft:sculk_sensor')
            phase = lambda: host.raw('getblock', *sensor)['block']
            host.until(phase, lambda state: 'sculk_sensor_phase=inactive' in state)
            player.wait(10)
            assert 'sculk_sensor_phase=inactive' in phase(), 'setup activated the sensor before player input'
            player.raw('key', 'w', 'down')
            try:
                player.wait(15)  # defined input duration; no wall-clock simulation sleep
            finally:
                player.raw('key', 'w', 'up')
            moved = {'client': math.dist(pos_of(player), start),
                     'server': math.dist(server_player(host, player)['position'], start)}
            assert all(d > 1 for d in moved.values()), f'{role} did not walk on both sides: {moved}'
            active = host.until(phase, lambda state: 'sculk_sensor_phase=active' in state, ticks=85)
            for viewer in (host, guest):
                viewer.until(lambda: state_of(viewer, sensor, 'sculk_sensor_phase'), lambda v: v == 'active')
            return {'moved': moved, 'phase': active}
        host.raw('setblock', *dispenser, 'minecraft:dispenser[facing=east]')
        host.raw('container', *dispenser, 'minecraft:diamond', 1)
        host.raw('setblock', *button, 'minecraft:stone_button[face=wall,facing=north]')
        player.until(lambda: player.raw('block', *button)['block'], lambda v: v == 'minecraft:stone_button')
        player.raw('interact', *button)
        slots = host.until(lambda: host.raw('container', *dispenser)['slots'], lambda v: all(s is None for s in v))
        dropped = {}
        for viewer in (host, guest):
            dropped[viewer.name] = viewer.until(
                lambda: [e for e in viewer.raw('entities', 24, 'minecraft:item')['entities']
                         if e.get('item', {}).get('item') == 'minecraft:diamond'
                         and math.dist(e['pos'], (x + 1.5, y + 0.5, z + 3.5)) < 3],
                lambda drops: len(drops) == 1 and drops[0]['item']['count'] == 1)
        player.shot(out, f'{kind}-input')
        return {'slots': slots, 'drops': dropped}
    finally:
        player.raw('key', 'w', 'up')
        host('command', command=f'fill {x - 2} {y} {z - 2} {x + 5} {y + 3} {z + 5} minecraft:air')


for _kind in ('sculk', 'dispenser'):
    for _role in ('host', 'guest'):
        _name = f'{_kind}_input_{_role}'
        CHECKS.append(_name)
        CHECK_FNS[_name] = lambda h, g, o, kind=_kind, role=_role: check_fixture_input(h, g, o, kind, role)


def check_hero_trading(host, guest, out):
    """Villager.updateSpecialPrices keeps the trading player's Hero effect: the offer sent to the
    client and the server's payment consumption agree, independently of the other player's effect."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    tag = 'hero_trade'
    seen = {}
    cases = [('ordinary', None, 20), ('hero-i', 0, 14), ('hero-iii', 2, 12), ('cleared', None, 20)]
    try:
        host('command', command=f'fill {x - 4} {y - 1} {z - 2} {x + 4} {y - 1} {z + 2} minecraft:stone')
        host('command', command=f'fill {x - 4} {y} {z - 2} {x + 4} {y + 3} {z + 2} minecraft:air')
        for game, other in ((host, guest), (guest, host)):
            back_to_game(game)
            back_to_game(other)
            other('command', command='effect clear @s')
            other('command', command='effect give @s minecraft:hero_of_the_village 600 4 true')
            other('command', command=f'tp @s {x - 3.5} {y} {z + 0.5}')
            game('command', command='gamemode survival')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            for label, amplifier, price in cases:
                game('command', command='effect clear @s')
                if amplifier is not None:
                    game('command', command=f'effect give @s minecraft:hero_of_the_village 600 {amplifier} true')
                game('command', command='clear @s')
                game('command', command='give @s minecraft:wheat 20')
                # A fresh, fixed offer avoids reputation, demand and level-ups between cases.
                host('command', command=f'summon minecraft:villager {x + 2.5} {y} {z + 0.5} {{Tags:["{tag}"],'
                     'NoAI:1b,PersistenceRequired:1b,Xp:1,VillagerData:{type:"minecraft:plains",'
                     'profession:"minecraft:farmer",level:1},Offers:{Recipes:[{buy:{id:"minecraft:wheat",count:20},'
                     'sell:{id:"minecraft:emerald",count:1},maxUses:16,uses:0,xp:0,priceMultiplier:0f,demand:0}]}}')
                game.wait(20)
                villager = next(e['id'] for e in game.raw('entities', 8, 'minecraft:villager')['entities']
                                if math.dist(e['pos'], [x + 2.5, y, z + 0.5]) < 0.5)
                game.raw('interact-entity', villager)
                game.raw('wait-for', 'screen', 'station', '--timeout', 10)
                assert game('screen')['station'] == 'Merchant', f'{game.name}-{label}: no merchant screen'
                # The first offer button is 50 pixels left of the usual menu origin. Selecting it
                # sends SelectTrade on a client; taking the result uses the authoritative menu.
                game.raw('click-gui', -1, 28)
                game.wait(10)
                assert station_slots(game).get(650) == ('minecraft:emerald', 1), f'{game.name}-{label}: no result'
                game.shot(out, f'hero-trading-{label}')
                game.raw('click-slot', 650, 'shift')
                game.wait(10)
                game.raw('key', 'esc', 'tap')
                game.wait(10)
                offer = server_reply(game, f'data get entity @e[type=minecraft:villager,tag={tag},limit=1] '
                                     'Offers.Recipes[0]', 'following entity data')
                special = re.search(r'specialPrice"?\s*:\s*(-?\d+)', offer)
                case = {'price': 20 + (int(special[1]) if special else 0),
                        'server': {item: server_count(game, item) for item in ('wheat', 'emerald')},
                        'hero': 'hero_of_the_village' in effect_ids(game)}
                seen[f'{game.name}-{label}'] = case
                want = {'price': price, 'server': {'wheat': 20 - price, 'emerald': 1},
                        'hero': amplifier is not None}
                assert case == want, f'{game.name}-{label}: expected {want}, got {case}'
                host('command', command=f'kill @e[tag={tag}]')
                game.wait(5)
    finally:
        for game in (host, guest):
            back_to_game(game)
            game('command', command='effect clear @s')
            game('command', command='clear @s')
        host('command', command=f'kill @e[tag={tag}]')
    return seen


CHECKS += ['hero-trading']
CHECK_FNS.update({'hero-trading': check_hero_trading})


def check_cure_discount(host, guest, out):
    """#3306: ZombieVillager.finishConversion credits the player who fed the golden apple
    (conversionStarter, level.getPlayerByUUID): the ZOMBIE_VILLAGER_CURED gossip (+20 major and +25
    minor positive, reputation 125) lowers that player's price by floor(125 * 0.05) = 6, for the
    host on its server thread and for a guest alike."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    tag = 'cure_trade'
    at = (x + 2.5, y, z + 0.5)
    seen = {}
    try:
        host('command', command=f'fill {x - 4} {y - 1} {z - 2} {x + 4} {y - 1} {z + 2} minecraft:stone')
        host('command', command=f'fill {x - 4} {y} {z - 2} {x + 4} {y + 3} {z + 2} minecraft:air')
        host('command', command='time set midnight')
        for game, other in ((host, guest), (guest, host)):
            back_to_game(game)
            back_to_game(other)
            other('command', command=f'tp @s {x - 3.5} {y} {z + 0.5}')
            game('command', command='gamemode survival')
            game('command', command='effect clear @s')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} -90 0')
            game('command', command='clear @s')
            game('command', command='give @s minecraft:golden_apple')
            game.wait(10)
            game.raw('select', 'minecraft:golden_apple')
            host('command', command=f'summon minecraft:zombie_villager {at[0]} {at[1]} {at[2]} '
                 f'{{Tags:["{tag}"],NoAI:1b,PersistenceRequired:1b,Xp:1,VillagerData:{{type:"minecraft:plains",'
                 'profession:"minecraft:farmer",level:1},Offers:{Recipes:[{buy:{id:"minecraft:wheat",count:20},'
                 'sell:{id:"minecraft:emerald",count:1},maxUses:16,uses:0,xp:0,priceMultiplier:0.05f,demand:0}]}}')
            host('command', command=f'effect give @e[tag={tag}] minecraft:weakness 60 0')
            game.wait(20)
            zombie = next(e['id'] for e in game.raw('entities', 8, 'minecraft:zombie_villager')['entities']
                          if math.dist(e['pos'], at) < 0.5)
            # ZombieVillager.mobInteract: the apple starts the cure; then it is cut short.
            game.raw('interact-entity', zombie)
            game.wait(10)
            # Horizon counts the cure down in the AI step, so NoAI goes for the last tick.
            host('command', command=f'data merge entity @e[tag={tag},limit=1] {{ConversionTime:1,NoAI:0b}}')
            villager = None
            for _ in range(30):
                game.wait(5)
                villager = next((e['id'] for e in game.raw('entities', 8, 'minecraft:villager')['entities']
                                 if math.dist(e['pos'], at) < 2), None)
                if villager is not None:
                    break
            assert villager is not None, f'{game.name}: the zombie villager was not cured'
            near = f'x={at[0]},y={at[1]},z={at[2]},distance=..3,sort=nearest,limit=1'
            host('command', command=f'data merge entity @e[type=minecraft:villager,{near}] '
                 f'{{NoAI:1b,Tags:["{tag}"]}}')
            game('command', command='clear @s')
            game('command', command='give @s minecraft:wheat 20')
            game.wait(10)
            game.raw('interact-entity', villager)
            game.raw('wait-for', 'screen', 'station', '--timeout', 10)
            assert game('screen')['station'] == 'Merchant', f'{game.name}: no merchant screen'
            game.raw('click-gui', -1, 28)
            game.wait(10)
            assert station_slots(game).get(650) == ('minecraft:emerald', 1), f'{game.name}: no result'
            game.shot(out, f'cure-discount-{game.name}')
            game.raw('click-slot', 650, 'shift')
            game.wait(10)
            game.raw('key', 'esc', 'tap')
            game.wait(10)
            case = {item: server_count(game, item) for item in ('wheat', 'emerald')}
            seen[game.name] = case
            want = {'wheat': 20 - 14, 'emerald': 1}
            assert case == want, f'{game.name}: expected {want}, got {case}'
            host('command', command=f'kill @e[tag={tag}]')
            game.wait(5)
    finally:
        for game in (host, guest):
            back_to_game(game)
            game('command', command='clear @s')
        host('command', command=f'kill @e[tag={tag}]')
        host('command', command='time set day')
    return seen


CHECKS += ['cure-discount']
CHECK_FNS.update({'cure-discount': check_cure_discount})


def check_spectator_travel(host, guest, out):
    """Player.tick's spectator noPhysics survives a replacement body after dimension travel (#1299)."""
    players = (host, guest) if SERVER_THREAD else (host,)
    seen = {}
    for game in players:
        home = pos_of(game)
        game('command', command='gamemode spectator')
        game.wait(10)
        try:
            for dimension in (NETHER, OVERWORLD):
                game('command', command=f'execute in {dimension} run tp @s .5 102 .5 0 0')
                there = wait_dimension(game, dimension, 600)
                assert there.get('dimension') == dimension, f'{game.name} stayed in {there.get("dimension")}'
                # A replacement body arrives before its chunks; fill needs the whole corridor loaded.
                for _ in range(30):
                    if all(game.raw('block', *pos).get('loaded') for pos in ((-2, 101, -1), (2, 105, 16))):
                        break
                    game.wait(20)
                game('command', command='fill -2 101 -1 2 105 16 minecraft:air')
                game('command', command='fill -2 101 3 2 105 3 minecraft:stone')
                game.wait(10)
                wall = game.raw('block', 0, 102, 3)
                assert wall.get('loaded') and wall.get('block') == 'minecraft:stone', f'wall: {wall}'
                label = f'spectator-{dimension.removeprefix("minecraft:")}'
                game.shot(out, f'{label}-before')
                game.raw('key', 'w', 'down')
                game.wait(20)
                game.raw('key', 'w', 'up')
                game.wait(10)
                state = game('state')
                client = state['player']
                server = server_player(host, game)
                seen[f'{game.name}-{dimension}'] = {'client': client['position'], 'server': server['position']}
                game.shot(out, f'{label}-after')
                assert state.get('gamemode') == 'spectator' and client['flying'], f'{game.name} lost spectator flight'
                assert client['position'][2] > 4 and server['position'][2] > 4, \
                    f'{game.name} collides after travel to {dimension}: {seen}'
                game('command', command='fill -2 101 3 2 105 3 minecraft:air')
        finally:
            game.raw('key', 'w', 'up')
            game('command', command=f'execute in {OVERWORLD} run tp @s {home[0]:.2f} {home[1]:.2f} {home[2]:.2f}')
            wait_dimension(game, OVERWORLD, 600)
            game('command', command='gamemode survival')
            game.wait(20)
    return seen


CHECKS += ['spectator-travel']
CHECK_FNS.update({'spectator-travel': check_spectator_travel})


def check_villagers(host, guest, out):
    """Villager.DATA_VILLAGER_DATA and ZombieVillager's reach the guest: summoned villagers show
    their type, profession and level, and an unemployed villager that takes a composter placed
    afterwards turns into a farmer on the guest too (VillagerProfessionLayer draws from that data)."""
    gx, gy, gz = (int(v // 1) for v in pos_of(guest))
    x, y, z = gx, gy, gz + 5
    tag = 'vs_villager'
    summoned = [('villager', -2, 'desert', 'librarian', 3), ('villager', 0, 'snow', 'weaponsmith', 5),
                ('zombie_villager', 2, 'taiga', 'cleric', 2), ('villager', 4, 'plains', 'none', 1)]

    def villagers(game):
        return {e['id']: e['villager'] for e in game.raw('entities', 16)['entities'] if 'villager' in e}

    seen = {}
    try:
        host('command', command='time set 2000')
        host('command', command=f'fill {x - 4} {y - 1} {z - 3} {x + 6} {y - 1} {z + 4} minecraft:stone')
        host('command', command=f'fill {x - 4} {y} {z - 3} {x + 6} {y + 3} {z + 4} minecraft:air')
        for kind, dx, t, p, level in summoned:
            # The unemployed one keeps its AI to take the job.
            ai = '' if p == 'none' else 'NoAI:1b,Invulnerable:1b,'
            host('command', command=f'summon minecraft:{kind} {x + dx + 0.5} {y} {z + 0.5} {{Tags:["{tag}"],{ai}'
                                    f'PersistenceRequired:1b,Rotation:[180f,0f],'
                                    f'VillagerData:{{type:"minecraft:{t}",profession:"minecraft:{p}",level:{level}}}}}')
        guest.wait(20)
        seen['summoned'] = {'host': villagers(host), 'guest': villagers(guest)}
        host('command', command=f'setblock {x + 5} {y} {z + 2} minecraft:composter')
        for waited in range(10, 1210, 10):
            guest.wait(10)
            farmers = [i for i, v in villagers(host).items() if v['profession'] == 'farmer']
            if farmers:
                seen['ticks to the job'] = waited
                break
        guest.wait(10)
        seen['job'] = {'host': villagers(host), 'guest': villagers(guest)}
        guest('command', command=f'tp @s {x + 1.5} {y} {z - 2.5} 0 10')
        guest.wait(10)
        guest.shot(out, 'villagers')
    finally:
        host('command', command=f'kill @e[tag={tag}]')
        host('command', command=f'fill {x - 4} {y} {z - 3} {x + 6} {y + 3} {z + 4} minecraft:air')
        host('command', command='time set day')
    host_view, guest_view = seen['summoned']['host'], seen['summoned']['guest']
    for _, _, t, p, level in summoned:
        want = {'type': t, 'profession': p, 'level': level}
        ids = [i for i, v in host_view.items() if v == want]
        assert ids, f'the host lacks {want}: {seen}'
        assert guest_view.get(ids[0]) == want, f'the guest sees {guest_view.get(ids[0])} for {want}: {seen}'
    farmers = [i for i, v in seen['job']['host'].items() if v['profession'] == 'farmer']
    assert farmers, f'no villager took the job: {seen}'
    assert seen['job']['guest'].get(farmers[0]) == seen['job']['host'][farmers[0]], \
        f'the guest missed the new job: {seen}'
    return seen


CHECKS += ['villagers']
CHECK_FNS.update(villagers=check_villagers)


def check_stopsound(host, guest, out):
    """StopSoundCommand sends each target ClientboundStopSoundPacket: a player's own /stopsound by
    source and by event reaches its own client's sound engine, and the host's /stopsound of the
    guest reaches the guest's (sounds stopped, counted in the status as `heard` is, audio or not)."""
    event = 'minecraft:music_disc.cat'

    def stopped(game):
        return game('status')['stopped_sounds']

    seen = {}
    for game in (host, guest):
        before = stopped(game)
        game('command', command='stopsound @s record')
        game('command', command=f'stopsound @s * {event}')
        game.wait(10)
        after = stopped(game)
        seen[game.name] = {k: after.get(k, 0) - before.get(k, 0) for k in after}
        assert seen[game.name].get('record *') == 1 and seen[game.name].get(f'* {event}') == 1, \
            f'{game.name}: its own /stopsound did not reach its client: {seen}'
    before = {g.name: stopped(g) for g in (host, guest)}
    host('command', command=f'stopsound {GUEST_NAME}')
    guest.wait(10)
    for game in (host, guest):
        after = stopped(game)
        seen[f'{game.name}-by-host'] = after.get('* *', 0) - before[game.name].get('* *', 0)
    assert seen['guest-by-host'] == 1 and seen['host-by-host'] == 0, \
        f"the host's /stopsound of the guest stopped the wrong client: {seen}"
    return seen


CHECKS += ['stopsound']
CHECK_FNS.update(stopsound=check_stopsound)


def check_riding_teleport(host, guest, out):
    """#1473: a chat /tp dismounts each player and leaves the horse behind in both modes;
    teleporting another player exercises the guest-target command path too."""
    spots = apart(host, guest)
    result = {}
    for label, player, source in (('host', host, None), ('guest', guest, None),
                                  ('guest-targeted', guest, host)):
        x, y, z = spots['host' if player is host else 'guest']
        horse_at = (x + 0.5, y, z + 2.5)
        destination = (x + 3.5, y, z + 5.5)
        tag = 'tp_horse'
        try:
            player('command', command='clear @s')
            player('command', command='gamemode survival')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
            player('command', command='item replace entity @s weapon.mainhand with minecraft:saddle')
            player('command', command=f'summon minecraft:horse {horse_at[0]} {y} {horse_at[2]} '
                                     f'{{Tame:1b,NoAI:1b,NoGravity:1b,Tags:["{tag}"]}}')
            player.wait(5)
            horses = [m for m in player.raw('mobs', 16)['near']
                      if m['kind'] == 'minecraft:horse' and m['health'] > 0]
            horse = next((m for m in horses if math.dist(m['pos'], horse_at) < 0.1), None)
            assert horse, f'{label}: no live horse at {horse_at}: {horses}'
            player.raw('look-at', horse_at[0], y + 0.8, horse_at[2])
            player.raw('mouse', 'right', 'click')
            player.wait(5)
            player('command', command='item replace entity @s weapon.mainhand with minecraft:air')
            player.wait(2)
            player.raw('mouse', 'right', 'click')
            player.wait(5)
            assert player('status').get('riding') == horse['id'], f'{label}: not riding the horse'
            player.shot(out, f'riding-tp-{label}-before')
            coordinates = ' '.join(map(str, destination))
            if source is None:
                player.raw('key', 't', 'tap')
                player.raw('type', f'/tp @s {coordinates}')
                player.raw('key', 'enter', 'tap')
            else:
                source('command', command=f'tp {GUEST_NAME} {coordinates}')
            player.wait(60)
            player.shot(out, f'riding-tp-{label}-after')
            seen = {'riding': player('status').get('riding'), 'client': pos_of(player),
                    'server': server_player(host, player)['position'],
                    'horse': mob_seen(player, horse['id'])['pos'],
                    'other_horse': mob_seen(guest if player is host else host, horse['id'])['pos']}
            assert seen['riding'] is None, f'{label}: still mounted after /tp: {seen}'
            assert all(math.dist(seen[side], destination) < 0.01 for side in ('client', 'server')), \
                f'{label}: /tp did not keep its destination after 60 ticks: {seen}'
            assert all(math.dist(seen[side], horse_at) < 0.01 for side in ('horse', 'other_horse')), \
                f'{label}: /tp moved the horse: {seen}'
            result[label] = seen
        finally:
            back_to_game(player)
            player('command', command=f'kill @e[tag={tag}]')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
            # The next case uses this spot again: let the previous horse's death animation finish.
            player.wait(25)
    return result


CHECKS += ['riding_teleport']
CHECK_FNS.update(riding_teleport=check_riding_teleport)


def check_portal_rider(host, guest, out):
    """#1486 (--server-thread): a survival guest rides a horse into a Nether portal. The horse goes
    through at once (NetherPortalBlock.getPortalTransitionTime is 0 for an entity that isn't a
    player; a passenger has no portal process of its own, Entity.canUsePortal) and carries its rider
    (Entity.teleportCrossDimension): the guest leaves well before a player's own 80 ticks and
    arrives in the Nether still on the same horse."""
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    x, y, z = apart(host, guest)['guest']
    horse_at = (x + 1.0, y, z + 2.5)
    # The portal's plane crosses the horse's box (z + 1.8 to z + 3.2), the rider's too.
    pz = z + 2
    tag = 'portal_rider'
    home = 'execute in minecraft:overworld run'
    try:
        guest('command', command='clear @s')
        guest('command', command='gamemode survival')
        guest('command', command='gamerule players_nether_portal_default_delay 80')
        guest('command', command='item replace entity @s weapon.mainhand with minecraft:saddle')
        guest('command', command=f'summon minecraft:horse {horse_at[0]} {y} {horse_at[2]} '
                                 f'{{Tame:1b,NoAI:1b,Tags:["{tag}"]}}')
        guest.wait(5)
        horses = [m for m in guest.raw('mobs', 16)['near'] if m['kind'] == 'minecraft:horse' and m['health'] > 0]
        horse = next((m for m in horses if math.dist(m['pos'], horse_at) < 0.1), None)
        assert horse, f'no live horse at {horse_at}: {horses}'
        guest.raw('look-at', horse_at[0], y + 0.8, horse_at[2])
        guest.raw('mouse', 'right', 'click')
        guest.wait(5)
        guest('command', command='item replace entity @s weapon.mainhand with minecraft:air')
        guest.wait(2)
        guest.raw('mouse', 'right', 'click')
        guest.wait(5)
        assert guest('status').get('riding') == horse['id'], 'the guest is not riding the horse'
        # The frame around the horse and its rider, then the portal inside it.
        for corner in (f'{x - 1} {y - 1} {pz} {x + 2} {y - 1} {pz}', f'{x - 1} {y + 3} {pz} {x + 2} {y + 3} {pz}',
                       f'{x - 1} {y} {pz} {x - 1} {y + 2} {pz}', f'{x + 2} {y} {pz} {x + 2} {y + 2} {pz}'):
            guest('command', command=f'fill {corner} minecraft:obsidian')
        guest('command', command=f'fill {x} {y} {pz} {x + 1} {y + 2} {pz} minecraft:nether_portal[axis=x]')
        guest.shot(out, 'portal-rider-in')
        left = None
        for waited in range(0, 120, 5):
            if guest('state').get('dimension') == NETHER:
                left = waited
                break
            guest.wait(5)
        assert left is not None, 'the guest on its horse did not go through the portal in 120 ticks'
        assert left < 40, f'the guest left after {left} ticks: its own delay (80), not the horse\'s (0)'
        # The server holds the guest until the chunks around the exit are lit, then places it there.
        there = wait_dimension(guest, NETHER, 600)
        seen = None
        for _ in range(30):
            guest.wait(20)
            seen = {'riding': guest('status').get('riding'), 'horse': mob_seen(guest, horse['id']),
                    'at': pos_of(guest)}
            if seen['horse'] and math.dist(seen['horse']['pos'], seen['at']) < 3:
                break
        guest.wait(40)
        seen = {'riding': guest('status').get('riding'), 'horse': mob_seen(guest, horse['id']), 'at': pos_of(guest)}
        guest.shot(out, 'portal-rider-nether')
        assert there.get('dimension') == NETHER, f'the guest is in {there.get("dimension")}'
        assert seen['horse'], f'the horse {horse["id"]} did not come to the Nether with its rider: {seen}'
        assert seen['riding'] == horse['id'], f'the guest arrived on foot: {seen}'
        assert math.dist(seen['horse']['pos'], seen['at']) < 3, f'the horse is not under its rider: {seen}'
        host_dim = host('state').get('dimension')
        assert host_dim == OVERWORLD, f'the host went to {host_dim} with the guest'
        return {'left_after_ticks': left, 'riding': seen['riding'], 'horse': seen['horse']['pos'], 'guest': seen['at']}
    finally:
        back_to_game(guest)
        guest('command', command=f'kill @e[tag={tag}]')
        guest('command', command=f'{home} fill {x - 1} {y - 1} {pz} {x + 2} {y + 3} {pz} minecraft:air')
        guest('command', command=f'{home} fill {x - 1} {y - 1} {pz} {x + 2} {y - 1} {pz} minecraft:stone')
        guest('command', command=f'{home} tp @s {x + 0.5} {y} {z + 0.5} 0 0')
        wait_dimension(guest, OVERWORLD, 600)
        guest.wait(20)


CHECKS += ['portal_rider']
CHECK_FNS.update(portal_rider=check_portal_rider)


def check_recipe_removal(host, guest, out):
    """ServerRecipeBook.removeRecipes sends ClientboundRecipeBookRemovePacket to the player:
    the open book updates immediately, reopening keeps it empty, and later grants still arrive."""
    seen = {}
    for game in (host, guest):
        back_to_game(game)
        game('command', command='clear @s')
        game('command', command='give @s minecraft:oak_log 4')
        game('command', command='recipe give @s minecraft:oak_planks')
        game('command', command='recipe give @s minecraft:stick')
        game.wait(40)
        game.raw('key', 'e', 'tap')
        game.wait(5)
        try:
            book = game('screen').get('recipe_book')
            assert book, f'{game.name}: the inventory has no recipe book'
            if len(book['targets']) == 1:
                game.raw('click-at', *centre(book['targets'][0]))
                game.wait(5)
                book = game('screen')['recipe_book']
            known = book['known']
            assert known > 1, f'{game.name}: recipes were not granted: {book}'
            game.shot(out, 'recipe-removal-before')

            game('command', command='recipe take @s *')
            game.wait(20)
            empty = game('screen')['recipe_book']['known']
            game.shot(out, 'recipe-removal-empty')
            assert empty == 0, f'{game.name}: forgotten recipes remain in the open book: {empty}'
            game.raw('key', 'esc', 'tap')
            game.wait(5)
            game.raw('key', 'e', 'tap')
            game.wait(5)
            reopened = game('screen')['recipe_book']['known']
            assert reopened == 0, f'{game.name}: reopening restored {reopened} forgotten recipes'

            game('command', command='recipe give @s minecraft:oak_planks')
            game('command', command='recipe give @s minecraft:stick')
            game.wait(20)
            given = game('screen')['recipe_book']['known']
            assert given == 2, f'{game.name}: two later grants did not appear: {given}'
            game('command', command='recipe take @s minecraft:oak_planks')
            game.wait(20)
            one = game('screen')['recipe_book']['known']
            assert one == 1, f'{game.name}: taking one recipe left {one}, expected 1'
            game('command', command='recipe take @s minecraft:stick')
            game.wait(20)
            assert game('screen')['recipe_book']['known'] == 0, f'{game.name}: the last recipe remains'

            game('command', command='recipe give @s minecraft:oak_planks')
            game.wait(20)
            granted = game('screen')['recipe_book']['known']
            assert granted == 1, f'{game.name}: a later grant did not appear: {granted}'
            game.shot(out, 'recipe-removal-granted')
            seen[game.name] = {'known': known, 'empty': empty, 'reopened': reopened,
                               'given': given, 'one': one, 'granted': granted}
        finally:
            back_to_game(game)
    return seen


CHECKS += ['recipe_removal']
CHECK_FNS.update(recipe_removal=check_recipe_removal)


def check_recipe_settings(host, guest, out):
    """RecipeBookComponent sends both settings to the server; each player's
    own book keeps them when the authoritative world saves and reopens (#2290)."""
    def inventory(game):
        back_to_game(game)
        game.raw('key', 'e', 'tap')
        game.wait(5)
        return game('screen')['recipe_book']

    def flags(book):
        return {'open': book['open'], 'filtering': book['filtering']}

    def set_flags(game, opened, filtering):
        book = inventory(game)
        if not book['open']:
            game.raw('click-at', *centre(book['targets'][0]))
            game.wait(5)
            book = game('screen')['recipe_book']
        if book['filtering'] != filtering:
            target = next(t for t in book['targets'] if t[2:] == [26.0, 16.0])
            game.raw('click-at', *centre(target))
            game.wait(5)
        if not opened:
            book = game('screen')['recipe_book']
            game.raw('click-at', *centre(book['targets'][0]))
            game.wait(5)
        actual = flags(game('screen')['recipe_book'])
        assert actual == {'open': opened, 'filtering': filtering}, f'{game.name}: {actual}'
        game.shot(out, 'recipe-settings-before')
        back_to_game(game)
        return actual

    # Deliberately different settings ensure neither player's book overwrites the other's.
    before = {host.name: set_flags(host, True, True),
              guest.name: set_flags(guest, False, True)}
    host.wait(20)
    guest.raw('disconnect')
    host.raw('pause')
    host.raw('click-button', 'Save and Quit to Title')
    host.raw('wait-for', 'screen', 'title', '--timeout', 120)
    host.raw('open', 'Lan')
    host('wait_ready', seconds=400)
    port = host.raw('lan', 'open', 0, 'commands')['port']
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    after = {}
    for game in (host, guest):
        after[game.name] = flags(inventory(game))
        game.shot(out, 'recipe-settings-reopened')
        back_to_game(game)
    assert after == before, f'saved recipe settings reset: before={before}, after={after}'
    return {'before': before, 'reopened': after}


CHECKS += ['recipe_settings']
CHECK_FNS.update(recipe_settings=check_recipe_settings)


def check_recipe_targets(host, guest, out):
    """RecipeCommand gives and takes the recipes of each target's own ServerRecipeBook: a
    `/recipe give|take <other player>` changes that player's book and reaches their client,
    not the book of the player who typed it. An in-frame host isn't one of the players a guest's
    command can name (`command_players` lists the guests), so the guest gives the host a recipe
    only when the host is a client of its server thread."""
    host_name = host.raw('lan', 'status').get('host', 'Player')
    books = {}
    for game in (host, guest):
        back_to_game(game)
        game('command', command='clear @s')
        game('command', command='recipe take @s *')
    host.wait(20)
    for game in (host, guest):
        game.raw('key', 'e', 'tap')
        game.wait(5)
        book = game('screen').get('recipe_book')
        assert book, f'{game.name}: the inventory has no recipe book'
        if len(book['targets']) == 1:
            game.raw('click-at', *centre(book['targets'][0]))
            game.wait(5)

    def known():
        return {game.name: game('screen')['recipe_book']['known'] for game in (host, guest)}

    try:
        books['empty'] = known()
        assert books['empty'] == {host.name: 0, guest.name: 0}, books
        host('command', command=f'recipe give {GUEST_NAME} minecraft:oak_planks')
        host('command', command=f'recipe give {GUEST_NAME} minecraft:stick')
        host.wait(20)
        books['host gives guest'] = known()
        if SERVER_THREAD:
            guest('command', command=f'recipe give {host_name} minecraft:stick')
            guest.wait(20)
        books['guest gives host'] = known()
        host('command', command=f'recipe take {GUEST_NAME} minecraft:stick')
        host.wait(20)
        books['host takes from guest'] = known()
        host.shot(out, 'recipe-targets-host')
        guest.shot(out, 'recipe-targets-guest')
        by_guest = int(SERVER_THREAD)
        expected = {'empty': (0, 0), 'host gives guest': (0, 2), 'guest gives host': (by_guest, 2),
                    'host takes from guest': (by_guest, 1)}
        for step, (in_host, in_guest) in expected.items():
            assert books[step] == {host.name: in_host, guest.name: in_guest}, (step, books)
    finally:
        back_to_game(host)
        back_to_game(guest)
    return books


CHECKS += ['recipe_targets']
CHECK_FNS.update(recipe_targets=check_recipe_targets)


def check_shulker_bullets(host, guest, out):
    """ShulkerAttackGoal's bullets reach ClientPacketListener.handleAddEntity, then the existing
    projectile renderer and interpolation, with distinct ids from the server's entity counter."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    tag = 'lan_shulker_bullets'
    selector = f'@e[type=minecraft:shulker,tag={tag},limit=1]'
    sound = 'minecraft:entity.shulker.shoot'
    paths = {g.name: {} for g in (host, guest)}
    before = {g.name: heard(g, sound) for g in (host, guest)}
    shots = dict.fromkeys(before, 0)
    first_shot = stopped = None
    photographed = False

    def bullets(status):
        return [p for p in status['projectiles'] if p['kind'] == 'ShulkerBullet']

    try:
        host('command', command='difficulty normal')
        host('command', command='gamemode creative')
        guest('command', command='gamemode survival')
        guest('command', command='effect give @s minecraft:resistance 120 4 true')
        host('command', command=f'fill {x - 10} {y - 1} {z - 10} {x + 10} {y - 1} {z + 18} minecraft:stone')
        host('command', command=f'fill {x - 10} {y} {z - 10} {x + 10} {y + 12} {z + 18} minecraft:air')
        # A west wall supports the shulker above the floor, leaving room for a short flight.
        host('command', command=f'fill {x - 1} {y} {z + 7} {x - 1} {y + 4} {z + 7} minecraft:stone')
        host('command', command=f'tp @s {x + 4.5} {y} {z - 0.5}')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        host('command', command=f'summon minecraft:shulker {x + 0.5} {y + 3} {z + 7.5} '
                                f'{{NoAI:1b,PersistenceRequired:1b,Tags:["{tag}"]}}')
        for game in (host, guest):
            game.raw('look-at', x + 0.5, y + 3.5, z + 7.5)
        guest.wait(20)
        host('command', command=f'data merge entity {selector} {{NoAI:0b}}')
        for waited in range(2, 402, 2):
            guest.wait(2)
            for game in (host, guest):
                status = game('status')
                live = bullets(status)
                ids = [p['id'] for p in live]
                assert all(ids) and len(set(ids)) == len(ids), f'{game.name}: invalid bullet ids {ids}'
                for p in live:
                    paths[game.name].setdefault(p['id'], []).append(p['position'])
                shots[game.name] = status['heard'].get(sound, 0) - before[game.name]
            if shots['host'] and first_shot is None:
                first_shot = waited
            if first_shot is not None and waited - first_shot >= 8 and not photographed:
                host('command', command='tick freeze')
                try:
                    for game in (host, guest):
                        game.raw('look-at', x + 0.5, y + 3.5, z + 7.5)
                        game.raw('key', 'f1', 'tap')
                        try:
                            game.shot(out, 'shulker-bullets')
                        finally:
                            game.raw('key', 'f1', 'tap')
                finally:
                    host('command', command='tick unfreeze')
                photographed = True
            if shots['host'] >= 2 and stopped is None:
                host('command', command=f'data merge entity {selector} {{NoAI:1b}}')
                stopped = waited
            if stopped is not None and waited - stopped >= 20:
                break
            # Keep the survival target in range after a hit applies levitation.
            if waited % 20 == 0:
                guest('command', command='effect clear @s minecraft:levitation')
                guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        lab.write_json(out / 'shulker-bullets.json', {'shots': shots, 'paths': paths})
        assert shots['host'] >= 2, f'the shulker did not attack twice: {shots}'
        for name, by_id in paths.items():
            assert len(by_id) == shots[name], f'{name}: {shots[name]} shots but bullet ids {list(by_id)}'
            for id, positions in by_id.items():
                assert any(math.dist(positions[0], p) > 0.5 for p in positions), \
                    f'{name}: bullet {id} did not move: {positions}'
        assert paths['host'].keys() == paths['guest'].keys(), f'different bullets: {paths}'
        for id, positions in paths['guest'].items():
            # The clients interpolate three ticks behind, and reads are not simultaneous.
            assert all(min(math.dist(p, q) for q in paths['host'][id]) < 1.5 for p in positions), \
                f'bullet {id} diverged from the host: {paths}'

        # Put blocks in the flight volume to exercise server removal without depending on homing.
        for game in (host, guest):
            game('command', command='gamemode creative')
            game('command', command=f'tp @s {x + 0.5} {y + 16} {z + 0.5}')
        host('command', command=f'fill {x - 10} {y} {z - 10} {x + 10} {y + 12} {z + 18} minecraft:stone')
        for _ in range(20):
            guest.wait(2)
            remaining = {g.name: bullets(g('status')) for g in (host, guest)}
            if not any(remaining.values()):
                break
        assert not any(remaining.values()), f'bullets survived block hits: {remaining}'
    finally:
        host('command', command='tick unfreeze')
        host('command', command=f'kill @e[tag={tag}]')
        host('command', command=f'fill {x - 10} {y} {z - 10} {x + 10} {y + 12} {z + 18} minecraft:air')
        for game in (host, guest):
            game('command', command='effect clear @s')
            game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            game('command', command='gamemode survival')
        face_each_other(host, guest)
    return {'shots': shots, 'ids': sorted(paths['host']), 'removed': True}


CHECKS += ['shulker_bullets']
CHECK_FNS.update(shulker_bullets=check_shulker_bullets)


def check_farmland(host, guest, out):
    """FarmlandBlock.fallOn's base-block change breaks the unsupported crop (#1492):
    the host and guest each land on mature wheat, get its drops, and both clients see dirt/air."""
    hx, hy, hz = (int(v // 1) for v in pos_of(host))
    y = max(hy + 12, 100)
    starts = {g.name: pos_of(g) for g in (host, guest)}
    seen = {}
    host('command', command='gamerule random_tick_speed 0')
    try:
        for game, other, dz in ((host, guest, -3), (guest, host, 3)):
            x, z = hx + 2, hz + dz
            soil, crop, marker = (x, y, z), (x, y + 1, z), (x + 4, y + 1, z)
            for g in (game, other):
                g('command', command='gamemode creative')
            host('command', command=f'fill {x - 4} {y} {z - 2} {x + 4} {y} {z + 2} minecraft:stone')
            host('command', command=f'fill {x - 4} {y + 1} {z - 2} {x + 4} {y + 7} {z + 2} minecraft:air')
            host('command', command=f'setblock {x} {y} {z} minecraft:farmland[moisture=7]')
            host('command', command=f'setblock {x} {y + 1} {z} minecraft:wheat[age=7]')
            other('command', command=f'tp @s {x + 3.5} {y + 1} {z + 0.5}')
            game('command', command=f'tp @s {x + 2.5} {y + 1} {z + 0.5}')
            game('command', command='clear @s')
            game('command', command='gamemode survival')
            game.wait(20)
            before = {g.name: [g.raw('block', *p).get('block') for p in (soil, crop)]
                      for g in (host, guest)}
            assert all(v == ['minecraft:farmland', 'minecraft:wheat'] for v in before.values()), \
                f'{game.name}: planted wheat missing before landing: {before}'
            other.raw('look-at', x + 0.5, y + 1, z + 0.5)
            other.shot(out, f'farmland-{game.name}-before')
            # More than 1.5 blocks of fall distance guarantees the trampling random check.
            game('command', command=f'tp @s {x + 0.5} {y + 6} {z + 0.5}')
            game.wait(80)
            after = {g.name: [g.raw('block', *p).get('block') for p in (soil, crop)]
                     for g in (host, guest)}
            # Test the authoritative blocks too, rather than only each client's view.
            game('command', command=f'execute if block {x} {y} {z} minecraft:dirt '
                                    f'if block {x} {y + 1} {z} minecraft:air run '
                                    f'setblock {marker[0]} {marker[1]} {marker[2]} minecraft:gold_block')
            game.wait(10)
            counts = {item: server_count(game, item) for item in ('wheat', 'wheat_seeds')}
            client_counts = game('inventory')['counts']
            seen[game.name] = {'blocks': after, 'server': game.raw('block', *marker).get('block'),
                               'drops': counts,
                               'client_drops': {item: client_counts.get('minecraft:' + item, 0)
                                                for item in counts}}
            other.shot(out, f'farmland-{game.name}-after')
    finally:
        host('command', command='gamerule random_tick_speed 3')
        host('command', command='kill @e[type=minecraft:item]')
        for g in (host, guest):
            x0, y0, z0 = starts[g.name]
            g('command', command=f'tp @s {x0} {y0} {z0}')
            g('command', command='gamemode survival')
    for name, s in seen.items():
        assert all(v == ['minecraft:dirt', 'minecraft:air'] for v in s['blocks'].values()), \
            f'{name}: unsupported wheat after landing: {seen}'
        assert s['server'] == 'minecraft:gold_block', f'{name}: server kept the unsupported wheat: {seen}'
        assert s['drops']['wheat'] == 1 and 1 <= s['drops']['wheat_seeds'] <= 4, \
            f'{name}: mature wheat drops missing: {seen}'
        assert s['client_drops'] == s['drops'], f'{name}: crop drops did not synchronize: {seen}'
    return seen


CHECKS += ['farmland']
CHECK_FNS.update(farmland=check_farmland)


def check_large_chest(host, guest, out):
    """Every player slot can be picked up or shift-clicked into a double chest (#1505, #1589).
    Reopening checks the authoritative contents, with a single chest as the control."""
    seen = {}
    for game in (host, guest):
        game('command', command='gamemode survival')
        x, y, z = (int(v // 1) for v in pos_of(game))
        chest, other = (x, y, z + 2), (x + 1, y, z + 2)
        try:
            for double in (False, True):
                size = 54 if double else 27
                # Double chest indices 54..89 include the old cutoff at 63/64 and every hotbar slot.
                sources = list(range(9, 36)) + list(range(9)) if double else [19, 0, 8]
                for shift in (False, True):
                    back_to_game(game)
                    for at in (chest, other):
                        game('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
                    game('command', command='kill @e[type=minecraft:item,distance=..8]')
                    game('command', command='clear @s')
                    block = 'minecraft:chest[facing=north,type=left]' if double else 'minecraft:chest'
                    game('command', command=f'setblock {x} {y} {z + 2} {block}')
                    if double:
                        game('command', command=f'setblock {x + 1} {y} {z + 2} minecraft:chest[facing=north,type=right]')
                    for slot in sources:
                        name = f'hotbar.{slot}' if slot < 9 else f'inventory.{slot - 9}'
                        game('command', command=f'item replace entity @s {name} with minecraft:diamond')
                    game.wait(10)
                    game.raw('interact', *chest)
                    game.raw('wait-for', 'screen', 'container', '--timeout', 10)
                    game.wait(10)
                    assert container_items(game)[1] == size, f'{game.name}: expected {size} chest slots'
                    label = f'{"double" if double else "single"}-chest-{"shift" if shift else "pickup"}'
                    for target, slot in enumerate(sources):
                        if shift:
                            game.raw('key', 'shift', 'down')
                        try:
                            mouse(game, slot)
                        finally:
                            if shift:
                                game.raw('key', 'shift', 'up')
                        if not shift:
                            held = game('inventory').get('carried') or {}
                            if held.get('item') != 'minecraft:diamond' or held.get('count') != 1:
                                game.shot(out, f'{label}-slot-{slot}-failed')
                                raise AssertionError(f'{game.name} {label}: slot {slot} picked up {held}')
                            mouse(game, 300 + target)
                    game.raw('key', 'esc', 'tap')
                    game.wait(10)
                    game.raw('interact', *chest)
                    game.raw('wait-for', 'screen', 'container', '--timeout', 10)
                    game.wait(10)
                    contents = [s for s in game('screen')['slots'] if 300 <= s['slot'] < 300 + size and s.get('item')]
                    inv = game('inventory')
                    stored = sum(s['count'] for s in contents if s['item'] == 'minecraft:diamond')
                    assert stored == len(sources) and not inv['counts'].get('minecraft:diamond') and not inv.get('carried'), \
                        f'{game.name} {label}: chest {contents}, inventory {inv}'
                    game.shot(out, label)
                    seen[f'{game.name}-{label}'] = stored
                    game.raw('key', 'esc', 'tap')
        finally:
            back_to_game(game)
            for at in (chest, other):
                game('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
            game('command', command='kill @e[type=minecraft:item,distance=..8]')
            game('command', command='clear @s')
    return seen


CHECKS += ['large_chest']
CHECK_FNS.update(large_chest=check_large_chest)


def check_falling(host, guest, out):
    """ItemEntity, FallingBlockEntity and PrimedTnt tick on a client too (gravity, collisions, drag),
    and their position packets snap them (Entity's NO_OP InterpolationHandler). The server sends an
    item's and a falling block's position every 20 ticks and TNT's every 10, so between two updates
    the guest's own tick is what moves them: sampled every 2 ticks over the first 16, all three are
    lower nearly every time. The TNT's fuse (sent once, at spawn) counts down on the guest too."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    top = y + 30
    tag = 'falling_e2e'
    host('command', command=f'fill {x + 2} {y} {z - 1} {x + 4} {top + 2} {z + 1} minecraft:air')
    host('command', command=f'summon minecraft:item {x + 3.5} {top} {z + 0.5} '
                            f'{{Tags:["{tag}"],Item:{{id:"minecraft:diamond",count:1}},PickupDelay:32767}}')
    host('command', command=f'summon minecraft:falling_block {x + 2.5} {top} {z - 0.5} '
                            f'{{Tags:["{tag}"],BlockState:{{Name:"minecraft:sand"}},DropItem:0b}}')
    # Horizon's selectors don't see primed TNT yet: its fuse ends it in the air, 19 blocks down.
    host('command', command=f'summon minecraft:tnt {x + 4.5} {top} {z + 0.5} {{fuse:34s}}')
    heights = {'item': [], 'falling block': [], 'primed tnt': []}
    fuses = []
    try:
        for _ in range(8):
            items = [e['pos'][1] for e in guest.raw('entities', 48, 'minecraft:item')['entities']
                     if abs(e['pos'][0] - x - 3.5) < 0.5 and abs(e['pos'][2] - z - 0.5) < 0.5]
            blocks = [o['position'][1] for o in guest('status')['projectiles']
                      if o.get('kind') == 'minecraft:falling_block'
                      and abs(o['position'][0] - x - 2.5) < 0.5 and abs(o['position'][2] - z + 0.5) < 0.5]
            tnts = [o for o in guest('status')['projectiles'] if o.get('kind') == 'minecraft:tnt'
                    and abs(o['position'][0] - x - 4.5) < 0.5 and abs(o['position'][2] - z - 0.5) < 0.5]
            heights['item'].append(items[0] if items else None)
            heights['falling block'].append(blocks[0] if blocks else None)
            heights['primed tnt'].append(tnts[0]['position'][1] if tnts else None)
            fuses.append(tnts[0]['fuse'] if tnts else None)
            guest.wait(2)
        guest.shot(out, 'falling')
    finally:
        host('command', command=f'kill @e[tag={tag}]')
        host('command', command=f'fill {x + 2} {y} {z - 1} {x + 4} {top + 2} {z + 1} minecraft:air')
    for name, ys in heights.items():
        seen = [h for h in ys if h is not None]
        assert len(seen) >= 6, f'the guest saw the {name} {len(seen)} times of 8: {heights}'
        drops = sum(b < a for a, b in zip(seen, seen[1:]))
        # A tick or two can pass between two samples with no move; never most of them.
        assert drops >= len(seen) - 2, f'the {name} fell in {drops} of {len(seen) - 1} samples: {heights}'
    # The server sends the TNT's fuse with each move, so each sample says how many ticks it has
    # fallen from rest: snapped to the server's place and ticking on, the guest has it exactly
    # there (it used to trail two ticks behind, interpolating towards each update).
    fell = [(h, 34 - f) for h, f in zip(heights['primed tnt'], fuses) if f is not None]
    assert fell and fell[-1][1] > fell[0][1], f'the TNT\'s fuse on the guest: {fuses}'
    off = max(abs(h - tnt_fall(top, n)) for h, n in fell)
    assert off < 0.01, f'the guest\'s TNT is {off:.3f} off its fall (height, ticks): {fell}'
    return {**heights, 'fuse': fuses}


def tnt_fall(top, ticks):
    """PrimedTnt.tick in open air from rest: its height after `ticks` ticks."""
    y, v = top, 0.0
    for _ in range(ticks):
        v -= 0.04
        y += v
        v *= 0.98
    return y


CHECKS += ['falling']
CHECK_FNS.update(falling=check_falling)


def check_crystals(host, guest, out):
    """#1532: a real left click detonates a placed End crystal in survival and creative,
    on the host and the guest; reach, block occlusion and passing right clicks still apply."""
    spots = apart(host, guest)
    seen = {}

    def crystals(game, at):
        return [p for p in game('status')['projectiles'] if p['kind'] == 'minecraft:end_crystal'
                and math.dist(p['position'], at) < 0.01]

    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        y += 32  # Above the terrain and its entities: EndCrystalItem requires an empty box.
        at = [x + 2.5, y, z + 0.5]
        other('command', command='gamemode creative')
        for mode in ('creative', 'survival'):
            case = f'{label}-{mode}'
            try:
                host('command', command=f'fill {x - 5} {y - 1} {z - 3} {x + 5} {y - 1} {z + 3} minecraft:obsidian')
                host('command', command=f'fill {x - 5} {y} {z - 3} {x + 5} {y + 3} {z + 3} minecraft:air')
                player('command', command=f'gamemode {mode}')
                player('command', command='clear @s')
                player('command', command='effect clear @s')
                player('command', command='effect give @s minecraft:instant_health 1 5 true')
                player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
                player('command', command='give @s minecraft:end_crystal 1')
                player.raw('select', 1)
                player.wait(10)
                player.raw('look-at', at[0], y - 0.01, at[2])
                player.raw('mouse', 'right', 'click')
                player.wait(10)
                placed = crystals(player, at)
                assert len(placed) == 1, f'{case}: placing the crystal: {placed}'
                crystal_id = placed[0]['id']
                tracked = crystals(other, at)
                assert crystal_id != 0 and [p['id'] for p in tracked] == [crystal_id], f'{case}: crystal not tracked on both sides'

                # The nearest face is six blocks away, beyond both modes' attack reach.
                player('command', command=f'tp @s {x - 4.5} {y} {z + 0.5}')
                player.wait(5)
                player.raw('look-at', at[0], y + 1, at[2])
                player.raw('mouse', 'left', 'click')
                player.wait(10)
                assert crystals(player, at), f'{case}: attacked a crystal out of reach'

                player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
                host('command', command=f'fill {x + 1} {y} {z} {x + 1} {y + 2} {z} minecraft:obsidian')
                player.wait(5)
                player.raw('look-at', at[0], y + 1, at[2])
                player.raw('mouse', 'left', 'click')
                player.wait(10)
                assert crystals(player, at), f'{case}: attacked a crystal through a block'
                host('command', command=f'fill {x + 1} {y} {z} {x + 1} {y + 2} {z} minecraft:air')
                player('command', command='item replace entity @s weapon.mainhand with minecraft:shield')
                player.wait(10)
                player.raw('look-at', at[0], y + 1, at[2])
                player.raw('mouse', 'right', 'down')
                player.wait(10)
                assert player('state').get('using') and server_using(host, player), f'{case}: crystal consumed right click'
                player.raw('mouse', 'right', 'up')
                player('command', command='clear @s')
                player.wait(5)
                player.shot(out, f'crystals-{mode}-before')

                player.raw('mouse', 'left', 'click')
                player.wait(40)
                remaining = {g.name: len(crystals(g, at)) for g in (player, other)}
                health = server_player(host, player)['health']
                screen = player('state').get('screen')
                seen[case] = {'id': crystal_id, 'remaining': remaining, 'health': health, 'screen': screen}
                player.shot(out, f'crystals-{mode}-after')
                assert not any(remaining.values()), f'{case}: left click left the crystal intact: {seen[case]}'
                assert health == (20 if mode == 'creative' else 0), f'{case}: explosion damage: {seen[case]}'
                assert screen == ('playing' if mode == 'creative' else 'dead'), f'{case}: explosion death: {seen[case]}'
            finally:
                if player('state').get('screen') == 'dead':
                    respawn_when_ready(player)
                    player.wait(30)
                else:
                    player.raw('mouse', 'right', 'up')
                player('command', command='gamemode creative')
    return seen


CHECKS += ['crystals']
CHECK_FNS.update(crystals=check_crystals)


def check_crystal_blast(host, guest, out):
    """#1533: TNT removes a crystal without a blast; a punched crystal blasts once.
    Both clients must see the surviving distant and invulnerable crystals."""
    spots = apart(host, guest)
    seen = {}
    for label, player, other in (('host', host, guest), ('guest', guest, host)):
        x, y, z = spots[label]
        y += 32
        player('command', command='gamemode creative')
        for offset, cause in enumerate(('tnt', 'melee')):
            # Leave survivors in separate arenas; the scratch world is discarded.
            z = spots[label][2] + (0 if label == 'host' else 80) + offset * 40
            case = f'{label}-{cause}'
            host('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 25} {y - 1} {z + 5} minecraft:bedrock')
            host('command', command=f'fill {x - 3} {y} {z - 3} {x + 25} {y + 4} {z + 5} minecraft:air')
            positions = [[x + dx, y, z + dz] for dx, dz in ((2.5, 0.5), (12.5, 0.5), (22.5, 0.5), (4.5, 3.5))]

            def remaining(game):
                crystals = [p for p in game('status')['projectiles'] if p['kind'] == 'minecraft:end_crystal']
                return [i for i, at in enumerate(positions)
                        if any(math.dist(p['position'], at) < 0.01 for p in crystals)]

            for i, at in enumerate(positions):
                host('command', command=f'summon minecraft:end_crystal {at[0]} {at[1]} {at[2]} {{Invulnerable:{1 if i == 3 else 0}b}}')
            player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
            other('command', command='gamemode creative')
            other('command', command=f'tp @s {x + 0.5} {y} {z + 4.5}')
            player('command', command='clear @s')
            player.wait(20)
            for game in (host, guest):
                assert remaining(game) == [0, 1, 2, 3], f'{case}: crystals not tracked on {game.name}'
            if cause == 'tnt':
                player('command', command=f'summon minecraft:tnt {x + 0.5} {y} {z + 0.5} {{fuse:10s}}')
            else:
                player.raw('look-at', positions[0][0], y + 1, positions[0][2])
                player.raw('mouse', 'left', 'click')
            player.wait(80)
            result = {game.name: remaining(game) for game in (host, guest)}
            seen[case] = result
            # TNT reaches the first crystal only. A direct crystal blast reaches
            # the second too, but that crystal must not detonate the third.
            expected = [1, 2, 3] if cause == 'tnt' else [2, 3]
            assert all(ids == expected for ids in result.values()), f'{case}: expected {expected}, got {result}'
    return seen


CHECKS += ['crystal_blast']
CHECK_FNS.update(crystal_blast=check_crystal_blast)


def check_boat_dismount_pose(host, guest, out):
    """Removing a boat detaches its mobs on the server and every observing client."""
    x, y, z = map(math.floor, pos_of(host))
    bx, bz = x + 3.5, z + 3.5
    tag = 'boat_dismount_pose'
    host('command', command='gamemode creative')
    host('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 8} {y - 1} {z + 8} minecraft:stone')
    host('command', command=f'fill {x - 2} {y} {z - 2} {x + 8} {y + 4} {z + 8} minecraft:air')
    host('command', command=f'summon minecraft:oak_boat {bx} {y} {bz} {{Tags:["{tag}"]}}')
    for _ in range(2):
        host('command', command=f'summon minecraft:husk {bx} {y} {bz} {{NoAI:1b,Tags:["{tag}"]}}')
        host.wait(5)
    try:
        host.wait(20)
        boat = next(o for o in host('status')['objects']
                    if o['kind'].endswith('oak_boat') and math.dist(o['position'], [bx, y, bz]) < 2)
        riders = [m['id'] for m in host.raw('mobs', 64)['near']
                  if m['kind'] == 'minecraft:husk' and m['mount'] == boat['id']]
        assert len(riders) == 2, f'the boat did not collect both husks: {riders}'
        for game in (host, guest):
            game.wait(10)
            for rider in riders:
                m = mob_seen(game, rider)
                assert m and m['mount'] == boat['id'] and m['passenger'], f'{game.name} did not see seated husk {rider}: {m}'
            game.raw('look-at', bx, y + 1, bz)
            game.shot(out, 'boat-passengers-seated')
        host('command', command=f'tp @s {bx - 3} {y} {bz} 0 0')
        host.wait(5)
        # Aim at the near rim, below the riders, so their boxes cannot
        # intercept the creative player's one hit that destroys the boat.
        host.raw('look-at', bx - 0.55, y + 0.1, bz)
        host.raw('mouse', 'left', 'click')
        seen = {}
        for game in (host, guest):
            game.wait(20)
            assert all(o['id'] != boat['id'] for o in game('status')['objects']), f'{game.name} still sees the boat'
            seen[game.name] = [mob_seen(game, rider) for rider in riders]
            game.raw('look-at', bx, y + 1, bz)
            game.shot(out, 'boat-passengers-dismounted')
            for m in seen[game.name]:
                assert m and m['mount'] is None and not m['passenger'], f'{game.name} still draws a removed boat\'s passenger: {m}'
        return {name: [{k: m[k] for k in ('id', 'mount', 'passenger')} for m in mobs] for name, mobs in seen.items()}
    finally:
        host('command', command=f'kill @e[tag={tag}]')


CHECKS += ['boat-dismount-pose']
CHECK_FNS['boat-dismount-pose'] = check_boat_dismount_pose


def check_sprint_fov(host, guest, out):
    """Releasing Ctrl must preserve a sprint until W is released, then reset FOV.

    PlayerInput reports keys; PlayerCommand starts/stops the sprint. Confusing
    them clears the client's sprint flag while leaving its speed modifier on.
    """
    seen = {}
    for game in (host, guest):
        game('command', command='gamemode creative')
        game('command', command='effect clear @s')
        game.raw('options-set', 'fovEffectScale', '1.0')
        x, y, z = map(math.floor, pos_of(game))
        game('command', command=f'fill {x - 2} {y} {z - 2} {x + 2} {y + 3} {z + 20} minecraft:air')
        game('command', command=f'fill {x - 2} {y - 1} {z - 2} {x + 2} {y - 1} {z + 20} minecraft:stone')
        game('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
        game.wait(20)
        try:
            game.raw('key', 'w', 'down')
            game.raw('key', 'ctrl', 'down')
            game.wait(10)
            started = game('state')['player']
            game.raw('key', 'ctrl', 'up')
            game.wait(10)
            released_ctrl = game('state')['player']
            game.raw('key', 'w', 'up')
            game.wait(20)
            stopped = game('state')['player']
        finally:
            game.raw('key', 'ctrl', 'up')
            game.raw('key', 'w', 'up')
        seen[game.name] = states = {
            label: {k: p[k] for k in ('sprinting', 'fov_modifier')}
            for label, p in [('started', started), ('released_ctrl', released_ctrl), ('stopped', stopped)]
        }
        for label in ('started', 'released_ctrl'):
            assert states[label]['sprinting'], f'{game.name} {label}: {states}'
            assert abs(states[label]['fov_modifier'] - 1.15) < 0.002, f'{game.name} {label}: {states}'
        assert not stopped['sprinting'], f'{game.name}: {states}'
        assert abs(stopped['fov_modifier'] - 1.0) < 0.002, f'{game.name}: {states}'
    return seen


CHECKS += ['sprint-fov']
CHECK_FNS['sprint-fov'] = check_sprint_fov


def check_critical_hits(host, guest, out):
    """#1898: a real falling melee hit emits crits on the attacker and observer.
    Grounded hits and refused falling hits emit none, against mobs and players.
    """
    x, y, z = map(math.floor, pos_of(host))
    target = [x + 2.5, y, z + 0.5]
    tag = 'critical_hit_particles'
    seen = {}
    host('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 6} {y - 1} {z + 6} minecraft:stone')
    host('command', command=f'fill {x - 3} {y} {z - 3} {x + 6} {y + 5} {z + 6} minecraft:air')
    host('command', command='time set midnight')
    host('command', command='gamerule natural_health_regeneration false')
    try:
        for label, player, other in (('host', host, guest), ('guest', guest, host)):
            for kind in ('zombie', 'player'):
                for hit in ('grounded', 'critical', 'refused'):
                    case = f'{label}-{kind}-{hit}'
                    for game in (player, other):
                        game('command', command='gamemode survival')
                        game('command', command='clear @s')
                        game('command', command='effect clear @s')
                        game('command', command='effect give @s minecraft:instant_health 1 5 true')
                    player('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
                    other_at = target if kind == 'player' else [x + 0.5, y, z + 4.5]
                    other('command', command=f'tp @s {other_at[0]} {other_at[1]} {other_at[2]}')
                    host('command', command=f'gamerule pvp {str(hit != "refused").lower()}')
                    mob_id = None
                    if kind == 'zombie':
                        host('command', command=f'summon minecraft:zombie {target[0]} {y} {target[2]} '
                             f'{{NoAI:1b,PersistenceRequired:1b,Invulnerable:{int(hit == "refused")}b,Tags:["{tag}"]}}')
                    player.wait(20)  # full attack charge and paired entities
                    if kind == 'zombie':
                        mob = next(m for m in player.raw('mobs', 16)['near']
                                   if m['kind'] == 'minecraft:zombie' and math.dist(m['pos'], target) < 0.1)
                        mob_id = mob['id']
                        assert mob_seen(other, mob_id), f'{case}: observer has not paired the zombie'
                    health_before = mob_seen(player, mob_id)['health'] if mob_id is not None else hearts(other)[1]
                    sound = 'minecraft:entity.player.attack.crit'
                    sounds_before = {side: heard(g, sound) for side, g in (('own', player), ('other', other))}

                    def strike():
                        if hit != 'grounded':
                            player.raw('key', 'space', 'down')
                            player.wait(1)
                            player.raw('key', 'space', 'up')
                            body = None
                            for _ in range(15):
                                body = player('state')['player']
                                # Let the server record the fall before the attack packet.
                                if not body['on_ground'] and body['velocity'][1] < -0.2:
                                    break
                                player.wait(1)
                            assert body and not body['on_ground'] and body['velocity'][1] < -0.2, \
                                f'{case}: jump never reached its falling phase: {body}'
                        player.raw('look-at', target[0], y + 1, target[2])
                        player.raw('mouse', 'left', 'click')

                    particles = sparked(player, other, ['crit'], strike)
                    player.shot(out, f'critical-hits-{kind}-{hit}-{label}')
                    player.wait(10)  # finish the three-tick emitter and health replication
                    health_after = mob_seen(player, mob_id)['health'] if mob_id is not None else hearts(other)[1]
                    sounds = {side: heard(g, sound) - sounds_before[side]
                              for side, g in (('own', player), ('other', other))}
                    seen[case] = {'particles': particles, 'crit_sounds': sounds,
                                  'health': [health_before, health_after]}
                    host('command', command=f'kill @e[tag={tag}]')
                    assert (health_after == health_before) if hit == 'refused' else (health_after < health_before), \
                        f'{case}: unexpected damage: {seen[case]}'
                    assert all(n == int(hit == 'critical') for n in sounds.values()), \
                        f'{case}: the server did not recognize the expected crit: {seen[case]}'
        wrong = {case: result for case, result in seen.items()
                 if any((n <= 0 if case.endswith('-critical') else n != 0)
                        for n in result['particles'].values())}
        assert not wrong, f'critical-hit particles: {wrong}'
        return seen
    finally:
        host('command', command=f'kill @e[tag={tag}]')
        host('command', command='gamerule pvp true')
        host('command', command='gamerule natural_health_regeneration true')
        host('command', command='time set day')
        for game in (host, guest):
            game.raw('key', 'space', 'up')


CHECKS += ['critical-hits']
CHECK_FNS['critical-hits'] = check_critical_hits


def check_sculk_container(host, guest, out):
    """A real guest's container interaction emits a server game event. Both
    clients see the sensor and the non-summoning shrieker activate and reset.
    The component oracle separately controls warning levels and seeded mobs.
    """
    x, y, z = (int(v // 1) for v in pos_of(guest))
    chest, sensor, shrieker = (x + 2, y, z), (x + 3, y, z), (x + 4, y, z)
    guest('command', command='gamemode survival')
    for at, block in ((chest, 'chest'), (shrieker, 'sculk_shrieker'), (sensor, 'sculk_sensor')):
        guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:{block}')
    guest.wait(60)
    seen = {}
    # Keep the sensor's 30-tick active window observable while both clients
    # acknowledge the container and the control socket inspects them.
    host('command', command='tick rate 4')
    try:
        guest.raw('interact', *chest)
        guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
        for game in (host, guest):
            active = wait_state(game, sensor, 'sculk_sensor_phase', 'active', 40)
            shrieking = wait_state(game, shrieker, 'shrieking', 'true', 40)
            seen[game.name] = (active, shrieking)
        assert all(s == ('active', 'true') for s in seen.values()), seen
        guest.raw('key', 'esc', 'tap')
        for game in (host, guest):
            assert wait_state(game, sensor, 'sculk_sensor_phase', 'inactive', 160) == 'inactive', seen
            assert wait_state(game, shrieker, 'shrieking', 'false', 160) == 'false', seen
    finally:
        host('command', command='tick rate 20')
        if guest('state').get('screen') == 'container':
            guest.raw('key', 'esc', 'tap')
        for at in (sensor, shrieker, chest):
            guest('command', command=f'setblock {at[0]} {at[1]} {at[2]} minecraft:air')
    return seen


CHECKS += ['sculk_container']
CHECK_FNS['sculk_container'] = check_sculk_container

def check_replant(host, guest, out):
    """Instant breaks leave no server dig that can destroy a replacement (#1880).

    ServerPlayerGameMode.handleBlockBreakAction's insta-mine branch finishes on
    START; MultiPlayerGameMode sends no STOP or ABORT when attack is released.
    Exercise actual flower placement and seed planting after a short click.
    """
    spots = apart(host, guest)
    seen = {}
    host('command', command='gamerule random_tick_speed 0')
    try:
        for label, player in (('host', host), ('guest', guest)):
            x, y, z = spots[label]
            cell = (x, y, z + 2)

            def observe(block):
                # The integrated host is itself a client: wait for both views.
                for _ in range(40):
                    blocks = [g.raw('block', *cell).get('block') for g in (host, guest)]
                    if blocks == [f'minecraft:{block}'] * 2:
                        return blocks
                    host.wait(2)
                raise AssertionError(f'{label} expected {block} on both clients: {blocks}')

            for mode, block, item, soil in (
                ('survival', 'dandelion', 'dandelion', 'dirt'),
                ('survival', 'wheat', 'wheat_seeds', 'farmland[moisture=7]'),
                ('survival', 'carrots', 'carrot', 'farmland[moisture=7]'),
                ('creative', 'dandelion', 'dandelion', 'dirt'),
            ):
                player('command', command=f'gamemode {mode}')
                player('command', command='clear @s')
                player('command', command=f'item replace entity @s weapon.mainhand with minecraft:{item} 8')
                player('command', command=f'setblock {x} {y - 1} {z + 2} minecraft:{soil}')
                player('command', command=f'setblock {x} {y} {z + 2} minecraft:{block}')
                player.wait(5)
                observe(block)
                # Flowers' narrow outlines have a position-dependent offset.
                for dx in (0.0, -0.2, 0.2):
                    player.raw('look-at', x + 0.5 + dx, y + (0.3 if block == 'dandelion' else 0.1), z + 2.5)
                    target = player('looking_at').get('block') or {}
                    if target.get('pos') == list(cell):
                        break
                assert target.get('pos') == list(cell), f'{label} does not aim at {block}: {target}'
                player.raw('mouse', 'left', 'click')
                player.wait(5)
                observe('air')
                # Keep aiming at the supporting soil and use the held flower/seeds.
                player.raw('mouse', 'right', 'click')
                player.wait(3)
                observe(block)
                player.wait(20)  # past the stale dig's five-tick destroy delay
                key = f'{label} {mode} {block}'
                seen[key] = [g.raw('block', *cell).get('block') for g in (host, guest)]
                player.shot(out, f'replant-{mode}-{block}')
                assert seen[key] == [f'minecraft:{block}'] * 2, f'{key}: {seen}'
                player('command', command=f'setblock {x} {y} {z + 2} minecraft:air')
    finally:
        host('command', command='gamerule random_tick_speed 3')
    return seen


CHECKS += ['replant']
CHECK_FNS['replant'] = check_replant


def check_guardian_beams(host, guest, out):
    """Guardian's synced target drives the same beam on each client (#1491).

    Guardian.defineSynchedData/onSyncedDataUpdated/aiStep and
    GuardianRenderer.extractRenderState: charge, hit, then target loss.
    Run with both --server-thread and the in-frame host.
    """
    x, y, z = map(math.floor, pos_of(host))
    host('command', command='difficulty normal')
    host('command', command='gamerule natural_health_regeneration false')
    host('command', command=f'fill {x - 10} {y - 2} {z - 10} {x + 10} {y + 4} {z + 10} minecraft:water')
    seen = {}
    try:
        for kind in ('guardian', 'elder_guardian'):
            for target, observer in ((host, guest), (guest, host)):
                for game in (host, guest):
                    game('command', command='gamemode creative')
                    game('command', command='effect give @s minecraft:water_breathing infinite')
                target('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
                observer('command', command=f'tp @s {x + 4.5} {y} {z + 0.5} 0 0')
                target('command', command='effect give @s minecraft:instant_health 1 10')
                target('command', command='gamemode survival')
                host('command', command=f'summon minecraft:{kind} {x + 0.5} {y} {z + 6.5}')

                def mob(game):
                    return next(m for m in game.raw('mobs', 32)['near'] if m['kind'] == f'minecraft:{kind}')

                def charging():
                    for _ in range(60):
                        target.wait(2)
                        states = [mob(g) for g in (target, observer)]
                        if all(m['guardian_beam'] is not None for m in states):
                            return states
                    raise Failed(f'{kind} never charged on both clients')

                states = charging()
                assert states[0]['target'] == 'Player', states
                # The host reads the level that ticks the guardian (its own in-frame, the
                # server's on the server thread), where a targeted player is `Player`.
                if observer is host:
                    assert states[1]['target'] == 'Player', states
                else:
                    assert states[1]['target'].startswith('Mob('), states
                first = states[0]['guardian_beam'][0]
                target.wait(4)
                assert mob(target)['guardian_beam'][0] > first, 'client charge clock did not advance'
                for game in (target, observer):
                    game.raw('look-at', x + 0.5, y + 0.5, z + 6.5)
                    game.shot(out, f'{kind}-{target.name}-charge')
                # The beam must end after the first hit (before another charge).
                before = target('status')['player']['health']
                for _ in range(70):
                    target.wait(2)
                    if all(mob(g)['guardian_beam'] is None for g in (target, observer)):
                        break
                else:
                    raise Failed(f'{kind} beam did not end after its attack')
                after = target('status')['player']['health']
                assert after < before, f'{kind} beam ended without damage: {before} -> {after}'
                charging()
                target('command', command='gamemode creative')
                target.wait(10)
                assert all(mob(g)['guardian_beam'] is None for g in (target, observer)), 'beam survived target loss'
                seen[f'{kind}-{target.name}'] = {
                    'charge': [{'target': m['target'], 'beam': m['guardian_beam']} for m in states],
                    'health': [before, after]}
                host('command', command=f'kill @e[type=minecraft:{kind}]')
                host.wait(10)
    finally:
        for kind in ('guardian', 'elder_guardian'):
            host('command', command=f'kill @e[type=minecraft:{kind}]')
        for game in (host, guest):
            game('command', command='gamemode creative')
            game('command', command='effect clear @s')
    return seen


CHECKS += ['guardian-beams']
CHECK_FNS['guardian-beams'] = check_guardian_beams


def check_fangs(host, guest, out):
    """EvokerFangs: authoritative spawn, event 4, client animation and server removal (#1482)."""
    x, y, z = (int(v // 1) for v in pos_of(host))
    tag = 'lan_evoker_fangs'
    selector = f'@e[type=minecraft:evoker,tag={tag},limit=1]'
    paths = {g.name: {} for g in (host, guest)}
    warmup = dict.fromkeys(paths, False)
    drawn = dict.fromkeys(paths, False)
    stopped = False
    photographed = False

    def fangs(status):
        return [p for p in status['projectiles'] if p['kind'] == 'minecraft:evoker_fangs']

    try:
        host('command', command='difficulty normal')
        host('command', command='gamemode creative')
        guest('command', command='gamemode survival')
        guest('command', command='effect give @s minecraft:resistance 120 4 true')
        host('command', command=f'fill {x - 10} {y - 1} {z - 10} {x + 10} {y - 1} {z + 18} minecraft:stone')
        host('command', command=f'fill {x - 10} {y} {z - 10} {x + 10} {y + 8} {z + 18} minecraft:air')
        host('command', command=f'tp @s {x + 4.5} {y} {z - 2.5}')
        guest('command', command=f'tp @s {x + 0.5} {y} {z + 0.5}')
        guest.wait(10)
        host('command', command=f'summon minecraft:evoker {x + 0.5} {y} {z + 9.5} '
                                f'{{PersistenceRequired:1b,Tags:["{tag}"]}}')
        for game in (host, guest):
            game.raw('look-at', x + 0.5, y + 0.5, z + 5.5)
        for _ in range(200):
            guest.wait(2)
            statuses = {g.name: g('status') for g in (host, guest)}
            for name, status in statuses.items():
                live = fangs(status)
                ids = [f['id'] for f in live]
                assert all(ids) and len(ids) == len(set(ids)), f'{name}: invalid fang ids {ids}'
                warmup[name] |= any(not f['started'] for f in live)
                drawn[name] |= status['fangs_vertices'] > 0
                for f in live:
                    paths[name].setdefault(f['id'], []).append(f)
            if paths['host'] and not stopped:
                host('command', command=f'data merge entity {selector} {{NoAI:1b}}')
                stopped = True
            if (all(drawn.values()) and not photographed
                    and any(f['started'] and 8 <= f['life'] <= 16
                            for f in fangs(statuses['guest']))):
                host('command', command='tick freeze')
                try:
                    for game in (host, guest):
                        game.raw('key', 'f1', 'tap')
                        try:
                            game.shot(out, 'evoker-fangs')
                        finally:
                            game.raw('key', 'f1', 'tap')
                finally:
                    host('command', command='tick unfreeze')
                photographed = True
            if stopped and not any(fangs(s) for s in statuses.values()):
                break
        lab.write_json(out / 'evoker-fangs.json', {'paths': paths, 'warmup': warmup, 'drawn': drawn})
        assert all(warmup.values()), f'fang warmup not received: {warmup}'
        assert all(drawn.values()), f'fang renderer produced no geometry: {drawn}'
        assert paths['host'].keys() == paths['guest'].keys(), 'clients received different fang ids'
        for id, samples in paths['host'].items():
            other = paths['guest'][id]
            assert all(f['position'] == samples[0]['position'] for f in samples + other), \
                f'fang {id} is not at its authoritative position'
            for name, frames in (('host', samples), ('guest', other)):
                life = {f['life'] for f in frames if f['started']}
                assert len(life) > 1, f'{name}: fang {id} did not animate: {frames}'
        assert not any(fangs(g('status')) for g in (host, guest)), 'fangs survived server removal'
    finally:
        host('command', command='tick unfreeze')
        host('command', command=f'kill @e[tag={tag}]')
        # The evoker can summon vexes before fangs; killing it leaves those summons alive.
        host('command', command=f'kill @e[type=minecraft:vex,x={x},y={y},z={z},distance=..32]')
        guest('command', command='effect clear @s')
        face_each_other(host, guest)
    return {'fangs': len(paths['host']), 'warmup': warmup, 'drawn': drawn, 'removed': True}


CHECKS += ['fangs']
CHECK_FNS.update(fangs=check_fangs)


def check_sonic(host, guest, out):
    """A real warden's ray reaches connected clients once, with normal damage.

    Run with and without --server-thread to cover the window's local renderer too.
    The creative observer sees the same ray nearby and none outside sendParticles' range.
    """
    x, y, z = (int(v // 1) for v in pos_of(host))
    result = {}
    guest('command', command='gamemode creative')
    host('command', command='difficulty normal')
    host('command', command='gamerule natural_health_regeneration false')
    try:
        host('command', command=f'fill {x - 3} {y - 1} {z - 3} {x + 85} {y - 1} {z + 14} minecraft:stone')
        host('command', command=f'fill {x - 3} {y} {z - 3} {x + 15} {y + 4} {z + 14} minecraft:air')
        host('command', command=f'fill {x - 2} {y} {z + 4} {x + 2} {y + 3} {z + 4} minecraft:stone')
        for near in (True, False):
            guest('command', command=f'tp @s {x + (3 if near else 80) + 0.5} {y} {z + 0.5}')
            host('command', command=f'tp @s {x + 0.5} {y} {z + 0.5} 0 0')
            host('command', command='gamemode survival')
            host('command', command='effect give @s minecraft:instant_health 1 5')
            host.wait(4)
            host('command', command=f'summon minecraft:warden {x + 0.5} {y} {z + 9.5} {{NoAI:1b,Tags:["sonic_test"]}}')
            host('command', command='attribute @e[tag=sonic_test,limit=1] minecraft:knockback_resistance base set 1')
            before = {side: made(g, 'sonic_boom') for side, g in (('host', host), ('guest', guest))}
            health = host('status')['player']['health']
            host('command', command='data merge entity @e[tag=sonic_test,limit=1] {NoAI:0b}')
            host('command', command='attribute @e[tag=sonic_test,limit=1] minecraft:movement_speed base set 0')
            host('command', command='damage @e[tag=sonic_test,limit=1] 1 minecraft:player_attack by @s')
            for _ in range(400):
                host.wait(2)
                if host('status')['player']['health'] < health:
                    break
            host.wait(2)  # let the observer process the same packet batch
            seen = {side: made(g, 'sonic_boom') - before[side] for side, g in (('host', host), ('guest', guest))}
            after = host('status')['player']['health']
            result['near' if near else 'far'] = {'particles': seen, 'health': [health, after]}
            host.shot(out, f'sonic-{"near" if near else "far"}')
            host('command', command='kill @e[tag=sonic_test]')
            assert after == health - 10, result
            assert seen['host'] == 15, result
            assert seen['guest'] == (15 if near else 0), result
    finally:
        host('command', command='kill @e[tag=sonic_test]')
        host('command', command='gamerule natural_health_regeneration true')
    return result


CHECKS += ['sonic']
CHECK_FNS['sonic'] = check_sonic


def check_creative_tools(host, guest, out):
    """#2035: switching Survival -> Creative cannot mine with a restricted tool.
    Verify both the player's prediction and the host's authoritative block;
    an ordinary tool must still break the same block.
    """
    seen = {}
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = (int(v // 1) for v in pos_of(player))
        cell = (x, y + 1, z + 2)
        host('command', command=f'fill {x - 1} {y} {z + 1} {x + 1} {y + 3} {z + 3} minecraft:air')
        player('command', command='gamemode survival')
        player('command', command='item replace entity @s weapon.mainhand with minecraft:netherite_sword')
        player('command', command='gamemode creative')
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:stone')
        player.wait(10)
        player.raw('look-at', x + 0.5, y + 1.5, z + 2.5)
        player.raw('mouse', 'left', 'down')
        try:
            player.wait(15)
        finally:
            player.raw('mouse', 'left', 'up')
        seen[label] = {'client': player.raw('block', *cell).get('block'),
                       'server': host.raw('block', *cell).get('block')}
        player.shot(out, 'creative-sword')
        assert seen[label]['client'] == seen[label]['server'] == 'minecraft:stone', seen
        player('command', command='item replace entity @s weapon.mainhand with minecraft:diamond_pickaxe')
        player.wait(6)
        player.raw('mouse', 'left', 'click')
        player.wait(10)
        seen[label]['ordinary_tool'] = host.raw('block', *cell).get('block')
        assert seen[label]['ordinary_tool'] == 'minecraft:air', seen
    return seen


CHECKS += ['creative_tools']
CHECK_FNS['creative_tools'] = check_creative_tools


def check_cushions(host, guest, out):
    """#1935: place a colored cushion, consume it, track it, sit, dismount and break it."""
    seen = {}
    for label, player, color in (('host', host, 'red'), ('guest', guest, 'blue')):
        x, y, z = (int(v // 1) for v in pos_of(player))
        at = (x + 0.5, y, z + 2.5)
        host('command', command=f'fill {x - 1} {y} {z + 1} {x + 1} {y + 3} {z + 3} minecraft:air')
        host('command', command=f'setblock {x} {y - 1} {z + 2} minecraft:stone')
        player('command', command='gamemode survival')
        player('command', command=f'item replace entity @s weapon.mainhand with minecraft:{color}_cushion')
        player.wait(10)
        player.raw('look-at', *at)
        player.raw('mouse', 'right', 'click')
        player.wait(15)
        def cushions(game):
            return [o for o in game('status')['objects'] if o['kind'] == 'minecraft:cushion'
                    and abs(o['position'][0] - at[0]) < .1 and abs(o['position'][2] - at[2]) < .1]
        server, client = cushions(host), cushions(player)
        seen[label] = {'server': server, 'client': client,
                       'remaining': player('inventory')['counts'].get(f'minecraft:{color}_cushion', 0)}
        assert len(server) == len(client) == 1, seen
        assert server[0]['color'] == client[0]['color'] == color, seen
        assert seen[label]['remaining'] == 0, seen
        player.shot(out, 'cushion-placed')
        player.raw('look-at', at[0], at[1] + .125, at[2])
        player.raw('mouse', 'right', 'click')
        player.wait(10)
        seen[label]['riding'] = player('status').get('riding')
        assert seen[label]['riding'] == server[0]['id'], seen
        player.raw('key', 'shift', 'down')
        player.wait(5)
        player.raw('key', 'shift', 'up')
        player.wait(5)
        assert player('status').get('riding') is None, seen
        player('command', command='gamemode creative')
        player.raw('look-at', at[0], at[1] + .125, at[2])
        before = {g.name: g('status').get('particles', {}).get('block', 0) for g in (host, guest)}
        player.raw('mouse', 'left', 'click')
        player.wait(10)
        assert not cushions(host) and not cushions(player), seen
        seen[label]['break_particles'] = {
            g.name: g('status').get('particles', {}).get('block', 0) - before[g.name]
            for g in (host, guest)
        }
        assert all(n == 10 for n in seen[label]['break_particles'].values()), seen
    return seen


CHECKS += ['cushions']
CHECK_FNS['cushions'] = check_cushions


def check_forced_levels(host, guest, out):
    """#1525: forced block/entity ticks outlive the last player and the 300-tick idle timeout.

    Also keep a resident, unforced furnace outside the forced ticket's neighbour ring: scheduling
    a playerless integrated-server level must not enable the fixtures' tick-everywhere fallback.
    """
    if not SERVER_THREAD:
        return 'skipped (needs --server-thread)'
    start = pos_of(guest)
    results = {}
    guest('command', command='gamemode creative')
    try:
        for dimension in (NETHER, END):
            into_level(guest, dimension, 8, 100, 8)
            host('command', command='tick freeze')
            furnace = ('minecraft:furnace{Items:[{Slot:0b,id:"minecraft:raw_iron",count:1},'
                       '{Slot:1b,id:"minecraft:coal",count:1}],cooking_total_time:200,'
                       'cooking_time_spent:0}')
            for x in (8, 56):
                guest('command', command=f'setblock {x} 100 8 {furnace}')
                server_reply(guest, f'execute if data block {x} 100 8 '
                             'Items[{Slot:0b,id:"minecraft:raw_iron"}] '
                             'run say forced-input-ready', 'forced-input-ready')
            guest('command', command='forceload add 8 8')
            guest('command', command='summon minecraft:armor_stand 8.5 104 8.5 {Tags:["forced_tick"]}')
            guest('command', command=f'execute in {OVERWORLD} run tp @s {start[0]} {start[1]} {start[2]}')
            wait_dimension(guest, OVERWORLD, 400)
            host('command', command='tick step 450')
            host.wait(500)
            prefix = f'execute in {dimension} '
            results[dimension] = server_reply(
                host, prefix + 'if data block 8 100 8 Items[{Slot:2b,id:"minecraft:iron_ingot"}] '
                'if data block 56 100 8 {cooking_time_spent:0} '
                'if data block 56 100 8 Items[{Slot:0b,id:"minecraft:raw_iron"}] '
                'if data entity @e[tag=forced_tick,limit=1] {Pos:[8.5d,101.0d,8.5d]} run say forced-level-pass',
                'forced-level-pass')
            assert dims(host, guest) == (OVERWORLD, OVERWORLD), dims(host, guest)
            host('command', command=prefix + 'run forceload remove 8 8')
            host('command', command=prefix + 'run kill @e[tag=forced_tick]')
            host('command', command='tick unfreeze')
    finally:
        host('command', command='tick unfreeze')
        for dimension in (NETHER, END):
            host('command', command=f'execute in {dimension} run forceload remove 8 8')
        guest('command', command=f'execute in {OVERWORLD} run tp @s {start[0]} {start[1]} {start[2]}')
        guest('command', command='gamemode survival')
    return results


CHECKS += ['forced-levels']
CHECK_FNS['forced-levels'] = check_forced_levels


def check_dig_stop(host, guest, out):
    """#1340 and #1908: a block the client breaks at once leaves the server no dig to keep.

    MultiPlayerGameMode sends no STOP or ABORT for a creative break or an instant
    survival break (its isDestroying stays false), so the server must forget the dig on
    its own, as ServerPlayerGameMode.handleBlockBreakAction's creative and insta-mine
    branches do. Otherwise the server keeps mining the old spot: the block put back
    there is destroyed again (#1340) and the mining sound keeps playing at it (#1908).
    """
    result = {}
    spots = apart(host, guest)
    for label, player in (('host', host), ('guest', guest)):
        x, y, z = spots[label]
        cell = (x, y + 1, z + 2)
        seen = result[label] = {}
        for mode, block in (('creative', 'minecraft:stone'), ('survival', 'minecraft:dead_bush')):
            player('command', command=f'gamemode {mode}')
            player('command', command='clear @s')
            host('command', command=f'fill {x - 1} {y} {z + 1} {x + 1} {y + 3} {z + 4} minecraft:air')
            host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} {block}')
            player.wait(10)
            before = {g.name: heard(g, 'minecraft:block.stone.hit') for g in (host, guest)}
            player.raw('look-at', f'{cell[0] + 0.5:.2f}', f'{cell[1] + 0.4:.2f}', f'{cell[2] + 0.5:.2f}')
            # One left-click, then the attack released.
            player.raw('mouse', 'left', 'click')
            player.wait(40)
            seen[f'{mode} broken'] = host.raw('block', *cell).get('block')
            seen[f'{mode} mining'] = player('state').get('mining')
            # The block put back must stay: the server has no dig left to destroy it.
            host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:stone')
            player.wait(100)
            seen[f'{mode} replacement'] = [g.raw('block', *cell).get('block') for g in (host, player)]
            seen[f'{mode} hits'] = {g.name: heard(g, 'minecraft:block.stone.hit') - before[g.name]
                                    for g in (host, guest)}
            host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:air')
        assert seen['creative broken'] == 'minecraft:air', f'{label}: the creative break failed: {seen}'
        assert seen['survival broken'] == 'minecraft:air', f'{label}: the instant break failed: {seen}'
        assert seen['creative mining'] is None, f'{label}: the client still digs: {seen}'
        assert seen['survival mining'] is None, f'{label}: the client still digs: {seen}'
        for mode in ('creative', 'survival'):
            assert seen[f'{mode} replacement'] == ['minecraft:stone'] * 2, \
                f'{label} {mode}: the server kept digging: {seen}'
            assert all(hits == 0 for hits in seen[f'{mode} hits'].values()), \
                f'{label} {mode}: the mining sound kept playing: {seen}'
    return result


CHECKS += ['dig_stop']
CHECK_FNS['dig_stop'] = check_dig_stop


def check_team_push(host, guest, out):
    """Another player's LivingEntity.pushEntities on the guest's screen: EntitySelector.pushableBy reads the
    pusher's team, which for a player goes by its profile name (Player.getScoreboardName), not its UUID. The
    host 0.3 east of the guest pushes it west, unless the host's team has collisionRule never."""
    for g in (host, guest):
        g('command', command='gamemode survival')
    host_at, guest_at = pos_of(host), pos_of(guest)
    x, y, z = guest_at
    x, z = x // 1 + 0.5, z // 1 + 0.5
    seen = {}
    try:
        for case in ('no team', 'never'):
            if case == 'never':
                host('command', command='team add parity_ghosts')
                host('command', command='team modify parity_ghosts collisionRule never')
                host('command', command=f'team join parity_ghosts {host.player}')
            guest('command', command=f'tp @s {x} {y:.4f} {z}')
            host('command', command=f'tp @s {x + 0.3} {y:.4f} {z}')
            guest.wait(40)
            nx, _, nz = pos_of(guest)
            guest.shot(out, 'team-push-' + case.replace(' ', '-'))
            seen[case] = {'dx': round(nx - x, 3), 'dz': round(nz - z, 3)}
            # Apart again before the next case.
            host('command', command=f'tp @s {x + 3} {y:.4f} {z}')
            guest.wait(10)
    finally:
        host('command', command='team remove parity_ghosts')
        # Back where they stood: later checks place the guest next to the host (face_each_other), and the
        # host's last spot here put the guest inside the hill east of the spawn (pvp then hit the wall).
        for g, (px, py, pz) in ((host, host_at), (guest, guest_at)):
            g('command', command=f'tp @s {px:.4f} {py:.4f} {pz:.4f}')
        guest.wait(10)
    assert seen['no team']['dx'] < -0.05, f'the host did not push the guest west: {seen}'
    assert abs(seen['never']['dx']) < 0.01 and abs(seen['never']['dz']) < 0.01, \
        f'a host on a collisionRule never team still pushed the guest: {seen}'
    return seen


CHECKS += ['team-push']
CHECK_FNS['team-push'] = check_team_push


def check_dig_sounds(host, guest, out):
    """#2277: the mining sound plays once per cadence, for everyone.

    Vanilla plays the block's hit sound from the server's destroy progress
    level events (2019 and 2020), so the digger and the players around hear
    the same hits. Horizon's digger heard every hit twice: once from its own
    client and once from the server, which is what the report called a sound
    played too often.
    """
    result = {}
    spots = apart(host, guest)
    for label, player in (('host', host), ('guest', guest)):
        other = guest if label == 'host' else host
        x, y, z = spots[label]
        cell = (x, y + 1, z + 2)
        player('command', command='gamemode survival')
        player('command', command='clear @s')
        host('command', command=f'fill {x - 1} {y} {z + 1} {x + 1} {y + 3} {z + 4} minecraft:air')
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:stone')
        player.wait(10)
        before = {g.name: heard(g, 'minecraft:block.stone.hit') for g in (host, guest)}
        player.raw('look-at', f'{cell[0] + 0.5:.2f}', f'{cell[1] + 0.5:.2f}', f'{cell[2] + 0.5:.2f}')
        player.raw('mouse', 'left', 'down')
        player.wait(40)
        player.raw('mouse', 'left', 'up')
        player.wait(5)
        hits = {g.name: heard(g, 'minecraft:block.stone.hit') - before[g.name] for g in (host, guest)}
        result[label] = hits
        assert host.raw('block', *cell).get('block') == 'minecraft:stone', f'{label}: the stone broke: {result}'
        assert hits[other.name] >= 3, f'{label}: the player nearby heard no mining sound: {result}'
        assert hits[player.name] <= hits[other.name] + 3, \
            f'{label}: the digger heard the mining sound twice: {result}'
        host('command', command=f'setblock {cell[0]} {cell[1]} {cell[2]} minecraft:air')
    return result


CHECKS += ['dig_sounds']
CHECK_FNS['dig_sounds'] = check_dig_sounds


def check_shulker_menu(host, guest, out):
    """A shulker box opens a ShulkerBoxMenu (ShulkerBoxBlockEntity.createMenu), whose ShulkerBoxSlots
    refuse shulker boxes. The server opened it as a 9x3 chest menu, which took them."""
    x, y, z = (int(v // 1) for v in pos_of(guest))
    box = (x, y, z + 2)
    try:
        guest('command', command='gamemode survival')
        guest('command', command='clear @s')
        guest('command', command='give @s minecraft:cobblestone 1')
        guest('command', command='give @s minecraft:red_shulker_box 1')
        guest('command', command=f'setblock {box[0]} {box[1]} {box[2]} minecraft:shulker_box')
        guest.wait(10)
        guest.raw('interact', *box)
        guest.raw('wait-for', 'screen', 'container', '--timeout', 10)
        guest.wait(10)
        # The cobblestone goes in; the red shulker box stays on the cursor.
        mouse(guest, hotbar_slot(guest, 'minecraft:cobblestone'))
        mouse(guest, 301)
        mouse(guest, hotbar_slot(guest, 'minecraft:red_shulker_box'))
        mouse(guest, 300)
        guest.wait(10)
        guest.shot(out, 'shulker-menu')
        guest.raw('key', 'esc', 'tap')
        guest.wait(10)
        inside, size = open_container(guest, box, out, 'shulker-menu-after')
        counts = guest('inventory')['counts']
        stored = host('command', command=f'data get block {box[0]} {box[1]} {box[2]} Items')
    finally:
        guest('command', command=f'setblock {box[0]} {box[1]} {box[2]} minecraft:air')
        guest('command', command='clear @s')
    assert size == 27 and inside == {1: 'minecraft:cobblestone'}, f'the shulker box holds {inside} ({size})'
    assert 'red_shulker_box' not in json.dumps(stored), f'the server stored {stored}'
    assert counts.get('minecraft:red_shulker_box') == 1, f'inventory after: {counts}'
    return {'inside': inside, 'inventory': counts}


CHECKS += ['shulker_menu']
CHECK_FNS['shulker_menu'] = check_shulker_menu


def check_keys_released(host, guest, out):
    """Gui.setScreen -> KeyMapping.releaseAll: the screen shown after leaving a server releases every held
    key, so W and Space held through the disconnect don't walk and jump on in the next world (#3246)."""
    port = host.raw('lan', 'status')['port']
    guest.raw('key', 'w', 'down')
    guest.raw('key', 'space', 'down')
    held = sorted(guest('status')['keys'])
    guest.raw('disconnect')
    left = sorted(guest('status')['keys'])
    guest.raw('connect', f'127.0.0.1:{port}', GUEST_NAME)
    guest.raw('wait-for', 'loaded', '--timeout', 300)
    guest.wait(20)
    rejoined = sorted(guest('status')['keys'])
    start = pos_of(guest)
    guest.wait(20)
    end = pos_of(guest)
    walked = round(math.hypot(end[0] - start[0], end[2] - start[2]), 3)
    seen = {'held': held, 'left': left, 'rejoined': rejoined, 'walked': walked}
    assert held == ['KeyW', 'Space'], f'the keys were not pressed: {seen}'
    assert left == [] and rejoined == [], f'keys stayed down after leaving: {seen}'
    assert walked < 0.1, f'the guest walked on its own after rejoining: {seen}'
    return seen


CHECKS += ['keys_released']
CHECK_FNS['keys_released'] = check_keys_released


if __name__ == '__main__':
    main()
