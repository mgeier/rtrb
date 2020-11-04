use std::io::{Read, Write};

use criterion::black_box;
use criterion::{criterion_group, criterion_main};
use rtrb::RingBuffer;

pub fn write_and_read(criterion: &mut criterion::Criterion) {
    criterion.bench_function("read-single-byte", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut buf = [0u8];
        let mut i = 0;
        b.iter(|| {
            p.push(black_box(i)).unwrap();
            if let Ok(n) = c.read(&mut buf) {
                debug_assert_eq!(n, 1);
                assert_eq!(buf[0], black_box(i));
            } else {
                unreachable!();
            }
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("write-single-byte", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut buf = [0u8];
        let mut i = 0;
        b.iter(|| {
            buf[0] = black_box(i);
            if let Ok(n) = p.write(&buf) {
                debug_assert_eq!(n, 1);
            } else {
                unreachable!();
            }
            assert_eq!(c.pop(), Ok(black_box(i)));
            i = i.wrapping_add(black_box(1));
        })
    });
}

criterion_group!(benches, write_and_read);
criterion_main!(benches);
