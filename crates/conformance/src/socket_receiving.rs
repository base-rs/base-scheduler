// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Portable pooled input assertions through the public reader resources.
use crate::socket::stream_pair;
use base_scheduler::*;
use futures_lite::future::poll_once;
use std::{io::IoSliceMut, net::Shutdown, pin::pin};

/// Idle readers release capacity so another connection can use the same pool.
pub async fn demand_driven<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketStreamReading + SocketStreamWriting,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 8)).unwrap();
    let mut idle = scheduler
        .socket_stream_reader_with_pool(sender, pool.clone())
        .unwrap();
    let mut active = scheduler
        .socket_stream_reader_with_pool(socket, pool.clone())
        .unwrap();
    let mut sender_write = scheduler.socket_stream_writer(sender).unwrap();
    let mut socket_write = scheduler.socket_stream_writer(socket).unwrap();
    {
        let mut waiting = pin!(idle.read(&never));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        assert!(
            pool.try_acquire().is_some(),
            "an idle connection must not reserve storage"
        );
        assert_eq!(sender_write.write(b"a", &never).await.unwrap(), 1);
        let chunk = active.read(&never).await.unwrap().unwrap();
        assert_eq!(chunk.as_ref(), b"a");
        drop(chunk);
        assert_eq!(socket_write.write(b"b", &never).await.unwrap(), 1);
        assert_eq!(waiting.await.unwrap().unwrap().as_ref(), b"b");
    }
    idle.close().await.unwrap();
    active.close().await.unwrap();
    sender_write.close().await.unwrap();
    socket_write.close().await.unwrap();
    assert!(pool.try_acquire().is_some());
}

pub async fn stream<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketStreamReading + SocketStreamWriting + SocketShutdown,
    S::Socket: 'static,
{
    crate::buffer_pool::bounded(scheduler).await;
    let never = Cancellation::never();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(2, 3)).unwrap();
    let mut reader = scheduler
        .socket_stream_reader_with_pool(socket, pool.clone())
        .unwrap();
    let mut writer = scheduler.socket_stream_writer(sender).unwrap();
    assert_eq!(writer.write(b"abcdef", &never).await.unwrap(), 6);
    let first = reader.read(&never).await.unwrap().unwrap();
    let second = reader.read(&never).await.unwrap().unwrap();
    assert_eq!(first.as_ref(), b"abc");
    assert_eq!(second.as_ref(), b"def");
    let token = Cancellation::new();
    {
        let mut waiting = pin!(reader.read(&token));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        token.cancel();
        assert!(waiting.await.is_err());
    }
    let frozen = first.freeze();
    let view = frozen.clone().slice(1..);
    drop((frozen, second));
    writer.close().await.unwrap();
    scheduler
        .socket_shutdown(sender, Shutdown::Write)
        .await
        .unwrap();
    assert!(reader.read(&never).await.unwrap().is_none());
    reader.close().await.unwrap();
    drop(reader);
    assert_eq!(view.as_ref(), b"bc");
    drop(view);
    assert!(pool.try_acquire().is_some());
}

pub async fn datagram<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketPacketReading + SocketPacketWriting + SocketLocalAddress,
    S::Socket: 'static,
{
    crate::socket_packet_io::owned(scheduler, sender, socket).await;
}

/// Retained packet storage survives subsequent input and retirement; pool exhaustion remains cancellable.
pub async fn messages<S>(scheduler: &S)
where
    S: SocketOpen
        + SocketBind
        + SocketLocalAddress
        + SocketClose
        + SocketPacketReading
        + SocketPacketWriting,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let sender = scheduler
        .socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never)
        .await
        .unwrap();
    let socket = scheduler
        .socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never)
        .await
        .unwrap();
    for socket in [&sender, &socket] {
        scheduler
            .socket_bind(socket, "127.0.0.1:0".parse().unwrap(), &never)
            .await
            .unwrap();
    }
    let address = scheduler.socket_local_address(&socket).unwrap();
    let source = scheduler.socket_local_address(&sender).unwrap();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 4)).unwrap();
    let mut reader = scheduler
        .socket_packet_reader_with_pool(&socket, pool.clone())
        .unwrap();
    let mut writer = scheduler.socket_packet_writer(&sender).unwrap();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    crate::socket::cancelled(reader.read(&cancelled).await);
    assert_eq!(
        writer
            .write(b"abcdef", Some(address), &never)
            .await
            .unwrap(),
        6
    );
    let first = reader.read(&never).await.unwrap().unwrap();
    assert_eq!(first.payload(), b"abcd");
    assert_eq!(first.address().unwrap().to_socket_address(), source);
    assert!(first.truncated());
    assert_eq!(writer.write(b"", Some(address), &never).await.unwrap(), 0);
    let mut waiting = Box::pin(reader.read(&never));
    assert!(poll_once(&mut waiting).await.is_none());
    assert_eq!(first.payload(), b"abcd");
    drop(first);
    let empty = waiting.await.unwrap().unwrap();
    assert!(empty.payload().is_empty());
    let token = Cancellation::new();
    let mut waiting = Box::pin(reader.read(&token));
    assert!(poll_once(&mut waiting).await.is_none());
    token.cancel();
    crate::socket::cancelled(waiting.await);
    let mut retry = Box::pin(reader.read(&never));
    assert!(poll_once(&mut retry).await.is_none());
    drop(retry);
    reader.close().await.unwrap();
    reader.close().await.unwrap();
    assert!(reader.read(&never).await.unwrap().is_none());
    writer.close().await.unwrap();
    drop((reader, writer));
    assert!(empty.payload().is_empty());
    drop(empty);
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

/// Stream chunks preserve ordering and EOF, including non-power-of-two pool capacities.
pub async fn stream_eof<S>(scheduler: &S)
where
    S: SocketOpen
        + SocketBind
        + SocketListen
        + SocketConnect
        + SocketAccept
        + SocketLocalAddress
        + SocketPeerAddress
        + SocketClose
        + SocketStreamWriting
        + SocketShutdown
        + SocketStreamReading,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let (sender, socket) = stream_pair(scheduler, SocketDomain::Ipv4).await;
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(3, 4)).unwrap();
    let mut reader = scheduler
        .socket_stream_reader_with_pool(&socket, pool)
        .unwrap();
    let mut writer = scheduler.socket_stream_writer(&sender).unwrap();
    let mut sent = 0;
    while sent < 9 {
        let count = writer.write(&b"abcdefghi"[sent..], &never).await.unwrap();
        assert!(count > 0);
        sent += count;
    }
    writer.close().await.unwrap();
    scheduler
        .socket_shutdown(&sender, Shutdown::Write)
        .await
        .unwrap();
    let mut payload = Vec::new();
    while let Some(chunk) = reader.read(&never).await.unwrap() {
        assert!(!chunk.as_ref().is_empty());
        payload.extend_from_slice(chunk.as_ref());
    }
    assert_eq!(payload, b"abcdefghi");
    assert!(reader.read(&never).await.unwrap().is_none());
    reader.close().await.unwrap();
    drop((reader, writer));
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

/// Immediate reads, scalar and vectored, reach the transport without waiting: they report no input as `WouldBlock`, then queued input and EOF after an established read, whatever readiness the backend has cached. A vectored attempt may fill fewer than all of its ranges, as `std::io::Read::read_vectored` may.
pub async fn stream_try_read<S>(scheduler: &S)
where
    S: SocketOpen
        + SocketBind
        + SocketListen
        + SocketConnect
        + SocketAccept
        + SocketLocalAddress
        + SocketPeerAddress
        + SocketClose
        + SocketStreamWriting
        + SocketShutdown
        + SocketStreamReading,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let (sender, socket) = stream_pair(scheduler, SocketDomain::Ipv4).await;
    let mut reader = scheduler.socket_stream_reader(&socket).unwrap();
    let mut writer = scheduler.socket_stream_writer(&sender).unwrap();
    let mut data = [0; 4];
    assert_eq!(reader.try_read_into(&mut []).unwrap(), 0);
    assert_eq!(
        reader
            .try_read_vectored(&mut [IoSliceMut::new(&mut [])])
            .unwrap(),
        0
    );
    let quiet = std::time::Duration::from_millis(5);
    assert_eq!(
        attempt(|| reader.try_read_into(&mut data), quiet)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(
        reader
            .try_read_vectored(&mut [IoSliceMut::new(&mut data)])
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    // A short read establishes the reader, after which backends may wait on cached readiness first.
    assert_eq!(writer.write(b"a", &never).await.unwrap(), 1);
    assert_eq!(reader.read_into(&mut data, &never).await.unwrap(), 1);
    assert_eq!(writer.write(b"b", &never).await.unwrap(), 1);
    assert_eq!(
        attempt(|| reader.try_read_into(&mut data), DELIVERY).unwrap(),
        1
    );
    assert_eq!(data[0], b'b');
    // A vectored read establishes the reader too; an immediate vectored read
    // then reaches queued input across its ranges.
    assert_eq!(writer.write(b"c", &never).await.unwrap(), 1);
    assert_eq!(
        reader
            .read_vectored(&mut [IoSliceMut::new(&mut data)], &never)
            .await
            .unwrap(),
        1
    );
    assert_eq!(writer.write(b"def", &never).await.unwrap(), 3);
    let (mut head, mut tail) = ([0; 1], [0; 2]);
    let count = {
        let mut vectors = [IoSliceMut::new(&mut head), IoSliceMut::new(&mut tail)];
        attempt(|| reader.try_read_vectored(&mut vectors), DELIVERY).unwrap()
    };
    // A scatter read may stop short of filling every range, as the trait's default does; the remainder stays queued in order. An empty remainder reads zero without establishing EOF.
    assert!((1..=3).contains(&count));
    // The remainder is already queued, so one read returns it.
    let mut rest = [0; 2];
    assert_eq!(
        reader
            .read_into(&mut rest[..3 - count], &never)
            .await
            .unwrap(),
        3 - count
    );
    let received = [&head[..], &tail[..]].concat();
    assert_eq!([&received[..count], &rest[..3 - count]].concat(), b"def");
    writer.close().await.unwrap();
    scheduler
        .socket_shutdown(&sender, Shutdown::Write)
        .await
        .unwrap();
    assert_eq!(
        attempt(|| reader.try_read_into(&mut data), DELIVERY).unwrap(),
        0
    );
    assert_eq!(reader.try_read_into(&mut data).unwrap(), 0);
    assert_eq!(
        reader
            .try_read_vectored(&mut [IoSliceMut::new(&mut data)])
            .unwrap(),
        0
    );
    assert_eq!(reader.read_into(&mut data, &never).await.unwrap(), 0);
    reader.close().await.unwrap();
    assert_eq!(
        reader.try_read_into(&mut data).unwrap_err().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        reader
            .try_read_vectored(&mut [IoSliceMut::new(&mut data)])
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    drop((reader, writer));
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

/// A stronger expectation than [`stream_try_read`]'s, which this workspace's backends meet: one immediate vectored attempt passes every range to a single transfer, so input already queued spans them. The trait's default reads only the first nonempty range, so backends relying on it should not run this case.
pub async fn stream_try_read_vectored_spans_ranges<S>(scheduler: &S)
where
    S: SocketOpen
        + SocketBind
        + SocketListen
        + SocketConnect
        + SocketAccept
        + SocketLocalAddress
        + SocketPeerAddress
        + SocketClose
        + SocketStreamWriting
        + SocketStreamReading,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let (sender, socket) = stream_pair(scheduler, SocketDomain::Ipv4).await;
    let mut reader = scheduler.socket_stream_reader(&socket).unwrap();
    let mut writer = scheduler.socket_stream_writer(&sender).unwrap();
    assert_eq!(writer.write(b"def", &never).await.unwrap(), 3);
    let (mut head, mut tail) = ([0; 1], [0; 2]);
    let count = {
        let mut vectors = [IoSliceMut::new(&mut head), IoSliceMut::new(&mut tail)];
        attempt(|| reader.try_read_vectored(&mut vectors), DELIVERY).unwrap()
    };
    assert_eq!(count, 3);
    assert_eq!((head, tail), (*b"d", *b"ef"));
    drop((reader, writer));
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

const DELIVERY: std::time::Duration = std::time::Duration::from_secs(10);

// Retry an immediate read for up to `patience` while the transport delivers loopback input, without yielding to the scheduler.
fn attempt(
    mut read: impl FnMut() -> std::io::Result<usize>,
    patience: std::time::Duration,
) -> std::io::Result<usize> {
    let start = std::time::Instant::now();
    loop {
        let result = read();
        let waiting =
            matches!(&result, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock);
        if !waiting || start.elapsed() >= patience {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
