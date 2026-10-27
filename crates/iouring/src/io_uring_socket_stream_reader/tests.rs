// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::retained_socket::SocketSource;
use base_scheduler::{SocketRegistration, SocketStreamReading};
use base_scheduler_selector::{SelectorDriver, SelectorWorker};
use futures_lite::future::{block_on, poll_once};
use std::{
    io::Write,
    time::{Duration, Instant},
};

fn drive<F: Future>(future: F, worker: &mut impl SelectorWorker) -> F::Output {
    let mut future = pin!(future);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(result) = block_on(poll_once(future.as_mut())) {
            return result;
        }
        assert!(Instant::now() < deadline, "stream reader stalled");
        worker.turn(Some(Duration::from_millis(10))).unwrap();
    }
}

fn urgent<const OOB: bool>(consume_urgent: bool) {
    let (selector, driver) = IoUringSelector::new(crate::IoUringOptions::default()).unwrap();
    let mut worker = driver.enter();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut peer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_nodelay(true).unwrap();
    let socket = selector
        .register_socket(listener.accept().unwrap().0)
        .unwrap();
    let mut reader = selector
        .socket_stream_reader(&socket)
        .unwrap()
        .with_oob_support::<OOB>();
    let never = Cancellation::never();
    let mut buffer = [0; 64];
    {
        let mut reading = Box::pin(reader.read_into(&mut buffer, &never));
        assert!(block_on(poll_once(&mut reading)).is_none());
        worker.turn(Some(Duration::from_millis(1))).unwrap();
        peer.write_all(b"x").unwrap();
        worker.turn(Some(Duration::from_millis(1))).unwrap();
        assert_eq!(drive(reading, &mut worker).unwrap(), 1);
    }
    if OOB {
        peer.write_all(b"before").unwrap();
    }
    socket2::SockRef::from(&peer)
        .send_with_flags(b"!", libc::MSG_OOB)
        .unwrap();
    if OOB {
        peer.write_all(b"after").unwrap();
    }
    std::thread::sleep(Duration::from_millis(10));
    if consume_urgent {
        let mut urgent = [std::mem::MaybeUninit::uninit(); 1];
        assert_eq!(
            socket
                .retain_socket()
                .get_ref()
                .recv_out_of_band(&mut urgent)
                .unwrap(),
            1
        );
    }
    if !OOB {
        worker.turn(Some(Duration::from_millis(10))).unwrap();
        assert_eq!(
            drive(reader.read_into(&mut buffer, &never), &mut worker)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        // Urgent state remains sticky after normal readiness is consumed.
        assert_eq!(
            block_on(reader.read_into(&mut buffer, &never))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
    }
    let mut reader = reader.with_oob_support::<true>();
    if !OOB {
        peer.write_all(b"beforeafter").unwrap();
    }
    let mut received = Vec::new();
    while received.len() < 11 {
        let count = drive(reader.read_into(&mut buffer, &never), &mut worker).unwrap();
        assert!(count != 0);
        received.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(received, b"beforeafter");
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(
        drive(reader.read_into(&mut buffer, &never), &mut worker).unwrap(),
        0
    );
    let mut reader = reader.with_oob_support::<false>();
    assert_eq!(block_on(reader.read_into(&mut buffer, &never)).unwrap(), 0);
    drive(reader.close(), &mut worker).unwrap();
    let mut reader = reader.with_oob_support::<true>();
    assert_eq!(
        block_on(reader.read_into(&mut buffer, &never))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn default_reader_rejects_reported_urgent_data_and_can_opt_in() {
    urgent::<false>(false);
}

#[test]
fn opt_in_reads_past_urgent_marks_with_or_without_an_alias_consuming_oob() {
    urgent::<true>(false);
    urgent::<true>(true);
}
