// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::SocketOperation;
use crate::{Cancellation, IoBufSequence, OperationResult, SocketMessage, SocketSendFlags};
use std::future::Future;

/// Send one scatter/gather message through an existing socket. Buffer owners are returned on every result. See [`SocketMessages`](crate::SocketMessages) for the shared message I/O contract.
///
/// Message sending does not imply message receiving:
///
/// ```compile_fail,E0599
/// use protocol_scheduler::{Cancellation, SocketRecvFlags, SocketRecvMsg, SocketSendMsg};
/// async fn receive<S: SocketSendMsg>(scheduler: &S, socket: &S::Socket) {
///     scheduler.socket_recvmsg(socket, vec![vec![0; 16]], SocketRecvFlags::default(), &Cancellation::never()).await;
/// }
/// ```
pub trait SocketSendMsg: SocketOperation {
    fn socket_sendmsg<Buffers: IoBufSequence>(
        &self,
        socket: &Self::Socket,
        message: SocketMessage<Buffers>,
        flags: SocketSendFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = (OperationResult<usize>, SocketMessage<Buffers>)> + Send;
}
