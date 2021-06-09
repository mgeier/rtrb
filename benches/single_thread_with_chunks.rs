//! Single-threaded benchmarks, writing and reading chunks.
//!
//! Single-threaded usage is *not* a typical use case,
//! but the used chunk size and capacity could be.

use std::io::{Read, Write};
use std::mem::MaybeUninit;

use criterion::{black_box, criterion_group, criterion_main};
use criterion::{AxisScale, PlotConfiguration};

use rtrb::{CopyToUninit, RingBuffer};

const CHUNK_SIZE: usize = 4096;

fn add_function<F, M>(group: &mut criterion::BenchmarkGroup<M>, id: impl Into<String>, mut f: F)
where
    F: FnMut(&[u8]) -> [u8; CHUNK_SIZE],
    M: criterion::measurement::Measurement,
{
    group.bench_function(id, |b| {
        let mut data = [0; CHUNK_SIZE];
        let mut i: u8 = 0;
        for dst in data.iter_mut() {
            *dst = i;
            i = i.wrapping_add(1);
        }
        b.iter(|| {
            assert_eq!(f(black_box(&data)), data);
        })
    });
}

pub fn criterion_benchmark(criterion: &mut criterion::Criterion) {
    let mut group = criterion.benchmark_group("single-thread-with-chunks");
    group.throughput(criterion::Throughput::Bytes(CHUNK_SIZE as u64));
    group.plot_config(PlotConfiguration::default().summary_scale(AxisScale::Logarithmic));

    let (mut p, mut c) = RingBuffer::new(1000 * CHUNK_SIZE + 1);

    add_function(&mut group, "1-pop", |data| {
        let mut result = [0; CHUNK_SIZE];
        let _ = p.write(&data).unwrap();
        for i in result.iter_mut() {
            *i = c.pop().unwrap();
        }
        result
    });

    add_function(&mut group, "1-push", |data| {
        let mut result = [0; CHUNK_SIZE];
        for &i in data.iter() {
            p.push(i).unwrap();
        }
        let _ = c.read(&mut result).unwrap();
        result
    });

    add_function(&mut group, "2-slice-read", |data| {
        let mut result = [0; CHUNK_SIZE];
        let _ = p.write(&data).unwrap();
        let chunk = c.read_chunk(data.len()).unwrap();
        let (first, second) = chunk.as_slices();
        let mid = first.len();
        result[..mid].copy_from_slice(first);
        result[mid..].copy_from_slice(second);
        chunk.commit_all();
        result
    });

    add_function(&mut group, "2-slice-write", |data| {
        let mut result = [0; CHUNK_SIZE];
        let mut chunk = p.write_chunk(data.len()).unwrap();
        let (first, second) = chunk.as_mut_slices();
        let mid = first.len();
        first.copy_from_slice(&data[..mid]);
        second.copy_from_slice(&data[mid..]);
        chunk.commit_all();
        let _ = c.read(&mut result).unwrap();
        result
    });

    add_function(&mut group, "2-slice-write-uninit", |data| {
        let mut result = [0; CHUNK_SIZE];
        let mut chunk = p.write_chunk_uninit(data.len()).unwrap();
        let (first, second) = chunk.as_mut_slices();
        let mid = first.len();
        data[..mid].copy_to_uninit(first);
        data[mid..].copy_to_uninit(second);
        unsafe { chunk.commit_all() };
        let _ = c.read(&mut result).unwrap();
        result
    });

    add_function(&mut group, "3-iterate-read", |data| {
        let mut result = [0; CHUNK_SIZE];
        let _ = p.write(&data).unwrap();
        let mut chunk = c.read_chunk(data.len()).unwrap();
        for (dst, src) in result.iter_mut().zip(&mut chunk) {
            *dst = *src;
        }
        chunk.commit_iterated();
        result
    });

    add_function(&mut group, "3-iterate-write", |data| {
        let mut result = [0; CHUNK_SIZE];
        let mut chunk = p.write_chunk(data.len()).unwrap();
        for (src, dst) in data.iter().zip(&mut chunk) {
            *dst = *src;
        }
        chunk.commit_iterated();
        let _ = c.read(&mut result).unwrap();
        result
    });

    add_function(&mut group, "3-iterate-write-uninit", |data| {
        let mut result = [0; CHUNK_SIZE];
        let mut chunk = p.write_chunk_uninit(data.len()).unwrap();
        for (src, dst) in data.iter().zip(&mut chunk) {
            *dst = MaybeUninit::new(*src);
        }
        unsafe { chunk.commit_iterated() };
        let _ = c.read(&mut result).unwrap();
        result
    });

    add_function(&mut group, "4-write-read", |data| {
        let mut result = [0; CHUNK_SIZE];
        let _ = p.write(&data).unwrap();
        let _ = c.read(&mut result).unwrap();
        result
    });

    let (p, c) = RingBuffer::new(1000 * CHUNK_SIZE);
    let mut p = p.try_fixed_chunk_size(CHUNK_SIZE).unwrap();
    let mut c = c.try_fixed_chunk_size(CHUNK_SIZE).unwrap();

    add_function(&mut group, "5-fixed-chunks", |data| {
        let mut result = [0; CHUNK_SIZE];
        p.push_chunk().unwrap().copy_from_slice(data);
        result.copy_from_slice(&c.pop_chunk().unwrap());
        result
    });

    add_function(&mut group, "5-fixed-chunks-uninit", |data| {
        let mut result = [0; CHUNK_SIZE];
        // Safety: all slots will be initialized by copy_to_uninit().
        unsafe {
            data.copy_to_uninit(&mut p.push_chunk_uninit().unwrap());
        }
        result.copy_from_slice(&c.pop_chunk().unwrap());
        result
    });

    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
