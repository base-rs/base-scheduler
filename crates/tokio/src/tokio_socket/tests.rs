// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::time::Duration;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn socket() -> socket2::Socket {
    socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap()
}

#[test]
fn explicit_close_rejects_retained_socket_ownership() {
    let runtime = runtime();
    let descriptor = TokioSocket::new(runtime.handle(), socket()).unwrap();
    let retained = descriptor.retain();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| descriptor.into_close()));
    assert!(result.is_err());
    assert!(retained.get_ref().set_nonblocking(true).is_ok());
    retained.into_close()().unwrap();
}

#[test]
fn nonblocking_configuration_errors_are_returned() {
    let runtime = runtime();
    let error = TokioSocket::new_with(
        runtime.handle(),
        socket(),
        |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        |_, _| unreachable!("socket registration follows nonblocking configuration"),
    )
    .err()
    .unwrap();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn registration_errors_are_returned() {
    let runtime = runtime();
    let error = TokioSocket::new_with(
        runtime.handle(),
        socket(),
        |_| Ok(()),
        |_, _| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
    )
    .err()
    .unwrap();

    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn invalid_accept_returns_without_waiting_for_readiness() {
    let runtime = runtime();
    let socket = runtime.block_on(async { TokioSocket::new(runtime.handle(), socket()).unwrap() });

    let result = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(1),
            socket.io(Interest::Readable, |socket| socket.accept().map(|_| ())),
        )
        .await
    });

    assert!(
        result
            .expect("invalid accept waited for readiness")
            .is_err()
    );
}
#[test]
fn transient_would_block_retries_with_the_requested_readiness_direction() {
    use core_scheduler::{Cancellation, SocketDomain};
    crate::test_support::run(|| {
        futures_lite::future::block_on(async {
            let scheduler = crate::test_support::scheduler();
            let (client, server) =
                core_scheduler_conformance::socket::stream_pair(&scheduler, SocketDomain::Ipv4)
                    .await;
            // Model a socket becoming ready between the initial syscall and readiness registration. Both reactor directions must retry the actual syscall.
            let mut writes = 0;
            let sent = client
                .io(Interest::Writable, |socket| {
                    writes += 1;
                    if writes == 1 {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    crate::socket_io::send(socket, b"x")
                })
                .await
                .unwrap();
            assert_eq!(sent, 1);
            assert!(writes >= 2);
            let mut reads = 0;
            let mut buffer = [0];
            let received = server
                .io(Interest::Readable, |socket| {
                    reads += 1;
                    if reads == 1 {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    crate::socket_io::recv(socket, &mut buffer)
                })
                .await
                .unwrap();
            assert_eq!(received, 1);
            assert_eq!(&buffer, b"x");
            assert!(reads >= 2);
            // Confirm normal public operations still use the same registration.
            use core_scheduler::{SocketClose, SocketWait};
            scheduler
                .socket_wait(&client, Interest::Writable, &Cancellation::never())
                .await
                .unwrap();
            scheduler.socket_close(client).await.unwrap();
            scheduler.socket_close(server).await.unwrap();
            scheduler.shutdown().await;
        })
    });
}
