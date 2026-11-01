//! The Microsoft account the launcher signed in, handed over in a session
//! file (docs/microsoft-account.md), and the Minecraft services call
//! online-mode servers need: authlib's MinecraftServicesSessionService.joinServer
//! through the services discovery document. The access token stays in this
//! process's memory: it is never logged, printed or written anywhere.
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime},
};

/// MinecraftServicesEnvironment.PROD.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";
/// A session file older than this is refused: it can't be replayed later.
const MAX_SESSION_FILE_AGE: Duration = Duration::from_secs(120);

/// A signed-in account (vanilla's User with type msa).
pub struct AuthSession {
    pub name: String,
    pub uuid: u128,
    pub xuid: Option<String>,
    access_token: String,
}
impl std::fmt::Debug for AuthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthSession")
            .field("name", &self.name)
            .field("uuid", &horizon_protocol::crypto::uuid_string(self.uuid))
            .field("access_token", &"<hidden>")
            .finish()
    }
}

/// A UUID with or without dashes.
pub fn parse_uuid(text: &str) -> Option<u128> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
}

impl AuthSession {
    /// For tests and harnesses.
    pub fn new(name: &str, uuid: u128, access_token: &str) -> Self {
        Self {
            name: name.to_owned(),
            uuid,
            xuid: None,
            access_token: access_token.to_owned(),
        }
    }
    /// The session file's JSON: {"name", "uuid", "access_token", "user_type": "msa", "xuid"}.
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|_| "the session is not valid JSON")?;
        let field = |k: &str| v.get(k).and_then(|s| s.as_str());
        let name = field("name").ok_or("the session has no name")?;
        if name.is_empty() || name.chars().count() > 16 {
            return Err("the session's name is not a valid player name".into());
        }
        let uuid = field("uuid")
            .and_then(parse_uuid)
            .ok_or("the session has no valid uuid")?;
        let access_token = field("access_token")
            .filter(|t| !t.is_empty())
            .ok_or("the session has no access token")?;
        if field("user_type").is_some_and(|t| t != "msa") {
            return Err("the session is not a Microsoft account session".into());
        }
        Ok(Self {
            name: name.to_owned(),
            uuid,
            xuid: field("xuid").map(str::to_owned),
            access_token: access_token.to_owned(),
        })
    }
    /// Read the session file the launcher wrote and delete it at once. It
    /// must be fresh and, on Unix, readable by its owner only.
    pub fn load(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let refuse = |why: &str| {
            let _ = std::fs::remove_file(path);
            Err(format!("{}: {why}", path.display()))
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return refuse("other users can read it");
            }
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or_default();
        if age > MAX_SESSION_FILE_AGE {
            return refuse("it is too old (start the game from the launcher)");
        }
        let text = std::fs::read_to_string(path);
        let _ = std::fs::remove_file(path);
        Self::parse(&text.map_err(|e| format!("{}: {e}", path.display()))?)
    }
}

static CURRENT: OnceLock<Arc<AuthSession>> = OnceLock::new();

/// The account this game was started with (set once at start).
pub fn set_current(session: AuthSession) {
    let _ = CURRENT.set(Arc::new(session));
}
pub fn current() -> Option<Arc<AuthSession>> {
    CURRENT.get().cloned()
}

/// Why joining a server through the session service failed, like the
/// AuthenticationException subclasses authlib throws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinError {
    /// No answer, or a 5xx (AuthenticationUnavailableException).
    Unavailable,
    /// 401 or ForbiddenOperationException (InvalidCredentialsException).
    InvalidSession,
    /// InsufficientPrivilegesException: multiplayer is off for the account.
    InsufficientPrivileges,
    /// multiplayer.access.banned or FORCED_USERNAME_CHANGE.
    Banned,
    /// Anything else, with the service's message.
    Other(String),
}
impl JoinError {
    /// MinecraftClientHttpException.toAuthenticationException.
    pub fn from_response(status: u16, body: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let error = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
        match error {
            "ForbiddenOperationException" => return Self::InvalidSession,
            "multiplayer.access.banned" | "FORCED_USERNAME_CHANGE" => return Self::Banned,
            "InsufficientPrivilegesException" => return Self::InsufficientPrivileges,
            _ => {}
        }
        match status {
            401 => Self::InvalidSession,
            500.. => Self::Unavailable,
            _ => Self::Other(
                v.get("errorMessage")
                    .and_then(|m| m.as_str())
                    .map_or_else(|| format!("Status: {status}"), str::to_owned),
            ),
        }
    }
    /// ClientHandshakePacketListenerImpl.authenticateServer's message: the
    /// key of the reason inside "disconnect.loginFailedInfo", or its text.
    pub fn reason(&self) -> Result<&'static str, &str> {
        match self {
            Self::Unavailable => Ok("disconnect.loginFailedInfo.serversUnavailable"),
            Self::InvalidSession => Ok("disconnect.loginFailedInfo.invalidSession"),
            Self::InsufficientPrivileges => Ok("disconnect.loginFailedInfo.insufficientPrivileges"),
            Self::Banned => Ok("disconnect.loginFailedInfo.userBanned"),
            Self::Other(message) => Err(message),
        }
    }
}

/// The services discovery document URL: `HORIZON_API_DISCOVERY` overrides
/// it like authlib's `-Dminecraft.api.discovery.host`.
pub fn discovery_url() -> String {
    std::env::var("HORIZON_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_DISCOVERY.to_owned())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build()
}

/// The discovery document, fetched once per game (a failure is retried next time).
pub(crate) fn discovery() -> Result<Arc<serde_json::Value>, JoinError> {
    static CACHE: Mutex<Option<Arc<serde_json::Value>>> = Mutex::new(None);
    if let Some(d) = CACHE.lock().unwrap().clone() {
        return Ok(d);
    }
    let doc: serde_json::Value = agent()
        .get(&discovery_url())
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or(JoinError::Unavailable)?;
    let doc = Arc::new(doc);
    *CACHE.lock().unwrap() = Some(doc.clone());
    Ok(doc)
}

/// MinecraftServicesDiscoveryService.getUrl(service, endpoint).
pub fn endpoint(doc: &serde_json::Value, service: &str, name: &str) -> Option<String> {
    doc.pointer(&format!("/discovery/{service}/endpoints/{name}/uri"))
        .and_then(|u| u.as_str())
        .map(str::to_owned)
}

/// `Minecraft`'s constructor futures for the title screen: `UserApiService.fetchProperties`
/// (the `player` service's `getAttributes`, with the bearer token) and
/// `SessionService.fetchProfile(uuid, true)` (`getProfileById`). A failed
/// attributes call falls back to the offline properties, a failed profile call
/// to no actions, as vanilla does. Blocks: call it off the main thread.
pub fn fetch_account_status(session: &AuthSession) -> crate::ban_notice::AccountStatus {
    use crate::ban_notice::{AccountStatus, parse_profile_actions, parse_properties};
    let Ok(doc) = discovery() else {
        return AccountStatus::default();
    };
    let properties = endpoint(&doc, "player", "getAttributes")
        .and_then(|url| {
            agent()
                .get(&url)
                .set("Authorization", &format!("Bearer {}", session.access_token))
                .call()
                .ok()?
                .into_string()
                .ok()
        })
        .and_then(|body| parse_properties(&body))
        .unwrap_or_default();
    let actions = endpoint(&doc, "session", "getProfileById")
        .and_then(|url| {
            let url = url.replace("{profileId}", &format!("{:032x}", session.uuid));
            agent()
                .get(&format!("{url}?unsigned=true"))
                .call()
                .ok()?
                .into_string()
                .ok()
        })
        .map(|body| parse_profile_actions(&body))
        .unwrap_or_default();
    AccountStatus {
        properties,
        actions,
    }
}

/// The JoinMinecraftServerRequest body; `selectedProfile` is the undashed UUID.
pub fn join_request(session: &AuthSession, server_hash: &str) -> String {
    serde_json::json!({
        "accessToken": session.access_token,
        "selectedProfile": format!("{:032x}", session.uuid),
        "serverId": server_hash,
    })
    .to_string()
}

/// MinecraftServicesSessionService.joinServer: tell the session service this
/// account is joining the server whose hash is `server_hash`.
pub fn join_server(session: &AuthSession, server_hash: &str) -> Result<(), JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "session", "join").ok_or(JoinError::Unavailable)?;
    match agent()
        .post(&url)
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&join_request(session, server_hash))
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(status, response)) => Err(JoinError::from_response(
            status,
            &response.into_string().unwrap_or_default(),
        )),
        Err(ureq::Error::Transport(_)) => Err(JoinError::Unavailable),
    }
}

/// The account's chat key pair (ProfileKeyPair): the private key that signs
/// its chat, the public key with the services' signature over it, and when
/// to fetch a new one (refreshedAfter, epoch milliseconds). Like the access
/// token it stays in memory: vanilla's profilekeys file is never written.
pub struct ChatKeyPair {
    pub private: horizon_protocol::rsa::RsaPrivateKey,
    pub public: horizon_protocol::secure_chat::ProfileKey,
    pub refreshed_after: i64,
}
impl std::fmt::Debug for ChatKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatKeyPair")
            .field("expires_at", &self.public.expires_at)
            .field("refreshed_after", &self.refreshed_after)
            .finish_non_exhaustive()
    }
}
impl ChatKeyPair {
    /// ProfileKeyPair.dueRefresh.
    pub fn due_refresh(&self, now: i64) -> bool {
        self.refreshed_after < now
    }
}

/// Epoch milliseconds now.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Instant.parse for the services' UTC times
/// (`2022-07-30T23:05:24.548227Z`): epoch milliseconds.
pub fn parse_instant(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (date.next()??, date.next()??, date.next()??);
    let (hms, fraction) = time.split_once('.').unwrap_or((time, ""));
    let mut hms = hms.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (h, min, s) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 || s > 60 {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis = format!("{fraction:0<3}")[..3].parse::<i64>().ok()?;
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + min) * 60_000 + s * 1000 + millis)
}

/// Crypt.stringToPemRsaPrivateKey / stringToRsaPublicKey: the base64
/// between a PEM block's armour lines.
fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars().filter(|c| !c.is_whitespace()))
        .collect();
    crate::skins::base64_decode(&body)
}

/// KeyPairResponse from the certificates endpoint, as
/// AccountProfileKeyPairManager.fetchProfileKeyPair reads it: the PEM key
/// pair, publicKeySignatureV2 (the signature over the profile id too),
/// expiresAt and refreshedAfter. None for anything malformed, or a private
/// key that isn't the public key's other half.
pub fn parse_certificates(body: &str) -> Option<ChatKeyPair> {
    use horizon_protocol::rsa::{RsaPrivateKey, RsaPublicKey};
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let field = |path: &str| v.pointer(path).and_then(|s| s.as_str());
    let private = RsaPrivateKey::from_der(&pem_der(field("/keyPair/privateKey")?)?).ok()?;
    let key = pem_der(field("/keyPair/publicKey")?)?;
    if RsaPublicKey::from_der(&key).ok()? != private.public {
        return None;
    }
    let signature = crate::skins::base64_decode(field("/publicKeySignatureV2")?)?;
    Some(ChatKeyPair {
        private,
        public: horizon_protocol::secure_chat::ProfileKey {
            expires_at: parse_instant(field("/expiresAt")?)?,
            key,
            signature,
        },
        refreshed_after: parse_instant(field("/refreshedAfter")?)?,
    })
}

/// UserApiService.getKeyPair: the certificates endpoint, POSTed with the
/// access token.
pub fn fetch_chat_keys(session: &AuthSession) -> Result<ChatKeyPair, JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "player", "getCertificates").ok_or(JoinError::Unavailable)?;
    let body = match agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", session.access_token))
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string("")
    {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(status, response)) => {
            return Err(JoinError::from_response(
                status,
                &response.into_string().unwrap_or_default(),
            ));
        }
        Err(ureq::Error::Transport(_)) => return Err(JoinError::Unavailable),
    };
    parse_certificates(&body)
        .ok_or_else(|| JoinError::Other("Malformed profile key pair response".to_owned()))
}

/// AccountProfileKeyPairManager.MINIMUM_PROFILE_KEY_REFRESH_INTERVAL.
const KEY_RETRY: Duration = Duration::from_secs(60 * 60);

/// The chat key pair as AccountProfileKeyPairManager keeps it: fetched on a
/// background thread, kept until it is due a refresh, and on a failure kept
/// as it was (the next try waits an hour, or the next join).
#[derive(Default)]
struct ChatKeys {
    current: Option<Arc<ChatKeyPair>>,
    fetching: bool,
    next_try: Option<std::time::Instant>,
}
static CHAT_KEYS: Mutex<ChatKeys> = Mutex::new(ChatKeys {
    current: None,
    fetching: false,
    next_try: None,
});

/// prepareKeyPair (`now`: on a join) or, from a tick, shouldRefreshKeyPair
/// then prepareKeyPair: start fetching when there is no key pair or it is
/// due a refresh. Nothing without a signed-in account.
pub fn prepare_chat_keys(now: bool) {
    let Some(session) = current() else {
        return;
    };
    let mut keys = CHAT_KEYS.lock().unwrap();
    let instant = std::time::Instant::now();
    if keys.fetching || !now && keys.next_try.is_some_and(|t| instant < t) {
        return;
    }
    keys.next_try = Some(instant + KEY_RETRY);
    if keys
        .current
        .as_ref()
        .is_some_and(|k| !k.due_refresh(now_millis()))
    {
        return;
    }
    keys.fetching = true;
    drop(keys);
    let spawned = std::thread::Builder::new()
        .name("Profile key fetch".into())
        .spawn(move || {
            let result = fetch_chat_keys(&session);
            let mut keys = CHAT_KEYS.lock().unwrap();
            keys.fetching = false;
            match result {
                Ok(pair) => keys.current = Some(Arc::new(pair)),
                Err(e) => log::error!("Failed to retrieve profile key pair: {e:?}"),
            }
        });
    if spawned.is_err() {
        CHAT_KEYS.lock().unwrap().fetching = false;
    }
}

/// The chat key pair fetched so far, unless it has expired.
pub fn chat_keys() -> Option<Arc<ChatKeyPair>> {
    let now = now_millis();
    CHAT_KEYS
        .lock()
        .unwrap()
        .current
        .clone()
        .filter(|k| !k.public.has_expired(now))
}

const MINECRAFT_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The signed-in account's active skin and cape from `GET /minecraft/profile`.
/// Single-player and integrated servers often send no `textures` property, or
/// one without the equipped cape; the client asks the profile endpoint once.
pub fn own_textures(session: &AuthSession) -> Option<crate::skins::Textures> {
    let response = agent()
        .get(MINECRAFT_PROFILE)
        .set("Authorization", &format!("Bearer {}", session.access_token))
        .call()
        .ok()?;
    if response.status() != 200 {
        return None;
    }
    let body = response.into_string().ok()?;
    let profile: serde_json::Value = serde_json::from_str(&body).ok()?;
    Some(crate::skins::services_profile_textures(&profile))
}

/// What the session service says about a player joining this server
/// (MinecraftServicesSessionService.hasJoinedServer).
#[derive(Clone, Debug, PartialEq)]
pub struct JoinedProfile {
    pub uuid: u128,
    pub properties: Vec<horizon_protocol::packets::login::Property>,
}

/// HttpDiscoveryService.buildQuery: `URLEncoder.encode(value, UTF-8)`.
fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                out.push(char::from(b));
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The session service's verify endpoint, asked by an online-mode server
/// whether `name` joined with `server_hash` (and from `ip`, when the server
/// prevents proxy connections). `Ok(None)`: it didn't, or the service said
/// no; `Err(Unavailable)`: no answer.
pub fn has_joined(
    name: &str,
    server_hash: &str,
    ip: Option<std::net::IpAddr>,
) -> Result<Option<JoinedProfile>, JoinError> {
    let doc = discovery()?;
    let url = endpoint(&doc, "session", "verify").ok_or(JoinError::Unavailable)?;
    let mut query = format!(
        "username={}&serverId={}",
        url_encode(name),
        url_encode(server_hash)
    );
    if let Some(ip) = ip {
        query.push_str(&format!("&ip={}", url_encode(&ip.to_string())));
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    let body = match agent().get(&format!("{url}{separator}{query}")).call() {
        Ok(response) => response.into_string().unwrap_or_default(),
        Err(ureq::Error::Status(status, response)) => {
            let error =
                JoinError::from_response(status, &response.into_string().unwrap_or_default());
            return match error {
                JoinError::Unavailable => Err(error),
                _ => Ok(None),
            };
        }
        Err(ureq::Error::Transport(_)) => return Err(JoinError::Unavailable),
    };
    Ok(parse_joined(&body))
}

/// HasJoinedMinecraftServerResponse: `id` and `properties`; no id is no profile.
pub fn parse_joined(body: &str) -> Option<JoinedProfile> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let uuid = parse_uuid(v.get("id")?.as_str()?)?;
    let properties = v
        .get("properties")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(horizon_protocol::packets::login::Property {
                name: p.get("name")?.as_str()?.to_owned(),
                value: p.get("value")?.as_str()?.to_owned(),
                signature: p
                    .get("signature")
                    .and_then(|s| s.as_str())
                    .map(str::to_owned),
            })
        })
        .collect();
    Some(JoinedProfile { uuid, properties })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: u128 = 0x069a79f444e94726a5befca90e38aaf5;

    fn write(dir: &Path, text: &str, mode: u32) -> std::path::PathBuf {
        let path = dir.join("session.json");
        std::fs::write(&path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let _ = mode;
        path
    }

    #[test]
    fn verify_answers() {
        assert_eq!(url_encode("Tester_1"), "Tester_1");
        assert_eq!(url_encode("-5a f"), "-5a+f");
        assert_eq!(url_encode("::1"), "%3A%3A1");
        let body = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch","properties":[{"name":"textures","value":"e30=","signature":"c2ln"}]}"#;
        let p = parse_joined(body).unwrap();
        assert_eq!(p.uuid, UUID);
        assert_eq!(p.properties.len(), 1);
        assert_eq!(p.properties[0].signature.as_deref(), Some("c2ln"));
        assert_eq!(parse_joined(""), None);
        assert_eq!(parse_joined(r#"{"name":"Notch"}"#), None);
    }

    #[test]
    fn session_files_are_read_once_and_must_be_private() {
        let dir = std::env::temp_dir().join(format!("horizon-auth-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"{"name":"Notch","uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","access_token":"mock-token","user_type":"msa"}"#;
        let path = write(&dir, text, 0o600);
        let s = AuthSession::load(&path).unwrap();
        assert_eq!((s.name.as_str(), s.uuid), ("Notch", UUID));
        assert!(!path.exists(), "deleted after reading");
        assert!(!format!("{s:?}").contains("mock-token"));
        #[cfg(unix)]
        {
            let path = write(&dir, text, 0o644);
            assert!(
                AuthSession::load(&path)
                    .unwrap_err()
                    .contains("other users")
            );
            assert!(!path.exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_file_fields() {
        assert!(
            AuthSession::parse(r#"{"name":"Notch","uuid":"069a79f444e94726a5befca90e38aaf5"}"#)
                .is_err()
        );
        assert!(
            AuthSession::parse(
                r#"{"name":"","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t"}"#
            )
            .is_err()
        );
        assert!(AuthSession::parse(r#"{"name":"A","uuid":"nope","access_token":"t"}"#).is_err());
        assert!(AuthSession::parse(r#"{"name":"A","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t","user_type":"legacy"}"#).is_err());
        let s = AuthSession::parse(r#"{"name":"A","uuid":"069a79f444e94726a5befca90e38aaf5","access_token":"t","xuid":"25"}"#).unwrap();
        assert_eq!(s.xuid.as_deref(), Some("25"));
    }

    #[test]
    fn join_request_like_authlib() {
        let s = AuthSession::new("Notch", UUID, "tok");
        let v: serde_json::Value = serde_json::from_str(&join_request(&s, "-7c9d")).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"accessToken": "tok", "selectedProfile": "069a79f444e94726a5befca90e38aaf5", "serverId": "-7c9d"})
        );
    }

    #[test]
    fn errors_map_like_authlib() {
        let e = |status, body| JoinError::from_response(status, body);
        assert_eq!(
            e(
                403,
                r#"{"error":"ForbiddenOperationException","errorMessage":"Invalid token"}"#
            ),
            JoinError::InvalidSession
        );
        assert_eq!(e(401, ""), JoinError::InvalidSession);
        assert_eq!(
            e(403, r#"{"error":"InsufficientPrivilegesException"}"#),
            JoinError::InsufficientPrivileges
        );
        assert_eq!(
            e(403, r#"{"error":"multiplayer.access.banned"}"#),
            JoinError::Banned
        );
        assert_eq!(e(503, "down"), JoinError::Unavailable);
        assert_eq!(
            e(429, r#"{"errorMessage":"Too many"}"#),
            JoinError::Other("Too many".into())
        );
        assert_eq!(
            JoinError::InvalidSession.reason(),
            Ok("disconnect.loginFailedInfo.invalidSession")
        );
    }

    #[test]
    fn instants_like_java() {
        assert_eq!(parse_instant("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_instant("1970-01-01T00:00:01.5Z"), Some(1500));
        // Instant.parse("2022-07-30T23:05:24.548227Z").toEpochMilli().
        assert_eq!(
            parse_instant("2022-07-30T23:05:24.548227Z"),
            Some(1_659_222_324_548)
        );
        assert_eq!(
            parse_instant("2024-02-29T12:00:00Z"),
            Some(1_709_208_000_000)
        );
        assert_eq!(parse_instant("1969-12-31T23:59:59Z"), Some(-1000));
        for bad in [
            "2022-07-30T23:05:24",
            "2022-13-01T00:00:00Z",
            "2022-07-30 23:05:24Z",
            "2022-07-30T23:05:24.5x1Z",
            "",
        ] {
            assert_eq!(parse_instant(bad), None, "{bad}");
        }
    }

    /// A throwaway 1024-bit key (`openssl genrsa`, PKCS#8 as the services
    /// send it under "RSA PRIVATE KEY") and its public half.
    const PRIVATE_B64: &str = "MIICdQIBADANBgkqhkiG9w0BAQEFAASCAl8wggJbAgEAAoGBAKSDqaLOF2Ltt4tzWblkFeNYhdss67BhXTbOMUYjpcPdtf4dzNtgZsTf4WEluvc8pItGWChdpq3xzPC9GL4PZ+Oh1o6VR/wSm20/50QT7V+YcSP1VrVKjDpDSg/UoC1Fmdc/vb9AO8Ip7TxuRYd/uyog/ThQ8aGcWtvvtwPqx4XrAgMBAAECgYA+ZGy2gQDflRZSPeoeZHXkphMF/7BXUWW/vk1zs1tPwEsgV/70iEgRjHL2nA+fapFg/Mm8bWAkghrehmjW1BLq5w8sGdGJCHfE/kauu78jJK0/ME38e9bsSEvv5HMMxyx0mlV8DQu2pzCT71ceT8VD/1pewUbpeZypj5pxtsCIMQJBANXgca/mnAQXTuowI8FqMiald6OuAcAWbOaNrCB5HHWuxH7hFatSqGrgp4YLqHv7bzeodU1y/uxwc+/01khd0rMCQQDE6mPfIdZxHhWkMUAKhMEnrLanfCL0dfyAe+lBpqgPwO3UurCfCaFX5RdYB3fCyvgy5foHN1/I8/Eobp+enrvpAkBgkZnjeXWiZz/Lf/b1AtUoG/g8IHpL2BRNuVQpyWVvQX0ZaNoW5BiFQXCClsr5SBr8f4SPdsfhzBidFOiX/jz/AkBDuk6Bd3T3UR6ma54YQBLjX38vWf330QD4/CV9RCH30Sa8DT4kZT6OKah+yYrCf+cgMoNtqfkS9/xRKMNT3krJAkAaFrAT6HF5Y9b5OLASbbFe7MZ4zNCzuEl6plKNUi+JCHESaU/VCCuYOLoWLeSDFYkZUoSmqiRxH8fVTsBz393x";
    const PUBLIC_B64: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCkg6mizhdi7beLc1m5ZBXjWIXbLOuwYV02zjFGI6XD3bX+HczbYGbE3+FhJbr3PKSLRlgoXaat8czwvRi+D2fjodaOlUf8EpttP+dEE+1fmHEj9Va1Sow6Q0oP1KAtRZnXP72/QDvCKe08bkWHf7sqIP04UPGhnFrb77cD6seF6wIDAQAB";

    fn pem(label: &str, b64: &str) -> String {
        let lines: Vec<&str> = b64
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect();
        format!(
            "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
            lines.join("\n")
        )
    }

    fn certificates(private: &str, public: &str) -> String {
        serde_json::json!({
            "keyPair": {
                "privateKey": pem("RSA PRIVATE KEY", private),
                "publicKey": pem("RSA PUBLIC KEY", public),
            },
            "publicKeySignature": "djE=",
            "publicKeySignatureV2": "c2ln",
            "expiresAt": "2022-07-30T23:05:24.548227Z",
            "refreshedAfter": "2022-07-30T15:05:24.548227Z",
        })
        .to_string()
    }

    #[test]
    fn certificates_response() {
        let pair = parse_certificates(&certificates(PRIVATE_B64, PUBLIC_B64)).unwrap();
        assert_eq!(
            pair.public.key,
            crate::skins::base64_decode(PUBLIC_B64).unwrap()
        );
        assert_eq!(pair.public.signature, b"sig", "publicKeySignatureV2");
        assert_eq!(pair.public.expires_at, 1_659_222_324_548);
        assert_eq!(pair.refreshed_after, 1_659_193_524_548);
        assert_eq!(pair.private.public_der(), pair.public.key);
        assert!(pair.due_refresh(1_659_193_524_549));
        assert!(!pair.due_refresh(1_659_193_524_548));
        assert!(!format!("{pair:?}").contains("MII"));
        // A public key that isn't the private key's, and broken fields.
        let other = horizon_protocol::rsa::RsaPrivateKey::generate(1024).unwrap();
        let other = base64(&other.public_der());
        assert!(parse_certificates(&certificates(PRIVATE_B64, &other)).is_none());
        assert!(parse_certificates(&certificates("bm9wZQ==", PUBLIC_B64)).is_none());
        assert!(parse_certificates("{}").is_none());
        assert!(parse_certificates("").is_none());
    }

    fn base64(bytes: &[u8]) -> String {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in bytes.chunks(3) {
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= c.len() {
                    char::from(A[(n >> (18 - 6 * i) & 63) as usize])
                } else {
                    '='
                });
            }
        }
        out
    }

    #[test]
    fn discovery_endpoints() {
        let doc = serde_json::json!({"discovery": {"session": {"endpoints": {"join": {"uri": "https://s/join"}}}}});
        assert_eq!(
            endpoint(&doc, "session", "join").as_deref(),
            Some("https://s/join")
        );
        assert_eq!(endpoint(&doc, "session", "verify"), None);
    }
}
