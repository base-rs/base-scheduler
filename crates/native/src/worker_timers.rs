// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    spin_lock::SpinGuard,
    task_service::Shared,
    worker::{self, WorkerState},
};
use base_scheduler::{Cancellation, Clock, OperationResult};
use base_scheduler_timers::local::{TimerKey, TimerQueue};
use std::{
    future::{Future, poll_fn},
    io,
    pin::pin,
    sync::{Arc, atomic::Ordering},
    task::{Poll, Waker},
    time::{Duration, Instant},
};

/// Published when a worker's queue holds no live deadline.
pub(crate) const NO_DEADLINE: u64 = u64::MAX;

/// Wakers collected under one queue lock by an expiry pass.
const BATCH: usize = 256;

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

/// A reusable output buffer for [`expire`]. It must keep spare capacity.
pub(crate) fn buffer() -> Vec<Waker> {
    Vec::with_capacity(BATCH)
}

// Clock nanoseconds since the scheduler's epoch. Earlier instants are already due; distant ones clamp below `NO_DEADLINE`.
fn nanos(worker: &WorkerState, instant: Instant) -> u64 {
    instant
        .saturating_duration_since(worker.timers.epoch)
        .as_nanos()
        .min(u128::from(NO_DEADLINE - 1)) as u64
}

// Nothing panics while a queue is locked: wakers are invoked and dropped only after unlocking.
fn lock(worker: &WorkerState) -> SpinGuard<'_, TimerQueue<Waker>> {
    worker.timers.queue.lock()
}

// The published deadline is a lower bound of the queue's earliest live deadline. Registration lowers it under the queue lock, and every pass that flushes the queue refreshes it exactly. Release leaves it early, which can only cause a redundant check. Every change is offered to the stall monitor, which wakes only for a deadline earlier than its plan.
fn lower(shared: &Shared, worker: &WorkerState, deadline: Instant) -> bool {
    let deadline = nanos(worker, deadline);
    let lowered = deadline < worker.timers.deadline.load(Ordering::Relaxed);
    if lowered {
        worker.timers.deadline.store(deadline, Ordering::Relaxed);
        shared.monitor.observe(deadline);
    }
    lowered
}

fn refresh(
    shared: &Shared,
    worker: &WorkerState,
    queue: &mut TimerQueue<Waker>,
) -> Option<Instant> {
    let next = queue.next_deadline();
    let published = next.map_or(NO_DEADLINE, |deadline| nanos(worker, deadline));
    if published != worker.timers.deadline.load(Ordering::Relaxed) {
        worker.timers.deadline.store(published, Ordering::Relaxed);
        shared.monitor.observe(published);
    }
    next
}

/// The current clock in the nanoseconds used for published deadlines.
pub(crate) fn clock(shared: &Shared) -> u64 {
    nanos(&shared.workers[0], now(shared))
}

/// A worker's published deadline, read under its queue lock.
pub(crate) fn published(worker: &WorkerState) -> u64 {
    let _queue = lock(worker);
    worker.timers.deadline.load(Ordering::Relaxed)
}

// The current time if the published deadline has passed. Reading it needs no lock: a stale value delays collection only until the next pass, and an idle owner computes its wait under the lock.
fn due(shared: &Shared, worker: &WorkerState) -> Option<Instant> {
    let deadline = worker.timers.deadline.load(Ordering::Relaxed);
    if deadline == NO_DEADLINE {
        return None;
    }
    let now = now(shared);
    (deadline <= nanos(worker, now)).then_some(now)
}

/// A key in one worker's queue. Keys never migrate; updates and release lock that queue from whichever thread holds the registration.
pub(crate) struct Registration {
    shared: Arc<Shared>,
    owner: usize,
    key: TimerKey,
}
impl Registration {
    pub(crate) fn update(&self, waker: Waker) {
        let result = lock(&self.shared.workers[self.owner]).update(self.key, waker);
        match result {
            Ok(previous) => crate::panic::discard(previous),
            // Expiry or closure can overtake an update. Its new context must still be notified.
            Err(replacement) => crate::panic::contain(|| replacement.wake()),
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let waker = lock(&self.shared.workers[self.owner]).release(self.key);
        crate::panic::discard(waker);
    }
}

/// Register on the current worker's queue, or on a round-robin worker's queue from any other thread.
pub(crate) fn register(
    shared: &Arc<Shared>,
    deadline: Instant,
    waker: Waker,
) -> io::Result<Registration> {
    let current = worker::current(shared);
    let owner = current.unwrap_or_else(|| {
        shared.timer_next.fetch_add(1, Ordering::Relaxed) % shared.workers.len()
    });
    let worker = &shared.workers[owner];
    let mut queue = lock(worker);
    // A worker drains its queue under this lock after observing closure, so either that drain removes this registration or this check rejects it.
    if worker.timers.closed.load(Ordering::Relaxed) {
        drop(queue);
        crate::panic::discard(waker);
        return Err(closed());
    }
    let result = queue.try_schedule(deadline, waker);
    let lowered = result.is_ok() && lower(shared, worker, deadline);
    drop(queue);
    let key = result.map_err(|waker| {
        crate::panic::discard(waker);
        io::Error::new(io::ErrorKind::WouldBlock, "worker timer queue is full")
    })?;
    // An idle owner advertises sleep before computing its wait under the queue lock, so either it observed this deadline or this wake observes it sleeping.
    if lowered && current != Some(owner) {
        worker.wake();
    }
    Ok(Registration {
        shared: Arc::clone(shared),
        owner,
        key,
    })
}

async fn wait(shared: &Arc<Shared>, deadline: Instant) -> io::Result<()> {
    let mut cached = poll_fn(|context| Poll::Ready(context.waker().clone())).await;
    let registration = register(shared, deadline, cached.clone())?;
    poll_fn(|context| {
        if !cached.will_wake(context.waker()) {
            let replacement = context.waker().clone();
            let previous = std::mem::replace(&mut cached, replacement.clone());
            crate::panic::discard(previous);
            registration.update(replacement);
        }
        Poll::Pending
    })
    .await
}

/// Invoke the wakers of `owner`'s expired timers, or of every timer once the scheduler is closed, and return how many were invoked.
///
/// The owner waits for its queue lock. A rescuer (`rescue`) skips a queue that is locked, because its holder is already making progress. Wakers run after the lock is released.
pub(crate) fn expire(
    shared: &Arc<Shared>,
    owner: usize,
    expired: &mut Vec<Waker>,
    rescue: bool,
) -> usize {
    let worker = &shared.workers[owner];
    let mut fired = 0;
    loop {
        // Once closure is observed, drain without trusting the published deadline.
        let now = if worker.timers.closed.load(Ordering::Relaxed) {
            None
        } else {
            let Some(now) = due(shared, worker) else {
                return fired;
            };
            Some(now)
        };
        let mut queue = if rescue {
            let Some(queue) = worker.timers.queue.try_lock() else {
                return fired;
            };
            queue
        } else {
            lock(worker)
        };
        if let Some(now) = now {
            queue.pop_expired_batch(now, expired);
        } else {
            for _ in expired.len()..expired.capacity() {
                let Some(waker) = queue.pop() else { break };
                expired.push(waker);
            }
        }
        refresh(shared, worker, &mut queue);
        drop(queue);
        let full = expired.len() == expired.capacity();
        fired += expired.len();
        for waker in expired.drain(..) {
            crate::panic::contain(|| waker.wake());
        }
        if !full {
            return fired;
        }
    }
}

/// Reject later registrations on `owner`'s queue. Closure calls this for every queue before it publishes scheduler closure.
pub(crate) fn close(shared: &Shared, owner: usize) {
    let worker = &shared.workers[owner];
    let queue = lock(worker);
    worker.timers.closed.store(true, Ordering::Relaxed);
    // Closure makes every remaining timer due, so the stall monitor watches a busy owner and drains its queue if it is stuck.
    if !queue.is_empty() {
        worker.timers.deadline.store(0, Ordering::Relaxed);
        shared.monitor.observe(0);
    }
}

/// The owner's wait bound, computed under its queue lock after advertising sleep.
pub(crate) fn timeout(shared: &Arc<Shared>, owner: usize) -> Option<Duration> {
    let worker = &shared.workers[owner];
    let next = refresh(shared, worker, &mut lock(worker));
    next.map(|deadline| deadline.saturating_duration_since(now(shared)))
}

/// Live registrations in `owner`'s queue.
#[cfg(test)]
pub(crate) fn len(shared: &Shared, owner: usize) -> usize {
    lock(&shared.workers[owner]).len()
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
