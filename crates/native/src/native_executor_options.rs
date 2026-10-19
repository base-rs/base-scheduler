// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

/// Task and clock limits when composing with an independently configured selector.
#[derive(Clone, Copy, Debug)]
pub struct NativeExecutorOptions {
    /// Number of work-stealing task workers. Must be nonzero.
    pub workers: usize,
    /// Reserve at least this many simultaneous timer registrations. Must be nonzero.
    pub timer_capacity: usize,
}

impl Default for NativeExecutorOptions {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(1, usize::from),
            timer_capacity: 1024,
        }
    }
}
