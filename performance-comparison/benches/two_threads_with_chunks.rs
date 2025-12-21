#[path = "../../benches/two_threads_with_chunks.rs"]
#[macro_use]
mod two_threads_with_chunks;

use std::io::{Read as _, Write as _};

use ringbuf::traits::Consumer as _;
use ringbuf::traits::Producer as _;
use ringbuf::traits::Split as _;

create_two_threads_with_chunks_benchmark!(
    "rtrb",
    rtrb::RingBuffer::new,
    |p, s| match p.write(s) {
        Ok(n) => &s[n..],
        _ => s,
    },
    |c, s| match c.read(s) {
        Ok(n) => &s[..n],
        _ => &[],
    },
    ::
    "ringbuf",
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
