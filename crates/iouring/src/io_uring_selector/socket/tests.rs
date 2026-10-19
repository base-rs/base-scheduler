// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::super::tests::{Failures, drain, drive, pair};
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
        let _failures = Failures::new(if write {
            vec![("socket-send", io::ErrorKind::WouldBlock.into())]
        } else {
            Vec::new()
        });
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

#[test]
fn ready_owned_sends_bypass_full_ring_capacity_without_allocating() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let never = Cancellation::never();
    let permit = block_on(selector.controllers[0].capacity.acquire(&never)).unwrap();
    selector.controllers[0]
        .next_identifier
        .store(CANCEL_TAG - 1, Ordering::Relaxed);
    let buffer = vec![1, 2, 3];
    let pointer = buffer.as_ptr();
    let allocations = {
        let mut operation = pin!(selector.socket_write(&sender, buffer, &never));
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        allocation_counter::measure(|| {
            let Poll::Ready((result, returned)) = operation.as_mut().poll(&mut context) else {
                panic!("ready writes bypass ring admission");
            };
            assert_eq!(result.unwrap(), 3);
            assert_eq!(returned.as_ptr(), pointer);
        })
    };
    assert_eq!(allocations.count_total, 0);
    let mut bytes = [0; 3];
    assert_eq!(
        receiver
            .try_recv(&mut bytes, SocketRecvFlags::default())
            .unwrap(),
        3
    );
    assert_eq!(bytes, [1, 2, 3]);
    drop(permit);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn owned_send_errors_and_interruption_preserve_the_owner() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let never = Cancellation::never();
    {
        let _failures = Failures::new([("socket-send", io::ErrorKind::PermissionDenied.into())]);
        let (result, returned) = block_on(selector.socket_write(&sender, vec![7], &never));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(returned, [7]);
    }
    {
        let _failures = Failures::new([("socket-send", io::ErrorKind::Interrupted.into())]);
        assert_eq!(
            drive(&mut driver, selector.socket_write(&sender, vec![9], &never))
                .0
                .unwrap(),
            1
        );
    }
    let token = Cancellation::new();
    token.cancel();
    let (result, buffer) = block_on(selector.socket_write(&sender, vec![3], &token));
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffer, [3]);
    selector.close();
    let (result, buffer) = block_on(selector.socket_write(&sender, vec![5], &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(buffer, [5]);
    drop((sender, receiver));
    drain(&selector, &mut driver);
}
