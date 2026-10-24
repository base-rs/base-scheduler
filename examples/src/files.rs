// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{
    Cancellation, FileReader, FileReading, FileWriter, FileWriting, IoBuf, OperationResult,
};
use std::io;

/// Reuse two allocations and bound reader/writer handles. Each joined iteration drives the next read and the preceding write.
pub async fn copy<S: FileReading + FileWriting>(
    scheduler: &S,
    input: &S::File,
    output: &S::File,
    buffers: [Vec<u8>; 2],
    cancellation: &Cancellation,
) -> OperationResult<usize> {
    if buffers.iter().any(Vec::is_empty) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty copy storage").into());
    }
    let mut reader = scheduler.file_reader(input)?;
    let mut writer = scheduler.file_writer(output)?;
    let result = async {
        let [mut buffer, mut spare] = buffers;
        let (count, received) = reader.read(buffer, cancellation).await;
        buffer = received;
        let mut count = count?;
        let mut copied = 0;
        while count != 0 {
            let (written, read) = futures_lite::future::zip(
                writer.write_all(buffer.slice(..count), cancellation),
                reader.read(spare, cancellation),
            )
            .await;
            // Both operations have completed, including on error, before either monitor is reused or closed.
            written.0?;
            copied += count;
            spare = written.1.into_inner();
            buffer = read.1;
            count = read.0?;
        }
        Ok(copied)
    }
    .await;
    let read_closed = reader.close().await;
    let write_closed = writer.close().await;
    read_closed?;
    write_closed?;
    result
}
