// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// End-to-end rescue scenarios on running schedulers. Placement uses the crate's worker identity, so these live beside the monitor rather than in the public integration tests.

use crate::{NativeOptions, NativeScheduler, NativeTask, test_support, worker};
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
// A blocking poll holds its worker for at most `HOLD`, and usually until the test releases it.
const HOLD: Duration = Duration::from_secs(5);
// Large compared with the rescue bound of about two milliseconds, and far below `HOLD`.
const LATE: Duration = Duration::from_millis(250);

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

// Hold the current worker inside one poll until released, standing in for undeclared blocking work.
fn block(released: &AtomicBool) {
    let deadline = Instant::now() + HOLD;
    while !released.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
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
    // Idle workers take spawned tasks promptly; the test watchdog bounds this search.
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
        if start.recv_timeout(HOLD).unwrap() {
            return task;
        }
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
    let waker = receive.recv_timeout(HOLD).unwrap();
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
            assert!(
                timer
                    .as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
            send.send(Instant::now()).unwrap();
            block(&released);
            drop(timer);
        }
    });
    (receive.recv_timeout(HOLD).unwrap(), task)
}

fn assert_prompt(label: &str, elapsed: Duration) {
    assert!(
        elapsed >= DELAY,
        "{label} completed early after {elapsed:?}"
    );
    assert!(
        elapsed < LATE,
        "{label} took {elapsed:?} for a {DELAY:?} deadline"
    );
}

#[test]
fn a_timer_left_on_a_worker_that_blocks_fires_for_its_new_waiter() {
    test_support::run(|| {
        let scheduler = scheduler(2);
        for timer in [sleep, reusable] {
            let released = Arc::new(AtomicBool::new(false));
            let (send, receive) = futures_channel::oneshot::channel::<(Instant, Sleep)>();
            let receive = Arc::new(Mutex::new(Some(receive)));
            let source = Arc::clone(&scheduler);
            // Register on worker 0, hand the pending wait to another task, then block worker 0.
            let send = Arc::new(Mutex::new(Some(send)));
            let blocker = spawn_on(&scheduler, 0, || {
                let send = Arc::clone(&send);
                let released = Arc::clone(&released);
                let mut wait = timer(&source, DELAY);
                async move {
                    assert!(
                        wait.as_mut()
                            .poll(&mut Context::from_waker(Waker::noop()))
                            .is_pending()
                    );
                    let registered = Instant::now();
                    // Only the copy that starts on worker 0 runs this body.
                    let send = send.lock().unwrap().take().unwrap();
                    assert!(send.send((registered, wait)).is_ok());
                    block(&released);
                }
            });
            // The new waiter runs on the other worker and updates the registration's waker in worker 0's queue.
            let waiter = scheduler
                .spawn(async move {
                    let receive = receive.lock().unwrap().take().unwrap();
                    let (registered, wait) = receive.await.unwrap();
                    wait.await.unwrap();
                    registered.elapsed()
                })
                .unwrap();
            let elapsed = block_on(waiter).unwrap();
            assert_prompt("waiter", elapsed);
            released.store(true, Ordering::Release);
            block_on(blocker).unwrap();
        }
        block_on(scheduler.shutdown());
    });
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
        let busy = receive.recv_timeout(HOLD).unwrap().unwrap();
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
        let (resumed, ran_on) = block_on(waiter.task).unwrap().unwrap();
        assert_prompt("rescued task", resumed - registered);
        assert_eq!(ran_on, Some(busy));
        stop.store(true, Ordering::Release);
        released.store(true, Ordering::Release);
        block_on(saturating).unwrap();
        block_on(blocker).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[test]
fn rescue_wakes_a_task_preferring_an_idle_worker_without_losing_it() {
    // The rescuer is either the monitor or the idle worker's pre-park check. The latter schedules into its own queue and must retry rather than park.
    test_support::run(|| {
        let scheduler = scheduler(2);
        for _ in 0..20 {
            let waiter = wait_on(&scheduler, 1);
            let released = Arc::new(AtomicBool::new(false));
            let (registered, blocker) = stall_with_timer(
                &scheduler,
                0,
                || sleep(&scheduler, DELAY),
                waiter.waker,
                &released,
            );
            let (resumed, _) = block_on(waiter.task).unwrap().unwrap();
            assert_prompt("woken task", resumed - registered);
            released.store(true, Ordering::Release);
            block_on(blocker).unwrap();
        }
        block_on(scheduler.shutdown());
    });
}

#[derive(Default)]
struct Probe(AtomicUsize);
impl Wake for Probe {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + HOLD;
    while !condition() {
        assert!(Instant::now() < deadline, "condition not reached");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_worker_that_blocks_while_the_monitor_sleeps_without_deadlines_is_still_rescued() {
    test_support::run(|| {
        let scheduler = scheduler(2);
        let monitor = &scheduler.tasks.shared.monitor;
        for _ in 0..20 {
            // No deadline is pending, so the monitor sleeps until a registration wakes it.
            wait_until(|| {
                monitor.next.load(Ordering::Relaxed) == crate::worker_timers::NO_DEADLINE
            });
            let probe = Arc::new(Probe::default());
            let released = Arc::new(AtomicBool::new(false));
            let (registered, blocker) = stall_with_timer(
                &scheduler,
                0,
                || sleep(&scheduler, DELAY),
                Waker::from(Arc::clone(&probe)),
                &released,
            );
            wait_until(|| probe.0.load(Ordering::SeqCst) > 0);
            assert_prompt("rescued timer", registered.elapsed());
            released.store(true, Ordering::Release);
            block_on(blocker).unwrap();
        }
        block_on(scheduler.shutdown());
    });
}

#[test]
fn runnable_tasks_queued_behind_a_long_poll_run_on_another_worker() {
    test_support::run(|| {
        let scheduler = scheduler(2);
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
                send.send(Instant::now()).unwrap();
                block(&released);
            }
        });
        let start = started.recv_timeout(HOLD).unwrap();
        // Remote wakes enter worker 0's inbox, or skip it once worker 0 is marked stuck.
        for waiter in &remote {
            waiter.waker.wake_by_ref();
        }
        for waiter in local.into_iter().chain(remote) {
            let (resumed, ran_on) = block_on(waiter.task).unwrap().unwrap();
            assert!(resumed - start < LATE);
            assert_eq!(ran_on, Some(1));
        }
        released.store(true, Ordering::Release);
        block_on(blocker).unwrap();
        block_on(scheduler.shutdown());
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
        let mut waits: Vec<_> = (0..4).map(|_| sleep(&scheduler, HOLD * 10)).collect();
        for wait in &mut waits {
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(&waker))
                    .is_pending()
            );
        }
        let closed = Instant::now();
        scheduler.close();
        // The worker is still stuck, so a rescuer drains its closed queue and notifies every waiter.
        wait_until(|| probe.0.load(Ordering::SeqCst) == 4);
        assert!(closed.elapsed() < LATE);
        assert!(!released.load(Ordering::Acquire));
        for wait in waits {
            assert_eq!(
                block_on(wait).unwrap_err().kind(),
                std::io::ErrorKind::BrokenPipe
            );
        }
        let mut shutdown = Box::pin(scheduler.shutdown());
        assert!(block_on(futures_lite::future::poll_once(&mut shutdown)).is_none());
        released.store(true, Ordering::Release);
        block_on(blocker).unwrap();
        block_on(shutdown);
    });
}

#[test]
fn reentrant_and_panicking_wakers_do_not_stop_rescue() {
    struct Reentrant(Arc<NativeScheduler>, Arc<AtomicUsize>);
    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            // This runs on a rescuing thread while the owner is stuck, and registers on the same queue.
            let mut nested = sleep(&self.0, HOLD * 10);
            assert!(
                nested
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            self.1.fetch_add(1, Ordering::SeqCst);
            panic!("rescued waker panic");
        }
    }
    test_support::run(|| {
        let scheduler = scheduler(1);
        let released = Arc::new(AtomicBool::new(false));
        let woken = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(Reentrant(
            Arc::clone(&scheduler),
            Arc::clone(&woken),
        )));
        let (_, blocker) =
            stall_with_timer(&scheduler, 0, || sleep(&scheduler, DELAY), waker, &released);
        wait_until(|| woken.load(Ordering::SeqCst) > 0);
        // Rescue continues after the panic.
        let start = Instant::now();
        block_on(sleep(&scheduler, DELAY)).unwrap();
        assert_prompt("later sleep", start.elapsed());
        released.store(true, Ordering::Release);
        block_on(blocker).unwrap();
        block_on(scheduler.shutdown());
    });
}

#[test]
fn repeated_short_stalls_near_the_threshold_complete() {
    test_support::run(|| {
        let scheduler = scheduler(2);
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
        for task in tasks {
            block_on(task).unwrap();
        }
        block_on(scheduler.shutdown());
    });
}
