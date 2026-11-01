//! Microsoft account sign-in (docs/microsoft-account.md): the OAuth device-code flow, then Xbox
//! Live, XSTS and the Minecraft services, the game ownership check and the profile. Secrets (the
//! Microsoft refresh token, the Minecraft access token, the last ownership answer) live in the OS
//! keychain; accounts.json keeps only names and UUIDs. The game gets the session in a file it
//! reads once (`--session-file`), never on its command line. Tokens never reach logs, errors or
//! output.
//!
//! The owner gate ([`launch`]): the launcher starts the game only while an account in it owns
//! Minecraft: Java Edition, as other launchers do. Offline names play behind such an account.
//! The game binary itself, its harnesses and the servers don't go through the launcher's gate.
//!
//! The Azure app (client ID) is Horizon's own and is configuration: `HORIZON_MSA_CLIENT_ID` at
//! build time, or at run time to override it. Without one, sign-in says it is not configured.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The keychain service name every secret is stored under.
pub const SERVICE: &str = "dev.playhorizon.launcher";
const SCOPE: &str = "XboxLive.signin offline_access";
pub const NOT_CONFIGURED: &str =
    "This build of the launcher has no Microsoft sign-in (HORIZON_MSA_CLIENT_ID), so it can't start the game.";
/// What every Play says while no account in the launcher owns the game.
pub const NEEDS_OWNER: &str =
    "Sign in with a Microsoft account that owns Minecraft: Java Edition to play";
const DOES_NOT_OWN: &str = "This account doesn't own Minecraft: Java Edition.";
const NO_PROFILE: &str =
    "This account has no Minecraft profile yet. Create one at minecraft.net, then sign in again.";
const OTHER_PROFILE: &str =
    "The Microsoft account now has another Minecraft profile. Sign in again.";
const SIGN_IN_AGAIN: &str = "Your Microsoft sign-in expired. Sign in again.";
/// The Minecraft token is renewed when it has less than this left.
const RENEW_BEFORE: u64 = 3600;

/// The configured Azure app, if any: the run-time variable wins over the build-time one, and an
/// empty value means none.
pub fn client_id() -> Option<String> {
    std::env::var("HORIZON_MSA_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("HORIZON_MSA_CLIENT_ID").map(str::to_owned))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
}

/// Where the requests go: the real services, or a mock in tests.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The OAuth 2.0 v2 endpoints for personal accounts (`…/devicecode`, `…/token`).
    pub microsoft: String,
    pub xbox_user: String,
    pub xsts: String,
    /// api.minecraftservices.com.
    pub minecraft: String,
}

impl Endpoints {
    pub fn live() -> Self {
        // Development builds only: `HORIZON_MSA_MICROSOFT_ENDPOINT` points the device-code and
        // token requests at a local mock, to see and screenshot the sign-in panel.
        let microsoft = std::env::var("HORIZON_MSA_MICROSOFT_ENDPOINT")
            .ok()
            .filter(|_| cfg!(debug_assertions))
            .unwrap_or_else(|| "https://login.microsoftonline.com/consumers/oauth2/v2.0".into());
        Self {
            microsoft,
            xbox_user: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            minecraft: "https://api.minecraftservices.com".into(),
        }
    }
}

/// A request to `url`: the launcher's user agent, a 20 s timeout, and the player's proxy when one
/// is set (`proxy`), since a sign-in has to get out the way downloads do.
fn request_to(method: &str, url: &str) -> ureq::Request {
    crate::proxy::agent(
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(20))
            .user_agent(&format!("horizon-launcher/{}", crate::backend::VERSION)),
        url,
    )
    .build()
    .request(method, url)
}

/// One request's result: the status and the JSON body (empty when there is none). Transport
/// errors name the service, never the URL's query or the request.
fn send(request: ureq::Request, body: Option<Body>, service: &str) -> Result<(u16, Value), String> {
    let result = match body {
        None => request.call(),
        Some(Body::Form(fields)) => request.send_form(&fields),
        Some(Body::Json(value)) => request.set("Accept", "application/json").send_json(value),
    };
    let response = match result {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(ureq::Error::Transport(_)) => {
            return Err(format!(
                "Could not reach {service}. Check your internet connection."
            ))
        }
    };
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

enum Body<'a> {
    Form(Vec<(&'a str, &'a str)>),
    Json(Value),
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ----- The device-code flow ----------------------------------------------------------------------

/// What the player types at the verification page.
#[derive(Clone)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_at: u64,
    device_code: String,
}

impl std::fmt::Debug for DeviceCode {
    // The device code can be traded for tokens once the player approves: never shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCode")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish_non_exhaustive()
    }
}

/// The Microsoft tokens; the access token only lasts for the Xbox step.
pub struct MsTokens {
    access: String,
    refresh: String,
}

impl std::fmt::Debug for MsTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MsTokens(<hidden>)")
    }
}

pub enum Poll {
    Pending,
    /// The server asked to poll less often: wait 5 s more.
    SlowDown,
    Done(MsTokens),
}

pub fn device_code(ep: &Endpoints, client_id: &str) -> Result<DeviceCode, String> {
    let (status, v) = send(
        request_to("POST", &format!("{}/devicecode", ep.microsoft)),
        Some(Body::Form(vec![("client_id", client_id), ("scope", SCOPE)])),
        "Microsoft",
    )?;
    match (
        status,
        str_field(&v, "user_code"),
        str_field(&v, "device_code"),
    ) {
        (200, Some(user_code), Some(device_code)) => Ok(DeviceCode {
            user_code,
            verification_uri: str_field(&v, "verification_uri")
                .unwrap_or_else(|| "https://www.microsoft.com/link".into()),
            interval: v
                .get("interval")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .max(1),
            expires_at: now() + v.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
            device_code,
        }),
        _ => Err(oauth_error(&v)),
    }
}

/// The OAuth error codes as messages for the player.
fn oauth_error(v: &Value) -> String {
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_declined") => "Sign-in was declined in the browser.".into(),
        Some("expired_token") => "The sign-in code expired. Start again.".into(),
        Some("bad_verification_code") => "The sign-in code was not recognised. Start again.".into(),
        Some("invalid_grant") => SIGN_IN_AGAIN.into(),
        Some("invalid_client" | "unauthorized_client") => {
            "Microsoft refused this launcher's app registration (client ID).".into()
        }
        Some(other) => format!("Microsoft sign-in failed ({other})."),
        None => "Microsoft sign-in failed.".into(),
    }
}

/// One poll of the token endpoint while the player signs in.
pub fn poll(ep: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Poll, String> {
    let (status, v) = send(
        request_to("POST", &format!("{}/token", ep.microsoft)),
        Some(Body::Form(vec![
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id),
            ("device_code", &code.device_code),
        ])),
        "Microsoft",
    )?;
    match v.get("error").and_then(Value::as_str) {
        Some("authorization_pending") => return Ok(Poll::Pending),
        Some("slow_down") => return Ok(Poll::SlowDown),
        _ => {}
    }
    ms_tokens(status, &v).map(Poll::Done)
}

fn ms_tokens(status: u16, v: &Value) -> Result<MsTokens, String> {
    match (
        status,
        str_field(v, "access_token"),
        str_field(v, "refresh_token"),
    ) {
        (200, Some(access), Some(refresh)) => Ok(MsTokens { access, refresh }),
        _ => Err(oauth_error(v)),
    }
}

/// Wait for the player to finish in the browser, polling as asked, until the code expires or
/// `cancelled` says stop.
pub fn wait_for_sign_in(
    ep: &Endpoints,
    client_id: &str,
    code: &DeviceCode,
    cancelled: &dyn Fn() -> bool,
) -> Result<MsTokens, String> {
    let mut interval = code.interval;
    loop {
        let until = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < until {
            if cancelled() {
                return Err("Sign-in cancelled.".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if now() > code.expires_at {
            return Err("The sign-in code expired. Start again.".into());
        }
        match poll(ep, client_id, code)? {
            Poll::Pending => {}
            Poll::SlowDown => interval += 5,
            Poll::Done(tokens) => return Ok(tokens),
        }
    }
}

fn refresh_ms(ep: &Endpoints, client_id: &str, refresh: &str) -> Result<MsTokens, String> {
    let (status, v) = send(
        request_to("POST", &format!("{}/token", ep.microsoft)),
        Some(Body::Form(vec![
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", refresh),
            ("scope", SCOPE),
        ])),
        "Microsoft",
    )?;
    ms_tokens(status, &v)
}

// ----- Xbox Live, XSTS, Minecraft ----------------------------------------------------------------

/// The XSTS `XErr` codes, as the official launcher explains them.
fn xsts_error(code: u64) -> String {
    match code {
        2148916233 => "This Microsoft account has no Xbox profile yet. Sign in once at xbox.com to create one, then try again.".into(),
        2148916235 => "Xbox Live is not available in your country or region.".into(),
        2148916236 | 2148916237 => "This account needs adult verification on xbox.com before it can play.".into(),
        2148916238 => "This is a child account: an adult has to add it to a Microsoft family first.".into(),
        other => format!("Xbox Live refused the sign-in (error {other})."),
    }
}

/// The Minecraft access token from the Microsoft one, through Xbox Live and XSTS.
struct Minecraft {
    token: String,
    expires_at: u64,
    xuid: Option<String>,
}

fn minecraft_login(ep: &Endpoints, ms_access: &str) -> Result<Minecraft, String> {
    let (status, xbl) = send(
        request_to("POST", &ep.xbox_user),
        Some(Body::Json(json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    let xbl_token = str_field(&xbl, "Token")
        .filter(|_| status == 200)
        .ok_or("Xbox Live refused the sign-in.")?;
    let (status, xsts) = send(
        request_to("POST", &ep.xsts),
        Some(Body::Json(json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbl_token] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        }))),
        "Xbox Live",
    )?;
    if let Some(code) = xsts.get("XErr").and_then(Value::as_u64) {
        return Err(xsts_error(code));
    }
    let claims = xsts.pointer("/DisplayClaims/xui/0");
    let (Some(xsts_token), Some(uhs)) = (
        str_field(&xsts, "Token").filter(|_| status == 200),
        claims.and_then(|c| str_field(c, "uhs")),
    ) else {
        return Err("Xbox Live refused the sign-in.".into());
    };
    let xuid = claims.and_then(|c| str_field(c, "xid"));
    let (token, expires_in) = minecraft_token(ep, &format!("XBL3.0 x={uhs};{xsts_token}"))?;
    Ok(Minecraft {
        token,
        expires_at: now() + expires_in,
        xuid,
    })
}

/// The Minecraft access token and its lifetime from the XSTS one (`XBL3.0 x=<uhs>;<token>`).
/// `launcher/login` first: a new account's token from `login_with_xbox` names no profile, so
/// joining an online-mode server answers "Profile not found" (found and diagnosed by
/// martipops). `login_with_xbox` stays the fallback, so a sign-in that worked before still does
/// when the other refuses.
fn minecraft_token(ep: &Endpoints, xtoken: &str) -> Result<(String, u64), String> {
    let lifetime = |v: &Value| v.get("expires_in").and_then(Value::as_u64).unwrap_or(86400);
    let launcher = send(
        request_to("POST", &format!("{}/launcher/login", ep.minecraft)),
        Some(Body::Json(
            json!({ "xtoken": xtoken, "platform": "PC_LAUNCHER" }),
        )),
        "the Minecraft services",
    );
    if let Ok((200, v)) = &launcher {
        if let Some(token) = str_field(v, "access_token") {
            return Ok((token, lifetime(v)));
        }
    }
    let (status, mc) = send(
        request_to(
            "POST",
            &format!("{}/authentication/login_with_xbox", ep.minecraft),
        ),
        Some(Body::Json(json!({ "identityToken": xtoken }))),
        "the Minecraft services",
    )?;
    match (status, str_field(&mc, "access_token")) {
        (200, Some(token)) => Ok((token, lifetime(&mc))),
        // New Azure apps are refused until Mojang approves them (Minecraft API access).
        (403, _) => Err(
            "The Minecraft services refused this launcher: its app is not approved by Mojang yet."
                .into(),
        ),
        (429, _) => Err("Too many sign-ins in a short time. Wait a minute and try again.".into()),
        _ => Err("The Minecraft services refused the sign-in.".into()),
    }
}

fn bearer(request: ureq::Request, token: &str) -> ureq::Request {
    request.set("Authorization", &format!("Bearer {token}"))
}

/// Whether the account owns Minecraft: Java Edition (Game Pass included).
fn owns_game(ep: &Endpoints, token: &str) -> Result<bool, String> {
    let (status, v) = send(
        bearer(
            request_to("GET", &format!("{}/entitlements/mcstore", ep.minecraft)),
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if status != 200 {
        return Err("Could not check which games this account owns.".into());
    }
    Ok(v.get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("name").and_then(Value::as_str),
                    Some("game_minecraft" | "product_minecraft")
                )
            })
        }))
}

/// A Minecraft profile, as the launcher shows and remembers it (no secrets).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Account {
    /// The profile UUID without dashes.
    pub uuid: String,
    pub name: String,
    /// The active skin's URL (textures.minecraft.net) and model ("classic" or "slim").
    #[serde(default)]
    pub skin: Option<(String, String)>,
    #[serde(default)]
    pub cape: Option<String>,
}

/// The profile; none when the account has none (it doesn't own the game, or never made one).
fn profile(ep: &Endpoints, token: &str) -> Result<Option<Account>, String> {
    let (status, v) = send(
        bearer(
            request_to("GET", &format!("{}/minecraft/profile", ep.minecraft)),
            token,
        ),
        None,
        "the Minecraft services",
    )?;
    if status == 404 {
        return Ok(None);
    }
    let (Some(uuid), Some(name)) = (str_field(&v, "id"), str_field(&v, "name")) else {
        return Err("Could not read the Minecraft profile.".into());
    };
    let active = |key: &str| {
        v.get(key)
            .and_then(Value::as_array)
            .and_then(|list| {
                list.iter()
                    .find(|s| s.get("state").and_then(Value::as_str) == Some("ACTIVE"))
            })
            .cloned()
    };
    Ok(Some(Account {
        uuid: uuid.replace('-', "").to_lowercase(),
        name,
        skin: active("skins").and_then(|s| {
            Some((
                str_field(&s, "url")?,
                str_field(&s, "variant")
                    .unwrap_or_else(|| "CLASSIC".into())
                    .to_lowercase(),
            ))
        }),
        cape: active("capes").and_then(|c| str_field(&c, "url")),
    }))
}

/// What the keychain keeps for one account.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secrets {
    refresh_token: String,
    minecraft_token: String,
    minecraft_expires_at: u64,
    #[serde(default)]
    xuid: Option<String>,
    /// The last answer the Minecraft services gave: whether the account owns the game. It is
    /// kept here, beside the tokens, so that no plain file can say yes. Accounts saved before it
    /// existed passed the same check at sign-in, which refuses everyone else.
    #[serde(default = "owned_at_sign_in")]
    owns: bool,
}
fn owned_at_sign_in() -> bool {
    true
}
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secrets(<hidden>)")
    }
}

/// From Microsoft tokens to a ready account: Xbox, XSTS, Minecraft, ownership, profile.
pub fn finish_sign_in(ep: &Endpoints, ms: MsTokens) -> Result<(Account, Secrets), String> {
    let mc = minecraft_login(ep, &ms.access)?;
    if !owns_game(ep, &mc.token)? {
        return Err(DOES_NOT_OWN.into());
    }
    let account = profile(ep, &mc.token)?.ok_or(NO_PROFILE)?;
    Ok((account, Secrets::new(ms.refresh, mc)))
}

impl Secrets {
    fn new(refresh_token: String, mc: Minecraft) -> Self {
        Self {
            refresh_token,
            minecraft_token: mc.token,
            minecraft_expires_at: mc.expires_at,
            xuid: mc.xuid,
            owns: true,
        }
    }
}

/// What asking the Minecraft services again found for one account.
enum Check {
    /// It owns the game: its profile as it is now (the name may have changed) and its tokens.
    Owns(Account, Secrets),
    /// The services said it doesn't own the game (or has no profile any more).
    DoesNot(Secrets),
    /// No answer (no network, a sign-in that can't be renewed): the last answer stands.
    NoAnswer(String, Secrets),
}

/// Asks whether `account` owns the game, with a Minecraft token renewed through the refresh token
/// first when it has less than an hour left. The secrets come back renewed whatever the answer.
fn check(ep: &Endpoints, client_id: Option<&str>, account: &Account, secrets: Secrets) -> Check {
    let mut secrets = secrets;
    if secrets.minecraft_expires_at <= now() + RENEW_BEFORE {
        let renewed = client_id
            .ok_or_else(|| NOT_CONFIGURED.to_string())
            .and_then(|id| refresh_ms(ep, id, &secrets.refresh_token))
            .and_then(|ms| Ok((minecraft_login(ep, &ms.access)?, ms.refresh)));
        match renewed {
            Ok((mc, refresh)) => {
                secrets = Secrets {
                    owns: secrets.owns,
                    ..Secrets::new(refresh, mc)
                }
            }
            Err(e) => return Check::NoAnswer(e, secrets),
        }
    }
    let token = &secrets.minecraft_token;
    match owns_game(ep, token).and_then(|owns| Ok(if owns { profile(ep, token)? } else { None })) {
        Err(e) => Check::NoAnswer(e, secrets),
        Ok(None) => Check::DoesNot(Secrets {
            owns: false,
            ..secrets
        }),
        Ok(Some(p)) if p.uuid != account.uuid => Check::NoAnswer(OTHER_PROFILE.into(), secrets),
        Ok(Some(p)) => Check::Owns(
            p,
            Secrets {
                owns: true,
                ..secrets
            },
        ),
    }
}

// ----- The keychain --------------------------------------------------------------------------------

/// Where secrets are kept: the OS keychain, or memory in tests and when there is none.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    /// False for memory: secrets are gone when the launcher closes.
    fn persistent(&self) -> bool;
}

#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.into(), secret.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
    fn persistent(&self) -> bool {
        false
    }
}

/// The Secret Service (GNOME Keyring, KWallet) through oo7, as gpui itself uses it.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub struct SystemStore;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl SystemStore {
    fn keyring() -> Result<oo7::Keyring, String> {
        futures_lite::future::block_on(async {
            // `oo7::Keyring::new` takes `GTK_USE_PORTAL=1` (set by many KDE desktops) for a
            // sandbox, and then keeps the secrets in a file under a key the Secret portal gives
            // per app id. Outside a sandbox that id comes from how the launcher was started (the
            // menu or a terminal), so the file opened in one start and not the next (#3249).
            // Flatpak (`/.flatpak-info`) and Snap (`$SNAP`) keep the portal's file.
            let sandboxed =
                Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some();
            let keyring = if sandboxed {
                oo7::Keyring::new().await.map_err(|e| e.to_string())?
            } else {
                let service = oo7::dbus::Service::new().await.map_err(|e| e.to_string())?;
                oo7::Keyring::DBus(
                    service
                        .default_collection()
                        .await
                        .map_err(|e| e.to_string())?,
                )
            };
            keyring.unlock().await.map_err(|e| e.to_string())?;
            Ok(keyring)
        })
    }
}
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl SecretStore for SystemStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            let items = keyring
                .search_items(&attributes)
                .await
                .map_err(|e| e.to_string())?;
            let Some(item) = items.into_iter().next() else {
                return Ok(None);
            };
            item.unlock().await.map_err(|e| e.to_string())?;
            let secret = item.secret().await.map_err(|e| e.to_string())?;
            Ok(String::from_utf8(secret.to_vec()).ok())
        })
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            keyring
                .create_item(
                    "Horizon launcher: Minecraft account",
                    &attributes,
                    secret.as_bytes(),
                    true,
                )
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        let keyring = Self::keyring()?;
        futures_lite::future::block_on(async {
            let attributes = vec![("service", SERVICE), ("account", key)];
            keyring.delete(&attributes).await.map_err(|e| e.to_string())
        })
    }
    fn persistent(&self) -> bool {
        true
    }
}

/// Keychain on macOS, Credential Manager on Windows (the keyring crate). Credential Manager takes at
/// most 2560 bytes of UTF-16 per item and a sign-in's secrets are longer, so a secret is kept in
/// pieces: the first under the account's key, the rest under `key.1`, `key.2`...
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub struct SystemStore;

/// Bytes of UTF-8 per keychain item: at most twice as many in UTF-16, well under 2560.
#[cfg(any(target_os = "macos", target_os = "windows"))]
const PIECE: usize = 1000;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn piece_key(key: &str, n: usize) -> String {
    if n == 0 {
        key.to_string()
    } else {
        format!("{key}.{n}")
    }
}

/// `secret` cut into pieces of at most `PIECE` bytes, on character boundaries.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn split_secret(mut secret: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    while secret.len() > PIECE {
        let mut cut = PIECE;
        while !secret.is_char_boundary(cut) {
            cut -= 1;
        }
        let (head, tail) = secret.split_at(cut);
        pieces.push(head);
        secret = tail;
    }
    pieces.push(secret);
    pieces
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn read_item(key: &str) -> Result<Option<String>, String> {
    match keyring::Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn write_item(key: &str, secret: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, key)
        .and_then(|e| e.set_password(secret))
        .map_err(|e| e.to_string())
}

/// Removes one item; false when there was none.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn remove_item(key: &str) -> Result<bool, String> {
    match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl SecretStore for SystemStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let Some(mut secret) = read_item(key)? else {
            return Ok(None);
        };
        for n in 1.. {
            match read_item(&piece_key(key, n))? {
                Some(piece) => secret.push_str(&piece),
                None => break,
            }
        }
        Ok(Some(secret))
    }
    fn set(&self, key: &str, secret: &str) -> Result<(), String> {
        let pieces = split_secret(secret);
        for (n, piece) in pieces.iter().enumerate() {
            write_item(&piece_key(key, n), piece)?;
        }
        // Pieces left over from a longer secret would be read back as part of this one.
        for n in pieces.len().. {
            if !remove_item(&piece_key(key, n))? {
                break;
            }
        }
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        for n in 0.. {
            if !remove_item(&piece_key(key, n))? {
                break;
            }
        }
        Ok(())
    }
    fn persistent(&self) -> bool {
        true
    }
}

pub const NO_KEYCHAIN: &str = "No keychain available: you are signed in until the launcher closes, then you will have to sign in again.";

/// What the keychain answered when [`store`] fell back to memory.
static NO_KEYCHAIN_WHY: OnceLock<String> = OnceLock::new();

/// What the window and the CLI say when sign-ins last only this run: [`NO_KEYCHAIN`] and why.
/// None while the keychain keeps them.
pub fn no_keychain() -> Option<String> {
    if store().persistent() {
        return None;
    }
    Some(match NO_KEYCHAIN_WHY.get() {
        Some(why) => format!("{NO_KEYCHAIN} The keychain said: {why}"),
        None => NO_KEYCHAIN.into(),
    })
}

/// The keychain if it answers, else memory for this run (said in the UI with [`no_keychain`]).
/// Never a file of its own: secrets are kept by the system or not at all.
pub fn store() -> &'static dyn SecretStore {
    static STORE: OnceLock<Box<dyn SecretStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            // Unit tests never reach the player's keychain.
            #[cfg(all(
                not(test),
                any(
                    target_os = "linux",
                    target_os = "freebsd",
                    target_os = "macos",
                    target_os = "windows"
                )
            ))]
            {
                let why = match std::env::var_os("HORIZON_NO_KEYCHAIN") {
                    Some(_) => "HORIZON_NO_KEYCHAIN is set.".to_string(),
                    None => match SystemStore.get("probe") {
                        Ok(_) => return Box::new(SystemStore),
                        Err(e) => e,
                    },
                };
                let _ = NO_KEYCHAIN_WHY.set(why);
            }
            Box::<MemoryStore>::default()
        })
        .as_ref()
}

// ----- Accounts ------------------------------------------------------------------------------------

/// accounts.json: the signed-in accounts and the one that plays. Never a token.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Accounts {
    pub accounts: Vec<Account>,
    /// The UUID of the account the game starts with; none plays offline.
    pub selected: Option<String>,
    /// Offline names the player keeps, to switch between (`--name`): no account, no skin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offline: Vec<String>,
    /// The offline name that plays when no account is selected; none: the name the game made up
    /// (`horizon-player.txt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_selected: Option<String>,
    /// Each offline name's skin and cape (`skins`), by the name in lowercase.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub looks: std::collections::BTreeMap<String, crate::skins::Look>,
}

/// Who plays, as the quick switch lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Player {
    /// A Microsoft account, by UUID.
    Microsoft(String),
    /// An offline name.
    Offline(String),
    /// Offline as the name the game made up.
    GameName,
}

/// The secrets an account keeps in `store`, if it still has them there.
fn secrets_in(store: &dyn SecretStore, uuid: &str) -> Option<Secrets> {
    store
        .get(uuid)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Whether `name` can be an offline name: 3 to 16 letters, digits or `_`, as account names are.
pub fn valid_offline_name(name: &str) -> bool {
    (3..=16).contains(&name.chars().count())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl Accounts {
    fn path() -> PathBuf {
        crate::backend::data_dir().join("accounts.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|e| e.to_string())
    }
    pub fn selected(&self) -> Option<&Account> {
        let uuid = self.selected.as_ref()?;
        self.accounts.iter().find(|a| &a.uuid == uuid)
    }
    /// Find an account by name (any case) or UUID (with or without dashes).
    pub fn find(&self, who: &str) -> Option<&Account> {
        let undashed = who.replace('-', "").to_lowercase();
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(who) || a.uuid == undashed)
    }
    /// Keep a new or renewed account, its secrets in the store, and select it.
    pub fn add(
        &mut self,
        store: &dyn SecretStore,
        account: Account,
        secrets: &Secrets,
    ) -> Result<(), String> {
        store.set(
            &account.uuid,
            &serde_json::to_string(secrets).map_err(|e| e.to_string())?,
        )?;
        self.selected = Some(account.uuid.clone());
        match self.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
            Some(a) => *a = account,
            None => self.accounts.push(account),
        }
        Ok(())
    }
    /// Whether an account here owned the game at its last check (the keychain's answer, no
    /// request): what Play and offline names need.
    pub fn has_owner(&self, store: &dyn SecretStore) -> bool {
        self.accounts
            .iter()
            .any(|a| secrets_in(store, &a.uuid).is_some_and(|s| s.owns))
    }

    /// Who plays now.
    pub fn player(&self) -> Player {
        match (self.selected(), &self.offline_selected) {
            (Some(a), _) => Player::Microsoft(a.uuid.clone()),
            (None, Some(name)) => Player::Offline(name.clone()),
            (None, None) => Player::GameName,
        }
    }

    /// Plays as `who`: an account, an offline name (kept, no account that owns the game needed),
    /// or the game's own name.
    pub fn play_as(&mut self, store: &dyn SecretStore, who: Player) -> Result<(), String> {
        self.play_as_with(store, who, false)
    }

    /// [`Self::play_as`], where `offline_without_owner` (the window's hidden developer option,
    /// `backend::Settings::dev_offline`) is accepted but does nothing here: this fork plays
    /// offline names without an account that owns the game, option or no option.
    pub fn play_as_with(
        &mut self,
        _store: &dyn SecretStore,
        who: Player,
        _offline_without_owner: bool,
    ) -> Result<(), String> {
        match who {
            Player::Microsoft(uuid) => {
                if !self.accounts.iter().any(|a| a.uuid == uuid) {
                    return Err("That account isn't signed in.".into());
                }
                self.selected = Some(uuid);
            }
            Player::Offline(name) => {
                // This fork plays without a Microsoft account: offline names need no owner.
                let name = self.add_offline(&name)?;
                self.selected = None;
                self.offline_selected = Some(name);
            }
            Player::GameName => {
                self.selected = None;
                self.offline_selected = None;
            }
        }
        Ok(())
    }

    /// Keeps an offline name (once, in any case); returns it as kept.
    fn add_offline(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if !valid_offline_name(name) {
            return Err(format!(
                "{name}: an offline name is 3 to 16 letters, digits or _"
            ));
        }
        if let Some(kept) = self.offline.iter().find(|n| n.eq_ignore_ascii_case(name)) {
            return Ok(kept.clone());
        }
        self.offline.push(name.to_string());
        Ok(name.to_string())
    }

    /// Forgets an offline name: its look and its skin and cape files (`skins`) go with it; if
    /// it was playing, the game's own name plays.
    pub fn remove_offline(&mut self, name: &str, skins_dir: &Path) -> Result<(), String> {
        let before = self.offline.len();
        self.offline.retain(|n| !n.eq_ignore_ascii_case(name));
        self.looks.remove(&name.to_ascii_lowercase());
        if self.offline.len() == before {
            return Err(format!("No offline name {name}."));
        }
        crate::skins::forget(skins_dir, name);
        if self
            .offline_selected
            .as_ref()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            self.offline_selected = None;
        }
        Ok(())
    }

    /// An offline name's look (the default: no skin of its own).
    pub fn look(&self, name: &str) -> crate::skins::Look {
        self.looks
            .get(&name.to_ascii_lowercase())
            .cloned()
            .unwrap_or_default()
    }

    /// The game's variables for the offline name that plays: its skin and cape.
    pub fn look_env(&self) -> Vec<(&'static str, String)> {
        match self.name_arg() {
            Some(name) => crate::skins::env(&crate::skins::dir(), name, &self.look(name)),
            None => Vec::new(),
        }
    }

    /// The game's `--name` for this run: the offline name, when one plays.
    pub fn name_arg(&self) -> Option<&str> {
        match self.selected() {
            Some(_) => None,
            None => self.offline_selected.as_deref(),
        }
    }

    /// A renewed profile (its name or skin changed) in place of the old one; who plays stays.
    fn update(&mut self, account: Account) {
        if let Some(a) = self.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
            *a = account;
        }
    }

    /// Sign out: the secrets leave the keychain, the account the list. Offline names stay, but
    /// play only once an account that owns the game is back.
    pub fn remove(&mut self, store: &dyn SecretStore, uuid: &str) -> Result<(), String> {
        store.delete(uuid)?;
        self.accounts.retain(|a| a.uuid != uuid);
        if self.selected.as_deref() == Some(uuid) {
            self.selected = None;
        }
        Ok(())
    }
}

/// No account in the launcher owns Minecraft: Java Edition: the game must not start.
#[derive(Debug, PartialEq)]
pub struct NoOwner;
impl std::fmt::Display for NoOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{NEEDS_OWNER}.")
    }
}

/// The owner gate's pass for one launch.
#[derive(Debug, Default)]
pub struct Launch {
    /// The selected account and its renewed session; none when an offline name plays, or when
    /// its sign-in couldn't be renewed now (`offline` says why).
    pub session: Option<(Account, Secrets)>,
    /// Why the selected account plays without its session this time.
    pub offline: Option<String>,
    /// An account the services said owns the game just now, the selected one first: who the
    /// launcher may download the game for.
    pub owner: Option<(Account, Secrets)>,
    /// A renewed sign-in the keychain refused to keep: it plays now, and may have to sign in
    /// again next time.
    pub unsaved: Option<String>,
}

/// The last [`Launch::unsaved`], for the window or the terminal to show once.
static UNSAVED: Mutex<Option<String>> = Mutex::new(None);

/// What the last launch couldn't save in the keychain, once.
pub fn take_unsaved() -> Option<String> {
    UNSAVED.lock().ok()?.take()
}

/// The owner gate: the game starts only while an account in the launcher owns Minecraft: Java
/// Edition. The accounts are asked again, the selected one first, until one owns it; each keeps
/// the answer in the keychain. An account that gets no answer (no network) keeps its last one, so
/// being offline never blocks a player who passed before, while "doesn't own" blocks at once.
/// The caller saves `accounts` (renewed names and skins).
pub fn launch(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
) -> Result<Launch, NoOwner> {
    let selected = accounts.selected.clone();
    let mut order = accounts.accounts.clone();
    order.sort_by_key(|a| Some(&a.uuid) != selected.as_ref());
    let mut pass = Launch::default();
    let mut owned = false;
    for account in order {
        if owned {
            break;
        }
        let plays = Some(&account.uuid) == selected.as_ref();
        let Some(secrets) = secrets_in(store, &account.uuid) else {
            if plays {
                pass.offline = Some(format!(
                    "{} is no longer signed in on this computer. Sign in again.",
                    account.name
                ));
            }
            continue;
        };
        let before = serde_json::to_string(&secrets).ok();
        let (said, secrets) = match check(ep, client_id, &account, secrets) {
            Check::Owns(renewed, secrets) => {
                accounts.update(renewed.clone());
                if plays {
                    pass.session = Some((renewed.clone(), secrets.clone()));
                }
                pass.owner = Some((renewed, secrets.clone()));
                (None, secrets)
            }
            Check::DoesNot(secrets) => (Some(DOES_NOT_OWN.to_string()), secrets),
            Check::NoAnswer(e, secrets) => (Some(e), secrets),
        };
        owned = secrets.owns;
        if plays {
            pass.offline = said;
        }
        let after = serde_json::to_string(&secrets).ok();
        if after != before {
            if let Some(text) = after {
                // The game still starts; the player hears why the next launch may ask again.
                if let Err(e) = store.set(&account.uuid, &text) {
                    let said = format!(
                        "{}'s renewed sign-in couldn't be saved in the keychain ({e}): you may \
                         have to sign in again next time.",
                        account.name
                    );
                    if let Ok(mut last) = UNSAVED.lock() {
                        *last = Some(said.clone());
                    }
                    pass.unsaved = Some(said);
                }
            }
        }
    }
    // This fork plays without a Microsoft account: no owner is not a refusal, the game starts
    // offline. The note only when nothing else already says why the run has no session.
    if !owned && pass.offline.is_none() {
        pass.offline =
            Some("Playing offline: no Microsoft account that owns Minecraft is signed in.".into());
    }
    Ok(pass)
}

/// The owner gate before the game starts. With the window's hidden developer option
/// (`backend::Settings::dev_offline`, `offline_without_owner`) no owner is no refusal: the game
/// starts offline, without a session and without an `owner`. Downloads never come through here:
/// `backend::download_for_owner` asks [`launch`] itself, so the option never downloads the game.
pub fn launch_for_game(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
    offline_without_owner: bool,
) -> Result<Launch, NoOwner> {
    match launch(ep, client_id, store, accounts) {
        Err(NoOwner) if offline_without_owner => Ok(Launch::default()),
        pass => pass,
    }
}

/// Write the session for the game, readable by this user only, in a fresh file it deletes
/// when it reads it.
pub fn write_session_file(
    dir: &Path,
    account: &Account,
    secrets: &Secrets,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Leftovers of games that never started (refused by the game after two minutes anyway).
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| m.elapsed().is_ok_and(|age| age > Duration::from_secs(120)));
            if old {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut name = [0u8; 12];
    getrandom::fill(&mut name).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "session-{}.json",
        name.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    let body = json!({
        "name": account.name,
        "uuid": account.uuid,
        "access_token": secrets.minecraft_token,
        "user_type": "msa",
        "xuid": secrets.xuid,
    });
    std::io::Write::write_all(&mut file, body.to_string().as_bytes()).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Where a background sign-in is (the window and the MCP server show it).
#[derive(Clone, Debug)]
pub enum SignInState {
    Starting,
    /// Waiting for the player at `verification_uri` with `user_code`.
    Code(DeviceCode),
    Done(Result<Account, String>),
}

/// A sign-in on its own thread, from the code to the saved account.
pub struct SignIn {
    state: std::sync::Arc<Mutex<SignInState>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SignIn {
    pub fn start() -> Self {
        use std::sync::atomic::Ordering;
        let state = std::sync::Arc::new(Mutex::new(SignInState::Starting));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (thread_state, thread_cancel) = (state.clone(), cancel.clone());
        std::thread::spawn(move || {
            let set = |s| *thread_state.lock().unwrap() = s;
            let result = (|| {
                let ep = Endpoints::live();
                let client_id = client_id().ok_or(NOT_CONFIGURED)?;
                let code = device_code(&ep, &client_id)?;
                set(SignInState::Code(code.clone()));
                let ms = wait_for_sign_in(&ep, &client_id, &code, &|| {
                    thread_cancel.load(Ordering::Relaxed)
                })?;
                let (account, secrets) = finish_sign_in(&ep, ms)?;
                let mut accounts = Accounts::load();
                accounts.add(store(), account.clone(), &secrets)?;
                accounts.save()?;
                Ok(account)
            })();
            set(SignInState::Done(result));
        });
        Self { state, cancel }
    }
    pub fn state(&self) -> SignInState {
        self.state.lock().unwrap().clone()
    }
    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The owner gate for the game about to start, then the selected account's session in a file
/// for it: the file (none: offline) and why the account plays offline this time, if it does.
/// `offline_without_owner`: see [`launch_for_game`].
pub fn session_for_game(
    game_home: &Path,
    offline_without_owner: bool,
) -> Result<(Option<PathBuf>, Option<String>), NoOwner> {
    let mut accounts = Accounts::load();
    let pass = launch_for_game(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &mut accounts,
        offline_without_owner,
    )?;
    let _ = accounts.save();
    let Some((account, secrets)) = pass.session else {
        return Ok((None, pass.offline));
    };
    match write_session_file(&game_home.join("sessions"), &account, &secrets) {
        Ok(file) => Ok((Some(file), None)),
        Err(e) => Ok((None, Some(e))),
    }
}

/// Who plays the real Minecraft: Java Edition (a Java instance), past the owner gate itself
/// ([`launch`]): the selected account when it owns the game, else one that does. With no answer
/// from the services, an account that owned it plays with its last token. Mojang's game takes the
/// token on its command line, as Mojang's launcher gives it; that command is never logged.
pub fn java_player() -> Result<crate::java_launch::Player, String> {
    let mut accounts = Accounts::load();
    let player = java_player_with(
        &Endpoints::live(),
        client_id().as_deref(),
        store(),
        &mut accounts,
    );
    let _ = accounts.save();
    player
}

fn java_player_with(
    ep: &Endpoints,
    client_id: Option<&str>,
    store: &dyn SecretStore,
    accounts: &mut Accounts,
) -> Result<crate::java_launch::Player, String> {
    let pass = launch(ep, client_id, store, accounts).map_err(|e| e.to_string())?;
    let kept = || {
        let order = accounts.selected().into_iter().chain(&accounts.accounts);
        order
            .into_iter()
            .find_map(|a| Some((a.clone(), secrets_in(store, &a.uuid).filter(|s| s.owns)?)))
    };
    let (account, secrets) = pass
        .session
        .or(pass.owner)
        .or_else(kept)
        .ok_or_else(|| NoOwner.to_string())?;
    Ok(crate::java_launch::Player {
        name: account.name,
        uuid: account.uuid,
        token: secrets.minecraft_token,
        xuid: secrets.xuid.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests;
