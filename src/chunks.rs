//! Writing and reading multiple items at once into and from a [`RingBuffer`].
//!
//! Multiple items can be written at once by using [`Producer::write_chunk()`]
//! or its `unsafe` (but probably slightly faster) cousin [`Producer::write_chunk_uninit()`].
//!
//! Multiple items can be read at once with [`Consumer::read_chunk()`].
//!
//! # Examples
//!
//! Producing and consuming multiple items at once with
//! [`Producer::write_chunk()`] and [`Consumer::read_chunk()`], respectively.
//! This example uses a single thread for simplicity, but in a real application,
//! `producer` and `consumer` would of course live on different threads:
//!
//! ```
//! use rtrb::RingBuffer;
//!
//! let (mut producer, mut consumer) = RingBuffer::new(5);
//!
//! if let Ok(mut chunk) = producer.write_chunk(4) {
//!     // NB: Don't use `chunk` as the first iterator in zip() if the other one might be shorter!
//!     for (src, dst) in vec![10, 11, 12].into_iter().zip(&mut chunk) {
//!         *dst = src;
//!     }
//!     // Don't forget to make the written slots available for reading!
//!     chunk.commit_iterated();
//!     // Note that we requested 4 slots but we've only written 3!
//! } else {
//!     unreachable!();
//! }
//!
//! assert_eq!(producer.slots(), 2);
//! assert_eq!(consumer.slots(), 3);
//!
//! if let Ok(mut chunk) = consumer.read_chunk(2) {
//!     // NB: Even though we are just reading, `chunk` needs to be mutable for iteration!
//!     assert_eq!((&mut chunk).collect::<Vec<_>>(), [&10, &11]);
//!     chunk.commit_iterated(); // Mark the slots as "consumed"
//!     // chunk.commit_all() would also work here.
//! } else {
//!     unreachable!();
//! }
//!
//! // One element is still in the queue:
//! assert_eq!(consumer.peek(), Ok(&12));
//!
//! let data = vec![20, 21, 22, 23];
//! if let Ok(mut chunk) = producer.write_chunk(4) {
//!     let (first, second) = chunk.as_mut_slices();
//!     let mid = first.len();
//!     first.copy_from_slice(&data[..mid]);
//!     second.copy_from_slice(&data[mid..]);
//!     chunk.commit_all();
//! } else {
//!     unreachable!();
//! }
//!
//! assert!(producer.is_full());
//! assert_eq!(consumer.slots(), 5);
//!
//! let mut v = Vec::<i32>::with_capacity(5);
//! if let Ok(chunk) = consumer.read_chunk(5) {
//!     let (first, second) = chunk.as_slices();
//!     v.extend(first);
//!     v.extend(second);
//!     chunk.commit_all();
//! } else {
//!     unreachable!();
//! }
//! assert_eq!(v, [12, 20, 21, 22, 23]);
//! assert!(consumer.is_empty());
//! ```
//!
//! ## Common Access Patterns
//!
//! The following examples show the [`Producer`] side;
//! similar patterns can of course be used with [`Consumer::read_chunk()`] as well.
//! Furthermore, the examples use [`Producer::write_chunk()`],
//! which requires the trait bound `T: Default`.
//! If that's too restrictive or if you want to squeeze out the last bit of performance,
//! you can use [`Producer::write_chunk_uninit()`] instead,
//! but this will force you to write some `unsafe` code.
//!
//! Copy a whole slice of items into the ring buffer, but only if space permits
//! (if not, the input slice is returned as an error):
//!
//! ```
//! use rtrb::Producer;
//!
//! fn push_entire_slice<'a, T>(queue: &mut Producer<T>, slice: &'a [T]) -> Result<(), &'a [T]>
//! where
//!     T: Copy + Default,
//! {
//!     if let Ok(mut chunk) = queue.write_chunk(slice.len()) {
//!         let (first, second) = chunk.as_mut_slices();
//!         let mid = first.len();
//!         first.copy_from_slice(&slice[..mid]);
//!         second.copy_from_slice(&slice[mid..]);
//!         chunk.commit_all();
//!         Ok(())
//!     } else {
//!         Err(slice)
//!     }
//! }
//! ```
//!
//! Copy as many items as possible from a given slice, returning the remainder of the slice
//! (which will be empty if there was space for all items):
//!
//! ```
//! use rtrb::{Producer, chunks::ChunkError::TooFewSlots};
//!
//! fn push_partial_slice<'a, T>(queue: &mut Producer<T>, slice: &'a [T]) -> &'a [T]
//! where
//!     T: Copy + Default,
//! {
//!     let mut chunk = match queue.write_chunk(slice.len()) {
//!         Ok(chunk) => chunk,
//!         // This is an optional optimization if the queue tends to be full:
//!         Err(TooFewSlots(0)) => return slice,
//!         // Remaining slots are returned, this will always succeed:
//!         Err(TooFewSlots(n)) => queue.write_chunk(n).unwrap(),
//!     };
//!     let end = chunk.len();
//!     let (first, second) = chunk.as_mut_slices();
//!     let mid = first.len();
//!     first.copy_from_slice(&slice[..mid]);
//!     second.copy_from_slice(&slice[mid..end]);
//!     chunk.commit_all();
//!     &slice[end..]
//! }
//! ```
//!
//! Write as many slots as possible, given an iterator
//! (and return the number of written slots):
//!
//! ```
//! use rtrb::{Producer, chunks::ChunkError::TooFewSlots};
//!
//! fn push_from_iter<T, I>(queue: &mut Producer<T>, iter: &mut I) -> usize
//! where
//!     T: Default,
//!     I: Iterator<Item = T>,
//! {
//!     let n = match iter.size_hint() {
//!         (_, None) => queue.slots(),
//!         (_, Some(n)) => n,
//!     };
//!     let mut chunk = match queue.write_chunk(n) {
//!         Ok(chunk) => chunk,
//!         // As above, this is an optional optimization:
//!         Err(TooFewSlots(0)) => return 0,
//!         // As above, this will always succeed:
//!         Err(TooFewSlots(n)) => queue.write_chunk(n).unwrap(),
//!     };
//!     for (source, target) in iter.zip(&mut chunk) {
//!         *target = source;
//!     }
//!     chunk.commit_iterated()
//! }
//! ```

use core::fmt;
use core::mem::MaybeUninit;
use core::sync::atomic::Ordering;

use crate::{Consumer, CopyToUninit, Producer};

// This is used in the documentation.
#[allow(unused_imports)]
use crate::RingBuffer;

impl<T> Producer<T> {
    /// Returns `n` slots (initially containing their [`Default`] value) for writing.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`WriteChunk::as_mut_slices()`] or
    /// by iterating over (a `&mut` to) the [`WriteChunk`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be read by the [`Consumer`].
    /// This has to be explicitly done by calling [`WriteChunk::commit()`],
    /// [`WriteChunk::commit_iterated()`] or [`WriteChunk::commit_all()`].
    ///
    /// For an unsafe alternative that has no restrictions on `T`,
    /// see [`Producer::write_chunk_uninit()`].
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](crate::chunks#examples) module for more examples.
    pub fn write_chunk(&mut self, n: usize) -> Result<WriteChunk<'_, T>, ChunkError>
    where
        T: Default,
    {
        self.write_chunk_uninit(n).map(WriteChunk::from)
    }

    /// Returns `n` (uninitialized) slots for writing.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`WriteChunkUninit::as_mut_slices()`] or
    /// by iterating over (a `&mut` to) the [`WriteChunkUninit`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be read by the [`Consumer`].
    /// This has to be explicitly done by calling [`WriteChunkUninit::commit()`],
    /// [`WriteChunkUninit::commit_iterated()`] or
    /// [`WriteChunkUninit::commit_all()`].
    ///
    /// # Safety
    ///
    /// This function itself is safe, but accessing the returned slots might not be,
    /// as well as invoking some methods of [`WriteChunkUninit`].
    ///
    /// For a safe alternative that provides [`Default`]-initialized slots,
    /// see [`Producer::write_chunk()`].
    pub fn write_chunk_uninit(&mut self, n: usize) -> Result<WriteChunkUninit<'_, T>, ChunkError> {
        let tail = self.tail.get();

        // Check if the queue has *possibly* not enough slots.
        if self.buffer.capacity - self.buffer.distance(self.head.get(), tail) < n {
            // Refresh the head ...
            let head = self.buffer.head.load(Ordering::Acquire);
            self.head.set(head);

            // ... and check if there *really* are not enough slots.
            let slots = self.buffer.capacity - self.buffer.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }
        let tail = self.buffer.collapse_position(tail);
        let first_len = n.min(self.buffer.capacity - tail);
        Ok(WriteChunkUninit {
            first_ptr: unsafe { self.buffer.data_ptr.add(tail) },
            first_len,
            second_ptr: self.buffer.data_ptr,
            second_len: n - first_len,
            producer: self,
            iterated: 0,
        })
    }
}

impl<T> Consumer<T> {
    /// Returns `n` slots for reading.
    ///
    /// If not enough slots are available, an error
    /// (containing the number of available slots) is returned.
    ///
    /// The elements can be accessed with [`ReadChunk::as_slices()`] or
    /// by iterating over (a `&mut` to) the [`ReadChunk`].
    ///
    /// The provided slots are *not* automatically made available
    /// to be written again by the [`Producer`].
    /// This has to be explicitly done by calling [`ReadChunk::commit()`],
    /// [`ReadChunk::commit_iterated()`] or [`ReadChunk::commit_all()`].
    /// You can "peek" at the contained values by simply
    /// not calling any of the "commit" methods.
    ///
    /// # Examples
    ///
    /// Items are dropped when [`ReadChunk::commit()`], [`ReadChunk::commit_iterated()`]
    /// or [`ReadChunk::commit_all()`] is called
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
    ///     let (mut p, mut c) = RingBuffer::new(2);
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
    ///         assert_eq!(chunk.len(), 2);
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
    ///
    /// See the documentation of the [`chunks`](crate::chunks#examples) module for more examples.
    pub fn read_chunk(&mut self, n: usize) -> Result<ReadChunk<'_, T>, ChunkError> {
        let head = self.head.get();

        // Check if the queue has *possibly* not enough slots.
        if self.buffer.distance(head, self.tail.get()) < n {
            // Refresh the tail ...
            let tail = self.buffer.tail.load(Ordering::Acquire);
            self.tail.set(tail);

            // ... and check if there *really* are not enough slots.
            let slots = self.buffer.distance(head, tail);
            if slots < n {
                return Err(ChunkError::TooFewSlots(slots));
            }
        }

        let head = self.buffer.collapse_position(head);
        let first_len = n.min(self.buffer.capacity - head);
        Ok(ReadChunk {
            first_ptr: unsafe { self.buffer.data_ptr.add(head) },
            first_len,
            second_ptr: self.buffer.data_ptr,
            second_len: n - first_len,
            consumer: self,
            iterated: 0,
        })
    }
}

/// Structure for writing into multiple ([`Default`]-initialized) slots in one go.
///
/// This is returned from [`Producer::write_chunk()`].
///
/// For an unsafe alternative that provides uninitialized slots,
/// see [`WriteChunkUninit`].
///
/// The slots (which initially contain [`Default`] values) can be accessed with
/// [`as_mut_slices()`](WriteChunk::as_mut_slices)
/// or by iteration, which yields mutable references (in other words: `&mut T`).
/// A mutable reference (`&mut`) to the `WriteChunk`
/// should be used to iterate over it.
/// Each slot can only be iterated once and the number of iterations is tracked.
///
/// After writing, the provided slots are *not* automatically made available
/// to be read by the [`Consumer`].
/// If desired, this has to be explicitly done by calling
/// [`commit()`](WriteChunk::commit),
/// [`commit_iterated()`](WriteChunk::commit_iterated) or
/// [`commit_all()`](WriteChunk::commit_all).
#[derive(Debug)]
pub struct WriteChunk<'a, T>(WriteChunkUninit<'a, T>);

impl<'a, T> From<WriteChunkUninit<'a, T>> for WriteChunk<'a, T>
where
    T: Default,
{
    /// Fills all slots with the [`Default`] value.
    fn from(chunk: WriteChunkUninit<'a, T>) -> Self {
        for i in 0..chunk.first_len {
            unsafe {
                chunk.first_ptr.add(i).write(Default::default());
            }
        }
        for i in 0..chunk.second_len {
            unsafe {
                chunk.second_ptr.add(i).write(Default::default());
            }
        }
        WriteChunk(chunk)
    }
}

impl<T> WriteChunk<'_, T>
where
    T: Default,
{
    /// Returns two slices for writing to the requested slots.
    ///
    /// All slots are initially filled with their [`Default`] value.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    pub fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) {
        // Safety: All slots have been initialized in From::from().
        unsafe {
            (
                core::slice::from_raw_parts_mut(self.0.first_ptr, self.0.first_len),
                core::slice::from_raw_parts_mut(self.0.second_ptr, self.0.second_len),
            )
        }
    }

    /// Makes the first `n` slots of the chunk available for reading.
    ///
    /// # Panics
    ///
    /// Panics if `n` is greater than the number of slots in the chunk.
    pub fn commit(self, n: usize) {
        // Safety: All slots have been initialized in From::from() and there are no destructors.
        unsafe { self.0.commit(n) }
    }

    /// Returns the number of iterated slots and makes them available for reading.
    pub fn commit_iterated(self) -> usize {
        // Safety: All slots have been initialized in From::from() and there are no destructors.
        unsafe { self.0.commit_iterated() }
    }

    /// Makes the whole chunk available for reading.
    pub fn commit_all(self) {
        // Safety: All slots have been initialized in From::from().
        unsafe { self.0.commit_all() }
    }

    /// Returns the number of slots in the chunk.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the chunk contains no slots.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a, T> Iterator for WriteChunk<'a, T>
where
    T: Default,
{
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|item| {
            // Safety: All slots have been initialized in From::from().
            unsafe { &mut *item.as_mut_ptr() }
        })
    }
}

/// Structure for writing into multiple (uninitialized) slots in one go.
///
/// This is returned from [`Producer::write_chunk_uninit()`].
///
/// For a safe alternative that provides [`Default`]-initialized slots, see [`WriteChunk`].
///
/// The slots can be accessed with
/// [`as_mut_slices()`](WriteChunkUninit::as_mut_slices)
/// or by iteration, which yields mutable references to possibly uninitialized data
/// (in other words: `&mut MaybeUninit<T>`).
/// A mutable reference (`&mut`) to the `WriteChunkUninit`
/// should be used to iterate over it.
/// Each slot can only be iterated once and the number of iterations is tracked.
///
/// After writing, the provided slots are *not* automatically made available
/// to be read by the [`Consumer`].
/// If desired, this has to be explicitly done by calling
/// [`commit()`](WriteChunkUninit::commit),
/// [`commit_iterated()`](WriteChunkUninit::commit_iterated) or
/// [`commit_all()`](WriteChunkUninit::commit_all).
#[derive(Debug)]
pub struct WriteChunkUninit<'a, T> {
    first_ptr: *mut T,
    first_len: usize,
    second_ptr: *mut T,
    second_len: usize,
    producer: &'a Producer<T>,
    iterated: usize,
}

impl<T> WriteChunkUninit<'_, T> {
    /// Returns two slices for writing to the requested slots.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    ///
    /// The extension trait [`CopyToUninit`] can be used to safely copy data into those slices.
    pub fn as_mut_slices(&mut self) -> (&mut [MaybeUninit<T>], &mut [MaybeUninit<T>]) {
        unsafe {
            (
                core::slice::from_raw_parts_mut(self.first_ptr as *mut _, self.first_len),
                core::slice::from_raw_parts_mut(self.second_ptr as *mut _, self.second_len),
            )
        }
    }

    /// Makes the first `n` slots of the chunk available for reading.
    ///
    /// # Panics
    ///
    /// Panics if `n` is greater than the number of slots in the chunk.
    ///
    /// # Safety
    ///
    /// The user must make sure that the first `n` elements
    /// (and not more, in case `T` implements [`Drop`]) have been initialized.
    pub unsafe fn commit(self, n: usize) {
        assert!(n <= self.len(), "cannot commit more than chunk size");
        self.commit_unchecked(n);
    }

    /// Returns the number of iterated slots and makes them available for reading.
    ///
    /// # Safety
    ///
    /// The user must make sure that all iterated elements have been initialized.
    pub unsafe fn commit_iterated(self) -> usize {
        let slots = self.iterated;
        self.commit_unchecked(slots)
    }

    /// Makes the whole chunk available for reading.
    ///
    /// # Safety
    ///
    /// The user must make sure that all elements have been initialized.
    pub unsafe fn commit_all(self) {
        let slots = self.len();
        self.commit_unchecked(slots);
    }

    unsafe fn commit_unchecked(self, n: usize) -> usize {
        let tail = self.producer.buffer.increment(self.producer.tail.get(), n);
        self.producer.buffer.tail.store(tail, Ordering::Release);
        self.producer.tail.set(tail);
        n
    }

    /// Returns the number of slots in the chunk.
    pub fn len(&self) -> usize {
        self.first_len + self.second_len
    }

    /// Returns `true` if the chunk contains no slots.
    pub fn is_empty(&self) -> bool {
        self.first_len == 0
    }
}

impl<'a, T> Iterator for WriteChunkUninit<'a, T> {
    type Item = &'a mut MaybeUninit<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = if self.iterated < self.first_len {
            unsafe { self.first_ptr.add(self.iterated) }
        } else if self.iterated < self.first_len + self.second_len {
            unsafe { self.second_ptr.add(self.iterated - self.first_len) }
        } else {
            return None;
        };
        self.iterated += 1;
        Some(unsafe { &mut *(ptr as *mut _) })
    }
}

/// Structure for reading from multiple slots in one go.
///
/// This is returned from [`Consumer::read_chunk()`].
///
/// The slots can be accessed with [`as_slices()`](ReadChunk::as_slices)
/// or by iteration.
/// Even though iterating yields immutable references (`&T`),
/// a mutable reference (`&mut`) to the `ReadChunk` should be used to iterate over it.
/// Each slot can only be iterated once and the number of iterations is tracked.
///
/// After reading, the provided slots are *not* automatically made available
/// to be written again by the [`Producer`].
/// If desired, this has to be explicitly done by calling [`commit()`](ReadChunk::commit),
/// [`commit_iterated()`](ReadChunk::commit_iterated) or [`commit_all()`](ReadChunk::commit_all).
/// Note that this runs the destructor of the committed items (if `T` implements [`Drop`]).
#[derive(Debug, PartialEq, Eq)]
pub struct ReadChunk<'a, T> {
    first_ptr: *const T,
    first_len: usize,
    second_ptr: *const T,
    second_len: usize,
    consumer: &'a mut Consumer<T>,
    iterated: usize,
}

impl<T> ReadChunk<'_, T> {
    /// Returns two slices for reading from the requested slots.
    ///
    /// The first slice can only be empty if `0` slots have been requested.
    /// If the first slice contains all requested slots, the second one is empty.
    pub fn as_slices(&self) -> (&[T], &[T]) {
        (
            unsafe { core::slice::from_raw_parts(self.first_ptr, self.first_len) },
            unsafe { core::slice::from_raw_parts(self.second_ptr, self.second_len) },
        )
    }

    /// Drops the first `n` slots of the chunk, making the space available for writing again.
    ///
    /// # Panics
    ///
    /// Panics if `n` is greater than the number of slots in the chunk.
    pub fn commit(self, n: usize) {
        assert!(n <= self.len(), "cannot commit more than chunk size");
        unsafe { self.commit_unchecked(n) };
    }

    /// Drops all slots that have been iterated, making the space available for writing again.
    ///
    /// Returns the number of iterated slots.
    pub fn commit_iterated(self) -> usize {
        let slots = self.iterated;
        unsafe { self.commit_unchecked(slots) }
    }

    /// Drops all slots of the chunk, making the space available for writing again.
    pub fn commit_all(self) {
        let slots = self.len();
        unsafe { self.commit_unchecked(slots) };
    }

    unsafe fn commit_unchecked(self, n: usize) -> usize {
        let head = self.consumer.head.get();
        // Safety: head has not yet been incremented
        let ptr = self.consumer.buffer.slot_ptr(head);
        let first_len = self.first_len.min(n);
        for i in 0..first_len {
            ptr.add(i).drop_in_place();
        }
        let ptr = self.consumer.buffer.data_ptr;
        let second_len = self.second_len.min(n - first_len);
        for i in 0..second_len {
            ptr.add(i).drop_in_place();
        }
        let head = self.consumer.buffer.increment(head, n);
        self.consumer.buffer.head.store(head, Ordering::Release);
        self.consumer.head.set(head);
        n
    }

    /// Returns the number of slots in the chunk.
    pub fn len(&self) -> usize {
        self.first_len + self.second_len
    }

    /// Returns `true` if the chunk contains no slots.
    pub fn is_empty(&self) -> bool {
        self.first_len == 0
    }
}

impl<'a, T> Iterator for ReadChunk<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        let ptr = if self.iterated < self.first_len {
            unsafe { self.first_ptr.add(self.iterated) }
        } else if self.iterated < self.first_len + self.second_len {
            unsafe { self.second_ptr.add(self.iterated - self.first_len) }
        } else {
            return None;
        };
        self.iterated += 1;
        Some(unsafe { &*ptr })
    }
}

#[cfg(feature = "std")]
impl std::io::Write for Producer<u8> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use ChunkError::TooFewSlots;
        let mut chunk = match self.write_chunk_uninit(buf.len()) {
            Ok(chunk) => chunk,
            Err(TooFewSlots(0)) => return Err(std::io::ErrorKind::WouldBlock.into()),
            Err(TooFewSlots(n)) => self.write_chunk_uninit(n).unwrap(),
        };
        let end = chunk.len();
        let (first, second) = chunk.as_mut_slices();
        let mid = first.len();
        // NB: If buf.is_empty(), chunk will be empty as well and the following are no-ops:
        buf[..mid].copy_to_uninit(first);
        buf[mid..end].copy_to_uninit(second);
        // Safety: All slots have been initialized
        unsafe {
            chunk.commit_all();
        }
        Ok(end)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        // Nothing to do here.
        Ok(())
    }
}

#[cfg(feature = "std")]
impl std::io::Read for Consumer<u8> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use ChunkError::TooFewSlots;
        let chunk = match self.read_chunk(buf.len()) {
            Ok(chunk) => chunk,
            Err(TooFewSlots(0)) => return Err(std::io::ErrorKind::WouldBlock.into()),
            Err(TooFewSlots(n)) => self.read_chunk(n).unwrap(),
        };
        let (first, second) = chunk.as_slices();
        let mid = first.len();
        let end = chunk.len();
        // NB: If buf.is_empty(), chunk will be empty as well and the following are no-ops:
        buf[..mid].copy_from_slice(first);
        buf[mid..end].copy_from_slice(second);
        chunk.commit_all();
        Ok(end)
    }
}

/// Error type for [`Consumer::read_chunk()`], [`Producer::write_chunk()`]
/// and [`Producer::write_chunk_uninit()`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ChunkError {
    /// Fewer than the requested number of slots were available.
    ///
    /// Contains the number of slots that were available.
    TooFewSlots(usize),
}

#[cfg(feature = "std")]
impl std::error::Error for ChunkError {}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkError::TooFewSlots(n) => {
                alloc::format!("only {} slots available in ring buffer", n).fmt(f)
            }
        }
    }
}
