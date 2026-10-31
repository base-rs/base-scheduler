// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::PipeReader;
use std::{io, os::fd::OwnedFd};

/// Adopt the readable end of an existing Unix pipe, such as `std::process::ChildStdout`, `ChildStderr` or `std::io::PipeReader`, as a scheduler-bound sequential reader.
///
/// The factory takes exclusive ownership of the descriptor and sets `O_NONBLOCK` on its open file description. Pass a descriptor whose file description is not shared with code that expects blocking reads; the other end of a pipe is a separate description and is unaffected. Construction performs no payload I/O; backends may defer readiness registration until a read must wait. Anonymous pipes are supported on Linux and macOS. FIFOs are not: a peer reopening a FIFO after a hang-up is not guaranteed to restore readiness waits on every backend. Regular files are not supported: use [`FileReading`](crate::FileReading) for positioned and cursor I/O.
pub trait PipeReading {
    /// Take ownership of a pipe descriptor and return a reader bound to this scheduler. The handle closes the descriptor when it is closed or dropped. On error the descriptor is closed.
    fn pipe_reader<'scheduler>(
        &'scheduler self,
        pipe: OwnedFd,
    ) -> io::Result<impl PipeReader + 'scheduler + use<'scheduler, Self>>;
}
