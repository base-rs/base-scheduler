// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Resource-local failure injection for worker poller tests. Production uses
//! the selected native poller directly, without this wrapper or fault state.

#[cfg(target_os = "linux")]
use super::epoll::{Events, Poller as Backend};
use polling::{Event, PollMode};
#[cfg(not(target_os = "linux"))]
use polling::{Events, Poller as Backend};
use std::{
    io,
    sync::atomic::{AtomicI32, Ordering},
    time::Duration,
};

thread_local! {
    pub(super) static INITIAL_FAILURE: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}
pub(super) struct Poller {
    inner: Backend,
    pub(super) failure: AtomicI32,
}
impl Poller {
    pub(super) fn new() -> io::Result<Self> {
        let result = match INITIAL_FAILURE.with(std::cell::Cell::take) {
            0 => Backend::new(),
            error => Err(io::Error::from_raw_os_error(error)),
        };
        result.map(|inner| Self {
            inner,
            failure: AtomicI32::new(0),
        })
    }
    fn check(&self) -> io::Result<()> {
        let error = self.failure.swap(0, Ordering::Relaxed);
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(())
    }
    pub(super) fn supports_edge(&self) -> bool {
        self.inner.supports_edge()
    }
    pub(super) unsafe fn add_with_mode(
        &self,
        socket: &socket2::Socket,
        event: Event,
        mode: PollMode,
    ) -> io::Result<()> {
        self.check()?;
        // SAFETY: The caller guarantees socket retention through deletion.
        unsafe { self.inner.add_with_mode(socket, event, mode) }
    }
    pub(super) fn delete(&self, socket: &socket2::Socket) -> io::Result<()> {
        self.check()?;
        self.inner.delete(socket)
    }
    #[cfg(target_os = "linux")]
    pub(super) fn clear_notification(&self) -> io::Result<()> {
        self.inner.clear_notification()
    }
    pub(super) fn notify(&self) -> io::Result<()> {
        self.inner.notify()
    }
    pub(super) fn wait(&self, events: &mut Events, timeout: Option<Duration>) -> io::Result<usize> {
        self.check()?;
        self.inner.wait(events, timeout)
    }
}
