// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Portable shared-pool receiver assertions.
use core_scheduler::*;
use futures_lite::future::poll_once;
use std::pin::pin;

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
        assert_eq!(message.buffer.as_ref(), b"a");
        drop(message);
        let (sent, _) = scheduler.socket_write(socket, b"b".to_vec(), &never).await;
        assert_eq!(sent.unwrap(), 1);
        let message = waiting.await.unwrap().unwrap();
        assert_eq!(message.buffer.as_ref(), b"b");
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
    assert_eq!(first.buffer.as_ref(), b"abc");
    assert_eq!(second.buffer.as_ref(), b"def");
    let token = Cancellation::new();
    {
        let mut waiting = pin!(receiver.recv(scheduler, &token));
        assert!(poll_once(waiting.as_mut()).await.is_none());
        token.cancel();
        assert!(waiting.await.is_err());
    }
    // Capacity-wait cancellation does not consume the receiver.
    let frozen = first.buffer.freeze();
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
        let message = receiver.recv(scheduler, &never).await.unwrap().unwrap();
        assert_eq!(message.buffer.as_ref(), &bytes[..bytes.len().min(3)]);
        assert_eq!(message.result.truncated, bytes.len() > 3);
        assert!(message.result.address.is_some());
    }
    receiver.close(scheduler).await.unwrap();
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
}
