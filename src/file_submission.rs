// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, FileOperation, IoBuf, IoBufMut, OperationError};
use std::future::Future;

/// Optional owned file I/O with distinct backend admission and completion waits.
///
/// `*_submit(...).await` waits for bounded backend admission, returning an owning completion future. Once accepted, I/O can progress without polling that completion. Admission means queued with the backend, not necessarily entered into the kernel or durable. Backend validation and shutdown racing admission may be reported by the returned completion. No independent executor task is required. An unpolled submission submits nothing; admission errors return the original buffer. After admission, await completion to recover the buffer, including on I/O error or cancellation. Dropping the completion requests abandonment but never releases storage still accessed by a worker or kernel. Cancellation is cooperative; a completion backend may observe it when the completion is next polled.
///
/// Reads and writes are single transfers. In particular, handle partial writes before starting a later ordered write. Separate files or explicit offsets allow independent operations; shared-cursor ordering remains the caller's responsibility.
pub trait FileSubmission: FileOperation {
    fn file_read_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)>,
    > + Send;
    fn file_write_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)>,
    > + Send;
    fn file_read_at_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)>,
    > + Send;
    fn file_write_at_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<
        Output = Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)>,
    > + Send;
}
