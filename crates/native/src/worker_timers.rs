// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{task_service::Shared, worker};
use base_scheduler::{Cancellation, Clock, OperationResult};
use base_scheduler_timers::local::TimerKey;
use std::{
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::Arc,
    task::{Poll, Waker},
    time::{Duration, Instant},
};

pub(crate) enum Command {
    Update(TimerKey, Waker),
    Release(TimerKey),
    #[cfg(test)]
    Inspect(std::sync::mpsc::Sender<usize>),
}

pub(crate) fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "clock is closed")
}

pub(crate) fn now(_shared: &Shared) -> Instant {
    #[cfg(test)]
    if let Some(now) = *_shared.paused.lock().unwrap() {
        return now;
    }
    Instant::now()
}

pub(crate) fn rejected(command: Command) {
    match command {
        // Closure or expiry can overtake an update. Its new context must still be notified.
        Command::Update(_, waker) => crate::panic::contain(|| waker.wake()),
        Command::Release(_) => {}
        #[cfg(test)]
        Command::Inspect(reply) => {
            let _ = reply.send(0);
        }
    }
}

// Keys never migrate. A retained owner makes cancellation/drop safe even when a pending sleep moves to a different executor or outlives scheduler shutdown. `None` owns a key in the shared root queue.
pub(crate) struct Registration {
    shared: Arc<Shared>,
    owner: Option<usize>,
    key: TimerKey,
}
impl Drop for Registration {
    fn drop(&mut self) {
        dispatch(&self.shared, self.owner, Command::Release(self.key));
    }
}

pub(crate) fn dispatch(shared: &Arc<Shared>, owner: Option<usize>, command: Command) {
    let Some(owner) = owner else {
        return root::dispatch(shared, command);
    };
    let mut command = Some(command);
    let result = worker::timers(shared, Some(owner), |_, timers| {
        // The owning worker cannot destroy its queue during this borrow. Late remote commands are rejected under admission instead.
        match command.take().unwrap() {
            Command::Update(key, waker) => match timers.update(key, waker) {
                Ok(previous) => (Some(previous), false),
                Err(replacement) => (Some(replacement), true),
            },
            Command::Release(key) => (timers.release(key), false),
            #[cfg(test)]
            Command::Inspect(_) => unreachable!("only updates and releases use direct dispatch"),
        }
    });
    if let Some((waker, wake)) = result {
        if wake {
            crate::panic::contain(|| waker.unwrap().wake());
        } else {
            crate::panic::discard(waker);
        }
    } else {
        shared.send_timer(owner, command.unwrap());
    }
}

pub(crate) fn registered(
    shared: &Arc<Shared>,
    owner: usize,
    result: Result<TimerKey, Waker>,
) -> io::Result<Registration> {
    result
        .map(|key| Registration {
            shared: Arc::clone(shared),
            owner: Some(owner),
            key,
        })
        .map_err(|waker| {
            crate::panic::discard(waker);
            io::Error::new(io::ErrorKind::WouldBlock, "worker timer queue is full")
        })
}

// Register with the current worker's queue, or with the shared root queue from any other thread.
pub(crate) fn register(
    shared: &Arc<Shared>,
    deadline: Instant,
    waker: Waker,
) -> io::Result<Registration> {
    let mut waker = Some(waker);
    let local = worker::timers(shared, None, |owner, timers| {
        (owner, timers.try_schedule(deadline, waker.take().unwrap()))
    });
    if let Some((owner, result)) = local {
        registered(shared, owner, result)
    } else {
        root::register(shared, deadline, waker.unwrap()).map(|key| Registration {
            shared: Arc::clone(shared),
            owner: None,
            key,
        })
    }
}

async fn wait(shared: &Arc<Shared>, deadline: Instant) -> io::Result<()> {
    let mut cached = poll_fn(|context| Poll::Ready(context.waker().clone())).await;
    let registration = register(shared, deadline, cached.clone())?;
    poll_fn(|context| {
        if !cached.will_wake(context.waker()) {
            let replacement = context.waker().clone();
            let previous = std::mem::replace(&mut cached, replacement.clone());
            crate::panic::discard(previous);
            dispatch(
                shared,
                registration.owner,
                Command::Update(registration.key, replacement),
            );
        }
        Poll::Pending
    })
    .await
}

pub(crate) fn turn(shared: &Arc<Shared>, owner: usize) {
    let incoming = &shared.workers[owner].timers;
    // A snapshot prevents concurrent producers from extending this turn indefinitely.
    for _ in 0..incoming.len() {
        // This worker is the only consumer; producers can only add to the snapshot.
        let command = incoming.pop().expect("snapshot commands have one consumer");
        if shared.is_closed() {
            rejected(command);
            continue;
        }
        match command {
            #[cfg(test)]
            Command::Inspect(reply) => {
                expire(shared, owner);
                let len = worker::timers(shared, Some(owner), |_, timers| timers.len()).unwrap();
                let _ = reply.send(len);
            }
            command => dispatch(shared, Some(owner), command),
        }
    }
    expire(shared, owner);
    root::expire(shared);
}

fn expire(shared: &Arc<Shared>, owner: usize) {
    // Empty queues avoid clock reads and heap flushes on the ordinary I/O path.
    if worker::timers(shared, Some(owner), |_, timers| timers.is_empty()).unwrap() {
        return;
    }
    let now = now(shared);
    loop {
        let wake = worker::timers(shared, Some(owner), |_, timers| {
            if shared.is_closed() {
                timers.pop()
            } else {
                timers.pop_expired(now)
            }
        })
        .unwrap();
        let Some(wake) = wake else { break };
        // Remove the timer first, release the queue borrow, then publish the runnable through its registered waker.
        crate::panic::contain(|| wake.wake());
    }
}

pub(crate) fn timeout(shared: &Arc<Shared>, owner: usize, watching: &mut bool) -> Option<Duration> {
    // Recheck after publishing sleeping=true: commands that arrived while the worker was active may have needed no interrupt.
    if !shared.workers[owner].timers.is_empty() {
        return Some(Duration::ZERO);
    }
    let local = worker::timers(shared, Some(owner), |_, timers| {
        if timers.is_empty() {
            None
        } else {
            timers
                .next_deadline()
                .map(|deadline| deadline.saturating_duration_since(now(shared)))
        }
    })
    .unwrap();
    // Only the watcher bounds its wait by root deadlines.
    local
        .into_iter()
        .chain(root::timeout(shared, owner, watching))
        .min()
}

impl<S: crate::Selector> Clock for crate::NativeScheduler<S> {
    fn now(&self) -> Instant {
        now(&self.tasks.shared)
    }

    async fn sleep_until(
        &self,
        deadline: Instant,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        let mut waiting = pin!(wait(&self.tasks.shared, deadline));
        let mut cancelled = pin!(cancellation.cancelled());
        poll_fn(|context| {
            // After waiting starts, observing expiry wins concurrent cancellation or closure.
            if self.now() >= deadline {
                return Poll::Ready(Ok(()));
            }
            if cancelled.as_mut().poll(context).is_ready() {
                return Poll::Ready(Err(base_scheduler::Cancelled.into()));
            }
            if self.tasks.shared.is_closed() {
                return Poll::Ready(Err(closed().into()));
            }
            waiting.as_mut().poll(context).map_err(Into::into)
        })
        .await
    }
}

#[cfg(test)]
mod tests;

mod native_timer;
pub use native_timer::NativeTimer;

mod root;
pub(crate) use root::{RootTimers, resign};
