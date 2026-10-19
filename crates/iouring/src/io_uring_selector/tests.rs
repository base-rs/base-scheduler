// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use futures_lite::future::{block_on, poll_once};
use std::{sync::atomic::AtomicUsize, time::Instant};

use std::cell::RefCell;
thread_local! {
    static FAILURES: RefCell<VecDeque<(&'static str, io::Error)>> = const { RefCell::new(VecDeque::new()) };
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
            failures.pop_front().map(|(_, error)| error)
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
        FAILURES.with(|state| *state.borrow_mut() = failures.into_iter().collect());
        Self
    }
}
impl Drop for Failures {
    fn drop(&mut self) {
        FAILURES.with(|failures| failures.borrow_mut().clear());
    }
}

fn pair(capacity: usize) -> (IoUringSelector, IoUringDriver) {
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
fn drain(selector: &IoUringSelector, driver: &mut IoUringDriver) {
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
    assert!(validate_completion_support(true).is_ok());
    assert_eq!(
        validate_completion_support(false).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert!(validate_operation_support(|_| true).is_ok());
    for missing in [
        opcode::Read::CODE,
        opcode::Write::CODE,
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
    for name in [
        "ring",
        "probe",
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
    assert_eq!(selector.controller.capacity.state.lock().unwrap().0, 0);
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let (result, buffer) = block_on(selector.file_read(&file, vec![7], &cancelled));
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffer, [7]);
    assert_eq!(
        block_on(selector.file_read_at(&file, Vec::new(), 0, &never))
            .0
            .unwrap(),
        0
    );
    let (result, buffer) = block_on(selector.file_write_at(&file, vec![9], u64::MAX, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(buffer, [9]);
    selector
        .controller
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
    assert_eq!(selector.controller.capacity.state.lock().unwrap().0, 0);
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
    assert_eq!(selector.controller.capacity.state.lock().unwrap().0, 1);
    drain(&selector, &mut driver);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(selector.controller.capacity.state.lock().unwrap().0, 0);
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
    while selector.controller.capacity.state.lock().unwrap().0 != 0 {
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
