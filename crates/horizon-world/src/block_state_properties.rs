//! A block state's properties: a few name/value pairs, sorted by name, read
//! like the `BTreeMap<String, String>` they replace. A registry's states share
//! their strings (35,723 vanilla states hold 158,409 properties made of 237
//! distinct strings): one string for the whole process instead of a B-tree
//! leaf (544 bytes) and two allocations per property. Shared strings are
//! never freed, nor counted: cloning a state copies pointers, so world
//! generation threads cloning the same states don't contend on counters.
use std::{
    borrow::Borrow,
    cmp::Ordering,
    collections::BTreeMap,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    sync::Mutex,
};

/// A property name or value: owned, or one string a registry shares.
#[derive(Clone)]
enum Text {
    Owned(String),
    Shared(&'static String),
}
impl Deref for Text {
    type Target = String;
    fn deref(&self) -> &String {
        match self {
            Self::Owned(s) => s,
            Self::Shared(s) => s,
        }
    }
}
impl Text {
    fn into_string(self) -> String {
        match self {
            Self::Owned(s) => s,
            Self::Shared(s) => s.clone(),
        }
    }
    fn to_mut(&mut self) -> &mut String {
        if let Self::Shared(s) = self {
            *self = Self::Owned((**s).clone());
        }
        match self {
            Self::Owned(s) => s,
            Self::Shared(_) => unreachable!(),
        }
    }
}

type Interned = horizon_core::fast_hash::FxHashMap<&'static str, &'static String>;
/// Every string a registry's states share, for the process: registries'
/// names and values only (a few hundred), never a player's.
static INTERNED: Mutex<Option<Interned>> = Mutex::new(None);

/// The strings one registry's states share ([`BlockProperties::share`]):
/// the process's, with the ones already met at hand.
#[derive(Default)]
pub struct SharedStrings(Interned);
impl SharedStrings {
    pub fn get(&mut self, text: &str) -> &'static String {
        if let Some(shared) = self.0.get(text) {
            return shared;
        }
        let mut interned = INTERNED.lock().unwrap_or_else(|e| e.into_inner());
        let interned = interned.get_or_insert_with(Interned::default);
        let shared = match interned.get(text) {
            Some(shared) => *shared,
            None => {
                let shared: &'static String = Box::leak(Box::new(text.to_owned()));
                interned.insert(shared.as_str(), shared);
                shared
            }
        };
        self.0.insert(shared.as_str(), shared);
        shared
    }
}

/// A block state's properties by name, in name order.
#[derive(Clone, Default)]
pub struct BlockProperties(Box<[(Text, Text)]>);

impl BlockProperties {
    pub fn new() -> Self {
        Self::default()
    }
    fn position<Q: Ord + ?Sized>(&self, key: &Q) -> Result<usize, usize>
    where
        String: Borrow<Q>,
    {
        self.0.binary_search_by(|(k, _)| (**k).borrow().cmp(key))
    }
    pub fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&String>
    where
        String: Borrow<Q>,
    {
        self.position(key).ok().map(|i| &*self.0[i].1)
    }
    pub fn get_mut<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<&mut String>
    where
        String: Borrow<Q>,
    {
        let i = self.position(key).ok()?;
        Some(self.0[i].1.to_mut())
    }
    pub fn contains_key<Q: Ord + ?Sized>(&self, key: &Q) -> bool
    where
        String: Borrow<Q>,
    {
        self.position(key).is_ok()
    }
    /// Sets `key` to `value`; the value it had, if any.
    pub fn insert(&mut self, key: String, value: String) -> Option<String> {
        match self.position(key.as_str()) {
            Ok(i) => Some(std::mem::replace(&mut self.0[i].1, Text::Owned(value)).into_string()),
            Err(i) => {
                let mut pairs = std::mem::take(&mut self.0).into_vec();
                pairs.insert(i, (Text::Owned(key), Text::Owned(value)));
                self.0 = pairs.into_boxed_slice();
                None
            }
        }
    }
    pub fn remove<Q: Ord + ?Sized>(&mut self, key: &Q) -> Option<String>
    where
        String: Borrow<Q>,
    {
        let i = self.position(key).ok()?;
        let mut pairs = std::mem::take(&mut self.0).into_vec();
        let (_, value) = pairs.remove(i);
        self.0 = pairs.into_boxed_slice();
        Some(value.into_string())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> Iter<'_> {
        Iter(self.0.iter())
    }
    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &String> + ExactSizeIterator {
        self.0.iter().map(|(k, _)| &**k)
    }
    pub fn values(&self) -> impl DoubleEndedIterator<Item = &String> + ExactSizeIterator {
        self.0.iter().map(|(_, v)| &**v)
    }
    /// These properties over the strings `strings` holds, which other states
    /// share (a registry's states, built once). Strings already shared stay.
    pub fn share(&mut self, strings: &mut SharedStrings) {
        for text in self.0.iter_mut().flat_map(|(k, v)| [k, v]) {
            if let Text::Owned(owned) = text {
                *text = Text::Shared(strings.get(owned));
            }
        }
    }
    /// Shared names and values, collected as a `BTreeMap` collects them.
    pub fn from_shared(mut pairs: Vec<(&'static String, &'static String)>) -> Self {
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let mut kept: Vec<(Text, Text)> = Vec::with_capacity(pairs.len());
        for (k, v) in pairs {
            match kept.last_mut() {
                Some(last) if *last.0 == *k => last.1 = Text::Shared(v),
                _ => kept.push((Text::Shared(k), Text::Shared(v))),
            }
        }
        Self(kept.into_boxed_slice())
    }
}

pub struct Iter<'a>(std::slice::Iter<'a, (Text, Text)>);
impl<'a> Iterator for Iter<'a> {
    type Item = (&'a String, &'a String);
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|(k, v)| (&**k, &**v))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(|(k, v)| (&**k, &**v))
    }
}
impl ExactSizeIterator for Iter<'_> {}

impl<'a> IntoIterator for &'a BlockProperties {
    type Item = (&'a String, &'a String);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}
impl IntoIterator for BlockProperties {
    type Item = (String, String);
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        IntoIter(self.0.into_vec().into_iter())
    }
}
pub struct IntoIter(std::vec::IntoIter<(Text, Text)>);
impl Iterator for IntoIter {
    type Item = (String, String);
    fn next(&mut self) -> Option<(String, String)> {
        self.0
            .next()
            .map(|(k, v)| (k.into_string(), v.into_string()))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
/// As a `BTreeMap` collects: sorted by name, the last value of a name wins.
impl FromIterator<(String, String)> for BlockProperties {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        let mut pairs: Vec<(String, String)> = iter.into_iter().collect();
        // Stable: equal names keep their order, and the last one is kept.
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut kept: Vec<(Text, Text)> = Vec::with_capacity(pairs.len());
        for (k, v) in pairs {
            match kept.last_mut() {
                Some(last) if *last.0 == k => last.1 = Text::Owned(v),
                _ => kept.push((Text::Owned(k), Text::Owned(v))),
            }
        }
        Self(kept.into_boxed_slice())
    }
}
impl Extend<(String, String)> for BlockProperties {
    fn extend<I: IntoIterator<Item = (String, String)>>(&mut self, iter: I) {
        for (k, v) in iter {
            self.insert(k, v);
        }
    }
}
impl<const N: usize> From<[(String, String); N]> for BlockProperties {
    fn from(pairs: [(String, String); N]) -> Self {
        pairs.into_iter().collect()
    }
}
impl From<BTreeMap<String, String>> for BlockProperties {
    fn from(map: BTreeMap<String, String>) -> Self {
        Self(
            map.into_iter()
                .map(|(k, v)| (Text::Owned(k), Text::Owned(v)))
                .collect(),
        )
    }
}
impl From<BlockProperties> for BTreeMap<String, String> {
    fn from(properties: BlockProperties) -> Self {
        properties.into_iter().collect()
    }
}

impl<Q: Ord + ?Sized> std::ops::Index<&Q> for BlockProperties
where
    String: Borrow<Q>,
{
    type Output = String;
    /// Panics when the key is missing, as a `BTreeMap` does.
    fn index(&self, key: &Q) -> &String {
        self.get(key).expect("no entry found for key")
    }
}
impl serde::Serialize for BlockProperties {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}
impl<'de> serde::Deserialize<'de> for BlockProperties {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BTreeMap::<String, String>::deserialize(deserializer).map(Self::from)
    }
}

// Compared, ordered and hashed as the `BTreeMap` they replace: pair by pair.
impl PartialEq for BlockProperties {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
impl Eq for BlockProperties {}
impl PartialEq<BTreeMap<String, String>> for BlockProperties {
    fn eq(&self, other: &BTreeMap<String, String>) -> bool {
        self.iter().eq(other.iter())
    }
}
impl PartialOrd for BlockProperties {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for BlockProperties {
    fn cmp(&self, other: &Self) -> Ordering {
        self.iter().cmp(other.iter())
    }
}
impl Hash for BlockProperties {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_usize(self.len());
        for pair in self {
            pair.hash(state);
        }
    }
}
impl fmt::Debug for BlockProperties {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_and_writes_as_the_map_it_replaces() {
        let pairs = [
            ("waterlogged", "false"),
            ("facing", "north"),
            ("half", "top"),
        ];
        let mut map = map(&pairs);
        let mut properties: BlockProperties = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let same = |p: &BlockProperties, m: &BTreeMap<String, String>| {
            assert!(p.iter().eq(m.iter()));
            assert_eq!(p.len(), m.len());
            assert_eq!(format!("{p:?}"), format!("{m:?}"));
        };
        same(&properties, &map);
        assert_eq!(properties.get("facing"), map.get("facing"));
        assert_eq!(properties.get("shape"), None);
        assert_eq!(
            properties.insert("facing".into(), "east".into()),
            map.insert("facing".into(), "east".into())
        );
        assert_eq!(
            properties.insert("axis".into(), "y".into()),
            map.insert("axis".into(), "y".into())
        );
        *properties.get_mut("half").unwrap() = "bottom".into();
        *map.get_mut("half").unwrap() = "bottom".into();
        assert_eq!(properties.remove("waterlogged"), map.remove("waterlogged"));
        assert_eq!(properties.remove("waterlogged"), None);
        same(&properties, &map);
        assert_eq!(BTreeMap::from(properties), map);
    }

    #[test]
    fn collects_as_a_map_does() {
        let pairs = [("b", "1"), ("a", "2"), ("b", "3")];
        let properties: BlockProperties = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert!(properties.iter().eq(map(&pairs).iter()));
    }

    #[test]
    fn orders_and_hashes_as_the_map_does() {
        let sets = [
            vec![],
            vec![("a", "1")],
            vec![("a", "2")],
            vec![("a", "1"), ("b", "1")],
            vec![("b", "0")],
        ];
        let hash = |value: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value(&mut hasher);
            hasher.finish()
        };
        for a in &sets {
            for b in &sets {
                let (pa, pb) = (BlockProperties::from(map(a)), BlockProperties::from(map(b)));
                assert_eq!(pa.cmp(&pb), map(a).cmp(&map(b)), "{a:?} {b:?}");
                assert_eq!(pa == pb, map(a) == map(b));
            }
            let p = BlockProperties::from(map(a));
            assert_eq!(hash(&|h| p.hash(h)), hash(&|h| map(a).hash(h)));
        }
    }

    #[test]
    fn shared_strings_read_and_change_as_owned_ones() {
        let mut strings = SharedStrings::default();
        let pairs = map(&[("facing", "north"), ("lit", "true")]);
        let mut a = BlockProperties::from(pairs.clone());
        let mut b = BlockProperties::from(pairs.clone());
        a.share(&mut strings);
        b.share(&mut strings);
        assert_eq!(a, BlockProperties::from(pairs.clone()));
        // One string for both states' value.
        let (Text::Shared(x), Text::Shared(y)) = (&a.0[0].1, &b.0[0].1) else {
            panic!("not shared");
        };
        assert!(std::ptr::eq(*x, *y));
        // Changing one state's value leaves the other's.
        a.get_mut("facing").unwrap().push_str("_east");
        assert_eq!(a.get("facing").map(String::as_str), Some("north_east"));
        assert_eq!(b.get("facing").map(String::as_str), Some("north"));
        assert_eq!(BTreeMap::from(b), pairs);
    }
}
