// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::SocketOperation;
use crate::SocketAddress;
use std::io;

/// Inspect the connected peer address of a socket.
pub trait SocketPeerAddress: SocketOperation {
    /// Return the remote address of a connected socket.
    fn socket_peer_address(&self, socket: &Self::Socket) -> io::Result<SocketAddress>;
}
