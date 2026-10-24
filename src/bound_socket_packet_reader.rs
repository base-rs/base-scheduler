// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPool, BufferPoolOptions, BufferPooling, Cancellation, Interest, MonitorStorage,
    OperationResult, OwnedSocketRecvMessage, ReceiveBuffer, SocketMonitor, SocketMonitoring,
    SocketPacketReader, SocketRecvFlags, SocketRecvResult, SocketType,
};
use std::{
    future::poll_fn,
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};

/// A packet reader retaining one scheduler and readable monitor. Borrowed reads allocate no payload storage; the first owned read lazily constructs a two-slot, 64 KiB-per-slot pool. Idle readiness waits return their unused lease to the pool.
///
/// This demand-driven composition does not receive ahead or select a backend's provided-buffer receiver. Cancellation and ordinary receive errors leave it usable. Close retires its registration; drop the handle afterward to release its retained socket before exclusive socket close.
pub struct BoundSocketPacketReader<'a, S>
where
    S: SocketMonitoring + BufferPooling + Sync,
    S::Socket: 'static,
{
    scheduler: &'a S,
    monitor: MonitorStorage<SocketMonitor<S::Socket, S::MonitorRegistration>>,
    pool: Option<S::Pool>,
    closing: bool,
}

impl<'a, S> BoundSocketPacketReader<'a, S>
where
    S: SocketMonitoring + BufferPooling + Sync,
    S::Socket: 'static,
{
    pub(crate) fn new(scheduler: &'a S, socket: &S::Socket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? == SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a packet reader requires a packet socket",
            ));
        }
        Ok(Self {
            scheduler,
            monitor: scheduler.socket_monitor(socket, Interest::Readable)?,
            pool: None,
            closing: false,
        })
    }

    /// Select reusable owned receive storage, including between completed reads. Outstanding messages retain their earlier pool independently. Borrowed reads never acquire from this pool.
    pub fn with_pool(mut self, pool: S::Pool) -> Self {
        self.pool = Some(pool);
        self
    }
}

impl<S> SocketPacketReader for BoundSocketPacketReader<'_, S>
where
    S: SocketMonitoring + BufferPooling + Sync,
    S::Socket: 'static,
{
    type Buffer = <S::Pool as BufferPool>::Buffer;

    async fn read(
        &mut self,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<OwnedSocketRecvMessage<Self::Buffer>>> {
        cancellation.check()?;
        if self.closing {
            return Ok(None);
        }
        if self.pool.is_none() {
            self.pool = Some(
                self.scheduler
                    .buffer_pool(BufferPoolOptions::new(2, 64 * 1024))?,
            );
        }
        let pool = self.pool.as_ref().unwrap();
        loop {
            let mut buffer = pool.acquire(cancellation).await?;
            let result = {
                let mut vectors = [IoSliceMut::new(buffer.as_mut())];
                let mut receive = pin!(self.scheduler.socket_monitor_recvmsg_poll(
                    &mut self.monitor,
                    &mut vectors,
                    SocketRecvFlags::default(),
                    cancellation,
                ));
                poll_fn(|context| Poll::Ready(receive.as_mut().poll(context))).await
            };
            match result {
                Poll::Ready(result) => {
                    return result.map(|result| {
                        buffer.truncate(result.bytes);
                        Some(OwnedSocketRecvMessage::new(result, buffer))
                    });
                }
                Poll::Pending => drop(buffer),
            }
            // The syscall-first probe consumed no message. Release scarce storage before waiting, including when this read future is later abandoned.
            self.scheduler
                .socket_monitor_wait(&mut self.monitor, cancellation)
                .await?;
        }
    }

    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.read_vectored(&mut [IoSliceMut::new(buffer)], cancellation)
            .await
    }

    async fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        cancellation.check()?;
        if self.closing {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
        }
        self.scheduler
            .socket_monitor_recvmsg_poll(
                &mut self.monitor,
                buffers,
                SocketRecvFlags::default(),
                cancellation,
            )
            .await
    }

    async fn close(&mut self) -> io::Result<()> {
        self.closing = true;
        self.scheduler.socket_monitor_close(&mut self.monitor).await
    }
}
