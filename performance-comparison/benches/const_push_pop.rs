#[path = "../../benches/const_push_pop.rs"]
#[macro_use]
mod const_push_pop;

use ringbuf::traits::*;

create_const_push_pop_benchmark!(
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
