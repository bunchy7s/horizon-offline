# Horizon's design system

One design language for the launcher, the website and the few overlays the game draws that
vanilla has no screen for (the on-screen keyboard a controller types with). Every game screen
and the HUD are not part of it: they are vanilla's, because parity is the point of the game, and
Horizon's own settings are pages built from vanilla's widgets, the way mods add theirs.

The tokens live in one place, `crates/horizon-theme`. The launcher (`launcher/src/theme.rs`)
and the game (`crates/horizon-client/src/ui_theme.rs`) read them as Rust constants, and the
website reads them as CSS custom properties generated from the same crate
(`cargo run -p xtask -- tokens` writes `crates/horizon-theme/tokens.css`, which the website's build
reads from main; a test fails when the file is out of date). Change a value in the crate and all three follow.

## Where it comes from

The direction was set by [mkdir.directory](https://mkdir.directory/), a directory site that is
almost nothing but type. What we take from it is its restraint rather than its content:

1. **Type is the interface.** One family, Geist, and its mono. Large, tight display type
   (medium weight, -2 % tracking) carries the hierarchy, so there is no need for boxes, icons or
   colour to say what matters. Instrument Serif is gone.
2. **Lists, not cards.** Content is a numbered list: a small mono index on the left (`01`,
   `02`…), the name large, a mono detail on the right, and a 1 px rule between rows. Versions,
   settings, worlds, servers, release notes and the parity table are all this one component.
3. **One accent, used once.** Neutrals do the work. The violet marks the single main action on a
   screen (Play, Download), focus, links and progress; selection is shown in ink, not in colour.
4. **Space instead of borders.** Generous margins (64 px on wide screens, 16 px on phones), no
   shadows, no gradients, no outlines around groups. A surface tint appears only under the
   pointer or behind something that must stand apart (a console, a dialog).
5. **Quiet motion.** Things arrive with a short fade and an 8 px rise, a few tens of
   milliseconds apart, eased out, never bouncy. Hovers fade a soft plate in behind the row.
6. **A signature.** A giant, faint wordmark: at the foot of the website, behind the launcher's
   opening and the game's title screen.

## Tokens

### Colour

Cool, slightly blue neutrals in two appearances with the same restraint, and Horizon's violet.
Every text colour passes WCAG AA (4.5:1) on every background it is used on; the tests in
`horizon-theme` check each pair in both palettes.

| Token        | Light     | Dark      | Use |
|--------------|-----------|-----------|-----|
| `page`       | `#fafafc` | `#0d0e13` | the background |
| `plate_soft` | `#f3f3f8` | `#181a22` | hover plate, fields, segmented track, console |
| `plate`      | `#ebecf4` | `#20222c` | pressed plate, switch off, the faint wordmark |
| `surface`    | `#ffffff` | `#16181f` | dialogs and menus that sit above the page |
| `rule`       | `#e2e3ea` | `#24262f` | 1 px rules between rows, tracks |
| `ink`        | `#16181f` | `#ecedf3` | text, selection, slider fill, the inverted segment |
| `ink_soft`   | `#3b3e49` | `#c5c7d1` | secondary text, values |
| `muted`      | `#646875` | `#8b8f9c` | labels, descriptions, mono indices and details |
| `accent`     | `#5b3fd6` | `#9a86ff` | the main action, links, focus, progress |
| `on_accent`  | `#ffffff` | `#0d0e13` | text on the accent |
| `danger`     | `#b4372a` | `#f07a68` | errors, the crash notice's dot |
| `success`    | `#1d7a48` | `#45c27a` | "Matches vanilla", "Ready" |

The game's menus draw over the live title backdrop, always in the dark palette, with the page
colour as a flat veil so text keeps its contrast whatever the world shows: `VEIL` (0.90) on
every page and in a world, `HOME_VEIL` (0.80) on the title's home page, which shows more of the
world and keeps to `ink` and `ink_soft`. Muted text over the backdrop is `BACKDROP_MUTED`
(`#9fa3ae`). A test holds every pair to AA over a backdrop of luminance 0.77 (the snowiest title
panorama measured reaches 0.49), on the bare veil and on a selected row's plate. Their plates
are the dark `ink` at 8 % and 14 % rather than `plate_soft` and `plate`: over the veil they come
out the same, and they stay visible over a bright backdrop.

### Type

Geist (400, 500, 600) and Geist Mono (400), bundled under the SIL Open Font License.

| Token      | App px | Web (min–max px) | Line | Tracking | Weight | Use |
|------------|--------|------------------|------|----------|--------|-----|
| `micro`    | 11     | 11               | 1.5  | +0.04em  | mono   | indices (`01`), counts, timestamps |
| `caption`  | 12     | 12               | 1.5  | 0        | 400    | footnotes, help under a control |
| `label`    | 13     | 13               | 1.5  | 0        | 400    | row labels, nav, buttons |
| `ui`       | 14     | 14               | 1.5  | 0        | 400    | body text in the apps |
| `body`     | 15     | 15–17            | 1.6  | 0        | 400    | paragraphs |
| `heading`  | 20     | 20–24            | 1.3  | -0.01em  | 500    | section heads |
| `title`    | 28     | 32–48            | 1.05 | -0.02em  | 500    | list item names |
| `display`  | 44     | 34–64            | 1.05 | -0.025em | 500    | page titles |
| `wordmark` | 220    | 18vw             | 0.8  | -0.04em  | 500    | the faint signature |

### Space, shape and lines

- Spacing scale: 4, 8, 12, 16, 24, 32, 48, 64, 96. Page gutter 64 (apps and wide web), 16 on
  phones. Text measure at most 56 characters for paragraphs.
- Radii: 8 for fields and small controls, 12 for plates (hover rows, console, dialogs), full for
  buttons, segmented controls and switches.
- Lines: 1 px, `rule`. No other borders.
- Heights: control 32, large control (Play) 40, segment 28, list row 64 (apps), 86 (web).

### Motion

| Token   | Duration | Easing                          | Use |
|---------|----------|---------------------------------|-----|
| `quick` | 150 ms   | `cubic-bezier(.4, 0, .2, 1)`    | hovers, presses, colour |
| `move`  | 240 ms   | `cubic-bezier(.23, 1, .32, 1)`  | selection sliding, switches, page changes |
| `enter` | 600 ms   | `cubic-bezier(.23, 1, .32, 1)`  | content arriving: opacity 0→1, rise 8 px |
| `stagger` | 40 ms  |                                 | delay between list rows arriving |

`prefers-reduced-motion` (web) turns `enter` into a plain fade.

## Components

- **Index list** (the signature): `micro` mono index in `muted` (left, 40 px column), the name
  in `title` (or `ui`/`label` for dense lists such as settings), details in `micro` mono
  `muted` on the right, a `rule` under every row. Hover: a `plate_soft` plate with radius 12
  fades in, extending 16 px past the row on each side, and the index turns `ink`.
- **Buttons**: pill shaped, `label` size, medium weight, 32 high (40 for the one large action).
  *Primary* is accent with `on_accent` text, at most one per screen. *Secondary* is
  `plate_soft` with `ink` text. *Ghost* is `muted` text that turns `ink` with a `plate_soft`
  plate under the pointer. Pressed: `plate` (or the accent at 85 %). Disabled: 40 % opacity.
- **Fields**: 36 high, `plate_soft`, radius 8, no border; focus draws a 1 px accent ring.
  Monospace for paths and commands.
- **Switch**: 36×20 pill, `plate` when off, accent when on, a white 16 px knob that slides over
  `move`.
- **Slider**: a 2 px `rule` track, the filled part `ink`, a 14 px `ink` knob. The value is
  written in mono beside the label, not on the track.
- **Segmented control / tabs**: a `plate_soft` pill; the selected segment is an `ink` pill with
  `page` text that slides over `move`.
- **Navigation**: text only. Launcher: a top bar with the mark and "Horizon" on the left and
  the pages on the right, `label` size, `muted`, the current one `ink` on a soft pill. Game
  menus keep their sidebar layout (it suits a controller), but as a plain column of text items
  under the mark, with no panel tint and no border: the current one `ink` on a soft pill that
  slides to it.
- **Page header**: `display` title, then one `body` line in `muted`, 56 characters at most.
- **Notices and dialogs**: a `plate_soft` plate, radius 12, 16–24 padding, no border; a 6 px dot
  in `danger` (or `accent`) leads the title. Actions are buttons in a row below.
- **Progress**: a 2 px `rule` track and an `accent` fill; indeterminate work shuttles a 30 %
  segment.
- **Console**: `plate_soft` plate, radius 12, `micro`/`caption` mono in `ink_soft`, newest line
  at the bottom, the command field inside the plate under a `rule`.
- **Empty states**: a short `muted` sentence and one secondary button, centred in the space.

## Surfaces

- **Launcher** (`launcher/`): moving to the Night look below, screen by screen; screens not rebuilt
  yet draw the Blocks parts in the night's colours (`Night::blocks`). A top bar (wordmark, the pages as
  tabs, the account chip, Settings), a sidebar (the instances, the hosted servers, the launcher's
  own version and channel at its foot), and the page over a stone texture. Home opens on a pixel
  landscape banner with the hero, the version picker and Play; Mods holds the mod, resource pack
  and modpack tabs; Versions and Patch notes are lists of panels; Settings is
  grouped panels of rows. Dark by default, Light or System in Settings (Appearance).
- **Website** (horizon-rust/horizon-site): the Blocks look below. The same page structure as before (what it
  is, screenshots, parity, launcher, patch notes, community), headings in Pixelify Sans, lists as
  stacks of panels, the stone texture behind the page and Horizon's pixel landscape (a dawn or a
  sunset) behind the hero. Light and dark follow the system, with a toggle. A giant wordmark in
  pixels sits in the footer.
- **Game**: vanilla's screens. The on-screen keyboard is a rounded plate of keys in the page
  colour, the focused key in the accent.

## Blocks

A blocky, Minecraft-like look in Horizon's purple, for the website and the redesigned launcher:
Minecraft's inventory rather than a document. Its colours are `horizon_theme::Blocks`
(`BLOCKS_LIGHT`, `BLOCKS_DARK`), which `tokens.css` carries under the same names as before plus
`--panel`, `--raised`, `--button`, `--well`, `--bevel`, `--lip`, `--outline` and the `--face`
of the main action. The game's menus keep `LIGHT` and `DARK`.

- **Pixel type for what names and what you act on.** Headings, navigation, buttons and the
  wordmark use Pixelify Sans (OFL; Medium, SemiBold, Bold); paragraphs and numbers stay in Geist
  and Geist Mono so long text reads easily. The bundled copies add the three uppercase Cyrillic
  letters upstream leaves out, О, П and І: the Latin O's glyph, the lowercase п lifted to that
  weight's cap height, and the Latin I's glyph, so a Russian or Ukrainian heading stays in the
  pixel face.
- **Blocks, not rounded cards.** Every surface is square: a 2 px dark outline, a 2 px light bevel
  along the top and a darker lip along the bottom. The main action is the violet face with its own
  bevel and a 4 px lip; it sinks a pixel when pressed. Code and fields are sunk wells.
- **Inventory tabs** switch the kinds in Mods: the open tab joins the panel under it.
- **Slots** hold pictures: a mod's or pack's own icon and gallery images from its source,
  downloaded when first shown and kept on disk (`launcher/src/browser/images.rs`); the launcher
  ships no third-party pictures. The Horizon instance shows a grass block drawn from the player's
  own prepared Minecraft textures. Sources are named in text ("from Modrinth"): Modrinth's logo is
  all rights reserved, so it is not shown.
- **Partner banner**: the one exception to shipping no third-party pictures. Sculk, a server host,
  sponsors Horizon: its banner (`launcher/assets/sculk-banner.webp`, `launcher/src/partner.rs`) sits
  under a muted mono "Partner" label at the end of the Servers page and opens Sculk's page in the
  browser. The sidebar of the other pages shows a card made for its width instead: Sculk's characters
  and logo cropped from the same banner, and the text drawn by the launcher in its language, so it
  stays readable; it keeps Sculk's dark card, cyan and rounded corners in both themes, as an ad does,
  and turns compact (characters beside the text) when the window is too short. After three minutes
  on screen, over every start and on any page, a "Hide" appears beside the label of the card and of
  the Servers banner alike; either one hides both for good (`Settings::partner_hidden`). The website shows the
  banner the same way on Home and the hosting docs, as a `rel="sponsored"` link.
- **Pixel art drawn in code.** The stone behind the pages is close shades of the page colour
  (`TILE_SHADES`), never Minecraft's stone; the landscape is a banded sky, a sun cut in slices like
  the mark, hills and square trees (a sunset in dark, a dawn in light). The launcher bakes them in
  `launcher/src/art.rs` at the window's scale so each texel covers whole device pixels; the website
  draws them in its `scripts/art.mjs`.
- **Icons**: Pixelarticons (MIT), 24 unit pixel SVGs in `crates/horizon-theme/icons/pixel/`, tinted
  with the text colour.
- **Contrast**: every text colour meets WCAG AA (4.5:1) on the panels, the page and the lightest
  stone shade in both palettes (`blocks_text_meets_wcag_aa_in_both_appearances`).

## Night

The launcher's look from 0.3: a pixel night sky that turns to a sunset at the horizon, with the
content on top. Its tokens are `horizon_theme::night` (`NIGHT`, and `DAWN` for players who pick
Light); `tokens.css` carries them as `--night-*`. The game's menus don't use it.

- **The sky is the brand, the content is the UI.** The sky, the stepped sun, the banded clouds and
  the skyline are pixel art drawn in code on one grid per layer, edges crisp, never smoothed. On
  top of it the screen is made of what the player owns: instance icons, world thumbnails, server
  icons, their skin's head, mods and counts. No ornaments, no empty cards.
- **Plates only where text needs them.** A plate is `card` at its opacity with a 1 px `stroke`;
  `card_hi` for a selected or raised one. Text sits on the bare sky only over its upper stops
  (`UNCARDED_STOPS`): the sunset is too bright for light text. A plate cuts each corner in two 4 px
  steps (`PIXEL_CORNER`), like the art, and its stroke follows the steps. The main action cuts one
  step; other controls round at 8 (at 1x a step vanishes on a pale fill and looks notched on a thin
  ring: compared in the specimen).
- **One accent.** `accent` fills the one main action (Play, Create, Install) and progress;
  `accent_hi` is the accent as text and the 2 px focus ring. Everything else is ink on plates.
- **Bricolage Grotesque** (SIL OFL 1.1, `fonts/OFL-BricolageGrotesque.txt`), cut from upstream's
  variable font by `tools/bake_bricolage.py`: the UI family at optical size 14 (Regular, Medium,
  SemiBold, Bold) and `Bricolage Grotesque Display` (Bold at optical size 72) for instance and page
  names. Geist Mono stays for paths, codes and counts. Upstream has no Cyrillic, so the bake merges
  Geist's (OFL, `fonts/OFL-Geist.txt`) into every cut: the letters drawn like Latin ones reuse
  Bricolage's, the others are Geist's scaled to Bricolage's cap and x-heights at a matching weight.
- **Two or three sizes per screen**, from `night::TYPE`:

| Token           | App px | Line | Tracking (web) | Weight | Use |
|-----------------|--------|------|----------------|--------|-----|
| `night-display` | 56     | 1.0  | -0.035em       | 700    | instance and page names (display cut) |
| `night-title`   | 20     | 1.25 | -0.01em        | 700    | section and card heads |
| `night-ui`      | 14     | 1.45 | -0.005em       | 500    | text, rows, buttons, navigation |
| `night-caption` | 12     | 1.4  | 0              | 400    | one-line states and details |

- **Colour** (night / dawn). Every text colour reaches AA on both plates over every stop of the sky,
  and on the upper stops, the page and the sidebar (`night_text_meets_wcag_aa_over_the_sky`).

| Token       | Night     | Dawn      | Use |
|-------------|-----------|-----------|-----|
| `sky-0…5`   | `#07061a` `#140f38` `#2a1656` `#5b2a78` `#b0477d` `#f08a5d` | `#c9d3f2` `#dcd8f3` `#ead5ec` `#f5d3d8` `#f9d6bd` `#fbe4c6` | the sky, top to horizon |
| `card`      | `#15112e` at 84 % | `#fdfcff` at 86 % | plates |
| `card-hi`   | `#231c4a` at 90 % | `#ffffff` at 94 % | a selected or raised plate |
| `stroke`, `rule` | `#b9acff` at 12 %, 10 % | `#3b2f78` at 12 %, 10 % | a plate's inner stroke, rules between rows |
| `ink`       | `#f4f2ff` | `#1b1530` | text |
| `ink-soft`  | `#cfc9ea` | `#3b3354` | secondary text |
| `muted`     | `#a59fc6` | `#585071` | labels, details |
| `accent`    | `#7656fa` | `#5b3fd6` | the main action, progress |
| `accent-hi` | `#a08cff` | `#5333c9` | accent text, focus ring |
| `success`, `warn`, `danger` | `#5fd39a` `#f5b45a` `#ff8a78` | `#136336` `#7a4f00` `#a3301d` | Ready, updates, errors |

- **Motion that means something**: the sky drifts slowly and stops when the window is hidden or
  unfocused; the interface moves only to show cause and effect (a row arrives where it was added,
  progress fills). Reduced motion: nothing moves. The launcher's sky (`launcher/src/sky.rs`) moves
  in whole texels on a half-second beat, so it asks for two frames a second at most;
  `HORIZON_LAUNCHER_MOTION=off` stills it, `always` keeps it moving unfocused (for recordings).
- **Copy says what happens** ("Downloads 26.3 and Java 21, about 600 MB"), never praises.
- The specimen (`HORIZON_LAUNCHER_PAGE=specimen horizon-launcher`) draws the type and colours in the
  launcher itself.
- **The frame** (`launcher/src/shell.rs`): the sidebar is one plate from the window's top to its
  bottom (the wordmark, instances by group, servers, the launcher's version); the top bar over the
  pages has no plate, only a rule. A selected row or tab sits on `card_hi` cut in steps (two for
  rows, one for tabs and icon buttons); a hovered one gets the same shape at half its opacity,
  eased in. A row's second line is its state, after a small square in the state's colour.
- **Home** (`launcher/src/home.rs`): the selected instance's name in the display cut over the bare
  sky, a caption above it naming what it plays and its state (the same square), and when it was
  played below. Play is the main action (`night::PLAY` tall, cut in one step); the version picker
  is a raised control of the same height; Mods and Folder are quiet buttons. A Horizon instance
  gets a plate on the right: a release's notes in one line (not a Nightly's) and Join a friend.
  While it gets ready, Play names it ("Getting ready… 34%") beside Cancel, and below sit the
  bar, what is happening with the time left, and the steps (game files, Java, sounds, prepare,
  play), the current one lit, those done ticked: steps, never the log
  (`HORIZON_LAUNCHER_PAGE=getting-ready` shows it for screenshots, nothing runs).
- **Instance icons** (`launcher/src/instance_icons.rs`): the sidebar's rows, Home's hero (beside
  the name) and the instance dialog show the same icon. Its own picture first (`icon.png` in its
  game folder: one the player chose with Choose image…, scaled to 128 pixels, or its modpack's
  icon from where it was installed), else the block or colour chosen in the dialog, else its
  mark: the loader's logo (Fabric, Quilt, Forge, NeoForge), the grass block for plain Minecraft,
  Horizon's for Horizon. The letter tile is the last resort. The New instance dialog's loader
  cards show the same logos. Pixel-art logos are scaled by whole
  pixels; each is decoded once per size.
- **The instance's tabs** (`launcher/src/home_tabs.rs`): under Home's hero, one tab at a time
  instead of one long page: Overview (what Play needs: the instance, the game or its resources,
  the account), Worlds, Multiplayer, Packs, then Screenshots and Console once they have something.
  A tab names how many it holds ("Worlds 3"); its own buttons (Add server, Open folder) sit at the
  strip's end, where a list's header would put them (`HORIZON_LAUNCHER_PAGE=worlds`, `packs`...
  opens one for screenshots).
- **The Mods tab** (`launcher/src/instance_mods.rs`): the instance's mods on one nearly opaque
  card, its loader in the header ("Fabric mods", "Horizon mods") beside a filter. A row is the
  mod's own icon (Modrinth's, else its jar's, else a glyph), its name and version, where it comes from (Modrinth,
  "needed by another mod", or a local file by name), an update as an amber pill when the browser
  found one, and a switch (a Java jar turned off is renamed `.disabled`). Update all and Add mods
  sit at the strip's end (`HORIZON_LAUNCHER_PAGE=instance-mods`).
- **Sign in with Microsoft** (`launcher/src/microsoft.rs`): every button that signs in is
  Microsoft's own, drawn as Microsoft publishes it: 41 pixels tall, its four-square symbol, the text
  semibold at 15 pixels, `#2F2F2F` with white text at night, white with `#5E5E5E` text and a
  `#8C8C8C` border at dawn, square corners. Not one of the launcher's cut plates: Microsoft's
  guidelines ask for the button as it is.
- **The sign-in sheet** (`launcher/src/owner_page.rs`, the owner gate): an opaque plate over the
  dimmed launcher, never instead of it. It says why (the account that owns Java Edition, no install
  needed), the code in mono on a raised box with Copy, and Open link as the main action; the window
  goes on by itself once Microsoft answers (`HORIZON_LAUNCHER_PAGE=sign-in` shows it with a made-up
  code for screenshots).
- **New instance** (`launcher/src/new_instance.rs`): an opaque plate over the dimmed launcher,
  tabs for the kind (Horizon, Vanilla, Modded). A Java kind lists Mojang's versions (search,
  Snapshots, Old; the newest release chosen), Modded adds the loader (Fabric or Quilt, at its newest
  stable version), and the name follows the choice ("Fabric 1.21.4") until the player types one.
  Create is the main action.

## The mark

**Horizon line.** A sun half set below a horizon, drawn as three horizontal bars of 4, 8 and
10 units over a 12 unit line, on a 16 unit grid. The bars are the sun cut by the horizon into
slices, which also reads as layers of terrain and as a nod to voxels without drawing a block. It
is built on whole units of the 16 px grid so it stays sharp at 16 px, and it scales without
detail to 1024 px.

- Monochrome: the mark in `ink` on light, in `ink` (dark) on dark; the line can take the accent.
- App icon: the mark in `#ecedf3` with the line in `#9a86ff`, on a `#16181f` rounded square
  (macOS grid: 824 px tile with 185 px corners in a 1024 canvas; Windows and Linux use the full
  square with 22 % corners).
- Exports and where they go: `docs/media/brand/README.md`.
