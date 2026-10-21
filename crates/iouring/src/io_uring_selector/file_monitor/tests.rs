// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler_selector::{SelectorDriver, SelectorWorker};
use futures_lite::future::{block_on, poll_once};
use std::{pin::Pin, task::Context, time::Instant};

fn drive<F: Future>(
    worker: &mut crate::IoUringWorker,
    context: &mut Context<'_>,
    mut future: Pin<&mut F>,
) -> F::Output {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(context) {
            return result;
        }
        assert!(Instant::now() < deadline, "file monitor test timed out");
        worker.turn(Some(Duration::ZERO)).unwrap();
    }
}

#[test]
fn repeated_operations_reuse_request_storage_without_allocations() {
    let (selector, driver) = IoUringSelector::new(IoUringOptions {
        entries: 16,
        ..Default::default()
    })
    .unwrap();
    let mut worker = driver.enter();
    let waker = worker.waker();
    let mut context = Context::from_waker(&waker);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, b"abcdef").unwrap();
    let never = Cancellation::never();
    let file = block_on(selector.file_open(&path, FileOptions::READ, &never)).unwrap();
    let mut monitor = selector.file_monitor(&file).unwrap();
    let mut buffer = Some(vec![0; 6]);
    let mut address = None;
    for iteration in 0..34 {
        let counts = allocation_counter::measure(|| {
            let mut operation = pin!(selector.file_monitor_read_at(
                &mut monitor,
                buffer.take().unwrap(),
                0,
                &never
            ));
            let (result, returned) = drive(&mut worker, &mut context, operation.as_mut());
            assert_eq!(result.unwrap(), 6);
            buffer = Some(returned);
        });
        if iteration >= 2 {
            assert_eq!(counts.count_total, 0, "ordinary monitor reuse allocated");
        }
        let current = monitor
            .registration()
            .cached
            .as_ref()
            .unwrap()
            .downcast_ref::<FileRequest<Vec<u8>>>()
            .unwrap() as *const _;
        if let Some(previous) = address {
            assert_eq!(current, previous);
        }
        address = Some(current);
    }
    let mut baseline_allocations = 0;
    for _ in 0..32 {
        let counts = allocation_counter::measure(|| {
            let mut operation =
                pin!(selector.file_read_at(&file, buffer.take().unwrap(), 0, &never));
            let (result, returned) = drive(&mut worker, &mut context, operation.as_mut());
            assert_eq!(result.unwrap(), 6);
            buffer = Some(returned);
        });
        baseline_allocations += counts.count_total;
    }
    assert_eq!(
        baseline_allocations, 64,
        "ordinary operations allocate one request and one completion channel"
    );
    assert_eq!(buffer.unwrap(), b"abcdef");
    block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
    drop(monitor);
    block_on(selector.file_close(file)).unwrap();
}

#[test]
fn abandoned_and_forgotten_operations_can_be_retired_and_reused() {
    let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
    let mut worker = driver.enter();
    let waker = worker.waker();
    let mut context = Context::from_waker(&waker);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, b"abcdef").unwrap();
    let never = Cancellation::never();
    let file = block_on(selector.file_open(&path, FileOptions::READ, &never)).unwrap();
    for forget in [false, true] {
        let mut monitor = selector.file_monitor(&file).unwrap();
        let mut operation =
            Box::pin(selector.file_monitor_read_at(&mut monitor, vec![0; 6], 0, &never));
        assert!(operation.as_mut().poll(&mut context).is_pending());
        if forget {
            std::mem::forget(operation);
        } else {
            drop(operation);
        }
        assert_eq!(
            io::Error::from(
                block_on(selector.file_monitor_read_at(&mut monitor, vec![0], 0, &never))
                    .0
                    .unwrap_err()
            )
            .kind(),
            io::ErrorKind::WouldBlock
        );
        while !monitor.registration().shared.ready.load(Ordering::Acquire) {
            worker.turn(Some(Duration::ZERO)).unwrap();
        }
        {
            let mut operation =
                pin!(selector.file_monitor_read_at(&mut monitor, vec![0; 3], 0, &never));
            let (result, buffer) = drive(&mut worker, &mut context, operation.as_mut());
            assert_eq!(result.unwrap(), 3);
            assert_eq!(buffer, b"abc");
        }
        block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
    }
    let mut monitor = selector.file_monitor(&file).unwrap();
    let mut operation =
        Box::pin(selector.file_monitor_read_at(&mut monitor, vec![0; 6], 0, &never));
    assert!(operation.as_mut().poll(&mut context).is_pending());
    std::mem::forget(operation);
    {
        let mut close = pin!(selector.file_monitor_close(&mut monitor, &never));
        drive(&mut worker, &mut context, close.as_mut()).unwrap();
    }
    drop(monitor);
    block_on(selector.file_close(file)).unwrap();
}

#[test]
fn incompatible_and_closed_selectors_reject_monitor_operations() {
    let (first, _driver) = IoUringSelector::new(Default::default()).unwrap();
    let (second, _other) = IoUringSelector::new(Default::default()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    std::fs::write(&path, b"abcdef").unwrap();
    let never = Cancellation::never();
    let file = block_on(first.file_open(&path, FileOptions::READ, &never)).unwrap();
    let mut monitor = first.file_monitor(&file).unwrap();
    let (result, _) = block_on(second.file_monitor_read_at(&mut monitor, vec![0; 3], 0, &never));
    assert_eq!(
        io::Error::from(result.unwrap_err()).kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        io::Error::from(block_on(second.file_monitor_close(&mut monitor, &never)).unwrap_err())
            .kind(),
        io::ErrorKind::InvalidInput
    );
    first.close();
    assert_eq!(
        io::Error::from(
            block_on(first.file_monitor_read_at(&mut monitor, vec![0], 0, &never))
                .0
                .unwrap_err()
        )
        .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn queued_requests_return_owners_but_unknown_kernel_retirement_never_does() {
    for submit in [false, true] {
        let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
        let mut worker = driver.enter();
        let waker = worker.waker();
        let mut context = Context::from_waker(&waker);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"abcdef").unwrap();
        let never = Cancellation::never();
        let file = block_on(selector.file_open(&path, FileOptions::READ, &never)).unwrap();
        let mut monitor = selector.file_monitor(&file).unwrap();
        let mut operation =
            Box::pin(selector.file_monitor_read_at(&mut monitor, vec![42; 6], 0, &never));
        assert!(operation.as_mut().poll(&mut context).is_pending());
        if submit {
            worker.turn(Some(Duration::ZERO)).unwrap();
        }
        drop(worker);
        if submit {
            let panic =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(operation)));
            assert!(panic.is_err());
            for _ in 0..2 {
                assert_eq!(
                    block_on(selector.file_monitor_close(&mut monitor, &never))
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::BrokenPipe
                );
            }
        } else {
            let (result, buffer) = block_on(operation);
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            assert_eq!(buffer, [42; 6]);
            block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
            drop(monitor);
            let retained = retain_file(&file);
            assert_eq!(
                Arc::strong_count(&retained),
                2,
                "queued request released its retained file before completion"
            );
            drop(retained);
            drop(file);
        }
    }
}

#[test]
fn close_and_reuse_both_report_an_abandoned_driver_failure() {
    for reuse in [false, true] {
        let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
        let mut worker = driver.enter();
        let waker = worker.waker();
        let mut context = Context::from_waker(&waker);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"abcdef").unwrap();
        let never = Cancellation::never();
        let file = block_on(selector.file_open(&path, FileOptions::READ, &never)).unwrap();
        let mut monitor = selector.file_monitor(&file).unwrap();
        let mut operation =
            Box::pin(selector.file_monitor_read_at(&mut monitor, vec![42; 6], 0, &never));
        assert!(operation.as_mut().poll(&mut context).is_pending());
        worker.turn(Some(Duration::ZERO)).unwrap();
        std::mem::forget(operation);
        drop(worker);
        if reuse {
            assert_eq!(
                block_on(selector.file_monitor_read_at(&mut monitor, vec![0], 0, &never))
                    .0
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
        }
        for _ in 0..2 {
            assert_eq!(
                block_on(selector.file_monitor_close(&mut monitor, &never))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::BrokenPipe
            );
        }
    }
}

#[test]
fn admission_cancellation_and_identifier_exhaustion_return_the_buffer() {
    let (selector, _driver) = super::super::tests::pair(1);
    let (_directory, file) = super::super::tests::file(&selector);
    let never = Cancellation::never();
    let mut monitor = selector.file_monitor(&file).unwrap();
    let controller = &selector.controllers[0];
    let permit = block_on(controller.capacity.acquire(&never)).unwrap();
    let cancellation = Cancellation::new();
    {
        let mut operation =
            pin!(selector.file_monitor_read_at(&mut monitor, vec![42], 0, &cancellation));
        assert!(block_on(poll_once(operation.as_mut())).is_none());
        cancellation.cancel();
        let (result, buffer) = block_on(operation);
        assert!(matches!(result, Err(OperationError::Cancelled(_))));
        assert_eq!(buffer, [42]);
    }
    drop(permit);
    controller
        .next_identifier
        .store(CANCEL_TAG - 1, Ordering::Relaxed);
    let (result, buffer) =
        block_on(selector.file_monitor_read_at(&mut monitor, vec![43], 0, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Other);
    assert_eq!(buffer, [43]);
    block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
}

#[test]
fn rejected_abandoned_command_is_reclaimed_before_reuse_reports_shutdown() {
    let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
    let worker = driver.enter();
    let waker = worker.waker();
    let mut context = Context::from_waker(&waker);
    let (_directory, file) = super::super::tests::file(&selector);
    let never = Cancellation::never();
    let mut monitor = selector.file_monitor(&file).unwrap();
    let mut operation = Box::pin(selector.file_monitor_read_at(&mut monitor, vec![42], 0, &never));
    assert!(operation.as_mut().poll(&mut context).is_pending());
    drop(operation);
    drop(worker);
    let (result, buffer) =
        block_on(selector.file_monitor_read_at(&mut monitor, vec![43], 0, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(buffer, [43]);
    assert!(monitor.registration().running.is_none());
    block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
}

struct ClosingBuffer {
    bytes: Vec<u8>,
    group: Arc<DriverGroup>,
}
impl AsRef<[u8]> for ClosingBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
// SAFETY: The Vec retains the same initialized allocation and range throughout ownership, without aliases.
unsafe impl IoBuf for ClosingBuffer {}
impl Drop for ClosingBuffer {
    fn drop(&mut self) {
        self.group.close();
    }
}

#[test]
fn disposal_of_an_abandoned_owner_can_close_admission_before_publication() {
    let (selector, driver) = IoUringSelector::new(Default::default()).unwrap();
    let mut worker = driver.enter();
    let waker = worker.waker();
    let mut context = Context::from_waker(&waker);
    let (_directory, file) = super::super::tests::file(&selector);
    let never = Cancellation::never();
    let mut monitor = selector.file_monitor(&file).unwrap();
    let buffer = ClosingBuffer {
        bytes: vec![42],
        group: Arc::clone(&selector.group),
    };
    let mut operation = Box::pin(selector.file_monitor_write_at(&mut monitor, buffer, 0, &never));
    assert!(operation.as_mut().poll(&mut context).is_pending());
    // Forget only the future; the queued request independently owns its buffer until retirement.
    std::mem::forget(operation);
    while !monitor.registration().shared.ready.load(Ordering::Acquire) {
        worker.turn(Some(Duration::ZERO)).unwrap();
    }
    // Changing buffer type discards the cached owner after admission but before publishing the new command.
    let (result, buffer) =
        block_on(selector.file_monitor_write_at(&mut monitor, vec![43], 0, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(buffer, [43]);
    block_on(selector.file_monitor_close(&mut monitor, &never)).unwrap();
}
