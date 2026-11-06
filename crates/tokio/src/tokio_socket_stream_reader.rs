// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
use crate::{TokioScheduler, TokioSocket};
use base_scheduler::{
    BufferPool, BufferPoolOptions, BufferPooling, Cancellation, OperationResult, SocketRecvFlags,
    SocketStreamReader,
};
use std::{
    future::{Future, poll_fn},
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};

/// Scheduler-bound stream reader; registration remains private to this backend.
///
/// With OOB support disabled (the default), scalar reads on Linux and Android clear their observed readiness after a positive short receive unless urgent data or closure was reported, as Tokio's own streams do; enabled support retains readiness until `WouldBlock`. Pooled reads keep the conservative policy for both settings.
pub(crate) struct TokioSocketStreamReader<'a, const SUPPORT_OOB: bool = false> {
    scheduler: &'a TokioScheduler,
    socket: TokioSocket,
    pool: Option<base_scheduler::HeapBufferPool>,
    eof: bool,
    closed: bool,
}

impl<'a, const SUPPORT_OOB: bool> TokioSocketStreamReader<'a, SUPPORT_OOB> {
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    pub(crate) fn new(scheduler: &'a TokioScheduler, socket: &TokioSocket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? != base_scheduler::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream readers require a stream socket",
            ));
        }
        Ok(Self {
            scheduler,
            socket: socket.retain(),
            pool: None,
            eof: false,
            closed: false,
        })
    }

    /// Select shared storage for subsequent owned reads. Already returned chunks retain their original pool. Borrowed reads require no pool; otherwise the first owned read creates two reusable 64 KiB slots.
    pub(crate) fn with_pool(mut self, pool: base_scheduler::HeapBufferPool) -> Self {
        self.pool = Some(pool);
        self
    }

    fn check_open(&self) -> io::Result<()> {
        if self.closed {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "stream reader is closed",
            ))
        } else {
            Ok(())
        }
    }
}

/// Never-cancelled reads on an established stream wait in one compact future instead of the generic cancellable operation; there is no listener to register, and the scheduler checks happen here instead. Other reads return `None` and take the generic path.
#[cfg(unix)]
fn established_read<'a>(
    scheduler: &TokioScheduler,
    socket: &'a TokioSocket,
    cancellation: &Cancellation,
) -> OperationResult<Option<crate::tokio_socket::EstablishedRead<'a>>> {
    if !cancellation.is_never() {
        return Ok(None);
    }
    scheduler.check_socket(socket)?;
    Ok(socket.established_read())
}

/// Whether a scalar receive of `count` into `length` bytes proves the queue drained, under the reader's OOB policy. Only Linux and Android clear readiness on that proof.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn short(count: usize, length: usize, support_oob: bool) -> bool {
    !support_oob && 0 < count && count < length
}

// The drain predicate for an established scalar read; only Linux and Android build one that can clear readiness.
#[cfg(unix)]
fn drain_predicate<const SUPPORT_OOB: bool>(length: usize) -> impl Fn(&usize) -> bool + Send {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    return move |&count: &usize| short(count, length, SUPPORT_OOB);
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = length;
        |_: &usize| false
    }
}

impl<const SUPPORT_OOB: bool> SocketStreamReader for TokioSocketStreamReader<'_, SUPPORT_OOB> {
    type Buffer = <base_scheduler::HeapBufferPool as BufferPool>::Buffer;

    fn with_oob_support<const OOB: bool>(self) -> impl SocketStreamReader<Buffer = Self::Buffer> {
        TokioSocketStreamReader::<OOB> {
            scheduler: self.scheduler,
            socket: self.socket,
            pool: self.pool,
            eof: self.eof,
            closed: self.closed,
        }
    }

    async fn read(&mut self, cancellation: &Cancellation) -> OperationResult<Option<Self::Buffer>> {
        cancellation.check()?;
        if self.closed || self.eof {
            return Ok(None);
        }
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
            let mut buffer = pool.acquire(cancellation).await?;
            let result = {
                let mut reading = pin!(self.scheduler.socket_recv_poll(
                    &self.socket,
                    buffer.as_mut(),
                    SocketRecvFlags::default(),
                    cancellation
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
                .socket_wait(&self.socket, Interest::Readable, cancellation)
                .await?;
        }
    }

    async fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.check_open()?;
        if self.eof || buffer.is_empty() {
            cancellation.check()?;
            return Ok(0);
        }
        #[cfg(unix)]
        if let Some(read) = established_read(self.scheduler, &self.socket, cancellation)? {
            let length = buffer.len();
            let count = read
                .read(
                    |socket| {
                        crate::socket_io::recv_poll(socket, buffer, SocketRecvFlags::default())
                    },
                    drain_predicate::<SUPPORT_OOB>(length),
                )
                .await?;
            if count == 0 {
                self.eof = true;
            }
            return Ok(count);
        }
        let count = self
            .scheduler
            .socket_recv_poll_with(
                &self.socket,
                buffer,
                SocketRecvFlags::default(),
                cancellation,
                !SUPPORT_OOB,
            )
            .await?;
        if count == 0 {
            self.eof = true;
        }
        Ok(count)
    }

    fn try_read_into(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check_open()?;
        if self.eof || buffer.is_empty() {
            return Ok(0);
        }
        let count =
            crate::socket_io::recv_poll(self.socket.get_ref(), buffer, SocketRecvFlags::default())?;
        if count == 0 {
            self.eof = true;
        }
        Ok(count)
    }

    async fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.read_vectored_with_flags(buffers, SocketRecvFlags::default(), cancellation)
            .await
            .map(|result| result.bytes)
    }

    async fn read_vectored_with_flags(
        &mut self,
        buffers: &mut [std::io::IoSliceMut<'_>],
        flags: base_scheduler::SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<base_scheduler::SocketRecvResult> {
        self.check_open()?;
        if self.eof || buffers.iter().all(|buffer| buffer.is_empty()) {
            cancellation.check()?;
            return Ok(base_scheduler::SocketRecvResult {
                bytes: 0,
                address: None,
                truncated: false,
                control_truncated: false,
            });
        }
        let result = self
            .scheduler
            .socket_recvmsg_poll(&self.socket, buffers, flags, cancellation)
            .await?;
        if result.bytes == 0 {
            self.eof = true;
        }
        Ok(result)
    }

    async fn read_owned_vectored<Buffers: base_scheduler::IoBufSequenceMut>(
        &mut self,
        buffers: Buffers,
        flags: base_scheduler::SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> (OperationResult<base_scheduler::SocketRecvResult>, Buffers) {
        if let Err(error) = self.check_open() {
            return (Err(error.into()), buffers);
        }
        if self.eof || (0..buffers.buffer_count()).all(|index| buffers.buffer(index).is_empty()) {
            return (
                cancellation
                    .check()
                    .map(|()| base_scheduler::SocketRecvResult {
                        bytes: 0,
                        address: None,
                        truncated: false,
                        control_truncated: false,
                    })
                    .map_err(Into::into),
                buffers,
            );
        }
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffers);
        }
        let (result, buffers) = self
            .scheduler
            .socket_recvmsg(&self.socket, buffers, flags, cancellation)
            .await;
        if matches!(&result, Ok(result) if result.bytes == 0) {
            self.eof = true;
        }
        (result, buffers)
    }

    async fn read_owned<B: base_scheduler::IoBufMut>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> base_scheduler::BufferResult<B> {
        if let Err(error) = self.check_open() {
            return (Err(error.into()), buffer);
        }
        if self.eof || buffer.as_ref().is_empty() {
            return (cancellation.check().map(|()| 0).map_err(Into::into), buffer);
        }
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        #[cfg(unix)]
        match established_read(self.scheduler, &self.socket, cancellation) {
            Err(error) => return (Err(error), buffer),
            Ok(Some(read)) => {
                let mut buffer = buffer;
                let length = buffer.as_mut().len();
                let result = read
                    .read(
                        |socket| crate::socket_io::recv(socket, buffer.as_mut()),
                        drain_predicate::<SUPPORT_OOB>(length),
                    )
                    .await;
                if matches!(result, Ok(0)) {
                    self.eof = true;
                }
                return (result.map_err(Into::into), buffer);
            }
            Ok(None) => (),
        }
        let (result, buffer) = self
            .scheduler
            .socket_read_with(&self.socket, buffer, cancellation, !SUPPORT_OOB)
            .await;
        if matches!(result, Ok(0)) {
            self.eof = true;
        }
        (result, buffer)
    }

    async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        Ok(())
    }
}
