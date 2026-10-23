// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{
    BufferResult, Cancellation, FileMonitor, FileMonitoring, IoBuf, OperationResult,
};
use std::io;

/// Reuse two allocations and two monitors. Each joined iteration drives the next read and the preceding write.
pub async fn copy<S: FileMonitoring>(
    scheduler: &S,
    input: &S::File,
    output: &S::File,
    buffers: [Vec<u8>; 2],
    cancellation: &Cancellation,
) -> OperationResult<usize> {
    if buffers.iter().any(Vec::is_empty) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty copy storage").into());
    }
    let mut reader = scheduler.file_monitor(input)?;
    let mut writer = scheduler.file_monitor(output)?;
    let result = async {
        let [mut buffer, mut spare] = buffers;
        let (count, received) = scheduler
            .file_monitor_read(&mut reader, buffer, cancellation)
            .await;
        buffer = received;
        let mut count = count?;
        let mut copied = 0;
        while count != 0 {
            let (written, read) = futures_lite::future::zip(
                write_all(scheduler, &mut writer, buffer.slice(..count), cancellation),
                scheduler.file_monitor_read(&mut reader, spare, cancellation),
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
    let read_closed = scheduler.file_monitor_close(&mut reader).await;
    let write_closed = scheduler.file_monitor_close(&mut writer).await;
    read_closed?;
    write_closed?;
    result
}

/// Advance an owned view after short writes and always return the original owner.
pub async fn write_all<S: FileMonitoring, B: IoBuf>(
    scheduler: &S,
    writer: &mut FileMonitor<S::File, S::FileRegistration>,
    mut buffer: B,
    cancellation: &Cancellation,
) -> BufferResult<B> {
    let mut offset = 0;
    while offset < buffer.as_ref().len() {
        let (result, written) = scheduler
            .file_monitor_write(writer, buffer.slice(offset..), cancellation)
            .await;
        buffer = written.into_inner();
        match result {
            Ok(0) => {
                return (
                    Err(io::Error::new(io::ErrorKind::WriteZero, "copy made no progress").into()),
                    buffer,
                );
            }
            Ok(count) => offset += count,
            Err(error) => return (Err(error), buffer),
        }
    }
    (Ok(offset), buffer)
}
