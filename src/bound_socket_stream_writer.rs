// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    Cancellation, Interest, IoBuf, MonitorStorage, OperationError, OperationResult,
    SocketMonitoring, SocketSendFlags, SocketStreamMonitor, SocketStreamOutput, SocketStreamWriter,
    SocketStreamWriting,
};
use std::io::{self, IoSlice};

/// A scheduler-bound output lane combining borrowed writes with ordered owned submission. Construct through [`SocketStreamWriting`].
pub struct BoundSocketStreamWriter<'a, S: SocketStreamWriting + SocketMonitoring + ?Sized, B: IoBuf>
where
    S::Socket: 'static,
{
    scheduler: &'a S,
    output: S::Writer<B>,
    monitor: MonitorStorage<SocketStreamMonitor<S::Socket, S::MonitorRegistration>>,
    closed: bool,
    submitted: bool,
}

impl<'a, S, B> BoundSocketStreamWriter<'a, S, B>
where
    S: SocketStreamWriting + SocketMonitoring + Sync + ?Sized,
    S::Socket: 'static,
    B: IoBuf,
{
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    #[doc(hidden)]
    pub fn new(scheduler: &'a S, socket: &S::Socket) -> io::Result<Self> {
        let output = scheduler.socket_stream_output(socket)?;
        let monitor = scheduler.socket_stream_monitor(socket, Interest::Writable)?;
        Ok(Self {
            scheduler,
            output,
            monitor,
            closed: false,
            submitted: false,
        })
    }

    fn check_open(&self) -> OperationResult<()> {
        if self.closed {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stream writer is closed").into())
        } else {
            Ok(())
        }
    }
}

impl<S, B> SocketStreamWriter for BoundSocketStreamWriter<'_, S, B>
where
    S: SocketStreamWriting + SocketMonitoring + Sync + ?Sized,
    S::Socket: 'static,
    B: IoBuf,
{
    type Buffer = B;

    async fn write(
        &mut self,
        buffer: &[u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.check_open()?;
        if self.submitted {
            self.output.flush(self.scheduler, cancellation).await?;
            self.submitted = false;
        }
        self.scheduler
            .socket_monitor_send_poll(
                &mut self.monitor,
                buffer,
                SocketSendFlags::default(),
                cancellation,
            )
            .await
    }

    async fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.check_open()?;
        if self.submitted {
            self.output.flush(self.scheduler, cancellation).await?;
            self.submitted = false;
        }
        self.scheduler
            .socket_monitor_sendmsg_poll(
                &mut self.monitor,
                buffers,
                None,
                SocketSendFlags::default(),
                cancellation,
            )
            .await
    }

    async fn submit(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<(), (OperationError, B)> {
        if let Err(error) = self.check_open() {
            return Err((error, buffer));
        }
        // Mark before polling admission: an abandoned admission may already have accepted output.
        self.submitted = true;
        self.output
            .submit(self.scheduler, buffer, cancellation)
            .await
    }

    async fn flush(&mut self, cancellation: &Cancellation) -> OperationResult<()> {
        self.check_open()?;
        self.output.flush(self.scheduler, cancellation).await?;
        self.submitted = false;
        Ok(())
    }

    async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        let output = self.output.close(self.scheduler).await;
        let monitor = self.scheduler.socket_monitor_close(&mut self.monitor).await;
        output.and(monitor)
    }
}
