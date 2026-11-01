//! Sets of section indexes in a chunk column, a bit per section.
//!
//! Vanilla's tallest level has 254 sections (`DimensionType.Y_SIZE / 16`), and
//! light data adds a section below and above it: 256 bits hold every layout a
//! dimension type allows.

/// Section indexes of one column, 0 being the lowest.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SectionMask([u64; 4]);

impl SectionMask {
    pub const CAPACITY: usize = 256;
    pub const EMPTY: Self = Self([0; 4]);
    pub const ALL: Self = Self([u64::MAX; 4]);

    /// The indexes `0..n`.
    pub const fn below(n: usize) -> Self {
        let mut words = [0; 4];
        let mut w = 0;
        while w < 4 {
            let bits = n.saturating_sub(w * 64);
            words[w] = if bits >= 64 {
                u64::MAX
            } else {
                (1 << bits) - 1
            };
            w += 1;
        }
        Self(words)
    }
    pub fn single(i: usize) -> Self {
        let mut mask = Self::EMPTY;
        mask.insert(i);
        mask
    }
    #[inline]
    pub fn contains(&self, i: usize) -> bool {
        i < Self::CAPACITY && self.0[i >> 6] >> (i & 63) & 1 != 0
    }
    #[inline]
    pub fn insert(&mut self, i: usize) {
        self.0[i >> 6] |= 1 << (i & 63);
    }
    #[inline]
    pub fn remove(&mut self, i: usize) {
        self.0[i >> 6] &= !(1 << (i & 63));
    }
    pub fn is_empty(&self) -> bool {
        self.0 == [0; 4]
    }
    pub fn len(&self) -> usize {
        self.0.iter().map(|w| w.count_ones() as usize).sum()
    }
    /// The indexes from `i` up.
    pub fn from(self, i: usize) -> Self {
        self & !Self::below(i)
    }
    /// The indexes, lowest first.
    pub fn iter(self) -> Indexes {
        Indexes(self)
    }
}

impl std::ops::BitOr for SectionMask {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(std::array::from_fn(|w| self.0[w] | other.0[w]))
    }
}
impl std::ops::BitOrAssign for SectionMask {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}
impl std::ops::BitAnd for SectionMask {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(std::array::from_fn(|w| self.0[w] & other.0[w]))
    }
}
impl std::ops::Not for SectionMask {
    type Output = Self;
    fn not(self) -> Self {
        Self(self.0.map(|w| !w))
    }
}
impl std::fmt::Debug for SectionMask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}
impl FromIterator<usize> for SectionMask {
    fn from_iter<I: IntoIterator<Item = usize>>(indexes: I) -> Self {
        let mut mask = Self::EMPTY;
        for i in indexes {
            mask.insert(i);
        }
        mask
    }
}

/// `SectionMask::iter`: the set's indexes in ascending order.
#[derive(Clone, Copy, Debug)]
pub struct Indexes(SectionMask);
impl Iterator for Indexes {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        let words = &mut (self.0).0;
        let w = words.iter().position(|&w| w != 0)?;
        let bit = words[w].trailing_zeros() as usize;
        words[w] &= words[w] - 1;
        Some(w * 64 + bit)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len();
        (len, Some(len))
    }
}
impl ExactSizeIterator for Indexes {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_across_words() {
        let mut mask = SectionMask::EMPTY;
        for i in [0, 23, 63, 64, 130, 255] {
            mask.insert(i);
        }
        assert_eq!(mask.iter().collect::<Vec<_>>(), [0, 23, 63, 64, 130, 255]);
        assert_eq!(mask.len(), 6);
        assert!(mask.contains(130) && !mask.contains(131) && !mask.contains(300));
        mask.remove(64);
        assert_eq!(mask.from(24).iter().collect::<Vec<_>>(), [63, 130, 255]);
        assert_eq!(mask.from(256), SectionMask::EMPTY);
        assert_eq!(mask.from(0), mask);
        assert!(SectionMask::EMPTY.is_empty() && !mask.is_empty());
    }

    #[test]
    fn below_counts_whole_and_partial_words() {
        for n in [0, 1, 24, 63, 64, 65, 131, 254, 256] {
            let mask = SectionMask::below(n);
            assert_eq!(mask.len(), n);
            assert_eq!(mask.iter().collect::<Vec<_>>(), (0..n).collect::<Vec<_>>());
        }
        assert_eq!(SectionMask::below(1000), SectionMask::ALL);
    }

    #[test]
    fn operators() {
        let a: SectionMask = [1, 70, 200].into_iter().collect();
        let b: SectionMask = [70, 3].into_iter().collect();
        assert_eq!((a | b).iter().collect::<Vec<_>>(), [1, 3, 70, 200]);
        assert_eq!((a & b).iter().collect::<Vec<_>>(), [70]);
        assert_eq!(
            (!a & SectionMask::below(4)).iter().collect::<Vec<_>>(),
            [0, 2, 3]
        );
        assert_eq!(format!("{a:?}"), "{1, 70, 200}");
    }
}
