// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioScheduler, TokioSocket, TokioTask, socket_io};
use core_scheduler::backend::{SocketStreamOutput, SocketStreamOutputting, SocketWriteState};
use core_scheduler::{Cancellation, Interest, IoBuf, OperationError, OperationResult};
use std::{
    future::{Future, poll_fn},
    io,
    marker::PhantomData,
    pin::pin,
    task::Poll,
};

/// Ordered output which completes ready sends inline and delegates stalled logical writes to the adapter's task service.
pub struct TokioStreamWriter<B> {
    socket: TokioSocket,
    state: SocketWriteState<TokioTask<OperationResult<()>>>,
    buffer: PhantomData<fn(B)>,
}

impl SocketStreamOutputting for TokioScheduler {
    type Writer<B: IoBuf> = TokioStreamWriter<B>;

    fn socket_stream_output<B: IoBuf>(&self, socket: &TokioSocket) -> io::Result<Self::Writer<B>> {
        self.check_socket(socket)?;
        if socket_io::recvmsg_socket_type(socket.get_ref())? != core_scheduler::SocketType::Stream {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ordered writers require a stream socket",
            ));
        }
        Ok(TokioStreamWriter {
            socket: socket.retain(),
            state: SocketWriteState::default(),
            buffer: PhantomData,
        })
    }
}

impl<B: IoBuf> SocketStreamOutput<TokioScheduler> for TokioStreamWriter<B> {
    type Buffer = B;

    async fn submit(
        &mut self,
        scheduler: &TokioScheduler,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<(), (OperationError, B)> {
        if let Err(error) = scheduler.check_socket(&self.socket).map_err(Into::into) {
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
        if let Err(error) = scheduler.check_socket(&self.socket) {
            return Err((error.into(), buffer));
        }
        let length = buffer.as_ref().len();
        if length == 0 {
            return Ok(());
        }
        // Admission precedes the first syscall. Errors from now on belong to this accepted logical write and are reported by flush or the next submission.
        let offset = match send(self.socket.get_ref(), buffer.as_ref()) {
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
        let cancellation = cancellation.clone();
        let stopping = self.state.cancellation();
        let writing = async move {
            let mut buffer = buffer;
            let result =
                write_remaining(&socket, &mut buffer, offset, &cancellation, &stopping).await;
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
        scheduler: &TokioScheduler,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        scheduler.check_socket_compatibility(&self.socket)?;
        self.state.flush(cancellation).await
    }

    async fn close(&mut self, scheduler: &TokioScheduler) -> io::Result<()> {
        scheduler.check_socket_compatibility(&self.socket)?;
        self.state.close().await
    }
}

async fn write_remaining<B: IoBuf>(
    socket: &TokioSocket,
    buffer: &mut B,
    mut offset: usize,
    cancellation: &Cancellation,
    stopping: &Cancellation,
) -> OperationResult<()> {
    let mut cancelled = pin!(cancellation.cancelled());
    let mut stopped = pin!(stopping.cancelled());
    while offset < buffer.as_ref().len() {
        cancellation.check()?;
        stopping.check()?;
        let bytes = &buffer.as_ref()[offset..];
        let operation = socket.io_with(Interest::Writable, |socket| send(socket, bytes), false);
        let mut operation = pin!(operation);
        let count = poll_fn(|context| {
            if let Poll::Ready(result) = operation.as_mut().poll(context) {
                return Poll::Ready(result.map_err(OperationError::from));
            }
            if cancelled.as_mut().poll(context).is_ready()
                || stopped.as_mut().poll(context).is_ready()
            {
                return Poll::Ready(Err(core_scheduler::Cancelled.into()));
            }
            Poll::Pending
        })
        .await;
        match count {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
            Ok(count) => offset += count,
            Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::Interrupted => {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn send(socket: &socket2::Socket, bytes: &[u8]) -> io::Result<usize> {
    #[cfg(test)]
    if tests::BLOCK_SENDS.with(std::cell::Cell::get) {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    #[cfg(test)]
    if let Some(result) = tests::SEND_RESULTS.with(|results| results.borrow_mut().pop_front()) {
        return result;
    }
    socket_io::send_poll(socket, bytes, core_scheduler::SocketSendFlags::default())
}

#[cfg(test)]
mod tests;

impl core_scheduler::SocketStreamWriting for TokioScheduler {
    fn socket_stream_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<
        impl core_scheduler::SocketStreamWriter<
            Buffer = <Self::Pool as core_scheduler::BufferPool>::Buffer,
        > + 'scheduler,
    >
    where
        Self::Socket: 'static,
    {
        core_scheduler::backend::BoundSocketStreamWriter::new(self, socket)
    }

    fn socket_stream_writer_with_buffer<'scheduler, B: IoBuf>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl core_scheduler::SocketStreamWriter<Buffer = B> + 'scheduler>
    where
        Self::Socket: 'static,
    {
        core_scheduler::backend::BoundSocketStreamWriter::new(self, socket)
    }
}
