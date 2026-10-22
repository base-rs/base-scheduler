// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioScheduler, TokioSocket};
use core_scheduler::*;
impl SocketReceiving for TokioScheduler {
    type PooledReceiver =
        SocketReceiver<SocketMonitor<TokioSocket, <Self as SocketMonitoring>::MonitorRegistration>>;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: BufferPool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::PooledReceiver> {
        cancellation.check()?;
        self.check_socket(socket)?;
        let kind = crate::socket_io::recvmsg_socket_type(socket.get_ref())?;
        self.socket_monitor(socket, Interest::Readable)
            .and_then(|monitor| SocketReceiver::new(monitor, pool, kind, flags))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
