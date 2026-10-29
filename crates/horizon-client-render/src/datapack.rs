//! Data packs (`<world>/datapacks`: folders and zips with `pack.mcmeta`) and
//! the parts of them commands use: `.mcfunction` functions (with macros and
//! line continuation), function tags and predicates.
use crate::zip::Zip;
use horizon_world::{
    nbt::Tag,
    worldgen::overrides::{self, PackFile, WorldgenOverrides},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// A pack on disk.
pub struct DataPack {
    /// `file/<name>` in `/datapack` and level.dat.
    pub id: String,
    source: Source,
}
enum Source {
    Dir(PathBuf),
    Zip(Box<Zip>),
}
impl DataPack {
    /// Every file path under `prefix` (relative to the pack root).
    fn list(&self, prefix: &str) -> Vec<String> {
        match &self.source {
            Source::Zip(z) => z
                .names()
                .filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
                .map(str::to_owned)
                .collect(),
            Source::Dir(root) => {
                let mut out = Vec::new();
                walk(root, &root.join(prefix), &mut out);
                out
            }
        }
    }
    /// The files of `data/<namespace>/<kind>/` for each kind, in the order
    /// `list("data/")` gives them. A folder pack walks only those folders:
    /// the built-in pack holds about 10,000 other data files.
    fn list_kinds(&self, kinds: &[&str]) -> Vec<String> {
        debug_assert!(kinds.is_sorted(), "kinds in walk order");
        let Source::Dir(root) = &self.source else {
            return self.list("data/");
        };
        let Ok(entries) = std::fs::read_dir(root.join("data")) else {
            return Vec::new();
        };
        let mut namespaces: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        namespaces.sort();
        let mut out = Vec::new();
        for ns in namespaces {
            for kind in kinds {
                walk(root, &ns.join(kind), &mut out);
            }
        }
        out
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        match &self.source {
            Source::Zip(z) => z.read(path).ok(),
            Source::Dir(root) => std::fs::read(root.join(path)).ok(),
        }
    }
    /// The `description` of `pack.mcmeta`, as text.
    pub fn description(&self) -> String {
        let Some(bytes) = self.read("pack.mcmeta") else {
            return String::new();
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        match &v["pack"]["description"] {
            serde_json::Value::String(s) => s.clone(),
            other => other["text"].as_str().unwrap_or_default().to_owned(),
        }
    }
}
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}
impl DataPack {
    /// The built-in `vanilla` pack: the game's own data directory.
    pub fn builtin(root: &Path) -> Self {
        DataPack {
            id: "vanilla".into(),
            source: Source::Dir(root.to_path_buf()),
        }
    }
}
/// The packs in `<world>/datapacks`, sorted by name.
pub fn discover(world: &Path) -> Vec<DataPack> {
    let Ok(entries) = std::fs::read_dir(world.join("datapacks")) else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if path.is_dir() && path.join("pack.mcmeta").exists() {
            packs.push(DataPack {
                id: format!("file/{name}"),
                source: Source::Dir(path),
            });
        } else if name.ends_with(".zip")
            && let Ok(z) = Zip::open(&path)
            && z.contains("pack.mcmeta")
        {
            packs.push(DataPack {
                id: format!("file/{name}"),
                source: Source::Zip(Box::new(z)),
            });
        }
    }
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    packs
}

// ---- Create World ------------------------------------------------------------

/// Create World's Data Packs screen (`PackSelectionScreen` on vanilla's
/// `mcworld-` folder): the folder's packs (`PackSource.WORLD`) and the
/// built-in one (`resources` holds its `pack.png`), which can be unselected
/// and stays at the bottom. Horizon has no experiments, so their feature
/// packs are not offered.
pub fn screen_packs(dir: &Path, resources: &Path) -> Vec<crate::pack_screen::PackInfo> {
    use crate::{
        pack_screen::{Compat, PackInfo, Source, load_icon},
        packs::Position,
        text::{Style, Text},
    };
    let format = horizon_core::version::DATA_PACK_FORMAT;
    let mut infos: Vec<PackInfo> = crate::packs::folder(dir, format)
        .into_iter()
        .map(|p| PackInfo {
            icon: p.icon_png().as_deref().and_then(load_icon),
            id: p.id,
            title: p.title,
            description: p.raw_description,
            description_style: p.description_style,
            source: Source::World,
            compat: p.compatibility,
            required: false,
            position: Position::Top,
            fixed: false,
        })
        .collect();
    infos.push(PackInfo {
        id: "vanilla".into(),
        title: Text::translate("dataPack.vanilla.name", &[]),
        description: Text::translate("dataPack.vanilla.description", &[]),
        description_style: Style::default(),
        source: Source::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(resources.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(load_icon),
        required: false,
        position: Position::Bottom,
        fixed: false,
    });
    infos
}
/// A new empty folder for Create World's packs
/// (`Files.createTempDirectory("mcworld-")`).
pub fn temp_folder() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("mcworld-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
/// What's in a folder of packs, to notice packs added or removed while the
/// screen shows it (`PackSelectionScreen.Watcher`).
pub fn folder_stamp(dir: &Path) -> Vec<(String, Option<std::time::SystemTime>)> {
    let mut stamp: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok();
            (e.file_name().to_string_lossy().into_owned(), modified)
        })
        .collect();
    stamp.sort();
    stamp
}
/// Create World's packs into the new world before it opens
/// (`copyTempDataPackDirToNewWorld`): every pack of `dir` goes to
/// `<world>/datapacks`, and `datapacks.json` enables those `selected`
/// (lowest priority first) and disables the rest.
pub fn install(dir: &Path, world: &Path, selected: &[String]) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let target = world.join("datapacks");
    std::fs::create_dir_all(&target).map_err(|e| err(&target, e))?;
    let mut disabled = Vec::new();
    for pack in crate::packs::folder(dir, horizon_core::version::DATA_PACK_FORMAT) {
        let (crate::packs::Source::Dir(from) | crate::packs::Source::Zip(from)) = &pack.source;
        copy(from, &target.join(pack.id.trim_start_matches("file/")))?;
        if !selected.contains(&pack.id) {
            disabled.push(pack.id);
        }
    }
    if !selected.iter().any(|id| id == "vanilla") {
        disabled.push("vanilla".into());
    }
    let json = serde_json::json!({ "enabled": selected, "disabled": disabled });
    let file = world.join("datapacks.json");
    std::fs::write(&file, json.to_string()).map_err(|e| err(&file, e))
}
/// A file, or a folder and everything in it.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{} -> {}: {e}", from.display(), to.display());
    if !from.is_dir() {
        return std::fs::copy(from, to).map(drop).map_err(err);
    }
    std::fs::create_dir_all(to).map_err(err)?;
    for e in std::fs::read_dir(from).map_err(err)?.flatten() {
        copy(&e.path(), &to.join(e.file_name()))?;
    }
    Ok(())
}

// ---- functions ---------------------------------------------------------------

/// A `.mcfunction`: plain command lines and macro lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub id: String,
    pub lines: Vec<FunctionLine>,
    /// Macro parameter names in order of first use.
    pub parameters: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionLine {
    Command(String),
    /// A `$` line: text with `$(name)` placeholders.
    Macro(String),
}
impl Function {
    pub fn is_macro(&self) -> bool {
        !self.parameters.is_empty()
    }
    /// `MacroFunction.instantiate`: the command lines with the arguments
    /// substituted. The error is the translation key and its arguments.
    pub fn instantiate(
        &self,
        arguments: Option<&Tag>,
    ) -> Result<Vec<String>, (String, Vec<String>)> {
        let mut values: BTreeMap<&str, String> = BTreeMap::new();
        if self.is_macro() {
            let Some(args) = arguments else {
                return Err((
                    "commands.function.error.missing_arguments".into(),
                    vec![self.id.clone()],
                ));
            };
            for p in &self.parameters {
                let Some(v) = args.get(p) else {
                    return Err((
                        "commands.function.error.missing_argument".into(),
                        vec![self.id.clone(), p.clone()],
                    ));
                };
                values.insert(p, stringify(v));
            }
        }
        Ok(self
            .lines
            .iter()
            .map(|l| match l {
                FunctionLine::Command(c) => c.clone(),
                FunctionLine::Macro(t) => substitute(t, &values),
            })
            .collect())
    }
}
/// `MacroFunction.stringify`.
fn stringify(tag: &Tag) -> String {
    match tag {
        Tag::Float(v) => decimal(f64::from(f32::from_bits(*v))),
        Tag::Double(v) => decimal(f64::from_bits(*v)),
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::String(s) => s.to_string_lossy(),
        other => crate::snbt::to_snbt(other),
    }
}
/// `DecimalFormat("#")` with up to 15 fraction digits.
fn decimal(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    let s = format!("{v:.15}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}
fn substitute(template: &str, values: &BTreeMap<&str, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("$(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find(')') {
            Some(j) => {
                if let Some(v) = values.get(&after[..j]) {
                    out.push_str(v);
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
fn macro_parameters(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("$(") {
        let after = &rest[i + 2..];
        let Some(j) = after.find(')') else {
            return Err(format!("Unterminated macro variable in '{line}'"));
        };
        let name = &after[..j];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("Invalid macro variable name '{name}' in '{line}'"));
        }
        out.push(name.to_owned());
        rest = &after[j + 1..];
    }
    Ok(out)
}
/// `CommandFunction.fromLines`: comments, blank lines, `\` continuation and
/// `$` macro lines. Command syntax is checked by the caller.
pub fn parse_function(id: &str, text: &str) -> Result<Function, String> {
    let raw: Vec<&str> = text.lines().collect();
    let mut lines = Vec::new();
    let mut parameters: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let number = i + 1;
        let mut line = raw[i].trim().to_owned();
        if line.ends_with('\\') {
            loop {
                i += 1;
                if i == raw.len() {
                    return Err("Line continuation at end of file".into());
                }
                line.pop();
                line.push_str(raw[i].trim());
                if !line.ends_with('\\') {
                    break;
                }
            }
        }
        i += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('/') {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
                .collect();
            return Err(if rest.starts_with('/') {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (if you intended to make a comment, use '#' not '//')"
                )
            } else {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (did you mean '{name}'? Do not use a preceding forwards slash.)"
                )
            });
        }
        if let Some(m) = line.strip_prefix('$') {
            for p in macro_parameters(m)? {
                if !parameters.contains(&p) {
                    parameters.push(p);
                }
            }
            lines.push(FunctionLine::Macro(m.to_owned()));
        } else {
            lines.push(FunctionLine::Command(line));
        }
    }
    Ok(Function {
        id: id.to_owned(),
        lines,
        parameters,
    })
}

/// The data of every enabled pack: functions, function tags, predicates.
#[derive(Default)]
pub struct DataStore {
    pub functions: BTreeMap<String, Function>,
    /// Function tag id -> entries (`ns:f`, `#ns:tag`), with `required` flags.
    pub function_tags: BTreeMap<String, Vec<(String, bool)>>,
    pub predicates: BTreeMap<String, serde_json::Value>,
    /// `item_modifier` files, for `/item modify`.
    pub item_modifiers: BTreeMap<String, serde_json::Value>,
    /// Functions that failed to parse (id, message), for logging.
    pub errors: Vec<(String, String)>,
}
/// `ns:path` for a file `data/<ns>/<kind>/<path>.<ext>`.
fn resource_id(file: &str, kind: &str, ext: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest
        .strip_prefix(kind)?
        .strip_prefix('/')?
        .strip_suffix(ext)?;
    Some(format!("{ns}:{path}"))
}
impl DataStore {
    pub fn load(packs: &[&DataPack]) -> Self {
        let mut s = Self::default();
        for pack in packs {
            for f in pack.list_kinds(&["function", "item_modifier", "predicate", "tags/function"]) {
                if let Some(id) = resource_id(&f, "function", ".mcfunction")
                    && let Some(bytes) = pack.read(&f)
                {
                    match parse_function(&id, &String::from_utf8_lossy(&bytes)) {
                        Ok(func) => {
                            s.functions.insert(id, func);
                        }
                        Err(e) => s.errors.push((id, e)),
                    }
                } else if let Some(id) = resource_id(&f, "tags/function", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    let entries = tag_entries(&v);
                    let slot = s.function_tags.entry(id).or_default();
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(entries);
                } else if let Some(id) = resource_id(&f, "predicate", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    s.predicates.insert(id, v);
                } else if let Some(id) = resource_id(&f, "item_modifier", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    s.item_modifiers.insert(id, v);
                }
            }
        }
        s
    }
    /// `FunctionManager.getTag`: the functions a tag expands to (nested tags
    /// included, each function once).
    pub fn tag_functions(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.expand(id, &mut out, 0);
        out
    }
    fn expand(&self, id: &str, out: &mut Vec<String>, depth: usize) {
        if depth > 16 {
            return;
        }
        let Some(entries) = self.function_tags.get(id) else {
            return;
        };
        for (e, required) in entries {
            match e.strip_prefix('#') {
                Some(inner) => self.expand(inner, out, depth + 1),
                None => {
                    if (self.functions.contains_key(e) || *required) && !out.contains(e) {
                        out.push(e.clone());
                    }
                }
            }
        }
    }
}
fn normalize(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
fn tag_entries(v: &serde_json::Value) -> Vec<(String, bool)> {
    v["values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| match e {
            serde_json::Value::String(s) => Some((hash_normalize(s), true)),
            serde_json::Value::Object(o) => Some((
                hash_normalize(o.get("id")?.as_str()?),
                o.get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
            )),
            _ => None,
        })
        .collect()
}
fn hash_normalize(s: &str) -> String {
    match s.strip_prefix('#') {
        Some(rest) => format!("#{}", normalize(rest)),
        None => normalize(s),
    }
}

/// The ids of `packs` (a world's, from `discover`) that are enabled, lowest
/// priority first: those `<world>/datapacks.json` lists as enabled, in its
/// order, then every pack it lists neither way, in name order.
pub fn enabled_ids(world: &Path, packs: &[DataPack]) -> Vec<String> {
    let saved: Option<serde_json::Value> = std::fs::read(world.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let list = |key: &str| -> Vec<String> {
        saved
            .as_ref()
            .and_then(|v| v[key].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let disabled = list("disabled");
    let mut enabled: Vec<String> = list("enabled")
        .into_iter()
        .filter(|id| packs.iter().any(|p| &p.id == id))
        .collect();
    for p in packs {
        if !enabled.contains(&p.id) && !disabled.contains(&p.id) {
            enabled.push(p.id.clone());
        }
    }
    enabled
}

// ---- worldgen ----------------------------------------------------------------

/// The worldgen entries of `packs` (lowest priority first, the built-in pack
/// left out: the prepared bundles are it) for the chunk generator, and what
/// it leaves out, for the log. `biomes` says which biomes the game has: a
/// pack's new biome is left out (the game has no colours, sounds or id for it).
pub fn worldgen_overrides(
    packs: &[&DataPack],
    biomes: impl Fn(&str) -> bool,
) -> (WorldgenOverrides, Vec<String>) {
    let mut out = WorldgenOverrides::default();
    let mut not_read: BTreeMap<String, usize> = BTreeMap::new();
    let mut warnings = Vec::new();
    for pack in packs {
        for file in pack.list_kinds(&overrides::PACK_KINDS) {
            let read = || String::from_utf8(pack.read(&file)?).ok();
            match out.add_file(&pack.id, &file, read) {
                PackFile::Read | PackFile::Other => {}
                PackFile::DimensionType { id, min_y, height } => warnings.push(format!(
                    "{}: dimension type {id} (min_y {min_y}, height {height}) is not used: \
                     the dimension keeps its vanilla build height",
                    pack.id
                )),
                PackFile::NotRead(kind) => {
                    *not_read.entry(format!("{}: {kind}", pack.id)).or_default() += 1
                }
                PackFile::Unreadable => warnings.push(format!("{}: {file}: unreadable", pack.id)),
            }
        }
    }
    warnings.extend(out.finish(biomes));
    for (kind, files) in not_read {
        warnings.push(format!("{kind}: {files} files the generator does not read"));
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_packs_override_worldgen_in_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-worldgen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join("datapacks").join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        for pack in ["a", "b", "off"] {
            write(pack, "pack.mcmeta", "{}");
            write(pack, "data/minecraft/worldgen/noise/cave.json", pack);
            write(
                pack,
                "data/minecraft/tags/worldgen/biome/is_hill.json",
                "{\"values\":[]}",
            );
        }
        write("a", "data/minecraft/worldgen/biome/new_peaks.json", "{}");
        write("a", "data/minecraft/worldgen/biome/plains.json", "{}");
        write(
            "a",
            "data/minecraft/worldgen/world_preset/normal.json",
            "{}",
        );
        write(
            "b",
            "data/minecraft/dimension_type/overworld.json",
            "{\"min_y\":-64,\"height\":2096}",
        );
        // `b` is enabled first, so `a` (enabled after it) wins.
        std::fs::write(
            dir.join("datapacks.json"),
            "{\"enabled\":[\"file/b\",\"file/a\"],\"disabled\":[\"file/off\"]}",
        )
        .unwrap();
        let packs = discover(&dir);
        let enabled = enabled_ids(&dir, &packs);
        assert_eq!(enabled, ["file/b", "file/a"]);
        let enabled: Vec<&DataPack> = enabled
            .iter()
            .map(|id| packs.iter().find(|p| &p.id == id).unwrap())
            .collect();
        let (o, warnings) = worldgen_overrides(&enabled, |b| b == "minecraft:plains");
        assert_eq!(
            o.entries("noises").collect::<Vec<_>>(),
            [("minecraft:cave", "a")]
        );
        assert_eq!(
            o.entries("biomes").collect::<Vec<_>>(),
            [("minecraft:plains", "{}")]
        );
        let tag_packs: Vec<&str> = o.tags("biome_tags").map(|(_, p, _)| p).collect();
        assert_eq!(tag_packs, ["file/b", "file/a"]);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("height 2096"));
        assert!(warnings[1].contains("minecraft:new_peaks"));
        assert!(warnings[2].contains("worldgen/world_preset"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    use horizon_world::nbt::parse_snbt;

    #[test]
    fn parses_comments_continuations_and_macros() {
        let f = parse_function(
            "a:b",
            "# a comment\n\nsay hi\n  say \\\n  there  \n$say $(who) is $(n)\n$tell $(who) $(who)\n",
        )
        .unwrap();
        assert_eq!(
            f.lines,
            vec![
                FunctionLine::Command("say hi".into()),
                FunctionLine::Command("say there".into()),
                FunctionLine::Macro("say $(who) is $(n)".into()),
                FunctionLine::Macro("tell $(who) $(who)".into()),
            ]
        );
        assert_eq!(f.parameters, ["who", "n"]);
        assert!(
            parse_function("a:b", "/say hi")
                .unwrap_err()
                .contains("did you mean 'say'")
        );
        assert!(
            parse_function("a:b", "//x")
                .unwrap_err()
                .contains("use '#'")
        );
        assert!(parse_function("a:b", "say \\").is_err());
        assert!(parse_function("a:b", "$say $(bad name)").is_err());
    }

    #[test]
    fn macros_substitute_vanilla_text() {
        let f = parse_function("a:b", "$say $(s) $(i) $(d) $(l) $(b) $(c)\nsay plain").unwrap();
        let args = parse_snbt("{s:\"hi\",i:5,d:1.5d,l:7L,b:1b,c:{x:1}}")
            .unwrap()
            .0;
        let out = f.instantiate(Some(&args)).unwrap();
        assert_eq!(out, ["say hi 5 1.5 7 1 {x:1}", "say plain"]);
        let args = parse_snbt("{s:\"x\",i:1,d:3.0d,l:1L,b:0b,c:{}}").unwrap().0;
        assert_eq!(f.instantiate(Some(&args)).unwrap()[0], "say x 1 3 1 0 {}");
        assert_eq!(
            f.instantiate(None).unwrap_err().0,
            "commands.function.error.missing_arguments"
        );
        let partial = parse_snbt("{s:\"x\"}").unwrap().0;
        assert_eq!(
            f.instantiate(Some(&partial)).unwrap_err().0,
            "commands.function.error.missing_argument"
        );
        // A plain function needs no arguments.
        let plain = parse_function("a:p", "say 1").unwrap();
        assert_eq!(plain.instantiate(None).unwrap(), ["say 1"]);
    }

    #[test]
    fn packs_provide_functions_and_tags() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/mypack");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            "{\"pack\":{\"description\":\"Hello\",\"pack_format\":1}}",
        );
        write(
            "data/mine/function/a.mcfunction",
            "say a\nfunction mine:sub/b\n",
        );
        write("data/mine/function/sub/b.mcfunction", "say b");
        write(
            "data/minecraft/tags/function/tick.json",
            "{\"values\":[\"mine:a\",\"#mine:more\",{\"id\":\"mine:gone\",\"required\":false}]}",
        );
        write(
            "data/mine/tags/function/more.json",
            "{\"values\":[\"mine:sub/b\",\"mine:a\"]}",
        );
        write(
            "data/mine/predicate/p.json",
            "{\"condition\":\"minecraft:random_chance\",\"chance\":1}",
        );
        let packs = discover(&dir);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/mypack");
        assert_eq!(packs[0].description(), "Hello");
        let store = DataStore::load(&[&packs[0]]);
        assert_eq!(store.functions.len(), 2);
        assert!(store.functions.contains_key("mine:sub/b"));
        assert_eq!(
            store.tag_functions("minecraft:tick"),
            ["mine:a", "mine:sub/b"]
        );
        assert!(store.predicates.contains_key("mine:p"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_packs_list_the_needed_kinds_in_walk_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p");
        let files = [
            "data/b/function/z.mcfunction",
            "data/b/function/a/y.mcfunction",
            "data/b/loot_table/l.json",
            "data/b/tags/block/t.json",
            "data/b/tags/function/load.json",
            "data/b/predicate/p.json",
            "data/a/item_modifier/m.json",
            "data/a/function/f.mcfunction",
            "data/a/functions/old.mcfunction",
            "data/a/recipe/r.json",
            "data/a/tags/function/tick.json",
            "data/c/worldgen/noise/n.json",
        ];
        for f in files {
            let path = pack.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
        let p = DataPack::builtin(&pack);
        let kinds = ["function", "item_modifier", "predicate", "tags/function"];
        let wanted: Vec<String> = p
            .list("data/")
            .into_iter()
            .filter(|f| {
                let rest = f.splitn(3, '/').nth(2).unwrap_or_default();
                kinds.iter().any(|k| rest.starts_with(&format!("{k}/")))
            })
            .collect();
        assert_eq!(wanted.len(), 7);
        assert_eq!(p.list_kinds(&kinds), wanted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_world_installs_its_packs() {
        let root = std::env::temp_dir().join(format!("hz-datapack-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, world) = (root.join("mcworld"), root.join("world"));
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write("fns", "pack.mcmeta", r#"{"pack":{"pack_format":121}}"#);
        write("fns", "data/x/function/f.mcfunction", "say hi");
        write("off", "pack.mcmeta", r#"{"pack":{"pack_format":121}}"#);
        std::fs::create_dir_all(&dir).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join("old.zip")).unwrap());
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, br#"{"pack":{"pack_format":48}}"#).unwrap();
        zip.finish().unwrap();
        let infos = screen_packs(&dir, &root);
        let ids: Vec<&str> = infos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip", "vanilla"]);
        let compat: Vec<_> = infos.iter().map(|p| p.compat).collect();
        use crate::pack_screen::Compat;
        assert_eq!(
            compat,
            [
                Compat::Compatible,
                Compat::Compatible,
                Compat::TooOld,
                Compat::Compatible
            ]
        );
        let selected = ["vanilla", "file/old.zip", "file/fns"].map(String::from);
        install(&dir, &world, &selected).unwrap();
        let packs = discover(&world);
        let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip"]);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(world.join("datapacks.json")).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"enabled": selected, "disabled": ["file/off"]})
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
