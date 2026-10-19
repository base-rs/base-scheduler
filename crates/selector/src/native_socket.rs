// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::Interest;
use std::{io, net::SocketAddr, sync::Arc};
type RegisteredSocket = async_io::Async<socket2::Socket>;

/// An owned nonblocking socket with a retained readiness registration.
///
/// Usable by any native scheduler instance. Readiness operations retain their buffers in the waiting future without leaving payload pointers in the reactor. Completion backends retain the socket and buffers independently until their operations retire. Finish outstanding operations before exclusive close. Imported TCP resources must remain nonblocking through external aliases.
#[derive(Debug)]
pub struct NativeSocket {
    pub(crate) inner: Arc<RegisteredSocket>,
}
impl NativeSocket {
    /// Retain the registered native socket for another I/O backend.
    ///
    /// # Safety
    /// Keep this owner alive until all submitted operations using its handle have retired, including after cancellation or abandonment. Do not close or replace the raw handle, remove its readiness registration, or change its nonblocking mode. Release the retained owner before reporting completion to callers that may immediately close the socket.
    pub unsafe fn retain_native(&self) -> Arc<RegisteredSocket> {
        Arc::clone(&self.inner)
    }

    pub(crate) fn new(socket: socket2::Socket) -> io::Result<Self> {
        Self::new_with(socket, RegisteredSocket::new)
    }
    fn new_with(
        socket: socket2::Socket,
        register: impl FnOnce(socket2::Socket) -> io::Result<RegisteredSocket>,
    ) -> io::Result<Self> {
        Ok(Self {
            inner: Arc::new(register(socket)?),
        })
    }
    pub(crate) fn retain(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
    pub(crate) fn into_close(self) -> impl FnOnce() -> io::Result<()> + Send + 'static {
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
        // Invalid socket states must return their immediate OS error instead of waiting for readiness that cannot occur.
        let result = operation(self.get_ref());
        if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
            return result;
        }
        match interest {
            Interest::Readable => self.inner.read_with(|socket| operation(socket)).await,
            Interest::Writable => self.inner.write_with(|socket| operation(socket)).await,
        }
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.get_ref().local_addr().map(crate::socket_io::address)
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.get_ref().peer_addr().map(crate::socket_io::address)
    }
}

#[cfg(test)]
mod tests;
