// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{SocketOperation, SocketPacketWriter};
use std::io;

/// Construct sequential packet output bound to this scheduler. Separate reader and writer handles permit concurrent reception and sending; backends can choose their own static writer.
///
/// The returned handle may borrow the scheduler, but never the temporary socket
/// reference. It retains backend access independently, so the socket and handle
/// can move together. Retire and drop handles before exclusively closing the socket.
pub trait SocketPacketWriting: SocketOperation {
    /// Construct a packet writer for `socket` without performing payload I/O. Stream sockets are rejected.
    fn socket_packet_writer<'scheduler>(
        &'scheduler self,
        socket: &Self::Socket,
    ) -> io::Result<impl SocketPacketWriter + 'scheduler + use<'scheduler, Self>>
    where
        Self: Sized,
        Self::Socket: 'static;
}
