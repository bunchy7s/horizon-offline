//! Where a pack keeps its random entity textures: the numbered variants of each texture and the
//! [`Rules`] that choose between them.
//!
//! A texture `assets/<ns>/textures/<path>.png` has its variants in the first of these folders
//! that has any, each mirroring `textures/` (the `mob/` ones mirror `textures/entity/`):
//!
//! 1. ETF's `assets/<ns>/etf/random/<path>`
//! 2. `assets/<ns>/optifine/random/<path>`
//! 3. the legacy `assets/<ns>/optifine/mob/`, then `assets/<ns>/mcpatcher/mob/`
//! 4. beside the texture itself under `textures/entity/` (ETF's extra). Horizon uses this
//!    folder only for a texture with rules there (its own `.properties` or the one of the name
//!    it extends), so that a pack without OptiFine files never changes a texture.
//!
//! Variant `N` (1 to [`MAX_INDEX`]) of `creeper` is `creeper<N>.png`; of a name ending in a
//! digit (`warden_pulsating_spots_2`), `warden_pulsating_spots_2.<N>.png`. Variant 1 is the
//! texture itself unless a `<name>1.png` exists. `<name>.properties` holds the rules; a texture
//! without its own uses the one of the name it extends in the same folder (`wolf_tame` and
//! `wolf_angry` use `wolf.properties`, `spider_eyes` uses `spider.properties`). With no rules
//! at all, an entity picks evenly between the texture and its variants 2, 3... up to the first
//! missing number.
//!
//! Textures have the names the entity atlas gives them: a vanilla texture its path under
//! `textures/` (`entity/zombie/zombie`), another namespace's `<ns>:<path>`, and a file outside
//! `textures/` its namespace and its path under the namespace
//! (`minecraft:optifine/random/entity/zombie/zombie2`). [`RandomTextures::files`] lists every
//! variant's file and name for the atlas, which skips those it already packs.
//!
//! Written from OptiFine's `random_entities.properties` and ETF's `RANDOM_GUIDE.md`; no mod's
//! source was read.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use crate::paths::files_under;
use crate::properties::Properties;
use crate::random::{MAX_INDEX, Rules};

/// Every texture a pack varies, by its atlas name.
#[derive(Debug, Default)]
pub struct RandomTextures {
    entries: HashMap<String, Entry>,
    rules: Vec<Rules>,
    /// Different for every scan: choices made from another scan's rules are stale.
    generation: u64,
}

/// One texture's variants.
#[derive(Debug, Default, PartialEq)]
pub struct Entry {
    /// Index into [`RandomTextures::rules`]: the rules that choose this texture's variant.
    pub rules: Option<usize>,
    variants: BTreeMap<u32, Variant>,
}

#[derive(Clone, Debug, PartialEq)]
struct Variant {
    /// Its atlas name.
    name: String,
    /// Its path from the resource root.
    file: String,
}

/// The folders under `assets/<ns>/` that hold variants, in priority order, and the folder under
/// `assets/<ns>/` each mirrors. The last is the texture's own folder.
const FOLDERS: [(&str, &str); 5] = [
    ("etf/random", "textures"),
    ("optifine/random", "textures"),
    ("optifine/mob", "textures/entity"),
    ("mcpatcher/mob", "textures/entity"),
    ("textures/entity", "textures/entity"),
];
/// The largest `.properties` file read, in bytes.
pub(crate) const MAX_PROPERTIES: u64 = 64 * 1024;

/// The texture's own folder, last in priority.
const BESIDE: usize = FOLDERS.len() - 1;

#[derive(Clone, Debug, Default)]
struct Source {
    properties: Option<String>,
    variants: BTreeMap<u32, Variant>,
}

impl RandomTextures {
    pub fn scan(root: &Path) -> Self {
        static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            generation,
            ..Self::scan_files(root)
        }
    }

    /// This scan's number, never 0 (the empty set's).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn scan_files(root: &Path) -> Self {
        let mut found: HashMap<String, [Source; FOLDERS.len()]> = HashMap::new();
        let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
            return Self::default();
        };
        let mut namespaces: Vec<String> = namespaces
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        namespaces.sort();
        for ns in &namespaces {
            let prefix = format!("assets/{ns}/");
            for (source, (folder, mirror)) in FOLDERS.iter().enumerate() {
                let folder = format!("{prefix}{folder}");
                for file in files_under(root, &folder, &[".png", ".properties"]) {
                    let inner = format!("{mirror}{}", &file[folder.len()..]);
                    let own = &file[prefix.len()..];
                    add(&mut found, source, ns, &inner, own, &file);
                }
            }
        }
        let mut out = Self::default();
        let mut rules_by_file: HashMap<String, usize> = HashMap::new();
        for (texture, sources) in &found {
            let Some(source) = sources
                .iter()
                .position(|s| s.properties.is_some() || !s.variants.is_empty())
            else {
                continue;
            };
            let chosen = &sources[source];
            // Its own rules, or those of the name it extends: `_`-parts cut from the file name.
            let (dir, mut name) = texture.rsplit_once('/').unwrap_or(("", texture));
            let mut rules_file = chosen.properties.clone();
            while rules_file.is_none()
                && let Some((shorter, _)) = name.rsplit_once('_')
            {
                name = shorter;
                rules_file = found
                    .get(&format!("{dir}/{name}"))
                    .and_then(|s| s[source].properties.clone());
            }
            if source == BESIDE && rules_file.is_none() {
                continue;
            }
            let rules = rules_file.map(|file| {
                if let Some(&id) = rules_by_file.get(&file) {
                    return id;
                }
                // A real rules file is a few KB, and parsing grows with the square of its size:
                // a bigger or unreadable one holds no rules, so the texture keeps its default.
                let path = root.join(&file);
                let text = fs::metadata(&path)
                    .is_ok_and(|m| m.len() <= MAX_PROPERTIES)
                    .then(|| fs::read(&path).ok())
                    .flatten()
                    .map(decode)
                    .unwrap_or_default();
                out.rules.push(Rules::parse(&Properties::parse(&text)));
                rules_by_file.insert(file, out.rules.len() - 1);
                out.rules.len() - 1
            });
            out.entries.insert(
                texture.clone(),
                Entry {
                    rules,
                    variants: chosen.variants.clone(),
                },
            );
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The variants of the texture with this atlas name.
    pub fn get(&self, texture: &str) -> Option<&Entry> {
        self.entries.get(texture)
    }

    pub fn rules(&self, id: usize) -> &Rules {
        &self.rules[id]
    }

    /// Every variant's file (its path from the resource root) and atlas name, for the entity
    /// atlas to pack. That includes variants beside their texture under `textures/entity/`,
    /// which the atlas may already walk.
    pub fn files(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries.values().flat_map(|e| {
            e.variants
                .values()
                .map(|v| (v.file.as_str(), v.name.as_str()))
        })
    }
}

impl Entry {
    /// The atlas name of variant `index`, or `None` for the texture itself: variant 0, 1 without
    /// a `<name>1.png`, and a variant whose file is missing.
    pub fn texture(&self, index: u32) -> Option<&str> {
        self.variants.get(&index).map(|v| v.name.as_str())
    }

    /// The even choice of an entity with this seed when no rules apply: the texture and its
    /// variants 2, 3... up to the first missing one.
    pub fn even(&self, seed: i32) -> u32 {
        let mut count = 1;
        while self.variants.contains_key(&(count + 1)) {
            count += 1;
        }
        seed.unsigned_abs() % count + 1
    }
}

/// UTF-8, or else ISO-8859-1 as Java's `Properties.load` reads a stream: a properties file
/// saved in Latin-1 keeps its rules.
pub(crate) fn decode(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes)
        .unwrap_or_else(|e| e.into_bytes().iter().map(|&b| char::from(b)).collect())
}

/// Records `file` (a path from the root), whose mirrored path under `assets/<ns>/` is `inner`
/// (`textures/entity/zombie/zombie2.png`) and own path there `own`: a texture's rules, or one of
/// its variants.
fn add(
    found: &mut HashMap<String, [Source; FOLDERS.len()]>,
    source: usize,
    ns: &str,
    inner: &str,
    own: &str,
    file: &str,
) {
    let Some((dir, name)) = inner.rsplit_once('/') else {
        return;
    };
    if let Some(stem) = name.strip_suffix(".properties") {
        let key = atlas_name(ns, &format!("{dir}/{stem}"));
        found.entry(key).or_default()[source].properties = Some(file.to_owned());
    } else if let Some((base, n)) = name.strip_suffix(".png").and_then(split_variant) {
        let key = atlas_name(ns, &format!("{dir}/{base}"));
        let variant = Variant {
            name: atlas_name(ns, own.strip_suffix(".png").unwrap_or(own)),
            file: file.to_owned(),
        };
        found.entry(key).or_default()[source]
            .variants
            .insert(n, variant);
    }
}

/// `creeper7` → (`creeper`, 7); `spots_2.3` → (`spots_2`, 3). Not a variant: `creeper`;
/// `spots_22` (a name ending in a digit numbers its variants after a `.`); `spots_2`, `bed_1`
/// (no texture name ends in `_`); `creeper02` (the docs number variants without leading
/// zeros); index 0 (the texture itself) and indices past [`MAX_INDEX`].
fn split_variant(stem: &str) -> Option<(&str, u32)> {
    let digits = stem.len() - stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (base, n) = stem.split_at(stem.len() - digits);
    if n.starts_with('0') {
        return None;
    }
    let n: u32 = n.parse().ok().filter(|n| (1..=MAX_INDEX).contains(n))?;
    match base.strip_suffix('.') {
        Some(b) if b.ends_with(|c: char| c.is_ascii_digit()) => Some((b, n)),
        None if !base.is_empty() && !base.ends_with(|c: char| c.is_ascii_digit() || c == '_') => {
            Some((base, n))
        }
        _ => None,
    }
}

/// A path under `assets/<ns>/` as the entity atlas names it.
pub(crate) fn atlas_name(ns: &str, inner: &str) -> String {
    match inner.strip_prefix("textures/") {
        Some(path) if ns == "minecraft" => path.to_owned(),
        Some(path) => format!("{ns}:{path}"),
        None => format!("{ns}:{inner}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::random::Facts;

    fn pack(files: &[(&str, &str)]) -> (std::path::PathBuf, RandomTextures) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-random-textures-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        let found = RandomTextures::scan(&root);
        (root, found)
    }

    const RANDOM: &str = "assets/minecraft/optifine/random/entity";

    #[test]
    fn variants_and_rules_by_texture() {
        let (root, t) = pack(&[
            (&format!("{RANDOM}/creeper/creeper2.png"), ""),
            (&format!("{RANDOM}/creeper/creeper3.png"), ""),
            (&format!("{RANDOM}/creeper/creeper5.png"), ""),
            (
                &format!("{RANDOM}/warden/warden_pulsating_spots_2.2.png"),
                "",
            ),
            (
                &format!("{RANDOM}/warden/warden_pulsating_spots_22.png"),
                "",
            ),
            (
                &format!("{RANDOM}/wolf/wolf.properties"),
                "textures.1=2\nbaby.1=true",
            ),
            (&format!("{RANDOM}/wolf/wolf2.png"), ""),
            (&format!("{RANDOM}/wolf/wolf_tame2.png"), ""),
            ("assets/minecraft/textures/entity/creeper/creeper.png", ""),
        ]);
        let creeper = t.get("entity/creeper/creeper").unwrap();
        assert_eq!(creeper.rules, None);
        assert_eq!(
            creeper.texture(3),
            Some("minecraft:optifine/random/entity/creeper/creeper3")
        );
        assert_eq!(
            creeper.texture(1),
            None,
            "no creeper1.png: the texture itself"
        );
        assert_eq!(creeper.texture(4), None, "missing");
        // No rules: an even choice of 1, 2 and 3; 5 is past the gap at 4.
        assert_eq!(creeper.even(0), 1);
        assert_eq!(creeper.even(4), 2);
        assert_eq!(creeper.even(-5), 3);
        let spots = t.get("entity/warden/warden_pulsating_spots_2").unwrap();
        assert_eq!(
            spots.texture(2),
            Some("minecraft:optifine/random/entity/warden/warden_pulsating_spots_2.2")
        );
        assert_eq!(spots.texture(22), None, "spots_22 is no variant of spots_2");
        // wolf_tame has no rules of its own: it uses wolf.properties.
        let wolf = t.get("entity/wolf/wolf").unwrap();
        let tame = t.get("entity/wolf/wolf_tame").unwrap();
        assert!(wolf.rules.is_some());
        assert_eq!(tame.rules, wolf.rules);
        let baby = Facts {
            baby: Some(true),
            ..Default::default()
        };
        assert_eq!(t.rules(tame.rules.unwrap()).select(&baby, 0), 2);
        // spots_22 is no variant at all: no texture is named `warden_pulsating_spots_`.
        assert_eq!(t.files().count(), 6);
        assert!(t.files().any(|(file, name)| {
            file == "assets/minecraft/optifine/random/entity/wolf/wolf_tame2.png"
                && name == "minecraft:optifine/random/entity/wolf/wolf_tame2"
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folders_in_priority_order() {
        let (root, t) = pack(&[
            ("assets/minecraft/etf/random/entity/pig/pig2.png", ""),
            (&format!("{RANDOM}/pig/pig2.png"), ""),
            (&format!("{RANDOM}/pig/pig3.png"), ""),
            ("assets/minecraft/mcpatcher/mob/cow/cow2.png", ""),
            ("assets/minecraft/optifine/mob/sheep/sheep2.png", ""),
            ("assets/mymod/optifine/random/entity/bug/bug2.png", ""),
            // Beside the vanilla texture: only with a properties file.
            ("assets/minecraft/textures/entity/fox/fox2.png", ""),
            (
                "assets/minecraft/textures/entity/cat/cat.properties",
                "textures.1=2",
            ),
            ("assets/minecraft/textures/entity/cat/cat2.png", ""),
        ]);
        let pig = t.get("entity/pig/pig").unwrap();
        assert_eq!(pig.texture(2), Some("minecraft:etf/random/entity/pig/pig2"));
        assert_eq!(pig.texture(3), None, "only the ETF folder is used");
        assert_eq!(
            t.get("entity/cow/cow").unwrap().texture(2),
            Some("minecraft:mcpatcher/mob/cow/cow2")
        );
        assert_eq!(
            t.get("entity/sheep/sheep").unwrap().texture(2),
            Some("minecraft:optifine/mob/sheep/sheep2")
        );
        assert_eq!(
            t.get("mymod:entity/bug/bug").unwrap().texture(2),
            Some("mymod:optifine/random/entity/bug/bug2")
        );
        assert!(t.get("entity/fox/fox").is_none());
        let cat = t.get("entity/cat/cat").unwrap();
        assert_eq!(cat.texture(2), Some("entity/cat/cat2"));
        assert!(cat.rules.is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_folders_after_optifine_random() {
        let both = [
            (format!("{RANDOM}/pig/pig2.png"), ""),
            ("assets/minecraft/optifine/mob/pig/pig2.png".into(), ""),
            ("assets/minecraft/mcpatcher/mob/pig/pig2.png".into(), ""),
        ];
        let files: Vec<(&str, &str)> = both.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let (root, t) = pack(&files);
        let pig = t.get("entity/pig/pig").unwrap();
        assert_eq!(
            pig.texture(2),
            Some("minecraft:optifine/random/entity/pig/pig2")
        );
        let _ = fs::remove_dir_all(root);
        let (root, t) = pack(&files[1..]);
        let pig = t.get("entity/pig/pig").unwrap();
        assert_eq!(pig.texture(2), Some("minecraft:optifine/mob/pig/pig2"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inherited_rules_stay_in_their_folder() {
        let (root, t) = pack(&[
            (
                &format!("{RANDOM}/spider/spider.properties"),
                "textures.1=2",
            ),
            (&format!("{RANDOM}/spider/spider_eyes2.png"), ""),
            (&format!("{RANDOM}/a/a.properties"), "textures.1=2"),
            (&format!("{RANDOM}/a/a_b_c2.png"), ""),
            // A flat legacy zombie's rules don't reach the zombie villager's folder.
            (
                "assets/minecraft/optifine/mob/zombie.properties",
                "textures.1=2",
            ),
            (
                "assets/minecraft/optifine/mob/zombie_villager/zombie_villager2.png",
                "",
            ),
            // Nor do another folder's.
            (&format!("{RANDOM}/wolf/wolf.properties"), "textures.1=2"),
            ("assets/minecraft/etf/random/entity/wolf/wolf_tame2.png", ""),
            (&format!("{RANDOM}/creeper/creeper1.png"), ""),
            (&format!("{RANDOM}/creeper/creeper2.png"), ""),
        ]);
        let spider = t.get("entity/spider/spider").map(|e| e.rules);
        assert!(spider.is_some_and(|r| r.is_some()));
        assert_eq!(
            t.get("entity/spider/spider_eyes").unwrap().rules,
            spider.unwrap()
        );
        assert!(t.get("entity/a/a_b_c").unwrap().rules.is_some());
        assert_eq!(
            t.get("entity/zombie_villager/zombie_villager")
                .unwrap()
                .rules,
            None
        );
        assert_eq!(t.get("entity/wolf/wolf_tame").unwrap().rules, None);
        // creeper1.png replaces variant 1.
        let creeper = t.get("entity/creeper/creeper").unwrap();
        assert_eq!(
            creeper.texture(1),
            Some("minecraft:optifine/random/entity/creeper/creeper1")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn beside_the_texture_with_rules() {
        const WOLF: &str = "assets/minecraft/textures/entity/wolf";
        let (root, t) = pack(&[
            (&format!("{WOLF}/wolf.properties"), "textures.1=5"),
            (&format!("{WOLF}/wolf.png"), ""),
            (&format!("{WOLF}/wolf2.png"), ""),
            (&format!("{WOLF}/wolf5.png"), ""),
            (&format!("{WOLF}/wolf_tame2.png"), ""),
        ]);
        // A rule may name a variant past a gap.
        assert_eq!(
            t.get("entity/wolf/wolf").unwrap().texture(5),
            Some("entity/wolf/wolf5")
        );
        let tame = t.get("entity/wolf/wolf_tame").unwrap();
        assert!(
            tame.rules.is_some(),
            "wolf_tame beside uses wolf.properties"
        );
        let _ = fs::remove_dir_all(root);
        // The texture's own folder comes after the others.
        let (root, t) = pack(&[
            (&format!("{WOLF}/wolf.properties"), "textures.1=2"),
            (&format!("{WOLF}/wolf2.png"), ""),
            (&format!("{RANDOM}/wolf/wolf2.png"), ""),
        ]);
        let wolf = t.get("entity/wolf/wolf").unwrap();
        assert_eq!(
            wolf.texture(2),
            Some("minecraft:optifine/random/entity/wolf/wolf2")
        );
        assert_eq!(wolf.rules, None, "optifine/random's folder holds no rules");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn an_oversized_rules_file_keeps_the_default() {
        let big = format!("textures.1=2\n#{}\n", "x".repeat(MAX_PROPERTIES as usize));
        let (root, t) = pack(&[
            (&format!("{RANDOM}/wolf/wolf.properties"), &big),
            (&format!("{RANDOM}/wolf/wolf2.png"), ""),
        ]);
        let wolf = t.get("entity/wolf/wolf").unwrap();
        let rules = t.rules(wolf.rules.expect("rules, empty ones"));
        assert_eq!(
            rules.select(&Facts::default(), 1),
            1,
            "not textures.1=2, not even"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn latin1_rules_are_read() {
        let root =
            std::env::temp_dir().join(format!("horizon-random-latin1-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join(RANDOM).join("creeper");
        fs::create_dir_all(&dir).unwrap();
        // `Ébène` in ISO-8859-1: 0xC9 and 0xE8 are no UTF-8.
        fs::write(
            dir.join("creeper.properties"),
            b"textures.1=2\nname.1=\xc9b\xe8ne\n",
        )
        .unwrap();
        fs::write(dir.join("creeper2.png"), b"").unwrap();
        let t = RandomTextures::scan(&root);
        let id = t.get("entity/creeper/creeper").unwrap().rules.unwrap();
        let named = |name| Facts {
            name: Some(name),
            ..Default::default()
        };
        assert_eq!(t.rules(id).select(&named("Ébène"), 0), 2);
        assert_eq!(t.rules(id).select(&named("Ebene"), 0), 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn no_files_no_entries() {
        let (root, t) = pack(&[("assets/minecraft/textures/entity/pig/pig.png", "")]);
        assert!(t.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn variant_names() {
        assert_eq!(split_variant("creeper12"), Some(("creeper", 12)));
        assert_eq!(split_variant("spots_2.3"), Some(("spots_2", 3)));
        assert_eq!(split_variant("creeper"), None);
        assert_eq!(
            split_variant("spots_2"),
            None,
            "no texture is named `spots_`"
        );
        assert_eq!(split_variant("creeper1"), Some(("creeper", 1)));
        assert_eq!(split_variant("creeper0"), None, "0 is the texture itself");
        assert_eq!(split_variant("creeper02"), None);
        assert_eq!(split_variant("spots_2.02"), None);
        assert_eq!(split_variant("2"), None);
        assert_eq!(split_variant("a.3"), None, "a `.` only follows a digit");
        assert_eq!(split_variant("x99999"), None);
    }
}
