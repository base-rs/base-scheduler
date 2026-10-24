// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use core_scheduler::{
    BufferPoolOptions, BufferPooling, OperationError, SocketClose, SocketDomain, SocketOpen,
    SocketType,
};
use futures_lite::future::block_on;

#[test]
fn cancellation_and_invalid_storage_leave_receive_ahead_uninitialized() {
    let (selector, _driver) = IoUringSelector::new(Default::default()).unwrap();
    let never = Cancellation::never();
    let socket =
        block_on(selector.socket_open(SocketDomain::Ipv4, SocketType::Stream, None, &never))
            .unwrap();
    let pool = selector.buffer_pool(BufferPoolOptions::new(2, 16)).unwrap();
    let mut ahead = ReceiveAhead::default();
    let cancelled = Cancellation::new();
    cancelled.cancel();
    assert!(matches!(
        block_on(ahead.read(&selector, &socket, &pool, &cancelled)),
        Err(OperationError::Cancelled(_))
    ));
    assert!(!ahead.active());
    assert!(matches!(
        block_on(ahead.read(&selector, &socket, &pool, &never)),
        Err(OperationError::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
    ));
    assert!(!ahead.active());
    let (result, buffers) =
        block_on(ahead.read_owned([vec![7, 8]], SocketRecvFlags::default(), true, &cancelled));
    assert!(matches!(result, Err(OperationError::Cancelled(_))));
    assert_eq!(buffers, [vec![7, 8]]);
    block_on(ahead.close()).unwrap();
    block_on(selector.socket_close(socket)).unwrap();
}

#[test]
#[cfg(target_pointer_width = "64")]
fn oversized_buffers_are_rejected_before_starting_or_accessing_a_receiver() {
    use crate::test_buffers::{EmptyRanges, LargeMapping};
    use core_scheduler::{IoBufSequence, IoBufSequenceMut};

    let mut ahead = ReceiveAhead::default();
    let never = Cancellation::never();
    let mut mapping = LargeMapping::new();
    assert_eq!(
        block_on(ahead.read_vectored(
            &mut [IoSliceMut::new(mapping.bytes_mut())],
            SocketRecvFlags::default(),
            true,
            &never,
        ))
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    drop(mapping);
    let mut buffers = EmptyRanges::new();
    assert!(buffers.buffer(EmptyRanges::COUNT - 1).is_empty());
    assert!(buffers.buffer_mut(0).is_empty());
    assert!(std::panic::catch_unwind(|| buffers.buffer(EmptyRanges::COUNT)).is_err());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            buffers.buffer_mut(EmptyRanges::COUNT);
        }))
        .is_err()
    );
    let (result, buffers) =
        block_on(ahead.read_owned(buffers, SocketRecvFlags::default(), true, &never));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(buffers.buffer_count(), EmptyRanges::COUNT);
    assert!(!ahead.active());
}
