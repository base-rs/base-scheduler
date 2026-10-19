// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use futures_lite::future::{block_on, poll_once};
use std::{sync::atomic::AtomicUsize, time::Instant};

use std::cell::RefCell;
thread_local! {
    static FAILURES: RefCell<VecDeque<(&'static str, Option<io::Error>)>> = const { RefCell::new(VecDeque::new()) };
}

pub(super) fn syscall<T>(
    name: &'static str,
    operation: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let failure = FAILURES.with(|failures| {
        let mut failures = failures.borrow_mut();
        if failures
            .front()
            .is_some_and(|(expected, _)| *expected == name)
        {
            failures.pop_front().and_then(|(_, error)| error)
        } else {
            None
        }
    });
    match failure {
        Some(error) => Err(error),
        None => operation(),
    }
}

struct Failures;
impl Failures {
    fn new(failures: impl IntoIterator<Item = (&'static str, io::Error)>) -> Self {
        FAILURES.with(|state| {
            *state.borrow_mut() = failures
                .into_iter()
                .map(|(name, error)| (name, Some(error)))
                .collect()
        });
        Self
    }
}
impl Drop for Failures {
    fn drop(&mut self) {
        FAILURES.with(|failures| failures.borrow_mut().clear());
    }
}

pub(super) fn pair(capacity: usize) -> (IoUringSelector, IoUringDriver) {
    IoUringSelector::new(IoUringOptions {
        capacity,
        ..Default::default()
    })
    .unwrap()
}
fn file(selector: &IoUringSelector) -> (tempfile::TempDir, NativeFile) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("file"), b"abcdef").unwrap();
    let file = block_on(selector.file_open(
        &directory.path().join("file"),
        FileOptions {
            write: true,
            ..FileOptions::READ
        },
        &Cancellation::never(),
    ))
    .unwrap();
    (directory, file)
}
fn fifo(selector: &IoUringSelector) -> (tempfile::TempDir, NativeFile) {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fifo");
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: The path is a valid terminated string and the fixture owns its parent directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    // Opening both ends avoids blocking construction; a read waits for actual bytes.
    let file = block_on(selector.file_open(
        &path,
        FileOptions {
            write: true,
            ..FileOptions::READ
        },
        &Cancellation::never(),
    ))
    .unwrap();
    (directory, file)
}
pub(super) fn drain(selector: &IoUringSelector, driver: &mut IoUringDriver) {
    selector.close();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !driver.turn(Some(Duration::from_millis(10))).unwrap() {
        assert!(
            Instant::now() < deadline,
            "driver did not retire accepted requests"
        );
    }
    block_on(selector.shutdown());
}

#[test]
fn setup_probes_and_errors_are_explicit() {
    assert!(validate_completion_support(true, true).is_ok());
    assert_eq!(
        validate_completion_support(false, true).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert_eq!(
        validate_completion_support(true, false).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert!(validate_operation_support(|_| true).is_ok());
    for missing in [
        opcode::Read::CODE,
        opcode::Write::CODE,
        opcode::Recv::CODE,
        opcode::Send::CODE,
        opcode::RecvMsg::CODE,
        opcode::SendMsg::CODE,
        opcode::AsyncCancel::CODE,
    ] {
        assert_eq!(
            validate_operation_support(|code| code != missing)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
    }
    for options in [
        IoUringOptions {
            entries: 0,
            ..Default::default()
        },
        IoUringOptions {
            capacity: 0,
            ..Default::default()
        },
        IoUringOptions {
            capacity: CAPACITY_CLOSED,
            ..Default::default()
        },
        IoUringOptions {
            fallback: core_scheduler_selector::SelectorOptions {
                threads: 0,
                capacity: 1,
            },
            ..Default::default()
        },
    ] {
        assert_eq!(
            IoUringSelector::new(options).err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
    }
    assert_eq!(
        IoUringSelector::with_workers(IoUringOptions::default(), 0)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    for name in [
        "ring",
        "probe",
        "completion-support",
        "operation-support",
        "notification-pair",
        "notification-mode",
        "receiver-mode",
        "poller",
        "add-notification",
        "add-ring",
    ] {
        let _failures = Failures::new([(name, io::ErrorKind::PermissionDenied.into())]);
        assert_eq!(
            IoUringSelector::new(IoUringOptions::default())
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::PermissionDenied,
            "{name}"
        );
    }
}

#[test]
fn lazy_precancelled_empty_invalid_and_closed_operations_preserve_the_buffer() {
    let (selector, mut driver) = pair(1);
    let (_directory, file) = file(&selector);
    let never = Cancellation::never();
    drop(selector.file_read(&file, vec![0; 1], &never));
    assert_eq!(
        selector.controllers[0]
            .capacity
            .state
            .load(Ordering::Acquire)
            & !CAPACITY_CLOSED,
        0
    );
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let (result, buffer) = block_on(selector.file_read(&file, vec![7], &cancelled));
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffer, [7]);
    let (result, buffer) = block_on(selector.file_write_at(&file, vec![9], u64::MAX, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(buffer, [9]);
    selector.controllers[0]
        .next_identifier
        .store(CANCEL_TAG - 1, Ordering::Relaxed);
    assert_eq!(
        block_on(selector.file_read(&file, vec![0], &never))
            .0
            .unwrap_err()
            .kind(),
        io::ErrorKind::Other
    );
    drain(&selector, &mut driver);
    assert_eq!(
        block_on(selector.file_read(&file, vec![0], &never))
            .0
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(matches!(
        block_on(selector.file_read(&file, vec![0], &cancelled)).0,
        Err(OperationError::Cancelled(_))
    ));
    assert!(driver.turn(None).unwrap());
}

#[test]
fn bounded_admission_wakes_for_cancellation_and_closure() {
    let (selector, mut driver) = pair(1);
    let (_directory, file) = fifo(&selector);
    let never = Cancellation::never();
    let mut first = Box::pin(selector.file_read(&file, vec![0], &never));
    assert!(block_on(poll_once(&mut first)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    let token = Cancellation::new();
    let mut second = Box::pin(selector.file_read(&file, vec![5], &token));
    assert!(block_on(poll_once(&mut second)).is_none());
    token.cancel();
    let (result, buffer) = block_on(second);
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffer, [5]);
    let mut third = Box::pin(selector.file_read(&file, vec![7], &never));
    assert!(block_on(poll_once(&mut third)).is_none());
    selector.close();
    assert_eq!(
        block_on(third).0.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    drain(&selector, &mut driver);
    assert!(block_on(first).0.is_err());
    assert_eq!(
        selector.controllers[0]
            .capacity
            .state
            .load(Ordering::Acquire)
            & !CAPACITY_CLOSED,
        0
    );
}

struct ObservedBuffer {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}
impl AsRef<[u8]> for ObservedBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsMut<[u8]> for ObservedBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}
// SAFETY: Exclusive ownership retains one stable initialized allocation across moves.
unsafe impl IoBuf for ObservedBuffer {}
// SAFETY: Writable access is exclusive and never changes the allocation or initialized range.
unsafe impl IoBufMut for ObservedBuffer {}
impl Drop for ObservedBuffer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn abandoning_submitted_io_keeps_storage_until_original_retirement() {
    let (selector, mut driver) = pair(1);
    let (_directory, file) = fifo(&selector);
    let drops = Arc::new(AtomicUsize::new(0));
    let buffer = ObservedBuffer {
        bytes: vec![0],
        drops: Arc::clone(&drops),
    };
    let never = Cancellation::never();
    let mut operation = Box::pin(selector.file_read(&file, buffer, &never));
    assert!(block_on(poll_once(&mut operation)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
    drop(operation);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(
        selector.controllers[0]
            .capacity
            .state
            .load(Ordering::Acquire)
            & !CAPACITY_CLOSED,
        1
    );
    drain(&selector, &mut driver);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        selector.controllers[0]
            .capacity
            .state
            .load(Ordering::Acquire)
            & !CAPACITY_CLOSED,
        0
    );
}

#[test]
fn cooperative_cancellation_waits_for_original_completion_and_returns_the_owner() {
    let (selector, mut driver) = pair(1);
    let (_directory, file) = fifo(&selector);
    let cancellation = Cancellation::new();
    let buffer = vec![3];
    let pointer = buffer.as_ptr();
    let mut operation = Box::pin(selector.file_read(&file, buffer, &cancellation));
    assert!(block_on(poll_once(&mut operation)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    cancellation.cancel();
    assert!(block_on(poll_once(&mut operation)).is_none());
    let deadline = Instant::now() + Duration::from_secs(10);
    let (result, buffer) = loop {
        driver.turn(Some(Duration::from_millis(10))).unwrap();
        if let Some(result) = block_on(poll_once(&mut operation)) {
            break result;
        }
        assert!(Instant::now() < deadline);
    };
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(pointer, buffer.as_ptr());
    drain(&selector, &mut driver);
}

#[test]
fn dropping_an_undriven_driver_returns_queued_buffers_and_closes_fallback() {
    let (selector, driver) = pair(1);
    let (_directory, file) = file(&selector);
    let never = Cancellation::never();
    let mut operation = Box::pin(selector.file_read(&file, vec![3], &never));
    assert!(block_on(poll_once(&mut operation)).is_none());
    drop(driver);
    let (result, buffer) = block_on(operation);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(buffer, [3]);
    assert_eq!(
        block_on(selector.file_seek(&file, std::io::SeekFrom::Start(0), &never))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    block_on(selector.shutdown());
}

#[test]
fn transient_syscall_errors_preserve_progress_and_fatal_errors_stop_the_driver() {
    for (name, error) in [
        ("submit", io::ErrorKind::Interrupted.into()),
        ("submit", io::Error::from_raw_os_error(libc::EBUSY)),
        ("submit", io::Error::from_raw_os_error(libc::EAGAIN)),
        ("notification-read", io::ErrorKind::Interrupted.into()),
        ("wait", io::ErrorKind::Interrupted.into()),
    ] {
        let (selector, mut driver) = pair(1);
        let _failures = Failures::new([(name, error)]);
        assert!(!driver.turn(Some(Duration::ZERO)).unwrap());
        drain(&selector, &mut driver);
    }
    for name in [
        "submit",
        "notification-read",
        "modify-notification",
        "modify-ring",
        "wait",
    ] {
        let (selector, mut driver) = pair(1);
        let _failures = Failures::new([(name, io::ErrorKind::PermissionDenied.into())]);
        assert_eq!(
            driver.turn(Some(Duration::ZERO)).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(driver.turn(None).unwrap());
        block_on(selector.shutdown());
    }
}

#[test]
fn event_loop_wakers_and_notification_errors_do_not_lose_work() {
    let (selector, mut driver) = pair(1);
    let waker = driver.waker();
    waker.wake_by_ref();
    assert!(!driver.turn(None).unwrap());
    #[allow(clippy::waker_clone_wake)]
    waker.clone().wake();
    assert!(!driver.turn(None).unwrap());
    {
        let _failures = Failures::new([("notify", io::ErrorKind::Interrupted.into())]);
        waker.wake_by_ref();
    }
    assert!(!driver.turn(None).unwrap());
    // A full notification pipe already holds a wakeup.
    waker.wake_by_ref();
    {
        let _failures = Failures::new([("notify", io::ErrorKind::WouldBlock.into())]);
        waker.wake_by_ref();
    }
    assert!(!driver.turn(None).unwrap());
    drain(&selector, &mut driver);
    waker.wake_by_ref();
}

#[test]
fn successful_completion_wins_cancellation_and_releases_the_file_before_waking() {
    let (selector, mut driver) = pair(1);
    let (_directory, file) = file(&selector);
    let token = Cancellation::new();
    let mut operation = Box::pin(selector.file_read_at(&file, vec![0; 6], 0, &token));
    assert!(block_on(poll_once(&mut operation)).is_none());
    let deadline = Instant::now() + Duration::from_secs(10);
    while selector.controllers[0]
        .capacity
        .state
        .load(Ordering::Acquire)
        & !CAPACITY_CLOSED
        != 0
    {
        driver.turn(Some(Duration::from_millis(10))).unwrap();
        assert!(Instant::now() < deadline);
    }
    token.cancel();
    let (result, buffer) = block_on(operation);
    assert_eq!(result.unwrap(), 6);
    assert_eq!(buffer, b"abcdef");
    block_on(selector.file_close(file)).unwrap();
    drain(&selector, &mut driver);
}

pub(super) fn drive<F: Future>(driver: &mut IoUringDriver, future: F) -> F::Output {
    let mut future = pin!(future);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(output) = block_on(poll_once(&mut future)) {
            return output;
        }
        driver.turn(Some(Duration::from_millis(10))).unwrap();
        assert!(Instant::now() < deadline);
    }
}

#[test]
fn empty_operations_keep_native_access_mode_errors() {
    let (selector, mut driver) = pair(2);
    let (directory, file) = file(&selector);
    let never = Cancellation::never();
    assert_eq!(
        drive(
            &mut driver,
            selector.file_read_at(&file, Vec::new(), 0, &never)
        )
        .0
        .unwrap(),
        0
    );
    assert_eq!(
        drive(&mut driver, selector.file_write(&file, Vec::new(), &never))
            .0
            .unwrap(),
        0
    );
    let read_only =
        block_on(selector.file_open(&directory.path().join("file"), FileOptions::READ, &never))
            .unwrap();
    let write_only = block_on(selector.file_open(
        &directory.path().join("file"),
        FileOptions {
            read: false,
            write: true,
            ..Default::default()
        },
        &never,
    ))
    .unwrap();
    assert_eq!(
        drive(
            &mut driver,
            selector.file_write(&read_only, Vec::new(), &never)
        )
        .0
        .unwrap_err()
        .kind(),
        io::Error::from_raw_os_error(libc::EBADF).kind()
    );
    assert_eq!(
        drive(
            &mut driver,
            selector.file_read(&write_only, Vec::new(), &never)
        )
        .0
        .unwrap_err()
        .kind(),
        io::Error::from_raw_os_error(libc::EBADF).kind()
    );
    drain(&selector, &mut driver);
}

#[test]
fn driver_failure_retains_submitted_storage_and_wakes_waiters_with_a_panic() {
    for fatal_turn in [false, true] {
        let (selector, mut driver) = pair(1);
        let (_directory, file) = fifo(&selector);
        let drops = Arc::new(AtomicUsize::new(0));
        let never = Cancellation::never();
        let mut operation = Box::pin(selector.file_read(
            &file,
            ObservedBuffer {
                bytes: vec![0],
                drops: Arc::clone(&drops),
            },
            &never,
        ));
        assert!(block_on(poll_once(&mut operation)).is_none());
        driver.turn(Some(Duration::ZERO)).unwrap();
        assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
        if fatal_turn {
            let _failures = Failures::new([("wait", io::ErrorKind::PermissionDenied.into())]);
            assert_eq!(
                driver.turn(Some(Duration::ZERO)).unwrap_err().kind(),
                io::ErrorKind::PermissionDenied
            );
        }
        drop(driver);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(operation))).is_err()
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            selector.controllers[0]
                .capacity
                .state
                .load(Ordering::Acquire)
                & !CAPACITY_CLOSED,
            1
        );
        // SAFETY: Observe the retained owner count without using its descriptor, and release this temporary alias immediately.
        assert_eq!(Arc::strong_count(&unsafe { file.retain_native() }), 3);
        block_on(selector.shutdown());
    }
}

#[test]
fn abandoned_buffer_destructor_panics_do_not_stop_other_operations() {
    struct PanicsOnDrop(Vec<u8>, Arc<AtomicUsize>);
    impl AsRef<[u8]> for PanicsOnDrop {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }
    impl AsMut<[u8]> for PanicsOnDrop {
        fn as_mut(&mut self) -> &mut [u8] {
            &mut self.0
        }
    }
    // SAFETY: The wrapper retains stable initialized heap storage across moves.
    unsafe impl IoBuf for PanicsOnDrop {}
    // SAFETY: The wrapper exclusively owns its writable storage.
    unsafe impl IoBufMut for PanicsOnDrop {}
    impl Drop for PanicsOnDrop {
        fn drop(&mut self) {
            self.1.fetch_add(1, Ordering::SeqCst);
            panic!("abandoned buffer destructor");
        }
    }
    for submitted in [false, true] {
        let (selector, mut driver) = pair(2);
        let (_directory, file) = file(&selector);
        let never = Cancellation::never();
        let drops = Arc::new(AtomicUsize::new(0));
        let mut operation =
            Box::pin(selector.file_read(&file, PanicsOnDrop(vec![0], Arc::clone(&drops)), &never));
        assert!(block_on(poll_once(&mut operation)).is_none());
        if submitted {
            driver.turn(Some(Duration::ZERO)).unwrap();
        }
        drop(operation);
        let (result, buffer) = drive(
            &mut driver,
            selector.file_read_at(&file, vec![0; 6], 0, &never),
        );
        assert_eq!(result.unwrap(), 6);
        assert_eq!(buffer, b"abcdef");
        drain(&selector, &mut driver);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn close_returns_pending_owners_and_cancellation_tolerates_retired_requests() {
    let (selector, mut driver) = pair(2);
    let (_directory, file) = file(&selector);
    let never = Cancellation::never();
    let drops = Arc::new(AtomicUsize::new(0));
    let mut operation = Box::pin(selector.file_read_at(
        &file,
        ObservedBuffer {
            bytes: vec![0],
            drops: Arc::clone(&drops),
        },
        0,
        &never,
    ));
    assert!(block_on(poll_once(&mut operation)).is_none());
    let deadline = Instant::now() + Duration::from_secs(10);
    while selector.controllers[0]
        .capacity
        .state
        .load(Ordering::Acquire)
        & !CAPACITY_CLOSED
        != 0
    {
        driver.turn(Some(Duration::from_millis(10))).unwrap();
        assert!(Instant::now() < deadline);
    }
    // Completion is stored, but the future has not observed it. Abandonment sends a now-stale cancellation which must be harmless.
    drop(operation);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    driver.turn(Some(Duration::ZERO)).unwrap();
    let mut queued = Box::pin(selector.file_read(&file, vec![5], &never));
    assert!(block_on(poll_once(&mut queued)).is_none());
    selector.close();
    assert!(driver.turn(None).unwrap());
    let (result, buffer) = block_on(queued);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(buffer, [5]);
    block_on(selector.shutdown());
}

#[test]
fn full_submission_queue_defers_cancellations_and_shutdown_tolerates_duplicate_requests() {
    let (selector, mut driver) = IoUringSelector::new(IoUringOptions {
        entries: 1,
        capacity: 4,
        ..Default::default()
    })
    .unwrap();
    let (_directory, file) = fifo(&selector);
    let never = Cancellation::never();
    let mut operations = Vec::new();
    for _ in 0..4 {
        let mut operation = Box::pin(selector.file_read(&file, vec![0], &never));
        assert!(block_on(poll_once(&mut operation)).is_none());
        driver.turn(Some(Duration::ZERO)).unwrap();
        operations.push(operation);
    }
    assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 4);
    selector.close();
    // Shutdown marks each request as cancelling; dropping each future also sends its own cancellation to the same owner.
    drop(operations);
    drain(&selector, &mut driver);
    assert_eq!(
        selector.controllers[0]
            .capacity
            .state
            .load(Ordering::Acquire)
            & !CAPACITY_CLOSED,
        0
    );
}

#[test]
fn submission_racing_shutdown_or_driver_exit_returns_unpublished_storage() {
    for driver_exited in [false, true] {
        let (selector, driver) = pair(1);
        let (_directory, file) = file(&selector);
        let never = Cancellation::never();
        // Stage the legitimate interval after admission and request preparation but before publication, so shutdown can deterministically win that race.
        let permit = block_on(selector.controllers[0].capacity.acquire(&never)).unwrap();
        let (reply, result) = oneshot::channel();
        let request = Box::new(Request {
            identifier: IoUringSelector::next_identifier(&selector.controllers[0]).unwrap(),
            _permit: permit,
            resource: Some(Resource::File(retain_file(&file), None)),
            buffer: Some(vec![3]),
            pointer: |buffer| buffer.as_mut_ptr(),
            write: false,
            reply: Cell::new(Some(reply)),
            cancelling: Cell::new(false),
        });
        selector.close();
        if driver_exited {
            drop(driver);
            let (result, buffer) = block_on(IoUringSelector::submit(
                Arc::clone(&selector.controllers[0]),
                request,
                result,
                &never,
            ));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(buffer, [3]);
        } else {
            let mut driver = driver;
            let (result, buffer) = block_on(IoUringSelector::submit(
                Arc::clone(&selector.controllers[0]),
                request,
                result,
                &never,
            ));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(buffer, [3]);
            drain(&selector, &mut driver);
        }
        assert_eq!(
            selector.controllers[0]
                .capacity
                .state
                .load(Ordering::Acquire)
                & !CAPACITY_CLOSED,
            0
        );
        block_on(selector.shutdown());
    }
}

#[test]
fn convenience_driver_loop_reports_the_original_failure() {
    let (selector, driver) = pair(1);
    let _failures = Failures::new([("submit", io::ErrorKind::PermissionDenied.into())]);
    assert_eq!(
        driver.run().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    block_on(selector.shutdown());
}

fn active(controller: &Controller) -> usize {
    controller.capacity.state.load(Ordering::Acquire) & !CAPACITY_CLOSED
}

#[test]
fn workers_route_locally_and_external_operations_share_ring_capacity_fairly() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    let (selector, drivers) = IoUringSelector::with_workers(
        IoUringOptions {
            capacity: 1,
            ..Default::default()
        },
        2,
    )
    .unwrap();
    let (_directory, file) = file(&selector);
    let never = Cancellation::never();
    let mut first = Box::pin(selector.file_read_at(&file, vec![0], 0, &never));
    let mut second = Box::pin(selector.file_read_at(&file, vec![0], 1, &never));
    assert!(block_on(poll_once(&mut first)).is_none());
    assert!(block_on(poll_once(&mut second)).is_none());
    assert_eq!(active(&selector.controllers[0]), 1);
    assert_eq!(active(&selector.controllers[1]), 1);
    let mut workers: Vec<_> = drivers.into_iter().map(SelectorDriver::enter).collect();
    let deadline = Instant::now() + Duration::from_secs(10);
    while selector
        .controllers
        .iter()
        .any(|controller| active(controller) != 0)
    {
        for worker in &mut workers {
            worker.turn(Some(Duration::ZERO)).unwrap();
        }
        assert!(Instant::now() < deadline);
    }
    assert_eq!(block_on(first).1, b"a");
    assert_eq!(block_on(second).1, b"b");
    assert_eq!(selector.external_next.load(Ordering::Relaxed), 2);
    // The latest entered worker belongs to ring one. Local submission must neither advance external routing nor write the notification pipe.
    let _failures = Failures::new([("notify", io::ErrorKind::PermissionDenied.into())]);
    let mut local = Box::pin(selector.file_read_at(&file, vec![0], 2, &never));
    assert!(block_on(poll_once(&mut local)).is_none());
    assert_eq!(FAILURES.with(|failures| failures.borrow().len()), 1);
    assert_eq!(selector.external_next.load(Ordering::Relaxed), 2);
    assert_eq!(active(&selector.controllers[0]), 0);
    assert_eq!(active(&selector.controllers[1]), 1);
    while active(&selector.controllers[1]) != 0 {
        workers[1].turn(Some(Duration::from_millis(10))).unwrap();
    }
    assert_eq!(block_on(local).1, b"c");
    selector.close();
    for worker in &mut workers {
        while !worker.turn(Some(Duration::from_millis(10))).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn moving_a_waiter_to_another_thread_keeps_cancellation_on_the_original_ring() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    let (selector, mut drivers) =
        IoUringSelector::with_workers(IoUringOptions::default(), 2).unwrap();
    let (_directory, file) = fifo(&selector);
    let second = drivers.pop().unwrap();
    let mut first = drivers.pop().unwrap().enter();
    let cancellation = Cancellation::new();
    let mut operation = Box::pin(selector.file_read(&file, vec![7], &cancellation));
    assert!(block_on(poll_once(&mut operation)).is_none());
    first.turn(Some(Duration::ZERO)).unwrap();
    let cancelled = &cancellation;
    let operation = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                cancelled.cancel();
                assert!(block_on(poll_once(&mut operation)).is_none());
                operation
            })
            .join()
            .unwrap()
    });
    assert_eq!(active(&selector.controllers[0]), 1);
    assert_eq!(
        selector.controllers[1]
            .next_identifier
            .load(Ordering::Relaxed),
        0
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while active(&selector.controllers[0]) != 0 {
        first.turn(Some(Duration::from_millis(10))).unwrap();
        assert!(Instant::now() < deadline);
    }
    let (result, buffer) = block_on(operation);
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffer, [7]);
    let mut second = second.enter();
    first.close();
    while !first.turn(Some(Duration::from_millis(10))).unwrap() {}
    while !second.turn(Some(Duration::from_millis(10))).unwrap() {}
    block_on(selector.shutdown());
}

#[test]
fn nested_selectors_restore_routes_and_never_use_another_selectors_ring() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    let (outer, driver) = pair(2);
    let (_directory, file) = file(&outer);
    let mut outer_worker = driver.enter();
    let (inner, driver) = pair(2);
    let inner_worker = driver.enter();
    assert!(Arc::ptr_eq(&outer.route(), &outer.controllers[0]));
    assert!(Arc::ptr_eq(&inner.route(), &inner.controllers[0]));
    assert_eq!(outer.external_next.load(Ordering::Relaxed), 0);
    assert_eq!(inner.external_next.load(Ordering::Relaxed), 0);
    // Dropping the nested worker restores the outer route and closes only its own selector.
    drop(inner_worker);
    block_on(inner.shutdown());
    let never = Cancellation::never();
    let mut operation = Box::pin(outer.file_read_at(&file, vec![0], 0, &never));
    assert!(block_on(poll_once(&mut operation)).is_none());
    while active(&outer.controllers[0]) != 0 {
        outer_worker.turn(Some(Duration::from_millis(10))).unwrap();
    }
    assert_eq!(block_on(operation).1, b"a");
    outer_worker.close();
    while !outer_worker.turn(Some(Duration::from_millis(10))).unwrap() {}
    block_on(outer.shutdown());
}

#[test]
fn dropping_local_driver_returns_queued_owners_before_shutdown_and_allows_reentrant_wakes() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    use std::task::{Context, Wake, Waker};
    struct Reenter(Arc<Controller>, AtomicUsize);
    impl Wake for Reenter {
        fn wake(self: Arc<Self>) {
            self.1.fetch_add(1, Ordering::SeqCst);
            let _ = self.0.send(Command::Cancel(0));
        }
    }
    let (selector, driver) = pair(1);
    let (_directory, file) = file(&selector);
    let worker = driver.enter();
    worker.waker().wake_by_ref();
    let never = Cancellation::never();
    let mut operation = Box::pin(selector.file_read(&file, vec![9], &never));
    let reenter = Arc::new(Reenter(
        Arc::clone(&selector.controllers[0]),
        AtomicUsize::new(0),
    ));
    let waker = Waker::from(Arc::clone(&reenter));
    assert!(
        operation
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    drop(worker);
    assert!(
        selector.controllers[0]
            .shutdown
            .finished
            .load(Ordering::Acquire)
    );
    assert_eq!(active(&selector.controllers[0]), 0);
    assert_eq!(reenter.1.load(Ordering::SeqCst), 1);
    let (result, buffer) = block_on(operation);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(buffer, [9]);
    block_on(selector.shutdown());
}

#[test]
fn closing_a_full_local_batch_drains_all_accepted_owners() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    for remote_close in [false, true] {
        let (selector, driver) = pair(300);
        let (_directory, file) = fifo(&selector);
        let mut worker = driver.enter();
        let never = Cancellation::never();
        let mut operations = Vec::new();
        for _ in 0..300 {
            let mut operation = Box::pin(selector.file_read(&file, vec![7], &never));
            assert!(block_on(poll_once(&mut operation)).is_none());
            operations.push(operation);
        }
        if remote_close {
            std::thread::scope(|scope| scope.spawn(|| selector.close()).join().unwrap());
        } else {
            selector.close();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while !worker.turn(Some(Duration::from_millis(10))).unwrap() {
            assert!(Instant::now() < deadline);
        }
        assert_eq!(active(&selector.controllers[0]), 0);
        for operation in operations {
            assert!(block_on(operation).0.is_err());
        }
        block_on(selector.shutdown());
    }
}

#[test]
fn stopping_one_ring_closes_the_group_and_shared_fallback() {
    let (selector, mut drivers) =
        IoUringSelector::with_workers(IoUringOptions::default(), 2).unwrap();
    drop(drivers.pop());
    assert!(
        selector
            .controllers
            .iter()
            .all(|controller| controller.closed.load(Ordering::Acquire))
    );
    let mut driver = drivers.pop().unwrap();
    assert!(driver.turn(None).unwrap());
    block_on(selector.shutdown());
    assert!(matches!(
        block_on(selector.spawn_blocking(|| 42, &Cancellation::never())),
        Err(BlockingError::SchedulerClosed)
    ));
}

#[test]
fn migrated_cancellation_cannot_overtake_a_queued_submission() {
    use core_scheduler_selector::{SelectorDriver, SelectorWorker};
    for submit_locally in [false, true] {
        let (selector, driver) = pair(4);
        let (_directory, file) = fifo(&selector);
        let never = Cancellation::never();
        // In either direction place another submission ahead of the target, so alternating queues observes the target cancellation first.
        let mut ahead = Box::pin(selector.file_read(&file, vec![0], &never));
        let mut target = Box::pin(selector.file_read(&file, vec![1], &never));
        let mut worker = if submit_locally {
            let worker = driver.enter();
            assert!(block_on(poll_once(&mut ahead)).is_none());
            assert!(block_on(poll_once(&mut target)).is_none());
            std::thread::scope(|scope| scope.spawn(move || drop(target)).join().unwrap());
            worker
        } else {
            assert!(block_on(poll_once(&mut ahead)).is_none());
            assert!(block_on(poll_once(&mut target)).is_none());
            let worker = driver.enter();
            drop(target);
            worker
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while active(&selector.controllers[0]) != 1 {
            worker.turn(Some(Duration::from_millis(10))).unwrap();
            assert!(
                Instant::now() < deadline,
                "cancellation overtook submission"
            );
        }
        // Only the unrelated read remains; cancelling it must allow ordinary shutdown.
        drop(ahead);
        worker.close();
        while !worker.turn(Some(Duration::from_millis(10))).unwrap() {
            assert!(Instant::now() < deadline);
        }
        block_on(selector.shutdown());
    }
}

#[test]
fn native_workers_use_distinct_rings_without_external_submission_routing() {
    use core_scheduler_native::{NativeExecutorOptions, NativeScheduler};
    let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), 2).unwrap();
    let (_directory, file) = file(&selector);
    let file = Arc::new(file);
    let scheduler = Arc::new(
        NativeScheduler::with_drivers(
            selector,
            drivers,
            NativeExecutorOptions {
                workers: 2,
                timer_capacity: 8,
            },
        )
        .unwrap(),
    );
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let scheduler2 = Arc::clone(&scheduler);
        let file = Arc::clone(&file);
        let barrier = Arc::clone(&barrier);
        tasks.push(
            scheduler
                .spawn(async move {
                    // Both native threads must be polling before either issues its first local operation.
                    barrier.wait();
                    let never = Cancellation::never();
                    for offset in 0..6 {
                        let (result, buffer) = scheduler2
                            .file_read_at(&file, vec![0], offset, &never)
                            .await;
                        assert_eq!(result.unwrap(), 1);
                        assert_eq!(buffer[0], b'a' + offset as u8);
                    }
                })
                .unwrap(),
        );
    }
    for task in tasks {
        block_on(task).unwrap();
    }
    assert_eq!(
        scheduler.selector().external_next.load(Ordering::Relaxed),
        0
    );
    assert!(
        scheduler
            .selector()
            .controllers
            .iter()
            .all(|controller| controller.next_identifier.load(Ordering::Relaxed) > 0)
    );
    block_on(scheduler.shutdown());
}

#[test]
fn partial_ring_construction_releases_previously_created_drivers() {
    let _failures = Failures;
    FAILURES.with(|failures| {
        *failures.borrow_mut() = VecDeque::from([
            ("ring", None),
            ("ring", Some(io::ErrorKind::PermissionDenied.into())),
        ])
    });
    assert_eq!(
        IoUringSelector::with_workers(IoUringOptions::default(), 3)
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(FAILURES.with(|failures| failures.borrow().is_empty()));
}

#[test]
fn external_admission_remains_bounded_under_contention() {
    let capacity = Arc::new(Capacity {
        state: AtomicUsize::new(0),
        available: CompletionEvent::new(),
        limit: 4,
    });
    std::thread::scope(|scope| {
        let barrier = Arc::new(std::sync::Barrier::new(8));
        for _ in 0..8 {
            let capacity = Arc::clone(&capacity);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..500 {
                    let permit = block_on(capacity.acquire(&Cancellation::never())).unwrap();
                    assert!(capacity.state.load(Ordering::Acquire) <= capacity.limit);
                    std::thread::yield_now();
                    drop(permit);
                }
            });
        }
    });
    assert_eq!(capacity.state.load(Ordering::Acquire), 0);
    capacity.close();
}
