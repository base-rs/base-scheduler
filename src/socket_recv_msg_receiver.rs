// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, IoBuf, OperationResult, SocketRecvMessage};
use std::{future::Future, io};

/// An owning receive operation advanced using an explicitly supplied scheduler.
///
/// Receivers retain socket, buffer, and registration state, without caching a scheduler. Resource compatibility follows [`SocketOperation`](crate::SocketOperation). A submitted completion operation may remain attached to its original driver.
///
/// Portable shutdown is: cancel the pending `recv`, await that wait's result, then call `close(scheduler).await` before exclusively closing the socket. Cancellation ends a wait; it is not proof that reception stopped or backend access retired. Close establishes retirement, and all delivered buffers remain valid. To continue receiving after cancellation, follow the construction capability's guarantee; portable code may instead close and construct a new receiver, accounting for data already consumed.
pub trait SocketRecvMsgReceiver<S: ?Sized>: Send {
    type Buffer: IoBuf;

    /// Receive the next message. `None` is terminal stream EOF or a closed receiver; an empty datagram is `Some` with an empty payload. Delivery preserves stream byte order and datagram boundaries. Receive-ahead and chunk sizes are backend choices. Do not run competing receives unless the protocol permits them.
    ///
    /// Holding every lease pauses reception until a buffer returns. Waiting must not spin, allocate another payload buffer, or discard incoming datagrams in userspace. The OS may still drop datagrams when its own queues fill. Cancel outstanding capacity waits before shutdown: backend admission closure alone need not wake a wait for an application-held buffer.
    ///
    /// Futures are lazy. Pre-cancellation leaves the receiver unchanged. Once waiting, observed successful progress wins cancellation. Otherwise cancellation ends the wait, but need not stop reception or retire backend requests; the construction capability specifies whether reception can continue. Use `close` to establish retirement. A terminal error is returned once, followed by `None`. Incompatible scheduler arguments are rejected before progress, preserving the receiver. Dropping a wait is safe but is not rollback.
    fn recv(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<Option<SocketRecvMessage<Self::Buffer>>>> + Send;

    /// Stop receiving, discard unread queued messages, and wait for original requests to retire. Returned leases remain valid. Repeated close is safe. This cleanup has no cancellation argument, so it remains usable after cancelling reception. Await it to completion: dropping the close future is not proof of retirement. Receiver drop also stops reception, but need not wait for retirement; completion backends retain all kernel-accessible storage independently.
    fn close(&mut self, scheduler: &S) -> impl Future<Output = io::Result<()>> + Send;
}
