// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, IoBuf, OperationResult, SocketAddress};
use std::{
    future::Future,
    io::{self, IoSlice},
};

/// Sequential packet sends through a fixed execution service. Every write is one send attempt, including empty packets; it never splits a packet into separate sends. `None` uses the connected peer.
pub trait SocketPacketWriter: Send {
    fn write(
        &mut self,
        buffer: &[u8],
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Send the vectors as one packet, preserving their order without advancing them. The returned count is the backend's single-transfer result.
    fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Retain the owner while sending one packet and return it on success, error, or cancellation. Abandonment drops the owner only after all borrowed access has ended.
    fn write_owned<B: IoBuf>(
        &mut self,
        buffer: B,
        address: Option<SocketAddress>,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send;

    /// Stop writing and await registration retirement without cancellation. Stop using the writer when close begins; retry close after errors or abandonment to establish retirement.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
