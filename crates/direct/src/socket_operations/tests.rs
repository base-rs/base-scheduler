// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use futures_lite::future::{block_on, poll_once};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(10);
fn scheduler(capacity: usize) -> DirectScheduler {
    DirectScheduler::new(crate::DirectOptions {
        threads: 1,
        capacity,
    })
    .unwrap()
}
fn socket() -> DirectSocket {
    DirectSocket::new(socket_io::open(SocketDomain::Ipv4, SocketType::Datagram, None).unwrap())
}

#[test]
fn pending_socket_attempts_release_capacity_and_observe_cancellation_and_scheduler_close() {
    let scheduler = scheduler(1);
    let socket = socket();
    let token = Cancellation::new();
    let (started, start) = mpsc::channel();
    let buffer = vec![1; 4];
    let allocation = buffer.as_ptr();
    let mut pending = Box::pin(
        scheduler.socket_operation(&socket, buffer, &token, move |_, _| {
            let _ = started.send(());
            Err::<usize, _>(io::ErrorKind::WouldBlock.into())
        }),
    );
    assert!(block_on(poll_once(&mut pending)).is_none());
    start.recv_timeout(TIMEOUT).unwrap();
    block_on(scheduler.work.drain());
    assert!(block_on(poll_once(&mut pending)).is_none());
    // The pending socket occupies neither the sole worker nor the sole admission slot.
    let work = block_on(scheduler.spawn_blocking(|| 42, &Cancellation::never())).unwrap();
    assert_eq!(block_on(work).unwrap(), 42);
    token.cancel();
    let (result, returned) = block_on(pending);
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(returned.as_ptr(), allocation);

    let never = Cancellation::never();
    let (started, start) = mpsc::channel();
    let mut pending =
        Box::pin(
            scheduler.socket_operation(&socket, vec![2], &never, move |_, _| {
                let _ = started.send(());
                Err::<usize, _>(io::ErrorKind::WouldBlock.into())
            }),
        );
    assert!(block_on(poll_once(&mut pending)).is_none());
    start.recv_timeout(TIMEOUT).unwrap();
    block_on(scheduler.work.drain());
    assert!(block_on(poll_once(&mut pending)).is_none());
    block_on(scheduler.shutdown());
    let (result, returned) = block_on(pending);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(returned, vec![2]);
}

#[test]
fn interrupted_attempts_retry_and_retry_service_errors_return_input() {
    let mut scheduler = scheduler(1);
    let socket = socket();
    let mut calls = 0;
    let (result, buffer) = block_on(scheduler.socket_operation(
        &socket,
        vec![3],
        &Cancellation::never(),
        move |_, _| {
            calls += 1;
            if calls == 1 {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                Ok(11)
            }
        },
    ));
    assert_eq!(result.unwrap(), 11);
    assert_eq!(buffer, vec![3]);
    // A fresh scheduler has not initialized its retry worker yet.
    block_on(scheduler.shutdown());
    scheduler = super::tests::scheduler(1);
    scheduler.socket_retry.fail_start = true;
    let (result, buffer) = block_on(scheduler.socket_operation(
        &socket,
        vec![5],
        &Cancellation::never(),
        |_, _| Err::<usize, _>(io::ErrorKind::WouldBlock.into()),
    ));
    assert_eq!(
        result.unwrap_err().to_string(),
        "injected socket retry startup failure"
    );
    assert_eq!(buffer, vec![5]);
    block_on(scheduler.shutdown());
}

#[test]
fn socket_cleanup_is_independent_bounded_and_preserves_closed_admission_errors() {
    let scheduler = scheduler(1);
    let (started, start) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let busy = block_on(scheduler.spawn_blocking(
        move || {
            started.send(()).unwrap();
            released.recv_timeout(TIMEOUT).unwrap();
        },
        &Cancellation::never(),
    ))
    .unwrap();
    start.recv_timeout(TIMEOUT).unwrap();
    block_on(scheduler.socket_close(socket())).unwrap();
    release.send(()).unwrap();
    block_on(busy).unwrap();

    let (started, start) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let busy = block_on(scheduler.cleanup.submit(move || {
        started.send(()).unwrap();
        released.recv_timeout(TIMEOUT).unwrap();
    }))
    .unwrap();
    start.recv_timeout(TIMEOUT).unwrap();
    let socket = socket();
    let weak = Arc::downgrade(&socket.0);
    let mut close = Box::pin(scheduler.socket_close(socket));
    assert!(block_on(poll_once(&mut close)).is_none());
    drop(close);
    assert_eq!(weak.strong_count(), 0);
    release.send(()).unwrap();
    block_on(busy).unwrap().unwrap();
    block_on(scheduler.shutdown());
    for submit in [false, true] {
        let socket = super::tests::socket();
        let error = if submit {
            block_on(scheduler.socket_close_submit(socket))
        } else {
            block_on(scheduler.socket_close(socket))
        }
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        let socket = super::tests::socket();
        let error = if submit {
            block_on(scheduler.socket_shutdown_submit(&socket, Shutdown::Both))
        } else {
            block_on(scheduler.socket_shutdown(&socket, Shutdown::Both))
        }
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}

#[test]
fn accepted_shutdown_and_close_survive_abandonment_and_drain_during_shutdown() {
    for submit in [false, true] {
        let scheduler = scheduler(2);
        let (client, server) = block_on(protocol_scheduler_conformance::socket::stream_pair(
            &scheduler,
            SocketDomain::Ipv4,
        ));
        let (started, start) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let busy = block_on(scheduler.cleanup.submit(move || {
            started.send(()).unwrap();
            released.recv_timeout(TIMEOUT).unwrap();
        }))
        .unwrap();
        start.recv_timeout(TIMEOUT).unwrap();
        if submit {
            block_on(scheduler.socket_shutdown_submit(&client, Shutdown::Write)).unwrap();
        } else {
            let mut shutdown = Box::pin(scheduler.socket_shutdown(&client, Shutdown::Write));
            assert!(block_on(poll_once(&mut shutdown)).is_none());
            drop(shutdown);
        }
        let weak = Arc::downgrade(&client.0);
        drop(client);
        assert_eq!(weak.strong_count(), 1);
        release.send(()).unwrap();
        block_on(busy).unwrap().unwrap();
        block_on(scheduler.cleanup.drain());
        assert_eq!(weak.strong_count(), 0);
        let (result, _) =
            block_on(scheduler.socket_read(&server, vec![0; 1], &Cancellation::never()));
        assert_eq!(result.unwrap(), 0);
        let (started, start) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let busy = block_on(scheduler.cleanup.submit(move || {
            started.send(()).unwrap();
            released.recv_timeout(TIMEOUT).unwrap();
        }))
        .unwrap();
        start.recv_timeout(TIMEOUT).unwrap();
        let mut close = Box::pin(scheduler.socket_close(server));
        assert!(block_on(poll_once(&mut close)).is_none());
        drop(close);
        let mut shutdown = Box::pin(scheduler.shutdown());
        assert!(block_on(poll_once(&mut shutdown)).is_none());
        release.send(()).unwrap();
        block_on(busy).unwrap().unwrap();
        block_on(shutdown);
    }
}

#[test]
fn exclusive_close_rejects_retained_access_and_unpolled_close_drops_normally() {
    let scheduler = scheduler(1);
    let socket = socket();
    let retained = socket.retain();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(
            scheduler.socket_close(socket)
        )))
        .is_err()
    );
    assert!(retained.local_addr().is_ok());
    let socket = super::tests::socket();
    let weak = Arc::downgrade(&socket.0);
    drop(scheduler.socket_close(socket));
    assert_eq!(weak.strong_count(), 0);
    block_on(scheduler.shutdown());
}

#[test]
fn vectored_sends_preserve_short_counts_without_implicitly_advancing_the_owner() {
    let scheduler = scheduler(1);
    let (client, server) = block_on(protocol_scheduler_conformance::socket::stream_pair(
        &scheduler,
        SocketDomain::Ipv4,
    ));
    client.0.set_send_buffer_size(4096).unwrap();
    server.0.set_recv_buffer_size(4096).unwrap();
    block_on(
        protocol_scheduler_conformance::socket_messages::partial_send(
            &scheduler,
            &client,
            [vec![1; 2 * 1024 * 1024], vec![2; 2 * 1024 * 1024]],
        ),
    );
    block_on(scheduler.socket_close(client)).unwrap();
    block_on(scheduler.socket_close(server)).unwrap();
    block_on(scheduler.shutdown());
}
