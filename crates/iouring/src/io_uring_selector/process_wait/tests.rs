// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use futures_lite::future::{block_on, poll_once};

thread_local! {
    pub(super) static FORCE_FALLBACK: Cell<bool> = const { Cell::new(false) };
}
struct Fallback;
impl Fallback {
    fn new() -> Self {
        FORCE_FALLBACK.set(true);
        Self
    }
}
impl Drop for Fallback {
    fn drop(&mut self) {
        FORCE_FALLBACK.set(false);
    }
}

#[test]
fn native_status_conversion_preserves_every_child_state() {
    assert_eq!(status(0, 0, 0).unwrap(), None);
    assert_eq!(
        status(123, libc::CLD_EXITED, 23)
            .unwrap()
            .unwrap()
            .status
            .code(),
        Some(23)
    );
    assert_eq!(
        status(123, libc::CLD_KILLED, libc::SIGTERM)
            .unwrap()
            .unwrap()
            .status
            .signal(),
        Some(libc::SIGTERM)
    );
    assert!(
        status(123, libc::CLD_DUMPED, libc::SIGABRT)
            .unwrap()
            .unwrap()
            .status
            .core_dumped()
    );
    for code in [libc::CLD_STOPPED, libc::CLD_TRAPPED] {
        assert_eq!(
            status(123, code, libc::SIGSTOP)
                .unwrap()
                .unwrap()
                .status
                .stopped_signal(),
            Some(libc::SIGSTOP)
        );
    }
    assert!(
        status(123, libc::CLD_CONTINUED, 0)
            .unwrap()
            .unwrap()
            .status
            .continued()
    );
    assert_eq!(
        status(123, -1, 0).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn optional_waitid_uses_the_shared_fallback_when_unavailable() {
    let _fallback = Fallback::new();
    let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
    let error = block_on(selector.process_wait(
        std::process::id() as i32,
        ProcessWaitFlags::default(),
        &Cancellation::never(),
    ))
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
    drop(driver);
    block_on(selector.shutdown());
}

#[test]
fn capacity_cancellation_closure_and_identifier_exhaustion_prevent_submission() {
    for cancel in [true, false] {
        let (selector, driver) = IoUringSelector::new(IoUringOptions {
            capacity: 1,
            ..Default::default()
        })
        .unwrap();
        let controller = &selector.controllers[0];
        let _permit = block_on(controller.capacity.acquire(&Cancellation::never())).unwrap();
        let cancellation = Cancellation::new();
        let mut wait =
            Box::pin(selector.process_wait(i32::MAX, ProcessWaitFlags::default(), &cancellation));
        assert!(block_on(poll_once(wait.as_mut())).is_none());
        if cancel {
            cancellation.cancel();
        } else {
            selector.close();
        }
        assert_eq!(
            block_on(wait).unwrap_err().kind(),
            if cancel {
                io::ErrorKind::Interrupted
            } else {
                io::ErrorKind::BrokenPipe
            }
        );
        drop(driver);
        block_on(selector.shutdown());
    }
    let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
    selector.controllers[0]
        .next_identifier
        .store(CANCEL_TAG - 2, Ordering::Relaxed);
    assert_eq!(
        block_on(selector.process_wait(
            i32::MAX,
            ProcessWaitFlags::default(),
            &Cancellation::never()
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::Other
    );
    drop(driver);
    block_on(selector.shutdown());
}

#[test]
fn unsubmitted_driver_drop_reports_an_error_and_unknown_kernel_access_is_retained() {
    for submitted in [false, true] {
        let (selector, mut driver) = IoUringSelector::new(Default::default()).unwrap();
        let never = Cancellation::never();
        let mut wait =
            Box::pin(selector.process_wait(i32::MAX, ProcessWaitFlags::default(), &never));
        assert!(block_on(poll_once(wait.as_mut())).is_none());
        if submitted {
            driver.turn(Some(Duration::ZERO)).unwrap();
            assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
        }
        drop(driver);
        if submitted {
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(wait))).is_err()
            );
        } else {
            assert_eq!(
                block_on(wait).unwrap_err().kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        block_on(selector.shutdown());
    }
}
