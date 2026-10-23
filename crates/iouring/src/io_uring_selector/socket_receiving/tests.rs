use crate::RecvMsgMultishotOptions;
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
    let receiving = block_on(selector.socket_receiver(
        socket,
        pool,
        SocketRecvFlags::default(),
        &Cancellation::never(),
    ))
    .unwrap();
    let IoUringReceiver(Strategy::Provided(receiving)) = receiving else {
        panic!("a worker pool selects provided-buffer reception")
    };
    receiving
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
        let pool = selector
            .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
            .unwrap();
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
        assert_eq!(message.payload(), b"retained");
    }
}

#[test]
fn invalid_sockets_report_receive_errors_and_detach_their_pool() {
    let (selector, mut driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
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
    assert!(
        selector
            .buffer_pool_per_worker(BufferPoolOptions::new(0, 16))
            .is_err()
    );
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(2, 16))
        .unwrap();
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
    assert!(
        selector
            .buffer_pool_per_worker(BufferPoolOptions::new(2, 16))
            .is_err()
    );
    assert!(
        block_on(selector.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never))
            .is_err()
    );
}

#[test]
fn pool_acquisition_waits_cancels_and_recovers_after_a_return() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(2);
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
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
fn selected_terminal_completion_returns_its_lease_and_close_accepts_exhaustion() {
    for (result, flags) in [(-libc::ECONNRESET, 1), (0, 1), (0, 0), (-libc::ENOBUFS, 1)] {
        let (selector, mut driver) = pair(2);
        let (_sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
            &selector,
            SocketDomain::Ipv4,
        ));
        let pool = selector
            .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
            .unwrap();
        let mut receiving = receiver(&selector, &socket, pool.clone());
        let reactor = driver.reactor.as_mut().unwrap();
        let Command::PoolReady(buffers) = reactor.incoming.try_recv().unwrap() else {
            panic!("pool admission")
        };
        let Command::Submit(mut request) = reactor.incoming.try_recv().unwrap() else {
            panic!("receive admission")
        };
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
fn repeated_unregister_failure_stops_the_driver_without_recycling_unknown_storage() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(2);
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
        .unwrap();
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
    assert_eq!(message.payload(), b"held");
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
            let pool = selector
                .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
                .unwrap();
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

#[test]
fn pool_creation_provisions_every_worker_before_any_receive() {
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 3).unwrap();
    let options = BufferPoolOptions {
        alignment: 4096,
        ..BufferPoolOptions::new(3, 257)
    };
    let pool = selector.buffer_pool_per_worker(options).unwrap();
    assert_eq!(pool.capacity(), 9);
    assert_eq!(pool.buffer_size(), 257);
    assert_eq!(pool.alignment(), 4096);
    let mut allocations = std::collections::HashSet::new();
    let group = pool.worker(&selector.controllers[0]).unwrap().group();
    for (controller, driver) in selector.controllers.iter().zip(&mut drivers) {
        let worker = pool.worker(controller).unwrap();
        assert_eq!(worker.group(), group);
        assert_eq!(worker.heap.capacity(), 3);
        let buffers: Vec<_> = (0..3).map(|_| worker.heap.try_acquire().unwrap()).collect();
        for buffer in &buffers {
            assert_eq!(buffer.as_ref().as_ptr() as usize % 4096, 0);
            assert!(allocations.insert(buffer.as_ref().as_ptr() as usize));
        }
        assert!(worker.heap.try_acquire().is_none());
        drop(buffers);
        // Initialization registers on this driver's first turn, without constructing a receiver.
        let registered = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&registered);
        BEFORE_SYSCALL.with(|hook| {
            *hook.borrow_mut() = Some((
                "register-buffer-ring",
                Box::new(move || {
                    observed.store(true, Ordering::Relaxed);
                }),
            ))
        });
        driver.turn(Some(Duration::ZERO)).unwrap();
        assert!(registered.load(Ordering::Relaxed));
        assert!(Arc::ptr_eq(
            driver.reactor.as_ref().unwrap().buffer_pools[group as usize]
                .as_ref()
                .unwrap(),
            worker
        ));
    }
    drop(pool);
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn pool_creation_validates_and_rolls_back_before_publishing_partial_state() {
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 2).unwrap();
    for options in [
        BufferPoolOptions::new(0, 8),
        BufferPoolOptions::new(32769, 8),
        BufferPoolOptions::new(1, 0),
        BufferPoolOptions::new(1, usize::MAX),
        BufferPoolOptions::new(1, i32::MAX as usize),
        BufferPoolOptions {
            alignment: 3,
            ..BufferPoolOptions::new(1, 8)
        },
        BufferPoolOptions {
            alignment: 1usize << (usize::BITS - 1),
            ..BufferPoolOptions::new(1, 8)
        },
    ] {
        assert_eq!(
            selector
                .buffer_pool_per_worker(options)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }
    // Fail the second ring allocation. The first worker must not have received an initialization command, and the shared ID must be reusable.
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some((
            "allocate-buffer-ring",
            Box::new(|| {
                BEFORE_SYSCALL.with(|hook| {
                    *hook.borrow_mut() = Some((
                        "allocate-buffer-ring",
                        Box::new(|| {
                            std::mem::forget(Failures::new([(
                                "allocate-buffer-ring",
                                io::ErrorKind::OutOfMemory.into(),
                            )]));
                        }),
                    ))
                });
            }),
        ))
    });
    assert_eq!(
        selector
            .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::OutOfMemory
    );
    assert!(drivers.iter().all(|driver| {
        driver
            .reactor
            .as_ref()
            .unwrap()
            .incoming
            .try_recv()
            .is_err()
    }));
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
        .unwrap();
    assert_eq!(pool.worker(&selector.controllers[0]).unwrap().group(), 0);
    drop(pool);
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn worker_capacity_is_independent_and_returns_home_from_other_threads() {
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 2).unwrap();
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let first = pool.try_acquire().unwrap();
    let second = pool.try_acquire().unwrap();
    let address = first.as_ref().as_ptr() as usize;
    let held = first.freeze();
    drop(second);
    assert!(
        pool.try_acquire().is_none(),
        "worker zero does not borrow worker one's free slot"
    );
    assert!(pool.try_acquire().is_some());
    std::thread::spawn(move || drop(held)).join().unwrap();
    assert_eq!(
        pool.try_acquire().unwrap().as_ref().as_ptr() as usize,
        address
    );
    drop(pool);
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn worker_local_acquisition_uses_the_entered_driver() {
    use core_scheduler_selector::SelectorDriver;
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 2).unwrap();
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let mut worker = drivers.pop().unwrap().enter();
    let first = pool.try_acquire().unwrap();
    assert!(
        pool.worker(&selector.controllers[1])
            .unwrap()
            .heap
            .try_acquire()
            .is_none()
    );
    assert!(
        pool.worker(&selector.controllers[0])
            .unwrap()
            .heap
            .try_acquire()
            .is_some()
    );
    drop(first);
    let lease = block_on(pool.acquire(&Cancellation::never())).unwrap();
    drop(lease);
    drop(pool);
    use core_scheduler_selector::SelectorWorker;
    selector.close();
    while !worker.turn(Some(Duration::ZERO)).unwrap() {}
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    drop(worker);
    block_on(selector.shutdown());
}

#[test]
fn group_ids_are_reserved_across_rings_until_all_registrations_retire() {
    let (selector, mut drivers) = IoUringSelector::with_workers(Default::default(), 2).unwrap();
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let retained = pool.try_acquire().unwrap().freeze();
    let address = retained.as_ref().as_ptr() as usize;
    let first_id = pool.worker(&selector.controllers[0]).unwrap().group();
    for driver in &mut drivers {
        driver.turn(Some(Duration::ZERO)).unwrap();
    }
    let legacy = multishot::pool::Group::new(&selector.controllers[1]).unwrap();
    assert_ne!(legacy.identifier, first_id);
    drop(pool);
    let next = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    assert_ne!(
        next.worker(&selector.controllers[0]).unwrap().group(),
        first_id
    );
    for driver in &mut drivers {
        driver.turn(Some(Duration::ZERO)).unwrap();
    }
    let replacement = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    assert_eq!(
        replacement
            .worker(&selector.controllers[0])
            .unwrap()
            .group(),
        first_id
    );
    assert_ne!(
        replacement.try_acquire().unwrap().as_ref().as_ptr() as usize,
        address
    );
    // Returning a lease from the retired pool cannot affect a newer pool which happens to reuse the numeric group ID.
    drop(retained);
    assert!(replacement.try_acquire().is_some());
    drop((replacement, next, legacy));
    selector.close();
    for driver in &mut drivers {
        while !driver.turn(Some(Duration::ZERO)).unwrap() {}
    }
    block_on(selector.shutdown());
}

#[test]
fn closure_during_initialization_rejects_the_pool() {
    let (selector, mut driver) = pair(2);
    let group = Arc::clone(&selector.group);
    BEFORE_SYSCALL.with(|hook| {
        *hook.borrow_mut() = Some(("allocate-buffer-ring", Box::new(move || group.close())))
    });
    assert_eq!(
        selector
            .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
            .err()
            .unwrap()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
    drain(&selector, &mut driver);
}

#[test]
fn factories_reject_foreign_pools_without_consuming_other_capacity() {
    let (selector, mut driver) = pair(2);
    let (other, mut other_driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never))
            .unwrap();
    for pool in [
        other.buffer_pool(BufferPoolOptions::new(2, 32)).unwrap(),
        other
            .buffer_pool_per_worker(BufferPoolOptions::new(2, 32))
            .unwrap(),
    ] {
        assert!(
            matches!(block_on(selector.socket_receiver(&socket, pool, SocketRecvFlags::default(), &never)), Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
        );
    }
    drain(&selector, &mut driver);
    drain(&other, &mut other_driver);
}

#[test]
fn preparing_one_receiver_does_not_consume_another_receivers_capacity_wakeup() {
    let (selector, mut driver) = pair(2);
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let worker = pool.worker(&selector.controllers[0]).unwrap();
    let held = worker.heap.try_acquire().unwrap();
    worker.attach(0);
    let reactor = driver.reactor.as_ref().unwrap();
    assert!(!worker.service(&reactor.ring.submitter()).unwrap());
    drop(held);
    worker.prepare(&reactor.ring.submitter()).unwrap();
    assert!(worker.has_buffers());
    assert!(worker.service(&reactor.ring.submitter()).unwrap());
    assert!(worker.service(&reactor.ring.submitter()).unwrap());
    worker.detach(0, &reactor.ring.submitter()).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn exhausted_receivers_resume_from_their_own_groups_only() {
    use futures_lite::future::poll_once;
    let (selector, mut driver) = pair(8);
    let never = Cancellation::never();
    let (sender, socket) = block_on(core_scheduler_conformance::socket::stream_pair(
        &selector,
        SocketDomain::Ipv4,
    ));
    let mut legacy = block_on(selector.socket_recvmsg_multishot(
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
    let old_message = drive(&mut driver, legacy.recv(&selector, &never))
        .unwrap()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while driver.reactor.as_ref().unwrap().dormant.is_empty() {
        driver.turn(Some(Duration::from_millis(1))).unwrap();
        assert!(std::time::Instant::now() < deadline);
    }
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let held = pool.try_acquire().unwrap();
    let mut pooled = receiver(&selector, &socket, pool.clone());
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert_eq!(driver.reactor.as_ref().unwrap().dormant.len(), 2);
    drop(held);
    let message = drive(&mut driver, pooled.recv(&selector, &never))
        .unwrap()
        .unwrap();
    assert_eq!(message.payload(), b"b");
    drop(message);
    driver.turn(Some(Duration::ZERO)).unwrap();
    assert!(block_on(poll_once(legacy.recv(&selector, &never))).is_none());
    drive(&mut driver, pooled.close(&selector)).unwrap();
    drive(&mut driver, legacy.close(&selector)).unwrap();
    drop(old_message);
    drain(&selector, &mut driver);
}

#[test]
fn released_pool_retains_its_registration_until_unregistration_succeeds() {
    let (selector, mut driver) = pair(2);
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let worker = Arc::clone(pool.worker(&selector.controllers[0]).unwrap());
    let submitter = driver.reactor.as_ref().unwrap().ring.submitter();
    worker.attach(0);
    worker.prepare(&submitter).unwrap();
    worker.abandon(0);
    drop(pool);
    {
        let _failures = Failures::new([(
            "unregister-buffer-ring",
            io::ErrorKind::PermissionDenied.into(),
        )]);
        assert!(worker.service(&submitter).is_err());
        assert!(worker.heap.try_acquire().is_none());
    }
    assert!(!worker.service(&submitter).unwrap());
    assert!(worker.heap.try_acquire().is_some());
    drain(&selector, &mut driver);
}

#[test]
fn failed_empty_reregistration_is_reported_without_republishing_storage() {
    let (selector, mut driver) = pair(2);
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let worker = pool.worker(&selector.controllers[0]).unwrap();
    let submitter = driver.reactor.as_ref().unwrap().ring.submitter();
    worker.attach(0);
    worker.prepare(&submitter).unwrap();
    worker.abandon(0);
    let _failures = Failures::new([("register-buffer-ring", io::ErrorKind::Unsupported.into())]);
    assert!(!worker.service(&submitter).unwrap());
    assert!(worker.heap.try_acquire().is_some());
    assert_eq!(
        worker.prepare(&submitter).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    drain(&selector, &mut driver);
}

#[test]
fn failed_empty_reregistration_does_not_fail_completed_detach() {
    let (selector, mut driver) = pair(2);
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
    let worker = pool.worker(&selector.controllers[0]).unwrap();
    let submitter = driver.reactor.as_ref().unwrap().ring.submitter();
    worker.attach(0);
    worker.prepare(&submitter).unwrap();
    let _failures = Failures::new([("register-buffer-ring", io::ErrorKind::Unsupported.into())]);
    worker.detach(0, &submitter).unwrap();
    assert!(worker.heap.try_acquire().is_some());
    assert_eq!(
        worker.prepare(&submitter).unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    assert!(!worker.service(&submitter).unwrap());
    driver.turn(Some(Duration::ZERO)).unwrap();
    drain(&selector, &mut driver);
}

#[test]
fn ordinary_pool_factory_preserves_validation_and_admission() {
    let (selector, mut driver) = pair(2);
    assert!(selector.buffer_pool(BufferPoolOptions::new(0, 8)).is_err());
    selector.close();
    assert!(selector.buffer_pool(BufferPoolOptions::new(2, 8)).is_err());
    drain(&selector, &mut driver);
}

#[test]
fn exhausted_request_identifiers_reject_receiver_admission() {
    let (selector, mut driver) = pair(2);
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never))
            .unwrap();
    let pool = selector
        .buffer_pool_per_worker(BufferPoolOptions::new(1, 32))
        .unwrap();
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
}
