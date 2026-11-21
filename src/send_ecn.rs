// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// The ECN marking of every datagram a packet socket sends: the ECN bits of `IP_TOS` on IPv4 and `IPV6_TCLASS` on IPv6. Setting reads the current byte first and preserves its DSCP bits.
///
/// A socket option marks every datagram, so use it only on a socket carrying one path whose ECN use is validated, such as a connected client socket, and set `Ecn::NotEct` when that path fails validation. On a dual-stack IPv6 socket, setting also configures the IPv4 side the platform uses for IPv4-mapped destinations, and fails if any of those calls fails, leaving any option already changed as the system holds it; getting returns the ECN bits of `IPV6_TCLASS`. Stream sockets reject the option with `ErrorKind::InvalidInput`, and a backend or platform that cannot set ECN returns `ErrorKind::Unsupported`.
#[derive(Clone, Copy, Debug)]
pub struct SendEcn;

impl crate::SocketOption for SendEcn {
    type Value = crate::Ecn;
}
