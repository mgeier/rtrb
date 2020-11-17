use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use criterion::black_box;
use criterion::{criterion_group, criterion_main};
use rtrb::RingBuffer;

const MANY: usize = 2_000_000;
const SLEEPTIME: Duration = Duration::from_millis(3);

pub fn push_and_pop(criterion: &mut criterion::Criterion) {
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
