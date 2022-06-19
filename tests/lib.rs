use rtrb::RingBuffer;

#[test]
fn capacity() {
    for i in 0..10 {
        let (p, c) = RingBuffer::<i32>::new(i);
        assert_eq!(p.buffer().capacity(), i);
        assert_eq!(c.buffer().capacity(), i);
    }
}

#[test]
fn zero_capacity() {
    let (mut p, mut c) = RingBuffer::<i32>::new(0);

    assert_eq!(p.slots(), 0);
    assert_eq!(c.slots(), 0);

    assert!(p.is_full());
    assert!(c.is_empty());

    assert!(p.push(10).is_err());
    assert!(c.pop().is_err());
}

#[test]
fn zero_sized_type() {
    struct ZeroSized;
    assert_eq!(std::mem::size_of::<ZeroSized>(), 0);

    let (mut p, mut c) = RingBuffer::new(1);
    assert_eq!(p.buffer().capacity(), 1);
    assert_eq!(p.slots(), 1);
    assert_eq!(c.slots(), 0);
    assert!(p.push(ZeroSized).is_ok());
    assert_eq!(p.slots(), 0);
    assert_eq!(c.slots(), 1);
    assert!(p.push(ZeroSized).is_err());
    assert!(c.peek().is_ok());
    assert!(c.pop().is_ok());
    assert_eq!(c.slots(), 0);
    assert!(c.peek().is_err());
}

#[cfg(not(miri))] // Miri is too slow for this
#[test]
fn parallel() {
    const COUNT: usize = 100_000;
    let (mut p, mut c) = RingBuffer::new(3);
    let pop_thread = std::thread::spawn(move || {
        for i in 0..COUNT {
            loop {
                if let Ok(x) = c.pop() {
                    assert_eq!(x, i);
                    break;
                }
            }
        }
        assert!(c.pop().is_err());
    });
    let push_thread = std::thread::spawn(move || {
        for i in 0..COUNT {
            while p.push(i).is_err() {}
        }
    });
    push_thread.join().unwrap();
    pop_thread.join().unwrap();
}

#[cfg(not(miri))] // Miri is too slow for this
#[test]
fn drops() {
    use rand::{thread_rng, Rng};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const RUNS: usize = 100;

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug, PartialEq)]
    struct DropCounter;

    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut rng = thread_rng();

    for _ in 0..RUNS {
        let steps = rng.gen_range(0..10_000);
        let additional = rng.gen_range(0..50);

        DROPS.store(0, Ordering::SeqCst);
        let (mut p, mut c) = RingBuffer::new(50);
        let pop_thread = std::thread::spawn(move || {
            for _ in 0..steps {
                while c.pop().is_err() {}
            }
        });
        let push_thread = std::thread::spawn(move || {
            for _ in 0..steps {
                while p.push(DropCounter).is_err() {
                    DROPS.fetch_sub(1, Ordering::SeqCst);
                }
            }
            p
        });
        p = push_thread.join().unwrap();
        pop_thread.join().unwrap();

        for _ in 0..additional {
            p.push(DropCounter).unwrap();
        }

        assert_eq!(DROPS.load(Ordering::SeqCst), steps);
        drop(p);
        assert_eq!(DROPS.load(Ordering::SeqCst), steps + additional);
    }
}

#[test]
fn trait_impls() {
    let (mut p, mut c) = RingBuffer::<u8>::new(0);

    assert!(format!("{:?}", p.buffer()).starts_with("RingBuffer {"));
    assert!(format!("{:?}", p).starts_with("Producer {"));
    assert!(format!("{:?}", c).starts_with("Consumer {"));

    assert_eq!(format!("{:?}", p.push(42).unwrap_err()), "Full(_)");
    assert_eq!(p.push(42).unwrap_err().to_string(), "full ring buffer");
    assert_eq!(format!("{:?}", c.pop().unwrap_err()), "Empty");
    assert_eq!(c.pop().unwrap_err().to_string(), "empty ring buffer");
    assert_eq!(format!("{:?}", c.peek().unwrap_err()), "Empty");
    assert_eq!(c.peek().unwrap_err().to_string(), "empty ring buffer");

    let (another_p, another_c) = RingBuffer::<u8>::new(0);
    assert_ne!(p, another_p);
    assert_ne!(c, another_c);
}

#[test]
fn no_race_with_is_abandoned() {
    static mut V: u32 = 0;
    let (p, c) = RingBuffer::<u8>::new(7);
    let t = std::thread::spawn(move || {
        unsafe { V = 10 };
        drop(p);
    });
    if c.is_abandoned() {
        unsafe { V = 20 };
    } else {
        // This is not synchronized, both Miri and ThreadSanitizer should detect a data race:
        //unsafe { V = 30 };
    }
    t.join().unwrap();
}
