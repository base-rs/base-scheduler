// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferLease, BufferPool, Cancellation, MonitorStorage, OperationResult,
    RecvMsgMultishotMessage, SocketMonitorState, SocketMonitoring, SocketRecvFlags, SocketType,
};
use std::io::{self, IoSliceMut};

/// A demand-driven receiver composed from a readable socket monitor and a shared pool. Works with general and stream monitors; it does not spawn a task, receive ahead, or allocate per message. Backends offering kernel receive-ahead retain their separate receiver implementation.
pub struct SocketReceiver<Monitor: crate::MonitorState> {
    monitor: MonitorStorage<Monitor>,
    pool: BufferPool,
    flags: SocketRecvFlags,
    stream: bool,
    terminal: bool,
}
impl<M: crate::MonitorState> SocketReceiver<M> {
    /// `kind` must describe the socket: empty datagrams are messages, whereas a zero-length stream receive is EOF. Peeking is rejected because a receiver must consume its messages.
    pub fn new(
        monitor: MonitorStorage<M>,
        pool: BufferPool,
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
    ) -> OperationResult<Option<RecvMsgMultishotMessage<BufferLease>>>
    where
        S: SocketMonitoring,
        S::Socket: 'static,
        M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
    {
        cancellation.check()?;
        if self.terminal {
            return Ok(None);
        }
        let mut buffer = self.pool.acquire(cancellation).await?;
        let result = scheduler
            .socket_monitor_recvmsg_poll(
                &mut self.monitor,
                &mut [IoSliceMut::new(buffer.as_mut())],
                self.flags,
                cancellation,
            )
            .await;
        match result {
            Ok(result) if !self.stream || result.bytes != 0 => {
                buffer.truncate(result.bytes);
                Ok(Some(RecvMsgMultishotMessage { result, buffer }))
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

    pub async fn close<S>(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<()>
    where
        S: SocketMonitoring,
        S::Socket: 'static,
        M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
    {
        cancellation.check()?;
        self.terminal = true;
        scheduler
            .socket_monitor_close(&mut self.monitor, cancellation)
            .await
    }
}

impl<S, M> crate::SocketRecvMsgReceiver<S> for SocketReceiver<M>
where
    S: SocketMonitoring,
    S::Socket: 'static,
    M: SocketMonitorState<S::Socket, S::MonitorRegistration>,
{
    type Buffer = BufferLease;
    async fn recv(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<RecvMsgMultishotMessage<BufferLease>>> {
        SocketReceiver::recv(self, scheduler, cancellation).await
    }
    async fn close(&mut self, scheduler: &S) -> io::Result<()> {
        SocketReceiver::close(self, scheduler, &Cancellation::never())
            .await
            .map_err(Into::into)
    }
}
