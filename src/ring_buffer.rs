use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::sync::atomic::{AtomicUsize, Ordering};

// Padded indices to avoid false sharing.
use crate::CachePadded;

use super::{Consumer, Producer};

/// A bounded single-producer single-consumer (SPSC) queue.
///
/// Elements can be written with a [`Producer`] and read with a [`Consumer`],
/// both of which can be obtained with [`RingBuffer::new()`].
///
/// *See also the [crate-level documentation](crate).*
#[derive(Debug)]
pub struct RingBuffer<T> {
    /// The head of the queue.
    ///
    /// This integer is in range `0 .. 2 * capacity`.
    pub(super) head: CachePadded<AtomicUsize>,

    /// The tail of the queue.
    ///
    /// This integer is in range `0 .. 2 * capacity`.
    pub(super) tail: CachePadded<AtomicUsize>,

    /// The buffer holding slots.
    data_ptr: *mut T,
    capacity: usize,

    /// Indicates that dropping a `RingBuffer<T>` may drop elements of type `T`.
    _marker: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    pub(super) fn data_ptr(&self) -> *mut T {
        self.data_ptr
    }
}

impl<T> Drop for RingBuffer<T> {
    /// Drops all non-empty slots and deallocates the storage.
    fn drop(&mut self) {
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        // Loop over all slots that hold a value and drop them.
        while head != tail {
            // SAFETY: All slots between head and tail have been initialized.
            unsafe { self.slot_ptr(head).drop_in_place() };
            head = self.increment1(head);
        }

        // Finally, deallocate the buffer, but don't run any destructors.
        // SAFETY: data_ptr and capacity are still valid from the original initialization.
        unsafe { Vec::from_raw_parts(self.data_ptr, 0, self.capacity()) };
    }
}

impl<T> PartialEq for RingBuffer<T> {
    /// This method tests for `self` and `other` values to be equal, and is used by `==`.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p1, c1) = RingBuffer::<f32>::new(1000);
    /// assert_eq!(p1.buffer(), c1.buffer());
    ///
    /// let (p2, c2) = RingBuffer::<f32>::new(1000);
    /// assert_ne!(p1.buffer(), p2.buffer());
    /// ```
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}

impl<T> Eq for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Creates a ring buffer with the given `capacity`
    /// and returns [`Producer`] and [`Consumer`].
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (producer, consumer) = RingBuffer::<f32>::new(100);
    /// ```
    ///
    /// Specifying an explicit type
    /// with the [turbofish](https://turbo.fish/)
    /// is is only necessary if it cannot be deduced by the compiler.
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (mut producer, consumer) = RingBuffer::new(100);
    /// assert_eq!(producer.push(0.0f32), Ok(()));
    /// ```
    #[allow(clippy::new_ret_no_self)]
    pub fn new(capacity: usize) -> (Producer<T>, Consumer<T>) {
        let buffer = Arc::new(RingBuffer {
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
            data_ptr: ManuallyDrop::new(Vec::with_capacity(capacity)).as_mut_ptr(),
            capacity,
            _marker: PhantomData,
        });
        let p = Producer {
            buffer: buffer.clone(),
            cached_head: Cell::new(0),
            cached_tail: Cell::new(0),
        };
        let c = Consumer {
            buffer,
            cached_head: Cell::new(0),
            cached_tail: Cell::new(0),
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
    /// let (producer, consumer) = RingBuffer::<f32>::new(100);
    /// assert_eq!(producer.buffer().capacity(), 100);
    /// assert_eq!(consumer.buffer().capacity(), 100);
    /// // Both producer and consumer of course refer to the same ring buffer:
    /// assert_eq!(producer.buffer(), consumer.buffer());
    /// ```
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub(super) fn collapse_position(&self, pos: usize) -> usize {
        // Wraps from the range `0 .. 2 * capacity` to `0 .. capacity`.
        debug_assert!(pos == 0 || pos < 2 * self.capacity());
        if pos < self.capacity() {
            pos
        } else {
            pos - self.capacity()
        }
    }

    /// Returns a pointer to the (possibly uninitialized) slot at position `pos`.
    ///
    /// # Safety
    ///
    /// `pos` must be valid.
    ///
    /// If `pos == 0 && capacity == 0`, the returned pointer must not be dereferenced!
    pub(super) unsafe fn slot_ptr(&self, pos: usize) -> *mut T {
        debug_assert!(pos == 0 || pos < 2 * self.capacity);
        // SAFETY: See docstring.
        unsafe { self.data_ptr().add(self.collapse_position(pos)) }
    }

    /// Increments a position by going `n` slots forward.
    pub(super) fn increment(&self, pos: usize, n: usize) -> usize {
        debug_assert!(pos == 0 || pos < 2 * self.capacity());
        debug_assert!(n <= self.capacity());
        let threshold = 2 * self.capacity() - n;
        if pos < threshold {
            pos + n
        } else {
            pos - threshold
        }
    }

    /// Increments a position by going one slot forward.
    ///
    /// This might be more efficient than self.increment(..., 1).
    pub(super) fn increment1(&self, pos: usize) -> usize {
        debug_assert_ne!(self.capacity(), 0);
        debug_assert!(pos < 2 * self.capacity());
        if pos < 2 * self.capacity() - 1 {
            pos + 1
        } else {
            0
        }
    }

    /// Returns the distance between two positions.
    pub(super) fn distance(&self, a: usize, b: usize) -> usize {
        debug_assert!(a == 0 || a < 2 * self.capacity());
        debug_assert!(b == 0 || b < 2 * self.capacity());
        if a <= b {
            b - a
        } else {
            2 * self.capacity() - a + b
        }
    }
}
