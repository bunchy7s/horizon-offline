# Cosmetics

Small items Horizon players wear: a hat, something on the back, a pet on the shoulder, a style for
their name tag. They are
drawn by Horizon clients only and change nothing in the game. **Off by default**: nothing below
runs until the feature flag is turned on.

## Rules

- **Only Horizon clients see them.** Other Horizon players learn what someone wears from Horizon's
  cosmetics service, keyed by the Minecraft UUID. The game protocol doesn't change: servers, vanilla
  clients and anticheats receive nothing.
- **No gameplay effect of any kind**, no capes or anything that looks or acts like one, no random
  draws.
- **A setting hides them**: other players' cosmetics, and your own.
- **Zero cost when none is in view**, and a vertex budget per item.

## The item format (`crates/horizon-cosmetics`)

One JSON file per item. Coordinates are model pixels (1/16 block) in the player model's space:
y points down, as in vanilla's `ModelPart`s and Blockbench's "Modded Entity" models.

```json
{
  "format": 1,
  "id": "example_hat",
  "slot": "head",
  "bone": "head",
  "attach": { "offset": [0, -8, 0], "rotation": [0, 0, 0], "scale": 1 },
  "texture": { "png": "<base64 PNG>" },
  "parts": [
    { "name": "brim", "pivot": [0, 0, 0], "rotation": [0, 0, 0],
      "cubes": [{ "origin": [-4, -2, -4], "size": [8, 2, 8], "uv": [0, 0], "inflate": 0, "mirror": false }],
      "children": [] }
  ],
  "animations": [
    { "part": "brim", "motion": "swing", "axis": "z", "amplitude": 10, "period": 40, "phase": 0 }
  ]
}
```

- `slot`: `head`, `back`, `shoulder` or `name_tag`; a player wears at most one item per slot.
- `bone`: the player model part the item follows (`head`, `body`, `left_arm`, `right_arm`,
  `left_leg`, `right_leg`; `head` if left out). The item sits at the bone's matrix times `attach` (offset in pixels,
  rotation in degrees, a uniform scale in (0, 2]).
- `parts`: a tree of parts, each with a pivot, a rotation in degrees and boxes with Minecraft's box
  UV (`ModelPart.Cube`'s faces and order), grown by `inflate`, mirrored in x on request. Names are
  unique.
- `animations`: loops on top of a part's rest pose, a sine of `period` ticks (4 to 1200) started
  `phase` periods in. `swing` turns the part `amplitude` degrees (at most 45) about the axis, `bob`
  moves it `amplitude` pixels (at most 4), `spin` turns it all the way round once per period.

A **name tag style** (`"slot": "name_tag"`) has no model: its texture is a square mark of at most
16×16 drawn before the player's name, and `"tag": {"line": "#rrggbb"}` the colour of a line under
it. `parts` stays empty and there are no animations; no other slot has a `tag`.

```json
{ "format": 1, "id": "example_tag", "slot": "name_tag", "texture": { "png": "<base64 8×8 PNG>" },
  "parts": [], "tag": { "line": "#a98fff" } }
```

Limits, checked when an item is read (`Item::parse`); an item past one is refused, never drawn:

| Limit | Value | Why |
|---|---|---|
| Vertices | 512 (21 boxes) | the player's own model and outer layer are 288 |
| Texture | 64×64 at most | it fits one of the atlas's skin slots |
| Parts | 16 | |
| Reach | 24 pixels from the bone's pivot, animations included | items stay near their wearer |

Unknown fields are refused, so a file never carries anything the game would silently ignore.

## What other players wear (`horizon_cosmetics::Cosmetics`)

The game asks Horizon's cosmetics service (HTTPS, JSON) what the players in view wear, and draws
from its answers. The frame never waits on the network: it reads a cache, and a thread of its own
does the asking.

- **One request for the players in view**: `POST /v1/worn` with up to 64 UUIDs
  (`{"players": ["<uuid hex>", …]}`) answers `{"ttl": seconds, "players": {"<uuid hex>": [{"id",
  "hash"}, …]}}`; a player who wears nothing is left out. Only the UUIDs of players the game draws
  are sent, nothing else about them or the server.
- **Cached for the service's TTL** (kept within 1 minute to 1 hour). Past it the last answer stays
  drawn while the player is asked for again. The 4096 players seen most recently are remembered.
- **Item files** come from `GET /v1/items/<id>?hash=<hash>` once and are kept on disk under their
  hash (they never change for one); a file past the limits is left out and not asked for again.
- **Offline**: a request that fails leaves every answer as it was, and the next one waits 15 s, then
  30 s, doubling up to 10 minutes. A player whose item didn't arrive is asked for again after 15 s.
- **Across restarts**: the last answers are kept beside the item files (`worn.json`: each player's item
  ids and hashes, written only when an answer changes them). They are read only when the service doesn't
  answer, so a game started while it is down still shows what players last wore, until it answers.

## In the game

- **The flag**: `horizon_cosmetics::ON_BY_DEFAULT` is `false`. Until it is turned on nothing runs:
  no thread, no request, no setting, no drawing. `HORIZON_COSMETICS=1` turns them on for one run
  (development, the bench); `HORIZON_COSMETICS_API=<url>` points the game at another service, and
  `HORIZON_COSMETICS_LOCAL=<folder>` reads items from a folder instead: every player and every
  mannequin wears its first item of each slot (the art and the bench, without the network).
- **Settings** (Extras, under "Cosmetics", shown only with the flag on), the same three as the
  launcher's Wardrobe (`horizon_cosmetics::prefs`, lines of `horizon-extras.txt`):
  - *My cosmetics: Shown / Hidden* is the account's (`PUT /v1/visible`): hidden, every Horizon
    player sees you without them, you included. The game signs in for that one call the way the
    launcher does (`horizon_cosmetics::account`, Mojang's `join` with the player's session) and
    notes what the service confirmed in `cosmetics-visible.txt`; offline it tries again after 15 s,
    doubling to 10 minutes, until the service takes it. An unchanged switch sends nothing.
  - *Other players' cosmetics: Shown → Name tags only → Hidden*. Hidden, the game asks for nothing
    about them; name tags only, their name tag styles show and nothing on their bodies.
  - *Others' auras and trails*, off by default (no item of that kind exists yet).
- **Name tag styles** (`game/names.rs`): the mark, 8 text pixels square, before the name and a
  one-pixel line in the style's colour under the name's background, in the name tag's own pass
  (the mark is in the atlas like the other items' textures).
- **Drawing** (`game/cosmetics.rs`): an item's texture takes one of the skin atlas's 64×64 slots
  and its boxes are posed on the wearer's bone each frame, through the same GPU skin path as the
  player model (the CPU path where that is off), lit and culled with the player. A helmet hides a
  head item, an elytra or cape a back item. The third person view and the
  inventory's player preview show your own.
- **Cost**: a player with nothing to wear adds one cache lookup; with no cosmetic in view the frame
  does no cosmetics work beyond that. The bench scene `stress_avatars` (300 mannequins, 20×15)
  measures the cost with `HORIZON_COSMETICS=1 HORIZON_COSMETICS_LOCAL=…` against the same run off.

## A still picture (`horizon_cosmetics::preview`)

The launcher shows items on the player's own skin without the game: `preview::render` draws the
player model at rest (vanilla's `PlayerModel` boxes on a 64×64 skin, wide or slim arms, the outer
layers included) and each item on its bone on the CPU, with a depth buffer and cutout alpha, seen
from any angle.

## The Wardrobe's account check (`launcher/src/wardrobe.rs`)

The launcher talks to the same service for the player's own items: the catalogue, what the account
owns, claiming the founder set while its window is open, what each slot wears and whether other
players see it.

- **Who the player is**: the service hands out a one-time server id (`POST /v1/auth/start`), the
  launcher tells Mojang's session server that the account joins it
  (`account::session::join_server`, the call a client makes before joining an online-mode server),
  and the service asks Mojang (`hasJoined`) before it answers `POST /v1/auth/finish` with a
  session of its own. The Minecraft token goes to Mojang only; the service never sees it.
- **Then**: `GET /v1/me`, `POST /v1/claim`, `PUT /v1/equipped {"slot", "item"}` and
  `PUT /v1/visible {"visible"}` ("Show my cosmetics": off, `/v1/worn` leaves the player out) with
  that session (`Authorization: Bearer`), kept in memory for the time the service gave, less a
  minute.
- **The founder set** is free and can be claimed only between the window's start and end; after
  that it is retired for good and never offered again. Players who claimed it keep it.
- **Launcher themes** (`"slot": "theme"`) have no item file: the catalogue carries their six sky
  stops and accent hue, and the game never asks for them.
- **Offline**: the last catalogue and answer, and the changes not sent yet, are kept per account
  in the launcher's data folder (`wardrobe/<uuid>.json`; never a session or token), item files
  under their hash. A change shows at once; the ones the service couldn't take (offline, or a
  failure on its side) are sent in order when it answers again, and a newer change of the same
  slot replaces a waiting one.

## The Wardrobe page (`launcher/src/wardrobe_page.rs`)

A top-bar page between Servers and Versions, shown only with the flag on, for the selected Microsoft
account (signed out, it asks you to sign in):

- **A row per slot** (head, back, shoulder pet, name tag, launcher theme) with what it wears,
  "Nothing equipped" or "None owned yet"; **your player** in the middle, on your own skin, wearing
  it, with your name tag style over the head (`horizon_cosmetics::preview`); **what you own for the
  chosen slot**, "Nothing" first. Equipping shows at once.
- **The founder set**: a line to claim it while its window is open, and once the window is closed,
  that it was free until that day and isn't offered any more.
- **Three switches**: *Show my cosmetics* (the account's, on the service: off, everyone sees you
  without them), *Hide other players' cosmetics* and *Show other players' auras and trails* (off by
  default). The last two are lines of the game's `horizon-extras.txt` (`horizon_cosmetics::prefs`:
  `cosmeticsOthers` = `shown`, `name_tags` or `hidden`, `cosmeticsAuras`), the same ones the
  game's settings change.
- **States**: offline, the page keeps working from what's kept on disk and says how many changes
  wait to sync; a service failure says what you wear still shows in the game, with *Try again* and
  *Copy details* (host, status, time).
- **A launcher theme** worn (First Light) replaces the sky's six stops and the accent's hue
  (`Saved::theme`, `Theme::with_sky`), from the outfit kept on disk, so it holds offline too. Taking
  it off brings the player's own look back.
- `HORIZON_LAUNCHER_PAGE=wardrobe` in a development build shows the page for a made-up account
  wearing `HORIZON_COSMETICS_LOCAL`'s items; `HORIZON_WARDROBE_STATE=offline|failed|closed|empty`
  shows that state, `theme` wears First Light.
