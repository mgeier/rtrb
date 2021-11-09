use core::{ops::DerefMut, sync::atomic::Ordering, task::Waker};

use parking_lot::Mutex;

use crate::{Consumer, Producer, reactor::Reactor};

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
