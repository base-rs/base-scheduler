// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::DirectScheduler;
use core_scheduler::{BufferResult, Cancellation, FileSubmission, IoBuf, IoBufMut, OperationError};
use std::future::Future;

impl FileSubmission for DirectScheduler {
    async fn file_read_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.buffer_operation_submit(file, buffer, cancellation, move |file, buffer| {
            crate::file_io::read(file, buffer.as_mut())
        })
        .await
    }
    async fn file_write_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.buffer_operation_submit(file, buffer, cancellation, move |file, buffer| {
            crate::file_io::write(file, buffer.as_ref())
        })
        .await
    }
    async fn file_read_at_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.buffer_operation_submit(file, buffer, cancellation, move |file, buffer| {
            crate::file_io::read_at(file, buffer.as_mut(), offset)
        })
        .await
    }
    async fn file_write_at_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        self.buffer_operation_submit(file, buffer, cancellation, move |file, buffer| {
            crate::file_io::write_at(file, buffer.as_ref(), offset)
        })
        .await
    }
}
