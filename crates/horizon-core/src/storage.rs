//! Palette indices packed into padded 64-bit words, not a continuous bitstream.

/// `n / per_word` for `n < MAGIC_LEN` by multiplication: `MAGIC[bits].0 =
/// ceil(2^20 / (64 / bits))` is exact while `n * (64 / bits) < 2^20`. `.1` is
/// the bits a word's entries fill (`64 / bits * bits`): looked up, not divided
/// for, as a division by a run-time value costs tens of cycles on x86.
pub(crate) const MAGIC: [(u32, u32); 33] = {
    let mut table = [(0, 0); 33];
    let mut bits = 1;
    while bits <= 32 {
        let per = 64 / bits as u32;
        table[bits] = ((1u32 << 20).div_ceil(per), per * bits as u32);
        bits += 1;
    }
    table
};
/// Entries `locate` handles exactly for every width: the smallest width (1 bit,
/// 64 per word) gives the bound `2^20 / 64`.
pub(crate) const MAGIC_LEN: usize = 1 << 14;
/// The multiply-shift replaces a ~20-26 cycle x86 divide, but arm64's divider
/// is as cheap as the table lookup: measured on an Apple M-series CPU, 65k
/// reads took 1.5 ms with the division and 2.5 ms with the table (bits 4-16),
/// so only the x86 targets take the table.
const MAGIC_BEATS_DIVISION: bool = cfg!(any(target_arch = "x86", target_arch = "x86_64"));
/// The word and bit shift of entry `i`; exact for `i < MAGIC_LEN`.
#[inline]
pub(crate) fn locate(bits: u8, i: usize) -> (usize, u32) {
    if bits.is_power_of_two() {
        // 64 / bits is a power of two: shifts instead of the multiplication.
        let log = bits.trailing_zeros();
        let word = i >> (6 - log);
        (word, ((i & ((64 >> log) - 1)) as u32) << log)
    } else {
        let (magic, filled) = MAGIC[usize::from(bits)];
        let word = (i as u32 * magic) >> 20;
        (word as usize, i as u32 * u32::from(bits) - word * filled)
    }
}
#[inline]
pub(crate) fn mask(bits: u8) -> u64 {
    (1u64 << bits) - 1
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    InvalidBits(u8),
    InvalidLength { expected: usize, actual: usize },
    OutOfBounds { index: usize, len: usize },
    ValueTooLarge(u32),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for StorageError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackedStorage {
    bits: u8,
    len: usize,
    per_word: usize,
    mask: u64,
    /// Whether `len` is past what [`locate`] handles exactly; such a storage
    /// falls back to division.
    wide: bool,
    words: Vec<u64>,
}

impl PackedStorage {
    pub fn new(bits: u8, len: usize) -> Result<Self, StorageError> {
        let per_word = Self::per_word(bits)?;
        Self::from_words(bits, len, vec![0; len.div_ceil(per_word)])
    }

    /// Retains padding and unused tail bits from input, as the Java long[] does.
    pub fn from_words(bits: u8, len: usize, words: Vec<u64>) -> Result<Self, StorageError> {
        let per_word = Self::per_word(bits)?;
        let expected = len.div_ceil(per_word);
        if words.len() != expected {
            return Err(StorageError::InvalidLength {
                expected,
                actual: words.len(),
            });
        }
        Ok(Self {
            bits,
            len,
            per_word,
            mask: (1_u64 << bits) - 1,
            wide: len > MAGIC_LEN,
            words,
        })
    }

    fn per_word(bits: u8) -> Result<usize, StorageError> {
        if !(1..=32).contains(&bits) {
            return Err(StorageError::InvalidBits(bits));
        }
        Ok(64 / usize::from(bits))
    }

    pub fn bits(&self) -> u8 {
        self.bits
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// The word and bit shift of an in-range index.
    #[inline]
    fn slot(&self, index: usize) -> (usize, u32) {
        if self.wide || !MAGIC_BEATS_DIVISION {
            (
                index / self.per_word,
                ((index % self.per_word) * usize::from(self.bits)) as u32,
            )
        } else {
            locate(self.bits, index)
        }
    }

    fn location(&self, index: usize) -> Result<(usize, u32), StorageError> {
        if index >= self.len {
            return Err(StorageError::OutOfBounds {
                index,
                len: self.len,
            });
        }
        Ok(self.slot(index))
    }

    pub fn get(&self, index: usize) -> Result<u32, StorageError> {
        let (word, shift) = self.location(index)?;
        Ok(((self.words[word] >> shift) & self.mask) as u32)
    }

    /// Reads `index` without a bounds check. Callers use this once they hold a
    /// slice length or a section index they have already validated.
    /// `index` must be less than `len`.
    pub fn read(&self, index: usize) -> u32 {
        debug_assert!(index < self.len);
        let (word, shift) = self.slot(index);
        ((self.words[word] >> shift) & self.mask) as u32
    }

    /// Writes `value` at `index` without revalidating either. `index` must be
    /// less than `len` and `value` must fit `bits`, which holds for a section
    /// palette index from the same container. Returns the previous value.
    pub fn write(&mut self, index: usize, value: u32) -> u32 {
        debug_assert!(index < self.len && u64::from(value) <= self.mask);
        let (word, shift) = self.slot(index);
        let old = ((self.words[word] >> shift) & self.mask) as u32;
        self.words[word] = (self.words[word] & !(self.mask << shift)) | (u64::from(value) << shift);
        old
    }

    /// Validation fails before mutation. Writes accept nonnegative Java ints;
    /// imported 32-bit words may still decode to any u32 bit pattern.
    pub fn set(&mut self, index: usize, value: u32) -> Result<u32, StorageError> {
        let (word, shift) = self.location(index)?;
        if u64::from(value) > self.mask || value > i32::MAX as u32 {
            return Err(StorageError::ValueTooLarge(value));
        }
        let old = ((self.words[word] >> shift) & self.mask) as u32;
        self.words[word] = (self.words[word] & !(self.mask << shift)) | (u64::from(value) << shift);
        Ok(old)
    }

    /// Vanilla's doubling fill also writes padding and unused tail slots.
    pub fn fill(&mut self, value: u32) -> Result<(), StorageError> {
        if u64::from(value) > self.mask || value > i32::MAX as u32 {
            return Err(StorageError::ValueTooLarge(value));
        }
        let mut word = u64::from(value);
        let mut shift = u32::from(self.bits);
        while shift < 64 {
            word |= word << shift;
            shift *= 2;
        }
        self.words.fill(word);
        Ok(())
    }

    /// [`unpack`](Self::unpack) into `out` (the first `len` slots), without allocating.
    pub fn unpack_into(&self, out: &mut [u32]) {
        let mut i = 0;
        for &word in &self.words {
            for slot in 0..self.per_word.min(self.len - i) {
                out[i] = ((word >> (slot * usize::from(self.bits))) & self.mask) as u32;
                i += 1;
            }
        }
    }
    pub fn unpack(&self) -> Vec<u32> {
        let mut values = Vec::with_capacity(self.len);
        for &word in &self.words {
            for slot in 0..self.per_word.min(self.len - values.len()) {
                values.push(((word >> (slot * usize::from(self.bits))) & self.mask) as u32);
            }
        }
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_words_never_split_entries() {
        let mut storage = PackedStorage::new(5, 13).unwrap();
        assert_eq!(storage.words().len(), 2);
        storage.set(11, 31).unwrap();
        storage.set(12, 7).unwrap();
        assert_eq!(storage.words(), &[31 << 55, 7]);
    }

    #[test]
    fn all_widths_survive_overwrites_and_unpack() {
        for bits in 1..=32 {
            let mut storage = PackedStorage::new(bits, 4096).unwrap();
            let mut expected = vec![0; 4096];
            for pass in 0..3 {
                for (i, item) in expected.iter_mut().enumerate() {
                    let value = ((i as u64 * 7 + pass) & storage.mask) as u32;
                    assert_eq!(storage.set(i, value).unwrap(), *item);
                    *item = value;
                }
                assert_eq!(storage.unpack(), expected);
            }
        }
    }

    /// The shared multiply-shift index equals division for every width at the
    /// sizes that use `PackedStorage`: 4,096-cell sections and 256-cell
    /// heightmaps. A storage past the exact bound keeps the division path.
    #[test]
    fn multiply_shift_matches_division_at_section_and_heightmap_sizes() {
        for bits in 1..=32u8 {
            for len in [256usize, 4096] {
                let per = 64 / usize::from(bits);
                let mut storage = PackedStorage::new(bits, len).unwrap();
                assert!(!storage.wide);
                let mask = storage.mask;
                for i in 0..len {
                    assert_eq!(
                        storage.slot(i),
                        (i / per, ((i % per) * usize::from(bits)) as u32),
                        "bits {bits} len {len} index {i}"
                    );
                    let value = (i as u64 & mask) as u32;
                    assert_eq!(storage.write(i, value), 0);
                    assert_eq!(storage.read(i), value);
                    assert_eq!(storage.get(i).unwrap(), value);
                }
            }
        }
        let mut wide = PackedStorage::new(1, MAGIC_LEN + 1).unwrap();
        assert!(wide.wide);
        wide.write(MAGIC_LEN, 1);
        assert_eq!(wide.read(MAGIC_LEN), 1);
        assert_eq!(wide.get(MAGIC_LEN).unwrap(), 1);
    }

    #[test]
    fn imported_padding_is_preserved_and_invalid_writes_are_atomic() {
        let mut storage = PackedStorage::from_words(5, 1, vec![u64::MAX]).unwrap();
        assert_eq!(storage.set(0, 0).unwrap(), 31);
        assert_eq!(storage.words(), &[u64::MAX << 5]);
        let unchanged = storage.clone();
        assert!(storage.set(1, 0).is_err());
        assert!(storage.set(0, 32).is_err());
        assert_eq!(storage, unchanged);
        assert!(PackedStorage::new(0, 1).is_err());
        assert!(PackedStorage::new(33, 1).is_err());
        assert!(PackedStorage::from_words(5, 13, vec![0]).is_err());
        assert!(PackedStorage::new(1, 0).unwrap().unpack().is_empty());
    }
}
