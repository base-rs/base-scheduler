// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::retained_socket::SocketSource;
use base_scheduler::{SocketRegistration, SocketStreamReading};
use base_scheduler_selector::NativeSocket;
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
        // Every consuming receive rejects it; a peek leaves it to later reads.
        assert_eq!(
            block_on(reader.read_vectored(&mut [IoSliceMut::new(&mut buffer)], &never))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            block_on(reader.read_into_with_flags(&mut buffer, SocketRecvFlags::default(), &never))
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            block_on(reader.read(&never)).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        let peek = SocketRecvFlags { peek: true };
        assert!(
            block_on(poll_once(reader.read_into_with_flags(
                &mut buffer,
                peek,
                &never
            )))
            .is_none()
        );
    }
    let mut reader = reader.with_oob_support::<true>();
    if !OOB {
        peer.write_all(b"beforeafter").unwrap();
    }
    let mut received = Vec::new();
    while received.len() < 11 {
        // Alternate shapes: each keeps readiness until `WouldBlock`.
        let count = if received.is_empty() {
            drive(reader.read_into(&mut buffer, &never), &mut worker)
        } else {
            drive(
                reader.read_vectored(&mut [IoSliceMut::new(&mut buffer)], &never),
                &mut worker,
            )
        }
        .unwrap();
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

fn setup() -> (
    IoUringSelector,
    impl SelectorWorker,
    NativeSocket,
    std::net::TcpStream,
) {
    let (selector, driver) = IoUringSelector::new(crate::IoUringOptions::default()).unwrap();
    let worker = driver.enter();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let peer = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_nodelay(true).unwrap();
    let socket = selector
        .register_socket(listener.accept().unwrap().0)
        .unwrap();
    (selector, worker, socket, peer)
}

// Turn the driver until the reader's next receive would reach the socket.
fn await_readable(reader: &IoUringSocketStreamReader<'_>, worker: &mut impl SelectorWorker) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while reader
        .scheduler
        .socket_stream_drained(&reader.monitor, reader.probe)
    {
        assert!(Instant::now() < deadline, "readability was not delivered");
        worker.turn(Some(Duration::from_millis(10))).unwrap();
    }
}

#[test]
fn every_consuming_receive_waits_on_readiness_and_clears_it_after_a_short_read() {
    let (selector, mut worker, socket, mut peer) = setup();
    let never = Cancellation::never();
    let mut reader = IoUringSocketStreamReader::<false>::new(&selector, &socket).unwrap();
    let drained = |reader: &IoUringSocketStreamReader<'_>| {
        selector.socket_stream_drained(&reader.monitor, reader.probe)
    };
    let mut bytes = [0; 8];
    // A fresh reader reaches the socket first.
    assert!(!drained(&reader));
    {
        let mut vectors = [IoSliceMut::new(&mut bytes)];
        let mut reading = pin!(reader.read_vectored(&mut vectors, &never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        peer.write_all(b"x").unwrap();
        assert_eq!(drive(reading, &mut worker).unwrap(), 1);
    }
    assert!(drained(&reader));
    // Until the driver publishes readiness, an established receive of any
    // shape waits instead of reaching the socket.
    peer.write_all(b"abc").unwrap();
    {
        let mut vectors = [IoSliceMut::new(&mut bytes)];
        let mut reading = pin!(reader.read_vectored(&mut vectors, &never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        assert_eq!(drive(reading, &mut worker).unwrap(), 3);
    }
    assert_eq!(&bytes[..3], b"abc");
    assert!(drained(&reader));
    peer.write_all(b"defg").unwrap();
    {
        let mut vectors = [IoSliceMut::new(&mut bytes)];
        let mut reading =
            pin!(reader.read_vectored_with_flags(&mut vectors, SocketRecvFlags::default(), &never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        assert_eq!(drive(reading, &mut worker).unwrap().bytes, 4);
    }
    assert_eq!(&bytes[..4], b"defg");
    assert!(drained(&reader));
    // A full receive does not prove the queue drained.
    peer.write_all(b"12345678").unwrap();
    await_readable(&reader, &mut worker);
    {
        let (head, tail) = bytes.split_at_mut(4);
        let mut vectors = [IoSliceMut::new(head), IoSliceMut::new(tail)];
        assert_eq!(
            block_on(reader.read_vectored(&mut vectors, &never)).unwrap(),
            8
        );
    }
    assert!(!drained(&reader));
    {
        let mut vectors = [IoSliceMut::new(&mut bytes)];
        let mut reading = pin!(reader.read_vectored(&mut vectors, &never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
    }
    assert!(drained(&reader));
    // A peek reaches the socket even while drained, and a short peek keeps
    // observed readiness.
    let peek = SocketRecvFlags { peek: true };
    peer.write_all(b"ab").unwrap();
    await_queued(&socket, 2);
    assert_eq!(
        block_on(poll_once(
            reader.read_into_with_flags(&mut bytes, peek, &never)
        ))
        .unwrap()
        .unwrap(),
        2
    );
    await_readable(&reader, &mut worker);
    assert_eq!(
        block_on(reader.read_into_with_flags(&mut bytes, peek, &never)).unwrap(),
        2
    );
    assert!(!drained(&reader));
    assert_eq!(block_on(reader.read_into(&mut bytes, &never)).unwrap(), 2);
    assert_eq!(&bytes[..2], b"ab");
    assert!(drained(&reader));
    // Peeks of nothing at the peer's shutdown leave EOF to consuming
    // receives, which record it.
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(
        drive(
            reader.read_into_with_flags(&mut bytes, peek, &never),
            &mut worker
        )
        .unwrap(),
        0
    );
    let (result, _) = drive(
        reader.read_owned_vectored([vec![0; 8]], peek, &never),
        &mut worker,
    );
    assert_eq!(result.unwrap().bytes, 0);
    assert!(!reader.eof);
    assert_eq!(
        drive(
            reader.read_vectored_with_flags(
                &mut [IoSliceMut::new(&mut bytes)],
                SocketRecvFlags::default(),
                &never
            ),
            &mut worker
        )
        .unwrap()
        .bytes,
        0
    );
    assert!(reader.eof);
    drive(reader.close(), &mut worker).unwrap();
}

#[test]
fn owned_vectored_receives_record_eof() {
    let (selector, mut worker, socket, peer) = setup();
    let never = Cancellation::never();
    let mut reader = IoUringSocketStreamReader::<false>::new(&selector, &socket).unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    let (result, _) = drive(
        reader.read_owned_vectored([vec![0; 8]], SocketRecvFlags::default(), &never),
        &mut worker,
    );
    assert_eq!(result.unwrap().bytes, 0);
    assert!(reader.eof);
    drive(reader.close(), &mut worker).unwrap();
}

// Wait until `count` bytes are queued without consuming them or touching the
// reader's readiness, so a syscall-first assertion cannot race delivery.
fn await_queued(socket: &NativeSocket, count: usize) {
    let socket = socket.retain_socket();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut bytes = [std::mem::MaybeUninit::<u8>::uninit(); 64];
    while !matches!(socket.get_ref().peek(&mut bytes), Ok(queued) if queued >= count) {
        assert!(Instant::now() < deadline, "input was not delivered");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn pooled_reads_wait_on_readiness_without_holding_a_slot() {
    let (selector, mut worker, socket, mut peer) = setup();
    let never = Cancellation::never();
    let pool = selector.buffer_pool(BufferPoolOptions::new(1, 8)).unwrap();
    let mut reader = IoUringSocketStreamReader::<false>::new(&selector, &socket)
        .unwrap()
        .with_pool(pool.clone());
    {
        let mut reading = pin!(reader.read(&never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        drop(block_on(poll_once(pool.acquire(&never))).unwrap().unwrap());
        peer.write_all(b"one").unwrap();
        assert_eq!(
            drive(reading, &mut worker).unwrap().unwrap().as_ref(),
            b"one"
        );
    }
    // The short receive cleared readiness, so the next read waits before
    // taking the slot, even with input already queued.
    assert!(selector.socket_stream_drained(&reader.monitor, reader.probe));
    peer.write_all(b"two").unwrap();
    {
        let mut reading = pin!(reader.read(&never));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        drop(block_on(poll_once(pool.acquire(&never))).unwrap().unwrap());
        assert_eq!(
            drive(reading, &mut worker).unwrap().unwrap().as_ref(),
            b"two"
        );
    }
    // A cancelled wait leaves the slot free.
    let cancellation = Cancellation::new();
    {
        let mut reading = pin!(reader.read(&cancellation));
        assert!(block_on(poll_once(reading.as_mut())).is_none());
        cancellation.cancel();
        assert!(matches!(
            block_on(reading),
            Err(base_scheduler::OperationError::Cancelled(_))
        ));
    }
    drop(block_on(poll_once(pool.acquire(&never))).unwrap().unwrap());
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    assert!(drive(reader.read(&never), &mut worker).unwrap().is_none());
    drive(reader.close(), &mut worker).unwrap();
}
