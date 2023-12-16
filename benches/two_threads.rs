use std::convert::TryInto as _;
use std::sync::{Arc, Barrier};

use criterion::{black_box, criterion_group, criterion_main};
use criterion::{AxisScale, PlotConfiguration};

use rtrb::RingBuffer;

pub fn add_function<P, C, Create, Push, Pop, M>(
    group: &mut criterion::BenchmarkGroup<M>,
    id: &str,
    create: Create,
    push: Push,
    pop: Pop,
) where
    P: Send + 'static,
    C: Send + 'static,
    Create: Fn(usize) -> (P, C),
    Push: Fn(&mut P, u8) -> bool + Send + Copy + 'static,
    Pop: Fn(&mut C) -> Option<u8> + Send + 'static,
    M: criterion::measurement::Measurement<Value = std::time::Duration>,
{
    // Just a quick check if the ring buffer works as expected:
    let (mut p, mut c) = create(2);
    assert!(pop(&mut c).is_none());
    assert!(push(&mut p, 1));
    assert!(push(&mut p, 2));
    assert!(!push(&mut p, 3));
    assert_eq!(pop(&mut c).unwrap(), 1);
    assert_eq!(pop(&mut c).unwrap(), 2);
    assert!(pop(&mut c).is_none());

    group.throughput(criterion::Throughput::Bytes(1));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    group.bench_function(["large", id].concat(), |b| {
        b.iter_custom(|iters| {
            // Queue is so long that there is no contention between threads.
            let (mut p, mut c) = create((2 * iters).try_into().unwrap());
            for _ in 0..iters {
                push(&mut p, 42);
            }
            let barrier = Arc::new(Barrier::new(2));
            let push_thread = {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..iters {
                        push(&mut p, black_box(42));
                    }
                    barrier.wait();
                })
            };
            barrier.wait();
            let start = std::time::Instant::now();
            for _ in 0..iters {
                black_box(pop(&mut c));
            }
            barrier.wait();
            let duration = start.elapsed();
            push_thread.join().unwrap();
            duration
        });
    });

    group.bench_function(["small", id].concat(), |b| {
        b.iter_custom(|iters| {
            // Queue is very short in order to force a lot of contention between threads.
            let (mut p, mut c) = create(2);
            let barrier = Arc::new(Barrier::new(2));
            let push_thread = {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..iters {
                        while !push(&mut p, black_box(42)) {
                            std::hint::spin_loop();
                        }
                    }
                    barrier.wait();
                })
            };
            barrier.wait();
            let start = std::time::Instant::now();
            for _ in 0..iters {
                while pop(&mut c).is_none() {
                    std::hint::spin_loop();
                }
            }
            barrier.wait();
            let duration = start.elapsed();
            push_thread.join().unwrap();
            duration
        });
    });
}

fn criterion_benchmark(criterion: &mut criterion::Criterion) {
    let mut group = criterion.benchmark_group("two-threads");
    add_function(
        &mut group,
        "",
        RingBuffer::new,
        |p, i| p.push(i).is_ok(),
        |c| c.pop().ok(),
    );
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
