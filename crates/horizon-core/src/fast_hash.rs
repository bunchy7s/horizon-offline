//! A fast, non-cryptographic hasher (the Fx hash used by rustc) for hot maps
//! keyed by small integers, such as chunk coordinates. Iteration order of
//! such maps carries no meaning: the default hasher's is random anyway.
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0xf135_7aea_2e62_a9c5;

#[derive(Clone, Copy, Default)]
pub struct FxHasher {
    hash: u64,
}
impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = self.hash.wrapping_add(word).wrapping_mul(SEED);
    }
}
impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let words = bytes.chunks_exact(8);
        let rest = words.remainder();
        for word in words {
            let mut w = [0u8; 8];
            w.copy_from_slice(word);
            self.add(u64::from_le_bytes(w));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(u64::from(i));
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.add(u64::from(i as u32));
    }
    #[inline]
    fn finish(&self) -> u64 {
        // The multiply leaves the low bits weakest; hash tables use them.
        self.hash.rotate_left(26)
    }
}
pub type FxBuild = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = std::collections::HashMap<K, V, FxBuild>;
pub type FxHashSet<K> = std::collections::HashSet<K, FxBuild>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_chunks_spread_over_buckets() {
        let mut map: FxHashMap<(i32, i32), i32> = FxHashMap::default();
        for x in -40..40 {
            for z in -40..40 {
                map.insert((x, z), x * z);
            }
        }
        assert_eq!(map.len(), 6400);
        assert_eq!(map[&(-7, 13)], -91);
        let mut low = std::collections::HashSet::new();
        for x in 0..64 {
            low.insert(std::hash::BuildHasher::hash_one(&FxBuild::default(), (x, 0)) & 63);
        }
        assert!(low.len() > 32, "low bits vary: {}", low.len());
    }
}
