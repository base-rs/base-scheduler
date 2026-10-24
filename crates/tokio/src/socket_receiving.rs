// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{TokioScheduler, TokioSocket};
use core_scheduler::backend::*;
use core_scheduler::*;
use std::io;
impl SocketReceiving for TokioScheduler {
    type PooledReceiver =
        SocketReceiver<SocketMonitor<TokioSocket, <Self as SocketMonitoring>::MonitorRegistration>>;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
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

impl SocketStreamReading for TokioScheduler {
    fn socket_stream_reader<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    {
        BoundSocketStreamReader::new(self, socket)
    }
    fn socket_stream_reader_with_pool<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
        pool: Self::Pool,
    ) -> io::Result<impl SocketStreamReader<Buffer = <Self::Pool as BufferPool>::Buffer> + 'scheduler>
    {
        BoundSocketStreamReader::new(self, socket).map(|reader| reader.with_pool(pool))
    }
}
impl SocketPacketReading for TokioScheduler {
    fn socket_packet_reader<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<
        impl SocketPacketReader<
            Message: SocketRecvMessageMut<Payload = <Self::Pool as BufferPool>::Buffer>,
        > + 'scheduler,
    > {
        BoundSocketPacketReader::new(self, socket)
    }
    fn socket_packet_reader_with_pool<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
        pool: Self::Pool,
    ) -> io::Result<
        impl SocketPacketReader<
            Message: SocketRecvMessageMut<Payload = <Self::Pool as BufferPool>::Buffer>,
        > + 'scheduler,
    > {
        BoundSocketPacketReader::new(self, socket).map(|reader| reader.with_pool(pool))
    }
}
impl SocketPacketWriting for TokioScheduler {
    fn socket_packet_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketPacketWriter + 'scheduler> {
        BoundSocketPacketWriter::new(self, socket)
    }
}
