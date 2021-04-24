//! Writing and reading fixed-size chunks into and from a [`RingBuffer`].
//!
//! An appropriately sized [`RingBuffer`] can be created with
//! [`RingBuffer::with_chunks(...).of_size(...)`](RingBuffer::with_chunks).
//!
//! Chunks can be produced by using [`ChunkProducer::push_chunk()`]
//! or its `unsafe` (but probably slightly faster) cousin [`ChunkProducer::push_chunk_uninit()`].
//!
//! Chunks can be consumed with [`ChunkConsumer::pop_chunk()`].
//!
//! Since only fixed-size chunks are allowed, all chunks are guaranteed to be contiguous in memory.
//! This is not the case for [`Producer::write_chunk()`], [`Producer::write_chunk_uninit()`] and
//! [`Consumer::read_chunk()`], where the ring buffer wrap-around
//! might cause any chunk to be stored in two separate slices.
//!
//! If not all chunks have the same size, or if individual elements should be handled as well,
//! the [`chunks`](crate::chunks) module can be used instead.
//!
//! # Examples
//!
//! The following examples use a single thread for simplicity, but in a real application,
//! the producer and consumer side would of course live on different threads.
//!
//! Producing single items, consuming chunks:
//!
//! ```
//! use rtrb::{RingBuffer, Producer, fixed_chunks::ChunkConsumer};
//!
//! let (mut p, mut c): (Producer<_>, ChunkConsumer<_>) = RingBuffer::with_chunks(3).of_size(2);
//! p.push(10).unwrap();
//! p.push(11).unwrap();
//! p.push(12).unwrap();
//! p.push(13).unwrap();
//!
//! if let Ok(chunk) = c.pop_chunk() {
//!     assert_eq!(*chunk, [10, 11]);
//! } else {
//!     unreachable!();
//! }
//! if let Ok(chunk) = c.peek_chunk() {
//!     assert_eq!(chunk, [12, 13]);
//! } else {
//!     unreachable!();
//! }
//! if let Ok(chunk) = c.pop_chunk() {
//!     assert_eq!(chunk.len(), 2);
//!     assert_eq!(chunk[0], 12);
//!     assert_eq!(chunk[1], 13);
//! } else {
//!     unreachable!();
//! }
//! assert_eq!(c.pop_chunk(), Err(rtrb::fixed_chunks::ChunkError::TooFewSlots(0)));
//! ```
//!
//! Producing chunks, consuming single items:
//!
//! ```
//! use rtrb::{RingBuffer, fixed_chunks::ChunkProducer, Consumer};
//!
//! let (mut p, mut c): (ChunkProducer<_>, Consumer<_>) = RingBuffer::with_chunks(3).of_size(2);
//!
//! if let Ok(mut chunk) = p.push_chunk() {
//!     chunk.copy_from_slice(&[10, 11]);
//! } else {
//!     unreachable!();
//! }
//! if let Ok(mut chunk) = p.push_chunk() {
//!     assert_eq!(chunk.len(), 2);
//!     chunk[0] = 12;
//!     chunk[1] = 13;
//! } else {
//!     unreachable!();
//! }
//!
//! use rtrb::CopyToUninit as _; // This is needed for copy_to_uninit().
//!
//! // Safety: chunk will be initialized before it's dropped.
//! if let Ok(mut chunk) = unsafe { p.push_chunk_uninit() } {
//!     [14, 15].copy_to_uninit(&mut chunk);
//! } else {
//!     unreachable!();
//! }
//!
//! assert_eq!(p.push_chunk(), Err(rtrb::fixed_chunks::ChunkError::TooFewSlots(0)));
//!
//! assert_eq!(c.pop(), Ok(10));
//! assert_eq!(c.pop(), Ok(11));
//! assert_eq!(c.pop(), Ok(12));
//! assert_eq!(c.pop(), Ok(13));
//! assert_eq!(c.pop(), Ok(14));
//! assert_eq!(c.pop(), Ok(15));
//! ```
//!
//! And finally, producing and consuming chunks:
//!
//! ```
//! use rtrb::{RingBuffer, fixed_chunks::ChunkProducer, fixed_chunks::ChunkConsumer};
//!
//! let (mut p, mut c): (ChunkProducer<_>, ChunkConsumer<_>) =
//!     RingBuffer::with_chunks(3).of_size(2);
//!
//! if let Ok(mut chunk) = p.push_chunk() {
//!     chunk.copy_from_slice(&[10, 11]);
//! } else {
//!     unreachable!();
//! }
//! if let Ok(chunk) = c.pop_chunk() {
//!     assert_eq!(chunk.as_ref(), [10, 11]);
//! } else {
//!     unreachable!();
//! }
//! assert_eq!(c.peek_chunk(), Err(rtrb::fixed_chunks::ChunkError::TooFewSlots(0)));
//! ```

use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::Ordering;

use crate::{Consumer, Producer, RingBuffer};

pub use crate::chunks::ChunkError;

impl<T> RingBuffer<T> {
    /// Creates a `RingBuffer` with the given number of fixed-size chunks.
    ///
    /// The call to `with_chunks(...)` has to be followed by `.of_size(...)`,
    /// which will return a pair of producer and consumer.
    /// Multiple combinations of return types are possible:
    ///
    /// * `(`[`Producer<T>`]`, `[`ChunkConsumer<T>`]`)`
    /// * `(`[`ChunkProducer<T>`]`, `[`Consumer<T>`]`)`
    /// * `(`[`ChunkProducer<T>`]`, `[`ChunkConsumer<T>`]`)`
    ///
    /// The desired combination can be selected with type annotations.
    ///
    /// The remaining possible combination `(`[`Producer<T>`]`, `[`Consumer<T>`]`)`
    /// can be created with [`RingBuffer::new()`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{fixed_chunks::ChunkConsumer, Producer, RingBuffer};
    ///
    /// let (p, c): (Producer<_>, ChunkConsumer<_>) = RingBuffer::<f32>::with_chunks(7).of_size(8);
    /// ```
    ///
    /// Typically, the producer and consumer are stored somewhere (probably in a `struct`).
    /// Since the return types are known in this case, no type annotations are necessary
    /// when calling `RingBuffer::with_chunks(...).of_size(...)`:
    ///
    /// ```
    /// use rtrb::{fixed_chunks::ChunkProducer, Consumer, RingBuffer};
    ///
    /// struct OneThing {
    ///     producer: ChunkProducer<f32>,
    /// }
    /// struct AnotherThing {
    ///     consumer: Consumer<f32>,
    /// }
    ///
    /// let (producer, consumer) = RingBuffer::with_chunks(100).of_size(64);
    ///
    /// let thing1 = OneThing { producer };
    /// let thing2 = AnotherThing { consumer };
    /// ```
    ///
    /// See the documentation of the [`fixed_chunks`](crate::fixed_chunks#examples) module
    /// for more examples.
    pub fn with_chunks(chunks: usize) -> Builder<T> {
        Builder {
            chunks,
            _phantom: PhantomData,
        }
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct Builder<T> {
    chunks: usize,
    _phantom: PhantomData<*mut T>,
}

impl<T> Builder<T> {
    /// part of with_chunks(...).of_size(...)
    pub fn of_size<R>(self, chunk_size: usize) -> R
    where
        R: ChunkyRingBufferParts<T>,
    {
        R::from_chunks_and_chunk_size::<private::NotPublic>(self.chunks, chunk_size)
    }
}

// https://jack.wrenn.fyi/blog/private-trait-methods/
pub(crate) mod private {
    pub enum NotPublic {}
    pub trait IsPrivate {}
    impl IsPrivate for NotPublic {}
}

#[doc(hidden)]
pub trait ChunkyRingBufferParts<T> {
    fn from_chunks_and_chunk_size<L>(chunks: usize, chunk_size: usize) -> Self
    where
        L: private::IsPrivate;
}

impl<T> ChunkyRingBufferParts<T> for (ChunkProducer<T>, Consumer<T>) {
    fn from_chunks_and_chunk_size<L>(chunks: usize, chunk_size: usize) -> Self
    where
        L: private::IsPrivate,
    {
        let (p, c) = RingBuffer::new(chunks * chunk_size);
        let p = ChunkProducer {
            inner: p,
            chunk_size,
        };
        (p, c)
    }
}

impl<T> ChunkyRingBufferParts<T> for (Producer<T>, ChunkConsumer<T>) {
    fn from_chunks_and_chunk_size<L>(chunks: usize, chunk_size: usize) -> Self
    where
        L: private::IsPrivate,
    {
        let (p, c) = RingBuffer::new(chunks * chunk_size);
        let c = ChunkConsumer {
            inner: c,
            chunk_size,
        };
        (p, c)
    }
}

impl<T> ChunkyRingBufferParts<T> for (ChunkProducer<T>, ChunkConsumer<T>) {
    fn from_chunks_and_chunk_size<L>(chunks: usize, chunk_size: usize) -> Self
    where
        L: private::IsPrivate,
    {
        let (p, c) = RingBuffer::new(chunks * chunk_size);
        let p = ChunkProducer {
            inner: p,
            chunk_size,
        };
        let c = ChunkConsumer {
            inner: c,
            chunk_size,
        };
        (p, c)
    }
}

/// The producer side of a [`RingBuffer`], using fixed-size chunks.
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can be created with [`RingBuffer::with_chunks(...).of_size(...)`](RingBuffer::with_chunks)
/// (together with one of its counterparts [`Consumer`] or [`ChunkConsumer`]).
///
/// Fixed-size chunks can be produced with [`ChunkProducer::push_chunk()`] or its `unsafe`
/// (but probably slightly faster) cousin [`ChunkProducer::push_chunk_uninit()`].
///
/// If not all chunks have the same size, or if individual elements should be produced,
/// a [`Producer`] can be used instead.
#[derive(Debug, PartialEq, Eq)]
pub struct ChunkProducer<T> {
    inner: Producer<T>,
    chunk_size: usize,
}

unsafe impl<T: Send> Send for ChunkProducer<T> {}

impl<T> ChunkProducer<T> {
    /// Returns a [`Default`]-initialized fixed-size chunk for writing,
    /// advancing the write position when dropped.
    ///
    /// If not enough data for a full chunk is available, an error is returned.
    ///
    /// For mutable access to its contents, the returned [`PushChunk<T>`]
    /// can be (auto-)dereferenced to a [`slice`].
    ///
    /// When the returned [`PushChunk<T>`] is dropped, all slots are made available for reading.
    ///
    /// If uninitialized chunks are desired, [`ChunkProducer::push_chunk_uninit()`] can be used.
    ///
    /// # Examples
    ///
    /// For examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub fn push_chunk(&mut self) -> Result<PushChunk<'_, T>, ChunkError>
    where
        T: Default,
    {
        // Safety: PushChunk::from() initializes all slots.
        unsafe { self.push_chunk_uninit().map(PushChunk::from) }
    }

    /// Returns an uninitialized fixed-size chunk for writing,
    /// advancing the write position when dropped.
    ///
    /// If not enough data for a full chunk is available, an error is returned.
    ///
    /// For mutable access to its contents, the returned [`PushChunkUninit<T>`]
    /// can be (auto-)dereferenced to a [`slice`] of [`MaybeUninit<T>`].
    ///
    /// When the returned [`PushChunkUninit<T>`] is dropped,
    /// all slots are made available for reading.
    /// At this time, the whole chunk has to be initialized.
    ///
    /// If [`Default`]-initialized chunks are desired, [`ChunkProducer::push_chunk()`] can be used.
    ///
    /// # Safety
    ///
    /// Calling this function is safe, but dropping the returned [`PushChunkUninit`]
    /// (which might also happen in case of a panic!)
    /// is only safe if all slots have been initialized.
    ///
    /// # Examples
    ///
    /// For examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub unsafe fn push_chunk_uninit(&mut self) -> Result<PushChunkUninit<'_, T>, ChunkError> {
        let tail = self.inner.check_chunk(self.chunk_size)?;
        debug_assert!(self.chunk_size <= self.inner.buffer.capacity - tail);
        Ok(PushChunkUninit {
            ptr: self.inner.buffer.data_ptr.add(tail),
            len: self.chunk_size,
            producer: &mut self.inner,
        })
    }

    /// Returns a read-only reference to the ring buffer.
    pub fn buffer(&self) -> &RingBuffer<T> {
        self.inner.buffer()
    }
}

/// A [`Default`]-initialized fixed-size chunk, which advances the write position when dropped.
///
/// This is returned from [`ChunkProducer::push_chunk()`].
///
/// This `struct` has no inherent methods, but it implements [`DerefMut`],
/// which provides mutable access to the data as a [`slice`].
///
/// For an unsafe alternative that provides an uninitialized chunk, see [`PushChunkUninit`].
#[derive(Debug, PartialEq, Eq)]
pub struct PushChunk<'a, T>(PushChunkUninit<'a, T>);

impl<'a, T> From<PushChunkUninit<'a, T>> for PushChunk<'a, T>
where
    T: Default,
{
    /// Fills all slots with the [`Default`] value.
    fn from(chunk: PushChunkUninit<'a, T>) -> Self {
        for i in 0..chunk.len {
            unsafe {
                chunk.ptr.add(i).write(Default::default());
            }
        }
        PushChunk(chunk)
    }
}

impl<T> Deref for PushChunk<'_, T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe { core::slice::from_raw_parts(self.0.ptr, self.0.len) }
    }
}

impl<T> DerefMut for PushChunk<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { core::slice::from_raw_parts_mut(self.0.ptr, self.0.len) }
    }
}

/// An uninitialized fixed-size chunk, which advances the write position when dropped.
///
/// This is returned from [`ChunkProducer::push_chunk_uninit()`].
///
/// This `struct` has no inherent methods, but it implements [`DerefMut`],
/// which provides mutable access to the data as a [`slice`] of [`MaybeUninit<T>`].
///
/// For a safe alternative that provides [`Default`]-initialized contents, see [`PushChunk`].
///
/// # Safety
///
/// All slots of the chunk must be initialized before it is dropped
/// (which might also happen in case of a panic!).
#[derive(Debug, PartialEq, Eq)]
pub struct PushChunkUninit<'a, T> {
    ptr: *mut T,
    len: usize,
    producer: &'a mut Producer<T>,
}

impl<T> Deref for PushChunkUninit<'_, T> {
    type Target = [MaybeUninit<T>];
    fn deref(&self) -> &Self::Target {
        unsafe { core::slice::from_raw_parts(self.ptr as _, self.len) }
    }
}

impl<T> DerefMut for PushChunkUninit<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { core::slice::from_raw_parts_mut(self.ptr as _, self.len) }
    }
}

impl<T> Drop for PushChunkUninit<'_, T> {
    /// Advances the write position.
    ///
    /// # Safety
    ///
    /// All slots must be initialized at this point.
    fn drop(&mut self) {
        let tail = self.producer.tail.get();
        let tail = self.producer.buffer.increment(tail, self.len);
        self.producer.buffer.tail.store(tail, Ordering::Release);
        self.producer.tail.set(tail);
    }
}

/// The consumer side of a [`RingBuffer`], using fixed-size chunks.
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can be created with [`RingBuffer::with_chunks(...).of_size(...)`](RingBuffer::with_chunks)
/// (together with one of its counterparts [`Producer`] or [`ChunkProducer`]).
///
/// Fixed-size chunks can be consumed with [`ChunkConsumer::pop_chunk()`]
/// or they can be read with [`ChunkConsumer::peek_chunk()`] without consuming them.
///
/// If not all chunks have the same size, or if individual elements should be consumed,
/// a [`Consumer`] can be used instead.
#[derive(Debug, PartialEq, Eq)]
pub struct ChunkConsumer<T> {
    inner: Consumer<T>,
    chunk_size: usize,
}

unsafe impl<T: Send> Send for ChunkConsumer<T> {}

impl<T> ChunkConsumer<T> {
    /// Returns a fixed-size chunk for reading, advancing the read position when dropped.
    ///
    /// If not enough data for a full chunk is available, an error is returned.
    ///
    /// For access to its contents, the returned [`PopChunk<T>`]
    /// can be (auto-)dereferenced to a [`slice`].
    ///
    /// When the returned [`PopChunk<T>`] is dropped, all contained elements are dropped as well
    /// (if `T` implements [`Drop`]) and the now empty slots are made available for writing.
    ///
    /// # Examples
    ///
    /// For examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub fn pop_chunk(&mut self) -> Result<PopChunk<'_, T>, ChunkError> {
        let head = self.inner.check_chunk(self.chunk_size)?;
        debug_assert!(self.chunk_size <= self.inner.buffer.capacity - head);
        Ok(PopChunk {
            ptr: unsafe { self.inner.buffer.data_ptr.add(head) },
            len: self.chunk_size,
            consumer: &mut self.inner,
            _marker: PhantomData,
        })
    }

    /// Returns a fixed-size chunk for reading, not advancing the read position.
    ///
    /// If not enough data for a full chunk is available, an error is returned.
    ///
    /// # Examples
    ///
    /// For examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub fn peek_chunk(&self) -> Result<&[T], ChunkError> {
        let head = self.inner.check_chunk(self.chunk_size)?;
        debug_assert!(self.chunk_size <= self.inner.buffer.capacity - head);
        Ok(unsafe {
            core::slice::from_raw_parts(self.inner.buffer.data_ptr.add(head), self.chunk_size)
        })
    }

    /// Returns a read-only reference to the ring buffer.
    pub fn buffer(&self) -> &RingBuffer<T> {
        self.inner.buffer()
    }
}

/// A fixed-size chunk, which drops its contents and advances the read position when dropped.
///
/// This is returned from [`ChunkConsumer::pop_chunk()`].
///
/// This `struct` has no inherent methods, but it implements [`Deref`],
/// which provides access to the data as a [`slice`].
#[derive(Debug, PartialEq, Eq)]
pub struct PopChunk<'a, T> {
    ptr: *const T,
    len: usize,
    consumer: &'a mut Consumer<T>,
    // Indicates that items are dropped when the slice goes out of scope.
    _marker: PhantomData<T>,
}

impl<T> Deref for PopChunk<'_, T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl<T> Drop for PopChunk<'_, T> {
    /// Drops all contained elements (if `T` implements [`Drop`]) and advances the read position.
    fn drop(&mut self) {
        let head = self.consumer.head.get();
        // Safety: head has not yet been incremented
        unsafe {
            let ptr = self.consumer.buffer.slot_ptr(head);
            for i in 0..self.len {
                ptr.add(i).drop_in_place();
            }
        }
        let head = self.consumer.buffer.increment(head, self.len);
        self.consumer.buffer.head.store(head, Ordering::Release);
        self.consumer.head.set(head);
    }
}
