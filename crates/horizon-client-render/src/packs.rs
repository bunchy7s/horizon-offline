//! Resource packs (PackRepository / FolderRepositorySource): discovery of
//! folder and zip packs in `resourcepacks/` plus the built-in Programmer Art
//! and High Contrast packs, `pack.mcmeta` metadata and format compatibility,
//! and an overlay resource root that layers the enabled packs over the
//! vanilla resources (later packs win), so every loader sees one tree.
mod cache;
use crate::text::Style;
use crate::{text::Text, zip::Zip};
pub use horizon_client_core::pattern;
use pattern::Pattern;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

/// SharedConstants resource pack format of 26.3 (major, minor).
pub const RESOURCE_FORMAT: (i64, i64) = horizon_core::version::RESOURCE_PACK_FORMAT;

#[derive(Clone, Debug)]
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}
#[derive(Clone, Debug)]
pub struct Pack {
    /// `file/<name>` for user packs, the bare name for built-in ones.
    pub id: String,
    pub source: Source,
    pub title: Text,
    pub description: Text,
    /// The description with its legacy codes, and its root style (the pack screen's).
    pub raw_description: Text,
    pub description_style: Style,
    pub compatible: bool,
    pub compatibility: crate::pack_screen::Compat,
    /// Overlay directories active for this game version, in `pack.mcmeta` order.
    pub overlays: Vec<String>,
    /// `filter.block` entries (namespace and path patterns) hiding resources
    /// of the packs below this one.
    pub filter: Vec<Filter>,
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    namespace: Option<Pattern>,
    path: Option<Pattern>,
}
impl Filter {
    fn blocks(&self, file: &str) -> bool {
        let Some(rest) = file.strip_prefix("assets/") else {
            return false;
        };
        let Some((namespace, path)) = rest.split_once('/') else {
            return false;
        };
        self.namespace
            .as_ref()
            .is_none_or(|p| p.is_match(namespace))
            && self.path.as_ref().is_none_or(|p| p.is_match(path))
    }
}
/// A text component from JSON (strings, `text`, `translate`, arrays, `extra`
/// and the style fields), legacy `§` codes resolved as the font does.
pub fn component(v: &serde_json::Value) -> Text {
    component_root(v).0.resolve_legacy()
}
/// The component, its legacy codes left in place (each span is a
/// component's contents and style), and its root style (`Component.getStyle`).
pub fn component_root(v: &serde_json::Value) -> (Text, Style) {
    let mut out = Text::empty();
    let root = build(v, &Style::default(), &mut out);
    (out, root)
}
fn style_of(o: &serde_json::Map<String, serde_json::Value>) -> Style {
    let flag = |k: &str| o.get(k).and_then(|b| b.as_bool());
    Style {
        color: o.get("color").and_then(|c| c.as_str()).and_then(|c| {
            crate::text::named_color(c).or_else(|| {
                c.strip_prefix('#')
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
            })
        }),
        bold: flag("bold"),
        italic: flag("italic"),
        underlined: flag("underlined"),
        strikethrough: flag("strikethrough"),
        obfuscated: flag("obfuscated"),
        font: o.get("font").and_then(crate::text::font_id),
        ..Style::default()
    }
}
/// Appends `v` under `parent`'s style; returns the component's own style.
fn build(v: &serde_json::Value, parent: &Style, out: &mut Text) -> Style {
    match v {
        serde_json::Value::Array(a) => {
            let Some(first) = a.first() else {
                return Style::default();
            };
            let own = build(first, parent, out);
            let mut style = own.clone();
            style.inherit(parent);
            for e in &a[1..] {
                build(e, &style, out);
            }
            own
        }
        serde_json::Value::Object(o) => {
            let own = style_of(o);
            let mut style = own.clone();
            style.inherit(parent);
            if let Some(s) = o.get("text").and_then(|s| s.as_str()) {
                out.push(Text::styled(s, style.clone()));
            } else if let Some(k) = o.get("translate").and_then(|s| s.as_str()) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(component).collect())
                    .unwrap_or_default();
                out.push(Text::translate(k, &args).with_style(&style));
            }
            match o.get("extra") {
                Some(serde_json::Value::Array(extra)) => {
                    for e in extra {
                        build(e, &style, out);
                    }
                }
                Some(e) => {
                    build(e, &style, out);
                }
                None => {}
            }
            own
        }
        serde_json::Value::String(s) => {
            out.push(Text::styled(s.as_str(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Number(n) => {
            out.push(Text::styled(n.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Bool(b) => {
            out.push(Text::styled(b.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Null => Style::default(),
    }
}
fn format_of(v: &serde_json::Value) -> Option<(i64, i64)> {
    match v {
        serde_json::Value::Number(n) => Some((n.as_i64()?, 0)),
        serde_json::Value::Array(a) => Some((
            a.first()?.as_i64()?,
            a.get(1).and_then(|m| m.as_i64()).unwrap_or(0),
        )),
        _ => None,
    }
}
/// PackCompatibility: inside [min_format, max_format] (or the legacy
/// `pack_format`/`supported_formats`).
pub fn compatible(meta: &serde_json::Value) -> bool {
    let pack = &meta["pack"];
    let current = RESOURCE_FORMAT;
    if let (Some(min), Some(max)) = (
        format_of(&pack["min_format"]),
        format_of(&pack["max_format"]),
    ) {
        return min <= current
            && current <= (max.0, max.1.max(if max.1 == 0 { i64::MAX } else { max.1 }));
    }
    if let Some(range) = pack["supported_formats"].as_array()
        && let (Some(lo), Some(hi)) = (
            range.first().and_then(|v| v.as_i64()),
            range.get(1).and_then(|v| v.as_i64()),
        )
    {
        return lo <= current.0 && current.0 <= hi;
    }
    format_of(&pack["pack_format"]).is_some_and(|f| f.0 == current.0)
}
/// `PackCompatibility.forVersion`: too old, too new, compatible, or unknown
/// when the metadata names no format.
pub fn compatibility(meta: &serde_json::Value) -> crate::pack_screen::Compat {
    compatibility_for(meta, RESOURCE_FORMAT)
}
/// `compatibility` against `current`, the resource or the data pack format.
pub fn compatibility_for(
    meta: &serde_json::Value,
    current: (i64, i64),
) -> crate::pack_screen::Compat {
    use crate::pack_screen::Compat;
    let pack = &meta["pack"];
    let range = if let (Some(min), Some(max)) = (
        format_of(&pack["min_format"]),
        format_of(&pack["max_format"]),
    ) {
        Some((min, (max.0, if max.1 == 0 { i64::MAX } else { max.1 })))
    } else if let Some(r) = pack["supported_formats"].as_array()
        && let (Some(lo), Some(hi)) = (
            r.first().and_then(|v| v.as_i64()),
            r.get(1).and_then(|v| v.as_i64()),
        )
    {
        Some(((lo, 0), (hi, i64::MAX)))
    } else {
        format_of(&pack["pack_format"]).map(|f| ((f.0, 0), (f.0, i64::MAX)))
    };
    match range {
        None => Compat::Unknown,
        Some((_, max)) if max < current => Compat::TooOld,
        Some((min, _)) if current < min => Compat::TooNew,
        _ => Compat::Compatible,
    }
}
/// Whether an `overlays.entries[]` item applies to the current format
/// (OverlayMetadataSection: `formats` as an int, `[min, max]` or
/// `{min_inclusive, max_inclusive}`, or `min_format`/`max_format`).
fn overlay_applies(entry: &serde_json::Value) -> bool {
    let current = RESOURCE_FORMAT;
    if let (Some(min), Some(max)) = (
        format_of(&entry["min_format"]),
        format_of(&entry["max_format"]),
    ) {
        return min <= current && current <= (max.0, if max.1 == 0 { i64::MAX } else { max.1 });
    }
    let formats = &entry["formats"];
    if let Some(n) = formats.as_i64() {
        return n == current.0;
    }
    if let Some(a) = formats.as_array() {
        return match (
            a.first().and_then(|v| v.as_i64()),
            a.get(1).and_then(|v| v.as_i64()),
        ) {
            (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
            _ => false,
        };
    }
    match (
        formats["min_inclusive"].as_i64(),
        formats["max_inclusive"].as_i64(),
    ) {
        (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
        _ => false,
    }
}
fn overlays_of(meta: &serde_json::Value) -> Vec<String> {
    meta["overlays"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| overlay_applies(e))
        .filter_map(|e| e["directory"].as_str().map(str::to_owned))
        .collect()
}
fn filter_of(meta: &serde_json::Value) -> Vec<Filter> {
    meta["filter"]["block"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| Filter {
            namespace: b["namespace"].as_str().and_then(Pattern::new),
            path: b["path"].as_str().and_then(Pattern::new),
        })
        .collect()
}
impl Pack {
    /// The pack's `pack.png` (`PackResources.getRootResource`), for its icon.
    pub fn icon_png(&self) -> Option<Vec<u8>> {
        match &self.source {
            Source::Dir(d) => fs::read(d.join("pack.png")).ok(),
            Source::Zip(z) => Zip::open(z).ok()?.read("pack.png").ok(),
        }
    }
}
fn read_meta(source: &Source) -> Option<serde_json::Value> {
    let bytes = match source {
        Source::Dir(d) => fs::read(d.join("pack.mcmeta")).ok()?,
        Source::Zip(z) => Zip::open(z).ok()?.read("pack.mcmeta").ok()?,
    };
    serde_json::from_slice(&bytes).ok()
}
fn pack(id: String, title: Text, source: Source) -> Option<Pack> {
    pack_for(id, title, source, RESOURCE_FORMAT)
}
/// A pack whose compatibility is against `format`.
fn pack_for(id: String, title: Text, source: Source, format: (i64, i64)) -> Option<Pack> {
    let meta = read_meta(&source)?;
    let (raw_description, description_style) = component_root(&meta["pack"]["description"]);
    Some(Pack {
        id,
        title,
        description: raw_description.resolve_legacy(),
        raw_description,
        description_style,
        compatible: compatible(&meta),
        compatibility: compatibility_for(&meta, format),
        overlays: overlays_of(&meta),
        filter: filter_of(&meta),
        source,
    })
}
/// Built-in and `resourcepacks/` packs (vanilla itself excluded), sorted like
/// the pack screen (built-ins first, then by name).
pub fn discover(resources: &Path, packs_dir: &Path) -> Vec<Pack> {
    let mut out = Vec::new();
    for (id, key) in [
        ("programmer_art", "resourcePack.programmer_art.name"),
        ("high_contrast", "resourcePack.high_contrast.name"),
    ] {
        let z = resources.join(format!("assets/minecraft/resourcepacks/{id}.zip"));
        if z.exists()
            && let Some(p) = pack(id.into(), Text::translate(key, &[]), Source::Zip(z))
        {
            out.push(p);
        }
    }
    out.extend(folder(packs_dir, RESOURCE_FORMAT));
    out
}
/// The packs (folders and zips) in `dir`, by name, as `FolderRepositorySource`
/// finds them; their compatibility is against `format`.
pub fn folder(dir: &Path, format: (i64, i64)) -> Vec<Pack> {
    let mut user: Vec<Pack> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let source = if path.is_dir() {
                Source::Dir(path)
            } else if name.ends_with(".zip") {
                Source::Zip(path)
            } else {
                return None;
            };
            pack_for(format!("file/{name}"), Text::literal(name), source, format)
        })
        .collect();
    user.sort_by(|a, b| a.id.cmp(&b.id));
    user
}
/// A file a pack provides: its path (folder packs) and its name inside the pack.
type Provided = (Option<PathBuf>, String);
/// Files a pack provides under `assets/`, keyed by their resource name; active
/// overlay directories are applied after the base, in order, later wins.
fn pack_files(pack: &Pack) -> io::Result<BTreeMap<String, Provided>> {
    let mut out = BTreeMap::new();
    let roots = std::iter::once(String::new())
        .chain(pack.overlays.iter().map(|d| format!("{d}/")))
        .collect::<Vec<_>>();
    match &pack.source {
        Source::Dir(d) => {
            for root in &roots {
                let base = d.join(root.trim_end_matches('/'));
                let mut stack = vec![base.join("assets")];
                while let Some(dir) = stack.pop() {
                    let Ok(entries) = fs::read_dir(&dir) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if let Ok(rel) = p.strip_prefix(&base) {
                            let rel = rel.to_string_lossy().replace('\\', "/");
                            out.insert(rel.clone(), (Some(p.clone()), format!("{root}{rel}")));
                        }
                    }
                }
            }
        }
        Source::Zip(z) => {
            let zip = Zip::open(z)?;
            for root in &roots {
                let prefix = format!("{root}assets/");
                for name in zip.names() {
                    if let Some(rest) = name.strip_prefix(&prefix) {
                        out.insert(format!("assets/{rest}"), (None, name.to_owned()));
                    }
                }
            }
        }
    }
    Ok(out)
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    // Relative sources are relative to the process, not the overlay directory.
    std::os::unix::fs::symlink(std::path::absolute(target)?, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    if target.is_dir() {
        Ok(())
    } else {
        // The mirror only reads these files, and unix links them: a hardlink gives
        // the same view without writing the 639 MB prepared root again, which was
        // 33 s of a cold Windows launch with a pack on. Another volume (or a
        // filesystem without hardlinks) still copies.
        fs::hard_link(target, at).or_else(|_| fs::copy(target, at).map(|_| ()))
    }
}
/// A directory junction, which Windows creates without privileges (a directory symlink needs
/// developer mode). The mirror falls back to copying the tree when it fails.
#[cfg(windows)]
fn junction(target: &Path, at: &Path) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    // cmd parses its own line: Rust quotes only paths with spaces, so `&` or `^` in an
    // unquoted path would split or change the command. A Windows path cannot hold `"`.
    let quoted = |path: &Path| -> io::Result<std::ffi::OsString> {
        let mut quoted = std::ffi::OsString::from("\"");
        quoted.push(std::path::absolute(path)?);
        quoted.push("\"");
        Ok(quoted)
    };
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .raw_arg(quoted(at)?)
        .raw_arg(quoted(target)?)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| io::Error::other("mklink /J failed"))
}
#[cfg(not(windows))]
fn junction(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
/// Build `out` as `vanilla` with the packs' files layered on top (packs in
/// load order, last wins). Untouched directories are links into vanilla.
pub fn build_overlay(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    cache::build(vanilla, packs, out)
}

fn build_overlay_uncached(
    vanilla: &Path,
    packs: &[Pack],
    out: &Path,
    dependencies: &mut cache::Dependencies,
) -> io::Result<()> {
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    // Final owner of every overridden file.
    let mut files: BTreeMap<String, (usize, Provided)> = BTreeMap::new();
    // Every pack's copy of the files vanilla reads as a resource stack.
    let mut stacks: BTreeMap<String, Vec<(usize, Provided)>> = BTreeMap::new();
    for (i, p) in packs.iter().enumerate() {
        // A pack's filter hides what the packs below it provide.
        files.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        stacks.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        for (name, provided) in pack_files(p)? {
            if merge_kind(&name).is_some() {
                stacks
                    .entry(name.clone())
                    .or_default()
                    .push((i, provided.clone()));
            }
            files.insert(name, (i, provided));
        }
    }
    // FallbackResourceManager: a file's `.mcmeta` counts only from the pack that
    // supplies the file or one above it; vanilla's own sits below every pack.
    let hidden: BTreeSet<String> = files
        .iter()
        .filter(|(name, _)| !name.ends_with(".mcmeta"))
        .filter_map(|(name, (owner, _))| {
            let meta = format!("{name}.mcmeta");
            match files.get(&meta) {
                Some((meta_owner, _)) if meta_owner >= owner => None,
                _ => Some(meta),
            }
        })
        .collect();
    for name in &hidden {
        files.remove(name);
    }
    // Vanilla sits below every pack, so filters hide its files too.
    let filters: Vec<&Filter> = packs.iter().flat_map(|p| &p.filter).collect();
    let touched_dirs: BTreeSet<String> = files
        .keys()
        .flat_map(|f| {
            let parts: Vec<&str> = f.split('/').collect();
            (1..parts.len()).map(move |n| parts[..n].join("/"))
        })
        .collect();
    let zips: Vec<Option<Zip>> = packs
        .iter()
        .map(|p| {
            if let Source::Zip(z) = &p.source {
                Zip::open(z).ok()
            } else {
                None
            }
        })
        .collect();
    // Stacked files merge every layer, vanilla's first (unless filtered).
    let mut merged: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (name, stack) in &stacks {
        let mut layers = Vec::new();
        dependencies.file(&vanilla.join(name));
        if !filters.iter().any(|f| f.blocks(name))
            && let Ok(bytes) = fs::read(vanilla.join(name))
        {
            layers.push(bytes);
        }
        for (owner, (path, inner)) in stack {
            let bytes = match path {
                Some(p) => fs::read(p).ok(),
                None => zips
                    .get(*owner)
                    .and_then(Option::as_ref)
                    .and_then(|z| z.read(inner).ok()),
            };
            layers.extend(bytes);
        }
        if let Some(kind) = merge_kind(name)
            && let Some(bytes) = merge(kind, &layers)
        {
            merged.insert(name.clone(), bytes);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn mirror(
        rel: &str,
        vanilla: &Path,
        out: &Path,
        touched: &BTreeSet<String>,
        hidden: &BTreeSet<String>,
        files: &BTreeMap<String, (usize, Provided)>,
        merged: &BTreeMap<String, Vec<u8>>,
        filters: &[&Filter],
        zips: &[Option<Zip>],
        dependencies: &mut cache::Dependencies,
    ) -> io::Result<()> {
        let src = if rel.is_empty() {
            vanilla.to_path_buf()
        } else {
            vanilla.join(rel)
        };
        let dst = if rel.is_empty() {
            out.to_path_buf()
        } else {
            out.join(rel)
        };
        fs::create_dir_all(&dst)?;
        dependencies.directory(&src);
        let mut names: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = fs::read_dir(&src) {
            names.extend(
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            );
        }
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        for f in files.keys().filter_map(|f| f.strip_prefix(&prefix)) {
            names.insert(f.split('/').next().unwrap_or("").to_owned());
        }
        for name in names {
            let child = format!("{prefix}{name}");
            if hidden.contains(&child) {
                continue;
            }
            if let Some(bytes) = merged.get(&child) {
                fs::write(out.join(&child), bytes)?;
            } else if let Some((owner, (path, inner))) = files.get(&child) {
                let target = out.join(&child);
                match path {
                    Some(p) => link(p, &target)?,
                    None => {
                        if let Some(Some(z)) = zips.get(*owner) {
                            // One unreadable entry costs that file, not the
                            // pack: an error here failed the build and the
                            // reload dropped every server pack in the stack.
                            match z.read(inner) {
                                Ok(bytes) => fs::write(&target, bytes)?,
                                Err(e) => log::warn!(
                                    "Horizon: skipping unreadable pack entry {inner}: {e}"
                                ),
                            }
                        }
                    }
                }
            } else if filters.is_empty()
                && !touched.contains(&child)
                && vanilla.join(&child).is_dir()
                && junction(&vanilla.join(&child), &out.join(&child)).is_ok()
            {
                // Linked whole, like the unix symlink: no file in it is visited or copied.
            } else if touched.contains(&child)
                // Without a junction Windows cannot link a directory the way unix does (`link`
                // above), so an untouched one has to be mirrored: skipping it hid textures/font
                // and the title panorama from the overlay, and a font json that names those
                // sheets then failed to load (#1688).
                || (vanilla.join(&child).is_dir() && (cfg!(not(unix)) || !filters.is_empty()))
            {
                mirror(
                    &child,
                    vanilla,
                    out,
                    touched,
                    hidden,
                    files,
                    merged,
                    filters,
                    zips,
                    dependencies,
                )?;
            } else if filters.iter().any(|f| f.blocks(&child)) {
                // Hidden by a pack's filter.
            } else {
                #[cfg(not(unix))]
                if vanilla.join(&child).is_file() {
                    dependencies.file(&vanilla.join(&child));
                }
                link(&vanilla.join(&child), &out.join(&child))?;
            }
        }
        Ok(())
    }
    mirror(
        "",
        vanilla,
        out,
        &touched_dirs,
        &hidden,
        &files,
        &merged,
        &filters,
        &zips,
        dependencies,
    )
}
/// Files vanilla loads from every pack in turn (`getResourceStack`) rather
/// than from the top pack only.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Merge {
    /// `ClientLanguage.appendFrom`: later packs override keys.
    Lang,
    /// `SoundManager.Preparations.handleRegistration`: sounds add up unless
    /// an event says `replace`.
    Sounds,
    /// `FontManager.loadResourceStack`: the top pack's providers come first.
    Font,
    /// `SpriteSourceList.load`: sources from the bottom pack up.
    Atlas,
}
fn merge_kind(name: &str) -> Option<Merge> {
    let (_, rest) = name.strip_prefix("assets/")?.split_once('/')?;
    if rest == "sounds.json" {
        return Some(Merge::Sounds);
    }
    let dir = rest.strip_suffix(".json")?.split_once('/')?.0;
    match dir {
        "lang" => Some(Merge::Lang),
        "font" => Some(Merge::Font),
        "atlases" => Some(Merge::Atlas),
        _ => None,
    }
}
/// One file with the effect of loading `layers` (bottom first) as vanilla
/// does. Layers that do not parse are skipped, as vanilla logs and skips them.
fn merge(kind: Merge, layers: &[Vec<u8>]) -> Option<Vec<u8>> {
    use serde_json::{Map, Value};
    let parsed: Vec<Map<String, Value>> = layers
        .iter()
        // SoundManager skips a sounds.json Gson rejects, whole.
        .filter(|b| kind != Merge::Sounds || crate::sound_events::is_valid_file(b))
        .filter_map(|b| match serde_json::from_slice(b) {
            Ok(Value::Object(m)) => Some(m),
            _ => None,
        })
        .collect();
    let list = |key: &str, layers: &mut dyn Iterator<Item = &Map<String, Value>>| {
        let items: Vec<Value> = layers
            .filter_map(|l| l.get(key).and_then(Value::as_array))
            .flatten()
            .cloned()
            .collect();
        Value::Object(Map::from_iter([(key.to_owned(), Value::Array(items))]))
    };
    let out = match kind {
        Merge::Lang => Value::Object(parsed.into_iter().flatten().collect()),
        Merge::Sounds => {
            let mut events = Map::new();
            for (event, def) in parsed.into_iter().flatten() {
                if !events.contains_key(&event)
                    || crate::sound_events::json_bool(&def["replace"], false)
                {
                    let mut fresh = Map::new();
                    if let Some(subtitle) = def.get("subtitle") {
                        fresh.insert("subtitle".into(), subtitle.clone());
                    }
                    fresh.insert("sounds".into(), Value::Array(Vec::new()));
                    events.insert(event.clone(), Value::Object(fresh));
                }
                if let (Some(Value::Array(sounds)), Some(Value::Array(more))) = (
                    events.get_mut(&event).and_then(|e| e.get_mut("sounds")),
                    def.get("sounds"),
                ) {
                    sounds.extend(more.iter().cloned());
                }
            }
            Value::Object(events)
        }
        Merge::Font => list("providers", &mut parsed.iter().rev()),
        Merge::Atlas => list("sources", &mut parsed.iter()),
    };
    serde_json::to_vec(&out).ok()
}
/// `DownloadedPackSource.loadRequestedPacks`: a server's downloaded pack,
/// "World Specific Resources"; None when its `pack.mcmeta` can't be read. Its
/// id names the file, which is named after its hash.
pub fn server_pack(id: u128, path: &Path) -> Option<Pack> {
    let file = path.file_name()?.to_string_lossy();
    pack(
        format!("server/{id:032x}/{file}"),
        Text::translate("resourcePack.server.name", &[]),
        Source::Zip(path.to_path_buf()),
    )
}
/// The packs to apply for an options.txt `resourcePacks` list (bottom first).
pub fn enabled(all: &[Pack], ids: &[String]) -> Vec<Pack> {
    ids.iter()
        .filter(|id| *id != "vanilla")
        .filter_map(|id| all.iter().find(|p| &p.id == id).cloned())
        .collect()
}
/// Resource root for a pack selection: vanilla itself when no pack is on.
pub fn resource_root(vanilla: &Path, packs: &[Pack], cache: &Path) -> io::Result<PathBuf> {
    if packs.is_empty() {
        return Ok(vanilla.to_path_buf());
    }
    let key: String = packs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let out = cache.join(format!("{hash:016x}"));
    build_overlay(vanilla, packs, &out)?;
    Ok(out)
}

/// `Pack.Position`: where a pack goes when it is added to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Top,
    Bottom,
}
/// A pack as the selection sees it (`PackSelectionConfig` and its
/// compatibility).
#[derive(Clone, Debug, PartialEq)]
pub struct Selectable {
    pub id: String,
    pub required: bool,
    pub position: Position,
    pub fixed: bool,
    pub compatible: bool,
}
impl Selectable {
    /// A pack from `resourcepacks/` or a built-in optional one: not
    /// required, added on top, free to move (`FolderRepositorySource`).
    pub fn optional(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: false,
            position: Position::Top,
            fixed: false,
            compatible,
        }
    }
    /// The vanilla pack: required, at the bottom (`BuiltInPackSource`).
    pub fn vanilla() -> Self {
        Self {
            id: "vanilla".into(),
            required: true,
            position: Position::Bottom,
            fixed: false,
            compatible: true,
        }
    }
}

/// `Pack.Position.insert`: below or above the packs fixed at that end.
fn insert(list: &mut Vec<Selectable>, pack: Selectable, reverse: bool) {
    let own = pack.position;
    let side = match (own, reverse) {
        (Position::Top, false) | (Position::Bottom, true) => Position::Top,
        _ => Position::Bottom,
    };
    let pinned = |p: &Selectable| p.fixed && p.position == own;
    if side == Position::Bottom {
        let i = list.iter().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    } else {
        let i = list.len() - list.iter().rev().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    }
}

/// `PackSelectionModel` over a `PackRepository`: the packs not selected
/// (in id order) and the selected ones, top first, as the pack screen's two
/// lists show them.
#[derive(Clone, Debug, Default)]
pub struct SelectionModel {
    available: Vec<Selectable>,
    pub unselected: Vec<Selectable>,
    pub selected: Vec<Selectable>,
}
impl SelectionModel {
    /// `available` in any order; `selected` as options.txt lists them,
    /// bottom first.
    pub fn new(mut available: Vec<Selectable>, selected: &[String]) -> Self {
        // PackRepository.discoverAvailable: every source's packs go into one
        // TreeMap by id (String.compareTo), so `file/` packs sort among the
        // built-in ones.
        available.sort_by(|a, b| a.id.encode_utf16().cmp(b.id.encode_utf16()));
        available.dedup_by(|a, b| a.id == b.id);
        let mut m = Self {
            available,
            ..Self::default()
        };
        let mut chosen = m.rebuild(selected);
        chosen.reverse();
        m.unselected = m
            .available
            .iter()
            .filter(|p| !chosen.iter().any(|c| c.id == p.id))
            .cloned()
            .collect();
        m.selected = chosen;
        m
    }
    /// `PackRepository.rebuildSelected`: the named packs that exist, and
    /// every required one.
    fn rebuild(&self, ids: &[String]) -> Vec<Selectable> {
        let mut out: Vec<Selectable> = ids
            .iter()
            .filter_map(|id| self.available.iter().find(|p| p.id == *id).cloned())
            .collect();
        for p in &self.available {
            if p.required && !out.iter().any(|o| o.id == p.id) {
                insert(&mut out, p.clone(), false);
            }
        }
        out
    }
    fn list(&self, selected: bool) -> &Vec<Selectable> {
        if selected {
            &self.selected
        } else {
            &self.unselected
        }
    }
    pub fn can_select(&self, selected: bool, _: usize) -> bool {
        !selected
    }
    pub fn can_unselect(&self, selected: bool, i: usize) -> bool {
        selected && !self.selected[i].required
    }
    pub fn can_move_up(&self, selected: bool, i: usize) -> bool {
        i > 0 && !self.list(selected)[i - 1].fixed
    }
    pub fn can_move_down(&self, selected: bool, i: usize) -> bool {
        let list = self.list(selected);
        i + 1 < list.len() && !list[i + 1].fixed
    }
    /// `EntryBase.toggleSelection`: to the other list, at its default
    /// position (from the other end).
    fn toggle(&mut self, selected: bool, i: usize) {
        let (from, to) = if selected {
            (&mut self.selected, &mut self.unselected)
        } else {
            (&mut self.unselected, &mut self.selected)
        };
        let p = from.remove(i);
        insert(to, p, true);
    }
    /// `PackEntry`'s activation (a click on the icon, or the select key):
    /// select, an incompatible pack only once its confirmation was
    /// accepted, or else unselect. True when the lists changed.
    pub fn activate(&mut self, selected: bool, i: usize, confirmed: bool) -> bool {
        if self.can_select(selected, i) {
            if self.unselected[i].compatible || confirmed {
                self.toggle(false, i);
                return true;
            }
            false
        } else if self.can_unselect(selected, i) {
            self.toggle(true, i);
            true
        } else {
            false
        }
    }
    /// `moveUp` / `moveDown` when allowed.
    pub fn move_by(&mut self, selected: bool, i: usize, up: bool) -> bool {
        let ok = if up {
            self.can_move_up(selected, i)
        } else {
            self.can_move_down(selected, i)
        };
        if ok {
            let list = if selected {
                &mut self.selected
            } else {
                &mut self.unselected
            };
            let p = list.remove(i);
            list.insert(if up { i - 1 } else { i + 1 }, p);
        }
        ok
    }
    /// `commit`: the repository's selection, bottom first, as options.txt
    /// stores it.
    pub fn commit(&self) -> Vec<String> {
        let ids: Vec<String> = self.selected.iter().rev().map(|p| p.id.clone()).collect();
        self.rebuild(&ids).into_iter().map(|p| p.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_ranges() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert!(compatible(&v(
            r#"{"pack":{"min_format":97,"max_format":98}}"#
        )));
        assert!(compatible(&v(
            r#"{"pack":{"min_format":[97,0],"max_format":[97,1]}}"#
        )));
        assert!(!compatible(&v(
            r#"{"pack":{"min_format":[97,2],"max_format":99}}"#
        )));
        assert!(compatible(&v(r#"{"pack":{"pack_format":97}}"#)));
        assert!(!compatible(&v(r#"{"pack":{"pack_format":34}}"#)));
        assert_eq!(
            component(&v(r#"["a",{"text":"b","color":"red"}]"#)).plain(),
            "ab"
        );
    }
    #[test]
    fn stacked_resources_merge_like_vanilla() {
        let json = |v: &str| v.as_bytes().to_vec();
        let parse = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        // ClientLanguage: later packs override keys, the rest stays.
        let lang = merge(
            Merge::Lang,
            &[
                json(r#"{"a":"vanilla a","b":"vanilla b"}"#),
                json(r#"{"b":"pack b"}"#),
            ],
        );
        assert_eq!(
            parse(lang.unwrap()),
            serde_json::json!({"a":"vanilla a","b":"pack b"})
        );
        // SoundManager: sounds add up; `replace` starts the event over.
        let sounds = merge(
            Merge::Sounds,
            &[
                json(r#"{"x":{"subtitle":"s.x","sounds":["v1"]},"y":{"sounds":["v2"]}}"#),
                json(
                    r#"{"x":{"sounds":["p1"]},"y":{"replace":true,"subtitle":"s.y","sounds":["p2"]}}"#,
                ),
                b"not json".to_vec(),
            ],
        );
        assert_eq!(
            parse(sounds.unwrap()),
            serde_json::json!({
                "x":{"subtitle":"s.x","sounds":["v1","p1"]},
                "y":{"subtitle":"s.y","sounds":["p2"]}
            })
        );
        // FontManager: the top pack's providers come first.
        let font = merge(
            Merge::Font,
            &[json(r#"{"providers":[1,2]}"#), json(r#"{"providers":[3]}"#)],
        );
        assert_eq!(
            parse(font.unwrap()),
            serde_json::json!({"providers":[3,1,2]})
        );
        // SpriteSourceList: sources from the bottom pack up.
        let atlas = merge(
            Merge::Atlas,
            &[json(r#"{"sources":[1]}"#), json(r#"{"sources":[2]}"#)],
        );
        assert_eq!(parse(atlas.unwrap()), serde_json::json!({"sources":[1,2]}));
        assert_eq!(
            merge_kind("assets/minecraft/lang/en_us.json"),
            Some(Merge::Lang)
        );
        assert_eq!(
            merge_kind("assets/minecraft/sounds.json"),
            Some(Merge::Sounds)
        );
        assert_eq!(
            merge_kind("assets/minecraft/font/default.json"),
            Some(Merge::Font)
        );
        assert_eq!(merge_kind("assets/minecraft/font/unifont.zip"), None);
        assert_eq!(
            merge_kind("assets/minecraft/atlases/blocks.json"),
            Some(Merge::Atlas)
        );
        assert_eq!(merge_kind("assets/minecraft/models/block/stone.json"), None);
    }
    #[test]
    fn overlay_layers_packs_over_vanilla() {
        // `&` would split an unquoted `cmd /C mklink` line.
        let base = std::env::temp_dir().join(format!("horizon-packs-&{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(vanilla.join("assets/minecraft/lang")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"vanilla dirt",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/lang/en_us.json"),
            br#"{"a":"vanilla","b":"vanilla"}"#,
        )
        .unwrap();
        fs::write(vanilla.join("facts.json"), b"[]").unwrap();
        // A directory no pack touches (#1688: a font json names sheets in it).
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/font")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/font/ascii.png"),
            b"font",
        )
        .unwrap();
        let pack_dir = base.join("packs/Mine");
        fs::create_dir_all(pack_dir.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"mine","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/textures/block/stone.png"),
            b"pack",
        )
        .unwrap();
        fs::create_dir_all(pack_dir.join("assets/minecraft/lang")).unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/lang/en_us.json"),
            br#"{"b":"pack"}"#,
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/Mine");
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/dirt.png")).unwrap(),
            b"vanilla dirt"
        );
        // A pack's language file adds to vanilla's instead of replacing it.
        assert_eq!(
            fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
            br#"{"a":"vanilla","b":"pack"}"#
        );
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"[]");
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/font/ascii.png")).unwrap(),
            b"font"
        );
        // Linked, not mirrored: a junction on Windows, a symlink on unix.
        assert!(
            fs::symlink_metadata(out.join("assets/minecraft/textures/font"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn built_in_and_folder_packs_give_their_pack_png() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let built_in = base.join("vanilla/assets/minecraft/resourcepacks");
        fs::create_dir_all(&built_in).unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(built_in.join("programmer_art.zip")).unwrap());
        for (name, data) in [("pack.mcmeta", &meta[..]), ("pack.png", b"dirt")] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        for (name, icon) in [("Iconless", None), ("Mine", Some(&b"mine"[..]))] {
            let dir = base.join("packs").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("pack.mcmeta"), meta).unwrap();
            if let Some(icon) = icon {
                fs::write(dir.join("pack.png"), icon).unwrap();
            }
        }
        let packs = discover(&base.join("vanilla"), &base.join("packs"));
        let icons: Vec<_> = packs
            .iter()
            .map(|p| (p.id.as_str(), p.icon_png()))
            .collect();
        assert_eq!(
            icons,
            [
                ("programmer_art", Some(b"dirt".to_vec())),
                ("file/Iconless", None),
                ("file/Mine", Some(b"mine".to_vec())),
            ]
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn overlays_and_filters() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":[90,96],"directory":"old"},
            {"formats":{"min_inclusive":97,"max_inclusive":97},"directory":"new"},
            {"min_format":97,"max_format":[97,5],"directory":"also"},
            {"formats":50,"directory":"no"}]}}"#);
        assert_eq!(overlays_of(&meta), ["new", "also"]);
        let base = std::env::temp_dir().join(format!("horizon-packs-ov-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"v",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"v",
        )
        .unwrap();
        let dir = base.join("packs/P");
        fs::create_dir_all(dir.join("new/assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(dir.join("old/assets/minecraft/textures/block")).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"p","pack_format":97},
            "overlays":{"entries":[{"formats":97,"directory":"new"},{"formats":1,"directory":"old"}]},
            "filter":{"block":[{"path":"textures/block/dirt\\.png"}]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("new/assets/minecraft/textures/block/stone.png"),
            b"overlay",
        )
        .unwrap();
        fs::write(
            dir.join("old/assets/minecraft/textures/block/stone.png"),
            b"old",
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"overlay"
        );
        assert!(
            !out.join("assets/minecraft/textures/block/dirt.png")
                .exists()
        );
        let _ = fs::remove_dir_all(base);
    }
}
