use core::{sync::atomic::Ordering, task::Waker};
use std::{cell::{Cell, UnsafeCell}, fmt::Debug, sync::atomic::AtomicUsize};

use crate::{Consumer, Producer,reactor::Reactor};

pub trait AsyncReactor:Reactor{
    #[must_use]
    fn register_read_slots_available<T>(consumer:&&mut Consumer<T,Self>,waker:&Waker,required_slots:usize)->AsyncReactorRegisterResult;
    #[must_use]
    fn register_write_slots_available<T>(producer:&&mut Producer<T,Self>,waker:&Waker,required_slots:usize)->AsyncReactorRegisterResult;

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

#[derive(Debug,Default)]
pub struct CommitterWaitFreeReactor{
    state:AtomicUsize,
    required_slots:Cell<usize>,
    producer_waker:UnsafeCell<Option<Waker>>,
    consumer_waker:UnsafeCell<Option<Waker>>,
    cached_abandoned:Cell<bool>,
}

const WAITING:usize = 0;
const REGISTERING:usize = 0b00001;
const ABANDONED:usize = 0b00010;
const UNREGISTERED_PRODUCER:usize = 0b00100;
const UNREGISTERED_CONSUMER:usize = 0b01000;
const WAKING:usize = 0b10000;
const MAX_WAKINGS:usize = usize::MAX >> WAKING.trailing_zeros();
impl Reactor for CommitterWaitFreeReactor{
    
    fn pushed<T>(producer:&Producer<T,Self>) {
        let buffer = &producer.buffer;
        let reactor = &buffer.reactor;
        if reactor.cached_abandoned.get(){
            return;
        }
        match reactor.state.fetch_add(WAKING,Ordering::Acquire){
            UNREGISTERED_CONSUMER => {
                unsafe{(*reactor.consumer_waker.get()) = None};
                reactor.state.fetch_sub(WAKING | UNREGISTERED_CONSUMER,Ordering::Release);
            },
            s => {
                if s & !UNREGISTERED_PRODUCER == 0 {
                    let available_slots = producer.buffer.distance(buffer.head.load(Ordering::Relaxed), producer.tail.get());
                    if reactor.required_slots.get() <= available_slots{
                        if let Some(waker) = unsafe{(*reactor.consumer_waker.get()).take()}{
                            waker.wake();
                        }
                    }
                    reactor.state.fetch_sub(WAKING | (s & UNREGISTERED_PRODUCER),Ordering::Release);
                    return;
                }
                if s & ABANDONED != 0{
                    producer.head.set(buffer.head.load(Ordering::Relaxed));
                    reactor.cached_abandoned.set(true);
                }
                if s >= !(WAKING-1){
                    panic!("Commited more than {} times while opponent thread is registering. This might be a bug or opponent thread has been aborted.",MAX_WAKINGS)
                }
            },
        }
    }

    fn popped<T>(consumer:&Consumer<T,Self>) {
        let buffer = &consumer.buffer;
        let reactor = &buffer.reactor;
        if reactor.cached_abandoned.get(){
            return;
        }
        match reactor.state.fetch_add(WAKING,Ordering::Acquire){
            UNREGISTERED_PRODUCER => {
                unsafe{(*reactor.producer_waker.get()) = None};
                reactor.state.fetch_sub(WAKING | UNREGISTERED_PRODUCER,Ordering::Release);
            },
            s => {
                if s & !UNREGISTERED_CONSUMER == 0 {
                    let available_slots = buffer.capacity - consumer.buffer.distance(consumer.head.get(), buffer.tail.load(Ordering::Relaxed));
                    if reactor.required_slots.get() <= available_slots{
                        if let Some(waker) = unsafe{(*reactor.producer_waker.get()).take()}{
                            waker.wake();
                        }
                    }
                    reactor.state.fetch_sub(WAKING | (s & UNREGISTERED_CONSUMER),Ordering::Release);
                    return;
                }
                if s & ABANDONED != 0{
                    consumer.tail.set(buffer.tail.load(Ordering::Relaxed));
                    reactor.cached_abandoned.set(true);
                }
                if s >= !(WAKING-1){
                    panic!("Commited more than {} times while opponent thread is registering. This might be a bug or opponent thread has been aborted.",MAX_WAKINGS)
                }
            },
        }
    }

    fn dropping_producer<T>(producer:&Producer<T,Self>) {
        let reactor = &producer.buffer.reactor;
        match reactor.state.fetch_or(ABANDONED,Ordering::AcqRel){
            WAITING=> {
                if let Some(waker) = unsafe{(*reactor.consumer_waker.get()).take()}{
                    waker.wake();
                }
            },
            _ => (),
        }
    }

    fn dropping_consumer<T>(consumer:&Consumer<T,Self>) {
        let reactor = &consumer.buffer.reactor;
        match reactor.state.fetch_or(ABANDONED,Ordering::AcqRel){
            WAITING=> {
                if let Some(waker) = unsafe{(*reactor.producer_waker.get()).take()}{
                    waker.wake();
                }
            },
            _ => (),
        }
    }
}

impl AsyncReactor for CommitterWaitFreeReactor{
    fn register_read_slots_available<T>(consumer:&&mut Consumer<T,Self>,waker:&Waker,required_slots:usize)->AsyncReactorRegisterResult {
        let buffer = consumer.buffer.as_ref();
        let reactor = &buffer.reactor;
        let cached_abandoned = &reactor.cached_abandoned;
        let head = consumer.head.get();
        if cached_abandoned.get(){
            let available_slots = buffer.distance(head, consumer.tail.get());
            return AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots);
        }
        let tail = &buffer.tail;
        let state = &reactor.state;
        let mut expected = WAITING;
        loop{
            match state.compare_exchange_weak(expected, REGISTERING, Ordering::Acquire, Ordering::Acquire){
                Ok(_) => {
                    if expected & UNREGISTERED_PRODUCER != 0{
                        unsafe{*reactor.producer_waker.get() = None};
                    }
                    if expected & UNREGISTERED_CONSUMER !=0{
                        unsafe{*reactor.consumer_waker.get() = None};
                    }
                    let read_slots = buffer.distance(head, tail.load(Ordering::Acquire));
                    let available_slots = read_slots;
                    if required_slots <= available_slots{
                        state.fetch_and(ABANDONED, Ordering::Release);
                        return AsyncReactorRegisterResult::AlreadyAvailable;
                    }                
                    if unsafe{(*reactor.producer_waker.get()).is_some()}{
                        state.fetch_and(ABANDONED, Ordering::Release);
                        let write_slots = buffer.capacity- read_slots;
                        let write_insufficient = reactor.required_slots.get() - write_slots;
                        return AsyncReactorRegisterResult::WillDeadlock(write_insufficient,available_slots);
                    }
                    unsafe{*reactor.consumer_waker.get() = Some(waker.clone())};
                    reactor.required_slots.set(required_slots);
                    let mut expected = REGISTERING;    
                    loop{
                        match state.compare_exchange_weak(expected, WAITING, Ordering::AcqRel, Ordering::Acquire){
                            Ok(_) => {
                                return AsyncReactorRegisterResult::Registered;
                            },
                            Err(actual) => {
                                if actual & ABANDONED != 0{
                                    break;
                                }
                                let available_slots = buffer.distance(head, tail.load(Ordering::Acquire));
                                if required_slots <= available_slots{
                                    unsafe{*reactor.consumer_waker.get() = None};
                                    state.fetch_and(ABANDONED, Ordering::Release);
                                    return AsyncReactorRegisterResult::AlreadyAvailable;
                                }
                                expected = actual;
                            },
                        }
                    }
                    break;
                },
                Err(actual) => {
                    if actual & ABANDONED !=0{
                        break;
                    }
                    let available_slots = buffer.distance(head, tail.load(Ordering::Acquire));
                    if required_slots <= available_slots{
                        return AsyncReactorRegisterResult::AlreadyAvailable;
                    }
                    expected = actual & (UNREGISTERED_PRODUCER | UNREGISTERED_CONSUMER);
                },
            }
        }
        // When acquired `ABANDONED`
        let tail = tail.load(Ordering::Relaxed);
        consumer.tail.set(tail);
        cached_abandoned.set(true);
        let available_slots = buffer.distance(head, tail);
        if available_slots < required_slots{
            return AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots);
        }else{
            return AsyncReactorRegisterResult::AlreadyAvailable;
        }
    }

    fn register_write_slots_available<T>(producer:&&mut Producer<T,Self>,waker:&Waker,required_slots:usize)->AsyncReactorRegisterResult {
        let buffer = producer.buffer.as_ref();
        let capacity = buffer.capacity;
        let reactor = &buffer.reactor;
        let cached_abandoned = &reactor.cached_abandoned;
        let tail = producer.tail.get();
        if cached_abandoned.get(){
            let available_slots = capacity - buffer.distance(producer.head.get(), tail);
            return AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots);
        }
        let head = &buffer.head;
        let state = &reactor.state;
        let mut expected = WAITING;
        loop{
            match state.compare_exchange_weak(expected, REGISTERING, Ordering::Acquire, Ordering::Acquire){
                Ok(_) => {
                    if expected & UNREGISTERED_PRODUCER != 0{
                        unsafe{*reactor.producer_waker.get() = None};
                    }
                    if expected & UNREGISTERED_CONSUMER !=0{
                        unsafe{*reactor.consumer_waker.get() = None};
                    }
                    let read_slots = buffer.distance(head.load(Ordering::Acquire), tail);
                    let available_slots = capacity - read_slots;
                    if required_slots <= available_slots{
                        state.fetch_and(ABANDONED, Ordering::Release);
                        return AsyncReactorRegisterResult::AlreadyAvailable;
                    }                
                    if unsafe{(*reactor.consumer_waker.get()).is_some()}{
                        state.fetch_and(ABANDONED, Ordering::Release);
                        let read_insufficient = reactor.required_slots.get() - read_slots;
                        return AsyncReactorRegisterResult::WillDeadlock(read_insufficient,available_slots);
                    }
                    unsafe{*reactor.producer_waker.get() = Some(waker.clone())};
                    reactor.required_slots.set(required_slots);
                    let mut expected = REGISTERING;    
                    loop{
                        match state.compare_exchange_weak(expected, WAITING, Ordering::AcqRel, Ordering::Acquire){
                            Ok(_) => {
                                return AsyncReactorRegisterResult::Registered;
                            },
                            Err(actual) => {
                                if actual & ABANDONED != 0{
                                    break;
                                }
                                let available_slots = capacity - buffer.distance(head.load(Ordering::Acquire), tail);
                                if required_slots <= available_slots{
                                    unsafe{*reactor.consumer_waker.get() = None};
                                    state.fetch_and(ABANDONED, Ordering::Release);
                                    return AsyncReactorRegisterResult::AlreadyAvailable;
                                }
                                expected = actual;
                            },
                        }
                    }
                    break;
                },
                Err(actual) => {
                    if actual & ABANDONED !=0{
                        break;
                    }
                    let available_slots = capacity - buffer.distance(head.load(Ordering::Acquire), tail);
                    if required_slots <= available_slots{
                        return AsyncReactorRegisterResult::AlreadyAvailable;
                    }
                    expected = actual & (UNREGISTERED_PRODUCER | UNREGISTERED_CONSUMER);
                },
            }
        }
        // When acquired `ABANDONED`
        let head = head.load(Ordering::Relaxed);
        producer.head.set(head);
        cached_abandoned.set(true);
        let available_slots = capacity - buffer.distance(head, tail);
        if available_slots < required_slots{
            return AsyncReactorRegisterResult::TooFewSlotsAndAbandoned(available_slots);
        }else{
            return AsyncReactorRegisterResult::AlreadyAvailable;
        }
    }

    fn unregister_read_slots_available(&self) {
        self.state.fetch_or(UNREGISTERED_CONSUMER, Ordering::Release);
    }

    fn unregister_write_slots_available(&self) {
        self.state.fetch_or(UNREGISTERED_PRODUCER, Ordering::Release);
    }
}