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
