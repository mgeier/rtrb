use rtrb::{chunks::ChunkError, RingBuffer};

#[test]
fn zero_capacity() {
    let (mut p, mut c) = RingBuffer::<i32>::new(0);

    assert_eq!(p.write_chunk(1).unwrap_err(), ChunkError::TooFewSlots(0));
    assert_eq!(c.read_chunk(1).unwrap_err(), ChunkError::TooFewSlots(0));

    if let Ok(mut chunk) = p.write_chunk(0) {
        let (first, second) = chunk.as_mut_slices();
        assert!(first.is_empty());
        assert!(second.is_empty());
        chunk.commit_all();
    } else {
        unreachable!();
    }

    if let Ok(chunk) = c.read_chunk(0) {
        let (first, second) = chunk.as_slices();
        assert!(first.is_empty());
        assert!(second.is_empty());
        chunk.commit_all();
    } else {
        unreachable!();
    }
}

#[test]
fn drop_write_chunk() {
    // Static variable to count all drop() invocations
    static mut DROP_COUNT: i32 = 0;

    #[derive(Default)]
    struct Thing;

    impl Drop for Thing {
        fn drop(&mut self) {
            unsafe {
                DROP_COUNT += 1;
            }
        }
    }

    {
        let (mut p, mut c) = RingBuffer::new(3);

        if let Ok(mut chunk) = p.write_chunk(3) {
            let (first, _second) = chunk.as_mut_slices();
            assert_eq!(unsafe { DROP_COUNT }, 0);
            first[0] = Thing;
            // Overwriting drops the original Default element:
            assert_eq!(unsafe { DROP_COUNT }, 1);
            chunk.commit(1);
            // After committing, 2 (unwritten) slots are dropped
            assert_eq!(unsafe { DROP_COUNT }, 3);
        } else {
            unreachable!();
        }

        let chunk = c.read_chunk(1).unwrap();
        // Drop count is unchanged:
        assert_eq!(unsafe { DROP_COUNT }, 3);
        chunk.commit_all();
        // The stored element is never read, but it is dropped:
        assert_eq!(unsafe { DROP_COUNT }, 4);

        let chunk = p.write_chunk(3).unwrap();
        // Drop count is unchanged:
        assert_eq!(unsafe { DROP_COUNT }, 4);
        drop(chunk);
        // All three slots of the chunk are dropped:
        assert_eq!(unsafe { DROP_COUNT }, 7);
    }
    // RingBuffer was already empty, nothing is dropped:
    assert_eq!(unsafe { DROP_COUNT }, 7);
}
