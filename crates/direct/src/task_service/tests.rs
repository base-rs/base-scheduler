// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{DirectScheduler, test_support};
use futures_lite::future::block_on;
use protocol_scheduler::{Cancellation, Spawn};
use protocol_scheduler_conformance::SpawnFixture;

impl SpawnFixture for DirectScheduler {
    type Scheduler = Self;
    fn scheduler(&self) -> &Self {
        self
    }
    fn registration_check(&self) -> impl FnOnce() + Send + 'static {
        let shared = Arc::clone(&self.tasks.shared);
        move || assert_eq!(shared.state.lock().unwrap().registered.len(), 1)
    }
    async fn settle(&self) {
        self.tasks.wait(false).await;
    }
    fn close(&self) {
        self.tasks.close();
    }
}
fn run(test: impl AsyncFnOnce(&DirectScheduler) + Send + 'static) {
    test_support::run(move || {
        let scheduler = test_support::scheduler();
        let result = catch_unwind(AssertUnwindSafe(|| block_on(test(&scheduler))));
        block_on(scheduler.shutdown());
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    });
}
mod conformance {
    protocol_scheduler_conformance::spawn_tests!(run: super::run);
}
mod dispatch {
    protocol_scheduler_conformance::spawn_dispatch_tests!(run: super::run);
}

#[test]
fn startup_and_identifier_exhaustion_are_reported_without_polling_inputs() {
    for fail_after in [0, 1] {
        thread::FAIL_AFTER.set(Some(fail_after));
        assert!(
            DirectScheduler::new(crate::DirectOptions {
                threads: 2,
                capacity: 1
            })
            .is_err()
        );
    }
    let scheduler = test_support::scheduler();
    scheduler.tasks.shared.state.lock().unwrap().next = usize::MAX;
    assert!(matches!(
        scheduler.spawn(async {}),
        Err(SpawnError::IdentifiersExhausted)
    ));
    block_on(scheduler.shutdown());
}

#[test]
fn tasks_progress_without_occupying_syscall_workers_and_survive_admission_close() {
    test_support::run(|| {
        block_on(async {
            use protocol_scheduler::Blocking;
            let scheduler = test_support::scheduler();
            let token = Cancellation::new();
            let child = token.clone();
            let task = scheduler
                .spawn(async move {
                    child.cancelled().await;
                    42
                })
                .unwrap();
            scheduler.tasks.wait(false).await;
            let result = scheduler
                .spawn_blocking(|| 73, &Cancellation::never())
                .await
                .unwrap();
            assert_eq!(result.await.unwrap(), 73);
            scheduler.tasks.close();
            token.cancel();
            assert_eq!(task.await.unwrap(), 42);
            scheduler.shutdown().await;
        })
    });
}

#[test]
fn future_destructor_panic_is_reported_and_worker_stays_available() {
    struct Panics;
    impl Future for Panics {
        type Output = ();
        fn poll(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    impl Drop for Panics {
        fn drop(&mut self) {
            std::panic::panic_any(String::from("future destructor"));
        }
    }
    run(async |scheduler| {
        let Err(TaskError::Panicked(payload)) = scheduler.spawn(Panics).unwrap().await else {
            panic!("lost destructor panic");
        };
        assert_eq!(*payload.downcast::<String>().unwrap(), "future destructor");
        assert_eq!(scheduler.spawn(async { 17 }).unwrap().await.unwrap(), 17);
    });
}
