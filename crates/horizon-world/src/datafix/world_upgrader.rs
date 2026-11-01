//! `net.minecraft.util.worldupdate.WorldUpgrader` and `RegionStorageUpgrader`:
//! Optimize World. A worker thread reads every chunk of each dimension's
//! `entities`, `poi` and `region` folders, brings it to the current DataVersion
//! and writes it back, reporting its counts through an [`UpgradeProgress`] the
//! screen reads. Unlike [`super::files::upgrade_world`] (the converter that
//! runs when an older world opens) it works on a world that is already at the
//! current layout and decides per chunk whether anything changed.
use super::dynamic::{as_int, has, remove};
use super::files::{generators, subdirectories};
use super::progress::{UpgradeDataFixType, UpgradeProgress, UpgradeStatus, UpgradeType};
use super::schema::Ty;
use super::{CURRENT, update_chunk_to, update_region_tag_to};
use crate::compression::Compression;
use crate::nbt::{Limits, Tag};
use crate::region::{ChunkRecord, RegionFile};
use horizon_core::position::ChunkPos;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

const NEW_DIRECTORY_PREFIX: &str = "new_";

/// `RegionStorageUpgrader.TagModifier`: may change the upgraded tag and
/// reports whether it did.
pub type TagModifier = Box<dyn Fn(ChunkPos, &mut Tag) -> bool + Send + Sync>;

pub const OVERWORLD: &str = "minecraft:overworld";
pub const NETHER: &str = "minecraft:the_nether";
pub const END: &str = "minecraft:the_end";

/// `LevelStorageAccess.getDimensionPath`: `dimensions/<namespace>/<path>`.
pub fn dimension_path(base: &Path, dimension: &str) -> PathBuf {
    let (namespace, path) = dimension
        .split_once(':')
        .unwrap_or(("minecraft", dimension));
    base.join("dimensions").join(namespace).join(path)
}

/// The dimensions of a world: the three of vanilla, then any other folder
/// below `dimensions/` (a datapack's, standing in for the `LEVEL_STEM` registry).
pub fn world_dimensions(base: &Path) -> Vec<String> {
    let mut out: Vec<String> = [OVERWORLD, NETHER, END].map(str::to_owned).into();
    for namespace in subdirectories(&base.join("dimensions")) {
        for dimension in subdirectories(&namespace) {
            let id = format!(
                "{}:{}",
                namespace.file_name().unwrap_or_default().to_string_lossy(),
                dimension.file_name().unwrap_or_default().to_string_lossy()
            );
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// `WorldUpgrader.verifyChunkPos`: warns about a chunk stored under another
/// position; never reports a change.
pub fn verify_chunk_pos(pos: ChunkPos, upgraded: &mut Tag) -> bool {
    let stored = ChunkPos::new(
        as_int(upgraded.get("xPos"), 0),
        as_int(upgraded.get("zPos"), 0),
    );
    if stored != pos {
        eprintln!("Chunk {pos:?} has invalid position {stored:?}");
    }
    false
}

/// `WorldUpgrader.verifyChunkPosAndEraseCache`: also drops the heightmaps,
/// the light flag and the stored light of every section.
pub fn verify_chunk_pos_and_erase_cache(pos: ChunkPos, upgraded: &mut Tag) -> bool {
    verify_chunk_pos(pos, upgraded);
    let mut changed = has(upgraded, "Heightmaps");
    remove(upgraded, "Heightmaps");
    changed = changed || has(upgraded, "isLightOn");
    remove(upgraded, "isLightOn");
    if let Some(Tag::List(sections)) = upgraded.get_mut("sections") {
        for section in sections {
            if matches!(section, Tag::Compound(_)) {
                changed = changed || has(section, "BlockLight");
                remove(section, "BlockLight");
                changed = changed || has(section, "SkyLight");
                remove(section, "SkyLight");
            }
        }
    }
    changed
}

/// `UpgradeStatusTranslator.translate`: the translation key of the status line.
pub fn status_key(progress: &UpgradeProgress) -> String {
    match progress.status() {
        UpgradeStatus::Failed => return "optimizeWorld.stage.failed".into(),
        UpgradeStatus::Counting => return "optimizeWorld.stage.counting".into(),
        _ => {}
    }
    let Some(ty) = progress.data_fix_type() else {
        return "optimizeWorld.stage.counting".into();
    };
    let kind = match ty {
        UpgradeDataFixType::Chunk => "chunks",
        UpgradeDataFixType::EntityChunk => "entities",
        UpgradeDataFixType::PoiChunk => "poi",
    };
    match progress.status() {
        UpgradeStatus::Upgrading => format!("optimizeWorld.stage.upgrading.{kind}"),
        _ => format!("optimizeWorld.stage.finished.{kind}"),
    }
}

/// `FileToUpgrade`: a region file and the chunks it holds.
struct FileToUpgrade {
    path: PathBuf,
    chunks: Vec<ChunkPos>,
}

/// `RegionStorageUpgrader.Builder`, as far as `WorldUpgrader` uses it.
#[derive(Clone)]
struct Builder {
    data_fix_type: UpgradeDataFixType,
    folder: &'static str,
    default_version: i32,
    recreate_region_files: bool,
    /// The `__context` of a chunk: the dimension and generator type.
    context: Option<(String, Option<String>)>,
    tag_modifiers: BTreeMap<i32, Arc<TagModifier>>,
}

impl Builder {
    fn new(data_fix_type: UpgradeDataFixType, folder: &'static str, recreate: bool) -> Self {
        Self {
            data_fix_type,
            folder,
            default_version: if data_fix_type == UpgradeDataFixType::PoiChunk {
                1945
            } else {
                -1
            },
            recreate_region_files: recreate,
            context: None,
            tag_modifiers: BTreeMap::new(),
        }
    }
    /// `addTagModifier`: at most one per data version.
    fn add_tag_modifier(&mut self, version: i32, modifier: TagModifier) {
        let previous = self.tag_modifiers.insert(version, Arc::new(modifier));
        assert!(
            previous.is_none(),
            "Can't add two fixers for the same data version"
        );
    }
}

/// `RegionStorageUpgrader`: one data type of one dimension.
struct RegionStorageUpgrader {
    builder: Builder,
    progress: Arc<UpgradeProgress>,
    start_index: i32,
    dimension: String,
    region_folder: PathBuf,
    files: Vec<FileToUpgrade>,
    /// The open file chunks are written to (the source file itself, or the
    /// new one of `recreate_region_files`).
    writer: Option<(PathBuf, RegionFile)>,
    /// The open file chunks are read from.
    reader: Option<(PathBuf, RegionFile)>,
}

impl RegionStorageUpgrader {
    /// `init`: counts the chunks of the dimension's folder.
    fn init(
        builder: Builder,
        progress: Arc<UpgradeProgress>,
        start_index: i32,
        dimension: &str,
        base: &Path,
    ) -> Self {
        let region_folder = dimension_path(base, dimension).join(builder.folder);
        let files = all_chunk_positions(&region_folder);
        progress.add_total_file_fix_operations(files.len() as i32);
        progress.add_total_chunks(files.iter().map(|f| f.chunks.len() as i32).sum());
        Self {
            builder,
            progress,
            start_index,
            dimension: dimension.to_owned(),
            region_folder,
            files,
            writer: None,
            reader: None,
        }
    }

    fn file_amount(&self) -> i32 {
        self.files.len() as i32
    }

    /// `upgrade`.
    fn upgrade(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let total_size = self.progress.total_file_fix_stats().total_operations() as f32;
        self.progress.set_status(UpgradeStatus::Upgrading);
        let files = std::mem::take(&mut self.files);
        let mut next = 0usize;
        while !self.progress.is_canceled() {
            let mut worked = false;
            if let Some(file) = files.get(next) {
                next += 1;
                let mut converted = true;
                for &pos in &file.chunks {
                    // `converted && processOnePosition(..)`: once a chunk was not
                    // converted the rest of the file is not looked at.
                    converted = converted && self.process_one_position(&file.path, pos);
                    worked = true;
                }
                if self.builder.recreate_region_files {
                    if converted {
                        self.on_file_finished(&file.path);
                    } else {
                        eprintln!("Failed to convert region file {}", file.path.display());
                    }
                }
            }
            let dimension_progress = next as f32 / total_size;
            let total_progress = (self.start_index as usize + next) as f32 / total_size;
            self.progress
                .set_dimension_progress(&self.dimension, dimension_progress);
            self.progress.set_total_progress(total_progress);
            if !worked {
                break;
            }
        }
        self.progress.set_status(UpgradeStatus::Finished);
        self.close();
    }

    /// `processOnePosition`: an IO error skips the chunk, the logged way.
    fn process_one_position(&mut self, file: &Path, pos: ChunkPos) -> bool {
        let converted = match self.try_process_one_position(file, pos) {
            Ok(converted) => converted,
            Err(e) => {
                eprintln!("Error upgrading chunk {pos:?}: {e}");
                false
            }
        };
        if converted {
            self.progress.increment_converted();
        } else {
            self.progress.increment_skipped();
        }
        converted
    }

    fn upgrade_tag(&self, tag: &mut Tag, target_version: i32) {
        let version = as_int(tag.get("DataVersion"), self.builder.default_version);
        if version >= target_version {
            return;
        }
        match self.builder.data_fix_type {
            UpgradeDataFixType::Chunk => {
                let (dimension, generator) = self
                    .builder
                    .context
                    .as_ref()
                    .map_or((self.dimension.as_str(), None), |(d, g)| {
                        (d.as_str(), g.as_deref())
                    });
                update_chunk_to(tag, dimension, generator, target_version);
            }
            UpgradeDataFixType::EntityChunk => {
                update_region_tag_to(Ty::EntityChunk, tag, target_version)
            }
            UpgradeDataFixType::PoiChunk => update_region_tag_to(Ty::PoiChunk, tag, target_version),
        }
    }

    /// `tryProcessOnePosition`.
    fn try_process_one_position(&mut self, file: &Path, pos: ChunkPos) -> io::Result<bool> {
        let limits = Limits::default();
        if self.reader.as_ref().is_none_or(|(path, _)| path != file) {
            self.reader = Some((file.to_path_buf(), RegionFile::open(file)?));
        }
        let Some(record) = self
            .reader
            .as_mut()
            .map_or(Ok(None), |(_, r)| r.read(pos))?
        else {
            return Ok(false);
        };
        let mut root = record.nbt(limits)?;
        let version = as_int(root.tag.get("DataVersion"), -1);
        let mut changed = false;
        let modifiers: Vec<(i32, Arc<TagModifier>)> = self
            .builder
            .tag_modifiers
            .iter()
            .map(|(v, m)| (*v, m.clone()))
            .collect();
        for (needed_version, modifier) in modifiers {
            self.upgrade_tag(&mut root.tag, needed_version);
            changed |= modifier(pos, &mut root.tag);
        }
        self.upgrade_tag(&mut root.tag, CURRENT);
        changed |= version < CURRENT;
        if changed || self.builder.recreate_region_files {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as u32);
            let out = ChunkRecord::from_nbt(&root, Compression::Zlib, timestamp, limits)?;
            let target = if self.builder.recreate_region_files {
                let dir = resolve_recreate_directory(&self.region_folder);
                fs::create_dir_all(&dir)?;
                dir.join(file.file_name().unwrap_or_default())
            } else {
                file.to_path_buf()
            };
            if self.writer.as_ref().is_none_or(|(path, _)| *path != target) {
                self.writer = Some((target.clone(), RegionFile::open_writable(&target, true)?));
            }
            if let Some((_, writer)) = self.writer.as_mut() {
                writer.write(pos, &out)?;
                writer.sync()?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// `onFileFinished`: the rewritten file replaces the old one.
    fn on_file_finished(&mut self, file: &Path) {
        self.writer = None;
        self.reader = None;
        let new_file = resolve_recreate_directory(file.parent().unwrap_or(Path::new(".")))
            .join(file.file_name().unwrap_or_default());
        if new_file.exists() {
            if let Err(e) = fs::remove_file(file).and_then(|()| fs::rename(&new_file, file)) {
                eprintln!("Failed to replace an old region file: {e}");
            }
        } else {
            eprintln!(
                "Failed to replace an old region file. New file {} does not exist.",
                new_file.display()
            );
        }
    }

    /// `RecreatingSimpleRegionStorage.close`: the folder of the new files goes.
    fn close(&mut self) {
        self.writer = None;
        self.reader = None;
        if self.builder.recreate_region_files {
            let _ = fs::remove_dir_all(resolve_recreate_directory(&self.region_folder));
        }
    }
}

/// `resolveRecreateDirectory`: `new_` + the folder's name, beside it.
fn resolve_recreate_directory(folder: &Path) -> PathBuf {
    let name = folder.file_name().unwrap_or_default().to_string_lossy();
    folder.with_file_name(format!("{NEW_DIRECTORY_PREFIX}{name}"))
}

/// `r.<x>.<z>.mca`'s coordinates (`REGEX`).
fn region_coordinates(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
    let (x, z) = rest.split_once('.')?;
    let digits = |s: &str| {
        let body = s.strip_prefix('-').unwrap_or(s);
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(x) || !digits(z) {
        return None;
    }
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// `getAllChunkPositions`: the region files of a folder and the chunks in each.
fn all_chunk_positions(folder: &Path) -> Vec<FileToUpgrade> {
    let Ok(read) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some((rx, rz)) = region_coordinates(&name) else {
            continue;
        };
        let (x_offset, z_offset) = (rx.wrapping_shl(5), rz.wrapping_shl(5));
        match RegionFile::open(&path) {
            Ok(region) => {
                let present: HashSet<ChunkPos> = region.chunks().map(|(p, _)| p).collect();
                let mut chunks = Vec::new();
                for x in 0..32 {
                    for z in 0..32 {
                        let pos = ChunkPos::new(x + x_offset, z + z_offset);
                        if present.contains(&pos) {
                            chunks.push(pos);
                        }
                    }
                }
                if !chunks.is_empty() {
                    out.push(FileToUpgrade { path, chunks });
                }
            }
            Err(e) => eprintln!(
                "Failed to read chunks from region file {}: {e}",
                path.display()
            ),
        }
    }
    out
}

/// `WorldUpgrader`: runs on its own thread from [`WorldUpgrader::new`] on.
pub struct WorldUpgrader {
    levels: Vec<String>,
    progress: Arc<UpgradeProgress>,
    thread: Option<JoinHandle<()>>,
}

impl WorldUpgrader {
    /// Start upgrading the world in `base` (the folder with `level.dat`):
    /// `levels` are its dimensions ([`world_dimensions`]).
    pub fn new(
        base: &Path,
        levels: Vec<String>,
        erase_cache: bool,
        recreate_region_files: bool,
    ) -> Self {
        let progress = Arc::new(UpgradeProgress::new());
        let worker_progress = progress.clone();
        let worker_levels = levels.clone();
        let base = base.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("World Upgrader #0".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(
                        &base,
                        &worker_levels,
                        &worker_progress,
                        erase_cache,
                        recreate_region_files,
                    );
                }));
                if result.is_err() {
                    // The uncaught exception handler.
                    eprintln!("Error upgrading world");
                    worker_progress.set_status(UpgradeStatus::Failed);
                    worker_progress.set_finished(true);
                }
            })
            .ok();
        Self {
            levels,
            progress,
            thread,
        }
    }

    /// `cancel`: stops after the file in hand and waits for the worker.
    pub fn cancel(&mut self) {
        self.progress.set_canceled();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
    pub fn is_finished(&self) -> bool {
        self.progress.is_finished()
    }
    pub fn levels(&self) -> &[String] {
        &self.levels
    }
    pub fn dimension_progress(&self, dimension: &str) -> f32 {
        self.progress.dimension_progress(dimension)
    }
    pub fn total_progress(&self) -> f32 {
        self.progress.total_progress()
    }
    pub fn total_chunks(&self) -> i32 {
        self.progress.total_chunks()
    }
    pub fn converted(&self) -> i32 {
        self.progress.converted()
    }
    pub fn skipped(&self) -> i32 {
        self.progress.skipped()
    }
    /// `getStatus`, as a translation key.
    pub fn status_key(&self) -> String {
        status_key(&self.progress)
    }
}

impl Drop for WorldUpgrader {
    fn drop(&mut self) {
        // The screen's `removed`: `cancel` then `close`.
        self.cancel();
    }
}

/// `WorldUpgrader.work`.
fn work(
    base: &Path,
    levels: &[String],
    progress: &Arc<UpgradeProgress>,
    erase_cache: bool,
    recreate: bool,
) {
    let started = std::time::Instant::now();
    upgrade_levels(
        base,
        levels,
        progress,
        Builder::new(UpgradeDataFixType::EntityChunk, "entities", recreate),
        |_, _| {},
    );
    upgrade_levels(
        base,
        levels,
        progress,
        Builder::new(UpgradeDataFixType::PoiChunk, "poi", recreate),
        |_, _| {},
    );
    let generators = generators(base).unwrap_or_default();
    upgrade_levels(
        base,
        levels,
        progress,
        Builder::new(UpgradeDataFixType::Chunk, "region", recreate),
        |builder, level| {
            let generator = generators
                .iter()
                .find(|(d, _)| d == level)
                .map(|(_, g)| g.clone());
            builder.context = Some((level.to_owned(), generator));
            let modifier: TagModifier = if erase_cache {
                Box::new(verify_chunk_pos_and_erase_cache)
            } else {
                Box::new(verify_chunk_pos)
            };
            builder.add_tag_modifier(CURRENT, modifier);
        },
    );
    eprintln!(
        "World optimization finished after {} seconds",
        started.elapsed().as_secs()
    );
    progress.set_finished(true);
}

/// `upgradeLevels`: counts every dimension first, then upgrades them in turn.
fn upgrade_levels(
    base: &Path,
    levels: &[String],
    progress: &Arc<UpgradeProgress>,
    builder: Builder,
    level_specific: impl Fn(&mut Builder, &str),
) {
    progress.reset(builder.data_fix_type);
    progress.set_type(UpgradeType::Regions);
    let mut previous_copies_file_amounts = 0;
    let mut upgraders = Vec::new();
    for level in levels {
        let mut copy = builder.clone();
        level_specific(&mut copy, level);
        let upgrader = RegionStorageUpgrader::init(
            copy,
            progress.clone(),
            previous_copies_file_amounts,
            level,
            base,
        );
        previous_copies_file_amounts += upgrader.file_amount();
        upgraders.push(upgrader);
    }
    for upgrader in &mut upgraders {
        upgrader.upgrade();
    }
}

#[cfg(test)]
#[path = "world_upgrader_tests.rs"]
mod tests;
