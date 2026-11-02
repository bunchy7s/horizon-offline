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
  (`owns`), so accounts.json can't say yes. Where there is no keychain the answer sits in the
  secrets file with the tokens; a player can edit that file, as they can write keychain items with
  `secret-tool` or Keychain Access. With no network (or a sign-in that can't be renewed) the
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
  exceed: on Windows a longer secret is split over the items `<uuid>`, `<uuid>.1`, `<uuid>.2`... The
  Keychain has no such limit, and every item is a consent prompt for a launcher signature it doesn't
  know yet, so on macOS a secret is one item (#4912); pieces older launchers left fold into it on the
  first read, and launchers that still split read a merged item as its first piece. A write or a
  clean-up delete the store refuses puts the old secret back: on Windows new pieces in front of old
  ones would read back as no secret, and the account would be signed out at the next start; on macOS
  a merged item beside leftover pieces would read as a longer secret that doesn't parse.
- **Windows: "Windows Credential Manager is full" (Windows error 8, #4313, #4366).** Credential
  Manager refuses every new item with `ERROR_NOT_ENOUGH_MEMORY` once the player's store reaches an
  undocumented total size, whatever the item's size; it is not the computer's memory. The Xbox app
  and the Minecraft Launcher are known to leave hundreds of `XblGrts…` and `Xbl|…` items there. The
  launcher says so and nothing is saved; the player opens Credential Manager (Start menu) → Windows
  Credentials, removes entries they no longer use (`cmdkey /list` in a terminal lists them), and
  signs in again. Removing an entry can sign the app that made it out. The launcher never deletes
  other apps' items.
- `accounts.json` in the launcher's data folder keeps only public data: UUID, name, skin URL, which
  account is selected. Never a token.
- Linux and FreeBSD with no keychain answering (no Secret Service, a locked keychain, #5095): the
  secrets the keychain refuses go to `account-secrets.json` in the launcher's data folder instead,
  readable and writable by the player only (0600 on Unix) and never in plain text: the file is
  encrypted (ChaCha20-Poly1305) with a key derived from the computer's identity (`/etc/machine-id`,
  `/etc/hostid`), so a copy taken off the computer (a backup, a synced home folder) can't be read
  elsewhere. On the computer itself the file's mode is what keeps other users out. A file that
  doesn't open (another computer's, or changed) is never written over: remove it and sign in again. The
  launcher never picks one store for a run: every read takes the file's copy first (it was written
  while the keychain was away, so it is the newer one) and the keychain's otherwise, and every write
  goes to the keychain and to the file only when it refuses. Each change to the file holds
  `account-secrets.lock` across its read and replace, and each write goes through a temporary file
  of its own, so launcher and CLI processes running at once lose nothing. The lock is never held
  while the keychain is asked (it can wait on an unlock prompt): a file entry the keychain took is
  removed only if it still holds what was read before, so another process's newer write stays and
  is read first. Writes to the keychain hold `account-secrets.keychain.lock` instead, with what they
  write read under it, so a hand-over never puts back a token another process renewed or signed out
  meanwhile; only keychain writes wait on a prompt, as they would on the keychain itself. Once the keychain refuses or doesn't answer (a dismissed prompt), the run stops
  asking it and keeps new sign-ins in the file, until it finds that another process handed the
  keychain a secret it saw in the file (one it put there, read there, or found there at its start). Every start
  hands the keychain what the file kept, one secret at a time up to the first it refuses; the rest
  stays in the file, which goes once it is empty. Signing out while the keychain is away leaves an empty entry for the
  account, which hides the keychain's copy until it can be deleted. A broken file is an error naming
  only its line and column, never what it holds. Only sign-ins go there: the Tailscale access token
  (Internet servers) administers the player's whole tailnet, so without a keychain it stays in
  memory for the run (`account::keychain`). MCP's `account_list` says `secrets_file: true` while
  the file holds a sign-in (sign-out marks alone, or a file that can't be read, don't count).
- No keychain on macOS or Windows, or `HORIZON_NO_KEYCHAIN` (the launcher's own tests): secrets
  stay in memory for the run, and the launcher says "No keychain available: you are signed in until
  the launcher closes".
- Tokens never appear in logs, errors, `horizon-launcher` CLI output, MCP results or crash reports.
  Requests are logged by endpoint name only.

## Account status and renewal

Settings shows each account's current authentication status. “Ready for online play” means
its saved access token is not near expiry and its last ownership check succeeded; service
revocation can only be discovered by a later request. “Needs refresh” means its access token
has an hour or less left. On launcher startup these due sessions are renewed in the background;
Play also keeps its existing ownership and renewal checks. Fresh tokens are not renewed on a
startup status check.

**Refresh** explicitly renews an account even when its access token is fresh. Microsoft
rejecting the refresh credential marks the account **Sign in again** until a successful browser
sign-in replaces it. A network outage offers retry rather than discarding the account or
claiming it requires a full login. Unreadable keychain data is never overwritten by refresh.
If Microsoft rotates its refresh credential before Xbox/Minecraft fails, the rotated credential
is still saved so another attempt can recover.

`horizon-launcher account refresh NAME_OR_UUID` exposes the same operation (MCP:
`account_refresh`). `account list --json` includes public authentication status and token
expiry, never tokens. `account refresh --json` returns a status object, including when renewal
needs attention; callers inspect `state` rather than treating transport success as an online
session. Inspection, refresh and sign-out run off the window's drawing thread. Credential
operations are serialized in-process and between launchers using the same launcher data folder.

The reusable `account::session` API has no GPUI dependency. Keychain reads and browser sign-in
remain owned by the launcher. A running game asks it for a renewed session through the CLI:
the launcher passes its own path to the game as `HORIZON_LAUNCHER_EXE`, and
`account session NAME_OR_UUID --json` renews that account and returns a new read-once session
file (`{"session_file"}`), or the public status (`{"authentication"}`) when the account needs a
browser sign-in or a retry. `account sign-in --for NAME_OR_UUID` is the targeted sign-in below,
from the command line: it prints the link and code, refuses another Microsoft account, and
leaves who plays unchanged. `account session` is not an MCP tool: its file holds a token, and
only the game reads it. Renewing in the launcher does not change the token a running game
already holds until the game asks.

### Profile heads, progress, and targeted re-sign-in

Microsoft accounts show their public Minecraft profile head in Settings, the selected-account
chip, and the account switcher. Skins are fetched on workers and cached by their texture URL
under `profile-heads/` in launcher data. Changing the skin invalidates that image; an unavailable
image uses the UUID's default skin from installed resources, or a neutral placeholder when
those resources are absent. The cache contains public images, never authentication tokens.

Account operations report the actual stage: waiting for account access, reading the keychain,
renewing the Microsoft sign-in, Xbox Live, getting a Minecraft session, checking ownership,
loading the profile, and saving. A browser flow separately says it is waiting for the player.
Progress is public metadata and does not expose credentials through the UI or MCP.

**Sign in again** on an account row targets that exact Minecraft UUID. The browser instructions
name the expected player. Signing in to a different account is rejected before storing its
credentials or adding/selecting that identity. Retry keeps the same target; Cancel leaves the
saved account alone. Successful re-sign-in restores that account without changing the player
or offline name currently selected for Play. Adding a new account remains a separate action.

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
- `--session-stdin` takes the same JSON on standard input instead, so nothing touches the disk (for other
  launchers: [the Launch API](api/launch.md)).

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
To chat on such servers the game (`auth.rs`, `game/remote_chat.rs`, `horizon_protocol::secure_chat`):

- fetches the profile key pair from the services `player/getCertificates` endpoint with the Minecraft
  token, on a background thread at start and on each join (AccountProfileKeyPairManager): the PKCS#8
  private key, the X.509 public key, `publicKeySignatureV2`, `expiresAt` and `refreshedAfter`. Like the
  token, the pair stays in memory (vanilla's `profilekeys/` file is not written). Past `refreshedAfter` a
  new pair is fetched (checked at most hourly while playing, and on every join); a failed fetch keeps the
  old pair and tries again an hour later;
- sends `ServerboundChatSessionUpdatePacket` (random session UUID, key expiry, public key, Mojang's
  key signature) after each login packet, and again when the pair is renewed, only when the server
  plays the account under its own profile id (`isLocalPlayer`): an offline-mode server's offline id
  gets no session, so chat stays unsigned there as before;
- signs each chat message and each message argument of a command (`SignableCommand.of` over the
  server's command tree, e.g. `/msg`'s `message`) with SHA256withRSA over vanilla's
  `PlayerChatMessage.updateSignature` layout, one chain link each, and keeps the last-seen tracker
  (`LastSeenMessagesTracker`: the 20-message window, offsets, checksums, a `ServerboundChatAckPacket`
  past 64) and the 128-entry signature cache that unpacks other players' last-seen lists.

`tools/secure_chat_client_e2e.py` checks it against vanilla 26.3's server with a mock of the services
that issues certificates: online mode with `enforce-secure-profile=true` (signed chat logged as secure,
a `/msg` whisper delivered, a message flushed from Chat Delay still counted as seen, nothing refused) and
offline mode (unsigned chat, as before). Both also click commands: `sendUnattendedCommand`'s confirmations
(`game/remote_chat.rs`) ask before anything but a plain command, and a clicked command with a message argument
only reaches the chat box, for the player to send.

Chat from other players is checked as vanilla's client does (`ClientPacketListener.handlePlayerChat`,
`game/remote.rs`, `secure_chat::MessageValidator`): a wrong global chat index disconnects, and a sender
with no list entry or a message its validator refuses shows the red, italic `chat.validation_error`. A
chat session the services' profile-key keys vouch for makes a keyed validator (the key not past its
expiry plus 8 hours, the signature, the chain); anything else makes the fallback, which accepts unsigned
messages, or none where the server enforces secure chat. The unit tests in `secure_chat.rs` cover the
validators; the script above doesn't send another player's chat.

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
