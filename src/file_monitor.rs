// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use event_listener::Event;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Reusable state for one sequential stream of file operations. Independent monitors may reference the same file; they still share its OS cursor. Use explicit offsets for independent positions.
///
/// The monitor retains its file. Close the monitor and drop it before exclusively closing the file. Backend registration is accessible to implementations; applications use their scheduler's `file_monitor_*` operations.
pub struct FileMonitor<F, R = ()> {
    file: F,
    registration: R,
    pub(crate) activity: Option<Arc<Activity>>,
    pub(crate) closed: bool,
}

impl<F, R> FileMonitor<F, R> {
    /// Construct backend-owned state without submitting I/O.
    pub fn new(file: F, registration: R) -> Self {
        Self {
            file,
            registration,
            activity: None,
            closed: false,
        }
    }

    /// Replace backend registration while retaining the file and fallback retirement state.
    pub fn map_registration<T>(self, map: impl FnOnce(R) -> T) -> FileMonitor<F, T> {
        FileMonitor {
            file: self.file,
            registration: map(self.registration),
            activity: self.activity,
            closed: self.closed,
        }
    }

    /// The retained file, for backend operation dispatch.
    pub fn file(&self) -> &F {
        &self.file
    }
    /// Backend state retained between operations.
    pub fn registration(&self) -> &R {
        &self.registration
    }
    /// Exclusively access backend state without replacing the operation's explicit scheduler.
    pub fn registration_mut(&mut self) -> &mut R {
        &mut self.registration
    }
}

pub(crate) struct Activity {
    pub(crate) running: AtomicBool,
    pub(crate) retired: Event,
}

impl Activity {
    pub(crate) fn new() -> Self {
        let retired = Event::new();
        retired.notify(0);
        Self {
            running: AtomicBool::new(false),
            retired,
        }
    }
}

// This owner travels with the payload, including through abandoned blocking work. Its field order releases the payload before publishing retirement.
pub(crate) struct MonitoredBuffer<B> {
    pub(crate) buffer: Option<B>,
    pub(crate) _retirement: Retirement,
}

pub(crate) struct Retirement(pub(crate) Arc<Activity>);
impl Drop for Retirement {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::Release);
        self.0.retired.notify(usize::MAX);
    }
}
impl<B: AsRef<[u8]>> AsRef<[u8]> for MonitoredBuffer<B> {
    fn as_ref(&self) -> &[u8] {
        self.buffer.as_ref().unwrap().as_ref()
    }
}
impl<B: AsMut<[u8]>> AsMut<[u8]> for MonitoredBuffer<B> {
    fn as_mut(&mut self) -> &mut [u8] {
        self.buffer.as_mut().unwrap().as_mut()
    }
}
// SAFETY: The wrapper transfers its complete owner and changes neither the allocation nor the exposed range.
unsafe impl<B: crate::IoBuf> crate::IoBuf for MonitoredBuffer<B> {}
unsafe impl<B: crate::IoBufMut> crate::IoBufMut for MonitoredBuffer<B> {}
