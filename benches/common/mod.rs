//! Common helper functions for benchmarks.

#![allow(clippy::incompatible_msrv)]

use std::{
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
    time::{Duration, Instant},
};

#[allow(dead_code, clippy::undocumented_unsafe_blocks)]
#[path = "../../src/cache_padded.rs"]
mod cache_padded;
use cache_padded::CachePadded;

// Check whether the other thread has finished after this number of steps.
const CHECK_INTERVAL: u32 = 1000;

// Time to spin before measurement starts.
// This makes sure a CPU core has fully "woken up" after being idle.
const WARM_UP: Duration = Duration::from_millis(1);

// For synchronizing start and stop of processing.
struct Synchro {
    // Number of threads (up to 2) that are ready for processing.
    ready: AtomicU8,
    // Either thread can set this to make the other stop.
    stop_requested: AtomicBool,
}

// `step` is called repeatedly, passing the items processed so far,
// until `iters` items have been processed or `stop_requested` has been set.
//
// Before starting, it spins until the other thread is ready
// and then spins some more for WARM_UP.
//
// Returns start/top time and number of processed items.
fn repeat_step(
    iters: u64,
    mut step: impl FnMut(u64) -> u64,
    synchro: &Synchro,
) -> (Instant, Instant, u64) {
    synchro.ready.fetch_add(1, Ordering::AcqRel);
    while synchro.ready.load(Ordering::Acquire) < 2 {
        std::hint::spin_loop();
    }
    let warmed_up = Instant::now() + WARM_UP;
    while Instant::now() < warmed_up {
        std::hint::spin_loop();
    }
    let start = Instant::now();
    let mut processed_items = 0;
    'outer: loop {
        for _ in 0..CHECK_INTERVAL {
            if processed_items >= iters {
                break 'outer;
            }
            processed_items += step(processed_items)
        }
        if synchro.stop_requested.load(Ordering::Relaxed) {
            break;
        }
    }
    let end = Instant::now();
    if processed_items >= iters {
        synchro.stop_requested.store(true, Ordering::Relaxed);
    }
    (start, end, processed_items)
}

// Repeatedly runs `spawned_step` (on a new thread) and `current_step` (on the current/main thread).
//
// When one thread has processed `iters` items, the other one is stopped,
// so that during (almost) all of the measured time both threads are processing
// and therefore keeping the atomic variables in contention.
//
// `spawned_step` is not allowed to borrow variables from the main thread to avoid false sharing
// (if relevant variables on the main thread's stack end up in the same cache line).
//
// Returns the (extrapolated) time the slower thread would have needed to process `iters` elements.
pub fn measure_concurrently(
    iters: u64,
    spawned_step: impl FnMut(u64) -> u64 + Send + 'static,
    current_step: impl FnMut(u64) -> u64,
) -> Duration {
    if let Ok(n) = std::thread::available_parallelism() {
        assert!(n.get() >= 2, "at least 2 CPU cores are needed");
    }
    // CachePadded makes sure this doesn't share a cache line with any other stack variable.
    let synchro = CachePadded::new(Synchro {
        ready: AtomicU8::new(0),
        stop_requested: AtomicBool::new(false),
    });
    let (spawned_result, current_result) = std::thread::scope(|s| {
        let spawned_thread = s.spawn(|| repeat_step(iters, spawned_step, &synchro));
        let current_result = repeat_step(iters, current_step, &synchro);
        (spawned_thread.join().unwrap(), current_result)
    });

    let extrapolated_time = |(start, end, processed): (Instant, Instant, u64)| {
        if processed == 0 {
            // This can only happen with a very small number of iterations,
            // when one thread finished before the other one has even started.
            Duration::ZERO
        } else {
            (end - start).mul_f64(iters as f64 / processed as f64)
        }
    };
    extrapolated_time(spawned_result).max(extrapolated_time(current_result))
}
