// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{MonitorState, SocketMonitor};
use std::ops::Deref;

/// A directional socket monitor which permits backend-specific byte-stream readiness optimizations.
///
/// Construct through `SocketMonitoring::socket_stream_monitor_state` or `socket_stream_monitor`, then use the ordinary `socket_monitor_*` operations. Readable and writable monitors remain independent. A stream monitor does not assert that a socket is connected, nor that every short receive exhausts readiness: backends select applicable policies for the actual socket and platform. Other socket types retain ordinary monitoring behavior.
///
/// Current backends preserve readiness after short reads and clear it only on `WouldBlock`; the distinct type is a specialization point for future stream policies.
///
/// Like `SocketMonitor`, this state can be embedded in an `Arc`-owned connection and projected with `MonitorStorage::from_arc`. The wrapper adds no allocation or registration.
pub struct SocketStreamMonitor<Socket, Registration> {
    monitor: SocketMonitor<Socket, Registration>,
}

impl<Socket, Registration> SocketStreamMonitor<Socket, Registration> {
    /// Enable stream-specific policies on unregistered monitor state.
    pub fn new(monitor: SocketMonitor<Socket, Registration>) -> Self {
        Self { monitor }
    }
}

impl<Socket, Registration> AsRef<SocketMonitor<Socket, Registration>>
    for SocketStreamMonitor<Socket, Registration>
{
    fn as_ref(&self) -> &SocketMonitor<Socket, Registration> {
        &self.monitor
    }
}

impl<Socket, Registration> Deref for SocketStreamMonitor<Socket, Registration> {
    type Target = SocketMonitor<Socket, Registration>;
    fn deref(&self) -> &Self::Target {
        &self.monitor
    }
}

impl<Socket: Send + Sync + 'static, Registration: MonitorState> MonitorState
    for SocketStreamMonitor<Socket, Registration>
{
    fn acquire(&self) {
        self.monitor.acquire();
    }
    fn release(&self) {
        self.monitor.release();
    }
}
