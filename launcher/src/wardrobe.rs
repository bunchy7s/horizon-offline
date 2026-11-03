//! The Wardrobe's link to Horizon's cosmetics service ([docs/cosmetics.md](../../docs/cosmetics.md)):
//! the catalogue, the account's own items, claiming the founder set while it is open, what each
//! slot wears and whether others see it. Off with the cosmetics flag (`horizon_cosmetics::enabled`).
//!
//! The last answers are kept on disk (`Saved`), so the Wardrobe shows the outfit offline and when
//! the service fails, and a change made offline waits there until the service answers again.
//!
//! The service learns who the player is the way an online-mode server does: it hands out a
//! one-time server id, the launcher tells Mojang's session server that the account joins it
//! (`account::session::join_server`), and the service asks Mojang. The Minecraft token goes to
//! Mojang only; the service gives back a session of its own, kept in memory here.
use crate::i18n::tr;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Why a request to the service failed, in words for the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No answer: offline, or the service is down.
    Offline,
    /// Mojang didn't confirm the account (the launcher's `join` failed or was refused).
    Account(String),
    /// The service's session ended: sign in to it again.
    SignedOut,
    /// The free set can't be claimed now (not open yet, or over).
    ClaimClosed,
    /// An item the account doesn't own, or one for another slot.
    NotYours,
    /// Anything else the service answered.
    Service(u16),
}

impl Error {
    pub fn message(&self) -> String {
        match self {
            Self::Offline => {
                tr("Could not reach Horizon's cosmetics service. Check your internet connection.")
                    .into()
            }
            Self::Account(e) => tr(e).into(),
            Self::SignedOut => tr("Your Wardrobe session ended. Open the Wardrobe again.").into(),
            Self::ClaimClosed => tr("The founder set can't be claimed right now.").into(),
            Self::NotYours => tr("That item isn't in your Wardrobe.").into(),
            Self::Service(_) => {
                tr("Horizon's cosmetics service had a problem. Try again later.").into()
            }
        }
    }

    /// Whether the change that failed should wait and be sent again: the service wasn't
    /// reached, or failed on its side.
    pub fn retry_later(&self) -> bool {
        matches!(self, Self::Offline | Self::Service(500..))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    NotYet,
    Open,
    Closed,
}

/// The founder set: free to claim between `starts_at` and `ends_at` (Unix seconds, `[start,
/// end)`), retired for good after.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Claim {
    pub state: ClaimState,
    pub items: Vec<String>,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    /// Whether this account claimed it (in `me` only).
    #[serde(default)]
    pub claimed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub hash: String,
    /// `head`, `back`, `shoulder`, `name_tag` (the game draws them) or `theme` (the launcher's).
    pub slot: String,
    /// The item's name per game language code (`en_us`, `fr_fr`...).
    pub names: BTreeMap<String, String>,
    /// A launcher theme's colours (`slot` is `theme`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<Theme>,
}

/// A launcher theme: the sky's six stops, top to bottom (`#rrggbb`), and the accent's hue.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Theme {
    pub sky: [String; 6],
    pub accent_hue: f32,
}

impl Theme {
    /// The sky as `0xrrggbb`, or None if a stop isn't a colour.
    pub fn sky(&self) -> Option<[u32; 6]> {
        let mut out = [0; 6];
        for (o, c) in out.iter_mut().zip(&self.sky) {
            let hex = c.strip_prefix('#').filter(|h| h.len() == 6)?;
            *o = u32::from_str_radix(hex, 16).ok()?;
        }
        Some(out)
    }
}

impl Entry {
    /// The name in `lang` (a game language code), else English, else the id.
    pub fn name(&self, lang: &str) -> &str {
        self.names
            .get(lang)
            .or_else(|| self.names.get("en_us"))
            .map_or(&self.id, String::as_str)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Catalogue {
    pub items: Vec<Entry>,
    pub claim: Claim,
}

/// The signed-in account's items: what it owns, what each slot wears, whether others see it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Me {
    pub uuid: String,
    pub name: String,
    pub owned: Vec<String>,
    pub equipped: BTreeMap<String, String>,
    /// "Show my cosmetics": off, other players see this account without them.
    #[serde(default = "shown")]
    pub visible: bool,
    pub claim: Claim,
}

fn shown() -> bool {
    true
}

/// A change to the outfit, sent at once or, offline, kept until the service answers.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Wear `item` in `slot`, or nothing there.
    Equip { slot: String, item: Option<String> },
    /// "Show my cosmetics".
    Visible(bool),
}

impl Me {
    /// What the outfit looks like with `change` made, before the service confirms it.
    pub fn apply(&mut self, change: &Change) {
        match change {
            Change::Equip {
                slot,
                item: Some(item),
            } => {
                self.equipped.insert(slot.clone(), item.clone());
            }
            Change::Equip { slot, item: None } => {
                self.equipped.remove(slot);
            }
            Change::Visible(on) => self.visible = *on,
        }
    }
}

/// What the Wardrobe keeps on disk per account: the last catalogue and answer, and the changes
/// still to send. No session or token is ever kept.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Saved {
    pub catalogue: Option<Catalogue>,
    pub me: Option<Me>,
    pub pending: Vec<Change>,
}

impl Saved {
    /// `<dir>/<uuid>.json`.
    pub fn path(dir: &Path, uuid: &str) -> PathBuf {
        dir.join(format!("{uuid}.json"))
    }

    pub fn load(dir: &Path, uuid: &str) -> Self {
        std::fs::read(Self::path(dir, uuid))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path, uuid: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let path = Self::path(dir, uuid);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(tmp, path)
    }

    /// The launcher theme worn: its sky (`0xrrggbb`, top to bottom) and accent hue.
    pub fn theme(&self) -> Option<([u32; 6], f32)> {
        let id = self.me.as_ref()?.equipped.get("theme")?;
        let entry = self
            .catalogue
            .as_ref()?
            .items
            .iter()
            .find(|e| &e.id == id)?;
        let theme = entry.theme.as_ref()?;
        Some((theme.sky()?, theme.accent_hue))
    }

    /// Make `change` here: the outfit shows it at once and it waits to be sent. A newer change of
    /// the same slot (or of visibility) replaces an older one still waiting.
    pub fn change(&mut self, change: Change) {
        if let Some(me) = &mut self.me {
            me.apply(&change);
        }
        let same = |c: &Change| match (c, &change) {
            (Change::Equip { slot: a, .. }, Change::Equip { slot: b, .. }) => a == b,
            (Change::Visible(_), Change::Visible(_)) => true,
            _ => false,
        };
        self.pending.retain(|c| !same(c));
        self.pending.push(change);
    }
}

/// A session with the service: an opaque token, good for the time it said.
pub struct Session {
    token: String,
    until: Instant,
    pub uuid: String,
    pub name: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session({}, <hidden>)", self.name)
    }
}

impl Session {
    /// A made-up session for the launcher's screenshot preview (`HORIZON_LAUNCHER_PAGE=wardrobe`):
    /// it carries no credential, so the service refuses anything sent with it.
    pub fn preview(uuid: &str, name: &str) -> Self {
        Self {
            token: String::new(),
            until: Instant::now() + Duration::from_secs(3600),
            uuid: uuid.into(),
            name: name.into(),
        }
    }

    pub fn live(&self) -> bool {
        Instant::now() < self.until
    }
}

pub struct Service {
    base: String,
}

impl Service {
    /// The service the game asks too (`HORIZON_COSMETICS_API` overrides it).
    pub fn live() -> Self {
        Self::at(
            &std::env::var("HORIZON_COSMETICS_API")
                .unwrap_or_else(|_| horizon_cosmetics::API.to_owned()),
        )
    }

    pub fn at(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// The service's host, for a failure's details.
    pub fn host(&self) -> &str {
        self.base.split_once("://").map_or(&self.base, |(_, h)| h)
    }

    pub fn catalogue(&self) -> Result<Catalogue, Error> {
        parse(self.call("GET", "/v1/catalogue", None, None)?)
    }

    /// One item's file, checked against the format's limits.
    pub fn item(&self, entry: &Entry) -> Result<horizon_cosmetics::Item, Error> {
        horizon_cosmetics::Item::parse(&self.item_bytes(entry)?).map_err(|_| Error::Service(200))
    }

    fn item_bytes(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        let url = format!("{}/v1/items/{}?hash={}", self.base, entry.id, entry.hash);
        let response = match agent(&url).get(&url).call() {
            Ok(r) => r,
            Err(ureq::Error::Status(status, _)) => return Err(Error::Service(status)),
            Err(ureq::Error::Transport(_)) => return Err(Error::Offline),
        };
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(
            &mut std::io::Read::take(response.into_reader(), 1 << 20),
            &mut bytes,
        )
        .map_err(|_| Error::Offline)?;
        Ok(bytes)
    }

    /// Sign `name` in: `join` tells Mojang the account joins the server id the service chose
    /// (`account::session::join_server` for a real account).
    pub fn sign_in(
        &self,
        name: &str,
        join: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<Session, Error> {
        #[derive(Deserialize)]
        struct Start {
            server_id: String,
        }
        #[derive(Deserialize)]
        struct Finish {
            session: String,
            uuid: String,
            name: String,
            expires_in: u64,
        }
        let start: Start = parse(self.call("POST", "/v1/auth/start", None, Some(json!({})))?)?;
        join(&start.server_id).map_err(Error::Account)?;
        let body = json!({ "server_id": start.server_id, "name": name });
        let finish: Finish = match self.call("POST", "/v1/auth/finish", None, Some(body)) {
            Err(Error::SignedOut) => {
                return Err(Error::Account(
                    tr("Mojang could not confirm your account. Sign in again and retry.").into(),
                ))
            }
            other => parse(other?)?,
        };
        Ok(Session {
            token: finish.session,
            // A minute early, so a request never races the end.
            until: Instant::now() + Duration::from_secs(finish.expires_in.saturating_sub(60)),
            uuid: finish.uuid,
            name: finish.name,
        })
    }

    pub fn me(&self, session: &Session) -> Result<Me, Error> {
        parse(self.call("GET", "/v1/me", Some(session), None)?)
    }

    /// Claim the founder set: the account owns its items from then on.
    pub fn claim(&self, session: &Session) -> Result<Me, Error> {
        parse(self.call("POST", "/v1/claim", Some(session), Some(json!({})))?)
    }

    /// Wear `item` in `slot`, or nothing there (`None`).
    pub fn equip(&self, session: &Session, slot: &str, item: Option<&str>) -> Result<Me, Error> {
        let body = json!({ "slot": slot, "item": item });
        parse(self.call("PUT", "/v1/equipped", Some(session), Some(body))?)
    }

    /// "Show my cosmetics": off, other players see this account without them.
    pub fn set_visible(&self, session: &Session, visible: bool) -> Result<Me, Error> {
        let body = json!({ "visible": visible });
        parse(self.call("PUT", "/v1/visible", Some(session), Some(body))?)
    }

    /// Send the changes still waiting, oldest first. Each one the service takes, or refuses for
    /// good (an item no longer owned), leaves `saved.pending`; the first that should be tried
    /// again stops the rest and is the error. `saved.me` is the service's last answer, with what
    /// still waits applied on top.
    pub fn sync(&self, session: &Session, saved: &mut Saved) -> Result<(), Error> {
        while let Some(change) = saved.pending.first().cloned() {
            let sent = match &change {
                Change::Equip { slot, item } => self.equip(session, slot, item.as_deref()),
                Change::Visible(on) => self.set_visible(session, *on),
            };
            match sent {
                Ok(me) => saved.me = Some(me),
                Err(e) if e.retry_later() || e == Error::SignedOut => return Err(e),
                Err(_) => {}
            }
            saved.pending.remove(0);
        }
        Ok(())
    }

    /// An item's file, from `cache` (`<id>-<hash>.json`, kept by hash: it never changes for one)
    /// or the service, then kept there.
    pub fn item_cached(
        &self,
        entry: &Entry,
        cache: &Path,
    ) -> Result<horizon_cosmetics::Item, Error> {
        let path = cache.join(format!("{}-{}.json", entry.id, entry.hash));
        if let Some(item) = std::fs::read(&path)
            .ok()
            .and_then(|b| horizon_cosmetics::Item::parse(&b).ok())
        {
            return Ok(item);
        }
        let bytes = self.item_bytes(entry)?;
        let item = horizon_cosmetics::Item::parse(&bytes).map_err(|_| Error::Service(200))?;
        if std::fs::create_dir_all(cache).is_ok() {
            let _ = std::fs::write(&path, &bytes);
        }
        Ok(item)
    }

    fn call(
        &self,
        method: &str,
        path: &str,
        session: Option<&Session>,
        body: Option<Value>,
    ) -> Result<Value, Error> {
        let url = format!("{}{path}", self.base);
        let mut request = agent(&url)
            .request(method, &url)
            .set("Accept", "application/json");
        if let Some(s) = session {
            if !s.live() {
                return Err(Error::SignedOut);
            }
            request = request.set("Authorization", &format!("Bearer {}", s.token));
        }
        let result = match body {
            Some(b) => request.send_json(b),
            None => request.call(),
        };
        let response = match result {
            Ok(r) => r,
            Err(ureq::Error::Status(status, r)) => {
                let code: Value = r.into_json().unwrap_or(Value::Null);
                return Err(match (status, code["error"].as_str().unwrap_or("")) {
                    (401, _) => Error::SignedOut,
                    (403, "claim_not_open" | "claim_closed") => Error::ClaimClosed,
                    (403, "not_owned") | (400, "wrong_slot") | (404, "item") => Error::NotYours,
                    (status, _) => Error::Service(status),
                });
            }
            Err(ureq::Error::Transport(_)) => return Err(Error::Offline),
        };
        response.into_json().map_err(|_| Error::Service(200))
    }
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, Error> {
    serde_json::from_value(v).map_err(|_| Error::Service(200))
}

/// The launcher's user agent, a 15 s timeout and the player's proxy.
fn agent(url: &str) -> ureq::Agent {
    crate::proxy::agent(
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(15))
            .user_agent(&format!("horizon-launcher/{}", crate::backend::VERSION)),
        url,
    )
    .build()
}

#[cfg(test)]
mod tests;
