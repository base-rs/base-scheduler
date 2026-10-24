// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationResult, ReceiveBuffer};
use std::{
    future::Future,
    io::{self, IoSliceMut},
};

/// One sequential input lane on a stream socket, bound to its creating scheduler. Independent reader and writer handles may operate concurrently. Backend registration and worker migration are internal details; moving a handle never changes its scheduler.
///
/// Owned chunks remain valid independently of the reader and can be retained for parsing or transferred to another task. Borrowed operations access payloads only synchronously within each poll, so dropping or forgetting a pending future cannot leave backend access to caller storage. Ordinary errors and cancellation end the current wait, without closing the reader or rolling back consumed bytes. No framing or read-ahead is required.
pub trait SocketStreamReader: Send {
    type Buffer: ReceiveBuffer;

    /// Read an owned initialized chunk. `None` means EOF or a closed reader. Holding every pool lease waits for capacity with cancellation. Idle readiness waits need not retain a payload buffer.
    fn read(
        &mut self,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Option<Self::Buffer>>> + Send;

    /// Read once into borrowed storage, returning a possibly short count. Zero with nonempty storage is EOF; an empty destination does not establish EOF. Successful progress wins concurrent cancellation.
    fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Scatter a single read across borrowed ranges without allocating an I/O-vector array or copying payloads.
    fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Stop input and await registration retirement. This does not close or shut down the socket. Cleanup remains available after cancellation; retry it if its future is dropped. Returned chunks remain valid. Drop the retired reader before exclusively closing its socket.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
