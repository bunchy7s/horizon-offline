# Controllers

Horizon plays with PlayStation (DualShock 4, DualSense), Xbox (360, One,
Series) and Nintendo Switch Pro controllers, and on the Steam Deck with its
built-in controls. Keyboard and mouse keep working exactly as before: with no
controller connected, or with controllers turned off, nothing changes.

## What vanilla has

Nothing. Minecraft Java 26.3 never calls GLFW's gamepad or joystick API, has
no controller options and ships no button glyphs. Its only non-mouse UI input
is keyboard focus navigation (`InputType.KEYBOARD_ARROW` / `KEYBOARD_TAB`:
arrows and Tab move a focus ring between widgets). So controller support is
Horizon's own feature. It follows what players already know from Minecraft on
consoles (Bedrock Edition's default controller layout) and Horizon's menu
design for any new screen.

## Input layer

[gilrs](https://crates.io/crates/gilrs) reads the pads: evdev on Linux (with
libudev for hot-plugging), IOKit on macOS, Windows.Gaming.Input on Windows.
Its SDL mapping database normalizes every pad to one positional layout
(South/East/West/North face buttons, bumpers, triggers, sticks, D-pad,
Select/Start). Horizon also injects a few SDL rows gilrs' bundled DB still
misses (notably the 8BitDo Ultimate 2 on Linux/Bluetooth, whose right stick
is on ABS_Z/ABS_RX without that row), and corrects that pad by vendor/product
when no SDL row matched. No second windowing or event stack: gilrs is polled
once per loop iteration next to winit's events.

- The pure logic (mapping table, deadzones and curves, button edges, the
  settings file, glyph choice) lives in `crates/horizon-client-input/src/gamepad.rs`
  and is unit-tested. The game owns a thin gilrs backend and applies actions.
- Controller actions go through the same path a keyboard key takes after
  rebinding: an action names a vanilla key mapping (`key.jump`, `key.attack`,
  ...), and the game handles it as that mapping's default key or mouse button.
  Toggle sneak/sprint/attack/use, screens releasing held keys, and the
  multiplayer client all behave as they do on the keyboard.
- Movement is analog: the left stick's direction and tilt feed the player's
  move vector directly (a half-tilted stick walks slowly, like Bedrock),
  instead of being rounded to WASD. The forward/back/left/right flags other
  code reads (sprint start, boats, minecarts, the server's input packet) are
  set when the stick passes a threshold. On a server (not this game's own
  server thread) the stick moves as those keys do, in 8 directions at full
  speed, as the server only hears the keys: an anticheat predicting from them
  pulls back a player moving at the stick's own angle and speed.
- The pad last touched is the active one. Several pads are not merged.
- `horizon-ctl pad ...` injects a virtual pad (buttons, sticks, triggers) into
  a running game, for tests without hardware.

## Default mapping

| Action | Xbox | PlayStation | Mapping |
|---|---|---|---|
| Move | Left stick | Left stick | analog `key.forward/back/left/right` |
| Look | Right stick | Right stick | camera |
| Jump / fly up | A | ✕ | `key.jump` |
| Drop item | B | ○ | `key.drop` |
| Swap offhand | X | □ | `key.swapOffhand` |
| Inventory | Y | △ | `key.inventory` |
| Hotbar left / right | LB / RB | L1 / R1 | Horizon `pad.hotbarPrevious/Next` |
| Use / place | LT | L2 | `key.use` |
| Attack / break | RT | R2 | `key.attack` |
| Sprint | Left stick click | L3 | `key.sprint` |
| Sneak (toggles) | Right stick click | R3 | `key.sneak` |
| Change camera | D-pad up | D-pad up | `key.togglePerspective` |
| Pick block | D-pad left | D-pad left | `key.pickItem` |
| Chat | D-pad right | D-pad right | `key.chat` |
| Advancements | D-pad down | D-pad down | `key.advancements` |
| Player list (hold) | View | Create / Share | `key.playerlist` |
| Pause / back | Menu | Options | Escape (fixed) |

The Guide / PS / Steam button is left to the system. Sneak on a stick click
toggles by default (holding a stick in while steering the camera is
awkward, which is why Bedrock does the same) and acts like vanilla's toggle
crouch: a screen stands you up while it is open and closing it crouches you
again. The controls options can make it hold-to-sneak.

## Menus and inventories

Vanilla's screens are all mouse-driven, and Horizon draws them 1:1. A pad
drives them in one of two ways, picked in the controller options ("Menu
Navigation: Focus / Pointer", `menuNavigation:focus|pointer`):

- **Focus** (the default): a console-style highlight that the D-pad and the
  left stick move between buttons, sliders, switches, text fields, list
  rows, tabs and inventory slots.
- **Pointer**: a virtual cursor the left stick moves, as Bedrock's classic
  inventory UI and Java's controller mods do.

Focus is the default because it is what a pad player expects from a console
game, it needs no aiming (every widget is one D-pad press away, which also
suits the Steam Deck's small screen), and it works from the couch. The
pointer stays one option away, and a **right stick click swaps the two for
the open screen**, for the few places a pointer is better (dragging the
advancements map, painting a crafting grid with a held stack).

Both modes feed the screens the same pointer events a mouse sends: in focus
mode the (hidden) pointer sits on the focused widget. So every screen keeps
its mouse behaviour (drags, sliders, scrollbars, hover highlights) with no
controller code of its own, and both modes reach every screen. Moving the
real mouse hands control back at once.

### Focus navigation

Every interactive rectangle a screen draws registers itself as a focus
target with its kind (`widgets::target_as`): classic buttons, sliders, edit
boxes and list rows (worlds, servers, languages), icon buttons (the title's
and pause menu's Friends button), inventory and container slots, creative and recipe-book
tabs, and the stations' own buttons (trades, loom patterns, beacon effects,
enchantments, stonecutter recipes, the recipe book's recipes, pages and
filter). A screen that registers nothing falls back to the pointer.

The step itself is **vanilla's arrow-key navigation**
(`ContainerEventHandler.nextFocusPathInDirection`, 26.3), applied to every
target of the screen at once: from a strip one pixel past the focused edge,
the nearest target that overlaps it on the other axis wins (ties go to the
leftmost or topmost); without one, the target whose facing edge's centre is
nearest the focused edge's centre (`nextFocusPathVaguelyInDirection`). That
gives vanilla-like jumps in containers too: hotbar to inventory, the grid to
the crafting result, armour to the player's grid.

| Input | Focus mode |
|---|---|
| D-pad, left stick | move focus (held: repeats after 0.35 s, every 0.09 s) |
| A | press the focused widget (held: drag); on a text field, type with the on-screen keyboard |
| B, Menu | back (Escape) |
| X | right click (inventories: take half, place one) |
| Y | quick move (shift-click) in inventories |
| View | throw one item from the focused slot (Q) |
| Left stick click | throw the focused stack (Ctrl+Q) |
| LB / RB | previous / next tab (creative, recipe book, the Horizon options' sections) |
| Right stick | scroll |
| Right stick click | swap focus and pointer for this screen |

- **Sliders**: left and right move the value (one pixel per press like
  vanilla's arrow keys, faster while held); A does nothing on a slider, as a
  click would jump the handle.
- **Lists** follow focus: a step past the last visible row scrolls the list
  and lands on the next row once it is drawn.
- **Inventories**: the carried stack is drawn on the focused slot (vanilla
  draws it under the pointer). A takes or places, X splits or places one, Y
  quick-moves, View and the left stick click throw.
- **Where focus lands** when a screen opens: the held hotbar slot in an
  inventory, else the first button.

It looks as vanilla's own keyboard focus does: buttons, sliders and slots
show their highlighted sprite (`isHoveredOrFocused`, drawn because the hidden
pointer is over them), and list rows and text fields get vanilla's white
one-pixel outline (`AbstractSelectionList`'s focused selection, `EditBox`'s
highlighted border).

### Keyboard

Vanilla's own keyboard focus navigation (`Screen.keyPressed`, 26.3) uses the
same focus, so it works with no controller and with controllers turned off
(`game/key_focus.rs` decides, `game/pad_menu.rs` applies it, and
`game/startup_menu.rs` on the title shown while resources load). It matters
most for the narrator: the focused widget is the narration's focused one.

| Key | Does |
|---|---|
| Tab / Shift+Tab | next / previous widget in tab order, wrapping (`TabNavigation`) |
| Arrows | the widget that way (`ArrowNavigation`, the step above; past a list's last visible row the list scrolls) |
| Enter, Space, keypad Enter | press the focused button, cycle button, checkbox or tab (`AbstractButton.keyPressed`: a click on it, with its sound) |

- **Sliders** (`AbstractSliderButton`): focused with Tab or a click, left
  and right change the value (one pixel per press, `1 / (width - 8)`);
  focused with an arrow they move the focus until Enter lets them change
  it. Enter toggles that. Up and down always move the focus.
- **Text fields** keep left and right for their caret (`EditBox.keyPressed`),
  and Enter and Space for themselves; Tab, up and down leave them. Focus
  landing on a field focuses it with the caret at the end; leaving it
  unfocuses it (title screens).
- **Lists**: a list's rows are one Tab stop (Tab enters on its first row,
  Shift+Tab on its last, the next Tab leaves); up and down move between rows.
  On the world list focus selects the row and Enter joins it, as vanilla;
  on other lists Enter clicks the row (selects it, or joins the selected
  server).
- With nothing focused, Enter and Space go to the screen (the world list's
  Enter joins the selected world) and an arrow starts from the screen's
  edge behind it. A click focuses what it lands on (`mouseClicked`), so the
  next Tab or arrow goes on from there.
- A new screen opened by a key starts on its first widget in tab order
  (`Screen.setInitialFocus`); one opened from the game has no focus.
- The keyboard doesn't reach inventory slots or the creative and recipe-book
  tabs (vanilla's screens don't focus them): in container screens only
  their buttons (the recipe book button, trades, beacon effects).
- Screens that keep the keys: chat (Tab completes commands), signs, books
  being written, the creative, recipe-book and mod searches, the anvil's
  name, command blocks, dialogs, a key or button being bound on Controls,
  the credits, the on-screen keyboard.

It looks as vanilla's keyboard focus does: the focused widget draws its
highlighted sprite, and rows and fields the white outline (as above). As
for the pad, the screens draw their highlight from the pointer, so the focus
puts Horizon's pointer (not the OS one, which stays where the mouse left it
and stays visible) on the focused widget, and Enter is a click there. Moving
the mouse hands the pointer back; a click or a scroll first puts it back
where the mouse is.

Approximations:

- **Tab order** is reading order (top to bottom, then left to right), not
  vanilla's add order (`children()` by `getTabOrderGroup`). Horizon has no
  widget tree, and a screen's buttons and other targets are registered
  separately, so their draw order isn't one sequence; vanilla's layouts add
  widgets row by row, which reading order follows.
- Tab visits the visible widgets only (a scrolled-away options row is
  reached with the arrows, which scroll); a list is entered on its first or
  last visible row, not its selected one.
- A focused slider whose arrows don't change it yet shows the hovered look,
  not vanilla's highlighted track.
- The widget under the real mouse isn't highlighted while the keyboard has
  the pointer (vanilla highlights both).
- The narration reads a slider's `canChangeValue` from the input type, so
  Enter sets the input type to match.

### Pointer

- Left stick moves the cursor with acceleration; in container screens the
  cursor is gently pulled toward the nearest slot centre.
- D-pad steps to the nearest target that way (a slot, a button) and snaps to
  it.
- A = left click, X = right click, Y = quick move, B = back; View and the left
  stick click throw, as in focus mode.
- LB / RB switch tabs; right stick scrolls.
- Sliders: hold A on the slider and move the stick, or nudge with the D-pad.

The cursor is drawn only while the pad is the last-used input.

### Text fields

A on a focused text field (A in pointer mode clicks into it) opens the
**on-screen keyboard**: a QWERTY grid and a symbols grid in Horizon's design
language, at the bottom of the screen (at the top when the field is in the
lower half, as chat is). D-pad or stick to move, A types, X deletes, Y is a
space, LT is Shift (twice: caps lock), RB switches to symbols and accents,
Menu is Enter, B closes it. It opens by itself when the pad opens chat or a
sign. It types into every field the keyboard reaches: chat (local and
server), signs, books, the title screens' fields, the options' searches, the
creative and recipe-book searches, the anvil, the LAN port.

On a Steam Deck under Steam (`SteamDeck=1`), A asks Steam for its own
keyboard instead (`steam://open/keyboard`), which types like a real keyboard;
Steam + X opens it at any time. "On-Screen Keyboard: Auto / Horizon / Steam"
(`keyboard:`) picks one.

### Screens covered

Both modes reach every screen through the pointer events. Focus mode
registers targets on: the title menus (main, world list and its search, create
and edit world, delete, multiplayer list and its edit screens), every options
page (vanilla's and Horizon Settings with the pages it opens, key and
controller binds, language list), pause and Play with Friends, death, chat, signs and books (their buttons), the
survival and creative inventories, every container and station (chests,
furnaces, brewing, crafter, crafting table with its recipe book, anvil,
grindstone, smithing, stonecutter, loom, cartography, enchanting, beacon,
villager and wandering-trader trades, mount inventories), and the LAN /
world options. Where a screen draws something clickable no target names
(the advancements map, the statistics lists, the pack screen's rows, the
book's page text), the right stick click gives the pointer for that screen.

## Glyphs

Horizon draws its own button glyphs in the HUD and screens (no Mojang
assets, nothing new from the player's jar): a small round or pill shape with
the label in the game font, sized to the GUI scale, with two styles:

- **Xbox / Steam Deck / generic**: A B X Y in their usual colours, LB RB LT
  RT, View, Menu, stick icons.
- **PlayStation**: × ○ □ △ in their colours (× rather than ✕, which the game fonts lack), L1 R1 L2 R2,
  Create/Share, Options.
- **Nintendo**: B A Y X by position.

The style comes from the pad's USB vendor (Sony 054c, Microsoft 045e,
Nintendo 057e, Valve 28de) unless the controls options force one. Glyphs
replace key names where a prompt names a key (the controls list, tutorial
hints, "Press X to dismount") only while the pad is the last-used input.

## Options and remapping

Controller settings are Horizon's own: **Options → Controls → Controller
Settings...** (or **Options → Horizon Settings... → Controller Settings...**),
a vanilla options list with **Controller Binds...** at the top. They live in
`horizon-controller.txt` next to `options.txt`, like `horizon-graphics.txt`: `options.txt` stays
exactly vanilla's, so a vanilla client can share the game folder.

```
enabled:true
vibration:true
adaptiveTriggers:true
lightbar:health
lookSensitivity:0.5
invertX:false
invertY:false
moveDeadzone:0.15
lookDeadzone:0.12
triggerThreshold:0.4
toggleSneak:true
glyphs:auto
bind_key.jump:south
bind_pad.hotbarNext:rb
```

Only bindings that differ from the defaults are written. The controls
options get a Controller page: on/off, vibration, adaptive triggers, lightbar, look sensitivity, invert
X/Y, deadzones, glyph style, sneak toggle, and a binding list like the key
binds screen (pick an action, press a button; conflicts marked as vanilla
does; reset).

Deadzones are radial (the stick's length, so diagonals aren't clipped) and the
remaining range is rescaled to 0..1. Look uses a squared curve for precise
aim near the centre, up to 200°/s yaw and 140°/s pitch at the default
sensitivity, slower while scoping with a spyglass.

## Rumble

Vibration (on by default, off in the options): taking damage (stronger with
the damage), a block broken, an explosion nearby (scaled by distance), a totem
of undying. Short, low-intensity pulses, as on Bedrock. A DualSense gets them
shaped, through its own output report (below), on every OS. Other pads go through
gilrs force feedback: Linux (evdev force feedback, including Steam Input's
virtual pad) and Windows support it; macOS gilrs has no force feedback, so it
is silently skipped there.

## DualSense output

Gilrs only reads the pads. A DualSense (054c:0ce6, the Edge 054c:0df2) is also
written to directly, over HID: the `horizon-dualsense` crate builds its output
report and [hidapi](https://crates.io/crates/hidapi) (MIT, bundling the C
hidapi under its BSD option) sends it: hidraw on Linux, IOKit on macOS (opened
shared, so gilrs keeps reading the same pad), the HID API on Windows.

- **Report.** One 47-byte block carries everything: which parts to apply
  (three flag bytes), the two compatible-vibration motors, both adaptive
  triggers' effects, the lightbar colour and the player LEDs. Over USB it is
  report 0x02; over Bluetooth report 0x31, led by a sequence number and a tag
  and closed by a CRC-32 of the seed byte 0xA2 and the report. Unit tests pin
  the bytes to the references below (the CRC to zlib's, the LED reset to the
  kernel's).
- **Rumble.** Every report carries the motors: a report without the rumble
  flags ends the motors' emulation and hands the actuators back to the audio
  haptics (that is how SDL stops rumble, and how the HD haptics are re-armed),
  so a trigger change mid-rumble would otherwise cut it. A pad with update
  version 0x0224 or later (feature report 0x20), and every Edge, takes the
  newer emulation (flags 2, bit 2); older or unknown firmware the legacy one
  (flags 0, bit 0), at half strength as SDL does. The kernel takes the newer
  one from 0x0215; between the two, legacy works on every firmware. The first
  report after opening clears the motor and trigger attenuation another host
  (a PS5's intensity settings) may have left (flags 1, bit 6, byte 36 = 0).
- **Lights.** Before the first colour, a report of its own resets the lights
  (flags 1 bit 3, and the lightbar setup "light out"; the kernel does the
  same). Over Bluetooth the pad plays a connection animation that paints over
  the lightbar: like SDL, the colour waits until the pad's sensor clock
  (input report 0x31) passes 10 200 000.
- **Bluetooth.** Any output report, even reading a feature report, switches a
  pad on Bluetooth to its full input report (0x31) until it is turned off. On
  Linux `hid-playstation` has done that already and gilrs reads its events,
  so output goes ahead. Gilrs on macOS and Windows parses the simple report
  0x01 instead and would stop seeing the pad (SDL's
  `SDL_HINT_JOYSTICK_ENHANCED_REPORTS`: "break Windows DirectInput for other
  applications"), so there a pad on Bluetooth gets no output unless
  `HORIZON_DUALSENSE_BT=1`. Doing it for everyone means reading the pad's
  input ourselves over HID, as SDL does (a later PR). A pad on both cables is
  written over USB.
- **Writer thread.** Writes block (a Bluetooth write can take milliseconds),
  so a thread owns the pad: the game hands it the state to show and timed
  vibrations, and it sends only what changed, and everything again when the
  pad reconnects. It opens the pad once this window has the pad in hand (the
  pad is active and the window has focus), and looks again whenever gilrs
  sees a controller come or go.
- **Off-safe.** Quitting (and the thread ending any other way) frees the
  triggers and stops the motors before the pad is closed. Another game on the
  same pad is left alone: nothing is written while this window is unfocused.
- **Fallback.** If the pad can't be opened (Linux without access to its
  hidraw node), vibration silently stays with gilrs, as before.
- **Linux access.** `/dev/hidraw*` is root-only by default. Steam's udev rules
  (`60-steam-input.rules`, installed with Steam or the `steam-devices`
  package) already give the logged-in user the DualSense, over USB and
  Bluetooth. Without Steam, this rule does the same (the launcher could
  install it):

  ```
  # /etc/udev/rules.d/70-horizon-dualsense.rules
  KERNEL=="hidraw*", ATTRS{idVendor}=="054c", ATTRS{idProduct}=="0ce6|0df2", TAG+="uaccess"
  KERNEL=="hidraw*", KERNELS=="*054C:0CE6*|*054C:0DF2*", TAG+="uaccess"
  ```

  The kernel's own `hid-playstation` driver keeps working alongside: its
  reports set only the parts their flags name.
- **Trying it without the game:** `cargo run -p horizon-dualsense --example
  dualsense -- all` plays each output on a connected pad and reads back what
  the pad reports: the triggers' active effect, and the accelerometer's
  shake while the motors run (`watch` logs the shake while the game drives
  the pad).

### Adaptive triggers

On by default with a DualSense ("Adaptive Triggers" in the controller
options, `adaptiveTriggers:`). Each trigger feels the action bound to it, so
rebinding moves the feel along: with the default mapping that is L2 for
`key.use` and R2 for `key.attack`.

| While | Trigger | Effect (zones 0..9, strength 1..8) | Feel |
|---|---|---|---|
| Drawing a bow | use | feedback from zone 2, strength 1 rising to 6 with the draw (`BowItem.getPowerForTime`) | the string's tension builds; letting go frees the trigger at once |
| Charging a trident | use | the same, at 6 after the 10 ticks a throw needs | |
| Loading a crossbow | use | the same, at 6 when it is loaded | |
| Holding a loaded crossbow | use | weapon: from two zones before the press zone to the press zone, strength 8 | a gun trigger: a hard catch that breaks with a click as it fires |
| Gliding with a rocket, riding a pig or a strider with its stick | use | weapon, the same zones, strength 5 | a lighter catch: pulling through it is the boost |
| A shield in the use hand (raised or ready) | use | feedback from zone 2, strength 3 | its weight |
| That shield disabled by an axe | use | feedback from zone 1, strength 8 | the trigger locks until the shield recovers |
| Mining (survival) | attack | vibration from zone 1: amplitude 2 (dirt) to 5 (obsidian) of 8, 40 Hz on sand and dirt, 30 on stone, 20 on wood, deepslate and ores, 10 on obsidian | a grit that gets heavier and slower on harder blocks; none on blocks that break at once |

The press zone is where Horizon presses the trigger's action: the trigger
threshold (0.4 by default) times ten, kept between 3 and 8, so with the
defaults a crossbow catches from zone 2 and breaks at zone 4, as its bolt
leaves. This assumes the ten zones split the travel evenly, as dualsensectl
documents them (0 at rest, 9 fully pressed); it is not measured.

Everything else, and every menu, the pause screen, a window without focus,
another pad being the active one, and controllers turned off, leaves the
triggers free; quitting frees them too. `pad status` shows the effects in
use.

Why these values, from what shipped games do and what the community
documents (none of it measured on Horizon's pad yet):

- **Only the official modes.** Off 0x05, feedback 0x21, weapon 0x25 and
  vibration 0x26 are the modes Nielk1's notes call official and expected in
  future firmware; bow 0x22, galloping 0x23 and machine 0x27 "might be
  removed", and the low "simple" modes (0x01, 0x02) that pydualsense and
  DualSense-Windows send are leftovers. The bow mode's snap-back would suit a
  bow, but a firmware update could take it away.
- **Bows build tension.** Kena: Bridge of Spirits and Astro's Playroom use
  the triggers to make you feel the bow being drawn (PlayStation Blog's
  DualSense round-up, as reported by
  [GameSpot](https://www.gamespot.com/articles/sony-reveals-how-ps5s-dualsense-controller-works-with-ratchet-clank-returnal-scarlet-nexus-and-more/1100-6491469/)). Minecraft holds the draw for
  as long as you like, so the tension stops at 6 of 8 (dualsensectl: 1 is
  "almost no resistance", 8 the maximum), and starts after the first two
  zones so the press that starts the draw stays light.
- **A loaded crossbow is a gun trigger.** The developer note GameSpot quotes
  describes a bazooka whose resistance "will cause the trigger to catch at the
  input position just before the bullet is fired, and as you continue to pull
  the trigger, the bullet will fire and release the tension": the weapon
  effect does exactly that, and Horizon moves its breaking point onto the
  threshold so the click is the shot. DSX community presets use the same
  shape (DSX++'s example `Weapon {2, 6, 8}`; dualsensectl's
  `weapon 2 3 8` "resists strongly and lets go almost immediately").
  Returnal and Ratchet & Clank: Rift Apart use a half-press stop to split two
  actions on one trigger; Minecraft has one action per trigger, so Horizon
  doesn't.
- **Boosts click lighter.** A rocket or a pig's carrot fires on the pull, so
  it gets a catch that gives way rather than a wall that never does (strength
  5, below the crossbow's 8).
- **A disabled shield locks the trigger**, as Deathloop locks R2 halfway
  when a gun jams, "to give to the player an immediate feedback even before
  the animation plays out" (its director, Dinga Bakaba, in
  [Destructoid's report](https://www.destructoid.com/when-a-gun-jams-in-deathloop-the-ps5-controllers-triggers-will-lock-up/)):
  the shield can't be raised, and the finger feels why.
- **Mining is a texture.** The vibration mode buzzes the trigger at a given
  rate; reWASD's preset uses 30 Hz and its "rifle" 2..20 Hz. Softer blocks
  get a finer, faster grit, harder ones a slower, heavier chug, never above
  5 of 8, since the attack trigger is held for seconds.

### Vibration shapes and HD haptics

Each event plays a short shape rather than one flat block: a hit opens at
full strength for 50 ms and dies away in two steps, an explosion rolls on for
half a second, a totem swells, a broken block is a 35 ms tick. Over HID they
drive the motors' emulation (the compatible-vibration bytes). Measured on the
pad, that emulation has inertia: after a full-strength step it takes about
half a second to settle, so short shapes blur.

The DualSense has no motors, but two voice-coil actuators, and on USB it is
also a four-channel 48 kHz audio device whose channels 3 and 4 drive them
(1 and 2 are its speaker and headset). Played there as waveforms (an 80 Hz
tone for the heavy part, 200 Hz for the light one, 4 ms ramps between steps,
channels 1 and 2 silent), the same shapes come out crisp. Per OS:

| | USB | Bluetooth |
|---|---|---|
| macOS | the pad lists as a 4-channel CoreAudio output ("DualSense Wireless Controller", seen on the Mac mini by dualsense-1); cpal opens it by name | no audio device |
| Linux | snd-usb-audio exposes the card "Controller" with 4 channels; cpal opens it through ALSA (`plughw:CARD=Controller`), which fails while PipeWire or PulseAudio holds the card, as desktops do: then the motors' emulation stays | no audio device |
| Windows | a WASAPI endpoint named "Wireless Controller" (or "DualSense Wireless Controller"), stereo until its speaker setup is set to Quadraphonic in the Sound control panel (`mmsys.cpl`, Configure); until then Horizon finds no 4-channel output and keeps the motors | no audio device |

The audio haptics play only while the pad is out of its rumble emulation:
any report with the rumble flags (flags 0, bits 0 and 1) silences them until
a report without them (SDL: "Leaving emulated rumble bits off will restore
audio haptics"; nondebug/dualsense and
[dse-haptics](https://github.com/iranran666/dse-haptics) arm them with an
all-zero report 0x02). While HD haptics are open the motors stay at zero, so
every report Horizon sends, starting with the one at opening, keeps them
armed; that first report also clears the attenuation a PS5 may have left.
The frame layout `[0, 0, left, right]` at 48 kHz is the one
[KytyPS5 #843](https://github.com/KytyPS5/KytyPS5/pull/843) plays on
Windows.

Over Bluetooth the haptics travel as 8-bit 3 kHz samples in vendor output
report 0x32, reverse-engineered by
[SAxense](https://github.com/egormanga/SAxense) and since used by DSX 3.2
and [dualsense-bt-haptics](https://github.com/awalol/dualsense-bt-haptics);
it needs Bluetooth output, which only Linux gets (see above), and isn't
done: there the motors' emulation is used. HD haptics are opt-in for now
(`HORIZON_DUALSENSE_HAPTICS=1`) until they are confirmed on a real pad;
games started for tests (`HORIZON_NO_AUDIO`) never open them, and the pad's
audio device is only ever opened by name, never the system's default output.

### Lightbar and player LEDs

"Lightbar: Health / Dimension / Off" in the controller options
(`lightbar:`, health by default). Health runs from green at full through
amber at half to red; Dimension is blue in the Overworld, orange in the
Nether, violet in the End. Both at half brightness (the pad's full light is
glaring); menus keep the last colour. The player LEDs show player 1 (the
middle LED). Off leaves both to the pad. When Horizon lets go of the pad, a
lightbar or LEDs it changed go back to a host's defaults: blue (0, 0, 128)
and the middle LED, as Linux's driver gives a first pad (neither SDL nor
the kernel resets anything when it lets go; Horizon does, so the pad never
keeps a game's colour or a stiff trigger). The microphone's mute LED is
never touched (flags 1, bit 0 stays clear): whatever the system set
stays. Over Bluetooth the colour waits for the pad's connection
animation (see "DualSense output"). Verified against the references only.

Byte layout from public references, read for facts; no code is copied from
them (the kernel and dualsensectl are GPL):

| Reference | Read at | What it settles |
| --- | --- | --- |
| SDL [`SDL_hidapi_ps5.c`](https://github.com/libsdl-org/SDL/blob/6defbc0129b5ba90bab3f6c67bbc1e9329e24ff1/src/joystick/hidapi/SDL_hidapi_ps5.c) and [`SDL_hints.h`](https://github.com/libsdl-org/SDL/blob/6defbc0129b5ba90bab3f6c67bbc1e9329e24ff1/include/SDL3/SDL_hints.h) (zlib) | `6defbc0` | the 47-byte block, rumble flags and stop, 0x0224, legacy at half, LED reset and the Bluetooth animation clock, USB report length 48, the enhanced-mode switch |
| Linux [`hid-playstation.c`](https://github.com/torvalds/linux/blob/045958acaa75205b31578cb22c4345e28dd2e4bf/drivers/hid/hid-playstation.c) (GPL) | `045958a` | valid flags, the Bluetooth sequence and tag 0x10, CRC seed 0xA2, `dualsense_reset_leds`, `use_vibration_v2` from 2.21, player LED patterns, the default blue |
| [dualsensectl](https://github.com/nowrep/dualsensectl/blob/d3ae2fa272e1bf9a5c08a0ded3f374b40f7c73ac/main.c) (GPL) | `d3ae2fa` | trigger valid bits (flags 0 bits 2, 3), `reduce_motor_power` (byte 36) and its flag, the trigger effect packing |
| Nielk1's [TriggerEffectGenerator](https://gist.github.com/Nielk1/6d54cc2c00d2201ccb8c2720ad7538db/f73a299fdc52f43be84d39e62734192ad82bb393) (MIT) | rev. `f73a299` | the trigger modes (official 0x05, 0x21, 0x25, 0x26) and their zone packing |
| [pydualsense](https://github.com/flok/pydualsense/blob/01445f5ed6ba/pydualsense/pydualsense.py) (MIT) | `01445f5` | sets the attenuation flag with 0 on every report |
| [nondebug/dualsense](https://github.com/nondebug/dualsense) | `b87450e` | an all-zero report 0x02 leaves rumble emulation for the audio haptics |
| [KytyPS5 #843](https://github.com/KytyPS5/KytyPS5/pull/843), [dse-haptics](https://github.com/iranran666/dse-haptics), [SAxense](https://github.com/egormanga/SAxense) | read 2026-10-15 | the 4-channel layout, the Windows and Linux device names and setup, the Bluetooth haptics report |

Verified on hardware (USB, dualsense-1, Mac mini): the motors with the V2
flags (the IMU shook, and settled after a stop that then still sent the
flags), the pad reporting a feedback and a weapon trigger effect as active,
and the writer thread. Verified against the references only: everything
else, the flag-less stop, the attenuation and LED reset reports, and all of
Bluetooth.

## Steam Deck

- In Game Mode, or any game launched through Steam, Steam Input presents the
  Deck's controls as a virtual Xbox pad: Horizon sees it like any Xbox pad
  and uses Xbox-letter glyphs (the Deck's face buttons are A B X Y). Steam's
  default "Gamepad" layout works with no setup; the trackpads stay mouse.
- Started outside Steam on the Deck, the kernel's `hid-steam` driver exposes
  the controls as a gamepad too (while Steam isn't holding them), so it also
  works out of the box.
- Text entry uses Steam's keyboard (Steam + X).

## PR plan

The first round (input layer, menus, glyphs and options, rumble) shipped in
PRs #120, #129, #145 and #149. The second round, each shippable alone:

1. **Focus navigation** (this section above): the menu navigation option,
   focus targets and vanilla's arrow-key step, vanilla's focus highlight,
   the on-screen keyboard.
2. **Button prompts**: contextual hints with the pad's glyphs in menus and
   optionally in game, and rebinding from them.
3. **DualSense output** (above): the HID output layer and rumble through it
   first, then adaptive triggers, shaped vibration and the lightbar and
   player LEDs.
4. **Launcher**: the same focus navigation in the gpui launcher.

Each is tested with unit tests (focus steps, prompt contexts, report bytes),
the injected pad and vscreen screenshots; each PR says what still needs a
real controller.
