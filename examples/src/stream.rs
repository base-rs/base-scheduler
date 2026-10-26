// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use base_scheduler::{Cancellation, OperationResult, SocketStreamReader, SocketStreamReading};
use std::io;

/// Consume each borrowed chunk before reusing its storage. The callback cannot retain the borrow.
pub async fn receive<S: SocketStreamReading>(
    scheduler: &S,
    socket: &S::Socket,
    buffer: &mut [u8],
    cancellation: &Cancellation,
    mut consume: impl FnMut(&[u8]) -> OperationResult<()> + Send,
) -> OperationResult<usize>
where
    S::Socket: 'static,
{
    if buffer.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty receive storage").into());
    }
    let mut reader = scheduler.socket_stream_reader(socket)?;
    let result = async {
        let mut total = 0;
        loop {
            let count = reader.read_into(buffer, cancellation).await?;
            if count == 0 {
                return Ok(total);
            }
            consume(&buffer[..count])?;
            total += count;
        }
    }
    .await;
    // Cleanup still runs after an I/O error, callback error, or cooperative cancellation.
    reader.close().await?;
    result
}
