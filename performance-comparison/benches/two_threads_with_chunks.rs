#[path = "../../benches/two_threads_with_chunks.rs"]
#[macro_use]
mod two_threads_with_chunks;

use core::num::NonZeroUsize;

use ringbuf::traits::Consumer as _;
use ringbuf::traits::Producer as _;
use ringbuf::traits::Split as _;

create_two_threads_with_chunks_benchmark!(
    "1-gil",
    |capacity| gil::spsc::channel(NonZeroUsize::new(capacity).unwrap()),
    |p, s| {
        let buffer = p.write_buffer();
        let n = s.len().min(buffer.len());
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), buffer.as_mut_ptr().cast(), n);
            p.commit(n);
        }
        &s[n..]
    },
    |c, s| {
        let buffer = c.read_buffer();
        let n = s.len().min(buffer.len());
        let subslice = &mut s[..n];
        subslice.copy_from_slice(&buffer[..n]);
        unsafe {
            c.advance(n);
        }
        subslice
    },
    ::
    "2-rtrb",
    rtrb::RingBuffer::new,
    |p, s| p.push_slice(s).1,
    |c, s| c.pop_slice(s).0,
    ::
    "3-ringbuf",
    |capacity| ringbuf::HeapRb::new(capacity).split(),
    |p, s| {
        let n = p.push_slice(s);
        &s[n..]
    },
    |c, s| {
        let n = c.pop_slice(s);
        &s[..n]
    },
    ::
);
