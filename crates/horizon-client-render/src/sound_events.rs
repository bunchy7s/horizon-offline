//! `sounds.json` sound events as vanilla's `SoundManager.Preparations` builds
//! them: `WeighedSoundEvents` holding files and references to other events
//! (whose weight is the referenced event's total weight, resolved lazily), the
//! `replace` and `subtitle` fields, files that do not exist being dropped, and
//! `WeighedSoundEvents.getSound` drawing `nextInt(totalWeight)` from the sound
//! instance's random.
use horizon_core::random::RandomSource;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

/// `SoundManager.EMPTY_SOUND_LOCATION`.
pub const EMPTY: &str = "minecraft:empty";
/// `SoundManager.INTENTIONALLY_EMPTY_SOUND_LOCATION`.
pub const INTENTIONALLY_EMPTY: &str = "minecraft:intentionally_empty";

/// `SampledFloat` as sounds use it: constants and `MultipliedFloats`.
#[derive(Clone, Debug, PartialEq)]
pub enum Sampled {
    Constant(f32),
    Product(Vec<Sampled>),
}
impl Sampled {
    /// `SampledFloat.sample`: constants draw nothing (sounds.json only has
    /// constants, but the random stays part of the call as in vanilla).
    #[allow(clippy::only_used_in_recursion)]
    pub fn sample(&self, random: &mut impl RandomSource) -> f32 {
        match self {
            Self::Constant(v) => *v,
            Self::Product(values) => {
                let mut result = 1.0f32;
                for v in values {
                    result *= v.sample(random);
                }
                result
            }
        }
    }
}
/// `Sound`.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// The file identifier (`namespace:path`, under `sounds/` with `.ogg`).
    pub location: String,
    pub volume: Sampled,
    pub pitch: Sampled,
    pub weight: i32,
    /// `Sound.Type.SOUND_EVENT` (a reference to another event).
    pub event: bool,
    pub stream: bool,
    pub preload: bool,
    pub attenuation_distance: i32,
    /// The `EMPTY_SOUND` / `INTENTIONALLY_EMPTY_SOUND` singletons (vanilla
    /// compares them by identity).
    pub sentinel: Sentinel,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sentinel {
    #[default]
    None,
    Empty,
    IntentionallyEmpty,
}
impl Sound {
    fn file(location: &str) -> Self {
        Self {
            location: location.to_owned(),
            volume: Sampled::Constant(1.0),
            pitch: Sampled::Constant(1.0),
            weight: 1,
            event: false,
            stream: false,
            preload: false,
            attenuation_distance: 16,
            sentinel: Sentinel::None,
        }
    }
    /// `SoundManager.EMPTY_SOUND`.
    pub fn empty() -> Self {
        Self {
            sentinel: Sentinel::Empty,
            ..Self::file(EMPTY)
        }
    }
    /// `SoundManager.INTENTIONALLY_EMPTY_SOUND`.
    pub fn intentionally_empty() -> Self {
        Self {
            sentinel: Sentinel::IntentionallyEmpty,
            ..Self::file(INTENTIONALLY_EMPTY)
        }
    }
    /// `Sound.getAttenuationDistance(volume)`.
    pub fn attenuation_distance(&self, volume: f32) -> f32 {
        volume.max(1.0) * self.attenuation_distance as f32
    }
    /// `Sound.getPath`: the resource path (`namespace`, `sounds/<path>.ogg`).
    pub fn path(&self) -> (&str, String) {
        let (namespace, path) = split(&self.location);
        (namespace, format!("sounds/{path}.ogg"))
    }
}
/// `WeighedSoundEvents`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeighedSoundEvents {
    /// Files, or references to events (`Sound.event`).
    pub sounds: Vec<Sound>,
    /// The subtitle translation key.
    pub subtitle: Option<String>,
}
/// A `sounds.json` entry: event, `replace`, subtitle and sounds.
type Registration = (String, bool, Option<String>, Vec<Sound>);
/// Parse a `sounds.json`; `None` when Gson would reject the whole file (a
/// malformed entry anywhere in it).
fn parse_file(namespace: &str, text: &[u8]) -> Option<Vec<Registration>> {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(text) else {
        return None;
    };
    let mut parsed = Vec::new();
    for (name, def) in &map {
        let Value::Object(object) = def else {
            return None;
        };
        let replace = json_bool(object.get("replace").unwrap_or(&Value::Null), false);
        let subtitle = match object.get("subtitle") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return None,
        };
        let mut sounds = Vec::new();
        if let Some(list) = object.get("sounds") {
            let Value::Array(list) = list else {
                return None;
            };
            for s in list {
                sounds.push(parse_sound(s)?);
            }
        }
        parsed.push((format!("{namespace}:{name}"), replace, subtitle, sounds));
    }
    Some(parsed)
}
/// Whether vanilla's `SoundManager` reads this `sounds.json` at all.
pub fn is_valid_file(text: &[u8]) -> bool {
    parse_file("minecraft", text).is_some()
}
/// The sound event registry (`SoundManager.registry`).
#[derive(Clone, Debug, Default)]
pub struct SoundRegistry {
    events: HashMap<String, WeighedSoundEvents>,
}
/// `Identifier.parse` with the default namespace.
fn split(id: &str) -> (&str, &str) {
    match id.split_once(':') {
        Some((namespace, path)) => (
            if namespace.is_empty() {
                "minecraft"
            } else {
                namespace
            },
            path,
        ),
        None => ("minecraft", id),
    }
}
pub fn normalize(id: &str) -> String {
    let (namespace, path) = split(id);
    format!("{namespace}:{path}")
}
/// `GsonHelper.getAsFloat`: Gson parses the number's text as a float.
fn json_float(v: &Value, fallback: f32) -> f32 {
    match v {
        Value::Number(n) => n.to_string().parse().unwrap_or(fallback),
        Value::String(s) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}
fn json_int(v: &Value, fallback: i32) -> i32 {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(|i| i as i32)
            .or_else(|| n.as_f64().map(|f| f as i32))
            .unwrap_or(fallback),
        Value::String(s) => s.parse().unwrap_or(fallback),
        _ => fallback,
    }
}
pub(crate) fn json_bool(v: &Value, fallback: bool) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => s == "true",
        _ => fallback,
    }
}
/// `SoundEventRegistrationSerializer.getSound`; None for an invalid entry
/// (vanilla rejects the whole file).
fn parse_sound(v: &Value) -> Option<Sound> {
    if let Value::String(name) = v {
        return Some(Sound::file(&normalize(name)));
    }
    let name = v.get("name")?.as_str()?;
    let event = match v.get("type") {
        None => false,
        Some(t) => match t.as_str()? {
            "file" => false,
            "event" => true,
            _ => return None,
        },
    };
    let volume = json_float(v.get("volume").unwrap_or(&Value::Null), 1.0);
    let pitch = json_float(v.get("pitch").unwrap_or(&Value::Null), 1.0);
    let weight = json_int(v.get("weight").unwrap_or(&Value::Null), 1);
    if !(volume > 0.0 && pitch > 0.0 && weight > 0) {
        return None;
    }
    Some(Sound {
        location: normalize(name),
        volume: Sampled::Constant(volume),
        pitch: Sampled::Constant(pitch),
        weight,
        event,
        stream: json_bool(v.get("stream").unwrap_or(&Value::Null), false),
        preload: json_bool(v.get("preload").unwrap_or(&Value::Null), false),
        attenuation_distance: json_int(v.get("attenuation_distance").unwrap_or(&Value::Null), 16),
        sentinel: Sentinel::None,
    })
}
/// The files under a namespace's `sounds/`, as `sounds/<path>` with `/`.
fn sound_files(namespace: &Path) -> HashSet<String> {
    let mut files = HashSet::new();
    let mut stack = vec![namespace.join("sounds")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            // The listing's own type; a link is followed, as `is_file` does.
            let Ok(kind) = e.file_type() else {
                continue;
            };
            let (dir, file) = if kind.is_symlink() {
                (path.is_dir(), path.is_file())
            } else {
                (kind.is_dir(), kind.is_file())
            };
            if dir {
                stack.push(path);
            } else if file && let Ok(rel) = path.strip_prefix(namespace) {
                files.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    files
}
impl SoundRegistry {
    /// Every `assets/<namespace>/sounds.json` under a resource root, files
    /// validated against `assets/<namespace>/sounds/`.
    pub fn load(root: &Path) -> Self {
        let assets = root.join("assets");
        let mut namespaces: Vec<String> = fs::read_dir(&assets)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .collect();
        namespaces.sort();
        // Each namespace's sound files are listed once, as the resource manager
        // lists a pack, instead of one file lookup per sound (thousands).
        let files: HashMap<&str, HashSet<String>> = namespaces
            .iter()
            .map(|ns| (ns.as_str(), sound_files(&assets.join(ns))))
            .collect();
        let exists = |ns: &str, path: &str| files.get(ns).is_some_and(|f| f.contains(path));
        let mut registry = Self::default();
        for namespace in &namespaces {
            let Ok(text) = fs::read(assets.join(namespace).join("sounds.json")) else {
                continue;
            };
            registry.add_file(namespace, &text, &exists);
        }
        registry
    }
    /// No sound event at all: the resources hold no `sounds.json` (sounds not
    /// imported from the player's launcher when the game was prepared).
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    /// One `sounds.json` (`Preparations.handleRegistration` for each entry).
    /// `exists(namespace, path)` says whether a sound file is present.
    pub fn add_file(&mut self, namespace: &str, text: &[u8], exists: &dyn Fn(&str, &str) -> bool) {
        let Some(parsed) = parse_file(namespace, text) else {
            return;
        };
        for (location, replace, subtitle, sounds) in parsed {
            let entry = match self.events.get_mut(&location) {
                Some(entry) if !replace => entry,
                _ => {
                    self.events.insert(
                        location.clone(),
                        WeighedSoundEvents {
                            sounds: Vec::new(),
                            subtitle,
                        },
                    );
                    self.events.get_mut(&location).expect("inserted")
                }
            };
            for sound in sounds {
                if !sound.event {
                    let (ns, path) = sound.path();
                    if !exists(ns, &path) {
                        continue;
                    }
                }
                entry.sounds.push(sound);
            }
        }
    }
    pub fn get(&self, event: &str) -> Option<&WeighedSoundEvents> {
        self.events.get(event)
    }
    pub fn contains(&self, event: &str) -> bool {
        self.events.contains_key(event)
    }
    /// Every registered event id.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.events.keys()
    }
    /// `Weighted.getWeight` of one entry.
    fn entry_weight(&self, sound: &Sound, depth: u32) -> i32 {
        if !sound.event {
            return sound.weight;
        }
        if depth > 64 {
            return 0;
        }
        self.events
            .get(&sound.location)
            .map_or(0, |e| self.total(e, depth + 1))
    }
    fn total(&self, event: &WeighedSoundEvents, depth: u32) -> i32 {
        event
            .sounds
            .iter()
            .fold(0i32, |sum, s| sum.wrapping_add(self.entry_weight(s, depth)))
    }
    /// `WeighedSoundEvents.getWeight`.
    pub fn weight(&self, event: &str) -> i32 {
        self.events.get(event).map_or(0, |e| self.total(e, 0))
    }
    /// `WeighedSoundEvents.getSound`: None when the event is not registered.
    pub fn sound(&self, event: &str, random: &mut impl RandomSource) -> Option<Sound> {
        let e = self.events.get(event)?;
        Some(self.pick(e, random, 0))
    }
    fn pick(
        &self,
        event: &WeighedSoundEvents,
        random: &mut impl RandomSource,
        depth: u32,
    ) -> Sound {
        let weight = self.total(event, depth);
        if event.sounds.is_empty() || weight == 0 {
            return Sound::empty();
        }
        let Ok(mut index) = random.next_int_bound(weight) else {
            return Sound::empty();
        };
        for sound in &event.sounds {
            index -= self.entry_weight(sound, depth);
            if index < 0 {
                if !sound.event {
                    return sound.clone();
                }
                // The anonymous reference Weighted: the referenced event's
                // sound with this entry's volume, pitch and stream folded in.
                let Some(inner) = self.events.get(&sound.location) else {
                    return Sound::empty();
                };
                if depth > 64 {
                    return Sound::empty();
                }
                let wrapped = self.pick(inner, random, depth + 1);
                return Sound {
                    location: wrapped.location,
                    volume: Sampled::Product(vec![wrapped.volume, sound.volume.clone()]),
                    pitch: Sampled::Product(vec![wrapped.pitch, sound.pitch.clone()]),
                    weight: sound.weight,
                    event: false,
                    stream: wrapped.stream || sound.stream,
                    preload: wrapped.preload,
                    attenuation_distance: wrapped.attenuation_distance,
                    sentinel: Sentinel::None,
                };
            }
        }
        Sound::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn load_keeps_the_sound_files_a_root_holds() {
        let root = std::env::temp_dir().join(format!("horizon-sounds-{}", std::process::id()));
        let ns = root.join("assets/minecraft");
        fs::create_dir_all(ns.join("sounds/block/note")).unwrap();
        fs::write(ns.join("sounds/block/note/harp.ogg"), b"").unwrap();
        fs::write(
            ns.join("sounds.json"),
            br#"{"note": {"sounds": ["block/note/harp", "block/note/gone"]}}"#,
        )
        .unwrap();
        let r = SoundRegistry::load(&root);
        let _ = fs::remove_dir_all(&root);
        let sounds = &r.get("minecraft:note").unwrap().sounds;
        let kept: Vec<&str> = sounds.iter().map(|s| s.location.as_str()).collect();
        assert_eq!(kept, ["minecraft:block/note/harp"]);
    }

    #[test]
    fn references_weigh_as_much_as_their_event() {
        let mut r = SoundRegistry::default();
        let json = br#"{
            "a": {"sounds": ["x", "y", "z"]},
            "b": {"sounds": [{"name": "a", "type": "event", "weight": 9, "volume": 0.5}, "w"], "subtitle": "s.b"}
        }"#;
        r.add_file("minecraft", json, &|_, _| true);
        assert_eq!(r.weight("minecraft:b"), 4);
        let mut random = LegacyRandom::new(1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let s = r.sound("minecraft:b", &mut random).unwrap();
            if s.location != "minecraft:w" {
                assert_eq!(s.volume.sample(&mut random), 0.5);
            }
            seen.insert(s.location);
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(
            r.get("minecraft:b").unwrap().subtitle.as_deref(),
            Some("s.b")
        );
    }

    #[test]
    fn missing_files_are_dropped_and_empty_events_give_the_empty_sound() {
        let mut r = SoundRegistry::default();
        r.add_file("minecraft", br#"{"a": {"sounds": ["gone"]}}"#, &|_, _| {
            false
        });
        assert_eq!(r.weight("minecraft:a"), 0);
        let mut random = LegacyRandom::new(1);
        assert_eq!(r.sound("minecraft:a", &mut random).unwrap(), Sound::empty());
        assert!(r.sound("minecraft:b", &mut random).is_none());
    }
}
