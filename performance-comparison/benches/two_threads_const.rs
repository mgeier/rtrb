#[path = "../../benches/two_threads_const.rs"]
#[macro_use]
mod two_threads_const;

use ringbuf::traits::*;

create_two_threads_const_benchmark!(
    "rtrb",
    { ($N:expr) => {
        rtrb::RingBuffer::new($N)
    }},
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok(),
    ::
    "ringbuf",
    { ($N:expr) => {
        Box::leak(Box::new(ringbuf::StaticRb::<u8, $N>::default())).split_ref()
    }},
    |p, i| p.try_push(i).is_ok(),
    |c| c.try_pop(),
    ::
    "heapless",
    { ($N:expr) => {
        Box::leak(Box::new(heapless::spsc::Queue::<u8, $N>::new())).split()
    }},
    |p, i| p.enqueue(i).is_ok(),
    |c| c.dequeue(),
    ::
);
