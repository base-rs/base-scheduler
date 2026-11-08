// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// End-to-end rescue scenarios on running schedulers. Placement uses the crate's worker identity, so these live beside the monitor rather than in the public integration tests.
//
// These tests assert causality, not lateness: a blocking poll holds its worker until the test releases it, after the work it stranded has progressed, so that progress can only have come from rescue. Rescue that never happens fails at the test watchdog, which names the last step the test reached. Wall-clock lateness depends on how the host schedules threads, so the bound *T* is checked by the benchmark lateness measurements instead (`design/stall-rescue.md`).

use crate::{
    NativeOptions, NativeScheduler, NativeTask,
    test_support::{self, step},
    worker,
};
use base_scheduler::{Cancellation, Clock, Spawn, Timers};
use futures_lite::future::{block_on, yield_now};
use std::{
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

const DELAY: Duration = Duration::from_millis(50);
// Longer than any test runs, for a timer that must not fire by itself.
const NEVER: Duration = Duration::from_secs(3600);

type Sleep = Pin<Box<dyn Future<Output = base_scheduler::OperationResult<()>> + Send>>;

fn scheduler(workers: usize) -> Arc<NativeScheduler> {
    Arc::new(
        NativeScheduler::with_options(NativeOptions {
            workers,
            ..Default::default()
        })
        .unwrap(),
    )
}

fn sleep(scheduler: &Arc<NativeScheduler>, delay: Duration) -> Sleep {
    let scheduler = Arc::clone(scheduler);
    Box::pin(async move { scheduler.sleep(delay, &Cancellation::never()).await })
}

fn reusable(scheduler: &Arc<NativeScheduler>, delay: Duration) -> Sleep {
    let scheduler = Arc::clone(scheduler);
    Box::pin(async move {
        let mut timer = scheduler.timer();
        let deadline = scheduler.now() + delay;
        scheduler
            .timer_wait_until(&mut timer, deadline, &Cancellation::never())
            .await
    })
}

// Hold the current worker inside one poll until released, standing in for undeclared blocking work. A failed test never releases it, and the worker stays held until the process exits.
fn block(released: &AtomicBool) {
    while !released.load(Ordering::Acquire) {
        std::thread::sleep(Duration::from_millis(1));
    }
}

// Timers fired by rescue so far.
fn rescued(scheduler: &NativeScheduler) -> usize {
    scheduler
        .tasks
        .shared
        .monitor
        .counters
        .timers
        .load(Ordering::Relaxed)
}

// Spawn copies of a task until one starts on `target`; the others return without running their body.
fn spawn_on<Body, F>(
    scheduler: &Arc<NativeScheduler>,
    target: usize,
    body: Body,
) -> NativeTask<Option<F::Output>>
where
    Body: Fn() -> F,
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    // Idle workers take spawned tasks; the test watchdog bounds this search.
    loop {
        let shared = Arc::clone(&scheduler.tasks.shared);
        let (started, start) = mpsc::channel();
        let future = body();
        let task = scheduler
            .spawn(async move {
                let matched = worker::current(&shared) == Some(target);
                started.send(matched).unwrap();
                if matched { Some(future.await) } else { None }
            })
            .unwrap();
        step(format!(
            "waiting for a task to start, looking for worker {target}"
        ));
        if start.recv().unwrap() {
            return task;
        }
        step(format!("joining a task that missed worker {target}"));
        assert!(block_on(task).unwrap().is_none());
    }
}

// A task that reports its waker and the worker it ran on, waits for one wake, then records when and where it resumed.
struct Waiter {
    task: NativeTask<Option<(Instant, Option<usize>)>>,
    waker: Waker,
}
fn wait_on(scheduler: &Arc<NativeScheduler>, target: usize) -> Waiter {
    let (send, receive) = mpsc::channel();
    let shared = Arc::clone(&scheduler.tasks.shared);
    let task = spawn_on(scheduler, target, move || {
        let send = send.clone();
        let shared = Arc::clone(&shared);
        async move {
            let mut reported = false;
            poll_fn(|context| {
                if reported {
                    return Poll::Ready(());
                }
                reported = true;
                send.send(context.waker().clone()).unwrap();
                Poll::Pending
            })
            .await;
            (Instant::now(), worker::current(&shared))
        }
    });
    step(format!(
        "waiting for a waiter on worker {target} to report its waker"
    ));
    let waker = receive.recv().unwrap();
    Waiter { task, waker }
}

// On `target`, register `timer` with `waker`, then block that worker until released. Returns the registration time.
fn stall_with_timer(
    scheduler: &Arc<NativeScheduler>,
    target: usize,
    timer: impl Fn() -> Sleep,
    waker: Waker,
    released: &Arc<AtomicBool>,
) -> (Instant, NativeTask<Option<()>>) {
    let (send, receive) = mpsc::channel();
    let task = spawn_on(scheduler, target, || {
        let mut timer = timer();
        let waker = waker.clone();
        let send = send.clone();
        let released = Arc::clone(released);
        async move {
            // The sleep fixes its deadline when first polled, so take the start time before that.
            let registered = Instant::now();
            assert!(
                timer
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            send.send(registered).unwrap();
            block(&released);
            drop(timer);
        }
    });
    step(format!("waiting for worker {target} to register its timer"));
    (receive.recv().unwrap(), task)
}

// A rescued timer still waits for its deadline.
fn assert_not_early(label: &str, elapsed: Duration) {
    assert!(
        elapsed >= DELAY,
        "{label} completed early after {elapsed:?}"
    );
}

// Rescue counts a timer after invoking its waker, so the count can trail the wake it explains.
fn wait_until(label: &str, condition: impl Fn() -> bool) {
    step(format!("waiting until {label}"));
    while !condition() {
        std::thread::sleep(Duration::from_millis(1));
    }
}

// Await `wait` and report whether this poller's first poll was pending. A wait first polled here after its deadline completes at once, without registering this poller's waker, so it shows nothing about rescue.
async fn pending_at_first_poll(mut wait: Sleep) -> bool {
    let mut first = None;
    poll_fn(|context| {
        let poll = wait.as_mut().poll(context);
        first.get_or_insert(poll.is_pending());
        poll
    })
    .await
    .unwrap();
    first.unwrap()
}

#[test]
fn a_timer_left_on_a_worker_that_blocks_fires_for_its_new_waiter() {
    test_support::run(|| {
        for workers in [2, 4] {
            let scheduler = scheduler(workers);
            for timer in [sleep, reusable] {
                // A waiter that never registered shows nothing, so repeat until one has; the watchdog bounds the repetition.
                let _ = (1..).find(|&attempt| fires_for_its_new_waiter(&scheduler, timer, attempt));
            }
            step(format!("shutting down {workers} workers"));
            block_on(scheduler.shutdown());
        }
    });
}

// One attempt, which reports whether the new waiter registered its waker before the deadline.
fn fires_for_its_new_waiter(
    scheduler: &Arc<NativeScheduler>,
    timer: fn(&Arc<NativeScheduler>, Duration) -> Sleep,
    attempt: usize,
) -> bool {
    let before = rescued(scheduler);
    let released = Arc::new(AtomicBool::new(false));
    let (send, receive) = futures_channel::oneshot::channel::<(Instant, Sleep)>();
    let receive = Arc::new(Mutex::new(Some(receive)));
    let source = Arc::clone(scheduler);
    // Register on worker 0 with a no-op waker, hand the pending wait to another task, then block worker 0.
    let send = Arc::new(Mutex::new(Some(send)));
    let blocker = spawn_on(scheduler, 0, || {
        let send = Arc::clone(&send);
        let released = Arc::clone(&released);
        let mut wait = timer(&source, DELAY);
        async move {
            // The wait fixes its deadline when first polled, so take the start time before that.
            let registered = Instant::now();
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            // Only the copy that starts on worker 0 runs this body.
            let send = send.lock().unwrap().take().unwrap();
            assert!(send.send((registered, wait)).is_ok());
            block(&released);
        }
    });
    // The new waiter runs on another worker and updates the registration's waker in worker 0's queue.
    let waiter = scheduler
        .spawn(async move {
            let receive = receive.lock().unwrap().take().unwrap();
            let (registered, wait) = receive.await.unwrap();
            let pending = pending_at_first_poll(wait).await;
            (registered.elapsed(), pending)
        })
        .unwrap();
    step(format!(
        "waiting for the new waiter while worker 0 is held ({} workers, attempt {attempt})",
        scheduler.tasks.shared.workers.len()
    ));
    let (elapsed, pending) = block_on(waiter).unwrap();
    assert_not_early("waiter", elapsed);
    // The waiter registered before the deadline and woke while worker 0 was still held, so rescue fired the timer with the waiter's waker: the old one is a no-op.
    wait_until("rescue counts the waiter's timer", || {
        !pending || rescued(scheduler) > before
    });
    released.store(true, Ordering::Release);
    step("joining the task that held worker 0");
    block_on(blocker).unwrap();
    pending
}

#[test]
fn a_rescued_task_avoids_its_stuck_preferred_worker_while_others_are_saturated() {
    test_support::run(|| {
        let scheduler = scheduler(2);
        let shared = Arc::clone(&scheduler.tasks.shared);
        // Keep one worker saturated with local work that never drains.
        let stop = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        let flag = Arc::clone(&stop);
        let saturating = scheduler
            .spawn(async move {
                send.send(worker::current(&shared)).unwrap();
                while !flag.load(Ordering::Acquire) {
                    yield_now().await;
                }
            })
            .unwrap();
        step("waiting for the saturating task to start");
        let busy = receive.recv().unwrap().unwrap();
        let stuck = 1 - busy;
        let waiter = wait_on(&scheduler, stuck);
        let released = Arc::new(AtomicBool::new(false));
        let (registered, blocker) = stall_with_timer(
            &scheduler,
            stuck,
            || sleep(&scheduler, DELAY),
            waiter.waker,
            &released,
        );
        // The stuck worker is still held and the busy one never drains, so the task ran because rescue woke it and the busy worker took it.
        step("waiting for the rescued task while its preferred worker is held");
        let (resumed, ran_on) = block_on(waiter.task).unwrap().unwrap();
        assert_not_early("rescued task", resumed - registered);
        assert_eq!(ran_on, Some(busy));
        wait_until("rescue counts the task's timer", || rescued(&scheduler) > 0);
        stop.store(true, Ordering::Release);
        released.store(true, Ordering::Release);
        step("joining the saturating task");
        block_on(saturating).unwrap();
        step("joining the task that held the stuck worker");
        block_on(blocker).unwrap();
        step("shutting down");
        block_on(scheduler.shutdown());
    });
}

#[test]
fn rescue_wakes_a_task_preferring_an_idle_worker_without_losing_it() {
    // The rescuer is either the monitor or the idle worker's pre-park check, which schedules into its own queue and must retry rather than park. The monitor usually rescues first, so this test reaches the pre-park path only incidentally; `a_rescue_turns_an_empty_idle_search_into_a_retry` in `worker/tests.rs` checks the retry itself.
    test_support::run(|| {
        for workers in [2, 4] {
            let scheduler = scheduler(workers);
            for iteration in 0..20 {
                let before = rescued(&scheduler);
                let waiter = wait_on(&scheduler, 1);
                let released = Arc::new(AtomicBool::new(false));
                let (registered, blocker) = stall_with_timer(
                    &scheduler,
                    0,
                    || sleep(&scheduler, DELAY),
                    waiter.waker,
                    &released,
                );
                // Whichever thread rescues, the task must run while worker 0 is held. If a pre-park rescuer lost it by parking, nothing else would wake that worker.
                step(format!(
                    "waiting for the woken task while worker 0 is held ({workers} workers, iteration {iteration})"
                ));
                let (resumed, _) = block_on(waiter.task).unwrap().unwrap();
                assert_not_early("woken task", resumed - registered);
                wait_until("rescue counts the task's timer", || {
                    rescued(&scheduler) > before
                });
                released.store(true, Ordering::Release);
                step("joining the task that held worker 0");
                block_on(blocker).unwrap();
            }
            step(format!("shutting down {workers} workers"));
            block_on(scheduler.shutdown());
        }
    });
}

#[derive(Default)]
struct Probe(AtomicUsize);
impl Wake for Probe {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_worker_that_blocks_while_the_monitor_sleeps_without_deadlines_is_still_rescued() {
    test_support::run(|| {
        for workers in [2, 4] {
            let scheduler = scheduler(workers);
            let monitor = &scheduler.tasks.shared.monitor;
            for iteration in 0..20 {
                // No deadline is pending, so the monitor sleeps until a registration wakes it.
                wait_until(
                    &format!(
                        "the monitor plans no wake ({workers} workers, iteration {iteration})"
                    ),
                    || monitor.next.load(Ordering::Relaxed) == crate::worker_timers::NO_DEADLINE,
                );
                let before = rescued(&scheduler);
                let probe = Arc::new(Probe::default());
                let released = Arc::new(AtomicBool::new(false));
                let (registered, blocker) = stall_with_timer(
                    &scheduler,
                    0,
                    || sleep(&scheduler, DELAY),
                    Waker::from(Arc::clone(&probe)),
                    &released,
                );
                // A monitor that missed the registration sleeps until woken again, and nothing else fires the timer while worker 0 is held.
                wait_until("rescue fires the timer while worker 0 is held", || {
                    probe.0.load(Ordering::SeqCst) > 0
                });
                assert_not_early("rescued timer", registered.elapsed());
                wait_until("rescue counts the timer", || rescued(&scheduler) > before);
                released.store(true, Ordering::Release);
                step("joining the task that held worker 0");
                block_on(blocker).unwrap();
            }
            step(format!("shutting down {workers} workers"));
            block_on(scheduler.shutdown());
        }
    });
}

#[test]
fn runnable_tasks_queued_behind_a_long_poll_run_on_another_worker() {
    test_support::run(|| {
        for workers in [2, 4] {
            let scheduler = scheduler(workers);
            let local: Vec<_> = (0..4).map(|_| wait_on(&scheduler, 0)).collect();
            let remote: Vec<_> = (0..4).map(|_| wait_on(&scheduler, 0)).collect();
            let released = Arc::new(AtomicBool::new(false));
            let (send, started) = mpsc::channel();
            let wakers: Vec<_> = local.iter().map(|waiter| waiter.waker.clone()).collect();
            // Wake from worker 0 itself, so the tasks enter its local ready queue, then block it.
            let blocker = spawn_on(&scheduler, 0, || {
                let wakers = wakers.clone();
                let send = send.clone();
                let released = Arc::clone(&released);
                async move {
                    for waker in wakers {
                        waker.wake();
                    }
                    send.send(()).unwrap();
                    block(&released);
                }
            });
            step("waiting for worker 0 to wake its local tasks");
            started.recv().unwrap();
            // Remote wakes enter worker 0's inbox, or skip it once worker 0 is marked stuck.
            for waiter in &remote {
                waiter.waker.wake_by_ref();
            }
            // Worker 0 is still held, so every task ran elsewhere or not at all.
            for (index, waiter) in local.into_iter().chain(remote).enumerate() {
                step(format!(
                    "waiting for queued task {index} while worker 0 is held ({workers} workers)"
                ));
                let (_, ran_on) = block_on(waiter.task).unwrap().unwrap();
                assert!(ran_on.is_some_and(|worker| worker != 0), "{ran_on:?}");
            }
            released.store(true, Ordering::Release);
            step("joining the task that held worker 0");
            block_on(blocker).unwrap();
            step(format!("shutting down {workers} workers"));
            block_on(scheduler.shutdown());
        }
    });
}

#[test]
fn closure_while_the_only_worker_is_stuck_notifies_its_timers_and_shutdown_waits() {
    test_support::run(|| {
        let scheduler = scheduler(1);
        let released = Arc::new(AtomicBool::new(false));
        let blocker = spawn_on(&scheduler, 0, || {
            let released = Arc::clone(&released);
            async move { block(&released) }
        });
        let probe = Arc::new(Probe::default());
        let waker = Waker::from(Arc::clone(&probe));
        let mut waits: Vec<_> = (0..4).map(|_| sleep(&scheduler, NEVER)).collect();
        for wait in &mut waits {
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
        }
        scheduler.close();
        // The worker is still held, so a rescuer drains its closed queue and notifies every waiter.
        wait_until("rescue notifies all four waiters after closure", || {
            probe.0.load(Ordering::SeqCst) == 4
        });
        wait_until("rescue counts the four timers", || rescued(&scheduler) >= 4);
        for wait in waits {
            assert_eq!(
                block_on(wait).unwrap_err().kind(),
                std::io::ErrorKind::BrokenPipe
            );
        }
        let mut shutdown = Box::pin(scheduler.shutdown());
        assert!(block_on(futures_lite::future::poll_once(&mut shutdown)).is_none());
        released.store(true, Ordering::Release);
        step("joining the task that held the only worker");
        block_on(blocker).unwrap();
        step("shutting down after the only worker returned");
        block_on(shutdown);
    });
}

#[test]
fn reentrant_and_panicking_wakers_do_not_stop_rescue() {
    struct Reentrant(
        Arc<NativeScheduler>,
        Arc<AtomicUsize>,
        Arc<Mutex<Vec<NativeTask<usize>>>>,
    );
    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            // This runs on a rescuing thread while the owner is stuck, and registers on the same queue.
            let mut nested = sleep(&self.0, NEVER);
            assert!(
                nested
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            // Spawning from a rescuing thread is allowed too; the task runs once the only worker returns.
            let spawned = self.0.spawn(async { 42 }).unwrap();
            self.2.lock().unwrap().push(spawned);
            self.1.fetch_add(1, Ordering::SeqCst);
            panic!("rescued waker panic");
        }
    }
    test_support::run(|| {
        let scheduler = scheduler(1);
        let released = Arc::new(AtomicBool::new(false));
        let woken = Arc::new(AtomicUsize::new(0));
        let spawned = Arc::new(Mutex::new(Vec::new()));
        let waker = Waker::from(Arc::new(Reentrant(
            Arc::clone(&scheduler),
            Arc::clone(&woken),
            Arc::clone(&spawned),
        )));
        let (_, blocker) =
            stall_with_timer(&scheduler, 0, || sleep(&scheduler, DELAY), waker, &released);
        wait_until("rescue runs the panicking waker", || {
            woken.load(Ordering::SeqCst) > 0
        });
        // The monitor counts that rescue once it has contained the panic.
        wait_until("rescue counts the panicking waker's timer", || {
            rescued(&scheduler) > 0
        });
        // Rescue continues after the panic: the only worker is still held, so only the monitor can fire this sleep. A sleep that did not register shows nothing, so repeat until one has; the watchdog bounds the repetition.
        let _ = (1..).find(|attempt| {
            let before = rescued(&scheduler);
            let start = Instant::now();
            step(format!(
                "waiting for a later sleep while the only worker is held (attempt {attempt})"
            ));
            let pending = block_on(pending_at_first_poll(sleep(&scheduler, DELAY)));
            assert_not_early("later sleep", start.elapsed());
            wait_until("rescue counts the later sleep", || {
                !pending || rescued(&scheduler) > before
            });
            pending
        });
        released.store(true, Ordering::Release);
        step("joining the task that held the only worker");
        block_on(blocker).unwrap();
        for task in std::mem::take(&mut *spawned.lock().unwrap()) {
            step("joining a task spawned by the reentrant waker");
            assert_eq!(block_on(task).unwrap(), 42);
        }
        step("shutting down");
        block_on(scheduler.shutdown());
    });
}

#[test]
fn repeated_short_stalls_near_the_threshold_complete() {
    test_support::run(|| {
        for workers in [2, 4] {
            let scheduler = scheduler(workers);
            let tasks: Vec<_> = (0..16)
                .map(|index| {
                    let scheduler = Arc::clone(&scheduler);
                    scheduler
                        .clone()
                        .spawn(async move {
                            for round in 0..20u64 {
                                scheduler
                                    .sleep(Duration::from_micros(500), &Cancellation::never())
                                    .await
                                    .unwrap();
                                // Spin around the stall threshold without yielding.
                                let spin = Duration::from_micros(500 + (index + round) % 4 * 500);
                                let start = Instant::now();
                                while start.elapsed() < spin {
                                    std::hint::spin_loop();
                                }
                            }
                        })
                        .unwrap()
                })
                .collect();
            for (index, task) in tasks.into_iter().enumerate() {
                step(format!("joining stalling task {index} ({workers} workers)"));
                block_on(task).unwrap();
            }
            step(format!("shutting down {workers} workers"));
            block_on(scheduler.shutdown());
        }
    });
}

#[test]
fn an_owner_returning_during_a_rescue_loses_no_timer() {
    // Worker 0 repeatedly blocks for three stall thresholds: timers falling due early in a block are rescued, and those falling due near its end race the owner's own timer pass when it returns.
    test_support::run(|| {
        let scheduler = scheduler(2);
        let done = Arc::new(AtomicBool::new(false));
        let stalls = {
            let done = Arc::clone(&done);
            spawn_on(&scheduler, 0, move || {
                let done = Arc::clone(&done);
                async move {
                    while !done.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(3));
                        yield_now().await;
                    }
                }
            })
        };
        let never = Cancellation::never();
        for index in 0..200u64 {
            // Root registrations alternate between workers, so half land on the stalling one.
            // A lost timer never fires: this thread is woken only by the sleep's waker.
            let delay = Duration::from_micros(500 + index % 7 * 400);
            step(format!("waiting for root sleep {index}"));
            block_on(scheduler.sleep(delay, &never)).unwrap();
        }
        done.store(true, Ordering::Release);
        step("joining the stalling task");
        block_on(stalls).unwrap();
        assert!(
            scheduler
                .tasks
                .shared
                .monitor
                .counters
                .timers
                .load(Ordering::Relaxed)
                > 0,
            "no timer was rescued, so the race was not exercised"
        );
        step("shutting down");
        block_on(scheduler.shutdown());
    });
}

#[test]
fn a_monitor_scheduled_after_closure_still_exits() {
    // The monitor passes its first check, then waits while the scheduler closes and finishes, so closure's wakes come before its first pass. They must still reach it, or shutdown waits for it forever.
    test_support::run(|| {
        for workers in [1, 2] {
            let gate = Arc::new(std::sync::Barrier::new(2));
            crate::monitor::START_GATE.set(Some(Arc::clone(&gate)));
            let scheduler = scheduler(workers);
            assert!(scheduler.tasks.shared.monitor.registered());
            step(format!(
                "waiting for the monitor to start ({workers} workers)"
            ));
            gate.wait();
            step("joining a task");
            block_on(scheduler.spawn(async {}).unwrap()).unwrap();
            // A joined task's runnable can retire after the join returns. Drained before closure, the scheduler is finished as soon as it closes; drained after, the wait would include the held monitor.
            step("draining tasks");
            block_on(scheduler.tasks.drain());
            scheduler.close();
            assert!(scheduler.tasks.shared.finished());
            step("releasing the monitor after closure");
            gate.wait();
            step("shutting down with a monitor scheduled after closure");
            block_on(scheduler.shutdown());
        }
    });
}
