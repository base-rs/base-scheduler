// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use futures_lite::future::block_on;
use protocol_scheduler::{
    Blocking, Cancellation, FileClose, FileMetadata, FileOpen, FileOptions, FileSeek, FileStat,
};
use protocol_scheduler_conformance::{blocking, file};
use protocol_scheduler_direct::{DirectOptions, DirectScheduler};
use std::{io::SeekFrom, sync::mpsc, time::Duration};

#[test]
fn direct_files_pass_capability_conformance_and_move_between_instances() {
    block_on(async {
        let first = DirectScheduler::new(DirectOptions::default()).unwrap();
        let second = DirectScheduler::new(DirectOptions {
            threads: 1,
            capacity: 1,
        })
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        let file = file::create(&first, &path, || assert!(!path.exists())).await;
        file::write(&first, &file, b"abcdef").await;
        assert_eq!(std::fs::read(&path).unwrap(), b"abcdef");
        first
            .file_seek(&file, SeekFrom::Start(0), &Cancellation::never())
            .await
            .unwrap();
        file::seek(&first, &file, 6).await;
        std::thread::scope(|scope| {
            scope
                .spawn(|| block_on(file::read(&second, &file, b"abcdef")))
                .join()
                .unwrap();
        });
        file::metadata(&second, &file, 6).await;
        let metadata = second
            .file_stat(&file, &Cancellation::never())
            .await
            .unwrap();
        assert_eq!(metadata.size(), Some(6));
        file::read_at(&first, &file, 1, b"bcde").await;
        file::write_at(&second, &file, 2, b"XY").await;
        assert_eq!(std::fs::read(&path).unwrap(), b"abXYef");
        file::truncate(&first, &file, 9).await;
        assert_eq!(std::fs::read(&path).unwrap(), b"abXYef\0\0\0");
        file::truncate(&second, &file, 3).await;
        assert_eq!(std::fs::read(&path).unwrap(), b"abX");
        file::sync(&second, &file).await;
        second.file_close(file).await.unwrap();
        let read_only = first
            .file_open(&path, FileOptions::READ, &Cancellation::never())
            .await
            .unwrap();
        file::write_error(&first, &read_only).await;
        first.file_close_submit(read_only).await.unwrap();
        first.shutdown().await;
        second.shutdown().await;
    });
}

#[cfg(unix)]
#[test]
fn unix_positioned_operations_preserve_the_shared_cursor() {
    block_on(async {
        let scheduler = DirectScheduler::new(DirectOptions::default()).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("file");
        std::fs::write(&path, b"abcdef").unwrap();
        let file = scheduler
            .file_open(
                &path,
                FileOptions {
                    write: true,
                    ..FileOptions::READ
                },
                &Cancellation::never(),
            )
            .await
            .unwrap();
        scheduler
            .file_seek(&file, SeekFrom::Start(3), &Cancellation::never())
            .await
            .unwrap();
        file::read_at(&scheduler, &file, 0, b"abcdef").await;
        file::write_at(&scheduler, &file, 0, b"ABC").await;
        assert_eq!(std::fs::read(&path).unwrap(), b"ABCdef");
        assert_eq!(
            scheduler
                .file_seek(&file, SeekFrom::Current(0), &Cancellation::never())
                .await
                .unwrap(),
            3
        );
        scheduler.file_close(file).await.unwrap();
        scheduler.shutdown().await;
    });
}

struct QueueFixture(DirectScheduler);
struct Hold {
    release: Option<mpsc::Sender<()>>,
    _handle: protocol_scheduler_direct::BlockingTask<()>,
}
impl Drop for Hold {
    fn drop(&mut self) {
        let _ = self.release.take().unwrap().send(());
    }
}
impl blocking::BlockingFixture for QueueFixture {
    type Scheduler = DirectScheduler;
    type Guard = Hold;
    fn scheduler(&self) -> &Self::Scheduler {
        &self.0
    }
    async fn hold_workers(&self) -> Hold {
        let (started, start) = futures_channel::oneshot::channel();
        let (release, released) = mpsc::channel();
        let handle = self
            .0
            .spawn_blocking(
                move || {
                    started.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(10)).unwrap();
                },
                &Cancellation::never(),
            )
            .await
            .unwrap();
        start.await.unwrap();
        Hold {
            release: Some(release),
            _handle: handle,
        }
    }
}

#[test]
fn direct_blocking_passes_lifecycle_and_bounded_queue_conformance() {
    block_on(async {
        let fixture = QueueFixture(
            DirectScheduler::new(DirectOptions {
                threads: 1,
                capacity: 2,
            })
            .unwrap(),
        );
        blocking::lifecycle(&fixture.0).await;
        blocking::queue(&fixture).await;
        fixture.0.shutdown().await;
    });
}

#[derive(Clone, Copy)]
enum QueueFault {
    IgnoreQueuedCancellation,
    IgnoreCapacity,
    AdmitOnCancellation,
    DetachOnDrop,
}
struct IncorrectQueue {
    direct: DirectScheduler,
    calls: std::sync::atomic::AtomicUsize,
    fault: QueueFault,
}
impl Blocking for IncorrectQueue {
    type Handle<T>
        = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<T, protocol_scheduler::TaskError>> + Send>,
    >
    where
        T: Send + 'static;
    async fn spawn_blocking<F, T>(
        &self,
        operation: F,
        cancellation: &Cancellation,
    ) -> Result<Self::Handle<T>, protocol_scheduler::BlockingError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if (call == 1 && matches!(self.fault, QueueFault::IgnoreCapacity))
            || (call == 1 && matches!(self.fault, QueueFault::AdmitOnCancellation))
        {
            if matches!(self.fault, QueueFault::AdmitOnCancellation) {
                cancellation.cancelled().await;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation))
                .map_err(protocol_scheduler::TaskError::Panicked);
            return Ok(Box::pin(std::future::ready(result)));
        }
        let never = Cancellation::never();
        let token = if call == 0 && matches!(self.fault, QueueFault::IgnoreQueuedCancellation) {
            &never
        } else {
            cancellation
        };
        let handle = self.direct.spawn_blocking(operation, token).await?;
        if call == 2 && matches!(self.fault, QueueFault::DetachOnDrop) {
            std::thread::spawn(move || {
                let _ = block_on(handle);
            });
            return Ok(Box::pin(std::future::pending()));
        }
        Ok(Box::pin(handle))
    }
}
struct IncorrectFixture(IncorrectQueue);
impl blocking::BlockingFixture for IncorrectFixture {
    type Scheduler = IncorrectQueue;
    type Guard = Hold;
    fn scheduler(&self) -> &IncorrectQueue {
        &self.0
    }
    async fn hold_workers(&self) -> Hold {
        let (started, start) = futures_channel::oneshot::channel();
        let (release, released) = mpsc::channel();
        let handle = self
            .0
            .direct
            .spawn_blocking(
                move || {
                    started.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(10)).unwrap();
                },
                &Cancellation::never(),
            )
            .await
            .unwrap();
        start.await.unwrap();
        Hold {
            release: Some(release),
            _handle: handle,
        }
    }
}
#[test]
fn queue_conformance_rejects_cancellation_capacity_and_handle_ownership_violations() {
    for fault in [
        QueueFault::IgnoreQueuedCancellation,
        QueueFault::IgnoreCapacity,
        QueueFault::AdmitOnCancellation,
        QueueFault::DetachOnDrop,
    ] {
        let fixture = IncorrectFixture(IncorrectQueue {
            direct: DirectScheduler::new(DirectOptions {
                threads: 1,
                capacity: 2,
            })
            .unwrap(),
            calls: std::sync::atomic::AtomicUsize::new(0),
            fault,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            block_on(blocking::queue(&fixture))
        }));
        block_on(fixture.0.direct.shutdown());
        assert!(result.is_err(), "conformance accepted an incorrect backend");
    }
}
