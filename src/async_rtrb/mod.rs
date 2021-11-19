pub(crate) mod chunks;
pub(crate) mod async_reactor;

use core::fmt;

use crate::{Consumer, Reactor, Producer, RingBuffer, chunks::{ReadChunk, WriteChunk, WriteChunkUninit}};

use self::{async_reactor::{AsyncReactor, CommitterWaitFreeReactor}, chunks::AsyncChunkError};

/// [`RingBuffer`] which supports async read write.
pub type AsyncRingBuffer<T> = RingBuffer<T,CommitterWaitFreeReactor>;
/// [`Producer`] which supports async write.
pub type AsyncProducer<T> = Producer<T,CommitterWaitFreeReactor>;
/// [`Consumer`] which supports async read.
pub type AsyncConsumer<T> = Consumer<T,CommitterWaitFreeReactor>;
/// [`WriteChunk`] from [`AsyncProducer`].
pub type AsyncWriteChunk<'a,T> = WriteChunk<'a,T,CommitterWaitFreeReactor>;
/// [`WriteChunkUninit`] from [`AsyncProducer`].
pub type AsyncWriteChunkUninit<'a,T> = WriteChunkUninit<'a,T,CommitterWaitFreeReactor>;
/// [`ReadChunk`] from [`AsyncConsumer`].
pub type AsyncReadChunk<'a,T> = ReadChunk<'a,T,CommitterWaitFreeReactor>;

impl<T> RingBuffer<T>{
    /// Creates a `RingBuffer` with the given `capacity` and returns [`Producer`] and [`Consumer`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (producer, consumer) = RingBuffer::<f32>::new(100);
    /// ```
    ///
    /// Specifying an explicit type with the [turbofish](https://turbo.fish/)
    /// is is only necessary if it cannot be deduced by the compiler.
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, consumer) = RingBuffer::new_async(100);
    /// assert_eq!(producer.push(0.0f32), Ok(()));
    /// ```
    #[allow(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new_async(capacity: usize) -> (AsyncProducer<T>, AsyncConsumer<T>) {
        RingBuffer::<T,CommitterWaitFreeReactor>::with_reactor(capacity)
    }
}



/// Error type for [`Producer::push_async()`].
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum AsyncPushError<T>{
    /// This ring buffer has capacity of 0.
    CapacityIs0(T),
    /// This ring buffer is full and abandoned.
    FullAndAbandoned(T),
}
impl<T> fmt::Display for AsyncPushError<T>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self{
            AsyncPushError::CapacityIs0(_) => 
            alloc::format!("Ring buffer has capacity of 0.").fmt(f),
            AsyncPushError::FullAndAbandoned(_) => 
            alloc::format!("Ring buffer is full and abandoned.").fmt(f),
        }
    }
}
impl<T> fmt::Debug for AsyncPushError<T>{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityIs0(_) => f.pad("CapacityIs0(_)"),
            Self::FullAndAbandoned(_) => f.pad("FullAndAbandoned(_)"),
        }
    }
}
#[cfg(feature = "std")]
impl<T> std::error::Error for AsyncPushError<T>{}
impl<T,U:AsyncReactor> Producer<T,U>{
    /// Attempts to asynchronously wait for at least 1 slots available, push an element into the queue.
    ///
    /// The element is *moved* into the ring buffer and its slot
    /// is made available to be read by the [`Consumer`].
    ///
    /// # Errors
    ///
    /// If the queue is full and abandoned, or the queue has capacity of 0, the element is returned back as an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{RingBuffer, PushError};
    ///
    /// let (mut p, c) = RingBuffer::new_async(1);
    ///
    /// assert_eq!(p.push_async(10).await, Ok(()));
    /// drop(c);
    /// assert_eq!(p.push_async(20).await, Err(AsyncPushError::FullAndAbandoned(20)));
    /// ```
    pub async fn push_async(&mut self,value:T) -> Result<(),AsyncPushError<T>>{
        // TODO:Faster implementation  
        match self.write_chunk_uninit_async(1).await{
            Ok(chunk) => {
                chunk.fill_from_iter(core::iter::once(value));
                Ok(())
            },
            Err(err) => {
                Err(match err{
                    AsyncChunkError::TooFewSlotsAndAbandoned(_) => {
                        AsyncPushError::FullAndAbandoned(value)
                    },
                    AsyncChunkError::ExceedCapacity(_) => {
                        AsyncPushError::CapacityIs0(value)
                    },
                    AsyncChunkError::WillDeadlock(_, _) => {
                        unreachable!();
                    },
                })
            },
        }
    }

}

/// Error type for [`Consumer::pop_async()`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AsyncPopError{
    /// This ring buffer has capacity of 0.
    CapacityIs0,
    /// This ring buffer is empty and abandoned.
    EmptyAndAbandoned,
}
impl fmt::Display for AsyncPopError{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self{
            AsyncPopError::CapacityIs0 => 
            alloc::format!("Ring buffer has capacity of 0.").fmt(f),
            AsyncPopError::EmptyAndAbandoned => 
            alloc::format!("Ring buffer is empty and abandoned.").fmt(f),
        }
    }
}
#[cfg(feature = "std")]
impl std::error::Error for AsyncPopError{}

/// Error type for [`Consumer::peek_async()`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AsyncPeekError{
    /// This ring buffer has capacity of 0.
    CapacityIs0,
    /// This ring buffer is empty and abandoned.
    EmptyAndAbandoned,
}
impl fmt::Display for AsyncPeekError{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self{
            AsyncPeekError::CapacityIs0 => 
            alloc::format!("Ring buffer has capacity of 0.").fmt(f),
            AsyncPeekError::EmptyAndAbandoned => 
            alloc::format!("Ring buffer is empty and abandoned.").fmt(f),
        }
    }
}
#[cfg(feature = "std")]
impl std::error::Error for AsyncPeekError{}

impl<T,U:AsyncReactor> Consumer<T,U>{
    /// Attempts to asynchronously wait for at least 1 slots available, pop an element from the queue.
    ///
    /// The element is *moved* out of the ring buffer and its slot
    /// is made available to be filled by the [`Producer`] again.
    ///
    /// # Errors
    ///
    /// If the queue is empty and abandoned, or the queue has capacity of 0, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PopError, RingBuffer};
    ///
    /// let (mut p, mut c) = RingBuffer::new_async(1);
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(c.pop_async().await, Ok(10));
    /// drop(p);
    /// assert_eq!(c.pop_async().await, Err(AsyncPopError::EmptyAndAbandoned));
    /// ```
    ///
    /// To obtain an [`Option<T>`](Option), use [`.ok()`](Result::ok) on the result.
    ///
    /// ```
    /// # use rtrb::RingBuffer;
    /// # let (mut p, mut c) = RingBuffer::new_async(1);
    /// assert_eq!(p.push_async(20).await, Ok(()));
    /// assert_eq!(c.pop_async().await.ok(), Some(20));
    /// ```
    pub async fn pop_async(&mut self) -> Result<T,AsyncPopError>{
        // TODO:Faster implementation  
        match self.read_chunk_async(1).await{
            Ok(chunk) => {
                Ok(chunk.into_iter().next().unwrap())
            },
            Err(err) => {
                Err(match err{
                    AsyncChunkError::TooFewSlotsAndAbandoned(_) => {
                        AsyncPopError::EmptyAndAbandoned
                    },
                    AsyncChunkError::ExceedCapacity(_) => {
                        AsyncPopError::CapacityIs0
                    },
                    AsyncChunkError::WillDeadlock(_, _) => {
                        unreachable!();
                    },
                })
            },
        }
    }
    /// Attempts to asynchronously wait for at least 1 slots available, read an element from the queue without removing it.
    ///
    /// # Errors
    ///
    /// If the queue is empty and abandoned, or the queue has capacity of 0, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PeekError, RingBuffer};
    ///
    /// let (mut p, c) = RingBuffer::new_async(1);
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(c.peek_async().await, Ok(&10));
    /// assert_eq!(c.peek_async().await, Ok(&10));
    /// ```
    pub async fn peek_async(&mut self) -> Result<&T,AsyncPeekError>{
        // TODO:Faster implementation  
        match self.read_chunk_async(1).await{
            Ok(chunk) => {
                Ok(unsafe{&*chunk.first_ptr})
            },
            Err(err) => {
                Err(match err{
                    AsyncChunkError::TooFewSlotsAndAbandoned(_) => {
                        AsyncPeekError::EmptyAndAbandoned
                    },
                    AsyncChunkError::ExceedCapacity(_) => {
                        AsyncPeekError::CapacityIs0
                    },
                    AsyncChunkError::WillDeadlock(_, _) => {
                        unreachable!();
                    },
                })
            },
        }
    }
    
}

impl<T,U:Reactor> Drop for Producer<T,U>{
    fn drop(&mut self) {
        U::dropping_producer(self);
    }
}
impl<T,U:Reactor> Drop for Consumer<T,U>{
    fn drop(&mut self) {
        U::dropping_consumer(self);
    }
}
