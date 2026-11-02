//! Resolve bundle dependencies without touching the staged workspace.
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

#[derive(Debug)]
pub(super) enum Error {
    Manifest {
        owner: String,
        target: String,
        reason: String,
    },
    Duplicate(String),
    Missing {
        owner: String,
        target: String,
    },
    NoApi {
        owner: String,
        target: String,
    },
    CargoConflict {
        owner: String,
        target: String,
    },
    Version {
        owner: String,
        target: String,
        version: String,
        requirement: String,
    },
    Cycle {
        mods: Vec<String>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest {
                owner,
                target,
                reason,
            } => write!(f, "{owner}: dependency {target}: {reason}"),
            Self::Duplicate(id) => write!(f, "Duplicate native mod id {id:?}"),
            Self::Missing { owner, target } => write!(f, "{owner} requires missing mod {target}"),
            Self::NoApi { owner, target } => {
                write!(f, "{owner} depends on {target}, which has no [api] package")
            }
            Self::CargoConflict { owner, target } => {
                write!(
                    f,
                    "{owner}: dependency {target} conflicts with its Cargo manifest"
                )
            }
            Self::Version {
                owner,
                target,
                version,
                requirement,
            } => write!(
                f,
                "{owner} requires {target} {requirement}, but installed version is {version}"
            ),
            Self::Cycle { mods } => write!(f, "Mod dependency cycle: {}", mods.join(" -> ")),
        }
    }
}

#[derive(Debug)]
pub(super) struct Dependency {
    pub id: String,
    pub requirement: VersionReq,
    pub optional: bool,
}

pub(super) fn parse(owner: &str, value: Option<&toml::Value>) -> Result<Vec<Dependency>, Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let invalid = |target: &str, reason: &str| Error::Manifest {
        owner: owner.into(),
        target: target.into(),
        reason: reason.into(),
    };
    let table = value
        .as_table()
        .ok_or_else(|| invalid("[dependencies]", "expected a table"))?;
    table
        .iter()
        .map(|(id, value)| {
            if !super::api::valid_id(id) {
                return Err(invalid(id, "invalid mod id"));
            }
            let (version, optional) = if let Some(version) = value.as_str() {
                (version, false)
            } else if let Some(details) = value.as_table() {
                if details
                    .keys()
                    .any(|key| key != "version" && key != "optional")
                {
                    return Err(invalid(id, "only version and optional are accepted"));
                }
                let version = details
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| invalid(id, "version must be a string"))?;
                let optional = match details.get("optional") {
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| invalid(id, "optional must be a boolean"))?,
                    None => false,
                };
                (version, optional)
            } else {
                return Err(invalid(id, "expected a version string or table"));
            };
            let requirement =
                VersionReq::parse(version).map_err(|e| invalid(id, &e.to_string()))?;
            Ok(Dependency {
                id: id.clone(),
                requirement,
                optional,
            })
        })
        .collect()
}

pub(super) struct Mod<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub has_api: bool,
    pub dependencies: &'a [Dependency],
}

pub(super) struct Resolution {
    /// Original bundle indices, in dispatch order.
    pub order: Vec<usize>,
}

pub(super) fn resolve(mods: &[Mod<'_>]) -> Result<Resolution, Error> {
    let mut ids = HashMap::new();
    for (index, item) in mods.iter().enumerate() {
        if ids.insert(item.id, index).is_some() {
            return Err(Error::Duplicate(item.id.into()));
        }
    }
    let mut edges = vec![Vec::new(); mods.len()];
    for (index, item) in mods.iter().enumerate() {
        for dep in item.dependencies {
            let Some(&target) = ids.get(dep.id.as_str()) else {
                if dep.optional {
                    continue;
                }
                return Err(Error::Missing {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                });
            };
            let other = &mods[target];
            if !item.has_api || !other.has_api {
                return Err(Error::NoApi {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                });
            }
            // A `[mod] version` may leave out minor and patch (`"1"`, `"1.2"`), as Cargo's
            // requirements do: they count as zero.
            let padded = match other.version.matches('.').count() {
                0 => format!("{}.0.0", other.version),
                1 => format!("{}.0", other.version),
                _ => other.version.to_owned(),
            };
            let version = Version::parse(&padded).map_err(|e| Error::Manifest {
                owner: item.id.into(),
                target: dep.id.clone(),
                reason: format!("invalid installed version {:?}: {e}", other.version),
            })?;
            if !dep.requirement.matches(&version) {
                return Err(Error::Version {
                    owner: item.id.into(),
                    target: dep.id.clone(),
                    version: other.version.into(),
                    requirement: dep.requirement.to_string(),
                });
            }
            edges[index].push(target);
        }
    }
    let mut done = vec![false; mods.len()];
    let mut order = Vec::with_capacity(mods.len());
    while order.len() < mods.len() {
        let next = (0..mods.len()).find(|&i| !done[i] && edges[i].iter().all(|&d| done[d]));
        let Some(next) = next else {
            // Follow remaining edges until a node repeats, reporting the actual cycle only.
            let mut seen = BTreeMap::new();
            let mut path: Vec<usize> = Vec::new();
            let mut current = done.iter().position(|done| !done).expect("unfinished node");
            loop {
                if let Some(&start) = seen.get(&current) {
                    let mut cycle: Vec<String> = path[start..]
                        .iter()
                        .map(|&i| mods[i].id.to_owned())
                        .collect();
                    cycle.push(mods[current].id.to_owned());
                    return Err(Error::Cycle { mods: cycle });
                }
                seen.insert(current, path.len());
                path.push(current);
                current = *edges[current]
                    .iter()
                    .find(|&&i| !done[i])
                    .expect("unfinished dependency");
            }
        };
        done[next] = true;
        order.push(next);
    }
    Ok(Resolution { order })
}

/// A target's staged Cargo package, or none for an absent optional mod.
pub(super) struct Package<'a> {
    pub dependency: &'a Dependency,
    pub package: Option<&'a str>,
}

/// Add bundle edges to the dependent package. Returns features to enable on the game edge.
pub(super) fn rewrite(
    owner: &str,
    manifest: &mut toml::Table,
    packages: &[Package<'_>],
) -> Result<Vec<String>, Error> {
    let mut updated = manifest.clone();
    let mut enabled = Vec::new();
    for target in packages {
        let id = &target.dependency.id;
        let normalized = id.replace('-', "_");
        let conflict = || Error::CargoConflict {
            owner: owner.into(),
            target: id.clone(),
        };
        let dependencies = updated
            .entry("dependencies")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(conflict)?;
        if dependencies
            .keys()
            .any(|key| key.replace('-', "_") == normalized)
        {
            return Err(conflict());
        }
        if let Some(package) = target.package {
            let mut entry = toml::Table::new();
            entry.insert("path".into(), format!("../{id}").into());
            entry.insert("package".into(), package.into());
            if target.dependency.optional {
                entry.insert("optional".into(), true.into());
            }
            dependencies.insert(id.clone(), toml::Value::Table(entry));
        } else if !target.dependency.optional {
            return Err(Error::Missing {
                owner: owner.into(),
                target: id.clone(),
            });
        }
        if target.dependency.optional {
            let features = updated
                .entry("features")
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(conflict)?;
            if features.contains_key(id)
                || features.values().any(|value| {
                    value.as_array().is_some_and(|entries| {
                        entries.iter().any(|entry| {
                            entry.as_str().is_some_and(|s| {
                                s == id
                                    || s == format!("dep:{id}")
                                    || s.starts_with(&format!("{id}/"))
                                    || s.starts_with(&format!("{id}?/"))
                            })
                        })
                    })
                })
            {
                return Err(conflict());
            }
            let entries = if target.package.is_some() {
                enabled.push(id.clone());
                vec![toml::Value::String(format!("dep:{id}"))]
            } else {
                Vec::new()
            };
            features.insert(id.clone(), toml::Value::Array(entries));
        }
    }
    *manifest = updated;
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn deps(text: &str) -> Vec<Dependency> {
        let value: toml::Table = text.parse().unwrap();
        parse("user", value.get("dependencies")).unwrap()
    }
    fn item<'a>(id: &'a str, version: &'a str, dependencies: &'a [Dependency]) -> Mod<'a> {
        Mod {
            id,
            version,
            has_api: true,
            dependencies,
        }
    }
    #[test]
    fn order_and_ties_include_present_optional() {
        let user = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mods = [
            item("user", "1.0.0", &user),
            item("base", "1.2", &[]),
            item("extra", "2", &[]),
            item("other", "1.0.0", &[]),
        ];
        let result = resolve(&mods).unwrap();
        assert_eq!(result.order, [1, 2, 0, 3]);
        assert_eq!(
            resolve(&[item("user", "1.0.0", &user), item("base", "1.2.0", &[])])
                .unwrap()
                .order,
            [1, 0]
        );
    }
    #[test]
    fn failures_name_mods() {
        let required = deps("[dependencies]\nbase = '^1'");
        let optional = deps("[dependencies]\nbase = { version = '^1', optional = true }");
        for dependencies in [&required, &optional] {
            let err = resolve(&[
                item("user", "1.0.0", dependencies),
                item("base", "2.0.0", &[]),
            ])
            .err()
            .unwrap()
            .to_string();
            assert!(err.contains("user") && err.contains("base"));
        }
        let err = resolve(&[item("user", "1.0.0", &required)])
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("user") && err.contains("base"));
        let back = deps("[dependencies]\nuser = '^1'");
        let err = resolve(&[
            item("user", "1.0.0", &required),
            item("base", "1.0.0", &back),
        ])
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("user") && err.contains("base"));
    }
    #[test]
    fn rewrites_installed_and_absent_optional_dependencies() {
        let deps = deps("[dependencies]\nbase = '^1'\nextra = { version = '^2', optional = true }");
        let mut manifest: toml::Table = "[dependencies]\nhorizon-client = '1'".parse().unwrap();
        let packages = [
            Package {
                dependency: &deps[0],
                package: Some("base-package"),
            },
            Package {
                dependency: &deps[1],
                package: Some("extra-package"),
            },
        ];
        assert_eq!(
            rewrite("user", &mut manifest, &packages).unwrap(),
            ["extra"]
        );
        assert_eq!(
            manifest["dependencies"]["base"]["path"].as_str(),
            Some("../base")
        );
        assert_eq!(
            manifest["dependencies"]["base"]["package"].as_str(),
            Some("base-package")
        );
        assert_eq!(
            manifest["dependencies"]["extra"]["optional"].as_bool(),
            Some(true)
        );
        assert_eq!(manifest["features"]["extra"][0].as_str(), Some("dep:extra"));
        let mut absent = toml::Table::new();
        assert!(rewrite(
            "user",
            &mut absent,
            &[Package {
                dependency: &deps[1],
                package: None
            }]
        )
        .unwrap()
        .is_empty());
        assert!(absent["features"]["extra"].as_array().unwrap().is_empty());
        assert!(absent["dependencies"].get("extra").is_none());
    }

    #[test]
    fn rejects_conflicting_features_and_dependency_names_atomically() {
        let deps = deps("[dependencies]\nextra = { version = '^2', optional = true }");
        for text in [
            "[features]\ndefault = ['extra']",
            "[dependencies]\nextra = '1'",
        ] {
            let mut manifest: toml::Table = text.parse().unwrap();
            let original = manifest.clone();
            assert!(rewrite(
                "user",
                &mut manifest,
                &[Package {
                    dependency: &deps[0],
                    package: Some("extra-package")
                }]
            )
            .is_err());
            assert_eq!(manifest, original);
        }
    }

    #[test]
    fn malformed_and_no_api() {
        let value: toml::Table = "[dependencies]\nbase = { version = '^1', optional = 'yes' }"
            .parse()
            .unwrap();
        assert!(parse("user", value.get("dependencies")).is_err());
        let required = deps("[dependencies]\nbase = '^1'");
        let mut base = item("base", "1.0.0", &[]);
        base.has_api = false;
        assert!(matches!(
            resolve(&[item("user", "1.0.0", &required), base]),
            Err(Error::NoApi { .. })
        ));
    }
}
