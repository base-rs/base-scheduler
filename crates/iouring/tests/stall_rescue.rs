// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#![cfg(target_os = "linux")]

// Native stall rescue does not yet reach in-flight io_uring operations. Under single-issuer deferred task running only the owning worker may reap its ring, so they wait for it (option 1 in `design/stall-rescue.md`). Timers on that worker are still rescued.

use base_scheduler::*;
use base_scheduler_iouring::{IoUringOptions, IoUringSelector};
use base_scheduler_native::{NativeExecutorOptions, NativeScheduler};
use futures_lite::future::block_on;
use std::{
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const HOLD: Duration = Duration::from_millis(500);
const DELAY: Duration = Duration::from_millis(50);
const LATE: Duration = Duration::from_millis(250);

#[test]
fn an_in_flight_ring_read_waits_for_its_stuck_owner_while_timers_are_rescued() {
    let (selector, drivers) = IoUringSelector::with_workers(IoUringOptions::default(), 1).unwrap();
    let scheduler = Arc::new(
        NativeScheduler::with_drivers(
            selector,
            drivers,
            NativeExecutorOptions {
                workers: 1,
                ..Default::default()
            },
        )
        .unwrap(),
    );
    let (sender, receiver) = block_on(base_scheduler_conformance::socket::stream_pair(
        &*scheduler,
        SocketDomain::Ipv4,
    ));
    let (started, start) = mpsc::channel();
    let owner = Arc::clone(&scheduler);
    let reader = scheduler
        .spawn(async move {
            let mut reader = owner.socket_stream_reader(&receiver).unwrap();
            started.send(()).unwrap();
            let (result, bytes) = reader.read_owned(vec![0; 4], &Cancellation::never()).await;
            assert_eq!(&bytes[..result.unwrap()], b"ping");
            Instant::now()
        })
        .unwrap();
    start.recv_timeout(HOLD).unwrap();
    // The read is pending on the only worker's ring. Block that worker inside one poll.
    let (held, hold) = mpsc::channel();
    let blocker = scheduler
        .spawn(async move {
            held.send(()).unwrap();
            std::thread::sleep(HOLD);
        })
        .unwrap();
    hold.recv_timeout(HOLD).unwrap();
    let sent = Instant::now();
    assert_eq!(
        sender
            .try_send(b"ping", SocketSendFlags::default())
            .unwrap(),
        4
    );
    let start = Instant::now();
    block_on(scheduler.sleep(DELAY, &Cancellation::never())).unwrap();
    assert!(start.elapsed() < LATE, "timer took {:?}", start.elapsed());
    let completed = block_on(reader).unwrap();
    assert!(
        completed - sent >= HOLD / 2,
        "ring read completed after {:?} while its owner was stuck",
        completed - sent
    );
    block_on(blocker).unwrap();
    block_on(scheduler.shutdown());
}
