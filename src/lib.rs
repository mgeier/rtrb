//! A bounded single-producer single-consumer queue.
//!
//! # Examples
//!
//! ```
//! use rtrb::RingBuffer;
//!
//! let (p, c) = RingBuffer::new(2).split();
//!
//! assert!(p.push(1).is_ok());
//! assert!(p.push(2).is_ok());
//! assert!(p.push(3).is_err());
//!
//! assert_eq!(c.pop(), Ok(1));
//! assert_eq!(c.pop(), Ok(2));
//! assert!(c.pop().is_err());
//! ```

use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cache_padded::CachePadded;

mod error;

pub use error::{PopError, PushError};

/// A single-producer single-consumer queue.
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

    /// Indicates that dropping a `Buffer<T>` may drop elements of type `T`.
    _marker: PhantomData<T>,
}

impl<T> RingBuffer<T> {
    /// Creates a [RingBuffer] with the given capacity.
    ///
    /// The returned [RingBuffer] is typically immediately split into
    /// the producer and the consumer side by [split()].
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
    /// let rb = RingBuffer::<i32>::new(100);
    /// assert_eq!(rb.capacity(), 100);
    /// ```
    pub fn new(capacity: usize) -> RingBuffer<T> {
        assert!(capacity > 0, "capacity must be non-zero");

        // Allocate a buffer of length `capacity`.
        let buffer = {
            let mut v = Vec::<T>::with_capacity(capacity);
            let ptr = v.as_mut_ptr();
            mem::forget(v);
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
    /// let rb = RingBuffer::<i32>::new(100);
    ///
    /// assert_eq!(rb.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns a pointer to the slot at position `pos`.
    ///
    /// The position must be in range `0 .. 2 * capacity`.
    #[inline]
    unsafe fn slot(&self, pos: usize) -> *mut T {
        if pos < self.capacity {
            self.buffer.add(pos)
        } else {
            self.buffer.add(pos - self.capacity)
        }
    }

    /// Increments a position by going one slot forward.
    ///
    /// The position must be in range `0 .. 2 * capacity`.
    #[inline]
    fn increment(&self, pos: usize) -> usize {
        if pos < 2 * self.capacity - 1 {
            pos + 1
        } else {
            0
        }
    }

    /// Returns the distance between two positions.
    ///
    /// Positions must be in range `0 .. 2 * capacity`.
    #[inline]
    fn distance(&self, a: usize, b: usize) -> usize {
        if a <= b {
            b - a
        } else {
            2 * self.capacity - a + b
        }
    }
}

impl<T> Drop for RingBuffer<T> {
    fn drop(&mut self) {
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Relaxed);

        // Loop over all slots that hold a value and drop them.
        while head != tail {
            unsafe {
                self.slot(head).drop_in_place();
            }
            head = self.increment(head);
        }

        // Finally, deallocate the buffer, but don't run any destructors.
        unsafe {
            Vec::from_raw_parts(self.buffer, 0, self.capacity);
        }
    }
}

/// The producer side of a bounded single-producer single-consumer queue.
///
/// # Examples
///
/// ```
/// use rtrb::{PushError, RingBuffer};
///
/// let (p, c) = RingBuffer::<i32>::new(1).split();
///
/// assert_eq!(p.push(10), Ok(()));
/// assert_eq!(p.push(20), Err(PushError::Full(20)));
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
    /// If the queue is full, the element is returned back as an error.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{RingBuffer, PushError};
    ///
    /// let (p, c) = RingBuffer::new(1).split();
    ///
    /// assert_eq!(p.push(10), Ok(()));
    /// assert_eq!(p.push(20), Err(PushError::Full(20)));
    /// ```
    pub fn push(&self, value: T) -> Result<(), PushError<T>> {
        let mut head = self.head.get();
        let mut tail = self.tail.get();

        // Check if the queue is *possibly* full.
        if self.rb.distance(head, tail) == self.rb.capacity {
            // We need to refresh the head and check again if the queue is *really* full.
            head = self.rb.head.load(Ordering::Acquire);
            self.head.set(head);

            // Is the queue *really* full?
            if self.rb.distance(head, tail) == self.rb.capacity {
                return Err(PushError::Full(value));
            }
        }

        // Write the value into the tail slot.
        unsafe {
            self.rb.slot(tail).write(value);
        }

        // Move the tail one slot forward.
        tail = self.rb.increment(tail);
        self.rb.tail.store(tail, Ordering::Release);
        self.tail.set(tail);

        Ok(())
    }

    /// Returns the capacity of the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<i32>::new(100).split();
    ///
    /// assert_eq!(p.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.rb.capacity
    }
}

impl<T> fmt::Debug for Producer<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Producer { .. }")
    }
}

/// The consumer side of a bounded single-producer single-consumer queue.
///
/// # Examples
///
/// ```
/// use rtrb::{RingBuffer, PopError};
///
/// let (p, c) = RingBuffer::new(1).split();
/// assert_eq!(p.push(10), Ok(()));
///
/// assert_eq!(c.pop(), Ok(10));
/// assert_eq!(c.pop(), Err(PopError::Empty));
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
    /// If the queue is empty, an error is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::{PopError, RingBuffer};
    ///
    /// let (p, c) = RingBuffer::new(1).split();
    /// assert_eq!(p.push(10), Ok(()));
    ///
    /// assert_eq!(c.pop(), Ok(10));
    /// assert_eq!(c.pop(), Err(PopError::Empty));
    /// ```
    pub fn pop(&self) -> Result<T, PopError> {
        let mut head = self.head.get();
        let mut tail = self.tail.get();

        // Check if the queue is *possibly* empty.
        if head == tail {
            // We need to refresh the tail and check again if the queue is *really* empty.
            tail = self.rb.tail.load(Ordering::Acquire);
            self.tail.set(tail);

            // Is the queue *really* empty?
            if head == tail {
                return Err(PopError::Empty);
            }
        }

        // Read the value from the head slot.
        let value = unsafe { self.rb.slot(head).read() };

        // Move the head one slot forward.
        head = self.rb.increment(head);
        self.rb.head.store(head, Ordering::Release);
        self.head.set(head);

        Ok(value)
    }

    /// Returns the capacity of the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use rtrb::RingBuffer;
    ///
    /// let (p, c) = RingBuffer::<i32>::new(100).split();
    ///
    /// assert_eq!(c.capacity(), 100);
    /// ```
    pub fn capacity(&self) -> usize {
        self.rb.capacity
    }
}

impl<T> fmt::Debug for Consumer<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("Consumer { .. }")
    }
}
