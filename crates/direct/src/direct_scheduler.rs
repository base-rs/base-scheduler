// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BlockingTask, DirectFile, DirectOptions, SystemFileMetadata,
    blocking_pool::{BlockingPool, admission_error},
    file_io,
};
use futures_channel::oneshot;
use protocol_scheduler::{
    Blocking, BlockingError, BufferResult, Cancellation, FileClose, FileOpen, FileOperation,
    FileOptions, FileRead, FileReadAt, FileSeek, FileStat, FileSync, FileTruncate, FileWrite,
    FileWriteAt, IoBuf, IoBufMut, OperationResult, SyncMode, TaskError,
};
use std::{
    io::{self, SeekFrom},
    path::Path,
};

/// Explicit file and blocking-work services, independent of the executor polling their futures.
///
/// All direct instances accept the same [`DirectFile`] resources. Ordinary operations use a bounded worker pool; close uses an independent bounded cleanup worker. This type implements neither asynchronous task spawning nor sockets, clocks, or signals.
pub struct DirectScheduler {
    pub(crate) work: BlockingPool,
    pub(crate) cleanup: BlockingPool,
}

impl DirectScheduler {
    /// Configure lazy worker startup. Counts and capacity must be nonzero.
    pub fn new(options: DirectOptions) -> io::Result<Self> {
        if options.threads == 0 || options.capacity == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "thread count and capacity must be nonzero",
            ));
        }
        Ok(Self {
            work: BlockingPool::new(options),
            cleanup: BlockingPool::new(DirectOptions {
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
    }

    /// Close admission and await retirement of all accepted work, including abandoned results.
    ///
    /// Coordinate application cancellation and asynchronous cleanup before calling this. Waiting for shutdown from a blocking job owned by this scheduler would wait for that same job. A syscall or closure that never returns can prevent shutdown from completing.
    pub async fn shutdown(&self) {
        self.close();
        self.work.drain().await;
        self.cleanup.drain().await;
    }

    async fn buffer_operation<B: IoBuf>(
        &self,
        file: &DirectFile,
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

    async fn submit_close(&self, file: DirectFile) -> io::Result<CloseCompletion> {
        let file = file.into_file();
        self.cleanup
            .submit(move || file_io::close(file))
            .await
            .map_err(|error| io::Error::from(admission_error(error)))
    }
}

impl Blocking for DirectScheduler {
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

impl FileOperation for DirectScheduler {
    type File = DirectFile;
}

impl FileOpen for DirectScheduler {
    async fn file_open(
        &self,
        path: &Path,
        options: FileOptions,
        cancellation: &Cancellation,
    ) -> OperationResult<DirectFile> {
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
                    .map(DirectFile::from_std)
            })
            .await
            .0
    }
}

impl FileRead for DirectScheduler {
    async fn file_read<B: IoBufMut>(
        &self,
        file: &DirectFile,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, |file, buffer| {
            file_io::read(file, buffer.as_mut())
        })
        .await
    }
}

impl FileWrite for DirectScheduler {
    async fn file_write<B: IoBuf>(
        &self,
        file: &DirectFile,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.buffer_operation(file, buffer, cancellation, |file, buffer| {
            file_io::write(file, buffer.as_ref())
        })
        .await
    }
}

impl FileReadAt for DirectScheduler {
    async fn file_read_at<B: IoBufMut>(
        &self,
        file: &DirectFile,
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

impl FileWriteAt for DirectScheduler {
    async fn file_write_at<B: IoBuf>(
        &self,
        file: &DirectFile,
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

impl FileSeek for DirectScheduler {
    async fn file_seek(
        &self,
        file: &DirectFile,
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

impl FileTruncate for DirectScheduler {
    async fn file_truncate(
        &self,
        file: &DirectFile,
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

impl FileSync for DirectScheduler {
    async fn file_sync(
        &self,
        file: &DirectFile,
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

impl FileStat for DirectScheduler {
    type Metadata = SystemFileMetadata;

    async fn file_stat(
        &self,
        file: &DirectFile,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::Metadata> {
        self.work
            .execute(file.retain(), cancellation, |file| file_io::stat(file))
            .await
            .0
    }
}

impl FileClose for DirectScheduler {
    async fn file_close(&self, file: DirectFile) -> io::Result<()> {
        wait_close(self.submit_close(file).await?).await
    }

    async fn file_close_submit(&self, file: DirectFile) -> io::Result<()> {
        self.submit_close(file).await.map(drop)
    }
}

type CloseCompletion = oneshot::Receiver<Result<io::Result<()>, TaskError>>;

async fn wait_close(completion: CloseCompletion) -> io::Result<()> {
    completion
        .await
        .map_err(io::Error::other)?
        .map_err(|error| io::Error::other(error.to_string()))?
}

#[cfg(test)]
mod tests;
