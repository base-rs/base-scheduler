// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::socket::retain_socket;
use super::*;
use std::io::{IoSlice, IoSliceMut};

// Readiness requests retain only the socket. Borrowed payloads and I/O vectors never enter the ring or its request bookkeeping.
struct ReadinessRequest {
    identifier: u64,
    _permit: Permit,
    socket: Option<RegisteredSocket>,
    interest: Interest,
    reply: Cell<Option<oneshot::Sender<io::Result<usize>>>>,
    cancelling: Cell<bool>,
}

impl PendingRequest for ReadinessRequest {
    fn identifier(&self) -> u64 {
        self.identifier
    }

    fn entry(&mut self) -> squeue::Entry {
        let socket = self.socket.as_ref().expect("request retains its socket");
        let events = match self.interest {
            Interest::Readable => libc::POLLIN,
            Interest::Writable => libc::POLLOUT,
        };
        opcode::PollAdd::new(types::Fd(socket.get_ref().as_raw_fd()), events as u32)
            .build()
            .user_data(self.identifier)
    }

    fn entry_may_wake(&self) -> bool {
        // Only fixed native socket metadata is accessed; no buffer callbacks.
        false
    }

    fn finish(self: Box<Self>, result: io::Result<usize>) {
        let reply = self
            .reply
            .take()
            .expect("unfinished request retains its sender");
        // Release the socket and capacity before waking a caller that may close the socket or start another wait.
        drop(self);
        discard(reply.send(result));
    }

    fn begin_cancellation(&self) -> bool {
        !self.cancelling.replace(true)
    }

    fn take_reply(&self) -> Option<Box<dyn Send>> {
        self.reply
            .take()
            .map(|reply| Box::new(reply) as Box<dyn Send>)
    }
}

impl Drop for ReadinessRequest {
    fn drop(&mut self) {
        drop(self.socket.take());
        // An unpublished request may be discarded during shutdown. Unknown in-flight requests follow the driver's existing retention policy.
        if let Some(reply) = self.reply.take() {
            discard(reply.send(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "selector closed before readiness submission",
            ))));
        }
    }
}

impl IoUringSelector {
    pub(super) async fn wait_ready(
        &self,
        controller: Arc<Controller>,
        socket: &NativeSocket,
        interest: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        cancellation.check()?;
        self.check_open()?;
        let permit = operation_result(
            controller.capacity.acquire(cancellation).await,
            cancellation,
        )?;
        let identifier = controller.next_identifier()?;
        let (reply, result) = oneshot::channel();
        let request = ReadinessRequest {
            identifier,
            _permit: permit,
            socket: Some(retain_socket(socket)),
            interest,
            reply: Cell::new(Some(reply)),
            cancelling: Cell::new(false),
        };
        operation_result(
            Self::submit(controller, Box::new(request), result, cancellation).await,
            cancellation,
        )
        .map(drop)
    }

    async fn socket_poll<T>(
        &self,
        socket: &NativeSocket,
        interest: Interest,
        cancellation: &Cancellation,
        mut operation: impl FnMut() -> io::Result<T> + Send,
    ) -> OperationResult<T> {
        cancellation.check()?;
        self.check_open()?;
        let mut controller = None;
        loop {
            match operation() {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    // Ready I/O needs neither routing nor request storage. Keep the first wait's ring across retries and task migration.
                    let controller = controller.get_or_insert_with(|| self.route());
                    self.wait_ready(Arc::clone(controller), socket, interest, cancellation)
                        .await?;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    cancellation.check()?;
                    self.check_open()?;
                }
                result => return result.map_err(Into::into),
            }
        }
    }
}

impl SocketWait for IoUringSelector {
    async fn socket_wait(
        &self,
        socket: &NativeSocket,
        interest: Interest,
        cancellation: &Cancellation,
    ) -> OperationResult<()> {
        self.wait_ready(self.route(), socket, interest, cancellation)
            .await
    }
}

impl SocketRecvPoll for IoUringSelector {
    async fn socket_recv_poll(
        &self,
        socket: &NativeSocket,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.socket_poll(socket, Interest::Readable, cancellation, || {
            socket.try_recv(buffer, flags)
        })
        .await
    }
}

impl SocketSendPoll for IoUringSelector {
    async fn socket_send_poll(
        &self,
        socket: &NativeSocket,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.socket_poll(socket, Interest::Writable, cancellation, || {
            socket.try_send(buffer, flags)
        })
        .await
    }
}

impl SocketRecvMsgPoll for IoUringSelector {
    async fn socket_recvmsg_poll(
        &self,
        socket: &NativeSocket,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<SocketRecvResult> {
        self.socket_poll(socket, Interest::Readable, cancellation, || {
            socket.try_recvmsg(buffers, flags)
        })
        .await
    }
}

impl SocketSendMsgPoll for IoUringSelector {
    async fn socket_sendmsg_poll(
        &self,
        socket: &NativeSocket,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<usize> {
        self.socket_poll(socket, Interest::Writable, cancellation, || {
            socket.try_sendmsg(buffers, address, flags)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
