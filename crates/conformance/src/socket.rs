// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Reusable IP socket assertions. Fixtures supply isolated connections and enforce test deadlines.

use crate::{SocketIoFixture, SocketIoGuard};
use core_scheduler::*;
use futures_lite::future::{poll_once, zip};
use std::{
    future::{Future, poll_fn},
    net::Shutdown,
    pin::pin,
};

fn cancelled<T>(result: OperationResult<T>) {
    assert!(
        matches!(result, Err(OperationError::Cancelled(_))),
        "expected cooperative cancellation"
    );
}

/// Create an isolated loopback connection, exercising separate open/bind/listen/connect/accept operations and pre-cancellation. The listener is explicitly closed; the caller owns both returned ends.
pub async fn stream_pair<S: SocketScheduler>(
    scheduler: &S,
    domain: SocketDomain,
) -> (S::Socket, S::Socket) {
    let never = Cancellation::never();
    let token = Cancellation::new();
    token.cancel();
    cancelled(
        scheduler
            .socket_open(
                domain,
                SocketType::Stream,
                Some(SocketProtocol::Tcp),
                &token,
            )
            .await,
    );
    drop(scheduler.socket_open(domain, SocketType::Stream, None, &never));
    let listener = scheduler
        .socket_open(
            domain,
            SocketType::Stream,
            Some(SocketProtocol::Tcp),
            &never,
        )
        .await
        .unwrap();
    let local = match domain {
        SocketDomain::Ipv4 => "127.0.0.1:0",
        SocketDomain::Ipv6 => "[::1]:0",
    }
    .parse()
    .unwrap();
    let unbound_address = scheduler.socket_local_address(&listener).ok();
    drop(scheduler.socket_bind(&listener, local, &never));
    cancelled(scheduler.socket_bind(&listener, local, &token).await);
    // Winsock rejects address lookup before binding; Unix generally reports port zero.
    assert_eq!(
        scheduler.socket_local_address(&listener).ok(),
        unbound_address
    );
    scheduler
        .socket_bind(&listener, local, &never)
        .await
        .unwrap();
    let local = scheduler.socket_local_address(&listener).unwrap();
    assert_ne!(local.port(), 0);
    drop(scheduler.socket_listen(&listener, 8, &never));
    cancelled(scheduler.socket_listen(&listener, 8, &token).await);
    scheduler.socket_listen(&listener, 8, &never).await.unwrap();
    let client = scheduler
        .socket_open(domain, SocketType::Stream, None, &never)
        .await
        .unwrap();
    // A prior explicit client binding must survive connection.
    scheduler
        .socket_bind(
            &client,
            match domain {
                SocketDomain::Ipv4 => "127.0.0.1:0",
                SocketDomain::Ipv6 => "[::1]:0",
            }
            .parse()
            .unwrap(),
            &never,
        )
        .await
        .unwrap();
    let client_address = scheduler.socket_local_address(&client).unwrap();
    drop(scheduler.socket_connect(&client, local, &never));
    cancelled(scheduler.socket_connect(&client, local, &token).await);
    assert!(scheduler.socket_peer_address(&client).is_err());
    drop(scheduler.socket_accept(&listener, &never));
    cancelled(scheduler.socket_accept(&listener, &token).await);
    let (connected, accepted) = zip(
        scheduler.socket_connect(&client, local, &never),
        scheduler.socket_accept(&listener, &never),
    )
    .await;
    connected.unwrap();
    let (server, peer) = accepted.unwrap();
    assert_eq!(peer, client_address);
    assert_eq!(
        scheduler.socket_local_address(&client).unwrap(),
        client_address
    );
    assert_eq!(scheduler.socket_peer_address(&client).unwrap(), local);
    assert_eq!(scheduler.socket_local_address(&server).unwrap(), local);
    assert_eq!(scheduler.socket_peer_address(&server).unwrap(), peer);
    scheduler.socket_close(listener).await.unwrap();
    (client, server)
}

/// Check lazy and pre-cancelled scalar I/O, allocation identity, short reads, readiness, write shutdown, and EOF on a fresh connected pair.
pub async fn stream<S: SocketRead + SocketWrite + SocketWait + SocketShutdown>(
    scheduler: &S,
    sender: &S::Socket,
    receiver: &S::Socket,
) {
    pre_cancelled(scheduler, sender, receiver).await;
    transfer(scheduler, sender, receiver).await;
    shutdown(scheduler, sender, receiver).await;
}

/// Pre-cancelled scalar I/O preserves the original owners and submits no payload.
pub async fn pre_cancelled<S: SocketRead + SocketWrite + SocketWait>(
    scheduler: &S,
    sender: &S::Socket,
    receiver: &S::Socket,
) {
    let never = Cancellation::never();
    let token = Cancellation::new();
    token.cancel();
    cancelled(
        scheduler
            .socket_wait(sender, Interest::Writable, &token)
            .await,
    );
    scheduler
        .socket_wait(sender, Interest::Writable, &never)
        .await
        .unwrap();
    let source = b"abcdef".to_vec();
    let allocation = source.as_ptr() as usize;
    let (result, source) = scheduler.socket_write(sender, source, &token).await;
    cancelled(result);
    assert_eq!(source.as_ptr() as usize, allocation);
    // A distinct sentinel exposes writes which incorrectly transferred bytes while reporting cancellation, and reads which consumed data despite pre-cancellation.
    let (result, _) = scheduler.socket_write(sender, vec![0xff], &never).await;
    assert_eq!(result.unwrap(), 1);
    let target = vec![0x5a; 2];
    let target_allocation = target.as_ptr() as usize;
    let (result, target) = scheduler.socket_read(receiver, target, &token).await;
    cancelled(result);
    assert_eq!(target.as_ptr() as usize, target_allocation);
    assert_eq!(target, vec![0x5a; 2]);
    let (result, buffer) = scheduler.socket_read(receiver, vec![0; 1], &never).await;
    assert_eq!(result.unwrap(), 1);
    assert_eq!(buffer, vec![0xff]);
}

/// Check lazy scalar operations, short transfers, readiness, payload, and allocation identity on a fresh stream.
pub async fn transfer<S: SocketRead + SocketWrite + SocketWait>(
    scheduler: &S,
    sender: &S::Socket,
    receiver: &S::Socket,
) {
    let never = Cancellation::never();
    let mut source = b"abcdef".to_vec();
    let allocation = source.as_ptr() as usize;
    let mut target = vec![0; 2];
    let target_allocation = target.as_ptr() as usize;
    drop(scheduler.socket_write(sender, b"unpolled".to_vec(), &never));
    let mut sent = 0;
    while sent < 6 {
        let (result, view) = scheduler
            .socket_write(sender, source.slice(sent..), &never)
            .await;
        let count = result.unwrap();
        assert!(count > 0 && count <= 6 - sent);
        sent += count;
        source = view.into_inner();
    }
    assert_eq!(source.as_ptr() as usize, allocation);
    scheduler
        .socket_wait(receiver, Interest::Readable, &never)
        .await
        .unwrap();
    drop(scheduler.socket_read(receiver, vec![0; 6], &never));
    let mut received = Vec::new();
    while received.len() < 6 {
        let (result, returned) = scheduler.socket_read(receiver, target, &never).await;
        let count = result.unwrap();
        assert!(count > 0 && count <= 2);
        assert_eq!(returned.as_ptr() as usize, target_allocation);
        assert_eq!(returned.len(), 2);
        received.extend_from_slice(&returned[..count]);
        target = returned;
    }
    assert_eq!(received, b"abcdef");
}

/// Check lazy write shutdown and EOF on a fresh stream.
pub async fn shutdown<S: SocketRead + SocketWrite + SocketShutdown>(
    scheduler: &S,
    sender: &S::Socket,
    receiver: &S::Socket,
) {
    let never = Cancellation::never();
    let target = vec![0; 2];
    let target_allocation = target.as_ptr() as usize;
    drop(scheduler.socket_shutdown(sender, Shutdown::Write));
    // The dropped shutdown must not prevent this write.
    let (result, _) = scheduler.socket_write(sender, vec![7], &never).await;
    assert_eq!(result.unwrap(), 1);
    let (result, _) = scheduler.socket_read(receiver, vec![0; 1], &never).await;
    assert_eq!(result.unwrap(), 1);
    scheduler
        .socket_shutdown(sender, Shutdown::Write)
        .await
        .unwrap();
    let (result, returned) = scheduler.socket_read(receiver, target, &never).await;
    assert_eq!(result.unwrap(), 0);
    assert_eq!(returned.as_ptr() as usize, target_allocation);
}

/// Cancel a pending read and readiness waiter on a quiet connected stream. No payload may arrive during these assertions.
pub async fn pending_cancellation<S: SocketRead + SocketWait>(scheduler: &S, socket: &S::Socket) {
    pending_read_cancellation(scheduler, socket).await;
    pending_wait_cancellation(scheduler, socket).await;
}

/// Cancel a pending read on a quiet stream and check that initialized bytes and allocation identity survive.
pub async fn pending_read_cancellation<S: SocketRead>(scheduler: &S, socket: &S::Socket) {
    let token = Cancellation::new();
    let buffer = vec![0x5a; 8];
    let allocation = buffer.as_ptr() as usize;
    let mut reading = pin!(scheduler.socket_read(socket, buffer, &token));
    assert!(
        poll_once(&mut reading).await.is_none(),
        "quiet stream read unexpectedly completed"
    );
    token.cancel();
    let (result, buffer) = reading.await;
    cancelled(result);
    assert_eq!(buffer.as_ptr() as usize, allocation);
    assert_eq!(buffer, vec![0x5a; 8]);
}

/// Cancel readiness waiting on a quiet stream, allowing spurious readiness notifications.
pub async fn pending_wait_cancellation<S: SocketWait>(scheduler: &S, socket: &S::Socket) {
    let token = Cancellation::new();
    let mut waiting = pin!(scheduler.socket_wait(socket, Interest::Readable, &token));
    if let Some(result) = poll_once(&mut waiting).await {
        // Readiness is allowed to be spurious. Pre-cancellation must still win on the next wait.
        result.unwrap();
        token.cancel();
        cancelled(
            scheduler
                .socket_wait(socket, Interest::Readable, &token)
                .await,
        );
    } else {
        token.cancel();
        cancelled(waiting.await);
    }
}

/// Check that a failed write preserves the original owner. The fixture supplies a socket whose writes fail, without relying on portable error kinds for broken connections.
pub async fn write_error<S: SocketWrite>(scheduler: &S, socket: &S::Socket) {
    let buffer = vec![7; 3];
    let allocation = buffer.as_ptr() as usize;
    let (result, buffer) = scheduler
        .socket_write(socket, buffer, &Cancellation::never())
        .await;
    assert!(matches!(result, Err(OperationError::Io(_))));
    assert_eq!(buffer, vec![7; 3]);
    assert_eq!(buffer.as_ptr() as usize, allocation);
}

/// A successful already-started I/O attempt wins concurrent cancellation and returns its original input.
pub async fn completion_wins<F: SocketIoFixture>(fixture: &F) {
    let token = Cancellation::new();
    let buffer = fixture.input();
    let mut reading = pin!(fixture.operation(buffer, &token));
    assert!(poll_once(&mut reading).await.is_none());
    let mut starting = pin!(fixture.start());
    // Submission may itself await capacity. Keep driving the operation until the fixture reaches its access boundary.
    let mut guard = poll_fn(|context| {
        assert!(
            reading.as_mut().poll(context).is_pending(),
            "operation completed before fixture release"
        );
        starting.as_mut().poll(context)
    })
    .await;
    token.cancel();
    fixture.assert_retained();
    guard.release();
    let (result, buffer) = reading.await;
    fixture.assert_completed(&result.unwrap(), &buffer);
    drop(buffer);
    guard.retired().await;
    fixture.assert_released();
}

/// Abandoning an accepted operation cannot destroy its input while backend access remains possible.
pub async fn abandoned_operation<F: SocketIoFixture>(fixture: &F) {
    let token = Cancellation::new();
    let mut reading = Box::pin(fixture.operation(fixture.input(), &token));
    assert!(poll_once(&mut reading).await.is_none());
    let mut starting = pin!(fixture.start());
    let mut guard = poll_fn(|context| {
        assert!(
            reading.as_mut().poll(context).is_pending(),
            "operation completed before fixture release"
        );
        starting.as_mut().poll(context)
    })
    .await;
    drop(reading);
    fixture.assert_retained();
    guard.release();
    guard.retired().await;
    fixture.assert_released();
}

/// A bounded portable receiver preserves datagrams, metadata, empty messages,
/// truncation, and lease lifetime across cancellation and close.
pub async fn multishot_messages<S: SocketScheduler + SocketSendMsg + SocketRecvMsgMultishot>(
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
    let options = RecvMsgMultishotOptions {
        buffers: 1,
        buffer_size: 4,
    };
    let cancelled = Cancellation::new();
    cancelled.cancel();
    self::cancelled(
        scheduler
            .socket_recvmsg_multishot(&socket, options, SocketRecvFlags::default(), &cancelled)
            .await,
    );
    assert!(
        scheduler
            .socket_recvmsg_multishot(&socket, options, SocketRecvFlags { peek: true }, &never)
            .await
            .is_err()
    );
    drop(scheduler.socket_recvmsg_multishot(&socket, options, SocketRecvFlags::default(), &never));
    let mut receiver = scheduler
        .socket_recvmsg_multishot(&socket, options, SocketRecvFlags::default(), &never)
        .await
        .unwrap();
    // Pre-cancellation must leave a persistent receiver usable.
    self::cancelled(receiver.recv(scheduler, &cancelled).await);
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
    assert_eq!(first.buffer.as_ref(), b"abcd");
    assert_eq!(first.result.bytes, 4);
    assert_eq!(first.result.address, Some(source));
    assert!(first.result.truncated);
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
    assert_eq!(first.buffer.as_ref(), b"abcd");
    drop(first);
    let empty = waiting.await.unwrap().unwrap();
    assert!(empty.buffer.as_ref().is_empty());
    // Cancellation while capacity is exhausted closes the receiver, without
    // invalidating the returned lease or requiring it to be released first.
    let token = Cancellation::new();
    let mut waiting = Box::pin(receiver.recv(scheduler, &token));
    assert!(poll_once(&mut waiting).await.is_none());
    token.cancel();
    self::cancelled(waiting.await);
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
    receiver.close(scheduler).await.unwrap();
    receiver.close(scheduler).await.unwrap();
    drop(receiver);
    assert!(empty.buffer.as_ref().is_empty());
    drop(empty);
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}

/// Stream receivers preserve ordering across leases, EOF, and explicit close.
/// Non-power-of-two buffer counts are portable, even when a kernel ring rounds up.
pub async fn multishot_stream<S: SocketScheduler + SocketRecvMsgMultishot>(scheduler: &S) {
    let never = Cancellation::never();
    let (sender, socket) = stream_pair(scheduler, SocketDomain::Ipv4).await;
    let mut receiver = scheduler
        .socket_recvmsg_multishot(
            &socket,
            RecvMsgMultishotOptions {
                buffers: 3,
                buffer_size: 4,
            },
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
        assert!(!message.buffer.as_ref().is_empty());
        assert_eq!(message.result.bytes, message.buffer.as_ref().len());
        assert!(!message.result.truncated);
        payload.extend_from_slice(message.buffer.as_ref());
    }
    assert_eq!(payload, b"abcdefghi");
    assert!(receiver.recv(scheduler, &never).await.unwrap().is_none());
    receiver.close(scheduler).await.unwrap();
    scheduler.socket_close(socket).await.unwrap();
    scheduler.socket_close(sender).await.unwrap();
}
