#[path = "../../benches/two_threads.rs"]
#[macro_use]
mod two_threads;

use magnetic::Consumer as _;
use magnetic::Producer as _;

use ringbuf::traits::Consumer as _;
use ringbuf::traits::Producer as _;
use ringbuf::traits::Split as _;

create_two_threads_benchmark!(

    "1-npnc",
    |capacity| npnc::bounded::spsc::channel(capacity.next_power_of_two()),
    |p, i| p.produce(i).is_ok(),
    |c| c.consume().ok();

    "2-crossbeam-queue-pr338",
    crossbeam_queue_pr338::spsc::new,
    |q, i| q.push(i).is_ok(),
    |q| q.pop().ok();

    "3-rtrb",
    rtrb::RingBuffer::new,
    |p, i| p.push(i).is_ok(),
    |c| c.pop().ok();

    "4-omango",
    |capacity| omango::queue::spsc::bounded(u32::try_from(capacity).unwrap()),
    |p, i| p.try_send(i).is_ok(),
    |c| c.try_recv().ok();

    "5-ringbuf",
    |capacity| ringbuf::HeapRb::new(capacity).split(),
    |p, i| p.try_push(i).is_ok(),
    |c| c.try_pop();

    "6-magnetic",
    |capacity| {
        let buffer = magnetic::buffer::dynamic::DynamicBuffer::new(capacity).unwrap();
        magnetic::spsc::spsc_queue(buffer)
    },
    |p, i| p.try_push(i).is_ok(),
    |c| c.try_pop().ok();

    "7-concurrent-queue",
    |capacity| {
        let q = std::sync::Arc::new(concurrent_queue::ConcurrentQueue::bounded(capacity));
        (q.clone(), q)
    },
    |q, i| q.push(i).is_ok(),
    |q| q.pop().ok();

    "8-crossbeam-queue",
    |capacity| {
        let q = std::sync::Arc::new(crossbeam_queue::ArrayQueue::new(capacity));
        (q.clone(), q)
    },
    |q, i| q.push(i).is_ok(),
    |q| q.pop()

);
