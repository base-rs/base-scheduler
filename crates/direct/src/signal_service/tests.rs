// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::test_support;
use futures_lite::future::block_on;
use protocol_scheduler::Cancellation;
use protocol_scheduler_conformance::SignalsFixture;

struct Fixture {
    scheduler: Arc<DirectScheduler>,
}
impl SignalsFixture for Fixture {
    type Scheduler = DirectScheduler;
    type Signal = i32;
    fn scheduler(&self) -> &Arc<DirectScheduler> {
        &self.scheduler
    }
    fn signals(&self) -> [i32; 2] {
        [libc::SIGUSR1, libc::SIGUSR2]
    }
    fn invalid_signal(&self) -> i32 {
        0
    }
    async fn emit(&self, signal: i32) {
        self.scheduler.signals.shared.dispatch(signal);
    }
}
fn run(test: impl AsyncFnOnce(&Fixture) + Send + 'static) {
    test_support::run(move || {
        let fixture = Fixture {
            scheduler: Arc::new(test_support::scheduler()),
        };
        let result = catch_unwind(AssertUnwindSafe(|| block_on(test(&fixture))));
        block_on(fixture.scheduler.shutdown());
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    });
}
mod conformance {
    protocol_scheduler_conformance::signals_tests!(run: super::run);
}
mod dispatch {
    protocol_scheduler_conformance::signals_dispatch_tests!(run: super::run);
}

#[test]
fn rejects_empty_forbidden_native_and_closed_registrations_and_startup_failure() {
    let scheduler = test_support::scheduler();
    for signals in [
        &[][..],
        &[0],
        &[128],
        &[libc::SIGKILL],
        &[libc::SIGSEGV],
        &[127],
    ] {
        assert!(
            scheduler
                .signal_subscribe(signals, Cancellation::never())
                .is_err()
        );
    }
    thread::FAIL_AFTER.set(Some(0));
    assert!(
        scheduler
            .signal_subscribe(&[libc::SIGUSR1], Cancellation::never())
            .is_err()
    );
    let subscription = scheduler
        .signal_subscribe(&[libc::SIGUSR1], Cancellation::never())
        .unwrap();
    assert!(
        scheduler
            .signal_subscribe(&[127], Cancellation::never())
            .is_err()
    );
    scheduler.signals.shared.state.lock().unwrap().next = usize::MAX;
    assert!(
        scheduler
            .signal_subscribe(&[libc::SIGUSR1], Cancellation::never())
            .is_err()
    );
    scheduler.close();
    assert!(
        scheduler
            .signal_subscribe(&[libc::SIGUSR1], Cancellation::never())
            .is_err()
    );
    block_on(scheduler.shutdown());
    drop(scheduler);
    drop(subscription);
}
#[test]
fn panicking_targets_do_not_poison_dispatch_or_prevent_other_targets() {
    struct Panics;
    impl Notify<i32> for Panics {
        fn notify(&self, _: i32) {
            panic!("signal callback");
        }
    }
    let scheduler = test_support::scheduler();
    let _panics = scheduler
        .signal_subscribe(&[libc::SIGUSR1], Panics)
        .unwrap();
    let token = Cancellation::new();
    let _other = scheduler
        .signal_subscribe(&[libc::SIGUSR1], token.clone())
        .unwrap();
    scheduler.signals.shared.dispatch(libc::SIGUSR1);
    assert!(token.is_cancelled());
    block_on(scheduler.shutdown());
}

#[test]
fn os_signal_bridge_delivers_outside_the_raw_handler() {
    const CHILD: &str = "PROTOCOL_DIRECT_SIGNAL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "signal_service::tests::os_signal_bridge_delivers_outside_the_raw_handler",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let scheduler = test_support::scheduler();
    let (sender, receiver) = std::sync::mpsc::channel();
    let _subscription = scheduler
        .signal_subscribe(&[libc::SIGUSR1], sender)
        .unwrap();
    // SAFETY: This child process installed its handler before raising the signal; no parallel tests share this process.
    assert_eq!(unsafe { libc::raise(libc::SIGUSR1) }, 0);
    assert_eq!(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
        libc::SIGUSR1
    );
    block_on(scheduler.shutdown());
}
