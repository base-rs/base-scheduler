// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Scoped task assertions. Every case borrows data from its own stack frame and checks that data after `scope` returns, so a task that outlived its scope is observed rather than assumed.

use crate::SpawnScopedFixture;
use base_scheduler::{SpawnError, SpawnScoped, TaskError};
use futures_channel::oneshot;
use std::{
    collections::HashSet,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(5);

/// Records into borrowed data after a delay, so a scope that returned before destroying the value's owner would observe the count unchanged.
struct LateDrop<'a>(&'a AtomicUsize);
impl Drop for LateDrop<'_> {
    fn drop(&mut self) {
        thread::sleep(Duration::from_millis(20));
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A task body that never completes: it counts its polls, signals its first, and records its destruction late.
struct Hold<'a> {
    polls: &'a AtomicUsize,
    started: Option<oneshot::Sender<()>>,
    _capture: LateDrop<'a>,
}
impl<'a> Hold<'a> {
    fn new(drops: &'a AtomicUsize, polls: &'a AtomicUsize) -> Self {
        Self {
            polls,
            started: None,
            _capture: LateDrop(drops),
        }
    }
    // Also return a receiver that completes on the first poll.
    fn started(drops: &'a AtomicUsize, polls: &'a AtomicUsize) -> (Self, oneshot::Receiver<()>) {
        let (started, entered) = oneshot::channel();
        let mut hold = Self::new(drops, polls);
        hold.started = Some(started);
        (hold, entered)
    }
}
impl Future for Hold<'_> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        this.polls.fetch_add(1, Ordering::SeqCst);
        let _ = this.started.take().map(|started| started.send(()));
        Poll::Pending
    }
}

/// Sends on drop, including while a panic unwinds through its owner.
struct Signal(Option<oneshot::Sender<()>>);
impl Drop for Signal {
    fn drop(&mut self) {
        let _ = self.0.take().unwrap().send(());
    }
}

/// Spin on the calling worker until `condition` holds, failing after the deadline.
fn spin_until(condition: impl Fn() -> bool, failure: &str) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < DEADLINE, "{failure}");
        thread::yield_now();
    }
}

/// A task borrowing the caller's data completes, and its output reaches the root. A task may also mutate data it borrows exclusively.
pub fn borrowed_output<F: SpawnScopedFixture>(fixture: &F) {
    let values = vec![1, 2, 3];
    let mut updates = 0;
    let (values_ref, updates_ref) = (&values, &mut updates);
    let total = fixture.scheduler().scope(async |scope| {
        let sum = scope
            .spawn(async move { values_ref.iter().sum::<i32>() })
            .unwrap();
        let update = scope.spawn(async move { *updates_ref += 1 }).unwrap();
        update.await.unwrap();
        sum.await.unwrap()
    });
    assert_eq!(total, 6);
    assert_eq!(updates, 1);
}

/// Tasks run on more than one worker at once: each waits, within a deadline, until both have started. Requires a fixture with at least two workers.
pub fn parallel<F: SpawnScopedFixture>(fixture: &F) {
    let arrived = AtomicUsize::new(0);
    let threads = Mutex::new(HashSet::new());
    let (arrived_ref, threads_ref) = (&arrived, &threads);
    fixture.scheduler().scope(async |scope| {
        let tasks: Vec<_> = (0..2)
            .map(|_| {
                scope
                    .spawn(async move {
                        threads_ref.lock().unwrap().insert(thread::current().id());
                        arrived_ref.fetch_add(1, Ordering::SeqCst);
                        spin_until(
                            || arrived_ref.load(Ordering::SeqCst) == 2,
                            "scoped tasks did not run in parallel",
                        );
                    })
                    .unwrap()
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
    });
    assert_eq!(threads.into_inner().unwrap().len(), 2);
}

/// Dropping a pending task's handle destroys the task before `scope` returns, whether or not it was polled.
pub fn pending_abandonment<F: SpawnScopedFixture>(fixture: &F) {
    let drops = AtomicUsize::new(0);
    let polls = AtomicUsize::new(0);
    let (drops_ref, polls_ref) = (&drops, &polls);
    fixture.scheduler().scope(async |scope| {
        drop(scope.spawn(Hold::new(drops_ref, polls_ref)).unwrap());
        let (hold, entered) = Hold::started(drops_ref, polls_ref);
        let polled = scope.spawn(hold).unwrap();
        entered.await.unwrap();
        drop(polled);
    });
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "a scoped task outlived its scope"
    );
}

/// Dropping a completed task's handle destroys its unconsumed output before `scope` returns.
pub fn completed_abandonment<F: SpawnScopedFixture>(fixture: &F) {
    let drops = AtomicUsize::new(0);
    let drops_ref = &drops;
    fixture.scheduler().scope(async |scope| {
        let (done, completed) = oneshot::channel();
        let task = scope
            .spawn(async move {
                done.send(()).unwrap();
                LateDrop(drops_ref)
            })
            .unwrap();
        completed.await.unwrap();
        // Let the output reach the task's storage; either order must destroy it.
        thread::sleep(Duration::from_millis(10));
        drop(task);
    });
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "a scoped task's output outlived its scope"
    );
}

/// Tasks spawned by tasks are waited for, including a task spawned after the root has returned, by a task whose poll is still active.
pub fn nested<F: SpawnScopedFixture>(fixture: &F) {
    let drops = AtomicUsize::new(0);
    let polls = AtomicUsize::new(0);
    let returned = AtomicBool::new(false);
    let (drops_ref, polls_ref, returned_ref) = (&drops, &polls, &returned);
    fixture.scheduler().scope(async |scope| {
        let parent = scope
            .spawn(async move { scope.spawn(async { 7 }).unwrap().await.unwrap() })
            .unwrap();
        assert_eq!(parent.await.unwrap(), 7);
        let (started, entered) = oneshot::channel();
        let _late = scope
            .spawn(async move {
                started.send(()).unwrap();
                spin_until(
                    || returned_ref.load(Ordering::SeqCst),
                    "the root did not return",
                );
                thread::sleep(Duration::from_millis(20));
                // The child's handle is this task's output, destroyed with it once abandoned.
                Some(scope.spawn(Hold::new(drops_ref, polls_ref)).unwrap())
            })
            .unwrap();
        entered.await.unwrap();
        returned_ref.store(true, Ordering::SeqCst);
    });
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "a task spawned by a task outlived its scope"
    );
}

/// The root's panic is resumed only after every task has been destroyed.
pub fn root_panic<F: SpawnScopedFixture>(fixture: &F) {
    let drops = AtomicUsize::new(0);
    let polls = AtomicUsize::new(0);
    let (drops_ref, polls_ref) = (&drops, &polls);
    let result = catch_unwind(AssertUnwindSafe(|| {
        fixture.scheduler().scope(async |scope| {
            let (hold, entered) = Hold::started(drops_ref, polls_ref);
            let _task = scope.spawn(hold).unwrap();
            entered.await.unwrap();
            std::panic::panic_any("root panic");
        })
    }));
    let payload = result.expect_err("the root's panic was not resumed");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"root panic"));
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "a scoped task outlived its panicking root"
    );
}

/// Awaiting a panicked task returns its payload and `scope` returns normally; a panic whose handle was dropped unobserved makes `scope` panic after every task has been destroyed.
pub fn task_panic<F: SpawnScopedFixture>(fixture: &F) {
    let observed = fixture.scheduler().scope(async |scope| {
        let task = scope
            .spawn(async { std::panic::panic_any::<&str>("observed") })
            .unwrap();
        let Err(TaskError::Panicked(payload)) = task.await else {
            unreachable!("an observed panic was not reported");
        };
        payload.downcast_ref::<&str>().copied()
    });
    assert_eq!(observed, Some("observed"));

    let drops = AtomicUsize::new(0);
    let drops_ref = &drops;
    let result = catch_unwind(AssertUnwindSafe(|| {
        fixture.scheduler().scope(async |scope| {
            let (panicking, entered) = oneshot::channel();
            let task = scope
                .spawn(async move {
                    let _signal = Signal(Some(panicking));
                    let _capture = LateDrop(drops_ref);
                    std::panic::panic_any::<&str>("unobserved")
                })
                .unwrap();
            entered.await.unwrap();
            drop(task);
        })
    }));
    assert!(result.is_err(), "an unobserved task panic was lost");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

/// Spawning after the scheduler closes returns `SchedulerClosed` and drops the future unpolled, and `scope` still returns.
pub fn closed<F: SpawnScopedFixture>(fixture: &F) {
    let drops = AtomicUsize::new(0);
    let polls = AtomicUsize::new(0);
    let (drops_ref, polls_ref) = (&drops, &polls);
    let result = fixture.scheduler().scope(async |scope| {
        fixture.close();
        scope.spawn(Hold::new(drops_ref, polls_ref)).map(drop)
    });
    assert!(matches!(result, Err(SpawnError::SchedulerClosed)));
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
