//! Resident `PalettedContainer` of Java Edition 26.3: single-value, linear and
//! hash-map palettes with vanilla's bit-width thresholds, and the global
//! palette beyond them, over `SimpleBitStorage` words (values never span
//! longs). One container type serves blocks (16^3 entries) and biomes (4^3).
//!
//! Growth follows `PalettedContainer.onResize`: a full palette asks for one
//! more bit, the new configuration is built by re-adding the distinct values
//! in index order, and the value that did not fit is added last. Palettes
//! never shrink while resident; `pack` writes the canonical saved form.

use crate::storage::{locate, mask};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `Strategy.createForBlockStates`: 4096 entries, linear palettes up to 4
    /// bits and hash-map palettes from 5 to 8.
    Blocks,
    /// `Strategy.createForBiomes`: 64 entries, linear palettes up to 3 bits.
    Biomes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strategy {
    pub kind: Kind,
    /// `ceillog2` of the registry size: the width of the global palette.
    pub global_bits: u8,
}
/// `getConfigurationForBitCount`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Config {
    Single,
    /// Palette of up to `1 << bits` entries, stored in `bits` bits.
    List(u8),
    Global,
}
impl Strategy {
    pub const fn blocks(global_bits: u8) -> Self {
        Self {
            kind: Kind::Blocks,
            global_bits,
        }
    }
    pub const fn biomes(global_bits: u8) -> Self {
        Self {
            kind: Kind::Biomes,
            global_bits,
        }
    }
    pub const fn entries(self) -> usize {
        match self.kind {
            Kind::Blocks => 4096,
            Kind::Biomes => 64,
        }
    }
    fn config(self, bits: u8) -> Config {
        match (self.kind, bits) {
            (_, 0) => Config::Single,
            (Kind::Blocks, 1..=4) => Config::List(4),
            (Kind::Blocks, 5..=8) => Config::List(bits),
            (Kind::Biomes, 1..=3) => Config::List(bits),
            _ => Config::Global,
        }
    }
    /// Whether a container written with leading byte `bits` lists its palette
    /// (all but the global one).
    pub fn wire_has_palette(self, bits: u8) -> bool {
        self.config(bits) != Config::Global
    }
    /// Bits of the storage a configuration keeps in memory.
    fn storage_bits(self, config: Config) -> u8 {
        match config {
            Config::Single => 0,
            Config::List(bits) => bits,
            Config::Global => self.global_bits,
        }
    }
}
/// `Mth.ceillog2`.
pub fn ceil_log2(n: usize) -> u8 {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as u8
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteError {
    /// A palette bigger than its configuration allows, or empty.
    BadPalette { len: usize },
    /// Storage with the wrong number of longs.
    BadStorage { expected: usize, actual: usize },
    /// A stored index past the palette.
    BadIndex { index: u32, palette: usize },
    /// The declared bit count does not match the palette.
    BadBits { declared: u8, expected: u8 },
}
impl std::fmt::Display for PaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PaletteError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Palette {
    Single(u32),
    /// Linear and hash-map palettes alike: ids in insertion order.
    List(Vec<u32>),
    Global,
}
/// Words of `bits`-wide values: `entries.div_ceil(64 / bits)`.
pub fn storage_len(bits: u8, entries: usize) -> usize {
    if bits == 0 {
        0
    } else {
        entries.div_ceil(64 / usize::from(bits))
    }
}
fn read(words: &[u64], bits: u8, i: usize) -> u32 {
    let (word, shift) = locate(bits, i);
    ((words[word] >> shift) & mask(bits)) as u32
}
fn write(words: &mut [u64], bits: u8, i: usize, value: u32) {
    let (word, shift) = locate(bits, i);
    words[word] = (words[word] & !(mask(bits) << shift)) | (u64::from(value) << shift);
}
/// Values below this are looked up in a table indexed by value once a
/// palette outgrows the scan: every block state and biome id of 26.3.
const DENSE_VALUES: u32 = 1 << 16;
thread_local! {
    /// `palette_ids`' table: 1 + the id of each value seen, 0 for the others.
    /// Only the entries of the palette being built are set, and they are
    /// cleared again before it returns.
    static DENSE_IDS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// The distinct `values` in first-occurrence order and each value's index in
/// them. A run of one value (most of a section) is matched once, a scan finds
/// up to 16 distinct values faster than any lookup, and past that a table
/// indexed by value finds them without hashing.
fn palette_ids(values: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut palette: Vec<u32> = Vec::new();
    let mut ids = Vec::with_capacity(values.len());
    let mut last: Option<(u32, u32)> = None;
    let mut rest = values;
    while let Some((&value, tail)) = rest.split_first() {
        let id = match last {
            Some((v, id)) if v == value => id,
            _ if palette.len() > 16 => break,
            _ => match palette.iter().position(|&v| v == value) {
                Some(id) => id as u32,
                None => {
                    palette.push(value);
                    palette.len() as u32 - 1
                }
            },
        };
        last = Some((value, id));
        ids.push(id);
        rest = tail;
    }
    if rest.is_empty() {
        return (palette, ids);
    }
    if palette.iter().chain(rest).all(|&v| v < DENSE_VALUES) {
        DENSE_IDS.with(|table| {
            let mut table = table.borrow_mut();
            for (id, &v) in palette.iter().enumerate() {
                let v = v as usize;
                if table.len() <= v {
                    table.resize(v + 1, 0);
                }
                table[v] = id as u32 + 1;
            }
            for &value in rest {
                let v = value as usize;
                if table.len() <= v {
                    table.resize(v + 1, 0);
                }
                let id = match table[v] {
                    0 => {
                        palette.push(value);
                        table[v] = palette.len() as u32;
                        palette.len() as u32 - 1
                    }
                    slot => slot - 1,
                };
                ids.push(id);
            }
            for &v in &palette {
                table[v as usize] = 0;
            }
        });
    } else {
        let mut lookup: std::collections::HashMap<u32, u32> = palette
            .iter()
            .enumerate()
            .map(|(i, &v)| (v, i as u32))
            .collect();
        for &value in rest {
            let id = *lookup.entry(value).or_insert_with(|| {
                palette.push(value);
                palette.len() as u32 - 1
            });
            ids.push(id);
        }
    }
    (palette, ids)
}
/// Words holding `ids` in `bits`-wide slots.
fn pack_words(bits: u8, entries: usize, ids: impl Iterator<Item = u32>) -> Vec<u64> {
    let mut words = vec![0u64; storage_len(bits, entries)];
    for (i, id) in ids.enumerate() {
        write(&mut words, bits, i, id);
    }
    words
}
/// The saved form: distinct values in index order, and the packed indices
/// when there is more than one value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packed {
    pub palette: Vec<u32>,
    /// Bits per index in `data` (0 when `data` is absent).
    pub bits: u8,
    pub data: Option<Vec<u64>>,
}

#[derive(Clone, Debug)]
pub struct PalettedContainer {
    strategy: Strategy,
    bits: u8,
    palette: Palette,
    words: Vec<u64>,
}
impl PalettedContainer {
    /// `new PalettedContainer(initialValue, strategy)`.
    pub fn new(strategy: Strategy, value: u32) -> Self {
        Self {
            strategy,
            bits: 0,
            palette: Palette::Single(value),
            words: Vec::new(),
        }
    }
    pub fn strategy(&self) -> Strategy {
        self.strategy
    }
    /// Bits per stored index (`BitStorage.getBits`).
    pub fn bits(&self) -> u8 {
        self.bits
    }
    /// Entries in the palette (the global palette reports its bit capacity).
    pub fn palette_len(&self) -> usize {
        match &self.palette {
            Palette::Single(_) => 1,
            Palette::List(list) => list.len(),
            Palette::Global => 1 << self.bits.min(31),
        }
    }
    pub fn is_global(&self) -> bool {
        matches!(self.palette, Palette::Global)
    }
    #[inline]
    fn raw(&self, i: usize) -> usize {
        let (word, shift) = locate(self.bits, i);
        ((self.words[word] >> shift) & mask(self.bits)) as usize
    }
    #[inline]
    pub fn get(&self, i: usize) -> u32 {
        match &self.palette {
            Palette::Single(value) => *value,
            Palette::List(list) => list[self.raw(i)],
            Palette::Global => self.raw(i) as u32,
        }
    }
    /// The value when every entry is equal, without the palette having to be
    /// minimal.
    pub fn uniform(&self) -> Option<u32> {
        match &self.palette {
            Palette::Single(value) => Some(*value),
            Palette::List(list) if list.len() == 1 => Some(list[0]),
            Palette::Global => self.all_equal(self.get(0)).then(|| self.get(0)),
            Palette::List(list) => {
                let first = list[self.raw(0)];
                self.all_equal(first).then_some(first)
            }
        }
    }
    /// Whether all entries equal `value`.
    pub fn is_uniform(&self, value: u32) -> bool {
        match &self.palette {
            Palette::Single(v) => *v == value,
            Palette::List(list) => match list.iter().position(|&v| v == value) {
                None => false,
                Some(_) if list.len() == 1 => true,
                Some(_) => self.all_equal(value),
            },
            Palette::Global => self.all_equal(value),
        }
    }
    fn all_equal(&self, value: u32) -> bool {
        let id = match &self.palette {
            Palette::Single(v) => return *v == value,
            Palette::List(list) => match list.iter().position(|&v| v == value) {
                Some(id) => id as u64,
                None => return false,
            },
            Palette::Global => u64::from(value),
        };
        let bits = usize::from(self.bits);
        let per = 64 / bits;
        let pattern = (0..per).fold(0u64, |word, slot| word | id << (slot * bits));
        let entries = self.strategy.entries();
        let full = entries / per;
        if self.words[..full].iter().any(|&w| w != pattern) {
            return false;
        }
        let tail = entries - full * per;
        tail == 0 || {
            let used = (1u64 << (tail * bits)) - 1;
            self.words[full] & used == pattern & used
        }
    }
    /// `getAndSet`: stores `value` and returns the previous one.
    pub fn set(&mut self, i: usize, value: u32) -> u32 {
        let id = self.id_for(value);
        match &self.palette {
            Palette::Single(old) => *old,
            Palette::List(list) => {
                let old = read(&self.words, self.bits, i) as usize;
                write(&mut self.words, self.bits, i, id as u32);
                list[old]
            }
            Palette::Global => {
                let old = read(&self.words, self.bits, i);
                write(&mut self.words, self.bits, i, id as u32);
                old
            }
        }
    }
    /// `Palette.idFor`, growing the container when the palette is full.
    fn id_for(&mut self, value: u32) -> usize {
        match &mut self.palette {
            Palette::Single(v) if *v == value => 0,
            Palette::Single(_) => self.resize(1, value),
            Palette::List(list) => {
                if let Some(id) = list.iter().position(|&v| v == value) {
                    id
                } else if list.len() < 1 << self.bits {
                    list.push(value);
                    list.len() - 1
                } else {
                    self.resize(self.bits + 1, value)
                }
            }
            Palette::Global => value as usize,
        }
    }
    /// `PalettedContainer.onResize`: rebuild with the configuration for
    /// `bits`, then add `last` and return its id.
    fn resize(&mut self, bits: u8, last: u32) -> usize {
        let strategy = self.strategy;
        let config = strategy.config(bits);
        let storage_bits = strategy.storage_bits(config);
        let entries = strategy.entries();
        let mut palette = match config {
            Config::Single => Palette::Single(last),
            Config::List(_) => Palette::List(Vec::new()),
            Config::Global => Palette::Global,
        };
        let mut words = vec![0u64; storage_len(storage_bits, entries)];
        let id_in = |palette: &mut Palette, value: u32| -> u32 {
            match palette {
                Palette::Single(_) => 0,
                Palette::List(list) => match list.iter().position(|&v| v == value) {
                    Some(id) => id as u32,
                    None => {
                        list.push(value);
                        list.len() as u32 - 1
                    }
                },
                Palette::Global => value,
            }
        };
        if let Palette::Single(only) = self.palette {
            let id = id_in(&mut palette, only);
            if storage_bits > 0 && id != 0 {
                for i in 0..entries {
                    write(&mut words, storage_bits, i, id);
                }
            }
        } else {
            let (mut last_old, mut new_id) = (u32::MAX, 0);
            for i in 0..entries {
                let old = read(&self.words, self.bits, i);
                if old != last_old {
                    last_old = old;
                    let value = match &self.palette {
                        Palette::List(list) => list[old as usize],
                        _ => old,
                    };
                    new_id = id_in(&mut palette, value);
                }
                if storage_bits > 0 {
                    write(&mut words, storage_bits, i, new_id);
                }
            }
        }
        let id = id_in(&mut palette, last);
        self.palette = palette;
        self.bits = storage_bits;
        self.words = words;
        id as usize
    }
    /// All values in index order.
    pub fn unpack(&self, out: &mut [u32]) {
        assert_eq!(out.len(), self.strategy.entries());
        match &self.palette {
            Palette::Single(value) => out.fill(*value),
            Palette::List(list) => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        let id = ((word >> (slot * bits)) & m) as usize;
                        *cell = list[id];
                    }
                }
            }
            Palette::Global => {
                let bits = usize::from(self.bits);
                let per = 64 / bits;
                let m = mask(self.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&self.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        *cell = ((word >> (slot * bits)) & m) as u32;
                    }
                }
            }
        }
    }
    /// A container holding `values` in the smallest configuration for their
    /// distinct count, with palette ids in first-occurrence order, like a
    /// container unpacked from disk.
    pub fn from_values(strategy: Strategy, values: &[u32]) -> Self {
        assert_eq!(values.len(), strategy.entries());
        let (palette, ids) = palette_ids(values);
        Self::build(strategy, palette, &ids)
    }
    /// A container over a distinct-value `palette` and an id per entry.
    fn build(strategy: Strategy, palette: Vec<u32>, ids: &[u32]) -> Self {
        let config = strategy.config(ceil_log2(palette.len()));
        match config {
            Config::Single => Self::new(strategy, palette[0]),
            Config::List(bits) => Self {
                strategy,
                bits,
                words: pack_words(bits, ids.len(), ids.iter().copied()),
                palette: Palette::List(palette),
            },
            Config::Global => {
                let bits = strategy.global_bits;
                Self {
                    strategy,
                    bits,
                    words: pack_words(bits, ids.len(), ids.iter().map(|&id| palette[id as usize])),
                    palette: Palette::Global,
                }
            }
        }
    }
    /// `PalettedContainer.read`: the container as sent on the wire. `bits` is
    /// the leading byte, `palette` the VarInt entries (a single one for
    /// `bits == 0`, none for the global palette) and `words` the longs.
    pub fn from_wire(
        strategy: Strategy,
        bits: u8,
        palette: Vec<u32>,
        words: Vec<u64>,
    ) -> Result<Self, PaletteError> {
        let config = strategy.config(bits);
        let storage_bits = strategy.storage_bits(config);
        let expected = storage_len(storage_bits, strategy.entries());
        if words.len() != expected {
            return Err(PaletteError::BadStorage {
                expected,
                actual: words.len(),
            });
        }
        match config {
            Config::Single => {
                if palette.len() != 1 {
                    return Err(PaletteError::BadPalette { len: palette.len() });
                }
                Ok(Self::new(strategy, palette[0]))
            }
            Config::List(bits) => {
                if palette.is_empty() || palette.len() > 1 << bits {
                    return Err(PaletteError::BadPalette { len: palette.len() });
                }
                let container = Self {
                    strategy,
                    bits,
                    palette: Palette::List(palette),
                    words,
                };
                container.check_indices()?;
                Ok(container)
            }
            Config::Global => Ok(Self {
                strategy,
                bits: storage_bits,
                palette: Palette::Global,
                words,
            }),
        }
    }
    fn check_indices(&self) -> Result<(), PaletteError> {
        let Palette::List(list) = &self.palette else {
            return Ok(());
        };
        if list.len() == 1 << self.bits {
            return Ok(());
        }
        let per = 64 / usize::from(self.bits);
        let mut remaining = self.strategy.entries();
        let mask = mask(self.bits);
        for &word in &self.words {
            let count = remaining.min(per);
            for slot in 0..count {
                let index = ((word >> (slot * usize::from(self.bits))) & mask) as u32;
                if index as usize >= list.len() {
                    return Err(PaletteError::BadIndex {
                        index,
                        palette: list.len(),
                    });
                }
            }
            remaining -= count;
        }
        Ok(())
    }
    /// `PalettedContainer.write`: the leading bits byte, the palette entries
    /// (none for the global palette) and the raw longs, exactly as resident.
    pub fn to_wire(&self) -> (u8, Vec<u32>, &[u64]) {
        let palette = match &self.palette {
            Palette::Single(value) => vec![*value],
            Palette::List(list) => list.clone(),
            Palette::Global => Vec::new(),
        };
        (self.bits, palette, &self.words)
    }
    /// `PalettedContainer.pack`: distinct values in index order, indices
    /// re-encoded for the saved bit width.
    pub fn pack(&self) -> Packed {
        let entries = self.strategy.entries();
        let mut values = vec![0u32; entries];
        self.unpack(&mut values);
        let (palette, ids) = palette_ids(&values);
        let bits = match self.strategy.config(ceil_log2(palette.len())) {
            Config::Single => 0,
            Config::List(bits) => bits,
            Config::Global => ceil_log2(palette.len()),
        };
        let data = (bits != 0).then(|| pack_words(bits, entries, ids.iter().copied()));
        Packed {
            palette,
            bits,
            data,
        }
    }
    /// `PalettedContainer.unpack`: a container from the saved form.
    pub fn from_packed(
        strategy: Strategy,
        palette: Vec<u32>,
        data: Option<&[u64]>,
    ) -> Result<Self, PaletteError> {
        if palette.is_empty() {
            return Err(PaletteError::BadPalette { len: 0 });
        }
        let config = strategy.config(ceil_log2(palette.len()));
        let entries = strategy.entries();
        let disc_bits = match config {
            Config::Single => return Ok(Self::new(strategy, palette[0])),
            Config::List(bits) => bits,
            Config::Global => ceil_log2(palette.len()),
        };
        let data = data.ok_or(PaletteError::BadStorage {
            expected: storage_len(disc_bits, entries),
            actual: 0,
        })?;
        let expected = storage_len(disc_bits, entries);
        if data.len() != expected {
            return Err(PaletteError::BadStorage {
                expected,
                actual: data.len(),
            });
        }
        if let Config::List(bits) = config {
            // List palettes already use the saved indices as resident storage.
            // Validate them without allocating an unpacked copy that is unused.
            let container = Self {
                strategy,
                bits,
                words: data.to_vec(),
                palette: Palette::List(palette),
            };
            container.check_indices()?;
            return Ok(container);
        }
        let mut ids = Vec::with_capacity(entries);
        for i in 0..entries {
            let index = read(data, disc_bits, i);
            if index as usize >= palette.len() {
                return Err(PaletteError::BadIndex {
                    index,
                    palette: palette.len(),
                });
            }
            ids.push(index);
        }
        Ok(Self::build(strategy, palette, &ids))
    }
    /// Whether any palette entry satisfies `predicate` (`maybeHas`): entries
    /// no longer used by any position still count.
    pub fn maybe_has(&self, mut predicate: impl FnMut(u32) -> bool) -> bool {
        match &self.palette {
            Palette::Single(value) => predicate(*value),
            Palette::List(list) => list.iter().any(|&v| predicate(v)),
            Palette::Global => (0..1u32 << self.bits.min(31)).any(predicate),
        }
    }
    /// Whether any entry satisfies `predicate`. Unlike [`Self::maybe_has`],
    /// palette entries no entry uses any more don't count.
    pub fn has(&self, mut predicate: impl FnMut(u32) -> bool) -> bool {
        let list = match &self.palette {
            Palette::Single(value) => return predicate(*value),
            Palette::List(list) => list,
            Palette::Global => {
                return (0..self.strategy.entries()).any(|i| predicate(self.raw(i) as u32));
            }
        };
        let matching: Vec<bool> = list.iter().map(|&v| predicate(v)).collect();
        let count = matching.iter().filter(|&&m| m).count();
        if count == 0 || count == list.len() {
            return count != 0;
        }
        let (bits, entries) = (usize::from(self.bits), self.strategy.entries());
        let per = 64 / bits;
        // The words whose fields all hold entries; the last may be partial.
        let full = entries / per;
        if count <= 4 {
            // A field-wise zero test of each word XORed with the matching
            // index in every field: `(v - lo) & !v & hi` is non-zero exactly
            // when some field of `v` is zero.
            let lo = (0..per).fold(0u64, |a, f| a | 1 << (f * bits));
            let hi = lo << (bits - 1);
            let words = &self.words[..full];
            let found = matching
                .iter()
                .enumerate()
                .filter(|(_, m)| **m)
                .any(|(i, _)| {
                    let pattern = lo * i as u64;
                    words.iter().any(|&w| {
                        let v = w ^ pattern;
                        v.wrapping_sub(lo) & !v & hi != 0
                    })
                });
            return found || (full * per..entries).any(|i| matching[self.raw(i)]);
        }
        (0..entries).any(|i| matching[self.raw(i)])
    }
    /// Heap bytes held by the palette and the storage.
    pub fn heap_bytes(&self) -> usize {
        self.words.capacity() * 8
            + match &self.palette {
                Palette::List(list) => list.capacity() * 4,
                _ => 0,
            }
    }
}
impl PartialEq for PalettedContainer {
    /// Equal when every entry is.
    fn eq(&self, other: &Self) -> bool {
        let entries = self.strategy.entries();
        entries == other.strategy.entries() && (0..entries).all(|i| self.get(i) == other.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BLOCKS: Strategy = Strategy::blocks(16);
    const BIOMES: Strategy = Strategy::biomes(7);

    #[test]
    fn locate_matches_division_for_every_width() {
        for bits in 1..=32u8 {
            let per = 64 / usize::from(bits);
            for i in 0..4096 {
                let expected = (i / per, ((i % per) * usize::from(bits)) as u32);
                assert_eq!(locate(bits, i), expected, "{bits} bits, entry {i}");
            }
        }
    }
    fn rng(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
    #[test]
    fn grows_through_vanilla_widths() {
        let mut c = PalettedContainer::new(BLOCKS, 0);
        assert_eq!((c.bits(), c.palette_len()), (0, 1));
        let mut expected = Vec::new();
        for n in 1..=300u32 {
            c.set(n as usize, n);
            expected.push(match n + 1 {
                2..=16 => 4,
                17..=32 => 5,
                33..=64 => 6,
                65..=128 => 7,
                129..=256 => 8,
                _ => 16,
            });
            assert_eq!(c.bits(), *expected.last().unwrap(), "after {n} values");
        }
        assert!(c.is_global());
        for n in 1..=300u32 {
            assert_eq!(c.get(n as usize), n);
        }
        let mut b = PalettedContainer::new(BIOMES, 3);
        for n in 1..=9u32 {
            b.set(n as usize, n + 3);
            let want = match n + 1 {
                2 => 1,
                3..=4 => 2,
                5..=8 => 3,
                _ => 7,
            };
            assert_eq!(b.bits(), want, "after {n} biomes");
        }
    }
    #[test]
    fn matches_a_plain_array_under_random_edits() {
        for (strategy, distinct) in [(BLOCKS, 700u64), (BLOCKS, 12), (BIOMES, 9), (BIOMES, 3)] {
            let entries = strategy.entries();
            let mut state = 0x1234_5678_9ABC_DEF1u64;
            let mut c = PalettedContainer::new(strategy, 5);
            let mut plain = vec![5u32; entries];
            for _ in 0..20_000 {
                let r = rng(&mut state);
                let i = (r % entries as u64) as usize;
                let v = ((r >> 20) % distinct) as u32 * 7;
                assert_eq!(c.set(i, v), plain[i]);
                plain[i] = v;
            }
            let mut out = vec![0; entries];
            c.unpack(&mut out);
            assert_eq!(out, plain);
            for (i, &v) in plain.iter().enumerate() {
                assert_eq!(c.get(i), v);
            }
            let packed = c.pack();
            let loaded = PalettedContainer::from_packed(
                strategy,
                packed.palette.clone(),
                packed.data.as_deref(),
            )
            .unwrap();
            assert_eq!(loaded, c);
            assert_eq!(loaded.pack(), packed);
            let again = PalettedContainer::from_values(strategy, &plain);
            assert_eq!(again, c);
            assert_eq!(again.pack(), packed);
            let (bits, palette, words) = c.to_wire();
            let wire =
                PalettedContainer::from_wire(strategy, bits, palette, words.to_vec()).unwrap();
            assert_eq!(wire, c);
            for wanted in [0, 7, 14, 35, 7 * 699, 5] {
                assert_eq!(
                    c.has(|v| v == wanted || v == wanted + 7),
                    plain.iter().any(|&v| v == wanted || v == wanted + 7),
                    "has {wanted} in {distinct} values"
                );
            }
        }
    }
    #[test]
    fn has_matches_a_scan_at_every_width() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for (strategy, distinct) in [
            (BLOCKS, 2u32),
            (BLOCKS, 3),
            (BLOCKS, 17),
            (BLOCKS, 33),
            (BLOCKS, 65),
            (BLOCKS, 129),
            (BIOMES, 2),
            (BIOMES, 3),
            (BIOMES, 5),
        ] {
            let entries = strategy.entries();
            let mut c = PalettedContainer::new(strategy, 0);
            let mut plain = vec![0u32; entries];
            for i in 0..entries {
                let v = (rng(&mut state) % u64::from(distinct)) as u32;
                c.set(i, v);
                plain[i] = v;
            }
            // Take some values away again (stale entries), the last index too.
            for gone in [0, 1, distinct - 1] {
                for i in 0..entries {
                    if plain[i] == gone {
                        c.set(i, distinct);
                        plain[i] = distinct;
                    }
                }
                for m in [3, 5, 7] {
                    let hit = |v: u32| v % m == gone % m;
                    assert_eq!(c.has(hit), plain.iter().any(|&v| hit(v)));
                }
                for wanted in 0..=distinct {
                    for other in [wanted, wanted + 1, wanted + 2, wanted + 3, wanted + 4] {
                        let hit = |v: u32| v == wanted || v == other;
                        assert_eq!(
                            c.has(hit),
                            plain.iter().any(|&v| hit(v)),
                            "{distinct} values, {wanted}/{other}, without {gone}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn has_ignores_stale_palette_entries() {
        let mut c = PalettedContainer::new(BLOCKS, 0);
        assert!(c.has(|v| v == 0) && !c.has(|v| v == 3));
        c.set(4095, 3);
        assert!(c.has(|v| v == 3));
        c.set(4095, 0);
        assert!(c.maybe_has(|v| v == 3) && !c.has(|v| v == 3));
        for v in 1..=300 {
            c.set(v as usize, v);
        }
        assert!(c.is_global() && c.has(|v| v == 300));
        c.set(300, 0);
        assert!(c.maybe_has(|v| v == 300) && !c.has(|v| v == 300));
    }
    #[test]
    fn uniform_detection_ignores_stale_palette_entries() {
        let mut c = PalettedContainer::new(BLOCKS, 0);
        c.set(10, 3);
        assert!(!c.is_uniform(0));
        c.set(10, 0);
        assert!(c.palette_len() == 2 && c.is_uniform(0) && c.uniform() == Some(0));
        assert!(!c.is_uniform(3));
        for i in 0..4096 {
            c.set(i, 3);
        }
        assert_eq!(c.uniform(), Some(3));
    }
    #[test]
    fn rejects_malformed_wire_data() {
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0; 255]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0x3333; 256]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![], vec![0; 256]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![1, 2], vec![]).is_err());
        assert!(PalettedContainer::from_wire(BIOMES, 2, vec![1, 2, 3, 4, 5], vec![0; 4]).is_err());
        assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![9], vec![]).is_ok());
    }
    #[test]
    fn saved_validation_matches_entrywise_reference() {
        // Keep the previous loader as an independent entry-at-a-time reference:
        // errors, resident palette order and unused storage bits must all agree.
        fn reference(
            strategy: Strategy,
            palette: Vec<u32>,
            data: Option<&[u64]>,
        ) -> Result<PalettedContainer, PaletteError> {
            if palette.is_empty() {
                return Err(PaletteError::BadPalette { len: 0 });
            }
            let config = strategy.config(ceil_log2(palette.len()));
            let entries = strategy.entries();
            let bits = match config {
                Config::Single => return Ok(PalettedContainer::new(strategy, palette[0])),
                Config::List(bits) => bits,
                Config::Global => ceil_log2(palette.len()),
            };
            let expected = storage_len(bits, entries);
            let data = data.ok_or(PaletteError::BadStorage {
                expected,
                actual: 0,
            })?;
            if data.len() != expected {
                return Err(PaletteError::BadStorage {
                    expected,
                    actual: data.len(),
                });
            }
            let mut ids = Vec::with_capacity(entries);
            for i in 0..entries {
                let index = read(data, bits, i);
                if index as usize >= palette.len() {
                    return Err(PaletteError::BadIndex {
                        index,
                        palette: palette.len(),
                    });
                }
                ids.push(index);
            }
            Ok(match config {
                Config::List(bits) => PalettedContainer {
                    strategy,
                    bits,
                    palette: Palette::List(palette),
                    words: data.to_vec(),
                },
                _ => PalettedContainer::build(strategy, palette, &ids),
            })
        }
        fn compare(strategy: Strategy, palette: &[u32], data: Option<&[u64]>) {
            let expected = reference(strategy, palette.to_vec(), data);
            let actual = PalettedContainer::from_packed(strategy, palette.to_vec(), data);
            match (actual, expected) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a.strategy, b.strategy);
                    assert_eq!(a.bits, b.bits);
                    assert_eq!(a.palette, b.palette);
                    assert_eq!(a.words, b.words);
                }
                (Err(a), Err(b)) => assert_eq!(a, b),
                (a, b) => panic!("loader disagreement: {a:?} vs {b:?}"),
            }
        }
        for strategy in [BLOCKS, BIOMES] {
            for len in [
                0, 1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255,
                256, 257, 511, 512, 513, 1023, 1024, 1025, 2047, 2048, 2049, 4096, 4097,
            ] {
                let palette: Vec<u32> = (0..len).map(|i| (i as u32 * 7) % 127).collect();
                compare(strategy, &palette, None);
                compare(strategy, &palette, Some(&[]));
                if len <= 1 {
                    compare(strategy, &palette, Some(&[u64::MAX]));
                    continue;
                }
                let bits = match strategy.config(ceil_log2(len)) {
                    Config::List(bits) => bits,
                    _ => ceil_log2(len),
                };
                let entries = strategy.entries();
                let mut words = pack_words(bits, entries, (0..entries).map(|i| (i % len) as u32));
                compare(strategy, &palette, Some(&words));
                compare(strategy, &palette, Some(&words[..words.len() - 1]));
                // Nonzero padding in every word and in the final partial word
                // is legal and must be preserved by resident list palettes.
                let per = 64 / usize::from(bits);
                for (word, positions) in words.iter_mut().zip((0..entries).step_by(per)) {
                    let used = (entries - positions).min(per) * usize::from(bits);
                    if used < 64 {
                        *word |= u64::MAX << used;
                    }
                }
                compare(strategy, &palette, Some(&words));
                if len < 1usize << bits {
                    for i in [0, per - 1, per, entries / 2, entries - 1] {
                        let mut invalid = words.clone();
                        write(&mut invalid, bits, i, len as u32);
                        // A second invalid entry verifies first-error ordering.
                        write(&mut invalid, bits, entries - 1, mask(bits) as u32);
                        compare(strategy, &palette, Some(&invalid));
                    }
                }
            }
        }
    }
    #[test]
    fn division_magic_is_exact() {
        for bits in 1..=32u8 {
            let per = 64 / usize::from(bits);
            for i in 0..4096usize {
                assert_eq!(locate(bits, i).0, i / per, "bits {bits} index {i}");
            }
        }
    }

    /// The pre-optimization `unpack`, kept to compare against in the benchmark.
    fn baseline_unpack(c: &PalettedContainer, out: &mut [u32]) {
        match &c.palette {
            Palette::Single(value) => out.fill(*value),
            palette => {
                let bits = usize::from(c.bits);
                let per = 64 / bits;
                let m = mask(c.bits);
                for (chunk, &word) in out.chunks_mut(per).zip(&c.words) {
                    for (slot, cell) in chunk.iter_mut().enumerate() {
                        let id = ((word >> (slot * bits)) & m) as u32;
                        *cell = match palette {
                            Palette::List(list) => list[id as usize],
                            _ => id,
                        };
                    }
                }
            }
        }
    }

    /// `unpack` against the pre-optimization implementation, for a list palette
    /// and the global palette. The result is checked before timing; 25 measured
    /// rounds alternate order and report the median and best microsecond-per-call.
    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_unpack() {
        use std::{hint::black_box, time::Instant};
        let values: Vec<u32> = (0..4096).map(|i| (i % 12) as u32).collect();
        let list = PalettedContainer::from_values(BLOCKS, &values);
        let global = PalettedContainer::from_values(
            BLOCKS,
            &(0..4096).map(|i| (i * 3) as u32).collect::<Vec<_>>(),
        );
        for (name, container) in [("list", &list), ("global", &global)] {
            let mut expected = vec![0u32; 4096];
            baseline_unpack(container, &mut expected);
            let mut out = vec![0u32; 4096];
            container.unpack(&mut out);
            assert_eq!(expected, out, "unpack {name} differs");
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..25 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let started = Instant::now();
                    for _ in 0..2000 {
                        if optimized {
                            container.unpack(black_box(&mut out));
                        } else {
                            baseline_unpack(black_box(container), black_box(&mut out));
                        }
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / 2000.0);
                    }
                }
            }
            for values in &mut times {
                values.sort_by(f64::total_cmp);
            }
            let median = |v: &[f64]| v[v.len() / 2];
            println!(
                "unpack {name} median: baseline {:.3} us optimized {:.3} us speedup {:.3}x",
                median(&times[0]),
                median(&times[1]),
                median(&times[0]) / median(&times[1])
            );
            println!(
                "unpack {name} best:   baseline {:.3} us optimized {:.3} us speedup {:.3}x",
                times[0][0],
                times[1][0],
                times[0][0] / times[1][0]
            );
        }
    }

    /// The pre-optimization `palette_ids` (scan, then a SipHash map), kept to
    /// compare against.
    fn baseline_palette_ids(values: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let mut palette: Vec<u32> = Vec::new();
        let mut small = true;
        let mut lookup: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut ids = Vec::with_capacity(values.len());
        for &value in values {
            let id = if small {
                match palette.iter().position(|&v| v == value) {
                    Some(id) => id as u32,
                    None => {
                        palette.push(value);
                        if palette.len() > 16 {
                            small = false;
                            lookup.extend(palette.iter().enumerate().map(|(i, &v)| (v, i as u32)));
                        }
                        palette.len() as u32 - 1
                    }
                }
            } else {
                *lookup.entry(value).or_insert_with(|| {
                    palette.push(value);
                    palette.len() as u32 - 1
                })
            };
            ids.push(id);
        }
        (palette, ids)
    }
    /// Sections like the ones saved: stone, ores and caves under dirt, grass
    /// and air; a mixed surface (plants, leaves, logs) in short runs; a
    /// global palette; ids past the table; and biomes.
    fn palette_cases() -> Vec<(&'static str, Vec<u32>)> {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut terrain = Vec::with_capacity(4096);
        for i in 0..4096u32 {
            let y = i >> 8;
            let r = rng(&mut state);
            terrain.push(match y {
                0..=8 if r % 40 == 0 => 2000 + (r >> 8) as u32 % 6,
                0..=8 if r % 13 == 0 => 0,
                0..=8 => 1,
                9..=10 => 10,
                11 => 9,
                _ => 0,
            });
        }
        let mut surface = Vec::with_capacity(4096);
        while surface.len() < 4096 {
            let r = rng(&mut state);
            let run = 1 + (r % 8) as usize;
            let v = 100 + ((r >> 8) % 24) as u32 * 37;
            surface.extend(std::iter::repeat_n(v, run.min(4096 - surface.len())));
        }
        let global: Vec<u32> = (0..4096)
            .map(|_| (rng(&mut state) % 600) as u32 * 41)
            .collect();
        let large: Vec<u32> = (0..4096)
            .map(|i| {
                if i % 3 == 0 {
                    70_000 + (rng(&mut state) % 40) as u32
                } else {
                    i as u32 % 20
                }
            })
            .collect();
        let biomes: Vec<u32> = (0..64).map(|i| [3, 3, 3, 7, 7, 12][i % 6]).collect();
        vec![
            ("terrain", terrain),
            ("surface", surface),
            ("global", global),
            ("large ids", large),
            ("biomes", biomes),
            ("single", vec![5; 4096]),
            ("seventeen", (0..4096).map(|i| (i % 17) as u32).collect()),
        ]
    }
    #[test]
    fn palette_ids_match_first_occurrence_order() {
        for (name, values) in palette_cases() {
            assert_eq!(
                palette_ids(&values),
                baseline_palette_ids(&values),
                "{name}"
            );
            // The table is left clean for the next section.
            assert_eq!(
                palette_ids(&values),
                baseline_palette_ids(&values),
                "{name} again"
            );
        }
        DENSE_IDS.with(|t| assert!(t.borrow().iter().all(|&slot| slot == 0)));
    }
    /// `palette_ids` against the pre-optimization implementation over the
    /// cases above. 25 measured rounds alternate order and report the median
    /// microseconds per call.
    #[test]
    #[ignore = "microbenchmark; run with --ignored --nocapture"]
    fn benchmark_palette_ids() {
        use std::{hint::black_box, time::Instant};
        for (name, values) in palette_cases() {
            let mut times = [Vec::new(), Vec::new()];
            for round in 0..25 {
                for optimized in [round % 2 == 0, round % 2 != 0] {
                    let started = Instant::now();
                    for _ in 0..500 {
                        if optimized {
                            black_box(palette_ids(black_box(&values)));
                        } else {
                            black_box(baseline_palette_ids(black_box(&values)));
                        }
                    }
                    if round > 0 {
                        times[usize::from(optimized)]
                            .push(started.elapsed().as_secs_f64() * 1e6 / 500.0);
                    }
                }
            }
            for t in &mut times {
                t.sort_by(f64::total_cmp);
            }
            let (base, new) = (times[0][times[0].len() / 2], times[1][times[1].len() / 2]);
            println!(
                "palette_ids {name}: baseline {base:.2} us, new {new:.2} us, {:.2}x",
                base / new
            );
        }
    }
}
