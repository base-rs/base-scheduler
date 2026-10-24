// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BoundSocketPacketReader, BufferPooling, SocketMonitoring};
use std::io;

/// Construct a sequential packet reader bound to this scheduler. Borrowed reads reuse a persistent monitor; owned reads lazily obtain reusable pooled storage.
pub trait SocketPacketReading: SocketMonitoring + BufferPooling + Sync {
    fn socket_packet_reader(
        &self,
        socket: &Self::Socket,
    ) -> io::Result<BoundSocketPacketReader<'_, Self>>
    where
        Self: Sized,
        Self::Socket: 'static,
    {
        BoundSocketPacketReader::new(self, socket)
    }
}

impl<S: SocketMonitoring + BufferPooling + Sync> SocketPacketReading for S {}
