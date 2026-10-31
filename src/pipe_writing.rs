// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::PipeWriter;
use std::{io, os::fd::OwnedFd};

/// Adopt the writable end of an existing Unix pipe, such as `std::process::ChildStdin` or `std::io::PipeWriter`, as a scheduler-bound sequential writer.
///
/// The factory takes exclusive ownership of the descriptor and sets `O_NONBLOCK` on its open file description. Pass a descriptor whose file description is not shared with code that expects blocking writes; the other end of a pipe is a separate description and is unaffected. Construction performs no payload I/O; backends may defer readiness registration until a write must wait. Anonymous pipes are supported on Linux and macOS. FIFOs are not: a peer reopening a FIFO after a hang-up is not guaranteed to restore readiness waits on every backend. Regular files are not supported: use [`FileWriting`](crate::FileWriting) for positioned and cursor I/O.
pub trait PipeWriting {
    /// Take ownership of a pipe descriptor and return a writer bound to this scheduler. The handle closes the descriptor when it is closed or dropped, which the reader observes as end of file once no other write ends remain. On error the descriptor is closed.
    fn pipe_writer<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeWriter + 'scheduler + use<'scheduler, Self>>;
}
