// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{io, net::SocketAddr, sync::Arc};
use tokio::runtime::{Handle, Id};

#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct RegisteredSocket {
    socket: Arc<socket2::Socket>,
    runtime: Handle,
    stream: bool,
    connected_stream: AtomicBool,
    read_waiting: AtomicBool,
    write_waiting: AtomicBool,
    registration: tokio::sync::OnceCell<tokio::io::unix::AsyncFd<Arc<socket2::Socket>>>,
}

#[cfg(unix)]
impl RegisteredSocket {
    fn new(runtime: &Handle, socket: socket2::Socket) -> Self {
        let stream = socket
            .r#type()
            .is_ok_and(|kind| kind == socket2::Type::STREAM);
        Self {
            socket: Arc::new(socket),
            stream,
            connected_stream: AtomicBool::new(false),
            runtime: runtime.clone(),
            read_waiting: AtomicBool::new(false),
            write_waiting: AtomicBool::new(false),
            registration: tokio::sync::OnceCell::new(),
        }
    }

    fn get_ref(&self) -> &socket2::Socket {
        &self.socket
    }

    /// Record a readiness-driven syscall's success. `try_io` already cleared readiness on `WouldBlock`.
    fn completed<Output>(
        &self,
        ready: &mut tokio::io::unix::AsyncFdReadyGuard<'_, Arc<socket2::Socket>>,
        result: &io::Result<Output>,
        mark_connected: bool,
        drained: &impl Fn(&Output) -> bool,
    ) {
        let Ok(output) = result else {
            return;
        };
        if mark_connected {
            self.connected_stream.store(true, Ordering::Relaxed);
        }
        // A short TCP receive empties the queue unless it stopped at an urgent mark. Linux reports urgent data as priority readiness, which this registration requests and never clears, so a socket that has received urgent data keeps draining through `WouldBlock`; priority interest also reports read closure, which likewise keeps readiness. Tokio's tick check ignores this clear if a newer event was recorded after `ready` was observed, and an edge for input arriving after the receive is still delivered, so no notification is lost; closed readiness survives the clear.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        if drained(output)
            && ready
                .get_ref()
                .try_io(tokio::io::Interest::PRIORITY, |_| Ok(()))
                .is_err()
        {
            ready.clear_ready();
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let _ = (ready, output, drained);
    }

    fn into_inner(self) -> socket2::Socket {
        drop(self.registration);
        Arc::try_unwrap(self.socket).expect("socket registration releases its private socket owner")
    }

    async fn registration(&self) -> io::Result<&tokio::io::unix::AsyncFd<Arc<socket2::Socket>>> {
        self.registration
            .get_or_try_init(|| async {
                let _scope = self.runtime.enter();
                let interest = tokio::io::Interest::READABLE | tokio::io::Interest::WRITABLE;
                // Priority readiness reports TCP urgent data, which stops an ordinary receive short at its mark; OOB-disabled stream reads keep their readiness while it is reported, rather than treating that short count as drained.
                #[cfg(any(target_os = "linux", target_os = "android"))]
                let interest = interest | tokio::io::Interest::PRIORITY;
                // SAFETY: Both this wrapper and the registration own the same socket. No operation can close or replace its descriptor while registered, including if the wrapper or a pending future is forgotten.
                unsafe {
                    tokio::io::unix::AsyncFd::register_with_interest(
                        Arc::clone(&self.socket),
                        interest,
                    )
                }
                .map_err(io::Error::from)
            })
            .await
    }
}

#[cfg(unix)]
struct PollWaiter<'a>(&'a AtomicBool);

#[cfg(unix)]
impl Drop for PollWaiter<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A held lightweight read reservation on an established stream; see `TokioSocket::established_read`.
#[cfg(unix)]
pub(crate) struct EstablishedRead<'a> {
    inner: &'a RegisteredSocket,
    registration: &'a tokio::io::unix::AsyncFd<Arc<socket2::Socket>>,
    waiter: PollWaiter<'a>,
}

#[cfg(unix)]
impl<'a> EstablishedRead<'a> {
    /// Attempt `operation` on cached readiness first, retrying it after each wake. `drained` is as for `TokioSocket::io_with`.
    pub(crate) fn read<Output, Operation, Drained>(
        self,
        operation: Operation,
        drained: Drained,
    ) -> ReadyIo<'a, Operation, Drained>
    where
        Operation: FnMut(&socket2::Socket) -> io::Result<Output>,
        Drained: Fn(&Output) -> bool,
    {
        ReadyIo {
            inner: self.inner,
            registration: self.registration,
            readable: true,
            operation,
            mark_connected: false,
            drained,
            _waiter: self.waiter,
        }
    }
}

/// The reserved-waiter readiness loop: one compact future, so a pending operation retains only borrows, its closure and the reservation.
#[cfg(unix)]
pub(crate) struct ReadyIo<'a, Operation, Drained> {
    inner: &'a RegisteredSocket,
    registration: &'a tokio::io::unix::AsyncFd<Arc<socket2::Socket>>,
    readable: bool,
    operation: Operation,
    mark_connected: bool,
    drained: Drained,
    _waiter: PollWaiter<'a>,
}

// No field is structurally pinned: the closure is only called through `&mut`.
#[cfg(unix)]
impl<Operation, Drained> Unpin for ReadyIo<'_, Operation, Drained> {}

#[cfg(unix)]
impl<Output, Operation, Drained> std::future::Future for ReadyIo<'_, Operation, Drained>
where
    Operation: FnMut(&socket2::Socket) -> io::Result<Output>,
    Drained: Fn(&Output) -> bool,
{
    type Output = io::Result<Output>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            let mut ready = std::task::ready!(if this.readable {
                this.registration.poll_read_ready(context)
            } else {
                this.registration.poll_write_ready(context)
            })?;
            if let Ok(result) = ready.try_io(|_| (this.operation)(this.inner.get_ref())) {
                this.inner
                    .completed(&mut ready, &result, this.mark_connected, &this.drained);
                return std::task::Poll::Ready(result);
            }
        }
    }
}

#[cfg(windows)]
type RegisteredSocket = async_io::Async<socket2::Socket>;

// Bounds a readiness wait whose wakeup was still lost; the timer's insertion, not its expiry, normally rescues the wait. Idle waits retry their syscall once per bound.
#[cfg(windows)]
const WINDOWS_READY_BOUND: std::time::Duration = std::time::Duration::from_secs(10);

/// Wait for readiness after a `WouldBlock` syscall; like `Async::read_with`, the caller optimistically retries the syscall after any wake.
///
/// polling's IOCP backend can leave interest registered from a non-reactor thread queued while the reactor thread blocks without a timeout (smol-rs/polling#271), so a ready socket is never reported. Inserting a timer after registering interest notifies the reactor, which then submits the queued poll before waiting again. Remove this once a polling release includes the fix (smol-rs/polling#272).
#[cfg(windows)]
async fn windows_ready(ready: impl std::future::Future<Output = io::Result<()>>) -> io::Result<()> {
    use std::{
        future::Future,
        pin::{Pin, pin},
        task::Poll,
    };
    let mut ready = pin!(ready);
    let mut rescue = None;
    std::future::poll_fn(|context| {
        if rescue.is_some() {
            return Poll::Ready(Ok(()));
        }
        // Register interest first, so the timer's reactor notification follows any queued registration.
        let registered = ready.as_mut().poll(context);
        let timer = rescue.insert(async_io::Timer::after(WINDOWS_READY_BOUND));
        let _ = Pin::new(timer).poll(context);
        registered
    })
    .await
}

/// A nonblocking socket associated with the adapter's Tokio runtime.
///
/// A descriptor can represent an unconnected, bound, listening, connected, or datagram socket. Unix readiness registration is deferred until an operation needs to wait, so an unconnected socket cannot retain stale closed readiness after connecting or listening. It is deliberately not `Clone`; operation futures borrow it, and accepted shutdown jobs privately retain its registration until completion. Explicit close consumes exclusive ownership.
pub struct TokioSocket {
    pub(crate) inner: Arc<RegisteredSocket>,
    runtime: Id,
}

impl TokioSocket {
    pub(super) fn new(runtime: &Handle, socket: socket2::Socket) -> io::Result<Self> {
        Self::new_with(
            runtime,
            socket,
            |socket| socket.set_nonblocking(true),
            |runtime, socket| {
                #[cfg(unix)]
                {
                    Ok(RegisteredSocket::new(runtime, socket))
                }

                #[cfg(windows)]
                {
                    let _ = runtime;
                    RegisteredSocket::new_nonblocking(socket)
                }
            },
        )
    }

    fn new_with(
        runtime: &Handle,
        socket: socket2::Socket,
        set_nonblocking: impl FnOnce(&socket2::Socket) -> io::Result<()>,
        register: impl FnOnce(&Handle, socket2::Socket) -> io::Result<RegisteredSocket>,
    ) -> io::Result<Self> {
        set_nonblocking(&socket)?;
        let inner = register(runtime, socket)?;

        Ok(Self {
            inner: Arc::new(inner),
            runtime: runtime.id(),
        })
    }

    pub(super) fn retain(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: self.runtime,
        }
    }

    pub(super) fn into_close(self) -> impl FnOnce() -> io::Result<()> + Send + 'static {
        let inner = Arc::try_unwrap(self.inner)
            .expect("cannot close a socket with outstanding operations; await them first");
        move || {
            #[cfg(unix)]
            {
                crate::socket_io::close(inner.into_inner())
            }
            #[cfg(windows)]
            {
                inner.into_inner().and_then(crate::socket_io::close)
            }
        }
    }

    pub(super) fn runtime(&self) -> Id {
        self.runtime
    }

    pub(super) fn get_ref(&self) -> &socket2::Socket {
        self.inner.get_ref()
    }

    #[cfg(test)]
    pub(super) async fn io<Output, Operation>(
        &self,
        interest: Interest,
        operation: Operation,
    ) -> io::Result<Output>
    where
        Operation: FnMut(&socket2::Socket) -> io::Result<Output> + Send,
    {
        self.io_with(interest, operation, false, |_| false).await
    }

    /// `drained` reports whether a successful result proves the receive queue was empty when the syscall returned, so readiness observed before a readiness-first attempt can be cleared without a further syscall returning `WouldBlock`. Only OOB-disabled consuming stream receives pass a predicate other than `|_| false`; Linux and Android honour it unless urgent data or closure has been reported.
    pub(super) async fn io_with<Output, Operation>(
        &self,
        interest: Interest,
        mut operation: Operation,
        read_nonempty: bool,
        drained: impl Fn(&Output) -> bool + Send,
    ) -> io::Result<Output>
    where
        Operation: FnMut(&socket2::Socket) -> io::Result<Output> + Send,
    {
        #[cfg(unix)]
        let known_connected = read_nonempty && self.inner.connected_stream.load(Ordering::Relaxed);
        #[cfg(unix)]
        let mark_connected = read_nonempty && self.inner.stream && !known_connected;
        #[cfg(unix)]
        let ready_first =
            read_nonempty && known_connected && self.inner.registration.get().is_some();
        // Only nonempty reads on a stream that already completed a read may skip the speculative syscall. Lifecycle operations, unknown socket states, and empty reads retain immediate OS results.
        #[cfg(unix)]
        if !ready_first {
            let result = operation(self.get_ref());
            if mark_connected && result.is_ok() {
                self.inner.connected_stream.store(true, Ordering::Relaxed);
            }
            if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
                return result;
            }
        }
        #[cfg(unix)]
        {
            self.ready_io(interest, operation, mark_connected, drained)
                .await
        }

        #[cfg(windows)]
        {
            let _ = (read_nonempty, drained);
            // A readiness registration error ends the operation in place of the next syscall.
            let mut ready = Ok(());
            loop {
                let result = ready.and_then(|()| operation(self.get_ref()));
                if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::WouldBlock) {
                    return result;
                }
                ready = match interest {
                    Interest::Readable => windows_ready(self.inner.readable()).await,
                    Interest::Writable => windows_ready(self.inner.writable()).await,
                };
            }
        }
    }

    pub(super) async fn wait(&self, interest: Interest) -> io::Result<()> {
        #[cfg(unix)]
        if self.inner.registration.get().is_some() {
            // A registered socket already has advisory readiness state. Waiting on it needs no extra poll(2) probe before or after Tokio wakes us.
            return self.ready_io(interest, |_| Ok(()), false, |_| false).await;
        }
        // Preserve syscall-first handling of unregistered sockets, including unconnected streams whose hangup must not become permanently cached readiness.
        self.io_with(
            interest,
            |socket| crate::socket_io::wait(socket, interest),
            false,
            |_| false,
        )
        .await
    }

    #[cfg(unix)]
    async fn ready_io<Output, Operation>(
        &self,
        interest: Interest,
        mut operation: Operation,
        mark_connected: bool,
        drained: impl Fn(&Output) -> bool + Send,
    ) -> io::Result<Output>
    where
        Operation: FnMut(&socket2::Socket) -> io::Result<Output> + Send,
    {
        // Linux can report EPOLLHUP for an unconnected TCP socket; Tokio retains closed readiness permanently, which would spin after the socket later connects and a syscall returns WouldBlock. Register only after a syscall needs to wait, then reuse the registration on subsequent waits.
        let registration = match self.inner.registration.get() {
            Some(registration) => registration,
            None => self.inner.registration().await?,
        };
        let interest = match interest {
            Interest::Readable => tokio::io::Interest::READABLE,
            Interest::Writable => tokio::io::Interest::WRITABLE,
        };
        // Tokio's poll methods keep one waker per direction. Reserve that slot for one operation until completion or drop; concurrent operations use Tokio's independent multi-waiter list. Forgetting a reservation only leaves later operations on the multi-waiter path.
        let waiting = if interest == tokio::io::Interest::READABLE {
            &self.inner.read_waiting
        } else {
            &self.inner.write_waiting
        };
        if waiting
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            loop {
                let mut ready = registration.ready(interest).await?;
                if let Ok(result) = ready.try_io(|_| operation(self.inner.get_ref())) {
                    self.inner
                        .completed(&mut ready, &result, mark_connected, &drained);
                    return result;
                }
            }
        }
        ReadyIo {
            inner: &self.inner,
            registration,
            readable: interest == tokio::io::Interest::READABLE,
            operation,
            mark_connected,
            drained,
            _waiter: PollWaiter(waiting),
        }
        .await
    }

    /// Whether nonempty stream reads wait on cached readiness first: the stream has completed a read and has a registration. A pooled read then waits before taking a slot.
    #[cfg(unix)]
    pub(super) fn read_established(&self) -> bool {
        self.inner.connected_stream.load(Ordering::Relaxed)
            && self.inner.registration.get().is_some()
    }
    /// Windows readiness has no cached state to wait on first.
    #[cfg(windows)]
    pub(super) fn read_established(&self) -> bool {
        false
    }

    /// Reserve the lightweight read waiter for a nonempty read on a known-connected stream with an existing registration, the state in which `io_with` waits on cached readiness first. Returns `None` when the generic path must decide, including while another operation holds the reservation.
    #[cfg(unix)]
    pub(super) fn established_read(&self) -> Option<EstablishedRead<'_>> {
        if !self.inner.connected_stream.load(Ordering::Relaxed) {
            return None;
        }
        let registration = self.inner.registration.get()?;
        self.inner
            .read_waiting
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()?;
        Some(EstablishedRead {
            inner: &self.inner,
            registration,
            waiter: PollWaiter(&self.inner.read_waiting),
        })
    }

    /// Wait for writability after a direct syscall returned `WouldBlock`, as `io_with` does after its own first attempt.
    #[cfg(unix)]
    pub(super) async fn write_after_would_block<Output>(
        &self,
        operation: impl FnMut(&socket2::Socket) -> io::Result<Output> + Send,
    ) -> io::Result<Output> {
        self.ready_io(Interest::Writable, operation, false, |_| false)
            .await
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
