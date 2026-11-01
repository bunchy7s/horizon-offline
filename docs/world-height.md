# World height: each level's shape from its dimension type

Vanilla takes each level's build range from its dimension type: `min_y` and `height`, multiples of 16,
with `height >= 16` and `min_y + height <= 2032` (`DimensionType.MIN_Y = -2032`, `Y_SIZE = 4064`: `BlockPos`
packs Y in 12 bits), plus `logical_height` (how high portals and chorus fruit reach) and `has_skylight`.
`LevelHeightAccessor` derives everything else from them. Data packs that reshape the Overworld make it taller:
JJThunder's To The Max sets `min_y -64, height 2096`, the largest top vanilla allows, which is 131 sections
against vanilla's 24.

## The model

**`chunk::LevelShape`** is one level's shape: `min_y`, `height`, `logical_height`, `sky_light`, checked as the
dimension type codec checks them (`LevelShape::new`). **`LevelShapes`** holds a world's three.

- **Where it comes from.** A local world's shapes are its dimension types, its enabled data packs' over
  vanilla's, through the level stems that name them (`worldgen::overrides::WorldgenOverrides::level_height`,
  read by `game/world_loading.rs`'s `level_shapes` when the world opens; the game keeps them in
  `Game.level_shapes`). A remote client takes a level's shape from the server's `minecraft:dimension_type`
  registry (`protocol::registry::DimensionType`, with its logical height). A type the codec refuses keeps the
  vanilla shape, with a warning.
- **Where it lives.** Each level's `World` holds its `shape`, `SaveDir::new(root, shape)` takes it, and the
  generator reads the same dimension type for the level's build range (`OverworldGenerator::level`).
- **The column.** `LevelShape::column()` is the span of sections the level's chunks hold (`ColumnLayout`):
  the level's own range, widened to the vanilla Overworld's `-64..320`. So vanilla's three levels keep the
  24 sections they have always had, and a taller Overworld gets its own column while its Nether and End stay
  at 24. Every `Chunk` records its own `min_y` and section count, and the hot accessors read the chunk.
- **No global.** The shape travels with the data (`World`, `SaveDir`, `Chunk`, the light and mesh jobs, the
  renderer's `clear_sections(layout)`, the LAN encoder's `set_dimension(shape)`, the remote `ChunkFormat`).
  Test binaries run tests in parallel in one process, so a process-wide height would leak between them.

| Subsystem | Reads |
| --- | --- |
| Chunk storage (`chunk.rs`) | the chunk's own `min_y` and sections |
| Light (`light.rs`) | the chunk's sections; the relighter the world's column (`LightWorld::layout`) |
| World, streaming, meshing, renderer | `World::layout()`, the chunk's `min_y`, the renderer's layout |
| Saves (`save.rs`) | `SaveDir.shape` (sections, `yPos`, heightmaps, light layers, far tiles) |
| Network (`lan/`, `game/remote_chunks.rs`) | the level's shape on the host, the server's dimension type on the client |
| Section caches, generation glue, far terrain, minimap, flat worlds | the chunk's column, the generator's level range |
| Gameplay (bounds, searches, commands, mobs) | `World::bottom_y`/`top_y` (the column), `World.shape`, `Game.level_shapes` |

Section sets are `SectionMask`s (256 bits: vanilla's tallest level has 254 sections, light data one more
below and above). Uniform sections (one block, one biome, one light value) are one shared allocation per kind
(`chunk::share_uniform`), and the sky-source scan passes over a uniform section light goes through: a tall
column is mostly such air.

## A world of data packs

The game, single player and LAN alike, runs through a host: the integrated server thread of single player
reads the world's shapes, encodes each level at its shape (`HubConfig.levels`) and sends the data of a
dimension type the packs changed even to a client that knows the vanilla pack (`NetConfig.levels`,
`SyncData::registry_packets`), and its client takes the heights from that registry data, as a vanilla client
of a vanilla server does.

## Checks

- Unit tests: a 131-section chunk stores, lights, relights, saves, loads, encodes and random-ticks its whole
  height; `tests/tall_column.rs` streams, lights and meshes a 131-section level to its top and remeshes after an
  edit at y 1800.
- `HORIZON_PARITY_DATAPACKS=tools/fixtures/tall_datapacks python3 tools/test_datapack_worldgen_parity.py`
  generates the fixture's 2096-high Overworld (`horizon_tall_test`, Horizon's own MIT pack, terrain from about
  y 475 to 1900) with the vanilla oracle, which takes each level's range from its dimension type, and with
  Horizon, and compares every block, biome and block entity.
- Vanilla worlds are unchanged at every step: `chunkgen_bench --snapshot`, `test_light_parity.py` and the save
  and LAN tests give the same results as before.

## What is left

- **Nether and End at their own height.** They keep the vanilla column (`-64..320`). Holding exactly their 16
  sections would save memory and match vanilla's `LevelChunk`, but Horizon's light only spreads inside a
  chunk's column while vanilla's spreads one section past the level, so it needs that border in the flood fill
  and the relighter first, checked with `test_light_parity.py`.
- The vanilla constants `MIN_Y`, `HEIGHT` and `SECTIONS` remain as the default column and in tests.
- Far terrain's detail for a camera far above the world uses vanilla's top (`lod.rs`).
- Looking down from far above the terrain shows fog: vanilla's render-distance fog uses `max(xz, |y|)`, so
  terrain more than the render distance below the camera is fogged in vanilla too.
