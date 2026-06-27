use alloc::boxed::Box;
use core::sync::atomic::{AtomicU8, Ordering};
use core::{cell::Cell, ptr::NonNull};

use super::IS_ABANDONED;
use super::{Consumer, Producer, RingBuffer};

// Non-public helper type.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ArcRingBuffer<T> {
    ptr: NonNull<RingBuffer<T>>,
}

// SAFETY: If RingBuffer is Send, ArcRingBuffer is as well.
unsafe impl<T> Send for ArcRingBuffer<T> where RingBuffer<T>: Send {}

impl<T> ArcRingBuffer<T> {
    // NB: this takes ownership of the RingBuffer, making sure that only one
    //     Producer and Consumer are ever created.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(rb: Box<RingBuffer<T>>) -> (Producer<T>, Consumer<T>) {
        debug_assert_eq!(rb.flags.load(Ordering::Relaxed) & IS_ABANDONED, 0);
        let head = rb.head.load(Ordering::Relaxed);
        let tail = rb.tail.load(Ordering::Relaxed);

        // We leak the `Box` here, but in the `Drop` implementation the pointer
        // will be turned back into a `Box` and its memory will be properly deallocated.
        let ptr = Box::leak(rb);
        // SAFETY: Pointer from `Box` is always non-null.
        let ptr = unsafe { NonNull::new_unchecked(ptr) };

        let p = Producer {
            buffer: Self { ptr },
            cached_head: Cell::new(head),
            cached_tail: Cell::new(tail),
        };
        let c = Consumer {
            buffer: Self { ptr },
            cached_head: Cell::new(head),
            cached_tail: Cell::new(tail),
        };
        (p, c)
    }
}

impl<T> Drop for ArcRingBuffer<T> {
    fn drop(&mut self) {
        // SAFETY: must point to initialized Storage.
        let flags: &AtomicU8 = unsafe { &self.ptr.as_ref().flags };
        // The "store" part of `fetch_or()` has to use `Release` to make sure that any previous writes
        // to the ring buffer happen before it (in the thread that drops first).
        // The "load" part can be `Relaxed` for the first thread,
        // but it must be `Acquire` for the second one (see below).
        if flags.fetch_or(IS_ABANDONED, Ordering::Release) & IS_ABANDONED == 0 {
            // The flag wasn't set before, so we are the first to drop our
            // producer/consumer and it should not be dropped yet.
        } else {
            // The flag was already set, i.e. the other thread has already dropped its
            // consumer/producer and it can be dropped now.

            // However, since the load of `flags` was `Relaxed`,
            // we have to use `Acquire` here to make sure that reading `head` and `tail`
            // in the destructor happens after this point.

            // Ideally, we would use a memory fence like this:
            //core::sync::atomic::fence(Ordering::Acquire);
            // ... but as long as ThreadSanitizer doesn't support fences,
            // we use load(Acquire) as a work-around to avoid false positives:
            let _ = flags.load(Ordering::Acquire);
            // SAFETY: RingBuffer has been allocated with `Box::new()`.
            unsafe {
                drop_slow(self.ptr);
            }
        }
    }
}

/// Non-inlined part of `ArcRingBuffer::drop()`.
#[inline(never)]
unsafe fn drop_slow<T>(ptr: NonNull<RingBuffer<T>>) {
    // SAFETY: The caller of `ArcRingBuffer::from_ptr()` has to guarantee that the memory
    // has been allocated with `Box::new()` or compatible.
    unsafe {
        // Turn the pointer into a `Box` and immediately drop it, which deallocates the memory.
        drop(Box::from_raw(ptr.as_ptr()));
    }
}

impl<T> core::ops::Deref for ArcRingBuffer<T> {
    type Target = RingBuffer<T>;

    fn deref(&self) -> &Self::Target {
        // SAFETY: There are never any mutable references.
        unsafe { self.ptr.as_ref() }
    }
}
