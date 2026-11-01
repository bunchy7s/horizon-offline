//! ops.json, whitelist.json, banned-players.json and banned-ips.json
//! (StoredUserList and its entries): a Java HashMap keyed by the UUID string
//! or the address, saved as a pretty-printed Gson array in that map's order.
use super::{
    java_properties::{civil_from_days, days_from_civil, now_millis},
    json::{self, Json},
    properties::parse_uuid,
};
use horizon_core::random::java_string_hash;
use horizon_protocol::crypto::uuid_string;
use horizon_world::java_hash_map::{JavaHashMap, JavaKey};
use std::{cmp::Ordering, path::PathBuf};

/// A map key: a Java String.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JKey(pub String);
impl JavaKey for JKey {
    fn java_hash(&self) -> i32 {
        java_string_hash(self.0.encode_utf16())
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.encode_utf16().cmp(other.0.encode_utf16()))
    }
    fn tie_break(&self, _: &Self) -> Ordering {
        Ordering::Less
    }
}

/// NameAndId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameAndId {
    pub id: u128,
    pub name: String,
}
impl NameAndId {
    pub fn offline(name: &str) -> Self {
        Self {
            id: horizon_protocol::crypto::offline_uuid(name),
            name: name.to_owned(),
        }
    }
    pub(crate) fn from_json(o: &Json) -> Option<Self> {
        let uuid = o.get("uuid")?.as_string()?;
        let name = o.get("name")?.as_string()?;
        Some(Self {
            id: parse_uuid(&uuid)?,
            name,
        })
    }
    pub(crate) fn append_to(&self, o: &mut Json) {
        o.add("uuid", Json::String(uuid_string(self.id)));
        o.add("name", Json::String(self.name.clone()));
    }
}

/// BanListEntry.DATE_FORMAT ("yyyy-MM-dd HH:mm:ss Z"), written in UTC.
pub fn format_date(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}
/// The lenient SimpleDateFormat parse of that pattern: fields may overflow
/// into the next (month 13 is January of the next year).
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (date, rest) = s.split_once(' ')?;
    let (time, zone) = rest.trim_start().split_once(' ')?;
    let num = |p: &str| -> Option<i64> {
        let p = p.trim();
        if p.is_empty()
            || !p
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
        {
            return None;
        }
        p.parse().ok()
    };
    let d: Vec<&str> = date.splitn(3, '-').collect();
    let t: Vec<&str> = time.splitn(3, ':').collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let (y, mo, da) = (num(d[0])?, num(d[1])?, num(d[2])?);
    let (h, mi, se) = (num(t[0])?, num(t[1])?, num(t[2])?);
    let zone = zone.trim();
    let sign = match zone.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = &zone[1..];
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let offset =
        sign * (digits[..2].parse::<i64>().ok()? * 60 + digits[2..].parse::<i64>().ok()?);
    // Lenient month and day arithmetic.
    let months = y * 12 + (mo - 1);
    let (year, month) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let days = days_from_civil(year, month as u32, 1) + da - 1;
    Some(((days * 86_400 + h * 3600 + mi * 60 + se) - offset * 60) * 1000)
}

/// One list entry kind.
pub trait Entry: Clone {
    type User: Clone;
    fn user(&self) -> &Self::User;
    fn key(user: &Self::User) -> String;
    fn from_json(o: &Json) -> Option<Self>;
    fn serialize(&self, o: &mut Json);
    fn expired(&self, _now: i64) -> bool {
        false
    }
    /// `infos.equals(previous)` in StoredUserList.add: only ban entries
    /// override equals; the others compare by identity, so a new entry is
    /// never equal to the stored one.
    fn same(&self, _other: &Self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpEntry {
    pub user: NameAndId,
    pub level: i32,
    pub bypasses_player_limit: bool,
}
impl Entry for OpEntry {
    type User = NameAndId;
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        let user = NameAndId::from_json(o)?;
        let level = o
            .get("level")
            .and_then(json_int)
            .map_or(0, |l| l.clamp(0, 4));
        let bypasses_player_limit = o.get("bypassesPlayerLimit").is_some_and(json_bool);
        Some(Self {
            user,
            level,
            bypasses_player_limit,
        })
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
        o.add("level", Json::Number(self.level.to_string()));
        o.add(
            "bypassesPlayerLimit",
            Json::Bool(self.bypasses_player_limit),
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhiteEntry {
    pub user: NameAndId,
}
impl Entry for WhiteEntry {
    type User = NameAndId;
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self {
            user: NameAndId::from_json(o)?,
        })
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
    }
}

/// BanListEntry for players (NameAndId) or addresses (String).
#[derive(Clone, Debug)]
pub struct BanEntry<T> {
    pub user: T,
    pub created: i64,
    pub source: String,
    pub expires: Option<i64>,
    pub reason: Option<String>,
}
impl<T: PartialEq> PartialEq for BanEntry<T> {
    fn eq(&self, o: &Self) -> bool {
        self.source == o.source
            && self.expires == o.expires
            && self.reason == o.reason
            && self.user == o.user
    }
}
impl<T> BanEntry<T> {
    pub fn new(
        user: T,
        source: Option<String>,
        expires: Option<i64>,
        reason: Option<String>,
    ) -> Self {
        Self {
            user,
            created: now_millis(),
            source: source.unwrap_or_else(|| "(Unknown)".into()),
            expires,
            reason,
        }
    }
    fn fields(user: T, o: &Json) -> Self {
        let date = |k: &str| {
            o.get(k)
                .and_then(Json::as_string)
                .and_then(|s| parse_date(&s))
        };
        Self {
            user,
            created: date("created").unwrap_or_else(now_millis),
            source: o
                .get("source")
                .and_then(Json::as_string)
                .unwrap_or_else(|| "(Unknown)".into()),
            expires: date("expires"),
            reason: o.get("reason").and_then(Json::as_string),
        }
    }
    fn serialize_ban(&self, o: &mut Json) {
        o.add("created", Json::String(format_date(self.created)));
        o.add("source", Json::String(self.source.clone()));
        o.add(
            "expires",
            Json::String(self.expires.map_or_else(|| "forever".into(), format_date)),
        );
        o.add(
            "reason",
            self.reason.clone().map_or(Json::Null, Json::String),
        );
    }
}
impl Entry for BanEntry<NameAndId> {
    type User = NameAndId;
    fn same(&self, other: &Self) -> bool {
        self == other
    }
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self::fields(NameAndId::from_json(o)?, o))
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
        self.serialize_ban(o);
    }
    fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e < now)
    }
}
impl Entry for BanEntry<String> {
    type User = String;
    fn same(&self, other: &Self) -> bool {
        self == other
    }
    fn user(&self) -> &String {
        &self.user
    }
    fn key(user: &String) -> String {
        user.clone()
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self::fields(o.get("ip")?.as_string()?, o))
    }
    fn serialize(&self, o: &mut Json) {
        o.add("ip", Json::String(self.user.clone()));
        self.serialize_ban(o);
    }
    fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e < now)
    }
}

/// JsonElement.getAsInt / getAsBoolean on primitives.
fn json_int(v: &Json) -> Option<i32> {
    match v {
        Json::Number(n) => n
            .parse::<i64>()
            .ok()
            .map(|v| v as i32)
            .or_else(|| n.parse::<f64>().ok().map(|f| f as i32)),
        Json::String(s) => s.trim().parse::<i32>().ok(),
        _ => None,
    }
}
fn json_bool(v: &Json) -> bool {
    match v {
        Json::Bool(b) => *b,
        Json::String(s) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// StoredUserList.
pub struct UserList<E: Entry> {
    pub file: PathBuf,
    map: JavaHashMap<JKey, E>,
}
impl<E: Entry> UserList<E> {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            map: JavaHashMap::default(),
        }
    }
    /// The entries in the map's iteration order.
    pub fn entries(&self) -> Vec<&E> {
        self.map.entries().map(|(_, v)| v).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn contains(&self, user: &E::User) -> bool {
        self.map.get(&JKey(E::key(user))).is_some()
    }
    /// StoredUserList.get: expired entries are dropped first.
    pub fn get(&mut self, user: &E::User) -> Option<&E> {
        let now = now_millis();
        let expired: Vec<JKey> = self
            .map
            .entries()
            .filter(|(_, e)| e.expired(now))
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired {
            self.map.remove(&k);
        }
        self.map.get(&JKey(E::key(user)))
    }
    /// Returns false when an equal entry is already there; saves otherwise.
    pub fn add(&mut self, entry: E) -> bool {
        let key = JKey(E::key(entry.user()));
        if self.map.get(&key).is_some_and(|old| entry.same(old)) {
            return false;
        }
        self.map.put(key, entry);
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after adding a user. {e}");
        }
        true
    }
    pub fn remove(&mut self, user: &E::User) -> bool {
        if self.map.remove(&JKey(E::key(user))).is_none() {
            return false;
        }
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after removing a user. {e}");
        }
        true
    }
    /// StoredUserList.clear: no entry left; saves.
    pub fn clear(&mut self) {
        self.map.clear();
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after removing a user. {e}");
        }
    }
    pub fn to_json(&self) -> String {
        let items = self
            .map
            .entries()
            .map(|(_, e)| {
                let mut o = Json::Object(Vec::new());
                e.serialize(&mut o);
                o
            })
            .collect();
        Json::Array(items).pretty()
    }
    pub fn save(&self) -> std::io::Result<()> {
        std::fs::write(&self.file, self.to_json())
    }
    /// StoredUserList.load: a missing file leaves the list as it is.
    pub fn load(&mut self) -> Result<(), String> {
        let Ok(text) = std::fs::read_to_string(&self.file) else {
            return Ok(());
        };
        self.load_str(&text)
    }
    pub fn load_str(&mut self, text: &str) -> Result<(), String> {
        self.map = JavaHashMap::default();
        if text.trim().is_empty() {
            return Ok(());
        }
        let Json::Array(items) = json::parse(text, false)? else {
            return Err("Expected a JSON array".into());
        };
        for item in items {
            if !matches!(item, Json::Object(_)) {
                return Err("Expected entry to be an object".into());
            }
            if let Some(e) = E::from_json(&item) {
                self.map.put(JKey(E::key(e.user())), e);
            }
        }
        Ok(())
    }
}

pub type OpList = UserList<OpEntry>;
pub type WhiteList = UserList<WhiteEntry>;
pub type BanList = UserList<BanEntry<NameAndId>>;
pub type IpBanList = UserList<BanEntry<String>>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_like_gson() {
        let mut ops = OpList::new("/nonexistent/ops.json");
        ops.load_str(
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":3}]"#,
        )
        .unwrap();
        assert_eq!(
            ops.to_json(),
            "[\n  {\n    \"uuid\": \"069a79f4-44e9-4726-a5be-fca90e38aaf5\",\n    \"name\": \"Notch\",\n    \"level\": 3,\n    \"bypassesPlayerLimit\": false\n  }\n]"
        );
        assert_eq!(
            parse_date("2026-10-12 10:00:00 +0200"),
            parse_date("2026-10-12 08:00:00 +0000")
        );
        assert_eq!(
            format_date(parse_date("2026-10-12 08:00:00 +0000").unwrap()),
            "2026-10-12 08:00:00 +0000"
        );
        assert_eq!(parse_date("forever"), None);
    }

    #[test]
    fn a_cleared_list_saves_empty() {
        let dir = std::env::temp_dir().join(format!("horizon-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut white = WhiteList::new(dir.join("whitelist.json"));
        assert!(white.add(WhiteEntry {
            user: NameAndId::offline("Kim")
        }));
        white.clear();
        assert!(white.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("whitelist.json")).unwrap(),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
