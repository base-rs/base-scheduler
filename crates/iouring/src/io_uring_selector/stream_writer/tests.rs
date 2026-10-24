// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::super::tests::{drive, pair};
use super::*;
use futures_lite::future::{block_on, poll_once};
use std::time::Instant;

#[derive(Debug)]
struct Buffer {
    bytes: Box<[u8]>,
    drops: Arc<AtomicUsize>,
}
impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
// SAFETY: The box owns stable initialized bytes until the complete owner is dropped.
unsafe impl IoBuf for Buffer {}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Release);
    }
}

fn buffer(length: usize, byte: u8, drops: &Arc<AtomicUsize>) -> Buffer {
    Buffer {
        bytes: vec![byte; length].into_boxed_slice(),
        drops: Arc::clone(drops),
    }
}

fn stream(selector: &IoUringSelector) -> (NativeSocket, NativeSocket) {
    block_on(core_scheduler_conformance::socket::stream_pair(
        selector,
        SocketDomain::Ipv4,
    ))
}

fn small_send_buffer(socket: &NativeSocket) {
    socket::retain_socket(socket)
        .get_ref()
        .set_send_buffer_size(4096)
        .unwrap();
}

#[test]
fn immediate_writes_release_buffers_and_reuse_writer_without_allocating() {
    let (selector, mut driver) = pair(16);
    let (sender, receiver) = stream(&selector);
    let mut writer = selector.socket_stream_writer::<Buffer>(&sender).unwrap();
    let never = Cancellation::never();
    let drops = Arc::new(AtomicUsize::new(0));
    for iteration in 0..8 {
        let bytes = buffer(16, iteration, &drops);
        let counts = allocation_counter::measure(|| {
            block_on(writer.submit(&selector, bytes, &never)).unwrap();
            block_on(writer.flush(&selector, &never)).unwrap();
        });
        assert_eq!(counts.count_total, 0);
        assert_eq!(drops.load(Ordering::Acquire), iteration as usize + 1);
        let (result, received) = drive(
            &mut driver,
            selector.socket_read(&receiver, vec![0; 16], &never),
        );
        assert_eq!(result.unwrap(), 16);
        assert_eq!(received, vec![iteration; 16]);
    }
    block_on(writer.submit(&selector, buffer(0, 0, &drops), &never)).unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 9);
    block_on(writer.close(&selector)).unwrap();
    block_on(writer.close(&selector)).unwrap();
    assert!(block_on(writer.flush(&selector, &never)).is_err());
    drop(writer);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
}

#[test]
fn stream_type_scheduler_and_pre_cancellation_checks_preserve_new_buffers() {
    let (selector, _driver) = pair(8);
    let (other, _other_driver) = pair(8);
    let never = Cancellation::never();
    let datagram =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Datagram, None, &never))
            .unwrap();
    assert!(
        matches!(selector.socket_stream_writer::<Vec<u8>>(&datagram), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
    );
    let (sender, receiver) = stream(&selector);
    let mut writer = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
    let token = Cancellation::new();
    token.cancel();
    let (error, bytes) = block_on(writer.submit(&selector, vec![1, 2], &token)).unwrap_err();
    assert!(matches!(error, OperationError::Cancelled(_)));
    assert_eq!(bytes, [1, 2]);
    let (error, bytes) = block_on(writer.submit(&other, vec![3], &never)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(bytes, [3]);
    assert_eq!(
        block_on(writer.flush(&other, &never)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        block_on(writer.close(&other)).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert!(!writer.closed);
    block_on(writer.close(&selector)).unwrap();
    let (error, bytes) = block_on(writer.submit(&selector, vec![4], &never)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(bytes, [4]);
    drop(writer);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
    block_on(selector.socket_close(datagram)).unwrap();
}

#[test]
fn stalled_write_finishes_and_releases_its_owner_without_polling_a_writer_future() {
    let (selector, mut driver) = pair(16);
    let (sender, receiver) = stream(&selector);
    small_send_buffer(&sender);
    let mut writer = selector.socket_stream_writer::<Buffer>(&sender).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let never = Cancellation::never();
    let length = 1024 * 1024;
    block_on(writer.submit(&selector, buffer(length, 0x5a, &drops), &never)).unwrap();
    assert!(
        writer.running.is_some(),
        "small send capacity must leave a pending write"
    );
    assert_eq!(drops.load(Ordering::Acquire), 0);
    let mut received = 0;
    let mut bytes = vec![0; 65536];
    let deadline = Instant::now() + Duration::from_secs(10);
    while received < length || drops.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        driver.turn(Some(Duration::ZERO)).unwrap();
        match receiver.try_recv(&mut bytes, SocketRecvFlags::default()) {
            Ok(count) => {
                assert!(count > 0);
                assert!(bytes[..count].iter().all(|byte| *byte == 0x5a));
                received += count;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("receive failed: {error}"),
        }
    }
    assert_eq!(received, length);
    assert_eq!(drops.load(Ordering::Acquire), 1);
    block_on(writer.flush(&selector, &never)).unwrap();
    assert!(writer.cached.is_some());
    block_on(writer.submit(&selector, buffer(length, 0x6b, &drops), &never)).unwrap();
    drive(&mut driver, writer.close(&selector)).unwrap();
    assert_eq!(drops.load(Ordering::Acquire), 2);
    drop(writer);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
}

#[test]
fn cancelled_flush_does_not_cancel_accepted_output_but_submission_token_does() {
    for before_preparation in [true, false] {
        let (selector, mut driver) = pair(16);
        let (sender, receiver) = stream(&selector);
        small_send_buffer(&sender);
        let mut writer = selector.socket_stream_writer::<Buffer>(&sender).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let token = Cancellation::new();
        block_on(writer.submit(&selector, buffer(4 * 1024 * 1024, 7, &drops), &token)).unwrap();
        assert!(writer.running.is_some());
        let wait = Cancellation::new();
        wait.cancel();
        assert!(matches!(
            block_on(writer.flush(&selector, &wait)),
            Err(OperationError::Cancelled(_))
        ));
        assert!(!token.is_cancelled());
        if !before_preparation {
            for _ in 0..16 {
                driver.turn(Some(Duration::ZERO)).unwrap();
            }
        }
        token.cancel();
        let deadline = Instant::now() + Duration::from_secs(5);
        while drops.load(Ordering::Acquire) == 0 {
            assert!(
                Instant::now() < deadline,
                "accepted cancellation did not progress"
            );
            driver.turn(Some(Duration::ZERO)).unwrap();
        }
        assert!(matches!(
            block_on(writer.flush(&selector, &Cancellation::never())),
            Err(OperationError::Cancelled(_))
        ));
        let (error, rejected) =
            block_on(writer.submit(&selector, buffer(1, 8, &drops), &Cancellation::never()))
                .unwrap_err();
        assert!(matches!(error, OperationError::Cancelled(_)));
        assert_eq!(rejected.as_ref(), [8]);
        drop(rejected);
        block_on(writer.close(&selector)).unwrap();
        drop(writer);
        block_on(selector.socket_close(sender)).unwrap();
        block_on(selector.socket_close(receiver)).unwrap();
    }
}

#[test]
fn next_submission_waits_and_cancelled_admission_does_not_touch_the_previous_write() {
    let (selector, mut driver) = pair(16);
    let (sender, receiver) = stream(&selector);
    small_send_buffer(&sender);
    let mut writer = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
    let never = Cancellation::never();
    block_on(writer.submit(&selector, vec![1; 4 * 1024 * 1024], &never)).unwrap();
    let token = Cancellation::new();
    let mut next = Box::pin(writer.submit(&selector, vec![2; 16], &token));
    assert!(block_on(poll_once(&mut next)).is_none());
    token.cancel();
    let (error, bytes) = block_on(next).unwrap_err();
    assert!(matches!(error, OperationError::Cancelled(_)));
    assert_eq!(bytes, vec![2; 16]);
    assert!(writer.running.is_some());
    drive(&mut driver, writer.close(&selector)).unwrap();
    drop(writer);
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
}

#[test]
fn direct_send_errors_are_accepted_then_reported_and_drop_retires_pending_output() {
    use super::super::tests::Failures;
    let (selector, mut driver) = pair(16);
    let (sender, receiver) = stream(&selector);
    let mut writer = selector.socket_stream_writer::<Vec<u8>>(&sender).unwrap();
    let _failure = Failures::new([("stream-write", io::ErrorKind::BrokenPipe.into())]);
    block_on(writer.submit(&selector, vec![1], &Cancellation::never())).unwrap();
    for _ in 0..2 {
        assert_eq!(
            block_on(writer.flush(&selector, &Cancellation::never()))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }
    block_on(writer.close(&selector)).unwrap();
    drop(writer);
    small_send_buffer(&sender);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut writer = selector.socket_stream_writer::<Buffer>(&sender).unwrap();
    block_on(writer.submit(
        &selector,
        buffer(4 * 1024 * 1024, 2, &drops),
        &Cancellation::never(),
    ))
    .unwrap();
    drop(writer);
    let deadline = Instant::now() + Duration::from_secs(5);
    while drops.load(Ordering::Acquire) == 0 {
        assert!(Instant::now() < deadline);
        driver.turn(Some(Duration::ZERO)).unwrap();
    }
    block_on(selector.socket_close(sender)).unwrap();
    block_on(selector.socket_close(receiver)).unwrap();
}
