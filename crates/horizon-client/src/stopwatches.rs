//! `Stopwatches`: the named wall-clock timers of `/stopwatch`, saved in
//! `data/minecraft/stopwatches.dat` as the elapsed milliseconds of each.
use crate::save;
use horizon_world::nbt::{Compound, JavaString, Tag};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

/// `Util.getMillis`, measured from the first call.
pub fn current_time() -> i64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as i64
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Stopwatch {
    creation_time: i64,
    accumulated: i64,
}
impl Stopwatch {
    fn elapsed_ms(self, now: i64) -> i64 {
        self.accumulated + (now - self.creation_time)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stopwatches {
    watches: BTreeMap<String, Stopwatch>,
    dirty: bool,
}
fn file(root: &Path) -> PathBuf {
    root.join("data/minecraft/stopwatches.dat")
}
impl Stopwatches {
    pub fn load(root: &Path) -> Self {
        let mut s = Self::default();
        let now = current_time();
        if let Some(Tag::Compound(data)) =
            save::read_dat(&file(root)).and_then(|t| t.get("data").cloned())
            && let Some(Tag::Compound(map)) = data.get(&JavaString::from("stopwatches"))
        {
            for (k, v) in map.iter() {
                if let Tag::Long(ms) = v {
                    s.watches.insert(
                        k.to_string_lossy(),
                        Stopwatch {
                            creation_time: now,
                            accumulated: *ms,
                        },
                    );
                }
            }
        }
        s
    }
    pub fn save(&mut self, root: &Path) {
        if !self.dirty && self.watches.is_empty() {
            return;
        }
        self.dirty = false;
        let now = current_time();
        let map: Compound = self
            .watches
            .iter()
            .map(|(k, w)| (k.as_str().into(), Tag::Long(w.elapsed_ms(now))))
            .collect();
        if let Err(e) = save::write_dat(
            &file(root),
            Tag::compound([
                ("data", Tag::compound([("stopwatches", Tag::Compound(map))])),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ]),
        ) {
            log::warn!("Horizon: the stopwatches are not saved: {e}");
        }
    }
    pub fn ids(&self) -> Vec<String> {
        self.watches.keys().cloned().collect()
    }
    /// `add`: false when the id exists.
    pub fn create(&mut self, id: &str) -> bool {
        if self.watches.contains_key(id) {
            return false;
        }
        self.watches.insert(
            id.to_owned(),
            Stopwatch {
                creation_time: current_time(),
                accumulated: 0,
            },
        );
        self.dirty = true;
        true
    }
    /// `update` with a fresh stopwatch.
    pub fn restart(&mut self, id: &str) -> bool {
        let Some(w) = self.watches.get_mut(id) else {
            return false;
        };
        *w = Stopwatch {
            creation_time: current_time(),
            accumulated: 0,
        };
        self.dirty = true;
        true
    }
    pub fn remove(&mut self, id: &str) -> bool {
        let removed = self.watches.remove(id).is_some();
        self.dirty |= removed;
        removed
    }
    /// `Stopwatch.elapsedSeconds`.
    pub fn elapsed_seconds(&self, id: &str) -> Option<f64> {
        self.watches
            .get(id)
            .map(|w| w.elapsed_ms(current_time()) as f64 / 1000.0)
    }
}
