// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! ECN on packet sockets: the `ReceiveEcn` and `SendEcn` options, and the control storage and parsing every packet receive uses. The readiness selector keeps the same implementation; backends do not depend on each other.

use base_scheduler::Ecn;
use socket2::{Socket, Type};
use std::io;

// Streams are rejected before support is considered: TCP manages ECN itself, and Linux would accept and then ignore the ECN bits of a stream's `IP_TOS`.
fn packet(socket: &Socket) -> io::Result<()> {
    if crate::socket_io::socket_type(socket)? == Type::STREAM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "ECN options apply to packet sockets",
        ));
    }
    Ok(())
}

// A one-shot local shim fails the option call with this index, counting from zero, so a test can fail the second call of a dual-stack set.
#[cfg(test)]
thread_local! {
    pub(crate) static OPTION_FAILURE: std::cell::Cell<Option<(usize, io::ErrorKind)>> = const { std::cell::Cell::new(None) };
}

// Each kernel option call of one set or get, in order.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "android", target_os = "macos")),
    allow(dead_code)
)]
struct Calls {
    #[cfg(test)]
    index: usize,
}

#[cfg_attr(
    not(any(target_os = "linux", target_os = "android", target_os = "macos")),
    allow(dead_code)
)]
impl Calls {
    fn new() -> Self {
        Self {
            #[cfg(test)]
            index: 0,
        }
    }
    fn call<T>(&mut self, call: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        #[cfg(test)]
        {
            let index = self.index;
            self.index += 1;
            if let Some((failing, error)) = OPTION_FAILURE.with(|slot| slot.get()) {
                if failing == index {
                    OPTION_FAILURE.with(|slot| slot.take());
                    return Err(error.into());
                }
            }
        }
        call()
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
mod options {
    use super::Calls;
    use base_scheduler::Ecn;
    use socket2::Socket;
    use std::io;

    const ECN_MASK: u32 = 0b11;

    #[derive(Clone, Copy)]
    enum Family {
        V4,
        /// An IPv6 socket, and whether it also carries IPv4-mapped traffic.
        V6 {
            dual_stack: bool,
        },
    }

    fn family(socket: &Socket) -> io::Result<Family> {
        let address = socket.local_addr()?;
        if address.is_ipv4() {
            Ok(Family::V4)
        } else if address.is_ipv6() {
            Ok(Family::V6 {
                dual_stack: !socket.only_v6()?,
            })
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ECN options apply to IP sockets",
            ))
        }
    }

    fn mark(byte: u32, ecn: Ecn) -> u32 {
        (byte & !ECN_MASK) | u32::from(ecn.bits())
    }

    pub(super) fn set_receive(socket: &Socket, enabled: bool) -> io::Result<()> {
        let mut calls = Calls::new();
        match family(socket)? {
            Family::V4 => calls.call(|| socket.set_recv_tos_v4(enabled)),
            Family::V6 { dual_stack } => {
                calls.call(|| socket.set_recv_tclass_v6(enabled))?;
                // Mapped IPv4 datagrams are reported through the IPv4 option.
                if dual_stack {
                    calls.call(|| socket.set_recv_tos_v4(enabled))?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn receive(socket: &Socket) -> io::Result<bool> {
        let mut calls = Calls::new();
        match family(socket)? {
            Family::V4 => calls.call(|| socket.recv_tos_v4()),
            Family::V6 { .. } => calls.call(|| socket.recv_tclass_v6()),
        }
    }

    pub(super) fn set_send(socket: &Socket, ecn: Ecn) -> io::Result<()> {
        let mut calls = Calls::new();
        let set_v4 = |calls: &mut Calls| {
            let tos = calls.call(|| socket.tos_v4())?;
            calls.call(|| socket.set_tos_v4(mark(tos, ecn)))
        };
        match family(socket)? {
            Family::V4 => set_v4(&mut calls),
            Family::V6 { dual_stack } => {
                let class = calls.call(|| socket.tclass_v6())?;
                calls.call(|| socket.set_tclass_v6(mark(class, ecn)))?;
                // Mapped IPv4 destinations use the IPv4 option.
                if dual_stack {
                    set_v4(&mut calls)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn send(socket: &Socket) -> io::Result<Ecn> {
        let mut calls = Calls::new();
        let byte = match family(socket)? {
            Family::V4 => calls.call(|| socket.tos_v4())?,
            Family::V6 { .. } => calls.call(|| socket.tclass_v6())?,
        };
        Ok(Ecn::from_bits(byte as u8))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
mod options {
    use base_scheduler::Ecn;
    use socket2::Socket;
    use std::io;

    pub(super) fn set_receive(_: &Socket, _: bool) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(super) fn receive(_: &Socket) -> io::Result<bool> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(super) fn set_send(_: &Socket, _: Ecn) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(super) fn send(_: &Socket) -> io::Result<Ecn> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

/// Set `ReceiveEcn`.
pub(crate) fn set_receive(socket: &Socket, enabled: bool) -> io::Result<()> {
    packet(socket)?;
    options::set_receive(socket, enabled)
}

/// Get `ReceiveEcn`: the IPv6 option's value on a dual-stack socket.
pub(crate) fn receive(socket: &Socket) -> io::Result<bool> {
    packet(socket)?;
    options::receive(socket)
}

/// Set `SendEcn`, preserving the DSCP bits.
pub(crate) fn set_send(socket: &Socket, ecn: Ecn) -> io::Result<()> {
    packet(socket)?;
    options::set_send(socket, ecn)
}

/// Get `SendEcn`: the ECN bits of `IPV6_TCLASS` on a dual-stack socket.
pub(crate) fn send(socket: &Socket) -> io::Result<Ecn> {
    packet(socket)?;
    options::send(socket)
}

/// Control storage for one ECN marking message of either form: a one-byte IPv4 TOS or an `int` IPv6 traffic class.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
#[repr(C, align(8))]
pub(crate) struct Control(pub(crate) [u8; CONTROL_SPACE]);

/// `CMSG_SPACE(sizeof(int))`, the room [`Control`] provides.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
// SAFETY: `CMSG_SPACE` only computes an aligned length.
pub(crate) const CONTROL_SPACE: usize =
    unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as u32) } as usize;

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
impl Control {
    /// Zeroed control storage.
    pub(crate) const fn new() -> Self {
        Self([0; CONTROL_SPACE])
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}

/// The marking in `control`, the control data a receive returned, unless the system `truncated` it: another control message can displace the marking, so truncation reports none even if the marking arrived intact. Other control messages are ignored.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
pub(crate) fn parse(control: &[u8], truncated: bool) -> Option<Ecn> {
    if truncated {
        return None;
    }
    let header_size = std::mem::size_of::<libc::cmsghdr>();
    // SAFETY: `CMSG_LEN` and `CMSG_SPACE` only compute aligned lengths.
    let data_offset = unsafe { libc::CMSG_LEN(0) } as usize;
    let mut offset = 0;
    while offset + header_size <= control.len() {
        // SAFETY: `header_size` bytes from `offset` are in bounds; the read is unaligned because control data inside a provided buffer need not be aligned.
        let header: libc::cmsghdr =
            unsafe { std::ptr::read_unaligned(control.as_ptr().add(offset).cast()) };
        let length = header.cmsg_len as usize;
        if length < data_offset || length > control.len() - offset {
            return None;
        }
        let data = &control[offset + data_offset..offset + length];
        if let Some(byte) = marking(header.cmsg_level, header.cmsg_type, data) {
            return Some(Ecn::from_bits(byte));
        }
        // SAFETY: As above.
        offset += unsafe { libc::CMSG_SPACE((length - data_offset) as u32) } as usize;
    }
    None
}

// The TOS or traffic class byte of a marking message, of either form.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn marking(level: libc::c_int, kind: libc::c_int, data: &[u8]) -> Option<u8> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const IPV4_TOS: libc::c_int = libc::IP_TOS;
    #[cfg(target_os = "macos")]
    const IPV4_TOS: libc::c_int = libc::IP_RECVTOS;
    match (level, kind) {
        (libc::IPPROTO_IP, IPV4_TOS) => byte(data),
        (libc::IPPROTO_IPV6, libc::IPV6_TCLASS) => byte(data),
        _ => None,
    }
}

// A one-byte value, or the low byte of an `int`.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn byte(data: &[u8]) -> Option<u8> {
    match data {
        [byte] => Some(*byte),
        [a, b, c, d, ..] => Some(libc::c_int::from_ne_bytes([*a, *b, *c, *d]) as u8),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
