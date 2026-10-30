# For server owners: `server.properties` to `server.toml`

You run a vanilla server today and edit `server.properties`. With `horizon-server` you edit one file,
`server.toml`, instead. Before every start the server writes `server.properties` from it, so the game reads
exactly what a vanilla server reads. This page maps one to the other. The full guide is
[hosting.md](hosting.md); the design is [server.md](server.md), section 4.

## Moving over

Run `horizon-server init` in your server folder, or `horizon-server import server.properties`. It reads the
file with vanilla's own parser:

- keys that have a field below go into their section of `server.toml`;
- any other key that differs from vanilla's default goes into `[properties]`;
- keys left at their defaults are dropped, so the file stays short;
- your original is kept as `server.properties.imported`.

The world folder, `ops.json`, `whitelist.json`, the ban lists and `usercache.json` keep vanilla's names and
formats, so going back to a vanilla server works too.

## Keys that have a field

| `server.properties` | `server.toml` | Default |
|---|---|---|
| `motd` | `[server] motd` | `"A Minecraft Server"` |
| `server-port` | `[server] port` | `25565` |
| `server-ip` | `[server] bind` | `""` (every address) |
| `max-players` | `[server] max_players` | `20` |
| `online-mode` | `[server] online_mode` | `true` |
| `white-list` | `[server] whitelist` | `true` |
| `enforce-whitelist` | `[server] enforce_whitelist` | `false` |
| `view-distance` | `[server] view_distance` | `10` |
| `simulation-distance` | `[server] simulation_distance` | `10` |
| `level-name` | `[world] name` | `"world"` |
| `level-seed` | `[world] seed` | `""` (random on first start) |
| `gamemode` | `[world] game_mode` | `"survival"` |
| `difficulty` | `[world] difficulty` | `"easy"` |
| `hardcore` | `[world] hardcore` | `false` |

Two details differ from vanilla's file: `white-list` becomes `whitelist`, and a seed may be a number or any
text (vanilla hashes text), written as a string.

## Every other key

Put any other vanilla key under `[properties]`, spelled as vanilla spells it:

```toml
[properties]
spawn-protection = 16
resource-pack = "https://example.com/pack.zip"
```

A key that has a field above is an error under `[properties]`; set it in its section.

## Settings that are not in `server.properties`

These belong to `horizon-server`, not to the game:

| `server.toml` | Meaning |
|---|---|
| `[engine] path`, `resources`, `restart_on_crash` | Which game program and prepared files to run, and whether to restart it after a crash. |
| `[backups] every`, `keep` | Automatic backups (`"6h"`, `"0"` turns them off) and how many to keep. |
| `[updates] keep` | Other releases kept besides the one in use and the one rollback goes to. |
| `[panel] bind`, `allow_public` | The web panel's address (`""` turns it off) and whether it may listen on a public address. |

## What to know when you switch

- **Edit `server.toml`, not `server.properties`.** The latter is rewritten at every start.
- **Mistakes stop the start.** Unknown keys and values vanilla would refuse are errors, not ignored.
- **In-game changes survive.** When the game changes a setting itself (`/whitelist on`), the supervisor copies
  it back into `server.toml` after the game stops, and keeps your comments and order.
- **Changes apply at the next restart.** `horizon-server config set KEY=VALUE` edits the file without opening
  it; on a running server it applies `difficulty`, `defaultgamemode` and `whitelist on|off` live. There is no
  full live reload yet.

## Planned: limiting a server to Horizon clients

The design (`docs/design.md`, section 6 of horizon-rust/horizon-server) adds one more setting. It is not in
`server.toml` yet.

```toml
[clients]
allow = "any"        # "any" (vanilla Java and Horizon) or "horizon" (Horizon clients only)
```

With `"horizon"` the server refuses a client that does not identify as Horizon. This is a convenience filter,
not security: Horizon's client is open source, so a modified client can claim to be Horizon. The real gate
stays `online_mode`, plus the whitelist and bans on top of it.
