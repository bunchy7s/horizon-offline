//! usercache.json (CachedUserNameToIdResolver): the players the server has
//! seen join or has looked up by name, so commands find a player by name
//! without asking the profile service again. Each entry expires a month after
//! it was added; the file keeps the 1000 most recently used.
use crate::{
    java_properties::{civil_from_days, days_from_civil, now_millis},
    json::{self, Json},
    profiles::valid_player_name,
    user_lists::{NameAndId, format_date, parse_date},
};
use std::{collections::HashMap, path::PathBuf};

const MRU_LIMIT: usize = 1000;

#[derive(Clone, Debug)]
struct Info {
    user: NameAndId,
    expires: i64,
    last_access: u64,
}

/// `Calendar.add(MONTH, 1)` on a UTC time: the same day and time a month
/// later, the day clamped to the end of a shorter month.
pub fn one_month_later(millis: i64) -> i64 {
    let days = millis.div_euclid(86_400_000);
    let time = millis.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let last = days_from_civil(
        if m == 12 { y + 1 } else { y },
        if m == 12 { 1 } else { m + 1 },
        1,
    ) - days_from_civil(y, m, 1);
    let d = d.min(last as u32);
    days_from_civil(y, m, d) * 86_400_000 + time
}

pub struct UserCache {
    file: PathBuf,
    /// Every entry ever added; the maps point into it, as vanilla's two maps
    /// share their GameProfileInfo objects.
    infos: Vec<Info>,
    by_name: HashMap<String, usize>,
    by_uuid: HashMap<u128, usize>,
    operations: u64,
    /// Off on an online-mode server: a name the profile service doesn't know
    /// is then no player at all, rather than an offline one.
    pub resolve_offline_users: bool,
}

impl UserCache {
    /// The cache in `file`, loaded; also returns the warnings vanilla logs.
    pub fn load(file: impl Into<PathBuf>) -> (Self, Vec<String>) {
        let mut cache = Self {
            file: file.into(),
            infos: Vec::new(),
            by_name: HashMap::new(),
            by_uuid: HashMap::new(),
            operations: 0,
            resolve_offline_users: true,
        };
        let mut warnings = Vec::new();
        let entries = match std::fs::read_to_string(&cache.file) {
            Ok(text) => cache.parse(&text, &mut warnings),
            Err(_) => Vec::new(),
        };
        for info in entries.into_iter().rev() {
            cache.safe_add(info);
        }
        (cache, warnings)
    }

    fn parse(&self, text: &str, warnings: &mut Vec<String>) -> Vec<Info> {
        let items = match json::parse(text, false) {
            Ok(Json::Array(items)) => items,
            // An empty file is Gson's null.
            Ok(Json::Null) => return Vec::new(),
            _ if text.trim().is_empty() => return Vec::new(),
            _ => {
                warnings.push(format!(
                    "Failed to load profile cache {}",
                    self.file.display()
                ));
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for item in &items {
            if !matches!(item, Json::Object(_)) {
                continue;
            }
            let Some(user) = NameAndId::from_json(item) else {
                continue;
            };
            let Some(date) = item.get("expiresOn").and_then(Json::as_string) else {
                continue;
            };
            match parse_date(&date) {
                Some(expires) => out.push(Info {
                    user,
                    expires,
                    last_access: 0,
                }),
                None => warnings.push(format!("Failed to parse date {date}")),
            }
        }
        out
    }

    fn safe_add(&mut self, mut info: Info) -> usize {
        self.operations += 1;
        info.last_access = self.operations;
        let index = self.infos.len();
        self.by_name.insert(info.user.name.to_lowercase(), index);
        self.by_uuid.insert(info.user.id, index);
        self.infos.push(info);
        index
    }

    /// `add`: a player who joined, remembered for a month; saves the file.
    pub fn add(&mut self, user: NameAndId) {
        self.add_internal(user, now_millis());
    }

    fn add_internal(&mut self, user: NameAndId, now: i64) -> NameAndId {
        let index = self.safe_add(Info {
            user,
            expires: one_month_later(now),
            last_access: 0,
        });
        self.save();
        self.infos[index].user.clone()
    }

    /// `get(UUID)`: the player with this id, if cached (expired or not, as vanilla's).
    pub fn get_by_id(&mut self, id: u128) -> Option<NameAndId> {
        let index = *self.by_uuid.get(&id)?;
        self.operations += 1;
        self.infos[index].last_access = self.operations;
        Some(self.infos[index].user.clone())
    }

    /// `get(String)`: the player with this name (any case). An expired entry is
    /// dropped; a name not cached is asked of `lookup` (the profile service,
    /// given the lower-case name) and, failing that, made an offline player
    /// when offline users resolve. What is found is cached.
    pub fn get_by_name(
        &mut self,
        name: &str,
        lookup: impl FnOnce(&str) -> Option<NameAndId>,
    ) -> Option<NameAndId> {
        self.get_by_name_at(name, now_millis(), lookup)
    }

    fn get_by_name_at(
        &mut self,
        name: &str,
        now: i64,
        lookup: impl FnOnce(&str) -> Option<NameAndId>,
    ) -> Option<NameAndId> {
        let name = name.to_lowercase();
        let mut found = self.by_name.get(&name).copied();
        let mut needs_save = false;
        if let Some(index) = found
            && now >= self.infos[index].expires
        {
            let user = self.infos[index].user.clone();
            self.by_uuid.remove(&user.id);
            self.by_name.remove(&user.name.to_lowercase());
            needs_save = true;
            found = None;
        }
        let result = if let Some(index) = found {
            self.operations += 1;
            self.infos[index].last_access = self.operations;
            Some(self.infos[index].user.clone())
        } else {
            let profile = if valid_player_name(&name) {
                lookup(&name)
            } else {
                None
            };
            let profile = profile.or_else(|| {
                self.resolve_offline_users
                    .then(|| NameAndId::offline(&name))
            });
            match profile {
                Some(profile) => {
                    needs_save = false;
                    Some(self.add_internal(profile, now))
                }
                None => None,
            }
        };
        if needs_save {
            self.save();
        }
        result
    }

    /// The file's text: the 1000 most recently used entries, most recent
    /// first, as Gson writes them (compact, HTML-safe).
    pub fn to_json(&self) -> String {
        let mut entries: Vec<&Info> = self.by_uuid.values().map(|&i| &self.infos[i]).collect();
        entries.sort_by_key(|info| std::cmp::Reverse(info.last_access));
        let items = entries
            .into_iter()
            .take(MRU_LIMIT)
            .map(|info| {
                let mut o = Json::Object(Vec::new());
                info.user.append_to(&mut o);
                o.add("expiresOn", Json::String(format_date(info.expires)));
                o
            })
            .collect();
        Json::Array(items).gson()
    }

    /// `save`: errors are ignored, as vanilla does.
    pub fn save(&self) {
        let _ = std::fs::write(&self.file, self.to_json());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    fn cache(text: &str) -> (UserCache, Vec<String>) {
        let dir = std::env::temp_dir().join(format!(
            "horizon-usercache-{}-{}",
            std::process::id(),
            text.len()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("usercache.json");
        std::fs::write(&file, text).unwrap();
        UserCache::load(file)
    }

    #[test]
    fn months() {
        let jan31 = days_from_civil(2026, 1, 31) * DAY + 3_723_000;
        assert_eq!(
            format_date(one_month_later(jan31)),
            "2026-02-28 01:02:03 +0000"
        );
        let dec15 = days_from_civil(2026, 12, 15) * DAY;
        assert_eq!(
            format_date(one_month_later(dec15)),
            "2027-01-15 00:00:00 +0000"
        );
        let jan31_leap = days_from_civil(2028, 1, 31) * DAY;
        assert_eq!(
            format_date(one_month_later(jan31_leap)),
            "2028-02-29 00:00:00 +0000"
        );
    }

    #[test]
    fn keeps_order_and_skips_bad_entries() {
        let (c, warnings) = cache(
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","expiresOn":"2099-01-01 00:00:00 +0000"},
                1, {"name":"NoId"}, {"uuid":"853c80ef-3c37-49fd-aa49-938b674adae6","name":"NoDate"},
                {"uuid":"853c80ef-3c37-49fd-aa49-938b674adae6","name":"BadDate","expiresOn":"soon"},
                {"uuid":"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6","name":"Alex","expiresOn":"2099-01-01 00:00:00 +0000"}]"#,
        );
        assert_eq!(warnings, vec!["Failed to parse date soon".to_owned()]);
        assert_eq!(
            c.to_json(),
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","expiresOn":"2099-01-01 00:00:00 +0000"},{"uuid":"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6","name":"Alex","expiresOn":"2099-01-01 00:00:00 +0000"}]"#
        );
    }

    #[test]
    fn names_resolve_through_the_cache() {
        let (mut c, _) = cache(
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","expiresOn":"2099-01-01 00:00:00 +0000"},
                {"uuid":"61699b2e-d327-4a01-9f1e-0ea8c3f06bc6","name":"Old","expiresOn":"2000-01-01 00:00:00 +0000"}]"#,
        );
        let now = days_from_civil(2026, 10, 17) * DAY;
        let never = |_: &str| -> Option<NameAndId> { panic!("looked up") };
        // A hit in any case, and it becomes the most recent.
        let notch = c.get_by_name_at("NOTCH", now, never).unwrap();
        assert_eq!(notch.name, "Notch");
        // By id: what the cache holds, expired or not (vanilla's `get(UUID)` doesn't check).
        assert_eq!(c.get_by_id(notch.id), Some(notch.clone()));
        assert_eq!(
            c.get_by_id(0x61699b2e_d327_4a01_9f1e_0ea8c3f06bc6)
                .unwrap()
                .name,
            "Old"
        );
        assert_eq!(c.get_by_id(1), None);
        // Expired: dropped and looked up again, by its lower-case name.
        let mut asked = Vec::new();
        let old = c
            .get_by_name_at("Old", now, |n| {
                asked.push(n.to_owned());
                None
            })
            .unwrap();
        assert_eq!(asked, ["old"]);
        assert_eq!(old, NameAndId::offline("old"));
        // Not a valid name: never looked up.
        let odd = c.get_by_name_at("a b", now, never).unwrap();
        assert_eq!(odd, NameAndId::offline("a b"));
        c.resolve_offline_users = false;
        assert_eq!(c.get_by_name_at("Nobody", now, |_| None), None);
        let found = NameAndId {
            id: 7,
            name: "Steve".into(),
        };
        let steve = c.get_by_name_at("steve", now, |_| Some(found.clone()));
        assert_eq!(steve, Some(found));
        let json = c.to_json();
        let names: Vec<&str> = json
            .split("\"name\":\"")
            .skip(1)
            .map(|s| s.split('"').next().unwrap())
            .collect();
        assert_eq!(names, ["Steve", "a b", "old", "Notch"]);
        assert!(json.contains(r#""name":"Steve","expiresOn":"2026-11-17 00:00:00 +0000"}"#));
    }
}
