//! The values OptiFine's `.properties` match against: names (`pattern:`, `ipattern:`, `regex:`,
//! `iregex:` or exact), integer range lists (`heights`, `damage`, `stackSize`), id lists
//! (`items`, OptiFine's plain `biomes`) and block lists with block states (`matchBlocks`, `blocks`).

use regex::{Regex, RegexBuilder};

/// A name test: custom names (`name`, `nbt.display.Name`), entity names, NBT strings.
#[derive(Clone, Debug)]
pub enum Name {
    Exact(String),
    /// `pattern:` (`*` any run, `?` one character); `ipattern:` ignores case.
    Glob {
        pattern: String,
        ignore_case: bool,
    },
    /// `regex:` and `iregex:`, matched against the whole name as Java's `String.matches`.
    Regex(Regex),
}

impl Name {
    /// `None` for a regex that does not compile.
    pub fn parse(value: &str) -> Option<Self> {
        let glob = |pattern: &str, ignore_case| Name::Glob {
            pattern: pattern.to_owned(),
            ignore_case,
        };
        let regex = |re: &str, ignore_case| {
            // On its own first: a stray `)` would otherwise close the anchoring group early.
            Regex::new(re).ok()?;
            RegexBuilder::new(&format!("^(?:{re})$"))
                .case_insensitive(ignore_case)
                .build()
                .ok()
                .map(Name::Regex)
        };
        if let Some(p) = value.strip_prefix("pattern:") {
            Some(glob(p, false))
        } else if let Some(p) = value.strip_prefix("ipattern:") {
            Some(glob(p, true))
        } else if let Some(r) = value.strip_prefix("regex:") {
            regex(r, false)
        } else if let Some(r) = value.strip_prefix("iregex:") {
            regex(r, true)
        } else {
            Some(Name::Exact(value.to_owned()))
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            Name::Exact(n) => n == name,
            Name::Regex(re) => re.is_match(name),
            Name::Glob {
                pattern,
                ignore_case: false,
            } => glob(
                &pattern.chars().collect::<Vec<_>>(),
                &name.chars().collect::<Vec<_>>(),
            ),
            Name::Glob {
                pattern,
                ignore_case: true,
            } => glob(
                &pattern.to_lowercase().chars().collect::<Vec<_>>(),
                &name.to_lowercase().chars().collect::<Vec<_>>(),
            ),
        }
    }
}

/// `*` any run, `?` one character, the whole name. Star backtracking: on a mismatch only the
/// last `*` takes one more character, so it runs in O(pattern × name), whatever the stars (a
/// name comes from whoever named the item, and this runs per item per frame).
fn glob(p: &[char], s: &[char]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while si < s.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, si));
                pi += 1;
            }
            Some(&c) if c == '?' || c == s[si] => {
                pi += 1;
                si += 1;
            }
            _ => match star {
                Some((sp, ss)) => {
                    pi = sp + 1;
                    si = ss + 1;
                    star = Some((sp, ss + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// `1 3-5 8-`, separated by spaces or commas; a negative bound is written in parentheses:
/// `(-64)-0`, or bare as ETF also reads it: `-64--30`. An open upper bound (`8-`) runs to
/// `i32::MAX`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeList(pub Vec<(i32, i32)>);

impl RangeList {
    /// `None` when a token is not a number or a range.
    pub fn parse(value: &str) -> Option<Self> {
        let mut out = Vec::new();
        for token in value.split([' ', ',']).filter(|t| !t.is_empty()) {
            let (low, rest) = bound(token)?;
            let range = match rest.strip_prefix('-') {
                None if rest.is_empty() => (low, low),
                Some("") => (low, i32::MAX),
                Some(high) => match bound(high)? {
                    (high, "") => (low, high),
                    _ => return None,
                },
                None => return None,
            };
            out.push(range);
        }
        Some(Self(out))
    }

    pub fn contains(&self, n: i32) -> bool {
        self.0.iter().any(|&(lo, hi)| (lo..=hi).contains(&n))
    }
}

/// A number at the start of `s`, bare (`5`, `-5`) or in parentheses, and what follows it.
fn bound(s: &str) -> Option<(i32, &str)> {
    if let Some(inner) = s.strip_prefix('(') {
        let (n, rest) = inner.split_once(')')?;
        return Some((n.trim().parse().ok()?, rest));
    }
    let sign = usize::from(s.starts_with('-'));
    let end = s[sign..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(s.len(), |i| i + sign);
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// Resource ids (`biomes`, `items`, `entities`): `plains`, `minecraft:plains`, `mymod:x`,
/// separated by whitespace; a `!` before an id excludes it. Ids are lowercased, and a bare id is
/// in `minecraft:`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdList {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl IdList {
    pub fn parse(value: &str) -> Self {
        let mut out = Self::default();
        for token in value.split_whitespace() {
            match token.strip_prefix('!') {
                Some(id) => out.exclude.push(namespaced(id)),
                None => out.include.push(namespaced(token)),
            }
        }
        out
    }

    /// An empty include list (only exclusions) admits everything not excluded.
    pub fn matches(&self, id: &str) -> bool {
        let id = namespaced(id);
        (self.include.is_empty() || self.include.contains(&id)) && !self.exclude.contains(&id)
    }
}

pub fn namespaced(id: &str) -> String {
    let id = id.to_ascii_lowercase();
    if id.contains(':') {
        id
    } else {
        format!("minecraft:{id}")
    }
}

/// One entry of a block list: `oak_log`, `minecraft:oak_log`, `oak_log:axis=x,y`,
/// `mymod:pillar:axis=y:waterlogged=false`. A state test admits any of its values.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockMatch {
    pub id: String,
    pub states: Vec<(String, Vec<String>)>,
}

impl BlockMatch {
    /// Every entry of a whitespace-separated list. Legacy numeric ids, which name no block since
    /// 1.13, are dropped.
    pub fn parse_list(value: &str) -> Vec<Self> {
        value.split_whitespace().filter_map(Self::parse).collect()
    }

    pub fn parse(entry: &str) -> Option<Self> {
        let (names, states): (Vec<&str>, Vec<&str>) =
            entry.split(':').partition(|s| !s.contains('='));
        let id = match names[..] {
            [name] => namespaced(name),
            [ns, name] => namespaced(&format!("{ns}:{name}")),
            _ => return None,
        };
        if id.rsplit(':').next()?.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let states = states
            .iter()
            .map(|s| {
                let (key, values) = s.split_once('=').unwrap_or((s, ""));
                (
                    key.to_owned(),
                    values.split(',').map(str::to_owned).collect(),
                )
            })
            .collect();
        Some(Self { id, states })
    }

    /// `state` answers a block state property's value.
    pub fn matches<'a>(&self, id: &str, state: impl Fn(&str) -> Option<&'a str>) -> bool {
        namespaced(id) == self.id
            && self
                .states
                .iter()
                .all(|(key, values)| state(key).is_some_and(|v| values.iter().any(|w| w == v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let m = |rule: &str, name: &str| Name::parse(rule).unwrap().matches(name);
        assert!(m("Dinnerbone", "Dinnerbone"));
        assert!(!m("Dinnerbone", "dinnerbone"));
        assert!(m("pattern:*Sword?", "Flame Sword!"));
        assert!(!m("pattern:*Sword?", "Flame sword!"));
        assert!(m("ipattern:*sword*", "FLAME SWORD of doom"));
        assert!(m("regex:Bob|Alice", "Alice"));
        // Java's String.matches: the whole name, not a substring.
        assert!(!m("regex:Bob|Alice", "Alice2"));
        assert!(m("iregex:j(eb)_", "JEB_"));
        assert!(Name::parse("regex:(").is_none());
        assert!(
            Name::parse("regex:Excalibur)|(.*").is_none(),
            "would match every name"
        );
        assert!(m("pattern:a*b*c", "aXbYc"));
        assert!(!m("pattern:a*b*c", "aXbY"));
        assert!(m("pattern:**", ""));
        assert!(!m("pattern:?", ""));
        // Many stars against a long name that never matches: linear, not exponential.
        let long = "a".repeat(5000);
        assert!(!m(&format!("pattern:{}b", "*a".repeat(20)), &long));
    }

    #[test]
    fn ranges() {
        let r = RangeList::parse("1 3-5,8- (-64)-(-10)").unwrap();
        for n in [1, 3, 4, 5, 8, 1000, -64, -10] {
            assert!(r.contains(n), "{n}");
        }
        for n in [0, 2, 6, 7, -9, -65] {
            assert!(!r.contains(n), "{n}");
        }
        assert!(RangeList::parse("-5").unwrap().contains(-5));
        // ETF's bare negative ranges.
        assert_eq!(
            RangeList::parse("-64--30 -5-2 -3-").unwrap().0,
            [(-64, -30), (-5, 2), (-3, i32::MAX)]
        );
        assert!(RangeList::parse("-").is_none());
        assert!(RangeList::parse("--5").is_none());
        assert!(RangeList::parse("1-x").is_none());
        assert!(RangeList::parse("abc").is_none());
    }

    #[test]
    fn id_lists() {
        let l = IdList::parse("Plains minecraft:desert mymod:bog");
        assert!(l.matches("minecraft:plains"));
        assert!(l.matches("desert"));
        assert!(l.matches("mymod:bog"));
        assert!(!l.matches("minecraft:bog"));
        let not_ocean = IdList::parse("!ocean !deep_ocean");
        assert!(not_ocean.matches("plains"));
        assert!(!not_ocean.matches("minecraft:deep_ocean"));
    }

    #[test]
    fn block_lists() {
        let list =
            BlockMatch::parse_list("glass minecraft:oak_log:axis=x,z mymod:pillar:axis=y 20");
        assert_eq!(list.len(), 3, "the numeric id 20 is dropped");
        let axis = |v: &'static str| move |k: &str| (k == "axis").then_some(v);
        assert!(list[0].matches("minecraft:glass", |_| None));
        assert!(list[1].matches("oak_log", axis("z")));
        assert!(!list[1].matches("oak_log", axis("y")));
        assert!(!list[1].matches("oak_log", |_| None));
        assert!(list[2].matches("mymod:pillar", axis("y")));
        assert!(!list[2].matches("minecraft:pillar", axis("y")));
    }
}
