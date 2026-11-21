// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

// The interface's scope, slot and panic paths, driven deterministically by a test-only backend that polls tasks on whichever thread calls `run`.

use base_scheduler::{ScopeToken, SpawnError, SpawnScoped, TaskError};
use std::{
    future::{Future, pending, poll_fn},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    task::{Context, Poll, Waker},
    thread,
    time::Duration,
};

type Erased = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

struct Entry {
    future: Mutex<Option<Erased>>,
    token: Mutex<Option<ScopeToken>>,
    dropped: AtomicBool,
}

impl Entry {
    // Destroy the future, containing a destructor panic, then release the token.
    fn destroy(&self, future: Option<Erased>) {
        if catch_unwind(AssertUnwindSafe(|| drop(future))).is_err() {
            DESTRUCTOR_PANICS.fetch_add(1, Ordering::SeqCst);
        }
        drop(self.token.lock().unwrap().take());
    }
}

static DESTRUCTOR_PANICS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Inline {
    tasks: Mutex<Vec<Arc<Entry>>>,
    closed: AtomicBool,
}

impl Inline {
    // Poll every live task once; destroy those that completed or whose handle was dropped during the poll.
    fn run(&self) {
        let tasks = self.tasks.lock().unwrap().clone();
        for entry in tasks {
            let mut future = entry.future.lock().unwrap();
            let Some(task) = future.as_mut() else {
                continue;
            };
            let ready = task
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_ready();
            if ready || entry.dropped.load(Ordering::SeqCst) {
                let task = future.take();
                drop(future);
                entry.destroy(task);
            }
        }
    }
    // Destroy every task, as an abrupt runtime teardown would.
    fn shutdown(&self) {
        for entry in self.tasks.lock().unwrap().drain(..) {
            let task = entry.future.lock().unwrap().take();
            entry.destroy(task);
        }
    }
}

struct InlineHandle(Arc<Entry>);

impl Drop for InlineHandle {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
        // During an active poll, `run` destroys the task once the poll returns.
        if let Ok(mut future) = self.0.future.try_lock() {
            let task = future.take();
            drop(future);
            self.0.destroy(task);
        }
    }
}

// SAFETY: A task's token is released only by `Entry::destroy`, after its future has been taken from the entry and dropped, and nothing polls an entry whose future was taken. A closed backend drops the future and token before returning.
unsafe impl SpawnScoped for Inline {
    type ScopedHandle = InlineHandle;
    unsafe fn spawn_unchecked<F>(
        &self,
        future: F,
        token: ScopeToken,
    ) -> Result<InlineHandle, SpawnError>
    where
        F: Future<Output = ()> + Send,
    {
        if self.closed.load(Ordering::SeqCst) {
            drop(future);
            drop(token);
            return Err(SpawnError::SchedulerClosed);
        }
        // SAFETY: The caller keeps the future's borrowed data valid until `token` is released, which happens only after the future is dropped.
        let future = unsafe {
            std::mem::transmute::<Pin<Box<dyn Future<Output = ()> + Send + '_>>, Erased>(Box::pin(
                future,
            ))
        };
        let entry = Arc::new(Entry {
            future: Mutex::new(Some(future)),
            token: Mutex::new(Some(token)),
            dropped: AtomicBool::new(false),
        });
        self.tasks.lock().unwrap().push(Arc::clone(&entry));
        Ok(InlineHandle(entry))
    }
}

struct DropCount<'a>(&'a AtomicUsize);
impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

// Poll a future once from the root, registering its waker.
async fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    poll_fn(|context| Poll::Ready(Pin::new(&mut *future).poll(context))).await
}

#[test]
fn output_wakes_the_parked_root() {
    let inline = Inline::default();
    let values = [1, 2, 3];
    let values = &values;
    let total = thread::scope(|threads| {
        threads.spawn(|| {
            thread::sleep(Duration::from_millis(20));
            inline.run();
        });
        inline.scope(async |scope| {
            let task = scope.spawn(async move { values.iter().sum::<i32>() });
            task.unwrap().await.unwrap()
        })
    });
    assert_eq!(total, 6);
}

#[test]
fn completed_output_is_taken_without_waiting() {
    let inline = Inline::default();
    let output = inline.scope(async |scope| {
        let task = scope.spawn(async { 7 }).unwrap();
        inline.run();
        task.await.unwrap()
    });
    assert_eq!(output, 7);
}

#[test]
fn destruction_before_completion_reports_cancellation() {
    let inline = Inline::default();
    let drops = AtomicUsize::new(0);
    let drops = &drops;
    let (polled, unpolled) = inline.scope(async |scope| {
        let capture = DropCount(drops);
        let mut first = scope
            .spawn(async move {
                let _capture = &capture;
                pending::<()>().await
            })
            .unwrap();
        let second = scope.spawn(pending::<()>()).unwrap();
        assert!(poll_once(&mut first).await.is_pending());
        inline.shutdown();
        (first.await, second.await)
    });
    assert!(matches!(polled, Err(TaskError::Cancelled)));
    assert!(matches!(unpolled, Err(TaskError::Cancelled)));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn destructor_panics_still_report_cancellation() {
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("destructor");
        }
    }
    let inline = Inline::default();
    let before = DESTRUCTOR_PANICS.load(Ordering::SeqCst);
    let result = inline.scope(async |scope| {
        let capture = PanicOnDrop;
        let task = scope
            .spawn(async move {
                let _capture = &capture;
                pending::<()>().await
            })
            .unwrap();
        inline.shutdown();
        task.await
    });
    assert!(matches!(result, Err(TaskError::Cancelled)));
    assert_eq!(DESTRUCTOR_PANICS.load(Ordering::SeqCst), before + 1);
}

#[test]
fn abandoned_tasks_are_destroyed_without_reporting() {
    let inline = Inline::default();
    let drops = AtomicUsize::new(0);
    let drops = &drops;
    inline.scope(async |scope| {
        let capture = DropCount(drops);
        drop(
            scope
                .spawn(async move {
                    let _capture = &capture;
                    pending::<()>().await
                })
                .unwrap(),
        );
        let completed = scope.spawn(async move { DropCount(drops) }).unwrap();
        inline.run();
        drop(completed);
    });
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

// A task that drops its own handle during its poll, so that it completes after being abandoned.
#[test]
fn output_completed_after_abandonment_is_destroyed() {
    let inline = Inline::default();
    let drops = AtomicUsize::new(0);
    let drops = &drops;
    inline.scope(async |scope| {
        let own = Arc::new(Mutex::new(None));
        let held = Arc::clone(&own);
        let task = scope
            .spawn(async move {
                drop(held.lock().unwrap().take());
                DropCount(drops)
            })
            .unwrap();
        *own.lock().unwrap() = Some(task);
        inline.run();
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn panic_completed_after_abandonment_panics_the_scope() {
    let inline = Inline::default();
    let result = catch_unwind(AssertUnwindSafe(|| {
        inline.scope(async |scope| {
            let own = Arc::new(Mutex::new(None));
            let held = Arc::clone(&own);
            let task = scope
                .spawn(async move {
                    drop(held.lock().unwrap().take());
                    panic!("abandoned task");
                })
                .unwrap();
            *own.lock().unwrap() = Some(task);
            inline.run();
        })
    }));
    assert!(result.is_err());
}

#[test]
fn unobserved_panic_panics_the_scope_and_observed_does_not() {
    let inline = Inline::default();
    let observed = inline.scope(async |scope| {
        let task = scope.spawn(async { panic!("observed") }).unwrap();
        inline.run();
        matches!(task.await, Err(TaskError::Panicked(_)))
    });
    assert!(observed);
    let result = catch_unwind(AssertUnwindSafe(|| {
        inline.scope(async |scope| {
            let task = scope.spawn(async { panic!("unobserved") }).unwrap();
            inline.run();
            drop(task);
        })
    }));
    let payload = result.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"a scoped task panicked and its panic was not observed")
    );
}

// The root panics while a task's poll is active on another thread, so the scope must wait for that poll's end before resuming the panic.
#[test]
fn root_panic_waits_for_an_active_poll() {
    let inline = Inline::default();
    let drops = AtomicUsize::new(0);
    let drops = &drops;
    let (entered, entry) = mpsc::channel();
    let (start, started) = mpsc::channel();
    let result = thread::scope(|threads| {
        let runner = &inline;
        threads.spawn(move || {
            started.recv().unwrap();
            runner.run();
        });
        catch_unwind(AssertUnwindSafe(|| {
            inline.scope(async |scope| {
                let capture = DropCount(drops);
                let _task = scope
                    .spawn(async move {
                        let _capture = &capture;
                        entered.send(()).unwrap();
                        thread::sleep(Duration::from_millis(50));
                        pending::<()>().await
                    })
                    .unwrap();
                start.send(()).unwrap();
                entry.recv().unwrap();
                panic!("root");
            })
        }))
    });
    assert_eq!(result.unwrap_err().downcast_ref::<&str>(), Some(&"root"));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn closed_backend_rejects_and_drops_the_future() {
    let inline = Inline::default();
    inline.closed.store(true, Ordering::SeqCst);
    let drops = AtomicUsize::new(0);
    let drops = &drops;
    let result = inline.scope(async |scope| {
        let capture = DropCount(drops);
        scope
            .spawn(async move {
                let _capture = &capture;
            })
            .map(drop)
    });
    assert!(matches!(result, Err(SpawnError::SchedulerClosed)));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
