// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
use base_scheduler::{SocketRecvFlags, SocketRecvResult, SocketSendFlags};
use std::{
    io,
    net::SocketAddr,
    sync::{Arc, Mutex, OnceLock},
};

/// A native socket shared with backends, which registers with the portable reactor only when a fallback wait first needs it.
#[derive(Debug)]
pub struct RegisteredSocket {
    // Declared first, so the reactor deregisters before the socket closes.
    fallback: OnceLock<async_io::Async<Descriptor>>,
    registering: Mutex<()>,
    socket: socket2::Socket,
}
impl RegisteredSocket {
    /// Own a socket, switching it to nonblocking mode.
    pub fn new(socket: socket2::Socket) -> io::Result<Self> {
        socket.set_nonblocking(true).map(|()| Self {
            fallback: OnceLock::new(),
            registering: Mutex::new(()),
            socket,
        })
    }
    /// The underlying nonblocking socket.
    pub fn get_ref(&self) -> &socket2::Socket {
        &self.socket
    }
    fn fallback(&self) -> io::Result<&async_io::Async<Descriptor>> {
        // Serialize registration: the reactor rejects a second addition of the same descriptor.
        // Only fallback waits get here, so the lock stays off readiness-driven paths.
        let _registering = self.registering.lock().unwrap();
        if let Some(fallback) = self.fallback.get() {
            return Ok(fallback);
        }
        let registered = async_io::Async::new_nonblocking(Descriptor::new(&self.socket));
        #[cfg(test)]
        let registered = tests::registration_result(registered);
        registered.map(|registered| self.fallback.get_or_init(|| registered))
    }
    fn into_inner(self) -> io::Result<socket2::Socket> {
        let Self {
            fallback, socket, ..
        } = self;
        // Report reactor deregistration failure before the descriptor closes.
        match fallback.into_inner() {
            Some(fallback) => fallback.into_inner().map(|_| socket),
            None => Ok(socket),
        }
    }
}

#[cfg(unix)]
impl std::os::fd::AsFd for RegisteredSocket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.socket.as_fd()
    }
}
#[cfg(unix)]
impl std::os::fd::AsRawFd for RegisteredSocket {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.socket.as_raw_fd()
    }
}
#[cfg(windows)]
impl std::os::windows::io::AsSocket for RegisteredSocket {
    fn as_socket(&self) -> std::os::windows::io::BorrowedSocket<'_> {
        self.socket.as_socket()
    }
}
#[cfg(windows)]
impl std::os::windows::io::AsRawSocket for RegisteredSocket {
    fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
        self.socket.as_raw_socket()
    }
}

// A borrowed view of the socket for the portable reactor, which never closes it.
#[derive(Debug)]
struct Descriptor(
    #[cfg(unix)] std::os::fd::RawFd,
    #[cfg(windows)] std::os::windows::io::RawSocket,
);
impl Descriptor {
    fn new(socket: &socket2::Socket) -> Self {
        #[cfg(unix)]
        return Self(std::os::fd::AsRawFd::as_raw_fd(socket));
        #[cfg(windows)]
        return Self(std::os::windows::io::AsRawSocket::as_raw_socket(socket));
    }
}
#[cfg(unix)]
impl std::os::fd::AsFd for Descriptor {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        // SAFETY: RegisteredSocket drops this view before closing its socket.
        unsafe { std::os::fd::BorrowedFd::borrow_raw(self.0) }
    }
}
#[cfg(windows)]
impl std::os::windows::io::AsSocket for Descriptor {
    fn as_socket(&self) -> std::os::windows::io::BorrowedSocket<'_> {
        // SAFETY: RegisteredSocket drops this view before closing its socket.
        unsafe { std::os::windows::io::BorrowedSocket::borrow_raw(self.0) }
    }
}

/// An owned nonblocking socket with lazily retained readiness registrations.
///
/// Usable by any native scheduler instance. Readiness operations retain their buffers in the waiting future without leaving payload pointers in the reactor. Completion backends retain the socket and buffers independently until their operations retire. Finish outstanding operations before exclusive close. Imported TCP resources must remain nonblocking through external aliases.
#[derive(Debug)]
pub struct NativeSocket {
    // Drop registrations before the final native owner. They hold only a weak native reference.
    #[cfg(unix)]
    pub(crate) registrations: Arc<crate::readiness_driver::SocketRegistrations>,
    pub(crate) inner: Arc<RegisteredSocket>,
}
impl NativeSocket {
    /// Retain the registered native socket for another I/O backend.
    ///
    /// # Safety
    /// Keep this owner alive until all submitted operations using its handle have retired, including after cancellation or abandonment. Do not close or replace the raw handle, remove its readiness registrations, including a lazily added portable reactor registration, or change its nonblocking mode. Release the retained owner before reporting completion to callers that may immediately close the socket.
    pub unsafe fn retain_native(&self) -> Arc<RegisteredSocket> {
        Arc::clone(&self.inner)
    }

    pub(crate) fn new(socket: socket2::Socket) -> io::Result<Self> {
        RegisteredSocket::new(socket).map(|inner| {
            let inner = Arc::new(inner);
            Self {
                #[cfg(unix)]
                registrations: Arc::new(crate::readiness_driver::SocketRegistrations::new(&inner)),
                inner,
            }
        })
    }
    pub(crate) fn retain(&self) -> Self {
        Self {
            #[cfg(unix)]
            registrations: Arc::clone(&self.registrations),
            inner: Arc::clone(&self.inner),
        }
    }
    pub(crate) fn into_close(self) -> impl FnOnce() -> io::Result<()> + Send + 'static {
        #[cfg(unix)]
        drop(self.registrations);
        let inner = Arc::try_unwrap(self.inner)
            .expect("cannot close a socket with outstanding operations; await them first");
        move || inner.into_inner().and_then(crate::socket_io::close)
    }
    pub(crate) fn get_ref(&self) -> &socket2::Socket {
        self.inner.get_ref()
    }
    pub(crate) async fn io<Output, Operation>(
        &self,
        interest: Interest,
        mut operation: Operation,
    ) -> io::Result<Output>
    where
        Operation: FnMut(&socket2::Socket) -> io::Result<Output> + Send,
    {
        loop {
            // Preserve immediate progress/errors. A pending descriptor must
            // observe actual readiness before retrying: an unrelated wake of
            // the containing task says nothing about this socket.
            let result = operation(self.get_ref());
            if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
                return result;
            }
            let fallback = self.inner.fallback()?;
            let ready = match interest {
                Interest::Readable => fallback.readable().await,
                Interest::Writable => fallback.writable().await,
            };
            #[cfg(test)]
            let ready = tests::readiness_result(ready);
            ready?;
        }
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.get_ref().local_addr().map(crate::socket_io::address)
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.get_ref().peer_addr().map(crate::socket_io::address)
    }

    /// Attempt a synchronous nonblocking receive. No buffer access survives this call; `WouldBlock` requires a separate readiness wait.
    #[inline]
    pub fn try_recv(&self, buffer: &mut [u8], flags: SocketRecvFlags) -> io::Result<usize> {
        crate::socket_io::recv_poll(self.get_ref(), buffer, flags)
    }

    /// Attempt a synchronous nonblocking send. No buffer access survives this call; `WouldBlock` requires a separate readiness wait.
    #[inline]
    pub fn try_send(&self, buffer: &[u8], flags: SocketSendFlags) -> io::Result<usize> {
        crate::socket_io::send_poll(self.get_ref(), buffer, flags)
    }

    /// Attempt a synchronous nonblocking message receive using the borrowed I/O vectors directly. No payload, vector, or metadata pointer survives this call.
    #[inline]
    pub fn try_recvmsg(
        &self,
        buffers: &mut [io::IoSliceMut<'_>],
        flags: SocketRecvFlags,
    ) -> io::Result<SocketRecvResult> {
        crate::socket_io::recvmsg_poll(self.get_ref(), buffers, flags)
    }

    /// Attempt a synchronous nonblocking message send using the borrowed I/O vectors directly. No payload, vector, or address pointer survives this call.
    #[inline]
    pub fn try_sendmsg(
        &self,
        buffers: &[io::IoSlice<'_>],
        address: Option<SocketAddr>,
        flags: SocketSendFlags,
    ) -> io::Result<usize> {
        crate::socket_io::sendmsg_poll(self.get_ref(), buffers, address, flags)
    }
}

#[cfg(test)]
mod tests;
