use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use criterion::black_box;
use criterion::{criterion_group, criterion_main};
use rtrb::RingBuffer;

const MANY: usize = 2_000_000;
const SLEEPTIME: Duration = Duration::from_millis(3);

pub fn push_and_pop(criterion: &mut criterion::Criterion) {
    criterion.bench_function("push-pop Vec", |b| {
        let mut v = Vec::<u8>::with_capacity(1);
        let mut i = 0;
        b.iter(|| {
            v.push(black_box(i));
            assert_eq!(v.pop(), black_box(Some(i)));
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("push-pop RingBuffer", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            p.push(black_box(i)).unwrap();
            assert_eq!(c.pop(), black_box(Ok(i)));
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("push-pop via thread", |b| {
        let (mut p1, mut c1) = RingBuffer::<u8>::new(1).split();
        let (mut p2, mut c2) = RingBuffer::<u8>::new(1).split();
        let keep_thread_running = Arc::new(AtomicBool::new(true));
        let keep_running = Arc::clone(&keep_thread_running);

        let echo_thread = std::thread::spawn(move || {
            while keep_running.load(Ordering::Acquire) {
                if let Ok(x) = c1.pop() {
                    // NB: return channel will always be ready
                    p2.push(x).unwrap();
                }
            }
        });

        let mut i = 0;
        let result = b.iter(|| {
            while p1.push(black_box(i)).is_err() {}
            let x = loop {
                if let Ok(x) = c2.pop() {
                    break x;
                }
            };
            assert_eq!(x, black_box(i));
            i = i.wrapping_add(black_box(1));
        });

        keep_thread_running.store(false, Ordering::Release);
        echo_thread.join().unwrap();
        result
    });

    let mut group = criterion.benchmark_group("many-items");
    group.sample_size(30);
    group.sampling_mode(criterion::SamplingMode::Flat);

    group.bench_function("push-many-pop-many", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(MANY).split();
        b.iter(|| {
            for i in 0..MANY {
                p.push(black_box(wrap(i))).unwrap();
            }
            for i in 0..MANY {
                assert_eq!(c.pop(), Ok(black_box(wrap(i))));
            }
        })
    });

    group.bench_function("parallel-push-pop", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(MANY).split();

        enum Condition {
            Wait,
            Continue,
            Finish,
        }

        let pair = Arc::new((Mutex::new(Condition::Wait), Condvar::new()));
        let pair2 = pair.clone();

        let push_thread = std::thread::spawn(move || {
            let (lock, cvar) = &*pair2;
            loop {
                let mut condition = cvar
                    .wait_while(lock.lock().unwrap(), |c| matches!(*c, Condition::Wait))
                    .unwrap();
                *condition = match *condition {
                    Condition::Finish => break,
                    Condition::Continue => Condition::Wait,
                    Condition::Wait => unreachable!(),
                };
                for i in 0..MANY {
                    p.push(black_box(wrap(i))).unwrap();
                }
            }
        });

        let (lock, cvar) = &*pair;
        let result = b.iter(|| {
            *lock.lock().unwrap() = Condition::Continue;
            cvar.notify_one();
            // Wait a bit to get more consistent timing
            std::thread::sleep(SLEEPTIME);
            for i in 0..MANY {
                loop {
                    if let Ok(value) = c.pop() {
                        assert_eq!(value, black_box(wrap(i)));
                        break;
                    }
                }
            }
        });
        *lock.lock().unwrap() = Condition::Finish;
        cvar.notify_one();
        push_thread.join().unwrap();
        result
    });

    group.finish();
}

fn wrap(n: usize) -> u8 {
    (n % u8::MAX as usize) as u8
}

criterion_group!(benches, push_and_pop);
criterion_main!(benches);
