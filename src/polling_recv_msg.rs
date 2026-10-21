// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    Cancellation, Cancelled, IoBuf, OperationResult, RecvMsgMultishotMessage,
    RecvMsgMultishotOptions, SocketRecvFlags, SocketRecvMsgPoll, SocketRecvMsgReceiver, SocketType,
};
use event_listener::Event;
use std::{
    future::{Future, poll_fn},
    io::{self, IoSliceMut},
    pin::pin,
    sync::{Arc, Mutex},
    task::Poll,
};

struct Pool {
    free: Mutex<Vec<Box<[u8]>>>,
    returned: Event,
}

impl Pool {
    fn try_lease(self: &Arc<Self>) -> Option<Lease> {
        self.free.lock().unwrap().pop().map(|bytes| Lease {
            bytes: Some(bytes),
            pool: Arc::clone(self),
        })
    }
}

struct Lease {
    bytes: Option<Box<[u8]>>,
    pool: Arc<Pool>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        // The fixed-capacity stack always has room for its own checked-out buffer.
        self.pool
            .free
            .lock()
            .unwrap()
            .push(self.bytes.take().unwrap());
        self.pool.returned.notify(1);
    }
}

/// An immutable completed receive, returning its stable allocation to its pool on drop.
pub struct PollingRecvMsgBuffer {
    lease: Lease,
    length: usize,
}

impl AsRef<[u8]> for PollingRecvMsgBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.lease.bytes.as_ref().unwrap()[..self.length]
    }
}

// SAFETY: The lease uniquely owns a fixed boxed allocation. Its completed prefix
// and allocation remain unchanged while the buffer moves or an operation owns it.
unsafe impl IoBuf for PollingRecvMsgBuffer {}

/// A bounded receive resource built from synchronous nonblocking message receives.
///
/// Each `recv` uses the explicitly supplied scheduler. The resource retains its
/// socket and payload pool, without retaining a scheduler or starting a background
/// task. Retaining all returned leases pauses the next receive until one is dropped.
pub struct PollingRecvMsg<Socket, Check> {
    socket: Option<Socket>,
    pool: Option<Arc<Pool>>,
    stream: bool,
    flags: SocketRecvFlags,
    check: Check,
}

impl<Socket, Check> PollingRecvMsg<Socket, Check> {
    /// Retain a socket and reserve its payload pool. `check` validates scheduler
    /// compatibility before each receive and close, without consuming the resource
    /// on rejection. Backends normally supply a function pointer and keep admission
    /// checks in their I/O operations, so close remains possible during shutdown.
    pub fn new(
        socket: Socket,
        kind: SocketType,
        options: RecvMsgMultishotOptions,
        flags: SocketRecvFlags,
        check: Check,
    ) -> io::Result<Self> {
        options.validate()?;
        if flags.peek {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "persistent receives do not support peeking",
            ));
        }
        let returned = Event::new();
        // Reserve event state during construction, including the first wait.
        returned.notify(0);
        let free = Mutex::new(
            (0..options.buffers)
                .map(|_| vec![0; options.buffer_size].into_boxed_slice())
                .collect(),
        );
        // Some platforms initialize their native mutex on the first lock.
        drop(free.lock().unwrap());
        Ok(Self {
            socket: Some(socket),
            pool: Some(Arc::new(Pool { free, returned })),
            stream: kind == SocketType::Stream,
            flags,
            check,
        })
    }

    fn retire(&mut self) {
        self.socket.take();
        self.pool.take();
    }
}

impl<S, Socket, Check> SocketRecvMsgReceiver<S> for PollingRecvMsg<Socket, Check>
where
    S: ?Sized + SocketRecvMsgPoll<Socket = Socket>,
    Socket: Send + Sync,
    Check: Fn(&S, &Socket) -> io::Result<()> + Send,
{
    type Buffer = PollingRecvMsgBuffer;

    async fn recv(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> OperationResult<Option<RecvMsgMultishotMessage<Self::Buffer>>> {
        cancellation.check()?;
        if let Some(socket) = self.socket.as_ref() {
            (self.check)(scheduler, socket)?;
        }
        let Some(pool) = self.pool.as_ref().map(Arc::clone) else {
            return Ok(None);
        };
        let mut lease = if let Some(lease) = pool.try_lease() {
            lease
        } else {
            let mut cancelled = pin!(cancellation.cancelled());
            loop {
                event_listener::listener!(pool.returned => listener);
                // Register before rechecking so a concurrently returned lease
                // cannot lose its wakeup between the empty check and this wait.
                if let Some(lease) = pool.try_lease() {
                    break lease;
                }
                let mut listener = pin!(listener);
                let ready = poll_fn(|context| {
                    if listener.as_mut().poll(context).is_ready() {
                        Poll::Ready(true)
                    } else {
                        cancelled.as_mut().poll(context).map(|_| false)
                    }
                })
                .await;
                if !ready {
                    self.retire();
                    return Err(Cancelled.into());
                }
            }
        };
        let result = scheduler
            .socket_recvmsg_poll(
                self.socket.as_ref().unwrap(),
                &mut [IoSliceMut::new(lease.bytes.as_mut().unwrap())],
                self.flags,
                cancellation,
            )
            .await;
        match result {
            Ok(result) if !self.stream || result.bytes != 0 => Ok(Some(RecvMsgMultishotMessage {
                result,
                buffer: PollingRecvMsgBuffer {
                    lease,
                    length: result.bytes,
                },
            })),
            Ok(_) => {
                self.retire();
                Ok(None)
            }
            Err(error) => {
                self.retire();
                Err(error)
            }
        }
    }

    async fn close(&mut self, scheduler: &S) -> io::Result<()> {
        if let Some(socket) = self.socket.as_ref() {
            (self.check)(scheduler, socket)?;
        }
        self.retire();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
