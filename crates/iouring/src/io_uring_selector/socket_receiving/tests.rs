// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::io_uring_selector::tests::{BEFORE_SYSCALL, Failures, drain, drive, pair};
use futures_lite::future::block_on;

fn receiver(
    selector: &IoUringSelector,
    socket: &NativeSocket,
    pool: crate::IoUringBufferPool,
) -> multishot::RecvMsgMultishot<IoUringBuffer> {
    block_on(selector.socket_receiver(
        socket,
        pool,
        SocketRecvFlags::default(),
        &Cancellation::never(),
    ))
    .unwrap()
}

#[test]
fn factories_reject_foreign_pools_invalid_sizes_and_failed_admission() {
    let (selector, mut driver) = pair(2);
    let (other, mut other_driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    let foreign = other.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    assert!(
        matches!(block_on(selector.socket_receiver(&socket, foreign, SocketRecvFlags::default(), &never)), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
    );
    for options in [
        BufferPoolOptions {
            alignment: 3,
            ..BufferPoolOptions::new(1, 8)
        },
        BufferPoolOptions::new(1, 0),
        BufferPoolOptions::new(1, usize::MAX),
        BufferPoolOptions::new(usize::MAX, 16),
    ] {
        assert!(selector.buffer_pool(options).is_err());
    }
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    {
        let _failures =
            Failures::new([("allocate-buffer-ring", io::ErrorKind::OutOfMemory.into())]);
        assert!(
            block_on(selector.socket_receiver(
                &socket,
                pool.clone(),
                SocketRecvFlags::default(),
                &never
            ))
            .is_err()
        );
    }
    *selector.controllers[0].buffer_groups.lock().unwrap() = vec![true; u16::MAX as usize + 1];
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags::default(),
            &never
        ))
        .is_err()
    );
    selector.controllers[0]
        .buffer_groups
        .lock()
        .unwrap()
        .clear();
    selector.controllers[0]
        .next_identifier
        .store(CANCEL_TAG - 2, Ordering::Relaxed);
    assert!(
        block_on(selector.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never))
            .is_err()
    );
    selector.controllers[0]
        .next_identifier
        .store(0, Ordering::Relaxed);
    drain(&selector, &mut driver);
    drain(&other, &mut other_driver);
}

#[test]
fn closure_after_admission_returns_a_terminal_receiver_without_publishing_storage() {
    let (selector, mut driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 16)).unwrap();
    let group = Arc::clone(&selector.group);
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some(("allocate-buffer-ring", Box::new(move || group.close())))
    });
    let mut receiving = receiver(&selector, &socket, pool.clone());
    assert!(block_on(receiving.recv(&selector, &never)).is_err());
    drain(&selector, &mut driver);
    assert!(pool.try_acquire().is_some());
}

#[test]
fn receive_failures_and_abandonment_preserve_retained_storage() {
    for failure in [
        "register-buffer-ring",
        "unregister-buffer-ring",
        "driver",
        "queued",
    ] {
        let (selector, mut driver) = pair(3);
        let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
        let mut receiving = receiver(&selector, &socket, pool.clone());
        let never = Cancellation::never();
        if failure == "queued" {
            drop(driver);
            assert!(block_on(receiving.recv(&selector, &never)).is_err());
            continue;
        }
        sender
            .try_send(b"retained", SocketSendFlags::default())
            .unwrap();
        if failure == "register-buffer-ring" {
            let _failures =
                Failures::new([("register-buffer-ring", io::ErrorKind::Unsupported.into())]);
            assert!(drive(&mut driver, receiving.recv(&selector, &never)).is_err());
            drain(&selector, &mut driver);
            continue;
        }
        let message = drive(&mut driver, receiving.recv(&selector, &never))
            .unwrap()
            .unwrap();
        if failure == "driver" {
            drop(driver);
            assert!(block_on(receiving.recv(&selector, &never)).is_err());
        } else {
            let _failures = Failures::new([(
                "unregister-buffer-ring",
                io::ErrorKind::PermissionDenied.into(),
            )]);
            assert!(drive(&mut driver, receiving.close(&selector)).is_err());
            drain(&selector, &mut driver);
        }
        drop((selector, receiving));
        if failure == "driver" {
            assert!(pool.try_acquire().is_none());
        }
        drop(pool);
        assert_eq!(message.buffer.as_ref(), b"retained");
    }
}

#[test]
fn invalid_sockets_report_receive_errors_and_detach_their_pool() {
    let (selector, mut driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    let pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
    let mut receiving = receiver(&selector, &socket, pool.clone());
    assert!(drive(&mut driver, receiving.recv(&selector, &never)).is_err());
    drain(&selector, &mut driver);
    assert!(pool.try_acquire().is_some());
}

#[test]
fn factory_preserves_admission_errors_and_socket_type_failure() {
    let (selector, _driver) = pair(2);
    let never = Cancellation::never();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    assert!(selector.buffer_pool(BufferPoolOptions::new(0, 16)).is_err());
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 16)).unwrap();
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags::default(),
            &cancelled
        ))
        .is_err()
    );
    let _failures = Failures::new([("socket-type", io::ErrorKind::PermissionDenied.into())]);
    assert!(
        matches!(block_on(selector.socket_receiver(&socket, pool.clone(), SocketRecvFlags::default(), &never)), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags { peek: true },
            &never
        ))
        .is_err()
    );
    let group = Arc::clone(&selector.group);
    BEFORE_SYSCALL
        .with(|hook| *hook.borrow_mut() = Some(("socket-type", Box::new(move || group.close()))));
    assert!(
        block_on(selector.socket_receiver(
            &socket,
            pool.clone(),
            SocketRecvFlags::default(),
            &never
        ))
        .is_err()
    );
    selector.close();
    assert!(selector.buffer_pool(BufferPoolOptions::new(2, 16)).is_err());
    assert!(
        block_on(selector.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never))
            .is_err()
    );
}

#[test]
fn pool_acquisition_waits_cancels_and_recovers_after_a_return() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(2);
    let pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
    let token = Cancellation::new();
    token.cancel();
    assert!(block_on(pool.acquire(&token)).is_err());
    let first = pool.try_acquire().unwrap();
    let never = Cancellation::never();
    let mut acquiring = Box::pin(pool.acquire(&never));
    assert!(block_on(poll_once(&mut acquiring)).is_none());
    drop(first);
    let buffer = block_on(acquiring).unwrap();
    assert_eq!(buffer.as_ref().len(), 32);
    drop(buffer);
    drain(&selector, &mut driver);
}

#[test]
fn exhausted_worker_reclaims_a_return_published_before_its_capacity_wake() {
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 2).unwrap();
    let pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
    let donor = pool.worker(&selector.controllers[0]).unwrap();
    let waiting = pool.worker(&selector.controllers[1]).unwrap();
    donor.attach(1);
    waiting.attach(2);
    let held = pool.try_acquire().unwrap();
    waiting.demand();
    assert!(
        !waiting
            .service(&drivers[1].reactor.as_ref().unwrap().ring.submitter())
            .unwrap()
    );

    // A different worker claims and publishes the return before the capacity waiter runs. Waking a waiter does not reserve storage for it.
    drop(held);
    donor.demand();
    let reactor = drivers[0].reactor.as_ref().unwrap();
    assert!(donor.service(&reactor.ring.submitter()).unwrap());
    assert!(
        !waiting
            .service(&drivers[1].reactor.as_ref().unwrap().ring.submitter())
            .unwrap()
    );
    let Command::PoolReclaim(reclaimed) = reactor.incoming.try_recv().unwrap() else {
        panic!("the exhausted retry must recall the donor's published storage")
    };
    assert!(Arc::ptr_eq(&reclaimed, &donor));
    donor.begin_reclaim();
    assert!(donor.service(&reactor.ring.submitter()).unwrap());
    assert!(
        waiting
            .service(&drivers[1].reactor.as_ref().unwrap().ring.submitter())
            .unwrap()
    );
    donor.detach(1, &reactor.ring.submitter()).unwrap();
    waiting
        .detach(2, &drivers[1].reactor.as_ref().unwrap().ring.submitter())
        .unwrap();
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn public_acquisition_reclaims_a_return_published_before_its_capacity_wake() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(2);
    let pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
    let donor = pool.worker(&selector.controllers[0]).unwrap();
    donor.attach(1);
    let held = pool.try_acquire().unwrap();
    let never = Cancellation::never();
    let mut acquiring = Box::pin(pool.acquire(&never));
    assert!(block_on(poll_once(&mut acquiring)).is_none());
    drop(held);
    donor.demand();
    let reactor = driver.reactor.as_ref().unwrap();
    assert!(donor.service(&reactor.ring.submitter()).unwrap());
    assert!(block_on(poll_once(&mut acquiring)).is_none());
    let Command::PoolReclaim(reclaimed) = reactor.incoming.try_recv().unwrap() else {
        panic!("the exhausted retry must recall the donor's published storage")
    };
    assert!(Arc::ptr_eq(&reclaimed, &donor));
    donor.begin_reclaim();
    assert!(donor.service(&reactor.ring.submitter()).unwrap());
    let buffer = block_on(poll_once(&mut acquiring)).unwrap().unwrap();
    assert_eq!(buffer.as_ref().len(), 32);
    drop(buffer);
    donor.detach(1, &reactor.ring.submitter()).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn reclaim_suspends_pending_receives_but_leaves_other_requests_alone() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(8);
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    let mut first = receiver(&selector, &socket, pool.clone());
    sender
        .try_send(b"first", SocketSendFlags::default())
        .unwrap();
    let message = drive(&mut driver, first.recv(&selector, &Cancellation::never()))
        .unwrap()
        .unwrap();
    let mut second = receiver(&selector, &socket, pool.clone());
    let other_pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
    let mut unrelated = receiver(&selector, &socket, other_pool);
    let never = Cancellation::never();
    let mut ordinary = Box::pin(selector.socket_read(&socket, vec![0; 32], &never));
    assert!(block_on(poll_once(&mut ordinary)).is_none());
    assert!(pool.try_acquire().is_none());
    let returned = drive(&mut driver, pool.acquire(&never)).unwrap();
    assert_eq!(message.buffer.as_ref(), b"first");
    drop((returned, message, ordinary));
    drive(&mut driver, first.close(&selector)).unwrap();
    drive(&mut driver, second.close(&selector)).unwrap();
    drive(&mut driver, unrelated.close(&selector)).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn reclamation_failure_retains_published_capacity_and_notifies_waiters() {
    let (selector, mut driver) = pair(2);
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    let mut receiving = receiver(&selector, &socket, pool.clone());
    let never = Cancellation::never();
    sender
        .try_send(b"held", SocketSendFlags::default())
        .unwrap();
    let message = drive(&mut driver, receiving.recv(&selector, &never))
        .unwrap()
        .unwrap();
    assert!(pool.try_acquire().is_none());
    let _failures = Failures::new((0..8).map(|_| {
        (
            "unregister-buffer-ring",
            io::ErrorKind::PermissionDenied.into(),
        )
    }));
    let mut failed = false;
    for _ in 0..100 {
        if driver.turn(Some(Duration::from_millis(1))).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(block_on(receiving.recv(&selector, &never)).is_err());
    drop((receiving, driver, selector));
    assert_eq!(message.buffer.as_ref(), b"held");
    drop(message);
    let available = pool.try_acquire().unwrap();
    assert!(
        pool.try_acquire().is_none(),
        "unknown published storage must not be recycled"
    );
    drop(available);
}

#[test]
fn selected_terminal_completion_returns_its_lease_and_close_accepts_exhaustion() {
    for (result, flags) in [(-libc::ECONNRESET, 1), (0, 1), (0, 0), (-libc::ENOBUFS, 1)] {
        let (selector, mut driver) = pair(2);
        let (_sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        let pool = selector.buffer_pool(BufferPoolOptions::new(1, 32)).unwrap();
        let mut receiving = receiver(&selector, &socket, pool.clone());
        let reactor = driver.reactor.as_mut().unwrap();
        let Command::PoolReady(buffers) = reactor.incoming.try_recv().unwrap() else {
            panic!("pool admission")
        };
        let Command::Submit(mut request) = reactor.incoming.try_recv().unwrap() else {
            panic!("receive admission")
        };
        buffers.demand();
        assert!(buffers.service(&reactor.ring.submitter()).unwrap());
        assert!(request.ready(&reactor.ring.submitter()).unwrap());
        // Decode legitimate terminal CQE forms independently of whether this kernel selects a buffer for error/EOF. No receive SQE is submitted, so the kernel cannot access the test lease.
        request.completion(result, flags); // Optional IORING_CQE_F_BUFFER, buffer zero, no MORE.
        request.retire_completion(&reactor.ring.submitter(), result);
        if result == -libc::ECONNRESET {
            assert!(block_on(receiving.recv(&selector, &Cancellation::never())).is_err());
        } else {
            block_on(receiving.close(&selector)).unwrap();
        }
        assert!(pool.try_acquire().is_some());
        drain(&selector, &mut driver);
    }
}

#[test]
fn mixed_receivers_rearm_only_their_own_dormant_requests() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(8);
    let never = Cancellation::never();
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let mut old = block_on(selector.socket_recvmsg_multishot(
        &socket,
        RecvMsgMultishotOptions {
            buffers: 1,
            buffer_size: 1,
        },
        SocketRecvFlags::default(),
        &never,
    ))
    .unwrap();
    sender.try_send(b"ab", SocketSendFlags::default()).unwrap();
    let old_message = drive(&mut driver, old.recv(&selector, &never))
        .unwrap()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while driver.reactor.as_ref().unwrap().dormant.is_empty() {
        driver.turn(Some(Duration::from_millis(1))).unwrap();
        assert!(std::time::Instant::now() < deadline);
    }
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    let mut pooled = receiver(&selector, &socket, pool.clone());
    let message = drive(&mut driver, pooled.recv(&selector, &never))
        .unwrap()
        .unwrap();
    // Keep unrelated ordinary I/O in flight while the pool reclaims its unused slot.
    let mut ordinary = Box::pin(selector.socket_read(&socket, vec![0; 32], &never));
    assert!(block_on(poll_once(&mut ordinary)).is_none());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert!(pool.try_acquire().is_none());
    let available = drive(&mut driver, pool.acquire(&never)).unwrap();
    drop((available, message, ordinary));
    drive(&mut driver, pooled.close(&selector)).unwrap();
    drive(&mut driver, old.close(&selector)).unwrap();
    drop(old_message);
    drain(&selector, &mut driver);
}

#[test]
fn repeated_unregister_failure_stops_the_driver_without_recycling_unknown_storage() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(2);
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
    let mut receiving = receiver(&selector, &socket, pool.clone());
    sender
        .try_send(b"held", SocketSendFlags::default())
        .unwrap();
    let message = drive(
        &mut driver,
        receiving.recv(&selector, &Cancellation::never()),
    )
    .unwrap()
    .unwrap();
    let _failures = Failures::new((0..4).map(|_| {
        (
            "unregister-buffer-ring",
            io::ErrorKind::PermissionDenied.into(),
        )
    }));
    let mut closing = Box::pin(receiving.close(&selector));
    assert!(block_on(poll_once(&mut closing)).is_none());
    let mut failed = false;
    for _ in 0..100 {
        if driver.turn(Some(Duration::from_millis(1))).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(block_on(closing).is_err());
    assert_eq!(message.buffer.as_ref(), b"held");
    drop((receiving, driver));
    assert!(pool.try_acquire().is_none());
}

#[test]
fn registration_capacity_errors_are_not_confused_with_receive_exhaustion() {
    for shared in [false, true] {
        let (selector, mut driver) = pair(2);
        let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        sender
            .try_send(b"ready", SocketSendFlags::default())
            .unwrap();
        let _failures = Failures::new([(
            "register-buffer-ring",
            io::Error::from_raw_os_error(libc::ENOBUFS),
        )]);
        let never = Cancellation::never();
        let result = if shared {
            let pool = selector.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap();
            let mut receiving = receiver(&selector, &socket, pool);
            drive(&mut driver, receiving.recv(&selector, &never)).map(|_| ())
        } else {
            let mut receiving = block_on(selector.socket_recvmsg_multishot(
                &socket,
                RecvMsgMultishotOptions::default(),
                SocketRecvFlags::default(),
                &never,
            ))
            .unwrap();
            drive(&mut driver, receiving.recv(&selector, &never)).map(|_| ())
        };
        assert!(
            matches!(result, Err(OperationError::Io(error)) if error.raw_os_error() == Some(libc::ENOBUFS))
        );
        drain(&selector, &mut driver);
    }
}
