// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::backend::{MonitorState, SocketMonitor, SocketStreamMonitor};

mod private {
    pub trait Sealed {}
}

/// Stable directional state accepted by the shared `socket_monitor_*` operations.
///
/// Implemented by `SocketMonitor` and `SocketStreamMonitor`. Sealed so the shared-state projection and application lifetime hooks always refer to the same registration. The stream policy is selected statically; it does not add dynamic dispatch or per-operation allocation.
pub trait SocketMonitorState<Socket, Registration>:
    MonitorState + AsRef<SocketMonitor<Socket, Registration>> + private::Sealed
{
    /// Permit backend-specific byte-stream readiness policies, subject to socket and platform checks.
    const STREAM: bool;
}

impl<Socket, Registration> private::Sealed for SocketMonitor<Socket, Registration> {}
impl<Socket, Registration> private::Sealed for SocketStreamMonitor<Socket, Registration> {}

impl<Socket: Send + Sync + 'static, Registration: MonitorState>
    SocketMonitorState<Socket, Registration> for SocketMonitor<Socket, Registration>
{
    const STREAM: bool = false;
}

impl<Socket: Send + Sync + 'static, Registration: MonitorState>
    SocketMonitorState<Socket, Registration> for SocketStreamMonitor<Socket, Registration>
{
    const STREAM: bool = true;
}
