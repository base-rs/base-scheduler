// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Ordered output assertions shared by completion and readiness backends.
use core_scheduler::*;
use futures_lite::future::{poll_once, zip};
use std::io;

/// Empty writes, sequential ordering, pre-cancelled admission, flush, and repeatable cleanup.
pub async fn ordering<S>(scheduler: &S, sender: &S::Socket, receiver: &S::Socket)
where
    S: SocketStreamWriting + SocketRecvPoll,
{
    let never = Cancellation::never();
    let mut writer = scheduler.socket_stream_writer::<Vec<u8>>(sender).unwrap();
    drop(writer.submit(scheduler, b"unpolled".to_vec(), &never));
    let cancellation = Cancellation::new();
    cancellation.cancel();
    let rejected = b"rejected".to_vec();
    let pointer = rejected.as_ptr();
    let (error, returned) = writer
        .submit(scheduler, rejected, &cancellation)
        .await
        .unwrap_err();
    crate::socket_monitoring::cancelled::<()>(Err(error));
    assert_eq!(returned.as_ptr(), pointer);
    assert_eq!(returned, b"rejected");
    let ((), ()) = zip(
        async {
            for value in [b"".as_slice(), b"header", b"", b"body", b"tail"] {
                writer
                    .submit(scheduler, value.to_vec(), &never)
                    .await
                    .map_err(|(error, _)| error)
                    .unwrap();
            }
            writer.flush(scheduler, &never).await.unwrap();
        },
        receive(scheduler, receiver, b"headerbodytail"),
    )
    .await;
    writer.close(scheduler).await.unwrap();
    writer.close(scheduler).await.unwrap();
    assert!(writer.flush(scheduler, &never).await.is_err());
    let (error, returned) = writer
        .submit(scheduler, b"closed".to_vec(), &never)
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(returned, b"closed");
}

/// An accepted large write and its partial retries progress without polling the writer. Its pooled owner is returned without calling flush.
pub async fn independent_progress<S>(scheduler: &S, sender: &S::Socket, receiver: &S::Socket)
where
    S: SocketStreamWriting + SocketRecvPoll,
{
    let never = Cancellation::never();
    let pool = HeapBufferPool::new(1, 8 * 1024 * 1024).unwrap();
    let mut buffer = pool.acquire(&never).await.unwrap();
    buffer.as_mut().fill(0x5a);
    let mut writer = scheduler.socket_stream_writer(sender).unwrap();
    writer
        .submit(scheduler, buffer, &never)
        .await
        .map_err(|(error, _)| error)
        .unwrap();

    // Cancelling a flush wait does not cancel accepted output.
    let cancellation = Cancellation::new();
    cancellation.cancel();
    let result = writer.flush(scheduler, &cancellation).await;
    if let Err(error) = result {
        crate::socket_monitoring::cancelled::<()>(Err(error));
    }
    let mut incoming = vec![0; 64 * 1024];
    let mut received = 0;
    while received < pool.buffer_size() {
        let length = scheduler
            .socket_recv_poll(receiver, &mut incoming, SocketRecvFlags::default(), &never)
            .await
            .unwrap();
        assert!(length > 0);
        assert!(incoming[..length].iter().all(|byte| *byte == 0x5a));
        received += length;
    }
    assert_eq!(received, pool.buffer_size());
    // Waiting for pool capacity, rather than a writer method, also proves ownership is released autonomously.
    let returned = pool.acquire(&never).await.unwrap();
    drop(returned);
    writer.flush(scheduler, &never).await.unwrap();
    writer.close(scheduler).await.unwrap();
}

/// Abandoning a later admission wait leaves the first accepted write intact.
pub async fn abandoned_admission<S>(scheduler: &S, sender: &S::Socket, receiver: &S::Socket)
where
    S: SocketStreamWriting + SocketRecvPoll,
{
    let never = Cancellation::never();
    for size in [8, 8 * 1024 * 1024] {
        let expected = vec![0x33; size];
        let mut writer = scheduler.socket_stream_writer(sender).unwrap();
        writer
            .submit(scheduler, expected.clone(), &never)
            .await
            .map_err(|(error, _)| error)
            .unwrap();
        let mut next = Box::pin(writer.submit(scheduler, b"tail".to_vec(), &never));
        let accepted = poll_once(next.as_mut()).await;
        // A platform may have buffered the whole first write already. Account for the second admission if it completed synchronously.
        let appended = accepted.is_some();
        if let Some(result) = accepted {
            result.map_err(|(error, _)| error).unwrap();
        }
        drop(next);
        receive(scheduler, receiver, &expected).await;
        if appended {
            receive(scheduler, receiver, b"tail").await;
        }
        writer.flush(scheduler, &never).await.unwrap();
        writer.close(scheduler).await.unwrap();
    }
}

/// Cleanup can cancel blocked output without a peer consuming it, and releases retained pool storage.
pub async fn close_pending<S>(scheduler: &S, sender: &S::Socket)
where
    S: SocketStreamWriting,
{
    let never = Cancellation::never();
    let pool = HeapBufferPool::new(1, 8 * 1024 * 1024).unwrap();
    let mut writer = scheduler.socket_stream_writer(sender).unwrap();
    writer
        .submit(scheduler, pool.acquire(&never).await.unwrap(), &never)
        .await
        .map_err(|(error, _)| error)
        .unwrap();
    drop(writer.close(scheduler));
    writer.close(scheduler).await.unwrap();
    writer.close(scheduler).await.unwrap();
    assert!(pool.try_acquire().is_some());
}

async fn receive<S: SocketRecvPoll>(scheduler: &S, socket: &S::Socket, expected: &[u8]) {
    let mut buffer = vec![0; expected.len().min(64 * 1024)];
    let mut received = 0;
    while received < expected.len() {
        let capacity = buffer.len().min(expected.len() - received);
        let count = scheduler
            .socket_recv_poll(
                socket,
                &mut buffer[..capacity],
                SocketRecvFlags::default(),
                &Cancellation::never(),
            )
            .await
            .unwrap();
        assert!(count > 0);
        assert_eq!(&buffer[..count], &expected[received..received + count]);
        received += count;
    }
}
