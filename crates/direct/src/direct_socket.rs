// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use socket2::Socket;
use std::sync::Arc;

/// An owned nonblocking IP socket, usable by any direct scheduler instance.
///
/// Private retention keeps accepted worker access alive after future abandonment. This handle has no scheduler affinity and is deliberately not `Clone`. Finish outstanding operations before explicit close. Native import and access are not exposed, so callers cannot change the nonblocking mode behind the scheduler.
#[derive(Debug)]
pub struct DirectSocket(pub(crate) Arc<Socket>);

impl DirectSocket {
    pub(crate) fn new(socket: Socket) -> Self {
        Self(Arc::new(socket))
    }

    pub(crate) fn retain(&self) -> Arc<Socket> {
        Arc::clone(&self.0)
    }

    pub(crate) fn into_socket(self) -> Socket {
        Arc::try_unwrap(self.0).expect(
            "cannot close a socket with outstanding operations; cancel and await them first",
        )
    }
}
