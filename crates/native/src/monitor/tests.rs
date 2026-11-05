// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{task_service::Shared, worker_timers};
use crossbeam_deque::Worker;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    task::Wake,
};

#[derive(Default)]
struct Probe(AtomicUsize);
impl Wake for Probe {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// Workers that are never started, so only the test collects their timers.
fn shared(workers: usize, rescue: bool) -> Arc<Shared> {
    shared_with(workers, rescue)
}
fn shared_with(workers: usize, rescue: bool) -> Arc<Shared> {
    let queues: Vec<_> = (0..workers)
        .map(|_| std::array::from_fn(|_| Worker::new_fifo()))
        .collect();
    Arc::new(Shared::new(&queues, 8, rescue))
}

// Register an expired timer on `owner`, which external registration selects round robin.
fn expired_timer<'a>(
    shared: &'a Arc<Shared>,
    owner: usize,
    probe: &Arc<Probe>,
) -> worker_timers::Registration<'a> {
    shared.timer_next.store(owner, Ordering::Relaxed);
    let deadline = worker_timers::now(shared) - Duration::from_millis(1);
    worker_timers::register(shared, deadline, Waker::from(Arc::clone(probe))).unwrap()
}

// Clock values for a pass, relative to the scheduler's epoch.
fn at(shared: &Shared, offset: Duration) -> u64 {
    worker_timers::clock(shared) + offset.as_nanos() as u64
}

#[test]
fn passes_rescue_timers_left_due_for_the_stall_on_busy_workers() {
    let shared = shared(2, true);
    let probe = Arc::new(Probe::default());
    let mut state = State::new(&shared);
    let start = at(&shared, Duration::ZERO);

    // Without deadlines the monitor sleeps until woken.
    assert_eq!(state.pass(&shared, start), NO_DEADLINE);
    assert_eq!(shared.monitor.next.load(Ordering::Relaxed), NO_DEADLINE);

    // A future deadline is covered: the monitor wakes once it has been due for the stall.
    shared.timer_next.store(1, Ordering::Relaxed);
    let deadline = worker_timers::now(&shared) + Duration::from_secs(10);
    let future =
        worker_timers::register(&shared, deadline, Waker::from(Arc::clone(&probe))).unwrap();
    let covered = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    assert_eq!(state.pass(&shared, start), covered + STALL);
    assert_eq!(shared.monitor.next.load(Ordering::Relaxed), covered);
    drop(future);

    // A timer due for less than the stall waits; passes are at least `PERIOD` apart.
    let first = expired_timer(&shared, 1, &probe);
    let due = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    assert_eq!(state.pass(&shared, due), due + PERIOD);
    assert_eq!(probe.0.load(Ordering::SeqCst), 0);
    // Due for the stall without collection: marked, and classified at the next pass.
    assert_eq!(state.pass(&shared, due + STALL), due + STALL + PERIOD);
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Unclassified));
    assert_eq!(probe.0.load(Ordering::SeqCst), 1);

    // A stuck worker keeps being sampled. No poll completed since it was marked, so it is in a stuck poll.
    assert_eq!(
        state.pass(&shared, due + STALL * 2),
        due + STALL * 2 + PERIOD
    );
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Poll));
    // A future deadline on a stuck worker is checked when it falls due, not `STALL` later, and a timer due during the stall is rescued without a grace period.
    shared.timer_next.store(1, Ordering::Relaxed);
    let pending = worker_timers::register(
        &shared,
        worker_timers::now(&shared) + Duration::from_secs(10),
        Waker::noop().clone(),
    )
    .unwrap();
    let soon = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    assert_eq!(state.pass(&shared, soon - PERIOD * 5), soon - PERIOD * 4);
    assert_eq!(
        shared.workers[1].stuck(),
        Some(Stuck::Poll),
        "the mark persists while the worker stays inside its poll"
    );
    drop(pending);
    let second = expired_timer(&shared, 1, &probe);
    shared.workers[1]
        .heartbeat
        .polls
        .fetch_add(1, Ordering::Relaxed);
    state.pass(&shared, due + STALL * 4);
    // A mark keeps its class: polls completing later do not turn a stuck poll into a long batch.
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Poll));
    assert_eq!(probe.0.load(Ordering::SeqCst), 2);
    let counters = &shared.monitor.counters;
    assert_eq!(counters.stuck_polls.load(Ordering::Relaxed), 2);
    assert_eq!(counters.long_batches.load(Ordering::Relaxed), 0);
    assert_eq!(counters.timers.load(Ordering::Relaxed), 2);

    // A turn boundary clears the mark, and the monitor sleeps again.
    shared.workers[1].beat();
    assert_eq!(state.pass(&shared, due + STALL * 5), NO_DEADLINE);
    assert_eq!(shared.workers[1].stuck(), None);

    // A sleeping worker services its own due deadline, but the monitor looks again in case it leaves idle straight into a stall.
    let third = expired_timer(&shared, 0, &probe);
    shared.workers[0].set_sleeping(true);
    assert_eq!(
        state.pass(&shared, due + STALL * 6),
        due + STALL * 6 + PERIOD
    );
    assert_eq!(shared.workers[0].stuck(), None);
    // Sleep also ends a stall that was marked.
    shared.workers[0].set_sleeping(false);
    state.pass(&shared, due + STALL * 7);
    assert!(shared.workers[0].stuck().is_some());
    shared.workers[0].set_sleeping(true);
    assert_eq!(state.pass(&shared, due + STALL * 8), NO_DEADLINE);
    assert_eq!(shared.workers[0].stuck(), None);
    shared.workers[0].set_sleeping(false);
    drop((first, second, third));
}

#[test]
fn rescues_are_classified_by_what_the_worker_was_doing() {
    let shared = shared(2, true);
    let probe = Arc::new(Probe::default());
    let mut state = State::new(&shared);
    let counters = &shared.monitor.counters;
    let count = |counter: &std::sync::atomic::AtomicUsize| counter.load(Ordering::Relaxed);

    // A worker that completes a poll within a period of being marked is in a long batch, and its first rescue counts as one.
    let first = expired_timer(&shared, 1, &probe);
    let due = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    state.pass(&shared, due + STALL);
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Unclassified));
    shared.workers[1]
        .heartbeat
        .polls
        .fetch_add(1, Ordering::Relaxed);
    state.pass(&shared, due + STALL + PERIOD);
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Batch));
    assert_eq!(count(&counters.long_batches), 1);
    shared.workers[1].beat();
    state.pass(&shared, due + STALL + PERIOD * 2);
    assert_eq!(shared.workers[1].stuck(), None);

    // One that reaches a turn boundary before it is classified was making progress too.
    let second = expired_timer(&shared, 1, &probe);
    let due = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    state.pass(&shared, due + STALL);
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Unclassified));
    shared.workers[1].beat();
    state.pass(&shared, due + STALL + PERIOD);
    assert_eq!(shared.workers[1].stuck(), None);
    assert_eq!(count(&counters.long_batches), 2);

    // A worker still in idle code after a wake has not resumed: a woken worker, classified at once.
    shared.workers[0]
        .heartbeat
        .idle
        .store(true, Ordering::Relaxed);
    let third = expired_timer(&shared, 0, &probe);
    let due = shared.workers[0].timers.deadline.load(Ordering::Relaxed);
    state.pass(&shared, due + STALL);
    assert_eq!(shared.workers[0].stuck(), Some(Stuck::Woken));
    assert_eq!(count(&counters.woken), 1);
    shared.workers[0]
        .heartbeat
        .idle
        .store(false, Ordering::Relaxed);
    shared.workers[0].beat();
    state.pass(&shared, due + STALL + PERIOD);
    assert_eq!(shared.workers[0].stuck(), None);

    // An idle worker's rescue of an unclassified victim waits for the victim's class.
    let fourth = expired_timer(&shared, 1, &probe);
    let due = shared.workers[1].timers.deadline.load(Ordering::Relaxed);
    state.pass(&shared, due + STALL);
    let fifth = expired_timer(&shared, 1, &probe);
    assert!(assist(&shared, 0, &mut worker_timers::buffer()));
    assert_eq!(
        shared.workers[1]
            .unclassified_rescues
            .load(Ordering::Relaxed),
        2
    );
    assert_eq!(count(&counters.stuck_polls), 0);
    state.pass(&shared, due + STALL + PERIOD);
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Poll));
    assert_eq!(count(&counters.stuck_polls), 2);
    assert_eq!(count(&counters.timers), 5);
    shared.workers[1].beat();
    state.pass(&shared, due + STALL + PERIOD * 2);
    drop((first, second, third, fourth, fifth));
}

#[test]
fn an_earlier_deadline_wakes_the_monitor() {
    let shared = shared(1, true);
    shared.monitor.register(thread::current());
    let mut state = State::new(&shared);
    let now = at(&shared, Duration::ZERO);
    assert_eq!(state.pass(&shared, now), NO_DEADLINE);
    // This thread is the monitor, so a registration earlier than its plan leaves an unpark token and this returns at once.
    let registration = worker_timers::register(
        &shared,
        worker_timers::now(&shared) + Duration::from_secs(10),
        Waker::noop().clone(),
    )
    .unwrap();
    thread::park();
    drop(registration);
    assert_eq!(sleep_for(now, now + 5), Some(Duration::from_nanos(5)));
    assert_eq!(sleep_for(now, NO_DEADLINE), None);
    // With rescue disabled, no deadline is earlier than the plan.
    assert_eq!(
        shared_with(1, false).monitor.next.load(Ordering::Relaxed),
        0
    );
}

#[test]
fn a_deadline_lowered_during_a_pass_runs_it_again() {
    let shared = shared(1, true);
    let mut state = State::new(&shared);
    let now = at(&shared, Duration::ZERO);
    assert_eq!(state.pass(&shared, now), NO_DEADLINE);
    // The registration lowers the queue's deadline below the value this pass read.
    let registration = worker_timers::register(
        &shared,
        worker_timers::now(&shared) + Duration::from_secs(10),
        Waker::noop().clone(),
    )
    .unwrap();
    let lowered = shared.workers[0].timers.deadline.load(Ordering::Relaxed);
    assert_eq!(state.confirm(now, NO_DEADLINE, &shared), lowered);
    state.pass(&shared, now);
    assert_eq!(state.confirm(now, NO_DEADLINE, &shared), NO_DEADLINE);
    drop(registration);
}

#[test]
fn a_deadline_raised_during_a_pass_runs_it_again() {
    let shared = shared(1, true);
    let probe = Arc::new(Probe::default());
    let mut state = State::new(&shared);
    let now = at(&shared, Duration::ZERO);
    // The pass reads an overdue deadline on a sleeping worker, which then leaves idle, collects it and publishes a later one before the plan is stored.
    let due = expired_timer(&shared, 0, &probe);
    let later = worker_timers::register(
        &shared,
        worker_timers::now(&shared) + Duration::from_secs(10),
        Waker::noop().clone(),
    )
    .unwrap();
    shared.workers[0].set_sleeping(true);
    state.pass(&shared, now);
    shared.workers[0].set_sleeping(false);
    worker_timers::expire(&shared, 0, &mut worker_timers::buffer(), false);
    let raised = shared.workers[0].timers.deadline.load(Ordering::Relaxed);
    assert!(raised > now);
    assert_eq!(state.confirm(now, NO_DEADLINE, &shared), raised);
    // A queue whose lock is busy is read again a period later.
    let held = shared.workers[0].timers.queue.lock();
    assert_eq!(state.confirm(now, NO_DEADLINE, &shared), now + PERIOD);
    drop(held);
    drop((due, later));
}

#[test]
fn idle_workers_assist_only_victims_marked_stuck() {
    let probe = Arc::new(Probe::default());
    let disabled = shared(2, false);
    let held = expired_timer(&disabled, 1, &probe);
    disabled.workers[1].set_stuck(Some(Stuck::Poll));
    let mut expired = worker_timers::buffer();
    assert!(!assist(&disabled, 0, &mut expired));
    drop(held);

    let shared = shared(2, true);
    let held = expired_timer(&shared, 1, &probe);
    assert!(!assist(&shared, 0, &mut expired));
    shared.workers[1].set_stuck(Some(Stuck::Poll));
    // A worker never assists itself.
    assert!(!assist(&shared, 1, &mut expired));
    assert!(assist(&shared, 0, &mut expired));
    assert_eq!(probe.0.load(Ordering::SeqCst), 1);
    assert!(!assist(&shared, 0, &mut expired));
    drop(held);
}
