// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::Interest;

thread_local! {
    static READINESS_ERROR: std::cell::Cell<Option<io::ErrorKind>> = const { std::cell::Cell::new(None) };
}

// Inject reactor failures after a real readiness wait without invalidating a live descriptor.
pub(super) fn readiness_result(result: io::Result<()>) -> io::Result<()> {
    match READINESS_ERROR.replace(None) {
        Some(kind) => Err(kind.into()),
        None => result,
    }
}

#[test]
fn direct_nonblocking_calls_support_scalar_and_vectored_datagrams() {
    crate::test_support::run(|| {
        futures_lite::future::block_on(async {
            let sender =
                NativeSocket::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap().into())
                    .unwrap();
            let receiver =
                NativeSocket::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap().into())
                    .unwrap();
            sender
                .get_ref()
                .connect(&receiver.local_addr().unwrap().into())
                .unwrap();
            let mut target = [0; 3];
            assert_eq!(
                receiver
                    .try_recv(&mut target, SocketRecvFlags::default())
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::WouldBlock
            );
            assert_eq!(
                sender.try_send(b"one", SocketSendFlags::default()).unwrap(),
                3
            );
            assert_eq!(
                receiver
                    .io(Interest::Readable, |_| receiver
                        .try_recv(&mut target, SocketRecvFlags::default()))
                    .await
                    .unwrap(),
                3
            );
            assert_eq!(&target, b"one");
            assert_eq!(
                sender
                    .try_sendmsg(
                        &[io::IoSlice::new(b"t"), io::IoSlice::new(b"wo")],
                        None,
                        SocketSendFlags::default()
                    )
                    .unwrap(),
                3
            );
            let result = receiver
                .io(Interest::Readable, |_| {
                    receiver.try_recvmsg(
                        &mut [io::IoSliceMut::new(&mut target)],
                        SocketRecvFlags::default(),
                    )
                })
                .await
                .unwrap();
            assert_eq!(result.bytes, 3);
            assert_eq!(result.address, Some(sender.local_addr().unwrap()));
            assert_eq!(&target, b"two");
        })
    });
}
#[test]
fn failed_registration_returns_the_original_error() {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let error = NativeSocket::new_with(socket, |_| Err(io::ErrorKind::PermissionDenied.into()))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn transient_would_block_retries_through_writable_readiness() {
    crate::test_support::run(|| {
        let socket = NativeSocket::new(
            socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap(),
        )
        .unwrap();
        socket
            .get_ref()
            .bind(
                &"127.0.0.1:0"
                    .parse::<std::net::SocketAddr>()
                    .unwrap()
                    .into(),
            )
            .unwrap();
        let mut attempts = 0;
        let result = futures_lite::future::block_on(socket.io(Interest::Writable, |socket| {
            attempts += 1;
            if attempts <= 2 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            socket.local_addr()
        }))
        .unwrap();
        assert!(attempts >= 3);
        assert_eq!(result.as_socket().unwrap(), socket.local_addr().unwrap());
    });
}

#[test]
fn native_retention_preserves_the_registration_until_completion() {
    let socket = NativeSocket::new(
        socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap(),
    )
    .unwrap();
    socket
        .get_ref()
        .bind(
            &"127.0.0.1:0"
                .parse::<std::net::SocketAddr>()
                .unwrap()
                .into(),
        )
        .unwrap();
    // SAFETY: The simulated backend only observes the address and drops its retained owner before exclusive close.
    let retained = unsafe { socket.retain_native() };
    assert_eq!(Arc::strong_count(&socket.inner), 2);
    assert_eq!(
        retained.get_ref().local_addr().unwrap().as_socket(),
        Some(socket.local_addr().unwrap())
    );
    drop(retained);
    assert_eq!(Arc::strong_count(&socket.inner), 1);
    socket.into_close()().unwrap();
}

#[test]
fn unrelated_polls_do_not_retry_a_blocked_socket_operation() {
    use futures_lite::future::{block_on, poll_once};
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};
    crate::test_support::run(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let socket = super::NativeSocket::new(listener.accept().unwrap().0.into()).unwrap();
        let calls = AtomicUsize::new(0);
        let mut buffer = [0; 8];
        let mut operation = Box::pin(socket.io(crate::Interest::Readable, |socket| {
            calls.fetch_add(1, Ordering::Relaxed);
            crate::socket_io::recv_poll(
                socket,
                &mut buffer,
                base_scheduler::SocketRecvFlags::default(),
            )
        }));
        for _ in 0..5 {
            assert!(block_on(poll_once(&mut operation)).is_none());
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
        peer.write_all(b"ready").unwrap();
        assert_eq!(block_on(operation).unwrap(), 5);
        assert_eq!(&buffer[..5], b"ready");
    });
}

#[test]
fn readiness_errors_return_without_retrying_the_operation() {
    crate::test_support::run(|| {
        for interest in [Interest::Readable, Interest::Writable] {
            let socket =
                NativeSocket::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap().into())
                    .unwrap();
            let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            peer.send_to(b"ready", socket.local_addr().unwrap())
                .unwrap();
            READINESS_ERROR.set(Some(io::ErrorKind::PermissionDenied));
            let mut calls = 0;
            let error = futures_lite::future::block_on(socket.io(interest, |_| {
                calls += 1;
                Err::<(), _>(io::ErrorKind::WouldBlock.into())
            }))
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(calls, 1);
            assert_eq!(READINESS_ERROR.get(), None);
        }
    });
}
