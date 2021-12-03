use core::{fmt, future::Future,  pin::Pin, sync::atomic::Ordering, task::{Context, Poll::{self, Ready}}};

use crate::{Producer,Consumer, chunks::{WriteChunk, WriteChunkUninit,ReadChunk}, reactor::AsyncReactorRegisterResult};

use super::AsyncReactor;

impl<T,U:AsyncReactor> Producer<T,U>{
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

impl<T,U:AsyncReactor> Consumer<T,U>{
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> core::fmt::Result {
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
                            AsyncReactorRegisterResult::Registered => {
                                this.registered = true;
                                return Poll::Pending
                            },
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
            producer: core::mem::take(&mut this.producer).unwrap(),
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
                            AsyncReactorRegisterResult::Registered => {
                                this.registered = true;
                                return Poll::Pending;
                            },
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
            consumer: core::mem::take(&mut this.consumer).unwrap(),
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