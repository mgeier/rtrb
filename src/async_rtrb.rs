use core::{fmt, future::Future, ops::DerefMut, pin::Pin, sync::atomic::Ordering, task::{Context, Poll::{self, Ready}, Waker}};

use parking_lot::Mutex;

use crate::{Consumer, Reactor, Producer, RingBuffer, chunks::{ReadChunk, WriteChunk, WriteChunkUninit}};

/// [`RingBuffer`] which supports async read write.
pub type AsyncRingBuffer<T> = RingBuffer<T,MutexReactor>;
/// [`Producer`] which supports async write.
pub type AsyncProducer<T> = Producer<T,MutexReactor>;
/// [`Consumer`] which supports async read.
pub type AsyncConsumer<T> = Consumer<T,MutexReactor>;
/// [`WriteChunk`] from [`AsyncProducer`].
pub type AsyncWriteChunk<'a,T> = WriteChunk<'a,T,MutexReactor>;
/// [`WriteChunkUninit`] from [`AsyncProducer`].
pub type AsyncWriteChunkUninit<'a,T> = WriteChunkUninit<'a,T,MutexReactor>;
/// [`ReadChunk`] from [`AsyncConsumer`].
pub type AsyncReadChunk<'a,T> = ReadChunk<'a,T,MutexReactor>;

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
        RingBuffer::<T,MutexReactor>::with_reactor(capacity)
    }
}

pub trait AsyncReactor:Reactor{
    #[must_use]
    fn register_read_slots_available<T>(consumer:&&mut Consumer<T,Self>,waker:&Waker,n:usize)->AsyncReactorRegisterResult;
    #[must_use]
    fn register_write_slots_available<T>(producer:&&mut Producer<T,Self>,waker:&Waker,n:usize)->AsyncReactorRegisterResult;

    fn unregister_read_slots_available(&self);
    fn unregister_write_slots_available(&self);
}
#[derive(Debug)]
pub enum AsyncReactorRegisterResult{
    Registered,
    AlreadyAvailable,
    TooFewSlotsAndAbandoned(usize),
    WillDeadlock(usize,usize),
}

/// Trigger futures to wake when new slots are available,or abandoned.
#[derive(Debug,Default)]
pub struct MutexReactor{
    state:Mutex<MutexReactorState>
}

#[derive(Debug)]
pub enum MutexReactorState{
    ProducerRegistered(Waker,usize),
    ConsumerRegistered(Waker,usize),
    Abandoned,
    Unregistered,
}

impl Default for MutexReactorState{
    fn default() -> Self {
        MutexReactorState::Unregistered
    }
}


impl Reactor for MutexReactor{
    fn pushed(&self,n:usize) {
        let mut guard = self.state.lock();
        let state = guard.deref_mut();
        match state {
            MutexReactorState::ConsumerRegistered(_,insufficient_slots) => {
                if *insufficient_slots > n{
                    *insufficient_slots -= n;
                }else{
                    if let MutexReactorState::ConsumerRegistered(waker,_) = std::mem::take(state){
                        drop(guard);
                        waker.wake();
                    }else{
                        unreachable!();
                    }
                }
            },
            MutexReactorState::ProducerRegistered(_,_) => unreachable!(),
            _ => (),
        }
    }

    fn popped(&self,n:usize) {
        let mut guard = self.state.lock();
        let state = guard.deref_mut();
        match state {
            MutexReactorState::ProducerRegistered(_,insufficient_slots) => {
                if *insufficient_slots > n{
                    *insufficient_slots -= n;
                }else{
                    if let MutexReactorState::ProducerRegistered(waker,_) = std::mem::take(state){
                        drop(guard);
                        waker.wake();
                    }else{
                        unreachable!();
                    }
                }
            },
            MutexReactorState::ConsumerRegistered(_,_) => unreachable!(),
            _ => (),
        }
    }

    fn abandoned(&self) {
        let mut guard = self.state.lock();
        if let MutexReactorState::ProducerRegistered(waker,_) | MutexReactorState::ConsumerRegistered(waker,_) = std::mem::replace(&mut *guard,MutexReactorState::Abandoned){
            drop(guard);
            waker.wake();
        }
    }
}

impl AsyncReactor for MutexReactor{
    fn register_read_slots_available<T>(consumer:&&mut Consumer<T,Self>,waker:&Waker,n:usize)->AsyncReactorRegisterResult {
        let buffer = consumer.buffer.as_ref();
        let mut guard = buffer.reactor.state.lock();
        // Refresh the tail ...
        let tail = buffer.tail.load(Ordering::Relaxed);
        consumer.tail.set(tail);

        // ... and check if there *really* are not enough slots.
        let slots = buffer.distance(consumer.head.get(), tail);
        if slots < n{
            let state = guard.deref_mut();
            return match *state{
                MutexReactorState::ProducerRegistered(_, insufficient_slots) => {
                    AsyncReactorRegisterResult::WillDeadlock(insufficient_slots,slots)
                },
                MutexReactorState::Abandoned => {
                    AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(slots)
                },
                MutexReactorState::Unregistered => {
                    *state = MutexReactorState::ConsumerRegistered(waker.clone(),n- slots);
                    AsyncReactorRegisterResult::Registered
                },
                MutexReactorState::ConsumerRegistered(_, _) => unreachable!(),
            }
        }else{
            AsyncReactorRegisterResult::AlreadyAvailable
        }
    }

    fn register_write_slots_available<T>(producer:&&mut Producer<T,Self>,waker:&Waker,n:usize)->AsyncReactorRegisterResult {
        let buffer = producer.buffer.as_ref();
        let capacity = buffer.capacity;
        let mut guard = buffer.reactor.state.lock();
        // Refresh the head ...
        let head = buffer.head.load(Ordering::Relaxed);
        producer.head.set(head);

        // ... and check if there *really* are not enough slots.
        let slots = capacity - buffer.distance(head, producer.tail.get());
        if slots < n{
            let state = guard.deref_mut();
            return match *state{
                MutexReactorState::ConsumerRegistered(_, insufficient_slots) => {
                    AsyncReactorRegisterResult::WillDeadlock(insufficient_slots,slots)
                },
                MutexReactorState::Abandoned => {
                    AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(slots)
                },
                MutexReactorState::Unregistered => {
                    *state = MutexReactorState::ProducerRegistered(waker.clone(),n- slots);
                    AsyncReactorRegisterResult::Registered
                },
                MutexReactorState::ProducerRegistered(_, _) => unreachable!(),
            }
        }else{
            AsyncReactorRegisterResult::AlreadyAvailable
        }
    }

    fn unregister_read_slots_available(&self) {
        let mut guard = self.state.lock();
        let state = guard.deref_mut();
        if matches!(state,MutexReactorState::ConsumerRegistered(_,_)){
            *state = MutexReactorState::Unregistered;
        }
    }

    fn unregister_write_slots_available(&self) {
        let mut guard = self.state.lock();
        let state = guard.deref_mut();
        if matches!(state,MutexReactorState::ProducerRegistered(_,_)){
            *state = MutexReactorState::Unregistered;
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
    pub async fn write_chunk_async(&mut self, n: usize) -> Result<WriteChunk<'_, T,U>, AsyncChunkError>
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
    pub fn write_chunk_uninit_async(&mut self,n:usize) -> impl Future<Output = Result<WriteChunkUninit<'_,T,U>,AsyncChunkError>>{ 
        WriteChunkUninitAsync{
            producer:Some(self),
            n:n,
            registered:false,
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
    pub fn read_chunk_async(&mut self, n: usize) -> impl Future<Output = Result<ReadChunk<'_, T,U>, AsyncChunkError>> {
       ReadChunkAsync{
           consumer:Some(self),
           n:n,
           registered:false,
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

struct WriteChunkUninitAsync<'a,T,U:AsyncReactor>{
    producer:Option<&'a mut Producer<T,U>>,
    n:usize,
    registered:bool,
}
impl<T,U:AsyncReactor> Unpin for WriteChunkUninitAsync<'_,T,U>{}
impl<'a,T,U:AsyncReactor> Future for WriteChunkUninitAsync<'a,T,U>{
    type Output = Result<WriteChunkUninit<'a,T,U>,AsyncChunkError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let n = this.n;
        let producer = this.producer.as_ref().unwrap();
        let buffer = &producer.buffer;
        let capacity = buffer.capacity;
        let tail = producer.tail.get();
        if !this.registered{
            

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
                        match U::register_write_slots_available(producer, cx.waker(), n){
                            AsyncReactorRegisterResult::Registered => return Poll::Pending,
                            AsyncReactorRegisterResult::AlreadyAvailable => (),
                            AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots) => return Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(available_slots))),
                            AsyncReactorRegisterResult::WillDeadlock(required, available_slots) => return Ready(Err(AsyncChunkError::WillDeadlock(required,available_slots))),
                        }
                    }
                }
            }
            
        }else{
            this.registered = false;
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
impl<T,U:AsyncReactor> Drop  for WriteChunkUninitAsync<'_,T,U>{
    fn drop(&mut self) {
        if self.registered{
            self.producer.as_ref().unwrap().buffer.reactor.unregister_write_slots_available();
        }
    }
}

struct ReadChunkAsync<'a,T,U:AsyncReactor>{
    consumer:Option<&'a mut Consumer<T,U>>,
    n:usize,
    registered:bool,
}
impl<T,U:AsyncReactor> Unpin for ReadChunkAsync<'_,T,U>{}
impl<'a,T,U:AsyncReactor> Future for ReadChunkAsync<'a,T,U>{
    type Output = Result<ReadChunk<'a,T,U>,AsyncChunkError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let n = this.n;
        let consumer = this.consumer.as_ref().unwrap();
        let buffer = &consumer.buffer;
        let capacity = buffer.capacity;
        let head = consumer.head.get();
        if !this.registered{
            

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
                        match U::register_read_slots_available(consumer, cx.waker(), n){
                            AsyncReactorRegisterResult::Registered => return Poll::Pending,
                            AsyncReactorRegisterResult::AlreadyAvailable => (),
                            AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots) => return Ready(Err(AsyncChunkError::TooFewSlotsAndAbandoned(available_slots))),
                            AsyncReactorRegisterResult::WillDeadlock(required, available_slots) => return Ready(Err(AsyncChunkError::WillDeadlock(required,available_slots))),
                        }
                    }
                }
            }
            
        }else{
            this.registered = false;
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
impl<T,U:AsyncReactor> Drop for ReadChunkAsync<'_,T,U>{
    fn drop(&mut self) {
        if self.registered{
            self.consumer.as_ref().unwrap().buffer.reactor.unregister_read_slots_available();
        }
    }
}