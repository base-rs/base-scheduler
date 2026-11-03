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
    let queues: Vec<_> = (0..workers)
        .map(|_| std::array::from_fn(|_| Worker::new_fifo()))
        .collect();
    Arc::new(Shared::new(&queues, 8, rescue))
}

// Register an expired timer on `owner`, which external registration selects round robin.
fn expired_timer(
    shared: &Arc<Shared>,
    owner: usize,
    probe: &Arc<Probe>,
) -> worker_timers::Registration {
    shared.timer_next.store(owner, Ordering::Relaxed);
    let deadline = worker_timers::now(shared) - Duration::from_millis(1);
    worker_timers::register(shared, deadline, Waker::from(Arc::clone(probe))).unwrap()
}

#[test]
fn sampling_marks_classifies_rescues_and_clears_stalled_workers() {
    let shared = shared(2, true);
    let probe = Arc::new(Probe::default());
    let start = Instant::now();
    let mut state = State::new(&shared);

    // The first busy observation starts the stall interval.
    let first = expired_timer(&shared, 1, &probe);
    assert!(!state.sample(&shared, start));
    assert_eq!(shared.workers[1].stuck(), None);
    assert_eq!(probe.0.load(Ordering::SeqCst), 0);

    // No poll completed between samples: a stuck poll. Worker 0 is stuck too, but has nothing to rescue.
    assert!(!state.sample(&shared, start + STALL));
    assert_eq!(shared.workers[0].stuck(), Some(Stuck::Poll));
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Poll));
    assert_eq!(probe.0.load(Ordering::SeqCst), 1);

    // Polls completed without a turn boundary: a long batch.
    let second = expired_timer(&shared, 1, &probe);
    shared.workers[1]
        .heartbeat
        .polls
        .fetch_add(1, Ordering::Relaxed);
    assert!(!state.sample(&shared, start + STALL * 2));
    assert_eq!(shared.workers[1].stuck(), Some(Stuck::Batch));
    assert_eq!(probe.0.load(Ordering::SeqCst), 2);
    let counters = &shared.monitor.counters;
    assert_eq!(counters.stuck_polls.load(Ordering::Relaxed), 1);
    assert_eq!(counters.long_batches.load(Ordering::Relaxed), 1);
    assert_eq!(counters.timers.load(Ordering::Relaxed), 2);

    // A turn boundary clears the mark and restarts the interval.
    shared.workers[1].beat();
    assert!(!state.sample(&shared, start + STALL * 3));
    assert_eq!(shared.workers[1].stuck(), None);
    drop((first, second));
}

#[test]
fn every_worker_idle_for_several_samples_enters_deep_sleep_until_one_resumes() {
    let shared = shared(2, true);
    let mut state = State::new(&shared);
    let now = Instant::now();
    for worker in &shared.workers {
        worker.set_sleeping(true);
    }
    for _ in 1..DEEP_AFTER {
        assert!(!state.sample(&shared, now));
    }
    assert!(state.sample(&shared, now));
    assert!(shared.monitor.deep.load(Ordering::SeqCst));
    // A worker leaving idle clears deep sleep and wakes the monitor once.
    shared.workers[0].set_sleeping(false);
    shared.monitor.resume();
    assert!(!shared.monitor.deep.load(Ordering::SeqCst));
    shared.monitor.resume();
    // The unpark token is retained, so this returns at once.
    thread::park();
    // A worker that woke after the sample keeps the monitor awake.
    assert!(!shared.monitor.sleep(&shared));
    assert!(!shared.monitor.deep.load(Ordering::SeqCst));
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
