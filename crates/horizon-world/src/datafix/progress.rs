//! `net.minecraft.util.worldupdate.UpgradeProgress`: what a world upgrade has
//! done so far, read by `FileFixerProgressScreen` while another thread works.
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};

/// `UpgradeProgress.FileFixStats`.
#[derive(Debug, Default)]
pub struct FileFixStats {
    finished_operations: AtomicI32,
    total_operations: AtomicI32,
}

impl FileFixStats {
    pub fn finished_operations(&self) -> i32 {
        self.finished_operations.load(Ordering::Relaxed)
    }
    pub fn total_operations(&self) -> i32 {
        self.total_operations.load(Ordering::Relaxed)
    }
    pub fn reset(&self) {
        self.finished_operations.store(0, Ordering::Relaxed);
        self.total_operations.store(0, Ordering::Relaxed);
    }
    /// `getProgress`: finished over total, 0 without operations, in 0..=1.
    pub fn progress(&self) -> f32 {
        if self.total_operations() == 0 {
            0.0
        } else {
            (self.finished_operations() as f32 / self.total_operations() as f32).clamp(0.0, 1.0)
        }
    }
}

/// `UpgradeProgress.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeType {
    Files,
    LegacyStructures,
    Regions,
}

impl UpgradeType {
    /// `getSerializedName`.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::LegacyStructures => "legacy_structures",
            Self::Regions => "regions",
        }
    }
    /// The key of `label()`: `upgradeWorld.progress.type.` + the serialized name.
    pub fn label_key(self) -> String {
        format!("upgradeWorld.progress.type.{}", self.serialized_name())
    }
    fn code(self) -> u8 {
        match self {
            Self::Files => 1,
            Self::LegacyStructures => 2,
            Self::Regions => 3,
        }
    }
}

/// `UpgradeProgress`, as far as the file fixer's screen reads it.
#[derive(Debug, Default)]
pub struct UpgradeProgress {
    total: FileFixStats,
    of_type: FileFixStats,
    running_fixer: FileFixStats,
    canceled: AtomicBool,
    /// `UpgradeProgress.type`: 0 for none.
    ty: AtomicU8,
    /// The upgrade runs the file fixers (`requiresFileFixing`), which is what
    /// opens `FileFixerProgressScreen` (`WorldOpenFlows.upgradeAndOpenWorld`).
    file_fixing: AtomicBool,
    // What `WorldUpgrader` (Optimize World) adds.
    finished: AtomicBool,
    total_progress: AtomicU32,
    total_chunks: AtomicI32,
    converted: AtomicI32,
    skipped: AtomicI32,
    /// `progressMap`: the dimension's id to its progress.
    dimension_progress: Mutex<HashMap<String, f32>>,
    status: AtomicU8,
    /// `dataFixType`: 0 for none.
    data_fix_type: AtomicU8,
}

/// `UpgradeProgress.Status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeStatus {
    Counting,
    Failed,
    Finished,
    Upgrading,
}

/// The `DataFixTypes` a `WorldUpgrader` run goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeDataFixType {
    Chunk,
    EntityChunk,
    PoiChunk,
}

impl UpgradeProgress {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn total_file_fix_stats(&self) -> &FileFixStats {
        &self.total
    }
    pub fn type_file_fix_stats(&self) -> &FileFixStats {
        &self.of_type
    }
    pub fn running_file_fixer_stats(&self) -> &FileFixStats {
        &self.running_fixer
    }
    pub fn add_total_file_fix_operations(&self, additional: i32) {
        self.total
            .total_operations
            .fetch_add(additional, Ordering::Relaxed);
        self.of_type
            .total_operations
            .fetch_add(additional, Ordering::Relaxed);
    }
    pub fn increment_finished_operations(&self) {
        self.increment_finished_operations_by(1);
    }
    pub fn increment_finished_operations_by(&self, count: i32) {
        self.total
            .finished_operations
            .fetch_add(count, Ordering::Relaxed);
        self.of_type
            .finished_operations
            .fetch_add(count, Ordering::Relaxed);
    }
    pub fn set_canceled(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }
    /// `setType`: the type's own stats start over.
    pub fn set_type(&self, ty: UpgradeType) {
        self.ty.store(ty.code(), Ordering::Relaxed);
        self.of_type.reset();
    }
    pub fn upgrade_type(&self) -> Option<UpgradeType> {
        match self.ty.load(Ordering::Relaxed) {
            1 => Some(UpgradeType::Files),
            2 => Some(UpgradeType::LegacyStructures),
            3 => Some(UpgradeType::Regions),
            _ => None,
        }
    }
    pub fn set_applicable_fixer_amount(&self, amount: i32) {
        self.running_fixer
            .total_operations
            .store(amount, Ordering::Relaxed);
    }
    pub fn increment_running_file_fixer(&self) {
        self.running_fixer
            .finished_operations
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
    pub fn set_finished(&self, finished: bool) {
        self.finished.store(finished, Ordering::Relaxed);
    }
    pub fn total_progress(&self) -> f32 {
        f32::from_bits(self.total_progress.load(Ordering::Relaxed))
    }
    pub fn set_total_progress(&self, progress: f32) {
        self.total_progress
            .store(progress.to_bits(), Ordering::Relaxed);
    }
    pub fn total_chunks(&self) -> i32 {
        self.total_chunks.load(Ordering::Relaxed)
    }
    pub fn add_total_chunks(&self, additional: i32) {
        self.total_chunks.fetch_add(additional, Ordering::Relaxed);
    }
    pub fn converted(&self) -> i32 {
        self.converted.load(Ordering::Relaxed)
    }
    pub fn increment_converted(&self) {
        self.converted.fetch_add(1, Ordering::Relaxed);
    }
    pub fn skipped(&self) -> i32 {
        self.skipped.load(Ordering::Relaxed)
    }
    pub fn increment_skipped(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_dimension_progress(&self, dimension: &str, progress: f32) {
        if let Ok(mut map) = self.dimension_progress.lock() {
            map.insert(dimension.to_owned(), progress);
        }
    }
    /// `getDimensionProgress`: 0 for a dimension nothing was reported for.
    pub fn dimension_progress(&self, dimension: &str) -> f32 {
        self.dimension_progress
            .lock()
            .ok()
            .and_then(|map| map.get(dimension).copied())
            .unwrap_or(0.0)
    }
    pub fn status(&self) -> UpgradeStatus {
        match self.status.load(Ordering::Relaxed) {
            1 => UpgradeStatus::Failed,
            2 => UpgradeStatus::Finished,
            3 => UpgradeStatus::Upgrading,
            _ => UpgradeStatus::Counting,
        }
    }
    pub fn set_status(&self, status: UpgradeStatus) {
        let code = match status {
            UpgradeStatus::Counting => 0,
            UpgradeStatus::Failed => 1,
            UpgradeStatus::Finished => 2,
            UpgradeStatus::Upgrading => 3,
        };
        self.status.store(code, Ordering::Relaxed);
    }
    pub fn data_fix_type(&self) -> Option<UpgradeDataFixType> {
        match self.data_fix_type.load(Ordering::Relaxed) {
            1 => Some(UpgradeDataFixType::Chunk),
            2 => Some(UpgradeDataFixType::EntityChunk),
            3 => Some(UpgradeDataFixType::PoiChunk),
            _ => None,
        }
    }
    /// `reset(DataFixTypes)`: the counts of the next data type start over.
    pub fn reset(&self, data_fix_type: UpgradeDataFixType) {
        self.total.reset();
        self.of_type.reset();
        self.total_chunks.store(0, Ordering::Relaxed);
        self.converted.store(0, Ordering::Relaxed);
        self.skipped.store(0, Ordering::Relaxed);
        let code = match data_fix_type {
            UpgradeDataFixType::Chunk => 1,
            UpgradeDataFixType::EntityChunk => 2,
            UpgradeDataFixType::PoiChunk => 3,
        };
        self.data_fix_type.store(code, Ordering::Relaxed);
    }
    pub fn set_file_fixing(&self) {
        self.file_fixing.store(true, Ordering::Relaxed);
    }
    pub fn is_file_fixing(&self) -> bool {
        self.file_fixing.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_follow_the_operations_and_the_type_starts_over() {
        let p = UpgradeProgress::new();
        assert_eq!(p.total_file_fix_stats().progress(), 0.0);
        p.set_type(UpgradeType::Files);
        p.add_total_file_fix_operations(4);
        p.increment_finished_operations();
        assert_eq!(p.total_file_fix_stats().progress(), 0.25);
        p.set_type(UpgradeType::Regions);
        assert_eq!(p.type_file_fix_stats().total_operations(), 0);
        assert_eq!(p.total_file_fix_stats().total_operations(), 4);
        p.increment_finished_operations_by(9);
        assert_eq!(p.type_file_fix_stats().progress(), 0.0);
        assert_eq!(p.total_file_fix_stats().progress(), 1.0);
        assert_eq!(
            p.upgrade_type().unwrap().label_key(),
            "upgradeWorld.progress.type.regions"
        );
    }
}
