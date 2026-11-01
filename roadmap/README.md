# The roadmap

`roadmap.toml` is the one source of the roadmap. `tools/roadmap.py build` turns it into
[ROADMAP.md](../ROADMAP.md); the website's hub (playhorizon.dev/hub) and Kira read the same items. The work
board stays the task-level view (every issue and pull request); the roadmap is the overview, and each pull
request carries the `roadmap:<id>` label of the item it moves.

## An item

```toml
[[item]]
id = "server-dialogs"            # lowercase words joined by dashes; never renamed (votes and links use it)
title = "Server dialogs and login screens"
status = "now"                   # now | next | later | help-wanted | shipped
area = "servers"                 # one of the [[area]] ids, listed in the order of priority
focus = true                     # optional: one of the 3 to 5 Now items in the overview
ongoing = true                   # optional: a workstream that goes on after its pull requests merge
owner = "levzzz5154"             # Now items: the GitHub login who leads it (a person, or the account an agent works as)
issue = "#3911"                  # Now items: the tracking issue that holds the checklist
plain = """Servers that show dialogs ... will work in Horizon."""   # players' words, at most 240 characters
dev = """26.3's dialog packets and model (#1720), ..."""             # crates, approach, what is left
links = ["#1720", "#1734", "docs/hosting.md"]                       # #N, a repository path, or https://
shipped_in = "v0.2.58"           # shipped items only: a game tag, launcher@x.y.z, or "website"
post = "..."                     # optional: the announcement when it ships, if `plain` doesn't read as one
announce = true                  # optional: when it ships, Kira also posts it on X (`post`, else `plain`)
```

- **Short lists** (issue #3859): **Now holds at most 8 items, each with an `owner` and a tracking `issue`**; Next
  at most 12; everything else is Later or an issue. The owner decides the order of the work inside the item and
  keeps its tracking issue current (agents help). A Now item without anyone on it moves to Next or Later.
- **No dates, ever**: Now, Next, Later. The check rejects month names, years, quarters and "next week".
  "Coming soon" is the only phrase for something waiting on someone else (Microsoft sign-in, CurseForge).
- **Only what is decided.** Ideas stay in issues and in the suggestion queue until they are.
- **Only what is true in a release** for shipped items: a merged pull request isn't shipped until a release has it.
- `plain` is for players: what they will be able to do, not how. `dev` is for contributors: the code, the
  vanilla classes, the pull requests, what is left. Both stay public: no machines, accounts or private notes.
- Help wanted items are good first contributions: a clear vanilla behaviour, an issue with steps, a place to
  start in `dev`.

## Keeping it current

Whoever moves the work (the orchestrator on each heartbeat, or anyone in their pull request):

1. `python3 tools/roadmap.py sync`: every item whose linked pull requests all went out in a release, whose
   linked issues are all closed and that isn't `ongoing` becomes `shipped`, with that release in
   `shipped_in`, on top of the shipped items. It rewrites `roadmap.toml` and ROADMAP.md; open a pull request
   with the change (`skip-release`).
2. `python3 tools/roadmap.py check`: the file's rules, then GitHub's state of every link. It fails when a
   link is not an issue or pull request, when a tracking issue is not an open issue, when a Now item has no open pull request and none merged in the last
   14 days (link the new work, or move it to Next or Later), and when every link of a Help wanted item is
   closed. It also says which items `sync` will ship once a release has them.
3. When work starts, opens a pull request or is decided: add or move the item, add its links, then
   `python3 tools/roadmap.py build`. Keep the overview's focus to the 3 to 5 biggest Now items.
4. `python3 tools/roadmap.py labels` (after the change merges): creates a `roadmap:<id>` label for each new
   item, updates their colour (by status) and description, and adds the label to the open pull requests an item
   links. Label a new pull request for an item by hand or by linking it and running `labels` again. Labels of
   removed items are listed, never deleted.
5. Once a week, write the week's summary in `roadmap/journal/<year>-<ISO week>.md` (for example
   `2026-43.md`): `# Title`, a paragraph or two for players, then `## Technical` and the same week for
   contributors, with its pull requests. `build` puts it into roadmap/roadmap.json, which the website reads from main; the hub's Journal tab
   shows it among the releases, and `/data/journal.json` carries it to Kira's #updates and to X.
   In the same pass, `python3 tools/parity_gaps.py readme --write` refreshes the parity headline: the
   player-visible gaps still open in the port ledger's ten lists (#3512 to #3521), on the README and the website.
6. Suggestions come only from #horizon-ideas posts on Discord: Kira sends each one to the maintainers' private
   channel. A good one becomes an item in a normal pull request; none is published automatically.

CI runs `tests/test_roadmap.py` (in the Python tests), which fails when ROADMAP.md is not what `build` writes
or when the file breaks a rule; `python3 tools/roadmap.py check --offline` runs the same rules without GitHub.

## The journal

The hub's Journal tab and `https://playhorizon.dev/data/journal.json` are built with each deploy of the website
(`src/hub.js` in horizon-rust/horizon-site). The tab shows every game and launcher release with its notes' changes, each placed under
the roadmap item it moves (the pull request's `roadmap:<id>` label, else the item that links it), the merged pull
requests no release has yet, and the weekly summaries above.

`journal.json` is what Kira reads for `#updates` and its X posts (kira's `src/journal.js`): one flat entry per
merged pull request, release, shipped item and weekly summary, newest first, the latest 300.

```json
{ "format": "horizon-journal", "version": 2, "generated": "…", "url": "https://playhorizon.dev/hub#journal",
  "entries": [
    { "id": "pr-1788", "kind": "merge", "date": "…", "title": "the pull request's title", "url": "…",
      "number": 1788, "item": "discord-presence", "item_title": "Discord Rich Presence" },
    { "id": "release-v0.2.57", "kind": "release", "date": "…", "title": "…", "plain": "one line", "url": "…",
      "tag": "v0.2.57" },
    { "id": "shipped-server-resource-packs", "kind": "shipped", "date": "the release's", "title": "…",
      "plain": "…", "url": "https://playhorizon.dev/hub#server-resource-packs", "item": "…", "item_title": "…",
      "shipped_in": "v0.2.48", "post": "at most 240 characters", "announce": false },
    { "id": "weekly-2026-43", "kind": "weekly", "date": "the week's last second", "title": "…", "plain": "…",
      "dev": "…", "url": "https://playhorizon.dev/hub#week-2026-43" } ] }
```

- An `id` never changes: readers post each one once. `merge` entries are the pull requests players see (no
  `skip-release` label), plus any that moves a roadmap item; `item` is absent when none does.
- `announce: true` (the item's `announce` key) asks Kira to post the shipped item on X too, with `post`.
- `https://playhorizon.dev/data/roadmap.json` is `roadmap.toml` as JSON (`areas`, `items` with every key above,
  `overview`, the weekly summaries); an item's page is `https://playhorizon.dev/hub#<id>`.

## The intake: votes and bug reports

The hub's forms and vote buttons talk to the telemetry server (`https://telemetry.playhorizon.dev`,
`crates/horizon-telemetry-server`, `src/hub.rs` and `src/intake.rs`; its README lists the limits and settings).
Kira uses the same endpoints, so a bug goes the same way from the site and from Discord.

| Endpoint | Body | Answer |
| --- | --- | --- |
| `POST /hub/vote` | `{"item": "<roadmap id>", "voter": "<random id, 16 to 64 characters>", "up": true}` | `{"item", "votes"}`; `"up": false` takes the vote back |
| `GET /hub/votes` | | `{"votes": {"<item>": n}}`, a minute old at most |
| `POST /hub/report` | `{"title", "did", "happened", "expected", "version"?, "system", "name"?}` | 201 `{"id", "issue": "<GitHub URL>"}`; 202 `{"id", "issue": null}` when GitHub was down (opened later) |

- A report becomes an issue at once, labels `bug` and `from-website`, in the `#horizon-bugs` template (What I did,
  What happened, What I expected, Horizon version, OS, CPU, GPU) with "Reported on playhorizon.dev". Every field
  but the version and the name is required; mentions, `#123` references and HTML in people's text are made inert.
- Errors are `{"error": "…"}`: 400 names the field (`"did: 3 to 2000 characters"`), 429 means too many from that
  address, 503 a full day or reports turned off (no GitHub token on the server).
- The page shows the forms and the buttons only once `GET /hub/votes` answers. Until then it points to GitHub and
  Discord. It keeps a random voter id and the items it voted for in `localStorage` (`hub:voter`, `hub:voted`).

