# Cosmetics

Small items Horizon players wear: a hat, something on the back, a pet on the shoulder. They are
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

- `slot`: `head`, `back` or `shoulder`; a player wears at most one item per slot.
- `bone`: the player model part the item follows (`head`, `body`, `left_arm`, `right_arm`,
  `left_leg`, `right_leg`). The item sits at the bone's matrix times `attach` (offset in pixels,
  rotation in degrees, a uniform scale in (0, 2]).
- `parts`: a tree of parts, each with a pivot, a rotation in degrees and boxes with Minecraft's box
  UV (`ModelPart.Cube`'s faces and order), grown by `inflate`, mirrored in x on request. Names are
  unique.
- `animations`: loops on top of a part's rest pose, a sine of `period` ticks (4 to 1200) started
  `phase` periods in. `swing` turns the part `amplitude` degrees (at most 45) about the axis, `bob`
  moves it `amplitude` pixels (at most 4), `spin` turns it all the way round once per period.

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

## In the game

- **The flag**: `horizon_cosmetics::ON_BY_DEFAULT` is `false`. Until it is turned on nothing runs:
  no thread, no request, no setting, no drawing. `HORIZON_COSMETICS=1` turns them on for one run
  (development, the bench); `HORIZON_COSMETICS_API=<url>` points the game at another service, and
  `HORIZON_COSMETICS_LOCAL=<folder>` reads items from a folder instead: every player and every
  mannequin wears its first item of each slot (the art and the bench, without the network).
- **Settings** (Extras, under "Cosmetics", shown only with the flag on): *My cosmetics* and *Other
  players' cosmetics*, both on. Off, the game asks for nothing about those players.
- **Drawing** (`game/cosmetics.rs`): an item's texture takes one of the skin atlas's 64×64 slots
  and its boxes are posed on the wearer's bone each frame, through the same GPU skin path as the
  player model (the CPU path where that is off), lit and culled with the player. A helmet hides a
  head item, an elytra or cape a back item. The third person view and the
  inventory's player preview show your own.
- **Cost**: a player with nothing to wear adds one cache lookup; with no cosmetic in view the frame
  does no cosmetics work beyond that. The bench scene `stress_avatars` (300 mannequins, 20×15)
  measures the cost with `HORIZON_COSMETICS=1 HORIZON_COSMETICS_LOCAL=…` against the same run off.
