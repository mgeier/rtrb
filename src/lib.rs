//! A realtime-safe single-producer single-consumer (SPSC) ring buffer.
//!
//! A [`RingBuffer`] consists of two parts:
//! a [`Producer`] for writing into the ring buffer and
//! a [`Consumer`] for reading from the ring buffer.
//!
//! A fixed-capacity buffer is allocated on construction.
//! After that, no more memory is allocated (unless the type `T` does that internally).
//! Reading from and writing into the ring buffer is *lock-free* and *wait-free*.
//! All reading and writing functions return immediately.
//! Attempts to write to a full buffer return an error;
//! values inside the buffer are *not* overwritten.
//! Attempts to read from an empty buffer return an error as well.
//! Only a single thread can write into the ring buffer and a single thread
//! (typically a different one) can read from the ring buffer.
//! If the queue is empty, there is no way for the reading thread to wait
//! for new data, other than trying repeatedly until reading succeeds.
//! Similarly, if the queue is full, there is no way for the writing thread
//! to wait for newly available space to write to, other than trying repeatedly.
//!
//! # Examples
//!
//! Moving single elements into and out of a queue with
//! [`Producer::push()`] and [`Consumer::pop()`], respectively:
//!
//! ```
//! use rtrb::{RingBuffer, PushError, PopError};
//!
//! let (mut producer, mut consumer) = RingBuffer::new(2);
//!
//! assert_eq!(producer.push(10), Ok(()));
//! assert_eq!(producer.push(20), Ok(()));
//! assert_eq!(producer.push(30), Err(PushError::Full(30)));
//!
//! std::thread::spawn(move || {
//!     assert_eq!(consumer.pop(), Ok(10));
//!     assert_eq!(consumer.pop(), Ok(20));
//!     assert_eq!(consumer.pop(), Err(PopError::Empty));
//! }).join().unwrap();
//! ```
//!
//! See the documentation of the [`chunks#examples`] module
//! for examples that write/read multiple items at once.
//! See also:
//!
//!   * [`Producer::write_chunk()`]
//!   * [`Producer::write_chunk_uninit()`]
//!   * [`Producer::push_partial_slice()`] (if `T: Copy`)
//!   * [`Producer::push_entire_slice()`] (if `T: Copy`)
//!
//!   * [`Consumer::read_chunk()`]
//!   * [`Consumer::pop_partial_slice()`] (if `T: Copy`)
//!   * [`Consumer::pop_partial_slice_uninit()`] (if `T: Copy`)
//!   * [`Consumer::pop_entire_slice()`] (if `T: Copy`)
//!   * [`Consumer::pop_entire_slice_uninit()`] (if `T: Copy`)
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(rust_2018_idioms)]
#![deny(missing_docs, missing_debug_implementations)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::undocumented_unsafe_blocks, clippy::unnecessary_safety_comment)]

extern crate alloc;

use core::fmt;
use core::mem::MaybeUninit;

#[allow(dead_code, clippy::undocumented_unsafe_blocks)]
mod cache_padded;
/// Re-export from [`crossbeam_utils::CachePadded`](https://docs.rs/crossbeam-utils/).
#[doc(inline)]
pub use cache_padded::CachePadded;

mod ring_buffer;
pub use ring_buffer::RingBuffer;
mod producer;
pub use producer::Producer;
mod consumer;
pub use consumer::Consumer;

pub mod chunks;

// This is used in the documentation.
#[allow(unused_imports)]
use chunks::WriteChunkUninit;

/// Extension trait used to provide a [`copy_to_uninit()`](CopyToUninit::copy_to_uninit)
/// method on built-in slices.
///
/// This can be used to safely copy data to the slices returned from
/// [`WriteChunkUninit::as_mut_slices()`].
///
/// To use this, the trait has to be brought into scope, e.g. with:
///
/// ```
/// use rtrb::CopyToUninit;
/// ```
pub trait CopyToUninit<T: Copy> {
    /// Copies contents to a possibly uninitialized slice.
    fn copy_to_uninit<'a>(&self, dst: &'a mut [MaybeUninit<T>]) -> &'a mut [T];
}

impl<T: Copy> CopyToUninit<T> for [T] {
    /// Copies contents to a possibly uninitialized slice.
    ///
    /// # Panics
    ///
    /// This function will panic if the two slices have different lengths.
    fn copy_to_uninit<'a>(&self, dst: &'a mut [MaybeUninit<T>]) -> &'a mut [T] {
        assert_eq!(
            self.len(),
            dst.len(),
            "source slice length does not match destination slice length"
        );
        let dst_ptr = dst.as_mut_ptr().cast();
        // SAFETY: The lengths have been checked to be equal and
        // the mutable reference makes sure that there is no overlap.
        unsafe {
            self.as_ptr().copy_to_nonoverlapping(dst_ptr, self.len());
            core::slice::from_raw_parts_mut(dst_ptr, self.len())
        }
    }
}

/// Error type for [`Consumer::pop()`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PopError {
    /// The queue was empty.
    Empty,
}

#[cfg(feature = "std")]
impl std::error::Error for PopError {}

impl fmt::Display for PopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PopError::Empty => "empty ring buffer".fmt(f),
        }
    }
}

/// Error type for [`Consumer::peek()`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PeekError {
    /// The queue was empty.
    Empty,
}

#[cfg(feature = "std")]
impl std::error::Error for PeekError {}

impl fmt::Display for PeekError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeekError::Empty => "empty ring buffer".fmt(f),
        }
    }
}

/// Error type for [`Producer::push()`].
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum PushError<T> {
    /// The queue was full.
    Full(T),
}

#[cfg(feature = "std")]
impl<T> std::error::Error for PushError<T> {}

impl<T> fmt::Debug for PushError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::Full(_) => f.pad("Full(_)"),
        }
    }
}

impl<T> fmt::Display for PushError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::Full(_) => "full ring buffer".fmt(f),
        }
    }
}

/// Error type for [`Consumer::read_chunk()`], [`Consumer::pop_entire_slice()`],
/// [`Consumer::pop_entire_slice_uninit()`], [`Producer::write_chunk()`],
/// [`Producer::write_chunk_uninit()`] and [`Producer::push_entire_slice()`].
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
