// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPool, Cancellation, FileMonitor, FileMonitoring, HeapBufferPool, OperationResult,
    ReceiveBuffer,
};

/// A sequential file reader composed from reusable monitor state and a bounded pool. Completed chunks own their leases independently of this receiver, allowing an output write to overlap the next read. Scheduler and cancellation remain explicit on every operation.
pub struct FileReceiver<File, Registration = (), Pool: BufferPool = HeapBufferPool> {
    monitor: FileMonitor<File, Registration>,
    pool: Pool,
    terminal: bool,
}
impl<File, Registration, P: BufferPool> FileReceiver<File, Registration, P> {
    pub fn new(monitor: FileMonitor<File, Registration>, pool: P) -> Self {
        Self {
            monitor,
            pool,
            terminal: false,
        }
    }

    /// Receive a completed initialized prefix. EOF is `None`. `WouldBlock` and `InvalidInput` reject this attempt without ending the receiver. Other read errors, including cancellation after I/O admission, are returned once; subsequent reads return `None`. Pre-cancellation and cancellation while waiting for pool capacity leave the receiver unchanged.
    pub async fn recv<S>(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<P::Buffer>>
    where
        S: FileMonitoring<File = File, FileRegistration = Registration>,
        Registration: Send,
    {
        cancellation.check()?;
        if self.terminal {
            return Ok(None);
        }
        let buffer = self.pool.acquire(cancellation).await?;
        let (result, mut buffer) = scheduler
            .file_monitor_read(&mut self.monitor, buffer, cancellation)
            .await;
        match result {
            Ok(0) => {
                self.terminal = true;
                Ok(None)
            }
            Ok(length) => {
                buffer.truncate(length);
                Ok(Some(buffer))
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::InvalidInput
                ) =>
            {
                Err(error)
            }
            Err(error) => {
                self.terminal = true;
                Err(error)
            }
        }
    }

    /// Stop admission and await accepted I/O retirement; completed leases remain valid. Does not close the underlying file.
    pub async fn close<S>(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<()>
    where
        S: FileMonitoring<File = File, FileRegistration = Registration>,
        Registration: Send,
    {
        cancellation.check()?;
        self.terminal = true;
        scheduler
            .file_monitor_close(&mut self.monitor, cancellation)
            .await
    }
}
