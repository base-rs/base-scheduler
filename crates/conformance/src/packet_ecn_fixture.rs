// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{
    ReceiveEcn, SendEcn, SocketAddress, SocketOperation, SocketOptions, SocketPacketReading,
    SocketPacketWriting,
};

/// A packet socket and a stream socket, for the options' rejections. Each case receives a fresh fixture.
pub trait EcnOptionFixture {
    type Scheduler: SocketOptions<ReceiveEcn> + SocketOptions<SendEcn>;
    fn scheduler(&self) -> &Self::Scheduler;
    /// A packet socket; for a [`PacketEcnFixture`], its receiver.
    fn packet(&self) -> &<Self::Scheduler as SocketOperation>::Socket;
    /// A stream socket.
    fn stream(&self) -> &<Self::Scheduler as SocketOperation>::Socket;
}

/// One of a [`PacketEcnFixture`]'s two packet sockets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EcnRole {
    Sender,
    Receiver,
}

/// A bound packet socket pair of one family, IPv4, IPv6 or dual-stack, over loopback, which preserves the TOS byte. Native setters reach the kernel options the interface has no portable options for. Each case receives a fresh fixture whose sockets have the kernel's default ECN options.
pub trait PacketEcnFixture:
    EcnOptionFixture<Scheduler: SocketPacketReading + SocketPacketWriting>
{
    fn socket(&self, role: EcnRole) -> &<Self::Scheduler as SocketOperation>::Socket;
    /// The address at which the sender reaches the receiver: an IPv4-mapped address for a dual-stack pair, so that the IPv4 side of both sockets is exercised.
    fn destination(&self) -> SocketAddress;
    /// Enable or disable the kernel's reporting natively: every option the family uses, both sides of a dual-stack socket.
    fn set_native_receive(&self, role: EcnRole, enabled: bool);
    /// The kernel's TOS byte, or the traffic class on an IPv6 or dual-stack socket.
    fn native_class(&self, role: EcnRole) -> u8;
    /// Set the TOS byte, or the traffic class, natively; on a dual-stack socket, both.
    fn set_native_class(&self, role: EcnRole, class: u8);
}
