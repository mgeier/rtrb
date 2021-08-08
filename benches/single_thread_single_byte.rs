//! Single-threaded benchmarks, pushing and popping a single byte using a single-element queue.
//!
//! This is *not* a typical use case but it should nevertheless be useful
//! for comparing the overhead of different methods.

use std::io::{Read, Write};
use std::mem::MaybeUninit;

use criterion::{black_box, criterion_group, criterion_main};
use criterion::{AxisScale, PlotConfiguration};

use rtrb::RingBuffer;

fn add_function<F, M>(group: &mut criterion::BenchmarkGroup<M>, id: impl Into<String>, mut f: F)
where
    F: FnMut(u8) -> u8,
    M: criterion::measurement::Measurement,
{
    group.bench_function(id, |b| {
        let mut i = 0;
        b.iter(|| {
            assert_eq!(f(black_box(i)), black_box(i));
            i = i.wrapping_add(1);
        });
    });
}

pub fn criterion_benchmark(criterion: &mut criterion::Criterion) {
    let mut group = criterion.benchmark_group("single-thread-single-byte");
    group.throughput(criterion::Throughput::Bytes(1));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    let mut v = Vec::<u8>::with_capacity(1);
    add_function(&mut group, "0-vec", |i| {
        v.push(i);
        v.pop().unwrap()
    });

    let (mut p, mut c) = RingBuffer::<u8>::new(1);

    add_function(&mut group, "1-push-pop", |i| {
        p.push(i).unwrap();
        c.pop().unwrap()
    });

    add_function(&mut group, "2-slice-read", |i| {
        p.push(i).unwrap();
        let chunk = c.read_chunk(1).unwrap();
        let (s, _) = chunk.as_slices();
        let result = s[0];
        chunk.commit_all();
        result
    });

    add_function(&mut group, "2-slice-write", |i| {
        let mut chunk = p.write_chunk(1).unwrap();
        let (s, _) = chunk.as_mut_slices();
        s[0] = i;
        chunk.commit_all();
        c.pop().unwrap()
    });

    add_function(&mut group, "2-slice-write-uninit", |i| {
        let mut chunk = p.write_chunk_uninit(1).unwrap();
        let (s, _) = chunk.as_mut_slices();
        s[0] = MaybeUninit::new(i);
        unsafe {
            chunk.commit_all();
        }
        c.pop().unwrap()
    });

    add_function(&mut group, "3-iterate-read", |i| {
        p.push(i).unwrap();
        let chunk = c.read_chunk(1).unwrap();
        chunk.into_iter().next().unwrap()
    });

    add_function(&mut group, "3-iterate-write", |i| {
        let chunk = p.write_chunk_uninit(1).unwrap();
        chunk.fill_from_iter(&mut std::iter::once(i));
        c.pop().unwrap()
    });

    let mut buf = [0];
    add_function(&mut group, "4-read", |i| {
        p.push(i).unwrap();
        let _ = c.read(&mut buf).unwrap();
        buf[0]
    });

    add_function(&mut group, "4-write", |i| {
        let _ = p.write(&[i]).unwrap();
        c.pop().unwrap()
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
