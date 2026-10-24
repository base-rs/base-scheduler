// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::Interest;

pub(super) fn retain_socket(socket: &impl SocketSource) -> RegisteredSocket {
    socket.retain_socket()
}

// O_NONBLOCK sockets can complete with EAGAIN. Retire that attempt before waiting for readiness and retrying on the operation's original ring. No borrowed payload reaches the readiness service.
pub(super) async fn retry_operation<B, T, O, F>(
    selector: &IoUringSelector,
    controller: &Arc<Controller>,
    socket: &impl SocketSource,
    mut buffers: B,
    mut operation: O,
    interest: Interest,
    cancellation: &Cancellation,
) -> (io::Result<T>, B)
where
    O: FnMut(B) -> F,
    F: Future<Output = (io::Result<T>, B)>,
{
    loop {
        if let Err(error) = cancellation.check() {
            return (
                Err(io::Error::new(io::ErrorKind::Interrupted, error)),
                buffers,
            );
        }
        let (result, returned) = operation(buffers).await;
        buffers = returned;
        match result {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if let Err(error) = selector
                    .wait_ready(Arc::clone(controller), socket, interest, cancellation)
                    .await
                {
                    return (Err(error.into()), buffers);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => return (result, buffers),
        }
    }
}

impl IoUringSelector {
    pub(crate) async fn socket_read<B: IoBufMut>(
        &self,
        socket: &impl SocketSource,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let controller = self.route();
        let (result, buffer) = retry_operation(
            self,
            &controller,
            socket,
            buffer,
            |buffer| {
                self.operation(
                    Arc::clone(&controller),
                    retain_socket(socket),
                    buffer,
                    false,
                    |buffer| buffer.as_mut().as_mut_ptr(),
                    cancellation,
                )
            },
            Interest::Readable,
            cancellation,
        )
        .await;
        (operation_result(result, cancellation), buffer)
    }
}

impl IoUringSelector {
    pub(crate) async fn socket_write<B: IoBuf>(
        &self,
        socket: &impl SocketSource,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        if let Err(error) = self.check_open() {
            return (Err(error.into()), buffer);
        }
        // A ready socket can finish synchronously without publishing a request. No access to the owned buffer survives this nonblocking syscall.
        match syscall("socket-write", || {
            socket.try_send(buffer.as_ref(), SocketSendFlags::default())
        }) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return (operation_result(result, cancellation), buffer),
        }
        let controller = self.route();
        let (result, buffer) = retry_operation(
            self,
            &controller,
            socket,
            buffer,
            |buffer| {
                self.operation(
                    Arc::clone(&controller),
                    retain_socket(socket),
                    buffer,
                    true,
                    |buffer| buffer.as_ref().as_ptr().cast_mut(),
                    cancellation,
                )
            },
            Interest::Writable,
            cancellation,
        )
        .await;
        (operation_result(result, cancellation), buffer)
    }
}

#[cfg(test)]
mod tests;
