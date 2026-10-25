#![allow(clippy::incompatible_msrv)]

pub(crate) mod common;

macro_rules! create_push_pop_benchmark {
    ($($id:literal, $create:expr, $push:expr, $pop:expr,::)+) => {

use std::convert::TryInto as _;
use std::hint::black_box;

use criterion::{BenchmarkId, criterion_group, criterion_main};

fn help_with_type_inference<P, C, Create, Push, Pop>(create: Create, push: Push, pop: Pop) -> (Create, Push, Pop)
where
    Create: Fn(usize) -> (P, C),
    Push: Fn(&mut P, u8) -> bool,
    Pop: Fn(&mut C) -> Option<u8>,
{
    (create, push, pop)
}

#[allow(unused)]
fn criterion_benchmark(criterion: &mut criterion::Criterion) {
$(
    let (create, push, pop) = help_with_type_inference($create, $push, $pop);
    // Just a quick check if the ring buffer works as expected:
    let (mut p, mut c) = create(2);
    assert!(pop(&mut c).is_none());
    assert!(push(&mut p, 1));
    assert!(push(&mut p, 2));
    assert!(!push(&mut p, 3));
    assert_eq!(pop(&mut c).unwrap(), 1);
    assert_eq!(pop(&mut c).unwrap(), 2);
    assert!(pop(&mut c).is_none());
)+

    let mut group_large = criterion.benchmark_group("large");
    group_large.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
    group_large.throughput(criterion::Throughput::Bytes(1));
$(
    group_large.bench_function($id, |b| {
        b.iter_custom(|iters| {
            let (create, push, pop) = help_with_type_inference($create, $push, $pop);
            // Queue is so long that there is no contention between threads.
            let (mut p, mut c) = create((2 * iters).try_into().unwrap());
            for i in 0..iters {
                push(&mut p, i as u8);
            }
            common::measure_concurrently(
                iters,
                move |i| {
                    black_box(push(&mut p, black_box(i as u8)));
                    1
                },
                |_| {
                    black_box(pop(&mut c));
                    1
                },
            )
        });
    });
)+
    group_large.finish();

    let mut group_small = criterion.benchmark_group("small");
    group_small.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
    group_small.throughput(criterion::Throughput::Bytes(1));
$(
    group_small.bench_function($id, |b| {
        b.iter_custom(|iters| {
            let (create, push, pop) = help_with_type_inference($create, $push, $pop);
            // Queue is very short in order to force a lot of contention between threads.
            let (mut p, mut c) = create(2);
            let push_thread = {
                std::thread::spawn(move || {
                    // The timing starts once both threads are ready.
                    let start = std::time::Instant::now();
                    for i in 0..iters {
                        while !push(&mut p, i as u8) {
                            std::hint::spin_loop();
                        }
                    }
                    start
                })
            };
            // While the second thread is still starting up, this thread will busy-wait.
            for i in 0..iters {
                loop {
                    if let Some(x) = pop(&mut c) {
                        assert_eq!(x, i as u8);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
            // The timing stops once all items have been received.
            let stop = std::time::Instant::now();
            let start = push_thread.join().unwrap();
            stop.duration_since(start)
        });
    });
)+
    group_small.finish();

    let queue_sizes = [4096];

    let mut group_eager_push = criterion.benchmark_group("eager-push");
    group_eager_push.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
$(
    for i in queue_sizes {
        group_eager_push.bench_with_input(BenchmarkId::new($id, i), &i, |b, &queue_size| {
            b.iter_custom(|iters| {
                let (create, push, pop) = help_with_type_inference($create, $push, $pop);
                let (mut p0, mut c0) = create(queue_size);
                let (mut p1, mut c1) = create(queue_size);

                let echo_thread = std::thread::spawn(move || {
                    for _ in 0..iters {
                        let x = loop {
                            if let Some(x) = black_box(pop(&mut c0)) {
                                break x;
                            }
                            std::hint::spin_loop();
                        };
                        while !push(&mut p1, black_box(x)) {
                            std::hint::spin_loop();
                        }
                    }
                });

                // fill both queues (if there is enough data) before starting the measurement:
                let mut sent_i = 0;
                while (sent_i < iters) && (sent_i < 2 * queue_size as u64) {
                    if push(&mut p0, sent_i as u8) {
                        sent_i += 1;
                    }
                }

                let start = std::time::Instant::now();

                let mut expected_i = 0;
                while expected_i < iters {
                    // eagerly push as many items as possible ...
                    while push(&mut p0, black_box(sent_i as u8)) {
                        sent_i += 1;
                    }
                    // ... then try to pop a single item:
                    if let Some(x) = black_box(pop(&mut c1)) {
                        assert_eq!(x, expected_i as u8);
                        expected_i += 1;
                    }
                }

                let stop = std::time::Instant::now();
                echo_thread.join().unwrap();
                stop.duration_since(start)
            });
        });
    }
)+
    group_eager_push.finish();

    let mut group_eager_pop = criterion.benchmark_group("eager-pop");
    group_eager_pop.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
$(
    for i in queue_sizes {
        group_eager_pop.bench_with_input(BenchmarkId::new($id, i), &i, |b, &queue_size| {
            b.iter_custom(|iters| {
                let (create, push, pop) = help_with_type_inference($create, $push, $pop);
                let (mut p0, mut c0) = create(queue_size);
                let (mut p1, mut c1) = create(queue_size);

                let echo_thread = std::thread::spawn(move || {
                    for _ in 0..iters {
                        let x = loop {
                            if let Some(x) = black_box(pop(&mut c0)) {
                                break x;
                            }
                            std::hint::spin_loop();
                        };
                        while !push(&mut p1, black_box(x)) {
                            std::hint::spin_loop();
                        }
                    }
                });

                let start = std::time::Instant::now();

                let mut sent_i = 0;
                let mut expected_i = 0;
                while expected_i < iters {
                    // eagerly pop as many items as possible ...
                    while let Some(x) = black_box(pop(&mut c1)) {
                        assert_eq!(x, expected_i as u8);
                        expected_i += 1;
                    }
                    // ... then try to push a single item:
                    if push(&mut p0, black_box(sent_i as u8)) {
                        sent_i += 1;
                    }
                }

                let stop = std::time::Instant::now();
                echo_thread.join().unwrap();
                stop.duration_since(start)
            });
        });
    }
)+
    group_eager_pop.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

    };
}

create_push_pop_benchmark!(
    "rtrb",
    rtrb::RingBuffer::new,
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok(),
    ::
);
