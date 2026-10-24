// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{SocketRecvFlags, SocketRecvResult, SocketSendFlags};
use core_scheduler_selector::NativeSocket;
use socket2::{SockAddr, Socket};
use std::{
    io::{self, IoSlice, IoSliceMut},
    net::SocketAddr,
    os::fd::AsRawFd,
    sync::Arc,
};

type RegisteredSocket = Arc<async_io::Async<Socket>>;

// Private dispatch permits resource handles to retain the existing native registration without exposing another public monitor or cloning protocol.
pub(crate) trait SocketSource: Send + Sync {
    fn retain_socket(&self) -> RegisteredSocket;
    fn try_recv(&self, bytes: &mut [u8], flags: SocketRecvFlags) -> io::Result<usize>;
    fn try_send(&self, bytes: &[u8], flags: SocketSendFlags) -> io::Result<usize>;
    fn try_recvmsg(
        &self,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
    ) -> io::Result<SocketRecvResult>;
    fn try_sendmsg(
        &self,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddr>,
        flags: SocketSendFlags,
    ) -> io::Result<usize>;
}
impl SocketSource for NativeSocket {
    fn retain_socket(&self) -> RegisteredSocket {
        // SAFETY: Backend state retains the owner through registration and operation retirement, with no raw-handle or nonblocking-mode changes.
        unsafe { self.retain_native() }
    }
    fn try_recv(&self, bytes: &mut [u8], flags: SocketRecvFlags) -> io::Result<usize> {
        self.try_recv(bytes, flags)
    }
    fn try_send(&self, bytes: &[u8], flags: SocketSendFlags) -> io::Result<usize> {
        self.try_send(bytes, flags)
    }
    fn try_recvmsg(
        &self,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
    ) -> io::Result<SocketRecvResult> {
        self.try_recvmsg(buffers, flags)
    }
    fn try_sendmsg(
        &self,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddr>,
        flags: SocketSendFlags,
    ) -> io::Result<usize> {
        self.try_sendmsg(buffers, address, flags)
    }
}
impl SocketSource for RegisteredSocket {
    fn retain_socket(&self) -> RegisteredSocket {
        Arc::clone(self)
    }
    #[inline]
    fn try_recv(&self, bytes: &mut [u8], flags: SocketRecvFlags) -> io::Result<usize> {
        // SAFETY: The mutable initialized range remains valid through this synchronous call. MSG_DONTWAIT never extends payload access beyond return.
        let bytes =
            unsafe { std::slice::from_raw_parts_mut(bytes.as_mut_ptr().cast(), bytes.len()) };
        self.get_ref()
            .recv_with_flags(bytes, libc::MSG_DONTWAIT | recv_flags(flags))
    }
    #[inline]
    fn try_send(&self, bytes: &[u8], flags: SocketSendFlags) -> io::Result<usize> {
        self.get_ref()
            .send_with_flags(bytes, libc::MSG_DONTWAIT | send_flags(flags))
    }
    #[inline]
    fn try_recvmsg(
        &self,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
    ) -> io::Result<SocketRecvResult> {
        validate(buffers.len(), buffers.iter().map(|buffer| buffer.len()))?;
        let empty = libc::iovec {
            iov_base: std::ptr::null_mut(),
            iov_len: 0,
        };
        let (vectors, count) = if buffers.is_empty() {
            (&empty as *const libc::iovec, 1)
        } else {
            (buffers.as_ptr().cast(), buffers.len())
        };
        // SAFETY: IoSliceMut is ABI-compatible with iovec; header and exclusive payload pointers remain live through recvmsg and are not retained afterward.
        let ((bytes, truncated, control_truncated), address) = unsafe {
            SockAddr::try_init(|address, length| {
                let mut header: libc::msghdr = std::mem::zeroed();
                header.msg_iov = vectors.cast_mut();
                header.msg_iovlen = count;
                header.msg_name = address.cast();
                header.msg_namelen = *length;
                let bytes = result_count(libc::recvmsg(
                    self.as_raw_fd(),
                    &mut header,
                    libc::MSG_DONTWAIT | recv_flags(flags),
                ))?;
                *length = header.msg_namelen;
                Ok((
                    bytes,
                    header.msg_flags & libc::MSG_TRUNC != 0,
                    header.msg_flags & libc::MSG_CTRUNC != 0,
                ))
            })
        }?;
        Ok(SocketRecvResult {
            bytes,
            address: address.as_socket(),
            truncated,
            control_truncated,
        })
    }
    #[inline]
    fn try_sendmsg(
        &self,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddr>,
        flags: SocketSendFlags,
    ) -> io::Result<usize> {
        validate(buffers.len(), buffers.iter().map(|buffer| buffer.len()))?;
        let empty = libc::iovec {
            iov_base: std::ptr::null_mut(),
            iov_len: 0,
        };
        let (vectors, count) = if buffers.is_empty() {
            (&empty as *const libc::iovec, 1)
        } else {
            (buffers.as_ptr().cast(), buffers.len())
        };
        let address = address.map(SockAddr::from);
        // SAFETY: IoSlice is ABI-compatible with iovec. Header, address and readable payloads remain live for this synchronous sendmsg only.
        let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
        header.msg_iov = vectors.cast_mut();
        header.msg_iovlen = count;
        if let Some(address) = &address {
            header.msg_name = address.as_ptr().cast_mut().cast();
            header.msg_namelen = address.len();
        }
        // SAFETY: All header pointers refer to the live values above; nonblocking send does not retain them after return.
        result_count(unsafe {
            libc::sendmsg(
                self.as_raw_fd(),
                &header,
                libc::MSG_DONTWAIT | send_flags(flags),
            )
        })
    }
}
fn validate(count: usize, mut lengths: impl Iterator<Item = usize>) -> io::Result<()> {
    if count > i32::MAX as usize
        || lengths
            .try_fold(0usize, usize::checked_add)
            .is_none_or(|length| length > i32::MAX as usize)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message exceeds native length limits",
        ));
    }
    Ok(())
}
fn send_flags(flags: SocketSendFlags) -> i32 {
    libc::MSG_NOSIGNAL
        | if flags.dont_route {
            libc::MSG_DONTROUTE
        } else {
            0
        }
}
fn recv_flags(flags: SocketRecvFlags) -> i32 {
    if flags.peek { libc::MSG_PEEK } else { 0 }
}
fn result_count(result: isize) -> io::Result<usize> {
    if result >= 0 {
        Ok(result as usize)
    } else {
        Err(io::Error::last_os_error())
    }
}
