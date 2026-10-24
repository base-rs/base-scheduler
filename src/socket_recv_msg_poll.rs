// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, OperationResult, SocketOperation, SocketRecvFlags, SocketRecvResult};
use std::{future::Future, io::IoSliceMut};

/// Receive one scatter/gather message into borrowed I/O vectors using nonblocking polling.
///
/// Both the I/O-vector array and its payload may use stack storage. Every access finishes before a poll returns, following [`SocketRecvPoll`](crate::backend::SocketRecvPoll). Futures are lazy, pre-cancellation prevents I/O, and successful progress wins concurrent cancellation. A consumed message returns `Ready` in the same poll, including empty datagrams and stream EOF; `Pending` consumes nothing. Pending operations register wakeups for readiness and cancellation. Only the returned byte count is filled, spread across buffers in order; vectors are never advanced implicitly. Truncation and sender metadata follow [`SocketRecvMsg`](crate::backend::SocketRecvMsg).
///
/// After synchronous validation and cancellation checks, the first poll must attempt a nonblocking receive before returning `Pending`. It must not defer that attempt until a later poll of the same future. This syscall-first contract lets pooled receivers probe once and release unused storage before independently waiting for readiness.
pub trait SocketRecvMsgPoll: SocketOperation {
    fn socket_recvmsg_poll(
        &self,
        socket: &Self::Socket,
        buffers: &mut [IoSliceMut<'_>],
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<SocketRecvResult>> + Send;
}
