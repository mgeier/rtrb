use crate::{Consumer, Producer};
#[cfg(feature = "async")]
pub use crate::async_rtrb::async_reactor::*;
/// Used for internal event handling.
pub trait Reactor:Default{
    /// Called when new read slots are available. Should be called from producer thread.
    fn pushed<T>(producer:&Producer<T,Self>);
    /// Called when new write slots are available. Should be called from consumer thread.
    fn popped<T>(consumer:&Consumer<T,Self>);
    /// Called when dropping producer.
    fn dropping_producer<T>(producer:&Producer<T,Self>);
    /// Called when dropping consumer.
    fn dropping_consumer<T>(consumer:&Consumer<T,Self>);
}
/// Notifier which doesn't do any actual notification.
#[derive(Debug,Default)]
pub struct DummyReactor;

impl Reactor for DummyReactor{
    #[inline]
    fn pushed<T>(_producer:&Producer<T,Self>) {}

    #[inline]
    fn popped<T>(_consumer:&Consumer<T,Self>) {}

    #[inline]
    fn dropping_producer<T>(_producer:&Producer<T,Self>) {}

    #[inline]
    fn dropping_consumer<T>(_consumer:&Consumer<T,Self>) {}
}
