//! Mod loaders for Java instances: Fabric, Quilt, Forge and NeoForge. For each Minecraft version,
//! Fabric's and Quilt's metadata services list their versions and serve a launcher profile: a
//! version JSON that `inheritsFrom` the game's own, with the loader's main class and its libraries
//! by Maven name and repository. Forge's and NeoForge's versions come from their Maven repositories
//! ([`forge`]). The profile is kept in `versions/` beside the game's, as the
//! official launcher keeps a loader's, and its libraries in `libraries/`, each checked against its
//! sha1 (the profile's, else the one the repository publishes beside the file).
//! `java_launch::resolve` merges the profile into the game's version.

mod forge;

use crate::sources::{fetch_checked, sha1_file, write_file, Fetch};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// A mod loader.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl Kind {
    /// Every loader, in the order a picker shows them.
    pub const ALL: [Kind; 4] = [Kind::Fabric, Kind::Quilt, Kind::Forge, Kind::NeoForge];

    pub fn parse(name: &str) -> Result<Kind, String> {
        let name = name.trim();
        Kind::ALL
            .into_iter()
            .find(|k| k.id().eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("No mod loader {name:?}: fabric, quilt, forge or neoforge."))
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Fabric => "Fabric",
            Kind::Quilt => "Quilt",
            Kind::Forge => "Forge",
            Kind::NeoForge => "NeoForge",
        }
    }

    /// As Modrinth and the instance's settings name it: `fabric`, `quilt`, `forge`, `neoforge`.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Fabric => "fabric",
            Kind::Quilt => "quilt",
            Kind::Forge => "forge",
            Kind::NeoForge => "neoforge",
        }
    }

    /// Its metadata service (Fabric's and Quilt's).
    fn meta(self) -> &'static str {
        match self {
            Kind::Quilt => "https://meta.quiltmc.org/v3",
            _ => "https://meta.fabricmc.net/v2",
        }
    }

    /// The id its profile has: `fabric-loader-0.16.9-1.21.4`, `1.20.1-forge-47.3.0`,
    /// `neoforge-21.1.77` (`1.20.1-neoforge-47.1.106` beside Forge's own 47.1 builds).
    pub fn profile_id(self, version: &str, minecraft: &str) -> String {
        match self {
            Kind::Forge => format!("{minecraft}-forge-{version}"),
            Kind::NeoForge if minecraft == "1.20.1" => format!("1.20.1-neoforge-{version}"),
            Kind::NeoForge => format!("neoforge-{version}"),
            _ => format!("{}-loader-{version}-{minecraft}", self.id()),
        }
    }
}

/// The loader a Java instance plays with, at one version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loader {
    pub kind: Kind,
    pub version: String,
}

/// One of a loader's versions for a Minecraft version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Listed {
    pub version: String,
    pub stable: bool,
}

/// How versions sort: by their numbers, a release above its betas (`0.20.0` > `0.20.0-beta.9`),
/// betas by their numbers too (`beta.10` > `beta.9`).
fn order(version: &str) -> (Vec<u64>, bool, Vec<(u64, String)>) {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version.split('+').next().unwrap_or(version), None),
    };
    let numbers = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
    let pre: Vec<(u64, String)> = pre
        .into_iter()
        .flat_map(|p| p.split(['.', '-', '+']))
        .map(|part| (part.parse().unwrap_or(0), part.to_string()))
        .collect();
    (numbers, pre.is_empty(), pre)
}

/// A service's list, `[{"loader": {"version", "stable"}}]`, the newest first (Quilt's comes in no
/// order). Quilt's has no `stable`: its betas carry a `-` (`0.20.0-beta.9`).
pub fn parse_versions(text: &str) -> Result<Vec<Listed>, String> {
    let list: Value =
        serde_json::from_str(text).map_err(|e| format!("The loader's version list: {e}"))?;
    let list = list
        .as_array()
        .ok_or("The loader's version list is not a list")?;
    let mut listed: Vec<Listed> = list
        .iter()
        .filter_map(|entry| {
            let loader = &entry["loader"];
            let version = loader["version"].as_str()?.to_string();
            let stable = loader["stable"].as_bool().unwrap_or(!version.contains('-'));
            Some(Listed { version, stable })
        })
        .collect();
    listed.sort_by_cached_key(|l| std::cmp::Reverse(order(&l.version)));
    Ok(listed)
}

/// `kind`'s versions for `minecraft`.
pub fn versions(kind: Kind, minecraft: &str, fetch: Fetch) -> Result<Vec<Listed>, String> {
    let listed = match kind {
        Kind::Forge | Kind::NeoForge => forge::versions(kind, minecraft, fetch)?,
        _ => {
            let body = fetch(&format!("{}/versions/loader/{minecraft}", kind.meta()))?;
            parse_versions(&String::from_utf8_lossy(&body))?
        }
    };
    if listed.is_empty() {
        return Err(format!(
            "{} has no loader for Minecraft {minecraft}.",
            kind.name()
        ));
    }
    Ok(listed)
}

/// The loader `kind` names at `version` (none or `latest`: the newest stable one) for
/// `minecraft`, checked against its service's list.
pub fn choose(kind: &str, minecraft: &str, version: Option<&str>) -> Result<Loader, String> {
    let kind = Kind::parse(kind)?;
    let listed = versions(kind, minecraft, &crate::backend::get_bytes)?;
    Ok(Loader {
        kind,
        version: pick(kind, &listed, version)?,
    })
}

/// `wanted` among them: a version, or none or `latest` for the newest stable one.
pub fn pick(kind: Kind, listed: &[Listed], wanted: Option<&str>) -> Result<String, String> {
    let found = match wanted.map(str::trim) {
        None | Some("latest") => listed.iter().find(|l| l.stable).or(listed.first()),
        Some(version) => listed.iter().find(|l| l.version == version),
    };
    found.map(|l| l.version.clone()).ok_or_else(|| {
        format!(
            "{} has no loader {} for this Minecraft version.",
            kind.name(),
            wanted.unwrap_or("latest")
        )
    })
}

/// A library's file in a Maven repository: `group:artifact:version[:classifier][@extension]`.
pub fn maven_path(name: &str) -> Result<String, String> {
    let (coordinates, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = coordinates.split(':').collect();
    let (group, artifact, version, classifier) = match parts[..] {
        [g, a, v] => (g, a, v, None),
        [g, a, v, c] => (g, a, v, Some(c)),
        _ => return Err(format!("Bad library name {name:?}")),
    };
    let file = match classifier {
        Some(c) => format!("{artifact}-{version}-{c}.{extension}"),
        None => format!("{artifact}-{version}.{extension}"),
    };
    Ok(format!(
        "{}/{artifact}/{version}/{file}",
        group.replace('.', "/")
    ))
}

/// Downloads `loader`'s profile for `minecraft` and its libraries into the game folder `dir`, and
/// keeps the profile in `versions/` with each library's file named the way Mojang's are
/// (`downloads.artifact`). Returns the profile's id.
pub fn install(
    loader: &Loader,
    minecraft: &str,
    dir: &Path,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<String, String> {
    if matches!(loader.kind, Kind::Forge | Kind::NeoForge) {
        return Err(format!("Horizon can't install {} yet.", loader.kind.name()));
    }
    let name = loader.kind.name();
    let url = format!(
        "{}/versions/loader/{minecraft}/{}/profile/json",
        loader.kind.meta(),
        loader.version
    );
    let mut profile: Value =
        serde_json::from_slice(&fetch(&url)?).map_err(|e| format!("{name}'s profile: {e}"))?;
    if profile["inheritsFrom"] != minecraft {
        return Err(format!(
            "{name}'s profile is for {}, not Minecraft {minecraft}",
            profile["inheritsFrom"]
        ));
    }
    let id = profile["id"]
        .as_str()
        .ok_or_else(|| format!("{name}'s profile has no id"))?
        .to_string();
    let libraries = profile["libraries"]
        .as_array_mut()
        .ok_or_else(|| format!("{name}'s profile has no libraries"))?;
    log(format!(
        "Downloading {name} {} ({} libraries)",
        loader.version,
        libraries.len()
    ));
    for library in libraries.iter_mut() {
        let path = maven_path(library["name"].as_str().ok_or("A library has no name")?)?;
        let repository = library["url"]
            .as_str()
            .unwrap_or("https://repo1.maven.org/maven2/");
        let url = format!("{}/{path}", repository.trim_end_matches('/'));
        let sha1 = match library["sha1"].as_str() {
            Some(sha1) => sha1.to_string(),
            None => String::from_utf8_lossy(&fetch(&format!("{url}.sha1"))?)
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string(),
        };
        let file = dir.join("libraries").join(crate::mojang::relative(&path)?);
        if sha1_file(&file).as_ref() != Some(&sha1) {
            let body = fetch_checked(fetch, &url, &sha1).map_err(|e| format!("{url} {e}"))?;
            write_file(&file, &body)?;
        }
        library["downloads"] = json!({"artifact": {"path": path, "url": url, "sha1": sha1}});
    }
    let out = dir
        .join("versions")
        .join(crate::mojang::relative(&id)?)
        .join(format!("{id}.json"));
    write_file(
        &out,
        &serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?,
    )?;
    Ok(id)
}

#[cfg(test)]
#[path = "loaders/tests.rs"]
mod tests;
