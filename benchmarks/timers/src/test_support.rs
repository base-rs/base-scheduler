// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_benchmark::{ResourceSource, ResourceUsage};
use std::io;

/// Resource usage that advances at every snapshot, so that tests measure on every platform, or that fails after a number of snapshots.
#[derive(Default)]
pub(crate) struct TestSource {
    remaining: Option<usize>,
    usage: ResourceUsage,
}

impl TestSource {
    /// A source whose first snapshot fails, so no interval starts.
    pub(crate) fn failing() -> Self {
        Self::failing_after(0)
    }

    /// A source that fails after `snapshots` successful ones: after one, an interval starts but cannot finish.
    pub(crate) fn failing_after(snapshots: usize) -> Self {
        Self {
            remaining: Some(snapshots),
            ..Self::default()
        }
    }
}

impl ResourceSource for TestSource {
    fn snapshot(&mut self) -> io::Result<ResourceUsage> {
        if let Some(remaining) = &mut self.remaining {
            if *remaining == 0 {
                return Err(io::Error::other("no resource usage in this test"));
            }
            *remaining -= 1;
        }
        self.usage.user_cpu_ns += 1_000;
        self.usage.voluntary_context_switches += 1;
        Ok(self.usage)
    }

    fn resolution_ns(&self) -> u64 {
        1
    }
}
