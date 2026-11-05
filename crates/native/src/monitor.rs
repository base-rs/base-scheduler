// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// Stall detection and rescue, described in `design/stall-rescue.md`. A stall strands a timer only if the timer becomes due while its worker stays inside one poll or batch, so the monitor sleeps until the earliest published deadline plus `STALL`. A deadline still published that long after it fell due, on a worker that is not sleeping, means the worker is stuck: the monitor and idle workers then fire its expired timers from their own threads. Rescue moves wakeups, not timer ownership: keys stay in their original queue. With no pending deadlines the monitor does not wake at all.

use crate::{
    task_service::Shared,
    worker::Stuck,
    worker_timers::{self, NO_DEADLINE},
};
use std::{
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    task::Waker,
    thread::{self, Thread},
    time::Duration,
};

/// How long a timer may stay due and uncollected on a worker that is not sleeping before that worker is stuck. Rescue follows the deadline by about this much.
const STALL: u64 = Duration::from_millis(1).as_nanos() as u64;
/// The monitor's shortest interval between passes, which bounds its wake rate while deadlines keep falling due on busy workers. It also samples a stuck worker this often, and a deadline less than this much earlier than its plan does not wake it, so rescue can be this much later.
const PERIOD: u64 = Duration::from_millis(1).as_nanos() as u64;

// Keep the monitor's planned deadline, which every lowering registration reads, off cache lines that workers write.
#[repr(align(128))]
pub(crate) struct Monitor {
    thread: OnceLock<Thread>,
    // The earliest future deadline that the monitor's current sleep covers, in clock nanoseconds. Publishing an earlier deadline wakes it. Zero when rescue is disabled, so nothing does.
    next: AtomicU64,
    #[cfg(any(test, feature = "diagnostics"))]
    pub(crate) counters: Counters,
}

impl Monitor {
    pub(crate) fn new(rescue: bool) -> Self {
        Self {
            thread: OnceLock::new(),
            next: AtomicU64::new(if rescue { NO_DEADLINE } else { 0 }),
            #[cfg(any(test, feature = "diagnostics"))]
            counters: Counters::default(),
        }
    }

    /// Whether publishing `deadline` must wake the monitor. Called under the worker's queue lock; the caller unparks after unlocking. The monitor stores its plan before rechecking each queue under its lock, so either it observes this deadline or this load observes its plan.
    pub(crate) fn needs_wake(&self, deadline: u64) -> bool {
        deadline.saturating_add(PERIOD) < self.next.load(Ordering::Relaxed)
    }

    /// Record the monitor thread's handle, through which it is unparked.
    pub(crate) fn register(&self, thread: Thread) {
        let _ = self.thread.set(thread);
    }

    #[cfg(test)]
    pub(crate) fn registered(&self) -> bool {
        self.thread.get().is_some()
    }

    /// Interrupt the monitor's current sleep.
    pub(crate) fn unpark(&self) {
        if let Some(thread) = self.thread.get() {
            thread.unpark();
        }
    }
}

/// Rescue counts, classified by the kind of stall that the monitor observed.
#[cfg(any(test, feature = "diagnostics"))]
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) stuck_polls: std::sync::atomic::AtomicUsize,
    pub(crate) long_batches: std::sync::atomic::AtomicUsize,
    pub(crate) leaving_idle: std::sync::atomic::AtomicUsize,
    pub(crate) timers: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "diagnostics"))]
impl Counters {
    // `rescue` holds unclassified rescues and `classify` is never given `Unclassified`, so that arm is unreachable; it shares the stuck-poll counter only to keep the match exhaustive.
    fn class(&self, kind: Stuck) -> &std::sync::atomic::AtomicUsize {
        match kind {
            Stuck::Poll | Stuck::Unclassified => &self.stuck_polls,
            Stuck::Batch => &self.long_batches,
            Stuck::LeavingIdle => &self.leaving_idle,
        }
    }
}

// Give a marked worker its class, and count the rescues made while it was unclassified under that class.
#[cfg(any(test, feature = "diagnostics"))]
fn classify(shared: &Shared, victim: usize, kind: Stuck) {
    let worker = &shared.workers[victim];
    worker.set_stuck(Some(kind));
    let rescues = worker.unclassified_rescues.swap(0, Ordering::Relaxed);
    shared
        .monitor
        .counters
        .class(kind)
        .fetch_add(rescues, Ordering::Relaxed);
}

/// Fire a stuck worker's expired timers from another thread, and return the number of wakers invoked.
fn rescue(shared: &Arc<Shared>, victim: usize, _kind: Stuck, expired: &mut Vec<Waker>) -> usize {
    let fired = worker_timers::expire(shared, victim, expired, true);
    #[cfg(any(test, feature = "diagnostics"))]
    if fired > 0 {
        let counters = &shared.monitor.counters;
        if _kind == Stuck::Unclassified {
            &shared.workers[victim].unclassified_rescues
        } else {
            counters.class(_kind)
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

// A worker marked stuck, with its heartbeat when it was marked, so that its next turn boundary clears the mark.
#[derive(Clone, Copy)]
struct Marked {
    turns: usize,
}

#[derive(Default)]
struct Observation {
    // The published deadline as this pass read it, for the locked recheck.
    deadline: u64,
    marked: Option<Marked>,
    // When the worker was marked, and its completed polls then: a period later, a long batch has completed more, and a stuck poll has not.
    #[cfg(any(test, feature = "diagnostics"))]
    marked_at: u64,
    #[cfg(any(test, feature = "diagnostics"))]
    polls: usize,
}

pub(crate) struct State {
    observations: Vec<Observation>,
    expired: Vec<Waker>,
}

impl State {
    pub(crate) fn new(shared: &Shared) -> Self {
        Self {
            observations: shared
                .workers
                .iter()
                .map(|_| Observation::default())
                .collect(),
            expired: worker_timers::buffer(),
        }
    }

    /// Mark and rescue stuck workers, and return when the monitor should next run, or `NO_DEADLINE` to sleep until woken.
    pub(crate) fn pass(&mut self, shared: &Arc<Shared>, now: u64) -> u64 {
        let (wake, covered) = self.watch(shared, now);
        shared.monitor.next.store(covered, Ordering::Relaxed);
        let wake = self.confirm(now, wake, shared);
        if wake == NO_DEADLINE {
            wake
        } else {
            wake.max(now + PERIOD)
        }
    }

    // Returns when to run next, and the earliest deadline still to be checked.
    //
    // A worker collects its due timers at every turn boundary and refreshes its published deadline, and an idle worker wakes for its own deadline. So a deadline that has been due for `STALL` on a worker that is not sleeping means that worker has not reached a turn boundary since: it is stuck. This needs one monitor wake per such deadline, and none while no deadline is pending.
    fn watch(&mut self, shared: &Arc<Shared>, now: u64) -> (u64, u64) {
        let mut wake = NO_DEADLINE;
        let mut covered = NO_DEADLINE;
        for (identifier, worker) in shared.workers.iter().enumerate() {
            let observation = &mut self.observations[identifier];
            let deadline = worker.timers.deadline.load(Ordering::Relaxed);
            observation.deadline = deadline;
            let turns = worker.heartbeat.turns.load(Ordering::Relaxed);
            #[cfg(any(test, feature = "diagnostics"))]
            let polls = worker.heartbeat.polls.load(Ordering::Relaxed);
            let sleeping = worker.is_sleeping();
            // A turn boundary, or sleep, ends a stall.
            if observation
                .marked
                .is_some_and(|marked| sleeping || marked.turns != turns)
            {
                // A worker that reached a turn boundary before it was classified was making progress.
                #[cfg(any(test, feature = "diagnostics"))]
                classify(
                    shared,
                    identifier,
                    match worker.stuck() {
                        Some(Stuck::Unclassified) | None => Stuck::Batch,
                        Some(kind) => kind,
                    },
                );
                observation.marked = None;
                worker.set_stuck(None);
            }
            // At least a period after marking, completed polls tell a long batch from a stuck poll. An earlier pass, after an unpark, leaves the worker unclassified, so that stuck polls stay a lower bound.
            #[cfg(any(test, feature = "diagnostics"))]
            if observation.marked.is_some()
                && worker.stuck() == Some(Stuck::Unclassified)
                && now >= observation.marked_at + PERIOD
            {
                let kind = if polls == observation.polls {
                    Stuck::Poll
                } else {
                    Stuck::Batch
                };
                classify(shared, identifier, kind);
            }
            // A worker already known to be stuck needs no grace period.
            let grace = if observation.marked.is_some() {
                0
            } else {
                STALL
            };
            let check = deadline.saturating_add(grace);
            if check > now || sleeping {
                if check > now {
                    covered = covered.min(deadline);
                    wake = wake.min(check);
                } else {
                    // A sleeping worker with an overdue deadline is about to collect it, but it may leave idle straight into a batch that stalls, and its unchanged deadline would never wake the monitor again. Look again until it is collected.
                    wake = wake.min(now + PERIOD);
                }
                if observation.marked.is_some() {
                    // Keep sampling a stuck worker until it recovers. This costs nothing while no worker is stuck.
                    wake = wake.min(now + PERIOD);
                }
                continue;
            }
            // A newly marked worker in idle code is leaving idle: woken and not yet running, or finishing its pre-park checks. Any other is classified at the next pass.
            #[cfg(any(test, feature = "diagnostics"))]
            let kind = match worker.stuck() {
                Some(kind) => kind,
                None if worker.heartbeat.idle.load(Ordering::Relaxed) => Stuck::LeavingIdle,
                None => {
                    observation.marked_at = now;
                    observation.polls = polls;
                    Stuck::Unclassified
                }
            };
            #[cfg(not(any(test, feature = "diagnostics")))]
            let kind = Stuck::Poll;
            observation.marked.get_or_insert(Marked { turns });
            worker.set_stuck(Some(kind));
            rescue(shared, identifier, kind, &mut self.expired);
            wake = wake.min(now + PERIOD);
        }
        (wake, covered)
    }

    // A deadline changed after the pass read it, but before the plan was stored, may not have woken the monitor: a lower one would have been compared with the old plan, and a raised one replaces a deadline the plan no longer covers. Rereading each queue closes that window: the monitor runs again by the new deadline. A queue whose lock is busy is read again a period later rather than waited for.
    fn confirm(&self, now: u64, wake: u64, shared: &Shared) -> u64 {
        let mut wake = wake;
        for (identifier, worker) in shared.workers.iter().enumerate() {
            match worker_timers::try_published(worker) {
                Some(published) if published == self.observations[identifier].deadline => {}
                Some(published) => wake = wake.min(published),
                None => wake = wake.min(now + PERIOD),
            }
        }
        wake
    }
}

// The monitor's sleep before `wake`, or `None` to sleep until woken.
fn sleep_for(now: u64, wake: u64) -> Option<Duration> {
    (wake != NO_DEADLINE).then(|| Duration::from_nanos(wake.saturating_sub(now)))
}

// Holds the monitor after its first check until a test has run past it, at two rendezvous on the barrier. Set on the thread that creates a scheduler, which carries it to that scheduler's monitor.
#[cfg(test)]
thread_local! {
    pub(crate) static START_GATE: std::cell::Cell<Option<Arc<std::sync::Barrier>>> = const { std::cell::Cell::new(None) };
}

/// The monitor thread. It holds only a weak reference between passes, and exits once closure has finished or the scheduler is gone.
pub(crate) fn run(shared: Weak<Shared>) {
    let mut state = None;
    loop {
        let Some(shared) = shared.upgrade().filter(|shared| !shared.finished()) else {
            return;
        };
        #[cfg(test)]
        if let Some(gate) = START_GATE.take() {
            gate.wait();
            gate.wait();
        }
        let now = worker_timers::clock(&shared);
        let wake = state
            .get_or_insert_with(|| State::new(&shared))
            .pass(&shared, now);
        drop(shared);
        match sleep_for(now, wake) {
            Some(duration) => thread::park_timeout(duration),
            None => thread::park(),
        }
    }
}

#[cfg(test)]
mod scenarios;
#[cfg(test)]
mod tests;
