// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{Cancellation, FileMonitoring, FileSeek};
use std::{
    future::{Future, poll_fn},
    io::SeekFrom,
    task::Poll,
};

pub async fn sequential<S: FileMonitoring + FileSeek>(scheduler: &S, file: &S::File) {
    let never = Cancellation::never();
    let mut monitor = scheduler.file_monitor(file).unwrap();
    let mut buffer = vec![0; 2];
    for _ in 0..3 {
        let result;
        (result, buffer) = scheduler
            .file_monitor_read_at(&mut monitor, buffer, 1, &never)
            .await;
        assert_eq!(result.unwrap(), 2);
        assert_eq!(buffer, b"bc");
    }
    // One record supports changing direction and owner type, not only repeating the same opcode.
    let (result, buffer) = scheduler
        .file_monitor_write_at(&mut monitor, Box::<[u8]>::from(&b"XY"[..]), 2, &never)
        .await;
    assert_eq!(result.unwrap(), 2);
    assert_eq!(&*buffer, b"XY");
    let (result, buffer) = scheduler
        .file_monitor_read_at(&mut monitor, vec![0; 8], 0, &never)
        .await;
    assert_eq!(result.unwrap(), 6);
    assert_eq!(&buffer[..6], b"abXYef");
    assert_eq!(
        scheduler
            .file_monitor_read_at(&mut monitor, buffer, 6, &never)
            .await
            .0
            .unwrap(),
        0
    );
    scheduler
        .file_seek(file, SeekFrom::Start(0), &never)
        .await
        .unwrap();
    assert_eq!(
        scheduler
            .file_monitor_read(&mut monitor, vec![0; 2], &never)
            .await
            .1,
        b"ab"
    );
    assert_eq!(
        scheduler
            .file_monitor_write(&mut monitor, b"12".to_vec(), &never)
            .await
            .0
            .unwrap(),
        2
    );
    assert_eq!(
        scheduler
            .file_monitor_read(&mut monitor, vec![0; 2], &never)
            .await
            .1,
        b"ef"
    );
    scheduler
        .file_monitor_close(&mut monitor, &never)
        .await
        .unwrap();
    scheduler
        .file_monitor_close(&mut monitor, &never)
        .await
        .unwrap();
    assert!(
        scheduler
            .file_monitor_read(&mut monitor, vec![0; 2], &never)
            .await
            .0
            .is_err()
    );
}

pub async fn independent<S: FileMonitoring>(scheduler: &S, file: &S::File) {
    let never = Cancellation::never();
    let mut first = scheduler.file_monitor(file).unwrap();
    let mut second = scheduler.file_monitor(file).unwrap();
    let (a, b) = futures_lite::future::zip(
        scheduler.file_monitor_read_at(&mut first, vec![0; 3], 0, &never),
        scheduler.file_monitor_read_at(&mut second, vec![0; 3], 3, &never),
    )
    .await;
    assert_eq!(a.0.unwrap(), 3);
    assert_eq!(a.1, b"abc");
    assert_eq!(b.0.unwrap(), 3);
    assert_eq!(b.1, b"def");
    scheduler
        .file_monitor_close(&mut first, &never)
        .await
        .unwrap();
    scheduler
        .file_monitor_close(&mut second, &never)
        .await
        .unwrap();
}

pub async fn cancellation<S: FileMonitoring>(scheduler: &S, file: &S::File) {
    let mut monitor = scheduler.file_monitor(file).unwrap();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let never = Cancellation::never();
    for error in [
        scheduler
            .file_monitor_read(&mut monitor, vec![42], &cancelled)
            .await
            .0
            .unwrap_err(),
        scheduler
            .file_monitor_write(&mut monitor, vec![42], &cancelled)
            .await
            .0
            .unwrap_err(),
        scheduler
            .file_monitor_read_at(&mut monitor, vec![42], 0, &cancelled)
            .await
            .0
            .unwrap_err(),
        scheduler
            .file_monitor_write_at(&mut monitor, vec![42], 0, &cancelled)
            .await
            .0
            .unwrap_err(),
        scheduler
            .file_monitor_close(&mut monitor, &cancelled)
            .await
            .unwrap_err(),
    ] {
        crate::file::cancelled::<()>(Err(error));
    }
    assert!(
        scheduler
            .file_monitor_read_at(&mut monitor, vec![0], u64::MAX, &never)
            .await
            .0
            .is_err()
    );
    assert!(
        scheduler
            .file_monitor_write_at(&mut monitor, vec![42], u64::MAX, &never)
            .await
            .0
            .is_err()
    );
    // Constructing and dropping an unpolled future changes neither the cursor nor monitor admission.
    drop(scheduler.file_monitor_write(&mut monitor, vec![42], &never));
    assert_eq!(
        scheduler
            .file_monitor_read_at(&mut monitor, vec![0], 0, &never)
            .await
            .1,
        b"a"
    );
    scheduler
        .file_monitor_close(&mut monitor, &never)
        .await
        .unwrap();
}

pub async fn abandonment<S: FileMonitoring>(scheduler: &S, file: &S::File) {
    let never = Cancellation::never();
    let mut monitor = scheduler.file_monitor(file).unwrap();
    {
        let mut operation =
            Box::pin(scheduler.file_monitor_read_at(&mut monitor, vec![0; 6], 0, &never));
        // Either immediate completion or pending is valid; close must establish retirement in both cases.
        poll_fn(|context| {
            let _ = operation.as_mut().poll(context);
            Poll::Ready(())
        })
        .await;
    }
    scheduler
        .file_monitor_close(&mut monitor, &never)
        .await
        .unwrap();
}

// Stop inside payload access, on the independent file worker or ring driver, so lifecycle assertions cannot accidentally test an already-completed operation.
struct GatedBuffer {
    bytes: Vec<u8>,
    started: Option<std::sync::mpsc::Sender<()>>,
    release: std::sync::mpsc::Receiver<()>,
}
impl AsRef<[u8]> for GatedBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
impl AsMut<[u8]> for GatedBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        if let Some(started) = self.started.take() {
            started.send(()).unwrap();
            self.release
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
        }
        &mut self.bytes
    }
}
// SAFETY: The Vec owns stable initialized storage, with exclusive access through the transferred owner. Gating never changes its allocation or range.
unsafe impl core_scheduler::IoBuf for GatedBuffer {}
unsafe impl core_scheduler::IoBufMut for GatedBuffer {}

pub async fn pending_retirement<S: FileMonitoring>(scheduler: &S, file: &S::File) {
    let never = Cancellation::never();
    let mut monitor = scheduler.file_monitor(file).unwrap();
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let buffer = GatedBuffer {
        bytes: vec![0; 6],
        started: Some(started),
        release: wait_release,
    };
    let mut operation = Box::pin(scheduler.file_monitor_read_at(&mut monitor, buffer, 0, &never));
    assert!(
        futures_lite::future::poll_once(operation.as_mut())
            .await
            .is_none()
    );
    wait_started
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    drop(operation);
    let error = scheduler
        .file_monitor_read_at(&mut monitor, vec![0], 0, &never)
        .await
        .0
        .unwrap_err();
    assert_eq!(
        std::io::Error::from(error).kind(),
        std::io::ErrorKind::WouldBlock
    );
    let cancellation = Cancellation::new();
    let mut close = Box::pin(scheduler.file_monitor_close(&mut monitor, &cancellation));
    assert!(
        futures_lite::future::poll_once(close.as_mut())
            .await
            .is_none()
    );
    cancellation.cancel();
    crate::file::cancelled(close.await);
    release.send(()).unwrap();
    scheduler
        .file_monitor_close(&mut monitor, &never)
        .await
        .unwrap();
    // Dropping monitor here leaves the caller free to immediately close the original file.
}

pub async fn inflight_cancellation<S: FileMonitoring>(scheduler: &S, file: &S::File) {
    let cancellation = Cancellation::new();
    let mut monitor = scheduler.file_monitor(file).unwrap();
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let buffer = GatedBuffer {
        bytes: vec![0; 6],
        started: Some(started),
        release: wait_release,
    };
    let allocation = buffer.as_ref().as_ptr() as usize;
    let mut operation =
        Box::pin(scheduler.file_monitor_read_at(&mut monitor, buffer, 0, &cancellation));
    assert!(
        futures_lite::future::poll_once(operation.as_mut())
            .await
            .is_none()
    );
    wait_started
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    cancellation.cancel();
    release.send(()).unwrap();
    let (_, mut buffer) = operation.await;
    // Either completed progress or cancellation may win, but ownership and the original stable allocation must always return after retirement.
    assert_eq!(buffer.as_ref().as_ptr() as usize, allocation);
    assert_eq!(buffer.as_mut().len(), 6);
    scheduler
        .file_monitor_close(&mut monitor, &Cancellation::never())
        .await
        .unwrap();
}
