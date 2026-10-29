//! Java Edition NBT with lossless UTF-16 strings and 26.3 mixed-list wrappers.
use std::collections::BTreeMap;

/// Java strings can contain unpaired surrogates, which Rust UTF-8 strings cannot.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaString(pub Vec<u16>);

impl From<&str> for JavaString {
    fn from(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }
}
impl JavaString {
    pub fn to_string(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
}

pub type Compound = BTreeMap<JavaString, Tag>;

/// Float and double variants hold their IEEE bit patterns. Java canonicalizes
/// NaNs on binary output. Loading, like vanilla's valueOf factories, normalizes
/// negative zero to positive zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(u32),
    Double(u64),
    ByteArray(Vec<u8>),
    String(JavaString),
    List(Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Self::End => 0,
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Self::Compound(map) => map.get(&JavaString::from(key)),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        match self {
            Self::Compound(map) => map.get_mut(&JavaString::from(key)),
            _ => None,
        }
    }
    pub fn compound(entries: impl IntoIterator<Item = (impl Into<JavaString>, Tag)>) -> Self {
        Self::Compound(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTag {
    pub name: JavaString,
    pub tag: Tag,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_nodes: 1_000_000,
            max_depth: 512,
        }
    }
}
impl Limits {
    /// The prepared per-state block tables (`cached-collision.nbt`,
    /// `block-properties.nbt`, ...): trusted files with a row per block
    /// state, which content mods' states grow past the default caps
    /// (vanilla's 35,723 states fill about a tenth of these; ATM10's ported
    /// mods, 193,762 states, about half).
    pub fn tables() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_nodes: 128_000_000,
            max_depth: 512,
        }
    }

    /// Chunks, entities, POI and the world's `.dat` files: vanilla reads and
    /// writes them with `NbtAccounter.unlimitedHeap()` (`NbtIo.read`), so a
    /// chunk past the default node cap (a few thousand full containers) still
    /// loads. The byte cap stays as the only guard on the decompressed data.
    pub fn saved() -> Self {
        Self {
            max_bytes: 1 << 30,
            max_nodes: usize::MAX,
            max_depth: 512,
        }
    }

    /// Structure templates (`structure/*.nbt`): vanilla reads them with
    /// `NbtAccounter.unlimitedHeap()` (TemplateSource.readStructure), and
    /// mods ship templates past the default node cap (YUNG's Better Ocean
    /// Monuments' start piece, 115x64x126 blocks, holds millions of tags).
    /// The byte cap stays as the only guard on the decompressed file.
    pub fn templates() -> Self {
        Self {
            max_bytes: 1 << 28,
            max_nodes: usize::MAX,
            max_depth: 512,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub reason: &'static str,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NBT at byte {}: {}", self.offset, self.reason)
    }
}
impl std::error::Error for Error {}

/// Hands a streamed list's elements to the caller one at a time (see
/// [`read_named_streaming`]).
type Visit<'s> = &'s mut dyn FnMut(&mut ListElements<'_, '_, '_>) -> Result<(), Error>;

struct Reader<'a, 's> {
    bytes: &'a [u8],
    offset: usize,
    limits: Limits,
    nodes: usize,
    /// Lists with this key in the root compound or in a compound directly
    /// under it are streamed to the visitor instead of kept.
    stream: Option<(&'s str, Visit<'s>)>,
}
impl<'a> Reader<'a, '_> {
    fn error(&self, reason: &'static str) -> Error {
        Error {
            offset: self.offset,
            reason,
        }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| self.error("length overflow"))?;
        if end > self.limits.max_bytes {
            return Err(self.error("byte limit exceeded"));
        }
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| self.error("truncated input"))?;
        self.offset = end;
        Ok(value)
    }
    fn number<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    fn byte(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn count(&mut self, element_bytes: usize) -> Result<usize, Error> {
        let count = i32::from_be_bytes(self.number()?);
        if count < 0 {
            return Err(self.error("negative collection length"));
        }
        let count = count as usize;
        let size = count
            .checked_mul(element_bytes)
            .ok_or_else(|| self.error("length overflow"))?;
        if size > self.bytes.len().saturating_sub(self.offset)
            || size > self.limits.max_bytes.saturating_sub(self.offset)
        {
            return Err(self.error("collection exceeds available bytes"));
        }
        Ok(count)
    }
    fn string(&mut self) -> Result<JavaString, Error> {
        let len = u16::from_be_bytes(self.number()?) as usize;
        let bytes = self.take(len)?;
        let mut units = Vec::with_capacity(len);
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            let (unit, size) = match b {
                0..=127 => (u16::from(b), 1),
                192..=223 if i + 1 < len && bytes[i + 1] & 0xc0 == 0x80 => {
                    ((u16::from(b & 31) << 6) | u16::from(bytes[i + 1] & 63), 2)
                }
                224..=239
                    if i + 2 < len
                        && bytes[i + 1] & 0xc0 == 0x80
                        && bytes[i + 2] & 0xc0 == 0x80 =>
                {
                    (
                        (u16::from(b & 15) << 12)
                            | (u16::from(bytes[i + 1] & 63) << 6)
                            | u16::from(bytes[i + 2] & 63),
                        3,
                    )
                }
                _ => return Err(self.error("invalid Java modified UTF-8")),
            };
            // Match DataInput.readUTF, including accepted overlong sequences.
            units.push(unit);
            i += size;
        }
        Ok(JavaString(units))
    }
    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag, Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if matches!(id, 9 | 10) && depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        Ok(match id {
            0 => Tag::End,
            1 => Tag::Byte(self.byte()? as i8),
            2 => Tag::Short(i16::from_be_bytes(self.number()?)),
            3 => Tag::Int(i32::from_be_bytes(self.number()?)),
            4 => Tag::Long(i64::from_be_bytes(self.number()?)),
            5 => {
                let bits = u32::from_be_bytes(self.number()?);
                Tag::Float(if f32::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            6 => {
                let bits = u64::from_be_bytes(self.number()?);
                Tag::Double(if f64::from_bits(bits) == 0.0 { 0 } else { bits })
            }
            7 => {
                let len = self.count(1)?;
                Tag::ByteArray(self.take(len)?.to_vec())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let element_id = self.byte()?;
                let len = self.count(0)?;
                if len > 0 && (element_id == 0 || element_id > 12) {
                    return Err(self.error("invalid list element type"));
                }
                if len > self.limits.max_nodes.saturating_sub(self.nodes) {
                    return Err(self.error("node limit exceeded"));
                }
                // At least one byte per valid non-End payload; do not allocate from an unchecked count.
                if len > self.bytes.len().saturating_sub(self.offset) {
                    return Err(self.error("truncated list"));
                }
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    let mut value = self.payload(element_id, depth + 1)?;
                    if let Tag::Compound(map) = &mut value {
                        if map.len() == 1 && map.contains_key(&JavaString::default()) {
                            value = map.remove(&JavaString::default()).unwrap();
                        }
                    }
                    values.push(value);
                }
                Tag::List(values)
            }
            10 => {
                let mut map = Compound::new();
                loop {
                    let id = self.byte()?;
                    if id == 0 {
                        break;
                    }
                    let name = self.string()?;
                    let value = if id == 9 && depth <= 1 && self.streams(&name) {
                        self.stream_list(depth + 1)?
                    } else {
                        self.payload(id, depth + 1)?
                    };
                    map.insert(name, value); // Java maps retain the last duplicate key.
                }
                Tag::Compound(map)
            }
            11 => {
                let len = self.count(4)?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(i32::from_be_bytes(self.number()?));
                }
                Tag::IntArray(values)
            }
            12 => {
                let len = self.count(8)?;
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    values.push(i64::from_be_bytes(self.number()?));
                }
                Tag::LongArray(values)
            }
            _ => return Err(self.error("unknown tag type")),
        })
    }
    fn streams(&self, name: &JavaString) -> bool {
        self.stream
            .as_ref()
            .is_some_and(|(key, _)| name.0.iter().copied().eq(key.encode_utf16()))
    }
    /// A list's payload handed to the visitor element by element; the tree
    /// keeps it empty. Elements the visitor leaves are read and dropped.
    fn stream_list(&mut self, depth: usize) -> Result<Tag, Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        let element_id = self.byte()?;
        let len = self.count(0)?;
        if len > 0 && (element_id == 0 || element_id > 12) {
            return Err(self.error("invalid list element type"));
        }
        if len > self.bytes.len().saturating_sub(self.offset) {
            return Err(self.error("truncated list"));
        }
        let (key, visit) = self.stream.take().expect("streams() checked the visitor");
        let mut elements = ListElements {
            reader: self,
            element_id,
            depth: depth + 1,
            len,
            left: len,
        };
        let rest = visit(&mut elements).and_then(|()| {
            while elements.next_element()?.is_some() {}
            Ok(())
        });
        self.stream = Some((key, visit));
        rest.map(|()| Tag::List(Vec::new()))
    }
}

/// The elements of a list [`read_named_streaming`] streams, read one at a
/// time.
pub struct ListElements<'r, 'a, 's> {
    reader: &'r mut Reader<'a, 's>,
    element_id: u8,
    depth: usize,
    len: usize,
    left: usize,
}
impl ListElements<'_, '_, '_> {
    /// The list's length, as its header gives it.
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// The next element, unwrapped as [`read_named`] unwraps mixed-list
    /// elements; `None` past the last.
    pub fn next_element(&mut self) -> Result<Option<Tag>, Error> {
        if self.left == 0 {
            return Ok(None);
        }
        self.left -= 1;
        let mut value = self.reader.payload(self.element_id, self.depth)?;
        if let Tag::Compound(map) = &mut value {
            if map.len() == 1 && map.contains_key(&JavaString::default()) {
                value = map.remove(&JavaString::default()).unwrap();
            }
        }
        Ok(Some(value))
    }
    /// An error at the current position in the input.
    pub fn error(&self, reason: &'static str) -> Error {
        self.reader.error(reason)
    }
}

/// Read a file root (type, modified-UTF name, payload), returning bytes consumed.
pub fn read_named(bytes: &[u8], limits: Limits) -> Result<(NamedTag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        stream: None,
    };
    let id = reader.byte()?;
    let name = if id == 0 {
        JavaString::default()
    } else {
        reader.string()?
    };
    let tag = reader.payload(id, 0)?;
    Ok((NamedTag { name, tag }, reader.offset))
}

/// [`read_named`], except that lists keyed `key` in the root compound or in
/// a compound directly under it are not kept: `visit` reads each one's
/// elements as they come, and the tree holds it as an empty list. Per-state
/// tables check hundreds of thousands of state identities this way without
/// holding them as a tree (crate::registry::BlockTableFile).
pub fn read_named_streaming(
    bytes: &[u8],
    limits: Limits,
    key: &str,
    visit: &mut dyn FnMut(&mut ListElements<'_, '_, '_>) -> Result<(), Error>,
) -> Result<(NamedTag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        stream: Some((key, visit)),
    };
    let id = reader.byte()?;
    let name = if id == 0 {
        JavaString::default()
    } else {
        reader.string()?
    };
    let tag = reader.payload(id, 0)?;
    Ok((NamedTag { name, tag }, reader.offset))
}

/// Read a network root (type and payload, without the file name field).
pub fn read_network(bytes: &[u8], limits: Limits) -> Result<(Tag, usize), Error> {
    let mut reader = Reader {
        bytes,
        offset: 0,
        limits,
        nodes: 0,
        stream: None,
    };
    let id = reader.byte()?;
    let tag = reader.payload(id, 0)?;
    Ok((tag, reader.offset))
}

struct Writer {
    bytes: Vec<u8>,
    limits: Limits,
    nodes: usize,
}
impl Writer {
    fn error(&self, reason: &'static str) -> Error {
        Error {
            offset: self.bytes.len(),
            reason,
        }
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.limits.max_bytes.saturating_sub(self.bytes.len()) {
            return Err(self.error("byte limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn count(&mut self, len: usize) -> Result<(), Error> {
        let len = i32::try_from(len).map_err(|_| self.error("collection too long"))?;
        self.put(&len.to_be_bytes())
    }
    fn string(&mut self, string: &JavaString) -> Result<(), Error> {
        let len: usize = string
            .0
            .iter()
            .map(|&c| match c {
                1..=127 => 1,
                0..=2047 => 2,
                _ => 3,
            })
            .sum();
        let len =
            u16::try_from(len).map_err(|_| self.error("modified UTF-8 exceeds 65535 bytes"))?;
        self.put(&len.to_be_bytes())?;
        for &c in &string.0 {
            match c {
                1..=127 => self.put(&[c as u8])?,
                0..=2047 => self.put(&[0xc0 | (c >> 6) as u8, 0x80 | (c & 63) as u8])?,
                _ => self.put(&[
                    0xe0 | (c >> 12) as u8,
                    0x80 | ((c >> 6) & 63) as u8,
                    0x80 | (c & 63) as u8,
                ])?,
            }
        }
        Ok(())
    }
    fn payload(&mut self, tag: &Tag, depth: usize) -> Result<(), Error> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.error("node limit exceeded"));
        }
        self.nodes += 1;
        if matches!(tag, Tag::List(_) | Tag::Compound(_)) && depth >= self.limits.max_depth {
            return Err(self.error("depth limit exceeded"));
        }
        match tag {
            Tag::End => {}
            Tag::Byte(v) => self.put(&[*v as u8])?,
            Tag::Short(v) => self.put(&v.to_be_bytes())?,
            Tag::Int(v) => self.put(&v.to_be_bytes())?,
            Tag::Long(v) => self.put(&v.to_be_bytes())?,
            Tag::Float(v) => self.put(
                &if f32::from_bits(*v).is_nan() {
                    0x7fc0_0000_u32
                } else {
                    *v
                }
                .to_be_bytes(),
            )?,
            Tag::Double(v) => self.put(
                &if f64::from_bits(*v).is_nan() {
                    0x7ff8_0000_0000_0000_u64
                } else {
                    *v
                }
                .to_be_bytes(),
            )?,
            Tag::ByteArray(v) => {
                self.count(v.len())?;
                self.put(v)?;
            }
            Tag::String(v) => self.string(v)?,
            Tag::List(values) => {
                if values.iter().any(|v| matches!(v, Tag::End)) {
                    return Err(self.error("End cannot be a list element"));
                }
                let first = values.first().map_or(0, Tag::id);
                let id = if values.iter().any(|v| v.id() != first) {
                    10
                } else {
                    first
                };
                self.put(&[id])?;
                self.count(values.len())?;
                for value in values {
                    let wrapper = matches!(value, Tag::Compound(c) if c.len() == 1 && c.contains_key(&JavaString::default()));
                    if id == 10 && (value.id() != 10 || wrapper) {
                        // Emit one synthetic compound layer without cloning the tree.
                        if depth + 1 >= self.limits.max_depth {
                            return Err(self.error("depth limit exceeded"));
                        }
                        if self.nodes >= self.limits.max_nodes {
                            return Err(self.error("node limit exceeded"));
                        }
                        self.nodes += 1;
                        self.put(&[value.id()])?;
                        self.string(&JavaString::default())?;
                        self.payload(value, depth + 2)?;
                        self.put(&[0])?;
                    } else {
                        self.payload(value, depth + 1)?;
                    }
                }
            }
            Tag::Compound(values) => {
                for (name, value) in values {
                    if matches!(value, Tag::End) {
                        return Err(self.error("End cannot be a named value"));
                    }
                    self.put(&[value.id()])?;
                    self.string(name)?;
                    self.payload(value, depth + 1)?;
                }
                self.put(&[0])?;
            }
            Tag::IntArray(v) => {
                self.count(v.len())?;
                for item in v {
                    self.put(&item.to_be_bytes())?;
                }
            }
            Tag::LongArray(v) => {
                self.count(v.len())?;
                for item in v {
                    self.put(&item.to_be_bytes())?;
                }
            }
        }
        Ok(())
    }
}

pub fn write_named(root: &NamedTag, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limits,
        nodes: 0,
    };
    writer.put(&[root.tag.id()])?;
    if root.tag.id() != 0 {
        writer.string(&root.name)?;
    }
    writer.payload(&root.tag, 0)?;
    Ok(writer.bytes)
}

pub fn write_network(tag: &Tag, limits: Limits) -> Result<Vec<u8>, Error> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limits,
        nodes: 0,
    };
    writer.put(&[tag.id()])?;
    writer.payload(tag, 0)?;
    Ok(writer.bytes)
}

/// SNBT (`TagParser`): compounds, lists, typed arrays, quoted and bare
/// strings, booleans and suffixed numbers. Returns the tag and the bytes read.
pub fn parse_snbt(text: &str) -> Result<(Tag, usize), String> {
    let mut p = Snbt {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let tag = p.value()?;
    Ok((tag, p.i))
}
/// Nesting deeper than NBT's own limit (`NbtAccounter`'s 512) is refused
/// rather than overflowing the stack.
const SNBT_MAX_DEPTH: usize = 512;
struct Snbt<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}
impl Snbt<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }
    fn bare(c: u8) -> bool {
        c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+')
    }
    fn string(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(q @ (b'"' | b'\'')) => {
                self.i += 1;
                let mut out = Vec::new();
                while let Some(&c) = self.s.get(self.i) {
                    self.i += 1;
                    if c == b'\\' {
                        if let Some(&n) = self.s.get(self.i) {
                            self.i += 1;
                            out.push(match n {
                                b'n' => b'\n',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                    } else if c == q {
                        return String::from_utf8(out).map_err(|e| e.to_string());
                    } else {
                        out.push(c);
                    }
                }
                Err("unterminated string".into())
            }
            _ => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|&c| Self::bare(c)) {
                    self.i += 1;
                }
                if start == self.i {
                    return Err(format!("expected a value at {start}"));
                }
                Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
            }
        }
    }
    fn value(&mut self) -> Result<Tag, String> {
        match self.peek() {
            Some(b'{') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let tag = self.compound();
                self.depth -= 1;
                tag
            }
            Some(b'[') => {
                if self.depth >= SNBT_MAX_DEPTH {
                    return Err(format!("tag nesting too deep at {}", self.i));
                }
                self.depth += 1;
                let tag = self.list();
                self.depth -= 1;
                tag
            }
            Some(b'"' | b'\'') => Ok(Tag::String(JavaString::from(self.string()?.as_str()))),
            _ => {
                let word = self.string()?;
                Ok(Self::scalar(&word))
            }
        }
    }
    fn compound(&mut self) -> Result<Tag, String> {
        self.i += 1;
        let mut map = Compound::new();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Tag::Compound(map));
        }
        loop {
            let key = self.string()?;
            self.expect(b':')?;
            map.insert(JavaString::from(key.as_str()), self.value()?);
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Tag::Compound(map));
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
            }
        }
    }
    fn list(&mut self) -> Result<Tag, String> {
        self.i += 1;
        let typed = match (self.s.get(self.i), self.s.get(self.i + 1)) {
            (Some(&t @ (b'B' | b'I' | b'L')), Some(b';')) => {
                self.i += 2;
                Some(t)
            }
            _ => None,
        };
        let mut items = Vec::new();
        if self.peek() != Some(b']') {
            loop {
                items.push(self.value()?);
                match self.peek() {
                    Some(b',') => self.i += 1,
                    Some(b']') => break,
                    _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                }
            }
        }
        self.i += 1;
        let long = |t: &Tag| match *t {
            Tag::Byte(v) => i64::from(v),
            Tag::Short(v) => i64::from(v),
            Tag::Int(v) => i64::from(v),
            Tag::Long(v) => v,
            _ => 0,
        };
        Ok(match typed {
            Some(b'B') => Tag::ByteArray(items.iter().map(|t| long(t) as u8).collect()),
            Some(b'I') => Tag::IntArray(items.iter().map(|t| long(t) as i32).collect()),
            Some(_) => Tag::LongArray(items.iter().map(long).collect()),
            None => Tag::List(items),
        })
    }
    /// A bare word: a typed number, a boolean, or else a string.
    fn scalar(word: &str) -> Tag {
        match word {
            "true" => return Tag::Byte(1),
            "false" => return Tag::Byte(0),
            _ => {}
        }
        let lower = word.to_ascii_lowercase();
        let (body, suffix) = match lower.as_bytes().last() {
            Some(c @ (b'b' | b's' | b'l' | b'f' | b'd')) => (&word[..word.len() - 1], Some(*c)),
            _ => (word, None),
        };
        // Only digits (and an exponent) make a number: a word like `nan` or
        // `infd` that Rust's float parse would take is a string here.
        if body
            .bytes()
            .any(|c| c.is_ascii_alphabetic() && c != b'e' && c != b'E')
        {
            return Tag::String(JavaString::from(word));
        }
        let parsed = match suffix {
            Some(b'b') => body.parse::<i8>().ok().map(Tag::Byte),
            Some(b's') => body.parse::<i16>().ok().map(Tag::Short),
            Some(b'l') => body.parse::<i64>().ok().map(Tag::Long),
            Some(b'f') => body.parse::<f32>().ok().map(|v| Tag::Float(v.to_bits())),
            Some(_) => body.parse::<f64>().ok().map(|v| Tag::Double(v.to_bits())),
            None if body.contains('.') || body.contains('e') || body.contains('E') => {
                body.parse::<f64>().ok().map(|v| Tag::Double(v.to_bits()))
            }
            None => body.parse::<i32>().ok().map(Tag::Int),
        };
        parsed.unwrap_or_else(|| Tag::String(JavaString::from(word)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_lists_reach_the_visitor_and_stay_empty_in_the_tree() {
        let list = |n: i32| Tag::List((0..n).map(Tag::Int).collect());
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("states", list(3)),
                ("inner", Tag::compound([("states", list(2))])),
                (
                    "deep",
                    Tag::compound([("inner", Tag::compound([("states", list(1))]))]),
                ),
                ("other", list(4)),
            ]),
        };
        let bytes = write_named(&root, Limits::default()).unwrap();
        let mut seen = Vec::new();
        let (read, used) = read_named_streaming(&bytes, Limits::default(), "states", &mut |l| {
            let mut values = vec![l.len() as i32];
            // Elements left unread are skipped.
            if let Some(Tag::Int(v)) = l.next_element()? {
                values.push(v);
            }
            seen.push(values);
            Ok(())
        })
        .unwrap();
        assert_eq!(used, bytes.len());
        // Compounds are written in key order: `inner` before `states`.
        assert_eq!(seen, [vec![2, 0], vec![3, 0]]);
        assert_eq!(read.tag.get("states"), Some(&Tag::List(Vec::new())));
        assert_eq!(
            read.tag.get("inner").unwrap().get("states"),
            Some(&Tag::List(Vec::new()))
        );
        // Only the root and its direct children stream.
        assert_eq!(read.tag.get("deep"), root.tag.get("deep"));
        assert_eq!(read.tag.get("other"), root.tag.get("other"));
        // A visitor's error stops the read.
        let failed = read_named_streaming(&bytes, Limits::default(), "states", &mut |l| {
            Err(l.error("refused"))
        });
        assert_eq!(failed.unwrap_err().reason, "refused");
    }
    #[test]
    fn wire_node_budget_includes_mixed_list_wrappers() {
        let tag = Tag::List(vec![Tag::Int(1), Tag::Byte(2)]);
        let limits = Limits {
            max_nodes: 5,
            ..Limits::default()
        };
        let bytes = write_network(&tag, limits).unwrap();
        assert_eq!(read_network(&bytes, limits).unwrap().0, tag);
        let limits = Limits {
            max_nodes: 4,
            ..limits
        };
        assert!(write_network(&tag, limits).is_err());
        assert!(read_network(&bytes, limits).is_err());
    }
    #[test]
    fn templates_read_past_the_default_node_cap() {
        // A big mod template (YUNG's monument start) holds more tags than the
        // default cap; vanilla reads templates with an unlimited accounter.
        let root = NamedTag {
            name: JavaString::default(),
            tag: Tag::compound([("blocks", Tag::List(vec![Tag::Byte(1); 1_000_001]))]),
        };
        let bytes = write_named(&root, Limits::templates()).unwrap();
        assert!(read_named(&bytes, Limits::default()).is_err());
        assert_eq!(read_named(&bytes, Limits::templates()).unwrap().0, root);
    }
    #[test]
    fn depth_boundary_and_every_truncated_prefix() {
        let mut tag = Tag::Int(1);
        for _ in 0..512 {
            tag = Tag::List(vec![tag]);
        }
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(read_network(&bytes, Limits::default()).unwrap().0, tag);
        tag = Tag::List(vec![tag]);
        assert!(write_network(&tag, Limits::default()).is_err());
        let deeper = write_network(
            &tag,
            Limits {
                max_depth: 513,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(read_network(&deeper, Limits::default()).is_err());
        let short = Tag::compound([(
            "list",
            Tag::List(vec![Tag::Long(42), Tag::ByteArray(vec![1, 2, 3])]),
        )]);
        let bytes = write_network(&short, Limits::default()).unwrap();
        for end in 0..bytes.len() {
            assert!(read_network(&bytes[..end], Limits::default()).is_err());
        }
    }
    #[test]
    fn modified_utf_and_mixed_lists_preserve_java_values() {
        let tag = Tag::List(vec![
            Tag::Int(3),
            Tag::String(JavaString(vec![0, 0xd800, 0xdc00, 0xdfff])),
            Tag::compound([("", Tag::Int(9))]),
            Tag::compound([("x", Tag::Byte(1))]),
        ]);
        let data = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(data[1], 10);
        assert_eq!(
            read_network(&data, Limits::default()).unwrap(),
            (tag, data.len())
        );
    }
    #[test]
    fn malformed_lengths_truncation_limits_and_utf_are_rejected() {
        for data in [
            &[7, 0xff, 0xff, 0xff, 0xff][..],
            &[9, 0, 0, 0, 0, 1],
            &[8, 0, 1, 0xc0],
            &[13],
            &[4, 0],
        ] {
            assert!(read_network(data, Limits::default()).is_err());
        }
        let tag = Tag::List(vec![Tag::List(vec![Tag::Int(1)])]);
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_depth: 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_nodes: 2,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            read_network(
                &bytes,
                Limits {
                    max_bytes: bytes.len() - 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        assert!(
            write_network(&Tag::String(JavaString(vec![0; 32768])), Limits::default()).is_err()
        );
    }
    #[test]
    fn duplicate_keys_last_wins_and_nan_output_is_canonical() {
        let bytes = [10, 1, 0, 1, b'x', 1, 1, 0, 1, b'x', 2, 0];
        assert_eq!(
            read_network(&bytes, Limits::default()).unwrap().0.get("x"),
            Some(&Tag::Byte(2))
        );
        assert_eq!(
            write_network(&Tag::Float(0xff80_0001), Limits::default()).unwrap(),
            [5, 0x7f, 0xc0, 0, 0]
        );
        assert_eq!(
            write_network(&Tag::Double((-0.0_f64).to_bits()), Limits::default()).unwrap()[1],
            0x80
        );
    }
    #[test]
    fn snbt_parses_typed_values() {
        let text = "{a:1b,b:[I;1,2],c:'x y',d:[{e:2.5f}],f:true,g:id}";
        let (tag, used) = parse_snbt(&format!("{text}rest")).unwrap();
        assert_eq!(used, text.len());
        assert_eq!(tag.get("a"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("b"), Some(&Tag::IntArray(vec![1, 2])));
        assert_eq!(tag.get("c"), Some(&Tag::String("x y".into())));
        assert_eq!(tag.get("f"), Some(&Tag::Byte(1)));
        assert_eq!(tag.get("g"), Some(&Tag::String("id".into())));
        let Some(Tag::List(d)) = tag.get("d") else {
            panic!()
        };
        assert_eq!(d[0].get("e"), Some(&Tag::Float(2.5f32.to_bits())));
    }
    #[test]
    fn snbt_nesting_is_bounded() {
        let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        assert!(parse_snbt(&nest(512)).is_ok());
        assert!(parse_snbt(&nest(513)).is_err());
        // Far past any limit it errs instead of overflowing the stack.
        assert!(parse_snbt(&nest(100_000)).is_err());
    }
    #[test]
    fn snbt_bare_words_stay_strings() {
        for word in ["nan", "NaN", "inf", "infd", "NaNf", "Infinityd", "nand"] {
            assert_eq!(
                parse_snbt(word).unwrap().0,
                Tag::String(JavaString::from(word)),
                "{word}"
            );
        }
        assert_eq!(
            parse_snbt("1e5").unwrap().0,
            Tag::Double(100000f64.to_bits())
        );
        assert_eq!(parse_snbt("1.5f").unwrap().0, Tag::Float(1.5f32.to_bits()));
        assert_eq!(parse_snbt("3b").unwrap().0, Tag::Byte(3));
        assert_eq!(parse_snbt("-2L").unwrap().0, Tag::Long(-2));
    }
}
