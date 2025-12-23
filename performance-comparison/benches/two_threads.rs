#[path = "../../benches/two_threads.rs"]
#[macro_use]
mod two_threads;

use ringbuf::traits::Consumer as _;
use ringbuf::traits::Producer as _;
use ringbuf::traits::Split as _;

create_two_threads_benchmark!(
    "1-bounded-spsc-queue", // calls next_power_of_two()
    bounded_spsc_queue::make,
    |p, i| p.try_push(i).is_none(),
    |c| c.try_pop(),
    ::
    "2-crossbeam-queue-pr338",
    crossbeam_queue_pr338::spsc::new,
    |q, i| q.push(i).is_ok(),
    |q| q.pop().ok(),
    ::
    "3-rtrb",
    rtrb::RingBuffer::new,
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok(),
    ::
    "4-omango",
    |capacity| omango::queue::spsc::bounded(u32::try_from(capacity).unwrap()),
    |p, i| p.try_send(i).is_ok(),
    |c| c.try_recv().ok(),
    ::
    "5-ringbuffer-spsc",
    |capacity| ringbuffer_spsc::ringbuffer(capacity.next_power_of_two()),
    |p, i| p.push(i).is_none(),
    |c| c.pull(),
    ::
    "6-ringbuf",
    |capacity| ringbuf::HeapRb::new(capacity).split(),
    |p, i| p.try_push(i).is_ok(),
    |c| c.try_pop(),
    ::
    "7-crossbeam-queue",
    |capacity| {
        let q = std::sync::Arc::new(crossbeam_queue::ArrayQueue::new(capacity));
        (q.clone(), q)
    },
    |q, i| q.push(i).is_ok(),
    |q| q.pop(),
    ::
);
