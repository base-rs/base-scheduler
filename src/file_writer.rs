// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use crate::{BufferResult, Cancellation, IoBuf};
use std::{future::Future, io};

/// Scheduler-bound, sequential access to a file's writable direction. Construct through [`FileWriting`](crate::FileWriting). Operations preserve the file's shared OS cursor; separate handles do not coordinate writes through aliases.
pub trait FileWriter: Send {
    /// Write once at the shared cursor, returning a possibly short count and the original owner on every result. Success does not imply durability; cancellation cannot roll back a transferred prefix.
    fn write<B: IoBuf>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send;

    /// Write once at an explicit offset, following [`FileWriteAt`](crate::FileWriteAt)'s platform cursor semantics. The handle maintains no private cursor.
    fn write_at<B: IoBuf>(
        &mut self,
        buffer: B,
        offset: u64,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send;

    /// Write the entire initialized range in order, returning its length and original owner. Partial writes advance an owned view without copying. A nonempty zero-progress write is `WriteZero`; failure may follow a transferred prefix.
    fn write_all<B: IoBuf>(
        &mut self,
        buffer: B,
        cancellation: &Cancellation,
    ) -> impl Future<Output = BufferResult<B>> + Send {
        async move {
            let length = buffer.as_ref().len();
            let (mut result, remaining) = self.write(buffer.slice(..), cancellation).await;
            let mut buffer = remaining.into_inner();
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
                let (next, remaining) = self.write(buffer.slice(written..), cancellation).await;
                buffer = remaining.into_inner();
                result = next;
            }
        }
    }

    /// Stop admission and await accepted operation retirement without cancellation. Repeated close is safe; dropping a pending close is not a retirement barrier. This does not synchronize or close the underlying file. Drop the retired handle before exclusive file close.
    fn close(&mut self) -> impl Future<Output = io::Result<()>> + Send;
}
