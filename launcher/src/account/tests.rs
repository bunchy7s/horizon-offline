//! The sign-in chain against a local mock of the Microsoft, Xbox Live and Minecraft endpoints
//! (recorded response shapes, mock tokens only), and the keychain abstraction in memory.
use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

const UUID: &str = "069a79f444e94726a5befca90e38aaf5";

/// What the mock answers and what it was asked.
#[derive(Default)]
struct Mock {
    /// Polls left before the token endpoint stops saying authorization_pending.
    pending: usize,
    xerr: Option<u64>,
    owns_game: bool,
    no_profile: bool,
    /// What `launcher/login` answers instead of a token (an older service, an outage).
    launcher_login_refused: Option<u16>,
    /// Every request: method, path and body.
    log: Vec<(String, String, String)>,
}

fn reply(status: u16, body: Value) -> (u16, String) {
    (status, body.to_string())
}

fn route(mock: &mut Mock, method: &str, path: &str, auth: &str, body: &str) -> (u16, String) {
    let form = |key: &str| {
        body.split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.replace('+', " ").replace("%3A", ":").replace("%20", " "))
    };
    match (method, path) {
        ("POST", "/ms/devicecode") => {
            assert_eq!(form("client_id").as_deref(), Some("test-client"));
            assert_eq!(form("scope").as_deref(), Some(SCOPE));
            reply(
                200,
                json!({"user_code": "ABCD-EFGH", "device_code": "dev-1", "verification_uri": "https://www.microsoft.com/link", "expires_in": 900, "interval": 1}),
            )
        }
        ("POST", "/ms/token") => match form("grant_type").as_deref() {
            Some("urn:ietf:params:oauth:grant-type:device_code") if mock.pending > 0 => {
                mock.pending -= 1;
                reply(400, json!({"error": "authorization_pending"}))
            }
            Some("urn:ietf:params:oauth:grant-type:device_code") => {
                assert_eq!(form("device_code").as_deref(), Some("dev-1"));
                reply(
                    200,
                    json!({"access_token": "ms-access-1", "refresh_token": "ms-refresh-1", "expires_in": 3600}),
                )
            }
            Some("refresh_token") if form("refresh_token").as_deref() == Some("ms-refresh-1") => {
                reply(
                    200,
                    json!({"access_token": "ms-access-2", "refresh_token": "ms-refresh-2"}),
                )
            }
            _ => reply(400, json!({"error": "invalid_grant"})),
        },
        ("POST", "/xbl") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert!(v["Properties"]["RpsTicket"]
                .as_str()
                .unwrap()
                .starts_with("d=ms-access-"));
            assert_eq!(v["RelyingParty"], "http://auth.xboxlive.com");
            reply(
                200,
                json!({"Token": "xbl-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1"}]}}),
            )
        }
        ("POST", "/xsts") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["Properties"]["UserTokens"][0], "xbl-token");
            assert_eq!(v["RelyingParty"], "rp://api.minecraftservices.com/");
            match mock.xerr {
                Some(code) => reply(401, json!({"Identity": "0", "XErr": code, "Message": ""})),
                None => reply(
                    200,
                    json!({"Token": "xsts-token", "DisplayClaims": {"xui": [{"uhs": "uhs-1", "xid": "2535400000000000"}]}}),
                ),
            }
        }
        ("POST", "/mc/launcher/login") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["xtoken"], "XBL3.0 x=uhs-1;xsts-token");
            assert_eq!(v["platform"], "PC_LAUNCHER");
            match mock.launcher_login_refused {
                Some(status) => reply(
                    status,
                    json!({"path": "/launcher/login", "error": "UNAUTHORIZED"}),
                ),
                None => reply(
                    200,
                    json!({"access_token": "mc-token", "token_type": "Bearer", "expires_in": 86400}),
                ),
            }
        }
        ("POST", "/mc/authentication/login_with_xbox") => {
            let v: Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["identityToken"], "XBL3.0 x=uhs-1;xsts-token");
            reply(
                200,
                json!({"username": "mock", "access_token": "mc-token", "token_type": "Bearer", "expires_in": 86400}),
            )
        }
        ("GET", "/mc/entitlements/mcstore") => {
            assert_eq!(auth, "Bearer mc-token");
            let items = if mock.owns_game {
                json!([{"name": "product_minecraft", "signature": "x"}, {"name": "game_minecraft", "signature": "x"}])
            } else {
                json!([])
            };
            reply(200, json!({"items": items, "signature": "x", "keyId": "1"}))
        }
        ("GET", "/mc/minecraft/profile") if mock.no_profile => reply(
            404,
            json!({"path": "/minecraft/profile", "errorType": "NOT_FOUND", "error": "NOT_FOUND"}),
        ),
        ("GET", "/mc/minecraft/profile") => reply(
            200,
            json!({
                "id": UUID, "name": "Notch",
                "skins": [{"id": "s1", "state": "ACTIVE", "url": "http://textures.minecraft.net/texture/abc", "variant": "SLIM"}],
                "capes": [{"id": "c1", "state": "INACTIVE", "url": "http://textures.minecraft.net/texture/cape1", "alias": "Migrator"},
                          {"id": "c2", "state": "ACTIVE", "url": "http://textures.minecraft.net/texture/cape2", "alias": "Vanilla"}],
            }),
        ),
        _ => reply(404, json!({"error": "NotFound"})),
    }
}

/// A one-thread HTTP/1.1 server: one request per connection.
fn serve(mock: Arc<Mutex<Mock>>) -> Endpoints {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (method, path) = (
                parts.next().unwrap_or("").to_owned(),
                parts.next().unwrap_or("").to_owned(),
            );
            let (mut length, mut auth) = (0, String::new());
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                let (k, v) = header.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "authorization" => auth = v.trim().to_owned(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8(body).unwrap();
            let (status, text) = {
                let mut m = mock.lock().unwrap();
                // A bare `GET /` is a local tool probing every new port (T3 Code does, on the Mac
                // mini), not the launcher: no endpoint here is the root.
                if (method.as_str(), path.as_str()) != ("GET", "/") {
                    m.log.push((method.clone(), path.clone(), body.clone()));
                }
                route(&mut m, &method, &path, &auth, &body)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
        }
    });
    Endpoints {
        microsoft: format!("{base}/ms"),
        xbox_user: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        minecraft: format!("{base}/mc"),
    }
}

fn mock(f: impl FnOnce(&mut Mock)) -> (Arc<Mutex<Mock>>, Endpoints) {
    let mut m = Mock {
        owns_game: true,
        ..Default::default()
    };
    f(&mut m);
    let m = Arc::new(Mutex::new(m));
    let ep = serve(m.clone());
    (m, ep)
}

#[test]
fn the_whole_chain() {
    let (m, ep) = mock(|m| m.pending = 2);
    let code = device_code(&ep, "test-client").unwrap();
    assert_eq!(
        (code.user_code.as_str(), code.verification_uri.as_str()),
        ("ABCD-EFGH", "https://www.microsoft.com/link")
    );
    let ms = wait_for_sign_in(&ep, "test-client", &code, &|| false).unwrap();
    let (account, secrets) = finish_sign_in(&ep, ms).unwrap();
    assert_eq!(
        account,
        Account {
            uuid: UUID.into(),
            name: "Notch".into(),
            skin: Some((
                "http://textures.minecraft.net/texture/abc".into(),
                "slim".into()
            )),
            cape: Some("http://textures.minecraft.net/texture/cape2".into()),
        }
    );
    assert_eq!(secrets.minecraft_token, "mc-token");
    assert_eq!(secrets.xuid.as_deref(), Some("2535400000000000"));
    assert!(secrets.minecraft_expires_at > now() + 80000);
    // Two pending polls, then the token; then Xbox, XSTS, Minecraft, entitlements, profile.
    let paths: Vec<String> = m
        .lock()
        .unwrap()
        .log
        .iter()
        .map(|(_, p, _)| p.clone())
        .collect();
    assert_eq!(
        paths,
        [
            "/ms/devicecode",
            "/ms/token",
            "/ms/token",
            "/ms/token",
            "/xbl",
            "/xsts",
            "/mc/launcher/login",
            "/mc/entitlements/mcstore",
            "/mc/minecraft/profile"
        ]
    );
    assert!(!format!("{secrets:?}").contains("mc-token"));
}

#[test]
fn login_with_xbox_signs_in_when_launcher_login_refuses() {
    for status in [401, 404, 503] {
        let (m, ep) = mock(|m| m.launcher_login_refused = Some(status));
        let ms = MsTokens {
            access: "ms-access-1".into(),
            refresh: "r".into(),
        };
        let (account, secrets) = finish_sign_in(&ep, ms).unwrap();
        assert_eq!(account.uuid, UUID);
        assert_eq!(secrets.minecraft_token, "mc-token");
        // The same owner check follows, with the token login_with_xbox gave.
        assert!(secrets.owns);
        let paths: Vec<String> = m
            .lock()
            .unwrap()
            .log
            .iter()
            .map(|(_, p, _)| p.clone())
            .collect();
        assert_eq!(
            paths[2..],
            [
                "/mc/launcher/login",
                "/mc/authentication/login_with_xbox",
                "/mc/entitlements/mcstore",
                "/mc/minecraft/profile"
            ]
        );
    }
    // Either way, an account that doesn't own the game is refused.
    let (_, ep) = mock(|m| {
        m.launcher_login_refused = Some(503);
        m.owns_game = false;
    });
    let ms = MsTokens {
        access: "ms-access-1".into(),
        refresh: "r".into(),
    };
    assert_eq!(finish_sign_in(&ep, ms).unwrap_err(), DOES_NOT_OWN);
}

fn notch() -> Account {
    Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    }
}

/// A signed-in account's secrets: a token good for two hours, and the last ownership answer.
fn secrets(token: &str, owns: bool) -> Secrets {
    Secrets {
        refresh_token: "ms-refresh-1".into(),
        minecraft_token: token.into(),
        minecraft_expires_at: now() + 7200,
        xuid: None,
        owns,
    }
}

/// Nothing listens there: every request fails as with no network.
fn offline() -> Endpoints {
    Endpoints {
        microsoft: "http://127.0.0.1:9/ms".into(),
        xbox_user: "http://127.0.0.1:9/xbl".into(),
        xsts: "http://127.0.0.1:9/xsts".into(),
        minecraft: "http://127.0.0.1:9/mc".into(),
    }
}

#[test]
fn refresh_only_when_the_token_runs_out() {
    let (m, ep) = mock(|_| {});
    let Check::Owns(_, kept) = check(
        &ep,
        Some("test-client"),
        &notch(),
        secrets("mc-token", true),
    ) else {
        panic!("owns");
    };
    assert_eq!(kept.refresh_token, "ms-refresh-1");
    // Entitlements and profile only: the token had two hours left.
    assert_eq!(m.lock().unwrap().log.len(), 2);
    let stale = Secrets {
        minecraft_expires_at: now() + 60,
        ..kept.clone()
    };
    let Check::Owns(renewed_account, renewed) = check(&ep, Some("test-client"), &notch(), stale)
    else {
        panic!("owns");
    };
    assert_eq!(
        (
            renewed.refresh_token.as_str(),
            renewed.minecraft_token.as_str()
        ),
        ("ms-refresh-2", "mc-token")
    );
    assert!(renewed_account.skin.is_some());
    let revoked = Secrets {
        refresh_token: "revoked".into(),
        minecraft_expires_at: 0,
        ..kept
    };
    assert!(matches!(
        check(&ep, Some("test-client"), &notch(), revoked),
        Check::NoAnswer(e, s) if e == SIGN_IN_AGAIN && s.owns
    ));
}

/// An accounts.json with Notch signed in and playing, his secrets in `store`.
fn signed_in(store: &MemoryStore, secrets: &Secrets) -> Accounts {
    let mut accounts = Accounts::default();
    accounts.add(store, notch(), secrets).unwrap();
    accounts
}

fn kept(store: &MemoryStore) -> Secrets {
    secrets_in(store, UUID).unwrap()
}

#[test]
fn an_owner_plays_with_a_session() {
    let (_, ep) = mock(|_| {});
    let store = MemoryStore::default();
    let mut accounts = signed_in(&store, &secrets("mc-token", true));
    let pass = launch(&ep, Some("test-client"), &store, &mut accounts).unwrap();
    let (account, session) = pass.session.unwrap();
    assert_eq!(
        (account.name.as_str(), session.minecraft_token.as_str()),
        ("Notch", "mc-token")
    );
    assert!(pass.owner.is_some() && pass.offline.is_none());
    // The profile as the services have it now (the skin is new).
    assert!(accounts.selected().unwrap().skin.is_some());
}

/// A keychain that reads but refuses to keep anything new (#3249).
struct Refusing(MemoryStore);
impl SecretStore for Refusing {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.0.get(key)
    }
    fn set(&self, _: &str, _: &str) -> Result<(), String> {
        Err("the keyring is locked".into())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        self.0.delete(key)
    }
    fn persistent(&self) -> bool {
        true
    }
}

#[test]
fn a_renewed_sign_in_the_keychain_refuses_still_plays_and_says_so() {
    let (_, ep) = mock(|_| {});
    let store = MemoryStore::default();
    let stale = Secrets {
        minecraft_expires_at: now() + 60,
        ..secrets("old-token", true)
    };
    let mut accounts = signed_in(&store, &stale);
    let refusing = Refusing(store);
    let pass = launch(&ep, Some("test-client"), &refusing, &mut accounts).unwrap();
    assert_eq!(pass.session.unwrap().1.minecraft_token, "mc-token");
    let said = "Notch's renewed sign-in couldn't be saved in the keychain (the keyring is \
                locked): you may have to sign in again next time.";
    assert_eq!(pass.unsaved.as_deref(), Some(said));
    assert_eq!(take_unsaved().as_deref(), Some(said));
    assert_eq!(take_unsaved(), None, "said once");
    // Nothing to save, nothing to say.
    let mut accounts = signed_in(&refusing.0, &secrets("mc-token", true));
    let pass = launch(&ep, Some("test-client"), &refusing, &mut accounts).unwrap();
    assert!(pass.unsaved.is_none());
}

#[test]
fn a_no_from_the_services_plays_offline() {
    let (_, ep) = mock(|m| m.owns_game = false);
    let store = MemoryStore::default();
    let mut accounts = signed_in(&store, &secrets("mc-token", true));
    assert!(accounts.has_owner(&store));
    // This fork plays without an owner: the game starts, offline, and says why.
    let pass = launch(&ep, Some("test-client"), &store, &mut accounts).unwrap();
    assert!(pass.session.is_none() && pass.owner.is_none());
    assert!(pass.offline.is_some());
    // The answer is kept beside the tokens: offline now, it still says no.
    assert!(!kept(&store).owns && !accounts.has_owner(&store));
    let pass = launch(&offline(), Some("test-client"), &store, &mut accounts).unwrap();
    assert!(pass.session.is_none() && pass.owner.is_none());
}

#[test]
fn no_network_never_blocks_a_past_owner() {
    let store = MemoryStore::default();
    let mut accounts = signed_in(&store, &secrets("mc-token", true));
    let pass = launch(&offline(), Some("test-client"), &store, &mut accounts).unwrap();
    // Plays, offline, and says why.
    assert!(pass.session.is_none() && pass.owner.is_none());
    assert!(pass.offline.unwrap().contains("Could not reach"));
    // A token to renew with no network, or a build without sign-in: the same.
    let stale = Secrets {
        minecraft_expires_at: 0,
        ..secrets("mc-token", true)
    };
    let mut accounts = signed_in(&store, &stale);
    assert!(launch(&offline(), Some("test-client"), &store, &mut accounts).is_ok());
    assert!(launch(&offline(), None, &store, &mut accounts).is_ok());
    // An account that never passed plays offline too (this fork has no owner gate).
    let mut accounts = signed_in(&store, &secrets("mc-token", false));
    assert!(launch(&offline(), Some("test-client"), &store, &mut accounts).is_ok());
}

#[test]
fn accounts_from_before_the_gate_owned_the_game_at_sign_in() {
    let old = r#"{"refresh_token":"r","minecraft_token":"t","minecraft_expires_at":1,"xuid":null}"#;
    assert!(serde_json::from_str::<Secrets>(old).unwrap().owns);
}

#[test]
fn any_owner_opens_the_gate_and_the_one_who_plays_brings_the_session() {
    let (m, ep) = mock(|_| {});
    let store = MemoryStore::default();
    let mut accounts = signed_in(&store, &secrets("mc-token", true));
    // A second account, no longer in the keychain, plays.
    accounts.accounts.push(Account {
        uuid: "other".into(),
        name: "Alex".into(),
        skin: None,
        cape: None,
    });
    accounts.selected = Some("other".into());
    let pass = launch(&ep, Some("test-client"), &store, &mut accounts).unwrap();
    assert!(pass.session.is_none());
    assert!(pass
        .offline
        .unwrap()
        .starts_with("Alex is no longer signed in"));
    assert_eq!(pass.owner.unwrap().0.name, "Notch");
    // An offline name plays: Notch is asked, and no session goes to the game.
    accounts.selected = None;
    m.lock().unwrap().log.clear();
    let pass = launch(&ep, Some("test-client"), &store, &mut accounts).unwrap();
    assert!(pass.session.is_none() && pass.offline.is_none());
    assert_eq!(m.lock().unwrap().log.len(), 2);
    // No account at all: still plays, offline.
    let pass = launch(&ep, Some("test-client"), &store, &mut Accounts::default()).unwrap();
    assert!(pass.session.is_none() && pass.owner.is_none() && pass.offline.is_some());
}

#[test]
fn offline_names_play_without_an_owner() {
    let store = MemoryStore::default();
    let mut accounts = Accounts::default();
    // This fork plays without a Microsoft account: offline names need no owner.
    accounts
        .play_as(&store, Player::Offline("Builder_1".into()))
        .unwrap();
    assert_eq!(accounts.offline, ["Builder_1"]);
    assert_eq!(accounts.player(), Player::Offline("Builder_1".into()));
    assert!(!accounts.has_owner(&store));
    let (_, ep) = mock(|_| {});
    assert!(launch(&ep, Some("test-client"), &store, &mut accounts).is_ok());
    accounts
        .play_as(&store, Player::Offline("Builder_2".into()))
        .unwrap();
    assert_eq!(accounts.offline, ["Builder_1", "Builder_2"]);
}

#[test]
fn refusals_have_messages() {
    let tokens = || MsTokens {
        access: "ms-access-1".into(),
        refresh: "r".into(),
    };
    let (_, ep) = mock(|m| m.xerr = Some(2148916233));
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("no Xbox profile"));
    let (_, ep) = mock(|m| m.xerr = Some(2148916238));
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("child account"));
    let (_, ep) = mock(|m| m.owns_game = false);
    assert_eq!(finish_sign_in(&ep, tokens()).unwrap_err(), DOES_NOT_OWN);
    let (_, ep) = mock(|m| m.no_profile = true);
    assert!(finish_sign_in(&ep, tokens())
        .unwrap_err()
        .contains("no Minecraft profile"));
    assert_eq!(
        oauth_error(&json!({"error": "authorization_declined"})),
        "Sign-in was declined in the browser."
    );
    // Nothing listening: a message, not a URL.
    assert_eq!(
        device_code(&offline(), "x").unwrap_err(),
        "Could not reach Microsoft. Check your internet connection."
    );
}

#[test]
fn cancelling_stops_the_wait() {
    let (_, ep) = mock(|m| m.pending = 100);
    let code = device_code(&ep, "test-client").unwrap();
    assert_eq!(
        wait_for_sign_in(&ep, "test-client", &code, &|| true).unwrap_err(),
        "Sign-in cancelled."
    );
}

#[test]
fn accounts_keep_secrets_in_the_store_only() {
    let store = MemoryStore::default();
    let mut accounts = Accounts::default();
    let account = Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let secrets = Secrets {
        refresh_token: "r".into(),
        minecraft_token: "mc-secret".into(),
        minecraft_expires_at: 1,
        xuid: None,
        owns: true,
    };
    accounts.add(&store, account.clone(), &secrets).unwrap();
    assert_eq!(accounts.selected(), Some(&account));
    assert_eq!(accounts.find("notch"), Some(&account));
    assert_eq!(
        accounts.find("069a79f4-44e9-4726-a5be-fca90e38aaf5"),
        Some(&account)
    );
    let saved = serde_json::to_string(&accounts).unwrap();
    assert!(!saved.contains("mc-secret") && !saved.contains("\"r\""));
    assert!(store.get(UUID).unwrap().unwrap().contains("mc-secret"));
    accounts.remove(&store, UUID).unwrap();
    assert_eq!(
        (accounts.selected(), store.get(UUID).unwrap()),
        (None, None)
    );
    assert!(!store.persistent());
}

/// The OS keychain itself: Credential Manager and the Keychain take one secret per item, and a
/// sign-in's secrets are longer than one item holds. Ignored by default: it writes to the keychain
/// (a test key, removed again).
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
#[ignore = "writes to the OS keychain"]
fn system_keychain_keeps_long_secrets() {
    let key = "test-long-secret";
    let secret = format!("{{\"refresh_token\":\"{}\"}}", "r".repeat(3000));
    SystemStore.set(key, &secret).unwrap();
    assert_eq!(SystemStore.get(key).unwrap(), Some(secret.clone()));
    SystemStore.delete(key).unwrap();
    assert_eq!(SystemStore.get(key).unwrap(), None);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn long_secrets_split_into_keychain_sized_pieces() {
    // Mixed 1-, 2- and 4-byte characters, so the cuts land inside some of them.
    let secret = "aé🔑".repeat(300);
    let pieces = split_secret(&secret);
    assert!(pieces.len() > 1);
    assert_eq!(pieces.concat(), secret);
    for piece in &pieces {
        assert!(piece.len() <= PIECE);
        assert!(piece.encode_utf16().count() * 2 <= 2560);
    }
    assert_eq!(split_secret("short"), vec!["short"]);
}

#[test]
fn session_files_are_private_and_complete() {
    let dir = std::env::temp_dir().join(format!("horizon-launcher-session-{}", std::process::id()));
    let account = Account {
        uuid: UUID.into(),
        name: "Notch".into(),
        skin: None,
        cape: None,
    };
    let secrets = Secrets {
        refresh_token: "r".into(),
        minecraft_token: "mc-token".into(),
        minecraft_expires_at: 1,
        xuid: Some("25".into()),
        owns: true,
    };
    let path = write_session_file(&dir, &account, &secrets).unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        v,
        json!({"name": "Notch", "uuid": UUID, "access_token": "mc-token", "user_type": "msa", "xuid": "25"})
    );
    // The refresh token never leaves the keychain.
    assert!(!v.to_string().contains("\"r\""));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let other = write_session_file(&dir, &account, &secrets).unwrap();
    assert_ne!(path, other);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn client_id_is_configuration() {
    // The run-time variable overrides; empty means not configured.
    std::env::set_var("HORIZON_MSA_CLIENT_ID", " ");
    assert_eq!(client_id(), None);
    std::env::set_var(
        "HORIZON_MSA_CLIENT_ID",
        "00000000-1111-2222-3333-444444444444",
    );
    assert_eq!(
        client_id().as_deref(),
        Some("00000000-1111-2222-3333-444444444444")
    );
    std::env::remove_var("HORIZON_MSA_CLIENT_ID");
}

#[test]
fn offline_names_switch_with_accounts() {
    let store = MemoryStore::default();
    store
        .set("abc", &serde_json::to_string(&secrets("t", true)).unwrap())
        .unwrap();
    let mut a = Accounts {
        accounts: vec![Account {
            uuid: "abc".into(),
            name: "Ludo".into(),
            skin: None,
            cape: None,
        }],
        selected: Some("abc".into()),
        ..Accounts::default()
    };
    assert_eq!(a.player(), Player::Microsoft("abc".into()));
    assert_eq!(a.name_arg(), None);
    a.play_as(&store, Player::Offline("Builder_1".into()))
        .unwrap();
    assert_eq!(a.player(), Player::Offline("Builder_1".into()));
    assert_eq!(a.name_arg(), Some("Builder_1"));
    // Kept once, as first written, whatever the case.
    a.play_as(&store, Player::Offline("builder_1".into()))
        .unwrap();
    assert_eq!(a.offline, ["Builder_1"]);
    assert_eq!(a.name_arg(), Some("Builder_1"));
    assert!(a.add_offline("no spaces").is_err());
    assert!(a.add_offline("ab").is_err());
    assert!(a.add_offline("seventeen_letters").is_err());
    a.play_as(&store, Player::Microsoft("abc".into())).unwrap();
    assert_eq!(a.name_arg(), None, "an account plays with its own name");
    assert!(a.play_as(&store, Player::Microsoft("zzz".into())).is_err());
    a.play_as(&store, Player::Offline("Builder_1".into()))
        .unwrap();
    // Forgetting drops the look and the name's skin and cape files too.
    let dir = std::env::temp_dir().join(format!("horizon-accounts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (skin, cape) = crate::skins::files(&dir, "builder_1");
    std::fs::write(&skin, b"skin").unwrap();
    std::fs::write(&cape, b"cape").unwrap();
    a.looks.insert(
        "builder_1".into(),
        crate::skins::Look {
            skin: true,
            cape: true,
            ..crate::skins::Look::default()
        },
    );
    a.remove_offline("BUILDER_1", &dir).unwrap();
    assert_eq!(a.player(), Player::GameName);
    assert!(a.looks.is_empty() && !skin.exists() && !cape.exists());
    let _ = std::fs::remove_dir_all(&dir);
    assert!(a.remove_offline("Builder_1", &dir).is_err());
    // An accounts.json from before offline names reads as it was, and writes nothing new.
    let old: Accounts = serde_json::from_str(r#"{"accounts":[],"selected":null}"#).unwrap();
    assert_eq!(old.player(), Player::GameName);
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"accounts":[],"selected":null}"#
    );
}

#[test]
fn the_java_edition_plays_as_an_owner_even_offline() {
    let (_, ep) = mock(|_| {});
    let store = MemoryStore::default();
    let mut accounts = signed_in(&store, &secrets("mc-token", true));
    let player = java_player_with(&ep, Some("test-client"), &store, &mut accounts).unwrap();
    assert_eq!(
        (
            player.name.as_str(),
            player.uuid.as_str(),
            player.token.as_str()
        ),
        ("Notch", UUID, "mc-token")
    );
    // No network: the last token of an account that owned the game.
    let player = java_player_with(&offline(), Some("test-client"), &store, &mut accounts).unwrap();
    assert_eq!(player.token, "mc-token");
    // Never for an account that doesn't own it.
    let mut accounts = signed_in(&store, &secrets("mc-token", false));
    assert_eq!(
        java_player_with(&offline(), Some("test-client"), &store, &mut accounts).unwrap_err(),
        NoOwner.to_string()
    );
}
