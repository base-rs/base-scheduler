// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationResult};
use std::{
    future::Future,
    io::{self, IoSliceMut},
};

/// Sequential, non-positional input from an adopted pipe, bound to its creating scheduler and owning its descriptor.
///
/// Reads consume bytes in pipe order. Each operation performs one possibly short transfer; there is no framing or read-ahead. Borrowed storage is accessed only synchronously within each poll, so dropping or forgetting a pending future leaves no backend access to it. Ordinary errors and cancellation end the current wait without closing the reader or rolling back consumed bytes; successful progress wins concurrent cancellation.
pub trait PipeReader: Send {
    /// Read once into borrowed storage, returning a possibly short count. Zero with nonempty storage is end of file: every write end has been closed. An empty destination completes with zero after checking that the handle is open and cancellation has not been requested.
    fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Scatter a single read across borrowed ranges, with the same end-of-file and empty-storage behavior as [`read_into`](Self::read_into). Short progress does not advance the caller's slices.
    fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Stop input, retire backend registration and close the descriptor, reporting the close result. Later operations fail with `BrokenPipe`; repeating `close` succeeds without further effect. Dropping the reader also closes the descriptor, without awaiting retirement.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
