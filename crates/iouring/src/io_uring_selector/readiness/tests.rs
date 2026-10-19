// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::super::tests::{drain, drive, pair};
use super::*;
use futures_lite::future::{block_on, poll_once};
use std::task::{Context, Waker};

fn streams(selector: &IoUringSelector) -> (NativeSocket, NativeSocket) {
    block_on(core_scheduler_conformance::socket::stream_pair(
        selector,
        SocketDomain::Ipv4,
    ))
}

fn submitted(driver: &IoUringDriver) -> Box<dyn PendingRequest> {
    match driver
        .reactor
        .as_ref()
        .unwrap()
        .incoming
        .try_recv()
        .unwrap()
    {
        Command::Submit(request) => request,
        _ => panic!("expected readiness submission"),
    }
}

#[test]
fn ready_scalar_and_vector_io_allocate_nothing_even_when_ring_capacity_is_full() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    let never = Cancellation::never();
    let mut waiting = Box::pin(selector.socket_wait(&receiver, Interest::Readable, &never));
    assert!(block_on(poll_once(&mut waiting)).is_none());
    let mut context = Context::from_waker(Waker::noop());
    let mut buffer = [0; 3];
    let allocations = allocation_counter::measure(|| {
        assert!(matches!(
            pin!(selector.socket_send_poll(&sender, b"one", SocketSendFlags::default(), &never))
                .poll(&mut context),
            Poll::Ready(Ok(3))
        ));
        assert!(matches!(
            pin!(selector.socket_recv_poll(
                &receiver,
                &mut buffer,
                SocketRecvFlags::default(),
                &never
            ))
            .poll(&mut context),
            Poll::Ready(Ok(3))
        ));
        assert_eq!(&buffer, b"one");
        assert!(matches!(
            pin!(selector.socket_sendmsg_poll(
                &sender,
                &[IoSlice::new(b"t"), IoSlice::new(b"wo")],
                None,
                SocketSendFlags::default(),
                &never
            ))
            .poll(&mut context),
            Poll::Ready(Ok(3))
        ));
        assert!(matches!(
            pin!(selector.socket_recvmsg_poll(
                &receiver,
                &mut [IoSliceMut::new(&mut buffer)],
                SocketRecvFlags::default(),
                &never
            ))
            .poll(&mut context),
            Poll::Ready(Ok(SocketRecvResult { bytes: 3, .. }))
        ));
        assert_eq!(&buffer, b"two");
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(
        selector.controllers[0]
            .next_identifier
            .load(Ordering::Relaxed),
        1
    );
    drop(waiting);
    drain(&selector, &mut driver);
}

#[test]
fn pending_read_uses_polladd_and_observes_data_arriving_before_submission() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    let never = Cancellation::never();
    let mut buffer = [0; 1];
    let mut receiving = Box::pin(selector.socket_recv_poll(
        &receiver,
        &mut buffer,
        SocketRecvFlags::default(),
        &never,
    ));
    assert!(block_on(poll_once(&mut receiving)).is_none());
    let mut request = submitted(&driver);
    assert_eq!(request.entry().get_opcode() as u8, opcode::PollAdd::CODE);
    // Data can arrive between the failed syscall and publication of PollAdd. Its level readiness must still complete the wait.
    assert_eq!(
        sender.try_send(b"x", SocketSendFlags::default()).unwrap(),
        1
    );
    selector.controllers[0]
        .send(Command::Submit(request))
        .unwrap();
    assert_eq!(drive(&mut driver, receiving).unwrap(), 1);
    assert_eq!(&buffer, b"x");
    block_on(selector.socket_close(receiver)).unwrap();
    block_on(selector.socket_close(sender)).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn ready_waits_and_cancelled_waits_retire_before_returning() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    drive(
        &mut driver,
        selector.socket_wait(&sender, Interest::Writable, &Cancellation::never()),
    )
    .unwrap();
    let token = Cancellation::new();
    let mut waiting = Box::pin(selector.socket_wait(&receiver, Interest::Readable, &token));
    assert!(block_on(poll_once(&mut waiting)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    token.cancel();
    assert!(matches!(
        drive(&mut driver, waiting),
        Err(OperationError::Cancelled(_))
    ));
    block_on(selector.socket_close(receiver)).unwrap();
    block_on(selector.socket_close(sender)).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn readiness_errors_and_unpublished_shutdown_release_the_socket() {
    for discard_request in [false, true] {
        let (selector, mut driver) = pair(1);
        let (sender, receiver) = streams(&selector);
        let never = Cancellation::never();
        let mut waiting = Box::pin(selector.socket_wait(&receiver, Interest::Readable, &never));
        assert!(block_on(poll_once(&mut waiting)).is_none());
        let request = submitted(&driver);
        if discard_request {
            drop(request);
        } else {
            request.finish(Err(io::ErrorKind::PermissionDenied.into()));
        }
        let Err(OperationError::Io(error)) = block_on(waiting) else {
            panic!("expected I/O error")
        };
        assert_eq!(
            error.kind(),
            if discard_request {
                io::ErrorKind::BrokenPipe
            } else {
                io::ErrorKind::PermissionDenied
            }
        );
        block_on(selector.socket_close(receiver)).unwrap();
        block_on(selector.socket_close(sender)).unwrap();
        drain(&selector, &mut driver);
    }
}

#[test]
fn admission_cancellation_exhaustion_and_closure_are_reported() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    let never = Cancellation::never();
    let mut first = Box::pin(selector.socket_wait(&receiver, Interest::Readable, &never));
    assert!(block_on(poll_once(&mut first)).is_none());
    let token = Cancellation::new();
    let mut second = Box::pin(selector.socket_wait(&receiver, Interest::Readable, &token));
    assert!(block_on(poll_once(&mut second)).is_none());
    token.cancel();
    assert!(matches!(
        block_on(second),
        Err(OperationError::Cancelled(_))
    ));
    drop(first);
    driver.turn(Some(Duration::ZERO)).unwrap();
    selector.controllers[0]
        .next_identifier
        .store(CANCEL_TAG - 1, Ordering::Relaxed);
    assert!(matches!(
        block_on(selector.socket_wait(&receiver, Interest::Readable, &never)),
        Err(OperationError::Io(_))
    ));
    selector.close();
    assert!(matches!(
        block_on(selector.socket_send_poll(&sender, b"x", SocketSendFlags::default(), &never)),
        Err(OperationError::Io(_))
    ));
    assert!(matches!(
        block_on(selector.socket_wait(&receiver, Interest::Readable, &never)),
        Err(OperationError::Io(_))
    ));
    drain(&selector, &mut driver);
}

#[test]
fn interrupted_syscalls_retry_but_check_cancellation_and_closure() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    let mut attempts = 0;
    assert_eq!(
        block_on(selector.socket_poll(
            &receiver,
            Interest::Readable,
            &Cancellation::never(),
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(7)
                }
            }
        ))
        .unwrap(),
        7
    );
    let token = Cancellation::new();
    assert!(matches!(
        block_on(
            selector.socket_poll(&receiver, Interest::Readable, &token, || {
                token.cancel();
                Err::<(), _>(io::ErrorKind::Interrupted.into())
            })
        ),
        Err(OperationError::Cancelled(_))
    ));
    assert!(matches!(
        block_on(
            selector.socket_poll(&sender, Interest::Writable, &Cancellation::never(), || {
                selector.close();
                Err::<(), _>(io::ErrorKind::Interrupted.into())
            })
        ),
        Err(OperationError::Io(_))
    ));
    drain(&selector, &mut driver);
}

#[test]
fn spurious_readiness_retries_on_the_original_ring_after_thread_migration() {
    let (selector, mut drivers) =
        IoUringSelector::with_workers(IoUringOptions::default(), 2).unwrap();
    let (_, receiver) = streams(&selector);
    let token = Cancellation::new();
    let mut attempts = 0;
    let mut receiving =
        Box::pin(
            selector.socket_poll(&receiver, Interest::Readable, &token, || {
                attempts += 1;
                if attempts < 3 {
                    Err(io::ErrorKind::WouldBlock.into())
                } else {
                    // Actual progress wins simultaneous cancellation.
                    token.cancel();
                    Ok(7)
                }
            }),
        );
    assert!(block_on(poll_once(&mut receiving)).is_none());
    submitted(&drivers[0]).finish(Ok(libc::POLLIN as usize));
    let receiving = std::thread::scope(|scope| {
        scope
            .spawn(move || {
                assert!(block_on(poll_once(&mut receiving)).is_none());
                receiving
            })
            .join()
            .unwrap()
    });
    assert!(
        drivers[1]
            .reactor
            .as_ref()
            .unwrap()
            .incoming
            .try_recv()
            .is_err()
    );
    submitted(&drivers[0]).finish(Ok(libc::POLLIN as usize));
    assert_eq!(block_on(receiving).unwrap(), 7);
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::from_millis(10))).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn blocked_scalar_and_vector_sends_wait_on_the_ring_and_cancel_safely() {
    for vectored in [false, true] {
        let (selector, mut driver) = pair(1);
        let (sender, receiver) = streams(&selector);
        retain_socket(&sender)
            .get_ref()
            .set_send_buffer_size(4096)
            .unwrap();
        retain_socket(&receiver)
            .get_ref()
            .set_recv_buffer_size(4096)
            .unwrap();
        let payload = [0x5a; 65536];
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match sender.try_send(&payload, SocketSendFlags::default()) {
                Ok(_) => assert!(std::time::Instant::now() < deadline),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("filling socket failed: {error}"),
            }
        }
        let token = Cancellation::new();
        let source = [IoSlice::new(b"pending")];
        let mut sending: std::pin::Pin<Box<dyn Future<Output = OperationResult<usize>>>> =
            if vectored {
                Box::pin(selector.socket_sendmsg_poll(
                    &sender,
                    &source,
                    None,
                    SocketSendFlags::default(),
                    &token,
                ))
            } else {
                Box::pin(selector.socket_send_poll(
                    &sender,
                    b"pending",
                    SocketSendFlags::default(),
                    &token,
                ))
            };
        assert!(block_on(poll_once(&mut sending)).is_none());
        let mut request = submitted(&driver);
        assert_eq!(request.entry().get_opcode() as u8, opcode::PollAdd::CODE);
        selector.controllers[0]
            .send(Command::Submit(request))
            .unwrap();
        token.cancel();
        assert!(matches!(
            drive(&mut driver, sending),
            Err(OperationError::Cancelled(_))
        ));
        block_on(selector.socket_close(sender)).unwrap();
        block_on(selector.socket_close(receiver)).unwrap();
        drain(&selector, &mut driver);
    }
}

#[test]
fn failed_driver_disconnects_readiness_waiters_without_accessing_borrowed_bytes() {
    let (selector, mut driver) = pair(1);
    let (sender, receiver) = streams(&selector);
    let never = Cancellation::never();
    let mut target = [0x5a];
    let mut receiving = Box::pin(selector.socket_recv_poll(
        &receiver,
        &mut target,
        SocketRecvFlags::default(),
        &never,
    ));
    assert!(block_on(poll_once(&mut receiving)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert_eq!(driver.reactor.as_ref().unwrap().in_flight.0.len(), 1);
    drop(driver);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| block_on(receiving))).is_err()
    );
    assert_eq!(target, [0x5a]);
    drop(sender);
    drop(receiver);
    block_on(selector.shutdown());
}
