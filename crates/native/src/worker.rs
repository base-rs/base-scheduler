// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::task_service::Shared;
use crate::task_service::{Runnable, UNASSIGNED_WORKER};
use crossbeam_deque::{Injector, Steal, Stealer, Worker};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::{self, Thread};
use std::{task::Waker, time::Duration};

// Check external work regularly even when the local queue stays busy.
const EXTERNAL_INTERVAL: usize = 61;

pub(crate) struct WorkerState {
    pub(crate) incoming: Injector<Runnable>,
    stealer: Stealer<Runnable>,
    sleeping: AtomicBool,
    thread: OnceLock<Thread>,
    driver_waker: OnceLock<Waker>,
}

impl WorkerState {
    pub(crate) fn new(stealer: Stealer<Runnable>) -> Self {
        Self {
            incoming: Injector::new(),
            stealer,
            sleeping: AtomicBool::new(false),
            thread: OnceLock::new(),
            driver_waker: OnceLock::new(),
        }
    }

    pub(crate) fn wake(&self) -> bool {
        if self.sleeping.swap(false, Ordering::SeqCst) {
            self.thread
                .get()
                .expect("sleeping worker has registered its thread")
                .unpark();
            if let Some(waker) = self.driver_waker.get() {
                waker.wake_by_ref();
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

pub(crate) fn schedule(shared: &Arc<Shared>, runnable: Runnable) {
    let preferred = runnable.metadata().worker.load(Ordering::Relaxed);
    let remote = CURRENT_WORKER.with(|current| {
        let current = current.borrow();
        if let Some(worker) = current.as_ref() {
            if std::ptr::eq(worker.scheduler.as_ptr(), Arc::as_ptr(shared))
                && (preferred == UNASSIGNED_WORKER || preferred == worker.identifier)
            {
                worker.ready.push(runnable);
                return None;
            }
        }
        Some(runnable)
    });
    if let Some(runnable) = remote {
        if let Some(worker) = shared.workers.get(preferred) {
            worker.incoming.push(runnable);
        } else {
            shared.ready.push(runnable);
        }
    }
    // Publication precedes the sleeping flag exchange. A worker publishes its sleeping flag before rechecking queues, so a racing wake is retained.
    shared.wake_idle(preferred);
}

fn search(shared: &Shared, local: &LocalWorker, iteration: usize) -> Steal<Runnable> {
    let incoming = &shared.workers[local.identifier].incoming;
    let mut retry = false;
    if iteration % EXTERNAL_INTERVAL == 0 {
        for queue in [&shared.ready, incoming] {
            if let Some(runnable) = take_stolen(queue.steal_batch_and_pop(&local.ready), &mut retry)
            {
                return Steal::Success(runnable);
            }
        }
    }
    if let Some(runnable) = local.ready.pop() {
        return Steal::Success(runnable);
    }
    for queue in [incoming, &shared.ready] {
        if let Some(runnable) = take_stolen(queue.steal_batch_and_pop(&local.ready), &mut retry) {
            return Steal::Success(runnable);
        }
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
        if let Some(runnable) =
            take_stolen(worker.stealer.steal_batch_and_pop(&local.ready), &mut retry)
        {
            return Steal::Success(runnable);
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
    driver: &mut Option<&mut dyn core_scheduler_selector::SelectorWorker>,
    timeout: Option<Duration>,
) {
    if let Some(active) = driver.as_mut() {
        if !matches!(active.turn(timeout), Ok(false)) {
            *driver = None;
        }
    } else if timeout.is_none() {
        thread::park();
    }
}

pub(crate) fn run(shared: Arc<Shared>, identifier: usize, ready: Worker<Runnable>) {
    run_loop(shared, identifier, ready, None);
}

pub(crate) fn run_driven<D: core_scheduler_selector::SelectorDriver>(
    shared: Arc<Shared>,
    identifier: usize,
    ready: Worker<Runnable>,
    driver: D,
) {
    use core_scheduler_selector::SelectorWorker;
    let mut driver = driver.enter();
    shared.workers[identifier]
        .driver_waker
        .set(driver.waker())
        .expect("worker driver registered once");
    run_loop(shared, identifier, ready, Some(&mut driver));
}

fn run_loop(
    shared: Arc<Shared>,
    identifier: usize,
    ready: Worker<Runnable>,
    mut driver: Option<&mut dyn core_scheduler_selector::SelectorWorker>,
) {
    let worker = &shared.workers[identifier];
    worker
        .thread
        .set(thread::current())
        .expect("worker thread registered once");
    CURRENT_WORKER.with(|current| {
        current.replace(Some(LocalWorker {
            identifier,
            scheduler: Arc::downgrade(&shared),
            ready,
        }))
    });
    let mut iteration = 0usize;
    loop {
        iteration = iteration.wrapping_add(1);
        if shared.finished() {
            if let Some(active) = driver.as_mut() {
                active.close();
                drive(&mut driver, None);
                continue;
            }
            break;
        }
        // Publish the whole completion batch before polling any task on this worker. Bound task polling so a permanently runnable task cannot starve ring progress.
        if iteration % EXTERNAL_INTERVAL == 1 {
            drive(&mut driver, Some(Duration::ZERO));
        }
        let runnable = match find_work(&shared, iteration) {
            Steal::Empty => {
                drive(&mut driver, Some(Duration::ZERO));
                worker.sleeping.store(true, Ordering::SeqCst);
                shared.idle_workers.fetch_add(1, Ordering::SeqCst);
                // Pair with the enqueuer's fence: either this search observes its queued work, or the enqueuer observes an idle worker.
                std::sync::atomic::fence(Ordering::SeqCst);
                // Recheck after publishing the sleeping flag. An unpark token remains available if work arrives immediately before park().
                finish_idle_search(
                    worker,
                    &shared.idle_workers,
                    find_work(&shared, iteration),
                    || !shared.finished(),
                    || drive(&mut driver, None),
                )
            }
            result => yield_on_retry(result).success(),
        };
        let Some(runnable) = runnable else {
            continue;
        };
        runnable
            .metadata()
            .worker
            .store(identifier, Ordering::Relaxed);
        // Fan out work taken in a batch when other workers are parked.
        CURRENT_WORKER.with(|current| {
            if !current
                .borrow()
                .as_ref()
                .expect("worker context is installed")
                .ready
                .is_empty()
            {
                shared.wake_idle(UNASSIGNED_WORKER);
            }
        });
        // Future panics become TaskError in TaskFuture. Also contain a panic propagated by task machinery so that it does not remove a worker.
        crate::panic::contain(|| {
            runnable.run();
        });
        shared.retire_runnable();
    }
    CURRENT_WORKER.with(|current| current.take());
}

#[cfg(test)]
mod tests;
