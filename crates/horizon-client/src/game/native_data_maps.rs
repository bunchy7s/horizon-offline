//! Data maps for native server mods (docs/native-mods.md): values a data pack
//! attaches to the entries of a registry, under the types a mod registered
//! (`native_data_map_types`). Vanilla has no data maps; the contract is
//! NeoForge's `DataMapType`: files at
//! `data/<map namespace>/data_maps/<registry path>/<map path>.json` hold
//! `replace`, `values` (entry ids and `#tags` to values) and `remove`, and a
//! newer value overrides an older one for the same entry. The files are the
//! world's data packs' and what mods contribute through
//! `native_data_pack_files` (key `data_maps/<ns>:<registry path>/<map path>`),
//! merged each time the server's tables are built: a world's first open and
//! every `/reload`.
use super::Game;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A data map type a mod registers (`RegisterDataMapTypesEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeDataMapType {
    /// `mod_id:name`: not in the `minecraft` namespace. Its files are
    /// `data/<mod_id>/data_maps/<registry path>/<name>.json`.
    pub id: String,
    /// The registry it is for: `minecraft:item` or `minecraft:block`.
    pub registry: String,
}

/// The registries data maps can be attached to so far, with their folder
/// (the registry's path: `minecraft` registries have no namespace folder).
const REGISTRIES: [(&str, &str); 2] = [("minecraft:item", "item"), ("minecraft:block", "block")];

/// The merged values of every registered type: `(registry, type id)` to the
/// value of each entry that has one.
#[derive(Default)]
pub(crate) struct DataMaps {
    maps: HashMap<(String, String), BTreeMap<String, Value>>,
}

/// `minecraft:` for an id written without a namespace.
fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// A file's value for an entry: the data itself, or the wrapper
/// `{"value": ..., "replace": false}` (`DataMapEntry`'s codec). `replace` only
/// matters to mergers, which types don't have yet: a newer value overrides.
fn entry_value(raw: &Value) -> Option<Value> {
    if let Value::Object(map) = raw
        && let Some(value) = map.get("value")
        && map
            .iter()
            .all(|(k, v)| k == "value" || (k == "replace" && v.is_boolean()))
    {
        return Some(value.clone());
    }
    (!raw.is_null()).then(|| raw.clone())
}

/// One file over what earlier files made of the type: `replace` clears it,
/// `values` attach (a tag to every entry in it), then `remove` takes entries
/// away, as `DataMapFile` is applied. False when the file isn't a data map.
fn apply_file(
    map: &mut BTreeMap<String, Value>,
    file: &Value,
    tags: &HashMap<String, HashSet<String>>,
) -> bool {
    let (Some(root), Some(values)) = (file.as_object(), file.get("values")) else {
        return false;
    };
    let Some(values) = values.as_object() else {
        return false;
    };
    if root.get("replace").and_then(Value::as_bool) == Some(true) {
        map.clear();
    }
    // The members of a tag or the entry itself.
    let members = |key: &str| -> Vec<String> {
        match key.strip_prefix('#') {
            Some(tag) => tags
                .get(&full_id(tag))
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default(),
            None => vec![full_id(key)],
        }
    };
    for (key, raw) in values {
        let Some(value) = entry_value(raw) else {
            continue;
        };
        for entry in members(key) {
            map.insert(entry, value.clone());
        }
    }
    for removal in root
        .get("remove")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // An entry or `{"key": entry}`, as `DataMapEntry.Removal` reads it.
        let key = removal.as_str().or_else(|| removal["key"].as_str());
        for entry in key.map(members).unwrap_or_default() {
            map.remove(&entry);
        }
    }
    true
}

impl DataMaps {
    /// The types' values from `files` (the server's tables' pack files, in
    /// pack order, mods' below the packs'), tags resolved against the
    /// loaded `item_tags` and `block_tags`. A type with an unsupported
    /// registry or a bad id is left out with a warning; a file that isn't a
    /// data map is skipped alone.
    pub(crate) fn load(
        types: &[NativeDataMapType],
        files: &[(String, Vec<Vec<u8>>)],
        item_tags: &HashMap<String, HashSet<String>>,
        block_tags: &HashMap<String, HashSet<String>>,
    ) -> Self {
        let mut maps: HashMap<(String, String), BTreeMap<String, Value>> = HashMap::new();
        for ty in types {
            let Some((_, dir)) = REGISTRIES.iter().find(|(r, _)| *r == ty.registry) else {
                log::warn!(
                    "Horizon: data map {} is for {}, which has no data maps yet: left out",
                    ty.id,
                    ty.registry
                );
                continue;
            };
            let valid = |s: &str| {
                !s.is_empty()
                    && s.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
            };
            let ok = ty
                .id
                .split_once(':')
                .is_some_and(|(ns, path)| ns != "minecraft" && valid(ns) && valid(path));
            if !ok {
                log::warn!(
                    "Horizon: data map type {} is not a mod's id: left out",
                    ty.id
                );
                continue;
            }
            let key = (ty.registry.clone(), ty.id.clone());
            if maps.contains_key(&key) {
                log::warn!(
                    "Horizon: data map {} registered twice: the first is kept",
                    ty.id
                );
                continue;
            }
            let tags = if *dir == "item" {
                item_tags
            } else {
                block_tags
            };
            let (ns, path) = ty.id.split_once(':').unwrap_or_default();
            let wanted = format!("data_maps/{ns}:{dir}/{path}");
            let mut map = BTreeMap::new();
            if let Some((_, versions)) = files.iter().find(|(k, _)| *k == wanted) {
                for bytes in versions {
                    let applied = serde_json::from_slice::<Value>(bytes)
                        .ok()
                        .is_some_and(|file| apply_file(&mut map, &file, tags));
                    if !applied {
                        log::warn!(
                            "Horizon: a data map file of {} is not a data map, skipped",
                            ty.id
                        );
                    }
                }
            }
            maps.insert(key, map);
        }
        Self { maps }
    }
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The merged data map of `type_id` for `registry`, none unless this is a
    /// server whose tables were built from packs or mods' files.
    fn native_data_map_of(
        &self,
        registry: &str,
        type_id: &str,
    ) -> Option<&BTreeMap<String, Value>> {
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return None;
        }
        let key = (registry.to_owned(), type_id.to_owned());
        self.world_tables.as_ref()?.data_maps.maps.get(&key)
    }
    /// `Holder.getData(type)`: the value the data pack and mod files gave
    /// `entry` (`minecraft:coal`) in data map `type_id` for `registry`
    /// (`minecraft:item`, `minecraft:block`), as the JSON the file held. None
    /// when the entry has none, the type isn't registered for that registry
    /// (`native_data_map_types`), or this isn't a server whose tables hold
    /// files.
    pub(in crate::game) fn native_data_map(
        &self,
        registry: &str,
        type_id: &str,
        entry: &str,
    ) -> Option<&Value> {
        self.native_data_map_of(registry, type_id)?
            .get(&full_id(entry))
    }
    /// Every entry that has a value in data map `type_id` for `registry`, in
    /// id order. Empty where `native_data_map` is none for all.
    pub(in crate::game) fn native_data_map_entries(
        &self,
        registry: &str,
        type_id: &str,
    ) -> Vec<(&str, &Value)> {
        self.native_data_map_of(registry, type_id)
            .map(|map| map.iter().map(|(k, v)| (k.as_str(), v)).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tags(list: &[(&str, &[&str])]) -> HashMap<String, HashSet<String>> {
        list.iter()
            .map(|(tag, members)| {
                (
                    (*tag).to_owned(),
                    members.iter().map(|m| (*m).to_owned()).collect(),
                )
            })
            .collect()
    }

    fn ty(id: &str, registry: &str) -> NativeDataMapType {
        NativeDataMapType {
            id: id.to_owned(),
            registry: registry.to_owned(),
        }
    }

    fn files(list: &[(&str, &[Value])]) -> Vec<(String, Vec<Vec<u8>>)> {
        list.iter()
            .map(|(key, versions)| {
                (
                    (*key).to_owned(),
                    versions
                        .iter()
                        .map(|v| v.to_string().into_bytes())
                        .collect(),
                )
            })
            .collect()
    }

    fn load(files: &[(String, Vec<Vec<u8>>)]) -> DataMaps {
        let items = tags(&[(
            "minecraft:logs",
            &["minecraft:oak_log", "minecraft:birch_log"],
        )]);
        DataMaps::load(
            &[ty("mymod:fuel", "minecraft:item")],
            files,
            &items,
            &HashMap::new(),
        )
    }

    fn get<'a>(maps: &'a DataMaps, entry: &str) -> Option<&'a Value> {
        maps.maps[&("minecraft:item".to_owned(), "mymod:fuel".to_owned())].get(entry)
    }

    #[test]
    fn values_attach_to_entries_and_to_every_member_of_a_tag() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"burn": 1600},
                "stick": 100,
                "#minecraft:logs": {"burn": 300}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 1600})));
        // An id without a namespace is a `minecraft` one.
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(100)));
        assert_eq!(get(&maps, "minecraft:oak_log"), Some(&json!({"burn": 300})));
        assert_eq!(
            get(&maps, "minecraft:birch_log"),
            Some(&json!({"burn": 300}))
        );
        assert_eq!(get(&maps, "minecraft:stone"), None);
    }

    #[test]
    fn a_newer_file_overrides_and_replace_clears_what_came_before() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), Some(&json!(2)));
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"minecraft:coal": 1, "minecraft:stick": 2}}),
                json!({"replace": true, "values": {"minecraft:coal": 10}}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(10)));
        assert_eq!(get(&maps, "minecraft:stick"), None);
    }

    #[test]
    fn remove_takes_entries_and_tags_away_after_the_files_own_values() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[
                json!({"values": {"#minecraft:logs": 5, "minecraft:coal": 1}}),
                // The file's own value for coal is attached, then removed.
                json!({"values": {"minecraft:coal": 9}, "remove": ["minecraft:coal", "#minecraft:logs"]}),
            ],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), None);
        assert_eq!(get(&maps, "minecraft:oak_log"), None);
    }

    #[test]
    fn the_wrapper_form_gives_its_value_and_a_plain_object_is_a_value() {
        let f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {
                "minecraft:coal": {"value": {"burn": 3}, "replace": true},
                "minecraft:stick": {"value": 1, "other": 2}
            }})],
        )]);
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!({"burn": 3})));
        // More than `value` and `replace`: the object is the data.
        assert_eq!(
            get(&maps, "minecraft:stick"),
            Some(&json!({"value": 1, "other": 2}))
        );
    }

    #[test]
    fn a_bad_file_is_skipped_alone_and_bad_types_are_left_out() {
        let mut f = files(&[(
            "data_maps/mymod:item/fuel",
            &[json!({"values": {"minecraft:coal": 1}})],
        )]);
        f[0].1.insert(0, b"{nope".to_vec());
        f[0].1
            .push(json!({"nothing": true}).to_string().into_bytes());
        let maps = load(&f);
        assert_eq!(get(&maps, "minecraft:coal"), Some(&json!(1)));
        let maps = DataMaps::load(
            &[
                ty("minecraft:fuel", "minecraft:item"),
                ty("mymod:x", "minecraft:biome"),
                ty("nonamespace", "minecraft:item"),
                ty("mymod:a", "minecraft:block"),
                ty("mymod:a", "minecraft:block"),
            ],
            &[],
            &HashMap::new(),
            &HashMap::new(),
        );
        // Only the one good type, registered once, empty.
        assert_eq!(maps.maps.len(), 1);
        assert!(
            maps.maps
                .contains_key(&("minecraft:block".to_owned(), "mymod:a".to_owned()))
        );
    }

    #[test]
    fn block_maps_read_the_block_tags() {
        let blocks = tags(&[("minecraft:logs", &["minecraft:oak_log"])]);
        let f = files(&[(
            "data_maps/mymod:block/hardness",
            &[json!({"values": {"#minecraft:logs": 2}})],
        )]);
        let maps = DataMaps::load(
            &[ty("mymod:hardness", "minecraft:block")],
            &f,
            &HashMap::new(),
            &blocks,
        );
        let map = &maps.maps[&("minecraft:block".to_owned(), "mymod:hardness".to_owned())];
        assert_eq!(map.get("minecraft:oak_log"), Some(&json!(2)));
    }
}
