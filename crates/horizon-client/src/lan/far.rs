//! Horizon's far ring (docs/hosting.md), off by default: past a dedicated server's view distance, up to
//! its `max_view_distance`, each player is sent the chunks the world has saved. They are read on threads
//! of their own and dropped once sent: nothing is generated, loaded or ticked for them, and only their
//! positions stay (the player's `sent_chunks`). Bodies sent are kept a while for players walking back. A
//! brake pulls the ring in while ticks run long and lets it out slowly once they are short again.

use super::chunks::ChunkEncoder;
use crate::{biome::Biomes, fast_hash::FxHashMap, save::SaveDir};
use horizon_world::registry::BlockRegistry;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

/// Saved chunks being read at once, all players together.
const ASKED: usize = 32;
/// Far chunks a player's batch looks at, closest first: a ring waiting on its reads is not scanned
/// to the end each tick.
pub const LOOKS: usize = 2 * ASKED;
/// The bodies kept for players coming back: this many bytes, this long.
const KEPT_BYTES: usize = 96 << 20;
const KEPT_FOR: Duration = Duration::from_secs(300);
/// How long a chunk the save doesn't have (or not as a full chunk with its light) is not looked for again.
const MISSING_FOR: Duration = Duration::from_secs(30);
/// Chunks waiting for the save's writer past which nothing is read: gameplay's saves go first.
const WRITE_BACKLOG: usize = 512;
/// The brake: the average tick above which the ring pulls in, the one below which it may grow, the
/// seconds of short ticks before it does, and then the seconds between its steps out.
const HOT_MS: f64 = 40.0;
const CALM_MS: f64 = 25.0;
const CALM_SECONDS: u32 = 30;
const GROW_EVERY: u32 = 5;
const READERS: usize = 2;

/// A level's place in `Far::levels`, and a chunk of it.
type Key = (usize, i32, i32);

/// What the readers read a level from.
pub struct Source {
    save: Arc<SaveDir>,
    /// The hub's encoder for the level, with caches of its own (`ChunkEncoder::detached`).
    encoder: ChunkEncoder,
}

/// Reads a saved chunk's body: the game's reads it with `SaveDir::load_far`, encoded by
/// `ChunkEncoder::body_alone`.
type Reader = Arc<dyn Fn(&Source, i32, i32) -> Option<Vec<u8>> + Send + Sync>;

struct Level {
    name: String,
    source: Arc<Source>,
    /// Bodies read of the level, with when.
    bodies: FxHashMap<(i32, i32), (Instant, Arc<[u8]>)>,
}

/// A far chunk as [`Far::saved`] finds it.
pub enum Saved {
    Body(Arc<[u8]>),
    /// The save has none to send: not sent.
    Missing,
    /// Being read, or to be asked for once there is room.
    Waiting,
}

pub struct Far {
    /// `max_view_distance`.
    pub max: i32,
    brake: Brake,
    levels: Vec<Level>,
    /// The level in place (`set_level`).
    here: Option<usize>,
    /// Every body kept, oldest first, with when it was read (a later read of the column leaves its
    /// entry behind, skipped once it comes up).
    order: VecDeque<(Key, Instant)>,
    bytes: usize,
    /// The reads asked for, true once what they read is out of date.
    asked: FxHashMap<Key, bool>,
    missing: FxHashMap<Key, Instant>,
    jobs: mpsc::Sender<(Key, Arc<Source>)>,
    done: mpsc::Receiver<(Key, Option<Vec<u8>>)>,
}

impl Far {
    /// A far ring out to `max`, its readers decoding with the game's blocks and biomes.
    pub fn new(max: i32, registry: &'static BlockRegistry, biomes: Arc<Biomes>) -> Self {
        Self::reading(
            max,
            Arc::new(move |source: &Source, x, z| {
                let chunk = source.save.load_far(x, z, registry, &biomes)?;
                Some(source.encoder.body_alone(&chunk))
            }),
        )
    }

    fn reading(max: i32, read_one: Reader) -> Self {
        let (jobs, asked) = mpsc::channel::<(Key, Arc<Source>)>();
        let (read, done) = mpsc::channel();
        let asked = Arc::new(Mutex::new(asked));
        for n in 0..READERS {
            let (asked, read, read_one) = (asked.clone(), read.clone(), read_one.clone());
            let _ = std::thread::Builder::new()
                .name(format!("Far chunk reader #{n}"))
                .spawn(move || {
                    loop {
                        // The far ring is gone once its sender is.
                        let Ok(((level, x, z), source)) = asked.lock().unwrap().recv() else {
                            return;
                        };
                        let body = read_one(&source, x, z);
                        if read.send(((level, x, z), body)).is_err() {
                            return;
                        }
                    }
                });
        }
        Self {
            max,
            brake: Brake {
                limit: max,
                calm: 0,
            },
            levels: Vec::new(),
            here: None,
            order: VecDeque::new(),
            bytes: 0,
            asked: Default::default(),
            missing: Default::default(),
            jobs,
            done,
        }
    }

    /// How far the ring reaches now, the brake's: from the view distance up to `max`.
    pub fn limit(&self) -> i32 {
        self.brake.limit
    }

    /// The level the hub has in place, its saves, and the hub's encoder as set for it.
    pub fn set_level(&mut self, name: &str, save: &Arc<SaveDir>, encoder: &ChunkEncoder) {
        let source = || {
            Arc::new(Source {
                save: save.clone(),
                encoder: encoder.detached(),
            })
        };
        let i = match self.levels.iter().position(|l| l.name == name) {
            Some(i) if Arc::ptr_eq(&self.levels[i].source.save, save) => i,
            Some(i) => {
                // Another world's level of the same name.
                self.here = Some(i);
                self.forget_level();
                self.levels[i].source = source();
                i
            }
            None => {
                self.levels.push(Level {
                    name: name.to_owned(),
                    source: source(),
                    bodies: Default::default(),
                });
                self.levels.len() - 1
            }
        };
        self.here = Some(i);
    }

    /// A saved chunk of the level in place, asking for it to be read when there is room.
    pub fn saved(&mut self, x: i32, z: i32) -> Saved {
        let Some(level) = self.here else {
            return Saved::Missing;
        };
        let key = (level, x, z);
        let bodies = &mut self.levels[level].bodies;
        match bodies.get(&(x, z)) {
            Some((at, body)) if at.elapsed() < KEPT_FOR => return Saved::Body(body.clone()),
            Some(_) => self.bytes -= bodies.remove(&(x, z)).map_or(0, |(_, b)| b.len()),
            None => {}
        }
        match self.missing.get(&key) {
            Some(at) if at.elapsed() < MISSING_FOR => return Saved::Missing,
            Some(_) => {
                self.missing.remove(&key);
            }
            None => {}
        }
        if self.asked.contains_key(&key) || self.asked.len() >= ASKED {
            return Saved::Waiting;
        }
        let source = self.levels[level].source.clone();
        if source.save.pending_writes() <= WRITE_BACKLOG && self.jobs.send((key, source)).is_ok() {
            self.asked.insert(key, false);
        }
        Saved::Waiting
    }

    /// Takes in what the readers have read.
    pub fn poll(&mut self) {
        while let Ok((key, body)) = self.done.try_recv() {
            match body {
                _ if self.asked.remove(&key).is_none_or(|stale| stale) => {}
                Some(body) => self.keep(key, body.into()),
                None => {
                    self.missing.insert(key, Instant::now());
                }
            }
        }
    }

    fn keep(&mut self, key: Key, body: Arc<[u8]>) {
        let (level, x, z) = key;
        let now = Instant::now();
        self.bytes += body.len();
        if let Some((_, old)) = self.levels[level].bodies.insert((x, z), (now, body)) {
            self.bytes -= old.len();
        }
        self.order.push_back((key, now));
        while self.bytes > KEPT_BYTES {
            self.drop_oldest();
        }
        if self.order.len() > 2 * self.bodies() + 1024 {
            let levels = &self.levels;
            self.order.retain(|((level, x, z), at)| {
                levels[*level]
                    .bodies
                    .get(&(*x, *z))
                    .is_some_and(|(t, _)| t == at)
            });
        }
    }

    fn bodies(&self) -> usize {
        self.levels.iter().map(|l| l.bodies.len()).sum()
    }

    /// Lets go of the oldest body kept (none when there is none).
    fn drop_oldest(&mut self) {
        let Some(((level, x, z), at)) = self.order.pop_front() else {
            self.bytes = 0;
            return;
        };
        let bodies = &mut self.levels[level].bodies;
        if bodies.get(&(x, z)).is_some_and(|(t, _)| *t == at) {
            self.bytes -= bodies.remove(&(x, z)).map_or(0, |(_, b)| b.len());
        }
    }

    /// A column of the level in place changed: what was read of it is out of date.
    pub fn changed(&mut self, x: i32, z: i32) {
        let Some(level) = self.here else {
            return;
        };
        if let Some((_, body)) = self.levels[level].bodies.remove(&(x, z)) {
            self.bytes -= body.len();
        }
        if let Some(stale) = self.asked.get_mut(&(level, x, z)) {
            *stale = true;
        }
    }

    /// The level in place has no players: what changes in it goes unlogged (`tick_level`), so nothing
    /// read of it is kept.
    pub fn forget_level(&mut self) {
        let Some(level) = self.here else {
            return;
        };
        let bodies = &mut self.levels[level].bodies;
        if !bodies.is_empty() {
            self.bytes -= bodies.drain().map(|(_, (_, b))| b.len()).sum::<usize>();
        }
        for (_, stale) in self.asked.iter_mut().filter(|((l, ..), _)| *l == level) {
            *stale = true;
        }
    }

    /// Once a second, with the average tick (`Brake::step`); what was kept too long is let go.
    pub fn brake(&mut self, mspt: f64, view: i32) {
        self.brake.step(mspt, view, self.max);
        while self
            .order
            .front()
            .is_some_and(|(_, at)| at.elapsed() >= KEPT_FOR)
        {
            self.drop_oldest();
        }
        self.missing.retain(|_, at| at.elapsed() < MISSING_FOR);
    }
}

/// How far the ring reaches, and the seconds of short ticks in a row.
struct Brake {
    limit: i32,
    calm: u32,
}

impl Brake {
    /// While ticks take over `HOT_MS` the ring pulls in by two chunks, never inside `view`; after
    /// `CALM_SECONDS` of ticks under `CALM_MS` it grows back by one every `GROW_EVERY` seconds.
    fn step(&mut self, mspt: f64, view: i32, max: i32) {
        if mspt > HOT_MS {
            self.calm = 0;
            self.limit -= 2;
        } else if mspt < CALM_MS {
            self.calm = self.calm.saturating_add(1);
            if self.calm >= CALM_SECONDS && (self.calm - CALM_SECONDS).is_multiple_of(GROW_EVERY) {
                self.limit += 1;
            }
        } else {
            self.calm = 0;
        }
        self.limit = self.limit.min(max).max(view);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::LevelShape;
    use horizon_protocol::registry::Builtin;

    #[test]
    fn the_brake_pulls_in_fast_and_lets_out_slowly() {
        let mut b = Brake { limit: 24, calm: 0 };
        let step = |b: &mut Brake, mspt| {
            b.step(mspt, 10, 24);
            b.limit
        };
        assert_eq!(step(&mut b, 60.0), 22);
        for _ in 0..10 {
            step(&mut b, 60.0);
        }
        // Never inside the view distance.
        assert_eq!(b.limit, 10);
        // Between the two marks it holds.
        for _ in 0..100 {
            assert_eq!(step(&mut b, 30.0), 10);
        }
        for _ in 0..CALM_SECONDS - 1 {
            assert_eq!(step(&mut b, 5.0), 10);
        }
        assert_eq!(step(&mut b, 5.0), 11);
        for _ in 0..GROW_EVERY - 1 {
            assert_eq!(step(&mut b, 5.0), 11);
        }
        assert_eq!(step(&mut b, 5.0), 12);
        // One long second starts the wait again.
        assert_eq!(step(&mut b, 45.0), 10);
        for _ in 0..CALM_SECONDS + 100 * GROW_EVERY {
            step(&mut b, 5.0);
        }
        assert_eq!(b.limit, 24);
    }

    /// Reads (x, z) as a body of x when x is even, none otherwise; counts them.
    fn far(reads: Arc<std::sync::atomic::AtomicUsize>) -> Far {
        let mut far = Far::reading(
            16,
            Arc::new(move |_: &Source, x, _| {
                reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                (x % 2 == 0).then(|| vec![x as u8])
            }),
        );
        let save = Arc::new(SaveDir::new(
            std::env::temp_dir().join("horizon-far-ring-test"),
            LevelShape::OVERWORLD,
        ));
        let encoder = ChunkEncoder::new(
            [(Arc::<str>::from("minecraft:air"), false)],
            &Builtin::default(),
            &[],
            &[],
        );
        far.set_level("overworld", &save, &encoder);
        far
    }

    /// `saved` until the read is in.
    fn read(far: &mut Far, x: i32, z: i32) -> Saved {
        for _ in 0..1000 {
            far.poll();
            match far.saved(x, z) {
                Saved::Waiting => std::thread::sleep(Duration::from_millis(1)),
                done => return done,
            }
        }
        panic!("({x}, {z}) was never read");
    }

    #[test]
    fn reads_are_kept_until_their_column_changes() {
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut far = far(reads.clone());
        let count = || reads.load(std::sync::atomic::Ordering::Relaxed);
        assert!(matches!(read(&mut far, 4, 1), Saved::Body(b) if *b == [4]));
        assert!(matches!(read(&mut far, 3, 1), Saved::Missing));
        assert_eq!(count(), 2);
        // Walking back: neither is read again.
        assert!(matches!(far.saved(4, 1), Saved::Body(_)));
        assert!(matches!(far.saved(3, 1), Saved::Missing));
        assert_eq!(count(), 2);
        // A block placed in it: read again.
        far.changed(4, 1);
        assert!(matches!(far.saved(4, 1), Saved::Waiting));
        assert!(matches!(read(&mut far, 4, 1), Saved::Body(_)));
        assert_eq!(count(), 3);
        // Nobody left in the level: nothing kept.
        far.forget_level();
        assert!(matches!(far.saved(4, 1), Saved::Waiting));
        assert_eq!(far.bytes, 0);
    }

    /// A read that a change overtook is not kept.
    #[test]
    fn a_read_overtaken_by_a_change_is_dropped() {
        let mut far = far(Arc::default());
        assert!(matches!(far.saved(6, 0), Saved::Waiting));
        far.changed(6, 0);
        // Its answer comes back stale and is let go: asked again.
        for _ in 0..1000 {
            far.poll();
            if far.asked.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(far.levels[0].bodies.is_empty());
        assert!(matches!(read(&mut far, 6, 0), Saved::Body(_)));
    }
}
