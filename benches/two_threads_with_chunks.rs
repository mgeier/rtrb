#![allow(clippy::incompatible_msrv)]

macro_rules! create_two_threads_with_chunks_benchmark {
    ($($id:literal, $create:expr, $push:expr, $pop:expr, ::)+) => {

use std::convert::TryFrom as _;
use std::convert::TryInto as _;
use std::hint::black_box;
use std::sync::Barrier;

use criterion::{BenchmarkId, criterion_group, criterion_main};

fn help_with_type_inference<P, C, Create, Push, Pop>(create: Create, push: Push, pop: Pop) -> (Create, Push, Pop)
where
    Create: Fn(usize) -> (P, C),
    Push: for<'a> Fn(&mut P, &'a [u8]) -> &'a [u8],
    Pop: for<'a> Fn(&mut C, &'a mut [u8]) -> &'a [u8],
{
    (create, push, pop)
}

#[allow(unused)]
fn criterion_benchmark(criterion: &mut criterion::Criterion) {
$(
    let (create, push, pop) = help_with_type_inference($create, $push, $pop);
    // Just a quick check if the ring buffer works as expected:
    let (mut p, mut c) = create(4);
    let mut dst = [0u8; 3];
    assert!(pop(&mut c, &mut dst).is_empty());
    assert!(push(&mut p, &[10, 20, 30]).is_empty());
    assert_eq!(push(&mut p, &[40, 50, 60]), &[50, 60]);
    assert_eq!(push(&mut p, &[70, 80, 90]), &[70, 80, 90]);
    assert_eq!(pop(&mut c, &mut dst), &[10, 20, 30]);
    assert_eq!(pop(&mut c, &mut dst), &[40]);
    assert!(pop(&mut c, &mut dst).is_empty());
    assert_eq!(dst, [40, 20, 30]);
)+

    let mut group_large = criterion.benchmark_group("large-with-chunks");
$(
    group_large.bench_function($id, |b| {
        b.iter_custom(|iters| {
            if iters < 3 {
                return std::time::Duration::ZERO;
            }
            let (create, push, pop) = help_with_type_inference($create, $push, $pop);
            // Queue is so long that there is no contention between threads.
            let (mut p, mut c) = create((3 * iters).try_into().unwrap());
            for i in 0..iters {
                let buffer = [i as u8];
                push(&mut p, &buffer);
            }
            let barrier = Barrier::new(3);
            #[allow(clippy::incompatible_msrv)] // stable since 1.63
            std::thread::scope(|s| {
                let push_thread = s.spawn(|| {
                    barrier.wait();
                    let start_pushing = std::time::Instant::now();
                    let mut sent_i = iters;
                    while sent_i < 2 * iters {
                        let buffer: [u8; 3] = std::array::from_fn(|x| (sent_i + x as u64) as u8);
                        let remainder = black_box(push(&mut p, black_box(&buffer)));
                        assert!(remainder.is_empty());
                        for i in buffer {
                            // This is to compensate for the assertions in the main thread:
                            assert_eq!(i, black_box(sent_i as u8));
                            sent_i += 1;
                        }
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
                let mut expected_i = 0;
                while expected_i < iters {
                        let mut buffer = [0u8; 3];
                        let popped = black_box(pop(&mut c, black_box(&mut buffer)));
                        for i in popped {
                            assert_eq!(*i, expected_i as u8);
                            expected_i += 1;
                        }
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

    let queue_sizes = [4096];

    let mut group_write_chunk = criterion.benchmark_group("eager-write-chunk");
    group_write_chunk.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
$(
    for i in queue_sizes {
        group_write_chunk.bench_with_input(BenchmarkId::new($id, i), &i, |b, &queue_size| {
            b.iter_custom(|iters| {
                let iters = usize::try_from(iters).unwrap();
                let (create, push, pop) = help_with_type_inference($create, $push, $pop);
                let (mut p0, mut c0) = create(queue_size);
                let (mut p1, mut c1) = create(queue_size);

                let echo_thread = std::thread::spawn(move || {
                    let mut counter = 0;
                    while counter < iters {
                        let mut buffer = [0u8; 3];
                        let mut to_push = loop {
                            if let popped @ [_, ..] = pop(&mut c0, black_box(&mut buffer)) {
                                break popped;
                            }
                            std::hint::spin_loop();
                        };
                        counter += to_push.len();
                        while let remainder @ [_, ..] = push(&mut p1, black_box(to_push)) {
                            std::hint::spin_loop();
                            to_push = remainder;
                        }
                    }
                });

                // fill both queues (if there is enough data) before starting the measurement:
                let mut sent_i = 0;
                while (sent_i < iters) && (sent_i < 2 * queue_size) {
                    let buffer: [u8; 3] = std::array::from_fn(|x| (sent_i + x) as u8);
                    let remainder = push(&mut p0, &buffer);
                    sent_i += 3 - remainder.len();
                }

                let start = std::time::Instant::now();

                let mut expected_i = 0;
                while expected_i < iters {
                    // eagerly push as many slices as possible ...
                    loop {
                        let buffer: [u8; 3] = std::array::from_fn(|x| (sent_i + x) as u8);
                        let remainder = push(&mut p0, black_box(&buffer));
                        let pushed = 3 - remainder.len();
                        if pushed == 0 {
                            break;
                        }
                        sent_i += pushed;
                    }
                    // ... then try to pop a single slice:
                    let mut buffer = [0u8; 3];
                    let popped = pop(&mut c1, black_box(&mut buffer));
                    for i in popped {
                        assert_eq!(*i, expected_i as u8);
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
    group_write_chunk.finish();

    let mut group_read_chunk = criterion.benchmark_group("eager-read-chunk");
    group_read_chunk.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));
$(
    for i in queue_sizes {
        group_read_chunk.bench_with_input(BenchmarkId::new($id, i), &i, |b, &queue_size| {
            b.iter_custom(|iters| {
                let (create, push, pop) = help_with_type_inference($create, $push, $pop);
                let (mut p0, mut c0) = create(queue_size);
                let (mut p1, mut c1) = create(queue_size);

                let echo_thread = std::thread::spawn(move || {
                    let mut counter = 0;
                    while counter < iters {
                        let mut buffer = [0u8; 3];
                        let mut to_push = loop {
                            if let popped @ [_, ..] = pop(&mut c0, black_box(&mut buffer)) {
                                break popped;
                            }
                            std::hint::spin_loop();
                        };
                        counter += to_push.len() as u64;
                        while let remainder @ [_, ..] = push(&mut p1, black_box(to_push)) {
                            std::hint::spin_loop();
                            to_push = remainder;
                        }
                    }
                });

                let start = std::time::Instant::now();

                let mut sent_i = 0;
                let mut expected_i = 0;
                while expected_i < usize::try_from(iters).unwrap() {
                    let mut buffer = [0u8; 3];
                    // eagerly pop as many slices as possible ...
                    while let popped @ [_, ..] = pop(&mut c1, black_box(&mut buffer)) {
                        for i in popped {
                            assert_eq!(*i, expected_i as u8);
                            expected_i += 1;
                        }
                    }
                    // ... then try to push a single slice:
                    let buffer: [u8; 3] = std::array::from_fn(|x| (sent_i + x) as u8);
                    let remainder = push(&mut p0, black_box(&buffer));
                    sent_i += 3 - remainder.len();
                }

                let stop = std::time::Instant::now();
                echo_thread.join().unwrap();
                stop.duration_since(start)
            });
        });
    }
)+
    group_read_chunk.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

    };
}

use std::io::{Read as _, Write as _};

create_two_threads_with_chunks_benchmark!(
    "write-read",
    rtrb::RingBuffer::new,
    |p, s| match p.write(s) {
        Ok(n) => &s[n..],
        _ => s,
    },
    |c, s| match c.read(s) {
        Ok(n) => &s[..n],
        _ => &[],
    },
    ::
    "push_slice-pop_slice",
    rtrb::RingBuffer::new,
    |p, s| p.push_slice(s).1,
    |c, s| c.pop_slice(s).0,
    ::
);
