# The chat filter

A dedicated server refuses hate slurs in what players write: chat, `/msg` (`/tell`, `/w`), `/me`, `/say`,
`/teammsg` (`/tm`), signs, anvil names and book titles. It is on by default; `server.toml` turns it off or
tunes it. LAN worlds and single player don't filter.

## What happens

- A refused text reaches nobody, the log's chat lines and horizon-server's `chat` events included. The
  sender reads, in red: "That message wasn't sent: slurs aren't allowed here." (a sign is left as it was, a
  book stays unsigned, an anvil keeps the name it had).
- `logs/chat-filter.log` keeps one line per refused text, with its original wording, for operators:
  `2026-10-31T18:05:39Z <name> <uuid> chat hit 2 muted: <text>`. The console only says who and what
  followed; horizon-server's event stream gets a `chat_filtered` event (`place`, `hit`, `action`), never
  the text.
- Escalation, counted per player: the 2nd hit within 10 minutes mutes them for 10 minutes (no chat, no
  message commands); the 3rd bans them for 24 hours (banned-players.json, with the date it ends). Time spent
  muted doesn't count towards the window, so the hit after a mute still counts as the next. Anvil names
  never count (the client renames at every key typed): they are only refused.
- Mutes are kept in `chat-filter-mutes.json` beside `server.properties`, and survive a restart.
  Operators of level 3 lift one with `/unmute <player>`; the console takes `unmute <player>`.

## Settings

```toml
[chat_filter]
enabled = true          # false: players' text goes out as they wrote it
mute_after = 2          # the hit, within window_minutes, that mutes; 0: never
tempban_after = 3       # the hit that bans for tempban_hours; 0: never
window_minutes = 10
mute_minutes = 10
tempban_hours = 24
words_file = ""         # your own list, relative to the server's folder; "": the built-in one
log = "logs/chat-filter.log"
```

horizon-server checks the section before every start, and the game reads it from the same file.

## The list

The built-in list is `crates/horizon-chat-filter/data/slurs.txt`; a `words_file` replaces it and uses the
same format, explained at its top: `word` matches inside longer words, `=word` only whole words, `~word`
also spread over several words, `!word` never matches (`!word*`: any word starting so).

Text is normalised before it is matched: case, accents, look-alike letters (Cyrillic, Greek, full-width,
small capitals, styled and circled letters), invisible characters, leetspeak (`1` i, `3` e, `4` a, `0` o,
`@` a, `$` s, `9` g), separators between letters (`n.i.g`, `n i g`) and repeated letters. A doubled letter
in an entry must be doubled in the text, so Niger and Nigeria pass, and so do "snigger", "knight",
"suspicious" and "raccoon". The list compiles once at start; a message costs a small fraction of a millisecond.

Keep the list to slurs: insults that aren't slurs are the moderators' to handle.

Code: `crates/horizon-chat-filter` (settings, matcher, escalation, mutes), `lan/hub/chat_filter.rs` (the
check before the server relays a text), `game/chat_filter.rs` (log, events, bans, `/unmute`).
