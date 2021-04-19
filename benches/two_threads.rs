use std::sync::{Arc, Barrier};

use criterion::{black_box, criterion_group, criterion_main};
use criterion::{AxisScale, PlotConfiguration};

use rtrb::RingBuffer;

pub fn criterion_benchmark(criterion: &mut criterion::Criterion) {
    let mut group = criterion.benchmark_group("two-threads");
    group.throughput(criterion::Throughput::Bytes(1));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    group.bench_function("large", |b| {
        b.iter_custom(|iters| {
            // Queue is so long that there is no contention between threads.
            let (mut p, mut c) = RingBuffer::<u8>::new(2 * iters as usize).split();
            for _ in 0..iters {
                p.push(42).unwrap();
            }
            let barrier = Arc::new(Barrier::new(2));
            let barrier_in_thread = Arc::clone(&barrier);
            let push_thread = std::thread::spawn(move || {
                barrier_in_thread.wait();
                for _ in 0..iters {
                    p.push(black_box(42)).unwrap();
                }
                barrier_in_thread.wait();
            });
            barrier.wait();
            let start = std::time::Instant::now();
            for _ in 0..iters {
                black_box(c.pop().unwrap());
            }
            barrier.wait();
            let duration = start.elapsed();
            push_thread.join().unwrap();
            duration
        });
    });

    group.bench_function("small", |b| {
        b.iter_custom(|iters| {
            // Queue is very short in order to force a lot of contention between threads.
            let (mut p, mut c) = RingBuffer::<u8>::new(2).split();
            let barrier = Arc::new(Barrier::new(2));
            let barrier_in_thread = Arc::clone(&barrier);
            let push_thread = std::thread::spawn(move || {
                barrier_in_thread.wait();
                for _ in 0..iters {
                    while p.push(black_box(42)).is_err() {}
                }
                barrier_in_thread.wait();
            });
            barrier.wait();
            let start = std::time::Instant::now();
            for _ in 0..iters {
                while c.pop().is_err() {}
            }
            barrier.wait();
            let duration = start.elapsed();
            push_thread.join().unwrap();
            duration
        });
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
