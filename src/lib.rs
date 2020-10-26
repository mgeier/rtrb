//! A realtime-safe single-producer single-consumer (SPSC) ring buffer.
//!
//! A [`RingBuffer`] consists of two parts:
//! a [`Producer`] for writing into the ring buffer and
//! a [`Consumer`] for reading from the ring buffer.
//!
//! Reading from and writing into the ring buffer is lock-free and wait-free.
//! All reading and writing functions return immediately.
//! Only a single thread can write into the ring buffer and a single thread
//! (typically a different one) can read from the ring buffer.
//! If the queue is empty, there is no way for the reading thread to wait
//! for new data, other than trying repeatedly until reading succeeds.
//! Similarly, if the queue is full, there is no way for the writing thread
//! to wait for newly available space to write to, other than trying repeatedly.
//!
//! # Examples
//!
//! ```
//! use rtrb::RingBuffer;
//!
//! let (mut producer, mut consumer) = RingBuffer::new(2).split();
//!
//! assert!(producer.push(1).is_ok());
//! assert!(producer.push(2).is_ok());
//! assert!(producer.push(3).is_err());
//!
//! std::thread::spawn(move || {
//!     assert_eq!(consumer.pop(), Ok(1));
//!     assert_eq!(consumer.pop(), Ok(2));
//!     assert!(consumer.pop().is_err());
//! }).join().unwrap();
//!
//! ```

#![warn(rust_2018_idioms)]
#![deny(missing_docs)]

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cache_padded::CachePadded;

mod error;

pub use error::{ChunkError, PeekError, PopError, PushError};

/// A bounded single-producer single-consumer queue.
///
/// *See also the [crate-level documentation](crate).*
pub struct RingBuffer<T> {
    /// The head of the queue.
    ///
    /// This integer is in range `0 .. 2 * capacity`.
    head: CachePadded<AtomicUsize>,

    /// The tail of the queue.
    ///
    /// This integer is in range `0 .. 2 * capacity`.
    tail: CachePadded<AtomicUsize>,

    /// The buffer holding slots.
    buffer: *mut T,

    /// The queue capacity.
    capacity: usize,

    /// Indicates that dropping a `RingBuffer<T>` may drop elements of type `T`.
    _marker: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Creates a [`RingBuffer`] with the given capacity.
    ///
    /// The returned [`RingBuffer`] is typically immediately split into
    /// the producer and the consumer side by [`RingBuffer::split`].
    ///
    /// # Panics
    ///
    /// Panics if the capacity is zero.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let rb = RingBuffer::<f32>::new(100);
    /// ```
    ///
    /// Specifying an explicit type with the [turbofish](https://turbo.fish/)
    /// is is only necessary if it cannot be deduced by the compiler.
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, consumer) = RingBuffer::new(100).split();
    /// assert!(producer.push(0.0f32).is_ok());
    /// ```
    pub fn new(capacity: usize) -> RingBuffer<T> {
        assert!(capacity > 0, "capacity must be non-zero");

        // Allocate a buffer of length `capacity`.
        let buffer = {
            let mut v = Vec::<T>::with_capacity(capacity);
            let ptr = v.as_mut_ptr();
            std::mem::forget(v);
            ptr
        };
        RingBuffer {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            buffer,
            capacity,
            _marker: PhantomData,
        }
    }

    /// Splits the [`RingBuffer`] into [`Producer`] and [`Consumer`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (producer, consumer) = RingBuffer::<f32>::new(100).split();
    /// ```
    pub fn split(self) -> (Producer<T>, Consumer<T>) {
        let rb = Arc::new(self);
        let p = Producer {
            rb: rb.clone(),
            head: Cell::new(0),
            tail: Cell::new(0),
        };
        let c = Consumer {
            rb,
            head: Cell::new(0),
            tail: Cell::new(0),
        };
        (p, c)
    }

    /// Returns the capacity of the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let rb = RingBuffer::<f32>::new(100);
    /// assert_eq!(rb.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Wraps a position from the range `0 .. 2 * capacity` to `0 .. capacity`.
    fn collapse_position(&self, pos: usize) -> usize {
        if pos < self.capacity {
            pos
        } else {
            pos - self.capacity
        }
    }

    /// Returns a pointer to the slot at position `pos`.
    ///
    /// The position must be in range `0 .. 2 * capacity`.
    unsafe fn slot_ptr(&self, pos: usize) -> *mut T {
        self.buffer.add(self.collapse_position(pos))
    }

    /// Increments a position by going `n` slots forward.
    ///
    /// The position must be in range `0 .. 2 * capacity`.
    fn increment(&self, pos: usize, n: usize) -> usize {
        let threshold = 2 * self.capacity - n;
        if pos < threshold {
            pos + n
        } else {
            pos - threshold
        }
    }

    /// Increments a position by going one slot forward.
    ///
    /// This is more efficient than self.increment(..., 1).
    ///
    /// The position must be in range `0 .. 2 * capacity`.
    fn increment1(&self, pos: usize) -> usize {
        if pos < 2 * self.capacity - 1 {
            pos + 1
        } else {
            0
        }
    }

    /// Returns the distance between two positions.
    ///
    /// Positions must be in range `0 .. 2 * capacity`.
    fn distance(&self, a: usize, b: usize) -> usize {
        if a <= b {
            b - a
        } else {
            2 * self.capacity - a + b
        }
    }
}

impl<T> Drop for RingBuffer<T> {
    /// Drops all non-empty slots.
    fn drop(&mut self) {
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        // Loop over all slots that hold a value and drop them.
        while head != tail {
            unsafe {
                self.slot_ptr(head).drop_in_place();
            }
            head = self.increment(head, 1);
        }

        // Finally, deallocate the buffer, but don't run any destructors.
        unsafe {
            Vec::from_raw_parts(self.buffer, 0, self.capacity);
        }
    }
}

/// The producer side of a [`RingBuffer`].
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can only be created with [`RingBuffer::split`]
/// (together with its counterpart, the [`Consumer`]).
///
/// # Examples
///
/// ```
/// use rtrb::RingBuffer;
///
/// let (producer, consumer) = RingBuffer::<f32>::new(1000).split();
/// ```
pub struct Producer<T> {
    /// The inner representation of the queue.
    rb: Arc<RingBuffer<T>>,

    /// A copy of `rb.head` for quick access.
    ///
    /// This value can be stale and sometimes needs to be resynchronized with `rb.head`.
    head: Cell<usize>,

    /// A copy of `rb.tail` for quick access.
    ///
    /// This value is always in sync with `rb.tail`.
    tail: Cell<usize>,
}

unsafe impl<T: Send> Send for Producer<T> {}

impl<T> Producer<T> {
    /// Attempts to push an element into the queue.
    ///
    /// The element is *moved* into the ring buffer and its slot
    /// is made available to be read by the [`Consumer`].
    /// If the queue is full, the element is returned back as an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{RingBuffer, PushError};
    ///
    /// let (mut p, c) = RingBuffer::new(1).split();
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(p.push(20), Err(PushError::Full(20)));
    /// ```
    pub fn push(&mut self, value: T) -> Result<(), PushError<T>> {
        if let Some(tail) = self.next_tail() {
            unsafe {
                self.rb.slot_ptr(tail).write(value);
            }
            let tail = self.rb.increment1(tail);
            self.rb.tail.store(tail, Ordering::Release);
            self.tail.set(tail);
            Ok(())
        } else {
            Err(PushError::Full(value))
        }
    }

    /// Returns `n` slots (initially containing their [`Default`] value) for writing.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`WriteChunk::as_mut_slices`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be read by the [`Consumer`].
    /// This has to be explicitly done by calling [`WriteChunk::commit`]
    /// or [`WriteChunk::commit_all`].
    ///
    /// The type parameter `T` has a trait bound of [`Copy`],
    /// which makes sure that no destructors are called at any time
    /// (because it implies [`!Drop`](Drop)).
    ///
    /// For an unsafe alternative that has no restrictions on `T`,
    /// see [`Producer::write_chunk_maybe_uninit`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut p, mut c) = RingBuffer::new(3).split();
    ///
    /// assert!(p.push(10).is_ok());
    /// assert_eq!(c.pop(), Ok(10));
    ///
    /// if let Ok(mut chunk) = p.write_chunk(3) {
    ///     let (first, second) = chunk.as_mut_slices();
    ///     assert_eq!(first.len(), 2);
    ///     first[0] = 20;
    ///     first[1] += 30; // Default value is 0
    ///     assert_eq!(second.len(), 1);
    ///     second[0] = 40;
    ///     chunk.commit_all(); // Make written items available for reading
    /// } else {
    ///     unreachable!();
    /// }
    ///
    /// assert_eq!(c.pop(), Ok(20));
    /// assert_eq!(c.pop(), Ok(30));
    /// assert_eq!(c.pop(), Ok(40));
    /// ```
    pub fn write_chunk(&mut self, n: usize) -> Result<WriteChunk<'_, T>, ChunkError>
    where
        T: Copy + Default,
    {
        self.write_chunk_maybe_uninit(n).map(WriteChunk)
    }

    /// Returns `n` (possibly uninitialized) slots for writing.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`WriteChunkMaybeUninit::as_mut_slices`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be read by the [`Consumer`].
    /// This has to be explicitly done by calling [`WriteChunkMaybeUninit::commit`]
    /// or [`WriteChunkMaybeUninit::commit_all`].
    ///
    /// # Safety
    ///
    /// This function itself is safe, but accessing the returned slots might not be,
    /// as well as invoking some methods of [`WriteChunkMaybeUninit`].
    ///
    /// For a safe alternative that provides only initialized slots,
    /// see [`Producer::write_chunk`].
    pub fn write_chunk_maybe_uninit(
        &mut self,
        n: usize,
    ) -> Result<WriteChunkMaybeUninit<'_, T>, ChunkError> {
        let tail = self.tail.get();

        // Check if the queue has *possibly* not enough slots.
        if self.rb.capacity - self.rb.distance(self.head.get(), tail) < n {
            // Refresh the head ...
            let head = self.rb.head.load(Ordering::Acquire);
            self.head.set(head);

            // ... and check if there *really* are not enough slots.
            let slots = self.rb.capacity - self.rb.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }
        let tail = self.rb.collapse_position(tail);
        let first_len = n.min(self.rb.capacity - tail);
        Ok(WriteChunkMaybeUninit {
            first_ptr: unsafe { self.rb.buffer.add(tail) },
            first_len,
            second_ptr: self.rb.buffer,
            second_len: n - first_len,
            producer: self,
        })
    }

    /// Returns the number of slots available for writing.
    ///
    /// To check for a single available slot,
    /// using [`Producer::is_full`] is often quicker
    /// (because it might not have to check an atomic variable).
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(1024).split();
    ///
    /// assert_eq!(p.slots(), 1024);
    /// ```
    pub fn slots(&self) -> usize {
        let head = self.rb.head.load(Ordering::Acquire);
        self.head.set(head);
        self.rb.capacity - self.rb.distance(head, self.tail.get())
    }

    /// Returns `true` if there are no slots available for writing.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(1).split();
    ///
    /// assert!(!p.is_full());
    /// ```
    pub fn is_full(&self) -> bool {
        self.next_tail().is_none()
    }

    /// Returns the capacity of the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(100).split();
    /// assert_eq!(p.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.rb.capacity
    }

    /// Get the tail position for writing the next slot, if available.
    ///
    /// This is a strict subset of the functionality implemented in write_chunk().
    /// For performance, this special case is immplemented separately.
    fn next_tail(&self) -> Option<usize> {
        let tail = self.tail.get();

        // Check if the queue is *possibly* full.
        if self.rb.distance(self.head.get(), tail) == self.rb.capacity {
            // Refresh the head ...
            let head = self.rb.head.load(Ordering::Acquire);
            self.head.set(head);

            // ... and check if it's *really* full.
            if self.rb.distance(head, tail) == self.rb.capacity {
                return None;
            }
        }
        Some(tail)
    }
}

impl<T> fmt::Debug for Producer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("Producer { .. }")
    }
}

/// The consumer side of a [`RingBuffer`].
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can only be created with [`RingBuffer::split`]
/// (together with its counterpart, the [`Producer`]).
///
/// # Examples
///
/// ```
/// use rtrb::RingBuffer;
///
/// let (producer, consumer) = RingBuffer::<f32>::new(1000).split();
/// ```
pub struct Consumer<T> {
    /// The inner representation of the queue.
    rb: Arc<RingBuffer<T>>,

    /// A copy of `rb.head` for quick access.
    ///
    /// This value is always in sync with `rb.head`.
    head: Cell<usize>,

    /// A copy of `rb.tail` for quick access.
    ///
    /// This value can be stale and sometimes needs to be resynchronized with `rb.tail`.
    tail: Cell<usize>,
}

unsafe impl<T: Send> Send for Consumer<T> {}

impl<T> Consumer<T> {
    /// Attempts to pop an element from the queue.
    ///
    /// The element is *moved* out of the ring buffer and its slot
    /// is made available to be filled by the [`Producer`] again.
    /// If the queue is empty, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PopError, RingBuffer};
    ///
    /// let (mut p, mut c) = RingBuffer::new(1).split();
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(c.pop(), Ok(10));
    /// assert_eq!(c.pop(), Err(PopError::Empty));
    /// ```
    ///
    /// To obtain an [`Option<T>`](Option), use [`.ok()`](Result::ok) on the result.
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut p, mut c) = RingBuffer::new(1).split();
    /// assert_eq!(p.push(20), Ok(()));
    /// assert_eq!(c.pop().ok(), Some(20));
    /// ```
    pub fn pop(&mut self) -> Result<T, PopError> {
        if let Some(head) = self.next_head() {
            let value = unsafe { self.rb.slot_ptr(head).read() };
            let head = self.rb.increment1(head);
            self.rb.head.store(head, Ordering::Release);
            self.head.set(head);
            Ok(value)
        } else {
            Err(PopError::Empty)
        }
    }

    /// Attempts to read an element from the queue without removing it.
    ///
    /// If the queue is empty, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PeekError, RingBuffer};
    ///
    /// let (mut p, c) = RingBuffer::new(1).split();
    ///
    /// assert_eq!(c.peek(), Err(PeekError::Empty));
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(c.peek(), Ok(&10));
    /// assert_eq!(c.peek(), Ok(&10));
    /// ```
    pub fn peek(&self) -> Result<&T, PeekError> {
        if let Some(head) = self.next_head() {
            Ok(unsafe { &*self.rb.slot_ptr(head) })
        } else {
            Err(PeekError::Empty)
        }
    }

    /// Returns `n` slots for reading.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`ReadChunk::as_slices`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be written again by the [`Producer`].
    /// This has to be explicitly done by calling [`ReadChunk::commit`]
    /// or [`ReadChunk::commit_all`].  You can "peek" at the contained values
    /// by simply not calling any of the "commit" methods.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{RingBuffer, ChunkError};
    ///
    /// let (mut p, mut c) = RingBuffer::new(3).split();
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(c.read_chunk(2).unwrap_err(), ChunkError::TooFewSlots(1));
    /// assert_eq!(p.push(20), Ok(()));
    ///
    /// if let Ok(chunk) = c.read_chunk(2) {
    ///     let (first, second) = chunk.as_slices();
    ///     assert_eq!(first, &[10, 20]);
    ///     assert_eq!(second, &[]);
    ///     chunk.commit_all(); // Make the whole chunk available for writing again
    /// } else {
    ///     unreachable!();
    /// }
    ///
    /// assert_eq!(c.read_chunk(2).unwrap_err(), ChunkError::TooFewSlots(0));
    /// assert_eq!(p.push(30), Ok(()));
    /// assert_eq!(p.push(40), Ok(()));
    ///
    /// if let Ok(chunk) = c.read_chunk(2) {
    ///     let (first, second) = chunk.as_slices();
    ///     assert_eq!(first, &[30]);
    ///     assert_eq!(second, &[40]);
    ///     chunk.commit(1); // Only one slot is made available for writing ...
    /// } else {
    ///     unreachable!();
    /// };
    ///
    /// // ... which means the last element is still in the queue:
    /// assert_eq!(c.pop(), Ok(40));
    /// ```
    ///
    /// Items are dropped when [`ReadChunk::commit`] or [`ReadChunk::commit_all`] is called
    /// (which is only relevant if `T` implements [`Drop`]).
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// // Static variable to count all drop() invocations
    /// static mut DROP_COUNT: i32 = 0;
    /// #[derive(Debug)]
    /// struct Thing;
    /// impl Drop for Thing {
    ///     fn drop(&mut self) { unsafe { DROP_COUNT += 1; } }
    /// }
    ///
    /// // Scope to limit lifetime of ring buffer
    /// {
    ///     let (mut p, mut c) = RingBuffer::new(2).split();
    ///
    ///     assert!(p.push(Thing).is_ok()); // 1
    ///     assert!(p.push(Thing).is_ok()); // 2
    ///     if let Ok(thing) = c.pop() {
    ///         // "thing" has been *moved* out of the queue but not yet dropped
    ///         assert_eq!(unsafe { DROP_COUNT }, 0);
    ///     } else {
    ///         unreachable!();
    ///     }
    ///     // First Thing has been dropped when "thing" went out of scope:
    ///     assert_eq!(unsafe { DROP_COUNT }, 1);
    ///     assert!(p.push(Thing).is_ok()); // 3
    ///
    ///     if let Ok(chunk) = c.read_chunk(2) {
    ///         let (first, second) = chunk.as_slices();
    ///         assert_eq!(first.len(), 1);
    ///         assert_eq!(second.len(), 1);
    ///         assert_eq!(unsafe { DROP_COUNT }, 1);
    ///         chunk.commit(1); // Drops only one of the two Things
    ///         assert_eq!(unsafe { DROP_COUNT }, 2);
    ///     } else {
    ///         unreachable!();
    ///     }
    ///     // The last Thing is still in the queue ...
    ///     assert_eq!(unsafe { DROP_COUNT }, 2);
    /// }
    /// // ... and it is dropped when the ring buffer goes out of scope:
    /// assert_eq!(unsafe { DROP_COUNT }, 3);
    /// ```
    pub fn read_chunk(&mut self, n: usize) -> Result<ReadChunk<'_, T>, ChunkError> {
        let head = self.head.get();

        // Check if the queue has *possibly* not enough slots.
        if self.rb.distance(head, self.tail.get()) < n {
            // Refresh the tail ...
            let tail = self.rb.tail.load(Ordering::Acquire);
            self.tail.set(tail);

            // ... and check if there *really* are not enough slots.
            let slots = self.rb.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }

        let head = self.rb.collapse_position(head);
        let first_len = n.min(self.rb.capacity - head);
        Ok(ReadChunk {
            first_ptr: unsafe { self.rb.buffer.add(head) },
            first_len,
            second_ptr: self.rb.buffer,
            second_len: n - first_len,
            consumer: self,
        })
    }

    /// Returns the number of slots available for reading.
    ///
    /// To check for a single available slot,
    /// using [`Consumer::is_empty`] is often quicker
    /// (because it might not have to check an atomic variable).
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(1024).split();
    ///
    /// assert_eq!(c.slots(), 0);
    /// ```
    pub fn slots(&self) -> usize {
        let tail = self.rb.tail.load(Ordering::Acquire);
        self.tail.set(tail);
        self.rb.distance(self.head.get(), tail)
    }

    /// Returns `true` if there are no slots available for reading.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(1).split();
    ///
    /// assert!(c.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.next_head().is_none()
    }

    /// Returns the capacity of the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<f32>::new(100).split();
    /// assert_eq!(c.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.rb.capacity
    }

    /// Get the head position for reading the next slot, if available.
    ///
    /// This is a strict subset of the functionality implemented in read_chunk().
    /// For performance, this special case is immplemented separately.
    fn next_head(&self) -> Option<usize> {
        let head = self.head.get();

        // Check if the queue is *possibly* empty.
        if head == self.tail.get() {
            // Refresh the tail ...
            let tail = self.rb.tail.load(Ordering::Acquire);
            self.tail.set(tail);

            // ... and check if it's *really* empty.
            if head == tail {
                return None;
            }
        }
        Some(head)
    }
}

/// Structure for writing into multiple slots in one go.
///
/// This is returned from [`Producer::write_chunk`].
///
/// For an unsafe alternative that provides possibly uninitialized slots,
/// see [`WriteChunkMaybeUninit`].
#[derive(Debug)]
pub struct WriteChunk<'a, T>(WriteChunkMaybeUninit<'a, T>);

impl<'a, T> WriteChunk<'a, T>
where
    T: Copy + Default,
{
    /// Returns two slices for writing to the requested slots.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    ///
    /// All slots are initially filled with their [`Default`] value.
    pub fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) {
        let (first, second) = self.0.as_mut_slices();
        for i in first.iter_mut().chain(second.iter_mut()) {
            unsafe {
                i.as_mut_ptr().write(Default::default());
            }
        }
        unsafe {
            (
                &mut *(first as *mut _ as *mut _),
                &mut *(second as *mut _ as *mut _),
            )
        }
    }

    /// Makes the given number of slots available for reading.
    pub fn commit(self, n: usize) {
        // Safety: All slots have been initialized in as_mut_slices() and there are no destructors.
        unsafe { self.0.commit(n) }
    }

    /// Makes the whole chunk available for reading.
    pub fn commit_all(self) {
        // Safety: All slots have been initialized in as_mut_slices().
        unsafe { self.0.commit_all() }
    }
}

/// Structure for writing into multiple (possibly uninitialized) slots in one go.
///
/// This is returned from [`Producer::write_chunk_maybe_uninit`].
///
/// For a safe alternative that only provides initialized slots, see [`WriteChunk`].
#[derive(Debug)]
pub struct WriteChunkMaybeUninit<'a, T> {
    first_ptr: *mut T,
    first_len: usize,
    second_ptr: *mut T,
    second_len: usize,
    producer: &'a Producer<T>,
}

impl<'a, T> WriteChunkMaybeUninit<'a, T> {
    /// Returns two slices for writing to the requested slots.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    pub fn as_mut_slices(&mut self) -> (&mut [MaybeUninit<T>], &mut [MaybeUninit<T>]) {
        unsafe {
            (
                std::slice::from_raw_parts_mut(self.first_ptr as *mut _, self.first_len),
                std::slice::from_raw_parts_mut(self.second_ptr as *mut _, self.second_len),
            )
        }
    }

    /// Makes the given number of slots available for reading.
    ///
    /// # Safety
    ///
    /// The user must make sure that the first `n` elements
    /// (and not more, in case `T` implements [`Drop`]) have been initialized.
    pub unsafe fn commit(self, n: usize) {
        let tail = self.producer.rb.increment(self.producer.tail.get(), n);
        self.producer.rb.tail.store(tail, Ordering::Release);
        self.producer.tail.set(tail);
    }

    /// Makes the whole chunk available for reading.
    ///
    /// # Safety
    ///
    /// The user must make sure that all elements have been initialized.
    pub unsafe fn commit_all(self) {
        let n = self.first_len + self.second_len;
        self.commit(n)
    }
}

/// Structure for reading from multiple slots in one go.
///
/// This is returned from [`Consumer::read_chunk`].
#[derive(Debug)]
pub struct ReadChunk<'a, T> {
    first_ptr: *const T,
    first_len: usize,
    second_ptr: *const T,
    second_len: usize,
    consumer: &'a mut Consumer<T>,
}

impl<'a, T> ReadChunk<'a, T> {
    /// Returns two slices for reading from the requested slots.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    pub fn as_slices(&self) -> (&[T], &[T]) {
        (
            unsafe { std::slice::from_raw_parts(self.first_ptr, self.first_len) },
            unsafe { std::slice::from_raw_parts(self.second_ptr, self.second_len) },
        )
    }

    /// Drops the given number of slots, making the space available for writing again.
    pub fn commit(self, n: usize) {
        let head = self.consumer.head.get();
        // Safety: head has not yet been incremented
        let ptr = unsafe { self.consumer.rb.slot_ptr(head) };
        let first_len = self.first_len.min(n);
        for i in 0..first_len {
            unsafe {
                ptr.add(i).drop_in_place();
            }
        }
        let ptr = self.consumer.rb.buffer;
        let second_len = self.second_len.min(n - first_len);
        for i in 0..second_len {
            unsafe {
                ptr.add(i).drop_in_place();
            }
        }
        let head = self.consumer.rb.increment(head, n);
        self.consumer.rb.head.store(head, Ordering::Release);
        self.consumer.head.set(head);
    }

    /// Drops all slots of the chunk, making the space available for writing again.
    pub fn commit_all(self) {
        let n = self.first_len + self.second_len;
        self.commit(n)
    }
}

impl<T> fmt::Debug for Consumer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("Consumer { .. }")
    }
}
