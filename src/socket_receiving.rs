// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPooling, Cancellation, OperationResult, SocketOperation, SocketRecvFlags,
    SocketRecvMsgReceiver,
};
use std::future::Future;

/// Construct an owned receive resource backed by a server-shared bounded pool. The backend chooses its receiver representation; applications choose the memory budget, not kernel registration or receive opcodes. Portable backends use syscall-first directional monitors; io_uring uses shared provided-buffer rings and multishot receive-ahead. Existing explicit multishot receivers remain available for comparing backend strategies.
pub trait SocketReceiving: SocketOperation + BufferPooling {
    type PooledReceiver: SocketRecvMsgReceiver<Self, Buffer: crate::ReceiveBuffer>;

    /// Lazy construction, with no receive before successful admission. An admitted receiver may receive ahead within the pool budget. Completed leases outlive the receiver and may move to other workers. Retire the receiver before exclusively closing its socket.
    fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Self::PooledReceiver>> + Send;
}
