// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::provided_buffers::WorkerBuffers;
use super::*;
use crate::IoUringBuffer;
use std::cell::UnsafeCell;
impl SocketReceiving for IoUringSelector {
    type PooledReceiver = multishot::RecvMsgMultishot<IoUringBuffer>;
    async fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> OperationResult<Self::PooledReceiver> {
        cancellation.check()?;
        self.check_open()?;
        if !Weak::ptr_eq(&pool.0.selector, &Arc::downgrade(&self.group)) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer pool belongs to another selector",
            )
            .into());
        }
        if flags.peek {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receivers do not support peeking",
            )
            .into());
        }
        let socket = super::socket::retain_socket(socket);
        let stream = syscall("socket-type", || socket.get_ref().r#type())? == socket2::Type::STREAM;
        let controller = self.route();
        let permit = operation_result(
            controller.capacity.acquire(cancellation).await,
            cancellation,
        )?;
        let identifier = controller.next_identifier()?;
        let buffers = pool.worker(&controller)?;
        let state = Arc::new(multishot::State::default());
        // SAFETY: Multishot receive uses this immutable boxed header only for name/control capacities.
        let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
        header.msg_namelen = std::mem::size_of::<libc::sockaddr_storage>() as _;
        buffers.attach(identifier);
        let _ = controller.send(Command::PoolReady(Arc::clone(&buffers)));
        let request = SharedReceive {
            identifier,
            _permit: permit,
            socket: Some(socket),
            buffers,
            state: Arc::clone(&state),
            header: UnsafeCell::new(header),
            stream,
            polling: Cell::new(false),
            ready: Cell::new(false),
            suspended: Cell::new(false),
            cancelling: Cell::new(false),
            eof: Cell::new(false),
            finished: false,
        };
        if let Err(mpsc::SendError(Command::Submit(request))) =
            controller.send(Command::Submit(Box::new(request)))
        {
            request.finish(Err(io::ErrorKind::BrokenPipe.into()));
        }
        Ok(multishot::RecvMsgMultishot {
            cancel_on_wait: false,
            state,
            cancel: CancelOnDrop {
                controller,
                identifier,
                armed: true,
            },
        })
    }
}

struct SharedReceive {
    identifier: u64,
    _permit: Permit,
    socket: Option<RegisteredSocket>,
    buffers: Arc<WorkerBuffers>,
    state: Arc<multishot::State<IoUringBuffer>>,
    header: UnsafeCell<libc::msghdr>,
    stream: bool,
    polling: Cell<bool>,
    ready: Cell<bool>,
    suspended: Cell<bool>,
    cancelling: Cell<bool>,
    eof: Cell<bool>,
    finished: bool,
}
// SAFETY: The stable boxed header is immutable after publication. One ring owner drives the request; completed payloads move into independent owning leases.
unsafe impl Send for SharedReceive {}

impl PendingRequest for SharedReceive {
    fn identifier(&self) -> u64 {
        self.identifier
    }
    fn buffer_pool(&self) -> Option<&Arc<WorkerBuffers>> {
        Some(&self.buffers)
    }
    fn suspend_for(&self, pool: &Arc<WorkerBuffers>) -> bool {
        if Arc::ptr_eq(&self.buffers, pool) {
            self.suspended.set(true);
            true
        } else {
            false
        }
    }
    fn ready(&mut self, submitter: &io_uring::Submitter<'_>) -> io::Result<bool> {
        if self.buffers.reclaiming() {
            return Ok(false);
        }
        // An empty group waits for socket readiness before requesting storage. Idle connections cannot repeatedly recall buffers from busy workers.
        if !self.ready.get() && !self.buffers.has_buffers() {
            self.polling.set(true);
            return Ok(true);
        }
        self.buffers.demand();
        self.buffers.service(submitter)?;
        if !self.buffers.has_buffers() {
            return Ok(false);
        }
        self.polling.set(false);
        self.buffers.submitted();
        self.state.received.lock().unwrap().statistics.submissions += 1;
        Ok(true)
    }
    fn entry(&mut self) -> squeue::Entry {
        let descriptor = types::Fd(
            self.socket
                .as_ref()
                .expect("the request retains its socket")
                .get_ref()
                .as_raw_fd(),
        );
        let entry = if self.polling.get() {
            opcode::PollAdd::new(descriptor, libc::POLLIN as u32).build()
        } else {
            opcode::RecvMsgMulti::new(descriptor, self.header.get(), self.buffers.group()).build()
        };
        entry.user_data(self.identifier)
    }
    fn entry_may_wake(&self) -> bool {
        false
    }
    fn completion(&self, result: i32, flags: u32) {
        if self.polling.get() {
            self.ready.set(result >= 0);
            return;
        }
        if !io_uring::cqueue::more(flags) {
            self.buffers.completed();
        }
        if result == -libc::ENOBUFS {
            self.state
                .received
                .lock()
                .unwrap()
                .statistics
                .buffer_exhaustions += 1;
        }
        let Some(identifier) = io_uring::cqueue::buffer_select(flags) else {
            if result == 0 {
                self.eof.set(true);
            }
            return;
        };
        let buffer = self.buffers.take(identifier);
        if result <= 0 {
            if result == 0 {
                self.eof.set(true);
            }
            return;
        }
        let start = self.buffers.publish_offset();
        let bytes = &buffer.as_ref()[start..start + result as usize];
        // SAFETY: The boxed header remains immutable throughout kernel and completion access.
        let parsed = types::RecvMsgOut::parse(bytes, unsafe { &*self.header.get() })
            .expect("kernel returns the recvmsg multishot layout");
        let payload = parsed.payload_data();
        if self.stream && payload.is_empty() {
            self.eof.set(true);
            return;
        }
        let offset = payload.as_ptr() as usize - buffer.as_ref().as_ptr() as usize;
        let address = if parsed.name_data().is_empty() {
            None
        } else {
            let mut address = socket2::SockAddrStorage::zeroed();
            // SAFETY: The initialized kernel name is bounded by the header's sockaddr_storage capacity.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    parsed.name_data().as_ptr(),
                    (&mut address as *mut socket2::SockAddrStorage).cast(),
                    parsed.name_data().len(),
                );
                socket2::SockAddr::new(address, parsed.name_data().len() as _)
            }
            .as_socket()
        };
        let length = payload.len();
        let truncated = parsed.is_payload_truncated();
        let control_truncated = parsed.is_control_data_truncated();
        let message = core_scheduler::RecvMsgMultishotMessage {
            result: SocketRecvResult {
                bytes: length,
                address,
                truncated,
                control_truncated,
            },
            buffer: IoUringBuffer::new(buffer, offset, length),
        };
        let notify = {
            let mut received = self.state.received.lock().unwrap();
            let empty = received.messages.is_empty();
            received.statistics.messages += 1;
            received.messages.push_back(message);
            empty
        };
        if notify {
            self.state.changed.notify(1);
        }
    }
    fn retry(&self, result: i32) -> Option<bool> {
        if self.cancelling.get() || self.eof.get() {
            return None;
        }
        if self.suspended.replace(false) {
            self.ready.set(false);
            return Some(true);
        }
        if self.polling.get() && result >= 0 {
            return Some(true);
        }
        if result >= 0 || result == -libc::ENOBUFS {
            self.ready.set(true);
            return Some(self.buffers.has_buffers());
        }
        None
    }
    fn retire(self: Box<Self>, submitter: &io_uring::Submitter<'_>, result: io::Result<usize>) {
        let result = self.buffers.detach(self.identifier, submitter).and(result);
        self.finish(result);
    }
    fn retire_completion(self: Box<Self>, submitter: &io_uring::Submitter<'_>, result: i32) {
        self.retire(submitter, multishot::retirement(result));
    }
    fn finish(mut self: Box<Self>, result: io::Result<usize>) {
        self.buffers.abandon(self.identifier);
        self.finished = true;
        let state = Arc::clone(&self.state);
        drop(self);
        state.finish(result);
    }
    fn begin_cancellation(&self) -> bool {
        !self.cancelling.replace(true)
    }
    fn take_reply(&self) -> Option<Box<dyn Send>> {
        struct Failed(Arc<multishot::State<IoUringBuffer>>);
        impl Drop for Failed {
            fn drop(&mut self) {
                self.0.finish(Err(io::Error::other(
                    "io_uring driver failed during pooled receive",
                )));
            }
        }
        Some(Box::new(Failed(Arc::clone(&self.state))))
    }
}
impl Drop for SharedReceive {
    fn drop(&mut self) {
        if !self.finished {
            self.buffers.abandon(self.identifier);
            drop(self.socket.take());
            self.state.finish(Err(io::ErrorKind::BrokenPipe.into()));
        }
    }
}

#[cfg(test)]
mod tests;
