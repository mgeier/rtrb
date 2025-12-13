macro_rules! create_two_threads_const_benchmark {
    ($($id:literal, $create:tt, $push:expr, $pop:expr,::)+) => {

use std::hint::black_box;
use criterion::{criterion_group, criterion_main, BenchmarkId};

fn help_with_type_inference<P, C, Push, Pop>(_: &P, _: &C, push: Push, pop: Pop) -> (Push, Pop)
where
    Push: Fn(&mut P, u8) -> bool,
    Pop: Fn(&mut C) -> Option<u8>,
{
    (push, pop)
}

#[allow(unused)]
fn criterion_benchmark(criterion: &mut criterion::Criterion) {
$(
    macro_rules! create $create
    let (mut p, mut c) = create!(8);
    let (push, pop) = help_with_type_inference(&p, &c, $push, $pop);
    // Just a quick check if the ring buffer works as expected:
    assert!(pop(&mut c).is_none());
    assert!(push(&mut p, 1));
    assert!(push(&mut p, 2));
    assert_eq!(pop(&mut c).unwrap(), 1);
    assert_eq!(pop(&mut c).unwrap(), 2);
    // NB: wrap-around differs between implementations (N vs. N-1 elements)
)+

    let mut group_eager_pop = criterion.benchmark_group("const-eager-pop");
    group_eager_pop.plot_config(criterion::PlotConfiguration::default()
        .summary_scale(criterion::AxisScale::Logarithmic));

$(
    macro_rules! add_bench {
        ($N:expr) => {
            group_eager_pop.bench_with_input(BenchmarkId::new($id, $N), &$N, |b, _| b.iter_custom(|iters| {
                macro_rules! create $create
                let (mut p0, mut c0) = create!($N);
                let (mut p1, mut c1) = create!($N);
                let (push, pop) = help_with_type_inference(&p0, &c0, $push, $pop);

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
                    // Try to push a single item ...
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
                    if push(&mut p0, black_box(sent_i as u8)) {
                        sent_i += 1;
                    }
                    // ... then eagerly pop as many items as possible:
                    #[allow(clippy::incompatible_msrv)] // stable since 1.49
                    while let Some(x) = black_box(pop(&mut c1)) {
                        assert_eq!(x, expected_i as u8);
                        expected_i += 1;
                    }
                }

                let stop = std::time::Instant::now();
                echo_thread.join().unwrap();
                stop.duration_since(start)
            }));
        };
    }

    add_bench!(2);
    add_bench!(4);
    add_bench!(8);
    add_bench!(16);
    add_bench!(32);
    add_bench!(64);
    add_bench!(128);
    add_bench!(256);
    add_bench!(512);
    add_bench!(1024);
    add_bench!(2048);
    add_bench!(4096);
    add_bench!(8192);
    add_bench!(16384);
    add_bench!(32768);
)+

    group_eager_pop.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

    };
}

create_two_threads_const_benchmark! {
    "rtrb",
    { ($N:expr) => {
        rtrb::RingBuffer::new($N)
    }},
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok(),
    ::
}
