// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, IoBuf, OperationError, OperationResult};
use std::{future::Future, io};

/// Bounded, ordered, owned output to a byte-stream socket.
///
/// Construct through [`SocketStreamWriting`](crate::SocketStreamWriting). Use one writer for a socket's ordered output; independent writers or direct writes through another socket alias do not share this ordering. The readable direction remains independent. Each operation receives its scheduler explicitly, and incompatible scheduler instances must be rejected before changing writer state or submitting I/O. Accepted work remains attached to the scheduler which accepted it until retirement.
///
/// Submission transfers ownership of a whole logical write. Implementations handle short transfers and make progress while the application is doing other work, including when no writer future is being polled. They may batch kernel submission at bounded event-loop boundaries. The interface does not require a copy, a task per write, or an immediate kernel entry. Buffer ownership is released as soon as backend access safely ends, independently of whether the application calls `flush`.
///
/// The writer admits at most one unfinished logical write. A later submission waits for its predecessor to finish before accepting new output. Backends may retain a bounded number of completed zero-copy owners after their send results, but never recycle storage before the kernel's release notification. The prototype does not promise zero-copy dispatch.
pub trait SocketStreamWriter<S: ?Sized>: Send {
    type Buffer: IoBuf;

    /// Accept all bytes in `buffer` as the next ordered write. Success reports ownership transfer, not completed output. Rejection returns the new buffer unchanged and guarantees none of its bytes were written. Errors discovered after acceptance are retained for `flush` and subsequent submissions, including errors discovered by an immediate syscall.
    ///
    /// The future is lazy. Pre-cancellation rejects the new buffer. Cancellation while waiting for the previous write cancels only that admission wait. Once accepted, this token continues to govern the new write until its send result; successful completion wins a cancellation race. Partial progress cannot be rolled back, so an accepted write error or cancellation makes the sequence terminal. Later submissions return their buffers without writing them. Dropping an admission future is safe but does not undo work already accepted.
    fn submit(
        &mut self,
        scheduler: &S,
        buffer: Self::Buffer,
        cancellation: &Cancellation,
    ) -> impl Future<Output = Result<(), (OperationError, Self::Buffer)>> + Send;

    /// Await completion of every accepted byte, including short-write retries, or report the retained terminal error. Success means socket acceptance, not peer acknowledgement. Memory-retirement notifications may still be outstanding. Cancelling or dropping this wait does not cancel previously accepted writes; their submission tokens remain authoritative. Once closed, flushing is rejected.
    fn flush(
        &mut self,
        scheduler: &S,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<()>> + Send;

    /// Stop admission, cancel unfinished output, and await retirement of all backend access. This is cleanup, not an implicit flush or socket shutdown. For successful output, flush first; on failure, still close. Cleanup remains available after ordinary cancellation and reports retirement failures separately from an earlier write error. Repeated close is safe. Dropping a close future stops waiting without reopening admission; await close again to establish retirement.
    ///
    /// Dropping the writer requests cancellation without blocking. Backend owners keep every kernel-accessible allocation alive through retirement, including when a writer or future is forgotten. Drop the retired writer before exclusively closing its socket.
    fn close(&mut self, scheduler: &S) -> impl Future<Output = io::Result<()>> + Send;
}
