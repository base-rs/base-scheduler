// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    BufferPooling, Cancellation, OperationResult, SocketOperation, SocketRecvFlags,
    SocketRecvMsgReceiver,
};
use std::future::Future;

/// Construct an owned receive resource backed by a server-shared bounded pool. Applications choose the memory budget and retain completed buffers; the backend chooses allocation, reuse order, registration, publication depth, and receive strategy. Applications need not select a kernel mechanism to use this capability.
///
/// Once close is first polled, callers must stop receiving, including after a close error or an abandoned close wait. Only close retries remain valid; successful close establishes retirement. An incompatible scheduler is rejected without submitting backend work, but does not authorize resuming reception.
///
/// Cancel and await an outstanding receive wait, then await [`SocketRecvMsgReceiver::close`] to establish retirement and drop the receiver before exclusively closing the socket. Successful delivery may win cancellation and must still be handled. Cancellation alone does not establish retirement, and close never invalidates delivered buffers.
pub trait SocketReceiving: SocketOperation + BufferPooling {
    type PooledReceiver: SocketRecvMsgReceiver<
            Self,
            Message: crate::SocketRecvMessageMut<Payload: crate::ReceiveBuffer>,
        >;

    /// Lazy construction, with no receive before successful admission. An admitted receiver may receive ahead within the pool budget, and waiting-call cancellation does not roll back data already received. Pre-cancellation and cancellation while acquiring pool capacity preserve the receiver. Cancellation after I/O starts may end the receive sequence or cancel only the wait; it does not guarantee backend retirement. Close stops reception and waits for retirement without invalidating delivered buffers. Retire the receiver before exclusively closing its socket.
    ///
    /// Completed leases outlive the receiver and may move to other workers; retained immutable views prevent reuse until their final owner retires.
    fn socket_receiver(
        &self,
        socket: &Self::Socket,
        pool: Self::Pool,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Self::PooledReceiver>> + Send;
}
