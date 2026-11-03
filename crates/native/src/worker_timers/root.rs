// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::{Command, closed, now};
use crate::task_service::Shared;
use base_scheduler_timers::local::TimerKey;
use base_scheduler_timers::local::TimerQueue;
use std::{
    io,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    task::Waker,
    time::{Duration, Instant},
};

const EMPTY: u64 = u64::MAX;
const NO_WATCHER: usize = usize::MAX;

// Registrations made outside this scheduler's workers. Any worker can collect them, so a long poll on one worker cannot hold them back while another worker runs.
pub(crate) struct RootTimers {
    queue: Mutex<TimerQueue<Waker>>,
    epoch: Instant,
    // Earliest deadline as nanoseconds after `epoch`, or `EMPTY`. Republished under the queue lock after every change that can move it.
    next: AtomicU64,
    // The idle worker bounding its wait by `next`. It resigns before running tasks, handing the deadline to another idle worker.
    watcher: AtomicUsize,
}

impl RootTimers {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(TimerQueue::with_capacity(capacity)),
            epoch: Instant::now(),
            next: AtomicU64::new(EMPTY),
            watcher: AtomicUsize::new(NO_WATCHER),
        }
    }

    fn encode(&self, instant: Instant) -> u64 {
        // Saturate to an earlier value: a premature check only rereads the queue.
        instant
            .checked_duration_since(self.epoch)
            .map_or(0, |offset| {
                u64::try_from(offset.as_nanos()).map_or(EMPTY - 1, |nanos| nanos.min(EMPTY - 1))
            })
    }

    #[cfg(test)]
    pub(crate) fn watcher(&self) -> Option<usize> {
        Some(self.watcher.load(Ordering::SeqCst)).filter(|&watcher| watcher != NO_WATCHER)
    }

    fn publish(&self, queue: &mut TimerQueue<Waker>) -> u64 {
        let next = queue
            .next_deadline()
            .map_or(EMPTY, |deadline| self.encode(deadline));
        self.next.store(next, Ordering::SeqCst);
        next
    }
}

pub(crate) fn register(shared: &Shared, deadline: Instant, waker: Waker) -> io::Result<TimerKey> {
    let root = &shared.root;
    let mut queue = root.queue.lock().unwrap();
    // Workers observing closure always drain this queue under its lock, so a registration admitted here is drained or rejected.
    let result = if shared.is_closed() {
        Err((waker, closed()))
    } else {
        let previous = root.next.load(Ordering::Relaxed);
        queue
            .try_schedule(deadline, waker)
            .map(|key| (key, root.publish(&mut queue) < previous))
            .map_err(|waker| {
                (
                    waker,
                    io::Error::new(io::ErrorKind::WouldBlock, "root timer queue is full"),
                )
            })
    };
    drop(queue);
    match result {
        Ok((key, earlier)) => {
            if earlier {
                notify(shared);
            }
            Ok(key)
        }
        Err((waker, error)) => {
            crate::panic::discard(waker);
            Err(error)
        }
    }
}

// An earlier deadline must reach a worker whose wait it bounds.
fn notify(shared: &Shared) {
    let watcher = shared.root.watcher.load(Ordering::SeqCst);
    if watcher == NO_WATCHER || !shared.workers[watcher].wake() {
        shared.wake_idle(usize::MAX);
    }
}

pub(crate) fn dispatch(shared: &Shared, command: Command) {
    let result = {
        let mut queue = shared.root.queue.lock().unwrap();
        match command {
            Command::Update(key, waker) => match queue.update(key, waker) {
                Ok(previous) => (Some(previous), false),
                Err(replacement) => (Some(replacement), true),
            },
            Command::Release(key) => {
                let waker = queue.release(key);
                // Without this, a completed root timeout would keep workers claiming and handing off the watch until its old deadline.
                shared.root.publish(&mut queue);
                (waker, false)
            }
            #[cfg(test)]
            Command::Inspect(_) => unreachable!("root timers are inspected directly"),
        }
    };
    // Wakers run and drop outside the queue lock; they can register again.
    match result {
        (Some(waker), true) => crate::panic::contain(|| waker.wake()),
        (waker, _) => crate::panic::discard(waker),
    }
}

// Collect expired root timers. Any worker can do this between batches; closure collects every remaining timer.
pub(crate) fn expire(shared: &Shared) {
    let root = &shared.root;
    // After closure, always drain under the lock: a registration which observed the open scheduler under that lock may not have published its deadline yet.
    let closed = shared.is_closed();
    let now = if closed {
        None
    } else {
        let next = root.next.load(Ordering::Acquire);
        if next == EMPTY {
            return;
        }
        let now = now(shared);
        if next > root.encode(now) {
            return;
        }
        Some(now)
    };
    loop {
        let wake = {
            let mut queue = root.queue.lock().unwrap();
            let wake = match now {
                None => queue.pop(),
                Some(now) => queue.pop_expired(now),
            };
            root.publish(&mut queue);
            wake
        };
        let Some(wake) = wake else { break };
        crate::panic::contain(|| wake.wake());
    }
}

// Bound an idle worker's wait by the earliest root deadline if it is, or becomes, the watcher. A worker claims the role only while root timers are pending, and keeps it until its next task batch. With no watcher, the first registration wakes an idle worker instead.
pub(crate) fn timeout(shared: &Shared, owner: usize, watching: &mut bool) -> Option<Duration> {
    let root = &shared.root;
    if !*watching {
        *watching = root.next.load(Ordering::SeqCst) != EMPTY
            && root.watcher.load(Ordering::SeqCst) == NO_WATCHER
            && root
                .watcher
                .compare_exchange(NO_WATCHER, owner, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok();
        if !*watching {
            return None;
        }
    }
    // Load after claiming: a registration either observes this watcher or publishes its deadline first.
    let next = root.next.load(Ordering::SeqCst);
    (next != EMPTY)
        .then(|| (root.epoch + Duration::from_nanos(next)).saturating_duration_since(now(shared)))
}

// Before a worker runs tasks, which can poll for arbitrarily long, it resigns the watch. A pending root deadline then passes to another idle worker. A worker woken for that hand-off can itself find tasks, so every worker without a watcher repeats it.
pub(crate) fn resign(shared: &Shared, watching: &mut bool) {
    let root = &shared.root;
    if *watching {
        *watching = false;
        root.watcher.store(NO_WATCHER, Ordering::SeqCst);
    } else if root.watcher.load(Ordering::SeqCst) != NO_WATCHER {
        return;
    }
    if root.next.load(Ordering::SeqCst) != EMPTY {
        shared.wake_idle(usize::MAX);
    }
}

// Stand in for a registration admitted under the lock whose deadline is not yet visible.
#[cfg(test)]
pub(crate) fn schedule_unpublished(shared: &Shared, deadline: Instant, waker: Waker) {
    let mut queue = shared.root.queue.lock().unwrap();
    assert!(queue.try_schedule(deadline, waker).is_ok());
}

#[cfg(test)]
pub(crate) fn len(shared: &Shared) -> usize {
    expire(shared);
    shared.root.queue.lock().unwrap().len()
}
