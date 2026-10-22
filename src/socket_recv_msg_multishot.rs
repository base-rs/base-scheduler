// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{
    Cancellation, OperationResult, RecvMsgMultishotOptions, SocketOperation, SocketRecvFlags,
    SocketRecvMsgReceiver,
};
use std::future::Future;

/// A bounded sequence of owned receive leases, optionally using kernel multishot I/O.
///
/// Readiness backends can receive directly into reusable buffers on demand. This capability requires neither kernel rearming, a background task, receive-ahead, nor a heap allocation per message. Successful creation permits receive-ahead, including before the first `recv`, so callers must not assume demand-only I/O. Concrete receiver and buffer types retain static dispatch. Socket metadata and truncation follow [`SocketRecvResult`](crate::SocketRecvResult). Peeking is rejected because a sequence could repeatedly return the same bytes.
///
/// Pre-cancellation leaves the receiver unchanged. During a pending `recv`, observed successful progress wins cancellation; otherwise cancellation closes and retires reception before returning. This is stricter than the cancellation contract of pooled receivers constructed through [`SocketReceiving`](crate::SocketReceiving).
///
/// ```
/// use core_scheduler::{Cancellation, OperationResult, RecvMsgMultishotOptions,
///     SocketRecvFlags, SocketRecvMsgMultishot, SocketRecvMsgReceiver};
///
/// async fn consume<S: SocketRecvMsgMultishot>(
///     scheduler: &S, socket: &S::Socket, cancellation: &Cancellation,
/// ) -> OperationResult<()> {
///     let mut receiver = scheduler.socket_recvmsg_multishot(
///         socket, RecvMsgMultishotOptions::default(), SocketRecvFlags::default(), cancellation,
///     ).await?;
///     while let Some(message) = receiver.recv(scheduler, cancellation).await? {
///         // Parse or forward message.buffer.as_ref(); drop returns its storage.
///         let _payload = message.buffer.as_ref();
///     }
///     receiver.close(scheduler).await?;
///     Ok(())
/// }
/// ```
pub trait SocketRecvMsgMultishot: SocketOperation {
    type Receiver: SocketRecvMsgReceiver<Self>;

    /// Lazily create a receiver retaining its socket and bounded storage. The token controls creation; subsequent waits receive their own explicit token. Retire the receiver before exclusive socket close. Backends may bound persistent receiver admission separately from payload buffers.
    fn socket_recvmsg_multishot(
        &self,
        socket: &Self::Socket,
        options: RecvMsgMultishotOptions,
        flags: SocketRecvFlags,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Self::Receiver>> + Send;
}
