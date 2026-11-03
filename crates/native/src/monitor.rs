// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// Stall detection and rescue, described in `design/stall-rescue.md`. A worker that has not reached a turn boundary for `STALL`, while it is neither idle nor closing, is marked stuck. The monitor and idle workers then fire its expired timers from their own threads. Rescue moves wakeups, not timer ownership: keys stay in their original queue.

use crate::{
    task_service::Shared,
    worker::{Stuck, WorkerState},
    worker_timers,
};
use std::{
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
    task::Waker,
    thread::{self, Thread},
    time::{Duration, Instant},
};

/// How long a busy worker's heartbeat must stay unchanged before it is stuck.
const STALL: Duration = Duration::from_millis(1);
/// How often the monitor samples workers while any of them is busy. The rescue bound is about `STALL + PERIOD`.
const PERIOD: Duration = Duration::from_millis(1);
/// Consecutive samples with every worker idle before the monitor sleeps until a worker leaves idle.
const DEEP_AFTER: usize = 4;

#[derive(Default)]
pub(crate) struct Monitor {
    thread: OnceLock<Thread>,
    deep: AtomicBool,
    #[cfg(any(test, feature = "diagnostics"))]
    pub(crate) counters: Counters,
}

impl Monitor {
    /// Called by a worker after it clears its sleeping flag. Both sides store with `SeqCst` before loading the other's flag, so either the monitor observes this worker awake or this load observes the monitor's deep sleep.
    pub(crate) fn resume(&self) {
        if self.deep.load(Ordering::SeqCst) && self.deep.swap(false, Ordering::SeqCst) {
            self.unpark();
        }
    }

    /// Interrupt the monitor's current sleep, for closure and deep-sleep exit.
    pub(crate) fn unpark(&self) {
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }

    // Advertise deep sleep, then confirm that every worker is still idle.
    fn sleep(&self, shared: &Shared) -> bool {
        self.deep.store(true, Ordering::SeqCst);
        if shared.workers.iter().all(WorkerState::is_sleeping) {
            return true;
        }
        self.deep.store(false, Ordering::SeqCst);
        false
    }
}

/// Rescue counts, classified by the kind of stall that the monitor observed.
#[cfg(any(test, feature = "diagnostics"))]
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) stuck_polls: std::sync::atomic::AtomicUsize,
    pub(crate) long_batches: std::sync::atomic::AtomicUsize,
    pub(crate) timers: std::sync::atomic::AtomicUsize,
}

/// Fire a stuck worker's expired timers from another thread, and return the number of wakers invoked.
fn rescue(shared: &Arc<Shared>, victim: usize, _kind: Stuck, expired: &mut Vec<Waker>) -> usize {
    let fired = worker_timers::expire(shared, victim, expired, true);
    #[cfg(any(test, feature = "diagnostics"))]
    if fired > 0 {
        let counters = &shared.monitor.counters;
        match _kind {
            Stuck::Poll => &counters.stuck_polls,
            Stuck::Batch => &counters.long_batches,
        }
        .fetch_add(1, Ordering::Relaxed);
        counters.timers.fetch_add(fired, Ordering::Relaxed);
    }
    fired
}

/// The pre-park check: after advertising sleep, an idle worker rescues each victim that the monitor has marked stuck. It costs one load per victim when none is stuck. Returns whether any waker was invoked.
pub(crate) fn assist(shared: &Arc<Shared>, identifier: usize, expired: &mut Vec<Waker>) -> bool {
    if !shared.rescue {
        return false;
    }
    let mut fired = 0;
    for (victim, worker) in shared.workers.iter().enumerate() {
        if victim == identifier {
            continue;
        }
        if let Some(kind) = worker.stuck() {
            fired += rescue(shared, victim, kind, expired);
        }
    }
    fired > 0
}

struct Observation {
    turns: usize,
    // The first sample that saw this heartbeat while the worker was busy.
    since: Option<Instant>,
    #[cfg(any(test, feature = "diagnostics"))]
    polls: usize,
}

pub(crate) struct State {
    observations: Vec<Observation>,
    idle: usize,
    expired: Vec<Waker>,
}

impl State {
    pub(crate) fn new(shared: &Shared) -> Self {
        let _ = shared.monitor.thread.set(thread::current());
        Self {
            observations: shared
                .workers
                .iter()
                .map(|worker| Observation {
                    turns: worker.heartbeat.turns.load(Ordering::Relaxed),
                    since: None,
                    #[cfg(any(test, feature = "diagnostics"))]
                    polls: worker.heartbeat.polls.load(Ordering::Relaxed),
                })
                .collect(),
            idle: 0,
            expired: worker_timers::buffer(),
        }
    }

    /// Observe every worker, mark and rescue stuck ones, and return whether the monitor should sleep until a worker leaves idle.
    pub(crate) fn sample(&mut self, shared: &Arc<Shared>, now: Instant) -> bool {
        let mut idle = true;
        for (identifier, worker) in shared.workers.iter().enumerate() {
            let observation = &mut self.observations[identifier];
            let turns = worker.heartbeat.turns.load(Ordering::Relaxed);
            #[cfg(any(test, feature = "diagnostics"))]
            let kind = {
                let polls = worker.heartbeat.polls.load(Ordering::Relaxed);
                let progressed = polls != observation.polls;
                observation.polls = polls;
                if progressed {
                    Stuck::Batch
                } else {
                    Stuck::Poll
                }
            };
            #[cfg(not(any(test, feature = "diagnostics")))]
            let kind = Stuck::Poll;
            // An idle worker services its own deadline. A closing worker is idle once it has finished.
            let sleeping = worker.is_sleeping();
            if sleeping || turns != observation.turns {
                observation.turns = turns;
                observation.since = (!sleeping).then_some(now);
                worker.set_stuck(None);
                idle &= sleeping;
                continue;
            }
            idle = false;
            let since = *observation.since.get_or_insert(now);
            if now.duration_since(since) < STALL {
                continue;
            }
            worker.set_stuck(Some(kind));
            rescue(shared, identifier, kind, &mut self.expired);
        }
        self.idle = if idle { self.idle + 1 } else { 0 };
        self.idle >= DEEP_AFTER && shared.monitor.sleep(shared)
    }
}

/// The monitor thread. It holds only a weak reference between passes, and exits once closure has finished or the scheduler is gone.
pub(crate) fn run(shared: Weak<Shared>) {
    let mut state = None;
    loop {
        let Some(shared) = shared.upgrade().filter(|shared| !shared.finished()) else {
            return;
        };
        let deep = state
            .get_or_insert_with(|| State::new(&shared))
            .sample(&shared, Instant::now());
        drop(shared);
        if deep {
            thread::park();
        } else {
            thread::park_timeout(PERIOD);
        }
    }
}

#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod tests;
