//! Biome modifiers: a content mod's placed features in biomes it does not
//! own (vanilla's), as NeoForge's `neoforge:add_features` and
//! `neoforge:remove_features` biome modifiers do. Each is a file in
//! `data/<ns>/neoforge/biome_modifier/` (older Forge mods:
//! `data/<ns>/forge/biome_modifier/`, type `forge:...`).
//!
//! They rewrite the biomes' `features` lists when the content root builds:
//! an added feature goes at the end of its step's list, as NeoForge adds it,
//! and the generator orders every biome's lists as vanilla's `FeatureSorter`
//! does. Biomes no modifier names keep their text byte for byte, and a root
//! with no modifiers keeps vanilla's bundle, so vanilla features keep their
//! indices and their random seeds.
use super::{J, Result};
use horizon_world::worldgen::structure::start::STEPS;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The folders a biome modifier sits in, under `data/<ns>`.
const FOLDERS: [&str; 2] = ["/neoforge/biome_modifier/", "/forge/biome_modifier/"];

/// Whether a mod file is a biome modifier.
pub(super) fn is_modifier(file: &str) -> bool {
    file.starts_with("data/") && file.ends_with(".json") && FOLDERS.iter().any(|f| file.contains(f))
}

/// What a modifier does to the features of its biomes.
#[derive(Debug, PartialEq)]
enum Action {
    /// Appends to one generation step.
    Add(usize),
    /// Takes out of these steps (every step when the file names none).
    Remove(Vec<usize>),
}

/// One biome modifier file.
#[derive(Debug)]
pub(super) struct Modifier {
    /// `<mod>: <file>`, for errors.
    name: String,
    biomes: Value,
    features: Value,
    action: Action,
}

/// A generation step's index from its name.
fn step(v: &Value) -> std::result::Result<usize, String> {
    let name = v.as_str().ok_or("step: not a name")?;
    STEPS
        .iter()
        .position(|s| *s == name)
        .ok_or_else(|| format!("unknown generation step {name:?}"))
}

impl Modifier {
    /// The modifier in `text`; `None` for `neoforge:none`, which does nothing,
    /// and for a type Horizon doesn't read yet (`add_spawns`, `add_carvers`, a
    /// mod's own serializer), which is skipped with a line in `warnings`: real
    /// mods ship those beside their feature modifiers, and one must not drop
    /// every content mod.
    pub(super) fn read(
        mod_id: &str,
        file: &str,
        text: &str,
        warnings: &mut Vec<String>,
    ) -> Result<Option<Self>> {
        let name = format!("mod {mod_id}: {file}");
        let at = |e: String| format!("{name}: {e}");
        let doc: Value = serde_json::from_str(text).map_err(|e| at(e.to_string()))?;
        let kind = doc["type"].as_str().ok_or_else(|| at("no type".into()))?;
        let kind = kind
            .strip_prefix("neoforge:")
            .or_else(|| kind.strip_prefix("forge:"))
            .unwrap_or(kind);
        let field = |key: &str| -> std::result::Result<Value, String> {
            doc.get(key).cloned().ok_or_else(|| at(format!("no {key}")))
        };
        let action = match kind {
            "none" => return Ok(None),
            "add_features" => Action::Add(step(&field("step")?).map_err(at)?),
            "remove_features" => Action::Remove(match doc.get("steps") {
                None => (0..STEPS.len()).collect(),
                Some(Value::Array(steps)) => steps
                    .iter()
                    .map(step)
                    .collect::<std::result::Result<_, _>>()
                    .map_err(at)?,
                Some(one) => vec![step(one).map_err(at)?],
            }),
            other => {
                warnings.push(at(format!(
                    "biome modifier type {other:?} skipped: only add_features, remove_features \
                     and none are read"
                )));
                return Ok(None);
            }
        };
        Ok(Some(Self {
            biomes: field("biomes")?,
            features: field("features")?,
            action,
            name,
        }))
    }
}

/// `ns:path` for an id that may leave out `minecraft:`.
fn full(id: &str) -> String {
    match id.contains(':') {
        true => id.to_owned(),
        false => format!("minecraft:{id}"),
    }
}

/// A registry the modifiers name entries of: its ids and its tags (tag id →
/// the tag file's text), as the worldgen loader's bundles hold them.
pub(super) struct Registry<'a> {
    pub what: &'static str,
    pub ids: &'a BTreeSet<String>,
    pub tags: &'a BTreeMap<String, String>,
}

impl Registry<'_> {
    /// The ids a holder set names, in its order without repeats: an id,
    /// `#tag`, a list of ids, or one of NeoForge's `any`, `or`, `and` and
    /// `not` objects. Unknown ids and tags are errors; inside a tag, entries
    /// nobody registers are left out, as the worldgen loader leaves them out.
    fn resolve(&self, set: &Value) -> std::result::Result<Vec<String>, String> {
        let mut out = Vec::new();
        match set {
            Value::String(s) => match s.strip_prefix('#') {
                Some(tag) => self.expand(&full(tag), &mut out, &mut BTreeSet::new())?,
                None => {
                    let id = full(s);
                    if !self.ids.contains(&id) {
                        return Err(format!("unknown {} {id}", self.what));
                    }
                    out.push(id);
                }
            },
            Value::Array(list) => {
                for v in list {
                    match v {
                        Value::String(s) if !s.starts_with('#') => {
                            extend(&mut out, self.resolve(v)?)
                        }
                        _ => return Err(format!("a {} list holds ids only", self.what)),
                    }
                }
            }
            Value::Object(o) => {
                let kind = o.get("type").and_then(Value::as_str).unwrap_or_default();
                let kind = kind.strip_prefix("neoforge:").unwrap_or(kind);
                let sets = || -> std::result::Result<Vec<Vec<String>>, String> {
                    o.get("values")
                        .and_then(Value::as_array)
                        .ok_or(format!("{kind}: no values"))?
                        .iter()
                        .map(|v| self.resolve(v))
                        .collect()
                };
                match kind {
                    "any" => out.extend(self.ids.iter().cloned()),
                    "or" => sets()?.into_iter().for_each(|s| extend(&mut out, s)),
                    "and" => {
                        let mut sets = sets()?.into_iter();
                        out = sets.next().unwrap_or_default();
                        for s in sets {
                            out.retain(|id| s.contains(id));
                        }
                    }
                    "not" => {
                        let not = self.resolve(o.get("value").ok_or("not: no value")?)?;
                        out.extend(self.ids.iter().filter(|id| !not.contains(id)).cloned());
                    }
                    _ => return Err(format!("{} set type {kind:?} is not read", self.what)),
                }
            }
            _ => return Err(format!("not a {} id, tag or list", self.what)),
        }
        Ok(out)
    }

    fn expand(
        &self,
        tag: &str,
        out: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> std::result::Result<(), String> {
        if !seen.insert(tag.to_owned()) {
            return Ok(());
        }
        let text = self
            .tags
            .get(tag)
            .ok_or_else(|| format!("unknown {} tag #{tag}", self.what))?;
        let doc: Value = serde_json::from_str(text).map_err(|e| format!("#{tag}: {e}"))?;
        for v in doc["values"].as_array().into_iter().flatten() {
            let (id, required) = match v {
                Value::String(s) => (s.as_str(), true),
                Value::Object(o) => match o.get("id").and_then(Value::as_str) {
                    Some(id) => (id, o.get("required") != Some(&Value::Bool(false))),
                    None => continue,
                },
                _ => continue,
            };
            match id.strip_prefix('#') {
                Some(inner) if required || self.tags.contains_key(&full(inner)) => {
                    self.expand(&full(inner), out, seen)?
                }
                Some(_) => {}
                None if self.ids.contains(&full(id)) => extend(out, [full(id)]),
                None => {}
            }
        }
        Ok(())
    }
}

/// Appends the ids `out` lacks.
fn extend(out: &mut Vec<String>, ids: impl IntoIterator<Item = String>) {
    for id in ids {
        if !out.contains(&id) {
            out.push(id);
        }
    }
}

/// Applies `modifiers` to the biome bundle `biomes` (id → the biome's text):
/// every addition first, then every removal, each in mod and file order (as
/// NeoForge's add phase runs before its remove phase). A modifier naming a
/// biome, feature or tag the root doesn't have (another mod's, a `#c:`
/// convention tag) is skipped with a line in `warnings`, as NeoForge skips a
/// holder set it can't resolve, instead of dropping every content mod.
pub(super) fn apply(
    biomes: &mut J,
    modifiers: &[Modifier],
    biome_reg: &Registry<'_>,
    feature_reg: &Registry<'_>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let adds = modifiers
        .iter()
        .filter(|m| matches!(m.action, Action::Add(_)));
    let removes = modifiers
        .iter()
        .filter(|m| matches!(m.action, Action::Remove(_)));
    // Each biome changed, parsed once, in the bundle's order.
    let mut changed: BTreeMap<String, J> = BTreeMap::new();
    for m in adds.chain(removes) {
        let at = |e: String| format!("{}: {e}", m.name);
        let resolved = biome_reg
            .resolve(&m.biomes)
            .and_then(|targets| Ok((targets, feature_reg.resolve(&m.features)?)));
        let (targets, features) = match resolved {
            Ok(both) => both,
            Err(e) => {
                warnings.push(at(format!("{e}: skipped")));
                continue;
            }
        };
        for biome in targets {
            let doc = match changed.entry(biome.clone()) {
                std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::btree_map::Entry::Vacant(e) => {
                    let Some(J::Str(text)) = biomes.get(&biome) else {
                        continue;
                    };
                    e.insert(serde_json::from_str(text).map_err(|e| at(format!("{biome}: {e}")))?)
                }
            };
            edit(doc, &m.action, &features).map_err(|e| at(format!("{biome}: {e}")))?;
        }
    }
    for (id, doc) in changed {
        biomes.set(&id, J::Str(serde_json::to_string(&doc)?));
    }
    Ok(())
}

/// One modifier's change to one biome's `features` (a list per step).
fn edit(biome: &mut J, action: &Action, features: &[String]) -> std::result::Result<(), String> {
    if biome.get("features").is_none() {
        biome.set("features", J::Arr(Vec::new()));
    }
    let Some(J::Arr(steps)) = biome.get_mut("features") else {
        return Err("features: not a list of steps".into());
    };
    let wanted: &[usize] = match action {
        Action::Add(step) => std::slice::from_ref(step),
        Action::Remove(steps) => steps,
    };
    if let Action::Add(step) = action {
        while steps.len() <= *step {
            steps.push(J::Arr(Vec::new()));
        }
    }
    for &i in wanted {
        let Some(list) = steps.get_mut(i) else {
            continue;
        };
        // A step may be one id; a tag cannot take more features.
        if let J::Str(one) = list {
            if one.starts_with('#') {
                return Err(format!("step {} is the feature tag {one}", STEPS[i]));
            }
            *list = J::Arr(vec![J::Str(one.clone())]);
        }
        let J::Arr(list) = list else {
            return Err(format!("step {}: not a list", STEPS[i]));
        };
        let held = |v: &J| match v {
            J::Str(s) => Some(full(s)),
            _ => None,
        };
        match action {
            Action::Add(_) => {
                for f in features {
                    // Twice in one step would be a cycle for the sorter.
                    if !list.iter().any(|v| held(v).as_ref() == Some(f)) {
                        list.push(J::Str(f.clone()));
                    }
                }
            }
            Action::Remove(_) => {
                list.retain(|v| held(v).is_none_or(|id| !features.contains(&id)));
            }
        }
    }
    Ok(())
}
