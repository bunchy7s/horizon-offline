//! A world's data packs over the prepared worldgen bundles.
//!
//! Vanilla builds its worldgen registries from the enabled packs in order:
//! the built-in pack first, then each enabled pack, a later pack's file for
//! an id replacing an earlier one's. Tags are the exception: every pack's
//! file for a tag adds its entries, and `replace` drops what came before.
//! The prepared bundles are the built-in pack, so the packs above it are
//! entries that replace or join a bundle's (`GeneratorData::load_with`).
use serde_json::Value;
use std::collections::BTreeMap;

/// The bundle of `dimension/<id>.json` level stems. Nothing prepared has
/// it: a pack's level stem stands in for the world preset's.
pub const DIMENSIONS: &str = "dimensions";

/// Pack folders (`data/<ns>/<kind>/`) the chunk generator reads, and the
/// bundle each joins.
const KINDS: [(&str, &str); 14] = [
    ("dimension", DIMENSIONS),
    ("worldgen/biome", "biomes"),
    // 26.3's names: 26.2 called them configured_carver and configured_feature.
    ("worldgen/carver", "carvers"),
    ("worldgen/feature", "features"),
    ("worldgen/density_function", "densities"),
    ("worldgen/material_condition", "conditions"),
    ("worldgen/material_rule", "rules"),
    ("worldgen/noise", "noises"),
    ("worldgen/noise_settings", "settings"),
    ("worldgen/placed_feature", "placed_features"),
    ("worldgen/processor_list", "processor_lists"),
    ("worldgen/structure", "structures"),
    ("worldgen/structure_set", "structure_sets"),
    ("worldgen/template_pool", "template_pools"),
];
/// Tag folders the chunk generator reads, and the tag bundle each joins.
const TAG_KINDS: [(&str, &str); 3] = [
    ("tags/block", "block_tags"),
    ("tags/fluid", "fluid_tags"),
    ("tags/worldgen/biome", "biome_tags"),
];
/// The pack folders `add_file` looks at, in walk order.
pub const PACK_KINDS: [&str; 4] = ["dimension", "dimension_type", "tags", "worldgen"];

/// `ns:path` for a pack file `data/<ns>/<kind>/<path>.json`.
fn resource_id(file: &str, kind: &str) -> Option<String> {
    let (ns, rest) = file.strip_prefix("data/")?.split_once('/')?;
    let path = rest
        .strip_prefix(kind)?
        .strip_prefix('/')?
        .strip_suffix(".json")?;
    Some(format!("{ns}:{path}"))
}

/// What `add_file` made of a pack file.
#[derive(Debug, PartialEq, Eq)]
pub enum PackFile {
    /// An entry or tag file the generator reads.
    Read,
    /// A dimension type: Horizon keeps each dimension's vanilla build
    /// height, so it is not read.
    DimensionType { id: String, min_y: i64, height: i64 },
    /// A worldgen file of a kind the generator does not read (its folder).
    NotRead(String),
    /// A generator file whose text could not be read.
    Unreadable,
    /// Not worldgen.
    Other,
}

/// Replacement and added bundle entries, and tag files, from data packs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct WorldgenOverrides {
    /// Bundle name → id → the highest pack's JSON text for it.
    entries: BTreeMap<String, BTreeMap<String, String>>,
    /// Tag bundle name → (tag id, pack id, tag file text), lowest pack first.
    tags: BTreeMap<String, Vec<(String, String, String)>>,
}
impl WorldgenOverrides {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.tags.is_empty()
    }
    /// A pack's file for `id` in `bundle`: called for packs from the lowest
    /// to the highest, so the last call for an id wins.
    pub fn set(&mut self, bundle: &str, id: &str, json: String) {
        self.entries
            .entry(bundle.to_owned())
            .or_default()
            .insert(id.to_owned(), json);
    }
    /// A pack's tag file for the tag `id` in the tag bundle `bundle`, in
    /// pack order.
    pub fn add_tag(&mut self, bundle: &str, id: &str, pack: &str, json: String) {
        self.tags.entry(bundle.to_owned()).or_default().push((
            id.to_owned(),
            pack.to_owned(),
            json,
        ));
    }
    /// The file `file` (`data/<ns>/...`, relative to the pack root) of the
    /// pack `pack`, whose text `read` gives. Packs go from the lowest to the
    /// highest, each in its own file order.
    pub fn add_file(
        &mut self,
        pack: &str,
        file: &str,
        read: impl FnOnce() -> Option<String>,
    ) -> PackFile {
        let Some((_, rest)) = file.strip_prefix("data/").and_then(|r| r.split_once('/')) else {
            return PackFile::Other;
        };
        if let Some((bundle, id)) = KINDS
            .iter()
            .find_map(|(kind, bundle)| Some((bundle, resource_id(file, kind)?)))
        {
            let Some(json) = read() else {
                return PackFile::Unreadable;
            };
            self.set(bundle, &id, json);
            PackFile::Read
        } else if let Some((bundle, id)) = TAG_KINDS
            .iter()
            .find_map(|(kind, bundle)| Some((bundle, resource_id(file, kind)?)))
        {
            let Some(json) = read() else {
                return PackFile::Unreadable;
            };
            self.add_tag(bundle, &id, pack, json);
            PackFile::Read
        } else if let Some(id) = resource_id(file, "dimension_type") {
            let v: Value = read()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or_default();
            PackFile::DimensionType {
                id,
                min_y: v["min_y"].as_i64().unwrap_or_default(),
                height: v["height"].as_i64().unwrap_or_default(),
            }
        } else if rest.starts_with("worldgen/") || rest.starts_with("tags/worldgen/") {
            PackFile::NotRead(rest.rsplit_once('/').map_or(rest, |(k, _)| k).to_owned())
        } else {
            PackFile::Other
        }
    }
    /// After the last pack: leaves out the packs' new biomes (`known` says
    /// which the game has), and what names them names the rest without
    /// them: their biome tags' other biomes, and the climate points of the
    /// other biomes in a level stem's multi-noise list (the left-out
    /// biome's climate goes to its nearest neighbours). Returns what it
    /// left out, for the log.
    pub fn finish(&mut self, known: impl Fn(&str) -> bool) -> Vec<String> {
        let left_out = self.retain("biomes", &known);
        let mut warnings: Vec<String> = left_out
            .iter()
            .map(|id| {
                format!("biome {id} is new: Horizon cannot add biomes yet, so it is left out")
            })
            .collect();
        if left_out.is_empty() {
            return warnings;
        }
        let gone = |v: &Value| {
            let id = v.as_str().or_else(|| v["id"].as_str()).unwrap_or_default();
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            };
            left_out.contains(&id)
        };
        for (_, _, json) in self.tags.get_mut("biome_tags").into_iter().flatten() {
            if let Ok(mut v) = serde_json::from_str::<Value>(json)
                && let Some(values) = v["values"].as_array_mut()
            {
                let before = values.len();
                values.retain(|e| !gone(e));
                if values.len() != before {
                    *json = v.to_string();
                }
            }
        }
        for (id, json) in self.entries.get_mut(DIMENSIONS).into_iter().flatten() {
            let Ok(mut v) = serde_json::from_str::<Value>(json) else {
                continue;
            };
            let Some(points) = v["generator"]["biome_source"]["biomes"].as_array_mut() else {
                continue;
            };
            let before = points.len();
            points.retain(|p| !gone(&p["biome"]));
            if points.len() != before {
                warnings.push(format!(
                    "dimension {id}: {} of its {before} biome climate points name left-out \
                     biomes and are removed",
                    before - points.len()
                ));
                *json = v.to_string();
            }
        }
        warnings
    }
    /// The entries packs give `bundle`.
    pub fn entries(&self, bundle: &str) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .get(bundle)
            .into_iter()
            .flatten()
            .map(|(id, json)| (id.as_str(), json.as_str()))
    }
    /// The tag files packs give the tag bundle `bundle`, in pack order.
    pub fn tags(&self, bundle: &str) -> impl Iterator<Item = (&str, &str, &str)> {
        self.tags
            .get(bundle)
            .into_iter()
            .flatten()
            .map(|(id, pack, json)| (id.as_str(), pack.as_str(), json.as_str()))
    }
    /// Removes the entries of `bundle` that `keep` refuses, returning their ids.
    pub fn retain(&mut self, bundle: &str, mut keep: impl FnMut(&str) -> bool) -> Vec<String> {
        let Some(entries) = self.entries.get_mut(bundle) else {
            return Vec::new();
        };
        let dropped: Vec<String> = entries.keys().filter(|id| !keep(id)).cloned().collect();
        for id in &dropped {
            entries.remove(id);
        }
        if entries.is_empty() {
            self.entries.remove(bundle);
        }
        dropped
    }
    /// `bundle` with the packs' entries over it.
    pub fn apply(
        &self,
        bundle: &str,
        mut base: BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        for (id, json) in self.entries(bundle) {
            base.insert(id.to_owned(), json.to_owned());
        }
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_highest_pack_wins_and_others_join() {
        let mut o = WorldgenOverrides::default();
        assert!(o.is_empty());
        o.set("noises", "minecraft:cave", "low".into());
        o.set("noises", "minecraft:cave", "high".into());
        o.set("noises", "pack:new", "new".into());
        let base = BTreeMap::from([
            ("minecraft:cave".to_owned(), "vanilla".to_owned()),
            ("minecraft:other".to_owned(), "kept".to_owned()),
        ]);
        let merged = o.apply("noises", base);
        assert_eq!(merged["minecraft:cave"], "high");
        assert_eq!(merged["minecraft:other"], "kept");
        assert_eq!(merged["pack:new"], "new");
        assert_eq!(o.apply("densities", BTreeMap::new()).len(), 0);
    }

    #[test]
    fn pack_files_go_to_their_bundles() {
        let mut o = WorldgenOverrides::default();
        let text = |t: &str| {
            let t = t.to_owned();
            move || Some(t)
        };
        let mut add = |file: &str, json: &str| o.add_file("file/p", file, text(json));
        assert_eq!(
            add("data/minecraft/worldgen/noise/cave.json", "1"),
            PackFile::Read
        );
        assert_eq!(
            add("data/p/worldgen/density_function/a/b.json", "2"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/noise_settings/overworld.json", "3"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/structure_set/mineshafts.json", "4"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/dimension/overworld.json", "5"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/tags/worldgen/biome/is_hill.json", "{}"),
            PackFile::Read
        );
        assert_eq!(
            add(
                "data/minecraft/dimension_type/overworld.json",
                r#"{"min_y":-64,"height":2096}"#
            ),
            PackFile::DimensionType {
                id: "minecraft:overworld".into(),
                min_y: -64,
                height: 2096
            }
        );
        assert_eq!(
            add("data/minecraft/worldgen/carver/cave.json", "6"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/feature/water_delta.json", "7"),
            PackFile::Read
        );
        assert_eq!(
            add("data/minecraft/worldgen/configured_carver/cave.json", "{}"),
            PackFile::NotRead("worldgen/configured_carver".into())
        );
        assert_eq!(
            add("data/minecraft/worldgen/world_preset/normal.json", "{}"),
            PackFile::NotRead("worldgen/world_preset".into())
        );
        assert_eq!(add("data/p/function/f.mcfunction", ""), PackFile::Other);
        assert_eq!(
            o.add_file("file/p", "data/p/worldgen/noise/x.json", || None),
            PackFile::Unreadable
        );
        let get = |bundle: &str| o.entries(bundle).collect::<Vec<_>>();
        assert_eq!(get("noises"), [("minecraft:cave", "1")]);
        assert_eq!(get("densities"), [("p:a/b", "2")]);
        assert_eq!(get("settings"), [("minecraft:overworld", "3")]);
        assert_eq!(get("structure_sets"), [("minecraft:mineshafts", "4")]);
        assert_eq!(get(DIMENSIONS), [("minecraft:overworld", "5")]);
        assert_eq!(get("carvers"), [("minecraft:cave", "6")]);
        assert_eq!(get("features"), [("minecraft:water_delta", "7")]);
        assert_eq!(get("structures"), []);
        assert_eq!(o.tags("biome_tags").count(), 1);
    }

    #[test]
    fn new_biomes_are_left_out_of_tags_and_climate_points() {
        let mut o = WorldgenOverrides::default();
        o.set("biomes", "minecraft:plains", "{}".into());
        o.set("biomes", "minecraft:molten_peaks", "{}".into());
        o.add_tag(
            "biome_tags",
            "minecraft:is_hill",
            "file/p",
            r#"{"values":["minecraft:plains","molten_peaks",{"id":"minecraft:molten_peaks","required":false}]}"#.into(),
        );
        o.set(
            DIMENSIONS,
            "minecraft:overworld",
            r#"{"generator":{"biome_source":{"biomes":[{"biome":"minecraft:plains"},{"biome":"minecraft:molten_peaks"}]}}}"#.into(),
        );
        let warnings = o.finish(|id| id == "minecraft:plains");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert_eq!(o.entries("biomes").count(), 1);
        let (_, _, tag) = o.tags("biome_tags").next().unwrap();
        assert_eq!(tag, r#"{"values":["minecraft:plains"]}"#);
        let (_, stem) = o.entries(DIMENSIONS).next().unwrap();
        assert!(!stem.contains("molten_peaks") && stem.contains("minecraft:plains"));
    }
}
