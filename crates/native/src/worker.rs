// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::task_service::Runnable;
use crate::task_service::Shared;
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::{self, Thread};
use std::{task::Waker, time::Duration};

/// How a stuck worker was observed. Release builds record every mark as `Poll`; test and diagnostic builds classify it: a worker in idle code that is no longer sleeping is `LeavingIdle`, and any other starts `Unclassified` until the monitor's next pass tells a `Batch` that completed a task poll from a `Poll` that did not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stuck {
    Poll,
    #[cfg_attr(not(any(test, feature = "diagnostics")), allow(dead_code))]
    Batch,
    #[cfg_attr(not(any(test, feature = "diagnostics")), allow(dead_code))]
    Unclassified,
    #[cfg_attr(not(any(test, feature = "diagnostics")), allow(dead_code))]
    LeavingIdle,
}

/// Written by the owner at each turn boundary and read by the stall monitor. It has its own cache line, so the owner's stores do not disturb workers reading its queues and flags.
#[repr(align(128))]
#[derive(Default)]
pub(crate) struct Heartbeat {
    pub(crate) turns: AtomicUsize,
    // Completed task polls, which tell a long batch from a single stuck poll.
    #[cfg(any(test, feature = "diagnostics"))]
    pub(crate) polls: AtomicUsize,
    // Set while the owner is in its idle path, from before it advertises sleeping until it resumes, so that a marked worker leaving idle is recognised.
    #[cfg(any(test, feature = "diagnostics"))]
    pub(crate) idle: AtomicBool,
}

/// A worker's timer queue and its published deadline. The owner mutates the queue on every timer operation, so it has its own cache lines, apart from the flags and queues that other workers read and write.
#[repr(align(128))]
pub(crate) struct Timers {
    // Locked by the owner for its own timer operations and passes, and directly by other threads for registration, update and release. Never held across a waker call or a task poll.
    pub(crate) queue: crate::spin_lock::SpinLock<base_scheduler_timers::local::TimerQueue<Waker>>,
    // A lower bound of the earliest live deadline in `queue`, in clock nanoseconds since `epoch`. Written only under the queue lock.
    pub(crate) deadline: AtomicU64,
    // The scheduler's construction time, copied here so that timer operations read only this worker's lines.
    pub(crate) epoch: std::time::Instant,
    // Set under the queue lock by closure, before the scheduler publishes it, so registrations need not read shared scheduler state.
    pub(crate) closed: AtomicBool,
}

// Keep each worker's flags off its neighbours' cache lines.
#[repr(align(128))]
pub(crate) struct WorkerState {
    pub(crate) incoming: Injector<Runnable>,
    pub(crate) timers: Timers,
    pub(crate) heartbeat: Heartbeat,
    // Set by the stall monitor while this worker's heartbeat is stalled: 0, or 1 + `Stuck`.
    stuck: AtomicU8,
    // Rescues of this worker while its mark is unclassified, counted under the class the monitor later assigns.
    #[cfg(any(test, feature = "diagnostics"))]
    pub(crate) unclassified_rescues: AtomicUsize,
    stealers: [Stealer<Runnable>; 2],
    sleeping: AtomicBool,
    thread: OnceLock<Thread>,
    driver_waker: OnceLock<Waker>,
}

impl WorkerState {
    pub(crate) fn new(
        stealers: [Stealer<Runnable>; 2],
        timer_capacity: usize,
        epoch: std::time::Instant,
    ) -> Self {
        let queue = crate::spin_lock::SpinLock::new(
            base_scheduler_timers::local::TimerQueue::with_capacity(timer_capacity),
        );
        Self {
            incoming: Injector::new(),
            timers: Timers {
                queue,
                deadline: AtomicU64::new(crate::worker_timers::NO_DEADLINE),
                epoch,
                closed: AtomicBool::new(false),
            },
            heartbeat: Heartbeat::default(),
            stuck: AtomicU8::new(0),
            #[cfg(any(test, feature = "diagnostics"))]
            unclassified_rescues: AtomicUsize::new(0),
            stealers,
            sleeping: AtomicBool::new(false),
            thread: OnceLock::new(),
            driver_waker: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn set_sleeping(&self, sleeping: bool) {
        self.sleeping.store(sleeping, Ordering::SeqCst);
    }

    pub(crate) fn is_sleeping(&self) -> bool {
        self.sleeping.load(Ordering::SeqCst)
    }

    pub(crate) fn stuck(&self) -> Option<Stuck> {
        match self.stuck.load(Ordering::Relaxed) {
            0 => None,
            1 => Some(Stuck::Poll),
            2 => Some(Stuck::Batch),
            3 => Some(Stuck::Unclassified),
            _ => Some(Stuck::LeavingIdle),
        }
    }

    // Store only on change, so sampling a healthy worker does not dirty its cache line.
    pub(crate) fn set_stuck(&self, stuck: Option<Stuck>) {
        let value = match stuck {
            None => 0,
            Some(Stuck::Poll) => 1,
            Some(Stuck::Batch) => 2,
            Some(Stuck::Unclassified) => 3,
            Some(Stuck::LeavingIdle) => 4,
        };
        if self.stuck.load(Ordering::Relaxed) != value {
            self.stuck.store(value, Ordering::Relaxed);
        }
    }

    // Record a turn boundary. Only the owner writes its heartbeat.
    pub(crate) fn beat(&self) {
        let turns = &self.heartbeat.turns;
        turns.store(
            turns.load(Ordering::Relaxed).wrapping_add(1),
            Ordering::Relaxed,
        );
    }

    pub(crate) fn wake(&self) -> bool {
        if self.sleeping.swap(false, Ordering::SeqCst) {
            let target = self
                .thread
                .get()
                .expect("sleeping worker has registered its thread");
            // A completion published by this worker makes its driver turn return. Interrupt only another thread; a self-notification would leave a stale permit for the next idle turn.
            if target.id() != thread::current().id() {
                target.unpark();
                if let Some(waker) = self.driver_waker.get() {
                    waker.wake_by_ref();
                }
            }
            true
        } else {
            false
        }
    }
}

struct LocalWorker {
    identifier: usize,
    scheduler: Weak<Shared>,
    ready: Worker<Runnable>,
    current: Worker<Runnable>,
}

thread_local! {
    static CURRENT_WORKER: RefCell<Option<LocalWorker>> = const { RefCell::new(None) };
}

pub(crate) fn is_worker() -> bool {
    CURRENT_WORKER.with(|current| current.borrow().is_some())
}

pub(crate) fn assert_outside_worker() {
    assert!(
        !is_worker(),
        "cannot synchronously wait on a native scheduler worker; use .await"
    );
}

/// The current thread's worker identifier, if it is a worker of `shared`.
pub(crate) fn current(shared: &Arc<Shared>) -> Option<usize> {
    CURRENT_WORKER.with(|current| {
        let current = current.borrow();
        let local = current.as_ref()?;
        std::ptr::eq(local.scheduler.as_ptr(), Arc::as_ptr(shared)).then_some(local.identifier)
    })
}

pub(crate) fn schedule(shared: &Arc<Shared>, runnable: Runnable) {
    let mut preferred = runnable.metadata().worker.load(Ordering::Relaxed);
    let remote = CURRENT_WORKER.with(|current| {
        let current = current.borrow();
        if let Some(worker) = current.as_ref() {
            // A stuck preferred worker cannot run the task, so a worker of the same scheduler keeps it instead.
            if std::ptr::eq(worker.scheduler.as_ptr(), Arc::as_ptr(shared))
                && (preferred == worker.identifier || shared.workers[preferred].stuck().is_some())
            {
                preferred = worker.identifier;
                worker.ready.push(runnable);
                return None;
            }
        }
        Some(runnable)
    });
    if let Some(runnable) = remote {
        // Otherwise skip a stuck preferred worker's inbox, which is searched only after every other worker runs out of local work.
        if shared.workers[preferred].stuck().is_some() {
            preferred = unstuck(shared, preferred);
        }
        shared.workers[preferred].incoming.push(runnable);
    } else if shared.workers.len() == 1 {
        // The sole worker is already executing this callback and will observe its local queue before waiting again.
        return;
    }
    // Publication precedes the sleeping flag exchange. A worker publishes its sleeping flag before rechecking queues, so a racing wake is retained.
    shared.wake_idle(preferred);
}

// The next worker after `preferred` that is not stuck, or `preferred` if every worker is.
fn unstuck(shared: &Shared, preferred: usize) -> usize {
    let count = shared.workers.len();
    (1..count)
        .map(|offset| (preferred + offset) % count)
        .find(|&candidate| shared.workers[candidate].stuck().is_none())
        .unwrap_or(preferred)
}

fn search(shared: &Shared, local: &LocalWorker, iteration: usize) -> Steal<Runnable> {
    let incoming = &shared.workers[local.identifier].incoming;
    let mut retry = false;
    // Admit external wakes at each batch boundary, even while local tasks keep waking themselves.
    if !incoming.is_empty() {
        let _ = take_stolen(incoming.steal_batch(&local.ready), &mut retry);
    }
    if let Some(runnable) = local.ready.pop() {
        return Steal::Success(runnable);
    }
    // Every queued or currently polled runnable holds an existing outstanding token. A racing publication still follows the idle fence, recheck, and wake protocol.
    if shared.workers.len() == 1 || !shared.has_runnables() {
        return search_result(retry);
    }
    // Only steal after exhausting this worker's own work. Rotate the first victim so idle workers do not always contend on worker zero.
    for offset in 0..shared.workers.len() {
        let victim = local
            .identifier
            .wrapping_add(iteration)
            .wrapping_add(offset)
            % shared.workers.len();
        if victim == local.identifier {
            continue;
        }
        let worker = &shared.workers[victim];
        // Both generations remain stealable while their owner swaps their roles.
        for stealer in &worker.stealers {
            if let Some(runnable) =
                take_stolen(stealer.steal_batch_and_pop(&local.ready), &mut retry)
            {
                return Steal::Success(runnable);
            }
        }
        // Remote wakes remain stealable while the previous worker is busy.
        if let Some(runnable) = take_stolen(
            worker.incoming.steal_batch_and_pop(&local.ready),
            &mut retry,
        ) {
            return Steal::Success(runnable);
        }
    }
    search_result(retry)
}

fn search_result(retry: bool) -> Steal<Runnable> {
    if retry { Steal::Retry } else { Steal::Empty }
}

fn take_stolen<Value>(result: Steal<Value>, retry: &mut bool) -> Option<Value> {
    match result {
        Steal::Success(value) => Some(value),
        Steal::Retry => {
            *retry = true;
            None
        }
        Steal::Empty => None,
    }
}

// A pre-park rescue may have scheduled a task into a queue the search already passed, or cleared this worker's sleeping flag without an unpark. Rescan instead of parking.
fn retry_after_rescue(rescued: bool, result: Steal<Runnable>) -> Steal<Runnable> {
    if rescued && matches!(result, Steal::Empty) {
        Steal::Retry
    } else {
        result
    }
}

fn yield_on_retry(result: Steal<Runnable>) -> Steal<Runnable> {
    if matches!(result, Steal::Retry) {
        thread::yield_now();
    }
    result
}

fn finish_idle_search(
    worker: &WorkerState,
    idle_workers: &std::sync::atomic::AtomicUsize,
    result: Steal<Runnable>,
    should_park: impl FnOnce() -> bool,
    park: impl FnOnce(),
) -> Option<Runnable> {
    let runnable = match result {
        Steal::Success(runnable) => Some(runnable),
        Steal::Retry => None,
        Steal::Empty => {
            if should_park() {
                park();
            }
            None
        }
    };
    worker.sleeping.store(false, Ordering::SeqCst);
    idle_workers.fetch_sub(1, Ordering::SeqCst);
    runnable
}

fn find_work(shared: &Shared, iteration: usize) -> Steal<Runnable> {
    CURRENT_WORKER.with(|current| {
        search(
            shared,
            current
                .borrow()
                .as_ref()
                .expect("worker context is installed"),
            iteration,
        )
    })
}

fn drive(
    driver: &mut Option<&mut dyn base_scheduler_selector::SelectorWorker>,
    timeout: Option<Duration>,
) {
    if let Some(active) = driver.as_mut() {
        if !matches!(active.turn(timeout), Ok(false)) {
            *driver = None;
        }
    } else {
        match timeout {
            Some(timeout) if !timeout.is_zero() => thread::park_timeout(timeout),
            Some(_) => {}
            None => thread::park(),
        }
    }
}

pub(crate) fn run(shared: Arc<Shared>, identifier: usize, queues: [Worker<Runnable>; 2]) {
    run_loop(shared, identifier, queues, None);
}

pub(crate) fn run_driven<D: base_scheduler_selector::SelectorDriver>(
    shared: Arc<Shared>,
    identifier: usize,
    queues: [Worker<Runnable>; 2],
    driver: D,
) {
    use base_scheduler_selector::SelectorWorker;
    let mut driver = driver.enter();
    shared.workers[identifier]
        .driver_waker
        .set(driver.waker())
        .expect("worker driver registered once");
    run_loop(shared, identifier, queues, Some(&mut driver));
}

fn run_loop(
    shared: Arc<Shared>,
    identifier: usize,
    [ready, current]: [Worker<Runnable>; 2],
    mut driver: Option<&mut dyn base_scheduler_selector::SelectorWorker>,
) {
    let worker = &shared.workers[identifier];
    worker
        .thread
        .set(thread::current())
        .expect("worker thread registered once");
    CURRENT_WORKER.with(|context| {
        context.replace(Some(LocalWorker {
            identifier,
            scheduler: Arc::downgrade(&shared),
            ready,
            current,
        }))
    });
    let mut expired = crate::worker_timers::buffer();
    let mut iteration = 0usize;
    let mut needs_turn = false;
    loop {
        iteration = iteration.wrapping_add(1);
        worker.beat();
        crate::worker_timers::expire(&shared, identifier, &mut expired, false);
        if shared.finished() {
            // Closure may have followed the open timer pass above. Notify local waiters before retiring this worker, even when no task futures remain.
            crate::worker_timers::expire(&shared, identifier, &mut expired, false);
            if let Some(active) = driver.as_mut() {
                active.close();
                drive(&mut driver, None);
                continue;
            }
            break;
        }
        let runnable = match find_work(&shared, iteration) {
            Steal::Empty => {
                #[cfg(any(test, feature = "diagnostics"))]
                worker.heartbeat.idle.store(true, Ordering::Relaxed);
                worker.sleeping.store(true, Ordering::SeqCst);
                shared.idle_workers.fetch_add(1, Ordering::SeqCst);
                // Pair with the enqueuer's fence: either this search observes its queued work, or the enqueuer observes an idle worker.
                std::sync::atomic::fence(Ordering::SeqCst);
                let rescued = crate::monitor::assist(&shared, identifier, &mut expired);
                // Recheck after publishing the sleeping flag. An unpark token remains available if work arrives immediately before park().
                let runnable = finish_idle_search(
                    worker,
                    &shared.idle_workers,
                    retry_after_rescue(rescued, find_work(&shared, iteration)),
                    || !shared.finished(),
                    || {
                        needs_turn = false;
                        drive(
                            &mut driver,
                            crate::worker_timers::timeout(&shared, identifier),
                        )
                    },
                );
                // Leaving idle is a turn boundary: the batch that follows starts a new heartbeat.
                worker.beat();
                #[cfg(any(test, feature = "diagnostics"))]
                worker.heartbeat.idle.store(false, Ordering::Relaxed);
                runnable
            }
            result => yield_on_retry(result).success(),
        };
        let Some(runnable) = runnable else {
            continue;
        };
        // Publish completions between ready batches. If the queue emptied, the idle path above already collected them with its actual wait timeout.
        if needs_turn {
            drive(&mut driver, Some(Duration::ZERO));
        }
        run_ready_batch(&shared, runnable);
        needs_turn = true;
    }
    CURRENT_WORKER.with(|current| current.take());
}

#[inline(always)]
fn run_ready_batch(shared: &Shared, first: Runnable) {
    // Freeze the batch. Wakes during task polling go into the now-empty next queue.
    let (identifier, has_more) = CURRENT_WORKER.with(|current| {
        let mut current = current.borrow_mut();
        let local = current.as_mut().expect("worker context is installed");
        std::mem::swap(&mut local.ready, &mut local.current);
        (local.identifier, !local.current.is_empty())
    });
    if has_more {
        shared.wake_idle(usize::MAX);
    }
    let mut runnable = Some(first);
    while let Some(task) = runnable {
        task.metadata().worker.store(identifier, Ordering::Relaxed);
        // Contain task-machinery panics as well as the guarded future's panics.
        crate::panic::contain(|| {
            task.run();
        });
        #[cfg(any(test, feature = "diagnostics"))]
        shared.workers[identifier]
            .heartbeat
            .polls
            .fetch_add(1, Ordering::Relaxed);
        shared.retire_runnable();
        runnable = CURRENT_WORKER.with(|current| {
            current
                .borrow()
                .as_ref()
                .expect("worker context is installed")
                .current
                .pop()
        });
    }
}

#[cfg(test)]
mod tests;
