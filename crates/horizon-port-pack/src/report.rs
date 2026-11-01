//! Step 7, the coverage report: per mod, ported, partial, not yet or not
//! needed, with the reason, and the totals by class, tier and licence
//! (`coverage.json`, `coverage.md`). It reads what the other steps wrote:
//! the inventory, each mod's `carry-report.json` and its harness result
//! (`tools/mod_port_harness.py`).
use crate::{
    inventory::Record,
    resourcepacks::{Outcome, Report, State},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fmt::Write, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Ported,
    Partial,
    NotYet,
    NotNeeded,
}
impl Status {
    pub const ALL: [Status; 4] = [
        Status::Ported,
        Status::Partial,
        Status::NotYet,
        Status::NotNeeded,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Status::Ported => "ported",
            Status::Partial => "partial",
            Status::NotYet => "not yet",
            Status::NotNeeded => "not needed",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Row {
    pub key: String,
    pub name: String,
    pub class: String,
    pub tier: String,
    pub license: String,
    /// `permissive`, `copyleft`, `all-rights-reserved`, `restricted`
    /// (non-commercial or no-derivatives), `custom` or `unknown`.
    pub license_kind: String,
    pub status: Status,
    pub reason: String,
}

/// The licence family of an SPDX id or a `mods.toml` licence text.
pub fn license_kind(license: &str) -> &'static str {
    let l = license.to_ascii_lowercase();
    if l.is_empty() || l.starts_with("unknown") {
        "unknown"
    } else if l.contains("-nc") || l.contains("-nd") || l.contains("noncommercial") {
        "restricted"
    } else if l.contains("all-rights-reserved") || l.contains("all rights reserved") || l == "arr" {
        "all-rights-reserved"
    } else if ["gpl", "mpl", "eupl", "general public"]
        .iter()
        .any(|k| l.contains(k))
    {
        "copyleft"
    } else if [
        "mit",
        "apache",
        "bsd",
        "cc0",
        "cc-by",
        "unlicense",
        "isc",
        "zlib",
    ]
    .iter()
    .any(|k| l.contains(k))
    {
        "permissive"
    } else {
        "custom"
    }
}

/// What still blocks a mod of this tier that has not been carried over.
fn not_yet(tier: &str) -> &'static str {
    match tier {
        "data" => "data tier: carry-over only, no agent; its jar is not here yet (fetch gets it)",
        "logic" => {
            "needs mod API 0.3: behaviour callbacks (use, place, break, ticks) and server events"
        }
        "machines" => "needs mod API 0.4: block entities, menus, capabilities and mod networking",
        "deep" => {
            "needs mod API 0.5: entities, custom renderers and models, dimensions, new vanilla hooks"
        }
        "client" => "needs the client interfaces (keys, target, inventory-read, ui, render)",
        _ => "not classified",
    }
}

fn not_needed(class: &str) -> &'static str {
    match class {
        "library" => "library: its API maps to Horizon's mod API and SDK",
        "performance" => "performance mod: Horizon's engine is the replacement",
        "other" => "pack tooling or an engine feature, folded into the pipeline or the engine",
        "client-ui" => "built into Horizon",
        _ => "nothing to port",
    }
}

/// A carried-over mod: ported when its harness passes with nothing left
/// out and nothing the templates miss; else partial, with what is missing.
fn carried(work: &Path, key: &str) -> Option<(Status, String)> {
    let dir = work.join("mods").join(key);
    let carry: Value =
        serde_json::from_slice(&std::fs::read(dir.join("carry-report.json")).ok()?).ok()?;
    let count = |k: &str| carry[k].as_object().map_or(0, |o| o.len());
    let (blocks, items) = (count("blocks"), carry["items"].as_u64().unwrap_or(0));
    let (left_blocks, left_items) = (count("blocks_unknown"), count("items_unknown"));
    // A class of its own is code the facts cannot show; anything else the
    // template misses (shapes, sounds) the harness also reports.
    let mut differences = 0;
    let mut own_code = 0;
    for list in carry["differences"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, v)| v)
    {
        let notes: Vec<&str> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if notes.iter().any(|n| n.starts_with("class ")) {
            own_code += 1;
        }
        if notes.iter().any(|n| !n.starts_with("class ")) {
            differences += 1;
        }
    }
    let mut gaps = Vec::new();
    if left_blocks > 0 {
        gaps.push(format!(
            "{left_blocks} blocks left out (no vanilla template)"
        ));
    }
    if left_items > 0 {
        gaps.push(format!("{left_items} items left out"));
    }
    if differences > 0 {
        gaps.push(format!(
            "{differences} blocks or items differ from their templates"
        ));
    }
    let harness: Option<Value> = std::fs::read(dir.join("harness/result.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let mut checked = 0;
    match &harness {
        Some(h) => {
            for (name, c) in h["checks"].as_object().into_iter().flatten() {
                let (p, f) = (
                    c["passed"].as_u64().unwrap_or(0),
                    c["failed"].as_u64().unwrap_or(0),
                );
                checked += p + f;
                if f > 0 {
                    gaps.push(format!("{name} {p}/{}", p + f));
                }
            }
        }
        None => gaps.push("harness not run".into()),
    }
    let worldgen = carry["worldgen"].as_u64().unwrap_or(0);
    if blocks == 0 && items == 0 && checked == 0 {
        let what = if worldgen > 0 {
            format!("{worldgen} worldgen files carried, not compared yet")
        } else {
            "nothing registered to compare".into()
        };
        return Some((Status::Partial, what));
    }
    let mut summary = format!("{blocks} blocks, {items} items");
    if own_code > 0 {
        let _ = write!(
            summary,
            " ({own_code} with code of their own, not compared)"
        );
    }
    if gaps.is_empty() {
        Some((Status::Ported, format!("{summary}; harness green")))
    } else {
        Some((Status::Partial, format!("{summary}; {}", gaps.join("; "))))
    }
}

fn resource_row(path: &str, outcome: Option<&Outcome>, record: Option<&Record>) -> Row {
    let state = outcome.map_or(State::Missing, |o| o.state);
    let shader = path.starts_with("shaderpacks/");
    Row {
        key: path.into(),
        name: path.into(),
        class: if shader { "shaderpack" } else { "resourcepack" }.into(),
        tier: "data".into(),
        license: record.map_or("unknown", |r| r.license.as_str()).into(),
        license_kind: license_kind(record.map_or("unknown", |r| r.license.as_str())).into(),
        status: if !shader && state == State::Available {
            Status::Partial
        } else {
            Status::NotYet
        },
        reason: outcome.map_or_else(
            || {
                if shader {
                    "shader-pack conversion is pending".into()
                } else {
                    "resource pack not obtained yet; run port".into()
                }
            },
            |o| format!("{:?}: {}", o.state, o.detail),
        ),
    }
}

pub fn build(records: &[Record], work: &Path) -> Vec<Row> {
    let resourcepacks = Report::load(work);
    let mut rows: Vec<Row> = records
        .iter()
        .map(|r| {
            if r.path.starts_with("resourcepacks/") || r.path.starts_with("shaderpacks/") {
                return resource_row(&r.path, resourcepacks.files.get(&r.path), Some(r));
            }
            let (status, reason) = if r.class == "unknown" {
                let why =
                    "unclassified: its jar (mods.toml, file census) or a reviewed row decides";
                (Status::NotYet, why.to_owned())
            } else if r.tier == "none" || matches!(r.class.as_str(), "library" | "performance") {
                (Status::NotNeeded, not_needed(&r.class).to_owned())
            } else if let Some(c) = carried(work, &r.key) {
                c
            } else {
                let mut why = not_yet(&r.tier).to_owned();
                if r.tier == "data" && r.jar.is_some() {
                    why = "data tier: carry-over not run yet".into();
                }
                if r.class == "integration" {
                    why = format!("after the mods it joins are ported; {why}");
                }
                (Status::NotYet, why)
            };
            Row {
                key: r.key.clone(),
                name: r.name.clone(),
                class: r.class.clone(),
                tier: r.tier.clone(),
                license: r.license.clone(),
                license_kind: license_kind(&r.license).to_owned(),
                status,
                reason,
            }
        })
        .collect();
    // A mod waits on its dependencies that are not ported.
    let status: BTreeMap<String, Status> = rows.iter().map(|r| (r.key.clone(), r.status)).collect();
    for (row, r) in rows.iter_mut().zip(records) {
        if row.status == Status::NotNeeded {
            continue;
        }
        let waiting: Vec<&str> = r
            .depends
            .iter()
            .filter(|d| status.get(*d) == Some(&Status::NotYet))
            .map(String::as_str)
            .collect();
        if !waiting.is_empty() {
            let _ = write!(row.reason, "; waits on {}", waiting.join(", "));
        }
    }
    for (path, outcome) in &resourcepacks.files {
        if !records.iter().any(|r| &r.path == path) {
            rows.push(resource_row(path, Some(outcome), None));
        }
    }
    rows
}

/// Rows counted by `field` and status.
fn table(rows: &[Row], title: &str, field: fn(&Row) -> &str) -> String {
    let mut counts: BTreeMap<&str, [usize; 4]> = BTreeMap::new();
    for r in rows {
        let i = Status::ALL.iter().position(|s| *s == r.status).unwrap_or(0);
        counts.entry(field(r)).or_default()[i] += 1;
    }
    let mut out = format!(
        "| {title} | ported | partial | not yet | not needed | all |\n|---|---:|---:|---:|---:|---:|\n"
    );
    let mut totals = [0usize; 4];
    for (k, c) in &counts {
        let all: usize = c.iter().sum();
        let _ = writeln!(
            out,
            "| {k} | {} | {} | {} | {} | {all} |",
            c[0], c[1], c[2], c[3]
        );
        for (t, n) in totals.iter_mut().zip(c) {
            *t += n;
        }
    }
    let all: usize = totals.iter().sum();
    let _ = writeln!(
        out,
        "| **all** | **{}** | **{}** | **{}** | **{}** | **{all}** |",
        totals[0], totals[1], totals[2], totals[3]
    );
    out
}

pub fn markdown(pack: &str, rows: &[Row]) -> String {
    let mut out = format!("# Coverage: {pack}\n\n");
    out += &table(rows, "class", |r| &r.class);
    out += "\n";
    out += &table(rows, "tier", |r| &r.tier);
    out += "\n";
    out += &table(rows, "licence", |r| &r.license_kind);
    out += "\nPorts stay local. Only permissive or copyleft ports could ever be published, and only after Ludovic asks the authors.\n\n";
    out += "| mod | class | tier | licence | status | why |\n|---|---|---|---|---|---|\n";
    let mut sorted: Vec<&Row> = rows.iter().collect();
    sorted.sort_by(|a, b| (a.status, &a.key).cmp(&(b.status, &b.key)));
    for r in sorted {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            r.key,
            r.class,
            r.tier,
            r.license.replace('|', "/"),
            r.status.label(),
            r.reason.replace('|', "/")
        );
    }
    out
}

pub fn counts(rows: &[Row]) -> Vec<(&'static str, usize)> {
    Status::ALL
        .iter()
        .map(|s| (s.label(), rows.iter().filter(|r| r.status == *s).count()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: &str, class: &str, tier: &str, depends: &[&str]) -> Record {
        Record {
            key: key.into(),
            class: class.into(),
            tier: tier.into(),
            license: "MIT".into(),
            depends: depends.iter().map(|d| (*d).to_owned()).collect(),
            ..Record::default()
        }
    }

    #[test]
    fn statuses_and_reasons() {
        let work = std::env::temp_dir().join(format!("horizon-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        let write = |key: &str, carry: Value, harness: Option<Value>| {
            let dir = work.join("mods").join(key);
            std::fs::create_dir_all(dir.join("harness")).unwrap();
            std::fs::write(dir.join("carry-report.json"), carry.to_string()).unwrap();
            if let Some(h) = harness {
                std::fs::write(dir.join("harness/result.json"), h.to_string()).unwrap();
            }
        };
        let green = serde_json::json!({"checks": {"blocks": {"passed": 3, "failed": 0}}});
        write(
            "doors",
            serde_json::json!({"blocks": {"a": "minecraft:oak_door"}, "items": 0,
                "differences": {"a": ["class m.Door (behaviour beyond DoorBlock)"]}}),
            Some(green),
        );
        write(
            "paths",
            serde_json::json!({"blocks": {"a": "minecraft:stone"}, "items": 1,
                "blocks_unknown": {"b": "no vanilla Block"}}),
            Some(serde_json::json!({"checks": {"shapes": {"passed": 1, "failed": 2}}})),
        );
        let records = [
            record("doors", "content", "data", &[]),
            record("paths", "content", "data", &[]),
            record("lib", "library", "none", &[]),
            record("machine", "content", "machines", &["lib"]),
            record("addon", "integration", "logic", &["machine"]),
            record("mystery", "unknown", "none", &[]),
        ];
        let rows = build(&records, &work);
        let by: BTreeMap<&str, &Row> = rows.iter().map(|r| (r.key.as_str(), r)).collect();
        assert_eq!(by["doors"].status, Status::Ported);
        assert!(
            by["doors"].reason.contains("1 with code of their own"),
            "{}",
            by["doors"].reason
        );
        assert_eq!(by["paths"].status, Status::Partial);
        assert!(
            by["paths"].reason.contains("1 blocks left out"),
            "{}",
            by["paths"].reason
        );
        assert!(
            by["paths"].reason.contains("shapes 1/3"),
            "{}",
            by["paths"].reason
        );
        assert_eq!(by["lib"].status, Status::NotNeeded);
        assert_eq!(by["machine"].status, Status::NotYet);
        assert!(by["machine"].reason.contains("0.4"));
        assert!(by["addon"].reason.starts_with("after the mods it joins"));
        assert!(
            by["addon"].reason.ends_with("waits on machine"),
            "{}",
            by["addon"].reason
        );
        assert_eq!(by["mystery"].status, Status::NotYet);
        assert_eq!(
            counts(&rows),
            [
                ("ported", 1),
                ("partial", 1),
                ("not yet", 3),
                ("not needed", 1)
            ]
        );
        let md = markdown("test", &rows);
        assert!(
            md.contains("| **all** | **1** | **1** | **3** | **1** | **6** |"),
            "{md}"
        );
        std::fs::remove_dir_all(&work).unwrap();
    }

    #[test]
    fn licence_families() {
        assert_eq!(license_kind("MIT"), "permissive");
        assert_eq!(license_kind("CC0-1.0"), "permissive");
        assert_eq!(license_kind("LGPL-3.0-only"), "copyleft");
        assert_eq!(
            license_kind("GNU Lesser General Public License v3.0"),
            "copyleft"
        );
        assert_eq!(
            license_kind("LicenseRef-All-Rights-Reserved"),
            "all-rights-reserved"
        );
        assert_eq!(license_kind("CC-BY-NC-SA-4.0"), "restricted");
        assert_eq!(license_kind("LicenseRef-Stardust-Labs-License"), "custom");
        assert_eq!(
            license_kind("unknown (read mods.toml at import)"),
            "unknown"
        );
    }
}
