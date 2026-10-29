// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
#[cfg(unix)]
use crate::readiness_driver::Registration;
use crate::socket_io;
use crate::{NativeSocket, ReadinessSelector};
use base_scheduler::{
    BufferPool, BufferPoolOptions, BufferPooling, Cancellation, OperationResult, SocketRecvFlags,
    SocketStreamReader,
};
#[cfg(unix)]
use std::sync::Arc;
use std::{
    future::{Future, poll_fn},
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};
#[cfg(unix)]
use std::{pin::Pin, task::Context};

/// Scheduler-bound stream reader; registration remains private to this backend.
pub(crate) struct ReadinessSocketStreamReader<'a, const SUPPORT_OOB: bool = false> {
    scheduler: &'a ReadinessSelector,
    socket: NativeSocket,
    #[cfg(unix)]
    registration: Option<Arc<Registration>>,
    #[cfg(unix)]
    established: bool,
    pool: Option<base_scheduler::HeapBufferPool>,
    eof: bool,
    closed: bool,
}

impl<'a, const SUPPORT_OOB: bool> ReadinessSocketStreamReader<'a, SUPPORT_OOB> {
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    pub(crate) fn new(scheduler: &'a ReadinessSelector, socket: &NativeSocket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? != base_scheduler::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream readers require a stream socket",
            ));
        }
        Ok(Self {
            scheduler,
            socket: socket.retain(),
            #[cfg(unix)]
            registration: None,
            #[cfg(unix)]
            established: false,
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

    // Scalar consuming stream reads may use readiness before their syscall.
    // Message receives retain their separate syscall-first contract.
    #[cfg(unix)]
    async fn read_with(
        &mut self,
        capacity: usize,
        cancellation: &Cancellation,
        mut receive: impl FnMut(&socket2::Socket) -> io::Result<usize> + Send + Unpin,
    ) -> OperationResult<usize> {
        // An established registration needs no setup, and a never-cancelled
        // worker wait observes driver closure itself. Keep its state compact.
        if let (true, Some(group), Some(registration)) = (
            cancellation.is_never(),
            &self.scheduler.readiness,
            &self.registration,
        ) {
            self.scheduler.check_open()?;
            let result = Receive::<_, SUPPORT_OOB> {
                group,
                direction: registration.direction(Interest::Readable),
                socket: self.socket.get_ref(),
                receive,
                capacity,
                probe: !self.established,
                listener: None,
            }
            .await;
            if matches!(result, Ok(count) if count != 0) {
                self.established = true;
            }
            return result.map_err(Into::into);
        }
        let result = {
            let scheduler = self.scheduler;
            let operation = pin!(async {
                let Some(group) = &self.scheduler.readiness else {
                    return self.socket.io(Interest::Readable, receive).await;
                };
                let mut probe = !self.established;
                if self.registration.is_none() {
                    // Preserve immediate errors and retry across registration.
                    let result = receive(self.socket.get_ref());
                    if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
                        return result;
                    }
                    self.registration = Some(
                        self.socket
                            .registrations
                            .get(group, self.socket.get_ref())?,
                    );
                    probe = true;
                }
                Receive::<_, SUPPORT_OOB> {
                    group,
                    direction: self
                        .registration
                        .as_ref()
                        .expect("a waiting stream retains its registration")
                        .direction(Interest::Readable),
                    socket: self.socket.get_ref(),
                    receive,
                    capacity,
                    probe,
                    listener: None,
                }
                .await
            });
            scheduler.cancellable(cancellation, operation).await
        };
        if matches!(result, Ok(count) if count != 0) {
            self.established = true;
        }
        result
    }
    #[cfg(not(unix))]
    async fn read_with(
        &mut self,
        _capacity: usize,
        cancellation: &Cancellation,
        receive: impl FnMut(&socket2::Socket) -> io::Result<usize> + Send,
    ) -> OperationResult<usize> {
        self.scheduler
            .cancellable(cancellation, self.socket.io(Interest::Readable, receive))
            .await
    }
}

// The registered receive loop. Between polls it retains only the borrowed
// socket state and a pending wait's listener.
#[cfg(unix)]
struct Receive<'a, F, const SUPPORT_OOB: bool> {
    group: &'a crate::readiness_driver::DriverGroup,
    direction: &'a crate::readiness_driver::Direction,
    socket: &'a socket2::Socket,
    receive: F,
    capacity: usize,
    probe: bool,
    listener: Option<crate::readiness_driver::Listener<'a>>,
}
#[cfg(unix)]
impl<F, const SUPPORT_OOB: bool> Future for Receive<'_, F, SUPPORT_OOB>
where
    F: FnMut(&socket2::Socket) -> io::Result<usize> + Unpin,
{
    type Output = io::Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            let observed = this.direction.snapshot();
            #[cfg(target_os = "linux")]
            if !SUPPORT_OOB && observed & 4 != 0 {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "TCP urgent data is not supported by this reader",
                )));
            }
            if !this.probe && observed & 1 == 0 {
                #[cfg(test)]
                tests::before_wait();
                if let Err(error) =
                    std::task::ready!(this.direction.poll_wait(this.group, &mut this.listener, cx))
                {
                    return Poll::Ready(Err(error));
                }
                continue;
            }
            this.probe = false;
            match (this.receive)(this.socket) {
                Ok(count) => {
                    // The backend and OOB policy determine when a short
                    // receive is sufficient to clear this generation.
                    this.direction
                        .clear_after_read::<SUPPORT_OOB>(observed, count, this.capacity);
                    return Poll::Ready(Ok(count));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    this.direction.clear(observed);
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
    }
}

impl<const SUPPORT_OOB: bool> SocketStreamReader for ReadinessSocketStreamReader<'_, SUPPORT_OOB> {
    type Buffer = <base_scheduler::HeapBufferPool as BufferPool>::Buffer;

    fn with_oob_support<const OOB: bool>(self) -> impl SocketStreamReader<Buffer = Self::Buffer> {
        ReadinessSocketStreamReader::<OOB> {
            scheduler: self.scheduler,
            socket: self.socket,
            #[cfg(unix)]
            registration: self.registration,
            #[cfg(unix)]
            established: self.established,
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
        let count = self
            .read_with(buffer.len(), cancellation, |socket| {
                socket_io::recv_poll(socket, buffer, SocketRecvFlags::default())
            })
            .await?;
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
        self.check_open()?;
        if self.eof || buffers.iter().all(|buffer| buffer.is_empty()) {
            cancellation.check()?;
            return Ok(0);
        }
        let result = self
            .scheduler
            .socket_recvmsg_poll(
                &self.socket,
                buffers,
                SocketRecvFlags::default(),
                cancellation,
            )
            .await?;
        if result.bytes == 0 {
            self.eof = true;
        }
        Ok(result.bytes)
    }

    async fn read_vectored_with_flags(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
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
        self.scheduler
            .socket_recvmsg_poll(&self.socket, buffers, flags, cancellation)
            .await
    }

    async fn read_owned_vectored<Buffers: base_scheduler::IoBufSequenceMut>(
        &mut self,
        buffers: Buffers,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> (OperationResult<base_scheduler::SocketRecvResult>, Buffers) {
        if let Err(error) = self.check_open() {
            return (Err(error.into()), buffers);
        }
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffers);
        }
        if self.eof || (0..buffers.buffer_count()).all(|index| buffers.buffer(index).is_empty()) {
            return (
                Ok(base_scheduler::SocketRecvResult {
                    bytes: 0,
                    address: None,
                    truncated: false,
                    control_truncated: false,
                }),
                buffers,
            );
        }
        self.scheduler
            .socket_recvmsg(&self.socket, buffers, flags, cancellation)
            .await
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
        let (result, buffer) = self
            .scheduler
            .socket_read(&self.socket, buffer, cancellation)
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

#[cfg(all(test, unix))]
mod tests;
