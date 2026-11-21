// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Boolean ECN reporting on a packet socket: `IP_RECVTOS` on IPv4 and `IPV6_RECVTCLASS` on IPv6. While enabled, each received datagram reports its marking through `SocketRecvResult::ecn` and `SocketRecvMessage::ecn`, for every reader and receiver of the socket.
///
/// On a dual-stack IPv6 socket, setting also configures the IPv4 side the platform uses for IPv4-mapped traffic, and fails if any of those calls fails, leaving any option already changed as the system holds it; getting returns the IPv6 option's value. Stream sockets reject the option with `ErrorKind::InvalidInput`, since TCP manages ECN itself, and a backend or platform that cannot report ECN returns `ErrorKind::Unsupported`.
#[derive(Clone, Copy, Debug)]
pub struct ReceiveEcn;

impl crate::SocketOption for ReceiveEcn {
    type Value = bool;
}
