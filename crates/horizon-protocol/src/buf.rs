//! Byte buffers with vanilla's primitive encodings (FriendlyByteBuf, VarInt,
//! VarLong, ByteBufCodecs) and the `Codec` building blocks packets are
//! composed from, mirroring `StreamCodec.composite`.
use crate::{Error, Result};
use horizon_world::nbt::{self, Tag};
use std::marker::PhantomData;

/// Vanilla's default NBT quota for network tags (NbtAccounter.defaultQuota).
pub const NBT_QUOTA: usize = 2_097_152;

/// A cursor over a received packet body.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn position(&self) -> usize {
        self.pos
    }
    /// The bytes read since `start` (a `position()`).
    pub fn since(&self, start: usize) -> &'a [u8] {
        &self.data[start.min(self.pos)..self.pos]
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let rest = &self.data[self.pos..];
        self.pos = self.data.len();
        rest
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::new(format!(
                "needed {n} bytes, {} left",
                self.remaining()
            )));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("length checked"))
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.array()?))
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.array()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.array()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_be_bytes(self.array()?))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.array()?))
    }
    /// VarInt.read: at most five 7-bit groups, least significant first.
    pub fn varint(&mut self) -> Result<i32> {
        let mut value = 0u32;
        for i in 0..5 {
            let b = self.u8()?;
            value |= u32::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        Err(Error::new("VarInt too big"))
    }
    /// VarLong.read: at most ten 7-bit groups.
    pub fn varlong(&mut self) -> Result<i64> {
        let mut value = 0u64;
        for i in 0..10 {
            let b = self.u8()?;
            value |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(value as i64);
            }
        }
        Err(Error::new("VarLong too big"))
    }
    /// A VarInt count that must fit in what is left (ByteBufCodecs.readCount).
    pub fn count(&mut self, max: usize) -> Result<usize> {
        let n = self.varint()?;
        let n = usize::try_from(n).map_err(|_| Error::new(format!("negative count {n}")))?;
        if n > max {
            return Err(Error::new(format!(
                "{n} elements is more than allowed {max}"
            )));
        }
        Ok(n)
    }
    /// Utf8String.read: VarInt byte length, UTF-8, at most `max` chars of Java's
    /// String (UTF-16 units: a character outside the BMP counts twice).
    pub fn string(&mut self, max: usize) -> Result<String> {
        let len = self.count(max.saturating_mul(3))?;
        let s = std::str::from_utf8(self.bytes(len)?)
            .map_err(|e| Error::new(format!("bad UTF-8: {e}")))?;
        if s.encode_utf16().count() > max {
            return Err(Error::new(format!("string longer than {max} chars")));
        }
        Ok(s.to_owned())
    }
    pub fn byte_array(&mut self, max: usize) -> Result<Vec<u8>> {
        let len = self.count(max)?;
        Ok(self.bytes(len)?.to_vec())
    }
    pub fn uuid(&mut self) -> Result<u128> {
        Ok(u128::from_be_bytes(self.array()?))
    }
    /// A network NBT tag (NbtIo.readAnyTag without a root name); `End` means none.
    pub fn nbt(&mut self) -> Result<Tag> {
        let limits = nbt::Limits {
            max_bytes: NBT_QUOTA,
            ..nbt::Limits::default()
        };
        let (tag, used) = nbt::read_network(&self.data[self.pos..], limits)
            .map_err(|e| Error::new(format!("NBT: {e}")))?;
        self.pos += used;
        Ok(tag)
    }
    pub fn long_array(&mut self) -> Result<Vec<i64>> {
        let n = self.count(self.remaining() / 8)?;
        (0..n).map(|_| self.i64()).collect()
    }
    pub fn fixed_longs(&mut self, n: usize) -> Result<Vec<i64>> {
        if self.remaining() < n * 8 {
            return Err(Error::new("fixed long array past the end"));
        }
        (0..n).map(|_| self.i64()).collect()
    }
}

/// An outgoing packet body.
#[derive(Default)]
pub struct Writer {
    pub data: Vec<u8>,
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }
    pub fn i8(&mut self, v: i8) {
        self.data.push(v as u8);
    }
    pub fn bool(&mut self, v: bool) {
        self.data.push(u8::from(v));
    }
    pub fn i16(&mut self, v: i16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn f64(&mut self, v: f64) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn varint(&mut self, v: i32) {
        write_varint(&mut self.data, v);
    }
    pub fn varlong(&mut self, v: i64) {
        let mut v = v as u64;
        loop {
            if v & !0x7F == 0 {
                self.data.push(v as u8);
                return;
            }
            self.data.push((v as u8 & 0x7F) | 0x80);
            v >>= 7;
        }
    }
    pub fn string(&mut self, s: &str) {
        self.varint(s.len() as i32);
        self.data.extend_from_slice(s.as_bytes());
    }
    pub fn byte_array(&mut self, b: &[u8]) {
        self.varint(b.len() as i32);
        self.data.extend_from_slice(b);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.data.extend_from_slice(b);
    }
    pub fn uuid(&mut self, v: u128) {
        self.data.extend_from_slice(&v.to_be_bytes());
    }
    pub fn nbt(&mut self, tag: &Tag) {
        match nbt::write_network(tag, nbt::Limits::default()) {
            Ok(bytes) => self.data.extend_from_slice(&bytes),
            // Unencodable tags (too deep) are sent as "no tag".
            Err(_) => self.data.push(0),
        }
    }
    pub fn long_array(&mut self, v: &[i64]) {
        self.varint(v.len() as i32);
        for &l in v {
            self.i64(l);
        }
    }
}
pub fn write_varint(out: &mut Vec<u8>, v: i32) {
    let mut v = v as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}
pub fn varint_size(v: i32) -> usize {
    let v = v as u32;
    (1..5).find(|i| v >> (7 * i) == 0).unwrap_or(5)
}

/// A wire encoding for values of type `T` (vanilla's StreamCodec).
pub trait Codec<T> {
    fn read(r: &mut Reader) -> Result<T>;
    fn write(v: &T, w: &mut Writer);
}
macro_rules! primitive {
    ($name:ident, $ty:ty, $read:ident, $write:ident) => {
        pub struct $name;
        impl Codec<$ty> for $name {
            fn read(r: &mut Reader) -> Result<$ty> {
                r.$read()
            }
            fn write(v: &$ty, w: &mut Writer) {
                w.$write(*v)
            }
        }
    };
}
primitive!(Bool, bool, bool, bool);
primitive!(Byte, i8, i8, i8);
primitive!(UByte, u8, u8, u8);
primitive!(Short, i16, i16, i16);
primitive!(UShort, u16, u16, u16);
primitive!(Int, i32, i32, i32);
primitive!(Long, i64, i64, i64);
primitive!(Float, f32, f32, f32);
primitive!(Double, f64, f64, f64);
primitive!(VarInt, i32, varint, varint);
primitive!(VarLong, i64, varlong, varlong);
primitive!(Uuid, u128, uuid, uuid);

/// ByteBufCodecs.STRING_UTF8 (32767 chars).
pub struct Str;
impl Codec<String> for Str {
    fn read(r: &mut Reader) -> Result<String> {
        r.string(32767)
    }
    fn write(v: &String, w: &mut Writer) {
        w.string(v)
    }
}
/// A string with a smaller vanilla limit, e.g. player names (16).
pub struct StrN<const N: usize>;
impl<const N: usize> Codec<String> for StrN<N> {
    fn read(r: &mut Reader) -> Result<String> {
        r.string(N)
    }
    fn write(v: &String, w: &mut Writer) {
        w.string(v)
    }
}
/// Identifier.STREAM_CODEC: a resource location string.
pub type Ident = StrN<32767>;
/// VarInt-prefixed bytes.
pub struct Bytes;
impl Codec<Vec<u8>> for Bytes {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        r.byte_array(r.remaining())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.byte_array(v)
    }
}
/// ByteBufCodecs.byteArray(N): VarInt-prefixed bytes, at most N.
pub struct BytesN<const N: usize>;
impl<const N: usize> Codec<Vec<u8>> for BytesN<N> {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        r.byte_array(N)
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.byte_array(v)
    }
}
/// Everything left in the packet (custom payloads).
pub struct Rest;
impl Codec<Vec<u8>> for Rest {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        Ok(r.rest().to_vec())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        w.bytes(v)
    }
}
/// Any network NBT tag (`Tag::End` when absent).
pub struct Nbt;
impl Codec<Tag> for Nbt {
    fn read(r: &mut Reader) -> Result<Tag> {
        r.nbt()
    }
    fn write(v: &Tag, w: &mut Writer) {
        w.nbt(v)
    }
}
/// ByteBufCodecs.LONG_ARRAY.
pub struct Longs;
impl Codec<Vec<i64>> for Longs {
    fn read(r: &mut Reader) -> Result<Vec<i64>> {
        r.long_array()
    }
    fn write(v: &Vec<i64>, w: &mut Writer) {
        w.long_array(v)
    }
}
/// ByteBufCodecs.list(): VarInt count then elements.
pub struct List<C>(PhantomData<C>);
impl<T, C: Codec<T>> Codec<Vec<T>> for List<C> {
    fn read(r: &mut Reader) -> Result<Vec<T>> {
        let n = r.count(r.remaining())?;
        (0..n).map(|_| C::read(r)).collect()
    }
    fn write(v: &Vec<T>, w: &mut Writer) {
        w.varint(v.len() as i32);
        for e in v {
            C::write(e, w);
        }
    }
}
/// ByteBufCodecs.optional(): a boolean presence flag.
pub struct Opt<C>(PhantomData<C>);
impl<T, C: Codec<T>> Codec<Option<T>> for Opt<C> {
    fn read(r: &mut Reader) -> Result<Option<T>> {
        if r.bool()? {
            Ok(Some(C::read(r)?))
        } else {
            Ok(None)
        }
    }
    fn write(v: &Option<T>, w: &mut Writer) {
        w.bool(v.is_some());
        if let Some(v) = v {
            C::write(v, w);
        }
    }
}
/// ByteBufCodecs.OPTIONAL_VAR_INT: 0 = empty, otherwise value + 1.
pub struct OptVarInt;
impl Codec<Option<i32>> for OptVarInt {
    fn read(r: &mut Reader) -> Result<Option<i32>> {
        let v = r.varint()?;
        Ok(if v == 0 { None } else { Some(v - 1) })
    }
    fn write(v: &Option<i32>, w: &mut Writer) {
        w.varint(v.map_or(0, |v| v + 1))
    }
}
/// Vec3i.STREAM_CODEC: three var ints.
pub struct VarVec3i;
impl Codec<[i32; 3]> for VarVec3i {
    fn read(r: &mut Reader) -> Result<[i32; 3]> {
        Ok([r.varint()?, r.varint()?, r.varint()?])
    }
    fn write(v: &[i32; 3], w: &mut Writer) {
        for n in v {
            w.varint(*n);
        }
    }
}
/// Vec3.STREAM_CODEC: three doubles.
pub struct Vec3d;
impl Codec<[f64; 3]> for Vec3d {
    fn read(r: &mut Reader) -> Result<[f64; 3]> {
        Ok([r.f64()?, r.f64()?, r.f64()?])
    }
    fn write(v: &[f64; 3], w: &mut Writer) {
        for c in v {
            w.f64(*c);
        }
    }
}
/// BlockPos.STREAM_CODEC: x (26 bits), z (26 bits), y (12 bits) in one long.
pub struct Pos;
impl Codec<[i32; 3]> for Pos {
    fn read(r: &mut Reader) -> Result<[i32; 3]> {
        Ok(unpack_block_pos(r.i64()?))
    }
    fn write(v: &[i32; 3], w: &mut Writer) {
        w.i64(pack_block_pos(*v))
    }
}
pub fn pack_block_pos(p: [i32; 3]) -> i64 {
    ((i64::from(p[0]) & 0x3FF_FFFF) << 38)
        | ((i64::from(p[2]) & 0x3FF_FFFF) << 12)
        | (i64::from(p[1]) & 0xFFF)
}
pub fn unpack_block_pos(v: i64) -> [i32; 3] {
    [
        (v >> 38) as i32,
        (v << 52 >> 52) as i32,
        (v << 26 >> 38) as i32,
    ]
}
/// ChunkPos packed as x in the low and z in the high 32 bits.
pub struct ChunkPos;
impl Codec<[i32; 2]> for ChunkPos {
    fn read(r: &mut Reader) -> Result<[i32; 2]> {
        let v = r.i64()?;
        Ok([v as i32, (v >> 32) as i32])
    }
    fn write(v: &[i32; 2], w: &mut Writer) {
        w.i64((i64::from(v[0]) & 0xFFFF_FFFF) | (i64::from(v[1]) << 32))
    }
}
/// SectionPos: x (22 bits), z (22 bits), y (20 bits).
pub struct SectionPos;
impl Codec<[i32; 3]> for SectionPos {
    fn read(r: &mut Reader) -> Result<[i32; 3]> {
        let v = r.i64()?;
        Ok([
            (v >> 42) as i32,
            (v << 44 >> 44) as i32,
            (v << 22 >> 42) as i32,
        ])
    }
    fn write(v: &[i32; 3], w: &mut Writer) {
        w.i64(
            ((i64::from(v[0]) & 0x3F_FFFF) << 42)
                | ((i64::from(v[2]) & 0x3F_FFFF) << 20)
                | (i64::from(v[1]) & 0xF_FFFF),
        )
    }
}
/// ByteBufCodecs.BIT_SET: java.util.BitSet as a VarInt-prefixed byte array.
pub struct BitSet;
impl Codec<Vec<u8>> for BitSet {
    fn read(r: &mut Reader) -> Result<Vec<u8>> {
        r.byte_array(r.remaining())
    }
    fn write(v: &Vec<u8>, w: &mut Writer) {
        // BitSet.toByteArray drops trailing zero bytes.
        let len = v.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        w.byte_array(&v[..len])
    }
}
pub fn bit(set: &[u8], i: usize) -> bool {
    set.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 != 0)
}
pub fn set_bit(set: &mut Vec<u8>, i: usize) {
    if set.len() <= i / 8 {
        set.resize(i / 8 + 1, 0);
    }
    set[i / 8] |= 1 << (i % 8);
}
/// LpVec3: a low-precision velocity (Vec3.LP_STREAM_CODEC).
pub struct LpVec3;
impl LpVec3 {
    fn unpack(v: u64) -> f64 {
        ((v & 32767) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0
    }
    fn pack(v: f64) -> u64 {
        ((v * 0.5 + 0.5) * 32766.0).round() as u64
    }
    /// `v` as a reader decodes it after it was written.
    pub fn quantize(v: [f64; 3]) -> [f64; 3] {
        let mut w = Writer::new();
        Self::write(&v, &mut w);
        Self::read(&mut Reader::new(&w.data)).unwrap_or([0.0; 3])
    }
    fn sanitize(v: f64) -> f64 {
        if v.is_nan() {
            0.0
        } else {
            v.clamp(-1.717_986_918_3e10, 1.717_986_918_3e10)
        }
    }
}
impl Codec<[f64; 3]> for LpVec3 {
    fn read(r: &mut Reader) -> Result<[f64; 3]> {
        let lowest = u64::from(r.u8()?);
        if lowest == 0 {
            return Ok([0.0; 3]);
        }
        let middle = u64::from(r.u8()?);
        let highest = u64::from(r.u32()?);
        let buffer = highest << 16 | middle << 8 | lowest;
        let mut scale = lowest & 3;
        if lowest & 4 == 4 {
            scale |= (u64::from(r.varint()? as u32)) << 2;
        }
        let s = scale as f64;
        Ok([
            Self::unpack(buffer >> 3) * s,
            Self::unpack(buffer >> 18) * s,
            Self::unpack(buffer >> 33) * s,
        ])
    }
    fn write(v: &[f64; 3], w: &mut Writer) {
        let [x, y, z] = v.map(Self::sanitize);
        let length = x.abs().max(y.abs()).max(z.abs());
        if length < 3.051_944_088_384_301e-5 {
            w.u8(0);
            return;
        }
        let scale = length.ceil() as u64;
        let partial = scale & 3 != scale;
        let markers = if partial { scale & 3 | 4 } else { scale };
        let s = scale as f64;
        let buffer =
            markers | Self::pack(x / s) << 3 | Self::pack(y / s) << 18 | Self::pack(z / s) << 33;
        w.u8(buffer as u8);
        w.u8((buffer >> 8) as u8);
        w.i32((buffer >> 16) as i32);
        if partial {
            w.varint((scale >> 2) as i32);
        }
    }
}
/// A degree angle sent as a byte (Mth.packDegrees / unpackDegrees).
pub fn pack_degrees(deg: f32) -> i8 {
    (deg * 256.0 / 360.0).floor() as i32 as i8
}
pub fn unpack_degrees(b: i8) -> f32 {
    f32::from(b) * 360.0 / 256.0
}

/// Define a packet struct and its codec from (field: type => codec) triples,
/// like vanilla's `StreamCodec.composite`.
#[macro_export]
macro_rules! packet {
    ($(#[$meta:meta])* $name:ident { $($field:ident : $ty:ty => $codec:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name {
            $(pub $field: $ty),*
        }
        impl $crate::buf::Codec<$name> for $name {
            #[allow(unused_variables)]
            fn read(r: &mut $crate::buf::Reader) -> $crate::Result<$name> {
                Ok($name { $($field: <$codec as $crate::buf::Codec<$ty>>::read(r)?),* })
            }
            #[allow(unused_variables)]
            fn write(v: &$name, w: &mut $crate::buf::Writer) {
                $(<$codec as $crate::buf::Codec<$ty>>::write(&v.$field, w);)*
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round<T: PartialEq + std::fmt::Debug, C: Codec<T>>(v: T) -> Vec<u8> {
        let mut w = Writer::new();
        C::write(&v, &mut w);
        let mut r = Reader::new(&w.data);
        assert_eq!(C::read(&mut r).unwrap(), v);
        assert_eq!(r.remaining(), 0);
        w.data
    }

    #[test]
    fn varint_matches_vanilla_examples() {
        // wiki.vg / VarInt.java examples.
        assert_eq!(round::<i32, VarInt>(0), [0x00]);
        assert_eq!(round::<i32, VarInt>(1), [0x01]);
        assert_eq!(round::<i32, VarInt>(127), [0x7f]);
        assert_eq!(round::<i32, VarInt>(128), [0x80, 0x01]);
        assert_eq!(round::<i32, VarInt>(255), [0xff, 0x01]);
        assert_eq!(round::<i32, VarInt>(25565), [0xdd, 0xc7, 0x01]);
        assert_eq!(round::<i32, VarInt>(2097151), [0xff, 0xff, 0x7f]);
        assert_eq!(
            round::<i32, VarInt>(i32::MAX),
            [0xff, 0xff, 0xff, 0xff, 0x07]
        );
        assert_eq!(round::<i32, VarInt>(-1), [0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert_eq!(
            round::<i32, VarInt>(i32::MIN),
            [0x80, 0x80, 0x80, 0x80, 0x08]
        );
        for v in [0, 1, 127, 128, 16383, 16384, 2097151, 2097152, i32::MAX, -1] {
            assert_eq!(varint_size(v), round::<i32, VarInt>(v).len());
        }
        assert!(Reader::new(&[0xff; 6]).varint().is_err());
    }

    #[test]
    fn varlong_matches_vanilla_examples() {
        assert_eq!(round::<i64, VarLong>(0), [0x00]);
        assert_eq!(
            round::<i64, VarLong>(2147483647),
            [0xff, 0xff, 0xff, 0xff, 0x07]
        );
        assert_eq!(
            round::<i64, VarLong>(i64::MAX),
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]
        );
        assert_eq!(
            round::<i64, VarLong>(-1),
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]
        );
        assert_eq!(
            round::<i64, VarLong>(i64::MIN),
            [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]
        );
    }

    #[test]
    fn strings_positions_and_options() {
        assert_eq!(round::<String, Str>("héllo".into()), b"\x06h\xc3\xa9llo");
        let mut w = Writer::new();
        w.string("seventeen chars!!");
        assert!(Reader::new(&w.data).string(16).is_err());
        // Java counts UTF-16 units: 8 emoji are 16 of them, more than 15.
        let mut w = Writer::new();
        w.string(&"😀".repeat(8));
        assert!(Reader::new(&w.data).string(16).is_ok());
        assert!(Reader::new(&w.data).string(15).is_err());
        for p in [
            [0, 0, 0],
            [-1, -64, -1],
            [33_554_431, 2047, -33_554_432],
            [123, -2048, -456],
        ] {
            round::<[i32; 3], Pos>(p);
            assert_eq!(unpack_block_pos(pack_block_pos(p)), p);
        }
        // BlockPos.asLong(1, 2, 3)
        assert_eq!(pack_block_pos([1, 2, 3]), (1 << 38) | (3 << 12) | 2);
        round::<[i32; 3], SectionPos>([-5, -4, 1_000]);
        round::<[i32; 2], ChunkPos>([-7, 12]);
        round::<Option<String>, Opt<Str>>(Some("x".into()));
        round::<Option<String>, Opt<Str>>(None);
        round::<Option<i32>, OptVarInt>(Some(0));
        round::<Vec<i32>, List<VarInt>>(vec![1, 300, -1]);
        round::<u128, Uuid>(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
    }

    #[test]
    fn bitset_drops_trailing_zero_bytes() {
        let mut set = vec![0u8; 4];
        set_bit(&mut set, 3);
        set_bit(&mut set, 9);
        let mut w = Writer::new();
        BitSet::write(&set, &mut w);
        assert_eq!(w.data, [2, 0b1000, 0b10]);
        let read = BitSet::read(&mut Reader::new(&w.data)).unwrap();
        assert!(bit(&read, 3) && bit(&read, 9) && !bit(&read, 4) && !bit(&read, 100));
    }

    #[test]
    fn lp_vec3_round_trips_within_precision() {
        for v in [
            [0.0, 0.0, 0.0],
            [0.1, -0.2, 0.3],
            [1.5, -3.25, 0.0],
            [12.0, 0.0, -40.5],
            [0.0, -3.92, 0.0],
        ] {
            let mut w = Writer::new();
            LpVec3::write(&v, &mut w);
            let back = LpVec3::read(&mut Reader::new(&w.data)).unwrap();
            let scale = v.iter().fold(0.0f64, |m, c| m.max(c.abs())).ceil().max(1.0);
            for i in 0..3 {
                assert!(
                    (back[i] - v[i]).abs() <= scale / 32766.0 * 1.01,
                    "{v:?} -> {back:?}"
                );
            }
        }
        // Zero is a single byte.
        let mut w = Writer::new();
        LpVec3::write(&[0.0, 1e-9, 0.0], &mut w);
        assert_eq!(w.data, [0]);
    }

    #[test]
    fn angles_pack_like_mth() {
        assert_eq!(pack_degrees(90.0), 64);
        assert_eq!(pack_degrees(-90.0), -64);
        assert_eq!(pack_degrees(180.0), -128);
        assert_eq!(unpack_degrees(64), 90.0);
    }
}
