use alloc::sync::Arc;
use core::cell::Cell;
use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;

use super::{chunks::ReadChunk, ChunkError, CopyToUninit, PeekError, PopError, RingBuffer};

// Only used in documentation:
#[allow(unused_imports)]
use super::Producer;

/// The consumer side of a [`RingBuffer`].
///
/// A `Consumer` can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Individual elements can be moved out of the ring buffer with [`Consumer::pop()`],
/// multiple elements at once can be read with [`Consumer::read_chunk()`],
/// [`Consumer::pop_partial_slice()`] and [`Consumer::pop_partial_slice_uninit()`].
///
/// The number of slots currently available for reading can be obtained with
/// [`Consumer::slots()`].
///
/// A `Consumer` can only be created with [`RingBuffer::new()`]
/// (together with its counterpart, the [`Producer`]).
///
/// When the `Consumer` is dropped, [`Producer::is_abandoned()`] will return `true`.
/// This can be used as a crude way to communicate to the sending thread
/// that no more data will be consumed.
/// When the `Consumer` is dropped after the [`Producer`] has already been dropped,
/// all items remaining in the ring buffer will be dropped and the allocated memory
/// will be deallocated.
#[derive(Debug, PartialEq, Eq)]
pub struct Consumer<T> {
    pub(super) buffer: Arc<RingBuffer<T>>,

    /// A copy of `buffer.head` for quick access.
    ///
    /// This value is always in sync with `buffer.head`.
    // NB: Caching the head seems to have little effect on Intel CPUs, but it seems to
    //     improve performance on AMD CPUs, see https://github.com/mgeier/rtrb/pull/132
    pub(super) cached_head: Cell<usize>,

    /// A copy of `buffer.tail` for quick access.
    ///
    /// This value can be stale and sometimes needs to be resynchronized with `buffer.tail`.
    pub(super) cached_tail: Cell<usize>,
}

/// It can be moved ...
/// ```
/// use rtrb::Consumer;
/// fn assert_send<X: Send>() {}
/// assert_send::<Consumer<u8>>();
/// ```
/// ... but not shared between threads:
/// ```compile_fail
/// # use rtrb::Consumer;
/// fn assert_sync<X: Sync>() {}
/// assert_sync::<Consumer<u8>>();
/// ```
// SAFETY: After moving the consumer to another thread, there is still only a single thread
// that can access the consumer side of the queue.
unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Consumer<T> {
    /// Attempts to pop the next element from the queue.
    ///
    /// The element is *moved* out of the ring buffer and its slot
    /// is made available to be filled by the [`Producer`] again.
    ///
    /// # Errors
    ///
    /// If the queue is empty, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PopError, RingBuffer};
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1);
    ///
    /// assert_eq!(producer.push(10), Ok(()));
    /// assert_eq!(consumer.pop(), Ok(10));
    /// assert_eq!(consumer.pop(), Err(PopError::Empty));
    /// ```
    ///
    /// To obtain an [`Option<T>`](Option), use [`.ok()`](Result::ok) on the result.
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// assert_eq!(producer.push(20), Ok(()));
    /// assert_eq!(consumer.pop().ok(), Some(20));
    /// ```
    pub fn pop(&mut self) -> Result<T, PopError> {
        if let Some(head) = self.next_head() {
            let b = &self.buffer;
            // SAFETY: head points to an initialized slot.
            let value = unsafe { b.slot_ptr(head).read() };
            let head = b.increment1(head);
            b.head.store(head, Ordering::Release);
            self.cached_head.set(head);
            Ok(value)
        } else {
            Err(PopError::Empty)
        }
    }

    /// Attempts to get read access to the next element in the queue without removing it.
    ///
    /// # Errors
    ///
    /// If the queue is empty, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PeekError, RingBuffer};
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1);
    ///
    /// assert_eq!(consumer.peek(), Err(PeekError::Empty));
    /// assert_eq!(producer.push(10), Ok(()));
    /// assert_eq!(consumer.peek(), Ok(&10));
    /// assert_eq!(consumer.peek(), Ok(&10));
    /// ```
    ///
    /// Note that `peek()` takes a shared reference to `self`,
    /// which means that other methods that take `&self` can be called
    /// while the returned reference is still in use.
    /// However, calling methods that take `&mut self`
    /// (like [`Consumer::pop()`] and [`Consumer::read_chunk()`]) leads to a compiler error:
    ///
    /// ```compile_fail
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(8);
    /// producer.push(10).unwrap();
    /// let shared_ref = consumer.peek().unwrap();
    /// let value = consumer.pop().unwrap();
    /// assert_eq!(shared_ref, &10);
    /// ```
    pub fn peek(&self) -> Result<&T, PeekError> {
        if let Some(head) = self.next_head() {
            // SAFETY: head points to an initialized slot.
            Ok(unsafe { &*self.buffer.slot_ptr(head) })
        } else {
            Err(PeekError::Empty)
        }
    }

    /// Returns the number of slots available for reading.
    ///
    /// This number will also be reported via a [`ChunkError`] when calling
    /// [`read_chunk()`](Consumer::read_chunk) with a larger number.
    ///
    /// Since items can be concurrently produced on another thread, the actual number
    /// of available slots may increase at any time (up to the [`RingBuffer::capacity()`]).
    ///
    /// To check for a single available slot,
    /// using [`Consumer::is_empty()`] is often quicker
    /// (because it might not have to check an atomic variable).
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1024);
    ///
    /// assert_eq!(consumer.slots(), 0);
    /// assert_eq!(producer.push(0.0), Ok(()));
    /// assert_eq!(consumer.slots(), 1);
    /// ```
    pub fn slots(&self) -> usize {
        let b = &self.buffer;
        let tail = b.tail.load(Ordering::Acquire);
        self.cached_tail.set(tail);
        b.distance(self.cached_head.get(), tail)
    }

    /// Returns the number of cached slots.
    ///
    /// In many cases, this will not provide all available slots,
    /// but it might be marginally faster than [`Consumer::slots()`]
    /// because it doesn't access the atomic write index.
    pub fn cached_slots(&self) -> usize {
        let head = self.cached_head.get();
        let tail = self.cached_tail.get();
        self.buffer.distance(head, tail)
    }

    /// Returns `true` if there are currently no slots available for reading.
    ///
    /// An empty ring buffer might cease to be empty at any time
    /// if the corresponding [`Producer`] is producing items in another thread.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(1);
    ///
    /// assert!(consumer.is_empty());
    /// assert_eq!(producer.push(0.0), Ok(()));
    /// assert!(!consumer.is_empty());
    /// ```
    ///
    /// Since items can be concurrently produced on another thread, the ring buffer
    /// might not be empty for long:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(0.0), Ok(()));
    /// if consumer.is_empty() {
    ///     // The buffer might be empty, but it might as well not be
    ///     // if an item was just produced on another thread.
    /// }
    /// ```
    ///
    /// However, if it's not empty, another thread cannot change that:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(0.0), Ok(()));
    /// if !consumer.is_empty() {
    ///     // At least one slot is guaranteed to be available for reading.
    /// }
    /// ```
    pub fn is_empty(&self) -> bool {
        self.next_head().is_none()
    }

    /// Returns `true` if the corresponding [`Producer`] has been destroyed.
    ///
    /// Note that since Rust version 1.74.0, this is not synchronizing with the producer thread
    /// anymore, see <https://github.com/mgeier/rtrb/issues/114>.
    /// In a future version of `rtrb`, the synchronizing behavior might be restored.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(7);
    /// assert!(!consumer.is_abandoned());
    /// assert_eq!(producer.push(10), Ok(()));
    /// drop(producer);
    /// assert!(consumer.is_abandoned());
    /// // The items that are left in the ring buffer can still be consumed:
    /// assert_eq!(consumer.pop(), Ok(10));
    /// ```
    ///
    /// Since the producer can be concurrently dropped on another thread,
    /// the consumer might become abandoned at any time:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut producer, mut consumer) = RingBuffer::new(1);
    /// # assert_eq!(producer.push(10), Ok(()));
    /// if !consumer.is_abandoned() {
    ///     // Right now, the producer might still be alive, but it might as well not be
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
    /// if consumer.is_abandoned() {
    ///     // This is needed since Rust 1.74.0, see https://github.com/mgeier/rtrb/issues/114:
    ///     std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    ///     // The producer does definitely not exist anymore.
    /// }
    /// ```
    pub fn is_abandoned(&self) -> bool {
        Arc::strong_count(&self.buffer) < 2
    }

    /// Returns a read-only reference to the ring buffer.
    pub fn buffer(&self) -> &RingBuffer<T> {
        &self.buffer
    }

    /// Get the `head` position for reading the next slot, if available.
    ///
    /// This is a strict subset of the functionality implemented in `read_chunk()`.
    /// For performance, this special case is implemented separately.
    fn next_head(&self) -> Option<usize> {
        let head = self.cached_head.get();
        let tail = self.cached_tail.get();

        // Check if the queue is *possibly* empty.
        if head == tail {
            // Refresh the tail ...
            let tail = self.buffer.tail.load(Ordering::Acquire);
            self.cached_tail.set(tail);
            // ... and check if it's *really* empty.
            if head == tail {
                // `tail` didn't change, queue is empty.
                return None;
            }
        }
        Some(head)
    }

    /// Prepares a chunk of `n` slots for reading.
    ///
    /// [`ReadChunk::as_slices()`]
    /// provides immutable access to the slots.
    /// After reading from those slots, they explicitly have to be made available
    /// to be written again by the [`Producer`] by calling [`ReadChunk::commit()`]
    /// or [`ReadChunk::commit_all()`].
    ///
    /// Alternatively, items can be moved out of the [`ReadChunk`] using iteration
    /// because it implements [`IntoIterator`]
    /// ([`ReadChunk::into_iter()`] can be used to explicitly turn it into an [`Iterator`]).
    /// All moved items are automatically made available to be written again by the [`Producer`].
    ///
    /// # Errors
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    /// Use
    /// [`Consumer::slots()`]
    /// to obtain the number of available slots beforehand.
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](super::chunks#examples) module.
    pub fn read_chunk(&mut self, n: usize) -> Result<ReadChunk<'_, T>, ChunkError> {
        let head = self.cached_head.get();
        let tail = self.cached_tail.get();
        let b = &self.buffer;
        // Check if the queue has *possibly* not enough slots.
        if b.distance(head, tail) < n {
            // Refresh the tail ...
            let tail = b.tail.load(Ordering::Acquire);
            self.cached_tail.set(tail);
            // ... and check if there *really* are not enough slots.
            let slots = b.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }
        let offset = b.collapse_position(head);
        // SAFETY: `offset` has been set to a valid position.
        Ok(unsafe { ReadChunk::new(self, n, offset) })
    }

    /// Copies as many items as possible from the ring buffer to the given `slice`.
    ///
    /// The copied slots are automatically made available to be written again by the [`Producer`].
    ///
    /// Returns two sub-slices of `slice`:
    /// - The part that has been filled with data from the ring buffer (possibly empty).
    /// - The unused remainder (possibly empty).
    ///
    /// To copy an entire slice (and fail otherwise), [`Consumer::pop_entire_slice()`] can be used.
    /// To copy into an uninitialized slice, [`Consumer::pop_partial_slice_uninit()`] can be used.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::Consumer;
    ///
    /// fn pop_at_least_one_element<'a>(
    ///     c: &mut Consumer<i32>,
    ///     s: &'a mut [i32],
    /// ) -> Result<&'a mut [i32], &'a mut [i32]> {
    ///     match c.pop_partial_slice(s) {
    ///         ([], remainder) => Err(remainder),
    ///         (_, remainder) => Ok(remainder),
    ///     }
    /// }
    ///
    /// fn block_while_popping_entire_slice(c: &mut Consumer<i32>, mut s: &mut [i32]) {
    ///     while let (_, remainder @ [_, ..]) = c.pop_partial_slice(s) {
    ///         std::thread::yield_now();
    ///         s = remainder;
    ///     }
    /// }
    /// ```
    ///
    /// For more examples, see the documentation of the [`chunks`](crate::chunks#examples) module.
    pub fn pop_partial_slice<'a>(&mut self, slice: &'a mut [T]) -> (&'a mut [T], &'a mut [T])
    where
        T: Copy,
    {
        // SAFETY: Transmuting &mut [T] to &mut [MaybeUninit<T>] is generally unsafe!
        // However, since we can guarantee that only valid T values will ever be written,
        // and the reference never leaves our control, it should be fine.
        let (popped, remainder) =
            unsafe { self.pop_partial_slice_uninit(&mut *(slice as *mut [_] as *mut _)) };
        // NB: This can be replaced by `assume_init_mut()` once stabilized:
        // SAFETY: `remainder` is a subslice of the original initialized buffer.
        (popped, unsafe { &mut *(remainder as *mut _ as *mut [_]) })
    }

    /// Copies as many items as possible from the ring buffer to the given uninitialized `slice`.
    ///
    /// The copied slots are automatically made available to be written again by the [`Producer`].
    ///
    /// Returns two sub-slices of `slice`:
    /// - The part that has been filled with data from the ring buffer (possibly empty).
    /// - The unused remainder (possibly empty).
    ///
    /// The first of the returned slices has been initialized,
    /// while the second one remains uninitialized.
    ///
    /// To copy an entire slice (and fail otherwise),
    /// [`Consumer::pop_entire_slice_uninit()`] can be used.
    /// To copy into an initialized slice, [`Consumer::pop_partial_slice()`] can be used.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::mem::MaybeUninit;
    ///
    /// use rtrb::Consumer;
    ///
    /// fn pop_at_least_one_element_uninit<'a>(
    ///     c: &mut Consumer<i32>,
    ///     s: &'a mut [MaybeUninit<i32>],
    /// ) -> Result<&'a mut [MaybeUninit<i32>], &'a mut [MaybeUninit<i32>]> {
    ///     match c.pop_partial_slice_uninit(s) {
    ///         ([], remainder) => Err(remainder),
    ///         (_, remainder) => Ok(remainder),
    ///     }
    /// }
    ///
    /// fn block_while_popping_entire_slice_uninit(
    ///     c: &mut Consumer<i32>,
    ///     mut s: &mut [MaybeUninit<i32>],
    /// ) {
    ///     while let (_, remainder @ [_, ..]) = c.pop_partial_slice_uninit(s) {
    ///         std::thread::yield_now();
    ///         s = remainder;
    ///     }
    /// }
    /// ```
    ///
    /// Typically, we might get an uninitialized buffer via FFI,
    /// but in this example we are using the uninitialized part of a [`Vec`]:
    ///
    /// ```
    /// use std::mem::MaybeUninit;
    ///
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, mut consumer) = RingBuffer::new(4);
    /// let (_, remainder) = producer.push_partial_slice(&[1, 2, 3]);
    /// assert!(remainder.is_empty());
    /// let mut buffer = Vec::with_capacity(5);
    /// let buffer_uninit = buffer.spare_capacity_mut();
    /// let (popped, remainder) = consumer.pop_partial_slice_uninit(buffer_uninit);
    /// assert_eq!(popped, [1, 2, 3]);
    /// // The returned slices are mutable ...
    /// popped[0] = -42;
    /// // ... but the second one is still uninitialized:
    /// remainder[0] = MaybeUninit::new(99);
    /// // All this happened in the uninitialized part of the buffer,
    /// // which is still "officially" empty:
    /// assert!(buffer.is_empty());
    /// // SAFETY: The first 4 elements have been initialized.
    /// unsafe {
    ///     buffer.set_len(4);
    /// }
    /// assert_eq!(buffer, [-42, 2, 3, 99]);
    /// ```
    #[inline]
    pub fn pop_partial_slice_uninit<'a>(
        &mut self,
        slice: &'a mut [MaybeUninit<T>],
    ) -> (&'a mut [T], &'a mut [MaybeUninit<T>])
    where
        T: Copy,
    {
        let slots = if self.cached_slots() < slice.len() {
            slice.len().min(self.slots())
        } else {
            slice.len()
        };
        let (buffer, remainder) = slice.split_at_mut(slots);
        // With MSRV 1.58, unwrap_unchecked() can be used.
        let popped = match self.pop_entire_slice_uninit(buffer) {
            Ok(popped) => popped,
            // SAFETY: The requested slots are available.
            Err(_) => unsafe { core::hint::unreachable_unchecked() },
        };
        (popped, remainder)
    }

    /// Copies as many items from the ring buffer as to fill the given `slice`.
    ///
    /// The copied slots are automatically made available to be written again by the [`Producer`].
    ///
    /// # Errors
    ///
    /// If not enough data is available in the ring buffer, no items are copied and
    /// a [`ChunkError`] with the available items is returned.
    ///
    /// To copy only the available slots, [`Consumer::pop_partial_slice()`] can be used.
    /// To copy into an uninitialized slice, [`Consumer::pop_entire_slice_uninit()`] can be used.
    pub fn pop_entire_slice(&mut self, slice: &mut [T]) -> Result<(), ChunkError>
    where
        T: Copy,
    {
        // SAFETY: Transmuting &mut [T] to &mut [MaybeUninit<T>] is generally unsafe!
        // However, since we can guarantee that only valid T values will ever be written,
        // and the reference never leaves our control, it should be fine.
        let _ = unsafe { self.pop_entire_slice_uninit(&mut *(slice as *mut [_] as *mut _))? };
        Ok(())
    }

    /// Copies as many items from the ring buffer as to fill the given uninitialized `slice`.
    ///
    /// The copied slots are automatically made available to be written again by the [`Producer`].
    ///
    /// Returns the given slice, but now initialized.
    ///
    /// # Errors
    ///
    /// If not enough data is available in the ring buffer, no items are copied and
    /// a [`ChunkError`] with the available items is returned.
    ///
    /// To copy only the available slots, [`Consumer::pop_partial_slice_uninit()`] can be used.
    /// To copy into an initialized slice, [`Consumer::pop_entire_slice()`] can be used.
    pub fn pop_entire_slice_uninit<'a>(
        &mut self,
        slice: &'a mut [MaybeUninit<T>],
    ) -> Result<&'a mut [T], ChunkError>
    where
        T: Copy,
    {
        let chunk = self.read_chunk(slice.len())?;
        let (one, two) = chunk.as_slices();
        let mid = one.len();
        // NB: If slice.is_empty(), chunk will be empty as well and the following are no-ops:
        one.copy_to_uninit(&mut slice[..mid]);
        two.copy_to_uninit(&mut slice[mid..]);
        chunk.commit_all();
        // NB: This can be replaced by `assume_init_mut()` once stabilized:
        // SAFETY: The entire `slice` has been initialized above.
        Ok(unsafe { &mut *(slice as *mut _ as *mut [_]) })
    }
}
