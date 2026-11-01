# Pregeneration: `/pregen`

`/pregen` generates every chunk of a square or a circle ahead of the players, lights it and saves it, as
Chunky does on a Java server. It is Horizon's own command (vanilla has none), available in single player
with cheats on and to operators (level 2) on a server, from the chat or the server console
(`horizon-server cmd pregen start 2500`).

It is off until you turn it on, so `/help` and the command tree stay vanilla's:

- a server: `horizon-pregen = true` under `[properties]` in `server.toml` (or `horizon-pregen=true` in a
  vanilla folder's `server.properties`), then restart it;
- single player and LAN: start the game with `--pregen`.

```text
/pregen start <radius> [square|circle] [<x> <z>]   radius in blocks; the centre defaults to where you
                                                   stand (the world spawn from the console)
/pregen status                                     chunks done, chunks per second, time left
/pregen pause | continue                           stop scheduling new chunks, and go on
/pregen cancel                                     stop; what is done stays saved
```

The area is the chunks within `radius / 16` chunks of the centre chunk: a square of side
`2 * (radius / 16) + 1` (radius 2500 is 313 x 313 = 97,969 chunks), or the chunks whose distance from the
centre chunk is at most `radius / 16`. It runs in the dimension the command was run in; that level keeps
ticking while the task runs, whoever is in it. The progress goes to the server log every five seconds
and to the player who started it, and once more when it is done. A task is not kept across a restart:
start it again and the chunks already saved load instead of generating.

On a server with `max_view_distance` set ([hosting](hosting.md#seeing-further-max_view_distance)), the
players see the pregenerated chunks past `view_distance` as well: that ring only sends saved chunks.

## How it works

- `horizon_client::pregen::Pregen` cuts the area into strips at most 256 chunks wide and sweeps each
  north to south with a band of rows (`streaming::Band`) holding about a thousand chunks.
- Streaming treats the band like a player's area: it generates its chunks and the rings around them that
  decoration and light need, and lights its own chunks. Once every chunk of the band's first row is lit
  and post-processed (`LevelChunk.postProcessGeneration`, which vanilla runs before a chunk is FULL), the
  band moves down a row; the rows that fall out of its reach unload and are saved like any chunk the
  players leave, so memory holds only the rows in flight.
- The chunks a band reaches are ranked for decoration row by row as they come within its reach
  (`Streaming::set_band`). Decoration is order dependent where neighbours' writes overlap
  (`DecorationOrder`), and this makes a task decorate in the same order however its jobs are timed and
  shared among the workers: the same seed gives the same chunks.
- Between server ticks the server waits for the workers' results and hands out the next jobs at once
  (`Streaming::wait`, `pump_chunks`), instead of once a tick: a chunk passes through five jobs
  (generate, decorate, finish, light, save), each needing its neighbours' earlier ones.

The chunks of the band's outer ring are finished (FULL) but not lit, as at the edge of what a player has
loaded; they light when their neighbours load later.
