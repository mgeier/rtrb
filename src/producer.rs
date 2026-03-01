use alloc::sync::Arc;
use core::cell::Cell;
use core::sync::atomic::Ordering;

use super::{
    chunks::{WriteChunk, WriteChunkUninit},
    ChunkError, CopyToUninit, PushError, RingBuffer,
};

// Only used in documentation:
#[allow(unused_imports)]
use super::Consumer;

/// The producer side of a [`RingBuffer`].
///
/// A `Producer` can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Individual elements can be moved into the ring buffer with [`Producer::push()`],
/// multiple elements at once can be written with [`Producer::write_chunk()`],
/// [`Producer::write_chunk_uninit()`], [`Producer::push_partial_slice()`] and
/// [`Producer::push_entire_slice()`].
///
/// The number of free slots currently available for writing can be obtained with
/// [`Producer::slots()`].
///
/// A `Producer` can only be created with [`RingBuffer::new()`]
/// (together with its counterpart, the [`Consumer`]).
///
/// When the `Producer` is dropped, [`Consumer::is_abandoned()`] will return `true`.
/// This can be used as a crude way to communicate to the receiving thread
/// that no more data will be produced.
/// When the `Producer` is dropped after the [`Consumer`] has already been dropped,
/// all items remaining in the ring buffer will be dropped and the allocated memory
/// will be deallocated.
#[derive(Debug, PartialEq, Eq)]
pub struct Producer<T> {
    pub(super) buffer: Arc<RingBuffer<T>>,

    /// A copy of `buffer.head` for quick access.
    ///
    /// This value can be stale and sometimes needs to be resynchronized with `buffer.head`.
    pub(super) cached_head: Cell<usize>,

    /// A copy of `buffer.tail` for quick access.
    ///
    /// This value is always in sync with `buffer.tail`.
    // NB: Caching the tail seems to have little effect on Intel CPUs, but it seems to
    //     improve performance on AMD CPUs, see https://github.com/mgeier/rtrb/pull/132
    pub(super) cached_tail: Cell<usize>,
}

/// It can be moved ...
/// ```
/// use rtrb::Producer;
/// fn assert_send<X: Send>() {}
/// assert_send::<Producer<u8>>();
/// ```
/// ... but not shared between threads:
/// ```compile_fail
/// # use rtrb::Producer;
/// fn assert_sync<X: Sync>() {}
/// assert_sync::<Producer<u8>>();
/// ```
// SAFETY: After moving the producer to another thread, there is still only a single thread
// that can access the producer side of the queue.
unsafe impl<T: Send> Send for Producer<T> {}

impl<T> Producer<T> {
    /// Attempts to push an element into the queue.
    ///
    /// The element is *moved* into the ring buffer and its slot
    /// is made available to be read by the [`Consumer`].
    ///
    /// # Errors
    ///
    /// If the queue is full, the element is returned back as an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PushError, RingBuffer};
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1);
    ///
    /// assert_eq!(producer.push(10), Ok(()));
    /// assert_eq!(producer.push(20), Err(PushError::Full(20)));
    /// ```
    pub fn push(&mut self, value: T) -> Result<(), PushError<T>> {
        if let Some(tail) = self.next_tail() {
            let b = &self.buffer;
            // SAFETY: tail points to an empty slot.
            unsafe { b.slot_ptr(tail).write(value) };
            let tail = b.increment1(tail);
            b.tail.store(tail, Ordering::Release);
            self.cached_tail.set(tail);
            Ok(())
        } else {
            Err(PushError::Full(value))
        }
    }

    /// Returns the number of slots available for writing.
    ///
    /// Since items can be concurrently consumed on another thread, the actual number
    /// of available slots may increase at any time (up to the [`RingBuffer::capacity()`]).
    ///
    /// To check for a single available slot,
    /// using [`Producer::is_full()`] is often quicker
    /// (because it might not have to check an atomic variable).
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(4096);
    /// assert_eq!(producer.push(0.5f32), Ok(()));
    ///
    /// assert_eq!(producer.slots(), 4095);
    /// ```
    pub fn slots(&self) -> usize {
        let b = &self.buffer;
        let head = b.head.load(Ordering::Acquire);
        self.cached_head.set(head);
        b.capacity() - b.distance(head, self.cached_tail.get())
    }

    /// Returns the number of cached slots.
    ///
    /// In many cases, this will not provide all available slots,
    /// but it might be marginally faster than [`Producer::slots()`]
    /// because it doesn't access the atomic read index.
    pub fn cached_slots(&self) -> usize {
        let b = &self.buffer;
        let head = self.cached_head.get();
        let tail = self.cached_tail.get();
        b.capacity() - b.distance(head, tail)
    }

    /// Returns `true` if there are currently no slots available for writing.
    ///
    /// A full ring buffer might cease to be full at any time
    /// if the corresponding [`Consumer`] is consuming items in another thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1);
    ///
    /// assert!(!producer.is_full());
    /// assert_eq!(producer.push(10), Ok(()));
    /// assert!(producer.is_full());
    /// ```
    ///
    /// Since items can be concurrently consumed on another thread, the ring buffer
    /// might not be full for long:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(10), Ok(()));
    /// if producer.is_full() {
    ///     // The buffer might be full, but it might as well not be
    ///     // if an item was just consumed on another thread.
    /// }
    /// ```
    ///
    /// However, if it's not full, another thread cannot change that:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(10), Ok(()));
    /// if !producer.is_full() {
    ///     // At least one slot is guaranteed to be available for writing.
    /// }
    /// ```
    pub fn is_full(&self) -> bool {
        self.next_tail().is_none()
    }

    /// Returns `true` if the corresponding [`Consumer`] has been destroyed.
    ///
    /// Note that since Rust version 1.74.0, this is not synchronizing with the consumer thread
    /// anymore, see <https://github.com/mgeier/rtrb/issues/114>.
    /// In a future version of `rtrb`, the synchronizing behavior might be restored.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(7);
    /// assert!(!producer.is_abandoned());
    /// assert_eq!(producer.push(10), Ok(()));
    /// drop(consumer);
    /// // The items that are still in the ring buffer are not accessible anymore.
    /// assert!(producer.is_abandoned());
    /// // Even though it's futile, items can still be written:
    /// assert_eq!(producer.push(11), Ok(()));
    /// ```
    ///
    /// Since the consumer can be concurrently dropped on another thread,
    /// the producer might become abandoned at any time:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(10), Ok(()));
    /// if !producer.is_abandoned() {
    ///     // Right now, the consumer might still be alive, but it might as well not be
    ///     // if another thread has just dropped it.
    /// }
    /// ```
    ///
    /// However, if it already is abandoned, it will stay that way:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(10), Ok(()));
    /// if producer.is_abandoned() {
    ///     // This is needed since Rust 1.74.0, see https://github.com/mgeier/rtrb/issues/114:
    ///     std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    ///     // The consumer does definitely not exist anymore.
    /// }
    /// ```
    pub fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.buffer) < 2
    }

    /// Returns a read-only reference to the ring buffer.
    pub fn buffer(&self) -> &RingBuffer<T> {
        &self.buffer
    }

    /// Get the tail position for writing the next slot, if available.
    ///
    /// This is a strict subset of the functionality implemented in `write_chunk_uninit()`.
    /// For performance, this special case is implemented separately.
    fn next_tail(&self) -> Option<usize> {
        let mut head = self.cached_head.get();
        let tail = self.cached_tail.get();
        let b = &self.buffer;
        // Check if the queue is *possibly* full.
        if b.distance(head, tail) == b.capacity() {
            // Refresh the head ...
            head = b.head.load(Ordering::Acquire);
            self.cached_head.set(head);
            // ... and check if it's *really* full.
            if b.distance(head, tail) == b.capacity() {
                // `head` didn't change, the buffer is definitely full.
                return None;
            }
        }
        Some(tail)
    }

    /// Prepares a chunk of `n` slots (initially containing their [`Default`] value)
    /// for writing.
    ///
    /// [`WriteChunk::as_mut_slices()`]
    /// provides mutable access to the slots.
    /// After writing to those slots, they explicitly have to be made available
    /// to be read by the [`Consumer`] by calling [`WriteChunk::commit()`]
    /// or [`WriteChunk::commit_all()`].
    ///
    /// For an alternative that does not require the trait bound [`Default`],
    /// see [`Producer::write_chunk_uninit()`].
    ///
    /// If items are supposed to be moved from an iterator into the ring buffer,
    /// [`Producer::write_chunk_uninit()`] followed by [`WriteChunkUninit::fill_from_iter()`]
    /// can be used.
    ///
    /// # Errors
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    /// Use
    /// [`Producer::slots()`]
    /// to obtain the number of available slots beforehand.
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](crate::chunks#examples) module.
    pub fn write_chunk(&mut self, n: usize) -> Result<WriteChunk<'_, T>, ChunkError>
    where
        T: Default,
    {
        self.write_chunk_uninit(n).map(WriteChunk::from)
    }

    /// Prepares a chunk of `n` (uninitialized) slots for writing.
    ///
    /// [`WriteChunkUninit::as_mut_slices()`]
    /// provides mutable access to the uninitialized slots.
    /// After writing to those slots, they explicitly have to be made available
    /// to be read by the [`Consumer`] by calling [`WriteChunkUninit::commit()`]
    /// or [`WriteChunkUninit::commit_all()`].
    ///
    /// Alternatively, [`WriteChunkUninit::fill_from_iter()`] can be used
    /// to move items from an iterator into the available slots.
    /// All moved items are automatically made available to be read by the [`Consumer`].
    ///
    /// # Errors
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    /// Use
    /// [`Producer::slots()`]
    /// to obtain the number of available slots beforehand.
    ///
    /// # Safety
    ///
    /// This function itself is safe, as is [`WriteChunkUninit::fill_from_iter()`].
    /// However, when using
    /// [`WriteChunkUninit::as_mut_slices()`],
    /// the user has to make sure that the relevant slots have been initialized
    /// before calling [`WriteChunkUninit::commit()`] or [`WriteChunkUninit::commit_all()`].
    ///
    /// For a safe alternative that provides
    /// mutable slices
    /// of [`Default`]-initialized slots, see [`Producer::write_chunk()`].
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](super::chunks#examples) module.
    pub fn write_chunk_uninit(&mut self, n: usize) -> Result<WriteChunkUninit<'_, T>, ChunkError> {
        let mut head = self.cached_head.get();
        let tail = self.cached_tail.get();
        let b = &self.buffer;
        // Check if the queue has *possibly* not enough slots.
        if b.capacity() - b.distance(head, tail) < n {
            // Refresh the head ...
            head = b.head.load(Ordering::Acquire);
            self.cached_head.set(head);
            // ... and check if there *really* are not enough slots.
            let slots = b.capacity() - b.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }
        let offset = b.collapse_position(tail);
        // SAFETY: `offset` has been set to a valid position.
        Ok(unsafe { WriteChunkUninit::new(self, n, offset) })
    }

    /// Copies as many items as possible from the given `slice` into the ring buffer.
    ///
    /// The written slots are automatically made available to be read by the [`Consumer`].
    ///
    /// Returns two sub-slices of `slice`:
    /// - The part that has been copied into the ring buffer (possibly empty).
    /// - The unused remainder (possibly empty).
    ///
    /// To copy an entire slice (and fail otherwise), [`Producer::push_entire_slice()`] can be used.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::Producer;
    ///
    /// fn push_at_least_one_element<'a>(
    ///     p: &mut Producer<i32>,
    ///     s: &'a [i32],
    /// ) -> Result<&'a [i32], &'a [i32]> {
    ///     match p.push_partial_slice(s) {
    ///         ([], remainder) => Err(remainder),
    ///         (_, remainder) => Ok(remainder),
    ///     }
    /// }
    ///
    /// fn block_while_pushing_entire_slice(p: &mut Producer<i32>, mut s: &[i32]) {
    ///     while let (_, remainder @ [_, ..]) = p.push_partial_slice(s) {
    ///         std::thread::yield_now();
    ///         s = remainder;
    ///     }
    /// }
    /// ```
    ///
    /// For more examples, see the documentation of the [`chunks`](crate::chunks#examples) module.
    pub fn push_partial_slice<'a>(&mut self, slice: &'a [T]) -> (&'a [T], &'a [T])
    where
        T: Copy,
    {
        let slots = if self.cached_slots() < slice.len() {
            slice.len().min(self.slots())
        } else {
            slice.len()
        };
        let (pushed, remainder) = slice.split_at(slots);
        // With MSRV 1.58, unwrap_unchecked() can be used.
        match self.push_entire_slice(pushed) {
            Ok(()) => {}
            // SAFETY: The requested slots are available.
            Err(_) => unsafe { core::hint::unreachable_unchecked() },
        };
        (pushed, remainder)
    }

    /// Copies all items from the given `slice` into the ring buffer.
    ///
    /// The written slots are automatically made available to be read by the [`Consumer`].
    ///
    /// To copy only into the available slots, [`Producer::push_partial_slice()`] can be used.
    ///
    /// # Errors
    ///
    /// If not enough free space is available in the ring buffer,
    /// a [`ChunkError`] with the available slots is returned.
    pub fn push_entire_slice(&mut self, slice: &[T]) -> Result<(), ChunkError>
    where
        T: Copy,
    {
        let mut chunk = self.write_chunk_uninit(slice.len())?;
        let (one, two) = chunk.as_mut_slices();
        let mid = one.len();
        // NB: If slice.is_empty(), chunk will be empty as well and the following are no-ops:
        slice[..mid].copy_to_uninit(one);
        slice[mid..].copy_to_uninit(two);
        // SAFETY: All slots have been initialized
        unsafe {
            chunk.commit_all();
        }
        Ok(())
    }
}
