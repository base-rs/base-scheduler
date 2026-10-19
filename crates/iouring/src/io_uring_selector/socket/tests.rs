// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::super::tests::{drain, drive, pair};
use super::*;
use futures_lite::future::{block_on, poll_once};

#[test]
fn transient_socket_failures_retry_and_preserve_the_owner() {
    block_on(async {
        let (selector, mut driver) = pair(1);
        let socket = selector
            .socket_open(
                SocketDomain::Ipv4,
                SocketType::Datagram,
                None,
                &Cancellation::never(),
            )
            .await
            .unwrap();
        selector
            .socket_bind(
                &socket,
                "127.0.0.1:0".parse().unwrap(),
                &Cancellation::never(),
            )
            .await
            .unwrap();
        for kind in [io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock] {
            let mut attempts = 0;
            let (result, buffer) = retry_operation(
                &selector,
                &socket,
                vec![7],
                |buffer| {
                    attempts += 1;
                    let result = if attempts == 1 {
                        Err(kind.into())
                    } else {
                        Ok(1)
                    };
                    async move { (result, buffer) }
                },
                Interest::Writable,
                &Cancellation::never(),
            )
            .await;
            assert_eq!(result.unwrap(), 1);
            assert_eq!(attempts, 2);
            assert_eq!(buffer, [7]);
        }
        let token = Cancellation::new();
        let mut operation = Box::pin(retry_operation(
            &selector,
            &socket,
            vec![9],
            |buffer| async { (Err::<usize, _>(io::ErrorKind::WouldBlock.into()), buffer) },
            Interest::Readable,
            &token,
        ));
        assert!(poll_once(&mut operation).await.is_none());
        token.cancel();
        assert_eq!(
            operation.await.0.unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        drain(&selector, &mut driver);
    });
}

#[test]
fn scalar_socket_requests_use_ring_opcodes_and_release_before_notification() {
    let (selector, mut driver) = pair(2);
    let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let never = Cancellation::never();
    for write in [false, true] {
        let token = Cancellation::new();
        let mut operation: std::pin::Pin<Box<dyn Future<Output = BufferResult<Vec<u8>>>>> = if write
        {
            Box::pin(selector.socket_write(&sender, vec![7], &token))
        } else {
            Box::pin(selector.socket_read(&receiver, vec![7], &token))
        };
        assert!(block_on(poll_once(&mut operation)).is_none());
        let Command::Submit(mut request) = driver
            .reactor
            .as_ref()
            .unwrap()
            .incoming
            .try_recv()
            .unwrap()
        else {
            panic!("expected submission")
        };
        assert_eq!(
            request.entry().get_opcode() as u8,
            if write {
                opcode::Send::CODE
            } else {
                opcode::Recv::CODE
            }
        );
        token.cancel();
        assert!(block_on(poll_once(&mut operation)).is_none());
        assert!(matches!(
            driver
                .reactor
                .as_ref()
                .unwrap()
                .incoming
                .try_recv()
                .unwrap(),
            Command::Cancel(_)
        ));
        request.finish(Err(io::Error::from_raw_os_error(libc::ECANCELED)));
        let (result, buffer) = block_on(operation);
        assert!(matches!(result, Err(OperationError::Cancelled(_))));
        assert_eq!(buffer, [7]);
    }
    assert_eq!(
        drive(
            &mut driver,
            selector.socket_write(&sender, vec![1, 2, 3], &never)
        )
        .0
        .unwrap(),
        3
    );
    let (result, bytes) = drive(
        &mut driver,
        selector.socket_read(&receiver, vec![0; 3], &never),
    );
    assert_eq!(result.unwrap(), 3);
    assert_eq!(bytes, [1, 2, 3]);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
    drain(&selector, &mut driver);
}
