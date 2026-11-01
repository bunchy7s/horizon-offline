//! Registry id tables. Built-in registries (items, entity types, menus, sound
//! events ...) have fixed network ids, read from the data generator's
//! registries report. Data-driven registries (dimension types, biomes, chat
//! types, damage types ...) are numbered in the order the server sends them
//! during configuration.
use crate::{
    Error, Result,
    packets::configuration::{RegistryData, UpdateTags},
};
use horizon_world::nbt::{JavaString, Tag};
use std::{collections::HashMap, path::Path};

/// One registry: names by network id and the reverse lookup.
#[derive(Clone, Debug, Default)]
pub struct Ids {
    pub names: Vec<String>,
    pub ids: HashMap<String, i32>,
}
impl Ids {
    pub fn from_names(names: Vec<String>) -> Self {
        let ids = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as i32))
            .collect();
        Self { names, ids }
    }
    pub fn name(&self, id: i32) -> Option<&str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.names.get(i))
            .map(String::as_str)
    }
    pub fn id(&self, name: &str) -> Option<i32> {
        self.ids.get(name).copied()
    }
}

/// The built-in registries from `registries.json` (`--reports`).
#[derive(Clone, Debug, Default)]
pub struct Builtin {
    pub registries: HashMap<String, Ids>,
}
/// The names of both kinds of registry a connection knows: the built-in ones
/// first (as a hub's `SyncData::id` does), then those the server synced.
pub struct SyncedNames<'a> {
    pub builtin: &'a Builtin,
    pub dynamic: &'a Dynamic,
}
impl crate::item::Names for SyncedNames<'_> {
    fn name(&self, registry: &str, id: i32) -> Option<String> {
        self.builtin
            .get(registry)
            .and_then(|ids| ids.name(id))
            .or_else(|| self.dynamic.name(registry, id))
            .or_else(|| {
                (self.dynamic.custom_components && registry == "minecraft:data_component_type")
                    .then(|| crate::item::custom::name(id))
                    .flatten()
            })
            .map(str::to_owned)
    }
}

impl Builtin {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::new(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self> {
        let json: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::new(format!("registries report: {e}")))?;
        let mut registries = HashMap::new();
        for (name, registry) in json
            .as_object()
            .ok_or_else(|| Error::new("registries report is not an object"))?
        {
            let Some(entries) = registry["entries"].as_object() else {
                continue;
            };
            let mut names = vec![String::new(); entries.len()];
            for (entry, value) in entries {
                let id = value["protocol_id"]
                    .as_u64()
                    .ok_or_else(|| Error::new(format!("{entry}: no protocol_id")))?
                    as usize;
                if id >= names.len() {
                    names.resize(id + 1, String::new());
                }
                names[id] = entry.clone();
            }
            registries.insert(name.clone(), Ids::from_names(names));
        }
        Ok(Self { registries })
    }
    pub fn get(&self, registry: &str) -> Option<&Ids> {
        self.registries.get(registry)
    }
    /// Look for the report next to prepared resources, then in the data
    /// generator output of a development checkout.
    pub fn find(resources: &Path) -> Option<std::path::PathBuf> {
        let candidates = [
            resources.join("registries.json"),
            resources.join("../../datagen/out/reports/registries.json"),
            resources.join("../datagen/out/reports/registries.json"),
        ];
        candidates.into_iter().find(|p| p.is_file())
    }
}

/// Registries synchronized during configuration, in arrival order.
#[derive(Clone, Debug, Default)]
pub struct Dynamic {
    pub registries: HashMap<String, Vec<(String, Option<Tag>)>>,
    /// The `minecraft:dialog` tags the server sent: each tag's entries by id.
    pub dialog_tags: HashMap<String, Vec<i32>>,
    /// The peer confirmed the native component registry identity.
    pub custom_components: bool,
}
impl Dynamic {
    /// The dialog tags of a `ClientboundUpdateTagsPacket`.
    pub fn add_tags(&mut self, packet: &UpdateTags) {
        for r in packet
            .registries
            .iter()
            .filter(|r| r.registry == "minecraft:dialog")
        {
            self.dialog_tags = r
                .tags
                .iter()
                .map(|t| (t.name.clone(), t.ids.clone()))
                .collect();
        }
    }
    pub fn add(&mut self, packet: RegistryData) {
        let entries = packet.entries.into_iter().map(|e| (e.id, e.data)).collect();
        self.registries.insert(packet.registry, entries);
    }
    pub fn name(&self, registry: &str, id: i32) -> Option<&str> {
        let i = usize::try_from(id).ok()?;
        self.registries
            .get(registry)?
            .get(i)
            .map(|(n, _)| n.as_str())
    }
    /// The id of an entry by name, the reverse of `name`. A registry has one
    /// entry per id, so the first match is the only one.
    pub fn id(&self, registry: &str, name: &str) -> Option<i32> {
        let list = self.registries.get(registry)?;
        list.iter().position(|(n, _)| n == name).map(|i| i as i32)
    }
    pub fn data(&self, registry: &str, id: i32) -> Option<&Tag> {
        let i = usize::try_from(id).ok()?;
        self.registries.get(registry)?.get(i)?.1.as_ref()
    }
    pub fn len(&self, registry: &str) -> usize {
        self.registries.get(registry).map_or(0, Vec::len)
    }
    pub fn is_empty(&self) -> bool {
        self.registries.is_empty()
    }
    /// The dimension type's vertical range and sky light, from its
    /// synchronized data or the vanilla defaults for known names.
    pub fn dimension(&self, id: i32) -> DimensionType {
        let name = self
            .name("minecraft:dimension_type", id)
            .unwrap_or("minecraft:overworld");
        let mut d = DimensionType::vanilla(name);
        if let Some(Tag::Compound(map)) = self.data("minecraft:dimension_type", id) {
            let int = |k: &str| match map.get(&JavaString::from(k)) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            if let Some(v) = int("min_y") {
                d.min_y = v;
            }
            if let Some(v) = int("height") {
                d.height = v;
            }
            if let Some(v) = int("logical_height") {
                d.logical_height = v;
            }
            if let Some(Tag::Byte(v)) = map.get(&JavaString::from("has_skylight")) {
                d.has_skylight = *v != 0;
            }
            // DimensionType.defaultClock: the world clock this level's day
            // time follows (absent in the Nether).
            d.default_clock = match map.get(&JavaString::from("default_clock")) {
                Some(Tag::String(s)) => Some(s.to_string_lossy()),
                Some(Tag::Compound(c)) => match c.get(&JavaString::from("id")) {
                    Some(Tag::String(s)) => Some(s.to_string_lossy()),
                    _ => d.default_clock,
                },
                _ => d.default_clock,
            };
        }
        d
    }
}
/// The parts of a DimensionType the client needs for chunk layout.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionType {
    pub name: String,
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    /// `DimensionType.defaultClock`: the `minecraft:world_clock` this level
    /// samples for day time (`None` in the Nether).
    pub default_clock: Option<String>,
}
impl DimensionType {
    pub fn vanilla(name: &str) -> Self {
        let (min_y, height, logical_height, has_skylight, default_clock) = match name {
            "minecraft:the_nether" => (0, 256, 128, false, None),
            "minecraft:the_end" => (0, 256, 256, true, Some("minecraft:the_end")),
            _ => (-64, 384, 384, true, Some("minecraft:overworld")),
        };
        Self {
            name: name.to_owned(),
            min_y,
            height,
            logical_height,
            has_skylight,
            default_clock: default_clock.map(str::to_owned),
        }
    }
    pub fn sections(&self) -> usize {
        (self.height / 16) as usize
    }
}

/// Mth.ceillog2(n): bits for a global palette over `n` values.
pub fn ceil_log2(n: usize) -> u8 {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::configuration::RegistryEntry;

    #[test]
    fn parses_registry_reports() {
        let report = r#"{"minecraft:item":{"default":"minecraft:air","entries":{"minecraft:stone":{"protocol_id":1},"minecraft:air":{"protocol_id":0}},"protocol_id":5}}"#;
        let b = Builtin::parse(report).unwrap();
        let items = b.get("minecraft:item").unwrap();
        assert_eq!(items.name(1), Some("minecraft:stone"));
        assert_eq!(items.id("minecraft:air"), Some(0));
        assert_eq!(items.name(2), None);
    }

    #[test]
    fn dynamic_registries_follow_arrival_order() {
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![
                RegistryEntry {
                    id: "minecraft:overworld".into(),
                    data: None,
                },
                RegistryEntry {
                    id: "minecraft:the_nether".into(),
                    data: Some(Tag::compound([
                        ("min_y", Tag::Int(0)),
                        ("height", Tag::Int(256)),
                        ("has_skylight", Tag::Byte(0)),
                    ])),
                },
            ],
        });
        assert_eq!(
            d.dimension(0),
            DimensionType::vanilla("minecraft:overworld")
        );
        assert_eq!(d.dimension(1).sections(), 16);
        assert!(!d.dimension(1).has_skylight);
        assert_eq!(d.dimension(1).default_clock, None);
        assert_eq!(ceil_log2(35723), 16);
        assert_eq!(ceil_log2(65), 7);
        assert_eq!(ceil_log2(64), 6);
    }

    #[test]
    fn dimension_reads_default_clock() {
        let mut d = Dynamic::default();
        d.add(RegistryData {
            registry: "minecraft:dimension_type".into(),
            entries: vec![RegistryEntry {
                id: "pack:custom".into(),
                data: Some(Tag::compound([
                    ("min_y", Tag::Int(-64)),
                    ("height", Tag::Int(384)),
                    ("logical_height", Tag::Int(384)),
                    ("has_skylight", Tag::Byte(1)),
                    ("default_clock", Tag::String("minecraft:overworld".into())),
                ])),
            }],
        });
        let dim = d.dimension(0);
        assert_eq!(dim.name, "pack:custom");
        assert_eq!(dim.default_clock.as_deref(), Some("minecraft:overworld"));
    }
}
