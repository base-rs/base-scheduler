// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPool, Cancellation, HeapBufferPool, MonitorStorage, OperationResult,
    OwnedSocketRecvMessage, ReceiveBuffer, SocketMonitorState, SocketMonitoring, SocketRecvFlags,
    SocketType,
};
use std::{
    future::poll_fn,
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};

/// A demand-driven receiver composed from a readable socket monitor and a shared pool. Works with general and stream monitors; it does not spawn a task, receive ahead, or allocate per message. A pending socket receive returns its buffer before waiting for readiness, so idle connections do not reserve pool capacity. Backends offering kernel receive-ahead retain their separate receiver implementation.
pub struct SocketReceiver<Monitor: crate::MonitorState, Pool: BufferPool = HeapBufferPool> {
    monitor: MonitorStorage<Monitor>,
    pool: Pool,
    flags: SocketRecvFlags,
    stream: bool,
    terminal: bool,
}
impl<M: crate::MonitorState, P: BufferPool> SocketReceiver<M, P> {
    /// `kind` must describe the socket: empty datagrams are messages, whereas a zero-length stream receive is EOF. Peeking is rejected because a receiver must consume its messages.
    pub fn new(
        monitor: MonitorStorage<M>,
        pool: P,
        kind: SocketType,
        flags: SocketRecvFlags,
    ) -> io::Result<Self> {
        if flags.peek {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receivers do not support peeking",
            ));
        }
        Ok(Self {
            monitor,
            pool,
            flags,
            stream: kind == SocketType::Stream,
            terminal: false,
        })
    }

    /// Receive one chunk or datagram, preserving message metadata. Returned storage is writable for in-place parsing/decryption, and may subsequently be frozen. Pre-cancellation and capacity-wait cancellation leave the receiver unchanged. `WouldBlock` and `InvalidInput` reject the attempt without ending reception. Other read errors terminate it; an empty datagram remains `Some`.
    pub async fn recv<S>(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<OwnedSocketRecvMessage<P::Buffer>>>
    where
        S: SocketMonitoring,
        S::Socket: 'static,
        M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
    {
        cancellation.check()?;
        if self.terminal {
            return Ok(None);
        }
        let result = loop {
            let mut buffer = self.pool.acquire(cancellation).await?;
            let result = {
                let mut buffers = [IoSliceMut::new(buffer.as_mut())];
                let mut receiving = pin!(scheduler.socket_monitor_recvmsg_poll(
                    &mut self.monitor,
                    &mut buffers,
                    self.flags,
                    cancellation,
                ));
                poll_fn(|context| Poll::Ready(receiving.as_mut().poll(context))).await
            };
            match result {
                Poll::Ready(result) => break result.map(|result| (result, buffer)),
                Poll::Pending => drop(buffer),
            }
            // No message was consumed. Return scarce storage before waiting, and retry after advisory readiness without reserving a buffer for an idle connection.
            if let Err(error) = scheduler
                .socket_monitor_wait(&mut self.monitor, cancellation)
                .await
            {
                break Err(error);
            }
        };
        match result {
            Ok((result, mut buffer)) if !self.stream || result.bytes != 0 => {
                buffer.truncate(result.bytes);
                Ok(Some(OwnedSocketRecvMessage::new(result, buffer)))
            }
            Ok(_) => {
                self.terminal = true;
                Ok(None)
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

    /// Stop reception and await monitor retirement without cancellation. Repeated close is safe; returned buffers remain valid. Drop the receiver after success to release its socket owner before exclusive socket close.
    pub async fn close<S>(&mut self, scheduler: &S) -> std::io::Result<()>
    where
        S: SocketMonitoring,
        S::Socket: 'static,
        M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
    {
        scheduler.socket_monitor_close(&mut self.monitor).await?;
        self.terminal = true;
        Ok(())
    }
}

impl<S, M, P> crate::SocketRecvMsgReceiver<S> for SocketReceiver<M, P>
where
    P: BufferPool,
    S: SocketMonitoring,
    S::Socket: 'static,
    M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
{
    type Message = OwnedSocketRecvMessage<P::Buffer>;
    async fn recv(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<OwnedSocketRecvMessage<P::Buffer>>> {
        SocketReceiver::recv(self, scheduler, cancellation).await
    }
    async fn close(&mut self, scheduler: &S) -> io::Result<()> {
        SocketReceiver::close(self, scheduler).await
    }
}
