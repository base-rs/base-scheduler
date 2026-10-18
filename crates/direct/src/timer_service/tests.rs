// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support;
use futures_lite::future::{block_on, poll_once};
use protocol_scheduler_conformance::ClockFixture;
use std::time::Duration;

pub(super) fn observe_wait(state: &State, deadline: bool) {
    if let Some(waits) = &state.waits {
        let _ = waits.send(deadline);
    }
}
fn wait_for_driver(waits: &std::sync::mpsc::Receiver<bool>, deadline: bool) {
    while waits.recv_timeout(Duration::from_secs(5)).unwrap() != deadline {}
}

impl ClockFixture for DirectScheduler {
    type Scheduler = Self;
    fn scheduler(&self) -> &Self {
        self
    }
    fn advance(&self, duration: Duration) {
        let wakes = {
            let mut state = self.timers.shared.state.lock().unwrap();
            *state.paused.as_mut().unwrap() += duration;
            state.expired()
        };
        for waker in wakes {
            waker.wake();
        }
        self.timers.shared.changed.notify_all();
    }
    fn registrations(&self) -> usize {
        self.timers.shared.state.lock().unwrap().timers.len()
    }
}
fn run(test: impl AsyncFnOnce(&DirectScheduler) + Send + 'static) {
    test_support::run(move || {
        let scheduler = test_support::scheduler();
        scheduler.timers.shared.state.lock().unwrap().paused = Some(Instant::now());
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(test(&scheduler))));
        block_on(scheduler.shutdown());
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    });
}
mod conformance {
    protocol_scheduler_conformance::clock_tests!(run: super::run);
}

#[test]
fn native_timer_driver_wakes_and_retires_deadlines() {
    test_support::run(|| {
        block_on(async {
            let scheduler = test_support::scheduler();
            let (waits, observed) = std::sync::mpsc::channel();
            scheduler.timers.shared.state.lock().unwrap().waits = Some(waits);
            scheduler.timers.start().unwrap();
            wait_for_driver(&observed, false);
            let never = Cancellation::never();
            let mut distant = Box::pin(scheduler.sleep(Duration::from_secs(3600), &never));
            assert!(poll_once(&mut distant).await.is_none());
            wait_for_driver(&observed, true);
            drop(distant);
            wait_for_driver(&observed, false);
            let deadline = scheduler.now() + Duration::from_millis(50);
            scheduler
                .sleep_until(deadline, &Cancellation::never())
                .await
                .unwrap();
            assert!(scheduler.now() >= deadline);
            scheduler.shutdown().await;
        })
    });
}
#[test]
fn startup_failure_exhaustion_and_shutdown_release_waiters() {
    run(async |scheduler| {
        scheduler.timers.shared.state.lock().unwrap().paused = None;
        let never = Cancellation::never();
        thread::FAIL_AFTER.set(Some(0));
        assert!(
            scheduler
                .sleep(Duration::from_secs(10), &never)
                .await
                .is_err()
        );
        scheduler.timers.shared.state.lock().unwrap().next = usize::MAX;
        assert!(
            scheduler
                .sleep(Duration::from_secs(10), &never)
                .await
                .is_err()
        );
        scheduler.timers.shared.state.lock().unwrap().next = 0;
        let mut wait = Box::pin(scheduler.sleep(Duration::from_secs(10), &never));
        assert!(poll_once(&mut wait).await.is_none());
        scheduler.close();
        assert_eq!(wait.await.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    });
}
