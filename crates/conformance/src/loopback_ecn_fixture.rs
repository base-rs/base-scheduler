// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::EcnOptionFixture;
use base_scheduler::{
    Cancellation, ReceiveEcn, SendEcn, SocketAddress, SocketBind, SocketClose, SocketDomain,
    SocketLocalAddress, SocketOpen, SocketOperation, SocketOptions, SocketProtocol, SocketType,
};
#[cfg(unix)]
use std::net::{Ipv4Addr, SocketAddrV6};

/// The packet socket family a [`LoopbackEcnFixture`] opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EcnFamily {
    Ipv4,
    /// IPv6 sockets with `IPV6_V6ONLY` enabled.
    Ipv6,
    /// IPv6 sockets with `IPV6_V6ONLY` disabled, the sender reaching the receiver at an IPv4-mapped address. Needs native access.
    #[cfg(unix)]
    DualStack,
}

/// A [`PacketEcnFixture`](crate::PacketEcnFixture) over loopback, opened through the scheduler's own socket capabilities: a bound sender and receiver of one family, and a stream socket. Its native setters borrow the sockets' descriptors through `AsFd`.
pub struct LoopbackEcnFixture<'s, S: SocketOperation> {
    scheduler: &'s S,
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "android", target_os = "macos")),
        allow(dead_code)
    )]
    family: EcnFamily,
    sender: S::Socket,
    receiver: S::Socket,
    stream: S::Socket,
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "android", target_os = "macos")),
        allow(dead_code)
    )]
    destination: SocketAddress,
}

#[cfg(unix)]
impl<'s, S> LoopbackEcnFixture<'s, S>
where
    S: SocketOpen + SocketBind + SocketLocalAddress + SocketClose,
    S::Socket: std::os::fd::AsFd,
{
    /// Open the fixture's sockets.
    pub async fn open(scheduler: &'s S, family: EcnFamily) -> Self {
        Self::open_with(scheduler, family, |socket, only_v6| {
            let value = libc::c_int::from(only_v6);
            set(
                descriptor(socket),
                libc::IPPROTO_IPV6,
                libc::IPV6_V6ONLY,
                value,
            );
        })
        .await
    }
}

#[cfg(not(unix))]
impl<'s, S> LoopbackEcnFixture<'s, S>
where
    S: SocketOpen + SocketBind + SocketLocalAddress + SocketClose,
{
    /// Open the fixture's sockets, leaving IPv6 sockets with the system's default `IPV6_V6ONLY`, which is enabled on Windows.
    pub async fn open(scheduler: &'s S, family: EcnFamily) -> Self {
        Self::open_with(scheduler, family, |_, _| {}).await
    }
}

impl<'s, S> LoopbackEcnFixture<'s, S>
where
    S: SocketOpen + SocketBind + SocketLocalAddress + SocketClose,
{
    async fn open_with(
        scheduler: &'s S,
        family: EcnFamily,
        only_v6: impl Fn(&S::Socket, bool),
    ) -> Self {
        let never = Cancellation::never();
        let (domain, local): (_, SocketAddress) = match family {
            EcnFamily::Ipv4 => (SocketDomain::Ipv4, "127.0.0.1:0".parse().unwrap()),
            EcnFamily::Ipv6 => (SocketDomain::Ipv6, "[::1]:0".parse().unwrap()),
            #[cfg(unix)]
            EcnFamily::DualStack => (SocketDomain::Ipv6, "[::]:0".parse().unwrap()),
        };
        let datagram = || async {
            let socket = scheduler
                .socket_open(
                    domain,
                    SocketType::Datagram,
                    Some(SocketProtocol::Udp),
                    &never,
                )
                .await
                .unwrap();
            if domain == SocketDomain::Ipv6 {
                only_v6(&socket, family == EcnFamily::Ipv6);
            }
            scheduler.socket_bind(&socket, local, &never).await.unwrap();
            socket
        };
        let sender = datagram().await;
        let receiver = datagram().await;
        let stream = scheduler
            .socket_open(
                domain,
                SocketType::Stream,
                Some(SocketProtocol::Tcp),
                &never,
            )
            .await
            .unwrap();
        let address = scheduler.socket_local_address(&receiver).unwrap();
        let destination = match family {
            #[cfg(unix)]
            EcnFamily::DualStack => {
                SocketAddrV6::new(Ipv4Addr::LOCALHOST.to_ipv6_mapped(), address.port(), 0, 0).into()
            }
            _ => address,
        };
        Self {
            scheduler,
            family,
            sender,
            receiver,
            stream,
            destination,
        }
    }

    /// Close the fixture's sockets.
    pub async fn close(self) {
        for socket in [self.sender, self.receiver, self.stream] {
            self.scheduler.socket_close_submit(socket).await.unwrap();
        }
    }
}

impl<S> EcnOptionFixture for LoopbackEcnFixture<'_, S>
where
    S: SocketOptions<ReceiveEcn> + SocketOptions<SendEcn>,
{
    type Scheduler = S;
    fn scheduler(&self) -> &S {
        self.scheduler
    }
    fn packet(&self) -> &S::Socket {
        &self.receiver
    }
    fn stream(&self) -> &S::Socket {
        &self.stream
    }
}

#[cfg(unix)]
fn descriptor(socket: &impl std::os::fd::AsFd) -> std::os::fd::RawFd {
    std::os::fd::AsRawFd::as_raw_fd(&socket.as_fd())
}

#[cfg(unix)]
fn set(descriptor: std::os::fd::RawFd, level: libc::c_int, name: libc::c_int, value: libc::c_int) {
    // SAFETY: The value is a live `c_int` of the length passed.
    let result = unsafe {
        libc::setsockopt(
            descriptor,
            level,
            name,
            (&value as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as _,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod native {
    use super::{EcnFamily, LoopbackEcnFixture, descriptor, set};
    use crate::{EcnRole, PacketEcnFixture};
    use base_scheduler::{
        ReceiveEcn, SendEcn, SocketAddress, SocketOperation, SocketOptions, SocketPacketReading,
        SocketPacketWriting,
    };

    fn get(descriptor: std::os::fd::RawFd, level: libc::c_int, name: libc::c_int) -> libc::c_int {
        let mut value: libc::c_int = 0;
        let mut length = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: The value and length are live and describe a `c_int`.
        let result = unsafe {
            libc::getsockopt(
                descriptor,
                level,
                name,
                (&mut value as *mut libc::c_int).cast(),
                &mut length,
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        value
    }

    impl<S: SocketOperation<Socket: std::os::fd::AsFd>> LoopbackEcnFixture<'_, S> {
        fn descriptor_of(&self, role: EcnRole) -> std::os::fd::RawFd {
            descriptor(match role {
                EcnRole::Sender => &self.sender,
                EcnRole::Receiver => &self.receiver,
            })
        }
        // The IPv4 and IPv6 options the family uses.
        fn sides(&self) -> (bool, bool) {
            match self.family {
                EcnFamily::Ipv4 => (true, false),
                EcnFamily::Ipv6 => (false, true),
                // macOS rejects the IPv4 options on an IPv6 socket.
                EcnFamily::DualStack => {
                    (cfg!(any(target_os = "linux", target_os = "android")), true)
                }
            }
        }
    }

    impl<S> PacketEcnFixture for LoopbackEcnFixture<'_, S>
    where
        S: SocketOptions<ReceiveEcn>
            + SocketOptions<SendEcn>
            + SocketPacketReading
            + SocketPacketWriting,
        S::Socket: std::os::fd::AsFd,
    {
        fn socket(&self, role: EcnRole) -> &S::Socket {
            match role {
                EcnRole::Sender => &self.sender,
                EcnRole::Receiver => &self.receiver,
            }
        }
        fn destination(&self) -> SocketAddress {
            self.destination
        }
        fn set_native_receive(&self, role: EcnRole, enabled: bool) {
            let descriptor = self.descriptor_of(role);
            let (v4, v6) = self.sides();
            if v4 {
                set(
                    descriptor,
                    libc::IPPROTO_IP,
                    libc::IP_RECVTOS,
                    enabled.into(),
                );
            }
            if v6 {
                set(
                    descriptor,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_RECVTCLASS,
                    enabled.into(),
                );
            }
        }
        fn native_class(&self, role: EcnRole) -> u8 {
            let descriptor = self.descriptor_of(role);
            match self.family {
                EcnFamily::Ipv4 => get(descriptor, libc::IPPROTO_IP, libc::IP_TOS) as u8,
                _ => get(descriptor, libc::IPPROTO_IPV6, libc::IPV6_TCLASS) as u8,
            }
        }
        fn set_native_class(&self, role: EcnRole, class: u8) {
            let descriptor = self.descriptor_of(role);
            let (v4, v6) = self.sides();
            if v4 {
                set(descriptor, libc::IPPROTO_IP, libc::IP_TOS, class.into());
            }
            if v6 {
                set(
                    descriptor,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_TCLASS,
                    class.into(),
                );
            }
        }
    }
}
