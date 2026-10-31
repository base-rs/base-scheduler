// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! A scheduler-local retry pulse, not a reactor. Waiting sockets occupy no worker or admission slot.

use base_scheduler::{Cancellation, OperationResult};
use event_listener::Event;
use std::future::{Future, poll_fn};
use std::io;
use std::pin::pin;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::task::Poll;
use std::thread;
use std::time::Duration;

struct State {
    closed: bool,
    /// The worker is in its indefinite wait and needs a notification to resume pulsing.
    sleeping: bool,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    retry: Event,
    #[cfg(test)]
    idle: Event,
}

pub(crate) struct SocketRetry {
    shared: Arc<Shared>,
    started: OnceLock<io::Result<()>>,
    #[cfg(test)]
    pub(crate) fail_start: bool,
}

impl SocketRetry {
    pub(crate) fn new() -> Self {
        let retry = Event::new();
        retry.notify(0);
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    closed: false,
                    sleeping: false,
                }),
                work: Condvar::new(),
                retry,
                #[cfg(test)]
                idle: Event::new(),
            }),
            started: OnceLock::new(),
            #[cfg(test)]
            fail_start: false,
        }
    }

    pub(crate) fn check_open(&self) -> io::Result<()> {
        if self.shared.state.lock().unwrap().closed {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scheduler is closed",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) async fn wait(&self, cancellation: &Cancellation) -> OperationResult<()> {
        event_listener::listener!(self.shared.retry => retry);
        {
            let state = self.shared.state.lock().unwrap();
            cancellation.check()?;
            if state.closed {
                return Err(
                    io::Error::new(io::ErrorKind::BrokenPipe, "scheduler is closed").into(),
                );
            }
        }
        self.started
            .get_or_init(|| {
                #[cfg(test)]
                if self.fail_start {
                    return Err(io::Error::other("injected socket retry startup failure"));
                }
                let shared = Arc::clone(&self.shared);
                thread::Builder::new()
                    .name("base-direct-socket-retry".into())
                    .spawn(move || work(shared))
                    .map(drop)
            })
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), error.to_string()))?;
        // Registration precedes this check. The worker checks listeners under the same lock before sleeping indefinitely, so either it observes this listener or it is already sleeping and we wake it. A pulse in progress already covers this listener; cutting its period short would let idle waiters keep waking each other.
        {
            let mut state = self.shared.state.lock().unwrap();
            if state.sleeping {
                state.sleeping = false;
                self.shared.work.notify_one();
            }
        }
        let mut cancelled = pin!(cancellation.cancelled());
        let mut retry = pin!(retry);
        poll_fn(|context| {
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(base_scheduler::Cancelled.into()));
            }
            retry.as_mut().poll(context).map(|()| Ok(()))
        })
        .await
    }

    pub(crate) fn close(&self) {
        self.shared.state.lock().unwrap().closed = true;
        self.shared.work.notify_all();
        self.shared.retry.notify(usize::MAX);
    }
}

impl Drop for SocketRetry {
    fn drop(&mut self) {
        self.close();
    }
}

fn work(shared: Arc<Shared>) {
    let mut state = shared.state.lock().unwrap();
    while !state.closed {
        if shared.retry.total_listeners() == 0 {
            state.sleeping = true;
            #[cfg(test)]
            shared.idle.notify(usize::MAX);
            state = shared.work.wait(state).unwrap();
            // A spurious wakeup simply rechecks listeners.
            state.sleeping = false;
        } else {
            // Only closure notifies this wait; new waiters join the pulse already in progress.
            state = shared
                .work
                .wait_timeout(state, Duration::from_millis(2))
                .unwrap()
                .0;
            // Wake executor code outside backend locks.
            drop(state);
            shared.retry.notify(usize::MAX);
            state = shared.state.lock().unwrap();
        }
    }
}

#[cfg(test)]
mod tests;
