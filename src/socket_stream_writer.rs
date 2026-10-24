// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{Cancellation, IoBuf, OperationError, OperationResult};
use std::{
    future::Future,
    io::{self, IoSlice},
};

/// One ordered output lane on a stream socket, bound to its creating scheduler. Independent readers may operate concurrently. Separate writers or direct socket aliases do not share this handle's ordering.
pub trait SocketStreamWriter: Send {
    type Buffer: IoBuf;

    /// Await previous accepted output, then write once from borrowed storage. The count can be short; this call does not accept the unwritten suffix. No backend accesses the borrowed bytes between polls.
    fn write(
        &mut self,
        buffer: &[u8],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Write once from borrowed vectors after previous accepted output completes. Short progress does not advance the caller's vectors.
    fn write_vectored(
        &mut self,
        buffers: &[IoSlice<'_>],
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<usize>> + Send;

    /// Accept a complete owned logical write, waiting for its predecessor before acceptance. Success transfers the owner and lets output progress without further polling. Rejection returns the unchanged owner and writes none of it. Cancellation governs accepted output until its send result; an accepted error or cancellation ends this output sequence because partial bytes cannot be rolled back. Owners remain retained until backend access retires, including after drop or forget.
    fn submit(
        &mut self,
        buffer: Self::Buffer,
        cancellation: &Cancellation,
    ) -> impl Future<Output = Result<(), (OperationError, Self::Buffer)>> + Send;

    /// Wait for accepted output to complete, including short-write retries. Success means socket acceptance, not peer acknowledgement or memory-retirement notification. Cancelling this wait does not cancel already accepted output.
    fn flush(
        &mut self,
        cancellation: &Cancellation,
    ) -> impl Future<Output = OperationResult<()>> + Send;

    /// Stop admission, cancel unfinished output, and await backend access and registration retirement. Flush first when successful delivery is required. This does not shut down or close the socket. Drop requests retirement without blocking; retry an abandoned close to establish completion.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
