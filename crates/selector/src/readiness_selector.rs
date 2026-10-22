// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BlockingTask, NativeFile, SelectorOptions, SystemFileMetadata,
    blocking_pool::{BlockingPool, admission_error},
    file_io,
};
use core_scheduler::{
    Blocking, BlockingError, BufferResult, Cancellation, FileClose, FileOpen, FileOperation,
    FileOptions, FileRead, FileReadAt, FileSeek, FileStat, FileSync, FileTruncate, FileWrite,
    FileWriteAt, IoBuf, IoBufMut, OperationResult, SyncMode, TaskError,
};
use futures_channel::oneshot;
use std::{
    io::{self, SeekFrom},
    path::Path,
};

/// Native I/O capabilities independent of task execution. Sockets use `async-io` readiness; files, resolution, and blocking work use bounded worker pools.
///
/// Compatible instances accept the same file and socket types. The readiness reactor progresses independently of the executor polling operation futures.
pub struct ReadinessSelector {
    pub(crate) work: BlockingPool,
    pub(crate) cleanup: BlockingPool,
    pub(crate) closed: Cancellation,
    #[cfg(test)]
    pub(crate) option_probe: std::sync::Arc<crate::socket_options::Probe>,
    #[cfg(test)]
    pub(crate) resolver_probe: std::sync::Arc<crate::resolver::Probe>,
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) file_probe: std::sync::Arc<crate::file_extensions::Probe>,
    #[cfg(unix)]
    pub(crate) signals: crate::signal_service::SignalService,
}

impl ReadinessSelector {
    /// Create lazy I/O services with default limits.
    pub fn new() -> io::Result<Self> {
        Self::with_options(SelectorOptions::default())
    }

    pub(crate) fn check_open(&self) -> io::Result<()> {
        if self.closed.is_cancelled() {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "scheduler is closed",
            ))
        } else {
            Ok(())
        }
    }

    /// Configure lazy I/O and signal service startup. Worker count and capacity must be nonzero.
    pub fn with_options(options: SelectorOptions) -> io::Result<Self> {
        if options.threads == 0 || options.capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "worker counts and capacities must be nonzero",
            ));
        }
        Ok(Self {
            #[cfg(test)]
            option_probe: Default::default(),
            #[cfg(test)]
            resolver_probe: Default::default(),
            #[cfg(all(test, target_os = "linux"))]
            file_probe: Default::default(),
            #[cfg(unix)]
            signals: crate::signal_service::SignalService::new(),
            closed: Cancellation::new(),
            work: BlockingPool::new(options),
            cleanup: BlockingPool::new(SelectorOptions {
                threads: 1,
                capacity: options.capacity,
            }),
        })
    }

    /// Stop admission. Queued general blocking jobs are discarded; accepted I/O and cleanup retire normally. Running closures cannot be interrupted.
    ///
    /// Dropping the scheduler also closes admission without blocking the dropping thread. Workers retain outstanding resources until access ends.
    pub fn close(&self) {
        self.work.close();
        self.cleanup.close();
        self.closed.cancel();
        #[cfg(unix)]
        self.signals.close();
    }

    /// Close admission and await retirement of accepted I/O, blocking work, cleanup, and signal delivery, including abandoned results.
    ///
    /// Coordinate application cancellation and cleanup first. A blocking job must not await shutdown of its own selector. A syscall or closure that never returns can prevent shutdown from completing.
    pub async fn shutdown(&self) {
        self.close();
        self.work.drain().await;
        self.cleanup.drain().await;
        #[cfg(unix)]
        self.signals.drain().await;
    }

    pub(crate) async fn buffer_operation_submit<B: IoBuf>(
        &self,
        file: &NativeFile,
        buffer: B,
        cancellation: &Cancellation,
        operation: impl FnOnce(&std::fs::File, &mut B) -> io::Result<usize> + Send + 'static,
    ) -> Result<
        impl std::future::Future<Output = BufferResult<B>> + Send,
        (core_scheduler::OperationError, B),
    > {
        match self
            .work
            .execute_submit(
                (file.retain(), buffer),
                cancellation,
                move |(file, buffer)| operation(file, buffer),
            )
            .await
        {
            Ok(completion) => Ok(async move {
                let (result, (_, buffer)) = completion.await;
                (result, buffer)
            }),
            Err((error, (_, buffer))) => Err((error, buffer)),
        }
    }

    async fn buffer_operation<B: IoBuf>(
        &self,
        file: &NativeFile,
        buffer: B,
        cancellation: &Cancellation,
        operation: impl FnOnce(&std::fs::File, &mut B) -> io::Result<usize> + Send + 'static,
    ) -> BufferResult<B> {
        let (result, (_, buffer)) = self
            .work
            .execute(
                (file.retain(), buffer),
                cancellation,
                move |(file, buffer)| operation(file, buffer),
            )
            .await;
        (result, buffer)
    }

    async fn submit_close(&self, file: NativeFile) -> io::Result<CloseCompletion> {
        let file = file.into_file();
        self.cleanup
            .submit(move || file_io::close(file))
            .await
            .map_err(|error| io::Error::from(admission_error(error)))
    }
}

impl Drop for ReadinessSelector {
    fn drop(&mut self) {
        self.close();
    }
}

impl Blocking for ReadinessSelector {
    type Handle<Output>
        = BlockingTask<Output>
    where
        Output: Send + 'static;

    async fn spawn_blocking<F, Output>(
        &self,
        operation: F,
        cancellation: &Cancellation,
    ) -> Result<Self::Handle<Output>, BlockingError>
    where
        F: FnOnce() -> Output + Send + 'static,
        Output: Send + 'static,
    {
        self.work.spawn(operation, cancellation).await
    }
}

impl FileOperation for ReadinessSelector {
    type File = NativeFile;
}

impl FileOpen for ReadinessSelector {
    async fn file_open(
        &self,
        path: &Path,
        options: FileOptions,
        cancellation: &Cancellation,
    ) -> OperationResult<NativeFile> {
        let path = path.to_owned();
        self.work
            .execute((), cancellation, move |_| {
                std::fs::OpenOptions::new()
                    .read(options.read)
                    .write(options.write)
                    .create(options.create)
                    .create_new(options.create_new)
                    .truncate(options.truncate)
                    .open(path)
                    .map(NativeFile::from_std)
            })
            .await
            .0
    }
}

impl FileRead for ReadinessSelector {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &NativeFile,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, |file, buffer| {
            file_io::read(file, buffer.as_mut())
        })
        .await
    }
}

impl FileWrite for ReadinessSelector {
    async fn file_write<B: IoBuf>(
        &self,
        file: &NativeFile,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, |file, buffer| {
            file_io::write(file, buffer.as_ref())
        })
        .await
    }
}

impl FileReadAt for ReadinessSelector {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &NativeFile,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, move |file, buffer| {
            file_io::read_at(file, buffer.as_mut(), offset)
        })
        .await
    }
}

impl FileWriteAt for ReadinessSelector {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &NativeFile,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, move |file, buffer| {
            file_io::write_at(file, buffer.as_ref(), offset)
        })
        .await
    }
}

impl FileSeek for ReadinessSelector {
    async fn file_seek(
        &self,
        file: &NativeFile,
        position: SeekFrom,
        cancellation: &Cancellation,
    ) -> OperationResult<u64> {
        self.work
            .execute(file.retain(), cancellation, move |file| {
                file_io::seek(file, position)
            })
            .await
            .0
    }
}

impl FileTruncate for ReadinessSelector {
    async fn file_truncate(
        &self,
        file: &NativeFile,
        length: u64,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.work
            .execute(file.retain(), cancellation, move |file| {
                file_io::truncate(file, length)
            })
            .await
            .0
    }
}

impl FileSync for ReadinessSelector {
    async fn file_sync(
        &self,
        file: &NativeFile,
        mode: SyncMode,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.work
            .execute(file.retain(), cancellation, move |file| {
                file_io::sync(file, mode)
            })
            .await
            .0
    }
}

impl FileStat for ReadinessSelector {
    type Metadata = SystemFileMetadata;

    async fn file_stat(
        &self,
        file: &NativeFile,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Metadata> {
        self.work
            .execute(file.retain(), cancellation, |file| file_io::stat(file))
            .await
            .0
    }
}

impl FileClose for ReadinessSelector {
    async fn file_close(&self, file: NativeFile) -> io::Result<()> {
        wait_close(self.submit_close(file).await?).await
    }

    async fn file_close_submit(&self, file: NativeFile) -> io::Result<()> {
        self.submit_close(file).await.map(drop)
    }
}

type CloseCompletion = oneshot::Receiver<Result<io::Result<()>, TaskError>>;

pub(crate) async fn wait_close(completion: CloseCompletion) -> io::Result<()> {
    completion
        .await
        .map_err(io::Error::other)?
        .map_err(|error| io::Error::other(error.to_string()))?
}

#[cfg(test)]
mod tests;

impl crate::Selector for ReadinessSelector {
    fn close(&self) {
        self.close();
    }
    async fn shutdown(&self) {
        self.shutdown().await;
    }
}
