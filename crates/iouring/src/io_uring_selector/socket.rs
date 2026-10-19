// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

pub(super) fn retain_socket(socket: &NativeSocket) -> RegisteredSocket {
    // SAFETY: Every submitted request retains this owner through its original completion, releases it before notification, and never changes the handle or registration.
    unsafe { socket.retain_native() }
}

// O_NONBLOCK sockets can complete with EAGAIN. Retire that attempt before waiting for readiness and retrying on the operation's original ring. No borrowed payload reaches the readiness service.
pub(super) async fn retry_operation<B, T, O, F>(
    selector: &IoUringSelector,
    socket: &NativeSocket,
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
                    .fallback
                    .socket_wait(socket, interest, cancellation)
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

impl SocketRead for IoUringSelector {
    async fn socket_read<B: IoBufMut>(
        &self,
        socket: &Self::Socket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        let controller = self.route();
        let (result, buffer) = retry_operation(
            self,
            socket,
            buffer,
            |buffer| {
                self.operation(
                    Arc::clone(&controller),
                    Resource::Socket(retain_socket(socket)),
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

impl SocketWrite for IoUringSelector {
    async fn socket_write<B: IoBuf>(
        &self,
        socket: &Self::Socket,
        buffer: B,
        cancellation: &Cancellation,
    ) -> BufferResult<B> {
        if let Err(error) = cancellation.check() {
            return (Err(error.into()), buffer);
        }
        if let Err(error) = self.check_open() {
            return (Err(error.into()), buffer);
        }
        // Ready sends need no ring request. The synchronous attempt retains no payload access; a blocked send still transfers ownership to the ring until original completion.
        let bytes = buffer.as_ref();
        match syscall("socket-send", || {
            socket.try_send(
                &bytes[..bytes.len().min(i32::MAX as usize)],
                SocketSendFlags::default(),
            )
        }) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) => {}
            result => return (result.map_err(Into::into), buffer),
        }
        let controller = self.route();
        let (result, buffer) = retry_operation(
            self,
            socket,
            buffer,
            |buffer| {
                self.operation(
                    Arc::clone(&controller),
                    Resource::Socket(retain_socket(socket)),
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
