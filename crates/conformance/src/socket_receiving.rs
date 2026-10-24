// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Portable shared-pool receiver assertions.
use crate::socket::stream_pair;
use core_scheduler::backend::*;
use core_scheduler::*;
use futures_lite::future::poll_once;
use std::{net::Shutdown, pin::pin};

/// Demand-driven monitors must release storage while idle, share one slot across connections, and resume after their bufferless readiness wait is woken.
pub async fn demand_driven<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketMonitoring + BufferPooling + SocketWrite,
    S::Socket: 'static,
{
    let never = Cancellation::never();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(1, 8)).unwrap();
    let mut idle = SocketReceiver::new(
        scheduler
            .socket_monitor(sender, Interest::Readable)
            .unwrap(),
        pool.clone(),
        SocketType::Stream,
        SocketRecvFlags::default(),
    )
    .unwrap();
    let mut active = SocketReceiver::new(
        scheduler
            .socket_monitor(socket, Interest::Readable)
            .unwrap(),
        pool.clone(),
        SocketType::Stream,
        SocketRecvFlags::default(),
    )
    .unwrap();
    {
        let mut waiting = pin!(idle.recv(scheduler, &never));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        assert!(
            pool.try_acquire().is_some(),
            "an idle connection must not reserve storage"
        );
        let (sent, _) = scheduler.socket_write(sender, b"a".to_vec(), &never).await;
        assert_eq!(sent.unwrap(), 1);
        let message = active.recv(scheduler, &never).await.unwrap().unwrap();
        assert_eq!(message.payload(), b"a");
        drop(message);
        let (sent, _) = scheduler.socket_write(socket, b"b".to_vec(), &never).await;
        assert_eq!(sent.unwrap(), 1);
        let message = waiting.await.unwrap().unwrap();
        assert_eq!(message.payload(), b"b");
    }
    SocketRecvMsgReceiver::close(&mut idle, scheduler)
        .await
        .unwrap();
    SocketRecvMsgReceiver::close(&mut active, scheduler)
        .await
        .unwrap();
    assert!(pool.try_acquire().is_some());
}

pub async fn stream<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketReceiving + SocketWrite + SocketShutdown,
{
    crate::buffer_pool::bounded(scheduler).await;
    let never = Cancellation::never();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(2, 3)).unwrap();
    let mut receiver = scheduler
        .socket_receiver(socket, pool.clone(), SocketRecvFlags::default(), &never)
        .await
        .unwrap();
    assert!(
        scheduler
            .socket_receiver(socket, pool.clone(), SocketRecvFlags { peek: true }, &never)
            .await
            .is_err()
    );
    let (result, _) = scheduler
        .socket_write(sender, b"abcdef".to_vec(), &never)
        .await;
    assert_eq!(result.unwrap(), 6);
    let first = receiver.recv(scheduler, &never).await.unwrap().unwrap();
    let second = receiver.recv(scheduler, &never).await.unwrap().unwrap();
    assert_eq!(first.payload(), b"abc");
    assert_eq!(second.payload(), b"def");
    let token = Cancellation::new();
    {
        let mut waiting = pin!(receiver.recv(scheduler, &token));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        token.cancel();
        assert!(waiting.await.is_err());
    }
    // Capacity-wait cancellation does not consume the receiver.
    let frozen = first.into_payload().freeze();
    let view = frozen.clone().slice(1..);
    drop(frozen);
    drop(second);
    scheduler
        .socket_shutdown(sender, std::net::Shutdown::Write)
        .await
        .unwrap();
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
    receiver.close(scheduler).await.unwrap();
    drop(receiver);
    assert_eq!(view.as_ref(), b"bc");
    drop(view);
    assert!(pool.try_acquire().is_some());
}

pub async fn datagram<S>(scheduler: &S, sender: &S::Socket, socket: &S::Socket)
where
    S: SocketReceiving + SocketSendMsg + SocketLocalAddress,
{
    let never = Cancellation::never();
    let pool = scheduler.buffer_pool(BufferPoolOptions::new(2, 3)).unwrap();
    let mut receiver = scheduler
        .socket_receiver(socket, pool.clone(), SocketRecvFlags::default(), &never)
        .await
        .unwrap();
    let address = scheduler.socket_local_address(socket).unwrap();
    for bytes in [b"".as_slice(), b"abcdef"] {
        let message = SocketMessage {
            buffers: [bytes.to_vec()],
            address: Some(address),
        };
        let (result, _) = scheduler
            .socket_sendmsg(sender, message, SocketSendFlags::default(), &never)
            .await;
        assert_eq!(result.unwrap(), bytes.len());
        let mut message = receiver.recv(scheduler, &never).await.unwrap().unwrap();
        assert_eq!(message.payload(), &bytes[..bytes.len().min(3)]);
        assert_eq!(message.truncated(), bytes.len() > 3);
        let source = message.address().map(|address| address.to_socket_address());
        assert!(source.is_some());
        if !message.payload().is_empty() {
            message.payload_mut()[0] = b'X';
        }
        assert_eq!(
            message.address().map(|address| address.to_socket_address()),
            source
        );
        assert_eq!(message.truncated(), bytes.len() > 3);
        let payload = message.into_payload();
        assert_eq!(payload.as_ref().len(), bytes.len().min(3));
        if !bytes.is_empty() {
            assert_eq!(payload.as_ref(), b"Xbc");
        }
    }
    receiver.close(scheduler).await.unwrap();
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
}

/// A bounded portable receiver preserves datagrams, metadata, empty messages, truncation, and lease lifetime across cancellation and close.
pub async fn messages<
    S: SocketOpen + SocketBind + SocketLocalAddress + SocketClose + SocketSendMsg + SocketReceiving,
>(
    scheduler: &S,
) {
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
    let cancelled = Cancellation::new();
    cancelled.cancel();
    crate::socket::cancelled(
        scheduler
            .socket_receiver(
                &socket,
                pool.clone(),
                SocketRecvFlags::default(),
                &cancelled,
            )
            .await,
    );
    assert!(
        scheduler
            .socket_receiver(
                &socket,
                pool.clone(),
                SocketRecvFlags { peek: true },
                &never
            )
            .await
            .is_err()
    );
    drop(scheduler.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never));
    let mut receiver = scheduler
        .socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never)
        .await
        .unwrap();
    // Pre-cancellation must leave a persistent receiver usable.
    crate::socket::cancelled(receiver.recv(scheduler, &cancelled).await);
    let (sent, _) = scheduler
        .socket_sendmsg(
            &sender,
            SocketMessage {
                buffers: [b"abcdef".to_vec()],
                address: Some(address),
            },
            SocketSendFlags::default(),
            &never,
        )
        .await;
    assert_eq!(sent.unwrap(), 6);
    let first = receiver.recv(scheduler, &never).await.unwrap().unwrap();
    assert_eq!(first.payload(), b"abcd");
    assert_eq!(first.payload().len(), 4);
    assert_eq!(
        first.address().map(|address| address.to_socket_address()),
        Some(source)
    );
    assert!(first.truncated());
    let (sent, _) = scheduler
        .socket_sendmsg(
            &sender,
            SocketMessage {
                buffers: [Vec::<u8>::new()],
                address: Some(address),
            },
            SocketSendFlags::default(),
            &never,
        )
        .await;
    assert_eq!(sent.unwrap(), 0);
    let mut waiting = Box::pin(receiver.recv(scheduler, &never));
    assert!(poll_once(&mut waiting).await.is_none());
    assert_eq!(first.payload(), b"abcd");
    drop(first);
    let empty = waiting.await.unwrap().unwrap();
    assert!(empty.payload().is_empty());
    // Cancellation while capacity is exhausted preserves the receiver and the returned lease.
    let token = Cancellation::new();
    let mut waiting = Box::pin(receiver.recv(scheduler, &token));
    assert!(poll_once(&mut waiting).await.is_none());
    token.cancel();
    crate::socket::cancelled(waiting.await);
    // With the same lease retained, another receive must still wait for capacity.
    let mut retry = Box::pin(receiver.recv(scheduler, &never));
    assert!(poll_once(&mut retry).await.is_none());
    drop(retry);
    receiver.close(scheduler).await.unwrap();
    receiver.close(scheduler).await.unwrap();
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
    drop(receiver);
    assert!(empty.payload().is_empty());
    drop(empty);
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

/// Stream receivers preserve ordering across leases, EOF, and explicit close. Non-power-of-two buffer counts are portable, even when a kernel ring rounds up.
pub async fn stream_eof<
    S: SocketOpen
        + SocketBind
        + SocketListen
        + SocketConnect
        + SocketAccept
        + SocketLocalAddress
        + SocketPeerAddress
        + SocketClose
        + SocketWrite
        + SocketShutdown
        + SocketReceiving,
>(
    scheduler: &S,
) {
    let never = Cancellation::never();
    let (sender, socket) = stream_pair(scheduler, SocketDomain::Ipv4).await;
    let mut receiver = scheduler
        .socket_receiver(
            &socket,
            scheduler.buffer_pool(BufferPoolOptions::new(3, 4)).unwrap(),
            SocketRecvFlags::default(),
            &never,
        )
        .await
        .unwrap();
    let mut sent = 0;
    while sent < 9 {
        let (result, _) = scheduler
            .socket_write(&sender, b"abcdefghi"[sent..].to_vec(), &never)
            .await;
        let count = result.unwrap();
        assert!(count > 0);
        sent += count;
    }
    scheduler
        .socket_shutdown(&sender, Shutdown::Write)
        .await
        .unwrap();
    let mut payload = Vec::new();
    while let Some(message) = receiver.recv(scheduler, &never).await.unwrap() {
        assert!(!message.payload().is_empty());
        assert!(!message.truncated());
        payload.extend_from_slice(message.payload());
    }
    assert_eq!(payload, b"abcdefghi");
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
    receiver.close(scheduler).await.unwrap();
    drop(receiver);
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}
