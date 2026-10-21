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
        assert!(matches!(
            error,
            core_scheduler::OperationError::Cancelled(_)
        ));
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
