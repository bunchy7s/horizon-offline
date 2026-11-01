//! Running independent work at once: the exporter JVMs of each side, the asset copy, hashing.
//! `HORIZON_PREPARE_JOBS=1` runs everything one step after another.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// How many threads each copying, hashing or compiling step spreads over: HORIZON_PREPARE_JOBS,
/// else the processors (at most 8). With 1 the lanes run one after another too.
pub(crate) fn jobs() -> usize {
    std::env::var("HORIZON_PREPARE_JOBS")
        .ok()
        .and_then(|n| n.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(8)
        })
}

/// `work` over every item on up to `jobs()` threads, the results in the items' order. The first
/// item that fails, in that order, gives the error; the threads stop taking items once one failed.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    work: impl Fn(&T) -> Result<R, String> + Sync,
) -> Result<Vec<R>, String> {
    map_with(jobs(), items, work)
}

fn map_with<T: Sync, R: Send>(
    jobs: usize,
    items: &[T],
    work: impl Fn(&T) -> Result<R, String> + Sync,
) -> Result<Vec<R>, String> {
    let threads = jobs.min(items.len());
    if threads <= 1 {
        return items.iter().map(work).collect();
    }
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let done = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let mut results = Vec::new();
                while !failed.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let result = work(item);
                    failed.fetch_or(result.is_err(), Ordering::Relaxed);
                    results.push((i, result));
                }
                done.lock().unwrap().extend(results);
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, result)| result).collect()
}

/// One sequence of steps that needs nothing from the others.
pub(crate) type Lane<'a, R> = Box<dyn FnOnce() -> Result<R, String> + Send + 'a>;

/// Runs the lanes at once, each to its end; the first lane's error, in the lanes' order, wins.
/// With `jobs()` 1 they run one after another and the first error stops them. A lane that
/// panics fails the preparation instead of the program.
pub(crate) fn lanes<R: Send>(lanes: Vec<Lane<'_, R>>) -> Result<Vec<R>, String> {
    lanes_with(jobs(), lanes)
}

fn lanes_with<R: Send>(jobs: usize, lanes: Vec<Lane<'_, R>>) -> Result<Vec<R>, String> {
    let stopped = |_| Err("A preparation step stopped unexpectedly".to_string());
    if jobs <= 1 {
        return lanes
            .into_iter()
            .map(|lane| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(lane)).unwrap_or_else(stopped)
            })
            .collect();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = lanes.into_iter().map(|lane| scope.spawn(lane)).collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(stopped))
            .collect();
        results.into_iter().collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_keep_the_items_order_and_the_first_failure_wins() {
        let items: Vec<u32> = (0..500).collect();
        for jobs in [1, 4] {
            let squares = map_with(jobs, &items, |n| Ok(n * n)).unwrap();
            assert_eq!(squares, items.iter().map(|n| n * n).collect::<Vec<_>>());
            let failure = map_with(jobs, &items, |n| {
                if n % 7 == 3 {
                    Err(format!("bad {n}"))
                } else {
                    Ok(*n)
                }
            });
            assert_eq!(failure, Err("bad 3".into()));
            assert_eq!(map_with(jobs, &[] as &[u32], |n| Ok(*n)), Ok(vec![]));
        }
    }

    #[test]
    fn lanes_run_at_once_to_their_end_and_the_first_error_is_reported() {
        let finished = AtomicUsize::new(0);
        let lane = |result: Result<u8, String>| -> Lane<'_, u8> {
            Box::new(|| {
                finished.fetch_add(1, Ordering::Relaxed);
                result
            })
        };
        let four = || {
            vec![
                lane(Ok(1)),
                lane(Err("second".into())),
                lane(Err("third".into())),
                lane(Ok(4)),
            ]
        };
        assert_eq!(
            lanes_with(4, vec![lane(Ok(1)), lane(Ok(2))]),
            Ok(vec![1, 2])
        );
        assert_eq!(lanes_with(4, four()), Err("second".into()));
        assert_eq!(finished.swap(0, Ordering::Relaxed), 6);
        // One after another, the first error stops the rest.
        assert_eq!(lanes_with(1, four()), Err("second".into()));
        assert_eq!(finished.load(Ordering::Relaxed), 2);
        for jobs in [1, 4] {
            let panics: Lane<'_, u8> = Box::new(|| panic!("exporter thread"));
            assert!(lanes_with(jobs, vec![lane(Ok(1)), panics]).is_err());
        }
    }
}
