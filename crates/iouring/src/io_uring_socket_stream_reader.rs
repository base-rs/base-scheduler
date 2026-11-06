// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
use crate::{IoUringBufferPool, IoUringSelector, MonitorStorage};
use base_scheduler::{
    BufferPool, BufferPoolOptions, BufferPooling, Cancellation, OperationResult, ReceiveBuffer,
    SocketRecvFlags, SocketRecvResult, SocketStreamReader,
};
use base_scheduler_selector::NativeSocket;
use std::{
    future::{Future, poll_fn},
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};

/// A scheduler-bound stream reader with reusable registration and lazily allocated receive storage. Normally obtained through [`SocketStreamReading`](base_scheduler::SocketStreamReading).
pub(crate) struct IoUringSocketStreamReader<'a, const SUPPORT_OOB: bool = false> {
    scheduler: &'a IoUringSelector,
    operations: crate::operation_state::OperationState,
    owned: Option<crate::io_uring_selector::socket_read::State>,
    monitor: MonitorStorage,
    pool: Option<IoUringBufferPool>,
    probe: bool,
    eof: bool,
    closed: bool,
}

impl<'a, const SUPPORT_OOB: bool> IoUringSocketStreamReader<'a, SUPPORT_OOB> {
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    pub(crate) fn new(scheduler: &'a IoUringSelector, socket: &NativeSocket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? != base_scheduler::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream readers require a stream socket",
            ));
        }
        Ok(Self {
            scheduler,
            operations: Default::default(),
            owned: None,
            monitor: scheduler.socket_monitor(socket, Interest::Readable),
            pool: None,
            probe: true,
            eof: false,
            closed: false,
        })
    }

    /// Select shared storage for subsequent owned reads. Already returned chunks retain their original pool. Borrowed reads require no pool; otherwise the first owned read creates two reusable 64 KiB slots.
    pub(crate) fn with_pool(mut self, pool: IoUringBufferPool) -> Self {
        self.pool = Some(pool);
        self
    }

    #[inline]
    fn check_open(&self) -> OperationResult<()> {
        if self.closed {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "stream reader is closed").into())
        } else {
            Ok(())
        }
    }

    // One immediate receive for the `try_` reads: no wait and no readiness.
    fn try_read_with(
        &mut self,
        empty: bool,
        receive: impl FnOnce(&crate::io_uring_selector::RegisteredSocket) -> io::Result<usize>,
    ) -> io::Result<usize> {
        self.check_open()?;
        self.operations.check_idle()?;
        if self.eof || empty {
            return Ok(0);
        }
        let count = receive(self.monitor.as_ref().socket())?;
        if count == 0 {
            self.eof = true;
        }
        Ok(count)
    }
}

impl<const SUPPORT_OOB: bool> SocketStreamReader for IoUringSocketStreamReader<'_, SUPPORT_OOB> {
    type Buffer = <IoUringBufferPool as BufferPool>::Buffer;

    fn with_oob_support<const OOB: bool>(self) -> impl SocketStreamReader<Buffer = Self::Buffer> {
        IoUringSocketStreamReader::<OOB> {
            scheduler: self.scheduler,
            operations: self.operations,
            owned: self.owned,
            monitor: self.monitor,
            pool: self.pool,
            probe: true,
            eof: self.eof,
            closed: self.closed,
        }
    }

    async fn read(&mut self, cancellation: &Cancellation) -> OperationResult<Option<Self::Buffer>> {
        cancellation.check()?;
        if self.closed || self.eof {
            return Ok(None);
        }
        self.operations.check_idle()?;
        if self.pool.is_none() {
            self.pool = Some(
                self.scheduler
                    .buffer_pool(BufferPoolOptions::new(2, 64 * 1024))?,
            );
        }
        let pool = self
            .pool
            .as_ref()
            .expect("owned reads initialize their pool");
        loop {
            // A drained stream would wait before its receive anyway; wait
            // without holding a slot.
            if self
                .scheduler
                .socket_stream_drained(&self.monitor, self.probe)
            {
                self.scheduler
                    .socket_monitor_wait(&mut self.monitor, cancellation)
                    .await?;
            }
            let mut buffer = pool.acquire(cancellation).await?;
            let result = {
                let bytes = buffer.as_mut();
                let capacity = bytes.len();
                let mut reading = pin!(self.scheduler.socket_stream_receive::<SUPPORT_OOB>(
                    &mut self.monitor,
                    &mut self.probe,
                    capacity,
                    cancellation,
                    |socket| crate::retained_socket::recv(
                        socket,
                        bytes,
                        SocketRecvFlags::default()
                    ),
                ));
                poll_fn(|context| Poll::Ready(reading.as_mut().poll(context))).await
            };
            match result {
                Poll::Ready(result) => {
                    let count = result?;
                    if count == 0 {
                        self.eof = true;
                        return Ok(None);
                    }
                    buffer.truncate(count);
                    return Ok(Some(buffer));
                }
                Poll::Pending => drop(buffer),
            }
            self.scheduler
                .socket_monitor_wait(&mut self.monitor, cancellation)
                .await?;
        }
    }

    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.check_open()?;
        self.operations.check_idle()?;
        if self.eof || buffer.is_empty() {
            cancellation.check()?;
            return Ok(0);
        }
        let capacity = buffer.len();
        let count = self
            .scheduler
            .socket_stream_receive::<SUPPORT_OOB>(
                &mut self.monitor,
                &mut self.probe,
                capacity,
                cancellation,
                |socket| crate::retained_socket::recv(socket, buffer, SocketRecvFlags::default()),
            )
            .await?;
        if count == 0 {
            self.eof = true;
        }
        Ok(count)
    }

    fn try_read_into(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let empty = buffer.is_empty();
        self.try_read_with(empty, |socket| {
            crate::retained_socket::recv(socket, buffer, SocketRecvFlags::default())
        })
    }

    fn try_read_vectored(&mut self, buffers: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        let empty = buffers.iter().all(|buffer| buffer.is_empty());
        self.try_read_with(empty, |socket| {
            crate::retained_socket::recvmsg(socket, buffers, SocketRecvFlags::default())
                .map(|result| result.bytes)
        })
    }

    async fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.check_open()?;
        self.operations.check_idle()?;
        if self.eof || buffers.iter().all(|buffer| buffer.is_empty()) {
            cancellation.check()?;
            return Ok(0);
        }
        let capacity = buffers.iter().map(|buffer| buffer.len()).sum();
        let count = self
            .scheduler
            .socket_stream_receive::<SUPPORT_OOB>(
                &mut self.monitor,
                &mut self.probe,
                capacity,
                cancellation,
                |socket| {
                    crate::retained_socket::recvmsg(socket, buffers, SocketRecvFlags::default())
                        .map(|result| result.bytes)
                },
            )
            .await?;
        if count == 0 {
            self.eof = true;
        }
        Ok(count)
    }

    async fn read_vectored_with_flags(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<base_scheduler::SocketRecvResult> {
        self.check_open()?;
        self.operations.check_idle()?;
        if self.eof || buffers.iter().all(|buffer| buffer.is_empty()) {
            cancellation.check()?;
            return Ok(SocketRecvResult::default());
        }
        // A peek consumes nothing, so it reaches the socket first and keeps
        // readiness until `WouldBlock`.
        if flags.peek {
            return self
                .scheduler
                .socket_monitor_recvmsg_poll(&mut self.monitor, buffers, flags, cancellation)
                .await;
        }
        let mut received = SocketRecvResult::default();
        let capacity = buffers.iter().map(|buffer| buffer.len()).sum();
        let count = self
            .scheduler
            .socket_stream_receive::<SUPPORT_OOB>(
                &mut self.monitor,
                &mut self.probe,
                capacity,
                cancellation,
                |socket| {
                    received = crate::retained_socket::recvmsg(socket, buffers, flags)?;
                    Ok(received.bytes)
                },
            )
            .await?;
        self.eof |= count == 0;
        Ok(received)
    }
    async fn read_owned_vectored<Buffers: base_scheduler::IoBufSequenceMut>(
        &mut self,
        buffers: Buffers,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> (OperationResult<base_scheduler::SocketRecvResult>, Buffers) {
        if let Err(error) = self.check_open() {
            return (Err(error), buffers);
        }
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffers);
        }
        if self.eof || (0..buffers.buffer_count()).all(|index| buffers.buffer(index).is_empty()) {
            return (Ok(SocketRecvResult::default()), buffers);
        }
        let buffers = match self.operations.begin(buffers) {
            Ok(buffers) => buffers,
            Err((error, buffers)) => return (Err(error), buffers),
        };
        let (result, buffers) = self
            .scheduler
            .socket_recvmsg(self.monitor.socket(), buffers, flags, cancellation)
            .await;
        // A consuming receive of nothing is EOF; a peek consumes nothing.
        self.eof |= !flags.peek && matches!(&result, Ok(result) if result.bytes == 0);
        (result, buffers.into_inner())
    }
    async fn read_owned<B: base_scheduler::IoBufMut>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> base_scheduler::BufferResult<B> {
        if let Err(error) = self.check_open() {
            return (Err(error), buffer);
        }
        if self.eof || buffer.as_ref().is_empty() {
            return (cancellation.check().map(|()| 0).map_err(Into::into), buffer);
        }
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        let buffer = match self.operations.begin(buffer) {
            Ok(buffer) => buffer,
            Err((error, buffer)) => return (Err(error), buffer),
        };
        let (result, buffer) = self
            .owned
            .get_or_insert_with(Default::default)
            .read(self.scheduler, self.monitor.socket(), buffer, cancellation)
            .await;
        if matches!(result, Ok(0)) {
            self.eof = true;
        }
        (result, self.operations.complete(buffer))
    }
    async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        self.operations.close().await;
        self.scheduler.socket_monitor_close(&mut self.monitor).await
    }
}

#[cfg(test)]
mod tests;
