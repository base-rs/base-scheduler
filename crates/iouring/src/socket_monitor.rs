// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::Interest;
use crate::monitor_state::MonitorState;
use std::io;

/// Persistent state for one direction of a socket. Create separate readable and writable monitors for concurrent reader and writer tasks.
///
/// `Registration` belongs to the backend. The socket owner outlives every retained registration, including after future abandonment. A monitor does not replace the scheduler explicitly passed to an operation. Await `socket_monitor_close`, then drop all monitor handles before exclusively closing their socket. Dropping the last handle requests retirement but does not wait for it.
pub(crate) struct SocketMonitor<Socket, Registration> {
    socket: Socket,
    interest: Interest,
    registration: Registration,
}

impl<Socket, Registration> SocketMonitor<Socket, Registration> {
    pub fn new(socket: Socket, interest: Interest, registration: Registration) -> Self {
        Self {
            socket,
            interest,
            registration,
        }
    }
    pub fn socket(&self) -> &Socket {
        &self.socket
    }
    pub fn interest(&self) -> Interest {
        self.interest
    }
    pub fn registration(&self) -> &Registration {
        &self.registration
    }

    /// Check the direction before attempting I/O.
    pub fn check_interest(&self, interest: Interest) -> io::Result<()> {
        if self.interest == interest {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket monitor has the wrong direction",
            ))
        }
    }
}

impl<Socket: Send + Sync + 'static, Registration: MonitorState> MonitorState
    for SocketMonitor<Socket, Registration>
{
    fn acquire(&self) {
        self.registration.acquire();
    }
    fn release(&self) {
        self.registration.release();
    }
}

impl<Socket, Registration> AsRef<SocketMonitor<Socket, Registration>>
    for SocketMonitor<Socket, Registration>
{
    fn as_ref(&self) -> &Self {
        self
    }
}
