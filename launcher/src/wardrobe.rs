//! The Wardrobe's link to Horizon's cosmetics service ([docs/cosmetics.md](../../docs/cosmetics.md)):
//! the catalogue, the account's own items, claiming the free set while it is open, and what each
//! slot wears. Off with the cosmetics flag (`horizon_cosmetics::enabled`).
//!
//! The service learns who the player is the way an online-mode server does: it hands out a
//! one-time server id, the launcher tells Mojang's session server that the account joins it
//! (`account::session::join_server`), and the service asks Mojang. The Minecraft token goes to
//! Mojang only; the service gives back a session of its own, kept in memory here.
use crate::i18n::tr;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
            Self::ClaimClosed => tr("The early-player set can't be claimed right now.").into(),
            Self::NotYours => tr("That item isn't in your Wardrobe.").into(),
            Self::Service(_) => {
                tr("Horizon's cosmetics service had a problem. Try again later.").into()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    NotYet,
    Open,
    Closed,
}

/// The free early-player set: its items and when it can be claimed (Unix seconds, `[start, end)`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Claim {
    pub state: ClaimState,
    pub items: Vec<String>,
    pub starts_at: Option<i64>,
    pub ends_at: Option<i64>,
    /// Whether this account claimed it (in `me` only).
    #[serde(default)]
    pub claimed: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub hash: String,
    pub slot: String,
    /// The item's name per game language code (`en_us`, `fr_fr`...).
    pub names: BTreeMap<String, String>,
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

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Catalogue {
    pub items: Vec<Entry>,
    pub claim: Claim,
}

/// The signed-in account's items: what it owns and what each slot wears.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Me {
    pub uuid: String,
    pub name: String,
    pub owned: Vec<String>,
    pub equipped: BTreeMap<String, String>,
    pub claim: Claim,
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

    pub fn catalogue(&self) -> Result<Catalogue, Error> {
        parse(self.call("GET", "/v1/catalogue", None, None)?)
    }

    /// One item's file, checked against the format's limits.
    pub fn item(&self, entry: &Entry) -> Result<horizon_cosmetics::Item, Error> {
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
        horizon_cosmetics::Item::parse(&bytes).map_err(|_| Error::Service(200))
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

    /// Claim the early-player set: the account owns its items from then on.
    pub fn claim(&self, session: &Session) -> Result<Me, Error> {
        parse(self.call("POST", "/v1/claim", Some(session), Some(json!({})))?)
    }

    /// Wear `item` in `slot`, or nothing there (`None`).
    pub fn equip(&self, session: &Session, slot: &str, item: Option<&str>) -> Result<Me, Error> {
        let body = json!({ "slot": slot, "item": item });
        parse(self.call("PUT", "/v1/equipped", Some(session), Some(body))?)
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
