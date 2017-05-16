#![crate_type = "lib"]
#![cfg_attr(feature = "clippy", feature(plugin))]
#![cfg_attr(feature = "clippy", plugin(clippy))]


extern crate tokio_core;
#[macro_use]
extern crate futures;

use std::time::{Duration, Instant};
use std::cell::RefCell;
use std::rc::Rc;
use futures::stream::Stream;
use futures::sink::{Sink};
use futures::Future;
use futures::{Poll, StartSend, AsyncSink, Async};
use std::mem;
use futures::task;
use tokio_core::reactor::Timeout;
use tokio_core::reactor::Handle;

pub struct Config {
    pub capacity: f64,
    pub quantum: f64,
    pub interval: Duration,
}

pub struct Shared {
    config: Config,
    available: f64,
    sink_task: Option<task::Task>,
    stream_task: Option<task::Task>,
    future_task: Option<task::Task>,
    need_timer_reset: bool,
    last_stat: usize,
    current_stat: usize,
    current_stat_from: Instant,
}

impl Shared {
    fn unpark_sink(&self) {
        if let Some(ref task) = self.sink_task {
            task.unpark();
        };
    }

    fn unpark_future(&self) {
        if let Some(ref task) = self.future_task {
            task.unpark();
        };
    }
}

#[derive(Clone)]
pub struct TokenBucketHandle {
    shared: Rc<RefCell<Shared>>,
}

impl TokenBucketHandle {
    pub fn add_tokens(&self, tokens: f64) {
        let shared = &mut *(self.shared.borrow_mut());
        shared.available += tokens;
        shared.unpark_sink()
    }

    pub fn set_capacity(&self, capacity: f64) {
        let shared = &mut *(self.shared.borrow_mut());
        shared.config.capacity = capacity;
        shared.unpark_sink()
    }

    pub fn set_quantum(&self, quantum: f64) {
        let shared = &mut *(self.shared.borrow_mut());
        shared.config.quantum = quantum;
        shared.unpark_sink()
    }

    pub fn set_interval(&self, interval: Duration) {
        let shared = &mut *(self.shared.borrow_mut());
        shared.config.interval = interval;
        shared.need_timer_reset = true;
        shared.unpark_future()
    }
}


pub struct TokenBucket {
    shared: Rc<RefCell<Shared>>,
    timer: Option<Timeout>,
    handle: Handle,
}

pub struct TokenBucketLimiter<U> {
    shared: Rc<RefCell<Shared>>,
    item: Option<U>,
}

impl<U> Stream for TokenBucketLimiter<U> {
    type Item = U;
    type Error = TokenBucketError;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        let mut shared = self.shared.borrow_mut();
        if self.item.is_none() {
            shared.stream_task = Some(task::park());
            return Ok(Async::NotReady);
        }
        let old = mem::replace(&mut self.item, None);
        (*shared).unpark_sink();
        Ok(Async::Ready(old))
    }
}

pub struct TokenBucketError;

impl<U> Sink for TokenBucketLimiter<U> {
    type SinkItem = (U, usize);
    type SinkError = bool; //FIXME: TokenBucketError?;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        let shared = &mut *(self.shared.borrow_mut());
        if self.item.is_some() {
            shared.sink_task = Some(task::park());
            return Ok(AsyncSink::NotReady(item)) //No space for new item
        }

        let need = item.1 as f64;

        if shared.available < need {
            shared.sink_task = Some(task::park());
            return Ok(AsyncSink::NotReady(item))
        }

        shared.current_stat += item.1;
        let now = Instant::now();

        //This is very naive implementation of bandwidth meter
        //Future should be polled each second to get the data
        if now.duration_since(shared.current_stat_from) > Duration::new(1, 0) {
            shared.last_stat = shared.current_stat;
            shared.current_stat = 0;
            shared.current_stat_from = now;
            println!("Bandwidth is {}bytes/sec", shared.last_stat);
        }

        shared.available -= need;
        self.item = Some(item.0);
        if let Some(ref task) = shared.stream_task {
            task.unpark();
        };
        Ok(AsyncSink::Ready)
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        Ok(Async::Ready(()))
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        unimplemented!()
    }
}


impl TokenBucket {
    pub fn configured(config: Config, handle: &Handle) -> TokenBucket {
        let capacity = config.capacity;
        TokenBucket {
            shared: Rc::new(RefCell::new(
                Shared {
                    config: config,
                    available: capacity,
                    sink_task: None,
                    stream_task: None,
                    future_task: None,
                    need_timer_reset: false,
                    last_stat: 0,
                    current_stat: 0,
                    current_stat_from: Instant::now(),
                }
            )),
            timer: None,
            handle: handle.clone(),
        }
    }

    pub fn limiter<U>(&self) -> TokenBucketLimiter<U> {
        TokenBucketLimiter {
            shared: self.shared.clone(),
            item: None,
        }
    }

    pub fn handle(&self) -> TokenBucketHandle {
        TokenBucketHandle {
            shared: self.shared.clone(),
        }
    }
}

impl Future for TokenBucket {
    type Item = ();
    type Error = std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let shared = &mut *(self.shared.borrow_mut());

        let task = task::park();
        shared.future_task = Some(task);

        if shared.need_timer_reset {
            mem::replace(&mut self.timer,
                         Some(
                             Timeout::new(shared.config.interval, &self.handle)
                                 .expect("Failed to initialize timer")));
            shared.need_timer_reset = false;
        }
        if let Some(ref mut timer) = self.timer {
            try_ready!(timer.poll());
            shared.available += shared.config.quantum;
            if shared.available > shared.config.capacity {
                shared.available = shared.config.capacity;
            }
            shared.unpark_sink();
        }

        mem::replace(&mut self.timer,
                     Some(
                         Timeout::new(shared.config.interval, &self.handle)
                             .expect("Failed to initialize timer")));

        shared.unpark_future();

        Ok(Async::NotReady)
    }
}
