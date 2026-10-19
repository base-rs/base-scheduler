// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler_selector::SelectorOptions;

/// Limits for the ring, retained requests, and readiness fallback.
#[derive(Clone, Copy, Debug)]
pub struct IoUringOptions {
    /// Requested submission queue entries. Must be nonzero.
    pub entries: u32,
    /// Maximum accepted original operations, including queued and in-flight requests. Must be nonzero.
    pub capacity: usize,
    /// Limits for synchronous fallback operations and cleanup.
    pub fallback: SelectorOptions,
}
impl Default for IoUringOptions {
    fn default() -> Self {
        Self {
            entries: 256,
            capacity: 256,
            fallback: SelectorOptions::default(),
        }
    }
}
