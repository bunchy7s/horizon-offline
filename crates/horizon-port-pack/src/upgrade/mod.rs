//! A 26.2 data pack's world generation, rewritten in 26.3's formats.
//!
//! 26.3 reworked the worldgen registries: configured carvers and features
//! became `carver` and `feature` with their `config` unwrapped, noise
//! parameters gained a base amplitude, density functions name their inputs
//! `input`/`left`/`right`, the noise settings moved aquifers, ore veins and
//! the surface rule out of the router into an `aquifers` object and a
//! `material_rule`, and biomes carry their mob spawns as attributes. Each
//! conversion here is the one vanilla's own data went through between 26.2
//! and 26.3, written so that the 26.3 pack generates what the 26.2 pack did:
//! the hard-coded parts of 26.2 (the beardifier added to the final density,
//! the deep dark aquifer exclusion, the ore veinifier, the interpolation
//! cell from `size_horizontal`/`size_vertical`) are spelled out as data.
//! Whatever has no faithful 26.3 form is kept as it is and listed in the
//! report.
mod density;
mod features;
mod noise;
mod registries;
mod settings;

use density::Context;
use noise::noise;
pub use noise::parity_base_amplitude;
use registries::{biome, carver, dimension_type, structure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

/// 26.3's data pack format (`version.json`'s `pack_version.data_major`).
pub const PACK_FORMAT: u64 = 121;

/// What the upgrade did and what it could not do faithfully.
#[derive(Default, Debug)]
pub struct Report {
    /// Files rewritten, per registry.
    pub upgraded: BTreeMap<String, usize>,
    /// Files written that the 26.2 pack did not have (named density
    /// functions the noise settings share).
    pub added: Vec<String>,
    /// Parts that changed meaning or were dropped, with how often.
    pub lossy: BTreeMap<String, usize>,
    /// Types the upgrade does not know, left as they are.
    pub unknown: BTreeMap<String, usize>,
}

impl Report {
    fn lossy(&mut self, what: impl Into<String>) {
        *self.lossy.entry(what.into()).or_default() += 1;
    }
    fn unknown(&mut self, what: impl Into<String>) {
        *self.unknown.entry(what.into()).or_default() += 1;
    }
}

/// Every file of a pack, by its path inside the pack (`data/...`,
/// `pack.mcmeta`), upgraded. Files outside world generation are kept as
/// they are.
pub fn upgrade_pack(files: &BTreeMap<String, Vec<u8>>) -> (BTreeMap<String, Vec<u8>>, Report) {
    let mut report = Report::default();
    let mut json: BTreeMap<String, Value> = BTreeMap::new();
    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, bytes) in files {
        let parsed = path
            .ends_with(".json")
            .then(|| serde_json::from_slice::<Value>(bytes).ok())
            .flatten();
        match parsed {
            Some(v) if path.starts_with("data/") => {
                json.insert(path.clone(), v);
            }
            _ => {
                out.insert(path.clone(), bytes.clone());
            }
        }
    }
    let mut ctx = Context::new(&json);
    let mut written: BTreeMap<String, Value> = BTreeMap::new();
    for (path, v) in &json {
        let Some(file) = PackFile::parse(path) else {
            written.insert(path.clone(), v.clone());
            continue;
        };
        let upgraded = match file.registry.as_str() {
            "worldgen/noise" => Some(noise(v, &mut report)),
            "worldgen/density_function" => Some(ctx.named_function(&file.id(), v, &mut report)),
            "worldgen/noise_settings" => Some(ctx.settings(&file.id(), v, &mut report)),
            "worldgen/configured_carver" => Some(carver(v, &mut report)),
            "worldgen/configured_feature" => Some(features::feature(v, &mut report)),
            "worldgen/placed_feature" => Some(features::placed(v, &mut report)),
            "worldgen/biome" => Some(biome(v, &mut report)),
            "worldgen/structure" => Some(structure(v)),
            "worldgen/world_preset" => Some(ctx.world_preset(v, &mut report)),
            "dimension" => Some(ctx.dimension(v, &mut report)),
            "dimension_type" => Some(dimension_type(v)),
            "worldgen/template_pool" | "worldgen/processor_list" => {
                let mut v = v.clone();
                states(&mut v);
                Some(v)
            }
            _ => None,
        };
        let path = match file.registry.as_str() {
            "worldgen/configured_carver" => file.with_registry("worldgen/carver"),
            "worldgen/configured_feature" => file.with_registry("worldgen/feature"),
            _ => path.clone(),
        };
        match upgraded {
            Some(u) => {
                *report.upgraded.entry(file.registry.clone()).or_default() += 1;
                written.insert(path, u);
            }
            None => {
                written.insert(path, v.clone());
            }
        }
    }
    for (id, v) in std::mem::take(&mut ctx.generated) {
        let path = registry_path("worldgen/density_function", &id);
        report.added.push(path.clone());
        written.insert(path, v);
    }
    for (id, v) in std::mem::take(&mut ctx.rules) {
        let path = registry_path("worldgen/material_rule", &id);
        report.added.push(path.clone());
        written.insert(path, v);
    }
    for (path, v) in written {
        out.insert(path, pretty(&v));
    }
    if let Some(meta) = out
        .get("pack.mcmeta")
        .and_then(|b| serde_json::from_slice(b).ok())
    {
        out.insert("pack.mcmeta".into(), pretty(&mcmeta(meta)));
    }
    (out, report)
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(v).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// `pack.mcmeta` for 26.3: the pack format range is 26.3's alone.
fn mcmeta(mut meta: Value) -> Value {
    if let Some(pack) = meta.get_mut("pack").and_then(Value::as_object_mut) {
        pack.remove("supported_formats");
        pack.insert("pack_format".into(), json!(PACK_FORMAT));
        pack.insert("min_format".into(), json!(PACK_FORMAT));
        pack.insert("max_format".into(), json!(PACK_FORMAT));
        if let Some(Value::String(d)) = pack.get_mut("description") {
            d.push_str(" (upgraded to 26.3)");
        }
    }
    meta
}

/// `data/<ns>/<registry...>/<path>.json`, split by the registries the
/// upgrade knows (a registry name has one or two segments).
struct PackFile {
    namespace: String,
    registry: String,
    path: String,
}

impl PackFile {
    fn parse(path: &str) -> Option<Self> {
        let rest = path.strip_prefix("data/")?.strip_suffix(".json")?;
        let (namespace, rest) = rest.split_once('/')?;
        let (registry, path) = if let Some(r) = rest.strip_prefix("worldgen/") {
            let (kind, path) = r.split_once('/')?;
            (format!("worldgen/{kind}"), path)
        } else {
            rest.split_once('/').map(|(k, p)| (k.to_owned(), p))?
        };
        Some(Self {
            namespace: namespace.into(),
            registry,
            path: path.into(),
        })
    }
    fn id(&self) -> String {
        format!("{}:{}", self.namespace, self.path)
    }
    fn with_registry(&self, registry: &str) -> String {
        format!("data/{}/{registry}/{}.json", self.namespace, self.path)
    }
}

fn registry_path(registry: &str, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("data/{ns}/{registry}/{path}.json")
}

/// A type name without its `minecraft:` prefix, as the codecs read both.
fn kind(v: &Value) -> Option<&str> {
    let t = v.get("type")?.as_str()?;
    Some(t.strip_prefix("minecraft:").unwrap_or(t))
}

/// An id with its namespace (`minecraft:` when it has none).
fn full_id(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// Whether an object of type `t` appears anywhere in `v`.
fn contains_type(v: &Value, t: &str) -> bool {
    match v {
        Value::Object(o) => kind(v) == Some(t) || o.values().any(|x| contains_type(x, t)),
        Value::Array(a) => a.iter().any(|x| contains_type(x, t)),
        _ => false,
    }
}

fn vanilla(name: &str) -> Value {
    Value::String(format!("minecraft:{name}"))
}

// ---------------------------------------------------------------------------
// Block states

/// 26.2 writes a block or fluid state as `{"Name": ..., "Properties":
/// {...}}`; 26.3 reads `{"id": ..., "properties": {...}}` (or the id alone
/// for the default state).
fn states(v: &mut Value) {
    match v {
        Value::Object(o) => {
            let state = o.get("Name").is_some_and(Value::is_string)
                && o.keys().all(|k| k == "Name" || k == "Properties");
            if state {
                let name = o.remove("Name").unwrap_or_default();
                match o.remove("Properties") {
                    Some(Value::Object(p)) if !p.is_empty() => {
                        o.insert("id".into(), name);
                        o.insert("properties".into(), Value::Object(p));
                    }
                    _ => *v = name,
                }
                return;
            }
            o.values_mut().for_each(states);
        }
        Value::Array(a) => a.iter_mut().for_each(states),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Files

/// A pack's files by their path inside it, from a zip or a folder. A zip
/// whose `pack.mcmeta` sits in one top folder is read from that folder.
pub fn read_pack(path: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut files = BTreeMap::new();
    if path.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).map_err(|e| at(&e))? {
                let p = entry.map_err(|e| at(&e))?.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(path) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    files.insert(rel, fs::read(&p).map_err(|e| at(&e))?);
                }
            }
        }
    } else {
        let file = fs::File::open(path).map_err(|e| at(&e))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| at(&e))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| at(&e))?;
            if entry.is_dir() {
                continue;
            }
            // A pack from the internet may name entries `../x` or `/x`.
            let name = entry
                .enclosed_name()
                .filter(|name| is_inside(name))
                .ok_or_else(|| at(&format!("entry {:?} leaves the pack", entry.name())))?;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(|e| at(&e))?;
            files.insert(name.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
    if !files.contains_key("pack.mcmeta") {
        let top = files
            .keys()
            .find_map(|k| k.strip_suffix("/pack.mcmeta").filter(|t| !t.contains('/')))
            .map(|t| format!("{t}/"));
        if let Some(top) = top {
            files = files
                .into_iter()
                .filter_map(|(k, v)| k.strip_prefix(&top).map(|k| (k.to_owned(), v)))
                .collect();
        }
    }
    if !files.contains_key("pack.mcmeta") {
        return Err(format!("{}: no pack.mcmeta", path.display()));
    }
    Ok(files)
}

/// Upgrades the pack at `input` into the folder `out`, which must not
/// exist yet (a world's `datapacks/<name>`).
pub fn upgrade_into(input: &Path, out: &Path) -> Result<Report, String> {
    if out.exists() {
        return Err(format!("{} already exists", out.display()));
    }
    let files = read_pack(input)?;
    let (upgraded, report) = upgrade_pack(&files);
    for (path, bytes) in upgraded {
        if !is_inside(Path::new(&path)) {
            return Err(format!("{path}: leaves {}", out.display()));
        }
        let target = out.join(&path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(report)
}

/// Whether `path` is a plain relative path: no root, drive, `.` or `..`.
fn is_inside(path: &Path) -> bool {
    path.components().next().is_some()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

impl Report {
    /// The report as lines for the terminal.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .upgraded
            .iter()
            .map(|(r, n)| format!("upgraded {n} {r}"))
            .collect();
        if !self.added.is_empty() {
            lines.push(format!(
                "added {} shared density functions",
                self.added.len()
            ));
        }
        lines.extend(
            self.lossy
                .iter()
                .map(|(w, n)| format!("not faithful ({n}x): {w}")),
        );
        lines.extend(
            self.unknown
                .iter()
                .map(|(w, n)| format!("left as it is ({n}x): {w}")),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(path: &str, v: Value) -> (Value, Report) {
        let files = BTreeMap::from([(path.to_owned(), serde_json::to_vec(&v).unwrap())]);
        let (out, report) = upgrade_pack(&files);
        let (_, bytes) = out
            .into_iter()
            .find(|(p, _)| !p.contains("/upgraded/"))
            .unwrap();
        (serde_json::from_slice(&bytes).unwrap(), report)
    }

    #[test]
    fn noise_keeps_the_octaves_and_finds_the_parity_amplitude() {
        let (v, _) = up(
            "data/x/worldgen/noise/n.json",
            json!({"firstOctave": -7, "amplitudes": [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]}),
        );
        // Vanilla 26.3's minecraft:ridge, which 26.2 wrote as above.
        assert_eq!(
            v,
            json!({"base_amplitude": 0.9147152149950137, "base_octave": -7, "octave_count": 6,
                "amplitude_modifiers": [1.0, 2.0, 1.0, 0.0, 0.0, 0.0]})
        );
        let (v, _) = up(
            "data/x/worldgen/noise/n.json",
            json!({"firstOctave": -3, "amplitudes": [1.0]}),
        );
        // Vanilla 26.3's minecraft:aquifer_barrier.
        assert_eq!(
            v,
            json!({"base_amplitude": 0.955388882960065, "base_octave": -3})
        );
    }

    #[test]
    fn density_arguments_and_removed_types() {
        let (v, _) = up(
            "data/x/worldgen/density_function/f.json",
            json!({"type": "minecraft:add", "argument1": {"type": "minecraft:constant", "argument": 2.0},
                "argument2": {"type": "minecraft:abs", "argument": {"type": "minecraft:y_clamped_gradient",
                    "from_y": -64, "to_y": 320, "from_value": 1.5, "to_value": -1.5}}}),
        );
        assert_eq!(
            v,
            json!({"type": "minecraft:add", "left": 2.0, "right": {"type": "minecraft:abs",
                "input": {"type": "minecraft:gradient", "axis": "y", "from_coordinate": -64,
                    "to_coordinate": 320, "from_value": 1.5, "to_value": -1.5}}})
        );
        let (v, _) = up(
            "data/x/worldgen/density_function/f.json",
            json!({"type": "minecraft:shifted_noise", "noise": "minecraft:erosion",
                "shift_x": "minecraft:shift_x", "shift_y": 0.0, "shift_z": "minecraft:shift_z",
                "xz_scale": 0.25, "y_scale": 0.0}),
        );
        assert_eq!(
            v,
            json!({"type": "minecraft:noise", "noise": "minecraft:erosion", "shift_x": "minecraft:shift_x",
                "shift_z": "minecraft:shift_z", "xz_scale": 0.25, "y_scale": 0.0})
        );
    }

    #[test]
    fn flat_cache_slices_at_y_zero_only_what_depends_on_y() {
        let flat = |argument: Value| {
            up(
                "data/x/worldgen/density_function/f.json",
                json!({"type": "minecraft:flat_cache", "argument": argument}),
            )
            .0
        };
        let n = json!({"type": "minecraft:noise", "noise": "x:n", "xz_scale": 1.0, "y_scale": 0.0});
        assert_eq!(
            flat(n.clone()),
            json!({"type": "minecraft:cache", "input": n})
        );
        let y = json!({"type": "minecraft:mul", "argument1": "minecraft:y", "argument2": 2.0});
        assert_eq!(
            flat(y),
            json!({"type": "minecraft:cache", "input": {"type": "minecraft:slice", "axis": "y",
                "coordinate": 0, "input": {"type": "minecraft:mul", "left": "minecraft:y", "right": 2.0}}})
        );
    }

    #[test]
    fn interpolation_cell_comes_from_the_settings() {
        let settings = json!({"noise": {"min_y": 0, "height": 128, "size_horizontal": 2, "size_vertical": 1},
            "noise_router": {}, "surface_rule": {"type": "minecraft:sequence", "sequence": []},
            "default_block": {"Name": "minecraft:stone"}, "default_fluid": {"Name": "minecraft:water",
                "Properties": {"level": "0"}}, "aquifers_enabled": false, "ore_veins_enabled": false,
            "sea_level": 32, "disable_mob_generation": false, "legacy_random_source": false,
            "spawn_target": []});
        let files = BTreeMap::from([
            (
                "data/x/worldgen/noise_settings/s.json".to_owned(),
                serde_json::to_vec(&settings).unwrap(),
            ),
            (
                "data/x/worldgen/density_function/f.json".to_owned(),
                serde_json::to_vec(&json!({"type": "minecraft:interpolated", "argument": 1.0}))
                    .unwrap(),
            ),
        ]);
        let (out, _) = upgrade_pack(&files);
        let f: Value =
            serde_json::from_slice(&out["data/x/worldgen/density_function/f.json"]).unwrap();
        assert_eq!(
            f,
            json!({"type": "minecraft:interpolated", "cell_size_xz": 8, "cell_size_y": 4, "input": 1.0})
        );
        let s: Value =
            serde_json::from_slice(&out["data/x/worldgen/noise_settings/s.json"]).unwrap();
        assert_eq!(s["noise"], json!({"min_y": 0, "height": 128}));
        assert_eq!(
            s["default_fluid"],
            json!({"id": "minecraft:water", "properties": {"level": "0"}})
        );
        assert_eq!(s["default_block"], json!("minecraft:stone"));
        assert!(s.get("aquifers").is_none());
        assert_eq!(
            s["noise_router"]["final_density"]["right"],
            json!({"type": "minecraft:beardifier"})
        );
    }

    #[test]
    fn settings_move_aquifers_veins_and_spawn_target() {
        let settings = json!({"noise": {"min_y": -64, "height": 384, "size_horizontal": 1, "size_vertical": 2},
            "noise_router": {"barrier": 1.0, "fluid_level_floodedness": 2.0, "fluid_level_spread": 3.0,
                "lava": 4.0, "temperature": "x:t", "vegetation": "x:v", "continents": "x:c",
                "erosion": "x:e", "depth": "x:d", "ridges": "x:r", "final_density": "x:f",
                "preliminary_surface_level": {"type": "minecraft:constant", "argument": 64.0},
                "vein_toggle": 0.5, "vein_ridged": -1.0, "vein_gap": 0.0},
            "surface_rule": {"type": "minecraft:block", "result_state": {"Name": "minecraft:dirt"}},
            "default_block": {"Name": "minecraft:stone"}, "default_fluid": {"Name": "minecraft:water"},
            "aquifers_enabled": true, "ore_veins_enabled": true, "sea_level": 63,
            "disable_mob_generation": false, "legacy_random_source": false,
            "spawn_target": [{"temperature": [-1, 1], "humidity": [-1, 1], "continentalness": [-0.11, 1],
                "erosion": [-1, 1], "weirdness": [-1, -0.16], "depth": 0, "offset": 0}]});
        let files = BTreeMap::from([(
            "data/x/worldgen/noise_settings/s.json".to_owned(),
            serde_json::to_vec(&settings).unwrap(),
        )]);
        let (out, _) = upgrade_pack(&files);
        let read = |p: &str| -> Value { serde_json::from_slice(&out[p]).unwrap() };
        let s = read("data/x/worldgen/noise_settings/s.json");
        assert_eq!(s["material_rule"], json!("x:s/upgraded/material_rule"));
        let rule = read("data/x/worldgen/material_rule/s/upgraded/material_rule.json");
        let aquifers = &s["aquifers"];
        assert_eq!(aquifers["barrier"], json!(1.0));
        assert_eq!(
            aquifers["surface_level"],
            json!("x:s/upgraded/preliminary_surface_level")
        );
        assert_eq!(aquifers["exclusion"]["left"]["right"], json!("x:e"));
        let rules = rule["sequence"].as_array().unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0]["type"], json!("minecraft:ore_vein"));
        assert_eq!(
            rules[2],
            json!({"type": "minecraft:block", "result_state": "minecraft:dirt"})
        );
        assert_eq!(
            s["spawn_target"],
            json!([{"x:t": [-1, 1], "x:v": [-1, 1], "x:c": [-0.11, 1], "x:e": [-1, 1], "x:r": [-1, -0.16]}])
        );
        assert_eq!(
            s["noise_router"]["chunk_surface_level"],
            json!({"type": "minecraft:interpolated", "cell_size_xz": 16, "cell_size_y": 1,
                "input": "x:s/upgraded/preliminary_surface_level"})
        );
    }

    #[test]
    fn features_lose_their_config_and_placements_their_spread() {
        let (v, _) = up(
            "data/x/worldgen/configured_feature/f.json",
            json!({"type": "minecraft:disk", "config": {"state_provider": {"type":
                "minecraft:simple_state_provider", "state": {"Name": "minecraft:clay"}}, "radius": 2}}),
        );
        assert_eq!(
            v,
            json!({"type": "minecraft:disk", "state_provider": "minecraft:clay", "radius": 2})
        );
        let (v, _) = up(
            "data/x/worldgen/placed_feature/p.json",
            json!({"feature": "x:f", "placement": [{"type": "minecraft:random_offset",
                "xz_spread": 0, "y_spread": -1}]}),
        );
        assert_eq!(
            v,
            json!({"feature": "x:f", "placement": [{"type": "minecraft:offset", "x": 0, "y": -1, "z": 0}]})
        );
    }

    #[test]
    fn biome_spawns_become_attributes() {
        let (v, _) = up(
            "data/x/worldgen/biome/b.json",
            json!({"spawners": {"monster": [{"type": "minecraft:zombie", "minCount": 4, "maxCount": 4,
                "weight": 95}], "ambient": []}, "spawn_costs": {}, "creature_spawn_probability": 0.07,
                "attributes": {"minecraft:visual/sky_color": "#78a7ff"}}),
        );
        assert_eq!(
            v["attributes"]["minecraft:gameplay/natural_mob_spawns"],
            json!({"modifier": "overlay", "argument": {"spawn_costs": {}, "spawns_by_category": {
                "monster": [{"type": "minecraft:zombie", "count": 4, "weight": 95}], "ambient": []}}})
        );
        assert_eq!(
            v["attributes"]["minecraft:gameplay/creature_world_gen_spawn_probability"],
            json!(0.07)
        );
        assert!(v.get("spawners").is_none());
    }

    #[test]
    fn entries_outside_the_pack_are_refused() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("upgrade-escape-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (i, name) in ["../escape.txt", "data/../../escape.txt", "/escape.txt"]
            .into_iter()
            .enumerate()
        {
            let zip_path = dir.join(format!("p{i}.zip"));
            let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
            for (file, bytes) in [("pack.mcmeta", &b"{}"[..]), (name, b"x")] {
                zip.start_file(file, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            let out = dir.join(format!("out{i}/datapacks/p-26.3"));
            let err = upgrade_into(&zip_path, &out).unwrap_err();
            assert!(err.contains("leaves the pack"), "{name}: {err}");
            assert!(!out.exists(), "{name}");
        }
        assert!(!dir.join("escape.txt").exists());
        assert!(is_inside(Path::new("data/x/worldgen/biome/b.json")));
        assert!(!is_inside(Path::new("data/../x.json")));
        assert!(!is_inside(Path::new("")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn carvers_move_and_spell_out_their_hard_coded_shape() {
        let files = BTreeMap::from([(
            "data/x/worldgen/configured_carver/c.json".to_owned(),
            serde_json::to_vec(&json!({"type": "minecraft:canyon", "config": {"probability": 0.01,
                "yScale": 3.0, "lava_level": {"above_bottom": 8}, "shape": {"width_smoothness": 3}}}))
            .unwrap(),
        )]);
        let (out, _) = upgrade_pack(&files);
        let v: Value = serde_json::from_slice(&out["data/x/worldgen/carver/c.json"]).unwrap();
        assert_eq!(
            v,
            json!({"type": "minecraft:canyon", "probability": 0.01,
                "shape": {"width_smoothness": 3, "y_scale": 3.0}})
        );
    }
}
