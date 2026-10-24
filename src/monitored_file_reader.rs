// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, FileMonitor, FileMonitoring, FileReader, IoBufMut};
use std::io;

/// A file reader retaining its construction scheduler and reusable monitor. Operations delegate directly to that scheduler without a task, boxed future, or additional allocation. An abandoned accepted operation must retire before this handle can be reused.
pub struct MonitoredFileReader<'scheduler, S: FileMonitoring + ?Sized> {
    pub(crate) scheduler: &'scheduler S,
    pub(crate) monitor: FileMonitor<S::File, S::FileRegistration>,
}

impl<S: crate::FileMonitorSubmission + Sync + ?Sized> MonitoredFileReader<'_, S> {
    /// Await operation admission separately from its owning completion, preserving monitor admission and retirement safety. The accepted operation can progress while the caller drives other work; the original buffer returns with the completion.
    pub async fn read_submit<B: IoBufMut>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl std::future::Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>
    {
        self.scheduler
            .file_monitor_read_submit(&mut self.monitor, buffer, cancellation)
            .await
    }

    /// Await operation admission separately from its owning completion, preserving monitor admission and retirement safety. The accepted operation can progress while the caller drives other work; the original buffer returns with the completion.
    pub async fn read_at_submit<B: IoBufMut>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl std::future::Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>
    {
        self.scheduler
            .file_monitor_read_at_submit(&mut self.monitor, buffer, offset, cancellation)
            .await
    }
}

impl<S: FileMonitoring + Sync + ?Sized> FileReader for MonitoredFileReader<'_, S> {
    async fn read<B: IoBufMut>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_monitor_read(&mut self.monitor, buffer, cancellation)
            .await
    }

    async fn read_at<B: IoBufMut>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_monitor_read_at(&mut self.monitor, buffer, offset, cancellation)
            .await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.scheduler.file_monitor_close(&mut self.monitor).await
    }
}
