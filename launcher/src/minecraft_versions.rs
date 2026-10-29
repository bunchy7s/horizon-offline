//! The versions of Minecraft: Java Edition a Java instance can play: Mojang's public version list
//! (`version_manifest_v2.json`, the one the official launcher reads), kept in the launcher's data
//! folder so the list still shows offline. It names each version and where its JSON is; the game's
//! files themselves are downloaded only for an account that owns the game (`mojang.rs`).

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// What kind of version it is, as Mojang lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Version {
    pub id: String,
    pub kind: Kind,
    /// Its version JSON, and that file's sha1.
    pub url: String,
    pub sha1: String,
    /// When it came out (ISO 8601).
    pub released: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Versions {
    pub latest_release: String,
    pub latest_snapshot: String,
    /// The newest first, as Mojang lists them.
    pub all: Vec<Version>,
}

impl Versions {
    pub fn parse(text: &str) -> Result<Versions, String> {
        let json: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("Mojang's version list: {e}"))?;
        let all = json["versions"]
            .as_array()
            .ok_or("Mojang's version list has no versions")?
            .iter()
            .filter_map(|v| {
                let kind = match v["type"].as_str()? {
                    "release" => Kind::Release,
                    "snapshot" => Kind::Snapshot,
                    "old_beta" => Kind::OldBeta,
                    "old_alpha" => Kind::OldAlpha,
                    _ => return None,
                };
                Some(Version {
                    id: v["id"].as_str()?.to_string(),
                    kind,
                    url: v["url"].as_str()?.to_string(),
                    sha1: v["sha1"].as_str()?.to_string(),
                    released: v["releaseTime"].as_str().unwrap_or("").to_string(),
                })
            })
            .collect();
        let latest = |key: &str| json["latest"][key].as_str().unwrap_or("").to_string();
        Ok(Versions {
            latest_release: latest("release"),
            latest_snapshot: latest("snapshot"),
            all,
        })
    }

    /// The version `name` names: an id (`1.21.4`), `latest` (the newest release) or
    /// `latest-snapshot`.
    pub fn find(&self, name: &str) -> Result<&Version, String> {
        let id = match name.trim() {
            "latest" | "release" => self.latest_release.as_str(),
            "latest-snapshot" | "snapshot" => self.latest_snapshot.as_str(),
            id => id,
        };
        self.all.iter().find(|v| v.id == id).ok_or_else(|| {
            format!("Minecraft has no version {name:?}. `horizon-launcher instance versions` lists them.")
        })
    }

    /// The versions of the kinds asked for, the newest first.
    pub fn of(&self, snapshots: bool, old: bool) -> impl Iterator<Item = &Version> {
        self.all.iter().filter(move |v| match v.kind {
            Kind::Release => true,
            Kind::Snapshot => snapshots,
            Kind::OldBeta | Kind::OldAlpha => old,
        })
    }
}

fn cache() -> PathBuf {
    crate::backend::data_dir()
        .join("minecraft")
        .join("version_manifest_v2.json")
}

/// Mojang's list, fetched now and kept; the kept one when Mojang can't be reached.
pub fn load() -> Result<Versions, String> {
    load_with(&cache(), &|url| {
        let mut body = String::new();
        std::io::Read::read_to_string(&mut crate::backend::get(url)?.into_reader(), &mut body)
            .map_err(|e| e.to_string())?;
        Ok(body)
    })
}

fn load_with(
    cache: &Path,
    fetch: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Versions, String> {
    match fetch(MANIFEST).and_then(|text| Versions::parse(&text).map(|v| (v, text))) {
        Ok((versions, text)) => {
            let _ =
                fs::create_dir_all(cache.parent().unwrap()).and_then(|_| fs::write(cache, text));
            Ok(versions)
        }
        Err(e) => fs::read_to_string(cache)
            .map_err(|_| format!("Mojang's version list could not be fetched: {e}"))
            .and_then(|text| Versions::parse(&text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{
      "latest": {"release": "1.21.4", "snapshot": "25w02a"},
      "versions": [
        {"id": "25w02a", "type": "snapshot", "url": "https://piston-meta.mojang.com/v1/packages/a/25w02a.json", "sha1": "a", "releaseTime": "2025-01-08T12:00:00+00:00"},
        {"id": "1.21.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/b/1.21.4.json", "sha1": "b", "releaseTime": "2024-12-03T10:12:57+00:00"},
        {"id": "b1.7.3", "type": "old_beta", "url": "https://piston-meta.mojang.com/v1/packages/c/b1.7.3.json", "sha1": "c", "releaseTime": "2011-07-07T22:00:00+00:00"},
        {"id": "x", "type": "experiment", "url": "u", "sha1": "d"}
      ]
    }"#;

    #[test]
    fn the_list_names_each_version_and_the_latest_ones() {
        let v = Versions::parse(LIST).unwrap();
        assert_eq!(v.all.len(), 3, "unknown kinds are left out");
        assert_eq!(v.find("latest").unwrap().id, "1.21.4");
        assert_eq!(v.find("latest-snapshot").unwrap().kind, Kind::Snapshot);
        assert_eq!(v.find("b1.7.3").unwrap().sha1, "c");
        assert!(v.find("1.99").unwrap_err().contains("no version \"1.99\""));
        let ids = |s, o| v.of(s, o).map(|v| v.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(false, false), ["1.21.4"]);
        assert_eq!(ids(true, true), ["25w02a", "1.21.4", "b1.7.3"]);
    }

    #[test]
    fn the_kept_list_shows_offline() {
        let dir = std::env::temp_dir().join(format!("horizon-mc-versions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = dir.join("minecraft/version_manifest_v2.json");
        let offline = |_: &str| -> Result<String, String> { Err("no network".into()) };
        assert!(load_with(&cache, &offline)
            .unwrap_err()
            .contains("could not be fetched: no network"));
        let online = |url: &str| -> Result<String, String> {
            assert_eq!(url, MANIFEST);
            Ok(LIST.into())
        };
        assert_eq!(load_with(&cache, &online).unwrap().latest_release, "1.21.4");
        assert_eq!(load_with(&cache, &offline).unwrap().all.len(), 3);
        fs::remove_dir_all(&dir).unwrap();
    }
}
