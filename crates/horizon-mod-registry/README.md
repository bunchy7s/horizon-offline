# horizon-mod-registry

The service behind the registry's submission form (`playhorizon.dev/mods/submit`). A creator signs in with GitHub and
**lists a project by its GitHub repository and a page** (`repo` plus name, summary, licence...): the service checks that the
repository is public and opens a pull request adding `mods/<id>.toml` to
[horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods), where a reviewer reads the repository once.
**It takes no versions for such a project**: they are the repository's GitHub releases, read and checked by that
repository's CI with no pull request ("publish a GitHub release in <repo>", a 409). Projects listed before that
(no `repo`) can still be sent as a file: the service runs the file's own checks
(`horizon_mods::index::inspect`, the signature, the entry's fields), uploads it to the `staging` release as
`<sha256>.<extension>` and opens a pull request adding or updating the entry. Formats and rules:
[docs/api/mod-index.md](../../docs/api/mod-index.md).

Signed-in players also rate published projects, 1 to 5 stars: one rating per GitHub account and project, which they can
change, never their own project's. That is the only data it keeps, in one SQLite file (`REGISTRY_DB`); it only ever
shows totals, never who rated. A session is a signed cookie holding the GitHub login and id, and the user's OAuth token
is used once, to read them, and dropped. The sign-in asks for no scope.

| Route | |
| --- | --- |
| `GET /api/mods/auth/login` | Starts the GitHub sign-in; `?back=/mods/<id>` (or `/mods`) comes back to that page |
| `GET /api/mods/auth/callback` | GitHub's redirect: sets the session, back to `/mods/submit` or the `back` page |
| `POST /api/mods/auth/logout` | Ends it |
| `GET /api/mods/me` | `{login, csrf}`, or 401 |
| `POST /api/mods/submit` | Multipart: `entry` (JSON: `id kind name summary description authors license icon key`, and either `repo` (no file: lists the project) or `source signature version game changelog` with the `file` (a project with no `repo`)); `{pull_request}` or `{error}` |
| `GET /api/mods/ratings` | `{projects: {<id>: {average, count}}}` for every rated project; cached a minute |
| `GET /api/mods/<id>/rating` | The signed-in account's own `{stars}` (null when it hasn't rated), or 401 |
| `POST /api/mods/<id>/rating` | JSON `{stars}`, 1 to 5: sets or changes it; `{stars, average, count}` or `{error}` |
| `GET /healthz` | `ok` |

A submission or a rating must come from `REGISTRY_PUBLIC_URL`'s origin with the session's `X-CSRF-Token`. Six
submissions per account and sixty in all per hour; thirty ratings per account and 600 in all. A project's owners can't
rate it, and only projects on the registry's `main` can be rated. Only an entry's `owners` publish its updates; a key change, an icon in the repository or anything else
is a pull request by hand.

## Owner updates

Only for projects with no `repo` (a project listed by its repository has no update to send). A reviewer's approval is not needed for an *owner update*: the submitter's login is in the entry's `owners` on the
registry's `main`, and the only change to the entry is one new version (key, owners, icon, kind, id, name, summary,
description, authors, licence and source as they were; a signature when the entry has a key). Then, and only when
`REGISTRY_UPDATE_KEY` is set, the pull request's body ends with a trailer and it gets the label `owner-update`:

```
<!-- owner-update
id: coords_hud
version: 0.1.0
sha256: <hex>
file: coords_hud-0.1.0.hmod
submitter: alice
hmac: <hex HMAC-SHA256 with REGISTRY_UPDATE_KEY of "owner-update-v1\n<id>\n<version>\n<sha256>\n<file>\n<submitter>">
-->
```

Everything else (a new project, a changed page or key, no secret) opens the pull request as before, for a reviewer.
The trailer is the only thing that marks an update as the form's own: the horizon-mods repository's `owner-update`
workflow verifies it with the same secret, re-checks the diff, waits for the checks and merges with `REGISTRY_MERGE_TOKEN`
(its README says how). Without the secret on both sides, nothing is merged by itself.

A refused submission is always `{error}` JSON, sent once the upload is over (a browser still sending when the answer
comes reports a network error instead): 413 for a file over 64 MiB, 504 when GitHub doesn't answer in time (30 s a
call, 5 minutes for the file's upload), 502 for GitHub's other failures.

## Running it

```sh
cargo test                     # against a mock GitHub (tests/submit.rs)
REGISTRY_PUBLIC_URL=http://localhost:4173 REGISTRY_SESSION_KEY=... GITHUB_CLIENT_ID=... GITHUB_CLIENT_SECRET=... \
  GITHUB_BOT_TOKEN=... REGISTRY_REPO=you/horizon-mods-test REGISTRY_DB=/tmp/registry.sqlite3 cargo run
```

| Variable | |
| --- | --- |
| `REGISTRY_PUBLIC_URL` | The site, `https://playhorizon.dev`: the OAuth callback is `<it>/api/mods/auth/callback` |
| `REGISTRY_SESSION_KEY` | 32 characters or more, random: signs sessions |
| `GITHUB_CLIENT_ID`, `GITHUB_CLIENT_SECRET` | The OAuth app |
| `GITHUB_BOT_TOKEN` | Fine-grained, the registry repository only: Contents and Pull requests read and write |
| `REGISTRY_UPDATE_KEY` | Optional, random, 32 characters or more: signs owner updates' trailers (below). Unset: every update waits for a reviewer. The same value is the secret `REGISTRY_UPDATE_KEY` of horizon-mods |
| `REGISTRY_REPO` | Default `horizon-rust/horizon-mods` |
| `REGISTRY_DB` | The ratings' SQLite file: default `/data/registry.sqlite3`, the volume |
| `PORT` | Default 8080 |

On Railway it is the service `horizon-mod-registry` of the project `horizon-site`, next to the website: built from
`main` with this `Dockerfile` from the repository root, health check `/healthz`, no public domain, and the volume
`registry-data` at `/data` for the ratings. The
website's Caddy sends `/api/mods/*` to it over Railway's private network (`REGISTRY_UPSTREAM` on the website service:
`http://horizon-mod-registry.railway.internal:8080`). `railway.json` lists the same settings; Railway reads them from
the service, not from the file.
