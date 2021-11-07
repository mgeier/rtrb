use core::{fmt, future::Future, ops::DerefMut, pin::Pin, sync::atomic::Ordering, task::{Context, Poll::{self, Ready}, Waker}};

use parking_lot::Mutex;

use crate::{Consumer, Reactor, Producer, RingBuffer, chunks::{ReadChunk, WriteChunk, WriteChunkUninit}};

/// [`RingBuffer`] which supports async read write.
pub type AsyncRingBuffer<T> = RingBuffer<T,AsyncReactor>;
/// [`Producer`] which supports async write.
pub type AsyncProducer<T> = Producer<T,AsyncReactor>;
/// [`Consumer`] which supports async read.
pub type AsyncConsumer<T> = Consumer<T,AsyncReactor>;
/// [`WriteChunk`] from [`AsyncProducer`].
pub type AsyncWriteChunk<'a,T> = WriteChunk<'a,T,AsyncReactor>;
/// [`WriteChunkUninit`] from [`AsyncProducer`].
pub type AsyncWriteChunkUninit<'a,T> = WriteChunkUninit<'a,T,AsyncReactor>;
/// [`ReadChunk`] from [`AsyncConsumer`].
pub type AsyncReadChunk<'a,T> = ReadChunk<'a,T,AsyncReactor>;

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
        RingBuffer::<T,AsyncReactor>::with_reactor(capacity)
    }
}

/// Trigger futures to wake when new slots are available,or abandoned.
#[derive(Debug,Default)]
pub struct AsyncReactor{
    state:Mutex<ReactorState>
}

#[derive(Debug)]
pub enum ReactorState{
    Registered(Waker,usize),
    Abandoned,
    Unregistered,
}

impl Default for ReactorState{
    fn default() -> Self {
        ReactorState::Unregistered
    }
}

impl AsyncReactor{
    fn pushed_or_popped(&self,n:usize){
        let mut guard = self.state.lock();
        let state = guard.deref_mut();
        if let ReactorState::Registered(_,insufficient_slots) = state{
            if *insufficient_slots > n{
                *insufficient_slots -= n;
            }else{
                if let ReactorState::Registered(waker,_) = std::mem::take(state){
                    drop(guard);
                    waker.wake();
                }else{
                    panic!();
                }
            }
        }
    }
}

impl Reactor for AsyncReactor{
    fn pushed(&self,n:usize) {
        self.pushed_or_popped(n);
    }

    fn popped(&self,n:usize) {
        self.pushed_or_popped(n);
    }

    fn abandoned(&self) {
        let mut guard = self.state.lock();
        if let ReactorState::Registered(waker,_) = std::mem::replace(&mut *guard,ReactorState::Abandoned){
            drop(guard);
            waker.wake();
        }
    }
}
/// Error type for [`Consumer::read_chunk_async()`], [`Producer::write_chunk_async()`]
/// and [`Producer::write_chunk_uninit_async()`].
#[derive(Debug)]
pub enum AsyncChunkError{
    /// Available slots are less than desired slots, and it will never increase because it is abandoned.
    /// Contains the number of slots that were available.
    TooFewSlotsAndAbandoned(usize),
    /// Desired slots exceeds capacity.
    /// Contains the capacity.
    ExceedCapacity(usize),
    /// Available slots are less than desired slots,and the other side of ring buffer is also awaiting for new slots.
    /// First value means the number of new slots the other sides requesting, second value means the number of slots available for you.
    WillDeadlock(usize,usize),
}

impl fmt::Display for AsyncChunkError{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        match self{
            AsyncChunkError::TooFewSlotsAndAbandoned(available_slots) => 
            alloc::format!("Only {} slots available in ring buffer ,and it will never increase because it was abandoned",available_slots).fmt(f),
            AsyncChunkError::ExceedCapacity(capacity) => 
            alloc::format!("Ring buffer has only {} capacity",capacity).fmt(f),
            AsyncChunkError::WillDeadlock(required, available_slots) => 
            alloc::format!("Tried to await new slots get available, but the opponent is already awaiting new slots. The opponent  requested {} new slots, you have {} slots available.",required,available_slots).fmt(f),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AsyncChunkError {}

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
impl<T> Producer<T,AsyncReactor>{
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
    /// Asynchronously waits `n` slots (initially containing their [`Default`] value) for writing are available and returns it.
    ///
    /// [`WriteChunk::as_mut_slices()`] provides mutable access to the slots.
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
    /// If not enough slots are available, and awaiting makes nonsense,
    /// error describing its reason is returned.
    /// Use [`Producer::slots()`] to obtain the number of available slots beforehand.
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](crate::chunks#examples) module.
    pub async fn write_chunk_async(&mut self, n: usize) -> Result<AsyncWriteChunk<'_, T>, AsyncChunkError>
    where
        T: Default,
    {
        self.write_chunk_uninit_async(n).await.map(WriteChunk::from)
    }
    /// Asynchronously waits `n` (uninitialized) slots for writing are available and returns it.
    ///
    /// [`WriteChunkUninit::as_mut_slices()`] provides mutable access
    /// to the uninitialized slots.
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
    /// If not enough slots are available, and awaiting makes nonsense,
    /// error describing its reason is returned.
    /// Use [`Producer::slots()`] to obtain the number of available slots beforehand.
    ///
    /// # Safety
    ///
    /// This function itself is safe, as is [`WriteChunkUninit::fill_from_iter()`].
    /// However, when using [`WriteChunkUninit::as_mut_slices()`],
    /// the user has to make sure that the relevant slots have been initialized
    /// before calling [`WriteChunkUninit::commit()`] or [`WriteChunkUninit::commit_all()`].
    ///
    /// For a safe alternative that provides mutable slices of [`Default`]-initialized slots,
    /// see [`Producer::write_chunk()`].
    pub fn write_chunk_uninit_async(&mut self,n:usize) -> impl Future<Output = Result<AsyncWriteChunkUninit<'_,T>,AsyncChunkError>>{ 
        WriteChunkUninitAsync{
            producer:Some(self),
            n:n,
            waker:None,
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

impl<T> Consumer<T,AsyncReactor>{
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
    /// Asynchronously waits `n` slots for reading are available and returns it.
    ///
    /// [`ReadChunk::as_slices()`] provides immutable access to the slots.
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
    /// If not enough slots are available, and awaiting makes nonsense,
    /// error describing its reason is returned.
    /// Use [`Consumer::slots()`] to obtain the number of available slots beforehand.
    ///
    /// # Examples
    ///
    /// See the documentation of the [`chunks`](crate::chunks#examples) module.
    pub fn read_chunk_async(&mut self, n: usize) -> impl Future<Output = Result<AsyncReadChunk<'_, T>, AsyncChunkError>> {
       ReadChunkAsync{
           consumer:Some(self),
           n:n,
           waker:None,
       }
    }
}

impl<T,U:Reactor> Drop for Producer<T,U>{
    fn drop(&mut self) {
        self.buffer.reactor.abandoned();
    }
}
impl<T,U:Reactor> Drop for Consumer<T,U>{
    fn drop(&mut self) {
        self.buffer.reactor.abandoned();
    }
}

struct WriteChunkUninitAsync<'a,T>{
    producer:Option<&'a mut Producer<T,AsyncReactor>>,
    n:usize,
    waker:Option<Waker>,
}
impl<T> Unpin for WriteChunkUninitAsync<'_,T>{}
impl<'a,T> Future for WriteChunkUninitAsync<'a,T>{
    type Output = Result<WriteChunkUninit<'a,T,AsyncReactor>,AsyncChunkError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let n = this.n;
        let producer = this.producer.as_ref().unwrap();
        let buffer = &producer.buffer;
        let capacity = buffer.capacity;
        let tail = producer.tail.get();
        let waker = &mut this.waker;
        if waker.is_none(){
            

            // Check if the queue has *possibly* not enough slots.
            
            if capacity - buffer.distance(producer.head.get(), tail) < n {
                // Refresh the head ...
                let head = buffer.head.load(Ordering::Acquire);
                producer.head.set(head);

                // ... and check if there *really* are not enough slots.
                let slots = capacity - buffer.distance(head, tail);
                if slots < n {
                    if capacity < n{
                        return Ready(Err(AsyncChunkError::ExceedCapacity(capacity)));
                    }else{
                        let mut guard = buffer.reactor.state.lock();
                        // Refresh the head ...
                        let head = buffer.head.load(Ordering::Acquire);
                        producer.head.set(head);

                        // ... and check if there *really* are not enough slots.
                        let slots = capacity - buffer.distance(head, tail);
                        if slots < n{
                            let state = guard.deref_mut();
                            return match *state{
                                ReactorState::Registered(_, insufficient_slots) => {
                                    Ready(Err(AsyncChunkError::WillDeadlock(insufficient_slots,slots)))
                                },
                                ReactorState::Abandoned => {
                                    Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(slots)))
                                },
                                ReactorState::Unregistered => {
                                    *state = ReactorState::Registered(cx.waker().clone(),n- slots);
                                    *waker = Some(cx.waker().clone());
                                    Poll::Pending
                                },
                            }
                        }
                    }
                }
            }
            
        }else{
            this.waker = None;
            // Refresh the head ...
            let head = buffer.head.load(Ordering::Acquire);
            producer.head.set(head);

            // ... and check if there *really* are not enough slots.
            let slots = capacity - buffer.distance(head, tail);
            if slots < n{
                return Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(slots)))
            }
        }
        let tail = buffer.collapse_position(tail);
        let first_len = n.min(capacity - tail);
        Ready(Ok(WriteChunkUninit {
            first_ptr: unsafe { buffer.data_ptr.add(tail) },
            first_len,
            second_ptr: buffer.data_ptr,
            second_len: n - first_len,
            producer: std::mem::take(&mut this.producer).unwrap(),
        }))
    }
}
impl<T> Drop  for WriteChunkUninitAsync<'_,T>{
    fn drop(&mut self) {
        if let Some(waker) = std::mem::take(&mut self.waker){
            let mut guard = self.producer.as_ref().unwrap().buffer.reactor.state.lock();
            let state = guard.deref_mut();
            if let ReactorState::Registered(current_waker,_) = state{
                if waker.will_wake(current_waker){
                    *state = ReactorState::Unregistered;
                }
            }
        }
    }
}

struct ReadChunkAsync<'a,T>{
    consumer:Option<&'a mut Consumer<T,AsyncReactor>>,
    n:usize,
    waker:Option<Waker>,
}
impl<T> Unpin for ReadChunkAsync<'_,T>{}
impl<'a,T> Future for ReadChunkAsync<'a,T>{
    type Output = Result<ReadChunk<'a,T,AsyncReactor>,AsyncChunkError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let n = this.n;
        let consumer = this.consumer.as_ref().unwrap();
        let buffer = &consumer.buffer;
        let capacity = buffer.capacity;
        let head = consumer.head.get();
        let waker = &mut this.waker;
        if waker.is_none(){
            

            // Check if the queue has *possibly* not enough slots.
            if buffer.distance(head, consumer.tail.get()) < n {
                // Refresh the tail ...
                let tail = buffer.tail.load(Ordering::Acquire);
                consumer.tail.set(tail);

                // ... and check if there *really* are not enough slots.
                let slots = buffer.distance(head, tail);
                if slots < n {
                    if capacity < n{
                        return Ready(Err(AsyncChunkError::ExceedCapacity(capacity)));
                    }else{
                        let mut guard = buffer.reactor.state.lock();
                        // Refresh the tail ...
                        let tail = buffer.tail.load(Ordering::Acquire);
                        consumer.tail.set(tail);

                        // ... and check if there *really* are not enough slots.
                        let slots = buffer.distance(head, tail);
                        if slots < n{
                            let state = guard.deref_mut();
                            return match *state{
                                ReactorState::Registered(_, insufficient_slots) => {
                                    Ready(Err(AsyncChunkError::WillDeadlock(insufficient_slots,slots)))
                                },
                                ReactorState::Abandoned => {
                                    Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(slots)))
                                },
                                ReactorState::Unregistered => {
                                    *state = ReactorState::Registered(cx.waker().clone(),n- slots);
                                    *waker = Some(cx.waker().clone());
                                    Poll::Pending
                                },
                            }
                        }
                    }
                }
            }
            
        }else{
            this.waker = None;
            // Refresh the tail ...
            let tail = buffer.tail.load(Ordering::Acquire);
            consumer.tail.set(tail);

            // ... and check if there *really* are not enough slots.
            let slots = buffer.distance(head, tail);
            if slots < n{
                return Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(slots)))
            }
        }
        let head = buffer.collapse_position(head);
        let first_len = n.min(buffer.capacity - head);
        Ready(Ok(ReadChunk {
            first_ptr: unsafe { buffer.data_ptr.add(head) },
            first_len,
            second_ptr: buffer.data_ptr,
            second_len: n - first_len,
            consumer: std::mem::take(&mut this.consumer).unwrap(),
        }))
    }
}
impl<T> Drop  for ReadChunkAsync<'_,T>{
    fn drop(&mut self) {
        if let Some(waker) = std::mem::take(&mut self.waker){
            let mut guard = self.consumer.as_ref().unwrap().buffer.reactor.state.lock();
            let state = guard.deref_mut();
            if let ReactorState::Registered(current_waker,_) = state{
                if waker.will_wake(current_waker){
                    *state = ReactorState::Unregistered;
                }
            }
        }
    }
}