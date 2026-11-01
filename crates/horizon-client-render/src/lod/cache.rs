//! Decorated far-terrain chunks kept on disk, beside a dimension's save, so a
//! world opened again shows them without generating them again. One append-only
//! file per 32x32 chunk region; a file written by another build, block registry
//! or format is cleared on first use.
use super::{Column, Span, Surface};
use crate::fast_hash::FxHashMap;
use horizon_world::registry::BlockStateId;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
};

const MAGIC: &[u8; 4] = b"HLOD";
const FORMAT: u32 = 1;
const HEADER: u64 = 16;
/// A record: chunk x and z, then the deflated columns' length.
const RECORD: u64 = 12;
/// Columns of a chunk are a few kilobytes deflated; anything larger is damage.
const MAX_RECORD: u32 = 1 << 22;

type Columns = Arc<[Column; 256]>;

struct Region {
    file: File,
    /// Each chunk's latest record: the offset of its deflated columns, and
    /// their length.
    index: FxHashMap<(i32, i32), (u64, u32)>,
}

pub struct DiskCache {
    dir: PathBuf,
    key: u64,
    // ponytail: one lock for every region, with file reads under it; per-region
    // locks if the sampling threads ever wait on it.
    regions: Mutex<FxHashMap<(i32, i32), Option<Region>>>,
}

impl DiskCache {
    /// `key`: what the cached columns depend on (`build_key`); files with
    /// another are cleared.
    pub fn new(dir: PathBuf, key: u64) -> Self {
        Self {
            dir,
            key,
            regions: Mutex::default(),
        }
    }

    pub fn get(&self, x: i32, z: i32) -> Option<Columns> {
        let mut regions = self.regions.lock().ok()?;
        let region = self.region(&mut regions, x, z, false)?;
        let &(offset, len) = region.index.get(&(x, z))?;
        let mut bytes = vec![0; len as usize];
        region.file.seek(SeekFrom::Start(offset)).ok()?;
        region.file.read_exact(&mut bytes).ok()?;
        drop(regions);
        decode(&bytes).map(Arc::new)
    }

    fn contains(&self, x: i32, z: i32) -> bool {
        let Ok(mut regions) = self.regions.lock() else {
            return false;
        };
        self.region(&mut regions, x, z, false)
            .is_some_and(|region| region.index.contains_key(&(x, z)))
    }

    /// Writes a chunk not cached yet. One already cached stays as it is: the
    /// build key clears stale ones, and a generation batch hands back
    /// neighbours cached before, which appended again grew the files each
    /// session.
    pub fn put(&self, x: i32, z: i32, columns: &[Column; 256]) {
        if self.contains(x, z) {
            return;
        }
        let bytes = encode(columns);
        let Ok(mut regions) = self.regions.lock() else {
            return;
        };
        let Some(region) = self.region(&mut regions, x, z, true) else {
            return;
        };
        // Another sampler's thread may have written it while this one encoded.
        if region.index.contains_key(&(x, z)) {
            return;
        }
        let mut record = Vec::with_capacity(RECORD as usize + bytes.len());
        record.extend_from_slice(&x.to_le_bytes());
        record.extend_from_slice(&z.to_le_bytes());
        record.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        record.extend_from_slice(&bytes);
        let Ok(end) = region.file.seek(SeekFrom::End(0)) else {
            return;
        };
        // One write, so a crash leaves at most a torn last record, which
        // the next open cuts off.
        if region.file.write_all(&record).is_ok() {
            region
                .index
                .insert((x, z), (end + RECORD, bytes.len() as u32));
        }
    }

    fn region<'a>(
        &self,
        regions: &'a mut FxHashMap<(i32, i32), Option<Region>>,
        x: i32,
        z: i32,
        create: bool,
    ) -> Option<&'a mut Region> {
        let at = (x >> 5, z >> 5);
        if create && matches!(regions.get(&at), Some(None)) {
            regions.remove(&at);
        }
        regions
            .entry(at)
            .or_insert_with(|| self.open(at, create))
            .as_mut()
    }

    fn open(&self, (rx, rz): (i32, i32), create: bool) -> Option<Region> {
        let path = self.dir.join(format!("r.{rx}.{rz}.hlod"));
        if !create && !path.exists() {
            return None;
        }
        if create {
            std::fs::create_dir_all(&self.dir).ok()?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(create)
            .truncate(false)
            .open(&path)
            .ok()?;
        let mut header = [0; HEADER as usize];
        let current = file.read_exact(&mut header).is_ok()
            && header[..4] == *MAGIC
            && header[4..8] == FORMAT.to_le_bytes()
            && header[8..] == self.key.to_le_bytes();
        let mut index = FxHashMap::default();
        if current {
            let size = file.metadata().ok()?.len();
            let mut at = HEADER;
            let mut record = [0; RECORD as usize];
            while at + RECORD <= size {
                file.seek(SeekFrom::Start(at)).ok()?;
                file.read_exact(&mut record).ok()?;
                let field = |i: usize| u32::from_le_bytes(record[i..i + 4].try_into().unwrap());
                let len = field(8);
                if len > MAX_RECORD || at + RECORD + u64::from(len) > size {
                    break;
                }
                index.insert((field(0) as i32, field(4) as i32), (at + RECORD, len));
                at += RECORD + u64::from(len);
            }
            if at < size {
                file.set_len(at).ok()?;
            }
        } else {
            file.set_len(0).ok()?;
            file.seek(SeekFrom::Start(0)).ok()?;
            let mut header = Vec::with_capacity(HEADER as usize);
            header.extend_from_slice(MAGIC);
            header.extend_from_slice(&FORMAT.to_le_bytes());
            header.extend_from_slice(&self.key.to_le_bytes());
            file.write_all(&header).ok()?;
        }
        Some(Region { file, index })
    }
}

/// What cached columns depend on: this build (its commit, or for a local
/// build the executable's own time, so a rebuilt generator starts afresh),
/// the block states and the biomes.
pub fn build_key(states: usize, biomes: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    FORMAT.hash(&mut hasher);
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    match option_env!("HORIZON_COMMIT") {
        Some(commit) => commit.hash(&mut hasher),
        None => std::env::current_exe()
            .and_then(|exe| exe.metadata()?.modified())
            .ok()
            .hash(&mut hasher),
    }
    states.hash(&mut hasher);
    biomes.hash(&mut hasher);
    hasher.finish()
}

fn encode(columns: &[Column; 256]) -> Vec<u8> {
    let mut out = Vec::new();
    for column in columns {
        match column.surface {
            None => out.push(0),
            Some(s) => {
                out.push(1 + u8::from(s.water.is_some()));
                out.extend_from_slice(&s.height.to_le_bytes());
                out.extend_from_slice(&s.state.0.to_le_bytes());
                out.extend_from_slice(&s.biome.to_le_bytes());
                if let Some((height, state)) = s.water {
                    out.extend_from_slice(&height.to_le_bytes());
                    out.extend_from_slice(&state.0.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&(column.spans.len() as u16).to_le_bytes());
        for span in &column.spans {
            out.extend_from_slice(&span.min.to_le_bytes());
            out.extend_from_slice(&span.max.to_le_bytes());
            out.extend_from_slice(&span.state.0.to_le_bytes());
            out.extend_from_slice(&span.biome.to_le_bytes());
            out.extend_from_slice(&span.light.to_le_bytes());
        }
    }
    let mut deflate =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to a Vec cannot fail.
    deflate.write_all(&out).unwrap();
    deflate.finish().unwrap()
}

fn decode(bytes: &[u8]) -> Option<[Column; 256]> {
    let mut raw = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .take(u64::from(MAX_RECORD) * 8)
        .read_to_end(&mut raw)
        .ok()?;
    let mut at = 0;
    let mut take = |n: usize| {
        let field = raw.get(at..at + n)?;
        at += n;
        Some(field)
    };
    let mut columns: [Column; 256] = std::array::from_fn(|_| Column::default());
    for column in &mut columns {
        let i32_ = |b: &[u8]| i32::from_le_bytes(b.try_into().unwrap());
        let u32_ = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap());
        let u16_ = |b: &[u8]| u16::from_le_bytes(b.try_into().unwrap());
        let i16_ = |b: &[u8]| i16::from_le_bytes(b.try_into().unwrap());
        let tag = take(1)?[0];
        if tag > 2 {
            return None;
        }
        if tag > 0 {
            let height = i32_(take(4)?);
            let state = BlockStateId(u32_(take(4)?));
            let biome = u16_(take(2)?);
            let water = if tag == 2 {
                Some((i32_(take(4)?), BlockStateId(u32_(take(4)?))))
            } else {
                None
            };
            column.surface = Some(Surface {
                height,
                state,
                water,
                biome,
            });
        }
        let count = u16_(take(2)?);
        column.spans.reserve(count.into());
        for _ in 0..count {
            let span = Span {
                min: i16_(take(2)?),
                max: i16_(take(2)?),
                state: BlockStateId(u32_(take(4)?)),
                biome: u16_(take(2)?),
                light: u16_(take(2)?),
            };
            if span.min >= span.max {
                return None;
            }
            column.spans.push(span);
        }
    }
    (at == raw.len()).then_some(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns(seed: u32) -> [Column; 256] {
        std::array::from_fn(|i| {
            let i = i as u32 + seed;
            Column {
                surface: (!i.is_multiple_of(3)).then(|| Surface {
                    height: 60 + (i % 7) as i32,
                    state: BlockStateId(i),
                    water: (i.is_multiple_of(2)).then_some((63, BlockStateId(9))),
                    biome: (i % 5) as u16,
                }),
                spans: (0..i % 4)
                    .map(|n| Span {
                        min: (n * 10) as i16 - 64,
                        max: (n * 10 + 4) as i16 - 64,
                        state: BlockStateId(n + i),
                        biome: 2,
                        light: 240,
                    })
                    .collect(),
            }
        })
    }

    #[test]
    fn chunks_survive_reopening_and_a_torn_last_record() {
        let dir = std::env::temp_dir().join(format!("horizon-lod-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DiskCache::new(dir.clone(), 7);
        assert!(cache.get(3, -40).is_none(), "nothing cached yet");
        cache.put(3, -40, &columns(0));
        cache.put(4, -40, &columns(1));
        let path = dir.join("r.0.-2.hlod");
        let size = std::fs::metadata(&path).unwrap().len();
        // A chunk cached already is not appended again.
        cache.put(3, -40, &columns(2));
        assert_eq!(*cache.get(3, -40).unwrap(), columns(0));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), size);
        drop(cache);

        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(size + 7).unwrap();
        drop(file);
        let cache = DiskCache::new(dir.clone(), 7);
        assert_eq!(*cache.get(3, -40).unwrap(), columns(0));
        assert_eq!(*cache.get(4, -40).unwrap(), columns(1));
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            size,
            "torn tail cut"
        );
        drop(cache);

        // Another build's files are cleared, not read.
        let cache = DiskCache::new(dir.clone(), 8);
        assert!(cache.get(3, -40).is_none());
        cache.put(5, -40, &columns(3));
        assert!(cache.get(4, -40).is_none());
        assert_eq!(*cache.get(5, -40).unwrap(), columns(3));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn damaged_records_read_as_missing() {
        let bytes = encode(&columns(0));
        assert_eq!(decode(&bytes).unwrap(), columns(0));
        assert!(decode(&bytes[..bytes.len() / 2]).is_none());
        assert!(decode(b"not deflate").is_none());
    }
}
