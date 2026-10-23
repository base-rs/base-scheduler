// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, FileSubmission, FileWrite, IoBuf, OperationError};
use std::{future::Future, io};

/// Write the entire initialized range, preserving the original owner on every result. Writes are ordered and partial writes advance an owned view without copying. Zero progress is `WriteZero`; success does not imply durability.
pub async fn file_write_all<S: FileWrite, B: IoBuf>(
    scheduler: &S,
    file: &S::File,
    buffer: B,
    cancellation: &Cancellation,
) -> BufferResult<B> {
    let (result, buffer) = scheduler.file_write(file, buffer, cancellation).await;
    finish(scheduler, file, result, buffer, cancellation).await
}

/// Await admission of the first write, then return an owning future that completes the ordered write sequence. The accepted first write progresses independently; after a short write, further writes are submitted when the completion future is polled. Admission failure returns the buffer without writing it.
pub async fn file_write_all_submit<S: FileSubmission + FileWrite, B: IoBuf>(
    scheduler: &S,
    file: &S::File,
    buffer: B,
    cancellation: &Cancellation,
) -> Result<impl Future<Output = BufferResult<B>> + Send, (OperationError, B)> {
    let completion = scheduler
        .file_write_submit(file, buffer, cancellation)
        .await?;
    Ok(async move {
        let (result, buffer) = completion.await;
        finish(scheduler, file, result, buffer, cancellation).await
    })
}

async fn finish<S: FileWrite, B: IoBuf>(
    scheduler: &S,
    file: &S::File,
    mut result: crate::OperationResult<usize>,
    mut buffer: B,
    cancellation: &Cancellation,
) -> BufferResult<B> {
    let length = buffer.as_ref().len();
    let mut written = 0;
    loop {
        match result {
            Err(error) => return (Err(error), buffer),
            Ok(count) => {
                written += count;
                if written == length {
                    return (Ok(written), buffer);
                }
                if count == 0 {
                    return (
                        Err(io::Error::from(io::ErrorKind::WriteZero).into()),
                        buffer,
                    );
                }
            }
        }
        let remaining = buffer.slice(written..);
        let (next, remaining) = scheduler.file_write(file, remaining, cancellation).await;
        buffer = remaining.into_inner();
        result = next;
    }
}
