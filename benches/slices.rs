use criterion::black_box;
use criterion::{criterion_group, criterion_main};
use rtrb::RingBuffer;

pub fn slices(criterion: &mut criterion::Criterion) {
    criterion.bench_function("pop-single-element-slices", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            p.push(black_box(i)).unwrap();
            if let Ok(chunk) = c.read_chunk(1) {
                let (first, second) = chunk.as_slices();
                assert_eq!(first[0], black_box(i));
                debug_assert!(second.is_empty());
                chunk.commit_all()
            } else {
                unreachable!();
            }
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("push-single-element-slices", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            if let Ok(mut chunk) = p.write_chunk(1) {
                let (first, second) = chunk.as_mut_slices();
                first[0] = black_box(i);
                debug_assert!(second.is_empty());
                chunk.commit_all();
            } else {
                unreachable!();
            }
            assert_eq!(c.pop(), Ok(black_box(i)));
            i = i.wrapping_add(black_box(1));
        })
    });

    criterion.bench_function("push-single-element-slices-maybe-uninit", |b| {
        let (mut p, mut c) = RingBuffer::<u8>::new(1).split();
        let mut i = 0;
        b.iter(|| {
            if let Ok(mut chunk) = p.write_chunk_maybe_uninit(1) {
                let (first, second) = chunk.as_mut_slices();
                let ptr = first[0].as_mut_ptr();
                unsafe {
                    ptr.write(black_box(i));
                }
                debug_assert!(second.is_empty());
                unsafe {
                    chunk.commit_all();
                }
            } else {
                unreachable!();
            }
            assert_eq!(c.pop(), Ok(black_box(i)));
            i = i.wrapping_add(black_box(1));
        })
    });
}

criterion_group!(benches, slices);
criterion_main!(benches);
