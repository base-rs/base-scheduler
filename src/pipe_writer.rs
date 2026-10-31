// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationResult};
use std::{
    future::Future,
    io::{self, IoSlice},
};

/// Sequential, non-positional output to an adopted pipe, bound to its creating scheduler and owning its descriptor.
///
/// Each operation performs one possibly partial transfer in pipe order; the unwritten suffix remains the caller's responsibility. Borrowed storage is accessed only synchronously within each poll, so dropping or forgetting a pending future leaves no backend access to it. Cancellation ends the current wait without rolling back transferred bytes; successful progress wins concurrent cancellation.
///
/// Writing after every read end has closed fails with `BrokenPipe` (`EPIPE`). The process must ignore `SIGPIPE`, as Rust executables do by default, for that error to be observable instead of terminating the process.
pub trait PipeWriter: Send {
    /// Make one possibly partial borrowed transfer. An empty buffer completes with zero after checking that the handle is open and cancellation has not been requested.
    fn write(
        &mut self,
        buffer: &[u8],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Gather one possibly partial transfer from borrowed ranges. Short progress does not advance the caller's slices.
    fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Stop output, retire backend registration and close the descriptor, reporting the close result. There is no buffered output to flush. Readers observe end of file once no other write ends remain. Later operations fail with `BrokenPipe`; repeating `close` succeeds without further effect. Dropping the writer also closes the descriptor, without awaiting retirement. Unlike socket and file handles, a pipe handle owns its descriptor. After backend shutdown, `close` can report the shutdown error; the descriptor is closed regardless.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
