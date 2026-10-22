// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
impl SocketReceiving for IoUringSelector {
    type PooledReceiver =
        SocketReceiver<SocketMonitor<NativeSocket, crate::IoUringMonitorRegistration>>;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: BufferPool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::PooledReceiver> {
        cancellation.check()?;
        self.check_open()?;
        let retained = super::socket::retain_socket(socket);
        let kind = syscall("socket-type", || retained.get_ref().r#type())?;
        let kind = if kind == socket2::Type::STREAM {
            SocketType::Stream
        } else {
            SocketType::Other(kind.into())
        };
        self.socket_monitor(socket, Interest::Readable)
            .and_then(|monitor| SocketReceiver::new(monitor, pool, kind, flags))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
