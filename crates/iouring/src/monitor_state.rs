// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Lifecycle hooks for retained monitor state. Application handles acquire and release a reference; backend retention keeps storage alive independently without keeping a registration open forever.
///
/// Implementations must tolerate aliases and concurrent handle creation and destruction. These hooks manage registration lifetime, never the validity of the containing allocation, which is independently retained by `MonitorStorage`.
pub(crate) trait MonitorState: Send + Sync + 'static {
    fn acquire(&self) {}
    fn release(&self) {}
}

impl MonitorState for () {}
