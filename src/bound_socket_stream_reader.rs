// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::backend::{MonitorStorage, SocketMonitoring, SocketStreamMonitor};
use crate::{
    BufferPool, BufferPoolOptions, BufferPooling, Cancellation, Interest, OperationResult,
    ReceiveBuffer, SocketRecvFlags, SocketStreamReader,
};
use std::{
    future::{Future, poll_fn},
    io::{self, IoSliceMut},
    pin::pin,
    task::Poll,
};

/// A scheduler-bound stream reader with reusable registration and lazily allocated receive storage. Normally obtained through [`SocketStreamReading`](crate::SocketStreamReading).
pub struct BoundSocketStreamReader<'a, S: SocketMonitoring + BufferPooling + ?Sized>
where
    S::Socket: 'static,
{
    scheduler: &'a S,
    monitor: MonitorStorage<SocketStreamMonitor<S::Socket, S::MonitorRegistration>>,
    pool: Option<S::Pool>,
    eof: bool,
    closed: bool,
}

impl<'a, S> BoundSocketStreamReader<'a, S>
where
    S: SocketMonitoring + BufferPooling + Sync + ?Sized,
    S::Socket: 'static,
{
    /// Construct the default backend composition without submitting payload I/O. Applications normally use the scheduler factory.
    pub fn new(scheduler: &'a S, socket: &S::Socket) -> io::Result<Self> {
        if scheduler.socket_type(socket)? != crate::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "stream readers require a stream socket",
            ));
        }
        Ok(Self {
            scheduler,
            monitor: scheduler.socket_stream_monitor(socket, Interest::Readable)?,
            pool: None,
            eof: false,
            closed: false,
        })
    }

    /// Select shared storage for subsequent owned reads. Already returned chunks retain their original pool. Borrowed reads require no pool; otherwise the first owned read creates two reusable 64 KiB slots.
    pub fn with_pool(mut self, pool: S::Pool) -> Self {
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

impl<S> SocketStreamReader for BoundSocketStreamReader<'_, S>
where
    S: SocketMonitoring + BufferPooling + Sync + ?Sized,
    S::Socket: 'static,
{
    type Buffer = <S::Pool as BufferPool>::Buffer;

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
                let mut reading = pin!(self.scheduler.socket_monitor_recv_poll(
                    &mut self.monitor,
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
        if self.eof || buffer.is_empty() {
            cancellation.check()?;
            return Ok(0);
        }
        let count = self
            .scheduler
            .socket_monitor_recv_poll(
                &mut self.monitor,
                buffer,
                SocketRecvFlags::default(),
                cancellation,
            )
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
            .socket_monitor_recvmsg_poll(
                &mut self.monitor,
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

    async fn close(&mut self) -> io::Result<()> {
        self.closed = true;
        self.scheduler.socket_monitor_close(&mut self.monitor).await
    }
}
