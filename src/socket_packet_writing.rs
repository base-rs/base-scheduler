// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{MonitoredSocketPacketWriter, SocketMonitoring};
use std::io;

/// Construct an independently monitored sequential packet writer bound to this scheduler. Separate reader and writer handles permit concurrent reception and sending.
pub trait SocketPacketWriting: SocketMonitoring + Sync {
    fn socket_packet_writer(
        &self,
        socket: &Self::Socket,
    ) -> io::Result<MonitoredSocketPacketWriter<'_, Self>>
    where
        Self: Sized,
        Self::Socket: 'static,
    {
        MonitoredSocketPacketWriter::new(self, socket)
    }
}

impl<S: SocketMonitoring + Sync> SocketPacketWriting for S {}
