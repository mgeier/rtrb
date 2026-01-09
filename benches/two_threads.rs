macro_rules! create_two_threads_benchmark {
    ($($id:literal, $create:expr, $push:expr, $pop:expr,::)+) => {

use std::convert::TryInto as _;
use std::hint::black_box;
use std::sync::Barrier;

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
            let barrier = Barrier::new(3);
            #[allow(clippy::incompatible_msrv)] // stable since 1.63
            std::thread::scope(|s| {
                let push_thread = s.spawn(|| {
                    barrier.wait();
                    let start_pushing = std::time::Instant::now();
                    for i in 0..iters {
                        // NB: This conversion truncates:
                        push(&mut p, i as u8);
                    }
                    let stop_pushing = std::time::Instant::now();
                    (start_pushing, stop_pushing)
                });

                let _trigger_thread = s.spawn(|| {
                    // Try to force other threads to go to sleep on barrier.
                    std::thread::yield_now();
                    std::thread::yield_now();
                    std::thread::yield_now();
                    barrier.wait();
                    // Hopefully, the other two threads now wake up at the same time.
                });

                barrier.wait();
                let start_popping = std::time::Instant::now();
                for _ in 0..iters {
                    #[allow(clippy::incompatible_msrv)]
                    black_box(pop(&mut c));
                }
                let stop_popping = std::time::Instant::now();
                let (start_pushing, stop_pushing) = push_thread.join().unwrap();
                let total = stop_pushing
                    .max(stop_popping)
                    .duration_since(start_pushing.min(start_popping));

                /*
                if start_pushing < start_popping {
                    println!(
                        "popping started {:?} after pushing",
                        start_popping.duration_since(start_pushing)
                    );
                } else {
                    println!(
                        "pushing started {:?} after popping",
                        start_pushing.duration_since(start_popping)
                    );
                }
                */

                // The goal is that both threads are finished at around the same time.
                // This can be checked with the following output.
                /*
                if stop_pushing < stop_popping {
                    let diff = stop_popping.duration_since(stop_pushing);
                    println!(
                        "popping stopped {diff:?} after pushing ({:.1}% of total time)",
                        (diff.as_secs_f64() / total.as_secs_f64()) * 100.0
                    );
                } else {
                    let diff = stop_pushing.duration_since(stop_popping);
                    println!(
                        "pushing stopped {diff:?} after popping ({:.1}% of total time)",
                        (diff.as_secs_f64() / total.as_secs_f64()) * 100.0
                    );
                }
                */

                #[allow(clippy::let_and_return)]
                total
            })
        });
    });
)+
    group_large.finish();

    let mut group_small = criterion.benchmark_group("small");
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
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
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
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
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
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
                            if let Some(x) = black_box(pop(&mut c0)) {
                                break x;
                            }
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
                            std::hint::spin_loop();
                        };
                        #[allow(clippy::incompatible_msrv)] // stable since 1.49
                        while !push(&mut p1, black_box(x)) {
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
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
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
                    while push(&mut p0, black_box(sent_i as u8)) {
                        sent_i += 1;
                    }
                    // ... then try to pop a single item:
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
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
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
                            if let Some(x) = black_box(pop(&mut c0)) {
                                break x;
                            }
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
                            std::hint::spin_loop();
                        };
                        #[allow(clippy::incompatible_msrv)] // stable since 1.49
                        while !push(&mut p1, black_box(x)) {
                            #[allow(clippy::incompatible_msrv)] // stable since 1.49
                            std::hint::spin_loop();
                        }
                    }
                });

                let start = std::time::Instant::now();

                let mut sent_i = 0;
                let mut expected_i = 0;
                while expected_i < iters {
                    // eagerly pop as many items as possible ...
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
                    while let Some(x) = black_box(pop(&mut c1)) {
                        assert_eq!(x, expected_i as u8);
                        expected_i += 1;
                    }
                    // ... then try to push a single item:
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
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

create_two_threads_benchmark!(
    "rtrb",
    rtrb::RingBuffer::new,
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok(),
    ::
);
