//! Forge's and NeoForge's versions, read from their public Maven repositories.
//!
//! Versions: Forge's `maven-metadata.xml` (`1.20.1-47.3.0`) and its promotions
//! (`promotions_slim.json`, the `recommended` build of each Minecraft version); NeoForge's version
//! list API (`21.1.77` plays 1.21.1, `26.3.0.101` plays 26.3; for 1.20.1 its `forge` artifact,
//! `1.20.1-47.1.106`).

use super::{Kind, Listed};
use crate::sources::Fetch;
use serde_json::Value;

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";

/// The Maven artifact a version lives under: `net.neoforged:forge` for NeoForge on 1.20.1, its
/// only version before it took its own name.
fn artifact(kind: Kind, minecraft: &str) -> (&'static str, &'static str, &'static str) {
    match kind {
        Kind::NeoForge if minecraft != "1.20.1" => (NEOFORGE_MAVEN, "net/neoforged", "neoforge"),
        Kind::NeoForge => (NEOFORGE_MAVEN, "net/neoforged", "forge"),
        _ => (FORGE_MAVEN, "net/minecraftforge", "forge"),
    }
}

/// NeoForge's versions for `minecraft` start with: `21.1.` for 1.21.1, `21.0.` for 1.21,
/// `26.3.0.` for 26.3. None for a snapshot.
fn neoforge_prefix(minecraft: &str) -> Option<String> {
    let parts: Vec<&str> = minecraft.split('.').collect();
    if parts.iter().any(|p| p.parse::<u32>().is_err()) {
        return None;
    }
    match parts[..] {
        ["1", minor] => Some(format!("{minor}.0.")),
        ["1", minor, patch] => Some(format!("{minor}.{patch}.")),
        [year, drop] => Some(format!("{year}.{drop}.0.")),
        [year, drop, patch] => Some(format!("{year}.{drop}.{patch}.")),
        _ => None,
    }
}

/// The `<version>`s of a `maven-metadata.xml`.
fn metadata_versions(xml: &str) -> Vec<&str> {
    xml.split("<version>")
        .skip(1)
        .filter_map(|v| v.split_once("</version>").map(|(v, _)| v.trim()))
        .collect()
}

/// Forge's versions for `minecraft` in its metadata, as players name them (`47.3.0`; an old one
/// keeps what follows: `10.13.4.1614-1.7.10`), stable up to the `recommended` build when
/// Forge names one, all of them when it doesn't.
pub(super) fn parse_forge(xml: &str, promotions: &str, minecraft: &str) -> Vec<Listed> {
    let prefix = format!("{minecraft}-");
    let promos: Value = serde_json::from_str(promotions).unwrap_or_default();
    let recommended = promos["promos"][format!("{minecraft}-recommended")]
        .as_str()
        .map(super::order);
    let mut listed: Vec<Listed> = metadata_versions(xml)
        .into_iter()
        .filter_map(|v| v.strip_prefix(&prefix))
        .map(|version| Listed {
            stable: recommended
                .as_ref()
                .is_none_or(|r| super::order(version) <= *r),
            version: version.to_string(),
        })
        .collect();
    listed.sort_by_cached_key(|l| std::cmp::Reverse(super::order(&l.version)));
    listed
}

/// NeoForge's versions for `minecraft` in its version list (`{"versions": [...]}`); a `-beta`
/// one is not stable.
pub(super) fn parse_neoforge(text: &str, minecraft: &str) -> Result<Vec<Listed>, String> {
    let list: Value =
        serde_json::from_str(text).map_err(|e| format!("NeoForge's version list: {e}"))?;
    let all = list["versions"]
        .as_array()
        .ok_or("NeoForge's version list has no versions")?
        .iter()
        .filter_map(Value::as_str);
    let mut listed: Vec<Listed> = if minecraft == "1.20.1" {
        all.filter_map(|v| v.strip_prefix("1.20.1-"))
            .map(String::from)
            .collect()
    } else {
        match neoforge_prefix(minecraft) {
            Some(prefix) => all
                .filter(|v| v.starts_with(&prefix))
                .map(String::from)
                .collect(),
            None => Vec::new(),
        }
    }
    .into_iter()
    .map(|version| Listed {
        stable: !version.contains('-'),
        version,
    })
    .collect();
    listed.sort_by_cached_key(|l| std::cmp::Reverse(super::order(&l.version)));
    Ok(listed)
}

/// `kind`'s versions for `minecraft`, the newest first.
pub(super) fn versions(kind: Kind, minecraft: &str, fetch: Fetch) -> Result<Vec<Listed>, String> {
    let text = |url: &str| fetch(url).map(|b| String::from_utf8_lossy(&b).into_owned());
    match kind {
        Kind::NeoForge => {
            let (_, group, name) = artifact(kind, minecraft);
            let url =
                format!("https://maven.neoforged.net/api/maven/versions/releases/{group}/{name}");
            parse_neoforge(&text(&url)?, minecraft)
        }
        _ => Ok(parse_forge(
            &text(&format!(
                "{FORGE_MAVEN}/net/minecraftforge/forge/maven-metadata.xml"
            ))?,
            // Without its promotions every version still installs: none is called stable.
            &text("https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json")
                .unwrap_or_default(),
            minecraft,
        )),
    }
}

#[cfg(test)]
#[path = "forge_tests.rs"]
mod tests;
