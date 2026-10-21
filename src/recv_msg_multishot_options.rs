// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::io;

/// Bounded storage for a sequence of received messages.
///
/// Backends may receive ahead, but need not do so. Held messages consume the
/// configured buffers; returning a lease makes its storage available again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecvMsgMultishotOptions {
    /// Number of reusable payload buffers, in 1..=32768.
    pub buffers: u16,
    /// Nonzero payload capacity per message. Receive metadata is additional.
    pub buffer_size: usize,
}

impl Default for RecvMsgMultishotOptions {
    fn default() -> Self {
        Self {
            buffers: 8,
            buffer_size: 65536,
        }
    }
}

impl RecvMsgMultishotOptions {
    /// Validate portable size limits. Backends may have additional storage limits.
    pub fn validate(self) -> io::Result<()> {
        if self.buffers == 0
            || self.buffers > 32768
            || self.buffer_size == 0
            || self.buffer_size.saturating_mul(self.buffers as usize) > isize::MAX as usize
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "multishot requires 1..=32768 nonempty buffers with total payload storage fitting isize",
            ));
        }
        Ok(())
    }
}
