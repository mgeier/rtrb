use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main};
use criterion::{AxisScale, PlotConfiguration};

use rtrb::RingBuffer;

pub fn criterion_benchmark(criterion: &mut criterion::Criterion) {
    let mut group = criterion.benchmark_group("two-threads-single-byte");
    group.throughput(criterion::Throughput::Bytes(1));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    let mut overflows = 0;
    group.bench_function("push", |b| {
        let mut i = 0;

        let (mut p, mut c) = RingBuffer::<u8>::new(10_000_000).split();

        let keep_thread_running = Arc::new(AtomicBool::new(true));
        let keep_running = Arc::clone(&keep_thread_running);

        let pop_thread = std::thread::spawn(move || {
            while keep_running.load(Ordering::Acquire) {
                while let Ok(x) = c.pop() {
                    debug_assert_eq!(x, i);
                    i = i.wrapping_add(1);
                }
            }
        });

        let mut j = 0;

        b.iter(|| {
            if p.push(black_box(j)).is_ok() {
                j = j.wrapping_add(1);
            } else {
                overflows += 1;
                std::thread::yield_now();
            }
        });

        keep_thread_running.store(false, Ordering::Release);
        pop_thread.join().unwrap();
    });
    println!("queue was full {} time(s)", overflows);

    let mut underflows = 0;
    group.bench_function("pop", |b| {
        let mut i = 0;

        let (mut p, mut c) = RingBuffer::<u8>::new(10_000_000).split();

        while p.push(i).is_ok() {
            i = i.wrapping_add(1);
        }

        let keep_thread_running = Arc::new(AtomicBool::new(true));
        let keep_running = Arc::clone(&keep_thread_running);

        let push_thread = std::thread::spawn(move || {
            while keep_running.load(Ordering::Acquire) {
                while p.push(i).is_ok() {
                    i = i.wrapping_add(1);
                }
            }
        });

        let mut j = 0;

        b.iter(|| {
            if let Ok(x) = black_box(c.pop()) {
                debug_assert_eq!(x, j);
                j = j.wrapping_add(1);
            } else {
                underflows += 1;
                std::thread::yield_now();
            }
        });
        keep_thread_running.store(false, Ordering::Release);
        push_thread.join().unwrap();
    });
    println!("queue was empty {} time(s)", underflows);

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
