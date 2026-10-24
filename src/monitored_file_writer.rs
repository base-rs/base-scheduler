// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, FileMonitor, FileMonitoring, FileWriter, IoBuf};
use std::io;

/// A file writer retaining its construction scheduler and reusable monitor. Operations delegate directly to that scheduler without a task, boxed future, or additional allocation. An abandoned accepted operation must retire before this handle can be reused.
pub struct MonitoredFileWriter<'scheduler, S: FileMonitoring + ?Sized> {
    pub(crate) scheduler: &'scheduler S,
    pub(crate) monitor: FileMonitor<S::File, S::FileRegistration>,
}

impl<S: crate::FileMonitorSubmission + Sync + ?Sized> MonitoredFileWriter<'_, S> {
    /// Await operation admission separately from its owning completion, preserving monitor admission and retirement safety. The accepted operation can progress while the caller drives other work; the original buffer returns with the completion.
    pub async fn write_submit<B: IoBuf>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl std::future::Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>
    {
        self.scheduler
            .file_monitor_write_submit(&mut self.monitor, buffer, cancellation)
            .await
    }

    /// Await operation admission separately from its owning completion, preserving monitor admission and retirement safety. The accepted operation can progress while the caller drives other work; the original buffer returns with the completion.
    pub async fn write_at_submit<B: IoBuf>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl std::future::Future<Output = BufferResult<B>> + Send, (crate::OperationError, B)>
    {
        self.scheduler
            .file_monitor_write_at_submit(&mut self.monitor, buffer, offset, cancellation)
            .await
    }
}

impl<S: FileMonitoring + Sync + ?Sized> FileWriter for MonitoredFileWriter<'_, S> {
    async fn write<B: IoBuf>(&mut self, buffer: B, cancellation: &Cancellation) -> BufferResult<B> {
        self.scheduler
            .file_monitor_write(&mut self.monitor, buffer, cancellation)
            .await
    }

    async fn write_at<B: IoBuf>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        self.scheduler
            .file_monitor_write_at(&mut self.monitor, buffer, offset, cancellation)
            .await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.scheduler.file_monitor_close(&mut self.monitor).await
    }
}
