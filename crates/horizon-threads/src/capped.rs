//! A global allocator with a hard ceiling on what the game may hold.
//!
//! [`crate::memory_limit`] caps the working set, which Windows enforces by moving
//! pages out of RAM: the game keeps its allocations and runs slower. This is the
//! other contract, the one `-Xmx` gives a Java game: past the ceiling an
//! allocation fails, so nothing runs the game past it. Freed memory comes back
//! to the budget at once, and a ceiling set from the launcher is in force before
//! the game allocates anything.
//!
//! What it counts is the bytes the game asked for. The allocator's own pages,
//! per-thread heaps and rounding sit on top, so the process stays a little
//! above the ceiling; a ceiling far below what the game needs ends it with a
//! message instead of paging, which is what `-Xmx` does too.
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// No ceiling: every allocation passes and only accounting runs.
const NONE: u64 = u64::MAX;

static LIMIT: AtomicU64 = AtomicU64::new(NONE);
/// Bytes the game holds now, as it asked for them.
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static REFUSED: AtomicU64 = AtomicU64::new(0);
/// The message is written once, by the allocation that first hits the ceiling.
static REPORTED: AtomicBool = AtomicBool::new(false);

/// What the allocator holds, for `/ramlimit` and the crash report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub live: u64,
    pub peak: u64,
    pub limit: Option<u64>,
    pub refused: u64,
}

/// Sets the ceiling in bytes, or removes it.
///
/// Counting costs a relaxed load per allocation and nothing else while no
/// ceiling is set, so a game that never asks for one pays no atomic.
/// Turning one on mid-run seeds the counter with what the process already
/// committed, so the ceiling starts from what the game holds rather than from
/// zero; the allocator's own pages are part of that seed, which makes the
/// ceiling a little stricter than the bytes the game asked for.
pub fn set_limit(bytes: Option<u64>) {
    apply(bytes, crate::allocator_commit().0 as u64);
}

/// [`set_limit`], with the counter seeded by `held` instead of the process's
/// commit: the tests count from zero, a game turning it on counts from what it
/// already holds.
fn apply(bytes: Option<u64>, held: u64) {
    match bytes {
        Some(bytes) => {
            LIVE.store(held, Ordering::Relaxed);
            PEAK.store(held, Ordering::Relaxed);
            REFUSED.store(0, Ordering::Relaxed);
            // Last: no allocation is counted before the seed is in place.
            LIMIT.store(bytes, Ordering::Relaxed);
        }
        None => {
            LIMIT.store(NONE, Ordering::Relaxed);
            LIVE.store(0, Ordering::Relaxed);
            PEAK.store(0, Ordering::Relaxed);
        }
    }
    REPORTED.store(false, Ordering::Relaxed);
}

/// The ceiling in bytes, if one is set.
pub fn limit() -> Option<u64> {
    Some(LIMIT.load(Ordering::Relaxed)).filter(|limit| *limit != NONE)
}

/// The bytes the game holds now.
pub fn live_bytes() -> u64 {
    LIVE.load(Ordering::Relaxed)
}

/// The most the game has held at once.
pub fn peak_bytes() -> u64 {
    PEAK.load(Ordering::Relaxed)
}

/// How many allocations the ceiling turned away.
pub fn refused() -> u64 {
    REFUSED.load(Ordering::Relaxed)
}

/// All three at once, without reading an atomic three times.
pub fn usage() -> Usage {
    Usage {
        live: live_bytes(),
        peak: peak_bytes(),
        limit: limit(),
        refused: refused(),
    }
}

/// An allocator behind a hard ceiling: [`Allocator`] is this over mimalloc.
pub struct Capped<A>(pub A);

impl<A> Capped<A> {
    pub const fn new(inner: A) -> Self {
        Capped(inner)
    }
}

/// The game's allocator, over mimalloc (`#[global_allocator]` in its `main.rs`).
pub type Allocator = Capped<mimalloc::MiMalloc>;
/// A const instance, so `main.rs` can name it in a `static`.
pub const ALLOCATOR: Allocator = Capped(mimalloc::MiMalloc);

// SAFETY: every method forwards to the wrapped allocator with the same layout
// and pointer, and only decides whether to pass the request on at all.
unsafe impl<A: GlobalAlloc> GlobalAlloc for Capped<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's layout, and the result is handed back to it.
        let ptr = unsafe { self.0.alloc(layout) };
        if ptr.is_null() {
            release(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !reserve(layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's layout, and the result is handed back to it.
        let ptr = unsafe { self.0.alloc_zeroed(layout) };
        if ptr.is_null() {
            release(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller's pointer and layout, passed on unchanged.
        unsafe { self.0.dealloc(ptr, layout) };
        release(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let grew = new_size > layout.size();
        if grew && !reserve(new_size - layout.size()) {
            return std::ptr::null_mut();
        }
        // SAFETY: the caller's pointer, layout and new size, passed on unchanged.
        let out = unsafe { self.0.realloc(ptr, layout, new_size) };
        if out.is_null() {
            // The old block is still the game's: give back only what was reserved.
            if grew {
                release(new_size - layout.size());
            }
        } else if !grew {
            release(layout.size() - new_size);
        }
        out
    }
}

/// Takes `bytes` from the budget; false when that would pass the ceiling.
#[inline]
fn reserve(bytes: usize) -> bool {
    let bytes = bytes as u64;
    if bytes == 0 {
        return true;
    }
    // One relaxed load while no ceiling is set: no shared line touched, so a
    // game without one allocates exactly as it did before this module existed.
    let limit = LIMIT.load(Ordering::Relaxed);
    if limit == NONE {
        return true;
    }
    let mut live = LIVE.load(Ordering::Relaxed);
    loop {
        let next = live.saturating_add(bytes);
        if next > limit {
            REFUSED.fetch_add(1, Ordering::Relaxed);
            report(limit, live, bytes);
            return false;
        }
        match LIVE.compare_exchange_weak(live, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                bump_peak(next);
                return true;
            }
            Err(actual) => live = actual,
        }
    }
}

/// Gives `bytes` back to the budget.
#[inline]
fn release(bytes: usize) {
    let bytes = bytes as u64;
    if bytes == 0 || LIMIT.load(Ordering::Relaxed) == NONE {
        return;
    }
    // A free without its allocation would wrap the counter and refuse every
    // later allocation; that can only come from a broken allocator contract.
    if LIVE.fetch_sub(bytes, Ordering::Relaxed) < bytes {
        LIVE.store(0, Ordering::Relaxed);
    }
}

#[inline]
fn bump_peak(live: u64) {
    if live > PEAK.load(Ordering::Relaxed) {
        PEAK.fetch_max(live, Ordering::Relaxed);
    }
}

/// Says, once, which ceiling ended the game. Built in a stack buffer and
/// written with no allocation, since this runs when allocating failed.
fn report(limit: u64, live: u64, wanted: u64) {
    // A test refuses on purpose; its output is not a game ending.
    if cfg!(test) || REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut buf = [0u8; 192];
    let mut at = 0;
    at += put(&mut buf[at..], b"Horizon: the hard memory limit of ");
    at += number(&mut buf[at..], limit >> 20);
    at += put(&mut buf[at..], b" MiB is reached (holding ");
    at += number(&mut buf[at..], live >> 20);
    at += put(&mut buf[at..], b" MiB, asked for ");
    at += number(&mut buf[at..], wanted);
    at += put(
        &mut buf[at..],
        b" more bytes). Raise HORIZON_MEMORY_HARD_MB.\n",
    );
    let _ = std::io::Write::write_all(&mut std::io::stderr(), &buf[..at]);
}

fn put(out: &mut [u8], text: &[u8]) -> usize {
    let len = text.len().min(out.len());
    out[..len].copy_from_slice(&text[..len]);
    len
}

fn number(out: &mut [u8], mut n: u64) -> usize {
    let mut digits = [0u8; 20];
    let mut at = digits.len();
    if n == 0 {
        at -= 1;
        digits[at] = b'0';
    }
    while n > 0 {
        at -= 1;
        digits[at] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    put(out, &digits[at..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    /// The budget is one process's: these tests take turns.
    static BUDGET: Mutex<()> = Mutex::new(());

    fn alone() -> MutexGuard<'static, ()> {
        let guard = BUDGET.lock().unwrap_or_else(|e| e.into_inner());
        LIVE.store(0, Ordering::Relaxed);
        PEAK.store(0, Ordering::Relaxed);
        REFUSED.store(0, Ordering::Relaxed);
        apply(None, 0);
        guard
    }

    #[test]
    fn a_ceiling_counts_up_and_down_with_the_allocations() {
        let _alone = alone();
        apply(Some(100), 0);
        assert!(reserve(40));
        assert!(reserve(60));
        assert_eq!(live_bytes(), 100);
        assert_eq!(peak_bytes(), 100);
        assert!(!reserve(1), "the budget is exactly used up");
        assert_eq!(refused(), 1);
        release(60);
        assert_eq!(live_bytes(), 40);
        assert!(reserve(60), "a free made room again");
        assert_eq!(live_bytes(), 100);
        assert_eq!(peak_bytes(), 100);
    }

    #[test]
    fn a_peak_stays_at_its_highest_after_the_budget_shrinks() {
        let _alone = alone();
        apply(Some(1 << 20), 0);
        assert!(reserve(900));
        assert_eq!(release_bytes(400), 500);
        assert!(reserve(200));
        assert_eq!(live_bytes(), 700);
        assert_eq!(peak_bytes(), 900);
    }

    /// `release` returns nothing; the test reads the counter after it.
    fn release_bytes(bytes: usize) -> u64 {
        release(bytes);
        live_bytes()
    }

    #[test]
    fn no_ceiling_counts_nothing_and_refuses_nothing() {
        let _alone = alone();
        assert!(reserve(1 << 30));
        assert_eq!(live_bytes(), 0, "a game without a ceiling pays no counter");
        assert!(reserve(1 << 30));
        assert_eq!(refused(), 0);
        release(1 << 31);
        assert_eq!(live_bytes(), 0);
    }

    #[test]
    fn a_free_without_its_allocation_does_not_wrap_the_budget() {
        let _alone = alone();
        apply(Some(1000), 0);
        reserve(10);
        release(500);
        assert_eq!(live_bytes(), 0, "clamped instead of wrapped");
        assert!(reserve(1000), "the whole budget is usable again");
    }

    #[test]
    fn the_allocator_itself_refuses_past_the_ceiling_and_frees_below_it() {
        let _alone = alone();
        apply(Some(4096), 0);
        let small = Layout::from_size_align(1024, 8).unwrap();
        // SAFETY: the layout is valid and the pointer is freed once, with the same layout.
        unsafe {
            let first = ALLOCATOR.alloc(small);
            assert!(!first.is_null());
            assert_eq!(live_bytes(), 1024);
            let second = ALLOCATOR.alloc(small);
            assert!(!second.is_null());
            let over = ALLOCATOR.alloc(Layout::from_size_align(4096, 8).unwrap());
            assert!(over.is_null(), "past the ceiling the request fails");
            assert_eq!(refused(), 1);
            assert_eq!(live_bytes(), 2048, "a refused request takes no budget");
            ALLOCATOR.dealloc(second, small);
            ALLOCATOR.dealloc(first, small);
            assert_eq!(live_bytes(), 0);
        }
    }

    #[test]
    fn growing_an_allocation_past_the_ceiling_keeps_the_old_one() {
        let _alone = alone();
        apply(Some(4096), 0);
        let layout = Layout::from_size_align(1024, 8).unwrap();
        // SAFETY: the layout is valid, and the pointer is freed once with the
        // layout it came back with.
        unsafe {
            let ptr = ALLOCATOR.alloc(layout);
            assert!(!ptr.is_null());
            let grown = ALLOCATOR.realloc(ptr, layout, 8192);
            assert!(grown.is_null(), "the growth would pass the ceiling");
            assert_eq!(live_bytes(), 1024, "and the old allocation still counts");
            let same = ALLOCATOR.realloc(ptr, layout, 512);
            assert!(!same.is_null());
            assert_eq!(live_bytes(), 512, "a shrink frees the difference");
            ALLOCATOR.dealloc(same, Layout::from_size_align(512, 8).unwrap());
            assert_eq!(live_bytes(), 0);
        }
    }

    #[test]
    fn the_ceiling_can_be_lifted_again() {
        let _alone = alone();
        apply(Some(64), 0);
        assert!(reserve(64));
        assert!(!reserve(1));
        apply(None, 0);
        assert!(reserve(1 << 20), "no ceiling: anything passes");
        assert_eq!(limit(), None);
        assert_eq!(live_bytes(), 0, "and nothing is counted again");
    }

    #[test]
    fn a_ceiling_turned_on_mid_run_counts_from_what_is_already_held() {
        let _alone = alone();
        apply(Some(1000), 900);
        assert_eq!(live_bytes(), 900);
        assert!(reserve(100), "the last tenth is there");
        assert!(!reserve(1), "and no more");
        release(400);
        assert_eq!(live_bytes(), 600);
        assert!(reserve(400));
    }

    #[test]
    fn the_message_names_the_ceiling_and_fits_its_buffer() {
        let mut buf = [0u8; 192];
        let mut at = 0;
        at += put(&mut buf[at..], b"limit ");
        at += number(&mut buf[at..], 4096);
        at += put(&mut buf[at..], b" MiB");
        assert_eq!(&buf[..at], b"limit 4096 MiB");
        assert_eq!(number(&mut buf, 0), 1);
        assert_eq!(&buf[..1], b"0");
    }
}
