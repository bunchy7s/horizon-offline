//! `net.minecraft.util.HashOps` over Guava's CRC32C: the canonical hash of a
//! codec-encoded value that `HashedStack` sends for each data component.
//!
//! Vanilla runs a `Codec` against `HashOps` (a `DynamicOps<HashCode>`), so every
//! value is hashed as its codec form: a tag byte per type, map entries sorted by
//! their hashes, lists in order. The functions here are `HashOps`' `create*`
//! methods and its record builder; a component's codec form is built from them.
use horizon_world::nbt::Tag;

/// Guava `Hashing.crc32c()`: the CRC-32C (Castagnoli) of the bytes.
pub fn crc32c(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut crc = i as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
                bit += 1;
            }
            table[i] = crc;
            i += 1;
        }
        table
    };
    let mut crc = !0u32;
    for &b in bytes {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

/// A Guava `HashCode` of CRC32C: four bytes (`IntHashCode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashCode(pub u32);
impl HashCode {
    /// `HashCode.asInt`: what `HashedPatchMap` sends for a component.
    pub fn as_int(self) -> i32 {
        self.0 as i32
    }
    /// `HashCode.asBytes`: little-endian, as `IntHashCode` stores it.
    pub fn as_bytes(self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

/// A Guava `Hasher`: little-endian primitives, then the CRC of everything put.
#[derive(Default)]
struct Hasher(Vec<u8>);
impl Hasher {
    fn byte(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn bytes(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self
    }
    fn hash(self) -> HashCode {
        HashCode(crc32c(&self.0))
    }
}

/// `HashOps.CRC32C_INSTANCE`.
pub struct HashOps;
impl HashOps {
    /// `HashOps.empty`: the hash of the payload `[TAG_EMPTY]`.
    pub fn empty() -> HashCode {
        Hasher::default().byte(1).hash()
    }
    pub fn empty_map() -> HashCode {
        Hasher::default().byte(2).byte(3).hash()
    }
    pub fn empty_list() -> HashCode {
        Hasher::default().byte(4).byte(5).hash()
    }
    pub fn create_byte(v: i8) -> HashCode {
        Hasher::default().byte(6).byte(v as u8).hash()
    }
    pub fn create_short(v: i16) -> HashCode {
        Hasher::default().byte(7).bytes(&v.to_le_bytes()).hash()
    }
    pub fn create_int(v: i32) -> HashCode {
        Hasher::default().byte(8).bytes(&v.to_le_bytes()).hash()
    }
    pub fn create_long(v: i64) -> HashCode {
        Hasher::default().byte(9).bytes(&v.to_le_bytes()).hash()
    }
    /// From the float's raw bits (`putFloat` writes `floatToRawIntBits`).
    pub fn create_float(bits: u32) -> HashCode {
        Hasher::default().byte(10).bytes(&bits.to_le_bytes()).hash()
    }
    pub fn create_double(bits: u64) -> HashCode {
        Hasher::default().byte(11).bytes(&bits.to_le_bytes()).hash()
    }
    pub fn create_f32(v: f32) -> HashCode {
        Self::create_float(v.to_bits())
    }
    /// `createString`: the length in UTF-16 units, then each unit (`putUnencodedChars`).
    pub fn create_string_units(units: &[u16]) -> HashCode {
        let mut h = Hasher::default()
            .byte(12)
            .bytes(&(units.len() as i32).to_le_bytes());
        for u in units {
            h = h.bytes(&u.to_le_bytes());
        }
        h.hash()
    }
    pub fn create_string(v: &str) -> HashCode {
        let units: Vec<u16> = v.encode_utf16().collect();
        Self::create_string_units(&units)
    }
    pub fn create_boolean(v: bool) -> HashCode {
        Hasher::default().byte(13).byte(v as u8).hash()
    }
    /// `createMap`: the entries sorted by key hash, then value hash (both as
    /// unsigned ints, `HashCode.padToLong`).
    pub fn create_map(mut entries: Vec<(HashCode, HashCode)>) -> HashCode {
        entries.sort_by_key(|(k, v)| (k.0, v.0));
        let mut h = Hasher::default().byte(2);
        for (k, v) in entries {
            h = h.bytes(&k.as_bytes()).bytes(&v.as_bytes());
        }
        h.byte(3).hash()
    }
    pub fn create_list(values: impl IntoIterator<Item = HashCode>) -> HashCode {
        let mut h = Hasher::default().byte(4);
        for v in values {
            h = h.bytes(&v.as_bytes());
        }
        h.byte(5).hash()
    }
    pub fn create_byte_list(values: &[u8]) -> HashCode {
        Hasher::default().byte(14).bytes(values).byte(15).hash()
    }
    pub fn create_int_list(values: &[i32]) -> HashCode {
        let mut h = Hasher::default().byte(16);
        for v in values {
            h = h.bytes(&v.to_le_bytes());
        }
        h.byte(17).hash()
    }
    pub fn create_long_list(values: &[i64]) -> HashCode {
        let mut h = Hasher::default().byte(18);
        for v in values {
            h = h.bytes(&v.to_le_bytes());
        }
        h.byte(19).hash()
    }

    /// `NbtOps.convertTo(HashOps, tag)`: how an NBT-valued codec (`CompoundTag.CODEC`,
    /// so `custom_data`) is hashed. `bool_keys` names the compound keys whose byte
    /// value is a `Codec.BOOL` (`NbtOps.createBoolean` writes a byte, `HashOps` hashes
    /// a boolean), for the text component codec.
    pub fn from_nbt(tag: &Tag, bool_keys: &[&str]) -> HashCode {
        match tag {
            Tag::End => Self::empty(),
            Tag::Byte(v) => Self::create_byte(*v),
            Tag::Short(v) => Self::create_short(*v),
            Tag::Int(v) => Self::create_int(*v),
            Tag::Long(v) => Self::create_long(*v),
            Tag::Float(bits) => Self::create_float(*bits),
            Tag::Double(bits) => Self::create_double(*bits),
            Tag::ByteArray(v) => Self::create_byte_list(v),
            Tag::String(s) => Self::create_string_units(&s.0),
            Tag::List(items) => {
                Self::create_list(items.iter().map(|t| Self::from_nbt(t, bool_keys)))
            }
            Tag::Compound(map) => Self::create_map(
                map.iter()
                    .map(|(k, v)| {
                        let value = match v {
                            Tag::Byte(b)
                                if bool_keys
                                    .iter()
                                    .any(|n| k.0.iter().copied().eq(n.encode_utf16())) =>
                            {
                                Self::create_boolean(*b != 0)
                            }
                            _ => Self::from_nbt(v, bool_keys),
                        };
                        (Self::create_string_units(&k.0), value)
                    })
                    .collect(),
            ),
            Tag::IntArray(v) => Self::create_int_list(v),
            Tag::LongArray(v) => Self::create_long_list(v),
        }
    }
}

/// `HashOps.MapHashBuilder`: a record codec's fields, hashed as a map. A field
/// that is `None` is absent, as `optionalFieldOf` leaves it out.
#[derive(Default)]
pub struct RecordHash(Vec<(HashCode, HashCode)>);
impl RecordHash {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn field(mut self, name: &str, value: HashCode) -> Self {
        self.0.push((HashOps::create_string(name), value));
        self
    }
    pub fn optional(self, name: &str, value: Option<HashCode>) -> Self {
        match value {
            Some(v) => self.field(name, v),
            None => self,
        }
    }
    pub fn build(self) -> HashCode {
        HashOps::create_map(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CRC-32C check value (RFC 3720, appendix B.4).
    #[test]
    fn crc32c_matches_the_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    }

    #[test]
    fn the_fixed_payload_hashes_are_the_crc_of_their_bytes() {
        assert_eq!(HashOps::empty(), HashCode(crc32c(&[1])));
        assert_eq!(HashOps::empty_map(), HashCode(crc32c(&[2, 3])));
        assert_eq!(HashOps::empty_list(), HashCode(crc32c(&[4, 5])));
        assert_eq!(HashOps::create_boolean(false), HashCode(crc32c(&[13, 0])));
        assert_eq!(HashOps::create_boolean(true), HashCode(crc32c(&[13, 1])));
        // An empty map builds to the same hash as the constant.
        assert_eq!(RecordHash::new().build(), HashOps::empty_map());
        assert_eq!(HashOps::create_list([]), HashOps::empty_list());
    }

    #[test]
    fn primitives_are_a_tag_byte_and_little_endian_payload() {
        assert_eq!(HashOps::create_int(1), HashCode(crc32c(&[8, 1, 0, 0, 0])));
        assert_eq!(
            HashOps::create_short(-2),
            HashCode(crc32c(&[7, 0xFE, 0xFF]))
        );
        assert_eq!(HashOps::create_byte(-1), HashCode(crc32c(&[6, 0xFF])));
        assert_eq!(
            HashOps::create_long(1),
            HashCode(crc32c(&[9, 1, 0, 0, 0, 0, 0, 0, 0]))
        );
        // 1.0f is 0x3F800000.
        assert_eq!(
            HashOps::create_f32(1.0),
            HashCode(crc32c(&[10, 0, 0, 0x80, 0x3F]))
        );
        // Strings: their UTF-16 length, then each unit.
        assert_eq!(
            HashOps::create_string("ab"),
            HashCode(crc32c(&[12, 2, 0, 0, 0, b'a', 0, b'b', 0]))
        );
        // A non-BMP character is two units.
        assert_eq!(
            HashOps::create_string("\u{1F600}"),
            HashCode(crc32c(&[12, 2, 0, 0, 0, 0x3D, 0xD8, 0x00, 0xDE]))
        );
    }

    #[test]
    fn maps_sort_their_entries_by_unsigned_hash_and_lists_keep_order() {
        let a = (HashOps::create_string("a"), HashOps::create_int(1));
        let b = (HashOps::create_string("b"), HashOps::create_int(2));
        assert_eq!(
            HashOps::create_map(vec![a, b]),
            HashOps::create_map(vec![b, a])
        );
        let (first, second) = if a.0.0 < b.0.0 { (a, b) } else { (b, a) };
        let mut bytes = vec![2];
        for (k, v) in [first, second] {
            bytes.extend(k.as_bytes());
            bytes.extend(v.as_bytes());
        }
        bytes.push(3);
        assert_eq!(HashOps::create_map(vec![b, a]), HashCode(crc32c(&bytes)));
        let (x, y) = (HashOps::create_int(1), HashOps::create_int(2));
        assert_ne!(HashOps::create_list([x, y]), HashOps::create_list([y, x]));
    }

    #[test]
    fn nbt_converts_like_nbt_ops_convert_to() {
        let tag = Tag::compound([
            ("n", Tag::Int(7)),
            ("l", Tag::List(vec![Tag::Byte(1)])),
            ("flag", Tag::Byte(1)),
        ]);
        let expected = HashOps::create_map(vec![
            (HashOps::create_string("n"), HashOps::create_int(7)),
            (
                HashOps::create_string("l"),
                HashOps::create_list([HashOps::create_byte(1)]),
            ),
            (HashOps::create_string("flag"), HashOps::create_byte(1)),
        ]);
        assert_eq!(HashOps::from_nbt(&tag, &[]), expected);
        // A text component's `bold` is a boolean in the hash.
        let text = Tag::compound([("bold", Tag::Byte(1))]);
        assert_eq!(
            HashOps::from_nbt(&text, &["bold"]),
            HashOps::create_map(vec![(
                HashOps::create_string("bold"),
                HashOps::create_boolean(true)
            )])
        );
        assert_eq!(
            HashOps::from_nbt(&Tag::List(vec![]), &[]),
            HashOps::empty_list()
        );
        assert_eq!(
            HashOps::from_nbt(&Tag::IntArray(vec![1, 2]), &[]),
            HashCode(crc32c(&[16, 1, 0, 0, 0, 2, 0, 0, 0, 17]))
        );
    }

    /// Values printed by vanilla's own `HashOps.CRC32C_INSTANCE` (`Codec.X.encodeStart`
    /// then `HashCode.asInt`) on the pinned 26.3 jars.
    #[test]
    fn matches_the_hashes_vanilla_prints() {
        assert_eq!(HashOps::create_int(1).as_int(), 1565579036);
        assert_eq!(HashOps::create_string("ab").as_int(), 1228822133);
        assert_eq!(HashOps::create_f32(1.0).as_int(), 1694772624);
        assert_eq!(HashOps::create_boolean(true).as_int(), -1019818302);
        let ints = [1, 2, 3].map(HashOps::create_int);
        assert_eq!(HashOps::create_list(ints).as_int(), -1179931408);
        assert_eq!(HashOps::create_list([]).as_int(), -1978007022);
        // `Unit.CODEC` encodes an empty map.
        assert_eq!(HashOps::empty_map().as_int(), -982207288);
        // CompoundTag.CODEC: {n: 7, flag: 1b, s: "hi", l: [1b], ia: [I;1,2], d: 0.5}.
        let tag = Tag::compound([
            ("n", Tag::Int(7)),
            ("flag", Tag::Byte(1)),
            ("s", Tag::String("hi".into())),
            ("l", Tag::List(vec![Tag::Byte(1)])),
            ("ia", Tag::IntArray(vec![1, 2])),
            ("d", Tag::Double(0.5f64.to_bits())),
        ]);
        assert_eq!(HashOps::from_nbt(&tag, &[]).as_int(), 675583264);
    }
}
