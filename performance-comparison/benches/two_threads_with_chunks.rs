#[path = "../../benches/two_threads_with_chunks.rs"]
#[macro_use]
mod two_threads_with_chunks;

use ringbuf::traits::Consumer as _;
use ringbuf::traits::Producer as _;
use ringbuf::traits::Split as _;

create_two_threads_with_chunks_benchmark!(
    "rtrb",
    rtrb::RingBuffer::new,
    |p, s| p.push_slice(s).1,
    |c, s| c.pop_slice(s).0,
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
