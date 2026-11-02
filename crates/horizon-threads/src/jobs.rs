//! One pool of worker threads for the whole process, shared by every source of
//! background work: chunk generation, light and meshing (the client's and each
//! of the server thread's levels), chunk encoding, far terrain.
//!
//! Before it, each of those started its own threads, about one per core each:
//! a single player world ran four or five such pools at once, ~100 threads on a
//! 32-thread CPU, and they fought each other and the render and server threads
//! for cores. Here the threads are counted once, for the machine
//! ([`configure`]), and the work goes to them in the order it matters.
//!
//! A [`Source`] keeps its own queue (it can rank its jobs again when the player
//! moves, or drop the ones that left the view); a worker that is free asks every
//! source for the [`Rank`] of its next job and runs the most urgent one: the
//! [`Class`] first, then the source's key (a chunk's distance from the player).
//! Every free worker takes the next job as soon as it is free, so no core waits
//! while work is queued, whichever source queued it. Jobs are coarse (a chunk, a
//! section, a tile: a millisecond or more), so one lock for the choice costs
//! nothing next to them.
//!
//! A worker asks the sources for their next rank under the pool's read lock
//! of its sources and nothing else, so a source may call [`Pool::notify`]
//! anywhere, its queue's lock held or not; a notify never waits for a worker.
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Duration;

/// How urgent a job is, before its source's key: every queued job of a lower
/// class runs before any of a higher one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Class {
    /// Work something waits for now: saves, a tick's chunk encoding.
    Urgent,
    /// The chunks around the player: generation, light, meshing.
    Near,
    /// Far terrain: runs on what the near work leaves, and never on every
    /// worker at once, so near work that arrives finds a worker free.
    Far,
}

/// A job's place in the pool's order: lower runs first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank {
    pub class: Class,
    /// The source's own order within the class (a column's distance).
    pub key: i64,
}

impl Rank {
    pub const fn new(class: Class, key: i64) -> Self {
        Self { class, key }
    }
}

/// Work queued for the pool.
pub trait Source: Send + Sync {
    /// The rank of the job [`Source::run`] would take next, or None while
    /// nothing is queued. Called under the pool's lock: cheap, no blocking.
    fn next(&self) -> Option<Rank>;
    /// Take the next job and run it on worker `worker` (0..[`Pool::threads`]).
    /// False if there was none after all.
    fn run(&self, worker: usize) -> bool;
    /// Worker `worker` has waited [`IDLE_RELEASE`] with no work: free what it
    /// keeps between jobs.
    fn idle(&self, _worker: usize) {}
    /// Every worker is idle: free what the workers share.
    fn all_idle(&self) {}
}

/// How long a worker waits with no job before it frees its buffers.
pub const IDLE_RELEASE: Duration = Duration::from_secs(5);

/// Per-worker state for a source: worker `i` only ever touches slot `i`, so
/// the locks are never contended. The source owns it, so it goes with the source.
pub struct PerWorker<T> {
    slots: Box<[Mutex<Option<T>>]>,
}

impl<T> PerWorker<T> {
    pub fn new(workers: usize) -> Self {
        Self {
            slots: (0..workers.max(1)).map(|_| Mutex::new(None)).collect(),
        }
    }
    /// `f` with worker `worker`'s state, made by `init` the first time.
    pub fn with<R>(
        &self,
        worker: usize,
        init: impl FnOnce() -> T,
        f: impl FnOnce(&mut T) -> R,
    ) -> R {
        let mut slot = lock(&self.slots[worker % self.slots.len()]);
        f(slot.get_or_insert_with(init))
    }
    /// `f` with worker `worker`'s state if it has one.
    pub fn with_existing<R>(&self, worker: usize, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        lock(&self.slots[worker % self.slots.len()]).as_mut().map(f)
    }
    /// Drop worker `worker`'s state.
    pub fn clear(&self, worker: usize) -> Option<T> {
        lock(&self.slots[worker % self.slots.len()]).take()
    }
}

struct Entry {
    id: u64,
    source: Arc<dyn Source>,
    /// Workers running one of its jobs.
    running: AtomicUsize,
    /// Taken out of the pool: the worker that ends its last job tells
    /// [`Registration`]'s drop, which waits for it.
    retired: AtomicBool,
}

struct Inner {
    /// Read by every choice, written only to add or take out a source.
    sources: RwLock<Vec<Arc<Entry>>>,
    next_id: AtomicU64,
    /// Bumped by every [`Pool::notify`]: a worker that found nothing sleeps
    /// until it changes, so a job queued while it looked is not missed. A
    /// notify is two atomic operations while every worker is busy, and takes
    /// `sleep` (held by no source code) only to wake one: the thread queuing
    /// work (a frame, a tick) never waits behind a worker's choice.
    epoch: AtomicU64,
    sleepers: AtomicUsize,
    sleep: Mutex<()>,
    wake: Condvar,
    /// Signalled when a retired source's job ends.
    finished: Condvar,
    /// Workers running a [`Class::Far`] job.
    far: AtomicUsize,
    /// Workers that freed their buffers and took no job since.
    released: AtomicUsize,
    threads: AtomicUsize,
    idle_release: Duration,
}

/// The pool: [`pool`] for the process's own, [`Pool::new`] for a test's.
pub struct Pool {
    inner: Arc<Inner>,
    threads: usize,
}

/// A source's place in the pool; dropping it takes the source out and waits
/// for the jobs of it that are running (so never drop it in one of its own jobs).
pub struct Registration {
    inner: Arc<Inner>,
    id: u64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic in a job is caught; the pool's own state is never left half
    // changed, so a poisoned lock is still consistent.
    m.lock().unwrap_or_else(|e| e.into_inner())
}

static CONFIGURED: AtomicUsize = AtomicUsize::new(0);
static POOL: OnceLock<Pool> = OnceLock::new();

/// The number of workers the machine gets by default: one thread per core
/// but one, which the render and server threads share with them (they run
/// ahead of the workers: [`crate::favour_current`], [`crate::behind_server`]).
pub fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .saturating_sub(1)
        .max(1)
}

/// Size the process's pool before its first use (later calls change
/// nothing): `threads` workers, or `HORIZON_JOB_THREADS` when it is set.
pub fn configure(threads: usize) {
    CONFIGURED.store(threads.max(1), Ordering::Relaxed);
}

/// The process's pool, started on first use.
pub fn pool() -> &'static Pool {
    POOL.get_or_init(|| {
        let threads = std::env::var("HORIZON_JOB_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or_else(|| match CONFIGURED.load(Ordering::Relaxed) {
                0 => default_threads(),
                n => n,
            });
        // The name the crash reports know a worker by (vanilla's `Util` workers).
        Pool::new(threads, "horizon-worker")
    })
}

impl Pool {
    /// A pool of `threads` workers named `name-<i>`. Workers that the system
    /// refuses to start leave it smaller ([`Pool::threads`] counts those that started).
    pub fn new(threads: usize, name: &str) -> Self {
        Self::with_idle_release(threads, name, IDLE_RELEASE)
    }

    /// [`Pool::new`] whose workers free their buffers after `idle_release`.
    pub fn with_idle_release(threads: usize, name: &str, idle_release: Duration) -> Self {
        let inner = Arc::new(Inner {
            sources: RwLock::new(Vec::new()),
            next_id: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            sleepers: AtomicUsize::new(0),
            sleep: Mutex::new(()),
            wake: Condvar::new(),
            finished: Condvar::new(),
            far: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            threads: AtomicUsize::new(threads.max(1)),
            idle_release,
        });
        let mut started = 0;
        for i in 0..threads.max(1) {
            let worker = inner.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("{name}-{i}"))
                .spawn(move || {
                    // Ticks and frames come first.
                    crate::behind_server();
                    work(&worker, i);
                });
            if spawned.is_ok() {
                started += 1;
            }
        }
        inner.threads.store(started.max(1), Ordering::Relaxed);
        Self {
            inner,
            threads: started.max(1),
        }
    }

    /// The workers: the size of a source's [`PerWorker`] and of the work it
    /// keeps queued.
    pub fn threads(&self) -> usize {
        self.threads
    }

    /// Add a source; its jobs run until the registration is dropped.
    pub fn register(&self, source: Arc<dyn Source>) -> Registration {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .sources
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::new(Entry {
                id,
                source,
                running: AtomicUsize::new(0),
                retired: AtomicBool::new(false),
            }));
        self.inner.notify(true);
        Registration {
            inner: self.inner.clone(),
            id,
        }
    }

    /// Wake a worker for a job just queued (after the queue's lock is released).
    pub fn notify(&self) {
        self.inner.notify(false);
    }

    /// Wake every worker: many jobs were queued at once.
    pub fn notify_all(&self) {
        self.inner.notify(true);
    }
}

impl Inner {
    fn notify(&self, all: bool) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        if self.sleepers.load(Ordering::SeqCst) > 0 {
            // A worker between counting itself asleep and waiting holds
            // `sleep`: taking it here means it is waiting when told.
            drop(lock(&self.sleep));
            if all {
                self.wake.notify_all();
            } else {
                self.wake.notify_one();
            }
        }
    }
}

impl Registration {
    /// Wake a worker for a job just queued: [`Pool::notify`].
    pub fn notify(&self) {
        self.inner.notify(false);
    }
    pub fn notify_all(&self) {
        self.inner.notify(true);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let entry = {
            let mut sources = self
                .inner
                .sources
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let Some(at) = sources.iter().position(|e| e.id == self.id) else {
                return;
            };
            sources.swap_remove(at)
        };
        // No worker takes a job of it now (a worker counts itself running
        // while it holds the sources' read lock); wait for those that did.
        entry.retired.store(true, Ordering::SeqCst);
        let mut sleep = lock(&self.inner.sleep);
        while entry.running.load(Ordering::SeqCst) > 0 {
            sleep = self
                .inner
                .finished
                .wait(sleep)
                .unwrap_or_else(|e| e.into_inner());
        }
        drop(sleep);
        // The source's last reference may go here, outside every lock.
        drop(entry);
    }
}

/// The most urgent queued job's source, counted as running, if any may run now.
fn choose(inner: &Inner) -> Option<(Arc<Entry>, bool)> {
    let threads = inner.threads.load(Ordering::Relaxed);
    // Far work keeps one worker free for near work (when there are two or more).
    let far_cap = threads.saturating_sub(1).max(1);
    let sources = inner.sources.read().unwrap_or_else(|e| e.into_inner());
    let mut skip_far = false;
    loop {
        let far_open = !skip_far && inner.far.load(Ordering::SeqCst) < far_cap;
        let mut best: Option<(&Arc<Entry>, Rank)> = None;
        for entry in sources.iter() {
            let Some(rank) = entry.source.next() else {
                continue;
            };
            if rank.class == Class::Far && !far_open {
                continue;
            }
            if best.is_none_or(|(_, b)| rank < b) {
                best = Some((entry, rank));
            }
        }
        let (entry, rank) = best?;
        let far = rank.class == Class::Far;
        if far && !take_far_place(inner, far_cap) {
            // Another worker took the last far place meanwhile.
            skip_far = true;
            continue;
        }
        entry.running.fetch_add(1, Ordering::SeqCst);
        return Some((entry.clone(), far));
    }
}

/// One more far job running, if fewer than `cap` are.
fn take_far_place(inner: &Inner, cap: usize) -> bool {
    let mut n = inner.far.load(Ordering::SeqCst);
    while n < cap {
        match inner
            .far
            .compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => return true,
            Err(now) => n = now,
        }
    }
    false
}

fn work(inner: &Inner, worker: usize) {
    let mut released = false;
    loop {
        let epoch = inner.epoch.load(Ordering::SeqCst);
        if let Some((entry, far)) = choose(inner) {
            if released {
                inner.released.fetch_sub(1, Ordering::Relaxed);
                released = false;
            }
            // A panicking job loses its own work, not the worker: the panic
            // hook has already reported it.
            let _ = catch_unwind(AssertUnwindSafe(|| entry.source.run(worker)));
            if far {
                inner.far.fetch_sub(1, Ordering::SeqCst);
            }
            entry.running.fetch_sub(1, Ordering::SeqCst);
            if entry.retired.load(Ordering::SeqCst) {
                drop(lock(&inner.sleep));
                inner.finished.notify_all();
            }
            continue;
        }
        let sleep = lock(&inner.sleep);
        inner.sleepers.fetch_add(1, Ordering::SeqCst);
        let still = |_: &mut ()| inner.epoch.load(Ordering::SeqCst) == epoch;
        let timed_out = if released {
            drop(
                inner
                    .wake
                    .wait_while(sleep, still)
                    .unwrap_or_else(|e| e.into_inner()),
            );
            false
        } else {
            let (sleep, wait) = inner
                .wake
                .wait_timeout_while(sleep, inner.idle_release, still)
                .unwrap_or_else(|e| e.into_inner());
            drop(sleep);
            wait.timed_out()
        };
        inner.sleepers.fetch_sub(1, Ordering::SeqCst);
        if !timed_out {
            continue;
        }
        // Nothing to do for a while (a server nobody is on, a player standing
        // still): free this worker's buffers until the next job needs them.
        released = true;
        let last = inner.released.fetch_add(1, Ordering::Relaxed) + 1
            == inner.threads.load(Ordering::Relaxed);
        let sources: Vec<_> = inner
            .sources
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|e| e.source.clone())
            .collect();
        for source in &sources {
            let _ = catch_unwind(AssertUnwindSafe(|| source.idle(worker)));
        }
        if last {
            // The allocator keeps freed pages until memory runs short: once
            // every worker has freed its own, hand them back.
            for source in &sources {
                let _ = catch_unwind(AssertUnwindSafe(|| source.all_idle()));
            }
            crate::release_free_memory();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc;
    use std::time::Instant;

    /// Jobs with fixed ranks; each records its name when it runs.
    struct Fixed {
        jobs: Mutex<VecDeque<(Rank, &'static str)>>,
        ran: mpsc::Sender<&'static str>,
        hold: Option<Arc<(Mutex<bool>, Condvar)>>,
    }
    impl Source for Fixed {
        fn next(&self) -> Option<Rank> {
            lock(&self.jobs).iter().map(|(r, _)| *r).min()
        }
        fn run(&self, _worker: usize) -> bool {
            let job = {
                let mut jobs = lock(&self.jobs);
                let Some(at) = jobs
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (r, _))| *r)
                    .map(|(i, _)| i)
                else {
                    return false;
                };
                jobs.remove(at).unwrap()
            };
            if let Some(hold) = &self.hold {
                let (open, cv) = &**hold;
                let mut open = lock(open);
                while !*open {
                    open = cv.wait(open).unwrap();
                }
            }
            let _ = self.ran.send(job.1);
            true
        }
    }
    fn fixed(jobs: &[(Rank, &'static str)], ran: mpsc::Sender<&'static str>) -> Arc<Fixed> {
        Arc::new(Fixed {
            jobs: Mutex::new(jobs.iter().copied().collect()),
            ran,
            hold: None,
        })
    }
    fn recv_n(rx: &mpsc::Receiver<&'static str>, n: usize) -> Vec<&'static str> {
        (0..n)
            .map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap())
            .collect()
    }

    #[test]
    fn one_worker_runs_every_source_in_rank_order() {
        let pool = Pool::new(1, "test-job");
        let (tx, rx) = mpsc::channel();
        // Held until both sources are in, so the order is the pool's choice.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let first = Arc::new(Fixed {
            jobs: Mutex::new([(Rank::new(Class::Urgent, i64::MIN), "gate")].into()),
            ran: tx.clone(),
            hold: Some(gate.clone()),
        });
        let _g = pool.register(first);
        let a = fixed(
            &[
                (Rank::new(Class::Near, 5), "near5"),
                (Rank::new(Class::Far, 0), "far0"),
            ],
            tx.clone(),
        );
        let b = fixed(
            &[
                (Rank::new(Class::Near, 1), "near1"),
                (Rank::new(Class::Urgent, 9), "urgent9"),
            ],
            tx,
        );
        let _a = pool.register(a);
        let _b = pool.register(b);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert_eq!(
            recv_n(&rx, 5),
            ["gate", "urgent9", "near1", "near5", "far0"]
        );
    }

    #[test]
    fn far_work_leaves_a_worker_for_near_work() {
        let pool = Pool::new(3, "test-job");
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let far = Arc::new(Fixed {
            jobs: Mutex::new((0..6).map(|i| (Rank::new(Class::Far, i), "far")).collect()),
            ran: tx.clone(),
            hold: Some(gate.clone()),
        });
        let _far = pool.register(far);
        // Two far jobs hold two workers; the third stays free for this one.
        std::thread::sleep(Duration::from_millis(100));
        let near = fixed(&[(Rank::new(Class::Near, 0), "near")], tx);
        let near_reg = pool.register(near);
        near_reg.notify();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), "near");
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert_eq!(recv_n(&rx, 6), ["far"; 6]);
    }

    /// A source that counts its jobs, queued from outside while the workers run.
    struct Counter {
        left: Mutex<u64>,
        done: AtomicUsize,
        per_worker: PerWorker<u64>,
        idle: AtomicBool,
        all_idle: AtomicBool,
    }
    impl Source for Counter {
        fn next(&self) -> Option<Rank> {
            (*lock(&self.left) > 0).then_some(Rank::new(Class::Near, 0))
        }
        fn run(&self, worker: usize) -> bool {
            {
                let mut left = lock(&self.left);
                if *left == 0 {
                    return false;
                }
                *left -= 1;
            }
            self.per_worker.with(worker, || 0, |n| *n += 1);
            self.done.fetch_add(1, Ordering::Relaxed);
            true
        }
        fn idle(&self, worker: usize) {
            self.per_worker.clear(worker);
            self.idle.store(true, Ordering::Relaxed);
        }
        fn all_idle(&self) {
            self.all_idle.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn no_job_is_missed_when_queued_while_workers_look() {
        let pool = Pool::new(4, "test-job");
        let counter = Arc::new(Counter {
            left: Mutex::new(0),
            done: AtomicUsize::new(0),
            per_worker: PerWorker::new(pool.threads()),
            idle: AtomicBool::new(false),
            all_idle: AtomicBool::new(false),
        });
        let reg = pool.register(counter.clone());
        for _ in 0..20_000 {
            *lock(&counter.left) += 1;
            reg.notify();
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while counter.done.load(Ordering::Relaxed) < 20_000 {
            assert!(Instant::now() < deadline, "jobs left behind");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn idle_workers_free_their_state() {
        let release = Duration::from_millis(300);
        let pool = Pool::with_idle_release(2, "test-job", release);
        let counter = Arc::new(Counter {
            left: Mutex::new(10),
            done: AtomicUsize::new(0),
            per_worker: PerWorker::new(pool.threads()),
            idle: AtomicBool::new(false),
            all_idle: AtomicBool::new(false),
        });
        let reg = pool.register(counter.clone());
        reg.notify_all();
        let deadline = Instant::now() + release * 20;
        while !counter.all_idle.load(Ordering::Relaxed) {
            assert!(Instant::now() < deadline, "workers never released");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(counter.idle.load(Ordering::Relaxed));
        assert_eq!(counter.done.load(Ordering::Relaxed), 10);
        assert!(counter.per_worker.slots.iter().all(|s| lock(s).is_none()));
    }

    #[test]
    fn dropping_a_registration_waits_for_its_running_job() {
        let pool = Pool::new(2, "test-job");
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (tx, rx) = mpsc::channel();
        let held = Arc::new(Fixed {
            jobs: Mutex::new(
                [
                    (Rank::new(Class::Near, 0), "held"),
                    (Rank::new(Class::Near, 1), "second"),
                ]
                .into(),
            ),
            ran: tx,
            hold: Some(gate.clone()),
        });
        let reg = pool.register(held.clone());
        std::thread::sleep(Duration::from_millis(100));
        let opener = {
            let gate = gate.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                *gate.0.lock().unwrap() = true;
                gate.1.notify_all();
            })
        };
        let began = Instant::now();
        drop(reg);
        // Both jobs may have started (two workers); none runs after the drop.
        assert!(began.elapsed() >= Duration::from_millis(150));
        opener.join().unwrap();
        let ran: Vec<_> = rx.try_iter().collect();
        assert!(ran.contains(&"held"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(rx.try_iter().next().is_none());
        // The pool's own references are gone.
        assert_eq!(Arc::strong_count(&held), 1);
    }

    #[test]
    fn a_panicking_job_keeps_its_worker() {
        struct Boom(AtomicUsize);
        impl Source for Boom {
            fn next(&self) -> Option<Rank> {
                (self.0.load(Ordering::Relaxed) < 2).then_some(Rank::new(Class::Near, 0))
            }
            fn run(&self, _worker: usize) -> bool {
                if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                    panic!("first job fails");
                }
                true
            }
        }
        let pool = Pool::new(1, "test-job");
        let boom = Arc::new(Boom(AtomicUsize::new(0)));
        let reg = pool.register(boom.clone());
        reg.notify();
        let deadline = Instant::now() + Duration::from_secs(10);
        while boom.0.load(Ordering::Relaxed) < 2 {
            assert!(Instant::now() < deadline, "the worker died with the panic");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
