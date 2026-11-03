//! Random entity textures (`optifine/random/entity/<path>.properties`, the legacy `mob/` folder):
//! the numbered rules that pick which of a texture's numbered variants an entity wears.
//!
//! A rule holds `textures.<n>` (or `skins.<n>`), the variant indices it chooses from (1 is the
//! default texture), their optional `weights.<n>`, and conditions on the entity. Rules are tried
//! in order of `<n>`; the first whose conditions all hold picks one of its textures from the
//! entity's seed, so the same entity keeps the same texture. No rule matching means the default.
//!
//! Conditions: `biomes`, `heights` (and the legacy `minHeight`/`maxHeight`), `name` (and ETF's
//! `names`), `professions`, `colors` (`collarColors`), `baby`, `health` (in points or percent),
//! `moonPhase`, `dayTime`, `weather`, `sizes` and `blocks`. The spawn-time ones (biome, height,
//! block, moon, time, weather) are the caller's to fix when the entity is first seen; the others
//! can change, and [`Rules::updates`] says whether a rule file reads any of them.
//!
//! Written from OptiFine's `random_entities.properties` and ETF's `RANDOM_GUIDE.md` and
//! `random_entities.properties` docs; no mod's source was read.

use crate::matchers::{BlockMatch, Name, RangeList, namespaced};
use crate::properties::Properties;

/// The most variants one rule lists (`textures.1=1-100000` stays small).
const MAX_TEXTURES: usize = 256;
/// The most rules one file holds.
pub const MAX_RULES: usize = 1024;

/// What a rule can ask about an entity. `None` is a fact the entity doesn't have (a creeper's
/// profession): a rule asking for it doesn't match.
#[derive(Clone, Debug, Default)]
pub struct Facts<'a> {
    /// Where the entity was first seen: its biome (`minecraft:plains`) and block height.
    pub biome: Option<&'a str>,
    pub y: Option<i32>,
    /// The block it stood on and the block it stood in when first seen (ETF checks both); empty
    /// when unknown, which fails a `blocks` rule.
    pub blocks: &'a [Block<'a>],
    /// Its custom name, with `§` formatting codes where the name has them (rules may ask for
    /// `\u00a74Pinky`).
    pub name: Option<&'a str>,
    /// Villager profession (`minecraft:farmer`) and level.
    pub profession: Option<(&'a str, i32)>,
    /// Collar, wool, carpet or shell colour, as a dye name (`light_blue`).
    pub color: Option<&'a str>,
    pub baby: Option<bool>,
    /// Health and max health.
    pub health: Option<(f32, f32)>,
    /// When first seen: moon phase (0-7), time of day (0-23999) and weather.
    pub moon_phase: Option<i32>,
    pub day_time: Option<i32>,
    pub weather: Option<Weather>,
    /// Slime, magma cube or phantom size as the docs number it: 0 small, 1 medium, 3 big.
    pub size: Option<i32>,
}

/// A block and the values of its state properties.
#[derive(Clone, Copy, Debug)]
pub struct Block<'a> {
    pub id: &'a str,
    pub states: &'a [(&'a str, &'a str)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    Clear,
    Rain,
    Thunder,
}

/// One `.properties` file's rules, in order.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug, Default)]
struct Rule {
    /// Variant indices; 1 is the default texture.
    textures: Vec<u32>,
    /// Same length as `textures`, or empty for an even choice.
    weights: Vec<u32>,
    /// ETF's `seedOffset`: added to the entity's seed.
    seed_offset: i32,
    /// Ids (`plains`, `mymod:bog`), or ETF's `pattern:`/`regex:` forms; each with `!`.
    biomes: Vec<(Name, bool)>,
    heights: Option<RangeList>,
    /// Each with whether it is negated (`!`).
    names: Vec<(Name, bool)>,
    professions: Vec<(String, Option<RangeList>)>,
    colors: Option<Vec<String>>,
    baby: Option<bool>,
    health: Option<(RangeList, bool)>,
    moon_phase: Option<RangeList>,
    day_time: Option<RangeList>,
    weather: Option<Vec<Weather>>,
    sizes: Option<RangeList>,
    blocks: Vec<(BlockMatch, bool)>,
}

/// The conditions a rule may hold, as `<key>.<n>`. A rule with any other (`nbt.<n>.*`, ETF's
/// `teams`, `items`...) is skipped rather than applied to every entity.
const KEYS: &[&str] = &[
    "textures",
    "skins",
    "weights",
    "seedOffset",
    "seedSource",
    "biomes",
    "heights",
    "minHeight",
    "maxHeight",
    "name",
    "names",
    "professions",
    "colors",
    "collarColors",
    "baby",
    "health",
    "moonPhase",
    "dayTime",
    "weather",
    "sizes",
    "blocks",
];

impl Rules {
    pub fn parse(props: &Properties) -> Self {
        let mut numbers: Vec<u32> = ["textures.", "skins."]
            .iter()
            .flat_map(|p| props.with_prefix(p))
            .filter_map(|(n, _)| n.parse().ok())
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        // Each rule reads every entry: a file of thousands of rules stops at the first MAX_RULES.
        numbers.truncate(MAX_RULES);
        let rules = numbers
            .into_iter()
            .filter_map(|n| Rule::parse(props, n))
            .collect();
        Self { rules }
    }

    /// The first matching rule's pick, or 1 when none matches. [`Rules::matching`] tells a miss
    /// (the texture itself, as is a rule's 0) from a pick of 1, which `<name>1.png` replaces.
    pub fn select(&self, facts: &Facts, seed: i32) -> u32 {
        self.matching(facts, seed).unwrap_or(1)
    }

    /// The first matching rule's pick, `None` when no rule matches: the texture itself, even
    /// where a `<name>1.png` replaces variant 1 (ETF).
    pub fn matching(&self, facts: &Facts, seed: i32) -> Option<u32> {
        self.rules
            .iter()
            .find(|r| r.matches(facts))
            .map(|r| r.pick(seed))
    }

    /// Whether any rule reads a fact that changes while the entity lives (name, profession,
    /// colour, baby, health), so the choice should be checked again now and then.
    pub fn updates(&self) -> bool {
        self.rules.iter().any(|r| {
            !r.names.is_empty()
                || !r.professions.is_empty()
                || r.colors.is_some()
                || r.baby.is_some()
                || r.health.is_some()
        })
    }

    /// The highest variant index any rule lists (at most [`MAX_INDEX`]).
    pub fn max_texture(&self) -> u32 {
        self.rules
            .iter()
            .flat_map(|r| r.textures.iter().copied())
            .max()
            .unwrap_or(1)
    }
}

impl Rule {
    /// `None` when the rule lists no textures, or holds a condition that doesn't parse or isn't
    /// supported: such a rule never matches, instead of matching every entity.
    fn parse(props: &Properties, n: u32) -> Option<Self> {
        // This rule's keys, `<key>.<n>` or `nbt.<n>.<path>` (a path may end in a list index:
        // `nbt.2.Tags.1`): one unknown, or an empty condition (`biomes.1=`), skips the rule. An
        // empty `weights` is only no weights.
        let unusable = props.entries.iter().any(|(key, value)| {
            let mut parts = key.splitn(3, '.');
            let (Some(head), Some(index)) = (parts.next(), parts.next()) else {
                return false;
            };
            index.parse::<u32>() == Ok(n)
                && (!KEYS.contains(&head)
                    || (value.is_empty() && !matches!(head, "textures" | "skins" | "weights")))
        });
        let get = |key: &str| props.get(&format!("{key}.{n}"));
        // ETF's `seedSource`: only the entity's own seed is read.
        if unusable || get("seedSource").is_some_and(|v| v != "entity") {
            return None;
        }
        // `Some(None)` for a missing key, `None` for one that doesn't parse.
        fn read<T>(
            value: Option<&str>,
            parse: impl FnOnce(&str) -> Option<T>,
        ) -> Option<Option<T>> {
            match value {
                None => Some(None),
                Some(v) => parse(v).map(Some),
            }
        }
        let textures = expand(&RangeList::parse(get("textures").or(get("skins"))?)?);
        if textures.is_empty() {
            return None;
        }
        let weights = get("weights")
            .and_then(RangeList::parse)
            .map(|w| w.0.iter().map(|&(lo, _)| lo.max(0) as u32).collect())
            .filter(|w: &Vec<u32>| w.len() == textures.len() && w.iter().any(|&w| w > 0))
            .unwrap_or_default();
        let ranges = |key| read(get(key), RangeList::parse);
        let heights = match ranges("heights")? {
            Some(h) => Some(h),
            None => {
                let min = read(get("minHeight"), |v| v.parse().ok())?;
                let max = read(get("maxHeight"), |v| v.parse().ok())?;
                (min.is_some() || max.is_some())
                    .then(|| RangeList(vec![(min.unwrap_or(i32::MIN), max.unwrap_or(i32::MAX))]))
            }
        };
        let mut names = Vec::new();
        if let Some(v) = get("name") {
            let (v, negated) = negation(v)?;
            names.push((Name::parse(v)?, negated));
        }
        for (word, negated) in quoted_words(get("names").unwrap_or("")) {
            if word.is_empty() {
                return None;
            }
            names.push((Name::parse(word)?, negated));
        }
        Some(Self {
            textures,
            weights,
            seed_offset: read(get("seedOffset"), |v| v.parse().ok())?.unwrap_or(0),
            biomes: read(get("biomes"), |v| {
                v.split_whitespace()
                    .map(|b| {
                        let (b, negated) = negation(b)?;
                        let name = match Name::parse(b)? {
                            Name::Exact(id) => Name::Exact(namespaced(&id)),
                            pattern => pattern,
                        };
                        Some((name, negated))
                    })
                    .collect()
            })?
            .unwrap_or_default(),
            heights,
            names,
            professions: read(get("professions"), parse_professions)?.unwrap_or_default(),
            colors: get("colors")
                .or(get("collarColors"))
                .map(|v| v.split_whitespace().map(str::to_ascii_lowercase).collect()),
            baby: read(get("baby"), parse_bool)?,
            health: read(get("health"), |v| {
                Some((RangeList::parse(&v.replace('%', ""))?, v.contains('%')))
            })?,
            moon_phase: ranges("moonPhase")?,
            day_time: ranges("dayTime")?,
            weather: read(get("weather"), |v| {
                v.split_whitespace()
                    .map(|w| match w.to_ascii_lowercase().as_str() {
                        "clear" => Some(Weather::Clear),
                        "rain" => Some(Weather::Rain),
                        "thunder" => Some(Weather::Thunder),
                        _ => None,
                    })
                    .collect()
            })?,
            sizes: ranges("sizes")?,
            blocks: read(get("blocks"), |v| {
                v.split_whitespace()
                    .map(|b| {
                        let (b, negated) = negation(b)?;
                        Some((BlockMatch::parse(b)?, negated))
                    })
                    .collect()
            })?
            .unwrap_or_default(),
        })
    }

    fn matches(&self, f: &Facts) -> bool {
        let ranged = |list: &Option<RangeList>, fact: Option<i32>| {
            list.as_ref()
                .is_none_or(|l| fact.is_some_and(|v| l.contains(v)))
        };
        (self.biomes.is_empty()
            || f.biome.is_some_and(|b| {
                let b = namespaced(b);
                let path = b.split_once(':').map_or(b.as_str(), |(_, p)| p);
                included(&self.biomes, |n| n.matches(&b) || n.matches(path))
            }))
            && ranged(&self.heights, f.y)
            && (self.names.is_empty()
                || match f.name {
                    Some(name) => included(&self.names, |n| n.matches(name)),
                    // An unnamed entity is no listed name: it passes only `!` entries.
                    None => self.names.iter().all(|(_, negated)| *negated),
                })
            && (self.professions.is_empty()
                || f.profession.is_some_and(|(p, level)| {
                    let p = namespaced(p);
                    self.professions.iter().any(|(want, levels)| {
                        *want == p && levels.as_ref().is_none_or(|l| l.contains(level))
                    })
                }))
            && self
                .colors
                .as_ref()
                .is_none_or(|c| f.color.is_some_and(|f| c.iter().any(|c| c == f)))
            && self.baby.is_none_or(|b| f.baby == Some(b))
            && self.health.as_ref().is_none_or(|(l, percent)| {
                f.health.is_some_and(|(health, max)| {
                    let value = if *percent {
                        health * 100.0 / max.max(f32::MIN_POSITIVE)
                    } else {
                        health
                    };
                    l.contains(value as i32)
                })
            })
            && ranged(&self.moon_phase, f.moon_phase)
            && ranged(&self.day_time, f.day_time)
            && self
                .weather
                .as_ref()
                .is_none_or(|w| f.weather.is_some_and(|f| w.contains(&f)))
            && ranged(&self.sizes, f.size)
            && (self.blocks.is_empty()
                || (!f.blocks.is_empty()
                    && included(&self.blocks, |m| {
                        f.blocks.iter().any(|b| {
                            m.matches(b.id, |key| {
                                b.states.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
                            })
                        })
                    })))
    }

    /// One of the rule's textures, by weight, from the entity's seed. OptiFine's docs don't say
    /// how the seed becomes a choice: this is Horizon's, the seed's magnitude modulo the total.
    fn pick(&self, seed: i32) -> u32 {
        let r = seed.wrapping_add(self.seed_offset).unsigned_abs();
        if self.weights.is_empty() {
            return self.textures[r as usize % self.textures.len()];
        }
        let total: u64 = self.weights.iter().map(|&w| u64::from(w)).sum();
        let mut left = u64::from(r) % total;
        for (&texture, &weight) in self.textures.iter().zip(&self.weights) {
            if left < u64::from(weight) {
                return texture;
            }
            left -= u64::from(weight);
        }
        unreachable!("left < total")
    }
}

/// An entity's seed from its UUID (most then least significant 64 bits): Java's
/// `UUID.hashCode`, so it is fixed for the entity's whole life, on every client.
pub fn seed(most: i64, least: i64) -> i32 {
    let x = most ^ least;
    ((x >> 32) ^ x) as i32
}

/// The highest variant index a rule may name (`textures.1=2000000000` is dropped).
pub const MAX_INDEX: u32 = 1024;

fn expand(list: &RangeList) -> Vec<u32> {
    let mut out = Vec::new();
    for &(lo, hi) in &list.0 {
        for i in lo.max(0)..=hi.min(MAX_INDEX as i32) {
            if out.len() == MAX_TEXTURES {
                return out;
            }
            out.push(i as u32);
        }
    }
    out
}

/// A value and whether it starts with `!`; `None` for a `!` with nothing after it.
fn negation(value: &str) -> Option<(&str, bool)> {
    match value.strip_prefix('!') {
        Some("") => None,
        Some(v) => Some((v, true)),
        None => Some((value, false)),
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Words split on spaces, a `"double quoted"` group kept as one word (ETF's `names`), each
/// with whether it starts with `!` (`!Bob`, `!"Blue Team"`).
fn quoted_words(value: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut rest = value.trim_start();
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (r, negated) = rest.strip_prefix('!').map_or((rest, false), |r| (r, true));
        let (word, after) = match r.strip_prefix('"') {
            Some(q) => q.split_once('"').unwrap_or((q, "")),
            None => r.split_once(' ').unwrap_or((r, "")),
        };
        rest = after.trim_start();
        Some((word, negated))
    })
}

/// `farmer librarian:1,3-4 minecraft:fisherman mymod:mayor:2`; `None` if a level list is bad.
fn parse_professions(value: &str) -> Option<Vec<(String, Option<RangeList>)>> {
    value
        .split_whitespace()
        .map(|entry| match entry.rsplit_once(':') {
            Some((name, levels)) if levels.starts_with(|c: char| c.is_ascii_digit()) => {
                Some((namespaced(name), Some(RangeList::parse(levels)?)))
            }
            _ => Some((namespaced(entry), None)),
        })
        .collect()
}

/// A list of (entry, negated): some entry that isn't negated hits (when there is one), and no
/// negated one does. `!red !blue` excludes both.
fn included<T>(list: &[(T, bool)], hit: impl Fn(&T) -> bool) -> bool {
    let mut wanted = list.iter().filter(|(_, negated)| !negated).peekable();
    let any_wanted = wanted.peek().is_none() || wanted.any(|(e, _)| hit(e));
    any_wanted && !list.iter().any(|(e, negated)| *negated && hit(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> Rules {
        Rules::parse(&Properties::parse(text))
    }

    #[test]
    fn first_matching_rule_wins_and_weights_split_seeds() {
        // OptiFine's own creeper example.
        let r = rules(
            "textures.1=10-14\nweights.1=10 10 10 3 10\nheights.1=0-55\n\
             textures.2=5 7 9\nbiomes.2=desert forest\nheights.2=80-255\n\
             textures.3=1-4 6 8 15-20\n",
        );
        assert_eq!(r.rules.len(), 3);
        assert_eq!(r.max_texture(), 20);
        assert!(!r.updates());
        let underground = Facts {
            y: Some(30),
            biome: Some("minecraft:plains"),
            ..Default::default()
        };
        // Weights 10 10 10 3 10 out of 43: seed % 43 in 30..33 is the fourth texture.
        assert_eq!(r.select(&underground, 0), 10);
        assert_eq!(r.select(&underground, 31), 13);
        assert_eq!(r.select(&underground, 42), 14);
        assert_eq!(r.select(&underground, 43 + 25), 12);
        let hills = Facts {
            y: Some(100),
            biome: Some("minecraft:desert"),
            ..Default::default()
        };
        assert_eq!(r.select(&hills, 4), 7, "4 % 3 = 1: the second of 5 7 9");
        // Plains at 100 falls to rule 3's 12 textures: 1 2 3 4 6 8 15..20.
        let plains = Facts {
            biome: Some("plains"),
            ..hills.clone()
        };
        assert_eq!(r.select(&plains, 5), 8);
        assert_eq!(r.select(&plains, -6), 15, "|-6| % 12 = 6");
        // No facts at all: height-bound rules fail, the fallback still applies.
        assert_eq!(r.select(&Facts::default(), 1), 2);
    }

    #[test]
    fn no_rule_matching_is_the_default() {
        let r = rules("skins.1=2 3\nbaby.1=true\n");
        assert!(r.updates());
        assert_eq!(r.select(&Facts::default(), 1), 1);
        let baby = Facts {
            baby: Some(true),
            ..Default::default()
        };
        assert_eq!(r.select(&baby, 1), 3);
        // `matching` tells a miss from a rule that picks 1.
        assert_eq!(r.matching(&Facts::default(), 1), None);
        assert_eq!(r.matching(&baby, 1), Some(3));
        assert_eq!(rules("textures.1=1").matching(&baby, 0), Some(1));
    }

    #[test]
    fn conditions() {
        let check = |text: &str, facts: Facts| rules(text).select(&facts, 0) == 2;
        let named = |name| Facts {
            name: Some(name),
            ..Default::default()
        };
        assert!(check("textures.1=2\nname.1=ipattern:pink*", named("Pinky")));
        assert!(!check("textures.1=2\nname.1=!Pinky", named("Pinky")));
        assert!(check("textures.1=2\nname.1=!Pinky", named("Bob")));
        assert!(check(
            "textures.1=2\nnames.1=Bob \"John Smith\"",
            named("John Smith")
        ));
        assert!(!check(
            "textures.1=2\nnames.1=Bob \"John Smith\"",
            named("John")
        ));

        let villager = |p, level| Facts {
            profession: Some((p, level)),
            ..Default::default()
        };
        let prof = "textures.1=2\nprofessions.1=farmer minecraft:librarian:1,3-4 mymod:mayor:2";
        assert!(check(prof, villager("minecraft:farmer", 5)));
        assert!(check(prof, villager("librarian", 3)));
        assert!(!check(prof, villager("librarian", 2)));
        assert!(check(prof, villager("mymod:mayor", 2)));
        assert!(!check(prof, Facts::default()));

        let hurt = |health| Facts {
            health: Some((health, 40.0)),
            ..Default::default()
        };
        assert!(check("textures.1=2\nhealth.1=0-50%", hurt(20.0)));
        assert!(!check("textures.1=2\nhealth.1=0-50%", hurt(21.0)));
        assert!(check("textures.1=2\nhealth.1=5-8 10-12", hurt(11.5)));
        assert!(!check("textures.1=2\nhealth.1=5-8 10-12", hurt(9.0)));
        // 53 of 100 is 53%, though 53 / 100 * 100 is 52.999996 in f32.
        let golem = Facts {
            health: Some((53.0, 100.0)),
            ..Default::default()
        };
        assert!(check("textures.1=2\nhealth.1=53-100%", golem));

        let colored = |c| Facts {
            color: Some(c),
            ..Default::default()
        };
        assert!(check(
            "textures.1=2\ncollarColors.1=pink magenta",
            colored("pink")
        ));
        assert!(!check(
            "textures.1=2\ncolors.1=pink magenta",
            colored("red")
        ));

        let world = Facts {
            moon_phase: Some(3),
            day_time: Some(18500),
            weather: Some(Weather::Thunder),
            size: Some(3),
            y: Some(-40),
            ..Default::default()
        };
        assert!(check(
            "textures.1=2\nmoonPhase.1=0-3\ndayTime.1=0-1000 18000-24000\n\
             weather.1=rain thunder\nsizes.1=3\nheights.1=(-64)-(-30)",
            world.clone()
        ));
        assert!(!check("textures.1=2\nweather.1=clear", world.clone()));
        assert!(check(
            "textures.1=2\nminHeight.1=-50\nmaxHeight.1=-40",
            world.clone()
        ));
        assert!(!check("textures.1=2\nminHeight.1=-39", world.clone()));
        // Each world condition alone, failing.
        for rule in [
            "moonPhase.1=4-7",
            "dayTime.1=0-18000",
            "sizes.1=0 1",
            "heights.1=-64--41",
            "colors.1=pink",
            "collarColors.1=pink",
            "baby.1=false",
        ] {
            assert!(
                !check(&format!("textures.1=2\n{rule}"), world.clone()),
                "{rule}"
            );
        }
        // ETF's bare negative height range.
        assert!(check("textures.1=2\nheights.1=-64--30", world));
    }

    #[test]
    fn name_lists() {
        let r = rules("textures.1=2\nnames.1=!Bob !\"Blue Team\"");
        let pick = |name| {
            r.select(
                &Facts {
                    name,
                    ..Default::default()
                },
                0,
            )
        };
        assert_eq!(pick(Some("Bob")), 1, "negated entries exclude");
        assert_eq!(pick(Some("Blue Team")), 1);
        assert_eq!(pick(Some("Green")), 2);
        assert_eq!(pick(None), 2, "an unnamed entity is neither");
        let r = rules("textures.1=2\nname.1=pattern:*");
        assert_eq!(
            r.select(&Facts::default(), 0),
            1,
            "an unnamed entity has no name"
        );
    }

    #[test]
    fn biome_lists() {
        let pick = |text: &str, biome| {
            rules(text).select(
                &Facts {
                    biome: Some(biome),
                    ..Default::default()
                },
                0,
            )
        };
        let not_wet = "textures.1=2\nbiomes.1=!ipattern:*OCEAN* !mymod:bog";
        assert_eq!(pick(not_wet, "minecraft:deep_ocean"), 1);
        assert_eq!(pick(not_wet, "mymod:bog"), 1);
        assert_eq!(pick(not_wet, "minecraft:bog"), 2);
        assert_eq!(pick(not_wet, "plains"), 2);
        // A pattern matches the id with or without its namespace.
        let some = "textures.1=2\nbiomes.1=pattern:deep_* Desert";
        assert_eq!(pick(some, "minecraft:deep_dark"), 2);
        assert_eq!(pick(some, "desert"), 2);
        assert_eq!(pick(some, "minecraft:dark_forest"), 1);
    }

    #[test]
    fn rules_run_in_number_order_with_their_seed_offset() {
        // Written 10 before 9: rule 9 is tried first.
        let r = rules("textures.10=5\ntextures.9=2 3 4\nseedOffset.9=1\n");
        assert_eq!(r.select(&Facts::default(), 0), 3, "(0 + 1) % 3: the second");
        assert_eq!(r.select(&Facts::default(), 2), 2, "(2 + 1) % 3: the first");
        // `textures` wins over `skins` in the same rule.
        assert_eq!(
            rules("textures.1=4\nskins.1=5").select(&Facts::default(), 0),
            4
        );
    }

    #[test]
    fn changing_facts_need_rechecks() {
        for rule in [
            "name.1=Bob",
            "names.1=Bob",
            "professions.1=farmer",
            "colors.1=red",
            "baby.1=true",
            "health.1=1-5",
        ] {
            assert!(rules(&format!("textures.1=2\n{rule}")).updates(), "{rule}");
        }
        assert!(!rules("textures.1=2\nbiomes.1=desert\nweather.1=rain").updates());
    }

    #[test]
    fn blocks() {
        let r = rules("textures.1=2\nblocks.1=sand oak_log:axis=y\ntextures.2=3\nblocks.2=!stone");
        let pick = |blocks: &[Block]| {
            r.select(
                &Facts {
                    blocks,
                    ..Default::default()
                },
                0,
            )
        };
        let block = |id| Block { id, states: &[] };
        assert_eq!(pick(&[block("minecraft:sand"), block("minecraft:air")]), 2);
        let upright = [Block {
            id: "minecraft:oak_log",
            states: &[("axis", "y")],
        }];
        assert_eq!(pick(&upright), 2);
        let sideways = [Block {
            id: "minecraft:oak_log",
            states: &[("axis", "x")],
        }];
        assert_eq!(pick(&sideways), 3, "rule 2: anything but stone");
        assert_eq!(pick(&[block("minecraft:stone")]), 1);
        assert_eq!(pick(&[]), 1, "no blocks known: not even `!stone` holds");
    }

    #[test]
    fn broken_rules_are_skipped() {
        let r = rules(
            "textures.1=abc\nbiomes.1=desert\n\
             skins.7=1-100000\n\
             textures.9=2000000000\n",
        );
        // Rule 7 stops at 256 textures, and rule 9's index is past MAX_INDEX.
        assert_eq!(r.max_texture(), 256);
        assert_eq!(r.select(&Facts::default(), 300), 300 % 256 + 1);
        // One weight for two textures, only zeros, or none: an even choice.
        for weights in ["5", "0 0", ""] {
            let r = rules(&format!("textures.1=2 3\nweights.1={weights}"));
            assert_eq!(r.select(&Facts::default(), 1), 3, "{weights}");
        }
    }

    #[test]
    fn rules_stop_at_max_rules() {
        let text: String = (1..=MAX_RULES + 5)
            .map(|n| format!("textures.{n}=2\nbaby.{n}=true\n"))
            .chain(["textures.99999=3\n".to_owned()])
            .collect();
        let r = rules(&text);
        assert_eq!(r.rules.len(), MAX_RULES);
        // Rule 99999 would match an adult; it is past the cap.
        let adult = Facts {
            baby: Some(false),
            ..Default::default()
        };
        assert_eq!(r.select(&adult, 0), 1);
    }

    #[test]
    fn bad_or_unknown_conditions_never_match() {
        // Each would match every entity if the condition were dropped.
        for rule in [
            "heights.1=0-55 x",
            "moonPhase.1=full",
            "baby.1=yes",
            "health.1=5.5-8",
            "professions.1=librarian:1,x",
            "weather.1=snow",
            "name.1=regex:(",
            "names.1=Bob regex:(",
            "minHeight.1=low",
            "nbt.1.CustomName=Bob",
            "teams.1=red",
            "seedSource.1=vehicle",
            "biomes.1=",
            "names.1=",
            "biomes.1=regex:(",
            "biomes.1=!",
            "blocks.1=!",
            "name.1=!",
            "names.1=Bob !",
        ] {
            let r = rules(&format!("textures.1=2\n{rule}\ntextures.2=3"));
            // A named mob in plains: what a dropped `!` entry would let through.
            let bob = Facts {
                biome: Some("minecraft:plains"),
                name: Some("Bob"),
                ..Default::default()
            };
            assert_eq!(r.select(&bob, 0), 3, "{rule}");
        }
        // Case doesn't matter where Java's parsers ignore it.
        let baby = Facts {
            baby: Some(true),
            weather: Some(Weather::Rain),
            ..Default::default()
        };
        assert_eq!(
            rules("textures.1=2\nbaby.1=TRUE\nweather.1=Rain").select(&baby, 0),
            2
        );
        // Another rule's nbt path ending in a list index leaves rule 1 alone.
        let r = rules("textures.1=2\nseedSource.1=entity\ntextures.2=3\nnbt.2.Tags.1=foo");
        assert_eq!(r.select(&Facts::default(), 0), 2);
    }

    #[test]
    fn seeds_are_java_uuid_hash_codes() {
        // new UUID(0x0123456789abcdefL, 0xfedcba9876543210L).hashCode():
        // msb ^ lsb = 0xffffffffffffffff, high ^ low = 0.
        assert_eq!(
            seed(0x0123_4567_89ab_cdef, 0xfedc_ba98_7654_3210_u64 as i64),
            0
        );
        // new UUID(1, 2).hashCode() = 3, and new UUID(1L << 32, 0) is 1.
        assert_eq!(seed(1, 2), 3);
        assert_eq!(seed(1 << 32, 0), 1);
    }
}
