// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;

impl FileSubmission for IoUringSelector {
    async fn file_read_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        match self
            .operation_submit(
                self.route(),
                Resource::File(retain_file(file), None),
                buffer,
                false,
                |buffer| buffer.as_mut().as_mut_ptr(),
                cancellation,
            )
            .await
        {
            Ok(completion) => Ok(async move {
                let (result, buffer) = completion.await;
                (operation_result(result, cancellation), buffer)
            }),
            Err((error, buffer)) => Err((
                operation_result::<()>(Err(error), cancellation).unwrap_err(),
                buffer,
            )),
        }
    }
    async fn file_write_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        match self
            .operation_submit(
                self.route(),
                Resource::File(retain_file(file), None),
                buffer,
                true,
                |buffer| buffer.as_ref().as_ptr().cast_mut(),
                cancellation,
            )
            .await
        {
            Ok(completion) => Ok(async move {
                let (result, buffer) = completion.await;
                (operation_result(result, cancellation), buffer)
            }),
            Err((error, buffer)) => Err((
                operation_result::<()>(Err(error), cancellation).unwrap_err(),
                buffer,
            )),
        }
    }
    async fn file_read_at_submit<B: IoBufMut>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        match self
            .operation_submit(
                self.route(),
                Resource::File(retain_file(file), Some(offset)),
                buffer,
                false,
                |buffer| buffer.as_mut().as_mut_ptr(),
                cancellation,
            )
            .await
        {
            Ok(completion) => Ok(async move {
                let (result, buffer) = completion.await;
                (operation_result(result, cancellation), buffer)
            }),
            Err((error, buffer)) => Err((
                operation_result::<()>(Err(error), cancellation).unwrap_err(),
                buffer,
            )),
        }
    }
    async fn file_write_at_submit<B: IoBuf>(
        &self,
        file: &Self::File,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
        match self
            .operation_submit(
                self.route(),
                Resource::File(retain_file(file), Some(offset)),
                buffer,
                true,
                |buffer| buffer.as_ref().as_ptr().cast_mut(),
                cancellation,
            )
            .await
        {
            Ok(completion) => Ok(async move {
                let (result, buffer) = completion.await;
                (operation_result(result, cancellation), buffer)
            }),
            Err((error, buffer)) => Err((
                operation_result::<()>(Err(error), cancellation).unwrap_err(),
                buffer,
            )),
        }
    }
}
