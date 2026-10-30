//! Read-only ZIP archives held in memory (resource packs, data packs and the
//! unifont glyph archive), on the `zip` crate. Like Java's `ZipFile` (what
//! vanilla's `FilePackResources` reads with), entries aren't CRC-checked.
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Cursor, Read},
    path::Path,
    sync::Arc,
};
use zip::{CompressionMethod, ZipArchive};

/// The most one entry inflates to: far above any real pack file, but a zip
/// bomb stops here.
const MAX_ENTRY: usize = 1 << 30;

/// One archive with its entries indexed by name (`\` read as `/`), files
/// and folders apart.
pub struct Zip {
    archive: ZipArchive<Cursor<Arc<[u8]>>>,
    entries: BTreeMap<String, usize>,
    /// Folder entries (`name/`) by their name without the slash.
    folders: BTreeMap<String, usize>,
    /// The archive's length: no size an entry claims can be bigger.
    len: usize,
}
fn invalid(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
impl Zip {
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }
    pub fn from_bytes(mut data: Vec<u8>) -> io::Result<Self> {
        normalize_end(&mut data);
        let len = data.len();
        let archive = ZipArchive::new(Cursor::new(Arc::from(data))).map_err(invalid)?;
        let (mut entries, mut folders) = (BTreeMap::new(), BTreeMap::new());
        for i in 0..archive.len() {
            let Some(name) = archive.name_for_index(i) else {
                continue;
            };
            let name = name.replace('\\', "/");
            match name.strip_suffix('/') {
                Some(folder) => folders.insert(folder.to_owned(), i),
                None => entries.insert(name, i),
            };
        }
        Ok(Self {
            archive,
            entries,
            folders,
            len,
        })
    }
    /// The files, as `FilePackResources.listResources` lists them (folder
    /// entries skipped).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
    /// `FilePackResources.getResource` through `ZipFile.getEntry`: the file
    /// `name`, else the folder entry `name/`, whose bytes Java reads like a
    /// file's (Loka's pack stores its `pack.mcmeta` and `pack.png` that way).
    fn index(&self, name: &str) -> Option<usize> {
        self.entries
            .get(name)
            .or_else(|| self.folders.get(name))
            .copied()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.index(name).is_some()
    }
    pub fn read(&self, name: &str) -> io::Result<Vec<u8>> {
        let index = self
            .index(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?;
        // A clone shares the bytes and the central directory: reading needs `&mut`.
        let mut archive = self.archive.clone();
        let mut entry = archive.by_index_raw(index).map_err(invalid)?;
        let method = entry.compression();
        let mut raw = Vec::with_capacity((entry.compressed_size() as usize).min(self.len));
        entry.read_to_end(&mut raw)?;
        match method {
            CompressionMethod::Stored => Ok(raw),
            CompressionMethod::Deflated => {
                // Java's ZipFile trusts neither size an entry declares: it
                // inflates the stream to its end and keeps what comes out.
                // Packs in the wild (a Nexo-generated one served through
                // Dropbox, for one) carry central-directory sizes that point
                // at other files, and miniz_oxide, which refuses input past
                // the stream's end, failed whole entries with "corrupt
                // deflate stream" and with them the whole pack. Only the
                // size limit stands between the archive and a zip bomb.
                let decoder = flate2::read::DeflateDecoder::new(raw.as_slice());
                let mut decoded = Vec::with_capacity(raw.len().min(MAX_ENTRY));
                decoder
                    .take(MAX_ENTRY as u64 + 1)
                    .read_to_end(&mut decoded)
                    .map_err(|_| invalid("corrupt deflate stream"))?;
                if decoded.len() > MAX_ENTRY {
                    return Err(invalid("entry inflates past the size limit"));
                }
                Ok(decoded)
            }
            _ => Err(invalid("unsupported zip compression")),
        }
    }
}

/// The 22 fixed bytes of the end-of-central-directory record.
const END_HEADER: usize = 22;

/// Java's `ZipFile` ignores the END record's disk numbers and file counts,
/// reading the central directory from `cenpos` to `cenpos + cenlen` whatever
/// they say; the `zip` crate insists the record agrees with the directory.
/// Packs written while streaming (Nexo serves them for gommehd.net, for one)
/// leave every file's local header a stub and the END counts and disk number
/// at 0 and 0xFFFF: archives Java opens but the crate refuses. Count the real
/// directory entries and normalize the record so it does not look multi-disk
/// or empty.
fn normalize_end(data: &mut [u8]) {
    if data.len() < END_HEADER {
        return;
    }
    let u16_at = |data: &[u8], at: usize| u16::from_le_bytes(data[at..at + 2].try_into().unwrap());
    let u32_at = |data: &[u8], at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    // The END record's comment can hide the signature: scan back through the
    // last 64 KiB, like every zip reader.
    let start = data.len().saturating_sub(END_HEADER + u16::MAX as usize);
    let mut end = None;
    for pos in (start..=data.len().saturating_sub(END_HEADER)).rev() {
        if data[pos..pos + 4] == [0x50, 0x4b, 0x05, 0x06]
            && pos + END_HEADER + u16_at(data, pos + 20) as usize <= data.len()
        {
            end = Some(pos);
            break;
        }
    }
    let Some(end) = end else { return };
    // The directory's real position and length, as Java derives them; the
    // recorded offset is relative to the archive's start.
    let cenlen = u32_at(data, end + 12) as usize;
    let cenpos = end.saturating_sub(cenlen);
    let mut count = 0u16;
    let mut p = cenpos;
    while p + 46 <= end && data[p..p + 4] == [0x50, 0x4b, 0x01, 0x02] {
        count = count.saturating_add(1);
        p += 46
            + u16_at(data, p + 28) as usize
            + u16_at(data, p + 30) as usize
            + u16_at(data, p + 32) as usize;
    }
    let disk = u16_at(data, end + 4);
    let with_directory = u16_at(data, end + 6);
    if disk == with_directory && u16_at(data, end + 8) == count && u16_at(data, end + 10) == count {
        return;
    }
    data[end + 4..end + 10].fill(0);
    data[end + 8..end + 10].copy_from_slice(&count.to_le_bytes());
    data[end + 10..end + 12].copy_from_slice(&count.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A hand-built archive with one stored and one deflated entry.
    fn archive() -> Vec<u8> {
        archive_of(&[
            ("pack.mcmeta", b"{\"pack\":{}}", false),
            ("a/b.txt", b"hello hello hello hello", true),
        ])
    }
    /// A hand-built archive of `(name, body, deflated)` entries.
    fn archive_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for &(name, body, deflate) in files {
            let data = if deflate {
                miniz_oxide::deflate::compress_to_vec(body, 6)
            } else {
                body.to_vec()
            };
            let offset = out.len() as u32;
            let local = |v: &mut Vec<u8>, sig: u32| {
                v.extend(sig.to_le_bytes());
            };
            local(&mut out, 0x0403_4b50);
            out.extend([20, 0, 0, 0]);
            out.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            out.extend([0; 8]);
            out.extend((data.len() as u32).to_le_bytes());
            out.extend((body.len() as u32).to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(&data);
            local(&mut central, 0x0201_4b50);
            central.extend([20, 0, 20, 0, 0, 0]);
            central.extend((if deflate { 8u16 } else { 0 }).to_le_bytes());
            central.extend([0; 8]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((body.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        out
    }
    #[test]
    fn reads_stored_and_deflated_entries() {
        // Its CRCs are zero: Java's ZipFile doesn't check them either.
        let zip = Zip::from_bytes(archive()).unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a/b.txt", "pack.mcmeta"]);
        assert_eq!(zip.read("pack.mcmeta").unwrap(), b"{\"pack\":{}}");
        assert_eq!(zip.read("a/b.txt").unwrap(), b"hello hello hello hello");
        assert!(zip.read("missing").is_err());
    }

    /// The offset of the central-directory header of `name`.
    fn central_header(bytes: &[u8], name: &str) -> usize {
        let mut at = 0;
        while let Some(i) = bytes[at..]
            .windows(4)
            .position(|s| s == [0x50, 0x4b, 0x01, 0x02])
        {
            let p = at + i;
            let nlen = u16::from_le_bytes(bytes[p + 28..p + 30].try_into().unwrap()) as usize;
            if &bytes[p + 46..p + 46 + nlen] == name.as_bytes() {
                return p;
            }
            at = p + 4;
        }
        panic!("no central header for {name}");
    }

    /// Java's `ZipFile` inflates the stream and keeps what comes out, so a
    /// central directory that names a size other than the stream's (the
    /// Nexo pack Dropbox serves for one server does) changes nothing.
    #[test]
    fn reads_a_deflate_stream_larger_than_its_declared_size() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "large.txt",
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(&vec![b'x'; 4096]).unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let central = central_header(&bytes, "large.txt");
        bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());

        let zip = Zip::from_bytes(bytes).unwrap();
        assert_eq!(zip.read("large.txt").unwrap(), vec![b'x'; 4096]);
    }

    /// Some pack writers leave an entry's compressed size longer than its
    /// deflate stream, so the bytes past the stream's end are part of it.
    /// Java inflates to the stream's end and ignores them; miniz_oxide
    /// refused the input and failed the entry, and with it the whole pack.
    #[test]
    fn reads_past_trailing_bytes_after_the_deflate_stream() {
        let mut bytes = written();
        let name = "Pack-main/assets/minecraft/lang/en_us.json";
        let central = central_header(&bytes, name);
        let local =
            u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap()) as usize;
        for at in [central + 20, local + 18] {
            let csize = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            bytes[at..at + 4].copy_from_slice(&(csize + 8).to_le_bytes());
        }

        let zip = Zip::from_bytes(bytes).unwrap();
        assert_eq!(
            zip.read(name).unwrap(),
            "{\"a\":\"b\"}".repeat(100).as_bytes()
        );
    }

    /// Loka's server pack (#3413) names its metadata `pack.mcmeta/`.
    /// `ZipFile.getEntry("pack.mcmeta")` falls back to that folder entry and
    /// vanilla reads its bytes; a file of the exact name still comes first.
    #[test]
    fn a_name_falls_back_to_its_folder_entry() {
        let zip = Zip::from_bytes(archive_of(&[
            ("pack.mcmeta/", b"{\"pack\":{\"pack_format\":75}}", true),
            ("pack.png/", b"png", false),
            ("a/", b"folder", false),
            ("a", b"file", false),
        ]))
        .unwrap();
        assert_eq!(zip.names().collect::<Vec<_>>(), ["a"]);
        assert!(zip.contains("pack.mcmeta"));
        assert_eq!(
            zip.read("pack.mcmeta").unwrap(),
            b"{\"pack\":{\"pack_format\":75}}"
        );
        assert_eq!(zip.read("pack.png").unwrap(), b"png");
        assert_eq!(zip.read("a").unwrap(), b"file");
        assert!(!zip.contains("pack"));
    }

    /// A pack as zip tools write it: folder entries, a nested pack root,
    /// stored and deflated files, and a Windows-style name.
    fn written() -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        w.add_directory("Pack-main/", stored).unwrap();
        w.add_directory("Pack-main/assets/minecraft/", stored)
            .unwrap();
        w.start_file("Pack-main/pack.mcmeta", stored).unwrap();
        w.write_all(b"{\"pack\":{\"pack_format\":64}}").unwrap();
        w.start_file("Pack-main/assets/minecraft/lang/en_us.json", deflated)
            .unwrap();
        w.write_all("{\"a\":\"b\"}".repeat(100).as_bytes()).unwrap();
        w.start_file(
            "Pack-main\\assets\\minecraft\\texts\\splashes.txt",
            deflated,
        )
        .unwrap();
        w.write_all(b"Also try Horizon!").unwrap();
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_nested_folders_and_skips_folder_entries() {
        let zip = Zip::from_bytes(written()).unwrap();
        assert_eq!(
            zip.names().collect::<Vec<_>>(),
            [
                "Pack-main/assets/minecraft/lang/en_us.json",
                "Pack-main/assets/minecraft/texts/splashes.txt",
                "Pack-main/pack.mcmeta",
            ]
        );
        assert!(zip.contains("Pack-main/pack.mcmeta"));
        assert!(!zip.contains("Pack-main/"));
        assert_eq!(
            zip.read("Pack-main/assets/minecraft/lang/en_us.json")
                .unwrap(),
            "{\"a\":\"b\"}".repeat(100).as_bytes()
        );
        assert_eq!(
            zip.read("Pack-main/assets/minecraft/texts/splashes.txt")
                .unwrap(),
            b"Also try Horizon!"
        );
    }

    #[test]
    fn a_bad_entry_fails_alone() {
        let mut bytes = written();
        // The deflated lang file's stream becomes a reserved block type.
        let (start, len) = {
            let mut archive = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
            let index = archive
                .index_for_name("Pack-main/assets/minecraft/lang/en_us.json")
                .unwrap();
            let entry = archive.by_index_raw(index).unwrap();
            (
                entry.data_start() as usize,
                entry.compressed_size() as usize,
            )
        };
        bytes[start..start + len].fill(0xff);
        let zip = Zip::from_bytes(bytes).unwrap();
        assert!(
            zip.read("Pack-main/assets/minecraft/lang/en_us.json")
                .is_err()
        );
        assert_eq!(
            zip.read("Pack-main/pack.mcmeta").unwrap(),
            b"{\"pack\":{\"pack_format\":64}}"
        );
    }

    #[test]
    fn refuses_what_is_not_a_zip() {
        assert!(Zip::from_bytes(b"PK not really".to_vec()).is_err());
        assert!(Zip::from_bytes(Vec::new()).is_err());
        let mut cut = written();
        cut.truncate(cut.len() / 2);
        assert!(Zip::from_bytes(cut).is_err());
    }

    /// A pack as a streaming writer serves it (Nexo/gommehd.net): every
    /// local header is a 30-byte stub and the END record's disk number and
    /// counts are wrong. Java 25's `ZipFile` opens it, with each size either
    /// in the directory or as 0xFFFFFFFF plus its Zip64 extra field (without
    /// that field it refuses the archive: "invalid zip64 extra len size").
    #[test]
    fn reads_a_streamed_archive() {
        for zip64 in [false, true] {
            let files: [(&str, &[u8]); 2] = [
                ("pack.mcmeta", b"{\"pack\":{}}"),
                ("a/b.txt", b"streamed entry"),
            ];
            let mut out = Vec::new();
            let mut central = Vec::new();
            for (name, body) in files {
                let data = miniz_oxide::deflate::compress_to_vec(body, 6);
                let offset = out.len() as u32;
                out.extend(0x0403_4b50u32.to_le_bytes());
                out.extend([0; 26]);
                out.extend(&data);
                let extra: Vec<u8> = if zip64 {
                    [1u16, 8]
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .chain((body.len() as u64).to_le_bytes())
                        .collect()
                } else {
                    Vec::new()
                };
                central.extend(0x0201_4b50u32.to_le_bytes());
                central.extend([20, 0, 20, 0, 0, 0]);
                central.extend(8u16.to_le_bytes());
                central.extend([0; 8]);
                central.extend((data.len() as u32).to_le_bytes());
                central.extend(if zip64 { u32::MAX } else { body.len() as u32 }.to_le_bytes());
                central.extend((name.len() as u16).to_le_bytes());
                central.extend((extra.len() as u16).to_le_bytes());
                central.extend([0; 10]);
                central.extend(offset.to_le_bytes());
                central.extend(name.as_bytes());
                central.extend(extra);
            }
            let start = out.len() as u32;
            out.extend(&central);
            out.extend(0x0605_4b50u32.to_le_bytes());
            out.extend(u16::MAX.to_le_bytes());
            out.extend([0; 6]);
            out.extend((central.len() as u32).to_le_bytes());
            out.extend(start.to_le_bytes());
            out.extend([0; 2]);
            let zip = Zip::from_bytes(out).unwrap();
            assert_eq!(zip.names().collect::<Vec<_>>(), ["a/b.txt", "pack.mcmeta"]);
            assert_eq!(zip.read("pack.mcmeta").unwrap(), b"{\"pack\":{}}");
            assert_eq!(zip.read("a/b.txt").unwrap(), b"streamed entry");
        }
    }

    /// A streamed archive whose zip64 extra lies: both sizes claimed are
    /// u64::MAX / 2. `Zip::from_bytes` may refuse it; if not, `read` errs or reads
    /// instead of dying on a capacity that no archive could hold.
    #[test]
    fn declared_sizes_past_the_archive_do_not_abort() {
        let name = "a/b.txt";
        let body = b"streamed entry";
        let data = miniz_oxide::deflate::compress_to_vec(body, 6);
        let mut out = Vec::new();
        let mut central = Vec::new();
        let offset = out.len() as u32;
        out.extend(0x0403_4b50u32.to_le_bytes());
        out.extend([0; 26]);
        out.extend(&data);
        // Both directory sizes 0xFFFFFFFF so the zip64 extra's uncompressed,
        // then compressed, sizes are the ones read.
        let extra: Vec<u8> = [1u16, 16]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .chain((u64::MAX / 2).to_le_bytes())
            .chain((u64::MAX / 2).to_le_bytes())
            .collect();
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend([20, 0, 20, 0, 0, 0]);
        central.extend(8u16.to_le_bytes());
        central.extend([0; 8]);
        central.extend(u32::MAX.to_le_bytes());
        central.extend(u32::MAX.to_le_bytes());
        central.extend((name.len() as u16).to_le_bytes());
        central.extend((extra.len() as u16).to_le_bytes());
        central.extend([0; 10]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
        central.extend(extra);
        let start = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend(u16::MAX.to_le_bytes());
        out.extend([0; 6]);
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0; 2]);
        if let Ok(zip) = Zip::from_bytes(out) {
            let _ = zip.read(name);
        }
    }
}
