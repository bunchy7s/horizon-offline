use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::{
    DATA_VERSION,
    chunk::{BlockState, ChunkData, PalettedContainer},
    compression::Compression,
    nbt::{JavaString, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile, RegionWriter},
};
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "horizon-world-test-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn region(&self) -> PathBuf {
        self.0.join("r.-1.0.mca")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(len: usize) -> ChunkRecord {
    ChunkRecord {
        compression: Compression::None,
        compressed: vec![37; len],
        timestamp: 0x8000_0001,
    }
}

#[test]
fn external_threshold_negative_coordinates_and_timestamps_roundtrip() {
    let dir = Directory::new();
    let a = ChunkPos::new(-32, 0);
    let b = ChunkPos::new(-1, 31);
    let inline = record(255 * 4096 - 5);
    let external = record(255 * 4096 - 4);
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer.write(a, &inline).unwrap();
    writer.write(b, &external).unwrap();
    writer.finish().unwrap();
    assert!(!dir.0.join("c.-32.0.mcc").exists());
    assert!(dir.0.join("c.-1.31.mcc").exists());
    let mut reader = RegionFile::open(dir.region()).unwrap();
    assert_eq!(
        reader.chunks().collect::<Vec<_>>(),
        [(a, inline.timestamp), (b, external.timestamp)]
    );
    for (pos, expected) in [(a, inline), (b, external)] {
        let actual = reader.read(pos).unwrap().unwrap();
        assert_eq!(actual.compressed, expected.compressed);
        assert_eq!(actual.timestamp, expected.timestamp);
    }
    assert!(reader.read(ChunkPos::new(-31, 0)).unwrap().is_none());
    assert!(reader.read(ChunkPos::new(0, 0)).is_err());
}

#[test]
fn in_place_writes_reuse_freed_sectors_and_keep_other_chunks() {
    let dir = Directory::new();
    let (a, b, c) = (
        ChunkPos::new(-32, 0),
        ChunkPos::new(-31, 0),
        ChunkPos::new(-1, 31),
    );
    let mut region = RegionFile::open_writable(dir.region(), true).unwrap();
    // RegionBitmap: a at sectors 2..4, b at 4.
    region.write(a, &record(2 * 4096 - 5)).unwrap();
    region.write(b, &record(100)).unwrap();
    let sectors = |path: &Path| fs::metadata(path).unwrap().len() / 4096;
    assert_eq!(sectors(&dir.region()), 5);
    // a grows: it moves past b, and its two old sectors are free again.
    region.write(a, &record(3 * 4096 - 5)).unwrap();
    assert_eq!(sectors(&dir.region()), 8);
    // c fits in a's old sectors; then an oversized c goes to its .mcc file
    // (a one-sector stub, at the end: sector 8) and back inline (sector 2).
    region.write(c, &record(2 * 4096 - 5)).unwrap();
    assert_eq!(sectors(&dir.region()), 8);
    region.write(c, &record(256 * 4096)).unwrap();
    assert!(dir.0.join("c.-1.31.mcc").exists());
    region.write(c, &record(10)).unwrap();
    assert!(!dir.0.join("c.-1.31.mcc").exists());
    assert!(region.clear(b, 7).unwrap());
    assert!(!region.clear(b, 7).unwrap());
    region.sync().unwrap();
    drop(region);
    let mut reader = RegionFile::open(dir.region()).unwrap();
    assert_eq!(reader.chunks().map(|(p, _)| p).collect::<Vec<_>>(), [a, c]);
    assert_eq!(
        reader.read(a).unwrap().unwrap().compressed,
        record(3 * 4096 - 5).compressed
    );
    assert_eq!(
        reader.read(c).unwrap().unwrap().compressed,
        record(10).compressed
    );
    assert!(reader.read(b).unwrap().is_none());
    // Reopened, the bitmap holds the chunks' sectors: new chunks take the
    // first free runs (sectors 3 and 4), not theirs.
    let mut region = RegionFile::open_writable(dir.region(), false).unwrap();
    region.write(b, &record(4096 - 5)).unwrap();
    region
        .write(ChunkPos::new(-30, 0), &record(4096 - 5))
        .unwrap();
    drop(region);
    let mut reader = RegionFile::open(dir.region()).unwrap();
    assert_eq!(
        reader.read(a).unwrap().unwrap().compressed,
        record(3 * 4096 - 5).compressed
    );
    assert_eq!(
        reader.read(c).unwrap().unwrap().compressed,
        record(10).compressed
    );
    assert_eq!(
        reader.read(b).unwrap().unwrap().compressed,
        record(4096 - 5).compressed
    );
    assert_eq!(sectors(&dir.region()), 9);
}

#[test]
fn failed_writer_cannot_commit_and_removes_its_own_external_files() {
    let dir = Directory::new();
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer
        .write(ChunkPos::new(-1, 0), &record(1_100_000))
        .unwrap();
    assert!(dir.0.join("c.-1.0.mcc").exists());
    assert!(writer.write(ChunkPos::new(0, 0), &record(4)).is_err());
    assert!(writer.finish().is_err());
    assert!(!dir.region().exists());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
}

#[test]
fn existing_external_and_concurrent_destination_are_preserved() {
    let dir = Directory::new();
    let external = dir.0.join("c.-1.0.mcc");
    fs::write(&external, b"owned by someone else").unwrap();
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    assert!(
        writer
            .write(ChunkPos::new(-1, 0), &record(1_100_000))
            .is_err()
    );
    drop(writer);
    assert_eq!(fs::read(&external).unwrap(), b"owned by someone else");
    let mut writer = RegionWriter::create(dir.region()).unwrap();
    writer.write(ChunkPos::new(-32, 0), &record(3)).unwrap();
    fs::write(dir.region(), b"another writer won").unwrap();
    assert!(writer.finish().is_err());
    assert_eq!(fs::read(dir.region()).unwrap(), b"another writer won");
    assert!(RegionWriter::create(dir.region()).is_err());
}

fn malformed_region(path: &Path, first: u32, second: u32, payload_len: u32, codec: u8) {
    let mut file = fs::File::create(path).unwrap();
    file.set_len(12288).unwrap();
    file.write_all(&first.to_be_bytes()).unwrap();
    file.write_all(&second.to_be_bytes()).unwrap();
    file.seek(SeekFrom::Start(8192)).unwrap();
    file.write_all(&payload_len.to_be_bytes()).unwrap();
    file.write_all(&[codec]).unwrap();
}

#[test]
fn corrupt_allocations_are_dropped_and_bad_payloads_fail() {
    let dir = Directory::new();
    // RegionFile's constructor drops (and logs) entries whose sector overlaps the
    // header, that have no sectors or that start past the end of the file; it
    // does not check overlaps between chunks.
    for (first, second, chunks) in [(257, 0, 0), (512, 0, 0), (1025, 0, 0), (513, 513, 2)] {
        malformed_region(&dir.region(), first, second, 1, 3);
        let reader = RegionFile::open(dir.region()).unwrap();
        assert_eq!(reader.chunks().count(), chunks);
    }
    // An empty or truncated file reads as a region without chunks.
    fs::write(dir.region(), b"").unwrap();
    assert_eq!(RegionFile::open(dir.region()).unwrap().chunks().count(), 0);
    fs::write(dir.region(), [0, 0, 2, 1]).unwrap();
    assert_eq!(RegionFile::open(dir.region()).unwrap().chunks().count(), 0);
    for (len, codec) in [
        (0, 3),
        (4093, 3),
        (u32::MAX, 3),
        (1, 127),
        (2, 131),
        (1, 131),
    ] {
        malformed_region(&dir.region(), 513, 0, len, codec);
        let mut reader = RegionFile::open(dir.region()).unwrap();
        assert!(reader.read(ChunkPos::new(-32, 0)).is_err());
    }
}

#[test]
fn chunk_storage_edit_preserves_unknown_fields_and_other_blocks() {
    let palette =
        PalettedContainer::uniform(Tag::String(JavaString::from("minecraft:air")), 4096, 4)
            .unwrap();
    let root = NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(-1)),
            ("zPos", Tag::Int(-1)),
            ("extra", Tag::ByteArray(vec![1, 2, 3])),
            (
                "sections",
                Tag::List(vec![Tag::compound([
                    ("Y", Tag::Byte(-4)),
                    ("block_states", palette.to_nbt()),
                    ("unknown", Tag::Long(99)),
                ])]),
            ),
        ]),
    };
    let mut chunk = ChunkData::new(root).unwrap();
    let p = BlockPos::new(-1, -64, -1);
    let state = BlockState {
        name: "minecraft:oak_log".to_owned(),
        properties: [("axis".to_owned(), "x".to_owned())].into(),
    };
    assert_eq!(
        chunk.set_block_data(p, state.clone()).unwrap().name,
        "minecraft:air"
    );
    assert_eq!(chunk.block(p).unwrap(), Some(state));
    assert_eq!(
        chunk.block(p.offset(-1, 0, 0)).unwrap().unwrap().name,
        "minecraft:air"
    );
    assert!(
        chunk
            .set_block_data(
                p.offset(1, 0, 0),
                BlockState {
                    name: "minecraft:stone".into(),
                    properties: Default::default()
                }
            )
            .is_err()
    );
    let root = chunk.into_nbt();
    assert_eq!(root.tag.get("extra"), Some(&Tag::ByteArray(vec![1, 2, 3])));
    let Some(Tag::List(sections)) = root.tag.get("sections") else {
        panic!()
    };
    assert_eq!(sections[0].get("unknown"), Some(&Tag::Long(99)));
    let record = ChunkRecord::from_nbt(&root, Compression::Lz4, 17, Limits::default()).unwrap();
    assert_eq!(record.nbt(Limits::default()).unwrap(), root);
}

/// Saving one chunk into a full region: the whole file rewritten (every
/// chunk read and written again, then the new file published), against
/// vanilla's in-place write. Run with --ignored --nocapture.
#[test]
#[ignore = "microbenchmark; run with --ignored --nocapture"]
fn benchmark_saving_into_a_full_region() {
    let dir = Directory::new();
    let chunks: Vec<ChunkPos> = (0..1024)
        .map(|i| ChunkPos::new(-32 + i % 32, i / 32))
        .collect();
    // About what a compressed overworld chunk weighs: two sectors.
    let saved = record(6000);
    let mut region = RegionFile::open_writable(dir.region(), true).unwrap();
    for &c in &chunks {
        region.write(c, &saved).unwrap();
    }
    region.sync().unwrap();
    drop(region);
    let rounds = 100;
    let start = std::time::Instant::now();
    for round in 0..rounds {
        let changed = chunks[round * 7 % 1024];
        let mut existing = RegionFile::open(dir.region()).unwrap();
        let mut records = Vec::new();
        for (p, _) in existing.chunks().collect::<Vec<_>>() {
            let r = if p == changed {
                saved.clone()
            } else {
                existing.read(p).unwrap().unwrap()
            };
            records.push((p, r));
        }
        let staging = dir.0.join("staging");
        fs::create_dir_all(&staging).unwrap();
        let temporary = staging.join("r.-1.0.mca");
        let _ = fs::remove_file(&temporary);
        let mut writer = RegionWriter::create(&temporary).unwrap();
        for (p, r) in &records {
            writer.write(*p, r).unwrap();
        }
        writer.finish().unwrap();
        fs::rename(&temporary, dir.region()).unwrap();
    }
    let rewrite = start.elapsed() / rounds as u32;
    let mut region = RegionFile::open_writable(dir.region(), false).unwrap();
    let start = std::time::Instant::now();
    for round in 0..rounds {
        region.write(chunks[round * 7 % 1024], &saved).unwrap();
        region.sync().unwrap();
    }
    let in_place = start.elapsed() / rounds as u32;
    println!("one chunk into a full region: rewrite {rewrite:?}, in place {in_place:?}");
}

/// `RegionFileStorage.read` reads the root straight off the decompressing
/// stream (#5064): bytes after it and a damaged stream past it are never read,
/// so the chunk loads. A root cut short still fails.
#[test]
fn a_chunk_loads_whatever_follows_its_root() {
    let root = NamedTag {
        name: JavaString::default(),
        tag: Tag::compound([("x", Tag::Int(7))]),
    };
    let bytes = horizon_world::nbt::write_named(&root, Limits::saved()).unwrap();
    let record = |compression: Compression, compressed: Vec<u8>| ChunkRecord {
        compression,
        compressed,
        timestamp: 0,
    };
    let with = |tail: &[u8]| [bytes.as_slice(), tail].concat();
    let filler: Vec<u8> = (0..70_000).map(|i| (i * 31) as u8).collect();
    let mut zlib_tail = Compression::Zlib.encode(&with(&filler)).unwrap();
    let end = zlib_tail.len();
    zlib_tail[end - 4..].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    let mut lz4_tail = Compression::Lz4.encode(&with(&filler)).unwrap();
    // The second block's checksum: the first block holds the root.
    let second = 21 + u32::from_le_bytes(lz4_tail[9..13].try_into().unwrap()) as usize;
    lz4_tail[second + 17] ^= 0xff;
    for (name, chunk) in [
        (
            "uncompressed, a byte after",
            record(Compression::None, with(&[0xff])),
        ),
        (
            "zlib, junk after",
            record(
                Compression::Zlib,
                Compression::Zlib.encode(&with(b"junk")).unwrap(),
            ),
        ),
        (
            "gzip, junk after",
            record(
                Compression::Gzip,
                Compression::Gzip.encode(&with(b"junk")).unwrap(),
            ),
        ),
        (
            "zlib, damaged checksum",
            record(Compression::Zlib, zlib_tail),
        ),
        (
            "lz4, damaged second block",
            record(Compression::Lz4, lz4_tail),
        ),
    ] {
        assert_eq!(chunk.nbt(Limits::saved()).unwrap(), root, "{name}");
    }
    let cut = Compression::Zlib.encode(&bytes[..bytes.len() - 2]).unwrap();
    assert!(record(Compression::Zlib, cut).nbt(Limits::saved()).is_err());
}

/// `RegionFile.write` rewrites both header sectors from memory
/// (`writeHeader`), so entries the constructor dropped as invalid (#5064:
/// overlapping the header, empty, past the end) are zero on disk afterwards;
/// their timestamps stay. Vanilla 26.3 leaves this file with these bytes.
#[test]
fn a_chunk_write_rewrites_the_whole_region_header() {
    let dir = Directory::new();
    let mut header = vec![0u8; 8192];
    header[20..24].copy_from_slice(&[0, 0, 1, 1]); // slot 5: sector 1
    header[200..204].copy_from_slice(&[0, 0, 5, 0]); // slot 50: no sectors
    header[400..404].copy_from_slice(&[0, 0, 0x64, 1]); // slot 100: past the end
    header[4096 + 20..4096 + 24].copy_from_slice(&12345u32.to_be_bytes());
    fs::write(dir.region(), &header).unwrap();
    let root = NamedTag {
        name: JavaString::default(),
        tag: Tag::compound([("x", Tag::Int(7))]),
    };
    let record = ChunkRecord::from_nbt(&root, Compression::Zlib, 1, Limits::saved()).unwrap();
    let mut region = RegionFile::open_writable(dir.region(), false).unwrap();
    region.write(ChunkPos::new(-32, 0), &record).unwrap();
    drop(region);
    let bytes = fs::read(dir.region()).unwrap();
    assert_eq!(bytes[0..4], [0, 0, 2, 1]);
    for slot in [5, 50, 100] {
        assert_eq!(bytes[slot * 4..slot * 4 + 4], [0; 4], "slot {slot}");
    }
    assert_eq!(bytes[4096 + 20..4096 + 24], 12345u32.to_be_bytes());
}
