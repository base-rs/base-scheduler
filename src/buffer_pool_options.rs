// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use std::io;

/// Usable payload capacity and alignment, independent of kernel registration strategy. Backends must satisfy these requirements or return an error; backend metadata and allocation padding may require additional memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferPoolOptions {
    /// The number of buffers, which is the pool's exact shared capacity. Must be nonzero.
    pub buffers: usize,
    /// The usable payload length, in bytes, of each buffer. Must be nonzero.
    pub buffer_size: usize,
    /// Nonzero power-of-two alignment of each acquired buffer's first byte.
    pub alignment: usize,
}

impl BufferPoolOptions {
    /// Request `buffers` buffers of `buffer_size` bytes each, with an `alignment` of 1.
    pub const fn new(buffers: usize, buffer_size: usize) -> Self {
        Self {
            buffers,
            buffer_size,
            alignment: 1,
        }
    }

    pub(crate) fn validate(self) -> io::Result<()> {
        if self.buffers == 0
            || self.buffer_size == 0
            || !self.alignment.is_power_of_two()
            || self
                .buffer_size
                .checked_add(self.alignment - 1)
                .and_then(|size| size.checked_mul(self.buffers))
                .is_none_or(|size| size > isize::MAX as usize)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "nonzero capacity and size, power-of-two alignment, and a representable allocation are required",
            ));
        }
        Ok(())
    }
}
