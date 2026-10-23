// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{NativeSocket, ReadinessSelector};
use core_scheduler::*;
impl SocketReceiving for ReadinessSelector {
    type PooledReceiver = SocketReceiver<
        SocketMonitor<NativeSocket, <Self as SocketMonitoring>::MonitorRegistration>,
    >;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::PooledReceiver> {
        cancellation.check()?;
        self.check_open()?;
        let kind = crate::socket_io::recvmsg_socket_type(socket.get_ref())?;
        self.socket_monitor(socket, Interest::Readable)
            .and_then(|monitor| SocketReceiver::new(monitor, pool, kind, flags))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
