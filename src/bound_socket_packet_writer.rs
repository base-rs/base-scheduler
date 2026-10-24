// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferResult, Cancellation, Interest, IoBuf, MonitorStorage, OperationResult, SocketAddress,
    SocketMonitor, SocketMonitoring, SocketPacketWriter, SocketSendFlags, SocketType,
};
use std::io::{self, IoSlice};

/// A packet writer retaining one scheduler and writable monitor. All writes use a single nonblocking message-send operation, without payload allocation or packet splitting. Cancellation and send errors leave it usable.
pub struct BoundSocketPacketWriter<'a, S>
where
    S: SocketMonitoring + Sync,
    S::Socket: 'static,
{
    scheduler: &'a S,
    monitor: MonitorStorage<SocketMonitor<S::Socket, S::MonitorRegistration>>,
    closing: bool,
}

impl<'a, S> BoundSocketPacketWriter<'a, S>
where
    S: SocketMonitoring + Sync,
    S::Socket: 'static,
{
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    #[doc(hidden)]
    pub fn new(scheduler: &'a S, socket: &S::Socket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? == SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a packet writer requires a packet socket",
            ));
        }
        Ok(Self {
            scheduler,
            monitor: scheduler.socket_monitor(socket, Interest::Writable)?,
            closing: false,
        })
    }
}

impl<S> SocketPacketWriter for BoundSocketPacketWriter<'_, S>
where
    S: SocketMonitoring + Sync,
    S::Socket: 'static,
{
    async fn write(
        &mut self,
        buffer: &[u8],
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.write_vectored(&[IoSlice::new(buffer)], address, cancellation)
            .await
    }

    async fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        cancellation.check()?;
        if self.closing {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
        }
        self.scheduler
            .socket_monitor_sendmsg_poll(
                &mut self.monitor,
                buffers,
                address,
                SocketSendFlags::default(),
                cancellation,
            )
            .await
    }

    async fn write_owned<B: IoBuf>(
        &mut self,
        buffer: B,
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let result = self.write(buffer.as_ref(), address, cancellation).await;
        (result, buffer)
    }

    async fn close(&mut self) -> io::Result<()> {
        self.closing = true;
        self.scheduler.socket_monitor_close(&mut self.monitor).await
    }
}
