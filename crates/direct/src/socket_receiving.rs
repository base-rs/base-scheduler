// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{DirectScheduler, DirectSocket};
use core_scheduler::*;
impl SocketReceiving for DirectScheduler {
    type PooledReceiver = SocketReceiver<
        SocketMonitor<DirectSocket, <Self as SocketMonitoring>::MonitorRegistration>,
    >;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::PooledReceiver> {
        cancellation.check()?;
        self.socket_retry.check_open()?;
        let kind = crate::socket_io::recvmsg_socket_type(&socket.0)?;
        self.socket_monitor(socket, Interest::Readable)
            .and_then(|monitor| SocketReceiver::new(monitor, pool, kind, flags))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
