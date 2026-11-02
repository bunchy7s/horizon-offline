//! A content mod's `assets/` and `data/` (API 0.2, docs/modpacks.md): the
//! files it adds to the game's resource tree, and the blocks, items, mob
//! effects, entity types and menu types it declares in
//! `data/<id>/horizon/{block,item,mob_effect,entity_type,menu}/`.
//!
//! Content is data, not code: nothing here runs in the sandbox. What a mod
//! may add is fenced instead. Its own namespaces (its id and the ones its
//! manifest lists), plus additions to vanilla tags and to the convention
//! tags (`c`, which NeoForge and Fabric mods share). Paths are plain, files
//! and totals are capped, vanilla ids cannot be redefined, and no two mods
//! share a namespace ([`clash`]).
use crate::manifest::{Capability, MANIFEST, Manifest};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};

/// Content files per mod, bytes per file and bytes per mod.
pub const MAX_FILES: usize = 65_536;
pub const MAX_FILE: u64 = 16 << 20;
pub const MAX_TOTAL: u64 = 512 << 20;

/// Checks a content path: `assets/` or `data/`, then `/`-separated parts of
/// `[a-z0-9_.-]`, none of them `.` or `..` (vanilla's resource-location
/// characters, so the path means the same on every OS).
pub fn check_path(name: &str) -> Result<(), String> {
    let bad = || format!("{name:?} is not a plain assets/ or data/ path");
    let mut parts = name.split('/');
    if !matches!(parts.next(), Some("assets" | "data")) {
        return Err(bad());
    }
    let mut count = 0;
    for part in parts {
        count += 1;
        let chars_ok = part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
        if part.is_empty() || part == "." || part == ".." || !chars_ok {
            return Err(bad());
        }
    }
    // assets/<namespace>/<file> at least.
    if count < 2 {
        return Err(bad());
    }
    Ok(())
}

/// A vanilla identifier, `minecraft:path`.
/// `Identifier.isValidNamespace` and `isValidPath` on an id with both parts
/// given: `[a-z0-9_.-]`, and `/` in the path too.
fn namespaced_id(id: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || b == b'/')
    })
}

fn vanilla_id(id: &str) -> bool {
    id.strip_prefix("minecraft:").is_some_and(|path| {
        !path.is_empty()
            && path
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_./-".contains(&b))
    })
}

/// What a declared block changes from its template; unset keeps the
/// template's value.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockProperties {
    /// Destroy time in seconds of hand mining (-1: unbreakable).
    pub hardness: Option<f32>,
    pub explosion_resistance: Option<f32>,
    /// Light emitted, 0 to 15.
    pub light: Option<u8>,
    pub requires_tool: Option<bool>,
    pub friction: Option<f32>,
    /// A `MapColor` id, 0 to 61.
    pub map_color: Option<u8>,
    /// A vanilla block whose sounds this block uses.
    pub sounds: Option<String>,
    /// Which states tick randomly, whatever the template does (`isRandomlyTicking`).
    pub random_ticks: Option<RandomTicks>,
}

/// Which of a block's states get random ticks (`BlockBehaviour.isRandomlyTicking(state)`), so a
/// crop, sapling or soil on a template that doesn't tick still grows. The sections' random tick
/// counts follow it, and `native_random_tick` fires for the states it picks.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RandomTicks {
    /// Every state (`true`), or none (`false`, for a template that ticks).
    All(bool),
    /// The states whose own state property `property` is one of `values`.
    States {
        property: String,
        values: Vec<String>,
    },
}

impl RandomTicks {
    /// Whether a state ticks, `combo` being its values of the block's own state properties.
    pub fn applies(&self, combo: &[(String, String)]) -> bool {
        match self {
            RandomTicks::All(all) => *all,
            RandomTicks::States { property, values } => combo
                .iter()
                .any(|(name, value)| name == property && values.contains(value)),
        }
    }
}

/// Per-state outline and collision boxes, selected by one state property.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockShapes {
    pub property: String,
    pub values: BTreeMap<String, Vec<[f64; 6]>>,
}

/// A state property a block declares on its own, beside its template's.
/// A block's states become the template's states times these values, so a
/// pipe can have a connection per side without a template that has one.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StateProperty {
    /// `false` and `true`, in vanilla's spelling.
    Boolean,
    /// A list of values, in the order the state space is built in.
    Enum { values: Vec<String> },
}

impl StateProperty {
    /// Its values, in the order the state space is built in; a boolean's two
    /// come `false` first, as vanilla orders them.
    pub fn values(&self) -> Vec<&str> {
        match self {
            StateProperty::Boolean => vec!["false", "true"],
            StateProperty::Enum { values } => values.iter().map(String::as_str).collect(),
        }
    }
}

/// A vanilla state property's name, and a value's: lower case, digits and
/// underscores (`minecraft:oak_log` is `axis`, a repeater is `delay`).
fn state_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockFile {
    template: String,
    #[serde(default)]
    properties: BlockProperties,
    shapes: Option<BlockShapes>,
    #[serde(default)]
    state_properties: BTreeMap<String, StateProperty>,
    #[serde(default)]
    default_state: BTreeMap<String, String>,
    #[serde(default)]
    connects_to: Option<String>,
    #[serde(default)]
    thickness: Option<f32>,
    #[serde(default = "yes")]
    item: bool,
    #[serde(default)]
    components: BTreeMap<String, serde_json::Value>,
}
fn yes() -> bool {
    true
}

/// A block the mod adds: a vanilla template's behaviour, states and shapes
/// under a new id, with `properties` applied. A native mod may build one in
/// code (`native_blocks`): `..Default::default()` leaves the rest as a file
/// leaves it out, except `item`, which a file sets unless told not to.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The vanilla block it copies, `minecraft:<path>`.
    pub template: String,
    pub properties: BlockProperties,
    pub shapes: Option<BlockShapes>,
    /// State properties of its own, which its template does not have.
    pub state_properties: BTreeMap<String, StateProperty>,
    /// State properties overlaid on the template default.
    pub default_state: BTreeMap<String, String>,
    /// Also add a block item with the same id.
    pub item: bool,
    /// Its item's component patch, as an [`Item`]'s.
    pub components: BTreeMap<String, serde_json::Value>,
    /// A connecting block (template `minecraft:chorus_plant`, a `PipeBlock`
    /// without the chorus plant's rules): the block tag, without `#`, whose
    /// blocks it joins besides itself (a cable's machines).
    pub connects_to: Option<String>,
    /// A connecting block's width in pixels (2 to 14): its shapes, the
    /// chorus plant's (10) when unset.
    pub thickness: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreativeTabFile {
    title: String,
    icon: String,
    items: Vec<String>,
}

/// A creative inventory tab the mod adds (`data/<id>/horizon/creative_tab/`):
/// its title, the item on its button, and its items in order. Items no mod
/// or vanilla registers are left out when the game loads.
#[derive(Clone, Debug, PartialEq)]
pub struct CreativeTab {
    /// `<mod id>:<path>`.
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    template: String,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
    #[serde(default)]
    components: BTreeMap<String, serde_json::Value>,
    crafting_remainder: Option<String>,
}

/// An item the mod adds: a vanilla template item's components under a new
/// id, with these changed. A native mod may build one in code
/// (`native_items`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub id: String,
    /// The vanilla item it copies, `minecraft:<path>`.
    pub template: String,
    pub max_stack_size: Option<u8>,
    pub rarity: Option<Rarity>,
    /// A 26.3 data component patch over the template's: `!minecraft:<type>` removes one.
    pub components: BTreeMap<String, serde_json::Value>,
    /// What it leaves in the crafting grid (`Item.Properties.craftRemainder`):
    /// a vanilla item or one of the mod's own.
    pub crafting_remainder: Option<String>,
}

/// `MobEffectCategory`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EffectCategory {
    Beneficial,
    Harmful,
    Neutral,
}

/// A mob effect the mod adds: what `new MobEffect(category, color)` takes.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MobEffect {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    pub category: EffectCategory,
    /// RGB, `0x000000` to `0xFFFFFF`.
    pub color: u32,
}

/// the vanilla entity types a mod's type may copy: thrown item
/// projectiles (`ThrowableItemProjectile`).
pub const ENTITY_TEMPLATES: [&str; 1] = ["minecraft:snowball"];

/// an entity type the mod adds: a copy of a vanilla template's, thrown
/// from `item` and drawn as it.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntityType {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    /// one of [`ENTITY_TEMPLATES`].
    pub template: String,
    /// one of the mod's own items, which throws it.
    pub item: String,
    /// what a hit deals to an entity, as `thrown` damage.
    #[serde(default)]
    pub damage: f32,
}

/// a menu type the mod adds (vanilla's `MenuType`, NeoForge's
/// `IMenuTypeExtension.create`): its container's slot count, the
/// `ContainerData` count its menu syncs, and its take-only slots
/// (`FurnaceResultSlot.mayPlace`). The slots come first in the menu, then the
/// player's 36, and a mod screen (`native_screen`) places them.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MenuType {
    /// `<mod id>:<path>`.
    #[serde(skip)]
    pub id: String,
    /// 1 to [`MAX_MENU_SLOTS`].
    pub slots: u16,
    /// 0 to [`MAX_MENU_SLOTS`].
    #[serde(default)]
    pub data_slots: u16,
    /// slots a player takes from but never puts into, each below `slots`.
    #[serde(default)]
    pub results: Vec<u16>,
}

/// the most container or data slots a mod menu type has, as many as a mod
/// container (`native_block_entity_container`) may.
pub const MAX_MENU_SLOTS: u16 = 256;

#[derive(Debug)]
enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
    /// No files: blocks and items a native mod declares in code.
    Code,
}

/// A mod's content: its files and declarations.
#[derive(Debug)]
pub struct Content {
    source: Source,
    /// Every content file (`assets/...`, `data/...`), sorted.
    pub files: Vec<String>,
    pub blocks: Vec<Block>,
    pub items: Vec<Item>,
    pub effects: Vec<MobEffect>,
    /// Its creative inventory tabs, by id.
    pub creative_tabs: Vec<CreativeTab>,
    pub entity_types: Vec<EntityType>,
    pub menu_types: Vec<MenuType>,
    /// The namespaces it owns: the mod id, then the manifest's `namespaces`.
    pub namespaces: Vec<String>,
}

/// Reads a mod's content files ([`Content::reader`]).
pub enum FileReader<'a> {
    Dir(&'a Path),
    Zip(Box<zip::ZipArchive<std::fs::File>>),
}
impl FileReader<'_> {
    /// One content file's bytes.
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        check_path(name)?;
        let reader: Box<dyn Read + '_> = match self {
            Self::Dir(root) => {
                Box::new(std::fs::File::open(root.join(name)).map_err(|e| format!("{name}: {e}"))?)
            }
            Self::Zip(zip) => Box::new(zip.by_name(name).map_err(|e| format!("{name}: {e}"))?),
        };
        let mut out = Vec::new();
        reader
            .take(MAX_FILE + 1)
            .read_to_end(&mut out)
            .map_err(|e| format!("{name}: {e}"))?;
        if out.len() as u64 > MAX_FILE {
            return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
        }
        Ok(out)
    }
}

/// Why two content mods cannot be on together: a namespace both own (one's
/// id or listed namespace is the other's), which would mix their files.
/// `a` and `b` are (mod id, owned namespaces).
pub fn clash(a: (&str, &[String]), b: (&str, &[String])) -> Option<String> {
    let shared = a.1.iter().find(|ns| b.1.contains(ns))?;
    Some(format!(
        "mods {} and {} both use the namespace {shared}; turn one of them off",
        a.0, b.0
    ))
}

impl Content {
    /// The content of the mod at `path` (a `.hmod` or a mod folder).
    pub fn read(path: &Path, manifest: &Manifest) -> Result<Content, String> {
        Self::read_declaring(path, manifest, &[])
    }
    /// [`Content::read`] for a mod whose native code also declares items (`native_items`):
    /// a file's `crafting_remainder` may name one of those `declared` ids, which are not in the
    /// files.
    pub fn read_declaring(
        path: &Path,
        manifest: &Manifest,
        declared: &[String],
    ) -> Result<Content, String> {
        let (source, sized) = if path.is_dir() {
            let mut out = Vec::new();
            for top in ["assets", "data"] {
                walk(path, &path.join(top), &mut out)?;
            }
            (Source::Dir(path.to_owned()), out)
        } else {
            let mut zip = open_zip(path)?;
            let mut out = Vec::new();
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(|e| e.to_string())?;
                let name = entry.name().map_err(|e| e.to_string())?.into_owned();
                if (name.starts_with("assets/") || name.starts_with("data/")) && !entry.is_dir() {
                    out.push((name.to_owned(), entry.size()));
                }
            }
            (Source::Zip(path.to_owned()), out)
        };
        let mut files = Vec::with_capacity(sized.len());
        let mut total = 0u64;
        for (name, size) in sized {
            check_path(&name)?;
            if size > MAX_FILE {
                return Err(format!("{name} is over {} MiB", MAX_FILE >> 20));
            }
            total += size;
            files.push(name);
        }
        if files.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
        if total > MAX_TOTAL {
            return Err(format!("content is over {} MiB", MAX_TOTAL >> 20));
        }
        files.sort();
        files.dedup();
        let mut content = Content {
            source,
            files,
            blocks: Vec::new(),
            items: Vec::new(),
            effects: Vec::new(),
            creative_tabs: Vec::new(),
            entity_types: Vec::new(),
            menu_types: Vec::new(),
            namespaces: manifest.owned_namespaces().map(str::to_owned).collect(),
        };
        content.check(manifest, declared)?;
        Ok(content)
    }

    /// Content with no files, for the blocks and items native mods declare
    /// in code in `namespace` ([`Content::declare`]).
    pub fn declared(namespace: &str) -> Content {
        Content {
            source: Source::Code,
            files: Vec::new(),
            blocks: Vec::new(),
            items: Vec::new(),
            effects: Vec::new(),
            creative_tabs: Vec::new(),
            entity_types: Vec::new(),
            menu_types: Vec::new(),
            namespaces: vec![namespace.to_owned()],
        }
    }

    /// Adds blocks and items a native mod declares in code (`native_blocks`,
    /// `native_items`), checked as declaration files are: ids in this mod's
    /// own namespace (its id, as `data/<id>/horizon/` is) and declared once
    /// with what it already has, a vanilla template, a crafting remainder
    /// that is vanilla or this mod's. All or nothing: on an error nothing is
    /// added.
    pub fn declare(&mut self, blocks: &[Block], items: &[Item]) -> Result<(), String> {
        let own = self
            .namespaces
            .first()
            .map(String::as_str)
            .unwrap_or_default();
        let mut ids: BTreeSet<&str> = self
            .blocks
            .iter()
            .map(|b| b.id.as_str())
            .chain(self.items.iter().map(|i| i.id.as_str()))
            .collect();
        let mut item_ids: BTreeSet<&str> = self
            .blocks
            .iter()
            .filter(|b| b.item)
            .map(|b| b.id.as_str())
            .chain(self.items.iter().map(|i| i.id.as_str()))
            .collect();
        let in_own =
            |id: &str| namespaced_id(id) && id.split_once(':').is_some_and(|(ns, _)| ns == own);
        for block in blocks {
            let id = block.id.as_str();
            if !in_own(id) {
                return Err(format!("block {id:?} is not an id in {own}:"));
            }
            check_block(block).map_err(|e| format!("block {id}: {e}"))?;
            if !ids.insert(id) {
                return Err(format!("block {id} is declared twice"));
            }
            if block.item {
                item_ids.insert(id);
            }
        }
        for item in items {
            let id = item.id.as_str();
            if !in_own(id) {
                return Err(format!("item {id:?} is not an id in {own}:"));
            }
            check_item(item).map_err(|e| format!("item {id}: {e}"))?;
            if !item_ids.insert(id) {
                return Err(format!(
                    "item {id} is declared twice, or is also a block's item"
                ));
            }
        }
        for item in items {
            if let Some(rest) = &item.crafting_remainder
                && (rest == "minecraft:air"
                    || !vanilla_id(rest) && !item_ids.contains(rest.as_str()))
            {
                return Err(format!(
                    "item {}: crafting_remainder {rest:?} is not a vanilla item (minecraft:...) or one of this mod's",
                    item.id
                ));
            }
        }
        self.blocks.extend_from_slice(blocks);
        self.items.extend_from_slice(items);
        Ok(())
    }

    /// Where the files are read from: the mod's folder or its `.hmod`.
    pub fn source(&self) -> &Path {
        match &self.source {
            Source::Dir(path) | Source::Zip(path) => path,
            Source::Code => Path::new(""),
        }
    }

    /// No files: a mod with code only.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// One content file's bytes.
    pub fn file(&self, name: &str) -> Result<Vec<u8>, String> {
        self.reader()?.read(name)
    }

    /// A reader of its content files. A packaged mod's archive is opened
    /// once for all of them (opening it reads its whole index).
    pub fn reader(&self) -> Result<FileReader<'_>, String> {
        Ok(match &self.source {
            Source::Dir(root) => FileReader::Dir(root),
            Source::Zip(path) => FileReader::Zip(Box::new(open_zip(path)?)),
            Source::Code => FileReader::Dir(Path::new("")),
        })
    }

    /// Where content may go, and the declarations.
    fn check(&mut self, manifest: &Manifest, declared: &[String]) -> Result<(), String> {
        let id = manifest.id.as_str();
        if !self.files.is_empty() && !manifest.capabilities.contains(&Capability::Registries) {
            return Err(format!(
                "has assets/ or data/ but not capability registries in {MANIFEST}"
            ));
        }
        let mut blocks = Vec::new();
        let mut items = Vec::new();
        let mut effects = Vec::new();
        let mut tabs = Vec::new();
        let mut types = Vec::new();
        let mut menus = Vec::new();
        for name in &self.files {
            let mut parts = name.splitn(3, '/');
            let (top, namespace, rest) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
                parts.next().unwrap_or(""),
            );
            let own = namespace == id;
            let owned = own || manifest.namespaces.iter().any(|n| n == namespace);
            // Vanilla's own files stay vanilla's; a mod may only add to its
            // tags, and to the convention tags mods share.
            let shared = matches!(namespace, "minecraft" | "c");
            let allowed = owned || (top == "data" && shared && rest.starts_with("tags/"));
            if !allowed {
                return Err(format!(
                    "{name}: a mod adds files in its own namespaces ({top}/{id}/ and those listed in namespaces) or to vanilla and convention (c) tags"
                ));
            }
            if !own && top == "data" && rest.starts_with("horizon/") {
                return Err(format!(
                    "{name}: blocks, items, mob effects, entity types and menu types are declared in data/{id}/horizon/"
                ));
            }
            let Some(decl) = (own && top == "data")
                .then(|| rest.strip_prefix("horizon/"))
                .flatten()
            else {
                continue;
            };
            let parse = |kind: &str| -> Option<String> {
                let path = decl
                    .strip_prefix(kind)?
                    .strip_prefix('/')?
                    .strip_suffix(".json")?;
                Some(format!("{id}:{path}"))
            };
            if let Some(block) = parse("block") {
                blocks.push((block, name.clone()));
            } else if let Some(item) = parse("item") {
                items.push((item, name.clone()));
            } else if let Some(effect) = parse("mob_effect") {
                effects.push((effect, name.clone()));
            } else if let Some(tab) = parse("creative_tab") {
                tabs.push((tab, name.clone()));
            } else if let Some(kind) = parse("entity_type") {
                types.push((kind, name.clone()));
            } else if let Some(menu) = parse("menu") {
                menus.push((menu, name.clone()));
            } else {
                return Err(format!(
                    "{name}: data/{id}/horizon/ holds block/, item/, mob_effect/, creative_tab/, entity_type/ and menu/*.json only"
                ));
            }
        }
        let mut ids = BTreeSet::new();
        for (block, name) in blocks {
            let file: BlockFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if file
                .connects_to
                .as_ref()
                .is_some_and(|t| !t.starts_with('#'))
            {
                return Err(format!(
                    "{name}: connects_to is a block tag, #namespace:path"
                ));
            }
            let block = Block {
                id: block,
                template: file.template,
                properties: file.properties,
                shapes: file.shapes,
                state_properties: file.state_properties,
                default_state: file.default_state,
                item: file.item,
                components: file.components,
                connects_to: file
                    .connects_to
                    .map(|t| t.trim_start_matches('#').to_owned()),
                thickness: file.thickness,
            };
            check_block(&block).map_err(|e| format!("{name}: {e}"))?;
            if block.item {
                ids.insert(block.id.clone());
            }
            self.blocks.push(block);
        }
        let mut remainders = Vec::new();
        for (item, name) in items {
            let file: ItemFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            let item = Item {
                id: item,
                template: file.template,
                max_stack_size: file.max_stack_size,
                rarity: file.rarity,
                components: file.components,
                crafting_remainder: file.crafting_remainder,
            };
            check_item(&item).map_err(|e| format!("{name}: {e}"))?;
            if !ids.insert(item.id.clone()) {
                return Err(format!("{name}: {} is also a block's item", item.id));
            }
            if let Some(rest) = &item.crafting_remainder {
                remainders.push((name, rest.clone()));
            }
            self.items.push(item);
        }
        for (effect, name) in effects {
            let file: MobEffect =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if file.color > 0xFF_FFFF {
                return Err(format!("{name}: color is RGB, 0 to 16777215 (0xFFFFFF)"));
            }
            self.effects.push(MobEffect { id: effect, ..file });
        }
        // After every item, so a remainder may be declared in a later file.
        for (name, rest) in remainders {
            if rest == "minecraft:air" {
                return Err(format!(
                    "{name}: crafting_remainder {rest:?} is no remainder (leave it out)"
                ));
            }
            if !vanilla_id(&rest) && !ids.contains(&rest) && !declared.contains(&rest) {
                return Err(format!(
                    "{name}: crafting_remainder {rest:?} is not a vanilla item (minecraft:...) or one of this mod's"
                ));
            }
        }
        for (tab, name) in tabs {
            let file: CreativeTabFile =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            let id_like = |s: &str| {
                s.split_once(':')
                    .is_some_and(|(n, p)| !n.is_empty() && !p.is_empty())
            };
            if !id_like(&file.icon) || !file.items.iter().all(|i| id_like(i)) {
                return Err(format!(
                    "{name}: icon and items are item ids (namespace:path)"
                ));
            }
            self.creative_tabs.push(CreativeTab {
                id: tab,
                title: file.title,
                icon: file.icon,
                items: file.items,
            });
        }
        for (kind, name) in types {
            let file: EntityType =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if !ENTITY_TEMPLATES.contains(&file.template.as_str()) {
                return Err(format!(
                    "{name}: template {:?} is not one of {ENTITY_TEMPLATES:?}",
                    file.template
                ));
            }
            // its own item, so no vanilla item loses its use to it.
            if !ids.contains(&file.item) {
                return Err(format!(
                    "{name}: item {:?} is not one of this mod's items",
                    file.item
                ));
            }
            if self.entity_types.iter().any(|t| t.item == file.item) {
                return Err(format!(
                    "{name}: item {:?} already throws another entity type",
                    file.item
                ));
            }
            if !(file.damage.is_finite() && file.damage >= 0.0) {
                return Err(format!("{name}: damage is 0 or more"));
            }
            self.entity_types.push(EntityType { id: kind, ..file });
        }
        for (menu, name) in menus {
            let file: MenuType =
                serde_json::from_slice(&self.file(&name)?).map_err(|e| format!("{name}: {e}"))?;
            if !(1..=MAX_MENU_SLOTS).contains(&file.slots) || file.data_slots > MAX_MENU_SLOTS {
                return Err(format!(
                    "{name}: slots is 1 to {MAX_MENU_SLOTS}, data_slots 0 to {MAX_MENU_SLOTS}"
                ));
            }
            let mut seen = BTreeSet::new();
            for &r in &file.results {
                if r >= file.slots {
                    return Err(format!(
                        "{name}: result {r} is past the last slot, {}",
                        file.slots - 1
                    ));
                }
                if !seen.insert(r) {
                    return Err(format!("{name}: result {r} is listed twice"));
                }
            }
            self.menu_types.push(MenuType { id: menu, ..file });
        }
        Ok(())
    }
}

/// Namespaced component types, removals as `{}`, and none of `fields`,
/// which the file sets with its own keys.
fn check_components(
    components: &BTreeMap<String, serde_json::Value>,
    fields: &[&str],
) -> Result<(), String> {
    for (key, value) in components {
        let (removed, kind) = match key.strip_prefix('!') {
            Some(kind) => (true, kind),
            None => (false, key.as_str()),
        };
        if !namespaced_id(kind) {
            return Err(format!("component {key:?} must be namespaced and valid"));
        }
        if let Some(own) = kind
            .strip_prefix("minecraft:")
            .filter(|k| fields.contains(k))
        {
            return Err(format!("component {key:?}: set {own} instead"));
        }
        if removed && !value.as_object().is_some_and(|o| o.is_empty()) {
            return Err(format!("component {key:?}: a removal's value is {{}}"));
        }
    }
    Ok(())
}

/// An item's fields, as a file or code gives them.
fn check_item(item: &Item) -> Result<(), String> {
    if !vanilla_id(&item.template) {
        return Err(format!(
            "template {:?} is not a vanilla item (minecraft:...)",
            item.template
        ));
    }
    if item.max_stack_size.is_some_and(|n| !(1..=99).contains(&n)) {
        return Err("max_stack_size is 1 to 99".into());
    }
    check_components(&item.components, &["max_stack_size", "rarity"])
}

/// A block's fields, as a file or code gives them (`connects_to` without
/// its `#`).
fn check_block(file: &Block) -> Result<(), String> {
    if !vanilla_id(&file.template) {
        return Err(format!(
            "template {:?} is not a vanilla block (minecraft:...)",
            file.template
        ));
    }
    if let Some(shapes) = &file.shapes {
        if shapes.property.is_empty() || shapes.values.is_empty() {
            return Err("shapes needs a property and values".into());
        }
        for boxes in shapes.values.values() {
            if boxes.len() > 16
                || boxes
                    .iter()
                    .any(|b| b.iter().any(|n| !n.is_finite()) || (0..3).any(|i| b[i] > b[i + 3]))
            {
                return Err("shape needs at most 16 finite boxes with ordered bounds".into());
            }
        }
    }
    for (name, property) in &file.state_properties {
        if !state_name(name) {
            return Err(format!(
                "state property {name:?} is lower case letters, digits and underscores"
            ));
        }
        let values = property.values();
        if values.is_empty() {
            return Err(format!("state property {name:?} needs a value"));
        }
        let mut seen = BTreeSet::new();
        for value in values {
            if !state_name(value) {
                return Err(format!(
                    "state property {name:?}: value {value:?} is lower case letters, digits and underscores"
                ));
            }
            if !seen.insert(value) {
                return Err(format!("state property {name:?}: value {value:?} repeats"));
            }
        }
    }
    for key in file.default_state.keys() {
        if !state_name(key) {
            return Err(format!(
                "default_state key {key:?} is lower case letters, digits and underscores"
            ));
        }
    }
    let p = &file.properties;
    if let Some(RandomTicks::States { property, values }) = &p.random_ticks {
        let declared = file
            .state_properties
            .get(property)
            .map(StateProperty::values);
        if declared.is_none_or(|all| {
            values.is_empty() || values.iter().any(|v| !all.contains(&v.as_str()))
        }) {
            return Err(format!(
                "random_ticks needs values of the block's own state property {property:?} (state_properties)"
            ));
        }
    }
    let finite = |v: Option<f32>| v.is_none_or(f32::is_finite);
    if !finite(p.hardness) || !finite(p.explosion_resistance) || !finite(p.friction) {
        return Err("hardness, explosion_resistance and friction are numbers".into());
    }
    if p.hardness.is_some_and(|h| h < 0.0 && h != -1.0) {
        return Err("hardness is 0 or more, or -1 (unbreakable)".into());
    }
    if p.explosion_resistance.is_some_and(|r| r < 0.0) {
        return Err("explosion_resistance is 0 or more".into());
    }
    if p.friction.is_some_and(|f| !(0.0..=2.0).contains(&f)) {
        return Err("friction is 0 to 2".into());
    }
    if p.light.is_some_and(|l| l > 15) {
        return Err("light is 0 to 15".into());
    }
    if p.map_color.is_some_and(|c| c > 61) {
        return Err("map_color is a MapColor id, 0 to 61".into());
    }
    if p.sounds.as_deref().is_some_and(|s| !vanilla_id(s)) {
        return Err("sounds names a vanilla block (minecraft:...)".into());
    }
    if !file.item && !file.components.is_empty() {
        return Err("item is false, so the block has no item to take components".into());
    }
    check_components(&file.components, &[])?;
    // A block file has no `max_stack_size` / `rarity` keys, so those two take the same checks here.
    if let Some(n) = file.components.get("minecraft:max_stack_size")
        && !n.as_u64().is_some_and(|n| (1..=99).contains(&n))
    {
        return Err("component \"minecraft:max_stack_size\" is an integer 1 to 99".into());
    }
    if let Some(r) = file.components.get("minecraft:rarity")
        && serde_json::from_value::<Rarity>(r.clone()).is_err()
    {
        return Err("component \"minecraft:rarity\" is common, uncommon, rare or epic".into());
    }
    if let Some(tag) = &file.connects_to {
        if file.template != "minecraft:chorus_plant" {
            return Err("connects_to needs template minecraft:chorus_plant (a PipeBlock)".into());
        }
        let id = tag.as_str();
        let valid = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-./".contains(c))
        };
        if !id
            .split_once(':')
            .is_some_and(|(ns, path)| valid(ns) && valid(path))
        {
            return Err("connects_to is a block tag, #namespace:path".into());
        }
    }
    match file.thickness {
        Some(_) if file.connects_to.is_none() => {
            return Err("thickness is a connecting block's (connects_to)".into());
        }
        Some(t) if !(2.0..=14.0).contains(&t) => return Err("thickness is 2 to 14 pixels".into()),
        Some(_) if file.shapes.is_some() => {
            return Err("thickness sets the shapes, so a block has one or the other".into());
        }
        _ => {}
    }
    Ok(())
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    zip::ZipArchive::new(file).map_err(|e| e.to_string())
}

/// Files under `dir`, relative to `root` with `/` separators. Symbolic links
/// are not followed: a mod folder holds its own files.
fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<(), String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read.flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?;
            let parts: Option<Vec<&str>> = rel.iter().map(|p| p.to_str()).collect();
            let name = parts.ok_or_else(|| format!("{} is not UTF-8", rel.display()))?;
            out.push((name.join("/"), meta.len()));
        }
        if out.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} content files"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Package;

    const MANIFEST_TEXT: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;
    const BLOCK: &[u8] = br#"{"template": "minecraft:iron_block",
        "properties": {"hardness": 4.0, "light": 3, "sounds": "minecraft:amethyst_block"}}"#;
    const ITEM: &[u8] = br#"{"template": "minecraft:diamond", "rarity": "uncommon"}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-content-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn hmod(name: &str, manifest: &str, files: &[(&str, &[u8])]) -> Result<Content, String> {
        let dir = scratch(name);
        let path = dir.join("m.hmod");
        std::fs::write(&path, Package::write_files(manifest, None, files)?).unwrap();
        let package = Package::read(&path);
        let content = package.and_then(|p| Content::read(&path, &p.manifest));
        std::fs::remove_dir_all(&dir).unwrap();
        content
    }

    #[test]
    fn blocks_and_items_declared_in_code_pass_the_files_checks() {
        let block = |id: &str| Block {
            id: id.into(),
            template: "minecraft:iron_block".into(),
            item: true,
            ..Block::default()
        };
        let item = |id: &str| Item {
            id: id.into(),
            template: "minecraft:diamond".into(),
            ..Item::default()
        };
        let mut content = Content::declared("ruby");
        assert!(content.is_empty());
        let wrench = Item {
            crafting_remainder: Some("ruby:gear".into()),
            ..item("ruby:wrench")
        };
        content
            .declare(&[block("ruby:gear"), block("ruby:casing")], &[wrench])
            .unwrap();
        assert_eq!(content.blocks.len(), 2);
        assert_eq!(content.items[0].id, "ruby:wrench");
        // Each refusal adds nothing.
        let refused = |blocks: &[Block], items: &[Item]| {
            let mut c = Content::declared("ruby");
            c.declare(&[block("ruby:gear")], &[]).unwrap();
            let e = c.declare(blocks, items).unwrap_err();
            assert_eq!((c.blocks.len(), c.items.len()), (1, 0), "{e}");
            e
        };
        assert!(refused(&[block("emerald:gear")], &[]).contains("not an id in ruby:"));
        assert!(refused(&[block("minecraft:gear")], &[]).contains("not an id in ruby:"));
        assert!(refused(&[], &[item("ruby")]).contains("not an id in ruby:"));
        assert!(refused(&[block("ruby:gear")], &[]).contains("declared twice"));
        assert!(refused(&[], &[item("ruby:gear")]).contains("block's item"));
        let modded = Block {
            template: "ruby:other".into(),
            ..block("ruby:casing")
        };
        assert!(refused(&[modded], &[]).contains("not a vanilla block"));
        let dim = Block {
            properties: BlockProperties {
                light: Some(16),
                ..BlockProperties::default()
            },
            ..block("ruby:lamp")
        };
        assert!(refused(&[dim], &[]).contains("light is 0 to 15"));
        let stack = Item {
            max_stack_size: Some(100),
            ..item("ruby:ingot")
        };
        assert!(refused(&[], &[stack]).contains("max_stack_size"));
        let rest = Item {
            crafting_remainder: Some("emerald:dust".into()),
            ..item("ruby:dust")
        };
        assert!(refused(&[block("ruby:pipe")], &[rest]).contains("crafting_remainder"));
        // A cable's tag comes bare from code.
        let cable = Block {
            template: "minecraft:chorus_plant".into(),
            connects_to: Some("ruby:cables".into()),
            ..block("ruby:cable")
        };
        content.declare(&[cable], &[]).unwrap();
    }

    #[test]
    fn a_connecting_block_file_names_its_tag_with_a_hash() {
        let cable = br#"{"template": "minecraft:chorus_plant", "connects_to": "ruby:cables"}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/horizon/block/cable.json", cable)];
        let e = hmod("bare-tag", MANIFEST_TEXT, files).unwrap_err();
        assert!(e.contains("#namespace:path"), "{e}");
    }

    #[test]
    fn content_only_mod_round_trips() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/horizon/block/ruby_block.json", BLOCK),
            ("data/ruby/horizon/item/ruby.json", ITEM),
            ("data/ruby/recipe/ruby_block.json", b"{}"),
            ("data/minecraft/tags/block/mineable/pickaxe.json", b"{}"),
            ("data/c/tags/item/gems/ruby.json", b"{}"),
            ("assets/ruby/textures/block/ruby_block.png", b"png"),
        ];
        let content = hmod("ok", MANIFEST_TEXT, files).unwrap();
        assert_eq!(content.files.len(), 6);
        assert_eq!(
            content.blocks,
            [Block {
                id: "ruby:ruby_block".into(),
                template: "minecraft:iron_block".into(),
                shapes: None,
                state_properties: BTreeMap::new(),
                default_state: BTreeMap::new(),
                properties: BlockProperties {
                    hardness: Some(4.0),
                    light: Some(3),
                    sounds: Some("minecraft:amethyst_block".into()),
                    ..BlockProperties::default()
                },
                item: true,
                components: BTreeMap::new(),
                connects_to: None,
                thickness: None,
            }]
        );
        assert_eq!(content.items[0].id, "ruby:ruby");
        assert_eq!(content.items[0].rarity, Some(Rarity::Uncommon));
        assert!(content.items[0].components.is_empty());
        assert_eq!(content.items[0].crafting_remainder, None);
    }

    #[test]
    fn items_name_their_crafting_remainder() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/horizon/item/a_stew.json",
                br#"{"template": "minecraft:mushroom_stew", "crafting_remainder": "minecraft:bowl"}"#,
            ),
            (
                "data/ruby/horizon/item/b_flask.json",
                br#"{"template": "minecraft:potion", "crafting_remainder": "ruby:c_flask"}"#,
            ),
            ("data/ruby/horizon/item/c_flask.json", ITEM),
        ];
        let content = hmod("remainder", MANIFEST_TEXT, files).unwrap();
        let rests: Vec<_> = content
            .items
            .iter()
            .map(|i| i.crafting_remainder.as_deref())
            .collect();
        assert_eq!(rests, [Some("minecraft:bowl"), Some("ruby:c_flask"), None]);
    }

    #[test]
    fn a_file_item_may_leave_a_code_declared_item_as_its_remainder() {
        // `native_items` declares ruby:wrench in code; a file item names it as its remainder.
        let files: &[(&str, &[u8])] = &[(
            "data/ruby/horizon/item/tool.json",
            br#"{"template": "minecraft:iron_pickaxe", "crafting_remainder": "ruby:wrench"}"#,
        )];
        let read = |declared: &[String]| {
            let dir = scratch("declared-remainder");
            let path = dir.join("m.hmod");
            std::fs::write(
                &path,
                Package::write_files(MANIFEST_TEXT, None, files).unwrap(),
            )
            .unwrap();
            let package = Package::read(&path).unwrap();
            let content = Content::read_declaring(&path, &package.manifest, declared);
            std::fs::remove_dir_all(&dir).unwrap();
            content
        };
        assert!(read(&[]).is_err(), "a remainder nobody declares is refused");
        let content = read(&["ruby:wrench".to_owned()]).expect("the code-declared item is known");
        assert_eq!(
            content.items[0].crafting_remainder.as_deref(),
            Some("ruby:wrench")
        );
    }

    #[test]
    fn items_patch_their_template_components() {
        let sword = br#"{"template": "minecraft:iron_sword", "components": {
            "minecraft:max_damage": 900,
            "minecraft:attribute_modifiers": [{"type": "minecraft:attack_damage",
                "id": "minecraft:base_attack_damage", "amount": 7.0,
                "operation": "add_value", "slot": "mainhand"}],
            "!minecraft:repairable": {}}}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/horizon/item/ruby_sword.json", sword)];
        let content = hmod("components", MANIFEST_TEXT, files).unwrap();
        let components = &content.items[0].components;
        assert_eq!(
            components.keys().collect::<Vec<_>>(),
            [
                "!minecraft:repairable",
                "minecraft:attribute_modifiers",
                "minecraft:max_damage"
            ]
        );
        assert_eq!(components["minecraft:max_damage"], 900);
    }

    #[test]
    fn mob_effects_take_a_category_and_a_colour() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/horizon/mob_effect/glow.json",
                br#"{"category": "beneficial", "color": 16711680}"#,
            ),
            (
                "data/ruby/horizon/mob_effect/curse/dark.json",
                br#"{"category": "harmful", "color": 0}"#,
            ),
        ];
        let content = hmod("effects", MANIFEST_TEXT, files).unwrap();
        let effects: Vec<_> = content
            .effects
            .iter()
            .map(|e| (e.id.as_str(), e.category, e.color))
            .collect();
        assert_eq!(
            effects,
            [
                ("ruby:curse/dark", EffectCategory::Harmful, 0),
                ("ruby:glow", EffectCategory::Beneficial, 0xFF0000),
            ]
        );
        for (case, file, why) in [
            (
                "effect-colour",
                r#"{"category": "neutral", "color": 16777216}"#,
                "color is RGB",
            ),
            (
                "effect-category",
                r#"{"category": "good", "color": 1}"#,
                "unknown variant",
            ),
            (
                "effect-key",
                r#"{"category": "neutral", "color": 1, "id": "x"}"#,
                "unknown field",
            ),
            (
                "effect-missing",
                r#"{"category": "neutral"}"#,
                "missing field",
            ),
        ] {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/horizon/mob_effect/a.json", file.as_bytes())];
            let e = hmod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn menu_types_take_slots_data_slots_and_results() {
        let files: &[(&str, &[u8])] = &[
            (
                "data/ruby/horizon/menu/press.json",
                br#"{"slots": 9, "data_slots": 2, "results": [7, 8]}"#,
            ),
            ("data/ruby/horizon/menu/bag.json", br#"{"slots": 4}"#),
        ];
        let content = hmod("menus", MANIFEST_TEXT, files).unwrap();
        let menus: Vec<_> = content
            .menu_types
            .iter()
            .map(|m| (m.id.as_str(), m.slots, m.data_slots, m.results.clone()))
            .collect();
        assert_eq!(
            menus,
            [("ruby:bag", 4, 0, vec![]), ("ruby:press", 9, 2, vec![7, 8])]
        );
        for (case, file, why) in [
            ("menu-empty", r#"{"slots": 0}"#, "slots is 1 to 256"),
            ("menu-large", r#"{"slots": 257}"#, "slots is 1 to 256"),
            (
                "menu-data",
                r#"{"slots": 1, "data_slots": 257}"#,
                "data_slots 0 to 256",
            ),
            (
                "menu-result",
                r#"{"slots": 3, "results": [3]}"#,
                "result 3 is past the last slot, 2",
            ),
            (
                "menu-twice",
                r#"{"slots": 3, "results": [1, 1]}"#,
                "result 1 is listed twice",
            ),
            ("menu-key", r#"{"slots": 3, "rows": 1}"#, "unknown field"),
            ("menu-missing", r#"{"data_slots": 1}"#, "missing field"),
        ] {
            let files: &[(&str, &[u8])] = &[("data/ruby/horizon/menu/a.json", file.as_bytes())];
            let e = hmod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn entity_types_copy_a_throwable_and_name_its_item() {
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/horizon/item/gem.json", ITEM),
            (
                "data/ruby/horizon/entity_type/bolt.json",
                br#"{"template": "minecraft:snowball", "item": "ruby:gem", "damage": 2.5}"#,
            ),
            ("data/ruby/horizon/item/dust.json", ITEM),
            (
                "data/ruby/horizon/entity_type/puff.json",
                br#"{"template": "minecraft:snowball", "item": "ruby:dust"}"#,
            ),
        ];
        let content = hmod("types", MANIFEST_TEXT, files).unwrap();
        let types: Vec<_> = content
            .entity_types
            .iter()
            .map(|t| (t.id.as_str(), t.item.as_str(), t.damage))
            .collect();
        assert_eq!(
            types,
            [
                ("ruby:bolt", "ruby:gem", 2.5),
                ("ruby:puff", "ruby:dust", 0.0)
            ]
        );
        let snowball = |rest: &str| format!(r#"{{"template": "minecraft:snowball"{rest}}}"#);
        for (case, file, why) in [
            (
                "type-template",
                r#"{"template": "minecraft:egg", "item": "ruby:gem"}"#.to_owned(),
                "is not one of",
            ),
            (
                "type-vanilla-item",
                snowball(r#", "item": "minecraft:stick""#),
                "is not one of this mod's items",
            ),
            (
                "type-unknown-item",
                snowball(r#", "item": "ruby:none""#),
                "is not one of this mod's items",
            ),
            (
                "type-damage",
                snowball(r#", "item": "ruby:gem", "damage": -1"#),
                "damage is 0 or more",
            ),
            (
                "type-key",
                snowball(r#", "item": "ruby:gem", "size": 1"#),
                "unknown field",
            ),
        ] {
            let files: &[(&str, &[u8])] = &[
                ("data/ruby/horizon/item/gem.json", ITEM),
                ("data/ruby/horizon/entity_type/a.json", file.as_bytes()),
            ];
            let e = hmod(case, MANIFEST_TEXT, files).unwrap_err();
            assert!(e.contains(why), "{case}: {e}");
        }
        let shared = snowball(r#", "item": "ruby:gem""#);
        let files: &[(&str, &[u8])] = &[
            ("data/ruby/horizon/item/gem.json", ITEM),
            ("data/ruby/horizon/entity_type/a.json", shared.as_bytes()),
            ("data/ruby/horizon/entity_type/b.json", shared.as_bytes()),
        ];
        let e = hmod("type-shared", MANIFEST_TEXT, files).unwrap_err();
        assert!(e.contains("already throws another entity type"), "{e}");
    }

    #[test]
    fn blocks_pick_the_states_that_tick_randomly() {
        let own = r#""state_properties": {"grown": {"type": "boolean"}}"#;
        let block = |ticks: &str| {
            let text = format!(
                r#"{{"template": "minecraft:dirt", {own}, "properties": {{"random_ticks": {ticks}}}}}"#
            );
            let files: &[(&str, &[u8])] = &[("data/ruby/horizon/block/crop.json", text.as_bytes())];
            hmod("random-ticks", MANIFEST_TEXT, files)
        };
        let ticks =
            |c: Result<Content, String>| c.unwrap().blocks[0].properties.random_ticks.clone();
        let state = |v: &str| [("grown".to_owned(), v.to_owned())];
        // Every state, whatever the template does; `false` stops a template that ticks.
        let all = ticks(block("true")).unwrap();
        assert!(all.applies(&state("true")) && all.applies(&[]));
        assert!(!ticks(block("false")).unwrap().applies(&state("false")));
        // Chosen states: a crop that stops ticking once it is grown.
        let chosen = ticks(block(r#"{"property": "grown", "values": ["false"]}"#)).unwrap();
        assert!(chosen.applies(&state("false")) && !chosen.applies(&state("true")));
        assert!(
            !chosen.applies(&[]),
            "a state without the property does not tick"
        );
        // Not the block's own property, no values, or a value it doesn't have.
        for bad in [
            r#"{"property": "age", "values": ["0"]}"#,
            r#"{"property": "grown", "values": []}"#,
            r#"{"property": "grown", "values": ["maybe"]}"#,
        ] {
            let e = block(bad).err().unwrap();
            assert!(e.contains("random_ticks"), "{bad}: {e}");
        }
        assert!(block("3").is_err());
    }

    #[test]
    fn block_items_patch_their_components() {
        let block = br#"{"template": "minecraft:iron_block", "components": {
            "minecraft:max_stack_size": 16, "!minecraft:lore": {}}}"#;
        let files: &[(&str, &[u8])] = &[("data/ruby/horizon/block/ruby_block.json", block)];
        let content = hmod("block-components", MANIFEST_TEXT, files).unwrap();
        let components = &content.blocks[0].components;
        assert_eq!(
            components.keys().collect::<Vec<_>>(),
            ["!minecraft:lore", "minecraft:max_stack_size"]
        );
        assert_eq!(components["minecraft:max_stack_size"], 16);
    }

    #[test]
    fn folder_mod_reads_the_same() {
        let dir = scratch("folder");
        std::fs::write(dir.join(MANIFEST), MANIFEST_TEXT).unwrap();
        let block = dir.join("data/ruby/horizon/block");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("ruby_block.json"), BLOCK).unwrap();
        let package = Package::read(&dir).unwrap();
        assert!(package.wasm.is_none());
        let content = Content::read(&dir, &package.manifest).unwrap();
        assert_eq!(content.files, ["data/ruby/horizon/block/ruby_block.json"]);
        assert_eq!(content.file(&content.files[0]).unwrap(), BLOCK);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A connecting block names a block tag and copies a `PipeBlock`.
    #[test]
    fn connecting_blocks_copy_a_pipe_and_name_a_tag() {
        let block = |json: &str| {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/horizon/block/cable.json", json.as_bytes())];
            hmod("cable", MANIFEST_TEXT, files)
        };
        let content = block(
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:cable_joins"}"##,
        )
        .unwrap();
        assert_eq!(
            content.blocks[0].connects_to.as_deref(),
            Some("ruby:cable_joins")
        );
        let thin =
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:j", "thickness": 4}"##;
        assert_eq!(block(thin).unwrap().blocks[0].thickness, Some(4.0));
        let e = block(r#"{"template": "minecraft:chorus_plant", "thickness": 4}"#).unwrap_err();
        assert!(e.contains("connects_to"), "{e}");
        let e = block(r##"{"template": "minecraft:chorus_plant", "connects_to": "#ruby:j", "thickness": 16}"##)
            .unwrap_err();
        assert!(e.contains("2 to 14"), "{e}");
        let e = block(r##"{"template": "minecraft:stone", "connects_to": "#ruby:cable_joins"}"##)
            .unwrap_err();
        assert!(e.contains("chorus_plant"), "{e}");
        for tag in ["ruby:cable_joins", "#cable_joins", "#Ruby:Cable"] {
            let json =
                format!(r#"{{"template": "minecraft:chorus_plant", "connects_to": "{tag}"}}"#);
            let e = block(&json).unwrap_err();
            assert!(e.contains("block tag"), "{tag}: {e}");
        }
    }

    /// A creative tab: a title, an icon and its items, all item ids.
    #[test]
    fn creative_tabs_name_their_items() {
        let tab = |json: &str| {
            let files: &[(&str, &[u8])] =
                &[("data/ruby/horizon/creative_tab/gems.json", json.as_bytes())];
            hmod("tab", MANIFEST_TEXT, files)
        };
        let content =
            tab(r#"{"title": "Ruby Gems", "icon": "ruby:ruby", "items": ["ruby:ruby", "minecraft:diamond"]}"#)
                .unwrap();
        assert_eq!(
            content.creative_tabs,
            [CreativeTab {
                id: "ruby:gems".into(),
                title: "Ruby Gems".into(),
                icon: "ruby:ruby".into(),
                items: vec!["ruby:ruby".into(), "minecraft:diamond".into()],
            }]
        );
        let e = tab(r#"{"title": "Gems", "icon": "ruby", "items": []}"#).unwrap_err();
        assert!(e.contains("item ids"), "{e}");
    }

    #[test]
    fn refuses_content_outside_its_fence() {
        let refused = |name: &str, files: &[(&str, &[u8])]| {
            let e = hmod(name, MANIFEST_TEXT, files).unwrap_err();
            assert!(!e.is_empty());
            e
        };
        let e = refused("other-ns", &[("data/other/horizon/block/a.json", BLOCK)]);
        assert!(e.contains("own namespaces"), "{e}");
        refused(
            "vanilla-asset",
            &[("assets/minecraft/textures/block/stone.png", b"x")],
        );
        refused(
            "vanilla-recipe",
            &[("data/minecraft/recipe/stone.json", b"{}")],
        );
        refused("convention-recipe", &[("data/c/recipe/stone.json", b"{}")]);
        refused("convention-asset", &[("assets/c/lang/en_us.json", b"{}")]);
        let e = refused(
            "modded-template",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "ruby:other"}"#,
            )],
        );
        assert!(e.contains("not a vanilla block"), "{e}");
        refused(
            "light",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "properties": {"light": 16}}"#,
            )],
        );
        refused(
            "unknown-key",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template": "minecraft:stone", "glow": true}"#,
            )],
        );
        refused("shape-reversed", &[("data/ruby/horizon/block/a.json", br#"{"template":"minecraft:stone","shapes":{"property":"axis","values":{"x":[[1,0,0,0,1,1]]}}}"#)]);
        refused("shape-malformed", &[("data/ruby/horizon/block/a.json", br#"{"template":"minecraft:stone","shapes":{"property":"axis","values":{"x":[[0,0,0,1,1]]}}}"#)]);
        refused(
            "shape-no-values",
            &[(
                "data/ruby/horizon/block/a.json",
                br#"{"template":"minecraft:stone","shapes":{"property":"axis","values":{}}}"#,
            )],
        );
        let e = refused(
            "clash",
            &[
                ("data/ruby/horizon/block/a.json", BLOCK),
                ("data/ruby/horizon/item/a.json", ITEM),
            ],
        );
        assert!(e.contains("also a block's item"), "{e}");
        refused("other-kind", &[("data/ruby/horizon/entity/a.json", b"{}")]);
        refused(
            "stack",
            &[(
                "data/ruby/horizon/item/a.json",
                br#"{"template": "minecraft:stick", "max_stack_size": 100}"#,
            )],
        );
        for (case, components, why) in [
            (
                "bare-component",
                r#"{"max_damage": 3}"#,
                "must be namespaced",
            ),
            (
                "uppercase-path",
                r#"{"minecraft:Foo": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "uppercase-namespace",
                r#"{"Example:charge": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "empty-namespace",
                r#"{":charge": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "empty-path",
                r#"{"example:": 3}"#,
                "must be namespaced and valid",
            ),
            (
                "invalid-removal",
                r#"{"!example:Foo": {}}"#,
                "must be namespaced and valid",
            ),
            (
                "own-key",
                r#"{"minecraft:rarity": "epic"}"#,
                "set rarity instead",
            ),
            (
                "own-removal",
                r#"{"!minecraft:max_stack_size": {}}"#,
                "set max_stack_size",
            ),
            (
                "removal-value",
                r#"{"!minecraft:food": true}"#,
                "a removal's value is {}",
            ),
        ] {
            let item = format!(r#"{{"template": "minecraft:stick", "components": {components}}}"#);
            let e = refused(case, &[("data/ruby/horizon/item/a.json", item.as_bytes())]);
            assert!(e.contains(why), "{case}: {e}");
        }
        let glass = br#"{"template": "minecraft:glass", "item": false}"#;
        for (case, rest) in [
            ("bare-remainder", "bowl"),
            ("missing-remainder", "ruby:bowl"),
            ("other-mod-remainder", "jade:bowl"),
            ("itemless-remainder", "ruby:glass"),
            ("air-remainder", "minecraft:air"),
        ] {
            let item =
                format!(r#"{{"template": "minecraft:stick", "crafting_remainder": "{rest}"}}"#);
            let files: &[(&str, &[u8])] = &[
                ("data/ruby/horizon/item/a.json", item.as_bytes()),
                ("data/ruby/horizon/block/glass.json", glass),
            ];
            let e = refused(case, files);
            assert!(e.contains("crafting_remainder"), "{case}: {e}");
        }
        for (case, block, why) in [
            (
                "block-removal-value",
                r#"{"template": "minecraft:stone", "components": {"!minecraft:lore": []}}"#,
                "a removal's value is {}",
            ),
            (
                "block-stack-zero",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": 0}}"#,
                "integer 1 to 99",
            ),
            (
                "block-stack-100",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": 100}}"#,
                "integer 1 to 99",
            ),
            (
                "block-stack-string",
                r#"{"template": "minecraft:stone", "components": {"minecraft:max_stack_size": "16"}}"#,
                "integer 1 to 99",
            ),
            (
                "block-rarity",
                r#"{"template": "minecraft:stone", "components": {"minecraft:rarity": "bogus"}}"#,
                "common, uncommon",
            ),
            (
                "block-without-item",
                r#"{"template": "minecraft:glass", "item": false,
                    "components": {"minecraft:max_stack_size": 16}}"#,
                "item is false",
            ),
        ] {
            let e = refused(
                case,
                &[("data/ruby/horizon/block/a.json", block.as_bytes())],
            );
            assert!(e.contains(why), "{case}: {e}");
        }
    }

    #[test]
    fn listed_namespaces_hold_content() {
        let manifest = format!("{MANIFEST_TEXT}namespaces = [\"kaisyn\"]\n");
        let files: &[(&str, &[u8])] = &[
            ("data/kaisyn/worldgen/template_pool/a.json", b"{}"),
            ("data/kaisyn/structure/a.nbt", b"nbt"),
            ("assets/kaisyn/lang/en_us.json", b"{}"),
        ];
        let content = hmod("listed", &manifest, files).unwrap();
        assert_eq!(content.files.len(), 3);
        assert_eq!(content.namespaces, ["ruby", "kaisyn"]);
        // Declarations stay in the id's namespace.
        let decl: &[(&str, &[u8])] = &[("data/kaisyn/horizon/block/a.json", BLOCK)];
        let e = hmod("listed-decl", &manifest, decl).unwrap_err();
        assert!(e.contains("declared in data/ruby/horizon/"), "{e}");
        // A namespace it does not list is still outside.
        let other: &[(&str, &[u8])] = &[("data/other/structure/a.nbt", b"nbt")];
        assert!(hmod("listed-other", &manifest, other).is_err());
    }

    #[test]
    fn mods_sharing_a_namespace_clash() {
        let t_and_t = ["t_and_t".to_owned(), "kaisyn".to_owned()];
        let kaisyn = ["kaisyn".to_owned()];
        let ruby = ["ruby".to_owned(), "gems".to_owned()];
        assert_eq!(
            clash(("t_and_t", &t_and_t), ("kaisyn", &kaisyn)).as_deref(),
            Some("mods t_and_t and kaisyn both use the namespace kaisyn; turn one of them off")
        );
        assert_eq!(clash(("t_and_t", &t_and_t), ("ruby", &ruby)), None);
    }

    #[test]
    fn content_needs_the_registries_capability() {
        let code_only = MANIFEST_TEXT
            .replace("api = \"0.2\"", "api = \"0.1\"")
            .replace("capabilities = [\"registries\"]", "capabilities = []");
        // No code and no registries: nothing to load.
        let e = hmod("empty", &code_only, &[]).unwrap_err();
        assert!(e.contains("mod.wasm: missing"), "{e}");
        // Registries on API 0.1.
        let old = MANIFEST_TEXT.replace("api = \"0.2\"", "api = \"0.1\"");
        assert!(Manifest::parse(&old).unwrap_err().contains("needs api"));
        // Content without the capability, in a mod with code.
        let dir = scratch("no-cap");
        let path = dir.join("m.hmod");
        let files: &[(&str, &[u8])] = &[("assets/ruby/lang/en_us.json", b"{}")];
        std::fs::write(
            &path,
            Package::write_files(&code_only, Some(b"\0asm"), files).unwrap(),
        )
        .unwrap();
        let package = Package::read(&path).unwrap();
        let e = Content::read(&path, &package.manifest).err().unwrap();
        assert!(e.contains("not capability registries"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_are_plain() {
        assert!(check_path("assets/ruby/textures/block/ruby_block.png").is_ok());
        assert!(check_path("data/ruby/loot_table/blocks/a.json").is_ok());
        for bad in [
            "assets/ruby",
            "assets/../x/y",
            "assets/ruby//a",
            "assets/Ruby/a",
            "assets/ruby/a b",
            "/assets/ruby/a",
            "other/ruby/a",
            "assets\\ruby\\a",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }
}
