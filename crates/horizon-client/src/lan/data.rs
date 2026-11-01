//! What the configuration phase sends: the synchronized registries (names in
//! network order, with their data for clients that do not know the vanilla
//! pack) and the tags of every registry, loaded from the prepared resources
//! (`data/minecraft/...`) and the registries report.
use crate::chunk::LevelShapes;
use horizon_protocol::{
    packets::configuration::{RegistryData, RegistryEntry, RegistryTags, TagEntry, UpdateTags},
    registry::{Builtin, Dynamic},
};
use horizon_world::nbt::{JavaString, Tag};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

/// RegistryDataLoader.SYNCHRONIZED_REGISTRIES, in vanilla's order.
pub const SYNCHRONIZED: &[&str] = &[
    "minecraft:worldgen/biome",
    "minecraft:chat_type",
    "minecraft:trim_pattern",
    "minecraft:trim_material",
    "minecraft:wolf_variant",
    "minecraft:wolf_sound_variant",
    "minecraft:pig_variant",
    "minecraft:pig_sound_variant",
    "minecraft:frog_variant",
    "minecraft:cat_variant",
    "minecraft:cat_sound_variant",
    "minecraft:cow_sound_variant",
    "minecraft:cow_variant",
    "minecraft:chicken_sound_variant",
    "minecraft:chicken_variant",
    "minecraft:zombie_nautilus_variant",
    "minecraft:painting_variant",
    "minecraft:sulfur_cube_archetype",
    "minecraft:dimension_type",
    "minecraft:damage_type",
    "minecraft:banner_pattern",
    "minecraft:enchantment",
    "minecraft:jukebox_song",
    "minecraft:instrument",
    "minecraft:test_environment",
    "minecraft:test_instance",
    "minecraft:dialog",
    "minecraft:world_clock",
    "minecraft:timeline",
    "minecraft:decorated_pot_pattern",
    "minecraft:block_transformer",
    "minecraft:worldgen/block_state_provider",
];
/// KnownPack.vanilla("core") for this version.
pub const CORE_PACK: (&str, &str, &str) = ("minecraft", "core", horizon_protocol::GAME_VERSION);

/// Synchronized registries a world's data packs add entries to (or replace
/// them in). The others stay the game's own: Horizon's gameplay reads them
/// from the prepared resources only.
const PACK_REGISTRIES: &[&str] = &["minecraft:dialog"];

/// A data-driven registry: entry names in network order and their JSON.
#[derive(Clone)]
struct Entries {
    names: Vec<String>,
    json: Vec<Value>,
    /// The entry comes from a world's data pack, which no client knows, so
    /// its data is always sent.
    from_pack: Vec<bool>,
}

pub struct SyncData {
    pub content_identity: Option<[u8; 32]>,
    pub builtin: Builtin,
    /// Entry names per synchronized registry (for id lookups).
    pub dynamic: Dynamic,
    registries: Vec<(String, Entries)>,
    pub tags: UpdateTags,
    pub features: Vec<String>,
    /// Vanilla's command tree (the commands report), when found.
    pub commands: Option<super::commands::CommandTree>,
    /// ClientboundUpdateRecipesPacket's payload.
    pub recipes: Vec<u8>,
    pub advancements: super::advancements::ServerAdvancements,
}

/// A resource id with its default namespace filled in.
fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// Every `*.json` under `dir`, as (path without extension relative to dir, value), sorted.
fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Value>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            walk(&path, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(".json")
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(value) = serde_json::from_str::<Value>(&text)
        {
            out.insert(format!("{prefix}{stem}"), value);
        }
    }
}

/// A synchronized registry's entries from the world's data packs (`<dir>/`
/// files, the top pack's for each id) over the game's own. Each entry is
/// ((namespace, path), JSON, from a pack); `file_order` sorts them.
fn pack_entries(
    dir: &str,
    overlay: &horizon_world::loot::PackOverlay,
    entries: &mut Vec<((String, String), Value, bool)>,
) {
    for (id, files) in overlay.kind(dir) {
        let Some((ns, path)) = id.split_once(':') else {
            continue;
        };
        let top = horizon_world::loot::PackOverlay::top(files);
        let value = match serde_json::from_slice::<Value>(top) {
            Ok(value) => value,
            Err(e) => {
                log::warn!("Horizon: data pack {dir} {id} is not read: {e}");
                continue;
            }
        };
        let key = (ns.to_owned(), path.to_owned());
        match entries.iter_mut().find(|(k, ..)| *k == key) {
            Some(entry) => {
                entry.1 = value;
                entry.2 = true;
            }
            None => entries.push((key, value, true)),
        }
    }
}

/// ResourceManagerRegistryLoadTask.load: the entries register sorted by the
/// file `Identifier` FileToIdConverter.listMatchingResources keys them by
/// (`<dir>/<path>.json`), path first, then namespace (Identifier.compareTo).
/// The `.json` counts: `x-y` comes before `x`, as `-` sorts before `.`.
fn file_order<T>(entries: &mut [((String, String), T, bool)]) {
    entries.sort_by_cached_key(|((ns, path), ..)| (format!("{path}.json"), ns.clone()));
}

/// A JSON value as the NBT that vanilla's NbtOps would produce from the codec
/// (integers are ints, other numbers doubles, booleans bytes).
pub fn json_to_nbt(v: &Value) -> Tag {
    match v {
        Value::Null => Tag::compound(Vec::<(&str, Tag)>::new()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => match i32::try_from(i) {
                Ok(i) => Tag::Int(i),
                Err(_) => Tag::Long(i),
            },
            None => Tag::Double(n.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(s) => Tag::String(JavaString::from(s.as_str())),
        Value::Array(a) => Tag::List(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), json_to_nbt(v)))),
    }
}

/// A component's JSON as NbtOps writes it. A list whose elements differ in type
/// (translation arguments mixing numbers and components) stays one list: the NBT
/// writer wraps each element that isn't a compound under the empty key, as
/// `ListTag.wrapIfNeeded` does, so wrapping it here too would leave the client a
/// `{"": 1}` argument it cannot read.
pub fn component_nbt(v: &Value) -> Tag {
    match v {
        Value::Array(a) => Tag::List(a.iter().map(component_nbt).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), component_nbt(v)))),
        other => json_to_nbt(other),
    }
}

impl SyncData {
    /// No registries, tags or recipes: enough for tests of the login phase.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            builtin: Builtin::default(),
            dynamic: Dynamic::default(),
            registries: Vec::new(),
            tags: UpdateTags {
                registries: Vec::new(),
            },
            features: Vec::new(),
            commands: None,
            recipes: Vec::new(),
            advancements: super::advancements::ServerAdvancements::load(
                Path::new(""),
                &horizon_world::loot::PackOverlay::NONE,
            ),
            content_identity: None,
        }
    }
    pub fn load(
        resources: &Path,
        overlay: &horizon_world::loot::PackOverlay,
    ) -> Result<Self, String> {
        let builtin = Builtin::find(resources)
            .and_then(|p| Builtin::load(&p).ok())
            .ok_or("registries.json not found next to the resources")?;
        let root: PathBuf = resources.join("data/minecraft");
        let mut registries = Vec::new();
        let mut dynamic = Dynamic {
            custom_components: horizon_protocol::item::custom::entries().next().is_some(),
            ..Default::default()
        };
        for key in SYNCHRONIZED {
            let path = key.trim_start_matches("minecraft:");
            let mut found = BTreeMap::new();
            walk(&root.join(path), "", &mut found);
            let mut entries: Vec<_> = found
                .into_iter()
                .map(|(p, v)| (("minecraft".to_owned(), p), v, false))
                .collect();
            if PACK_REGISTRIES.contains(key) {
                pack_entries(path, overlay, &mut entries);
            }
            file_order(&mut entries);
            let mut e = Entries {
                names: Vec::new(),
                json: Vec::new(),
                from_pack: Vec::new(),
            };
            for ((ns, p), v, from_pack) in entries {
                e.names.push(format!("{ns}:{p}"));
                e.json.push(v);
                e.from_pack.push(from_pack);
            }
            dynamic.registries.insert(
                (*key).to_owned(),
                e.names.iter().map(|n| (n.clone(), None)).collect(),
            );
            registries.push(((*key).to_owned(), e));
        }
        // The world's packs' dialogs joined the registry above
        // (`pack_entries`); their `minecraft:dialog` tags join the root's here,
        // for the Quick Actions key and `show_dialog`.
        let mut tags = Self::load_tags(&root, &builtin, &dynamic);
        Self::merge_dialog_tags(&mut tags, &root, overlay, &builtin, &dynamic);
        let content_identity = if resources.join(crate::content_identity::MARKER).exists()
            || horizon_protocol::item::custom::entries().next().is_some()
        {
            Some(crate::content_identity::identity(resources).map_err(|error| error.to_string())?)
        } else {
            None
        };
        let mut sync = Self {
            content_identity,
            builtin,
            dynamic,
            registries,
            tags,
            features: vec!["minecraft:vanilla".to_owned()],
            commands: super::commands::CommandTree::load(resources),
            recipes: Vec::new(),
            advancements: super::advancements::ServerAdvancements::load(resources, overlay),
        };
        sync.recipes = super::recipes::update_recipes(resources, &sync, overlay);
        Ok(sync)
    }

    /// MinecraftServer.reloadResources: `/reload` keeps the synchronized
    /// registries the server started with (`kept`'s, its
    /// LayeredRegistryAccess) and reads the tags against them
    /// (TagLoader.loadTagsForExistingRegistries); the rest stays reloaded.
    pub fn keep_registries(&mut self, resources: &Path, kept: &SyncData) {
        self.dynamic = kept.dynamic.clone();
        self.registries = kept.registries.clone();
        self.tags = Self::load_tags(
            &resources.join("data/minecraft"),
            &self.builtin,
            &self.dynamic,
        );
    }

    /// Whether a synchronized registry holds an entry from a world's pack.
    pub fn has_pack_entries(&self) -> bool {
        self.registries
            .iter()
            .any(|(_, e)| e.from_pack.contains(&true))
    }

    /// The id of a name in a built-in or synchronized registry.
    pub fn id(&self, registry: &str, name: &str) -> Option<i32> {
        if registry == "minecraft:data_component_type"
            && self.dynamic.custom_components
            && let Some(id) = horizon_protocol::item::custom::id(name)
        {
            return Some(id);
        }
        if let Some(ids) = self.builtin.get(registry) {
            return ids.id(name);
        }
        self.dynamic
            .registries
            .get(registry)?
            .iter()
            .position(|(n, _)| n == name)
            .map(|i| i as i32)
    }

    /// ClientboundRegistryDataPacket per registry. With `known_pack` the
    /// client rebuilds the entries from its own data, so none is sent, the
    /// dialogs among them (`RegistrySynchronization.packRegistry`): only an
    /// entry a world's pack added or replaced (`from_pack`) carries its data.
    /// The registries for a client, with the data of a dimension type the
    /// world's data packs changed (`levels`) even when it knows the vanilla
    /// pack.
    pub fn registry_packets(&self, known_pack: bool, levels: &LevelShapes) -> Vec<RegistryData> {
        self.registries
            .iter()
            .map(|(key, e)| RegistryData {
                registry: key.clone(),
                entries: e
                    .names
                    .iter()
                    .zip(&e.json)
                    .zip(&e.from_pack)
                    .map(|((name, json), &from_pack)| {
                        let changed = (key == "minecraft:dimension_type")
                            .then(|| changed_dimension_type(name, json, levels))
                            .flatten();
                        RegistryEntry {
                            id: name.clone(),
                            data: match changed {
                                Some(json) => Some(json_to_nbt(&json)),
                                None if known_pack && !from_pack => None,
                                None => Some(json_to_nbt(json)),
                            },
                        }
                    })
                    .collect(),
            })
            .collect()
    }
    /// The `minecraft:dialog` tags of the pack stack: the resource root's
    /// files, then each world pack's in order (`TagLoader`: a file adds its
    /// values, or replaces the earlier ones with `"replace": true`), nested
    /// tags expanded to the registry's ids.
    fn merge_dialog_tags(
        tags: &mut UpdateTags,
        root: &Path,
        overlay: &horizon_world::loot::PackOverlay,
        builtin: &Builtin,
        dynamic: &Dynamic,
    ) {
        let Some(names) = Self::registry_names(builtin, dynamic, "minecraft:dialog") else {
            return;
        };
        let mut files: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut own = BTreeMap::new();
        walk(&root.join("tags/dialog"), "", &mut own);
        for (name, value) in own {
            Self::add_tag_values(files.entry(qualified(&name)).or_default(), &value);
        }
        for (id, packs) in overlay.tag_kind("dialog") {
            for bytes in packs {
                if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
                    Self::add_tag_values(files.entry(id.clone()).or_default(), &value);
                }
            }
        }
        let mut entries = Vec::new();
        for name in files.keys() {
            let mut ids = Vec::new();
            let mut seen = HashSet::new();
            Self::expand_dialog_tag(&files, name, &names, &mut ids, &mut seen);
            entries.push(TagEntry {
                name: name.clone(),
                ids,
            });
        }
        tags.registries.retain(|r| r.registry != "minecraft:dialog");
        if !entries.is_empty() {
            tags.registries.push(RegistryTags {
                registry: "minecraft:dialog".into(),
                tags: entries,
            });
        }
    }

    /// A tag file's values, after `"replace": true` empties the earlier ones.
    fn add_tag_values(entries: &mut Vec<Value>, json: &Value) {
        if json["replace"].as_bool() == Some(true) {
            entries.clear();
        }
        if let Some(values) = json["values"].as_array() {
            entries.extend(values.iter().cloned());
        }
    }

    /// A dialog tag's entries as registry ids, nested tags expanded once.
    fn expand_dialog_tag(
        files: &BTreeMap<String, Vec<Value>>,
        tag: &str,
        names: &HashMap<&str, i32>,
        out: &mut Vec<i32>,
        seen: &mut HashSet<String>,
    ) {
        if !seen.insert(tag.to_owned()) {
            return;
        }
        let Some(values) = files.get(tag) else {
            return;
        };
        for value in values {
            let id = value
                .as_str()
                .or_else(|| value["id"].as_str())
                .unwrap_or_default();
            if let Some(reference) = id.strip_prefix('#') {
                Self::expand_dialog_tag(files, &qualified(reference), names, out, seen);
            } else if let Some(&n) = names.get(qualified(id).as_str())
                && !out.contains(&n)
            {
                out.push(n);
            }
        }
    }

    fn registry_names<'a>(
        builtin: &'a Builtin,
        dynamic: &'a Dynamic,
        key: &str,
    ) -> Option<HashMap<&'a str, i32>> {
        if let Some(ids) = builtin.get(key) {
            return Some(
                ids.names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.as_str(), i as i32))
                    .collect(),
            );
        }
        dynamic.registries.get(key).map(|v| {
            v.iter()
                .enumerate()
                .map(|(i, (n, _))| (n.as_str(), i as i32))
                .collect()
        })
    }

    /// TagNetworkSerialization.serializeTagsToNetwork from the tag files.
    fn load_tags(root: &Path, builtin: &Builtin, dynamic: &Dynamic) -> UpdateTags {
        let tag_root = root.join("tags");
        let mut registries = Vec::new();
        let mut keys: Vec<String> = builtin
            .registries
            .keys()
            .chain(dynamic.registries.keys())
            .cloned()
            .collect();
        keys.sort();
        for key in keys {
            let dir = tag_root.join(key.trim_start_matches("minecraft:"));
            if !dir.is_dir() {
                continue;
            }
            let mut files = BTreeMap::new();
            walk(&dir, "", &mut files);
            if files.is_empty() {
                continue;
            }
            let Some(names) = Self::registry_names(builtin, dynamic, &key) else {
                continue;
            };
            let mut tags = Vec::new();
            for name in files.keys() {
                let mut ids = Vec::new();
                let mut seen = HashSet::new();
                Self::expand(&files, name, &names, &mut ids, &mut seen);
                tags.push(TagEntry {
                    name: format!("minecraft:{name}"),
                    ids,
                });
            }
            registries.push(RegistryTags {
                registry: key,
                tags,
            });
        }
        UpdateTags { registries }
    }

    fn expand(
        files: &BTreeMap<String, Value>,
        tag: &str,
        names: &HashMap<&str, i32>,
        out: &mut Vec<i32>,
        seen: &mut HashSet<String>,
    ) {
        if !seen.insert(tag.to_owned()) {
            return;
        }
        let Some(values) = files.get(tag).and_then(|v| v["values"].as_array()) else {
            return;
        };
        for value in values {
            let id = value
                .as_str()
                .or_else(|| value["id"].as_str())
                .unwrap_or_default();
            if let Some(reference) = id.strip_prefix('#') {
                let reference = reference.trim_start_matches("minecraft:");
                Self::expand(files, reference, names, out, seen);
            } else if let Some(&n) = names.get(id)
                && !out.contains(&n)
            {
                out.push(n);
            }
        }
    }
}

/// The vanilla dimension type `name` (its JSON `json`) with the shape the
/// world gives the level of that name, if its data packs changed it.
fn changed_dimension_type(name: &str, json: &Value, levels: &LevelShapes) -> Option<Value> {
    if !matches!(
        name,
        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
    ) {
        return None;
    }
    let shape = levels.get(name);
    if shape == LevelShapes::VANILLA.get(name) {
        return None;
    }
    let mut json = json.clone();
    json["min_y"] = shape.min_y.into();
    json["height"] = shape.height.into();
    json["logical_height"] = shape.logical_height.into();
    json["has_skylight"] = shape.sky_light.into();
    Some(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_packs_supply_dialog_entries_and_tags() {
        use horizon_world::loot::PackOverlay;
        let vanilla = |p: &str| {
            (
                ("minecraft".to_owned(), p.to_owned()),
                serde_json::json!({ "type": "minecraft:dialog_list" }),
                false,
            )
        };
        let mut found = vec![vanilla("quick_actions"), vanilla("server_links")];
        let files = vec![
            (
                "dialog/test:menu".to_owned(),
                vec![br#"{"type":"minecraft:multi_action","title":"Menu"}"#.to_vec()],
            ),
            (
                "dialog/minecraft:quick_actions".to_owned(),
                vec![br#"{"type":"minecraft:dialog_list","title":"Quick"}"#.to_vec()],
            ),
            (
                "tags/minecraft:dialog/quick_actions".to_owned(),
                vec![br##"{"values":["test:menu","#minecraft:more"]}"##.to_vec()],
            ),
            (
                "tags/minecraft:dialog/more".to_owned(),
                vec![br#"{"values":["minecraft:server_links"]}"#.to_vec()],
            ),
        ];
        let overlay = PackOverlay { files: &files };
        pack_entries("dialog", &overlay, &mut found);
        file_order(&mut found);
        let names: Vec<String> = found
            .iter()
            .map(|((ns, p), _, _)| format!("{ns}:{p}"))
            .collect();
        // Identifier order: by path, then namespace.
        assert_eq!(
            names,
            [
                "test:menu",
                "minecraft:quick_actions",
                "minecraft:server_links"
            ]
        );
        assert_eq!(found[1].1["title"], "Quick", "a pack overrides an entry");
        let dynamic = Dynamic {
            registries: HashMap::from([(
                "minecraft:dialog".to_owned(),
                names.iter().map(|n| (n.clone(), None)).collect(),
            )]),
            ..Default::default()
        };
        let mut tags = UpdateTags {
            registries: Vec::new(),
        };
        SyncData::merge_dialog_tags(
            &mut tags,
            Path::new("a-resource-root-without-tags"),
            &overlay,
            &Builtin::default(),
            &dynamic,
        );
        let dialog = tags
            .registries
            .iter()
            .find(|r| r.registry == "minecraft:dialog")
            .expect("the dialog tags are sent");
        let quick = dialog
            .tags
            .iter()
            .find(|t| t.name == "minecraft:quick_actions")
            .unwrap();
        assert_eq!(
            quick.ids,
            [0, 2],
            "the pack's dialog, then the nested tag's"
        );
    }

    /// RegistrySynchronization.packRegistry: a client that knows the core
    /// pack gets the vanilla dialogs without data; a pack's dialog has it.
    #[test]
    fn known_pack_dialogs_are_sent_without_data() {
        let mut sync = SyncData::empty();
        sync.registries.push((
            "minecraft:dialog".to_owned(),
            Entries {
                names: vec!["minecraft:quick_actions".into(), "test:menu".into()],
                json: vec![serde_json::json!({ "type": "minecraft:dialog_list" }); 2],
                from_pack: vec![false, true],
            },
        ));
        let data = |known_pack| -> Vec<bool> {
            sync.registry_packets(known_pack, &LevelShapes::VANILLA)[0]
                .entries
                .iter()
                .map(|e| e.data.is_some())
                .collect()
        };
        assert_eq!(data(true), [false, true]);
        assert_eq!(data(false), [true, true]);
    }

    #[test]
    fn json_becomes_nbt() {
        let v: Value = serde_json::from_str(r#"{"a":1,"b":[true,2.5],"c":"x"}"#).unwrap();
        let t = json_to_nbt(&v);
        assert!(matches!(t.get("a"), Some(Tag::Int(1))));
        assert!(matches!(t.get("c"), Some(Tag::String(_))));
        assert!(matches!(t.get("b"), Some(Tag::List(l)) if l.len() == 2));
    }

    #[test]
    fn a_changed_dimension_type_is_sent_with_its_shape() {
        let vanilla: Value =
            serde_json::from_str(r#"{"min_y":-64,"height":384,"logical_height":384,"has_skylight":true,"ultrawarm":false}"#)
                .unwrap();
        let mut levels = LevelShapes::VANILLA;
        assert_eq!(
            changed_dimension_type("minecraft:overworld", &vanilla, &levels),
            None
        );
        levels.overworld = crate::chunk::LevelShape::new(-64, 2096, 2096, true).unwrap();
        let sent = changed_dimension_type("minecraft:overworld", &vanilla, &levels).unwrap();
        assert_eq!(
            (sent["min_y"].as_i64(), sent["height"].as_i64()),
            (Some(-64), Some(2096))
        );
        assert_eq!(sent["ultrawarm"], false, "the rest of the type is kept");
        assert_eq!(
            changed_dimension_type("minecraft:the_nether", &vanilla, &levels),
            None
        );
        assert_eq!(
            changed_dimension_type("minecraft:overworld_caves", &vanilla, &levels),
            None
        );
    }
}

#[cfg(test)]
mod component_tests {
    use super::*;
    use horizon_world::nbt::{Limits, read_network, write_network};

    /// `/give`'s reply: a count, then components. On the wire each element is wrapped
    /// once, and read back (ListTag.addAndUnwrap) it is the number again.
    #[test]
    fn mixed_translation_arguments_are_wrapped_once() {
        let json = serde_json::json!({
            "translate": "commands.give.success.single",
            "with": [1, {"text": "Raw Iron"}, {"text": "Tester"}],
        });
        let tag = component_nbt(&json);
        let Some(Tag::List(args)) = tag.get("with") else {
            panic!("no arguments: {tag:?}");
        };
        assert_eq!(args[0], Tag::Int(1));
        // Read back as ListTag.addAndUnwrap does: a wrapper wrapped twice stays a compound.
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(read_network(&bytes, Limits::default()).unwrap().0, tag);
    }
}
