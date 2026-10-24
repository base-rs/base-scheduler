// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BoundSocketPacketWriter, SocketMonitoring, SocketPacketWriter};
use std::io;

/// Construct sequential packet output bound to this scheduler. Separate reader and writer handles permit concurrent reception and sending; backends can choose their own static writer.
pub trait SocketPacketWriting: SocketMonitoring + Sync {
    fn socket_packet_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketPacketWriter + 'scheduler>
    where
        Self: Sized,
        Self::Socket: 'static,
    {
        BoundSocketPacketWriter::new(self, socket)
    }
}
