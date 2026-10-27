// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::Instant,
};

#[test]
fn notification_is_retained_drained_and_reusable() {
    let poller = Poller::new().unwrap();
    let mut events = Events::new();
    poller.notify().unwrap();
    poller.notify().unwrap();
    assert_eq!(poller.wait(&mut events, None).unwrap(), 1);
    assert_eq!(events.iter().next().unwrap().key, INTERRUPT);
    poller.clear_notification().unwrap();
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    // A notification arriving after the previous drain survives into the next turn.
    poller.notify().unwrap();
    assert_eq!(poller.wait(&mut events, None).unwrap(), 1);
    poller.clear_notification().unwrap();
    events.clear();
    assert!(events.iter().next().is_none());
}

#[test]
fn another_thread_can_interrupt_an_indefinite_wait() {
    let poller = Poller::new().unwrap();
    std::thread::scope(|scope| {
        let (started, ready) = std::sync::mpsc::channel();
        let notify = &poller;
        scope.spawn(move || {
            ready.recv().unwrap();
            notify.notify().unwrap();
        });
        started.send(()).unwrap();
        let mut events = Events::new();
        assert_eq!(poller.wait(&mut events, None).unwrap(), 1);
        poller.clear_notification().unwrap();
    });
}

#[test]
fn edge_registration_reuses_the_native_buffer_and_deletes_synchronously() {
    let poller = Poller::new().unwrap();
    let (stream, mut peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let socket: socket2::Socket = stream.into();
    // SAFETY: The socket stays owned until after successful synchronous deletion.
    unsafe {
        poller
            .add_with_mode(&socket, Event::readable(17), PollMode::Edge)
            .unwrap();
    }
    peer.write_all(b"a").unwrap();
    let mut events = Events::new();
    assert_eq!(
        poller
            .wait(&mut events, Some(Duration::from_secs(1)))
            .unwrap(),
        1
    );
    let event = events.iter().next().unwrap();
    assert_eq!(event.key, 17);
    assert!(event.readable);
    assert!(!event.writable);
    let allocation = events.raw.as_ptr();
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    assert_eq!(events.raw.as_ptr(), allocation);
    let mut byte = [0];
    (&socket).read_exact(&mut byte).unwrap();
    assert_eq!(byte, [b'a']);
    peer.write_all(b"b").unwrap();
    assert_eq!(
        poller
            .wait(&mut events, Some(Duration::from_secs(1)))
            .unwrap(),
        1
    );
    poller.delete(&socket).unwrap();
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
}

use super::syscalls::fault::{Call, drained, plan};

// Exercise real backpressure without allowing a fixture error to masquerade as WouldBlock.
fn until_blocked(mut operation: impl FnMut() -> io::Result<usize>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "socket did not reach WouldBlock");
        match operation() {
            Ok(0) => panic!("unexpected zero progress"),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("unexpected socket failure: {error}"),
        }
    }
}

fn native(poller: &Poller) -> &NativePoller {
    let Backend::Native(native) = &poller.backend else {
        panic!("expected native epoll")
    };
    native
}

fn disarmed(poller: &Poller) {
    let mut value = std::mem::MaybeUninit::uninit();
    // SAFETY: timerfd_gettime initializes the complete setting on success.
    assert_eq!(
        unsafe { libc::timerfd_gettime(native(poller).timer.as_raw_fd(), value.as_mut_ptr()) },
        0
    );
    let value = unsafe { value.assume_init() };
    assert_eq!(value.it_value.tv_sec, 0);
    assert_eq!(value.it_value.tv_nsec, 0);
}

#[test]
fn precise_expiry_is_filtered_and_does_not_leave_stale_readiness() {
    let poller = Poller::new().unwrap();
    assert!(poller.supports_edge());
    let mut events = Events::new();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    let started = Instant::now();
    assert_eq!(
        poller
            .wait(&mut events, Some(Duration::from_micros(100)))
            .unwrap(),
        1
    );
    assert!(started.elapsed() >= Duration::from_micros(100));
    assert!(events.iter().next().is_none());
    disarmed(&poller);
    // Appending a later notification leaves the retained timer record filtered.
    poller.notify().unwrap();
    poller.wait(&mut events, None).unwrap();
    assert_eq!(
        events.iter().map(|event| event.key).collect::<Vec<_>>(),
        [INTERRUPT]
    );
    poller.clear_notification().unwrap();
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    std::thread::scope(|scope| {
        scope.spawn(|| poller.notify().unwrap());
        poller.wait(&mut events, None).unwrap();
    });
    assert_eq!(events.iter().next().unwrap().key, INTERRUPT);
}

#[test]
fn early_socket_and_notification_wakes_disarm_the_timer() {
    let poller = Poller::new().unwrap();
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let socket: socket2::Socket = stream.into();
    unsafe {
        poller
            .add_with_mode(&socket, Event::readable(23), PollMode::Edge)
            .unwrap()
    };
    let mut events = Events::new();
    peer.write_all(b"a").unwrap();
    poller
        .wait(&mut events, Some(Duration::from_millis(2)))
        .unwrap();
    assert_eq!(events.iter().next().unwrap().key, 23);
    disarmed(&poller);
    poller.notify().unwrap();
    events.clear();
    poller
        .wait(&mut events, Some(Duration::from_millis(2)))
        .unwrap();
    assert_eq!(events.iter().next().unwrap().key, INTERRUPT);
    disarmed(&poller);
    poller.clear_notification().unwrap();
    std::thread::sleep(Duration::from_millis(4));
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    poller.delete(&socket).unwrap();
}

#[test]
fn constructor_failures_preserve_compatible_backends() {
    plan(&[(Call::Queue, libc::EMFILE)]);
    assert_eq!(
        Poller::new().err().unwrap().raw_os_error(),
        Some(libc::EMFILE)
    );
    drained();
    for call in [Call::Interrupt, Call::Timer] {
        for error in [
            libc::ENOSYS,
            libc::EPERM,
            libc::EACCES,
            libc::EMFILE,
            libc::ENOMEM,
        ] {
            plan(&[(call, error)]);
            let poller = Poller::new().unwrap();
            drained();
            assert!(matches!(poller.backend, Backend::Compatible(_)));
            assert!(poller.supports_edge());
            let mut events = Events::new();
            assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
            poller.notify().unwrap();
            assert_eq!(poller.wait(&mut events, None).unwrap(), 0);
            let start = Instant::now();
            poller
                .wait(&mut events, Some(Duration::from_micros(100)))
                .unwrap();
            assert!(start.elapsed() >= Duration::from_micros(100));
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let socket: socket2::Socket = stream.into();
            unsafe {
                poller
                    .add_with_mode(&socket, Event::all(25), PollMode::Edge)
                    .unwrap()
            };
            peer.write_all(b"x").unwrap();
            poller.wait(&mut events, None).unwrap();
            let event = events.iter().next().unwrap();
            assert_eq!(event.key, 25);
            assert!(event.readable && event.writable);
            events.clear();
            assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
            poller.delete(&socket).unwrap();
        }
    }
    for prefix in [vec![], vec![(Call::Control, 0)]] {
        let mut calls = prefix;
        calls.push((Call::Control, libc::EIO));
        plan(&calls);
        assert_eq!(Poller::new().err().unwrap().raw_os_error(), Some(libc::EIO));
        drained();
    }
}

#[test]
fn native_control_retries_and_propagates_failures_for_every_interest() {
    plan(&[(Call::Control, libc::EINTR)]);
    let poller = Poller::new().unwrap();
    drained();
    for interest in [
        Event::none(1),
        Event::readable(2),
        Event::writable(3),
        Event::all(4),
    ] {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let socket: socket2::Socket = stream.into();
        plan(&[(Call::Control, libc::EIO)]);
        assert_eq!(
            unsafe { poller.add_with_mode(&socket, interest, PollMode::Edge) }
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EIO)
        );
        drained();
        plan(&[(Call::Control, libc::EINTR)]);
        unsafe {
            poller
                .add_with_mode(&socket, interest, PollMode::Edge)
                .unwrap()
        };
        drained();
        plan(&[(Call::Control, libc::EIO)]);
        assert_eq!(
            poller.delete(&socket).unwrap_err().raw_os_error(),
            Some(libc::EIO)
        );
        drained();
        poller.delete(&socket).unwrap();
    }
}

#[test]
fn notification_retries_and_saturation_preserve_wakes() {
    let poller = Poller::new().unwrap();
    poller.clear_notification().unwrap(); // real EAGAIN
    let value = u64::MAX - 1;
    assert_eq!(
        unsafe {
            libc::write(
                native(&poller).interrupt.as_raw_fd(),
                (&value as *const u64).cast(),
                8,
            )
        },
        8
    );
    poller.notify().unwrap(); // real saturation/EAGAIN
    let mut events = Events::new();
    poller.wait(&mut events, None).unwrap();
    assert_eq!(events.iter().next().unwrap().key, INTERRUPT);
    plan(&[(Call::Read, libc::EINTR)]);
    poller.clear_notification().unwrap();
    drained();
    plan(&[(Call::Write, libc::EINTR)]);
    poller.notify().unwrap();
    drained();
    plan(&[(Call::Read, libc::EIO)]);
    assert_eq!(
        poller.clear_notification().unwrap_err().raw_os_error(),
        Some(libc::EIO)
    );
    drained();
    poller.clear_notification().unwrap();
    plan(&[(Call::Write, libc::EIO)]);
    assert_eq!(poller.notify().unwrap_err().raw_os_error(), Some(libc::EIO));
    drained();
}

#[test]
fn wait_errors_keep_retained_events_and_disarm_before_returning() {
    let poller = Poller::new().unwrap();
    let mut events = Events::new();
    poller
        .wait(&mut events, Some(Duration::from_micros(1)))
        .unwrap();
    poller.notify().unwrap();
    poller.wait(&mut events, None).unwrap();
    let length = events.raw.len();
    for timeout in [Some(Duration::ZERO), Some(Duration::from_secs(1)), None] {
        for error in [libc::EINTR, libc::EIO] {
            plan(&[(Call::Wait, error)]);
            assert_eq!(
                poller
                    .wait(&mut events, timeout)
                    .unwrap_err()
                    .raw_os_error(),
                Some(error)
            );
            drained();
            disarmed(&poller);
            assert_eq!(events.raw.len(), length);
            assert_eq!(
                events.iter().map(|event| event.key).collect::<Vec<_>>(),
                [INTERRUPT]
            );
        }
    }
    plan(&[(Call::SetTime, libc::EINTR)]);
    poller
        .wait(&mut events, Some(Duration::from_secs(1)))
        .unwrap();
    drained();
    disarmed(&poller);
    plan(&[(Call::SetTime, libc::EIO)]);
    assert_eq!(
        poller
            .wait(&mut events, Some(Duration::from_secs(1)))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EIO)
    );
    drained();
    for wait in [0, libc::EINTR, libc::EBADF] {
        plan(&[
            (Call::SetTime, 0),
            (Call::Wait, wait),
            (Call::SetTime, libc::EIO),
        ]);
        assert_eq!(
            poller
                .wait(&mut events, Some(Duration::from_secs(1)))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EIO)
        );
        drained();
        // Cleanup failure is terminal for workers; this lower-level test explicitly repairs it.
        native(&poller).set_timeout(Duration::ZERO).unwrap();
    }
    plan(&[
        (Call::SetTime, 0),
        (Call::Wait, libc::EBADF),
        (Call::SetTime, libc::EINTR),
    ]);
    assert_eq!(
        poller
            .wait(&mut events, Some(Duration::from_secs(1)))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EBADF)
    );
    drained();
    disarmed(&poller);
    poller.clear_notification().unwrap();
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
}

#[test]
fn fatal_notification_and_timer_cleanup_errors_close_the_worker() {
    use crate::{ReadinessSelector, SelectorDriver, SelectorOptions, SelectorWorker};
    for notification in [true, false] {
        let (selector, drivers) =
            ReadinessSelector::with_workers(SelectorOptions::default(), 1).unwrap();
        let mut worker = drivers.into_iter().next().unwrap().enter();
        worker.waker().wake_by_ref();
        if notification {
            plan(&[(Call::Read, libc::EIO)]);
        } else {
            plan(&[
                (Call::SetTime, 0),
                (Call::Wait, libc::EINTR),
                (Call::SetTime, libc::EIO),
            ]);
        }
        assert_eq!(
            worker
                .turn(Some(Duration::from_secs(1)))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EIO)
        );
        drained();
        assert!(selector.readiness.as_ref().unwrap().closed.is_cancelled());
        assert!(worker.turn(None).unwrap());
    }
}

#[test]
fn compatible_events_preserve_direction_and_reuse_their_scratch() {
    plan(&[(Call::Timer, libc::EPERM)]);
    let poller = Poller::new().unwrap();
    drained();
    let (stream, mut peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut socket: socket2::Socket = stream.into();
    // Saturate output so only readable readiness can be delivered.
    let bytes = [0; 8192];
    until_blocked(|| socket.write(&bytes));
    unsafe {
        poller
            .add_with_mode(&socket, Event::all(37), PollMode::Edge)
            .unwrap()
    };
    peer.write_all(b"x").unwrap();
    let mut events = Events::new();
    poller
        .wait(&mut events, Some(Duration::from_secs(1)))
        .unwrap();
    let event = events.iter().next().unwrap();
    assert_eq!(event.key, 37);
    assert!(event.readable);
    assert!(!event.writable);
    poller.delete(&socket).unwrap();
    events.clear();
    let (stream, _peer) = UnixStream::pair().unwrap();
    let socket: socket2::Socket = stream.into();
    unsafe {
        poller
            .add_with_mode(&socket, Event::all(41), PollMode::Edge)
            .unwrap()
    };
    poller
        .wait(&mut events, Some(Duration::from_secs(1)))
        .unwrap();
    let event = events.iter().next().unwrap();
    assert_eq!(event.key, 41);
    assert!(!event.readable);
    assert!(event.writable);
    poller.delete(&socket).unwrap();
}

#[test]
fn compatible_worker_preserves_duplex_waiters_cancellation_and_closure() {
    use crate::{NativeSocket, ReadinessSelector, SelectorDriver, SelectorOptions, SelectorWorker};
    use base_scheduler::*;
    use futures_lite::future::{block_on, poll_once};
    plan(&[(Call::Timer, libc::EPERM)]);
    let (selector, drivers) =
        ReadinessSelector::with_workers(SelectorOptions::default(), 1).unwrap();
    drained();
    let mut worker = drivers.into_iter().next().unwrap().enter();
    let (stream, mut peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut socket: socket2::Socket = stream.into();
    let bytes = [0; 8192];
    until_blocked(|| socket.write(&bytes));
    let socket = NativeSocket::new(socket).unwrap();
    let never = Cancellation::never();
    let cancel = Cancellation::new();
    let mut first = Box::pin(selector.socket_read(&socket, vec![0; 1], &cancel));
    let mut second = Box::pin(selector.socket_read(&socket, vec![0; 1], &never));
    let mut output = Box::pin(selector.socket_write(&socket, b"y".to_vec(), &never));
    assert!(block_on(poll_once(first.as_mut())).is_none());
    assert!(block_on(poll_once(second.as_mut())).is_none());
    assert!(block_on(poll_once(output.as_mut())).is_none());
    cancel.cancel();
    assert!(matches!(
        block_on(first).0,
        Err(OperationError::Cancelled(_))
    ));
    peer.write_all(b"x").unwrap();
    worker.turn(Some(Duration::from_secs(1))).unwrap();
    assert_eq!(block_on(second).0.unwrap(), 1);
    assert!(block_on(poll_once(output.as_mut())).is_none());
    peer.set_nonblocking(true).unwrap();
    until_blocked(|| peer.read(&mut [0; 8192]));
    worker.turn(Some(Duration::from_secs(1))).unwrap();
    assert_eq!(block_on(output).0.unwrap(), 1);
    let mut pending = Box::pin(selector.socket_read(&socket, vec![0; 1], &never));
    assert!(block_on(poll_once(pending.as_mut())).is_none());
    worker.close();
    assert_eq!(
        block_on(pending).0.unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert!(worker.turn(None).unwrap());
    drop(socket);
}

#[test]
fn native_failed_registration_and_deletion_quarantine_until_stop() {
    use crate::{NativeSocket, ReadinessSelector, SelectorDriver, SelectorOptions};
    for add in [false, true] {
        let (selector, drivers) =
            ReadinessSelector::with_workers(SelectorOptions::default(), 1).unwrap();
        let worker = drivers.into_iter().next().unwrap().enter();
        let group = selector.readiness.as_ref().unwrap();
        let owner = &group.controllers[0];
        let socket =
            NativeSocket::new(std::net::UdpSocket::bind("127.0.0.1:0").unwrap().into()).unwrap();
        if add {
            plan(&[(Call::Control, libc::EIO)]);
            assert_eq!(
                socket
                    .registrations
                    .get(group, socket.get_ref())
                    .err()
                    .unwrap()
                    .raw_os_error(),
                Some(libc::EIO)
            );
            drained();
        } else {
            let registration = socket.registrations.get(group, socket.get_ref()).unwrap();
            let weak = std::sync::Arc::downgrade(&registration);
            drop(registration);
            plan(&[(Call::Control, libc::EIO)]);
            drop(socket);
            drained();
            assert!(
                weak.upgrade()
                    .unwrap()
                    .inactive
                    .load(std::sync::atomic::Ordering::Acquire)
            );
        }
        assert_eq!(owner.table.lock().unwrap().registrations.len(), 1);
        selector.close();
        assert_eq!(owner.table.lock().unwrap().registrations.len(), 1);
        drop(worker);
        assert!(owner.table.lock().unwrap().registrations.is_empty());
    }
}

#[test]
fn compatible_creation_and_wait_errors_preserve_os_errors_and_retained_events() {
    plan(&[
        (Call::Timer, libc::EPERM),
        (Call::CompatibleCreate, libc::EMFILE),
    ]);
    assert_eq!(
        Poller::new().err().unwrap().raw_os_error(),
        Some(libc::EMFILE)
    );
    drained();
    plan(&[(Call::Interrupt, libc::ENOSYS)]);
    let poller = Poller::new().unwrap();
    drained();
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let socket: socket2::Socket = stream.into();
    unsafe {
        poller
            .add_with_mode(&socket, Event::readable(43), PollMode::Edge)
            .unwrap()
    };
    peer.write_all(b"x").unwrap();
    let mut events = Events::new();
    poller.wait(&mut events, None).unwrap();
    for error in [libc::EINTR, libc::EIO] {
        plan(&[(Call::CompatibleWait, error)]);
        assert_eq!(
            poller
                .wait(&mut events, Some(Duration::ZERO))
                .unwrap_err()
                .raw_os_error(),
            Some(error)
        );
        drained();
        let retained = events.iter().collect::<Vec<_>>();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].key, 43);
        assert!(retained[0].readable && !retained[0].writable);
    }
    events.clear();
    assert_eq!(poller.wait(&mut events, Some(Duration::ZERO)).unwrap(), 0);
    poller.delete(&socket).unwrap();
}
