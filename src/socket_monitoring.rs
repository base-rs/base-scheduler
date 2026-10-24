// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::backend::{
    MonitorState, MonitorStorage, SocketMonitor, SocketMonitorState, SocketRecvMsgPoll,
    SocketRecvPoll, SocketSendMsgPoll, SocketSendPoll, SocketStreamMonitor, SocketWait,
};
use crate::{
    Cancellation, Interest, OperationResult, SocketAddress, SocketRecvFlags, SocketRecvResult,
    SocketSendFlags,
};
use std::{
    future::Future,
    io::{self, IoSlice, IoSliceMut},
};

/// Optional reusable directional socket operations. Existing polling capabilities remain unchanged.
///
/// The default implementation uses the backend's ordinary polling operations. Completion backends may override it to reuse stable registration and completion storage. Separate monitors support simultaneous reads and writes. Cloned storage aliases the same registration; a backend may reject overlapping waits on that registration with `WouldBlock`. Dropping or cancelling a wait removes its waiter without requiring a persistent registration to close. Close a monitor explicitly to await retirement, or drop its final application handle to request retirement in the background. Borrowed payload storage never remains in the kernel between polls. Receive overrides preserve the ordinary polling contract: consumed bytes, empty datagrams, and EOF return `Ready` in the same poll, and `Pending` consumes nothing.
pub trait SocketMonitoring:
    SocketRecvPoll + SocketSendPoll + SocketRecvMsgPoll + SocketSendMsgPoll + SocketWait
{
    type MonitorRegistration: MonitorState;

    /// Inspect transport semantics and scheduler compatibility before constructing a directional reader or writer.
    fn socket_type(&self, socket: &Self::Socket) -> io::Result<crate::SocketType>;

    /// Construct unregistered state. Embed it in an `Arc`-owned connection and project it with `MonitorStorage::from_arc`, or use `socket_monitor` for separate allocation.
    fn socket_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketMonitor<Self::Socket, Self::MonitorRegistration>>;

    fn socket_monitor(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<MonitorStorage<SocketMonitor<Self::Socket, Self::MonitorRegistration>>>
    where
        Self::Socket: 'static,
    {
        self.socket_monitor_state(socket, interest)
            .map(MonitorStorage::new)
    }

    /// Construct stream-specific state for embedding, without a separate wrapper allocation. Backend registration setup may allocate. This does not assert connectedness; the backend checks which optimizations apply.
    fn socket_stream_monitor_state(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<SocketStreamMonitor<Self::Socket, Self::MonitorRegistration>> {
        self.socket_monitor_state(socket, interest)
            .map(SocketStreamMonitor::new)
    }

    /// Allocate stream-specific monitor state once. Use the same `socket_monitor_*` operations as a general monitor.
    fn socket_stream_monitor(
        &self,
        socket: &Self::Socket,
        interest: Interest,
    ) -> io::Result<MonitorStorage<SocketStreamMonitor<Self::Socket, Self::MonitorRegistration>>>
    where
        Self::Socket: 'static,
    {
        self.socket_stream_monitor_state(socket, interest)
            .map(MonitorStorage::new)
    }

    fn socket_monitor_wait<M>(
        &self,
        monitor: &mut MonitorStorage<M>,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<()>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move {
            self.socket_wait(
                monitor.as_ref().socket(),
                monitor.as_ref().interest(),
                cancellation,
            )
            .await
        }
    }

    /// Retire a persistent registration before dropping its handles and exclusively closing the socket. No waiter may be active on this monitor. This lazy cleanup takes no cancellation token and remains available after ordinary work is cancelled or scheduler admission closes. Dropping a pending close does not establish retirement; await close again to finish. Repeated close is safe. Stop using this monitor and its aliases for I/O once close is first polled; fallback implementations need not reject subsequent use. Backends without persistent registrations need no additional work.
    fn socket_monitor_close<M>(
        &self,
        _monitor: &mut MonitorStorage<M>,
    ) -> impl Future<Output = io::Result<()>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move { Ok(()) }
    }

    fn socket_monitor_recv_poll<M>(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &mut [u8],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move {
            cancellation.check()?;
            monitor.as_ref().check_interest(Interest::Readable)?;
            self.socket_recv_poll(monitor.as_ref().socket(), buffer, flags, cancellation)
                .await
        }
    }

    fn socket_monitor_send_poll<M>(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffer: &[u8],
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move {
            cancellation.check()?;
            monitor.as_ref().check_interest(Interest::Writable)?;
            self.socket_send_poll(monitor.as_ref().socket(), buffer, flags, cancellation)
                .await
        }
    }

    /// Receive with reusable monitor state, preserving [`SocketRecvMsgPoll`]'s syscall-first contract: after synchronous preflight, attempt the nonblocking receive on the first poll, and return any consumed message in that poll. A caller may drop a pending attempt, return its buffer, then use `socket_monitor_wait` before constructing another attempt.
    fn socket_monitor_recvmsg_poll<M>(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<SocketRecvResult>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move {
            cancellation.check()?;
            monitor.as_ref().check_interest(Interest::Readable)?;
            self.socket_recvmsg_poll(monitor.as_ref().socket(), buffers, flags, cancellation)
                .await
        }
    }

    fn socket_monitor_sendmsg_poll<M>(
        &self,
        monitor: &mut MonitorStorage<M>,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send
    where
        Self::Socket: 'static,
        M: SocketMonitorState<Self::Socket, Self::MonitorRegistration>,
    {
        async move {
            cancellation.check()?;
            monitor.as_ref().check_interest(Interest::Writable)?;
            self.socket_sendmsg_poll(
                monitor.as_ref().socket(),
                buffers,
                address,
                flags,
                cancellation,
            )
            .await
        }
    }
}
