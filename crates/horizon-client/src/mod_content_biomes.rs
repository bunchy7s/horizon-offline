//! Biome modifiers: a content mod's placed features and mob spawns in biomes
//! it does not own (vanilla's), as NeoForge's `add_features`,
//! `remove_features`, `add_spawns`, `remove_spawns`, `add_spawn_costs` and
//! `remove_spawn_costs` biome modifiers do. Each is a file in
//! `data/<ns>/neoforge/biome_modifier/` (older Forge mods:
//! `data/<ns>/forge/biome_modifier/`, type `forge:...`).
//!
//! They rewrite the biomes' `features` lists and
//! `minecraft:gameplay/natural_mob_spawns` attributes when the content root
//! builds: an added feature goes at the end of its step's list, as NeoForge
//! adds it, and the generator orders every biome's lists as vanilla's
//! `FeatureSorter` does; an added spawner goes at the end of its entity type's
//! spawn category with `minCount` and `maxCount` converted to 26.3's `count`.
//! Biomes no modifier names keep their text byte for byte, and a root with no
//! modifiers keeps vanilla's bundle, so vanilla features keep their indices
//! and their random seeds.
use super::{J, Result};
use horizon_world::worldgen::structure::start::STEPS;
use serde_json::{Number, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The folders a biome modifier sits in, under `data/<ns>`.
const FOLDERS: [&str; 2] = ["/neoforge/biome_modifier/", "/forge/biome_modifier/"];

/// Whether a mod file is a biome modifier.
pub(super) fn is_modifier(file: &str) -> bool {
    file.starts_with("data/") && file.ends_with(".json") && FOLDERS.iter().any(|f| file.contains(f))
}

/// One mob spawner entry in an `add_spawns` modifier.
#[derive(Debug, PartialEq)]
struct Spawner {
    entity: String,
    weight: i64,
    min_count: i64,
    max_count: i64,
}

/// What a modifier does to its biomes.
#[derive(Debug, PartialEq)]
enum Action {
    /// Appends features to one generation step.
    AddFeatures { features: Value, step: usize },
    /// Takes features out of these steps (every step when the file names none).
    RemoveFeatures { features: Value, steps: Vec<usize> },
    /// Appends spawners to their entity types' spawn categories.
    AddSpawns(Vec<Spawner>),
    /// Takes these entity types out of every spawn category.
    RemoveSpawns(Value),
    /// Sets `spawn_costs` for these entity types.
    AddSpawnCosts {
        entity_types: Value,
        energy_budget: Number,
        charge: Number,
    },
    /// Takes these entity types out of `spawn_costs`.
    RemoveSpawnCosts(Value),
}

impl Action {
    fn is_add(&self) -> bool {
        matches!(
            self,
            Self::AddFeatures { .. } | Self::AddSpawns(_) | Self::AddSpawnCosts { .. }
        )
    }
}

/// One biome modifier file.
#[derive(Debug)]
pub(super) struct Modifier {
    /// `<mod>: <file>`, for errors.
    name: String,
    biomes: Value,
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

/// One NeoForge `SpawnerData` object from `spawners`.
fn spawner(v: &Value) -> std::result::Result<Spawner, String> {
    let o = v.as_object().ok_or("spawner: not an object")?;
    let entity = o
        .get("type")
        .and_then(Value::as_str)
        .ok_or("spawner: no type")?;
    let int = |k: &str| {
        o.get(k)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("spawner: no {k}"))
    };
    let weight = int("weight")?;
    let min_count = int("minCount")?;
    let max_count = int("maxCount")?;
    if weight < 0 {
        return Err(format!("spawner: weight ({weight}) < 0"));
    }
    if min_count <= 0 {
        return Err(format!("spawner: minCount ({min_count}) must be positive"));
    }
    if min_count > max_count {
        return Err(format!(
            "spawner: minCount ({min_count}) > maxCount ({max_count})"
        ));
    }
    Ok(Spawner {
        entity: full(entity),
        weight,
        min_count,
        max_count,
    })
}

impl Modifier {
    /// The modifier in `text`; `None` for `neoforge:none`, which does nothing,
    /// and for a type Horizon doesn't read yet (`add_carvers`, a mod's own
    /// serializer), which is skipped with a line in `warnings`: real mods ship
    /// those beside their feature and spawn modifiers, and one must not drop
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
            "add_features" => Action::AddFeatures {
                features: field("features")?,
                step: step(&field("step")?).map_err(at)?,
            },
            "remove_features" => Action::RemoveFeatures {
                features: field("features")?,
                steps: match doc.get("steps") {
                    None => (0..STEPS.len()).collect(),
                    Some(Value::Array(steps)) => steps
                        .iter()
                        .map(step)
                        .collect::<std::result::Result<_, _>>()
                        .map_err(at)?,
                    Some(one) => vec![step(one).map_err(at)?],
                },
            },
            "add_spawns" => {
                let raw = field("spawners")?;
                let spawners = match &raw {
                    Value::Array(list) => list
                        .iter()
                        .map(spawner)
                        .collect::<std::result::Result<_, _>>()
                        .map_err(at)?,
                    one @ Value::Object(_) => vec![spawner(one).map_err(at)?],
                    _ => return Err(at("spawners: not a spawner or list".into()).into()),
                };
                Action::AddSpawns(spawners)
            }
            "remove_spawns" => Action::RemoveSpawns(field("entity_types")?),
            "add_spawn_costs" => {
                let entity_types = field("entity_types")?;
                let cost = field("spawn_cost")?;
                let num = |k: &str| {
                    cost.get(k)
                        .and_then(Value::as_number)
                        .cloned()
                        .ok_or_else(|| at(format!("spawn_cost: no {k}")))
                };
                Action::AddSpawnCosts {
                    entity_types,
                    energy_budget: num("energy_budget")?,
                    charge: num("charge")?,
                }
            }
            "remove_spawn_costs" => Action::RemoveSpawnCosts(field("entity_types")?),
            other => {
                warnings.push(at(format!(
                    "biome modifier type {other:?} skipped: only feature, spawn and none \
                     modifiers are read"
                )));
                return Ok(None);
            }
        };
        Ok(Some(Self {
            biomes: field("biomes")?,
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
/// biome, feature, entity type or tag the root doesn't have (another mod's, a
/// `#c:` convention tag) is skipped with a line in `warnings`, as NeoForge
/// skips a holder set it can't resolve, instead of dropping every content mod.
pub(super) fn apply(
    biomes: &mut J,
    modifiers: &[Modifier],
    biome_reg: &Registry<'_>,
    feature_reg: &Registry<'_>,
    entity_reg: &Registry<'_>,
    entity_categories: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<Vec<(String, String)>> {
    let adds = modifiers.iter().filter(|m| m.action.is_add());
    let removes = modifiers.iter().filter(|m| !m.action.is_add());
    // Each biome changed, parsed once, in the bundle's order.
    let mut changed: BTreeMap<String, J> = BTreeMap::new();
    for m in adds.chain(removes) {
        let at = |e: String| format!("{}: {e}", m.name);
        let resolved: std::result::Result<(Vec<String>, Vec<String>), String> =
            biome_reg.resolve(&m.biomes).and_then(|targets| {
                let ids = match &m.action {
                    Action::AddFeatures { features, .. }
                    | Action::RemoveFeatures { features, .. } => feature_reg.resolve(features)?,
                    Action::AddSpawns(spawners) => {
                        if let Some(s) = spawners
                            .iter()
                            .find(|s| !entity_reg.ids.contains(&s.entity))
                        {
                            return Err(format!("unknown {} {}", entity_reg.what, s.entity));
                        }
                        Vec::new()
                    }
                    Action::RemoveSpawns(entity_types)
                    | Action::AddSpawnCosts { entity_types, .. }
                    | Action::RemoveSpawnCosts(entity_types) => entity_reg.resolve(entity_types)?,
                };
                Ok((targets, ids))
            });
        let (targets, ids) = match resolved {
            Ok(both) => both,
            Err(e) => {
                warnings.push(at(format!("{e}: skipped")));
                continue;
            }
        };
        if matches!(&m.action, Action::AddSpawns(s) if s.is_empty()) {
            continue;
        }
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
            edit(doc, &m.action, &ids, entity_categories)
                .map_err(|e| at(format!("{biome}: {e}")))?;
        }
    }
    let mut out = Vec::with_capacity(changed.len());
    for (id, doc) in changed {
        let text = serde_json::to_string(&doc)?;
        biomes.set(&id, J::Str(text.clone()));
        out.push((id, text));
    }
    Ok(out)
}

/// One modifier's change to one biome.
fn edit(
    biome: &mut J,
    action: &Action,
    ids: &[String],
    entity_categories: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    match action {
        Action::AddFeatures { step, .. } => {
            edit_features(biome, std::slice::from_ref(step), true, ids)
        }
        Action::RemoveFeatures { steps, .. } => edit_features(biome, steps, false, ids),
        Action::AddSpawns(spawners) => add_spawns(biome, spawners, entity_categories),
        Action::RemoveSpawns(_) => remove_spawns(biome, ids),
        Action::AddSpawnCosts {
            energy_budget,
            charge,
            ..
        } => add_spawn_costs(biome, ids, energy_budget, charge),
        Action::RemoveSpawnCosts(_) => remove_spawn_costs(biome, ids),
    }
}

/// One feature modifier's change to one biome's `features` (a list per step).
fn edit_features(
    biome: &mut J,
    wanted: &[usize],
    add: bool,
    features: &[String],
) -> std::result::Result<(), String> {
    if biome.get("features").is_none() {
        biome.set("features", J::Arr(Vec::new()));
    }
    let Some(J::Arr(steps)) = biome.get_mut("features") else {
        return Err("features: not a list of steps".into());
    };
    if add && let Some(&step) = wanted.first() {
        while steps.len() <= step {
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
        if add {
            for f in features {
                // Twice in one step would be a cycle for the sorter.
                if !list.iter().any(|v| held(v).as_ref() == Some(f)) {
                    list.push(J::Str(f.clone()));
                }
            }
        } else {
            list.retain(|v| held(v).is_none_or(|id| !features.contains(&id)));
        }
    }
    Ok(())
}

/// The `minecraft:gameplay/natural_mob_spawns` argument object on `biome`,
/// creating it when the biome does not yet have one.
fn natural_spawns(biome: &mut J) -> std::result::Result<&mut J, String> {
    if biome.get("attributes").is_none() {
        biome.set("attributes", J::Obj(Vec::new()));
    }
    let Some(attrs @ J::Obj(_)) = biome.get_mut("attributes") else {
        return Err("attributes: not an object".into());
    };
    const KEY: &str = "minecraft:gameplay/natural_mob_spawns";
    if attrs.get(KEY).is_none() {
        attrs.set(
            KEY,
            J::Obj(vec![
                (
                    "argument".into(),
                    J::Obj(vec![
                        ("spawn_costs".into(), J::Obj(Vec::new())),
                        ("spawns_by_category".into(), J::Obj(Vec::new())),
                    ]),
                ),
                ("modifier".into(), J::Str("overlay".into())),
            ]),
        );
    }
    let Some(nat) = attrs.get_mut(KEY) else {
        unreachable!()
    };
    if nat.get("argument").is_some() {
        match nat.get_mut("argument") {
            Some(arg @ J::Obj(_)) => Ok(arg),
            _ => Err(format!("{KEY}.argument: not an object")),
        }
    } else {
        match nat {
            J::Obj(_) => Ok(nat),
            _ => Err(format!("{KEY}: not an object")),
        }
    }
}

fn add_spawns(
    biome: &mut J,
    spawners: &[Spawner],
    entity_categories: &BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    let nat = natural_spawns(biome)?;
    if nat.get("spawns_by_category").is_none() {
        nat.set("spawns_by_category", J::Obj(Vec::new()));
    }
    let Some(by_cat @ J::Obj(_)) = nat.get_mut("spawns_by_category") else {
        return Err("spawns_by_category: not an object".into());
    };
    for s in spawners {
        let cat = entity_categories
            .get(&s.entity)
            .map_or("misc", String::as_str);
        if by_cat.get(cat).is_none() {
            by_cat.set(cat, J::Arr(Vec::new()));
        }
        let Some(J::Arr(list)) = by_cat.get_mut(cat) else {
            return Err(format!("spawns_by_category.{cat}: not a list"));
        };
        let count = if s.min_count == s.max_count {
            J::Num(s.min_count.into())
        } else {
            J::Obj(vec![
                ("type".into(), J::Str("minecraft:uniform".into())),
                ("min_inclusive".into(), J::Num(s.min_count.into())),
                ("max_inclusive".into(), J::Num(s.max_count.into())),
            ])
        };
        list.push(J::Obj(vec![
            ("type".into(), J::Str(s.entity.clone())),
            ("count".into(), count),
            ("weight".into(), J::Num(s.weight.into())),
        ]));
    }
    Ok(())
}

fn remove_spawns(biome: &mut J, entities: &[String]) -> std::result::Result<(), String> {
    let Some(nat) = biome
        .get_mut("attributes")
        .and_then(|a| a.get_mut("minecraft:gameplay/natural_mob_spawns"))
    else {
        return Ok(());
    };
    let nat = if nat.get("argument").is_some() {
        nat.get_mut("argument").unwrap()
    } else {
        nat
    };
    let Some(by_cat) = nat.get_mut("spawns_by_category") else {
        return Ok(());
    };
    let J::Obj(cats) = by_cat else {
        return Err("spawns_by_category: not an object".into());
    };
    for (cat, list) in cats {
        let J::Arr(entries) = list else {
            return Err(format!("spawns_by_category.{cat}: not a list"));
        };
        entries.retain(|entry| {
            entry
                .get("type")
                .and_then(J::as_str)
                .is_none_or(|t| !entities.contains(&full(t)))
        });
    }
    Ok(())
}

fn add_spawn_costs(
    biome: &mut J,
    entities: &[String],
    energy_budget: &Number,
    charge: &Number,
) -> std::result::Result<(), String> {
    let nat = natural_spawns(biome)?;
    if nat.get("spawn_costs").is_none() {
        nat.set("spawn_costs", J::Obj(Vec::new()));
    }
    let Some(costs @ J::Obj(_)) = nat.get_mut("spawn_costs") else {
        return Err("spawn_costs: not an object".into());
    };
    for entity in entities {
        if let J::Obj(entries) = costs {
            entries.retain(|(k, _)| k == entity || full(k) != *entity);
        }
        costs.set(
            entity,
            J::Obj(vec![
                ("energy_budget".into(), J::Num(energy_budget.clone())),
                ("charge".into(), J::Num(charge.clone())),
            ]),
        );
    }
    Ok(())
}

fn remove_spawn_costs(biome: &mut J, entities: &[String]) -> std::result::Result<(), String> {
    let Some(nat) = biome
        .get_mut("attributes")
        .and_then(|a| a.get_mut("minecraft:gameplay/natural_mob_spawns"))
    else {
        return Ok(());
    };
    let nat = if nat.get("argument").is_some() {
        nat.get_mut("argument").unwrap()
    } else {
        nat
    };
    let Some(costs) = nat.get_mut("spawn_costs") else {
        return Ok(());
    };
    let J::Obj(entries) = costs else {
        return Err("spawn_costs: not an object".into());
    };
    entries.retain(|(k, _)| !entities.contains(&full(k)));
    Ok(())
}
