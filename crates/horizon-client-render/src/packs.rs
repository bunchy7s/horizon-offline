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
/// `PackFormat`: major and minor.
type Format = (i64, i64);
/// `PackFormat.lastPreMinorVersion`: the last major without minor versions,
/// for the resource pack format (64) or the data pack one (81).
fn last_pre_minor(current: Format) -> i64 {
    if current.0 == horizon_core::version::DATA_PACK_FORMAT.0 {
        81
    } else {
        64
    }
}
/// `PackFormat.fullCodec(default_minor)`: an int, or a list of 1 to 256
/// non-negative ints whose second is the minor. None: it does not parse.
fn format_codec(v: &serde_json::Value, default_minor: i64) -> Option<Format> {
    let non_negative =
        |v: &serde_json::Value| v.as_i64().filter(|n| *n >= 0 && *n <= i64::from(i32::MAX));
    match v {
        serde_json::Value::Array(a) if (1..=256).contains(&a.len()) => {
            let list: Option<Vec<i64>> = a.iter().map(non_negative).collect();
            let list = list?;
            Some((list[0], list.get(1).copied().unwrap_or(default_minor)))
        }
        serde_json::Value::Array(_) => None,
        v => Some((non_negative(v)?, default_minor)),
    }
}
/// `InclusiveRange.codec(Codec.INT)`: `[min, max]` or
/// `{min_inclusive, max_inclusive}`, min not above max.
fn int_range_codec(v: &serde_json::Value) -> Option<(i64, i64)> {
    let int = |v: &serde_json::Value| v.as_i64().filter(|n| i32::try_from(*n).is_ok());
    let (min, max) = match v {
        serde_json::Value::Array(a) if a.len() == 2 => (int(&a[0])?, int(&a[1])?),
        serde_json::Value::Object(o) => {
            (int(o.get("min_inclusive")?)?, int(o.get("max_inclusive")?)?)
        }
        _ => return None,
    };
    (min <= max).then_some((min, max))
}
/// An `optionalFieldOf`: absent is `Ok(None)`, one that does not parse an error.
fn optional<T>(
    v: &serde_json::Value,
    parse: impl Fn(&serde_json::Value) -> Option<T>,
) -> Result<Option<T>, ()> {
    if v.is_null() {
        Ok(None)
    } else {
        parse(v).map(Some).ok_or(())
    }
}
/// `PackFormat.IntermediaryFormat.validatePackFormatForRange`.
fn validate_pack_format_for_range(main_format: i64, min: i64, max: i64) -> Option<String> {
    if main_format < min || main_format > max {
        Some(format!(
            "Pack declared support for versions {min} to {max} but declared main format is {main_format}"
        ))
    } else if main_format < 15 {
        Some("Multi-version packs cannot support minimum version of less than 15, since this will leave versions in range unable to load pack.".into())
    } else {
        None
    }
}
/// `PackFormat` as `toString` prints it.
fn format_text(f: Format) -> String {
    if f.1 == i64::from(i32::MAX) {
        format!("{}.*", f.0)
    } else {
        format!("{}.{}", f.0, f.1)
    }
}
/// `PackFormat.IntermediaryFormat.validate` for a pack's `pack` section
/// (`hasPackFormatField`, no `requireOldField`): the supported range, or why
/// vanilla refuses the metadata (it then falls back to a pack of unknown
/// format).
pub fn validate_pack_range(
    pack: &serde_json::Value,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    const CONTEXT: &str = "Pack";
    const OLD: &str = "supported_formats";
    let bad = |field: &str| format!("{CONTEXT} field {field} does not parse");
    let min =
        optional(&pack["min_format"], |v| format_codec(v, 0)).map_err(|()| bad("min_format"))?;
    let max = optional(&pack["max_format"], |v| {
        format_codec(v, i64::from(i32::MAX))
    })
    .map_err(|()| bad("max_format"))?;
    let format = optional(&pack["pack_format"], |v| {
        v.as_i64().filter(|n| i32::try_from(*n).is_ok())
    })
    .map_err(|()| bad("pack_format"))?;
    let supported = optional(&pack[OLD], int_range_codec).map_err(|()| bad(OLD))?;
    if min.is_some() != max.is_some() {
        return Err(format!(
            "{CONTEXT} missing field, must declare both min_format and max_format"
        ));
    }
    if let (Some(min), Some(max)) = (min, max) {
        return validate_new_format(min, max, format, supported, last_pre_minor);
    }
    if let Some(supported) = supported {
        return validate_old_format(supported, format, last_pre_minor);
    }
    match format {
        Some(main) if main > last_pre_minor => Err(format!(
            "{CONTEXT} declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        )),
        Some(main) => Ok(((main, 0), (main, 0))),
        None => Err(format!(
            "{CONTEXT} could not be parsed, missing format version information"
        )),
    }
}
/// `PackFormat.IntermediaryFormat.validateNewFormat`.
fn validate_new_format(
    min: Format,
    max: Format,
    format: Option<i64>,
    supported: Option<(i64, i64)>,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    const OLD: &str = "supported_formats";
    let (major_min, major_max) = (min.0, max.0);
    if min > max {
        return Err(format!(
            "Pack min_format ({}) is greater than max_format ({})",
            format_text(min),
            format_text(max)
        ));
    }
    if major_min > last_pre_minor {
        if supported.is_some() {
            return Err(format!(
                "Pack key {OLD} is deprecated starting from pack format {}. Remove {OLD} from your pack.mcmeta.",
                last_pre_minor + 1
            ));
        }
        if let Some(main) = format
            && let Some(error) = validate_pack_format_for_range(main, major_min, major_max)
        {
            return Err(error);
        }
    } else {
        let Some(old) = supported else {
            return Err(format!(
                "Pack declares support for format {major_min}, but game versions supporting formats 15 to {last_pre_minor} require a {OLD} field. Add \"{OLD}\": [{major_min}, {last_pre_minor}] or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if old.0 != major_min {
            return Err(format!(
                "Pack version declaration mismatch between {OLD} (from {}) and min_format ({})",
                old.0,
                format_text(min)
            ));
        }
        if old.1 != major_max && old.1 != last_pre_minor {
            return Err(format!(
                "Pack version declaration mismatch between {OLD} (up to {}) and max_format ({})",
                old.1,
                format_text(max)
            ));
        }
        let Some(main) = format else {
            return Err(format!(
                "Pack declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {major_min} or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if let Some(error) = validate_pack_format_for_range(main, major_min, major_max) {
            return Err(error);
        }
    }
    Ok((min, max))
}
/// `PackFormat.IntermediaryFormat.validateOldFormat`.
fn validate_old_format(
    supported: (i64, i64),
    format: Option<i64>,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    let (min, max) = supported;
    if max > last_pre_minor {
        return Err(format!(
            "Pack declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        ));
    }
    let Some(main) = format else {
        return Err(format!(
            "Pack declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {min} or require a version greater or equal to {}.0.",
            last_pre_minor + 1
        ));
    };
    if let Some(error) = validate_pack_format_for_range(main, min, max) {
        return Err(error);
    }
    Ok(((min, 0), (max, 0)))
}
/// PackCompatibility: inside the pack's declared range.
pub fn compatible(meta: &serde_json::Value) -> bool {
    compatibility(meta) == crate::pack_screen::Compat::Compatible
}
/// `PackCompatibility.forVersion`: too old, too new, compatible, or unknown
/// when the metadata names no (valid) format.
pub fn compatibility(meta: &serde_json::Value) -> crate::pack_screen::Compat {
    compatibility_for(meta, RESOURCE_FORMAT)
}
/// `compatibility` against `current`, the resource or the data pack format.
/// Metadata that fails `PackFormat.packCodec` parses as the fallback type,
/// whose format is `Integer.MAX_VALUE`: unknown.
pub fn compatibility_for(meta: &serde_json::Value, current: Format) -> crate::pack_screen::Compat {
    use crate::pack_screen::Compat;
    match validate_pack_range(&meta["pack"], last_pre_minor(current)) {
        Err(_) => Compat::Unknown,
        Ok((_, max)) if max < current => Compat::TooOld,
        Ok((min, _)) if current < min => Compat::TooNew,
        Ok(_) => Compat::Compatible,
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
        // OverlayMetadataSection's DIR_VALIDATOR: one folder of the pack, `[-_a-zA-Z0-9.]+`.
        .filter(|d| {
            stays_inside(d)
                && !d.contains('/')
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
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
    parse_meta(&bytes)
}
/// The first JSON value of a `pack.mcmeta`: what follows it is ignored, as some packs sign the
/// file with a comment after the closing brace. Gson reads past a UTF-8 byte order mark;
/// serde_json rejects it.
fn parse_meta(bytes: &[u8]) -> Option<serde_json::Value> {
    serde_json::Deserializer::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
        .into_iter()
        .next()?
        .ok()
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
                for name in zip.names().filter(|name| stays_inside(name)) {
                    if let Some(rest) = name.strip_prefix(&prefix) {
                        out.insert(format!("assets/{rest}"), (None, name.to_owned()));
                    }
                }
            }
        }
    }
    Ok(out)
}
/// Whether a zip entry's name, joined to the overlay folder, stays in it: `/`-separated parts
/// (zip.rs turns `\` into `/`), none empty, `.` or `..`, and no `:` (a drive or a stream on
/// Windows). A pack, a server's too, may name entries anything.
fn stays_inside(name: &str) -> bool {
    !name.contains([':', '\0'])
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
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
/// A directory junction (`horizon_threads::junction`); the mirror falls back to copying the
/// tree when it fails.
#[cfg(windows)]
fn junction(target: &Path, at: &Path) -> io::Result<()> {
    horizon_threads::junction(target, at)
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
/// `PackRepository.isAbleToClearAnyPack`: clearing the selection
/// (`rebuildSelected([])`, which keeps only the required vanilla pack)
/// changes it, that is, some selected pack still exists.
pub fn able_to_clear_any_pack(all: &[Pack], selected: &[String]) -> bool {
    !enabled(all, selected).is_empty()
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
    /// A server's downloaded pack (`DownloadedPackSource.DOWNLOADED_PACK_SELECTION`):
    /// required, fixed at the top, so the pack screen cannot remove or reorder it.
    pub fn downloaded(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: true,
            position: Position::Top,
            fixed: true,
            compatible,
        }
    }
}

/// Ids from `DownloadedPackSource` (`server/…`); they must not go into
/// `options.txt` `resourcePacks`.
pub fn is_downloaded_pack_id(id: &str) -> bool {
    id.starts_with("server/")
}

/// The player's own selection: drop every downloaded server/world pack id.
pub fn player_pack_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !is_downloaded_pack_id(id))
        .collect()
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
    /// stores it (downloaded `server/…` packs stay out of that file).
    pub fn commit(&self) -> Vec<String> {
        let ids: Vec<String> = self.selected.iter().rev().map(|p| p.id.clone()).collect();
        player_pack_ids(self.rebuild(&ids).into_iter().map(|p| p.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `DownloadedPackSource.DOWNLOADED_PACK_SELECTION`: the pack is selected,
    /// fixed at the top, cannot be removed, and stays out of options.txt.
    #[test]
    fn a_downloaded_server_pack_is_fixed_selected_and_not_committed() {
        let server = "server/00000000000000000000000000000001/hash.zip";
        let available = vec![
            Selectable::optional("file/Mine", true),
            Selectable::downloaded(server, true),
            Selectable::vanilla(),
        ];
        let model = SelectionModel::new(available, &["file/Mine".into()]);
        assert_eq!(
            model
                .selected
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            [server, "file/Mine", "vanilla"]
        );
        assert!(!model.can_unselect(true, 0));
        // A neighbour cannot move up past the fixed server pack.
        assert!(!model.can_move_up(true, 1));
        assert_eq!(
            model.commit(),
            vec!["vanilla".to_owned(), "file/Mine".to_owned()]
        );
        assert!(is_downloaded_pack_id(server));
        assert!(!is_downloaded_pack_id("file/Mine"));
    }

    #[test]
    fn a_comment_after_the_metadata_does_not_hide_a_pack() {
        let meta = br#"{"pack":{"description":"d","pack_format":97}}
// Signed by the pack's author"#;
        assert!(parse_meta(meta).is_some());
        assert!(parse_meta(b"not json").is_none());
    }

    #[test]
    fn mcmeta_with_utf8_bom_is_read() {
        let dir = std::env::temp_dir().join(format!("horizon-bom-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut meta = b"\xEF\xBB\xBF".to_vec();
        meta.extend_from_slice(br#"{"pack":{"description":"d","pack_format":97}}"#);
        fs::write(dir.join("pack.mcmeta"), &meta).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        // Without a BOM the same JSON still reads; other leading bytes still fail.
        fs::write(dir.join("pack.mcmeta"), &meta[3..]).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        fs::write(
            dir.join("pack.mcmeta"),
            [b"\xEF\xBB".as_slice(), &meta[3..]].concat(),
        )
        .unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_none());
        fs::remove_dir_all(&dir).ok();
    }
    /// `PackFormat.IntermediaryFormat.validate` and its three helpers, with
    /// the resource pack's last pre-minor version, 64.
    #[test]
    fn pack_format_validation() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let check = |s: &str| validate_pack_range(&v(s), 64);
        // validate: both min_format and max_format, or neither.
        assert!(
            check(r#"{"min_format":97}"#)
                .unwrap_err()
                .contains("both min_format and max_format")
        );
        assert!(check(r#"{"max_format":97}"#).is_err());
        assert!(check("{}").unwrap_err().contains("missing format version"));
        // A pack_format alone: fine up to 64, not above.
        assert_eq!(check(r#"{"pack_format":34}"#), Ok(((34, 0), (34, 0))));
        assert!(
            check(r#"{"pack_format":97}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        // validateNewFormat: min above max, the deprecated key above 64.
        assert!(
            check(r#"{"min_format":98,"max_format":97}"#)
                .unwrap_err()
                .contains("greater than max_format")
        );
        assert!(check(r#"{"min_format":[97,2],"max_format":[97,1]}"#).is_err());
        assert!(
            check(r#"{"min_format":97,"max_format":98,"supported_formats":[97,98]}"#)
                .unwrap_err()
                .contains("deprecated")
        );
        assert_eq!(
            check(r#"{"min_format":97,"max_format":98}"#),
            Ok(((97, 0), (98, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":[97,0],"max_format":[97,1]}"#),
            Ok(((97, 0), (97, 1)))
        );
        // validatePackFormatForRange: a pack_format outside the range.
        assert!(
            check(r#"{"min_format":97,"max_format":98,"pack_format":99}"#)
                .unwrap_err()
                .contains("declared main format is 99")
        );
        assert!(check(r#"{"min_format":97,"max_format":98,"pack_format":98}"#).is_ok());
        // Below 65 the old fields are required and must agree.
        assert!(
            check(r#"{"min_format":34,"max_format":70}"#)
                .unwrap_err()
                .contains("require a supported_formats")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[35,64],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,60],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(r#"{"min_format":34,"max_format":70,"supported_formats":[34,64]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":33}"#
            )
            .is_err()
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":34}"#
            )
            .is_ok()
        );
        // Multi-version packs below 15 cannot work.
        assert!(
            check(
                r#"{"min_format":10,"max_format":70,"supported_formats":[10,64],"pack_format":10}"#
            )
            .unwrap_err()
            .contains("less than 15")
        );
        // validateOldFormat: a range above 64, or no pack_format.
        assert!(
            check(r#"{"supported_formats":[34,70],"pack_format":34}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        assert!(
            check(r#"{"supported_formats":[34,60]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert_eq!(
            check(r#"{"supported_formats":[34,60],"pack_format":40}"#),
            Ok(((34, 0), (60, 0)))
        );
        assert!(
            check(
                r#"{"supported_formats":{"min_inclusive":34,"max_inclusive":60},"pack_format":40}"#
            )
            .is_ok()
        );
        assert!(check(r#"{"supported_formats":[60,34],"pack_format":40}"#).is_err());
        // A refused pack is of unknown format.
        use crate::pack_screen::Compat;
        assert_eq!(
            compatibility(&v(r#"{"pack":{"pack_format":97}}"#)),
            Compat::Unknown
        );
        assert_eq!(
            compatibility(&v(
                r#"{"pack":{"min_format":97,"max_format":98,"pack_format":99}}"#
            )),
            Compat::Unknown
        );
    }
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
    fn zip_entries_never_leave_the_overlay() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-escape-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::create_dir_all(base.join("packs")).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(base.join("packs/evil.zip")).unwrap());
        for (name, data) in [
            (
                "pack.mcmeta",
                &br#"{"pack":{"description":"d","pack_format":97}}"#[..],
            ),
            ("assets/minecraft/textures/block/stone.png", b"pack"),
            ("assets/../../escaped.txt", b"out"),
            ("assets/minecraft/../../../escaped_lang.json", b"{}"),
            ("assets\\..\\..\\escaped_backslash.txt", b"out"),
            ("assets/./minecraft/dot.txt", b"in"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        let out = base.join("work/overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        for escaped in [
            "escaped.txt",
            "escaped_lang.json",
            "escaped_backslash.txt",
            "work/escaped.txt",
            "work/escaped_lang.json",
            "work/escaped_backslash.txt",
        ] {
            assert!(!base.join(escaped).exists(), "{escaped} was written");
        }
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
        // Only one plain folder of the pack, as vanilla's DIR_VALIDATOR has it.
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":97,"directory":".."},
            {"formats":97,"directory":"../outside"},
            {"formats":97,"directory":"/etc"},
            {"formats":97,"directory":"a/b"},
            {"formats":97,"directory":"C:"},
            {"formats":97,"directory":"ok_1.21-x"}]}}"#);
        assert_eq!(overlays_of(&meta), ["ok_1.21-x"]);
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
