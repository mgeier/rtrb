use criterion::black_box;
use criterion::{criterion_group, criterion_main};
use rtrb::RingBuffer;

pub fn iterators(criterion: &mut criterion::Criterion) {
    criterion.bench_function("iterate-to-read-single-element", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            p.push(black_box(i)).unwrap();
            if let Ok(mut chunk) = c.read_chunk(1) {
                for elem in &mut chunk {
                    assert_eq!(*elem, black_box(i));
                }
                chunk.commit_iterated();
            } else {
                unreachable!();
            }
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("iterate-to-write-single-element", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            if let Ok(mut chunk) = p.write_chunk(1) {
                for elem in &mut chunk {
                    *elem = black_box(i);
                }
                chunk.commit_iterated();
            } else {
                unreachable!();
            }
            assert_eq!(c.pop(), Ok(black_box(i)));
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("iterate-to-write-single-element-maybe-uninit", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            if let Ok(mut chunk) = p.write_chunk_maybe_uninit(1) {
                for elem in &mut chunk {
                    unsafe {
                        elem.as_mut_ptr().write(black_box(i));
                    }
                }
                unsafe {
                    chunk.commit_iterated();
                }
            } else {
                unreachable!();
            }
            assert_eq!(c.pop(), Ok(black_box(i)));
            i = i.wrapping_add(black_box(1));
        })
    });
}

criterion_group!(benches, iterators);
criterion_main!(benches);
