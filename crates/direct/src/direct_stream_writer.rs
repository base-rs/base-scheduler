// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectScheduler, DirectSocket, DirectTask, socket_io, socket_retry::SocketRetry};
use core_scheduler::{
    Cancellation, IoBuf, OperationError, OperationResult, SocketStreamWriter, SocketStreamWriting,
    SocketWriteState,
};
use std::{
    future::{Future, poll_fn},
    io,
    marker::PhantomData,
    pin::pin,
    sync::Arc,
    task::Poll,
};

/// Ordered output which completes ready sends inline and retries stalled writes on the direct task service.
pub struct DirectStreamWriter<B> {
    socket: DirectSocket,
    state: SocketWriteState<DirectTask<OperationResult<()>>>,
    buffer: PhantomData<fn(B)>,
}

impl SocketStreamWriting for DirectScheduler {
    type Writer<B: IoBuf> = DirectStreamWriter<B>;

    fn socket_stream_writer<B: IoBuf>(&self, socket: &DirectSocket) -> io::Result<Self::Writer<B>> {
        self.socket_retry.check_open()?;
        if socket_io::recvmsg_socket_type(&socket.0)? != core_scheduler::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ordered writers require a stream socket",
            ));
        }
        Ok(DirectStreamWriter {
            socket: DirectSocket(socket.retain()),
            state: SocketWriteState::default(),
            buffer: PhantomData,
        })
    }
}

impl<B: IoBuf> SocketStreamWriter<DirectScheduler> for DirectStreamWriter<B> {
    type Buffer = B;

    async fn submit(
        &mut self,
        scheduler: &DirectScheduler,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<(), (OperationError, B)> {
        if let Err(error) = scheduler.socket_retry.check_open().map_err(Into::into) {
            return Err((error, buffer));
        }
        if let Err(error) = cancellation.check() {
            return Err((error.into(), buffer));
        }
        if let Err(error) = self.state.flush(cancellation).await {
            return Err((error, buffer));
        }
        if let Err(error) = cancellation.check() {
            return Err((error.into(), buffer));
        }
        if let Err(error) = scheduler.socket_retry.check_open() {
            return Err((error.into(), buffer));
        }
        let length = buffer.as_ref().len();
        if length == 0 {
            return Ok(());
        }
        let offset = match send(&self.socket.0, buffer.as_ref()) {
            Ok(count) if count == length => return Ok(()),
            Ok(0) => {
                self.state
                    .fail(io::Error::from(io::ErrorKind::WriteZero).into());
                return Ok(());
            }
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                0
            }
            Err(error) => {
                self.state.fail(error.into());
                return Ok(());
            }
        };
        let socket = self.socket.retain();
        let retry = Arc::clone(&scheduler.socket_retry);
        let cancellation = cancellation.clone();
        let stopping = self.state.cancellation();
        let writing = async move {
            let mut buffer = buffer;
            let result = write_remaining(
                &socket,
                &mut buffer,
                offset,
                &retry,
                &cancellation,
                &stopping,
            )
            .await;
            drop(buffer);
            drop(socket);
            result
        };
        match scheduler.tasks.spawn(writing) {
            Ok(task) => self.state.accept(task),
            Err(error) => self.state.fail(io::Error::other(error.to_string()).into()),
        }
        Ok(())
    }

    async fn flush(
        &mut self,
        _scheduler: &DirectScheduler,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.state.flush(cancellation).await
    }

    async fn close(&mut self, _scheduler: &DirectScheduler) -> io::Result<()> {
        self.state.close().await
    }
}

async fn write_remaining<B: IoBuf>(
    socket: &socket2::Socket,
    buffer: &mut B,
    mut offset: usize,
    retry: &SocketRetry,
    cancellation: &Cancellation,
    stopping: &Cancellation,
) -> OperationResult<()> {
    let mut stopped = pin!(stopping.cancelled());
    while offset < buffer.as_ref().len() {
        cancellation.check()?;
        stopping.check()?;
        match send(socket, &buffer.as_ref()[offset..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(count) => offset += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                let mut waiting = pin!(retry.wait(cancellation));
                poll_fn(|context| {
                    // A retry pulse reports no I/O progress; it must not postpone an already requested stop.
                    if stopped.as_mut().poll(context).is_ready() {
                        return Poll::Ready(Err(core_scheduler::Cancelled.into()));
                    }
                    waiting.as_mut().poll(context)
                })
                .await?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn send(socket: &socket2::Socket, bytes: &[u8]) -> io::Result<usize> {
    #[cfg(test)]
    if let Some(result) = tests::send_result() {
        return result;
    }
    socket_io::send_poll(socket, bytes, core_scheduler::SocketSendFlags::default())
}

#[cfg(test)]
mod tests;
