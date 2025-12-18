#![allow(clippy::incompatible_msrv)]

//! Microbenchmarks for checking if at least one slot is available.

use criterion::Criterion;
use criterion::{criterion_group, criterion_main, AxisScale, PlotConfiguration};

use rtrb::RingBuffer;

use std::hint::black_box;

const CAPACITY: usize = 1024;

pub fn criterion_benchmark(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("slot-checks");
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    // Producer: empty (many free slots).
    let (p_empty, _c_empty) = RingBuffer::<u8>::new(CAPACITY);
    group.bench_function("producer/has_slots(1)/empty", |b| {
        b.iter(|| black_box(p_empty.has_slots(black_box(1))))
    });
    group.bench_function("producer/slots()>=1/empty", |b| {
        b.iter(|| black_box(p_empty.slots() >= black_box(1)))
    });

    // Producer: full (no free slots).
    let (mut p_full, _c_full) = RingBuffer::<u8>::new(CAPACITY);
    for _ in 0..CAPACITY {
        p_full.push(0).unwrap();
    }
    group.bench_function("producer/has_slots(1)/full", |b| {
        b.iter(|| black_box(p_full.has_slots(black_box(1))))
    });
    group.bench_function("producer/slots()>=1/full", |b| {
        b.iter(|| black_box(p_full.slots() >= black_box(1)))
    });

    // Consumer: empty (no readable slots).
    let (_p_empty, c_empty) = RingBuffer::<u8>::new(CAPACITY);
    group.bench_function("consumer/has_slots(1)/empty", |b| {
        b.iter(|| black_box(c_empty.has_slots(black_box(1))))
    });
    group.bench_function("consumer/slots()>=1/empty", |b| {
        b.iter(|| black_box(c_empty.slots() >= black_box(1)))
    });

    // Consumer: non-empty (at least one readable slot).
    let (mut p_nonempty, c_nonempty) = RingBuffer::<u8>::new(CAPACITY);
    p_nonempty.push(0).unwrap();
    // Warm the consumer's cached tail so `has_slots(1)` can hit the fast path.
    black_box(c_nonempty.has_slots(1));
    group.bench_function("consumer/has_slots(1)/nonempty", |b| {
        b.iter(|| black_box(c_nonempty.has_slots(black_box(1))))
    });
    group.bench_function("consumer/slots()>=1/nonempty", |b| {
        b.iter(|| black_box(c_nonempty.slots() >= black_box(1)))
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
