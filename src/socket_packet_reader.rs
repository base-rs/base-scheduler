// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationResult, ReceiveBuffer, SocketRecvMessageMut, SocketRecvResult};
use std::{
    future::Future,
    io::{self, IoSliceMut},
};

/// Sequential packet reception through a fixed execution service. Each read consumes at most one packet and preserves its sender and truncation metadata. Empty packets are messages, not EOF.
pub trait SocketPacketReader: Send {
    type Message: SocketRecvMessageMut<Payload: ReceiveBuffer>;

    /// Receive independently owned storage. `None` means the reader has been closed. Retained messages remain valid after subsequent reads, pool replacement, and close. Pool exhaustion waits with cancellation.
    fn read(
        &mut self,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Option<Self::Message>>> + Send;

    /// Receive one packet into reused borrowed storage. A short buffer truncates that packet; no subsequent read recovers its discarded suffix. An empty packet returns zero bytes successfully.
    fn read_into(
        &mut self,
        buffer: &mut [u8],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<SocketRecvResult>> + Send;

    /// Receive one packet across borrowed vectors in order. The vectors are not advanced; the returned count and truncation flags describe the whole packet attempt.
    fn read_vectored(
        &mut self,
        buffers: &mut [IoSliceMut<'_>],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<SocketRecvResult>> + Send;

    /// Stop reading and await registration retirement without cancellation. Stop using the reader when close begins, including after errors or an abandoned close wait. Close may be retried; completed messages stay valid.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
