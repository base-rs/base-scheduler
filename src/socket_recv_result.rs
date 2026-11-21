// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Ecn, SocketAddress};

/// Metadata from one receive. Only `bytes` bytes, spread across the buffers in order, were written; their initialized lengths remain unchanged. The default is an empty receive: no bytes, no address and no truncation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SocketRecvResult {
    /// The number of bytes written into the supplied buffers.
    pub bytes: usize,
    /// The sender when supplied by the OS, usually for datagrams. Stream receives may return `None`; use `socket_peer_address` for the connected peer.
    pub address: Option<SocketAddress>,
    /// A datagram exceeded the total buffer length. With nonzero capacity, remaining bytes were discarded unless this was a peek. Zero-capacity receive behavior is OS-specific. `bytes` is the copied length, not the original datagram length.
    pub truncated: bool,
    /// The OS reported discarded ancillary data. Packet receives supply control storage for one ECN marking only, so another control message, such as one enabled outside the backend, sets this. Control messages other than the marking are not exposed.
    pub control_truncated: bool,
    /// The datagram's ECN marking, when the system supplied it: while the socket's reporting is enabled, through `ReceiveEcn` or otherwise. `None` when reporting is disabled, the system supplied no marking, the control data was truncated, or for stream receives; absence is never reported as `Ecn::NotEct`.
    pub ecn: Option<Ecn>,
}
