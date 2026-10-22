// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPool, Cancellation, OperationResult, SocketOperation, SocketRecvFlags,
    SocketRecvMsgReceiver,
};
use std::future::Future;

/// Construct an owned receive resource backed by a server-shared bounded pool. The backend chooses its receiver representation; applications choose the memory budget, not kernel registration or receive opcodes. The initial implementations use syscall-first directional monitors. Existing explicit multishot receivers remain available for comparing backend strategies.
pub trait SocketReceiving: SocketOperation {
    type PooledReceiver: SocketRecvMsgReceiver<Self, Buffer: crate::ReceiveBuffer>;

    /// Lazy construction, with no receive before successful admission. Completed leases outlive the receiver and may move to other workers. Retire the receiver before exclusively closing its socket.
    fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: BufferPool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Self::PooledReceiver>> + Send;
}
