#[cfg(feature = "async")]
pub use crate::async_rtrb::MutexReactor;
/// Actually used when async read write operations.
pub trait Reactor:Default{
    /// Should call when new read slots are available. Should be called from producer thread.
    fn pushed(&self,n:usize);
    /// Should call when new read slots are available. Should be called from producer thread.
    fn pushed1(&self){
        self.pushed(1);
    }
    /// Should call when new write slots are available. Should be called from consumer thread.
    fn popped(&self,n:usize);
    /// Should call when new write slots are available. Should be called from consumer thread.
    fn popped1(&self){
        self.popped(1);
    }
    /// Should call when consumer or producer has been abandoned.
    fn abandoned(&self);
    }

/// Notifier which doesn't do any actual notification.
#[derive(Debug,Default)]
pub struct DummyReactor;

impl Reactor for DummyReactor{
    #[inline]
    fn pushed(&self,_n:usize) {}

    #[inline]
    fn pushed1(&self){}

    #[inline]
    fn popped(&self,_n:usize) {}

    #[inline]
    fn popped1(&self){}

    #[inline]
    fn abandoned(&self) {}
}
