#![allow(clippy::incompatible_msrv)]

macro_rules! create_two_threads_with_chunks_benchmark {
    ($($id:literal, $create:expr, $push:expr, $pop:expr, ::)+) => {

use std::convert::TryFrom as _;
use std::hint::black_box;

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
    "rtrb-write-read",
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
);
