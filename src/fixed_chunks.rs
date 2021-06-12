//! Writing and reading fixed-size chunks into and from a [`RingBuffer`].
//!
//! The `capacity` used in [`RingBuffer::new()`] must be an integer multiple of the chunk size.
//!
//! Chunks can be produced by using [`FixedChunkProducer::push_chunk()`] or its `unsafe`
//! (but probably slightly faster) cousin [`FixedChunkProducer::push_chunk_uninit()`].
//! A [`FixedChunkProducer`] can be obtained with [`Producer::try_fixed_chunk_size()`].
//!
//! Chunks can be consumed with [`FixedChunkConsumer::pop_chunk()`].
//! A [`FixedChunkConsumer`] can be obtained with [`Consumer::try_fixed_chunk_size()`].
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
//! use rtrb::RingBuffer;
//!
//! const CHUNKS: usize = 3;
//! const CHUNK_SIZE: usize = 2;
//!
//! let (mut p, c) = RingBuffer::new(CHUNKS * CHUNK_SIZE);
//! p.push(10).unwrap();
//! p.push(11).unwrap();
//! p.push(12).unwrap();
//! p.push(13).unwrap();
//!
//! let mut c = c.try_fixed_chunk_size(CHUNK_SIZE).unwrap();
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
//! use rtrb::RingBuffer;
//!
//! const CHUNKS: usize = 3;
//! const CHUNK_SIZE: usize = 2;
//!
//! let (p, mut c) = RingBuffer::new(CHUNKS * CHUNK_SIZE);
//!
//! let mut p = p.try_fixed_chunk_size(CHUNK_SIZE).unwrap();
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
//! // Safety: chunk will be initialized before it is dropped.
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
//! use rtrb::RingBuffer;
//!
//! let (p, c) = RingBuffer::new(6);
//!
//! let mut p = p.try_fixed_chunk_size(3).unwrap();
//! let mut c = c.try_fixed_chunk_size(2).unwrap();
//!
//! if let Ok(mut chunk) = p.push_chunk() {
//!     chunk.copy_from_slice(&[10, 11, 12]);
//! } else {
//!     unreachable!();
//! }
//! if let Ok(chunk) = c.pop_chunk() {
//!     assert_eq!(*chunk, [10, 11]);
//! } else {
//!     unreachable!();
//! }
//! assert_eq!(c.peek_chunk(), Err(rtrb::fixed_chunks::ChunkError::TooFewSlots(1)));
//! ```
//!
//! As can be seen in the last example, the chunk sizes can be different between producer and
//! consumer, but the `capacity` must be an integer multiple of each one of them.

use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::Ordering;

use crate::{Consumer, Producer, RingBuffer};

pub use crate::chunks::ChunkError;

impl<T> Producer<T> {
    /// Converts `self` into a [`FixedChunkProducer`] with the given `chunk_size`.
    ///
    /// # Examples
    ///
    /// The `capacity` of the [`RingBuffer`] must be an integer multiple of the chunk size:
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<i32>::new(4);
    /// assert!(p.try_fixed_chunk_size(3).is_err());
    /// ```
    ///
    /// The current write position must be at an integer multiple as well:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// let (mut p, c) = RingBuffer::new(4);
    /// assert_eq!(p.push(10), Ok(()));
    /// assert!(p.try_fixed_chunk_size(2).is_err());
    ///
    /// let (mut p, c) = RingBuffer::new(4);
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(p.push(20), Ok(()));
    /// assert!(p.try_fixed_chunk_size(2).is_ok());
    /// ```
    ///
    /// For more examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub fn try_fixed_chunk_size(
        self,
        chunk_size: usize,
    ) -> Result<FixedChunkProducer<T>, Producer<T>> {
        if chunk_size == 0
            || (self.buffer().capacity() % chunk_size == 0 && self.tail.get() % chunk_size == 0)
        {
            Ok(FixedChunkProducer {
                inner: self,
                chunk_size,
            })
        } else {
            Err(self)
        }
    }
}

/// The producer side of a [`RingBuffer`], using fixed-size chunks.
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can be created with [`Producer::try_fixed_chunk_size()`].
///
/// Fixed-size chunks can be produced with [`FixedChunkProducer::push_chunk()`] or its `unsafe`
/// (but probably slightly faster) cousin [`FixedChunkProducer::push_chunk_uninit()`].
///
/// If not all chunks have the same size, or if individual elements should be produced,
/// a [`Producer`] can be used instead.
#[derive(Debug, PartialEq, Eq)]
pub struct FixedChunkProducer<T> {
    inner: Producer<T>,
    chunk_size: usize,
}

unsafe impl<T: Send> Send for FixedChunkProducer<T> {}

impl<T> From<FixedChunkProducer<T>> for Producer<T> {
    fn from(p: FixedChunkProducer<T>) -> Self {
        p.inner
    }
}

impl<T> FixedChunkProducer<T> {
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
    /// If uninitialized chunks are desired, [`FixedChunkProducer::push_chunk_uninit()`] can be used.
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
    /// If [`Default`]-initialized chunks are desired, [`FixedChunkProducer::push_chunk()`] can be used.
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
/// This is returned from [`FixedChunkProducer::push_chunk()`].
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
/// This is returned from [`FixedChunkProducer::push_chunk_uninit()`].
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

impl<T> Consumer<T> {
    /// Converts `self` into a [`FixedChunkConsumer`] with the given `chunk_size`.
    ///
    /// # Examples
    ///
    /// The `capacity` of the [`RingBuffer`] must be an integer multiple of the chunk size:
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<i32>::new(4);
    /// assert!(c.try_fixed_chunk_size(3).is_err());
    /// ```
    ///
    /// The current read position must be at an integer multiple as well:
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// let (mut p, mut c) = RingBuffer::new(4);
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(p.push(20), Ok(()));
    /// assert_eq!(c.pop(), Ok(10));
    /// assert!(c.try_fixed_chunk_size(2).is_err());
    ///
    /// let (mut p, mut c) = RingBuffer::new(4);
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(p.push(20), Ok(()));
    /// assert_eq!(p.push(30), Ok(()));
    /// assert_eq!(c.pop(), Ok(10));
    /// assert_eq!(c.pop(), Ok(20));
    /// assert!(c.try_fixed_chunk_size(2).is_ok());
    /// ```
    ///
    /// For more examples see the documentation of the
    /// [`fixed_chunks`](crate::fixed_chunks#examples) module.
    pub fn try_fixed_chunk_size(
        self,
        chunk_size: usize,
    ) -> Result<FixedChunkConsumer<T>, Consumer<T>> {
        if chunk_size == 0
            || (self.buffer().capacity() % chunk_size == 0 && self.head.get() % chunk_size == 0)
        {
            Ok(FixedChunkConsumer {
                inner: self,
                chunk_size,
            })
        } else {
            Err(self)
        }
    }
}

/// The consumer side of a [`RingBuffer`], using fixed-size chunks.
///
/// Can be moved between threads,
/// but references from different threads are not allowed
/// (i.e. it is [`Send`] but not [`Sync`]).
///
/// Can be created with [`Consumer::try_fixed_chunk_size()`].
///
/// Fixed-size chunks can be consumed with [`FixedChunkConsumer::pop_chunk()`]
/// or they can be read with [`FixedChunkConsumer::peek_chunk()`] without consuming them.
///
/// If not all chunks have the same size, or if individual elements should be consumed,
/// a [`Consumer`] can be used instead.
#[derive(Debug, PartialEq, Eq)]
pub struct FixedChunkConsumer<T> {
    inner: Consumer<T>,
    chunk_size: usize,
}

unsafe impl<T: Send> Send for FixedChunkConsumer<T> {}

impl<T> From<FixedChunkConsumer<T>> for Consumer<T> {
    fn from(c: FixedChunkConsumer<T>) -> Self {
        c.inner
    }
}

impl<T> FixedChunkConsumer<T> {
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
/// This is returned from [`FixedChunkConsumer::pop_chunk()`].
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
