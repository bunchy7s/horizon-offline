# Microsoft account sign-in and online-mode play

Horizon signs players in with their Microsoft account in the launcher and joins online-mode
(authenticated) servers like vanilla 26.3 does. The launcher plays only with an account that owns
Minecraft: Java Edition ([the owner gate](#the-owner-gate)); offline names, LAN and offline-mode
servers play behind it. This note is the design, the status of each part and what Ludovic has to
register.

## What Ludovic registers (needs his accounts)

Nothing below can be done by an agent: the app registrations belong to Ludovic. A build without
the client ID can't sign in, so its launcher can't start the game ("This build of the launcher has
no Microsoft sign-in"); release builds have it.

1. **Azure app registration** (https://portal.azure.com → Microsoft Entra ID → App registrations →
   New registration):
   - Name: `Horizon` (shown to players on the Microsoft consent page).
   - Supported account types: **Personal Microsoft accounts only** (Minecraft accounts are
     consumer accounts; the launcher uses the `consumers` tenant).
   - Redirect URI: none needed for the device-code flow. If the form insists, add a
     "Mobile and desktop applications" redirect `https://login.microsoftonline.com/common/oauth2/nativeclient`.
   - After creating it: Authentication → Advanced settings → **Allow public client flows: Yes**
     (this enables the device-code flow; there is no client secret, Horizon is a public client).
   - API permissions: none to add. The launcher requests the scopes `XboxLive.signin offline_access`
     at sign-in time; `offline_access` gives the refresh token.
   - Copy the **Application (client) ID** (a GUID).
2. **Minecraft API access** (Mojang): new Azure apps are refused by
   `api.minecraftservices.com/authentication/login_with_xbox` (HTTP 403, "Invalid app registration")
   until Mojang approves them. Apply with the form linked from https://aka.ms/mce-reviewappid
   (Minecraft Java "App ID review"), giving the client ID above, the app name, the project URL
   (https://github.com/horizon-rust/horizon) and a short description ("open-source Minecraft Java
   Edition client written in Rust; signs players in to join online-mode servers"). Approval takes
   days to weeks; sign-in fails at the Minecraft step with "This app is not approved by Mojang yet"
   until then.
3. **Where the ID goes**:
   - Release builds: the GitHub repository variable (not a secret: client IDs are public)
     `HORIZON_MSA_CLIENT_ID`; the launcher's build reads it with `option_env!` at compile time.
   - Any build, at run time: the environment variable `HORIZON_MSA_CLIENT_ID` overrides the built-in
     value (empty or unset = not configured).

Never use another application's client ID (the official launcher's or another launcher's).

## Flow (launcher)

All requests use HTTPS and JSON except the two OAuth calls (form-encoded):

1. **Device code**: `POST https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode`
   (`client_id`, `scope=XboxLive.signin offline_access`). The launcher shows the `user_code` and
   `verification_uri` (https://www.microsoft.com/link) with a copy button and opens the browser.
2. **Poll** `POST …/consumers/oauth2/v2.0/token` (`grant_type=urn:ietf:params:oauth:grant-type:device_code`)
   every `interval` seconds; `authorization_pending` waits, `slow_down` adds 5 s, `authorization_declined`,
   `expired_token` and `bad_verification_code` end the sign-in with a message. Result: the Microsoft
   access token and refresh token.
3. **Xbox Live**: `POST https://user.auth.xboxlive.com/user/authenticate` with
   `AuthMethod=RPS, SiteName=user.auth.xboxlive.com, RpsTicket=d=<MS access token>`,
   `RelyingParty=http://auth.xboxlive.com`. Result: the XBL token and the user hash (`uhs`).
4. **XSTS**: `POST https://xsts.auth.xboxlive.com/xsts/authorize` with the XBL token,
   `SandboxId=RETAIL`, `RelyingParty=rp://api.minecraftservices.com/`. The `XErr` codes become
   messages: 2148916233 no Xbox profile yet, 2148916235 Xbox Live not available in the country,
   2148916236/2148916237 adult verification (South Korea), 2148916238 child account (needs a family).
5. **Minecraft**: `POST https://api.minecraftservices.com/authentication/login_with_xbox`
   (`identityToken = "XBL3.0 x=<uhs>;<XSTS token>"`). Result: the Minecraft access token (24 h).
6. **Entitlements**: `GET https://api.minecraftservices.com/entitlements/mcstore`: the account must
   own `game_minecraft` or `product_minecraft` (Game Pass included), else "This account doesn't own
   Minecraft Java Edition".
7. **Profile**: `GET https://api.minecraftservices.com/minecraft/profile`: UUID, name, skins, capes.
   404 = owns the game but has no profile yet ("Create your Minecraft profile on minecraft.net first").

## The owner gate

Decided 2026-10-27 (Ludovic, after a modder's suggestion): like other launchers, Horizon's
launcher starts the game only while an account in it owns Java Edition. Offline play stays
possible; the launcher just doesn't offer it on its own.

- **Play needs an owner.** Every Play (Home, instances and packs, `horizon-launcher play`,
  `modpack play`, the MCP's `play`) asks `account::launch` first. It asks the Minecraft services
  again for each account, the selected one first, until one owns the game: the token renewal
  (below) if due, then entitlements and profile. Without an owner the game doesn't start and Play
  says "Sign in with a Microsoft account that owns Minecraft: Java Edition to play", with Sign in.
- **The window says it once.** A player without an owner (one who played offline until now) sees
  the gate's screen once when the launcher opens: why Horizon needs the account ("like other
  launchers"), that worlds, offline names and settings stay where they are, and Sign in, which shows
  the device code on the same screen. It comes back whenever Play is refused; Not now closes it,
  and Home keeps a Sign in row (`launcher/src/owner_page.rs`).
- **The answer lives with the tokens.** Each account's keychain item keeps the last answer
  (`owns`), so no plain file can say yes. With no network (or a sign-in that can't be renewed) the
  last answer stands: being offline never blocks a player who passed before, and the game starts
  without a session, offline. An answer of "doesn't own" (or no profile) blocks at once.
- **Offline names play behind an owner.** They can be added and played only while an owning
  account is signed in. Signing the last one out keeps them, and they play again once one is back.
- **Who isn't affected:** the game binary itself (`horizon-client`, `horizon-ctl launch`, the
  harnesses, CI), the dedicated server and `horizon-server`, the website. There is no bypass flag:
  the launcher's tests use accounts in a memory store (`MemoryStore`, and `HORIZON_NO_KEYCHAIN`
  for the CLI and MCP tests, which then have no owner).

## Downloading the game for an owner

Decided 2026-10-27. This is the only exception to "Horizon never downloads the game", and it
applies only on the player's machine, for an account the owner gate just asked about. The
launcher's "Download and prepare" (`horizon-launcher prepare --download`) fetches the pinned
version straight from Mojang (`launcher/src/mojang.rs`): the version file, the client jar,
libraries, the asset index and the assets, each checked against the size and sha1 Mojang lists.
They go to `minecraft/` in the launcher's data folder, and the preparation runs as usual. The
download can be cancelled and goes on where it stopped. CI, agents, harnesses and the repo never
download or keep Mojang's files.

Refresh: the Minecraft token is renewed from the Microsoft refresh token (steps 2-5 with
`grant_type=refresh_token`) when it has less than an hour left, before every launch; steps 6-7 run
at every launch (the owner gate). A refused
refresh token (`invalid_grant`) asks the player to sign in again. Several accounts can be signed in;
one is selected. "Sign out" deletes the account's secrets from the keychain.

## Token storage

- Secrets (the Microsoft refresh token, the Minecraft access token and its expiry, the last
  ownership answer) go to the OS
  keychain, one item per account: Keychain on macOS, Credential Manager on Windows, the Secret
  Service (GNOME Keyring, KWallet) on Linux. Service name `dev.playhorizon.launcher`, account = the
  Minecraft UUID. Credential Manager takes at most 2560 bytes per item, which the secrets of a sign-in
  exceed: on macOS and Windows a longer secret is split over the items `<uuid>`, `<uuid>.1`, `<uuid>.2`...
- `accounts.json` in the launcher's data folder keeps only public data: UUID, name, skin URL, which
  account is selected. Never a token.
- No keychain available (a Linux box without a Secret Service, a locked keychain): sign-in still
  works for the session but nothing is saved, and the launcher says "No keychain available: you will
  have to sign in again next time" (it never falls back to a plain file).
- Tokens never appear in logs, errors, `horizon-launcher` CLI output, MCP results or crash reports.
  Requests are logged by endpoint name only.

## Handing the session to the game

Vanilla's launcher passes `--accessToken <token>` on the command line, where every user of the
machine can read it in the process list. Horizon keeps it out of argv:

- The launcher writes `session-<random>.json` (mode 0600 on Unix, created new, in the game home's
  `sessions/` folder) with `{"name", "uuid", "access_token", "user_type": "msa", "xuid"}` and starts
  the game with `--session-file PATH`.
- The game reads the file at start and deletes it at once; the token lives only in the game's memory.
  A file older than two minutes is refused (and deleted), so a stale file can't be replayed.
- Without a session file the game is offline as today (`--name`, offline UUID).
- `HORIZON_SESSION_FILE` is the same as `--session-file` (development and harnesses).

## Online-mode login (game)

Vanilla's `ClientHandshakePacketListenerImpl.handleHello`, `Crypt` and authlib's
`MinecraftServicesSessionService.joinServer`:

1. `ClientboundHelloPacket` (server id, X.509 RSA public key, challenge, should_authenticate).
2. Generate a random 16-byte AES key (the shared secret).
3. Server hash: SHA-1 over `serverId` (ISO-8859-1), the shared secret, the encoded public key;
   `new BigInteger(digest).toString(16)` (signed two's complement hex, e.g. "Notch" →
   `4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48`, "jeb_" → `-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1`).
4. If `should_authenticate`: `POST` the session service's `join` endpoint
   `{"accessToken", "selectedProfile" (undashed UUID), "serverId": hash}`, the URL taken from the
   services discovery document (`https://discovery.minecraftservices.com/minecraft/client`, fetched
   once per game). Errors map like authlib's `MinecraftClientHttpException`:
   `ForbiddenOperationException` or 401 → "Failed to log in: Invalid session (Try restarting your
   game and the launcher)", `InsufficientPrivilegesException` → "Multiplayer is disabled…",
   `multiplayer.access.banned` / `FORCED_USERNAME_CHANGE` → "You are banned from playing online",
   5xx or no network → "The authentication servers are currently not reachable…". Without a session
   the game says "This server is in online mode. Sign in with your Microsoft account in the Horizon
   launcher to join it." (Vanilla goes on after a failed join on LAN-discovered servers, which then
   refuse the player anyway; Horizon stops at once.)
5. Send `ServerboundKeyPacket` (the secret and the challenge, each RSA PKCS#1 v1.5 encrypted with
   the server key), then AES/CFB8 (key = IV = the secret) both ways.

The connecting screen follows vanilla's states: "Connecting to the server..." until the encryption
request, "Logging in..." while joining the session service, "Encrypting...", then "Joining world...".
Code: `horizon-protocol` (`rsa.rs`, `crypto::server_hash`), `horizon-client` (`auth.rs`, `net.rs`).

Horizon's dedicated server verifies players when `online-mode=true`. A LAN or Play with Friends world
verifies its guests when the host is signed in, as vanilla's `IntegratedServer` does
(`OnlineMode::lan`); under an offline name it stays offline-mode. The host's own player joins through
memory and is never asked to authenticate.

`HORIZON_API_DISCOVERY` overrides the discovery URL, the counterpart of authlib's
`-Dminecraft.api.discovery.host`; it is how the harness points both the vanilla server and Horizon at
a local mock.

## Skins and capes (part b)

Profile properties (`textures`, base64 JSON) arrive in `ClientboundPlayerInfoUpdatePacket` for every
player and in the login profile for ourselves. Like authlib's `unpackTextures`, only URLs under the
discovery document's `profiles/getTexture` `validUris` (textures.minecraft.net) are fetched; skins
(classic/slim from the `metadata.model`), capes and elytra textures are downloaded over HTTPS on a
worker thread, cached on disk by texture hash (`assets/skins/xx/hash`, like vanilla's
`SkinManager`), converted from legacy 64×32 the way `SkinTextureDownloader` does and given to the
player renderer. Until a skin arrives the default skin for the UUID shows (as today).

## Chat signing (part d)

26.3's dedicated servers default to `enforce-secure-profile=true`. A client without a chat session
can join and play, but every chat message and signed command argument is refused with "Chat
disabled due to missing profile public key. Please try reconnecting." (`SignedMessageChain.Decoder.unsigned`).
To chat on such servers the game will:

- fetch the profile key pair from the services `player/getCertificates` endpoint (Minecraft token);
- send `ServerboundChatSessionUpdatePacket` (random session UUID, key expiry, public key, Mojang's
  key signature) after joining;
- sign each chat message and signable command argument with SHA256withRSA over vanilla's
  `SignedMessageBody` layout and keep the last-seen message tracker and acknowledgements.

Until part d lands, Horizon joins online servers but chat works only where
`enforce-secure-profile=false` (or the server can't validate keys).

## Harnesses

`tools/test_online_mode_parity.py` (dev only, needs no account): a local mock of the Minecraft
services (discovery document, `join`, `hasJoined`/`verify`, no public keys) and a vanilla 26.3
dedicated server in online mode started with `-Dminecraft.api.discovery.host=<mock>` (authlib
10.0.77's only override: it replaces the whole discovery URL; the old `minecraft.api.session.host`
style properties are gone). Horizon's
headless client (`HORIZON_NET_REPORT`) joins with a session file holding a mock token: the server
verifies the join through the mock with the hash it computed itself (so the RSA wrapping, the hash
and AES/CFB8 are all checked against vanilla's code), the player spawns, chats and runs `/list`.
Negative cases: a token the mock rejects (the client must show vanilla's "Invalid session" message)
and a client that skips the join (the server must say "Failed to verify username!").

Rust unit tests: server hash vectors (Notch, jeb_, simon), RSA wrapping round trip with a test key,
X.509 key parsing, AES/CFB8 vectors (already in `horizon-protocol`), the session file (permissions,
age, deletion), the error mapping. Launcher: the whole token chain against a local mock of the
Microsoft, Xbox and Minecraft endpoints, and the keychain abstraction with an in-memory backend.

## PR plan

- (a) Game: online-mode handshake, encryption, session join, session file, harness (15/15 checks
  against vanilla 26.3 on macOS).
- (c) Launcher: device-code sign-in, token chain, keychain, accounts UI, CLI/MCP (`account list`,
  `account sign-in`, `account sign-out`, never tokens), session file handoff.
- (b) Game: skins and capes over HTTPS, the real names and skins in game.
- (d) Game: chat signing (profile keys, chat session, signed messages and commands).

Each part ships on its own: online mode needs a session, which only (c) creates.
