// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::SocketOperation;
use crate::{Cancellation, IoBufSequenceMut, OperationResult, SocketRecvFlags, SocketRecvResult};
use std::future::Future;

/// Receive one scatter/gather message through an existing socket. Buffer owners are returned on every result. See [`SocketMessages`](crate::SocketMessages) for the shared message I/O contract.
///
/// A receive-only adapter need not provide sending or any core socket operations:
///
/// ```
/// use protocol_scheduler::{Cancellation, IoBufSequenceMut, OperationResult, SocketOperation, SocketRecvFlags, SocketRecvMsg, SocketRecvResult};
/// struct Receiver<S>(S);
/// impl<S: SocketRecvMsg> SocketOperation for Receiver<S> {
///     type Socket = S::Socket;
/// }
/// impl<S: SocketRecvMsg> SocketRecvMsg for Receiver<S> {
///     async fn socket_recvmsg<Buffers: IoBufSequenceMut>(&self, socket: &Self::Socket, buffers: Buffers, flags: SocketRecvFlags, cancellation: &Cancellation) -> (OperationResult<SocketRecvResult>, Buffers) {
///         self.0.socket_recvmsg(socket, buffers, flags, cancellation).await
///     }
/// }
/// ```
pub trait SocketRecvMsg: SocketOperation {
    /// Receive one message into owned buffers. `WSAEMSGSIZE` on Windows is represented as a successful truncated receive, matching Unix; other OS errors are preserved. A zero-byte datagram is a message, whereas a zero-byte stream receive with nonempty capacity normally indicates EOF.
    fn socket_recvmsg<Buffers: IoBufSequenceMut>(
        &self,
        socket: &Self::Socket,
        buffers: Buffers,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = (OperationResult<SocketRecvResult>, Buffers)> + Send;
}
