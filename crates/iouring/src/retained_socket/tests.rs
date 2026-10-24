// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::*;
use crate::{IoUringSelector, test_buffers::LargeMapping};
use core_scheduler::{Cancellation, SocketClose, SocketDomain, SocketOpen, SocketType};
use futures_lite::future::block_on;

#[test]
fn oversized_native_messages_are_rejected_before_accessing_payload_pages() {
    let (selector, _driver) = IoUringSelector::new(Default::default()).unwrap();
    let socket = block_on(selector.socket_open(
        SocketDomain::Ipv4,
        SocketType::Datagram,
        None,
        &Cancellation::never(),
    ))
    .unwrap();
    let retained = socket.retain_socket();
    let mut mapping = LargeMapping::new();
    assert_eq!(
        recvmsg(
            &retained,
            &mut [IoSliceMut::new(mapping.bytes_mut())],
            SocketRecvFlags::default(),
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(
        sendmsg(
            &retained,
            &[IoSlice::new(mapping.bytes())],
            None,
            SocketSendFlags::default(),
        )
        .unwrap_err()
        .kind(),
        io::ErrorKind::InvalidInput
    );
    drop(mapping);
    drop(retained);
    block_on(selector.socket_close(socket)).unwrap();
}
