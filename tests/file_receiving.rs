// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::*;
#[path = "support/buffer_pool.rs"]
mod pool_support;
use futures_lite::future::block_on;
use pool_support::TestPool;
use std::{
    future::{Future, ready},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Files {
    written: Mutex<Vec<u8>>,
    limit: usize,
}
impl FileOperation for Files {
    type File = Arc<AtomicUsize>;
}
impl FileRead for Files {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &Self::File,
        mut buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let result = cancellation.check().map_err(Into::into).and_then(|_| {
            match file.load(Ordering::Relaxed) {
                0 => Ok(0),
                1 => {
                    buffer.as_mut().fill(42);
                    Ok(buffer.as_ref().len())
                }
                2 => Err(io::Error::from(io::ErrorKind::InvalidInput).into()),
                3 => Err(io::Error::from(io::ErrorKind::WouldBlock).into()),
                _ => Err(io::Error::from(io::ErrorKind::BrokenPipe).into()),
            }
        });
        (result, buffer)
    }
}
impl FileWrite for Files {
    async fn file_write<B: IoBuf>(
        &self,
        _: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let result = cancellation
            .check()
            .map_err(OperationError::from)
            .and_then(|_| {
                if self.limit == usize::MAX {
                    return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
                }
                let count = buffer.as_ref().len().min(self.limit);
                self.written
                    .lock()
                    .unwrap()
                    .extend_from_slice(&buffer.as_ref()[..count]);
                Ok(count)
            });
        (result, buffer)
    }
}
impl FileReadAt for Files {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_read(file, buffer, cancellation).await
    }
}
impl FileWriteAt for Files {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.file_write(file, buffer, cancellation).await
    }
}
impl FileMonitoring for Files {
    type FileRegistration = ();
    async fn file_monitor_close(&self, monitor: &mut FileMonitor<Self::File>) -> io::Result<()> {
        match monitor.file().load(Ordering::Relaxed) {
            100 => std::future::pending().await,
            101 => Err(io::ErrorKind::InvalidInput.into()),
            102 => Err(io::ErrorKind::BrokenPipe.into()),
            _ => Ok(()),
        }
    }
    fn file_monitor(&self, file: &Self::File) -> io::Result<FileMonitor<Self::File>> {
        Ok(FileMonitor::new(file.clone(), ()))
    }
}

#[test]
fn close_abandonment_stops_receiving_but_incompatible_close_preserves_it() {
    block_on(async {
        let scheduler = Files {
            written: Mutex::new(Vec::new()),
            limit: 8,
        };
        let never = Cancellation::never();
        let file = Arc::new(AtomicUsize::new(101));
        let pool = TestPool(HeapBufferPool::new(1, 8).unwrap());
        let mut receiver = FileReceiver::new(scheduler.file_monitor(&file).unwrap(), pool.clone());
        assert_eq!(
            receiver.close(&scheduler).await.unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        file.store(1, Ordering::Relaxed);
        let held = receiver.recv(&scheduler, &never).await.unwrap().unwrap();
        assert_eq!(held.as_ref(), &[42; 8]);
        assert!(pool.try_acquire().is_none());
        file.store(100, Ordering::Relaxed);
        {
            let mut close = std::pin::pin!(receiver.close(&scheduler));
            assert!(
                futures_lite::future::poll_once(close.as_mut())
                    .await
                    .is_none()
            );
        }
        let result = futures_lite::future::poll_once(receiver.recv(&scheduler, &never))
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_none());
        file.store(102, Ordering::Relaxed);
        assert_eq!(
            receiver.close(&scheduler).await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        file.store(101, Ordering::Relaxed);
        assert!(receiver.close(&scheduler).await.is_err());
        assert!(receiver.recv(&scheduler, &never).await.unwrap().is_none());
        file.store(1, Ordering::Relaxed);
        receiver.close(&scheduler).await.unwrap();
        receiver.close(&scheduler).await.unwrap();
        assert_eq!(held.as_ref(), &[42; 8]);
    });
}
impl FileSubmission for Files {
    async fn file_read_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        if let Err(error) = cancellation.check() {
            return Err((error.into(), buffer));
        }
        Ok(ready(self.file_read(file, buffer, cancellation).await))
    }
    async fn file_write_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        if let Err(error) = cancellation.check() {
            return Err((error.into(), buffer));
        }
        Ok(ready(self.file_write(file, buffer, cancellation).await))
    }
    async fn file_read_at_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_read_submit(file, buffer, cancellation).await
    }
    async fn file_write_at_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        _: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.file_write_submit(file, buffer, cancellation).await
    }
}
fn files(limit: usize) -> Files {
    Files {
        written: Mutex::new(Vec::new()),
        limit,
    }
}

#[test]
fn write_all_advances_short_writes_and_returns_the_original_owner() {
    block_on(async {
        for submitted in [false, true] {
            let scheduler = files(2);
            let file = Arc::new(AtomicUsize::new(1));
            let buffer = b"abcdef".to_vec();
            let pointer = buffer.as_ptr();
            let never = Cancellation::never();
            let (result, buffer) = if submitted {
                file_write_all_submit(&scheduler, &file, buffer, &never)
                    .await
                    .unwrap()
                    .await
            } else {
                file_write_all(&scheduler, &file, buffer, &never).await
            };
            assert_eq!(result.unwrap(), 6);
            assert_eq!(buffer, b"abcdef");
            assert_eq!(buffer.as_ptr(), pointer);
            assert_eq!(*scheduler.written.lock().unwrap(), b"abcdef");
        }
    });
}

#[test]
fn write_all_handles_empty_writes_errors_zero_progress_and_pre_cancellation() {
    block_on(async {
        let file = Arc::new(AtomicUsize::new(1));
        let never = Cancellation::never();
        for (limit, kind) in [
            (0, io::ErrorKind::WriteZero),
            (usize::MAX, io::ErrorKind::BrokenPipe),
        ] {
            let scheduler = files(limit);
            for submitted in [false, true] {
                let buffer = vec![42; 3];
                let (result, buffer) = if submitted {
                    file_write_all_submit(&scheduler, &file, buffer, &never)
                        .await
                        .unwrap()
                        .await
                } else {
                    file_write_all(&scheduler, &file, buffer, &never).await
                };
                assert_eq!(result.unwrap_err().kind(), kind);
                assert_eq!(buffer, [42; 3]);
            }
        }
        let scheduler = files(1);
        assert_eq!(
            file_write_all(&scheduler, &file, Vec::new(), &never)
                .await
                .0
                .unwrap(),
            0
        );
        let token = Cancellation::new();
        token.cancel();
        assert!(
            file_write_all(&scheduler, &file, vec![42], &token)
                .await
                .0
                .is_err()
        );
        assert!(
            file_write_all_submit(&scheduler, &file, vec![42], &token)
                .await
                .is_err()
        );
    });
}

#[test]
fn receiver_distinguishes_retryable_rejection_from_terminal_io_failure() {
    block_on(async {
        let scheduler = files(1);
        let file = Arc::new(AtomicUsize::new(2));
        let never = Cancellation::never();
        let mut receiver = FileReceiver::new(
            scheduler.file_monitor(&file).unwrap(),
            TestPool(HeapBufferPool::new(1, 8).unwrap()),
        );
        for state in [2, 3] {
            file.store(state, Ordering::Relaxed);
            assert!(receiver.recv(&scheduler, &never).await.is_err());
        }
        file.store(1, Ordering::Relaxed);
        assert_eq!(
            receiver
                .recv(&scheduler, &never)
                .await
                .unwrap()
                .unwrap()
                .as_ref(),
            [42; 8]
        );
        let cancelled = Cancellation::new();
        cancelled.cancel();
        drop(receiver.close(&scheduler));
        assert!(receiver.recv(&scheduler, &never).await.unwrap().is_some());
        file.store(4, Ordering::Relaxed);
        assert!(receiver.recv(&scheduler, &never).await.is_err());
        assert!(receiver.recv(&scheduler, &never).await.unwrap().is_none());
        receiver.close(&scheduler).await.unwrap();
    });
}

#[test]
fn cancellation_during_capacity_wait_preserves_file_reception() {
    use futures_lite::future::poll_once;
    use std::pin::pin;
    block_on(async {
        let scheduler = files(1);
        let file = Arc::new(AtomicUsize::new(1));
        let never = Cancellation::never();
        let mut receiver = FileReceiver::new(
            scheduler.file_monitor(&file).unwrap(),
            TestPool(HeapBufferPool::new(1, 8).unwrap()),
        );
        let held = receiver.recv(&scheduler, &never).await.unwrap().unwrap();
        let token = Cancellation::new();
        {
            let mut next = pin!(receiver.recv(&scheduler, &token));
            assert!(poll_once(next.as_mut()).await.is_none());
            token.cancel();
            assert!(next.await.is_err());
        }
        drop(held);
        assert!(receiver.recv(&scheduler, &never).await.unwrap().is_some());
        receiver.close(&scheduler).await.unwrap();
    });
}
