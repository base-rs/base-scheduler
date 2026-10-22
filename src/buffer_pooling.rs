// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferPool, BufferPoolOptions};
use std::io;

/// Construct a backend-selected pool with portable ownership semantics. Applications choose the payload budget and alignment; implementations choose allocation and registration strategy. Pool construction performs no I/O on application resources.
///
/// Pools may retain backend resources for registration lifetime, but operations still receive their scheduler and cancellation explicitly. Backends document instance affinity and reject incompatible resources before submitting I/O. Closing a scheduler does not invalidate completed buffers or their retained views.
///
/// ```
/// use core_scheduler::{BufferPoolOptions, BufferPooling, Cancellation, IoBuf,
///     OperationResult, ReceiveBuffer, SocketReceiving, SocketRecvFlags, SocketRecvMsgReceiver};
///
/// async fn receive<S: SocketReceiving>(scheduler: &S, socket: &S::Socket,
///     cancellation: &Cancellation) -> OperationResult<()> {
///     let pool = scheduler.buffer_pool(BufferPoolOptions::new(128, 64 * 1024))?;
///     let mut receiver = scheduler.socket_receiver(socket, pool, SocketRecvFlags::default(), cancellation).await?;
///     if let Some(message) = receiver.recv(scheduler, cancellation).await? {
///         // Parse or decrypt through message.buffer.as_mut() before freezing.
///         let frozen = message.buffer.freeze();
///         let frame = frozen.clone().slice(..);
///         // Either owner can move to another worker; the slot stays unavailable until both retire.
///         drop((frame, frozen));
///     }
///     receiver.close(scheduler).await?;
///     Ok(())
/// }
/// ```
pub trait BufferPooling {
    type Pool: BufferPool;
    fn buffer_pool(&self, options: BufferPoolOptions) -> io::Result<Self::Pool>;

    /// Provision `options.buffers` slots for every I/O worker. Construction reserves every worker's storage and schedules its registration independently of the first receive. `capacity()` reports the total across workers. Acquisition and receivers use their selected worker's capacity; exhaustion waits there even when another worker has free slots. Completed leases can move between workers and always return to their original storage.
    ///
    /// This is an explicit locality policy, rather than a larger shared budget. A synchronous constructor does not wait for other workers: drivers perform registration on their next turn, including startup, and report setup errors when preparing receivers. Backends without worker-owned pools return `Unsupported`.
    fn buffer_pool_per_worker(&self, _options: BufferPoolOptions) -> io::Result<Self::Pool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "worker-owned buffer pools are unavailable",
        ))
    }
}
