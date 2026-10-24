// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use core_scheduler::{Cancellation, OperationResult, SocketPacketReader, SocketRecvMessage, Spawn};
use std::{collections::VecDeque, io};

/// Transfer complete messages, including peer metadata, to bounded independent tasks. The receiver never copies payloads for handoff.
///
/// This finite packet sequence collects task handles in submission order. A handler error is observed at collection, not immediately while waiting for another packet; the caller's cancellation can end that wait. Handlers must finish without requiring another receive from this sequence.
pub async fn receive<S, R, F>(
    scheduler: &S,
    mut reader: R,
    packets: usize,
    concurrency: usize,
    cancellation: &Cancellation,
    process: F,
) -> OperationResult<()>
where
    S: Spawn,
    R: SocketPacketReader,
    F: Fn(R::Message) -> OperationResult<()> + Clone + Send + 'static,
{
    let mut tasks = VecDeque::with_capacity(concurrency);
    let result: OperationResult<()> = async {
        if concurrency == 0 {
            return Err(
                io::Error::new(io::ErrorKind::InvalidInput, "zero packet concurrency").into(),
            );
        }
        cancellation.check()?;
        for _ in 0..packets {
            if tasks.len() == concurrency {
                let task: S::Handle<OperationResult<()>> = tasks.pop_front().unwrap();
                task.await
                    .map_err(|error| io::Error::other(error.to_string()))??;
            }
            let Some(message) = reader.read(cancellation).await? else {
                unreachable!("an open packet reader cannot produce EOF");
            };
            if message.truncated() || message.control_truncated() {
                return Err(
                    io::Error::new(io::ErrorKind::InvalidData, "truncated datagram").into(),
                );
            }
            let process = process.clone();
            tasks.push_back(
                scheduler
                    .spawn(async move { process(message) })
                    .map_err(io::Error::other)?,
            );
        }
        Ok(())
    }
    .await;
    let closed = reader.close().await;
    // Owning task handles are awaited even after cancellation or a handler failure. Dropping them would abort accepted processing.
    let mut drained = Ok(());
    for task in tasks {
        let result = task
            .await
            .map_err(|error| io::Error::other(error.to_string()))
            .map_err(Into::into)
            .and_then(|result| result);
        drained = drained.and(result);
    }
    closed?;
    result?;
    drained
}
